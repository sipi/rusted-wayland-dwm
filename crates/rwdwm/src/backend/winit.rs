//! Backend *winit* : rwdwm tourne dans une fenêtre d'une session Wayland ou
//! X11 existante. Pratique pour essayer une configuration sans quitter sa
//! session.

use std::time::Duration;

use rwdwm_core::config::Color;
use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::backend::egl::EGLDevice;
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::renderer::{ImportDma, ImportMemWl};
use smithay::backend::winit::{self, WinitEvent, WinitGraphicsBackend};
use smithay::output::{Mode, Output, PhysicalProperties, Subpixel};
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay::utils::{Physical, Rectangle, Transform};
use smithay::wayland::dmabuf::DmabufFeedbackBuilder;
use tracing::{info, warn};

use super::{create, run_loop, BackendData, Error, FrameEvent, FrameStats, Options};
use crate::render::{color32, output_elements, send_frames, OutputElement};
use crate::state::{LoopData, State};

/// Données du backend winit.
pub struct WinitData {
    backend: WinitGraphicsBackend<GlesRenderer>,
    damage_tracker: OutputDamageTracker,
    output: Output,
    stats: FrameStats,
    /// La surface EGL de la fenêtre n'est peut-être plus courante : avant la
    /// première image, ou depuis qu'un import de texture ou de dmabuf a
    /// utilisé le contexte sans elle.
    surface_stale: bool,
}

impl WinitData {
    /// Importe un dmabuf dans le moteur de rendu.
    pub fn import_dmabuf(&mut self, dmabuf: &Dmabuf) -> bool {
        self.surface_stale = true;
        self.backend.renderer().import_dmabuf(dmabuf, None).is_ok()
    }

    /// Compose et présente une image. Renvoie `false` si rien n'a changé.
    fn render(&mut self, state: &mut State) -> Result<bool, Error> {
        let clear = color32(Color(state.wm.config().colors.root.0));
        // L'âge du tampon ne se lit que si la surface EGL est courante (sinon
        // EGL signale BAD_SURFACE). Quand elle ne l'est plus, tout est redessiné.
        let age = if self.surface_stale { 0 } else { self.backend.buffer_age().unwrap_or(0) };
        // Construire les éléments importe des textures, ce qui détache la
        // surface du contexte ; seul un rendu effectif la rend de nouveau courante.
        self.surface_stale = true;
        let damage: Option<Vec<Rectangle<i32, Physical>>> = {
            let (renderer, mut framebuffer) = self.backend.bind()?;
            let elements: Vec<OutputElement<GlesRenderer>> = output_elements(state, renderer, &self.output);
            let result = self
                .damage_tracker
                .render_output(renderer, &mut framebuffer, age, &elements, clear)
                .map_err(|err| format!("rendu : {err:?}"))?;
            result.damage.cloned()
        };
        match damage {
            Some(damage) => {
                // Le rendu vient de rendre la surface courante.
                self.surface_stale = false;
                self.backend.submit(Some(&damage))?;
                Ok(true)
            }
            None => Ok(false),
        }
    }
}

/// Exécute rwdwm dans une fenêtre.
pub fn run(options: Options) -> Result<(), Error> {
    let (event_loop, mut data) = create(&options)?;
    let state = &mut data.state;

    let (mut backend, winit_source) =
        winit::init::<GlesRenderer>().map_err(|err| format!("initialisation de winit : {err}"))?;
    backend.window().set_title("rwdwm");

    let mode = Mode { size: backend.window_size(), refresh: 60_000 };
    let output = Output::new(
        "winit".to_string(),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "rwdwm".into(),
            model: "winit".into(),
        },
    );
    // Le tampon de la fenêtre a son origine en bas à gauche.
    output.change_current_state(Some(mode), Some(Transform::Flipped180), None, None);
    output.set_preferred(mode);

    // Formats de tampons acceptés.
    state.shm_state.update_formats(backend.renderer().shm_formats());
    let dmabuf_formats = backend.renderer().dmabuf_formats();
    let render_node = EGLDevice::device_for_display(backend.renderer().egl_context().display())
        .and_then(|device| device.try_get_render_node());
    let feedback = match render_node {
        Ok(Some(node)) => DmabufFeedbackBuilder::new(node.dev_id(), dmabuf_formats.clone()).build().ok(),
        _ => None,
    };
    let global = match feedback {
        Some(feedback) => {
            state.dmabuf_state.create_global_with_default_feedback::<State>(&state.display_handle, &feedback)
        }
        None => {
            warn!("nœud de rendu introuvable : linux-dmabuf en version 3");
            state.dmabuf_state.create_global::<State>(&state.display_handle, dmabuf_formats)
        }
    };
    state.dmabuf_global = Some(global);

    // Le compositeur dessine le curseur ; celui de l'hôte est masqué.
    backend.window().set_cursor_visible(false);
    state.draw_cursor = true;

    let damage_tracker = OutputDamageTracker::from_output(&output);
    state.add_output(output.clone());
    state.backend = BackendData::Winit(Box::new(WinitData {
        backend,
        damage_tracker,
        output: output.clone(),
        stats: FrameStats::new("winit"),
        surface_stale: true,
    }));

    let handle = event_loop.handle();
    handle.insert_source(winit_source, move |event, _, data: &mut LoopData| {
        let state = &mut data.state;
        match event {
            WinitEvent::Resized { size, .. } => {
                output.change_current_state(Some(Mode { size, refresh: 60_000 }), None, None, None);
                state.output_changed(&output);
            }
            WinitEvent::Input(event) => state.process_input_event(event),
            WinitEvent::Redraw => redraw(state),
            WinitEvent::CloseRequested => state.quit(),
            WinitEvent::Focus(_) => {}
        }
    })?;

    // La fenêtre est redessinée à 60 Hz ; sans changement, rien n'est
    // présenté (le suivi des dégâts rend l'opération quasi gratuite).
    handle.insert_source(Timer::immediate(), |_, _, data: &mut LoopData| {
        if let BackendData::Winit(winit) = &data.state.backend {
            winit.backend.window().request_redraw();
        }
        TimeoutAction::ToDuration(Duration::from_millis(16))
    })?;

    crate::status::watch_stdin(&handle);
    info!(largeur = mode.size.w, hauteur = mode.size.h, "backend winit prêt");
    run_loop(event_loop, data, &options)
}

fn redraw(state: &mut State) {
    // Les données du backend sont empruntées à l'état le temps du rendu.
    let BackendData::Winit(mut winit) = std::mem::replace(&mut state.backend, BackendData::None) else { return };
    match winit.render(state) {
        Ok(true) => winit.stats.record(FrameEvent::Presented),
        Ok(false) => winit.stats.record(FrameEvent::Idle),
        Err(err) => {
            warn!(%err, "échec du rendu");
            winit.stats.record(FrameEvent::Error);
        }
    }
    send_frames(state, &winit.output);
    state.backend = BackendData::Winit(winit);
}
