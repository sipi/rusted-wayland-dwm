//! Point d'entrée : ligne de commande, journalisation, choix du backend.

#![forbid(unsafe_code)]

use std::process::ExitCode;

use rwdwm::backend::{self, Options};

const USAGE: &str = "\
Usage : rwdwm [options]

Options :
  --backend <auto|udev|winit|headless>
                    Backend à utiliser (défaut : auto).
                      auto     : winit si WAYLAND_DISPLAY ou DISPLAY est
                                 défini, sinon udev ;
                      udev     : natif (DRM/KMS, libinput, libseat) ;
                      winit    : fenêtre dans une session existante ;
                      headless : sortie virtuelle sans affichage (tests).
  -s <commande>     Commande lancée au démarrage (par /bin/sh -c).
  -v                Journalisation détaillée (ou variable RUST_LOG).
  --version         Affiche la version.
  -h, --help        Affiche cette aide.

Le texte de statut est lu sur l'entrée standard, ligne par ligne :
  slstatus -s | rwdwm

La configuration se modifie dans crates/rwdwm/src/config.rs, puis se recompile.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Backend {
    Auto,
    Udev,
    Winit,
    Headless,
}

struct Cli {
    backend: Backend,
    startup: Option<String>,
    verbose: bool,
}

enum Parsed {
    Run(Cli),
    Exit(String),
}

fn parse(args: impl Iterator<Item = String>) -> Result<Parsed, String> {
    let mut cli = Cli { backend: Backend::Auto, startup: None, verbose: false };
    let mut args = args;
    while let Some(arg) = args.next() {
        let (flag, inline) = match arg.split_once('=') {
            Some((flag, value)) if flag.starts_with("--") => (flag.to_string(), Some(value.to_string())),
            _ => (arg.clone(), None),
        };
        let mut value = |name: &str| inline.clone().or_else(|| args.next()).ok_or(format!("{name} attend une valeur"));
        match flag.as_str() {
            "--backend" => {
                cli.backend = match value("--backend")?.as_str() {
                    "auto" => Backend::Auto,
                    "udev" => Backend::Udev,
                    "winit" => Backend::Winit,
                    "headless" => Backend::Headless,
                    other => return Err(format!("backend inconnu : {other}")),
                }
            }
            "-s" => cli.startup = Some(value("-s")?),
            "-v" => cli.verbose = true,
            "--version" => return Ok(Parsed::Exit(format!("rwdwm {}", env!("CARGO_PKG_VERSION")))),
            "-h" | "--help" => return Ok(Parsed::Exit(USAGE.to_string())),
            other => return Err(format!("option inconnue : {other}")),
        }
    }
    Ok(Parsed::Run(cli))
}

fn init_logging(verbose: bool) {
    use tracing_subscriber::EnvFilter;
    let default = if verbose { "rwdwm=debug,smithay=info" } else { "rwdwm=info,smithay=warn" };
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default));
    // Les journaux vont sur la sortie d'erreur : la sortie standard reste
    // disponible pour d'éventuels scripts.
    tracing_subscriber::fmt().with_env_filter(filter).with_writer(std::io::stderr).init();
}

fn run(cli: Cli) -> Result<(), backend::Error> {
    let options = Options { config: rwdwm::config::config(), socket: None, startup: cli.startup };
    let backend = match cli.backend {
        Backend::Auto => {
            let nested = std::env::var_os("WAYLAND_DISPLAY").is_some() || std::env::var_os("DISPLAY").is_some();
            if nested {
                Backend::Winit
            } else {
                Backend::Udev
            }
        }
        other => other,
    };
    tracing::info!(?backend, version = env!("CARGO_PKG_VERSION"), "démarrage de rwdwm");
    match backend {
        #[cfg(feature = "udev")]
        Backend::Udev => backend::udev::run(options),
        #[cfg(feature = "winit")]
        Backend::Winit => backend::winit::run(options),
        #[cfg(feature = "headless")]
        Backend::Headless => {
            let mut headless = backend::headless::HeadlessOptions::new(options.config.clone());
            headless.common = options;
            backend::headless::run(headless)
        }
        #[allow(unreachable_patterns)]
        other => Err(format!("ce binaire a été compilé sans le backend {other:?}").into()),
    }
}

fn main() -> ExitCode {
    let cli = match parse(std::env::args().skip(1)) {
        Ok(Parsed::Run(cli)) => cli,
        Ok(Parsed::Exit(text)) => {
            println!("{text}");
            return ExitCode::SUCCESS;
        }
        Err(message) => {
            eprintln!("rwdwm : {message}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    init_logging(cli.verbose);
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("rwdwm : {err}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cli(args: &[&str]) -> Result<Parsed, String> {
        parse(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn defaults() {
        let Ok(Parsed::Run(c)) = cli(&[]) else { panic!() };
        assert_eq!((c.backend, c.startup, c.verbose), (Backend::Auto, None, false));
    }

    #[test]
    fn all_options() {
        let Ok(Parsed::Run(c)) = cli(&["--backend", "headless", "-s", "foot", "-v"]) else { panic!() };
        assert_eq!((c.backend, c.startup.as_deref(), c.verbose), (Backend::Headless, Some("foot"), true));
        let Ok(Parsed::Run(c)) = cli(&["--backend=winit"]) else { panic!() };
        assert_eq!(c.backend, Backend::Winit);
    }

    #[test]
    fn informational_flags_exit() {
        assert!(matches!(cli(&["--version"]), Ok(Parsed::Exit(v)) if v.starts_with("rwdwm ")));
        assert!(matches!(cli(&["--help"]), Ok(Parsed::Exit(v)) if v.contains("--backend")));
    }

    #[test]
    fn errors() {
        assert!(cli(&["--backend", "x11"]).is_err());
        assert!(cli(&["--backend"]).is_err());
        assert!(cli(&["-s"]).is_err());
        assert!(cli(&["--nope"]).is_err());
    }
}
