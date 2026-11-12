//! Client Wayland minimal pour les tests : de vrais objets `wl_surface`,
//! `xdg_toplevel`, `wl_shm`… pilotés pas à pas par le test.

#![allow(dead_code)]

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Once;

use rwdwm::backend::headless::{self, Handle, HeadlessOptions};
use rwdwm::core::Config;
use wayland_client::protocol::{
    wl_buffer, wl_callback, wl_compositor, wl_keyboard, wl_output, wl_pointer, wl_region, wl_registry, wl_seat, wl_shm,
    wl_shm_pool, wl_surface,
};
use wayland_client::{delegate_noop, Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum};
use wayland_protocols::xdg::activation::v1::client::{xdg_activation_token_v1, xdg_activation_v1};
use wayland_protocols::xdg::shell::client::{xdg_popup, xdg_positioner, xdg_surface, xdg_toplevel, xdg_wm_base};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};

// Keycodes XKB (code evdev + 8) utilisés par les tests.
pub const KEY_ALT: u32 = 64;
pub const KEY_SHIFT: u32 = 50;
pub const KEY_CTRL: u32 = 37;
pub const KEY_RETURN: u32 = 36;
pub const KEY_SPACE: u32 = 65;
pub const KEY_TAB: u32 = 23;
pub const KEY_1: u32 = 10;
pub const KEY_2: u32 = 11;
pub const KEY_3: u32 = 12;
pub const KEY_0: u32 = 19;
pub const KEY_Q: u32 = 24;
pub const KEY_E: u32 = 26;
pub const KEY_T: u32 = 28;
pub const KEY_I: u32 = 31;
pub const KEY_A: u32 = 38;
pub const KEY_D: u32 = 40;
pub const KEY_F: u32 = 41;
pub const KEY_H: u32 = 43;
pub const KEY_J: u32 = 44;
pub const KEY_K: u32 = 45;
pub const KEY_L: u32 = 46;
pub const KEY_SEMICOLON: u32 = 47;
pub const KEY_C: u32 = 54;
pub const KEY_B: u32 = 56;
pub const KEY_M: u32 = 58;
pub const KEY_COMMA: u32 = 59;
pub const KEY_DOT: u32 = 60;

pub const BTN_LEFT: u32 = 0x110;
pub const BTN_RIGHT: u32 = 0x111;
pub const BTN_MIDDLE: u32 = 0x112;

// États `xdg_toplevel` (valeurs du protocole).
pub const STATE_FULLSCREEN: u32 = 2;
pub const STATE_ACTIVATED: u32 = 4;
pub const STATE_TILED_LEFT: u32 = 5;

/// Garantit un `XDG_RUNTIME_DIR` utilisable (les sockets y sont créées).
pub fn runtime_dir() -> PathBuf {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        let usable = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).is_some_and(|p| p.is_dir());
        if !usable {
            let dir = std::env::temp_dir().join(format!("rwdwm-test-{}", std::process::id()));
            std::fs::create_dir_all(&dir).expect("création du répertoire d'exécution");
            std::env::set_var("XDG_RUNTIME_DIR", &dir);
        }
    });
    PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR").expect("XDG_RUNTIME_DIR défini"))
}

/// Configuration de test : celle de rwdwm, clavier forcé en QWERTY.
pub fn test_config() -> Config {
    let mut config = rwdwm::config::config();
    config.keyboard.layout = "us";
    config
}

/// Démarre un compositeur headless (une sortie 1280×720).
pub fn start(config: Config) -> Handle {
    start_with(HeadlessOptions::new(config))
}

/// Démarre un compositeur headless avec des options complètes.
pub fn start_with(options: HeadlessOptions) -> Handle {
    runtime_dir();
    headless::spawn(options).expect("démarrage du compositeur headless")
}

/// Configuration reçue par un toplevel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Configure {
    pub width: i32,
    pub height: i32,
    pub states: Vec<u32>,
}

impl Configure {
    pub fn has(&self, state: u32) -> bool {
        self.states.contains(&state)
    }
}

