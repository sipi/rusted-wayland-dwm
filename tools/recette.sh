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
#             RECETTE_FORCER=1 (compiler même si des bibliothèques semblent
#             manquer), RECETTE_TERMINAL (terminal à ouvrir dans la session,
#             au lieu du premier installé), CARGO_TARGET_DIR.
#
# Le script n'installe rien : s'il manque des bibliothèques, il affiche la
# commande à lancer et s'arrête avant la compilation.

set -u

DUREE=45
ETAPE=auto
AVEC_TESTS=1
while [ $# -gt 0 ]; do
    case "$1" in
        --duree) DUREE="${2:-}"; shift ;;
        --duree=*) DUREE="${1#*=}" ;;
        --sans-tests) AVEC_TESTS=0 ;;
        -h | --help) sed -n '2,33p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
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

# Bibliothèques nécessaires, sous la forme « nom:paquet Debian ».
# Pour compiler (noms pkg-config) : sondées par pkg-config ou liées par -l.
DEV_REQUISES="xkbcommon:libxkbcommon-dev pixman-1:libpixman-1-dev libudev:libudev-dev
    libinput:libinput-dev libseat:libseat-dev gbm:libgbm-dev"
# Pour s'exécuter : chargées dynamiquement, absentes de la sortie de ldd.
EXEC_REQUISES="libEGL.so.1:libegl1 libwayland-client.so.0:libwayland-client0 libwayland-egl.so.1:libwayland-egl1"
# Idem, seulement pour une fenêtre dans une session X11.
EXEC_X11="libX11.so.6:libx11-6 libX11-xcb.so.1:libx11-xcb1 libxcb.so.1:libxcb1 libXcursor.so.1:libxcursor1
    libXi.so.6:libxi6 libxkbcommon-x11.so.0:libxkbcommon-x11-0"
DEPS_VERIFIEES=0
MANQUE_DEV=()
MANQUE_EXEC=()
PAQUETS=()
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

# --------------------------------------------------------------- dépendances

# Liste des bibliothèques connues de l'éditeur de liens dynamique.
liste_ldconfig() {
    ldconfig -p 2> /dev/null || /sbin/ldconfig -p 2> /dev/null || /usr/sbin/ldconfig -p 2> /dev/null
}

# Remplit MANQUE_DEV, MANQUE_EXEC et PAQUETS, et affiche l'état de chaque dépendance.
verifier_dependances() {
    local entree nom paquet version connues
    DEPS_VERIFIEES=1
    MANQUE_DEV=()
    MANQUE_EXEC=()
    PAQUETS=()
    say "-- outils --"
    if command -v cc > /dev/null 2>&1; then
        say "cc               : $(cc --version 2>&1 | head -n 1)"
    else
        say "cc               : MANQUANT"
        MANQUE_DEV+=("cc")
        PAQUETS+=("build-essential")
    fi
    if command -v pkg-config > /dev/null 2>&1; then
        say "pkg-config       : $(pkg-config --version 2>&1 | head -n 1)"
    else
        say "pkg-config       : MANQUANT"
        MANQUE_DEV+=("pkg-config")
        PAQUETS+=("pkg-config")
    fi
    say ""
    say "-- bibliothèques nécessaires à la compilation (pkg-config) --"
    for entree in $DEV_REQUISES; do
        nom="${entree%%:*}"
        paquet="${entree#*:}"
        if version="$(pkg-config --modversion "$nom" 2> /dev/null)"; then
            printf '%-16s : %s\n' "$nom" "$version"
        else
            printf '%-16s : MANQUANTE (paquet Debian : %s)\n' "$nom" "$paquet"
            MANQUE_DEV+=("$nom")
            PAQUETS+=("$paquet")
        fi
    done
    say ""
    say "-- autres bibliothèques (pour information, non nécessaires à la compilation) --"
    for nom in egl glesv2 libdrm wayland-client wayland-server wayland-egl; do
        printf '%-16s : %s\n' "$nom" "$(pkg-config --modversion "$nom" 2> /dev/null || echo 'pas de fichier .pc')"
    done
    say ""
    say "-- bibliothèques chargées à l'exécution --"
    connues="$(liste_ldconfig)"
    if [ -z "$connues" ]; then
        say "(ldconfig indisponible : vérification impossible)"
        return 0
    fi
    for entree in $EXEC_REQUISES $EXEC_X11; do
        nom="${entree%%:*}"
        paquet="${entree#*:}"
        if printf '%s\n' "$connues" | grep -q "[[:space:]]$nom "; then
            printf '%-24s : présente\n' "$nom"
        else
            case " $EXEC_X11 " in
                *"$entree"*)
                    if [ -n "${WAYLAND_DISPLAY:-}" ] || [ -z "${DISPLAY:-}" ]; then
                        printf '%-24s : absente (utile seulement sous X11)\n' "$nom"
                        continue
                    fi
                    ;;
            esac
            printf '%-24s : MANQUANTE (paquet Debian : %s)\n' "$nom" "$paquet"
            MANQUE_EXEC+=("$nom")
            PAQUETS+=("$paquet")
        fi
    done
}

