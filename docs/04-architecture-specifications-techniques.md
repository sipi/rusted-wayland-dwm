# Phase 4 — Architecture et spécifications techniques détaillées

## 1. Vue d'ensemble

```
                    ┌──────────────────────────────────────────────────────────┐
                    │                    crate `rwdwm` (bin + lib)             │
 clients Wayland ◄──┤  handlers/  ── protocoles (xdg-shell, layer-shell, …)    │
                    │  input.rs   ── clavier, souris, raccourcis               │
   stdin (statut) ──┤  status.rs                                               │
                    │  state.rs   ── État global + synchronisation ◄──────┐    │
                    │  render.rs  ── composition des éléments de rendu    │    │
                    │  config.rs  ── configuration (équivalent config.h)  │    │
                    │  backend/   ── udev │ winit │ headless              │    │
                    └───────┬──────────────────────────────────────────── │ ───┘
                            │ actions, événements        modèle (lecture) │
                    ┌───────▼─────────────────────────────────────────────┴────┐
                    │                 crate `rwdwm-core` (lib, pure)           │
                    │  model.rs (Wm, Monitor, Client) · layout.rs · bar.rs     │
                    │  action.rs · config.rs (types) · font.rs · geometry.rs   │
                    └──────────────────────────────────────────────────────────┘
```

Deux crates dans un espace de travail Cargo :

| Crate | Nature | Dépendances | Rôle |
|-------|--------|-------------|------|
| `rwdwm-core` | bibliothèque | aucune | Toute la sémantique dwm : tags, listes, focus, dispositions, règles, modèle et dessin de la barre. Déterministe, sans E/S. |
| `rwdwm` | bibliothèque + binaire | smithay, tracing, xcursor | Compositeur Wayland : protocoles, entrées, rendu, backends. Traduit les événements en appels au noyau et applique le modèle. |

### Principe de fonctionnement : « modèle → synchronisation »

1. Un événement (touche, clic, nouveau client, sortie branchée…) est traduit en appel au noyau (`Wm::apply(action)`, `Wm::manage(...)`, …).
2. Le noyau met à jour son modèle (géométries comprises) et renvoie d'éventuels **effets** (`Spawn`, `Close`, `Quit`, …) qu'il ne peut exécuter lui-même.
3. Le compositeur appelle `State::sync()` : il **relit le modèle** et l'applique (configure des `xdg_toplevel`, positions, activation, focus clavier), puis demande un rendu.

Ce flux unidirectionnel évite toute duplication d'état : le noyau est la **seule source de vérité** pour « qui est où ».

## 2. Crate `rwdwm-core`

### 2.1 Types de base (`geometry.rs`, `config.rs`, `action.rs`)

```rust
pub struct Rect { pub x: i32, pub y: i32, pub w: i32, pub h: i32 }

pub type ClientId = u32;
pub type MonitorId = u32;
pub type TagMask = u32;

pub enum LayoutKind { Tile, Floating, Monocle }
pub struct Layout { pub symbol: &'static str, pub kind: LayoutKind }

pub enum Action {
    Spawn(Vec<String>), ToggleBar, FocusStack(i32), IncNMaster(i32), SetMFact(f32),
    Zoom, View(TagMask), ToggleView(TagMask), Tag(TagMask), ToggleTag(TagMask),
    KillClient, SetLayout(Option<usize>), ToggleFloating, ToggleFullscreen,
    FocusMon(i32), TagMon(i32), MoveMouse, ResizeMouse, ChVt(i32), Quit,
}

pub enum Effect { Spawn(Vec<String>), Close(ClientId), Quit, ChVt(i32), BeginMove(ClientId), BeginResize(ClientId) }

pub struct Config {            // équivalent de config.h
    pub border_px: i32, pub snap: i32, pub show_bar: bool, pub top_bar: bool,
    pub colors: Colors, pub tags: Vec<String>, pub rules: Vec<Rule>,
    pub mfact: f32, pub nmaster: i32, pub lock_fullscreen: bool,
    pub layouts: Vec<Layout>, pub monitor_rules: Vec<MonitorRule>,
    pub keys: Vec<Key>, pub buttons: Vec<Button>, /* clavier, pavé tactile… */
}
```