/// Fenêtre (toplevel) du client de test.
pub struct TestWindow {
    pub surface: wl_surface::WlSurface,
    pub xdg_surface: xdg_surface::XdgSurface,
    pub toplevel: xdg_toplevel::XdgToplevel,
    /// Couleur de remplissage `0xRRGGBB`.
    pub color: u32,
    /// Taille dessinée quand le compositeur laisse le choix (0×0).
    pub natural_size: (i32, i32),
    /// Dernière configuration `xdg_toplevel` reçue, pas encore acquittée.
    pending: Option<Configure>,
    /// Configurations acquittées, dans l'ordre.
    pub configures: Vec<Configure>,
    /// Taille du dernier tampon attaché.
    pub size: (i32, i32),
    /// Le compositeur a demandé la fermeture.
    pub close_requested: bool,
    /// Tampon courant (gardé en vie).
    buffer: Option<wl_buffer::WlBuffer>,
}

impl TestWindow {
    pub fn last_configure(&self) -> &Configure {
        self.configures.last().expect("au moins une configuration")
    }
}

/// Surface *layer-shell* du client de test.
pub struct TestLayer {
    pub surface: wl_surface::WlSurface,
    pub layer_surface: zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
    pub color: u32,
    pub configures: Vec<(u32, u32)>,
    buffer: Option<wl_buffer::WlBuffer>,
}

/// Popup du client de test.
pub struct TestPopup {
    pub surface: wl_surface::WlSurface,
    pub xdg_surface: xdg_surface::XdgSurface,
    pub popup: xdg_popup::XdgPopup,
    pub color: u32,
    pub size: (i32, i32),
    pub configured: bool,
    pub done: bool,
    buffer: Option<wl_buffer::WlBuffer>,
}

/// État du client de test, mis à jour par les événements.
#[derive(Default)]
pub struct ClientState {
    compositor: Option<wl_compositor::WlCompositor>,
    shm: Option<wl_shm::WlShm>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    seat: Option<wl_seat::WlSeat>,
    layer_shell: Option<zwlr_layer_shell_v1::ZwlrLayerShellV1>,
    activation: Option<xdg_activation_v1::XdgActivationV1>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    pointer: Option<wl_pointer::WlPointer>,
    /// Noms des globals annoncés.
    pub globals: Vec<String>,
    pub windows: Vec<TestWindow>,
    pub layers: Vec<TestLayer>,
    pub popups: Vec<TestPopup>,
    /// Surface ayant le focus clavier.
    pub keyboard_focus: Option<wl_surface::WlSurface>,
    /// Touches reçues : (code evdev, enfoncée).
    pub keys: Vec<(u32, bool)>,
    /// Surface survolée et position locale du pointeur.
    pub pointer_focus: Option<wl_surface::WlSurface>,
    pub pointer_pos: (f64, f64),
    /// Boutons reçus : (code evdev, enfoncé).
    pub buttons: Vec<(u32, bool)>,
    /// Jeton d'activation reçu.
    pub activation_token: Option<String>,
    /// Nombre de rappels de trame reçus.
    pub frames: u32,
}

/// Options de création d'une fenêtre.
#[derive(Clone, Debug)]
pub struct WindowOptions {
    pub app_id: String,
    pub title: String,
    pub color: u32,
    pub natural_size: (i32, i32),
    pub parent: Option<usize>,
    pub fixed_size: Option<(i32, i32)>,
    pub fullscreen: bool,
}

impl WindowOptions {
    pub fn new(app_id: &str, color: u32) -> Self {
        WindowOptions {
            app_id: app_id.into(),
            title: app_id.into(),
            color,
            natural_size: (320, 240),
            parent: None,
            fixed_size: None,
            fullscreen: false,
        }
    }
}

/// Client Wayland de test.
pub struct Client {
    pub conn: Connection,
    queue: EventQueue<ClientState>,
    qh: QueueHandle<ClientState>,
    pub state: ClientState,
}

