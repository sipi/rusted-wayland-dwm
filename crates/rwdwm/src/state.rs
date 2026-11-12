//! L'état global du compositeur.
//!
//! [`State`] réunit le noyau ([`Wm`], seule source de vérité sur les clients),
//! les états des protocoles Smithay et les données du backend. Après chaque
//! événement qui modifie le modèle, [`State::sync`] relit celui-ci et
//! l'applique aux fenêtres : configuration, placement, focus.

use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::sync::Arc;
use std::time::Instant;

use rwdwm_core::action::{Action, Effect};
use rwdwm_core::bar::BarView;
use rwdwm_core::config::LayoutKind;
use rwdwm_core::model::{ClientId, MonitorId, NewClient, Wm};
use rwdwm_core::{Config, Rect};
use smithay::backend::renderer::element::memory::MemoryRenderBuffer;
use smithay::backend::renderer::element::solid::SolidColorBuffer;
use smithay::desktop::{layer_map_for_output, PopupManager, Space, Window};
use smithay::input::keyboard::XkbConfig;
use smithay::input::pointer::{CursorImageStatus, MotionEvent};
use smithay::input::{Seat, SeatState};
use smithay::output::{Output, Scale};
use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::calloop::{Interest, LoopHandle, LoopSignal, Mode, PostAction, RegistrationToken};
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::reexports::wayland_server::backend::{ClientData, ClientId as WlClientId, DisconnectReason, GlobalId};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{Display, DisplayHandle};
use smithay::utils::{Logical, Point, Rectangle, SERIAL_COUNTER};
use smithay::wayland::compositor::{with_states, CompositorClientState, CompositorState};
use smithay::wayland::cursor_shape::CursorShapeManagerState;
use smithay::wayland::dmabuf::{DmabufGlobal, DmabufState};
use smithay::wayland::output::OutputManagerState;
use smithay::wayland::selection::data_device::DataDeviceState;
use smithay::wayland::selection::primary_selection::PrimarySelectionState;
use smithay::wayland::selection::wlr_data_control::DataControlState;
use smithay::wayland::shell::wlr_layer::{KeyboardInteractivity, Layer, WlrLayerShellState};
use smithay::wayland::shell::xdg::decoration::XdgDecorationState;
use smithay::wayland::shell::xdg::{SurfaceCachedState, XdgShellState, XdgToplevelSurfaceData};
use smithay::wayland::shm::ShmState;
use smithay::wayland::single_pixel_buffer::SinglePixelBufferState;
use smithay::wayland::socket::ListeningSocketSource;
use smithay::wayland::viewporter::ViewporterState;
use smithay::wayland::xdg_activation::XdgActivationState;
use tracing::{debug, info, warn};

use crate::backend::BackendData;
use crate::cursor::CursorTheme;

/// Données de la boucle d'événements : l'état et le `Display` Wayland.
///
/// Le `Display` vit à côté de l'état (et non dedans) pour que la distribution
/// des requêtes — `display.dispatch_clients(&mut state)` — n'exige aucun
/// emprunt simultané ni code `unsafe`.
pub struct LoopData {
    /// L'état du compositeur.
    pub state: State,
    /// Le `Display` Wayland.
    pub display: Display<State>,
}

/// Données attachées à chaque client Wayland.
#[derive(Debug, Default)]
pub struct ClientState {
    /// État requis par `wl_compositor`.
    pub compositor_state: CompositorClientState,
}

impl ClientData for ClientState {
    fn initialized(&self, _client_id: WlClientId) {}
    fn disconnected(&self, _client_id: WlClientId, _reason: DisconnectReason) {}
}

/// Fenêtre gérée : la fenêtre Smithay et ses ressources de rendu.
#[derive(Debug)]
pub struct ManagedWindow {
    /// La fenêtre (toplevel xdg-shell).
    pub window: Window,
    /// Les quatre rectangles de bordure : haut, bas, gauche, droite.
    pub borders: [SolidColorBuffer; 4],
    /// Le client a déjà fourni un tampon (il est affichable).
    pub mapped: bool,
}

