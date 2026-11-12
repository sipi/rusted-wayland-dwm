# Phase 2 — Recherche documentaire

Ce document consigne les sources étudiées, ce qui en a été retenu, et les choix qui en découlent.
Sauf mention contraire, les dépôts cités ont été **clonés et lus** (code source), et non seulement leur documentation.

## 1. dwm — la référence fonctionnelle

Sources : [dwm.suckless.org](https://dwm.suckless.org/), [tutoriel dwm](https://dwm.suckless.org/tutorial/), `dwm.c`, `config.def.h` et `dwm.1` de la version 6.5.

### 1.1 Modèle de données

- `Monitor` : géométrie écran (`mx,my,mw,mh`) et zone utile (`wx,wy,ww,wh`), `tagset[2]` + `seltags` (permet de revenir au tagset précédent), `lt[2]` + `sellt` (idem pour la disposition), `mfact`, `nmaster`, liste chaînée `clients` (ordre d'apparition, tête = maître), liste chaînée `stack` (ordre de focus), `sel`.
- `Client` : `tags` (masque de bits), géométrie, `isfloating`, `isfixed`, `isurgent`, `isfullscreen`, `oldstate`, `bw`/`oldbw`.
- Visibilité : `ISVISIBLE(c) = c->tags & c->mon->tagset[c->mon->seltags]`.

### 1.2 Algorithmes retenus (repris à l'identique)

- **`tile`** : `mw = n > nmaster ? (nmaster ? ww * mfact : 0) : ww` ; chaque client de la zone maître reçoit `h = (wh - my) / (min(n, nmaster) - i)`, chaque client de la pile `h = (wh - ty) / (n - i)` : le reste de la division est absorbé par les derniers clients.
- **`monocle`** : tous les clients tuilés à `(wx, wy, ww, wh)` ; symbole `[n]`.
- **`view`** : sans effet si `arg == tagset courant` ; sinon bascule `seltags` puis écrit le nouveau tagset (d'où le « retour arrière » de `MODKEY+Tab`).
- **`setlayout`** : bascule `sellt` si l'argument est nul ou différent de la disposition courante.
- **`setmfact`** : valeur relative si `< 1.0`, absolue (`1.0 + f`) sinon ; rejet hors de [0,05 ; 0,95].
- **`zoom`** / `pop` : détache le client, le rattache en tête, le focalise.
- **`focus(NULL)`** : premier client visible de la pile de focus.
- **`manage`** : règles → `attach` (tête) → `attachstack` → `focus` ; transitoires : tags et moniteur du parent, flottants.
- **`setfullscreen`** : sauvegarde `bw` et `isfloating`, force flottant sans bordure aux dimensions du moniteur.
- **`movemouse` / `resizemouse`** : seuil `snap` (32 px) pour l'aimantation aux bords de la zone utile et pour le passage automatique en flottant ; `sendmon` au relâchement si le client a changé de moniteur.
- **`drawbar`** : `bh = hauteur_police + 2`, `lrpad = hauteur_police`, indicateur d'occupation `boxs = h/9`, `boxw = h/6 + 2`.

### 1.3 Ce qui n'est pas transposable

- Propriétés X11 (`WM_HINTS`, `WM_NORMAL_HINTS`, EWMH), fenêtre racine, `xsetroot`.
- `resizehints` (incréments), positionnement demandé par le client.
- Le WM comme simple client : sous Wayland il faut aussi faire le travail du serveur X.

## 2. dwl — la référence « dwm sous Wayland »

Sources : [dwl sur Codeberg](https://codeberg.org/dwl/dwl), miroir archivé [djpohly/dwl](https://github.com/djpohly/dwl) (`dwl.c`, `config.def.h`, `CHANGELOG.md`).

Enseignements :

- dwl repose sur **wlroots** (C) ; il doit « implémenter les fonctionnalités de dwm **et** celles du serveur X », ce qui rend intenable la limite historique de 2000 lignes.
- **Barre** : dwl n'a pas de barre intégrée, « pour éviter les dépendances de rendu de police » ; il publie son état sur stdout pour des barres externes. → Nous conservons la barre (signature de dwm) en levant l'obstacle par une police bitmap embarquée.
- **Statut** : lu sur l'entrée standard. → Repris.
- **Non-objectifs** : décorations côté client, gestion de fenêtres initiée par le client (déplacement/redimensionnement/maximisation), animations. → Repris.
- **Couches d'empilement** : `LyrBg, LyrBottom, LyrTile, LyrFloat, LyrTop, LyrFS, LyrOverlay`. → Repris, la barre s'insérant entre tuilés et flottants comme dans dwm.
- **Compléments Wayland** : raccourci plein écran (`MODKEY+e`), changement de VT (`Ctrl+Alt+Fn`), règles de moniteur (échelle, `mfact`, `nmaster`, disposition), option `-s` (commande de démarrage), urgence via `xdg-activation`, fenêtres flottantes centrées, réglages libinput. → Repris.
- **Raccourcis et dispositions clavier** : dwl compare les keysyms du niveau 0 de la touche. → Repris, complété par des raccourcis sur keycode pour la rangée de chiffres (AZERTY).

## 3. Écosystème des compositeurs Wayland

| Brique | Langage | Utilisée par | Évaluation pour ce projet |
|--------|---------|--------------|---------------------------|
| **[Smithay](https://github.com/Smithay/smithay)** | Rust | niri, cosmic-comp (System76), anvil | Bibliothèque de « briques » pour compositeurs ; modulaire (features Cargo) ; API sûre. **Retenue.** |
| wlroots | C | sway, dwl, river-classic, Hyprland (historiquement) | Très mûre, mais les liaisons Rust (`wlroots-rs`) sont abandonnées ; imposerait beaucoup d'`unsafe`. Écartée. |
| wayland-rs seul | Rust | — | Ne fournit que le protocole : tout le reste (sièges, shm, rendu, DRM…) serait à écrire. Écarté. |
| Mutter / KWin | C / C++ | GNOME / KDE | Non réutilisables comme bibliothèques. Hors sujet. |

Le [comparatif des compositeurs tuilants de l'ArchWiki](https://wiki.archlinux.org/title/Comparison_of_tiling_compositors) confirme le paysage : dwl (C, wlroots, « dynamic »), niri (Rust, Smithay), river (Zig, wlroots).

## 4. Smithay — étude technique

Sources : dépôt `Smithay/smithay` (étiquette `v0.7.0`, 24/06/2025, et branche `master`), exemples `smallvil` et `anvil`, index crates.io.

### 4.1 Version

- Dernière version **publiée** : **0.7.0** (MSRV 1.80.1). La branche `master` porte la même version mais a divergé (édition 2024, MSRV 1.88, nouvelles dépendances obligatoires).
- → Décision : dépendre de la version publiée `0.7` (reproductible, compatible `cargo publish`), plutôt que d'une révision git.

### 4.2 Architecture de Smithay

- **Boucle d'événements** : `calloop` ; toutes les sources (socket Wayland, DRM, libinput, minuteries, canaux) y sont enregistrées et reçoivent `&mut État`.
- **Motif État / Handler / `delegate_*!`** : pour chaque protocole, un objet `XxxState` stocké dans l'état global, un trait `XxxHandler` à implémenter, une macro `delegate_xxx!` qui câble la distribution des requêtes.
- **`desktop`** : `Window`, `Space` (plan 2D de fenêtres et de sorties), `PopupManager`, `LayerSurface` / `layer_map_for_output`.
- **`input`** : `Seat`, `KeyboardHandle::input` (avec filtre permettant d'intercepter les raccourcis), `PointerHandle`.
- **Rendu** : traits `Renderer`, `ImportAll`, `ImportMem` ; implémentations `GlesRenderer` (OpenGL ES 2/3) et `PixmanRenderer` (logiciel) ; *render elements* (`WaylandSurfaceRenderElement`, `SolidColorRenderElement`, `MemoryRenderBufferRenderElement`) ; `OutputDamageTracker` (suivi des dégâts).
- **Backends** : `winit` (fenêtre imbriquée), `udev` + `drm` + `gbm` + `egl` + `libinput` + `session::libseat` (TTY).

### 4.3 Features Cargo et bibliothèques système

| Feature Smithay | Rôle | Bibliothèque système |
|-----------------|------|----------------------|
| `wayland_frontend`, `desktop` | protocoles, abstractions de bureau | libxkbcommon |
| `backend_winit`, `renderer_gl`, `backend_egl` | mode imbriqué | libEGL (chargée dynamiquement), libwayland-client/libX11 (chargées dynamiquement) |
| `backend_udev`, `backend_drm`, `backend_gbm`, `backend_libinput`, `backend_session_libseat` | mode natif | libudev, libgbm, libinput, libseat |
| `renderer_pixman` | rendu logiciel (headless) | libpixman-1 |

`wayland-rs` dispose d'une implémentation **pur Rust** du protocole : libwayland-server n'est pas requise.

### 4.4 Points d'API vérifiés dans le code (0.7.0)

- `KeysymHandle::raw_syms()` / `raw_code()` : symboles de niveau 0 et keycode → raccourcis indépendants de Maj et de la disposition.
- `Window::render_elements`, `LayerSurface::render_elements` : permettent de composer soi-même l'ordre d'empilement (nécessaire pour insérer la barre entre tuilés et flottants).
- `GbmBufferedSurface` (échange de tampons simple) *vs* `DrmCompositor` (plans matériels, scanout direct) → le premier est retenu pour la v0.1 (plus simple), le second est une optimisation ultérieure.
- `MemoryRenderBuffer` : importe un tampon ARGB en mémoire → rendu de la barre et du curseur.

## 5. Protocoles Wayland

Sources : [wayland.app](https://wayland.app/protocols/) (explorateur), dépôts `wayland-protocols` et `wlr-protocols`.

| Protocole | Utilité | Statut v0.1 |
|-----------|---------|-------------|
| `wl_compositor`, `wl_subcompositor`, `wl_shm`, `wl_seat`, `wl_output`, `wl_data_device` | socle | ✔ |
| `xdg-shell` | fenêtres et popups | ✔ |
| `xdg-decoration-unstable-v1` | imposer les décorations serveur | ✔ |
| `wlr-layer-shell-unstable-v1` | lanceurs, barres, fonds, notifications | ✔ |
| `linux-dmabuf-v1` | tampons GPU | ✔ |
| `viewporter`, `single-pixel-buffer-v1` | rognage/mise à l'échelle, aplats | ✔ |
| `xdg-output-unstable-v1` | géométrie logique des sorties | ✔ |
| `primary-selection-unstable-v1`, `wlr-data-control-unstable-v1` | sélection primaire, gestionnaires de presse-papiers | ✔ |
| `cursor-shape-v1` | curseurs nommés | ✔ |
| `xdg-activation-v1` | urgence | ✔ |
| `ext-session-lock-v1`, `ext-idle-notify-v1`, `idle-inhibit` | verrouillage, veille | feuille de route |
| `wlr-screencopy`, `wlr-output-management`, `fractional-scale-v1`, `pointer-constraints`, `text-input`/`input-method` | capture, configuration des sorties, HiDPI fractionnaire, jeux, IME | feuille de route |

## 6. Rendu du texte de la barre

| Option | Dépendances | Qualité | Décision |
|--------|-------------|---------|----------|
| Pango/Cairo, FreeType/HarfBuzz | lourdes (C) | excellente | Écartée (contraire à P2). |
| `fontdue`, `cosmic-text` (pur Rust) | moyennes + fichier de police | bonne | Envisageable plus tard. |
| **Police bitmap embarquée** | aucune | nette, monochasse | **Retenue.** |

Police retenue : **[Spleen](https://github.com/fcambus/spleen) 8×16** (Frédéric Cambus, licence BSD-2-Clause), 1004 glyphes (ASCII, Latin-1, Latin étendu, tracés de boîtes, Braille…). Elle est convertie en table binaire compacte embarquée dans l'exécutable.

## 7. Synthèse des décisions issues de la recherche

1. **Smithay 0.7 (crates.io)** comme bibliothèque de compositeur.
2. **Sémantique dwm reprise à l'identique** (algorithmes de `dwm.c`) dans un noyau pur, sans dépendance.
3. **Compléments Wayland repris de dwl** (statut sur stdin, plein écran, VT, règles de moniteurs, non-objectifs).
4. **Barre intégrée** conservée grâce à une police bitmap embarquée.
5. **Composition « maison » de l'ordre d'empilement** à partir des *render elements* de Smithay.
6. **Trois backends** : `udev` (natif), `winit` (imbriqué), `headless` (tests, rendu logiciel Pixman).