impl Client {
    /// Se connecte au compositeur et lie les globals.
    pub fn connect(handle: &Handle) -> Client {
        let path = runtime_dir().join(handle.socket_name());
        let stream = UnixStream::connect(&path).unwrap_or_else(|e| panic!("connexion à {path:?} : {e}"));
        let conn = Connection::from_socket(stream).expect("connexion Wayland");
        let queue = conn.new_event_queue();
        let qh = queue.handle();
        conn.display().get_registry(&qh, ());
        let mut client = Client { conn, queue, qh, state: ClientState::default() };
        client.roundtrip(); // globals
        client.roundtrip(); // capacités du siège
        client
    }

    /// Aller-retour : toutes les requêtes émises ont été traitées par le
    /// compositeur, et ses événements par le client.
    pub fn roundtrip(&mut self) {
        self.queue.roundtrip(&mut self.state).expect("aller-retour Wayland");
    }

    /// Plusieurs allers-retours : laisse aboutir les échanges en chaîne
    /// (configure → ack + tampon → nouvelle configure…).
    pub fn settle(&mut self) {
        for _ in 0..4 {
            self.roundtrip();
        }
    }

    /// Crée une fenêtre et attend qu'elle soit affichée.
    pub fn create_window(&mut self, app_id: &str, color: u32) -> usize {
        self.create_window_with(WindowOptions::new(app_id, color))
    }

    /// Crée une fenêtre avec des options complètes.
    pub fn create_window_with(&mut self, options: WindowOptions) -> usize {
        let index = self.state.windows.len();
        let compositor = self.state.compositor.as_ref().expect("wl_compositor");
        let wm_base = self.state.wm_base.as_ref().expect("xdg_wm_base");
        let surface = compositor.create_surface(&self.qh, ());
        let xdg_surface = wm_base.get_xdg_surface(&surface, &self.qh, index);
        let toplevel = xdg_surface.get_toplevel(&self.qh, index);
        toplevel.set_app_id(options.app_id.clone());
        toplevel.set_title(options.title.clone());
        if let Some(parent) = options.parent {
            toplevel.set_parent(Some(&self.state.windows[parent].toplevel));
        }
        if let Some((w, h)) = options.fixed_size {
            toplevel.set_min_size(w, h);
            toplevel.set_max_size(w, h);
        }
        if options.fullscreen {
            toplevel.set_fullscreen(None);
        }
        // Commit initial, sans tampon.
        surface.commit();
        self.state.windows.push(TestWindow {
            surface,
            xdg_surface,
            toplevel,
            color: options.color,
            natural_size: options.fixed_size.unwrap_or(options.natural_size),
            pending: None,
            configures: Vec::new(),
            size: (0, 0),
            close_requested: false,
            buffer: None,
        });
        self.settle();
        index
    }

    /// Détruit une fenêtre.
    pub fn destroy_window(&mut self, index: usize) {
        let window = &self.state.windows[index];
        window.toplevel.destroy();
        window.xdg_surface.destroy();
        window.surface.destroy();
        self.settle();
    }

    /// Démappe une fenêtre (tampon nul).
    pub fn unmap_window(&mut self, index: usize) {
        let window = &self.state.windows[index];
        window.surface.attach(None, 0, 0);
        window.surface.commit();
        self.settle();
    }

    /// Refait le commit initial d'une fenêtre démappée.
    pub fn remap_window(&mut self, index: usize) {
        self.state.windows[index].surface.commit();
        self.settle();
    }

    /// Crée une surface layer-shell ancrée en haut, de hauteur `height`.
    pub fn create_top_panel(&mut self, height: u32, exclusive: bool, keyboard: bool, color: u32) -> usize {
        let index = self.state.layers.len();
        let compositor = self.state.compositor.as_ref().expect("wl_compositor");
        let layer_shell = self.state.layer_shell.as_ref().expect("zwlr_layer_shell_v1");
        let surface = compositor.create_surface(&self.qh, ());
        let layer_surface = layer_shell.get_layer_surface(
            &surface,
            None,
            zwlr_layer_shell_v1::Layer::Top,
            "panneau-de-test".into(),
            &self.qh,
            index,
        );
        use zwlr_layer_surface_v1::{Anchor, KeyboardInteractivity};
        layer_surface.set_anchor(Anchor::Top | Anchor::Left | Anchor::Right);
        layer_surface.set_size(0, height);
        if exclusive {
            layer_surface.set_exclusive_zone(height as i32);
        }
        if keyboard {
            layer_surface.set_keyboard_interactivity(KeyboardInteractivity::Exclusive);
        }
        surface.commit();
        self.state.layers.push(TestLayer { surface, layer_surface, color, configures: Vec::new(), buffer: None });
        self.settle();
        index
    }