/// Toplevel créé mais pas encore géré (avant son commit initial).
#[derive(Debug)]
pub struct PendingWindow {
    /// La fenêtre.
    pub window: Window,
    /// Le client a demandé le plein écran avant d'être géré.
    pub fullscreen: bool,
}

/// Barre rendue, conservée tant que son contenu ne change pas.
#[derive(Debug)]
pub struct BarCache {
    /// Contenu représenté.
    pub view: BarView,
    /// Largeur logique.
    pub width: i32,
    /// Échelle.
    pub scale: i32,
    /// Image prête à être composée.
    pub buffer: MemoryRenderBuffer,
}

/// Association sortie Smithay ↔ moniteur du noyau.
#[derive(Debug)]
pub struct OutputEntry {
    /// La sortie.
    pub output: Output,
    /// Le global `wl_output` annoncé aux clients.
    pub global: GlobalId,
    /// Le moniteur correspondant.
    pub mon: MonitorId,
    /// La barre en cache.
    pub bar: Option<BarCache>,
    /// Fond noir placé derrière un client en plein écran.
    pub fullscreen_backdrop: SolidColorBuffer,
}

/// Mode du pointeur.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CursorMode {
    /// Les événements vont aux clients.
    Normal,
    /// Déplacement d'un client ; `offset` = pointeur − coin du client.
    Move {
        /// Client déplacé.
        client: ClientId,
        /// Décalage entre le pointeur et le coin supérieur gauche.
        offset: (i32, i32),
    },
    /// Redimensionnement d'un client.
    Resize {
        /// Client redimensionné.
        client: ClientId,
        /// Position du pointeur au début.
        start: (i32, i32),
        /// Taille extérieure au début.
        size: (i32, i32),
    },
}

/// Icône de glisser-déposer.
#[derive(Debug)]
pub struct DndIcon {
    /// Surface de l'icône.
    pub surface: WlSurface,
    /// Décalage par rapport au pointeur.
    pub offset: Point<i32, Logical>,
}

/// Répétition en cours d'un raccourci clavier.
#[derive(Debug)]
pub struct KeyRepeat {
    /// Keycode de la touche maintenue.
    pub keycode: u32,
    /// Minuterie.
    pub token: RegistrationToken,
}

/// L'état complet du compositeur.
pub struct State {
    /// Instant de démarrage (base des horodatages).
    pub start_time: Instant,
    /// Accès au `Display`.
    pub display_handle: DisplayHandle,
    /// Accès à la boucle d'événements.
    pub loop_handle: LoopHandle<'static, LoopData>,
    /// Signal d'arrêt de la boucle.
    pub loop_signal: LoopSignal,
    /// Nom de la socket Wayland (`WAYLAND_DISPLAY`).
    pub socket_name: OsString,
    /// `false` dès que l'arrêt est demandé.
    pub running: bool,

    /// Le noyau : moniteurs, clients, tags, focus.
    pub wm: Wm,
    /// Fenêtres gérées.
    pub windows: HashMap<ClientId, ManagedWindow>,
    /// Toplevels en attente de leur commit initial.
    pub unmapped: Vec<PendingWindow>,
    /// Sorties et suivi `wl_surface.enter/leave`.
    pub space: Space<Window>,
    /// Popups.
    pub popups: PopupManager,
    /// Sorties connues.
    pub outputs: Vec<OutputEntry>,
    /// Moniteur conservé sans sortie (toutes débranchées).
    pub orphan_monitor: Option<MonitorId>,

    /// Le siège (clavier + pointeur).
    pub seat: Seat<State>,
    /// Position du pointeur, en coordonnées globales.
    pub pointer_location: Point<f64, Logical>,
    /// Mode du pointeur.
    pub cursor_mode: CursorMode,
    /// Image de curseur demandée par le client survolé.
    pub cursor_status: CursorImageStatus,
    /// Thème de curseurs.
    pub cursor_theme: CursorTheme,
    /// Le compositeur dessine-t-il le curseur ?
    pub draw_cursor: bool,
    /// Icône de glisser-déposer.
    pub dnd_icon: Option<DndIcon>,
    /// Client actuellement survolé.
    pub hovered: Option<ClientId>,
    /// Touches dont l'appui a déclenché un raccourci (relâchement à avaler).
    pub suppressed_keys: HashSet<u32>,
    /// Boutons dont l'appui a déclenché une liaison (relâchement à avaler).
    pub suppressed_buttons: HashSet<u32>,
    /// Répétition de raccourci en cours.
    pub key_repeat: Option<KeyRepeat>,

