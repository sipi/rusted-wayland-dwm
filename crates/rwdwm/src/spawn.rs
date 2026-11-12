//! Lancement de processus.

use std::ffi::OsStr;
use std::process::{Command, Stdio};

use tracing::{info, warn};

/// Lance `argv` dans la session Wayland du compositeur.
///
/// Le processus est attendu par un fil dédié : il ne reste jamais à l'état
/// de zombie, et aucun gestionnaire de signal n'est nécessaire.
pub fn spawn(argv: &[String], wayland_display: &OsStr) {
    let Some((program, args)) = argv.split_first() else { return };
    let mut command = Command::new(program);
    command.args(args).env("WAYLAND_DISPLAY", wayland_display).stdin(Stdio::null());
    match command.spawn() {
        Ok(mut child) => {
            info!(?argv, pid = child.id(), "processus lancé");
            let reaper = std::thread::Builder::new().name("rwdwm-wait".into()).spawn(move || {
                let _ = child.wait();
            });
            if let Err(err) = reaper {
                warn!(?err, "impossible de surveiller le processus");
            }
        }
        Err(err) => warn!(?argv, %err, "échec du lancement"),
    }
}