# Dit quoi installer, sur le terminal comme dans le journal.
conseil_installation() {
    [ ${#PAQUETS[@]} -gt 0 ] || return 0
    dire ""
    if [ ${#MANQUE_DEV[@]} -gt 0 ]; then
        dire "Il manque de quoi COMPILER : ${MANQUE_DEV[*]}"
    fi
    if [ ${#MANQUE_EXEC[@]} -gt 0 ]; then
        dire "Il manque de quoi EXÉCUTER : ${MANQUE_EXEC[*]}"
    fi
    if command -v apt-get > /dev/null 2>&1; then
        dire "À installer (le script ne le fait pas lui-même) :"
        dire ""
        dire "    sudo apt install ${PAQUETS[*]}"
        dire ""
    else
        dire "Installer les paquets de développement correspondants de la distribution"
        dire "(noms Debian, pour repère : ${PAQUETS[*]})."
    fi
    dire "Puis relancer la même commande."
}

# --------------------------------------------------------------------- infos

etape_infos() {
    titre "1. Environnement"
    say "date             : $(date '+%Y-%m-%d %H:%M:%S %z')"
    say "commit           : $(git rev-parse HEAD 2> /dev/null || echo inconnu)"
    say "fichiers modifiés: $(git status --short 2> /dev/null | tr '\n' ';')"
    say "noyau            : $(uname -srmo)"
    if [ -r /etc/os-release ]; then
        say "distribution     : $(. /etc/os-release && echo "${PRETTY_NAME:-$NAME}")"
    fi
    say "rustc            : $(rustc --version 2>&1)"
    say "cargo            : $(cargo --version 2>&1)"
    say ""
    verifier_dependances
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

# Lance cargo build --release avec les arguments donnés ; la sortie est
# aussi gardée dans $WORK/build.log.
construire() {
    local debut fin code
    say "\$ cargo build --release $* ${CARGO_ARGS[*]}"
    debut=$(date +%s)
    cargo build --release "$@" "${CARGO_ARGS[@]}" 2>&1 | tee "$WORK/build.log"
    code=${PIPESTATUS[0]}
    fin=$(date +%s)
    say "-> code de sortie $code ($((fin - debut)) s)"
    return "$code"
}

etape_build() {
    local reussi
    titre "2. Compilation (cargo build --release)"
    if ! command -v cargo > /dev/null 2>&1; then
        say "cargo est introuvable : installer Rust (https://rustup.rs)."
        noter "compilation : ÉCHEC (cargo absent)"
        return 1
    fi
    [ "$DEPS_VERIFIEES" = 1 ] || verifier_dependances
    if [ ${#MANQUE_DEV[@]} -gt 0 ] && [ -z "${RECETTE_FORCER:-}" ]; then
        say "Compilation non tentée : il manque ${MANQUE_DEV[*]}."
        say "(RECETTE_FORCER=1 pour compiler malgré tout.)"
        noter "compilation : NON TENTÉE (il manque : ${MANQUE_DEV[*]})"
        return 1
    fi
    VERROU=(--locked)
    if construire --locked; then
        noter "compilation : OK (Cargo.lock respecté)"
    elif grep -q 'lock file' "$WORK/build.log"; then
        # Seul cas où un second essai a un sens : Cargo.lock n'est pas à jour.
        say ""
        say "Cargo.lock n'est pas à jour ; nouvel essai en laissant cargo le compléter."
        VERROU=()
        construire
        reussi=$?
        say "-- différences de Cargo.lock --"
        git --no-pager diff --stat -- Cargo.lock 2>&1 | sed 's/^/    /'
        if [ "$reussi" != 0 ]; then
            noter "compilation : ÉCHEC (et Cargo.lock n'était pas à jour)"
            return 1
        fi
        noter "compilation : OK, mais SANS --locked (Cargo.lock a été modifié par cargo)"
    else
        noter "compilation : ÉCHEC"
        return 1
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
# session rwdwm, y relève ce qu'il voit et y ouvre deux terminaux. Le premier
# est lancé avec WAYLAND_DEBUG=1 : ses messages et le début de ses échanges
# avec rwdwm sont recopiés dans le journal. S'il s'arrête avant trois
# secondes, le terminal installé suivant est essayé.
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
work=$(dirname "$0")
candidats=""
for candidat in ${RECETTE_TERMINAL:-foot alacritty kitty wezterm weston-terminal}; do
    if command -v "$candidat" > /dev/null 2>&1; then candidats="$candidats $candidat"; fi
done
if [ -z "$candidats" ]; then
    log "AUCUN TERMINAL WAYLAND TROUVÉ (foot, alacritty, kitty, wezterm, weston-terminal) : l'écran restera vide sous la barre"
    exit 0
fi
log "terminaux installés :$candidats"

# Lance un terminal en arrière-plan. Sa sortie va dans $work/<nom>.log et son
# code de retour dans $work/<nom>.code.
#   $1 : terminal ; $2 : nom ; $3 : « trace » pour relever les échanges Wayland
lancer() {
    rm -f "$work/$2.code" "$work/$2.log"
    (
        if [ "$3" = trace ]; then
            WAYLAND_DEBUG=1
            export WAYLAND_DEBUG
        fi
        "$1" > "$work/$2.log" 2>&1
        echo $? > "$work/$2.code"
    ) &
}
etat() {
    if [ -s "$work/$1.code" ]; then echo "TERMINÉ, code $(cat "$work/$1.code")"; else echo "en cours"; fi
}
# Les lignes de la trace WAYLAND_DEBUG commencent par « [horodatage] ».
trace='^\[ *[0-9]*\.[0-9]*\]'
# Ce que le terminal a écrit lui-même (erreurs, avertissements).
messages() {
    if [ -f "$work/$1.log" ] && grep -v "$trace" "$work/$1.log" | grep -q .; then
        log "$1, messages du terminal :"
        grep -v "$trace" "$work/$1.log" | head -n 60 | sed "s/^/[$1] /"
    else
        log "$1 : le terminal n'a rien écrit"
    fi
}
# Début des échanges Wayland : c'est là que se lit un refus du compositeur.
debut_trace() {
    [ -f "$work/$1.log" ] || return 0
    log "$1, échanges Wayland ($(grep -c "$trace" "$work/$1.log") lignes relevées), début :"
    grep "$trace" "$work/$1.log" | head -n 220 | sed "s/^/[$1] /"
}
fin_trace() {
    [ -f "$work/$1.log" ] || return 0
    log "$1, échanges Wayland, fin :"
    grep "$trace" "$work/$1.log" | tail -n 30 | sed "s/^/[$1] /"
}

# Premier terminal : le premier candidat qui tient trois secondes.
term=""
essai=0
for candidat in $candidats; do
    essai=$((essai + 1))
    nom="essai$essai-$candidat"
    log "lancement de $candidat ($nom), échanges Wayland relevés"
    lancer "$candidat" "$nom" trace
    sleep 3
    log "$nom : $(etat "$nom")"
    messages "$nom"
    debut_trace "$nom"
    if [ ! -s "$work/$nom.code" ]; then
        term="$candidat"
        premier="$nom"
        break
    fi
    log "$candidat S'EST ARRÊTÉ AVANT TROIS SECONDES : terminal suivant"
done
if [ -z "$term" ]; then
    log "AUCUN TERMINAL N'A TENU TROIS SECONDES : l'écran restera vide sous la barre"
    exit 0
fi
log "terminal retenu : $term"

second="second-$term"
lancer "$term" "$second"
sleep 3
log "$premier : $(etat "$premier")"
log "$second : $(etat "$second")"
wait
log "$premier : $(etat "$premier")"
log "$second : $(etat "$second")"
fin_trace "$premier"
messages "$second"
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
    conseil_installation
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
