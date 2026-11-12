//! Tests d'intégration : un vrai compositeur (backend headless, rendu
//! logiciel Pixman) et de vrais clients Wayland dans le même processus.
//!
//! Chaque test démarre son propre compositeur sur sa propre socket ; ils
//! peuvent donc s'exécuter en parallèle.

mod common;

use std::time::{Duration, Instant};

use common::*;
use rwdwm::backend::headless::{Handle, HeadlessCommand, HeadlessOptions};
use rwdwm::core::action::Action;
use rwdwm::core::config::{Key, Mods, Trigger};
use rwdwm::core::keysyms::sym;
use rwdwm::core::model::{Client as WmClient, Wm};
use rwdwm::core::Rect;

// Couleurs de la configuration par défaut.
const ROOT: u32 = 0x222222;
const BORDER_NORM: u32 = 0x444444;
const BORDER_SEL: u32 = 0x005577;
const BORDER_URGENT: u32 = 0xff0000;
const BAR_FG: u32 = 0xbbbbbb;

// Couleurs des fenêtres de test (distinctes de celles du compositeur).
const RED: u32 = 0xc03030;
const GREEN: u32 = 0x30c030;
const BLUE: u32 = 0x3030c0;
const YELLOW: u32 = 0xc0c030;

/// Attend que le compositeur ait traité les commandes envoyées et que le
/// client ait répondu aux configurations, puis renvoie le modèle.
fn sync(handle: &Handle, client: &mut Client) -> Wm {
    handle.snapshot();
    client.settle();
    let wm = handle.snapshot();
    wm.check_invariants().expect("invariants du modèle");
    wm
}

fn by_app<'a>(wm: &'a Wm, app_id: &str) -> &'a WmClient {
    wm.client_ids()
        .filter_map(|id| wm.client(id))
        .find(|c| c.app_id == app_id)
        .unwrap_or_else(|| panic!("client {app_id} introuvable"))
}

fn selected_app(wm: &Wm) -> Option<String> {
    wm.selected().and_then(|id| wm.client(id)).map(|c| c.app_id.clone())
}

fn center(r: Rect) -> (i32, i32) {
    (r.x + r.w / 2, r.y + r.h / 2)
}

/// Attend qu'une condition devienne vraie (au plus 5 s).
fn wait_until(mut condition: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if condition() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    condition()
}

// ------------------------------------------------------------- protocoles

#[test]
fn advertises_expected_globals() {
    let handle = start(test_config());
    let client = Client::connect(&handle);
    for global in [
        "wl_compositor",
        "wl_subcompositor",
        "wl_shm",
        "wl_seat",
        "wl_output",
        "wl_data_device_manager",
        "xdg_wm_base",
        "zxdg_output_manager_v1",
        "zxdg_decoration_manager_v1",
        "zwlr_layer_shell_v1",
        "xdg_activation_v1",
        "wp_viewporter",
        "wp_cursor_shape_manager_v1",
        "zwp_primary_selection_device_manager_v1",
        "wp_single_pixel_buffer_manager_v1",
    ] {
        assert!(client.state.globals.iter().any(|g| g == global), "global manquant : {global}");
    }
}

// --------------------------------------------------------------- pavage

#[test]
fn single_window_fills_the_window_area() {
    let handle = start(test_config());
    let mut client = Client::connect(&handle);
    let w = client.create_window("un", RED);
    let wm = sync(&handle, &mut client);

    let mon = wm.selected_monitor().unwrap();
    assert_eq!(mon.screen, Rect::new(0, 0, 1280, 720));
    assert_eq!(wm.bar_rect(mon.id), Some(Rect::new(0, 0, 1280, 18)));
    assert_eq!(mon.window_area, Rect::new(0, 18, 1280, 702));

    let c = by_app(&wm, "un");
    assert_eq!(c.geom, mon.window_area);
    assert_eq!(c.inner(), Rect::new(1, 19, 1278, 700));
    // Le client a été configuré à la taille intérieure et s'y est conformé.
    let window = &client.state.windows[w];
    assert_eq!(window.size, (1278, 700));
    assert!(window.last_configure().has(STATE_ACTIVATED));
    assert!(window.last_configure().has(STATE_TILED_LEFT), "les clients pavés sont déclarés « tiled »");
    assert_eq!(client.focused_window(), Some(w));

    let shot = handle.screenshot(0);
    assert_eq!(shot.pixel(640, 400), RED);
    assert_eq!(shot.pixel(0, 18), BORDER_SEL, "bordure du client sélectionné");
    assert_eq!(shot.pixel(1279, 719), BORDER_SEL);
    assert_eq!(shot.pixel(1, 19), RED);
    assert_eq!(shot.count(c.inner(), RED), (1278 * 700) as usize);
}

#[test]
fn tile_layout_matches_dwm() {
    let handle = start(test_config());
    let mut client = Client::connect(&handle);
    let a = client.create_window("a", RED);
    let b = client.create_window("b", GREEN);
    let c = client.create_window("c", BLUE);
    let wm = sync(&handle, &mut client);

    // dwm : le dernier arrivé prend la zone maître et le focus.
    let mon = wm.selected_monitor().unwrap();
    let order: Vec<&str> = mon.clients.iter().map(|id| wm.client(*id).unwrap().app_id.as_str()).collect();
    assert_eq!(order, ["c", "b", "a"]);
    assert_eq!(selected_app(&wm).as_deref(), Some("c"));
    assert_eq!(client.focused_window(), Some(c));

    // mfact 0.55 : maître 704 px, pile 576 px partagée en hauteur.
    assert_eq!(by_app(&wm, "c").geom, Rect::new(0, 18, 704, 702));
    assert_eq!(by_app(&wm, "b").geom, Rect::new(704, 18, 576, 351));
    assert_eq!(by_app(&wm, "a").geom, Rect::new(704, 369, 576, 351));
    assert_eq!(client.state.windows[c].size, (702, 700));
    assert_eq!(client.state.windows[b].size, (574, 349));
    assert_eq!(client.state.windows[a].size, (574, 349));
    assert!(client.state.windows[c].last_configure().has(STATE_ACTIVATED));
    assert!(!client.state.windows[a].last_configure().has(STATE_ACTIVATED));

    let shot = handle.screenshot(0);
    assert_eq!(shot.pixel(350, 360), BLUE);
    assert_eq!(shot.pixel(990, 190), GREEN);
    assert_eq!(shot.pixel(990, 540), RED);
    assert_eq!(shot.pixel(0, 300), BORDER_SEL);
    assert_eq!(shot.pixel(1279, 100), BORDER_NORM);
    assert_eq!(shot.pixel(704, 500), BORDER_NORM);
}

