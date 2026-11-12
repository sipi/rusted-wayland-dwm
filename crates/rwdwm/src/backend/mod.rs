//! Backends : `udev` (natif, DRM/KMS), `winit` (imbriqué dans une session
//! existante) et `headless` (sortie virtuelle, pour les tests).

use std::time::Duration;

use rwdwm_core::Config;
use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay::reexports::calloop::EventLoop;
use smithay::reexports::wayland_server::Display;
use tracing::info;

use crate::state::{LoopData, State};

#[cfg(feature = "headless")]
pub mod headless;
#[cfg(feature = "udev")]
pub mod udev;
#[cfg(feature = "winit")]
pub mod winit;

/// Erreur d'initialisation ou d'exécution d'un backend.
pub type Error = Box<dyn std::error::Error>;

/// Données propres au backend actif.
pub enum BackendData {
    /// Aucun backend (pendant l'initialisation, ou emprunt temporaire).
    None,
    /// Backend de test.
    #[cfg(feature = "headless")]
    Headless(headless::HeadlessData),
    /// Backend imbriqué.
    #[cfg(feature = "winit")]
    Winit(Box<winit::WinitData>),
    /// Backend natif.
    #[cfg(feature = "udev")]
    Udev(Box<udev::UdevData>),
}

impl BackendData {
    /// Importe un dmabuf dans le moteur de rendu ; `false` s'il est refusé.
    pub fn import_dmabuf(&mut self, dmabuf: &Dmabuf) -> bool {
        match self {
            #[cfg(feature = "winit")]
            BackendData::Winit(data) => data.import_dmabuf(dmabuf),
            #[cfg(feature = "udev")]
            BackendData::Udev(data) => data.import_dmabuf(dmabuf),
            _ => {
                let _ = dmabuf;
                false
            }
        }
    }

    /// Bascule sur un autre terminal virtuel (backend natif uniquement).
    pub fn change_vt(&mut self, vt: i32) {
        match self {
            #[cfg(feature = "udev")]
            BackendData::Udev(data) => data.change_vt(vt),
            _ => {
                let _ = vt;
            }
        }
    }
}

/// Ce qu'a donné une tentative de rendu.
#[cfg(any(feature = "udev", feature = "winit"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FrameEvent {
    /// Une image a été composée et envoyée à l'écran.
    Presented,
    /// Rien n'avait changé : aucune image envoyée.
    Idle,
    /// Le rendu ou la présentation a échoué.
    Error,
    /// L'écran a confirmé l'affichage d'une image (backend natif).
    VBlank,
}

/// Compteurs de rendu d'une sortie, journalisés toutes les cinq secondes
/// (niveau `debug`) : de quoi savoir, journal en main, si l'écran est
/// réellement alimenté en images.
#[cfg(any(feature = "udev", feature = "winit"))]
#[derive(Debug)]
pub(crate) struct FrameStats {
    output: String,
    since: std::time::Instant,
    started: bool,
    presented: u32,
    idle: u32,
    errors: u32,
    vblanks: u32,
}

#[cfg(any(feature = "udev", feature = "winit"))]
impl FrameStats {
    const PERIOD: Duration = Duration::from_secs(5);

    pub(crate) fn new(output: &str) -> Self {
        FrameStats {
            output: output.to_string(),
            since: std::time::Instant::now(),
            started: false,
            presented: 0,
            idle: 0,
            errors: 0,
            vblanks: 0,
        }
    }

    pub(crate) fn record(&mut self, event: FrameEvent) {
        match event {
            FrameEvent::Presented => {
                if !self.started {
                    self.started = true;
                    info!(sortie = self.output, "première image présentée");
                }
                self.presented += 1;
            }
            FrameEvent::Idle => self.idle += 1,
            FrameEvent::Error => self.errors += 1,
            FrameEvent::VBlank => self.vblanks += 1,
        }
        if self.since.elapsed() >= Self::PERIOD {
            tracing::debug!(
                sortie = self.output,
                presentees = self.presented,
                sans_changement = self.idle,
                erreurs = self.errors,
                vblanks = self.vblanks,
                "rendu sur les 5 dernières secondes"
            );
            (self.presented, self.idle, self.errors, self.vblanks) = (0, 0, 0, 0);
            self.since = std::time::Instant::now();
        }
    }
}

/// Options communes aux backends.
#[derive(Clone, Debug)]
pub struct Options {
    /// La configuration.
    pub config: Config,
    /// Nom imposé de la socket Wayland (`None` : premier `wayland-N` libre).
    pub socket: Option<String>,
    /// Commande lancée au démarrage (option `-s`), exécutée par `/bin/sh -c`.
    pub startup: Option<String>,
    /// Arrêt automatique après ce délai (option `--exit-after`) : garde-fou
    /// pour essayer un backend sans risquer de rester bloqué.
    pub exit_after: Option<Duration>,
}

impl Options {
    /// Options par défaut pour une configuration donnée.
    pub fn new(config: Config) -> Self {
        Options { config, socket: None, startup: None, exit_after: None }
    }
}

/// Crée la boucle d'événements, le `Display` et l'état.
pub fn create(options: &Options) -> Result<(EventLoop<'static, LoopData>, LoopData), Error> {
    let event_loop: EventLoop<'static, LoopData> = EventLoop::try_new()?;
    let mut display: Display<State> = Display::new()?;
    let state = State::new(
        options.config.clone(),
        event_loop.handle(),
        event_loop.get_signal(),
        &mut display,
        options.socket.as_deref(),
    )?;
    Ok((event_loop, LoopData { state, display }))
}

/// Lance la commande de démarrage puis exécute la boucle jusqu'à l'arrêt.
pub fn run_loop(
    mut event_loop: EventLoop<'static, LoopData>,
    mut data: LoopData,
    options: &Options,
) -> Result<(), Error> {
    if let Some(cmd) = &options.startup {
        let argv = ["/bin/sh".to_string(), "-c".to_string(), cmd.clone()];
        crate::spawn::spawn(&argv, &data.state.socket_name);
    }
    if let Some(delay) = options.exit_after {
        event_loop.handle().insert_source(Timer::from_duration(delay), move |_, _, data: &mut LoopData| {
            info!(secondes = delay.as_secs_f64(), "délai --exit-after écoulé : arrêt");
            data.state.quit();
            TimeoutAction::Drop
        })?;
    }
    event_loop.run(None, &mut data, |data| data.state.after_dispatch())?;
    info!("arrêt de rwdwm");
    Ok(())
}