### 2.2 Modèle (`model.rs`)

```rust
pub struct Client {
    pub id: ClientId, pub mon: MonitorId,
    pub title: String, pub app_id: String,
    pub tags: TagMask,
    pub geom: Rect,              // géométrie EXTÉRIEURE (bordure comprise)
    pub bw: i32,                 // largeur de bordure courante (0 en plein écran)
    pub is_floating: bool, pub is_fixed: bool, pub is_urgent: bool, pub is_fullscreen: bool,
    pub needs_initial_geometry: bool,  // flottant dont la taille naturelle n'est pas encore connue
    /* sauvegardes pour la sortie du plein écran : old_geom, old_bw, old_floating */
}

pub struct Monitor {
    pub id: MonitorId, pub name: String,
    pub screen: Rect,            // mx,my,mw,mh
    pub usable: Rect,            // écran moins les zones exclusives layer-shell
    pub window_area: Rect,       // wx,wy,ww,wh : usable moins la barre
    pub show_bar: bool, pub top_bar: bool,
    pub tagset: [TagMask; 2], pub seltags: usize,
    pub lt: [usize; 2], pub sellt: usize,
    pub mfact: f32, pub nmaster: i32,
    pub clients: Vec<ClientId>,  // ordre d'attache, tête = maître
    pub stack: Vec<ClientId>,    // ordre de focus, tête = plus récent
    pub sel: Option<ClientId>,
}

pub struct Wm { /* config, monitors, selmon, clients, status, compteurs d'identifiants */ }
```

Invariants (vérifiés par `Wm::check_invariants()` dans tous les tests) :

- tout client appartient à exactement un moniteur, et figure exactement une fois dans `clients` et dans `stack` de ce moniteur ;
- `tags != 0` pour tout client ; `tagset[seltags] != 0` pour tout moniteur ;
- `sel` est `None` ou un client **visible** du moniteur ;
- `0.05 ≤ mfact ≤ 0.95`, `nmaster ≥ 0`.

### 2.3 API du noyau

| Famille | Fonctions |
|---------|-----------|
| Moniteurs | `add_monitor(name, screen) -> MonitorId`, `remove_monitor(id)`, `set_monitor_geometry(id, screen, usable)`, `monitor_at(x, y)` |
| Clients | `manage(NewClient) -> ClientId`, `unmanage(id)`, `set_title`, `set_app_id`, `set_urgent`, `set_fullscreen`, `set_natural_size(id, w, h)`, `set_size_limits` |
| Actions | `apply(&Action) -> Vec<Effect>` |
| Souris | `focus_client(id)`, `select_monitor(id)`, `mouse_move(id, x, y)`, `mouse_resize(id, w, h)`, `mouse_release(id)` |
| Lecture | `is_visible(id)`, `selected()`, `render_order(mon) -> Vec<ClientId>`, `bar_view(mon) -> BarView`, `bar_rect(mon) -> Option<Rect>` |

`manage` reproduit `manage()` de dwm : application des règles, héritage du parent, `attach` en tête, `attachstack`, focus, `arrange`.
`arrange(mon)` recalcule `geom` de chaque client tuilé visible selon la disposition courante.

### 2.4 Dispositions (`layout.rs`)

Fonctions pures, testées indépendamment du modèle :

```rust
pub fn tile(area: Rect, n: usize, nmaster: i32, mfact: f32) -> Vec<Rect>;
pub fn monocle(area: Rect, n: usize) -> Vec<Rect>;
```

Elles renvoient les géométries **extérieures** ; la surface cliente est placée en `(x + bw, y + bw)` avec la taille `(w − 2·bw, h − 2·bw)`. La somme des hauteurs d'une colonne est exactement `area.h` (le reste des divisions entières est réparti comme dans dwm).