#[test]
fn master_area_keys() {
    let handle = start(test_config());
    let mut client = Client::connect(&handle);
    client.create_window("a", RED);
    client.create_window("b", GREEN);
    client.create_window("c", BLUE);

    // Mod+l / Mod+h : facteur maître.
    handle.key_chord(&[KEY_ALT], KEY_L);
    let wm = sync(&handle, &mut client);
    assert_eq!(by_app(&wm, "c").geom.w, 768); // 1280 × 0.60
    handle.key_chord(&[KEY_ALT], KEY_H);
    handle.key_chord(&[KEY_ALT], KEY_H);
    let wm = sync(&handle, &mut client);
    assert_eq!(by_app(&wm, "c").geom.w, 640); // 1280 × 0.50

    // Mod+i : deux clients en zone maître.
    handle.key_chord(&[KEY_ALT], KEY_I);
    let wm = sync(&handle, &mut client);
    assert_eq!(by_app(&wm, "c").geom, Rect::new(0, 18, 640, 351));
    assert_eq!(by_app(&wm, "b").geom, Rect::new(0, 369, 640, 351));
    assert_eq!(by_app(&wm, "a").geom, Rect::new(640, 18, 640, 702));
    // Mod+d : retour à un seul.
    handle.key_chord(&[KEY_ALT], KEY_D);
    let wm = sync(&handle, &mut client);
    assert_eq!(by_app(&wm, "c").geom, Rect::new(0, 18, 640, 702));

    // Mod+j puis Mod+Entrée : le client suivant devient maître.
    handle.key_chord(&[KEY_ALT], KEY_J);
    let wm = sync(&handle, &mut client);
    assert_eq!(selected_app(&wm).as_deref(), Some("b"));
    handle.key_chord(&[KEY_ALT], KEY_RETURN);
    let wm = sync(&handle, &mut client);
    assert_eq!(by_app(&wm, "b").geom, Rect::new(0, 18, 640, 702));
    assert_eq!(selected_app(&wm).as_deref(), Some("b"));
    let shot = handle.screenshot(0);
    assert_eq!(shot.pixel(320, 360), GREEN);
}

#[test]
fn layouts_monocle_and_floating() {
    let handle = start(test_config());
    let mut client = Client::connect(&handle);
    let a = client.create_window("a", RED);
    let b = client.create_window("b", GREEN);

    // Mod+m : monocle — chaque client occupe toute la zone.
    handle.key_chord(&[KEY_ALT], KEY_M);
    let wm = sync(&handle, &mut client);
    let mon = wm.selected_monitor().unwrap();
    assert_eq!(wm.layout_symbol(mon.id), "[2]");
    assert_eq!(by_app(&wm, "a").geom, mon.window_area);
    assert_eq!(by_app(&wm, "b").geom, mon.window_area);
    assert_eq!(client.state.windows[a].size, (1278, 700));
    let shot = handle.screenshot(0);
    assert_eq!(shot.pixel(640, 400), GREEN, "le client sélectionné est au-dessus");
    handle.key_chord(&[KEY_ALT], KEY_J);
    sync(&handle, &mut client);
    assert_eq!(handle.screenshot(0).pixel(640, 400), RED);
    assert_eq!(client.focused_window(), Some(a));

    // Mod+f : flottant — les géométries ne sont plus imposées.
    handle.key_chord(&[KEY_ALT], KEY_F);
    let wm = sync(&handle, &mut client);
    assert_eq!(wm.layout_symbol(wm.selected_monitor().unwrap().id), "><>");
    assert_eq!(by_app(&wm, "a").geom, mon.window_area);

    // Mod+Espace : retour à la disposition précédente ; Mod+t : pavage.
    handle.key_chord(&[KEY_ALT], KEY_SPACE);
    let wm = sync(&handle, &mut client);
    assert_eq!(wm.layout_symbol(wm.selected_monitor().unwrap().id), "[2]");
    handle.key_chord(&[KEY_ALT], KEY_T);
    let wm = sync(&handle, &mut client);
    assert_eq!(wm.layout_symbol(wm.selected_monitor().unwrap().id), "[]=");
    assert_eq!(by_app(&wm, "b").geom, Rect::new(0, 18, 704, 702));
    assert_eq!(client.state.windows[b].size, (702, 700));
}

#[test]
fn toggle_floating_restores_natural_size() {
    let handle = start(test_config());
    let mut client = Client::connect(&handle);
    let a = client.create_window("a", RED);
    client.create_window("b", GREEN);
    handle.key_chord(&[KEY_ALT], KEY_J); // sélectionne « a »
    handle.key_chord(&[KEY_ALT, KEY_SHIFT], KEY_SPACE);
    let wm = sync(&handle, &mut client);
    let c = by_app(&wm, "a");
    assert!(c.is_floating);
    // « b » récupère toute la zone, « a » flotte au-dessus.
    assert_eq!(by_app(&wm, "b").geom, Rect::new(0, 18, 1280, 702));
    let shot = handle.screenshot(0);
    assert_eq!(shot.pixel(c.geom.x + c.geom.w / 2, c.geom.y + c.geom.h / 2), RED);
    assert_eq!(shot.pixel(c.geom.x, c.geom.y), BORDER_SEL);
    assert_eq!(client.state.windows[a].size, (c.inner().w, c.inner().h));
    // Choix assumé (comme dwl) : même flottant, le client reste déclaré
    // « tiled » pour qu'il ne dessine ni ombre ni coins arrondis.
    assert!(client.state.windows[a].last_configure().has(STATE_TILED_LEFT));

    handle.key_chord(&[KEY_ALT, KEY_SHIFT], KEY_SPACE);
    let wm = sync(&handle, &mut client);
    assert!(!by_app(&wm, "a").is_floating);
    assert_eq!(by_app(&wm, "a").geom.w + by_app(&wm, "b").geom.w, 1280);
}

#[test]
fn toggle_bar() {
    let handle = start(test_config());
    let mut client = Client::connect(&handle);
    let a = client.create_window("a", RED);
    handle.key_chord(&[KEY_ALT], KEY_B);
    let wm = sync(&handle, &mut client);
    let mon = wm.selected_monitor().unwrap();
    assert!(!mon.show_bar);
    assert_eq!(by_app(&wm, "a").geom, Rect::new(0, 0, 1280, 720));
    assert_eq!(client.state.windows[a].size, (1278, 718));
    let shot = handle.screenshot(0);
    assert_eq!(shot.pixel(0, 0), BORDER_SEL);
    assert_eq!(shot.pixel(640, 5), RED);

    handle.key_chord(&[KEY_ALT], KEY_B);
    let wm = sync(&handle, &mut client);
    assert_eq!(by_app(&wm, "a").geom, Rect::new(0, 18, 1280, 702));
    assert_ne!(handle.screenshot(0).pixel(640, 5), RED);
}

// ----------------------------------------------------------------- tags

