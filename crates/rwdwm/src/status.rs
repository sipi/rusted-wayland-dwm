//! Texte de statut : chaque ligne lue sur l'entrée standard remplace le
//! texte affiché à droite de la barre (`slstatus -s | rwdwm`).

use std::io::{BufRead, BufReader, IsTerminal, Read};

use smithay::reexports::calloop::channel::{channel, Event};
use smithay::reexports::calloop::LoopHandle;
use tracing::{debug, warn};

use crate::state::LoopData;

/// Démarre la lecture de l'entrée standard, sauf si c'est un terminal.
pub fn watch_stdin(handle: &LoopHandle<'static, LoopData>) {
    let stdin = std::io::stdin();
    if stdin.is_terminal() {
        debug!("entrée standard interactive : pas de texte de statut");
        return;
    }
    watch(stdin, handle);
}

/// Lit `source` ligne à ligne ; chaque ligne devient le texte de statut.
///
/// La lecture bloquante se fait dans un fil dédié ; les lignes arrivent dans
/// la boucle d'événements par un canal. Les octets qui ne sont pas de l'UTF-8
/// valide sont remplacés, sans interrompre la lecture.
pub fn watch(source: impl Read + Send + 'static, handle: &LoopHandle<'static, LoopData>) {
    let (sender, receiver) = channel::<String>();
    let reader = std::thread::Builder::new().name("rwdwm-status".into()).spawn(move || {
        let mut source = BufReader::new(source);
        let mut line = Vec::new();
        loop {
            line.clear();
            match source.read_until(b'\n', &mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    let text = String::from_utf8_lossy(&line);
                    if sender.send(text.trim_end_matches(['\n', '\r']).to_string()).is_err() {
                        break;
                    }
                }
            }
        }
    });
    if let Err(err) = reader {
        warn!(?err, "impossible de lire le texte de statut");
        return;
    }
    let inserted = handle.insert_source(receiver, |event, _, data: &mut LoopData| {
        if let Event::Msg(line) = event {
            data.state.wm.set_status(&line);
        }
    });
    if let Err(err) = inserted {
        warn!(?err, "source de statut refusée");
    }
}
