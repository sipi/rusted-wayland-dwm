//! Backend *headless* : sorties virtuelles, rendu logiciel (Pixman), entrées
//! injectées par un canal de commandes.
//!
//! Il sert aux tests d'intégration : le compositeur tourne dans un fil du
//! processus de test, de vrais clients Wayland s'y connectent, et le test
//! pilote le clavier et la souris puis inspecte le modèle ([`Handle::snapshot`])
//! ou l'image produite ([`Handle::screenshot`]). Le code de rendu exercé est
//! le même que celui des backends matériels.

use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::Duration;

use rwdwm_core::action::Action;
use rwdwm_core::config::Color;
use rwdwm_core::model::Wm;
use smithay::backend::allocator::Fourcc;
use smithay::backend::input::{ButtonState, KeyState};
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::pixman::PixmanRenderer;
use smithay::backend::renderer::{Bind, ExportMem, Offscreen};
use smithay::input::keyboard::Keycode;
use smithay::output::{Mode, Output, PhysicalProperties, Subpixel};
use smithay::reexports::calloop::channel::{channel, Event, Sender};
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay::reexports::calloop::EventLoop;
use smithay::reexports::pixman::Image;
use smithay::utils::{Point, Rectangle, Size, Transform};

use super::{create, run_loop, BackendData, Error, Options};
use crate::render::{color32, output_elements, send_frames, OutputElement};
use crate::state::{LoopData, State};

/// Options du backend headless.
#[derive(Clone, Debug)]
pub struct HeadlessOptions {
    /// Options communes.
    pub common: Options,
    /// Taille (en pixels) de chaque sortie virtuelle, placées côte à côte.
    pub outputs: Vec<(i32, i32)>,
    /// Dessiner le curseur dans les captures.
    pub draw_cursor: bool,
}

impl HeadlessOptions {
    /// Une sortie 1280×720, configuration donnée, socket automatique.
    pub fn new(config: rwdwm_core::Config) -> Self {
        HeadlessOptions {
            common: Options { config, socket: None, startup: None },
            outputs: vec![(1280, 720)],
            draw_cursor: false,
        }
    }
}

/// Données du backend headless.
pub struct HeadlessData {
    renderer: PixmanRenderer,
    /// Nombre de sorties virtuelles créées (sert à les nommer).
    output_serial: u32,
}

/// Commande envoyée au compositeur headless.
#[derive(Debug)]
pub enum HeadlessCommand {
    /// Exécuter une action.
    Action(Action),
    /// Injecter une touche (`keycode` XKB = code evdev + 8).
    Key {
        /// Keycode XKB.
        keycode: u32,
        /// Appui (`true`) ou relâchement.
        pressed: bool,
    },
    /// Déplacer le pointeur (coordonnées globales).
    PointerMotion {
        /// Abscisse.
        x: f64,
        /// Ordonnée.
        y: f64,
    },
    /// Injecter un bouton de souris (code evdev).
    Button {
        /// Code du bouton.
        button: u32,
        /// Appui (`true`) ou relâchement.
        pressed: bool,
    },
    /// Remplacer le texte de statut.
    Status(String),
    /// Brancher une sortie virtuelle supplémentaire.
    AddOutput {
        /// Largeur en pixels.
        width: i32,
        /// Hauteur en pixels.
        height: i32,
    },
    /// Débrancher la sortie d'indice donné.
    RemoveOutput(usize),
    /// Renvoyer une copie du modèle.
    Snapshot(mpsc::Sender<Wm>),
    /// Rendre une sortie et renvoyer l'image.
    Screenshot {
        /// Indice de la sortie.
        output: usize,
        /// Canal de réponse.
        reply: mpsc::Sender<Result<Screenshot, String>>,
    },
    /// Arrêter le compositeur.
    Quit,
}

/// Image d'une sortie.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Screenshot {
    /// Largeur en pixels.
    pub width: i32,
    /// Hauteur en pixels.
    pub height: i32,
    /// Pixels ARGB8888 (octets `B, G, R, A`), lignes de haut en bas.
    pub data: Vec<u8>,
}

impl Screenshot {
    /// Couleur `0xRRGGBB` du pixel `(x, y)`.
    pub fn pixel(&self, x: i32, y: i32) -> u32 {
        let o = ((y * self.width + x) * 4) as usize;
        u32::from_le_bytes([self.data[o], self.data[o + 1], self.data[o + 2], 0])
    }