#[test]
fn tags_view_tag_and_toggle() {
    let handle = start(test_config());
    let mut client = Client::connect(&handle);
    let a = client.create_window("a", RED);
    let b = client.create_window("b", GREEN);

    // Mod+Maj+2 : envoie « b » sur le tag 2 ; il disparaît.
    handle.key_chord(&[KEY_ALT, KEY_SHIFT], KEY_2);
    let wm = sync(&handle, &mut client);
    assert_eq!(by_app(&wm, "b").tags, 0b10);
    assert!(!wm.is_visible(by_app(&wm, "b").id));
    assert_eq!(by_app(&wm, "a").geom, Rect::new(0, 18, 1280, 702));
    assert_eq!(client.focused_window(), Some(a));
    let shot = handle.screenshot(0);
    assert_eq!(shot.count(Rect::new(0, 18, 1280, 702), GREEN), 0);
    assert_eq!(shot.pixel(640, 400), RED);

    // Mod+2 : affiche le tag 2.
    handle.key_chord(&[KEY_ALT], KEY_2);
    let wm = sync(&handle, &mut client);
    assert_eq!(wm.selected_monitor().unwrap().current_tagset(), 0b10);
    assert_eq!(client.focused_window(), Some(b));
    let shot = handle.screenshot(0);
    assert_eq!(shot.count(Rect::new(0, 18, 1280, 702), RED), 0);
    assert_eq!(shot.pixel(640, 400), GREEN);

    // Mod+Tab : tagset précédent.
    handle.key_chord(&[KEY_ALT], KEY_TAB);
    let wm = sync(&handle, &mut client);
    assert_eq!(wm.selected_monitor().unwrap().current_tagset(), 0b01);
    assert_eq!(client.focused_window(), Some(a));

    // Mod+Ctrl+2 : affiche aussi le tag 2 ; les deux clients sont pavés.
    handle.key_chord(&[KEY_ALT, KEY_CTRL], KEY_2);
    let wm = sync(&handle, &mut client);
    assert_eq!(wm.selected_monitor().unwrap().current_tagset(), 0b11);
    assert!(wm.is_visible(by_app(&wm, "a").id) && wm.is_visible(by_app(&wm, "b").id));
    let shot = handle.screenshot(0);
    assert!(shot.count(Rect::new(0, 18, 1280, 702), RED) > 0);
    assert!(shot.count(Rect::new(0, 18, 1280, 702), GREEN) > 0);

    // Mod+Ctrl+Maj+3 : ajoute le tag 3 au client sélectionné.
    let before = wm.client(wm.selected().unwrap()).unwrap().tags;
    handle.key_chord(&[KEY_ALT, KEY_CTRL, KEY_SHIFT], KEY_3);
    let wm = sync(&handle, &mut client);
    assert_eq!(wm.client(wm.selected().unwrap()).unwrap().tags, before | 0b100);

    // Mod+0 : tous les tags ; Mod+Maj+0 : client sur tous les tags.
    handle.key_chord(&[KEY_ALT], KEY_0);
    let wm = sync(&handle, &mut client);
    assert_eq!(wm.selected_monitor().unwrap().current_tagset(), 0x1ff);
    handle.key_chord(&[KEY_ALT, KEY_SHIFT], KEY_0);
    let wm = sync(&handle, &mut client);
    assert_eq!(wm.client(wm.selected().unwrap()).unwrap().tags, 0x1ff);
}

#[test]
fn tag_keys_follow_physical_position_on_azerty() {
    // Sur AZERTY la touche « 2 » produit « é » sans Maj : le raccourci est
    // lié au code de touche, pas au symbole.
    let mut config = test_config();
    config.keyboard.layout = "fr";
    let handle = start(config);
    let mut client = Client::connect(&handle);
    client.create_window("a", RED);
    handle.key_chord(&[KEY_ALT], KEY_2);
    let wm = sync(&handle, &mut client);
    assert_eq!(wm.selected_monitor().unwrap().current_tagset(), 0b10);
    // Les raccourcis par symbole suivent la disposition : « q » est sur la
    // touche physique « a » en AZERTY, « m » sur la touche « ; ».
    handle.key_chord(&[KEY_ALT], KEY_SEMICOLON);
    let wm = sync(&handle, &mut client);
    // (« [M] » et non « [0] » : comme dwm, le compteur n'apparaît qu'avec
    // au moins un client visible, et le tag 2 est vide.)
    assert_eq!(wm.layout_symbol(wm.selected_monitor().unwrap().id), "[M]");
}

// ---------------------------------------------------------------- focus

#[test]
fn focus_stack_cycles_and_updates_keyboard_focus() {
    let handle = start(test_config());
    let mut client = Client::connect(&handle);
    let a = client.create_window("a", RED);
    let b = client.create_window("b", GREEN);
    let c = client.create_window("c", BLUE);
    assert_eq!(client.focused_window(), Some(c));

    for expected in [b, a, c] {
        handle.key_chord(&[KEY_ALT], KEY_J);
        sync(&handle, &mut client);
        assert_eq!(client.focused_window(), Some(expected));
    }
    for expected in [a, b, c] {
        handle.key_chord(&[KEY_ALT], KEY_K);
        sync(&handle, &mut client);
        assert_eq!(client.focused_window(), Some(expected));
    }
    // La bordure suit la sélection.
    handle.key_chord(&[KEY_ALT], KEY_J);
    let wm = sync(&handle, &mut client);
    let shot = handle.screenshot(0);
    let (gb, gc) = (by_app(&wm, "b").geom, by_app(&wm, "c").geom);
    assert_eq!(shot.pixel(gb.x, gb.y + 10), BORDER_SEL);
    assert_eq!(shot.pixel(gc.x, gc.y + 10), BORDER_NORM);
    assert!(client.state.windows[b].last_configure().has(STATE_ACTIVATED));
    assert!(!client.state.windows[c].last_configure().has(STATE_ACTIVATED));
}

#[test]
fn sloppy_focus_follows_the_pointer() {
    let handle = start(test_config());
    let mut client = Client::connect(&handle);
    let a = client.create_window("a", RED);
    let b = client.create_window("b", GREEN);
    let wm = sync(&handle, &mut client);
    assert_eq!(client.focused_window(), Some(b));

    let (x, y) = center(by_app(&wm, "a").geom);
    handle.pointer_motion(x as f64, y as f64);
    let wm = sync(&handle, &mut client);
    assert_eq!(selected_app(&wm).as_deref(), Some("a"));
    assert_eq!(client.focused_window(), Some(a));
    assert_eq!(client.hovered_window(), Some(a));
    // Coordonnées locales à la surface (origine = intérieur de la bordure).
    let inner = by_app(&wm, "a").inner();
    assert_eq!(client.state.pointer_pos, ((x - inner.x) as f64, (y - inner.y) as f64));

    // Bouger à l'intérieur du même client ne change rien (une sélection
    // faite au clavier n'est pas annulée par un petit mouvement).
    handle.key_chord(&[KEY_ALT], KEY_J);
    handle.pointer_motion(x as f64 + 3.0, y as f64 + 3.0);
    let wm = sync(&handle, &mut client);
    assert_eq!(selected_app(&wm).as_deref(), Some("b"));
}

#[test]
fn click_to_focus_without_sloppy_focus() {
    let mut config = test_config();
    config.sloppy_focus = false;
    let handle = start(config);
    let mut client = Client::connect(&handle);
    let a = client.create_window("a", RED);
    let b = client.create_window("b", GREEN);
    let wm = sync(&handle, &mut client);

    let (x, y) = center(by_app(&wm, "a").geom);
    handle.pointer_motion(x as f64, y as f64);
    let wm = sync(&handle, &mut client);
    assert_eq!(selected_app(&wm).as_deref(), Some("b"), "le survol ne suffit pas");
    assert_eq!(client.focused_window(), Some(b));
    assert_eq!(client.hovered_window(), Some(a), "mais le pointeur est bien transmis");

    handle.click(BTN_LEFT);
    let wm = sync(&handle, &mut client);
    assert_eq!(selected_app(&wm).as_deref(), Some("a"));
    assert_eq!(client.focused_window(), Some(a));
    // Le clic est aussi transmis au client (comme dwm, qui rejoue le clic).
    assert_eq!(client.state.buttons, [(BTN_LEFT, true), (BTN_LEFT, false)]);
}

