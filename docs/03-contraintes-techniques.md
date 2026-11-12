# Phase 3 — Contraintes techniques

Les contraintes **imposées** par la demande sont : **Wayland** et **Rust**. Les autres en découlent ou sont fixées par le projet.

## 1. Contraintes imposées

| ID | Contrainte | Implications |
|----|------------|--------------|
| CT-01 | **Wayland** | Le logiciel est un *compositeur* : il parle le protocole Wayland côté serveur, pilote l'affichage (DRM/KMS) et les entrées (libinput). Aucune dépendance à un serveur X. |
| CT-02 | **Rust** | Tout le code du projet est en Rust. Les bibliothèques C ne sont admises que derrière des crates de liaison maintenues. |

## 2. Langage et outillage

| ID | Contrainte |
|----|------------|
| CT-10 | Rust **stable**, édition 2021, `rust-version = "1.88"` : c'est la seule version avec laquelle le projet a été compilé et testé. Smithay 0.7 déclare un minimum de 1.80.1 et le code du projet n'utilise aucune API postérieure à Rust 1.82 ; une version plus ancienne que 1.88 conviendrait donc probablement, mais ce n'est **pas vérifié**. |
| CT-11 | `#![forbid(unsafe_code)]` dans les deux crates du projet. |
| CT-12 | Compilation sans avertissement (`cargo check`, `cargo clippy`), formatage `rustfmt`. |
| CT-13 | Espace de travail Cargo ; `Cargo.lock` versionné (application binaire). |
| CT-14 | Aucune panique sur le chemin des requêtes clientes : pas d'`unwrap()` sur une donnée d'origine externe. |

## 3. Dépendances

| ID | Contrainte |
|----|------------|
| CT-20 | Bibliothèque de compositeur : **Smithay 0.7** (crates.io), features strictement nécessaires (pas de features par défaut). |
| CT-21 | Le noyau `rwdwm-core` n'a **aucune dépendance** (bibliothèque standard uniquement). |
| CT-22 | Dépendances directes du compositeur limitées à : `smithay`, `tracing`, `tracing-subscriber`, `xcursor`. Pour les tests seulement : `wayland-client`, `wayland-protocols`, `wayland-protocols-wlr`. |
| CT-23 | Licences compatibles MIT (MIT, Apache-2.0, BSD, ISC, Zlib). |

Bibliothèques système à l'exécution :

| Bibliothèque | Requise par | Backend |
|--------------|-------------|---------|
| libxkbcommon (+ xkeyboard-config) | dispositions clavier | tous |
| libudev, libinput, libseat, libgbm, libEGL/libGLESv2 (Mesa) | matériel, session, rendu | `udev` |
| libEGL + libwayland-client ou libX11/libxcb (chargées dynamiquement) | fenêtre hôte | `winit` |
| libpixman-1 | rendu logiciel | `headless` |

## 4. Plateforme cible

| ID | Contrainte |
|----|------------|
| CT-30 | Linux ≥ 5.x, architectures x86-64 et aarch64. |
| CT-31 | Pilote graphique **KMS** avec **GBM** et **OpenGL ES 2.0** minimum (Mesa : Intel, AMD, nouveau, virtio, etc.). |
| CT-32 | Gestion de siège par **libseat** (seatd ou systemd-logind) : pas d'exécution en root. |
| CT-33 | Variables d'environnement : `XDG_RUNTIME_DIR` obligatoire ; `WAYLAND_DISPLAY` est positionnée pour les processus enfants. |

## 5. Architecture

| ID | Contrainte |
|----|------------|
| CT-40 | Séparation stricte : **noyau** (logique dwm, pure, déterministe) / **compositeur** (Smithay) / **backends** (matériel). Le noyau ne connaît ni Wayland ni Smithay. |
| CT-41 | Mono-thread : une seule boucle d'événements `calloop` ; aucun verrou dans le code du projet. |
| CT-42 | Backends sélectionnables à la compilation (features Cargo `udev`, `winit`, `headless`) et à l'exécution (`--backend`). |
| CT-43 | Le code de rendu est **générique sur le moteur de rendu** (`GlesRenderer`, `MultiRenderer`, `PixmanRenderer`) : le même code est exercé par les tests headless et en production. |
| CT-44 | Configuration à la compilation dans un fichier unique (`config.rs`), exprimée en données typées. |

## 6. Performance et ressources

| ID | Contrainte |
|----|------------|
| CT-50 | Rendu avec suivi des dégâts : une image inchangée n'est pas recomposée. |
| CT-51 | Aucune allocation proportionnelle au nombre de pixels par image, hors redessin de la barre (uniquement quand son contenu change). |
| CT-52 | Synchronisation verticale sur chaque sortie (page-flip). |

## 7. Sécurité

| ID | Contrainte |
|----|------------|
| CT-60 | Socket Wayland dans `XDG_RUNTIME_DIR`, permissions utilisateur. |
| CT-61 | Les protocoles privilégiés (`wlr-data-control`, `wlr-layer-shell`) sont exposés à tous les clients en v0.1 — comportement identique à dwl/sway ; un filtrage par client est prévu avec le verrouillage de session. |
| CT-62 | Les commandes lancées héritent de l'environnement de `rwdwm` ; aucune interprétation par un shell sauf demande explicite (`-s`, `shcmd`). |

## 8. Qualité et tests

| ID | Contrainte |
|----|------------|
| CT-70 | Tests unitaires du noyau : chaque exigence fonctionnelle « M » relevant de la logique a au moins un test. |
| CT-71 | Tests d'intégration : compositeur réel + backend headless + clients Wayland de test dans le même processus ; aucun matériel requis. |
| CT-72 | Tests de rendu : captures headless comparées pixel à pixel à des invariants (couleurs de bordure, barre). |
| CT-73 | Les backends matériels (`udev`, `winit`) ne sont pas testables automatiquement : ils sont maintenus aussi minces que possible et vérifiés par compilation et recette manuelle. |

## 9. Contraintes de l'environnement de réalisation (bac à sable)

La réalisation a été menée dans un environnement **sans GPU, sans écran, sans siège et sans accès à crates.io**. Conséquences :

- les dépendances ont été reconstituées hors ligne depuis leurs dépôts sources (132 paquets compilés sous Linux). `Cargo.lock` enregistre ces versions avec les sommes de contrôle de l'index crates.io, mais le projet n'a **jamais été construit contre le registre réel** : si `cargo build --locked` y échouait, `cargo update` régénérerait le fichier ;
- le noyau et le compositeur en mode **headless** sont **compilés, liés et exécutés** : 86 tests unitaires du noyau, 11 tests unitaires du compositeur, 37 tests d'intégration avec de vrais clients Wayland, 3 tests du binaire ;
- les backends `udev` et `winit` sont **vérifiés par le compilateur uniquement** (`cargo check --all-features` et `cargo clippy --all-features` sans erreur ni avertissement). Ils n'ont pu être **ni liés ni exécutés** : les bibliothèques système correspondantes (libudev, libinput, libseat, libgbm, EGL) sont absentes de l'environnement. Leur première exécution sur du matériel réel reste à faire (recette décrite dans le document 05) ;
- `cargo build` avec les features par défaut n'a donc jamais produit de binaire ici ; seul le binaire `--no-default-features --features headless` a été construit et lancé.