### 2.5 Barre (`bar.rs`, `font.rs`)

- `BarView` : description déclarative du contenu (tags avec états, symbole, titre, statut). Comparable (`PartialEq`) → la barre n'est redessinée que si sa vue change.
- `bar::segments(view, width) -> Vec<Segment>` : calcule les zones (tags, symbole, titre, statut) ; partagé par le dessin et par le test de clic → pas de divergence possible.
- `bar::draw(view, width, scale, colors) -> BarPixmap` : rend la barre en ARGB8888 (pré-multiplié, opaque).
- `bar::click_at(view, width, x) -> BarClick`.
- `font.rs` : table binaire Spleen 8×16 embarquée (`include_bytes!`), recherche dichotomique par point de code, repli sur `?`.

Métriques, identiques à dwm : `bar_height = 16 + 2`, `lrpad = 16`, `TEXTW(s) = 8·len(s) + lrpad`, `boxs = 16/9 = 1`, `boxw = 16/6 + 2 = 4`.

## 3. Crate `rwdwm`

### 3.1 Modules

| Module | Contenu |
|--------|---------|
| `main.rs` | Ligne de commande, journalisation, choix du backend. |
| `lib.rs` | Exporte l'API (nécessaire aux tests d'intégration). |
| `config.rs` | **La configuration** : `pub fn config() -> Config`. |
| `state.rs` | `State` : état Smithay + `Wm` + table `ClientId → Window` ; `sync()`, `run_effects()`. |
| `handlers/` | Un fichier par protocole : `compositor`, `xdg_shell`, `layer_shell`, `seat`, `selection`, `output`, `misc` (décoration, activation, viewporter, cursor-shape, dmabuf…). |
| `input.rs` | Traduction des événements d'entrée, recherche de raccourcis, modes déplacement/redimensionnement. |
| `render.rs` | `output_elements()` : liste ordonnée des éléments de rendu d'une sortie ; générique sur le moteur. |
| `cursor.rs` | Thème XCursor, flèche de repli. |
| `status.rs` | Source `calloop` lisant stdin. |
| `spawn.rs` | Lancement de processus détachés. |
| `backend/{udev,winit,headless}.rs` | Backends. |

### 3.2 État global

```rust
pub struct State {
    pub wm: Wm,                                   // noyau — source de vérité
    pub windows: HashMap<ClientId, Window>,       // fenêtres gérées
    pub unmapped: Vec<Window>,                    // toplevels avant leur premier commit
    pub space: Space<Window>,                     // sorties + suivi entrée/sortie des surfaces
    pub outputs: Vec<OutputEntry>,                // Output ↔ MonitorId, barre mise en cache
    pub seat: Seat<State>, pub pointer_location: Point<f64, Logical>,
    pub cursor_mode: CursorMode,                  // Normal | Move{..} | Resize{..}
    pub backend: BackendData,                     // données propres au backend actif
    /* états Smithay : compositor, xdg_shell, layer_shell, shm, dmabuf, seat, data_device, … */
}
```

### 3.3 Cycle de vie d'un client (xdg-shell)

```
get_toplevel ──► new_toplevel() : Window créée, rangée dans `unmapped`
     │
 1er commit (sans tampon ; app_id/titre/parent connus)
     │        wm.manage(NewClient{app_id, title, parent, min/max})
     │        ─ tuilé   : géométrie calculée → configure(taille tuilée, états Tiled*)
     │        ─ flottant: configure(taille libre) ; géométrie finalisée plus tard
     ▼
 commit avec tampon ─► si flottant en attente : wm.set_natural_size() (centrage)
     │                 sync() : placement, activation, focus
     ▼
 toplevel_destroyed ─► wm.unmanage(id) ; sync()
```

