#!/usr/bin/env bash
# Recette de rwdwm sur une vraie machine.
#
#   tools/recette.sh > recette.log 2>&1
#
# Le script n'attend aucune saisie. Il enchaîne :
#   1. un relevé de l'environnement (système, Rust, bibliothèques, session) ;
#   2. la compilation (cargo build --release) ;
#   3. les tests automatisés ;
#   4. un lancement à blanc, sans affichage (backend headless) ;
#   5. un lancement réel de rwdwm pendant quelques dizaines de secondes :
#        - dans une fenêtre  si une session graphique tourne (backend winit) ;
#        - sur l'écran entier depuis un terminal virtuel     (backend udev).
#      rwdwm s'arrête tout seul (--exit-after) : aucun risque de rester bloqué.
#
# Tout ce qui est utile au diagnostic part sur la sortie standard ; les
# consignes destinées à la personne devant l'écran sont aussi affichées sur
# le terminal, même si la sortie est redirigée.
#
# Usage : tools/recette.sh [options] [étape]
#   étapes : auto (défaut) | infos | build | tests | headless | winit | udev
#   options :
#     --duree <s>    durée du lancement réel, en secondes (défaut : 45)
#     --sans-tests   saute les tests automatisés
#     -h, --help     cette aide
#
# Variables : RECETTE_CARGO_ARGS (arguments supplémentaires pour cargo),
#             CARGO_TARGET_DIR.

set -u

DUREE=45
ETAPE=auto
AVEC_TESTS=1
while [ $# -gt 0 ]; do
    case "$1" in
        --duree) DUREE="${2:-}"; shift ;;
        --duree=*) DUREE="${1#*=}" ;;
        --sans-tests) AVEC_TESTS=0 ;;
        -h | --help) sed -n '2,28p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        auto | infos | build | tests | headless | winit | udev) ETAPE="$1" ;;
        *) echo "recette.sh : argument inconnu : $1" >&2; exit 2 ;;
    esac
    shift
done
case "$DUREE" in
    '' | *[!0-9]*) echo "recette.sh : --duree attend un nombre entier de secondes" >&2; exit 2 ;;
esac

cd "$(dirname "$0")/.." || exit 1
export CARGO_TERM_COLOR=never NO_COLOR=1 LC_MESSAGES=C
# shellcheck disable=SC2206
CARGO_ARGS=(${RECETTE_CARGO_ARGS:-})
BIN="${CARGO_TARGET_DIR:-target}/release/rwdwm"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/rwdwm-recette.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

RESUME=()
# Niveaux de journalisation de rwdwm pendant la recette.
JOURNAL="${RUST_LOG:-info,rwdwm=debug,smithay=debug}"

# ----------------------------------------------------------------- affichage

# Vrai si la sortie standard est redirigée et qu'un terminal est disponible.
tty_dispo() { [ ! -t 1 ] && { : > /dev/tty; } 2> /dev/null; }

say() { printf '%s\n' "$*"; }

# Message pour la personne devant l'écran : journal + terminal.
dire() {
    printf '%s\n' "$*"
    if tty_dispo; then printf '%s\n' "$*" > /dev/tty; fi
}

titre() {
    say ""
    say "================================================================"
    say "== $*"
    say "================================================================"
    if tty_dispo; then printf '\n[recette] %s\n' "$*" > /dev/tty; fi
}

# Exécute une commande en la journalisant ; renvoie son code de sortie.
run() {
    local debut fin code
    say "\$ $*"
    debut=$(date +%s)
    "$@"
    code=$?
    fin=$(date +%s)
    say "-> code de sortie $code ($((fin - debut)) s)"
    return $code
}

# Affiche la sortie d'une commande si elle existe, sans jamais échouer.
essai() {
    if command -v "$1" > /dev/null 2>&1; then
        say "\$ $*"
        "$@" 2>&1 | sed 's/^/    /'
    else
        say "($1 : commande absente)"
    fi
}

noter() { RESUME+=("$1"); }

# --------------------------------------------------------------------- infos