// -------------------------------------------------------------- clavier

#[test]
fn bound_keys_are_intercepted_others_are_forwarded() {
    let handle = start(test_config());
    let mut client = Client::connect(&handle);
    client.create_window("a", RED);
    client.create_window("b", GREEN);

    // Touche ordinaire : transmise (codes evdev = keycode XKB − 8).
    handle.key_chord(&[], KEY_A);
    sync(&handle, &mut client);
    assert_eq!(client.state.keys, [(30, true), (30, false)]);

    // Raccourci : le modificateur est transmis, pas la touche liée —
    // ni son appui, ni son relâchement.
    client.state.keys.clear();
    handle.key_chord(&[KEY_ALT], KEY_J);
    sync(&handle, &mut client);
    assert!(!client.state.keys.iter().any(|&(key, _)| key == 36), "touches reçues : {:?}", client.state.keys);

    // Mod+a n'est pas lié : transmis tel quel.
    client.state.keys.clear();
    handle.key_chord(&[KEY_ALT], KEY_A);
    sync(&handle, &mut client);
    assert!(client.state.keys.contains(&(30, true)) && client.state.keys.contains(&(30, false)));
}

#[test]
fn key_repeat_repeats_compositor_bindings() {
    let mut config = test_config();
    config.keyboard.repeat_delay = 100;
    config.keyboard.repeat_rate = 50;
    let handle = start(config);
    let mut client = Client::connect(&handle);
    client.create_window("a", RED);

    handle.send(HeadlessCommand::Key { keycode: KEY_ALT, pressed: true });
    handle.send(HeadlessCommand::Key { keycode: KEY_H, pressed: true });
    // 0,55 → 0,50 à l'appui, puis −0,05 toutes les 20 ms après 100 ms.
    assert!(wait_until(|| handle.snapshot().selected_monitor().unwrap().mfact < 0.36));
    handle.send(HeadlessCommand::Key { keycode: KEY_H, pressed: false });
    handle.send(HeadlessCommand::Key { keycode: KEY_ALT, pressed: false });
    let after_release = sync(&handle, &mut client).selected_monitor().unwrap().mfact;
    std::thread::sleep(Duration::from_millis(250));
    let later = handle.snapshot().selected_monitor().unwrap().mfact;
    assert_eq!(after_release, later, "la répétition s'arrête au relâchement");
}

#[test]
fn kill_client_asks_the_client_to_close() {
    let handle = start(test_config());
    let mut client = Client::connect(&handle);
    let a = client.create_window("a", RED);
    let b = client.create_window("b", GREEN);
    handle.key_chord(&[KEY_ALT, KEY_SHIFT], KEY_C);
    sync(&handle, &mut client);
    assert!(client.state.windows[b].close_requested);
    assert!(!client.state.windows[a].close_requested);

    // Le client obtempère : la fenêtre disparaît, le focus revient à « a ».
    client.destroy_window(b);
    let wm = sync(&handle, &mut client);
    assert_eq!(wm.client_ids().count(), 1);
    assert_eq!(by_app(&wm, "a").geom, Rect::new(0, 18, 1280, 702));
    assert_eq!(client.focused_window(), Some(a));
    assert_eq!(client.state.windows[a].size, (1278, 700));
}

#[test]
fn spawn_runs_commands_with_wayland_display() {
    let dir = runtime_dir();
    let marker = dir.join(format!("spawn-marker-{}", std::process::id()));
    let _ = std::fs::remove_file(&marker);
    let mut config = test_config();
    config.keys.push(Key {
        mods: Mods::ALT,
        trigger: Trigger::Sym(sym('a')),
        action: Action::shcmd(&format!("printf %s \"$WAYLAND_DISPLAY\" > '{}'", marker.display())),
    });
    let handle = start(config);
    handle.key_chord(&[KEY_ALT], KEY_A);
    handle.snapshot();
    assert!(wait_until(|| std::fs::read_to_string(&marker).is_ok_and(|s| !s.is_empty())), "commande non exécutée");
    assert_eq!(std::fs::read_to_string(&marker).unwrap(), handle.socket_name());
    let _ = std::fs::remove_file(&marker);
}

#[test]
fn startup_command_runs() {
    let dir = runtime_dir();
    let marker = dir.join(format!("startup-marker-{}", std::process::id()));
    let _ = std::fs::remove_file(&marker);
    let mut options = HeadlessOptions::new(test_config());
    options.common.startup = Some(format!("touch '{}'", marker.display()));
    let _handle = start_with(options);
    assert!(wait_until(|| marker.exists()));
    let _ = std::fs::remove_file(&marker);
}

#[test]
fn quit_binding_stops_the_compositor() {
    let handle = start(test_config());
    let mut client = Client::connect(&handle);
    client.create_window("a", RED);
    assert!(!handle.is_finished());
    handle.key_chord(&[KEY_ALT, KEY_SHIFT], KEY_Q);
    assert!(wait_until(|| handle.is_finished()));
}

// --------------------------------------------------------- plein écran

#[test]
fn fullscreen_toggle_covers_the_whole_output() {
    let handle = start(test_config());
    let mut client = Client::connect(&handle);
    let a = client.create_window("a", RED);
    let b = client.create_window("b", GREEN);

    handle.key_chord(&[KEY_ALT], KEY_E);
    let wm = sync(&handle, &mut client);
    let c = by_app(&wm, "b");
    assert!(c.is_fullscreen);
    assert_eq!(c.inner(), Rect::new(0, 0, 1280, 720));
    let configure = client.state.windows[b].last_configure();
    assert!(configure.has(STATE_FULLSCREEN));
    assert_eq!((configure.width, configure.height), (1280, 720));
    let shot = handle.screenshot(0);
    assert_eq!(shot.count(Rect::new(0, 0, 1280, 720), GREEN), 1280 * 720, "ni barre, ni bordure, ni voisin");

    handle.key_chord(&[KEY_ALT], KEY_E);
    let wm = sync(&handle, &mut client);
    assert!(!by_app(&wm, "b").is_fullscreen);
    assert_eq!(by_app(&wm, "b").geom, Rect::new(0, 18, 704, 702));
    assert!(!client.state.windows[b].last_configure().has(STATE_FULLSCREEN));
    assert_eq!(client.state.windows[b].size, (702, 700));
    assert_eq!(client.state.windows[a].size, (574, 700));
    let shot = handle.screenshot(0);
    assert!(shot.count(Rect::new(0, 18, 1280, 702), RED) > 0);
}

#[test]
fn client_requested_fullscreen() {
    let handle = start(test_config());
    let mut client = Client::connect(&handle);
    let mut options = WindowOptions::new("film", BLUE);
    options.fullscreen = true;
    let w = client.create_window_with(options);
    let wm = sync(&handle, &mut client);
    assert!(by_app(&wm, "film").is_fullscreen);
    assert_eq!(client.state.windows[w].size, (1280, 720));
    assert_eq!(handle.screenshot(0).count(Rect::new(0, 0, 1280, 720), BLUE), 1280 * 720);

    client.state.windows[w].toplevel.unset_fullscreen();
    let wm = sync(&handle, &mut client);
    assert!(!by_app(&wm, "film").is_fullscreen);
    assert_eq!(client.state.windows[w].size, (1278, 700));
}

