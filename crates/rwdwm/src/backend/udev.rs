//! Backend natif : session libseat, périphériques DRM/KMS découverts par
//! udev, rendu OpenGL ES par GBM/EGL, entrées par libinput.
//!
//! Limites de cette version : seul le GPU principal pilote des écrans ;
//! le mode préféré de chaque écran est utilisé ; la composition se fait
//! toujours sur le plan primaire (ni *direct scan-out*, ni plan curseur).

use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use rwdwm_core::config::{Color, Touchpad};
use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::backend::allocator::format::FormatSet;
use smithay::backend::allocator::gbm::{GbmAllocator, GbmBufferFlags, GbmDevice};
use smithay::backend::allocator::Fourcc;
use smithay::backend::drm::compositor::FrameFlags;
use smithay::backend::drm::exporter::gbm::GbmFramebufferExporter;
use smithay::backend::drm::output::{DrmOutput, DrmOutputManager, DrmOutputRenderElements};
use smithay::backend::drm::{DrmDevice, DrmDeviceFd, DrmEvent, DrmNode, NodeType};
use smithay::backend::egl::context::ContextPriority;
use smithay::backend::input::InputEvent;
use smithay::backend::libinput::{LibinputInputBackend, LibinputSessionInterface};
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::renderer::multigpu::gbm::GbmGlesBackend;
use smithay::backend::renderer::multigpu::{GpuManager, MultiRenderer};
use smithay::backend::renderer::{ImportDma, ImportMemWl};
use smithay::backend::session::libseat::LibSeatSession;
use smithay::backend::session::{Event as SessionEvent, Session};
use smithay::backend::udev::{all_gpus, primary_gpu, UdevBackend, UdevEvent};
use smithay::output::{Mode, Output, PhysicalProperties};
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay::reexports::calloop::RegistrationToken;
use smithay::reexports::drm::control::{connector, crtc, Device as ControlDevice, ModeTypeFlags};
use smithay::reexports::input::{Device as InputDevice, Libinput};
use smithay::reexports::rustix::fs::OFlags;
use smithay::utils::DeviceFd;
use smithay::wayland::dmabuf::DmabufFeedbackBuilder;
use tracing::{debug, error, info, warn};

use super::{create, run_loop, BackendData, Error, Options};
use crate::render::{color32, output_elements, send_frames, OutputElement};
use crate::state::{LoopData, State};

/// Formats de couleur essayés, dans l'ordre, pour le plan primaire.
const COLOR_FORMATS: &[Fourcc] = &[Fourcc::Argb8888, Fourcc::Xrgb8888, Fourcc::Abgr8888, Fourcc::Xbgr8888];

type Gpus = GpuManager<GbmGlesBackend<GlesRenderer, DrmDeviceFd>>;
type UdevRenderer<'a> =
    MultiRenderer<'a, 'a, GbmGlesBackend<GlesRenderer, DrmDeviceFd>, GbmGlesBackend<GlesRenderer, DrmDeviceFd>>;
type OutputManager = DrmOutputManager<GbmAllocator<DrmDeviceFd>, GbmFramebufferExporter<DrmDeviceFd>, (), DrmDeviceFd>;
type Surface = DrmOutput<GbmAllocator<DrmDeviceFd>, GbmFramebufferExporter<DrmDeviceFd>, (), DrmDeviceFd>;

/// Où en est la boucle de rendu d'un écran. Un seul rendu est en attente à
/// la fois : soit programmé par minuterie, soit suspendu au prochain VBlank.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pending {
    Idle,
    Scheduled,
    WaitingVBlank,
}

struct Screen {
    output: Output,
    surface: Surface,
    pending: Pending,
}

struct Device {
    token: RegistrationToken,
    manager: OutputManager,
    render_node: DrmNode,
    screens: HashMap<crtc::Handle, Screen>,
    connectors: HashMap<connector::Handle, crtc::Handle>,
}

/// Données du backend natif.
pub struct UdevData {
    session: LibSeatSession,
    primary_gpu: DrmNode,
    gpus: Gpus,
    devices: HashMap<DrmNode, Device>,
    active: bool,
}

impl UdevData {
    /// Importe un dmabuf dans le moteur de rendu du GPU principal.
    pub fn import_dmabuf(&mut self, dmabuf: &Dmabuf) -> bool {
        match self.gpus.single_renderer(&self.primary_gpu) {
            Ok(mut renderer) => renderer.import_dmabuf(dmabuf, None).is_ok(),
            Err(_) => false,
        }
    }