    /// Nombre de pixels de la couleur `0xRRGGBB` dans un rectangle.
    pub fn count(&self, rect: rwdwm_core::Rect, color: u32) -> usize {
        let mut n = 0;
        for y in rect.y.max(0)..rect.bottom().min(self.height) {
            for x in rect.x.max(0)..rect.right().min(self.width) {
                n += usize::from(self.pixel(x, y) == color);
            }
        }
        n
    }

    /// Encode l'image en PNG (RVB 8 bits, non compressé).
    pub fn to_png(&self) -> Vec<u8> {
        let (w, h) = (self.width as usize, self.height as usize);
        let mut raw = Vec::with_capacity(h * (1 + 3 * w));
        for row in self.data.chunks_exact(4 * w) {
            raw.push(0); // filtre « aucun »
            for px in row.chunks_exact(4) {
                raw.extend_from_slice(&[px[2], px[1], px[0]]);
            }
        }
        png::encode_rgb(self.width as u32, self.height as u32, &raw)
    }
}

/// Encodeur PNG minimal (blocs « stockés », sans compression) : de quoi
/// produire des captures sans dépendance.
mod png {
    fn crc32(chunks: &[&[u8]]) -> u32 {
        let mut crc = !0u32;
        for byte in chunks.iter().flat_map(|c| c.iter()) {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                crc = if crc & 1 != 0 { 0xedb8_8320 ^ (crc >> 1) } else { crc >> 1 };
            }
        }
        !crc
    }

    fn adler32(data: &[u8]) -> u32 {
        let (mut a, mut b) = (1u32, 0u32);
        for chunk in data.chunks(5552) {
            for &byte in chunk {
                a += u32::from(byte);
                b += a;
            }
            a %= 65521;
            b %= 65521;
        }
        (b << 16) | a
    }

    fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        out.extend_from_slice(kind);
        out.extend_from_slice(data);
        out.extend_from_slice(&crc32(&[kind, data]).to_be_bytes());
    }

    /// `raw` : lignes préfixées de leur octet de filtre.
    pub fn encode_rgb(width: u32, height: u32, raw: &[u8]) -> Vec<u8> {
        let mut out = vec![0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'];
        let mut ihdr = Vec::with_capacity(13);
        ihdr.extend_from_slice(&width.to_be_bytes());
        ihdr.extend_from_slice(&height.to_be_bytes());
        ihdr.extend_from_slice(&[8, 2, 0, 0, 0]); // 8 bits, RVB
        chunk(&mut out, b"IHDR", &ihdr);

        let mut zlib = vec![0x78, 0x01];
        let mut blocks = raw.chunks(65535).peekable();
        if blocks.peek().is_none() {
            zlib.extend_from_slice(&[1, 0, 0, 0xff, 0xff]);
        }
        while let Some(block) = blocks.next() {
            let len = block.len() as u16;
            zlib.push(u8::from(blocks.peek().is_none()));
            zlib.extend_from_slice(&len.to_le_bytes());
            zlib.extend_from_slice(&(!len).to_le_bytes());
            zlib.extend_from_slice(block);
        }
        zlib.extend_from_slice(&adler32(raw).to_be_bytes());
        chunk(&mut out, b"IDAT", &zlib);
        chunk(&mut out, b"IEND", &[]);
        out
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn checksums_match_reference_values() {
            assert_eq!(crc32(&[b"123456789"]), 0xcbf4_3926);
            assert_eq!(crc32(&[b"1234", b"56789"]), 0xcbf4_3926);
            assert_eq!(adler32(b"Wikipedia"), 0x11e6_0398);
        }

        #[test]
        fn png_structure() {
            // 2×1 pixels : rouge, vert.
            let png = encode_rgb(2, 1, &[0, 255, 0, 0, 0, 255, 0]);
            assert_eq!(&png[..8], &[0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n']);
            assert_eq!(&png[12..16], b"IHDR");
            assert_eq!(&png[16..24], &[0, 0, 0, 2, 0, 0, 0, 1]);
            assert_eq!(&png[png.len() - 8..png.len() - 4], b"IEND");
        }
    }
}

/// Poignée sur un compositeur headless lancé par [`spawn`].
///
/// L'abandon de la poignée arrête le compositeur.
pub struct Handle {
    socket_name: String,
    sender: Sender<HeadlessCommand>,
    thread: Option<JoinHandle<()>>,
}

const REPLY_TIMEOUT: Duration = Duration::from_secs(10);

impl Handle {
    /// Nom de la socket Wayland (valeur de `WAYLAND_DISPLAY`).
    pub fn socket_name(&self) -> &str {
        &self.socket_name
    }

    /// Envoie une commande ; elles sont traitées dans l'ordre d'envoi.
    pub fn send(&self, command: HeadlessCommand) {
        let _ = self.sender.send(command);
    }

    /// Exécute une action.
    pub fn action(&self, action: Action) {
        self.send(HeadlessCommand::Action(action));
    }

    /// Appuie puis relâche une touche, les modificateurs `mods` (keycodes
    /// XKB) étant maintenus.
    pub fn key_chord(&self, mods: &[u32], keycode: u32) {
        for &m in mods {
            self.send(HeadlessCommand::Key { keycode: m, pressed: true });
        }
        self.send(HeadlessCommand::Key { keycode, pressed: true });
        self.send(HeadlessCommand::Key { keycode, pressed: false });
        for &m in mods.iter().rev() {
            self.send(HeadlessCommand::Key { keycode: m, pressed: false });
        }
    }

    /// Déplace le pointeur.
    pub fn pointer_motion(&self, x: f64, y: f64) {
        self.send(HeadlessCommand::PointerMotion { x, y });
    }

    /// Appuie sur un bouton ou le relâche.
    pub fn button(&self, button: u32, pressed: bool) {
        self.send(HeadlessCommand::Button { button, pressed });
    }

    /// Clic complet (appui puis relâchement).
    pub fn click(&self, button: u32) {
        self.button(button, true);
        self.button(button, false);
    }

    /// Copie du modèle, après traitement des commandes déjà envoyées.
    pub fn snapshot(&self) -> Wm {
        let (tx, rx) = mpsc::channel();
        self.send(HeadlessCommand::Snapshot(tx));
        rx.recv_timeout(REPLY_TIMEOUT).expect("le compositeur headless ne répond pas")
    }

    /// Image de la sortie d'indice `output`.
    pub fn screenshot(&self, output: usize) -> Screenshot {
        let (tx, rx) = mpsc::channel();
        self.send(HeadlessCommand::Screenshot { output, reply: tx });
        rx.recv_timeout(REPLY_TIMEOUT).expect("le compositeur headless ne répond pas").expect("échec du rendu")
    }

    /// Le compositeur s'est arrêté (action `Quit`, erreur…).
    pub fn is_finished(&self) -> bool {
        self.thread.as_ref().is_none_or(|t| t.is_finished())
    }

    /// Arrête le compositeur et attend la fin de son fil.
    pub fn quit(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        let _ = self.sender.send(HeadlessCommand::Quit);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Démarre un compositeur headless dans un fil dédié.
pub fn spawn(options: HeadlessOptions) -> Result<Handle, String> {
    let (ready_tx, ready_rx) = mpsc::channel();
    let thread = std::thread::Builder::new()
        .name("rwdwm-headless".into())
        .spawn(move || match setup(&options) {
            Ok((event_loop, data, sender)) => {
                let socket = data.state.socket_name.to_string_lossy().into_owned();
                let _ = ready_tx.send(Ok((socket, sender)));
                if let Err(err) = run_loop(event_loop, data, &options.common) {
                    tracing::error!(%err, "boucle headless interrompue");
                }
            }
            Err(err) => {
                let _ = ready_tx.send(Err(err.to_string()));
            }
        })
        .map_err(|e| e.to_string())?;
    let (socket_name, sender) = ready_rx.recv().map_err(|e| e.to_string())??;
    Ok(Handle { socket_name, sender, thread: Some(thread) })
}

/// Exécute un compositeur headless dans le fil courant (`--backend headless`).
pub fn run(options: HeadlessOptions) -> Result<(), Error> {
    let (event_loop, data, _sender) = setup(&options)?;
    crate::status::watch_stdin(&event_loop.handle());
    run_loop(event_loop, data, &options.common)
}

type Setup = (EventLoop<'static, LoopData>, LoopData, Sender<HeadlessCommand>);

fn setup(options: &HeadlessOptions) -> Result<Setup, Error> {
    let (event_loop, mut data) = create(&options.common)?;
    let state = &mut data.state;
    state.backend = BackendData::Headless(HeadlessData { renderer: PixmanRenderer::new()?, output_serial: 0 });
    state.draw_cursor = options.draw_cursor;

    for &(w, h) in &options.outputs {
        add_virtual_output(state, w, h);
    }

    // Commandes de pilotage.
    let (sender, receiver) = channel::<HeadlessCommand>();
    event_loop.handle().insert_source(receiver, |event, _, data: &mut LoopData| match event {
        Event::Msg(command) => handle_command(&mut data.state, command),
        Event::Closed => data.state.quit(),
    })?;

    // Pas d'écran, donc pas de VBlank : une minuterie cadence les rappels
    // de trame pour que les clients continuent de dessiner.
    event_loop.handle().insert_source(
        Timer::from_duration(Duration::from_millis(16)),
        |_, _, data: &mut LoopData| {
            let outputs: Vec<Output> = data.state.outputs.iter().map(|e| e.output.clone()).collect();
            for output in &outputs {
                send_frames(&data.state, output);
            }
            TimeoutAction::ToDuration(Duration::from_millis(16))
        },
    )?;

    Ok((event_loop, data, sender))
}

/// Crée une sortie virtuelle `HEADLESS-n` et la déclare au compositeur.
fn add_virtual_output(state: &mut State, width: i32, height: i32) {
    let BackendData::Headless(backend) = &mut state.backend else { return };
    backend.output_serial += 1;
    let output = Output::new(
        format!("HEADLESS-{}", backend.output_serial),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "rwdwm".into(),
            model: "headless".into(),
        },
    );
    let mode = Mode { size: (width, height).into(), refresh: 60_000 };
    output.change_current_state(Some(mode), Some(Transform::Normal), None, None);
    output.set_preferred(mode);
    state.add_output(output);
}

fn handle_command(state: &mut State, command: HeadlessCommand) {
    let time = state.time_msec();
    match command {
        HeadlessCommand::Action(action) => state.apply_action(&action),
        HeadlessCommand::Key { keycode, pressed } => {
            let key_state = if pressed { KeyState::Pressed } else { KeyState::Released };
            state.on_key(Keycode::new(keycode), key_state, time);
        }
        HeadlessCommand::PointerMotion { x, y } => state.on_pointer_motion(Point::from((x, y)), time),
        HeadlessCommand::Button { button, pressed } => {
            let button_state = if pressed { ButtonState::Pressed } else { ButtonState::Released };
            state.on_pointer_button(button, button_state, time);
        }
        HeadlessCommand::Status(text) => state.wm.set_status(&text),
        HeadlessCommand::AddOutput { width, height } => add_virtual_output(state, width, height),
        HeadlessCommand::RemoveOutput(index) => {
            if let Some(output) = state.outputs.get(index).map(|e| e.output.clone()) {
                state.remove_output(&output);
            }
        }
        HeadlessCommand::Snapshot(reply) => {
            let _ = reply.send(state.wm.clone());
        }
        HeadlessCommand::Screenshot { output, reply } => {
            let _ = reply.send(screenshot(state, output).map_err(|e| e.to_string()));
        }
        HeadlessCommand::Quit => state.quit(),
    }
}

/// Rend une sortie dans une image en mémoire.
fn screenshot(state: &mut State, index: usize) -> Result<Screenshot, Error> {
    let output = state.outputs.get(index).map(|e| e.output.clone()).ok_or("sortie inconnue")?;
    let size = output.current_mode().ok_or("sortie sans mode")?.size;
    let clear = color32(Color(state.wm.config().colors.root.0));

    // Le moteur de rendu est emprunté à l'état le temps du rendu.
    let BackendData::Headless(mut backend) = std::mem::replace(&mut state.backend, BackendData::None) else {
        return Err("backend headless absent".into());
    };
    let result = (|| -> Result<Screenshot, Error> {
        let renderer = &mut backend.renderer;
        let elements: Vec<OutputElement<PixmanRenderer>> = output_elements(state, renderer, &output);
        let buffer_size: Size<i32, smithay::utils::Buffer> = (size.w, size.h).into();
        let mut image: Image<'static, 'static> = renderer.create_buffer(Fourcc::Argb8888, buffer_size)?;
        let mut framebuffer = renderer.bind(&mut image)?;
        let mut tracker = OutputDamageTracker::from_output(&output);
        tracker.render_output(renderer, &mut framebuffer, 0, &elements, clear).map_err(|e| format!("rendu : {e:?}"))?;
        let mapping = renderer.copy_framebuffer(&framebuffer, Rectangle::from_size(buffer_size), Fourcc::Argb8888)?;
        let data = renderer.map_texture(&mapping)?.to_vec();
        Ok(Screenshot { width: size.w, height: size.h, data })
    })();
    state.backend = BackendData::Headless(backend);
    result
}