// ------------------------------------------------- flottants et règles

#[test]
fn fixed_size_and_transient_windows_float() {
    let handle = start(test_config());
    let mut client = Client::connect(&handle);
    let main = client.create_window("main", RED);

    let mut options = WindowOptions::new("dialogue", GREEN);
    options.fixed_size = Some((300, 200));
    client.create_window_with(options);
    let wm = sync(&handle, &mut client);
    let dialog = by_app(&wm, "dialogue");
    assert!(dialog.is_floating && dialog.is_fixed);
    assert_eq!((dialog.inner().w, dialog.inner().h), (300, 200));
    // Centré dans la zone des fenêtres.
    let area = wm.selected_monitor().unwrap().window_area;
    assert_eq!(center(dialog.geom), center(area));
    // Le client pavé garde toute la zone ; le flottant est dessiné dessus.
    assert_eq!(by_app(&wm, "main").geom, area);
    let shot = handle.screenshot(0);
    assert_eq!(shot.count(dialog.inner(), GREEN), 300 * 200);
    assert_eq!(shot.pixel(100, 100), RED);

    let mut options = WindowOptions::new("enfant", BLUE);
    options.parent = Some(main);
    options.natural_size = (400, 300);
    client.create_window_with(options);
    let wm = sync(&handle, &mut client);
    let child = by_app(&wm, "enfant");
    assert!(child.is_floating, "une fenêtre transitoire flotte");
    assert_eq!((child.inner().w, child.inner().h), (400, 300));
    assert_eq!(child.tags, by_app(&wm, "main").tags);
}

#[test]
fn rules_apply_by_app_id() {
    let handle = start(test_config());
    let mut client = Client::connect(&handle);
    let a = client.create_window("a", RED);
    client.create_window("Gimp", GREEN);
    let wm = sync(&handle, &mut client);
    assert!(by_app(&wm, "Gimp").is_floating);
    assert_eq!(by_app(&wm, "Gimp").tags, 1);

    // firefox va sur le tag 9 : invisible, et il ne prend pas le focus.
    client.create_window("org.mozilla.firefox", BLUE);
    let wm = sync(&handle, &mut client);
    let firefox = by_app(&wm, "org.mozilla.firefox");
    assert_eq!(firefox.tags, 1 << 8);
    assert!(!wm.is_visible(firefox.id));
    assert_eq!(handle.screenshot(0).count(Rect::new(0, 0, 1280, 720), BLUE), 0);
    assert_ne!(client.focused_window(), None);
    let _ = a;
}

#[test]
fn mouse_move_and_resize() {
    let handle = start(test_config());
    let mut client = Client::connect(&handle);
    let a = client.create_window("a", RED);
    client.create_window("b", GREEN);
    let wm = sync(&handle, &mut client);
    let start = by_app(&wm, "a").geom; // (704, 18, 576, 702)

    // Mod + bouton gauche : déplacement ; le client devient flottant.
    handle.pointer_motion(900.0, 300.0);
    handle.send(HeadlessCommand::Key { keycode: KEY_ALT, pressed: true });
    handle.button(BTN_LEFT, true);
    handle.pointer_motion(800.0, 360.0);
    let wm = sync(&handle, &mut client);
    let moved = by_app(&wm, "a");
    assert!(moved.is_floating);
    assert_eq!((moved.geom.x, moved.geom.y), (start.x - 100, start.y + 60));
    handle.button(BTN_LEFT, false);
    handle.send(HeadlessCommand::Key { keycode: KEY_ALT, pressed: false });
    let wm = sync(&handle, &mut client);
    let moved = by_app(&wm, "a").geom;
    assert_eq!((moved.x, moved.y), (start.x - 100, start.y + 60));
    // Ni l'appui ni le relâchement du bouton ne parviennent au client.
    assert!(client.state.buttons.is_empty(), "boutons reçus : {:?}", client.state.buttons);
    assert_eq!(by_app(&wm, "b").geom, Rect::new(0, 18, 1280, 702));

    // Mod + bouton droit : redimensionnement depuis le coin bas-droit.
    handle.pointer_motion((moved.x + 50) as f64, (moved.y + 50) as f64);
    handle.send(HeadlessCommand::Key { keycode: KEY_ALT, pressed: true });
    handle.button(BTN_RIGHT, true);
    handle.pointer_motion((moved.x + 50 - 120) as f64, (moved.y + 50 - 100) as f64);
    handle.button(BTN_RIGHT, false);
    handle.send(HeadlessCommand::Key { keycode: KEY_ALT, pressed: false });
    let wm = sync(&handle, &mut client);
    let resized = by_app(&wm, "a");
    assert_eq!((resized.geom.x, resized.geom.y), (moved.x, moved.y));
    assert_eq!((resized.geom.w, resized.geom.h), (moved.w - 120, moved.h - 100));
    assert_eq!(client.state.windows[a].size, (resized.inner().w, resized.inner().h));
    let shot = handle.screenshot(0);
    assert_eq!(shot.count(resized.inner(), RED), (resized.inner().w * resized.inner().h) as usize);
}

// ---------------------------------------------------------------- barre

#[test]
fn bar_is_drawn_and_reflects_state() {
    let handle = start(test_config());
    let mut client = Client::connect(&handle);

    // Sans client : tag 1 sélectionné (fond cyan), les autres normaux.
    let shot = handle.screenshot(0);
    assert_eq!(shot.pixel(22, 16), BORDER_SEL);
    assert_eq!(shot.pixel(24 + 22, 16), ROOT);
    assert!(shot.count(Rect::new(24, 0, 24, 18), BAR_FG) > 0, "libellé du tag 2");
    assert_eq!(shot.count(Rect::new(0, 18, 1280, 702), ROOT), 1280 * 702, "fond uni sous la barre");

    // Statut : dessiné à droite.
    handle.send(HeadlessCommand::Status("charge 0.42".into()));
    let wm = handle.snapshot();
    assert_eq!(wm.status(), "charge 0.42");
    let view = wm.bar_view(wm.selected_monitor().unwrap().id).unwrap();
    assert_eq!(view.status.as_deref(), Some("charge 0.42"));
    let before = shot.count(Rect::new(1100, 0, 180, 18), BAR_FG);
    let shot = handle.screenshot(0);
    assert!(shot.count(Rect::new(1100, 0, 180, 18), BAR_FG) > before);

    // Avec un client : le titre apparaît sur fond « sélectionné ».
    client.create_window("mon-titre", RED);
    let wm = sync(&handle, &mut client);
    let view = wm.bar_view(wm.selected_monitor().unwrap().id).unwrap();
    assert_eq!(view.title.as_deref(), Some("mon-titre"));
    assert!(view.tags[0].occupied && view.tags[0].filled && !view.tags[1].occupied);
    let shot = handle.screenshot(0);
    assert_eq!(shot.pixel(600, 16), BORDER_SEL);

    // Changement de titre.
    client.state.windows[0].toplevel.set_title("autre titre".into());
    client.state.windows[0].surface.commit();
    let wm = sync(&handle, &mut client);
    assert_eq!(by_app(&wm, "mon-titre").title, "autre titre");
}