    /// `wl_compositor`, `wl_subcompositor`.
    pub compositor_state: CompositorState,
    /// `xdg_wm_base`.
    pub xdg_shell_state: XdgShellState,
    /// `zxdg_decoration_manager_v1`.
    pub xdg_decoration_state: XdgDecorationState,
    /// `zwlr_layer_shell_v1`.
    pub layer_shell_state: WlrLayerShellState,
    /// `wl_shm`.
    pub shm_state: ShmState,
    /// `zwp_linux_dmabuf_v1`.
    pub dmabuf_state: DmabufState,
    /// Global dmabuf (présent si le backend sait importer des dmabufs).
    pub dmabuf_global: Option<DmabufGlobal>,
    /// `wl_output`, `zxdg_output_manager_v1`.
    pub output_manager_state: OutputManagerState,
    /// `wl_seat`.
    pub seat_state: SeatState<State>,
    /// `wl_data_device_manager`.
    pub data_device_state: DataDeviceState,
    /// `zwp_primary_selection_device_manager_v1`.
    pub primary_selection_state: PrimarySelectionState,
    /// `zwlr_data_control_manager_v1`.
    pub data_control_state: DataControlState,
    /// `xdg_activation_v1`.
    pub xdg_activation_state: XdgActivationState,
    /// `wp_viewporter`.
    pub viewporter_state: ViewporterState,
    /// `wp_single_pixel_buffer_manager_v1`.
    pub single_pixel_buffer_state: SinglePixelBufferState,
    /// `wp_cursor_shape_manager_v1`.
    pub cursor_shape_state: CursorShapeManagerState,

    /// Données du backend actif.
    pub backend: BackendData,
}