etape_infos() {
    titre "1. Environnement"
    say "date             : $(date '+%Y-%m-%d %H:%M:%S %z')"
    say "commit           : $(git rev-parse HEAD 2> /dev/null || echo inconnu) ($(git status --short 2> /dev/null | wc -l) fichier(s) modifié(s))"
    say "noyau            : $(uname -srmo)"
    if [ -r /etc/os-release ]; then
        say "distribution     : $(. /etc/os-release && echo "${PRETTY_NAME:-$NAME}")"
    fi
    say "rustc            : $(rustc --version 2>&1)"
    say "cargo            : $(cargo --version 2>&1)"
    say "cc               : $(cc --version 2>&1 | head -n 1)"
    say ""
    say "-- bibliothèques (pkg-config) --"
    if command -v pkg-config > /dev/null 2>&1; then
        for lib in xkbcommon pixman-1 libudev libinput libseat gbm egl glesv2 wayland-client wayland-egl libdrm; do
            printf '%-16s : %s\n' "$lib" "$(pkg-config --modversion "$lib" 2>&1 | head -n 1)"
        done
    else
        say "pkg-config ABSENT (nécessaire à la compilation)"
    fi
    say ""
    say "-- session --"
    for var in XDG_SESSION_TYPE XDG_CURRENT_DESKTOP XDG_RUNTIME_DIR XDG_SEAT XDG_VTNR XDG_SESSION_ID \
        WAYLAND_DISPLAY DISPLAY XCURSOR_THEME XCURSOR_SIZE XKB_DEFAULT_LAYOUT LIBSEAT_BACKEND TERM; do
        printf '%-20s = %s\n' "$var" "${!var-<non définie>}"
    done
    say "terminal (stdin)     = $(tty 2>&1)"
    say "utilisateur root     = $([ "$(id -u)" = 0 ] && echo OUI || echo non)"
    say "groupes              = $(id -Gn 2>&1)"
    if [ -n "${XDG_SESSION_ID:-}" ]; then
        essai loginctl show-session "$XDG_SESSION_ID" -p Type -p Class -p State -p Active -p Seat -p VTNr -p Remote
    fi
    say "seatd en cours       = $(pgrep -x seatd > /dev/null 2>&1 && echo oui || echo non)"
    say ""
    say "-- matériel graphique --"
    essai ls -l /dev/dri
    for f in /sys/class/drm/card*-*/status; do
        [ -r "$f" ] && say "    $(basename "$(dirname "$f")") : $(cat "$f")"
    done
    for f in /sys/class/drm/card[0-9]/device/driver; do
        [ -e "$f" ] && say "    $(basename "$(dirname "$(dirname "$f")")") : pilote $(basename "$(readlink -f "$f")")"
    done
    say ""
    say "-- programmes utiles --"
    for prog in foot alacritty kitty wezterm weston-terminal wmenu-run wayland-info slstatus timeout; do
        printf '%-16s : %s\n' "$prog" "$(command -v "$prog" 2> /dev/null || echo absent)"
    done
}

# --------------------------------------------------------------------- build

etape_build() {
    titre "2. Compilation (cargo build --release)"
    if ! command -v cargo > /dev/null 2>&1; then
        say "cargo est introuvable : installer Rust (https://rustup.rs)."
        noter "compilation : ÉCHEC (cargo absent)"
        return 1
    fi
    VERROU=(--locked)
    if run cargo build --release --locked "${CARGO_ARGS[@]}"; then
        noter "compilation : OK (Cargo.lock respecté)"
    else
        say ""
        say "Échec avec --locked ; nouvel essai en laissant cargo ajuster Cargo.lock."
        VERROU=()
        if run cargo build --release "${CARGO_ARGS[@]}"; then
            noter "compilation : OK, mais SANS --locked (Cargo.lock a dû être modifié)"
            say "-- différences de Cargo.lock --"
            git --no-pager diff --stat -- Cargo.lock 2>&1 | sed 's/^/    /'
        else
            noter "compilation : ÉCHEC"
            return 1
        fi
    fi
    say ""
    run ls -l "$BIN"
    run "$BIN" --version
    say "-- bibliothèques dynamiques --"
    essai ldd "$BIN"
    return 0
}