#[test]
fn bar_clicks() {
    let handle = start(test_config());
    let mut client = Client::connect(&handle);
    client.create_window("a", RED);

    // Chaque tag occupe 24 px (8 px de glyphe + 16 px de marge).
    handle.pointer_motion(36.0, 8.0);
    handle.click(BTN_LEFT);
    let wm = sync(&handle, &mut client);
    assert_eq!(wm.selected_monitor().unwrap().current_tagset(), 0b10);

    // Clic droit sur le tag 1 : l'ajoute à la vue.
    handle.pointer_motion(12.0, 8.0);
    handle.click(BTN_RIGHT);
    let wm = sync(&handle, &mut client);
    assert_eq!(wm.selected_monitor().unwrap().current_tagset(), 0b11);

    // Mod + clic gauche sur le tag 3 : y envoie le client sélectionné.
    handle.pointer_motion(60.0, 8.0);
    handle.send(HeadlessCommand::Key { keycode: KEY_ALT, pressed: true });
    handle.click(BTN_LEFT);
    handle.send(HeadlessCommand::Key { keycode: KEY_ALT, pressed: false });
    let wm = sync(&handle, &mut client);
    assert_eq!(by_app(&wm, "a").tags, 0b100);

    // Clic sur le symbole de disposition (juste après les 9 tags) :
    // bascule vers la disposition précédente ; clic droit : monocle.
    handle.pointer_motion(9.0 * 24.0 + 10.0, 8.0);
    handle.click(BTN_LEFT);
    let wm = sync(&handle, &mut client);
    assert_eq!(wm.layout_symbol(wm.selected_monitor().unwrap().id), "><>");
    handle.click(BTN_RIGHT);
    let wm = sync(&handle, &mut client);
    assert!(wm.layout_symbol(wm.selected_monitor().unwrap().id).starts_with('['));
    // Les clics sur la barre ne sont transmis à aucun client.
    assert!(client.state.buttons.is_empty());
}

// --------------------------------------------------- cycle de vie

#[test]
fn unmap_remap_and_destroy() {
    let handle = start(test_config());
    let mut client = Client::connect(&handle);
    let a = client.create_window("a", RED);
    let b = client.create_window("b", GREEN);

    client.unmap_window(b);
    let wm = sync(&handle, &mut client);
    assert_eq!(wm.client_ids().count(), 1);
    assert_eq!(client.focused_window(), Some(a));
    assert_eq!(handle.screenshot(0).count(Rect::new(0, 0, 1280, 720), GREEN), 0);

    client.remap_window(b);
    let wm = sync(&handle, &mut client);
    assert_eq!(wm.client_ids().count(), 2);
    assert_eq!(by_app(&wm, "b").geom, Rect::new(0, 18, 704, 702));
    assert_eq!(client.focused_window(), Some(b));
    assert_eq!(handle.screenshot(0).pixel(350, 360), GREEN);

    client.destroy_window(a);
    client.destroy_window(b);
    let wm = sync(&handle, &mut client);
    assert_eq!(wm.client_ids().count(), 0);
    assert_eq!(client.focused_window(), None);
    assert_eq!(handle.screenshot(0).count(Rect::new(0, 18, 1280, 702), ROOT), 1280 * 702);
}

#[test]
fn disconnecting_client_is_cleaned_up() {
    let handle = start(test_config());
    let mut survivor = Client::connect(&handle);
    let s = survivor.create_window("survivant", RED);
    {
        let mut doomed = Client::connect(&handle);
        doomed.create_window("x", GREEN);
        doomed.create_window("y", BLUE);
        assert_eq!(sync(&handle, &mut doomed).client_ids().count(), 3);
    } // connexion fermée sans détruire les objets
    assert!(wait_until(|| handle.snapshot().client_ids().count() == 1));
    let wm = sync(&handle, &mut survivor);
    assert_eq!(by_app(&wm, "survivant").geom, Rect::new(0, 18, 1280, 702));
    assert_eq!(survivor.focused_window(), Some(s));
}

#[test]
fn frame_callbacks_are_delivered() {
    let handle = start(test_config());
    let mut client = Client::connect(&handle);
    let a = client.create_window("a", RED);
    client.request_frame(a);
    assert!(wait_until(|| {
        client.roundtrip();
        client.state.frames >= 1
    }));
}

// ------------------------------------------- activation, calques, popups

#[test]
fn activation_request_marks_urgent_without_stealing_focus() {
    let handle = start(test_config());
    let mut client = Client::connect(&handle);
    let a = client.create_window("a", RED);
    let b = client.create_window("b", GREEN);
    client.request_activation(a);
    let wm = sync(&handle, &mut client);
    assert!(by_app(&wm, "a").is_urgent);
    assert_eq!(client.focused_window(), Some(b), "le focus n'est pas volé");
    let g = by_app(&wm, "a").geom;
    assert_eq!(handle.screenshot(0).pixel(g.x, g.y + 10), BORDER_URGENT);
    assert!(wm.bar_view(wm.selected_monitor().unwrap().id).unwrap().tags[0].urgent);

    // Le focus efface l'urgence.
    handle.key_chord(&[KEY_ALT], KEY_J);
    let wm = sync(&handle, &mut client);
    assert!(!by_app(&wm, "a").is_urgent);

    // Activer le client déjà sélectionné ne le marque pas urgent.
    client.request_activation(a);
    let wm = sync(&handle, &mut client);
    assert!(!by_app(&wm, "a").is_urgent);
}

#[test]
fn layer_shell_exclusive_zone_and_keyboard() {
    let handle = start(test_config());
    let mut client = Client::connect(&handle);
    let a = client.create_window("a", RED);

    let panel = client.create_top_panel(30, true, false, YELLOW);
    let wm = sync(&handle, &mut client);
    assert_eq!(client.state.layers[panel].configures.last(), Some(&(1280, 30)));
    let mon = wm.selected_monitor().unwrap();
    assert_eq!(mon.usable, Rect::new(0, 30, 1280, 690));
    assert_eq!(wm.bar_rect(mon.id), Some(Rect::new(0, 30, 1280, 18)));
    assert_eq!(by_app(&wm, "a").geom, Rect::new(0, 48, 1280, 672));
    assert_eq!(client.state.windows[a].size, (1278, 670));
    let shot = handle.screenshot(0);
    assert_eq!(shot.count(Rect::new(0, 0, 1280, 30), YELLOW), 1280 * 30);
    assert_eq!(shot.pixel(22, 30 + 16), BORDER_SEL, "la barre est sous le panneau");
    assert_eq!(client.focused_window(), Some(a));

    client.destroy_layer(panel);
    let wm = sync(&handle, &mut client);
    assert_eq!(by_app(&wm, "a").geom, Rect::new(0, 18, 1280, 702));
    assert_eq!(client.state.windows[a].size, (1278, 700));

    // Un calque à interactivité clavier exclusive prend le focus, puis le rend.
    let menu = client.create_top_panel(40, false, true, YELLOW);
    sync(&handle, &mut client);
    assert_eq!(client.state.keyboard_focus.as_ref(), Some(&client.state.layers[menu].surface));
    client.state.keys.clear();
    handle.key_chord(&[], KEY_A);
    sync(&handle, &mut client);
    assert_eq!(client.state.keys, [(30, true), (30, false)]);
    client.destroy_layer(menu);
    sync(&handle, &mut client);
    assert_eq!(client.focused_window(), Some(a));
}