    /// Détruit une surface layer-shell.
    pub fn destroy_layer(&mut self, index: usize) {
        let layer = &self.state.layers[index];
        layer.layer_surface.destroy();
        layer.surface.destroy();
        self.settle();
    }

    /// Crée un popup de taille `size`, dont le coin supérieur gauche est
    /// placé en `(x, y)` dans la géométrie de la fenêtre parente.
    pub fn create_popup(&mut self, parent: usize, x: i32, y: i32, size: (i32, i32), color: u32) -> usize {
        let index = self.state.popups.len();
        let compositor = self.state.compositor.as_ref().expect("wl_compositor");
        let wm_base = self.state.wm_base.as_ref().expect("xdg_wm_base");
        let positioner = wm_base.create_positioner(&self.qh, ());
        positioner.set_size(size.0, size.1);
        positioner.set_anchor_rect(x, y, 1, 1);
        positioner.set_anchor(xdg_positioner::Anchor::TopLeft);
        positioner.set_gravity(xdg_positioner::Gravity::BottomRight);
        let surface = compositor.create_surface(&self.qh, ());
        let xdg_surface = wm_base.get_xdg_surface(&surface, &self.qh, usize::MAX - index);
        let popup = xdg_surface.get_popup(Some(&self.state.windows[parent].xdg_surface), &positioner, &self.qh, index);
        positioner.destroy();
        surface.commit();
        self.state.popups.push(TestPopup {
            surface,
            xdg_surface,
            popup,
            color,
            size,
            configured: false,
            done: false,
            buffer: None,
        });
        self.settle();
        index
    }

    /// Demande l'activation d'une fenêtre (xdg-activation) : obtient un
    /// jeton, puis l'utilise.
    pub fn request_activation(&mut self, index: usize) {
        let activation = self.state.activation.as_ref().expect("xdg_activation_v1");
        let token = activation.get_activation_token(&self.qh, ());
        token.commit();
        self.roundtrip();
        let value = self.state.activation_token.take().expect("jeton d'activation");
        token.destroy();
        let activation = self.state.activation.as_ref().expect("xdg_activation_v1");
        activation.activate(value, &self.state.windows[index].surface);
        self.settle();
    }

    /// Demande un rappel de trame pour une fenêtre (avec un nouveau commit).
    pub fn request_frame(&mut self, index: usize) {
        let window = &self.state.windows[index];
        window.surface.frame(&self.qh, ());
        window.surface.commit();
        let _ = self.conn.flush();
    }

    /// Indice de la fenêtre qui a le focus clavier.
    pub fn focused_window(&self) -> Option<usize> {
        let focus = self.state.keyboard_focus.as_ref()?;
        self.state.windows.iter().position(|w| &w.surface == focus)
    }

    /// Indice de la fenêtre survolée par le pointeur.
    pub fn hovered_window(&self) -> Option<usize> {
        let focus = self.state.pointer_focus.as_ref()?;
        self.state.windows.iter().position(|w| &w.surface == focus)
    }
}

/// Crée un tampon `wl_shm` uni. Le contenu est écrit dans un fichier
/// temporaire anonyme (pas de `mmap`, donc pas de code `unsafe`).
fn solid_buffer(
    state: &ClientState,
    qh: &QueueHandle<ClientState>,
    size: (i32, i32),
    color: u32,
) -> wl_buffer::WlBuffer {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let (w, h) = (size.0.max(1), size.1.max(1));
    let name = format!("rwdwm-shm-{}-{}", std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed));
    let path = runtime_dir().join(name);
    let mut file: File = OpenOptions::new().read(true).write(true).create_new(true).open(&path).expect("fichier shm");
    let _ = std::fs::remove_file(&path);
    let pixel = (0xff00_0000 | color).to_le_bytes();
    let row: Vec<u8> = pixel.iter().copied().cycle().take(w as usize * 4).collect();
    for _ in 0..h {
        file.write_all(&row).expect("écriture shm");
    }
    file.flush().expect("écriture shm");
    let shm = state.shm.as_ref().expect("wl_shm");
    let pool = shm.create_pool(file.as_fd(), w * h * 4, qh, ());
    let buffer = pool.create_buffer(0, w, h, w * 4, wl_shm::Format::Argb8888, qh, ());
    pool.destroy();
    buffer
}