    /// Bascule sur le terminal virtuel `vt`.
    pub fn change_vt(&mut self, vt: i32) {
        if let Err(err) = self.session.change_vt(vt) {
            warn!(%err, vt, "changement de terminal virtuel impossible");
        }
    }
}

/// Emprunte les données du backend à l'état le temps d'une opération qui a
/// besoin des deux.
fn with_udev<T>(state: &mut State, f: impl FnOnce(&mut State, &mut UdevData) -> T) -> Option<T> {
    let BackendData::Udev(mut udev) = std::mem::replace(&mut state.backend, BackendData::None) else { return None };
    let result = f(state, &mut udev);
    state.backend = BackendData::Udev(udev);
    Some(result)
}

/// Exécute rwdwm sur le matériel (depuis un terminal virtuel).
pub fn run(options: Options) -> Result<(), Error> {
    let (event_loop, mut data) = create(&options)?;
    let state = &mut data.state;
    let handle = event_loop.handle();

    // Session : accès aux périphériques sans privilèges (logind ou seatd).
    let (session, notifier) = LibSeatSession::new().map_err(|err| {
        format!("ouverture de la session impossible ({err}) ; rwdwm doit être lancé depuis un terminal virtuel, avec logind ou seatd")
    })?;
    let seat_name = session.seat();

    // GPU principal.
    let primary_path = match primary_gpu(&seat_name)? {
        Some(path) => path,
        None => all_gpus(&seat_name)?.into_iter().next().ok_or("aucun GPU trouvé")?,
    };
    let primary_card = DrmNode::from_path(&primary_path)?;
    let primary_render = primary_card.node_with_type(NodeType::Render).and_then(Result::ok).unwrap_or(primary_card);
    info!(gpu = %primary_path.display(), "GPU principal");

    let gpus: Gpus = GpuManager::new(GbmGlesBackend::with_context_priority(ContextPriority::High))
        .map_err(|err| format!("initialisation du rendu : {err}"))?;
    state.backend = BackendData::Udev(Box::new(UdevData {
        session: session.clone(),
        primary_gpu: primary_render,
        gpus,
        devices: HashMap::new(),
        active: true,
    }));
    state.draw_cursor = true;

    // Entrées.
    let mut libinput = Libinput::new_with_udev::<LibinputSessionInterface<LibSeatSession>>(session.clone().into());
    libinput.udev_assign_seat(&seat_name).map_err(|()| "libinput : siège inaccessible")?;
    let touchpad = options.config.touchpad;
    handle.insert_source(LibinputInputBackend::new(libinput.clone()), move |mut event, _, data: &mut LoopData| {
        if let InputEvent::DeviceAdded { device } = &mut event {
            configure_input_device(device, &touchpad);
        }
        data.state.process_input_event(event);
    })?;

    // Changements de terminal virtuel.
    handle.insert_source(notifier, move |event, _, data: &mut LoopData| match event {
        SessionEvent::PauseSession => {
            info!("session suspendue");
            libinput.suspend();
            with_udev(&mut data.state, |_, udev| {
                udev.active = false;
                for device in udev.devices.values_mut() {
                    device.manager.pause();
                }
            });
        }
        SessionEvent::ActivateSession => {
            info!("session reprise");
            if libinput.resume().is_err() {
                error!("libinput : reprise impossible");
            }
            let screens = with_udev(&mut data.state, |_, udev| {
                udev.active = true;
                let mut screens = Vec::new();
                for (node, device) in udev.devices.iter_mut() {
                    if let Err(err) = device.manager.activate(false) {
                        error!(%err, "réactivation du périphérique DRM impossible");
                    }
                    for (crtc, screen) in device.screens.iter_mut() {
                        // Un VBlank attendu avant la suspension n'arrivera plus.
                        screen.pending = Pending::Idle;
                        screens.push((*node, *crtc));
                    }
                }
                screens
            });
            for (node, crtc) in screens.unwrap_or_default() {
                schedule_render(&mut data.state, node, crtc, Duration::ZERO);
            }
        }
    })?;

    // Périphériques graphiques présents, puis branchements à chaud.
    let udev_backend = UdevBackend::new(&seat_name)?;
    for (dev_id, path) in udev_backend.device_list() {
        match DrmNode::from_dev_id(dev_id) {
            Ok(node) => device_added(state, node, path),
            Err(err) => warn!(%err, "nœud DRM illisible"),
        }
    }
    handle.insert_source(udev_backend, |event, _, data: &mut LoopData| match event {
        UdevEvent::Added { device_id, path } => {
            if let Ok(node) = DrmNode::from_dev_id(device_id) {
                device_added(&mut data.state, node, &path);
            }
        }
        UdevEvent::Changed { device_id } => {
            if let Ok(node) = DrmNode::from_dev_id(device_id) {
                scan_connectors(&mut data.state, node);
            }
        }
        UdevEvent::Removed { device_id } => {
            if let Ok(node) = DrmNode::from_dev_id(device_id) {
                device_removed(&mut data.state, node);
            }
        }
    })?;

    // Formats de tampons acceptés (shm et dmabuf).
    let formats = with_udev(state, |_, udev| {
        let renderer = udev.gpus.single_renderer(&udev.primary_gpu).ok()?;
        Some((renderer.shm_formats(), renderer.dmabuf_formats()))
    })
    .flatten()
    .ok_or("le GPU principal n'a pas pu être initialisé")?;
    state.shm_state.update_formats(formats.0);
    match DmabufFeedbackBuilder::new(primary_render.dev_id(), formats.1).build() {
        Ok(feedback) => {
            let global =
                state.dmabuf_state.create_global_with_default_feedback::<State>(&state.display_handle, &feedback);
            state.dmabuf_global = Some(global);
        }
        Err(err) => warn!(%err, "linux-dmabuf indisponible"),
    }

    if state.outputs.is_empty() {
        warn!("aucun écran connecté");
    }
    crate::status::watch_stdin(&handle);
    info!("backend udev prêt");
    run_loop(event_loop, data, &options)
}