Gérer le client **dès le commit initial** permet d'envoyer la bonne taille dans la toute première configuration : un client tuilé dessine directement à sa taille définitive (pas de redimensionnement visible à l'ouverture).

### 3.4 Synchronisation (`State::sync`)

Pour chaque client géré :

- visible → `with_pending_state` (taille intérieure, états `Fullscreen` / `Tiled*` / `Activated`), `send_pending_configure()` (n'émet que si l'état change), `space.map_element(...)` à la position intérieure ;
- invisible → `space.unmap_elem(...)` (la surface reçoit `wl_surface.leave`).

Puis : focus clavier = surface *layer-shell* exclusive s'il y en a une, sinon client sélectionné ; invalidation des barres ; demande de rendu.

### 3.5 Entrées

**Clavier** — `KeyboardHandle::input` avec filtre :

1. masque de modificateurs courant (Ctrl, Alt, Maj, Logo) ;
2. keycode et keysyms de niveau 0 de la touche ;
3. recherche dans `config.keys` : `mods` égaux **et** (`Trigger::Code == keycode` **ou** `Trigger::Sym ∈ keysyms`) ;
4. correspondance → `FilterResult::Intercept(action)` et mémorisation du keycode pour avaler aussi le relâchement ; sinon → transmission au client.

**Pointeur** :

- mouvement → position bornée à l'union des sorties ; en mode `Move`/`Resize`, appel direct au noyau ; sinon recherche de la surface sous le curseur (dans l'ordre d'empilement, §3 du document 01) et, si le client survolé **change**, `wm.focus_client()` (focus suit la souris) ;
- bouton → cible du clic (`TagBar(i)`, `LtSymbol`, `WinTitle`, `StatusText`, `ClientWin`, `RootWin`) puis recherche dans `config.buttons` ; à défaut, le clic est transmis au client.

### 3.6 Rendu

`render::output_elements<R>(state, renderer, output) -> Vec<OutputElement<R>>` construit la liste **de l'avant vers l'arrière** :

```
curseur · overlay · plein écran · top · flottants(+bordures) · barre · tuilés(+bordures) · bottom · background
```

- `R: Renderer + ImportAll + ImportMem` : même code pour `GlesRenderer` et `PixmanRenderer`.
- Fenêtres et couches : `render_elements()` de Smithay (popups compris).
- Bordures : 4 `SolidColorRenderElement` par client, tracées autour de la géométrie **allouée**.
- Barre : `MemoryRenderBuffer` par sortie, régénéré seulement si `BarView` change.
- `OutputDamageTracker::render_output` calcule les dégâts ; après rendu, les rappels de trame (`wl_surface.frame`) sont envoyés aux surfaces visibles.

### 3.7 Backends

| | `udev` (natif) | `winit` (imbriqué) | `headless` (tests) |
|---|---|---|---|
| Session | libseat | — | — |
| Sorties | connecteurs DRM (branchement à chaud) | une fenêtre redimensionnable | une sortie virtuelle (taille paramétrable) |
| Rendu | GLES sur GBM, `GbmBufferedSurface`, page-flip | GLES dans la fenêtre hôte | Pixman, en mémoire, à la demande |
| Entrées | libinput | événements de la fenêtre | injectées par canal (`HeadlessCommand`) |
| Curseur | dessiné par le compositeur | curseur de l'hôte | non dessiné |
| Cadencement | VBlank ; minuterie de repli s'il n'y a pas de dégâts | redessin continu demandé à l'hôte | minuterie 60 Hz (rappels de trame) |

Interface commune minimale (`BackendData`) : importer un dmabuf, changer de VT, demander un rendu. Les fonctions d'initialisation (`backend::udev::run`, `backend::winit::run`, `backend::headless::run`) créent la boucle `calloop`, l'état, la sortie, puis exécutent la boucle.

**Backend headless et tests** : `headless::spawn(opts) -> Handle` démarre le compositeur dans un fil dédié et renvoie un canal de commandes :