# --------------------------------------------------------------------- tests

etape_tests() {
    titre "3. Tests automatisés (cargo test --release --workspace)"
    local rt="$WORK/runtime-tests"
    mkdir -p "$rt" && chmod 700 "$rt"
    # Répertoire d'exécution dédié : les tests n'approchent pas la session en cours.
    if run env -u WAYLAND_DISPLAY -u DISPLAY XDG_RUNTIME_DIR="$rt" \
        cargo test --release --workspace "${VERROU[@]}" "${CARGO_ARGS[@]}"; then
        noter "tests : OK"
    else
        noter "tests : ÉCHEC"
        return 1
    fi
}

# ------------------------------------------------------- script de la session

# Programme lancé par rwdwm au démarrage (option -s) : il s'exécute DANS la
# session rwdwm, y relève ce qu'il voit et y ouvre deux terminaux.
ecrire_session() {
    cat > "$WORK/session.sh" << 'EOF'
#!/bin/sh
mode="$1"
log() { printf '[session] %s\n' "$*"; }
log "démarrée : WAYLAND_DISPLAY=$WAYLAND_DISPLAY XDG_RUNTIME_DIR=$XDG_RUNTIME_DIR"
if [ -S "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY" ]; then log "socket présente"; else log "SOCKET ABSENTE"; fi
if command -v wayland-info > /dev/null 2>&1; then
    log "wayland-info :"
    wayland-info 2>&1 | sed 's/^/[wayland-info] /'
else
    log "wayland-info absent (facultatif : paquet wayland-utils)"
fi
[ "$mode" = headless ] && exit 0

# Les applications ne doivent pas pouvoir se rabattre sur la session hôte.
unset DISPLAY
term=""
for candidate in foot alacritty kitty wezterm weston-terminal; do
    if command -v "$candidate" > /dev/null 2>&1; then term="$candidate"; break; fi
done
if [ -z "$term" ]; then
    log "AUCUN TERMINAL WAYLAND TROUVÉ (foot, alacritty, kitty, wezterm, weston-terminal) : l'écran restera vide sous la barre"
    exit 0
fi
etat() { if kill -0 "$1" 2> /dev/null; then echo "en cours"; else wait "$1"; echo "TERMINÉ, code $?"; fi; }
log "terminal retenu : $term"
"$term" &
t1=$!
sleep 3
log "terminal 1 (pid $t1) : $(etat $t1)"
"$term" &
t2=$!
sleep 3
log "terminal 1 (pid $t1) : $(etat $t1)"
log "terminal 2 (pid $t2) : $(etat $t2)"
wait
log "terminée"
EOF
    chmod +x "$WORK/session.sh"
}

# Lance rwdwm avec un texte de statut qui change chaque seconde.
#   $1 : backend ; $2 : durée ; $3 : mode du script de session
# rwdwm s'arrête de lui-même (--exit-after). Un garde-fou le tue s'il ne l'a
# pas fait vingt secondes plus tard (initialisation bloquée, boucle figée).
lancer_rwdwm() {
    local backend="$1" duree="$2" mode="$3" code garde
    ecrire_session
    rm -f "$WORK/pid" "$WORK/garde"
    (
        exec > /dev/null 2>&1
        sleep $((duree + 20))
        [ -s "$WORK/pid" ] || exit 0
        echo TERM > "$WORK/garde"
        kill -TERM "$(cat "$WORK/pid")" || exit 0
        sleep 5
        echo KILL > "$WORK/garde"
        kill -KILL "$(cat "$WORK/pid")"
    ) &
    garde=$!
    say "\$ (statut chaque seconde) | RUST_LOG=$JOURNAL $BIN --backend $backend --exit-after $duree -v -s \"'$WORK/session.sh' $mode\""
    say "---------------- journal de rwdwm ----------------"
    (
        trap '' PIPE
        while printf 'recette %s\n' "$(date +%H:%M:%S)" 2> /dev/null; do sleep 1; done
    ) | RUST_LOG="$JOURNAL" RUST_BACKTRACE=1 PIDFILE="$WORK/pid" \
        bash -c 'echo $$ > "$PIDFILE"; exec "$@"' rwdwm \
        "$BIN" --backend "$backend" --exit-after "$duree" -v -s "'$WORK/session.sh' $mode"
    code=${PIPESTATUS[1]}
    kill "$garde" 2> /dev/null
    wait "$garde" 2> /dev/null
    # Laisse au script de session le temps d'écrire ses dernières lignes.
    sleep 1
    say "---------------- fin du journal ------------------"
    say "-> code de sortie de rwdwm : $code"
    if [ -e "$WORK/garde" ]; then
        say "-> GARDE-FOU DÉCLENCHÉ (signal $(cat "$WORK/garde")) : rwdwm ne s'est pas arrêté seul."
    fi
    return "$code"
}