impl State {
    /// Crée l'état, les globals Wayland et la socket d'écoute.
    ///
    /// `socket` impose le nom de la socket ; `None` choisit le premier
    /// `wayland-N` libre.
    pub fn new(
        config: Config,
        loop_handle: LoopHandle<'static, LoopData>,
        loop_signal: LoopSignal,
        display: &mut Display<State>,
        socket: Option<&str>,
    ) -> Result<State, Box<dyn std::error::Error>> {
        let dh = display.handle();

        let compositor_state = CompositorState::new::<State>(&dh);
        // Seul le plein écran est proposé aux clients : ni maximisation ni
        // minimisation dans un gestionnaire à tuiles.
        let xdg_shell_state =
            XdgShellState::new_with_capabilities::<State>(&dh, [xdg_toplevel::WmCapabilities::Fullscreen]);
        let xdg_decoration_state = XdgDecorationState::new::<State>(&dh);
        let layer_shell_state = WlrLayerShellState::new::<State>(&dh);
        let shm_state = ShmState::new::<State>(&dh, Vec::new());
        let dmabuf_state = DmabufState::new();
        let output_manager_state = OutputManagerState::new_with_xdg_output::<State>(&dh);
        let data_device_state = DataDeviceState::new::<State>(&dh);
        let primary_selection_state = PrimarySelectionState::new::<State>(&dh);
        let data_control_state = DataControlState::new::<State, _>(&dh, Some(&primary_selection_state), |_client| true);
        let xdg_activation_state = XdgActivationState::new::<State>(&dh);
        let viewporter_state = ViewporterState::new::<State>(&dh);
        let single_pixel_buffer_state = SinglePixelBufferState::new::<State>(&dh);
        let cursor_shape_state = CursorShapeManagerState::new::<State>(&dh);

        let mut seat_state = SeatState::new();
        let mut seat: Seat<State> = seat_state.new_wl_seat(&dh, "seat0");
        let kb = &config.keyboard;
        let xkb = XkbConfig {
            rules: kb.rules,
            model: kb.model,
            layout: kb.layout,
            variant: kb.variant,
            options: kb.options.map(str::to_string),
        };
        if let Err(err) = seat.add_keyboard(xkb, kb.repeat_delay, kb.repeat_rate) {
            warn!(?err, "disposition de clavier invalide, retour à celle par défaut");
            seat.add_keyboard(XkbConfig::default(), kb.repeat_delay, kb.repeat_rate)
                .map_err(|e| format!("impossible d'initialiser le clavier : {e:?}"))?;
        }
        seat.add_pointer();

        // Socket d'écoute : chaque connexion devient un client.
        let source = match socket {
            Some(name) => ListeningSocketSource::with_name(name)?,
            None => ListeningSocketSource::new_auto()?,
        };
        let socket_name = source.socket_name().to_os_string();
        loop_handle.insert_source(source, |stream, _, data: &mut LoopData| {
            if let Err(err) = data.state.display_handle.insert_client(stream, Arc::new(ClientState::default())) {
                warn!(?err, "connexion d'un client refusée");
            }
        })?;

        // Requêtes des clients.
        let fd = display.backend().poll_fd().try_clone_to_owned()?;
        loop_handle.insert_source(Generic::new(fd, Interest::READ, Mode::Level), |_, _, data: &mut LoopData| {
            if let Err(err) = data.display.dispatch_clients(&mut data.state) {
                warn!(?err, "erreur de distribution des requêtes");
            }
            Ok(PostAction::Continue)
        })?;

        info!(socket = ?socket_name, "socket Wayland ouverte");

        Ok(State {
            start_time: Instant::now(),
            display_handle: dh,
            loop_handle,
            loop_signal,
            socket_name,
            running: true,
            wm: Wm::new(config),
            windows: HashMap::new(),
            unmapped: Vec::new(),
            space: Space::default(),
            popups: PopupManager::default(),
            outputs: Vec::new(),
            orphan_monitor: None,
            seat,
            pointer_location: Point::from((0.0, 0.0)),
            cursor_mode: CursorMode::Normal,
            cursor_status: CursorImageStatus::default_named(),
            cursor_theme: CursorTheme::load(),
            draw_cursor: false,
            dnd_icon: None,
            hovered: None,
            suppressed_keys: HashSet::new(),
            suppressed_buttons: HashSet::new(),
            key_repeat: None,
            compositor_state,
            xdg_shell_state,
            xdg_decoration_state,
            layer_shell_state,
            shm_state,
            dmabuf_state,
            dmabuf_global: None,
            output_manager_state,
            seat_state,
            data_device_state,
            primary_selection_state,
            data_control_state,
            xdg_activation_state,
            viewporter_state,
            single_pixel_buffer_state,
            cursor_shape_state,
            backend: BackendData::None,
        })
    }

    /// Millisecondes écoulées depuis le démarrage (horodatage des événements).
    pub fn time_msec(&self) -> u32 {
        self.start_time.elapsed().as_millis() as u32
    }

    /// Demande l'arrêt du compositeur.
    pub fn quit(&mut self) {
        self.running = false;
        self.loop_signal.stop();
        self.loop_signal.wakeup();
    }

    /// Entretien à effectuer après chaque tour de boucle.
    pub fn after_dispatch(&mut self) {
        self.space.refresh();
        self.popups.cleanup();
        if let Err(err) = self.display_handle.flush_clients() {
            debug!(?err, "échec de l'envoi aux clients");
        }
    }

    // ------------------------------------------------------------------
    // Sorties
    // ------------------------------------------------------------------

    /// Déclare une sortie : global `wl_output`, placement à droite des
    /// sorties existantes, moniteur dans le noyau.
    ///
    /// Le mode courant de la sortie doit déjà être défini.
    pub fn add_output(&mut self, output: Output) -> MonitorId {
        let global = output.create_global::<State>(&self.display_handle);
        let x = self.space.outputs().filter_map(|o| self.space.output_geometry(o)).map(|g| g.loc.x + g.size.w).max();
        let location = Point::from((x.unwrap_or(0), 0));

        let name = output.name();
        let mon = match self.orphan_monitor.take() {
            Some(mon) => {
                self.wm.rename_monitor(mon, &name);
                mon
            }
            None => self.wm.add_monitor(&name, Rect::new(location.x, location.y, 1, 1)),
        };
        let scale = self.wm.monitor(mon).map_or(1, |m| m.scale);
        output.change_current_state(None, None, Some(Scale::Integer(scale)), Some(location));
        self.space.map_output(&output, location);
        self.outputs.push(OutputEntry {
            output: output.clone(),
            global,
            mon,
            bar: None,
            fullscreen_backdrop: SolidColorBuffer::default(),
        });
        self.output_changed(&output);
        info!(name, scale, "sortie ajoutée");
        mon
    }