```rust
pub enum HeadlessCommand {
    Action(Action),                               // exécuter une action
    Key { keycode: u32, pressed: bool },          // injecter une touche
    PointerMotion { x: f64, y: f64 }, Button { button: u32, pressed: bool },
    Snapshot(Sender<Wm>),                         // copie du modèle
    Screenshot(Sender<Screenshot>),               // rendu de la sortie (pixels ARGB)
    Quit,
}
```

### 3.8 Configuration (`config.rs`)

Fichier Rust unique, structuré comme `config.def.h` : apparence, tags, règles, dispositions, commandes, touches, boutons. Modifier, recompiler, relancer — comme dwm. Les types (`Key`, `Button`, `Rule`, `Action`) vivent dans le noyau : une erreur de configuration est une erreur de compilation.

### 3.9 Ligne de commande

```
rwdwm [--backend auto|udev|winit|headless] [-s <commande>] [-v] [--version] [--help]
```

`auto` : `winit` si `WAYLAND_DISPLAY` ou `DISPLAY` est défini, sinon `udev`. Journalisation via `tracing` (niveau par `-v` ou `RUST_LOG`).

## 4. Décisions d'architecture

| ID | Décision | Alternatives écartées | Justification |
|----|----------|-----------------------|---------------|
| DA-01 | Smithay 0.7 publié | wlroots + FFI ; Smithay `master` | Rust sûr ; version reproductible. |
| DA-02 | Noyau pur séparé | Logique mêlée aux handlers (comme dwl) | Testabilité totale de la sémantique dwm ; déterminisme. |
| DA-03 | Flux « modèle → synchronisation » | Mutations croisées noyau/compositeur | Une seule source de vérité ; `sync()` idempotent. |
| DA-04 | Composition manuelle des éléments de rendu | `space::render_output` | Ordre d'empilement de dwm (barre entre tuilés et flottants). |
| DA-05 | Configuration compilée (`config.rs`) | Fichier TOML/KDL à l'exécution | Fidélité à dwm, typage, zéro dépendance. La structure `Config` étant une donnée, un chargeur à l'exécution pourra s'y greffer. |
| DA-06 | Barre intégrée, police bitmap | Barre externe (dwl) ; FreeType | Identité de dwm sans dépendance de rendu de texte. |
| DA-07 | `GbmBufferedSurface` | `DrmCompositor` | Simplicité pour la v0.1 ; scanout direct en optimisation ultérieure. |
| DA-08 | Raccourcis de tags sur keycode | Keysyms `1`…`9` | Fonctionne sur AZERTY sans configuration. |
| DA-09 | Backend headless Pixman + tests en processus | Tests manuels seulement | Vérification de bout en bout sans matériel ; exerce le même code de rendu. |
| DA-10 | Statut sur stdin | Protocole/IPC dédié | Simplicité, compatibilité avec `slstatus -s` et les scripts dwl. |
| DA-11 | Gestion du client au commit initial | Gestion au premier tampon (dwl) | Première configuration à la bonne taille. |

## 5. Gestion des erreurs et journalisation

- Initialisation : erreurs fatales remontées avec contexte (`Box<dyn Error>`), message clair, code de sortie non nul.
- Exécution : une erreur liée à un client ou à une trame est journalisée (`warn!`) et n'interrompt pas la boucle.
- Aucune panique volontaire hors invariants internes du noyau (`debug_assert!`).

## 6. Stratégie de test

| Niveau | Cible | Moyen |
|--------|-------|-------|
| Unitaire | `layout`, `model`, `bar`, `font` | `cargo test -p rwdwm-core` — scénarios dwm, invariants vérifiés après chaque opération. |
| Intégration | protocoles, focus, raccourcis, configuration des tailles | `cargo test -p rwdwm --features headless` — clients `wayland-client` réels. |
| Rendu | bordures, barre, empilement | Captures headless, assertions sur les pixels. |
| Recette | `udev`, `winit` | Liste de contrôle manuelle (document 05). |