#[test]
fn popups_are_drawn_above_and_not_cropped() {
    let handle = start(test_config());
    let mut client = Client::connect(&handle);
    let a = client.create_window("a", RED);
    client.create_window("b", GREEN);
    let wm = sync(&handle, &mut client);
    let inner = by_app(&wm, "a").inner(); // pile, à droite : (705, 19, 574, 700)

    // Popup ancré à gauche de sa fenêtre : il déborde sur le client maître.
    let popup = client.create_popup(a, -60, 40, (120, 80), YELLOW);
    sync(&handle, &mut client);
    assert!(client.state.popups[popup].configured);
    let rect = Rect::new(inner.x - 60, inner.y + 40, 120, 80);
    let shot = handle.screenshot(0);
    assert_eq!(shot.count(rect, YELLOW), 120 * 80);

    // Le pointeur atteint le popup, y compris hors de sa fenêtre parente.
    handle.pointer_motion((rect.x + 10) as f64, (rect.y + 10) as f64);
    sync(&handle, &mut client);
    assert_eq!(client.state.pointer_focus.as_ref(), Some(&client.state.popups[popup].surface));
    assert_eq!(client.state.pointer_pos, (10.0, 10.0));
}

// ------------------------------------------------------- multi-moniteur

fn two_outputs() -> Handle {
    let mut options = HeadlessOptions::new(test_config());
    options.outputs = vec![(800, 600), (1024, 768)];
    start_with(options)
}

#[test]
fn two_monitors_focus_and_send() {
    let handle = two_outputs();
    let mut client = Client::connect(&handle);
    let wm = handle.snapshot();
    assert_eq!(wm.monitors().len(), 2);
    assert_eq!(wm.monitors()[0].screen, Rect::new(0, 0, 800, 600));
    assert_eq!(wm.monitors()[1].screen, Rect::new(800, 0, 1024, 768));
    let (m0, m1) = (wm.monitors()[0].id, wm.monitors()[1].id);
    assert_eq!(wm.selected_monitor().unwrap().id, m0);

    let a = client.create_window("a", RED);
    let wm = sync(&handle, &mut client);
    assert_eq!(by_app(&wm, "a").mon, m0);
    assert_eq!(client.state.windows[a].size, (798, 580));

    // Mod+Maj+. : envoie le client sur le moniteur suivant.
    handle.key_chord(&[KEY_ALT, KEY_SHIFT], KEY_DOT);
    let wm = sync(&handle, &mut client);
    assert_eq!(by_app(&wm, "a").mon, m1);
    assert_eq!(by_app(&wm, "a").geom, Rect::new(800, 18, 1024, 750));
    assert_eq!(client.state.windows[a].size, (1022, 748));
    assert_eq!(wm.selected_monitor().unwrap().id, m0, "le focus reste sur le moniteur d'origine");
    assert_eq!(client.focused_window(), None);
    assert_eq!(handle.screenshot(0).count(Rect::new(0, 0, 800, 600), RED), 0);
    assert_eq!(handle.screenshot(1).pixel(500, 400), RED);
    assert_eq!(handle.screenshot(1).pixel(0, 18), BORDER_NORM);

    // Mod+. : focus sur le moniteur suivant.
    handle.key_chord(&[KEY_ALT], KEY_DOT);
    let wm = sync(&handle, &mut client);
    assert_eq!(wm.selected_monitor().unwrap().id, m1);
    assert_eq!(client.focused_window(), Some(a));
    assert_eq!(handle.screenshot(1).pixel(0, 18), BORDER_SEL);

    // Une nouvelle fenêtre apparaît sur le moniteur sélectionné.
    client.create_window("b", GREEN);
    let wm = sync(&handle, &mut client);
    assert_eq!(by_app(&wm, "b").mon, m1);

    // Mod+, : retour.
    handle.key_chord(&[KEY_ALT], KEY_COMMA);
    let wm = sync(&handle, &mut client);
    assert_eq!(wm.selected_monitor().unwrap().id, m0);
}

#[test]
fn pointer_selects_monitor_and_tags_are_per_monitor() {
    let handle = two_outputs();
    let mut client = Client::connect(&handle);
    let wm = handle.snapshot();
    let (m0, m1) = (wm.monitors()[0].id, wm.monitors()[1].id);

    // Survoler la zone vide du second moniteur le sélectionne.
    handle.pointer_motion(1200.0, 300.0);
    let wm = sync(&handle, &mut client);
    assert_eq!(wm.selected_monitor().unwrap().id, m1);

    // Les tags sont propres à chaque moniteur.
    handle.key_chord(&[KEY_ALT], KEY_3);
    let wm = sync(&handle, &mut client);
    assert_eq!(wm.monitor(m1).unwrap().current_tagset(), 0b100);
    assert_eq!(wm.monitor(m0).unwrap().current_tagset(), 0b001);

    // La barre du moniteur non sélectionné n'affiche pas le statut.
    handle.send(HeadlessCommand::Status("statut".into()));
    let wm = handle.snapshot();
    assert_eq!(wm.bar_view(m1).unwrap().status.as_deref(), Some("statut"));
    assert!(wm.bar_view(m1).unwrap().is_selected_monitor);
    assert!(!wm.bar_view(m0).unwrap().is_selected_monitor);
    // Chaque sortie a sa barre.
    assert_eq!(handle.screenshot(0).pixel(22, 16), BORDER_SEL);
    assert_eq!(handle.screenshot(1).pixel(48 + 22, 16), BORDER_SEL);
    assert_eq!(handle.screenshot(1).pixel(22, 16), ROOT);
}