/// Applique la configuration aux pavés tactiles.
fn configure_input_device(device: &mut InputDevice, touchpad: &Touchpad) {
    if device.config_tap_finger_count() == 0 {
        return; // pas un pavé tactile
    }
    let _ = device.config_tap_set_enabled(touchpad.tap_to_click);
    if device.config_scroll_has_natural_scroll() {
        let _ = device.config_scroll_set_natural_scroll_enabled(touchpad.natural_scrolling);
    }
    if device.config_dwt_is_available() {
        let _ = device.config_dwt_set_enabled(touchpad.disable_while_typing);
    }
    debug!(name = device.name(), "pavé tactile configuré");
}

fn device_added(state: &mut State, node: DrmNode, path: &Path) {
    let loop_handle = state.loop_handle.clone();
    let added = with_udev(state, |_, udev| -> Result<bool, Error> {
        if udev.devices.contains_key(&node) {
            return Ok(false);
        }
        let render_node = node.node_with_type(NodeType::Render).and_then(Result::ok).unwrap_or(node);
        if render_node != udev.primary_gpu {
            warn!(device = %path.display(), "GPU secondaire ignoré (non pris en charge)");
            return Ok(false);
        }

        let fd = udev.session.open(path, OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOCTTY | OFlags::NONBLOCK)?;
        let fd = DrmDeviceFd::new(DeviceFd::from(fd));
        let (drm, notifier) = DrmDevice::new(fd.clone(), true)?;
        let gbm = GbmDevice::new(fd)?;
        udev.gpus.as_mut().add_node(render_node, gbm.clone())?;

        let render_formats: FormatSet = {
            let mut renderer = udev.gpus.single_renderer(&render_node).map_err(|err| err.to_string())?;
            renderer.as_mut().egl_context().dmabuf_render_formats().iter().copied().collect()
        };
        let allocator = GbmAllocator::new(gbm.clone(), GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT);
        let exporter = GbmFramebufferExporter::new(gbm.clone(), Some(render_node));
        let manager: OutputManager =
            DrmOutputManager::new(drm, allocator, exporter, Some(gbm), COLOR_FORMATS.iter().copied(), render_formats);

        let token = loop_handle.insert_source(notifier, move |event, _, data: &mut LoopData| match event {
            DrmEvent::VBlank(crtc) => frame_finished(&mut data.state, node, crtc),
            DrmEvent::Error(err) => error!(%err, "erreur DRM"),
        })?;
        udev.devices
            .insert(node, Device { token, manager, render_node, screens: HashMap::new(), connectors: HashMap::new() });
        info!(device = %path.display(), "périphérique graphique ajouté");
        Ok(true)
    });
    match added {
        Some(Ok(true)) => scan_connectors(state, node),
        Some(Err(err)) => error!(%err, device = %path.display(), "périphérique graphique inutilisable"),
        _ => {}
    }
}