    /// Retire une sortie ; ses clients rejoignent un autre moniteur.
    pub fn remove_output(&mut self, output: &Output) {
        let Some(idx) = self.outputs.iter().position(|e| &e.output == output) else { return };
        let entry = self.outputs.remove(idx);
        self.display_handle.remove_global::<State>(entry.global);
        self.space.unmap_output(output);
        if !self.wm.remove_monitor(entry.mon) {
            // Dernière sortie : le moniteur est conservé pour héberger les
            // clients jusqu'au prochain branchement.
            self.orphan_monitor = Some(entry.mon);
        }
        info!(name = output.name(), "sortie retirée");
        self.repack_outputs();
        self.sync();
    }

    /// Replace les sorties côte à côte, de gauche à droite, sans trou (après
    /// un débranchement).
    fn repack_outputs(&mut self) {
        let outputs: Vec<Output> = self.outputs.iter().map(|e| e.output.clone()).collect();
        let mut x = 0;
        for output in outputs {
            let location = Point::from((x, 0));
            if self.space.output_geometry(&output).map(|g| g.loc) != Some(location) {
                output.change_current_state(None, None, None, Some(location));
                self.space.map_output(&output, location);
                self.output_changed(&output);
            }
            x += self.space.output_geometry(&output).map_or(0, |g| g.size.w);
        }
    }

    /// À appeler quand le mode, l'échelle ou les zones réservées d'une sortie
    /// changent : met à jour la géométrie du moniteur.
    pub fn output_changed(&mut self, output: &Output) {
        let Some(entry) = self.outputs.iter().find(|e| &e.output == output) else { return };
        let mon = entry.mon;
        let Some(geo) = self.space.output_geometry(output) else { return };
        let zone = {
            let mut map = layer_map_for_output(output);
            map.arrange();
            map.non_exclusive_zone()
        };
        let screen = Rect::new(geo.loc.x, geo.loc.y, geo.size.w, geo.size.h);
        let usable = Rect::new(geo.loc.x + zone.loc.x, geo.loc.y + zone.loc.y, zone.size.w, zone.size.h);
        self.wm.set_monitor_geometry(mon, screen, usable);
        self.sync();
    }

    /// La sortie associée à un moniteur.
    pub fn output_for_monitor(&self, mon: MonitorId) -> Option<&Output> {
        self.outputs.iter().find(|e| e.mon == mon).map(|e| &e.output)
    }

    /// Union des géométries des sorties.
    pub fn outputs_bounds(&self) -> Option<Rectangle<i32, Logical>> {
        self.space.outputs().filter_map(|o| self.space.output_geometry(o)).reduce(|a, b| a.merge(b))
    }

    // ------------------------------------------------------------------
    // Clients
    // ------------------------------------------------------------------

    /// Le client géré dont `surface` est la surface racine.
    pub fn client_for_surface(&self, surface: &WlSurface) -> Option<ClientId> {
        self.windows
            .iter()
            .find(|(_, w)| w.window.toplevel().is_some_and(|t| t.wl_surface() == surface))
            .map(|(&id, _)| id)
    }