#[test]
fn output_hotplug_migrates_clients() {
    let handle = two_outputs();
    let mut client = Client::connect(&handle);
    let wm = handle.snapshot();
    let (m0, m1) = (wm.monitors()[0].id, wm.monitors()[1].id);
    assert_eq!(client.state.globals.iter().filter(|g| *g == "wl_output").count(), 2);

    // Un client sur chaque moniteur.
    let a = client.create_window("a", RED);
    handle.key_chord(&[KEY_ALT], KEY_DOT);
    let b = client.create_window("b", GREEN);
    let wm = sync(&handle, &mut client);
    assert_eq!((by_app(&wm, "a").mon, by_app(&wm, "b").mon), (m0, m1));

    // Débranchement du second écran : son client rejoint le premier.
    handle.send(HeadlessCommand::RemoveOutput(1));
    let wm = sync(&handle, &mut client);
    assert_eq!(wm.monitors().len(), 1);
    assert_eq!((by_app(&wm, "a").mon, by_app(&wm, "b").mon), (m0, m0));
    assert_eq!(wm.selected_monitor().unwrap().id, m0);
    assert_eq!(by_app(&wm, "a").geom.w + by_app(&wm, "b").geom.w, 800);
    assert_eq!(client.state.windows[a].size.1, 580);
    assert_eq!(client.state.windows[b].size.1, 580);
    let shot = handle.screenshot(0);
    assert!(shot.count(Rect::new(0, 18, 800, 582), RED) > 0 && shot.count(Rect::new(0, 18, 800, 582), GREEN) > 0);

    // Rebranchement : un nouveau moniteur vide, à droite.
    handle.send(HeadlessCommand::AddOutput { width: 640, height: 480 });
    let wm = sync(&handle, &mut client);
    assert_eq!(wm.monitors().len(), 2);
    assert_eq!(wm.monitors()[1].screen, Rect::new(800, 0, 640, 480));
    assert!(wm.monitors()[1].clients.is_empty());
    assert_eq!(handle.screenshot(1).count(Rect::new(0, 18, 640, 462), ROOT), 640 * 462);

    // Débranchement du premier : les clients migrent, l'écran restant est
    // ramené à l'origine.
    handle.send(HeadlessCommand::RemoveOutput(0));
    let wm = sync(&handle, &mut client);
    assert_eq!(wm.monitors().len(), 1);
    assert_eq!(wm.monitors()[0].screen, Rect::new(0, 0, 640, 480));
    assert_eq!(by_app(&wm, "a").geom.w + by_app(&wm, "b").geom.w, 640);
    assert_eq!(client.state.windows[a].size.1, 460);

    // Plus aucun écran : les clients sont conservés…
    handle.send(HeadlessCommand::RemoveOutput(0));
    let wm = sync(&handle, &mut client);
    assert_eq!(wm.monitors().len(), 1, "le dernier moniteur est conservé pour héberger les clients");
    assert_eq!(wm.client_ids().count(), 2);
    // … et retrouvent un écran au prochain branchement.
    handle.send(HeadlessCommand::AddOutput { width: 1024, height: 768 });
    let wm = sync(&handle, &mut client);
    assert_eq!(wm.monitors().len(), 1);
    assert_eq!(wm.monitors()[0].screen, Rect::new(0, 0, 1024, 768));
    assert_eq!(by_app(&wm, "a").geom.w + by_app(&wm, "b").geom.w, 1024);
    assert_eq!(client.state.windows[a].size.1, 748);
    let shot = handle.screenshot(0);
    assert!(shot.count(Rect::new(0, 18, 1024, 750), RED) > 0 && shot.count(Rect::new(0, 18, 1024, 750), GREEN) > 0);
}

#[test]
fn cursor_is_drawn_when_requested() {
    let mut options = HeadlessOptions::new(test_config());
    options.draw_cursor = true;
    let with_cursor = start_with(options);
    let without_cursor = start(test_config());
    let area = Rect::new(600, 300, 48, 48);
    for handle in [&with_cursor, &without_cursor] {
        handle.pointer_motion(608.0, 308.0);
    }
    let plain = without_cursor.screenshot(0);
    assert_eq!(plain.count(area, ROOT), 48 * 48);
    let shot = with_cursor.screenshot(0);
    let drawn = 48 * 48 - shot.count(area, ROOT);
    assert!(drawn > 20, "curseur attendu près de (608, 308), {drawn} pixels dessinés");
    // Le curseur suit le pointeur.
    with_cursor.pointer_motion(100.0, 500.0);
    let shot = with_cursor.screenshot(0);
    assert_eq!(shot.count(area, ROOT), 48 * 48);
    assert!(shot.count(Rect::new(92, 492, 48, 48), ROOT) < 48 * 48);
}

// ----------------------------------------------------------------- HiDPI

#[test]
fn integer_output_scale() {
    use rwdwm::core::config::MonitorRule;
    let mut config = test_config();
    config.monitor_rules.push(MonitorRule { name: Some("HEADLESS-1"), scale: Some(2), ..MonitorRule::default() });
    let handle = start(config);
    let mut client = Client::connect(&handle);
    let a = client.create_window("a", RED);
    let wm = sync(&handle, &mut client);

    // Le modèle raisonne en pixels logiques : 1280×720 physiques → 640×360.
    let mon = wm.selected_monitor().unwrap();
    assert_eq!(mon.scale, 2);
    assert_eq!(mon.screen, Rect::new(0, 0, 640, 360));
    assert_eq!(by_app(&wm, "a").geom, Rect::new(0, 18, 640, 342));
    assert_eq!(client.state.windows[a].size, (638, 340));

    // L'image, elle, est en pixels physiques : tout est doublé.
    let shot = handle.screenshot(0);
    assert_eq!((shot.width, shot.height), (1280, 720));
    assert_eq!(shot.pixel(44, 32), BORDER_SEL, "barre : fond du tag sélectionné");
    assert_eq!(shot.pixel(48 + 44, 32), ROOT, "barre : tag 2 non sélectionné");
    assert_eq!(shot.count(Rect::new(0, 36, 1280, 2), BORDER_SEL), 1280 * 2, "bordure de 2 pixels physiques");
    // Le client de test fournit un tampon à l'échelle 1, agrandi par le moteur
    // de rendu : le filtrage bilinéaire adoucit son pourtour d'un pixel.
    assert_eq!(shot.count(Rect::new(3, 39, 1274, 678), RED), 1274 * 678);
    // La barre est dessinée à l'échelle 2 (glyphes nets, 16×32), pas agrandie
    // après coup : le texte occupe des blocs de 2×2 pixels.
    assert!(shot.count(Rect::new(48, 0, 48, 36), BAR_FG) > 0);

    // Les coordonnées du pointeur restent logiques.
    handle.pointer_motion(320.0, 200.0);
    sync(&handle, &mut client);
    assert_eq!(client.hovered_window(), Some(a));
    assert_eq!(client.state.pointer_pos, (319.0, 181.0));
}

// ---------------------------------------------------------------- statut

#[test]
fn status_text_is_read_line_by_line() {
    use std::io::Write;
    use std::os::unix::net::UnixStream;
    use std::time::Duration;

    // Un compositeur sans backend suffit : la boucle d'événements est
    // déroulée à la main, le « stdin » est une paire de sockets.
    runtime_dir();
    let options = rwdwm::backend::Options::new(test_config());
    let (mut event_loop, mut data) = rwdwm::backend::create(&options).expect("création du compositeur");
    let (mut writer, reader) = UnixStream::pair().unwrap();
    rwdwm::status::watch(reader, &event_loop.handle());
    assert!(data.state.wm.status().starts_with("rwdwm-"), "statut par défaut : {:?}", data.state.wm.status());

    let mut expect = |data: &mut rwdwm::state::LoopData, wanted: &str| {
        for _ in 0..200 {
            event_loop.dispatch(Some(Duration::from_millis(10)), data).unwrap();
            if data.state.wm.status() == wanted {
                return;
            }
        }
        panic!("statut attendu {wanted:?}, obtenu {:?}", data.state.wm.status());
    };
    writer.write_all(b"premi\xc3\xa8re ligne\n").unwrap();
    expect(&mut data, "première ligne");
    // Plusieurs lignes d'un coup : la dernière l'emporte ; fin de ligne CRLF.
    writer.write_all(b"deux\ntrois\r\n").unwrap();
    expect(&mut data, "trois");
    // Une séquence UTF-8 invalide n'interrompt pas la lecture.
    writer.write_all(b"cass\xff\n").unwrap();
    expect(&mut data, "cass\u{fffd}");
    writer.write_all(b"apr\xc3\xa8s\n").unwrap();
    expect(&mut data, "après");
    // Fermeture du flux : le dernier statut reste affiché.
    drop(writer);
    for _ in 0..5 {
        event_loop.dispatch(Some(Duration::from_millis(10)), &mut data).unwrap();
    }
    assert_eq!(data.state.wm.status(), "après");
}