fn device_removed(state: &mut State, node: DrmNode) {
    let loop_handle = state.loop_handle.clone();
    let outputs = with_udev(state, |_, udev| {
        let Some(device) = udev.devices.remove(&node) else { return Vec::new() };
        loop_handle.remove(device.token);
        udev.gpus.as_mut().remove_node(&device.render_node);
        device.screens.into_values().map(|screen| screen.output).collect()
    });
    for output in outputs.unwrap_or_default() {
        state.remove_output(&output);
    }
}

/// Compare les connecteurs branchés à ceux déjà pilotés ; crée ou retire les
/// sorties correspondantes.
fn scan_connectors(state: &mut State, node: DrmNode) {
    let changes = with_udev(state, |_, udev| {
        let Some(device) = udev.devices.get_mut(&node) else { return (Vec::new(), Vec::new()) };
        let drm = device.manager.device();
        let resources = match drm.resource_handles() {
            Ok(resources) => resources,
            Err(err) => {
                warn!(%err, "ressources DRM illisibles");
                return (Vec::new(), Vec::new());
            }
        };
        let connected: Vec<connector::Info> = resources
            .connectors()
            .iter()
            .filter_map(|handle| drm.get_connector(*handle, true).ok())
            .filter(|info| info.state() == connector::State::Connected && !info.modes().is_empty())
            .collect();

        // Connecteurs débranchés.
        let gone: Vec<connector::Handle> = device
            .connectors
            .keys()
            .filter(|handle| !connected.iter().any(|info| info.handle() == **handle))
            .copied()
            .collect();
        let mut removed = Vec::new();
        for handle in gone {
            if let Some(screen) = device.connectors.remove(&handle).and_then(|crtc| device.screens.remove(&crtc)) {
                removed.push(screen.output);
            }
        }

        // Nouveaux connecteurs : choisir un CRTC libre.
        let mut added = Vec::new();
        let mut used: Vec<crtc::Handle> = device.connectors.values().copied().collect();
        for info in connected {
            if device.connectors.contains_key(&info.handle()) {
                continue;
            }
            let encoders: Vec<_> = info.encoders().iter().filter_map(|e| drm.get_encoder(*e).ok()).collect();
            let current = info
                .current_encoder()
                .and_then(|e| drm.get_encoder(e).ok())
                .and_then(|e| e.crtc())
                .filter(|crtc| !used.contains(crtc));
            let crtc = current.or_else(|| {
                encoders
                    .iter()
                    .flat_map(|e| resources.filter_crtcs(e.possible_crtcs()))
                    .find(|crtc| !used.contains(crtc))
            });
            match crtc {
                Some(crtc) => {
                    used.push(crtc);
                    added.push((info, crtc));
                }
                None => warn!(connector = ?info.interface(), "aucun CRTC libre pour ce connecteur"),
            }
        }
        (removed, added)
    });
    let Some((removed, added)) = changes else { return };
    for output in removed {
        state.remove_output(&output);
    }
    for (info, crtc) in added {
        connector_connected(state, node, info, crtc);
    }
}

