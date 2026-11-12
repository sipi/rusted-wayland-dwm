//! Backends : `udev` (natif, DRM/KMS), `winit` (imbriqué dans une session
//! existante) et `headless` (sortie virtuelle, pour les tests).

use rwdwm_core::Config;
use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::reexports::calloop::EventLoop;
use smithay::reexports::wayland_server::Display;

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

/// Options communes aux backends.
#[derive(Clone, Debug)]
pub struct Options {
    /// La configuration.
    pub config: Config,
    /// Nom imposé de la socket Wayland (`None` : premier `wayland-N` libre).
    pub socket: Option<String>,
    /// Commande lancée au démarrage (option `-s`), exécutée par `/bin/sh -c`.
    pub startup: Option<String>,
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
    event_loop.run(None, &mut data, |data| data.state.after_dispatch())?;
    Ok(())
}