# ------------------------------------------------------------------ headless

etape_headless() {
    titre "4. Lancement à blanc, sans affichage (backend headless, 3 s)"
    local rt="$WORK/runtime-headless"
    mkdir -p "$rt" && chmod 700 "$rt"
    if XDG_RUNTIME_DIR="$rt" lancer_rwdwm headless 3 headless; then
        noter "lancement headless : OK"
    else
        noter "lancement headless : ÉCHEC"
        return 1
    fi
}

# ----------------------------------------------------------- lancement réel

questionnaire() {
    dire ""
    dire "---------------- QUESTIONNAIRE (à renvoyer avec ce fichier) ----------------"
    dire "Répondre par oui / non / autre (décrire) :"
    dire " Q1. Une barre est-elle apparue en haut (tags 1 à 9, « []= », titre) ?"
    dire " Q2. À droite de la barre, « recette HH:MM:SS » avançait-il chaque seconde ?"
    dire " Q3. Un terminal s'est-il ouvert en plein cadre, puis un second à côté"
    dire "     (le nouveau à gauche, l'ancien à droite), chacun avec une fine bordure ?"
    dire " Q4. Le curseur de la souris était-il visible et suivait-il la souris ?"
    dire " Q5. Le clavier écrivait-il dans le terminal survolé ?"
    dire " Q6. Alt+j / Alt+k déplaçaient-ils la bordure bleue d'un terminal à l'autre ?"
    dire " Q7. Alt+Maj+Entrée ouvrait-il un terminal (si foot est installé) ?"
    dire " Q8. Alt+2 vidait-il l'écran, et Alt+1 ramenait-il les terminaux ?"
    dire " Q9. rwdwm s'est-il fermé tout seul, en rendant un écran et un clavier utilisables ?"
    dire " Q10. Autre chose d'anormal (écran noir, image figée, scintillement, couleurs) ?"
    dire "----------------------------------------------------------------------------"
}

consignes() {
    dire ""
    dire "rwdwm va tourner $DUREE secondes puis s'arrêter tout seul. Pendant ce temps :"
    dire "  - regarder la barre en haut et l'horloge « recette HH:MM:SS » à droite ;"
    dire "  - deux terminaux doivent s'ouvrir, à 3 secondes d'intervalle ;"
    dire "  - bouger la souris, taper quelques lettres dans un terminal ;"
    dire "  - essayer Alt+j, Alt+k, Alt+Maj+Entrée, Alt+2 puis Alt+1."
    dire "Ne taper aucun mot de passe : c'est un essai."
}

decompte() {
    if tty_dispo || [ -t 1 ]; then
        for n in 5 4 3 2 1; do
            dire "  lancement dans $n s (Ctrl+C pour annuler)"
            sleep 1
        done
    fi
}

