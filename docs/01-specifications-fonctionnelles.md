# Phase 1 — Spécifications fonctionnelles

> Projet : **rusted-wayland-dwm** (binaire : `rwdwm`)
> Objet : clone de [dwm](https://dwm.suckless.org/) — le *dynamic window manager* de suckless — sous forme de **compositeur Wayland écrit en Rust**.
> Référence fonctionnelle : dwm 6.5 (comportement « vanilla », sans patch) et, pour les aspects propres à Wayland, [dwl](https://codeberg.org/dwl/dwl).

## 1. Vision et périmètre

### 1.1 Vision

`rwdwm` reproduit l'expérience utilisateur de dwm : un gestionnaire de fenêtres **dynamique**, piloté au clavier, à base d'**étiquettes (tags)** et de **dispositions (layouts)** maître/pile, avec une barre d'état minimaliste intégrée. Sous Wayland, le gestionnaire de fenêtres *est* le serveur d'affichage : `rwdwm` est donc un compositeur complet (affichage, entrées, protocoles), et non un simple client comme l'est dwm sous X11.

### 1.2 Principes directeurs

| # | Principe | Conséquence |
|---|----------|-------------|
| P1 | **Fidélité à dwm** | Mêmes concepts, mêmes raccourcis par défaut, mêmes algorithmes de placement, même barre. |
| P2 | **Simplicité** | Pas d'animations, pas d'effets, pas de décorations côté client. Peu de dépendances. |
| P3 | **Configuration à la compilation** | Équivalent de `config.h` : un fichier `config.rs` typé, lu par le compilateur. |
| P4 | **Logique testable** | Toute la logique « gestionnaire de fenêtres » est isolée du compositeur et couverte par des tests. |
| P5 | **Sûreté** | Rust sûr ; aucun bloc `unsafe` dans le code du projet. |

### 1.3 Acteurs

- **Utilisateur** : interagit au clavier et à la souris.
- **Applications clientes** : clients Wayland (terminaux, navigateurs, lanceurs, barres, notifications…).
- **Script de statut** : programme externe qui écrit le texte d'état sur l'entrée standard de `rwdwm` (ex. `slstatus -s | rwdwm`).
- **Système** : noyau (DRM/KMS, evdev), gestionnaire de siège (seatd/logind), udev.

### 1.4 Glossaire

| Terme | Définition |
|-------|------------|
| **Client** | Fenêtre de premier niveau gérée (`xdg_toplevel`). |
| **Moniteur** | Sortie d'affichage ; possède son propre jeu de tags, sa disposition, sa barre. |
| **Tag** | Étiquette (1 à 9) portée par un client ; un client peut porter plusieurs tags. Un moniteur *affiche* un ensemble de tags (le *tagset*). |
| **Zone maître / pile** | En disposition tuilée, colonne gauche (maître) et colonne droite (pile). |
| **`mfact`** | Fraction de la largeur accordée à la zone maître (0,05 à 0,95). |
| **`nmaster`** | Nombre de clients placés dans la zone maître. |
| **Pile de focus** | Ordre des clients du plus récemment au moins récemment focalisé. |
| **`MODKEY`** | Modificateur principal (Alt par défaut, comme dwm). |

## 2. Exigences fonctionnelles

Priorités : **M** = indispensable (v0.1), **S** = souhaitable (v0.2+), **C** = envisageable (ultérieur).
Les exigences « M » qui relèvent de la logique, des protocoles ou du rendu sont couvertes par des tests automatisés (voir document 05). Font exception celles qui exigent du matériel ou une session réelle — EF-SES-04 et EF-SES-06 (exécution native et imbriquée, terminaux virtuels), EF-SOU-06 (pavé tactile), la partie `linux-dmabuf` de EF-PRO-04 — ainsi que les échanges de presse-papiers entre clients (EF-PRO-03), dont seuls les globals sont vérifiés : elles relèvent de la recette manuelle.

### 2.1 Tags et vues

| ID | Exigence | Prio |
|----|----------|------|
| EF-TAG-01 | Chaque moniteur dispose de 9 tags nommés « 1 » à « 9 » (configurable, 31 max). | M |
| EF-TAG-02 | `view(tag)` : afficher uniquement les clients portant ce tag. Sans effet si le tagset demandé est déjà affiché. | M |
| EF-TAG-03 | `view(0)` (`MODKEY+Tab`) : basculer vers le tagset précédent. | M |
| EF-TAG-04 | `view(~0)` (`MODKEY+0`) : afficher tous les tags. | M |
| EF-TAG-05 | `toggleview(tag)` : ajouter/retirer un tag du tagset affiché ; refusé si le tagset deviendrait vide. | M |
| EF-TAG-06 | `tag(tag)` : affecter le client sélectionné à ce(s) tag(s) exclusivement. | M |
| EF-TAG-07 | `toggletag(tag)` : ajouter/retirer un tag au client sélectionné ; refusé si le client n'aurait plus de tag. | M |
| EF-TAG-08 | Un nouveau client reçoit le tagset courant de son moniteur (sauf règle contraire). | M |

### 2.2 Dispositions (layouts)

| ID | Exigence | Prio |
|----|----------|------|
| EF-LAY-01 | **Tuile `[]=`** (défaut) : les `nmaster` premiers clients se partagent verticalement la zone maître (largeur `mfact`), les autres la pile. Si le nombre de clients ≤ `nmaster`, la zone maître occupe toute la largeur. Si `nmaster = 0`, tous les clients sont dans la pile sur toute la largeur. | M |
| EF-LAY-02 | **Monocle `[M]`** : chaque client tuilé occupe toute la zone utile ; le symbole devient `[n]` où n = nombre de clients visibles. | M |
| EF-LAY-03 | **Flottant `><>`** : aucun placement automatique ; les clients conservent leur géométrie. | M |
| EF-LAY-04 | `setlayout(l)` : sélectionner une disposition ; `setlayout()` sans argument (`MODKEY+espace`) bascule vers la précédente. | M |
| EF-LAY-05 | `setmfact(±0,05)` : modifier `mfact`, borné à [0,05 ; 0,95]. | M |
| EF-LAY-06 | `incnmaster(±1)` : modifier `nmaster`, minimum 0. | M |
| EF-LAY-07 | `zoom` : promouvoir le client sélectionné en tête de liste (maître) ; s'il est déjà maître, promouvoir le suivant. Sans effet en disposition flottante ou sur un client flottant. | M |
| EF-LAY-08 | Disposition, `mfact` et `nmaster` sont propres à chaque moniteur. | M |
| EF-LAY-09 | Dispositions additionnelles (bstack, grille, spirale…). | C |

### 2.3 Gestion des clients

| ID | Exigence | Prio |
|----|----------|------|
| EF-CLI-01 | Un nouveau client est inséré **en tête** de la liste (il devient maître) et reçoit le focus. | M |
| EF-CLI-02 | `focusstack(±1)` : focaliser le client visible suivant/précédent, de façon cyclique. | M |
| EF-CLI-03 | `killclient` : demander la fermeture du client sélectionné (`xdg_toplevel.close`). | M |
| EF-CLI-04 | `togglefloating` : basculer le client sélectionné entre tuilé et flottant (interdit en plein écran). | M |
| EF-CLI-05 | Un client **transitoire** (ayant un parent) ou de **taille fixe** (min = max) est flottant ; un transitoire hérite des tags et du moniteur de son parent. | M |
| EF-CLI-06 | Un client flottant apparaît centré sur la zone utile de son moniteur, à sa taille naturelle. | M |
| EF-CLI-07 | **Plein écran** : sur demande du client ou par `MODKEY+e` ; le client couvre tout le moniteur (barre comprise), sans bordure ; l'état antérieur est restauré à la sortie. | M |
| EF-CLI-08 | À la fermeture d'un client, le focus revient au client visible le plus récemment focalisé. | M |
| EF-CLI-09 | **Bordures** : cadre de `borderpx` pixels ; couleur normale, sélectionnée, urgente. | M |
| EF-CLI-10 | **Urgence** : une demande d'activation (`xdg-activation`) d'un client non focalisé le marque urgent (tag inversé dans la barre, bordure urgente) ; l'urgence est levée au focus. | M |
| EF-CLI-11 | Les demandes de maximisation/minimisation des clients sont ignorées. | M |
| EF-CLI-12 | Les décorations sont imposées côté serveur (`xdg-decoration`) : pas de barre de titre cliente. | M |

### 2.4 Règles

| ID | Exigence | Prio |
|----|----------|------|
| EF-RUL-01 | Des règles associent (`app_id`, titre) — par recherche de sous-chaîne — à : masque de tags, état flottant, moniteur. | M |
| EF-RUL-02 | Les règles sont évaluées une fois, à l'apparition du client ; toutes les règles correspondantes se cumulent. | M |
| EF-RUL-03 | Règles par défaut de dwm : `Gimp` → flottant ; `firefox` → tag 9. | M |

### 2.5 Barre d'état intégrée

| ID | Exigence | Prio |
|----|----------|------|
| EF-BAR-01 | Une barre par moniteur, en haut (défaut) ou en bas, de hauteur `police + 2 px`. | M |
| EF-BAR-02 | De gauche à droite : **tags**, **symbole de disposition**, **titre** du client sélectionné, **texte de statut** (aligné à droite, moniteur sélectionné uniquement). | M |
| EF-BAR-03 | Tag affiché en couleurs « sélection » s'il fait partie du tagset ; en couleurs inversées s'il contient un client urgent. | M |
| EF-BAR-04 | Un petit carré signale un tag occupé ; il est plein si le client sélectionné porte ce tag. | M |
| EF-BAR-05 | Le titre est sur fond « sélection » pour le moniteur sélectionné ; un carré signale un client flottant (plein si taille fixe). | M |
| EF-BAR-06 | `togglebar` (`MODKEY+b`) affiche/masque la barre ; la zone utile est recalculée. | M |
| EF-BAR-07 | Le texte de statut est lu ligne à ligne sur l'**entrée standard** ; par défaut `rwdwm-<version>`. | M |
| EF-BAR-08 | Les textes trop longs sont tronqués avec `...`. | M |
| EF-BAR-09 | La barre est cliquable (voir EF-SOU-04). | M |
| EF-BAR-10 | Rendu par police bitmap embarquée (Spleen 8×16, Latin étendu) ; les glyphes absents sont remplacés par `?`. | M |
| EF-BAR-11 | Polices vectorielles, emoji, CJK. | C |

### 2.6 Multi-moniteurs

| ID | Exigence | Prio |
|----|----------|------|
| EF-MON-01 | Chaque sortie connectée est un moniteur indépendant (tags, disposition, barre). | M |
| EF-MON-02 | `focusmon(±1)` : sélectionner le moniteur suivant/précédent. | M |
| EF-MON-03 | `tagmon(±1)` : envoyer le client sélectionné sur le moniteur suivant/précédent ; il y prend le tagset courant. | M |
| EF-MON-04 | Au débranchement d'un moniteur, ses clients sont transférés sur un moniteur restant. | M |
| EF-MON-05 | Branchement/débranchement à chaud. | M |
| EF-MON-06 | Règles par moniteur (nom → `mfact`, `nmaster`, disposition, échelle entière). | M |
| EF-MON-07 | Échelle fractionnaire, rotation, positionnement libre, protocole `wlr-output-management`. | S |

### 2.7 Clavier

Raccourcis par défaut (identiques à dwm 6.5, compléments issus de dwl signalés par †) :

| Raccourci | Action |
|-----------|--------|
| `MODKEY+p` | lancer le menu (`wmenu-run`) † |
| `MODKEY+Maj+Entrée` | lancer le terminal (`foot`) † |
| `MODKEY+b` | afficher/masquer la barre |
| `MODKEY+j` / `MODKEY+k` | focus client suivant / précédent |
| `MODKEY+i` / `MODKEY+d` | `nmaster` +1 / −1 |
| `MODKEY+h` / `MODKEY+l` | `mfact` −0,05 / +0,05 |
| `MODKEY+Entrée` | zoom |
| `MODKEY+Tab` | tagset précédent |
| `MODKEY+Maj+c` | fermer le client |
| `MODKEY+t` / `f` / `m` | disposition tuile / flottante / monocle |
| `MODKEY+espace` | disposition précédente |
| `MODKEY+Maj+espace` | basculer flottant |
| `MODKEY+e` | basculer plein écran † |
| `MODKEY+0` / `MODKEY+Maj+0` | voir tous les tags / client sur tous les tags |
| `MODKEY+,` / `MODKEY+.` | moniteur précédent / suivant |
| `MODKEY+Maj+,` / `MODKEY+Maj+.` | envoyer le client au moniteur précédent / suivant |
| `MODKEY+[1-9]` | `view` |
| `MODKEY+Ctrl+[1-9]` | `toggleview` |
| `MODKEY+Maj+[1-9]` | `tag` |
| `MODKEY+Ctrl+Maj+[1-9]` | `toggletag` |
| `MODKEY+Maj+q` | quitter |
| `Ctrl+Alt+F1…F12` | changer de terminal virtuel † |

| ID | Exigence | Prio |
|----|----------|------|
| EF-CLA-01 | Les raccourcis ci-dessus sont actifs par défaut et entièrement redéfinissables dans `config.rs`. | M |
| EF-CLA-02 | Un raccourci peut cibler un **symbole** (keysym, indépendant de Maj) ou une **touche physique** (keycode). Les touches de tags sont liées par keycode : elles fonctionnent sur AZERTY comme sur QWERTY. | M |
| EF-CLA-03 | Un raccourci intercepté n'est pas transmis au client (ni l'appui, ni le relâchement). | M |
| EF-CLA-04 | Disposition clavier XKB configurable (règles, modèle, disposition, variante, options) ; à défaut, variables `XKB_DEFAULT_*`. | M |
| EF-CLA-05 | Répétition des touches configurable (défaut : 25 Hz après 600 ms). | M |

### 2.8 Souris

| ID | Exigence | Prio |
|----|----------|------|
| EF-SOU-01 | **Le focus suit la souris** : entrer dans un client le focalise ; entrer sur un autre moniteur le sélectionne. | M |
| EF-SOU-02 | Cliquer dans un client le focalise. | M |
| EF-SOU-03 | `MODKEY+clic gauche` : déplacer ; `MODKEY+clic milieu` : basculer flottant ; `MODKEY+clic droit` : redimensionner. Déplacer/redimensionner un client tuilé au-delà du seuil `snap` le rend flottant ; les bords s'aimantent à la zone utile à moins de `snap` pixels ; relâcher sur un autre moniteur y transfère le client. | M |
| EF-SOU-04 | Clics sur la barre : tag — gauche `view`, droit `toggleview`, `MODKEY`+gauche `tag`, `MODKEY`+droit `toggletag` ; symbole — gauche disposition précédente, droit monocle ; titre — milieu `zoom` ; statut — milieu terminal. | M |
| EF-SOU-05 | Curseur : image fournie par le client, forme nommée (`cursor-shape`), thème XCursor, flèche intégrée de repli. | M |
| EF-SOU-06 | Pavé tactile : tap-to-click, défilement naturel (réglables). | M |
| EF-SOU-07 | Gestes, tablettes, écrans tactiles, contraintes de pointeur. | S |

### 2.9 Applications et session

| ID | Exigence | Prio |
|----|----------|------|
| EF-SES-01 | `spawn` : lancer une commande avec `WAYLAND_DISPLAY` positionné. | M |
| EF-SES-02 | Option `-s <cmd>` : commande de démarrage exécutée via `/bin/sh -c`. | M |
| EF-SES-03 | `quit` : arrêt propre du compositeur. | M |
| EF-SES-04 | Exécution **native** (TTY, DRM/KMS + libinput + libseat) et **imbriquée** (fenêtre dans une session Wayland/X11 existante). | M |
| EF-SES-05 | Mode **sans affichage** (headless) pour les tests automatisés et les captures. | M |
| EF-SES-06 | Changement de terminal virtuel ; suspension/reprise de session. | M |

### 2.10 Compatibilité applicative (protocoles)

| ID | Exigence | Prio |
|----|----------|------|
| EF-PRO-01 | Fenêtres : `xdg-shell` (toplevels, popups). | M |
| EF-PRO-02 | Surfaces de bureau : `wlr-layer-shell` (lanceurs, barres externes, fonds d'écran, notifications), zones exclusives respectées. | M |
| EF-PRO-03 | Presse-papiers : `wl_data_device`, sélection primaire, `wlr-data-control`. | M |
| EF-PRO-04 | Tampons : `wl_shm`, `linux-dmabuf` (accélération GPU), `viewporter`, `single-pixel-buffer`. | M |
| EF-PRO-05 | Sorties : `wl_output`, `xdg-output`. | M |
| EF-PRO-06 | Divers : `xdg-decoration`, `xdg-activation`, `cursor-shape`. | M |
| EF-PRO-07 | Applications X11 via XWayland. | S |
| EF-PRO-08 | Verrouillage de session (`ext-session-lock`), inactivité (`idle-notify`, `idle-inhibit`). | S |
| EF-PRO-09 | Capture d'écran (`wlr-screencopy` / `ext-image-copy-capture`), `fractional-scale`, méthodes de saisie. | S |

## 3. Ordre d'empilement

Du fond vers l'avant, pour chaque moniteur :

1. couleur de fond ;
2. couches `background` puis `bottom` (layer-shell) ;
3. clients tuilés (le plus récemment focalisé au-dessus) ;
4. **barre** ;
5. clients flottants (le plus récemment focalisé au-dessus) ;
6. popups (menus, infobulles) des clients ;
7. couche `top` ;
8. client plein écran, sur fond noir : il masque tout ce qui précède ;
9. couche `overlay` ;
10. curseur (et icône de glisser-déposer).

## 4. Exigences non fonctionnelles

| ID | Exigence |
|----|----------|
| ENF-01 | **Réactivité** : aucune latence perceptible ; pas de rendu si rien n'a changé (suivi des dégâts). |
| ENF-02 | **Robustesse** : un client défaillant ne doit pas faire tomber le compositeur ; aucune panique sur entrée cliente. |
| ENF-03 | **Sobriété** : binaire unique, démarrage instantané, mémoire résidente de quelques dizaines de Mo. |
| ENF-04 | **Testabilité** : logique métier testée unitairement ; compositeur testé de bout en bout sans matériel. |
| ENF-05 | **Maintenabilité** : code documenté, découpage clair logique / compositeur / backends. |
| ENF-06 | **Portabilité** : Linux (x86-64, aarch64) ; pilotes KMS libres. |

## 5. Hors périmètre de la v0.1

XWayland, verrouillage de session, gestion de l'inactivité, capture d'écran, configuration à l'exécution et rechargement à chaud, IPC, animations, décorations côté client, zone de notification (systray), multi-GPU. Voir la feuille de route (document 05).

## 6. Écarts assumés par rapport à dwm

| dwm (X11) | rwdwm (Wayland) | Raison |
|-----------|-----------------|--------|
| Texte de statut via le nom de la fenêtre racine (`xsetroot`) | Entrée standard | Pas de fenêtre racine sous Wayland (choix de dwl). |
| Règles sur `WM_CLASS` / instance / titre | Règles sur `app_id` / titre | Équivalent `xdg-shell`. |
| `dmenu`, `st` | `wmenu-run`, `foot` | Équivalents Wayland natifs. |
| Fenêtres flottantes placées où le client le demande | Centrées | Un client Wayland ne connaît pas sa position. |
| `resizehints` (incréments de taille) | Non applicable | Pas d'incréments dans `xdg-shell` ; seules les tailles min/max existent. |
| Polices Xft | Police bitmap embarquée | Aucune dépendance de rendu de texte (P2). |
| Pas de raccourci plein écran | `MODKEY+e` | Repris de dwl. |