impl Dispatch<wl_registry::WlRegistry, ()> for ClientState {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        let wl_registry::Event::Global { name, interface, version } = event else { return };
        match interface.as_str() {
            "wl_compositor" => state.compositor = Some(registry.bind(name, version.min(5), qh, ())),
            "wl_shm" => state.shm = Some(registry.bind(name, 1, qh, ())),
            "xdg_wm_base" => state.wm_base = Some(registry.bind(name, version.min(5), qh, ())),
            "wl_seat" => state.seat = Some(registry.bind(name, version.min(7), qh, ())),
            "zwlr_layer_shell_v1" => state.layer_shell = Some(registry.bind(name, version.min(4), qh, ())),
            "xdg_activation_v1" => state.activation = Some(registry.bind(name, 1, qh, ())),
            _ => {}
        }
        state.globals.push(interface);
    }
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for ClientState {
    fn event(
        _: &mut Self,
        wm_base: &xdg_wm_base::XdgWmBase,
        event: xdg_wm_base::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            wm_base.pong(serial);
        }
    }
}

impl Dispatch<xdg_toplevel::XdgToplevel, usize> for ClientState {
    fn event(
        state: &mut Self,
        _: &xdg_toplevel::XdgToplevel,
        event: xdg_toplevel::Event,
        index: &usize,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let window = &mut state.windows[*index];
        match event {
            xdg_toplevel::Event::Configure { width, height, states } => {
                let states = states.chunks_exact(4).map(|c| u32::from_ne_bytes([c[0], c[1], c[2], c[3]])).collect();
                window.pending = Some(Configure { width, height, states });
            }
            xdg_toplevel::Event::Close => window.close_requested = true,
            _ => {}
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, usize> for ClientState {
    fn event(
        state: &mut Self,
        xdg_surface: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        index: &usize,
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        let xdg_surface::Event::Configure { serial } = event else { return };
        xdg_surface.ack_configure(serial);

        // Les popups utilisent des indices comptés depuis `usize::MAX`.
        if *index > usize::MAX / 2 {
            let i = usize::MAX - *index;
            let (size, color) = (state.popups[i].size, state.popups[i].color);
            let buffer = solid_buffer(state, qh, size, color);
            let popup = &mut state.popups[i];
            popup.surface.attach(Some(&buffer), 0, 0);
            popup.surface.damage_buffer(0, 0, size.0, size.1);
            popup.surface.commit();
            popup.buffer = Some(buffer);
            popup.configured = true;
            return;
        }

        // Fenêtre : obéir à la taille configurée, ou choisir la taille
        // naturelle si le compositeur laisse le choix.
        let Some(configure) = state.windows[*index].pending.take() else { return };
        let natural = state.windows[*index].natural_size;
        let size = (
            if configure.width > 0 { configure.width } else { natural.0 },
            if configure.height > 0 { configure.height } else { natural.1 },
        );
        let buffer = solid_buffer(state, qh, size, state.windows[*index].color);
        let window = &mut state.windows[*index];
        window.surface.attach(Some(&buffer), 0, 0);
        window.surface.damage_buffer(0, 0, size.0, size.1);
        window.surface.commit();
        window.buffer = Some(buffer);
        window.size = size;
        window.configures.push(configure);
    }
}

impl Dispatch<xdg_popup::XdgPopup, usize> for ClientState {
    fn event(
        state: &mut Self,
        _: &xdg_popup::XdgPopup,
        event: xdg_popup::Event,
        index: &usize,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_popup::Event::PopupDone = event {
            state.popups[*index].done = true;
        }
    }
}

impl Dispatch<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, usize> for ClientState {
    fn event(
        state: &mut Self,
        layer_surface: &zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        index: &usize,
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        let zwlr_layer_surface_v1::Event::Configure { serial, width, height } = event else { return };
        layer_surface.ack_configure(serial);
        let size = (width as i32, height as i32);
        let buffer = solid_buffer(state, qh, size, state.layers[*index].color);
        let layer = &mut state.layers[*index];
        layer.surface.attach(Some(&buffer), 0, 0);
        layer.surface.damage_buffer(0, 0, size.0, size.1);
        layer.surface.commit();
        layer.buffer = Some(buffer);
        layer.configures.push((width, height));
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for ClientState {
    fn event(
        state: &mut Self,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities { capabilities: WEnum::Value(caps) } = event {
            if caps.contains(wl_seat::Capability::Keyboard) && state.keyboard.is_none() {
                state.keyboard = Some(seat.get_keyboard(qh, ()));
            }
            if caps.contains(wl_seat::Capability::Pointer) && state.pointer.is_none() {
                state.pointer = Some(seat.get_pointer(qh, ()));
            }
        }
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for ClientState {
    fn event(
        state: &mut Self,
        _: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_keyboard::Event::Enter { surface, .. } => state.keyboard_focus = Some(surface),
            wl_keyboard::Event::Leave { surface, .. } => {
                if state.keyboard_focus.as_ref() == Some(&surface) {
                    state.keyboard_focus = None;
                }
            }
            wl_keyboard::Event::Key { key, state: WEnum::Value(key_state), .. } => {
                state.keys.push((key, key_state == wl_keyboard::KeyState::Pressed));
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_pointer::WlPointer, ()> for ClientState {
    fn event(
        state: &mut Self,
        _: &wl_pointer::WlPointer,
        event: wl_pointer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_pointer::Event::Enter { surface, surface_x, surface_y, .. } => {
                state.pointer_focus = Some(surface);
                state.pointer_pos = (surface_x, surface_y);
            }
            wl_pointer::Event::Leave { surface, .. } => {
                if state.pointer_focus.as_ref() == Some(&surface) {
                    state.pointer_focus = None;
                }
            }
            wl_pointer::Event::Motion { surface_x, surface_y, .. } => state.pointer_pos = (surface_x, surface_y),
            wl_pointer::Event::Button { button, state: WEnum::Value(button_state), .. } => {
                state.buttons.push((button, button_state == wl_pointer::ButtonState::Pressed));
            }
            _ => {}
        }
    }
}

impl Dispatch<xdg_activation_token_v1::XdgActivationTokenV1, ()> for ClientState {
    fn event(
        state: &mut Self,
        _: &xdg_activation_token_v1::XdgActivationTokenV1,
        event: xdg_activation_token_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_activation_token_v1::Event::Done { token } = event {
            state.activation_token = Some(token);
        }
    }
}

impl Dispatch<wl_callback::WlCallback, ()> for ClientState {
    fn event(
        state: &mut Self,
        _: &wl_callback::WlCallback,
        event: wl_callback::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_callback::Event::Done { .. } = event {
            state.frames += 1;
        }
    }
}

delegate_noop!(ClientState: ignore wl_compositor::WlCompositor);
delegate_noop!(ClientState: ignore wl_surface::WlSurface);
delegate_noop!(ClientState: ignore wl_shm::WlShm);
delegate_noop!(ClientState: ignore wl_shm_pool::WlShmPool);
delegate_noop!(ClientState: ignore wl_buffer::WlBuffer);
delegate_noop!(ClientState: ignore wl_region::WlRegion);
delegate_noop!(ClientState: ignore wl_output::WlOutput);
delegate_noop!(ClientState: ignore xdg_positioner::XdgPositioner);
delegate_noop!(ClientState: ignore zwlr_layer_shell_v1::ZwlrLayerShellV1);
delegate_noop!(ClientState: ignore xdg_activation_v1::XdgActivationV1);

/// Vérifie que l'identité d'un proxy reste exploitable (anti-code mort).
pub fn surface_id(surface: &wl_surface::WlSurface) -> u32 {
    surface.id().protocol_id()
}