fn connector_connected(state: &mut State, node: DrmNode, info: connector::Info, crtc: crtc::Handle) {
    let name = format!("{}-{}", info.interface().as_str(), info.interface_id());
    let modes = info.modes();
    let drm_mode =
        modes.iter().find(|mode| mode.mode_type().contains(ModeTypeFlags::PREFERRED)).copied().unwrap_or(modes[0]);
    let mode = Mode::from(drm_mode);
    let (width_mm, height_mm) = info.size().unwrap_or((0, 0));
    let output = Output::new(
        name.clone(),
        PhysicalProperties {
            size: (width_mm as i32, height_mm as i32).into(),
            subpixel: info.subpixel().into(),
            make: "Inconnu".into(),
            model: "Inconnu".into(),
        },
    );
    output.set_preferred(mode);
    output.change_current_state(Some(mode), None, None, None);

    let created = with_udev(state, |_, udev| -> Result<(), Error> {
        let device = udev.devices.get_mut(&node).ok_or("périphérique inconnu")?;
        let mut renderer = udev.gpus.single_renderer(&device.render_node).map_err(|err| err.to_string())?;
        let surface: Surface = device
            .manager
            .initialize_output::<_, OutputElement<UdevRenderer<'_>>>(
                crtc,
                drm_mode,
                &[info.handle()],
                &output,
                None,
                &mut renderer,
                &DrmOutputRenderElements::default(),
            )
            .map_err(|err| err.to_string())?;
        device.screens.insert(crtc, Screen { output: output.clone(), surface, pending: Pending::Idle });
        device.connectors.insert(info.handle(), crtc);
        Ok(())
    });
    match created {
        Some(Ok(())) => {
            info!(name, width = mode.size.w, height = mode.size.h, refresh = mode.refresh, "écran connecté");
            state.add_output(output);
            schedule_render(state, node, crtc, Duration::ZERO);
        }
        Some(Err(err)) => error!(%err, name, "écran inutilisable"),
        None => {}
    }
}

/// Programme le prochain rendu d'un écran, s'il n'y en a pas déjà un.
fn schedule_render(state: &mut State, node: DrmNode, crtc: crtc::Handle, delay: Duration) {
    let loop_handle = state.loop_handle.clone();
    with_udev(state, |_, udev| {
        let Some(screen) = udev.devices.get_mut(&node).and_then(|d| d.screens.get_mut(&crtc)) else { return };
        if screen.pending != Pending::Idle {
            return;
        }
        let timer = Timer::from_duration(delay);
        let inserted = loop_handle.insert_source(timer, move |_, _, data: &mut LoopData| {
            render(&mut data.state, node, crtc);
            TimeoutAction::Drop
        });
        if inserted.is_ok() {
            screen.pending = Pending::Scheduled;
        }
    });
}

/// Compose une image pour un écran et la met en file pour le prochain VBlank.
fn render(state: &mut State, node: DrmNode, crtc: crtc::Handle) {
    let clear = color32(Color(state.wm.config().colors.root.0));
    let outcome = with_udev(state, |state, udev| {
        let active = udev.active;
        let device = udev.devices.get_mut(&node)?;
        let screen = device.screens.get_mut(&crtc)?;
        screen.pending = Pending::Idle;
        if !active {
            return None; // autre terminal virtuel : la reprise relancera le rendu
        }
        let output = screen.output.clone();
        let mut renderer = udev.gpus.single_renderer(&device.render_node).ok()?;
        let elements: Vec<OutputElement<UdevRenderer<'_>>> = output_elements(state, &mut renderer, &output);
        // Composition systématique sur le plan primaire.
        let rendered = match screen.surface.render_frame(&mut renderer, &elements, clear, FrameFlags::empty()) {
            Ok(result) => !result.is_empty,
            Err(err) => {
                warn!(%err, "échec du rendu");
                false
            }
        };
        let queued = rendered
            && match screen.surface.queue_frame(()) {
                Ok(()) => true,
                Err(err) => {
                    warn!(%err, "échec de la présentation");
                    false
                }
            };
        if queued {
            screen.pending = Pending::WaitingVBlank;
        }
        Some((output, queued))
    })
    .flatten();

    let Some((output, queued)) = outcome else { return };
    send_frames(state, &output);
    if !queued {
        // Rien à présenter : nouvel essai à la prochaine période.
        let period = output.current_mode().map_or(16_667, |mode| 1_000_000_000 / mode.refresh.max(1) as u64);
        schedule_render(state, node, crtc, Duration::from_micros(period));
    }
}

/// VBlank : l'image précédente est à l'écran, la suivante peut être composée.
fn frame_finished(state: &mut State, node: DrmNode, crtc: crtc::Handle) {
    let known = with_udev(state, |_, udev| {
        let Some(screen) = udev.devices.get_mut(&node).and_then(|d| d.screens.get_mut(&crtc)) else { return false };
        if let Err(err) = screen.surface.frame_submitted() {
            warn!(%err, "acquittement de trame");
        }
        screen.pending = Pending::Idle;
        true
    });
    if known == Some(true) {
        schedule_render(state, node, crtc, Duration::ZERO);
    }
}
