//! Tests du binaire `rwdwm` : ligne de commande et démarrage réel en mode
//! headless.

mod common;

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_rwdwm");

#[test]
fn version_and_help() {
    let out = Command::new(BIN).arg("--version").output().unwrap();
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), format!("rwdwm {}", env!("CARGO_PKG_VERSION")));

    let out = Command::new(BIN).arg("--help").output().unwrap();
    assert!(out.status.success());
    let help = String::from_utf8_lossy(&out.stdout);
    for needle in ["--backend", "-s <commande>", "--exit-after", "slstatus", "config.rs"] {
        assert!(help.contains(needle), "aide sans « {needle} »");
    }
}

#[test]
fn invalid_arguments_exit_with_usage_error() {
    for args in [&["--backend", "x11"][..], &["--inconnu"], &["-s"], &["--exit-after", "demain"]] {
        let out = Command::new(BIN).args(args).output().unwrap();
        assert_eq!(out.status.code(), Some(2), "arguments {args:?}");
        assert!(String::from_utf8_lossy(&out.stderr).contains("Usage"));
    }
}

#[test]
fn exit_after_stops_the_compositor_cleanly() {
    let dir = common::runtime_dir().join(format!("cli-exit-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let started = Instant::now();
    let output = Command::new(BIN)
        .args(["--backend", "headless", "--exit-after", "0.3", "-v"])
        .env("XDG_RUNTIME_DIR", &dir)
        .env_remove("RUST_LOG")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let log = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "code de sortie {:?} ; journal :\n{log}", output.status.code());
    assert!(started.elapsed() >= Duration::from_millis(300));
    assert!(started.elapsed() < Duration::from_secs(10));
    assert!(log.contains("--exit-after"), "journal :\n{log}");
    // Arrêt propre : la socket et son verrou sont retirés.
    let leftovers: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name()).collect();
    assert!(leftovers.is_empty(), "fichiers restants : {leftovers:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn headless_binary_starts_listens_and_runs_startup_command() {
    // Répertoire d'exécution propre à ce test.
    let dir = common::runtime_dir().join(format!("cli-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let marker = dir.join("marker");
    let mut child = Command::new(BIN)
        .args(["--backend", "headless", "-s"])
        .arg(format!("printf %s \"$WAYLAND_DISPLAY\" > '{}'", marker.display()))
        .env("XDG_RUNTIME_DIR", &dir)
        .env_remove("WAYLAND_DISPLAY")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(10);
    let mut display = String::new();
    while Instant::now() < deadline && display.is_empty() {
        std::thread::sleep(Duration::from_millis(20));
        display = std::fs::read_to_string(&marker).unwrap_or_default();
    }
    let still_running = child.try_wait().unwrap().is_none();
    let _ = child.kill();
    let output = child.wait_with_output().unwrap();
    let log = String::from_utf8_lossy(&output.stderr);

    assert!(display.starts_with("wayland-"), "WAYLAND_DISPLAY = {display:?} ; journal :\n{log}");
    assert!(dir.join(&display).exists(), "socket absente");
    assert!(still_running, "le compositeur s'est arrêté prématurément :\n{log}");
    let _ = std::fs::remove_dir_all(&dir);
}