    /// Prend en charge un toplevel à son commit initial.
    pub fn manage_window(&mut self, pending: PendingWindow) {
        let PendingWindow { window, fullscreen } = pending;
        let Some(toplevel) = window.toplevel().cloned() else { return };
        let surface = toplevel.wl_surface().clone();

        let (app_id, title, parent, min, max) = with_states(&surface, |states| {
            let (app_id, title, parent) = states
                .data_map
                .get::<XdgToplevelSurfaceData>()
                .and_then(|data| data.lock().ok().map(|d| (d.app_id.clone(), d.title.clone(), d.parent.clone())))
                .unwrap_or_default();
            let mut guard = states.cached_state.get::<SurfaceCachedState>();
            let cached = guard.current();
            (app_id, title, parent, cached.min_size, cached.max_size)
        });
        let parent = parent.and_then(|p| self.client_for_surface(&p));

        let new = NewClient {
            app_id: app_id.unwrap_or_default(),
            title: title.unwrap_or_default(),
            parent,
            min_size: (min.w, min.h),
            max_size: (max.w, max.h),
        };
        debug!(app_id = new.app_id, title = new.title, "nouveau client");
        let Some(id) = self.wm.manage(new) else {
            // Aucun moniteur : le toplevel reste en attente.
            self.unmapped.push(PendingWindow { window, fullscreen });
            return;
        };
        if fullscreen {
            self.wm.set_fullscreen(id, true);
        }
        let borders = std::array::from_fn(|_| SolidColorBuffer::default());
        self.windows.insert(id, ManagedWindow { window, borders, mapped: false });
        self.sync();
        if !toplevel.is_initial_configure_sent() {
            toplevel.send_configure();
        }
    }

    /// Cesse de gérer un client (fenêtre détruite ou démappée).
    pub fn unmanage_window(&mut self, id: ClientId) -> Option<ManagedWindow> {
        let managed = self.windows.remove(&id)?;
        self.space.unmap_elem(&managed.window);
        self.wm.unmanage(id);
        if matches!(self.cursor_mode, CursorMode::Move { client, .. } | CursorMode::Resize { client, .. } if client == id)
        {
            self.cursor_mode = CursorMode::Normal;
        }
        self.sync();
        Some(managed)
    }

    // ------------------------------------------------------------------
    // Actions et synchronisation
    // ------------------------------------------------------------------

    /// Exécute une action du noyau, ses effets, puis synchronise.
    pub fn apply_action(&mut self, action: &Action) {
        let effects = self.wm.apply(action);
        self.run_effects(effects);
        self.sync();
    }

    /// Réalise les effets demandés par le noyau.
    pub fn run_effects(&mut self, effects: Vec<Effect>) {
        for effect in effects {
            match effect {
                Effect::Spawn(argv) => crate::spawn::spawn(&argv, &self.socket_name),
                Effect::Close(id) => {
                    if let Some(toplevel) = self.windows.get(&id).and_then(|w| w.window.toplevel()) {
                        toplevel.send_close();
                    }
                }
                Effect::Quit => self.quit(),
                Effect::ChVt(vt) => {
                    let mut backend = std::mem::replace(&mut self.backend, BackendData::None);
                    backend.change_vt(vt);
                    self.backend = backend;
                }
                Effect::BeginMove(id) => {
                    if let Some(c) = self.wm.client(id) {
                        let (px, py) = (self.pointer_location.x as i32, self.pointer_location.y as i32);
                        self.cursor_mode = CursorMode::Move { client: id, offset: (px - c.geom.x, py - c.geom.y) };
                    }
                }
                Effect::BeginResize(id) => {
                    if let Some(c) = self.wm.client(id) {
                        let (px, py) = (self.pointer_location.x as i32, self.pointer_location.y as i32);
                        self.cursor_mode =
                            CursorMode::Resize { client: id, start: (px, py), size: (c.geom.w, c.geom.h) };
                    }
                }
            }
        }
    }