etape_winit() {
    titre "5. Lancement réel dans une fenêtre (backend winit, $DUREE s)"
    if [ -z "${WAYLAND_DISPLAY:-}" ] && [ -z "${DISPLAY:-}" ]; then
        say "Ni WAYLAND_DISPLAY ni DISPLAY : pas de session graphique où ouvrir une fenêtre."
        noter "lancement winit : NON TENTÉ (pas de session graphique)"
        return 1
    fi
    consignes
    dire "Une fenêtre « rwdwm » va s'ouvrir : cliquer dedans pour lui donner le clavier."
    dire "(Certains raccourcis Alt+… peuvent être interceptés par le bureau hôte.)"
    decompte
    lancer_rwdwm winit "$DUREE" graphique
    local code=$?
    noter "lancement winit : code de sortie $code"
    if [ "$code" != 0 ]; then
        dire ""
        dire "rwdwm s'est terminé en erreur (code $code) : la cause est dans le journal ci-dessus."
    fi
    questionnaire
    return $code
}

etape_udev() {
    titre "5. Lancement réel sur l'écran (backend udev, $DUREE s)"
    case "$(tty 2> /dev/null)" in
        /dev/tty[0-9]*) ;;
        *) say "ATTENTION : l'entrée standard n'est pas un terminal virtuel ($(tty 2>&1)) ; libseat risque de refuser la session." ;;
    esac
    if [ "$(id -u)" = 0 ]; then
        say "ATTENTION : lancement en root ; rwdwm est prévu pour un utilisateur ordinaire."
    fi
    consignes
    dire "L'écran va basculer en mode graphique. S'il reste noir, attendre : rwdwm"
    dire "s'arrête seul au bout de $DUREE s (au pire $((DUREE + 25)) s). Ctrl+Alt+F2 change de terminal."
    decompte
    lancer_rwdwm udev "$DUREE" graphique
    local code=$?
    noter "lancement udev : code de sortie $code"
    if [ "$code" != 0 ]; then
        dire ""
        dire "rwdwm s'est terminé en erreur (code $code) : la cause est dans le journal ci-dessus."
    fi
    questionnaire
    return $code
}

etape_reelle_auto() {
    if [ -n "${WAYLAND_DISPLAY:-}" ] || [ -n "${DISPLAY:-}" ]; then
        etape_winit
        dire ""
        dire "Pour essayer aussi le mode natif : se connecter sur un terminal virtuel"
        dire "(Ctrl+Alt+F3) et y relancer la même commande, vers un autre fichier."
    else
        case "$(tty 2> /dev/null)" in
            /dev/tty[0-9]*) etape_udev ;;
            *)
                titre "5. Lancement réel"
                say "Ni session graphique, ni terminal virtuel (connexion distante ?) : étape sautée."
                say "Relancer ce script depuis une session graphique (fenêtre) ou un terminal virtuel (natif)."
                noter "lancement réel : NON TENTÉ (ni session graphique, ni terminal virtuel)"
                ;;
        esac
    fi
}

# ------------------------------------------------------------------ résumé

resume() {
    titre "Résumé"
    for ligne in "${RESUME[@]}"; do dire "  - $ligne"; done
    dire ""
    dire "Renvoyer le fichier produit en entier, avec les réponses au questionnaire."
}

besoin_binaire() {
    if [ ! -x "$BIN" ]; then
        say "$BIN est absent : lancer d'abord l'étape « build »."
        noter "binaire absent"
        return 1
    fi
}

VERROU=(--locked)
say "Recette rwdwm — étape : $ETAPE — durée du lancement réel : $DUREE s"
case "$ETAPE" in
    infos) etape_infos ;;
    build) etape_infos; etape_build ;;
    tests) etape_tests ;;
    headless) besoin_binaire && etape_headless ;;
    winit) etape_infos; besoin_binaire && etape_winit ;;
    udev) etape_infos; besoin_binaire && etape_udev ;;
    auto)
        etape_infos
        if etape_build; then
            if [ "$AVEC_TESTS" = 1 ]; then etape_tests; else noter "tests : sautés (--sans-tests)"; fi
            if etape_headless; then
                etape_reelle_auto
            else
                say "Le lancement à blanc a échoué : le lancement réel n'est pas tenté."
                noter "lancement réel : NON TENTÉ (échec du lancement headless)"
            fi
        fi
        ;;
esac
resume
