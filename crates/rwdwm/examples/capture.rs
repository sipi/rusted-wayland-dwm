//! Produit une capture d'écran de rwdwm sans écran ni GPU : compositeur
//! headless, rendu logiciel, et quelques clients Wayland de démonstration.
//!
//! ```sh
//! cargo run -p rwdwm --example capture -- docs/capture-headless.png
//! ```

#[path = "../tests/common/mod.rs"]
mod common;

use common::{Client, WindowOptions, KEY_ALT, KEY_J};
use rwdwm::backend::headless::{HeadlessCommand, HeadlessOptions};

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| "capture-headless.png".into());

    let mut options = HeadlessOptions::new(common::test_config());
    options.outputs = vec![(1280, 720)];
    options.draw_cursor = true;
    let handle = common::start_with(options);
    let mut client = Client::connect(&handle);

    // Trois clients pavés…
    for (app_id, title, color) in [
        ("journal", "journalctl -f", 0x2e3440),
        ("editeur", "vim src/main.rs", 0x3b4252),
        ("terminal", "foot — cargo test", 0x1d2021),
    ] {
        let mut window = WindowOptions::new(app_id, color);
        window.title = title.into();
        client.create_window_with(window);
    }
    // … un dialogue flottant (taille fixe)…
    let mut dialog = WindowOptions::new("dialogue", 0xd08770);
    dialog.title = "Ouvrir un fichier".into();
    dialog.fixed_size = Some((360, 220));
    client.create_window_with(dialog);
    // … et un client sur le tag 9 (règle « firefox »), pour montrer un tag occupé non affiché.
    client.create_window("org.mozilla.firefox", 0x88c0d0);

    handle.send(HeadlessCommand::Status("rwdwm 0.1 | charge 0.42 | jeu. 12 nov. 14:07".into()));
    handle.key_chord(&[KEY_ALT], KEY_J);
    handle.pointer_motion(455.0, 404.0);
    handle.snapshot();
    client.settle();

    let shot = handle.screenshot(0);
    std::fs::write(&path, shot.to_png()).expect("écriture de la capture");
    println!("{path} : {}×{}", shot.width, shot.height);
}