    /// Applique le modèle aux fenêtres : taille et états configurés,
    /// placement, visibilité, focus clavier, focus pointeur.
    ///
    /// Idempotent : sans changement du modèle, rien n'est envoyé.
    pub fn sync(&mut self) {
        let selected = self.wm.selected();
        for (&id, managed) in &self.windows {
            let Some(client) = self.wm.client(id) else { continue };
            let Some(toplevel) = managed.window.toplevel() else { continue };
            let inner = client.inner();
            let bounds = self.wm.monitor(client.mon).map(|m| m.window_area);
            let bw = client.bw;

            toplevel.with_pending_state(|state| {
                // Un client flottant qui vient d'apparaître choisit sa taille.
                state.size = (!client.needs_initial_geometry).then(|| (inner.w.max(1), inner.h.max(1)).into());
                state.bounds = bounds.map(|a| ((a.w - 2 * bw).max(1), (a.h - 2 * bw).max(1)).into());
                if client.is_fullscreen {
                    state.states.set(xdg_toplevel::State::Fullscreen);
                } else {
                    state.states.unset(xdg_toplevel::State::Fullscreen);
                }
                // Tous les clients sont déclarés « tuilés » sur leurs quatre
                // bords, y compris les flottants (comme dwl) : les clients
                // renoncent ainsi à leurs ombres et à leurs coins arrondis.
                for edge in [
                    xdg_toplevel::State::TiledLeft,
                    xdg_toplevel::State::TiledRight,
                    xdg_toplevel::State::TiledTop,
                    xdg_toplevel::State::TiledBottom,
                ] {
                    state.states.set(edge);
                }
                if selected == Some(id) {
                    state.states.set(xdg_toplevel::State::Activated);
                } else {
                    state.states.unset(xdg_toplevel::State::Activated);
                }
            });
            if toplevel.is_initial_configure_sent() {
                toplevel.send_pending_configure();
            }

            if self.wm.is_visible(id) && managed.mapped {
                // `Space` place l'origine de la géométrie de la fenêtre.
                let loc = Point::from((inner.x, inner.y));
                if self.space.element_location(&managed.window) != Some(loc) {
                    self.space.map_element(managed.window.clone(), loc, false);
                }
            } else {
                self.space.unmap_elem(&managed.window);
            }
        }
        if let CursorMode::Move { client, .. } | CursorMode::Resize { client, .. } = self.cursor_mode {
            if !self.wm.is_visible(client) || self.wm.client(client).is_some_and(|c| c.is_fullscreen) {
                self.cursor_mode = CursorMode::Normal;
            }
        }
        self.update_keyboard_focus();
        self.refresh_pointer_focus();
    }

    /// Surface qui doit recevoir le clavier : une surface *layer-shell*
    /// exclusive (couches `overlay` puis `top`) s'il en existe, sinon le
    /// client sélectionné.
    fn keyboard_focus_target(&self) -> Option<WlSurface> {
        for layer in [Layer::Overlay, Layer::Top] {
            for entry in &self.outputs {
                let map = layer_map_for_output(&entry.output);
                let found = map
                    .layers_on(layer)
                    .rev()
                    .find(|l| l.cached_state().keyboard_interactivity == KeyboardInteractivity::Exclusive)
                    .map(|l| l.wl_surface().clone());
                if found.is_some() {
                    return found;
                }
            }
        }
        let selected = self.wm.selected()?;
        self.windows.get(&selected)?.window.toplevel().map(|t| t.wl_surface().clone())
    }

    /// Donne le focus clavier à la bonne surface, si elle a changé.
    pub fn update_keyboard_focus(&mut self) {
        let target = self.keyboard_focus_target();
        let Some(keyboard) = self.seat.get_keyboard() else { return };
        if keyboard.current_focus() != target {
            keyboard.set_focus(self, target, SERIAL_COUNTER.next_serial());
        }
    }

    /// Recalcule la surface sous le pointeur sans le déplacer (la disposition
    /// a pu changer sous lui). Le focus clavier n'est **pas** modifié : comme
    /// dwm, seul un vrai mouvement de souris fait suivre le focus.
    pub fn refresh_pointer_focus(&mut self) {
        let Some(pointer) = self.seat.get_pointer() else { return };
        let location = self.pointer_location;
        let hit = self.hit_test(location);
        self.hovered = hit.client();
        let under = if self.cursor_mode == CursorMode::Normal { hit.surface() } else { None };
        // Aucun événement n'est émis si la surface survolée n'a pas changé.
        if pointer.current_focus() == under.as_ref().map(|(surface, _)| surface.clone()) {
            return;
        }
        let event = MotionEvent { location, serial: SERIAL_COUNTER.next_serial(), time: self.time_msec() };
        pointer.motion(self, under, &event);
        pointer.frame(self);
    }

    /// La disposition courante du moniteur place-t-elle les clients ?
    pub fn layout_kind(&self, mon: MonitorId) -> Option<LayoutKind> {
        let m = self.wm.monitor(mon)?;
        Some(self.wm.config().layouts[m.current_layout()].kind)
    }
}
