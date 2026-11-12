#![allow(clippy::field_reassign_with_default)]

use super::*;
use crate::config::{MonitorRule, Rule};

const SCREEN: Rect = Rect::new(0, 0, 1280, 720);
const AREA: Rect = Rect::new(0, 18, 1280, 702);

fn wm() -> Wm {
    let mut wm = Wm::new(Config::default());
    wm.add_monitor("TEST-1", SCREEN);
    wm
}

fn wm2() -> (Wm, MonitorId, MonitorId) {
    let mut wm = Wm::new(Config::default());
    let a = wm.add_monitor("A", SCREEN);
    let b = wm.add_monitor("B", Rect::new(1280, 0, 1920, 1080));
    (wm, a, b)
}

fn add(wm: &mut Wm, app_id: &str) -> ClientId {
    let id = wm
        .manage(NewClient { app_id: app_id.into(), title: app_id.into(), ..Default::default() })
        .expect("un moniteur existe");
    wm.check_invariants().unwrap();
    id
}

fn act(wm: &mut Wm, a: Action) -> Vec<Effect> {
    let fx = wm.apply(&a);
    wm.check_invariants().unwrap_or_else(|e| panic!("après {a:?} : {e}"));
    fx
}

fn geom(wm: &Wm, id: ClientId) -> Rect {
    wm.client(id).unwrap().geom
}

fn mon(wm: &Wm) -> &Monitor {
    wm.selected_monitor().unwrap()
}

// ---------------------------------------------------------------- moniteurs

#[test]
fn monitor_areas_account_for_the_bar() {
    let wm = wm();
    let m = mon(&wm);
    assert_eq!(m.screen, SCREEN);
    assert_eq!(m.window_area, AREA);
    assert_eq!(wm.bar_rect(m.id), Some(Rect::new(0, 0, 1280, 18)));
}

#[test]
fn bottom_bar() {
    let mut wm = Wm::new(Config { top_bar: false, ..Config::default() });
    let m = wm.add_monitor("M", SCREEN);
    assert_eq!(wm.monitor(m).unwrap().window_area, Rect::new(0, 0, 1280, 702));
    assert_eq!(wm.bar_rect(m), Some(Rect::new(0, 702, 1280, 18)));
}

#[test]
fn toggle_bar_gives_the_space_back() {
    let mut wm = wm();
    let a = add(&mut wm, "a");
    assert_eq!(geom(&wm, a), AREA);
    act(&mut wm, Action::ToggleBar);
    assert_eq!(mon(&wm).window_area, SCREEN);
    assert_eq!(wm.bar_rect(mon(&wm).id), None);
    assert_eq!(geom(&wm, a), SCREEN);
    act(&mut wm, Action::ToggleBar);
    assert_eq!(geom(&wm, a), AREA);
}

#[test]
fn layer_shell_exclusive_zone_shrinks_everything() {
    let mut wm = wm();
    let a = add(&mut wm, "a");
    let id = mon(&wm).id;
    wm.set_monitor_geometry(id, SCREEN, Rect::new(0, 30, 1280, 690));
    wm.check_invariants().unwrap();
    assert_eq!(wm.bar_rect(id), Some(Rect::new(0, 30, 1280, 18)));
    assert_eq!(geom(&wm, a), Rect::new(0, 48, 1280, 672));
}

#[test]
fn monitor_rules_apply_by_name() {
    let mut cfg = Config::default();
    cfg.monitor_rules = vec![
        MonitorRule { name: Some("eDP"), mfact: Some(0.6), nmaster: Some(2), layout: Some(2), scale: Some(2) },
        MonitorRule { name: Some("HDMI"), mfact: Some(9.0), layout: Some(42), ..Default::default() },
    ];
    let mut wm = Wm::new(cfg);
    let a = wm.add_monitor("eDP-1", SCREEN);
    let b = wm.add_monitor("HDMI-A-1", Rect::new(1280, 0, 800, 600));
    let c = wm.add_monitor("DP-3", Rect::new(2080, 0, 800, 600));
    let ma = wm.monitor(a).unwrap();
    assert_eq!((ma.mfact, ma.nmaster, ma.current_layout(), ma.scale), (0.6, 2, 2, 2));
    let mb = wm.monitor(b).unwrap();
    assert_eq!((mb.mfact, mb.current_layout()), (0.95, 0));
    assert_eq!(wm.monitor(c).unwrap().mfact, 0.55);
    wm.check_invariants().unwrap();
}

#[test]
fn monitor_at_falls_back_to_selected() {
    let (wm, a, b) = wm2();
    assert_eq!(wm.monitor_at(10, 10), Some(a));
    assert_eq!(wm.monitor_at(1300, 900), Some(b));
    assert_eq!(wm.monitor_at(-50, 5000), Some(a));
    assert_eq!(Wm::new(Config::default()).monitor_at(0, 0), None);
}

// ------------------------------------------------------------------ clients

#[test]
fn no_monitor_no_client() {
    let mut wm = Wm::new(Config::default());
    assert_eq!(wm.manage(NewClient::default()), None);
    assert_eq!(wm.apply(&Action::Zoom), vec![]);
    assert_eq!(wm.apply(&Action::Quit), vec![Effect::Quit]);
    wm.check_invariants().unwrap();
}

#[test]
fn new_client_becomes_master_and_focused() {
    let mut wm = wm();
    let a = add(&mut wm, "a");
    assert_eq!(wm.selected(), Some(a));
    assert_eq!(geom(&wm, a), AREA);
    assert_eq!(wm.client(a).unwrap().inner(), Rect::new(1, 19, 1278, 700));

    let b = add(&mut wm, "b");
    assert_eq!(wm.selected(), Some(b));
    assert_eq!(mon(&wm).clients, vec![b, a]);
    assert_eq!(geom(&wm, b), Rect::new(0, 18, 704, 702));
    assert_eq!(geom(&wm, a), Rect::new(704, 18, 576, 702));

    let c = add(&mut wm, "c");
    assert_eq!(geom(&wm, c), Rect::new(0, 18, 704, 702));
    assert_eq!(geom(&wm, b), Rect::new(704, 18, 576, 351));
    assert_eq!(geom(&wm, a), Rect::new(704, 369, 576, 351));
}

#[test]
fn unmanage_refocuses_most_recent() {
    let mut wm = wm();
    let a = add(&mut wm, "a");
    let b = add(&mut wm, "b");
    let c = add(&mut wm, "c");
    wm.focus_client(a);
    wm.focus_client(c);
    wm.unmanage(c);
    wm.check_invariants().unwrap();
    assert_eq!(wm.selected(), Some(a), "le focus revient au précédent dans la pile de focus");
    wm.unmanage(a);
    assert_eq!(wm.selected(), Some(b));
    assert_eq!(geom(&wm, b), AREA);
    wm.unmanage(b);
    wm.unmanage(b);
    assert_eq!(wm.selected(), None);
    wm.check_invariants().unwrap();
}

#[test]
fn rules_set_tags_floating_and_monitor() {
    let mut cfg = Config::default();
    cfg.rules = vec![
        Rule { app_id: Some("Gimp"), is_floating: true, ..Default::default() },
        Rule { app_id: Some("firefox"), tags: 1 << 8, ..Default::default() },
        Rule { title: Some("sur B"), monitor: Some(1), ..Default::default() },
        Rule { app_id: Some("hors-masque"), tags: 1 << 20, ..Default::default() },
    ];
    let mut wm = Wm::new(cfg);
    let a = wm.add_monitor("A", SCREEN);
    let b = wm.add_monitor("B", Rect::new(1280, 0, 1920, 1080));

    let gimp = add(&mut wm, "Gimp-2.10");
    assert!(wm.client(gimp).unwrap().is_floating);
    assert!(wm.client(gimp).unwrap().needs_initial_geometry);

    let ff = add(&mut wm, "org.mozilla.firefox");
    assert_eq!(wm.client(ff).unwrap().tags, 1 << 8);
    assert!(!wm.is_visible(ff));
    assert_eq!(wm.selected(), Some(gimp), "un client invisible ne prend pas le focus");

    let on_b =
        wm.manage(NewClient { app_id: "x".into(), title: "fenêtre sur B".into(), ..Default::default() }).unwrap();
    assert_eq!(wm.client(on_b).unwrap().mon, b);
    assert_eq!(wm.selected_monitor().unwrap().id, a, "le moniteur sélectionné ne change pas");
    assert_eq!(wm.monitor(b).unwrap().sel, Some(on_b));

    let odd = add(&mut wm, "hors-masque");
    assert_eq!(wm.client(odd).unwrap().tags, 1, "des tags hors masque retombent sur le tagset courant");
    wm.check_invariants().unwrap();
}

#[test]
fn transient_inherits_parent_and_floats() {
    let (mut wm, _, b) = wm2();
    act(&mut wm, Action::FocusMon(1));
    act(&mut wm, Action::View(1 << 3));
    let parent = add(&mut wm, "app");
    act(&mut wm, Action::FocusMon(1));
    let child = wm.manage(NewClient { app_id: "app".into(), parent: Some(parent), ..Default::default() }).unwrap();
    let c = wm.client(child).unwrap();
    assert_eq!((c.mon, c.tags, c.is_floating), (b, 1 << 3, true));
    wm.check_invariants().unwrap();
}

#[test]
fn fixed_size_clients_float_centered() {
    let mut wm = wm();
    let id = wm.manage(NewClient { min_size: (300, 200), max_size: (300, 200), ..Default::default() }).unwrap();
    let c = wm.client(id).unwrap();
    assert!(c.is_fixed && c.is_floating && !c.needs_initial_geometry);
    assert_eq!(c.geom, Rect::new(489, 268, 302, 202));
    // On ne peut pas tuiler un client de taille fixe.
    act(&mut wm, Action::ToggleFloating);
    assert!(wm.client(id).unwrap().is_floating);
}

#[test]
fn natural_size_centers_floating_clients_once() {
    let mut cfg = Config::default();
    cfg.rules = vec![Rule { app_id: Some("float"), is_floating: true, ..Default::default() }];
    let mut wm = Wm::new(cfg);
    wm.add_monitor("M", SCREEN);
    let id = add(&mut wm, "float");
    assert!(wm.set_natural_size(id, 400, 300));
    assert_eq!(geom(&wm, id), Rect::new(439, 218, 402, 302));
    assert!(!wm.set_natural_size(id, 100, 100), "seule la première taille compte");
    assert_eq!(geom(&wm, id), Rect::new(439, 218, 402, 302));
    let tiled = add(&mut wm, "tiled");
    assert!(!wm.set_natural_size(tiled, 100, 100));
    assert_eq!(geom(&wm, tiled), AREA);
}

#[test]
fn titles_app_ids_and_status() {
    let mut wm = wm();
    let a = add(&mut wm, "a");
    wm.set_title(a, "Nouveau titre");
    wm.set_app_id(a, "autre");
    wm.set_status("12:34");
    let c = wm.client(a).unwrap();
    assert_eq!((c.title.as_str(), c.app_id.as_str()), ("Nouveau titre", "autre"));
    assert_eq!(wm.status(), "12:34");
    wm.set_title(999, "inconnu");
}

// -------------------------------------------------------------------- focus

#[test]
fn focusstack_cycles_through_visible_clients() {
    let mut wm = wm();
    let a = add(&mut wm, "a");
    let b = add(&mut wm, "b");
    let c = add(&mut wm, "c");
    // Ordre de la liste : c, b, a.
    assert_eq!(wm.selected(), Some(c));
    act(&mut wm, Action::FocusStack(1));
    assert_eq!(wm.selected(), Some(b));
    act(&mut wm, Action::FocusStack(1));
    assert_eq!(wm.selected(), Some(a));
    act(&mut wm, Action::FocusStack(1));
    assert_eq!(wm.selected(), Some(c), "bouclage vers le début");
    act(&mut wm, Action::FocusStack(-1));
    assert_eq!(wm.selected(), Some(a), "bouclage vers la fin");
    act(&mut wm, Action::FocusStack(-1));
    assert_eq!(wm.selected(), Some(b));
}

#[test]
fn focusstack_skips_hidden_clients() {
    let mut wm = wm();
    let a = add(&mut wm, "a");
    let b = add(&mut wm, "b");
    let c = add(&mut wm, "c");
    wm.focus_client(b);
    act(&mut wm, Action::Tag(1 << 1));
    assert!(!wm.is_visible(b));
    assert_eq!(wm.selected(), Some(c));
    act(&mut wm, Action::FocusStack(1));
    assert_eq!(wm.selected(), Some(a));
    act(&mut wm, Action::FocusStack(1));
    assert_eq!(wm.selected(), Some(c));
}

#[test]
fn focusstack_is_locked_by_fullscreen() {
    let mut wm = wm();
    let _a = add(&mut wm, "a");
    let b = add(&mut wm, "b");
    act(&mut wm, Action::ToggleFullscreen);
    act(&mut wm, Action::FocusStack(1));
    assert_eq!(wm.selected(), Some(b));

    let mut wm = Wm::new(Config { lock_fullscreen: false, ..Config::default() });
    wm.add_monitor("M", SCREEN);
    let a = add(&mut wm, "a");
    let _b = add(&mut wm, "b");
    act(&mut wm, Action::ToggleFullscreen);
    act(&mut wm, Action::FocusStack(1));
    assert_eq!(wm.selected(), Some(a));
}

#[test]
fn focus_client_ignores_hidden_and_unknown() {
    let mut wm = wm();
    let a = add(&mut wm, "a");
    act(&mut wm, Action::Tag(1 << 4));
    let b = add(&mut wm, "b");
    wm.focus_client(a);
    assert_eq!(wm.selected(), Some(b));
    wm.focus_client(12345);
    assert_eq!(wm.selected(), Some(b));
    wm.check_invariants().unwrap();
}

// --------------------------------------------------------------------- tags

#[test]
fn view_switches_and_remembers_previous_tagset() {
    let mut wm = wm();
    let a = add(&mut wm, "a");
    act(&mut wm, Action::View(1 << 1));
    assert_eq!(mon(&wm).current_tagset(), 1 << 1);
    assert!(!wm.is_visible(a));
    assert_eq!(wm.selected(), None);
    let b = add(&mut wm, "b");
    assert_eq!(wm.client(b).unwrap().tags, 1 << 1);
    assert_eq!(geom(&wm, b), AREA);

    act(&mut wm, Action::View(0));
    assert_eq!(mon(&wm).current_tagset(), 1);
    assert_eq!(wm.selected(), Some(a));
    act(&mut wm, Action::View(0));
    assert_eq!(mon(&wm).current_tagset(), 1 << 1);
    assert_eq!(wm.selected(), Some(b));

    // Revoir le tagset courant ne fait rien (et n'écrase pas le précédent).
    act(&mut wm, Action::View(1 << 1));
    act(&mut wm, Action::View(0));
    assert_eq!(mon(&wm).current_tagset(), 1);
}

#[test]
fn view_all_shows_everything() {
    let mut wm = wm();
    let a = add(&mut wm, "a");
    act(&mut wm, Action::View(1 << 2));
    let b = add(&mut wm, "b");
    act(&mut wm, Action::View(!0));
    assert_eq!(mon(&wm).current_tagset(), 0b1_1111_1111);
    assert!(wm.is_visible(a) && wm.is_visible(b));
    assert_eq!(geom(&wm, b), Rect::new(0, 18, 704, 702));
    assert_eq!(geom(&wm, a), Rect::new(704, 18, 576, 702));
}

#[test]
fn toggleview_adds_and_removes_but_never_empties() {
    let mut wm = wm();
    let a = add(&mut wm, "a");
    act(&mut wm, Action::View(1 << 1));
    let b = add(&mut wm, "b");
    act(&mut wm, Action::ToggleView(1));
    assert_eq!(mon(&wm).current_tagset(), 0b11);
    assert!(wm.is_visible(a) && wm.is_visible(b));
    act(&mut wm, Action::ToggleView(1 << 1));
    assert_eq!(mon(&wm).current_tagset(), 0b01);
    assert_eq!(wm.selected(), Some(a));
    act(&mut wm, Action::ToggleView(1));
    assert_eq!(mon(&wm).current_tagset(), 0b01, "le dernier tag ne peut pas être retiré");
}

#[test]
fn tag_moves_the_selected_client() {
    let mut wm = wm();
    let a = add(&mut wm, "a");
    let b = add(&mut wm, "b");
    act(&mut wm, Action::Tag(1 << 2));
    assert_eq!(wm.client(b).unwrap().tags, 1 << 2);
    assert_eq!(wm.selected(), Some(a));
    assert_eq!(geom(&wm, a), AREA);
    act(&mut wm, Action::Tag(0));
    assert_eq!(wm.client(a).unwrap().tags, 1, "un masque nul est ignoré");
    act(&mut wm, Action::Tag(!0));
    assert_eq!(wm.client(a).unwrap().tags, 0b1_1111_1111);
    act(&mut wm, Action::View(1 << 6));
    assert!(wm.is_visible(a), "un client portant tous les tags est visible partout");
}

#[test]
fn toggletag_never_leaves_a_client_without_tag() {
    let mut wm = wm();
    let a = add(&mut wm, "a");
    act(&mut wm, Action::ToggleTag(1 << 1));
    assert_eq!(wm.client(a).unwrap().tags, 0b11);
    act(&mut wm, Action::ToggleTag(1));
    assert_eq!(wm.client(a).unwrap().tags, 0b10);
    assert_eq!(wm.selected(), None);
    wm.focus_client(a);
    act(&mut wm, Action::View(1 << 1));
    assert_eq!(wm.selected(), Some(a));
    act(&mut wm, Action::ToggleTag(1 << 1));
    assert_eq!(wm.client(a).unwrap().tags, 0b10);
}

// -------------------------------------------------------------- dispositions

#[test]
fn zoom_promotes_and_swaps() {
    let mut wm = wm();
    let a = add(&mut wm, "a");
    let b = add(&mut wm, "b");
    let c = add(&mut wm, "c");
    // Liste : c, b, a. Zoom sur le maître : le suivant prend sa place.
    act(&mut wm, Action::Zoom);
    assert_eq!(mon(&wm).clients, vec![b, c, a]);
    assert_eq!(wm.selected(), Some(b));
    // Zoom sur un client de la pile : il devient maître.
    wm.focus_client(a);
    act(&mut wm, Action::Zoom);
    assert_eq!(mon(&wm).clients, vec![a, b, c]);
    assert_eq!(wm.selected(), Some(a));
    assert_eq!(geom(&wm, a), Rect::new(0, 18, 704, 702));
}

#[test]
fn zoom_ignores_floating_and_single_clients() {
    let mut wm = wm();
    let a = add(&mut wm, "a");
    act(&mut wm, Action::Zoom);
    assert_eq!(mon(&wm).clients, vec![a]);
    let b = add(&mut wm, "b");
    act(&mut wm, Action::ToggleFloating);
    act(&mut wm, Action::Zoom);
    assert_eq!(mon(&wm).clients, vec![b, a]);
    act(&mut wm, Action::ToggleFloating);
    act(&mut wm, Action::SetLayout(Some(1)));
    act(&mut wm, Action::Zoom);
    assert_eq!(mon(&wm).clients, vec![b, a], "pas de zoom en disposition flottante");
}

#[test]
fn mfact_is_relative_absolute_and_bounded() {
    let mut wm = wm();
    let a = add(&mut wm, "a");
    let _b = add(&mut wm, "b");
    act(&mut wm, Action::SetMFact(0.05));
    assert!((mon(&wm).mfact - 0.60).abs() < 1e-6);
    assert_eq!(geom(&wm, a).x, 768);
    act(&mut wm, Action::SetMFact(1.25));
    assert!((mon(&wm).mfact - 0.25).abs() < 1e-6);
    for _ in 0..40 {
        act(&mut wm, Action::SetMFact(0.05));
    }
    assert!((mon(&wm).mfact - 0.95).abs() < 1e-3, "mfact = {}", mon(&wm).mfact);
    for _ in 0..40 {
        act(&mut wm, Action::SetMFact(-0.05));
    }
    assert!((mon(&wm).mfact - 0.05).abs() < 1e-3, "mfact = {}", mon(&wm).mfact);
    // Sans effet en disposition flottante.
    act(&mut wm, Action::SetLayout(Some(1)));
    let before = mon(&wm).mfact;
    act(&mut wm, Action::SetMFact(0.05));
    assert_eq!(mon(&wm).mfact, before);
}

#[test]
fn nmaster_changes_the_columns() {
    let mut wm = wm();
    let a = add(&mut wm, "a");
    let b = add(&mut wm, "b");
    let c = add(&mut wm, "c");
    act(&mut wm, Action::IncNMaster(1));
    assert_eq!(geom(&wm, c), Rect::new(0, 18, 704, 351));
    assert_eq!(geom(&wm, b), Rect::new(0, 369, 704, 351));
    assert_eq!(geom(&wm, a), Rect::new(704, 18, 576, 702));
    for _ in 0..5 {
        act(&mut wm, Action::IncNMaster(-1));
    }
    assert_eq!(mon(&wm).nmaster, 0);
    assert_eq!(geom(&wm, c), Rect::new(0, 18, 1280, 234));
}

#[test]
fn setlayout_and_toggle_previous() {
    let mut wm = wm();
    let a = add(&mut wm, "a");
    let b = add(&mut wm, "b");
    let id = mon(&wm).id;
    assert_eq!(wm.layout_symbol(id), "[]=");
    act(&mut wm, Action::SetLayout(Some(2)));
    assert_eq!(wm.layout_symbol(id), "[2]");
    assert_eq!((geom(&wm, a), geom(&wm, b)), (AREA, AREA));
    act(&mut wm, Action::SetLayout(None));
    assert_eq!(wm.layout_symbol(id), "[]=");
    act(&mut wm, Action::SetLayout(None));
    assert_eq!(wm.layout_symbol(id), "[2]");
    // Choisir la disposition courante ne change pas la « précédente ».
    act(&mut wm, Action::SetLayout(Some(2)));
    act(&mut wm, Action::SetLayout(None));
    assert_eq!(wm.layout_symbol(id), "[]=");
    // Indice invalide : traité comme « précédente ».
    act(&mut wm, Action::SetLayout(Some(99)));
    assert_eq!(wm.layout_symbol(id), "[2]");
}

#[test]
fn monocle_symbol_without_client_is_the_configured_one() {
    let mut wm = wm();
    act(&mut wm, Action::SetLayout(Some(2)));
    assert_eq!(wm.layout_symbol(mon(&wm).id), "[M]");
}

#[test]
fn floating_layout_keeps_geometries() {
    let mut wm = wm();
    let a = add(&mut wm, "a");
    let b = add(&mut wm, "b");
    let (ga, gb) = (geom(&wm, a), geom(&wm, b));
    act(&mut wm, Action::SetLayout(Some(1)));
    assert_eq!(wm.layout_symbol(mon(&wm).id), "><>");
    assert_eq!((geom(&wm, a), geom(&wm, b)), (ga, gb));
    // Tous les clients se déplacent librement, sans devenir « flottants ».
    wm.mouse_move(a, 100, 100);
    assert_eq!(geom(&wm, a), Rect::new(100, 100, ga.w, ga.h));
    assert!(!wm.client(a).unwrap().is_floating);
    act(&mut wm, Action::SetLayout(Some(0)));
    assert_eq!((geom(&wm, a), geom(&wm, b)), (ga, gb));
}

#[test]
fn toggle_floating_keeps_geometry_and_retiles_the_rest() {
    let mut wm = wm();
    let a = add(&mut wm, "a");
    let b = add(&mut wm, "b");
    let before = geom(&wm, b);
    act(&mut wm, Action::ToggleFloating);
    assert!(wm.client(b).unwrap().is_floating);
    assert_eq!(geom(&wm, b), before);
    assert_eq!(geom(&wm, a), AREA);
    act(&mut wm, Action::ToggleFloating);
    assert_eq!(geom(&wm, a), Rect::new(704, 18, 576, 702));
}

#[test]
fn fullscreen_covers_the_screen_and_restores() {
    let mut wm = wm();
    let a = add(&mut wm, "a");
    let b = add(&mut wm, "b");
    let before = geom(&wm, b);
    act(&mut wm, Action::ToggleFullscreen);
    let c = wm.client(b).unwrap();
    assert!(c.is_fullscreen && c.is_floating);
    assert_eq!((c.geom, c.bw), (SCREEN, 0));
    assert_eq!(c.inner(), SCREEN);
    assert_eq!(geom(&wm, a), AREA, "les autres clients se partagent la zone");
    // Ni flottement ni déplacement en plein écran.
    act(&mut wm, Action::ToggleFloating);
    assert!(wm.client(b).unwrap().is_fullscreen);
    assert_eq!(act(&mut wm, Action::MoveMouse), vec![]);
    wm.mouse_move(b, 50, 50);
    wm.mouse_resize(b, 50, 50);
    assert_eq!(geom(&wm, b), SCREEN);

    act(&mut wm, Action::ToggleFullscreen);
    let c = wm.client(b).unwrap();
    assert!(!c.is_fullscreen && !c.is_floating);
    assert_eq!((c.geom, c.bw), (before, 1));
    wm.set_fullscreen(b, false);
    wm.set_fullscreen(999, true);
    wm.check_invariants().unwrap();
}

#[test]
fn fullscreen_follows_monitor_resize() {
    let mut wm = wm();
    let a = add(&mut wm, "a");
    act(&mut wm, Action::ToggleFullscreen);
    let id = mon(&wm).id;
    let bigger = Rect::new(0, 0, 1920, 1080);
    wm.set_monitor_geometry(id, bigger, bigger);
    wm.check_invariants().unwrap();
    assert_eq!(geom(&wm, a), bigger);
}

// ------------------------------------------------------------------- effets

#[test]
fn effects_are_reported() {
    let mut wm = wm();
    assert_eq!(act(&mut wm, Action::KillClient), vec![]);
    assert_eq!(act(&mut wm, Action::MoveMouse), vec![]);
    let a = add(&mut wm, "a");
    assert_eq!(act(&mut wm, Action::KillClient), vec![Effect::Close(a)]);
    assert!(wm.client(a).is_some(), "le client reste géré jusqu'à sa disparition effective");
    assert_eq!(act(&mut wm, Action::MoveMouse), vec![Effect::BeginMove(a)]);
    assert_eq!(act(&mut wm, Action::ResizeMouse), vec![Effect::BeginResize(a)]);
    assert_eq!(act(&mut wm, Action::spawn(&["foot"])), vec![Effect::Spawn(vec!["foot".into()])]);
    assert_eq!(
        act(&mut wm, Action::shcmd("a | b")),
        vec![Effect::Spawn(vec!["/bin/sh".into(), "-c".into(), "a | b".into()])]
    );
    assert_eq!(act(&mut wm, Action::ChVt(2)), vec![Effect::ChVt(2)]);
    assert_eq!(act(&mut wm, Action::Quit), vec![Effect::Quit]);
}

#[test]
fn bar_click_actions_receive_the_clicked_tag() {
    assert_eq!(Action::View(0).with_tag_if_unset(4), Action::View(4));
    assert_eq!(Action::ToggleView(0).with_tag_if_unset(4), Action::ToggleView(4));
    assert_eq!(Action::Tag(0).with_tag_if_unset(4), Action::Tag(4));
    assert_eq!(Action::ToggleTag(0).with_tag_if_unset(4), Action::ToggleTag(4));
    assert_eq!(Action::View(2).with_tag_if_unset(4), Action::View(2));
    assert_eq!(Action::Zoom.with_tag_if_unset(4), Action::Zoom);
}

// ------------------------------------------------------------------- souris

#[test]
fn mouse_move_detaches_tiled_client_past_snap() {
    let mut wm = wm();
    let a = add(&mut wm, "a");
    let b = add(&mut wm, "b");
    // En deçà du seuil (32 px) : rien ne bouge.
    wm.mouse_move(b, 20, 40);
    assert!(!wm.client(b).unwrap().is_floating);
    assert_eq!(geom(&wm, b), Rect::new(0, 18, 704, 702));
    // Au-delà : le client devient flottant et suit la souris.
    wm.mouse_move(b, 100, 200);
    wm.check_invariants().unwrap();
    assert!(wm.client(b).unwrap().is_floating);
    assert_eq!(geom(&wm, b), Rect::new(100, 200, 704, 702));
    assert_eq!(geom(&wm, a), AREA);
}

#[test]
fn mouse_move_snaps_to_window_area_edges() {
    let mut cfg = Config::default();
    cfg.rules = vec![Rule { is_floating: true, ..Default::default() }];
    let mut wm = Wm::new(cfg);
    wm.add_monitor("M", SCREEN);
    let a = add(&mut wm, "a");
    wm.set_natural_size(a, 198, 98);
    wm.mouse_move(a, 20, 30);
    assert_eq!((geom(&wm, a).x, geom(&wm, a).y), (0, 18));
    wm.mouse_move(a, 1280 - 200 - 10, 720 - 100 + 15);
    assert_eq!((geom(&wm, a).x, geom(&wm, a).y), (1080, 620));
    wm.mouse_move(a, 500, 300);
    assert_eq!(geom(&wm, a), Rect::new(500, 300, 200, 100));
}

#[test]
fn mouse_resize_respects_size_limits() {
    let mut wm = wm();
    let a = add(&mut wm, "a");
    wm.set_size_limits(a, (100, 80), (800, 600));
    wm.mouse_resize(a, 400, 300);
    assert!(wm.client(a).unwrap().is_floating, "redimensionner détache un client tuilé");
    assert_eq!(geom(&wm, a), Rect::new(0, 18, 400, 300));
    wm.mouse_resize(a, 10, 10);
    assert_eq!((geom(&wm, a).w, geom(&wm, a).h), (102, 82));
    wm.mouse_resize(a, 5000, 5000);
    assert_eq!((geom(&wm, a).w, geom(&wm, a).h), (802, 602));
    wm.set_size_limits(a, (0, 0), (0, 0));
    wm.mouse_resize(a, -5, -5);
    assert_eq!((geom(&wm, a).w, geom(&wm, a).h), (3, 3), "jamais de surface vide");
    wm.check_invariants().unwrap();
}

#[test]
fn mouse_release_moves_client_to_the_monitor_under_it() {
    let (mut wm, a, b) = wm2();
    let c = add(&mut wm, "c");
    act(&mut wm, Action::ToggleFloating);
    wm.mouse_resize(c, 400, 300);
    // 280 px sur A, 120 px sur B : le client reste sur A.
    wm.mouse_move(c, 1000, 100);
    wm.mouse_release(c);
    assert_eq!(wm.client(c).unwrap().mon, a);
    // 80 px sur A, 320 px sur B : il passe sur B.
    wm.mouse_move(c, 1200, 100);
    wm.mouse_release(c);
    wm.check_invariants().unwrap();
    assert_eq!(wm.client(c).unwrap().mon, b);
    assert_eq!(wm.selected_monitor().unwrap().id, b);
    assert_eq!(wm.selected(), Some(c));
    assert_eq!(geom(&wm, c), Rect::new(1200, 100, 400, 300), "la position de dépôt est conservée");
}

// ----------------------------------------------------------- multi-moniteur

#[test]
fn focusmon_cycles() {
    let (mut wm, a, b) = wm2();
    assert_eq!(wm.selected_monitor().unwrap().id, a);
    act(&mut wm, Action::FocusMon(1));
    assert_eq!(wm.selected_monitor().unwrap().id, b);
    act(&mut wm, Action::FocusMon(1));
    assert_eq!(wm.selected_monitor().unwrap().id, a);
    act(&mut wm, Action::FocusMon(-1));
    assert_eq!(wm.selected_monitor().unwrap().id, b);
    // Avec un seul moniteur : sans effet.
}

#[test]
fn monitor_actions_are_noops_with_a_single_monitor() {
    let mut wm = wm();
    let a = add(&mut wm, "a");
    act(&mut wm, Action::FocusMon(1));
    act(&mut wm, Action::TagMon(1));
    assert_eq!(wm.selected(), Some(a));
    assert_eq!(geom(&wm, a), AREA);
}

#[test]
fn tagmon_sends_client_and_keeps_focus_here() {
    let (mut wm, a, b) = wm2();
    let x = add(&mut wm, "x");
    let y = add(&mut wm, "y");
    act(&mut wm, Action::FocusMon(1));
    act(&mut wm, Action::View(1 << 4));
    act(&mut wm, Action::FocusMon(1));
    assert_eq!(wm.selected(), Some(y));

    act(&mut wm, Action::TagMon(1));
    let c = wm.client(y).unwrap();
    assert_eq!((c.mon, c.tags), (b, 1 << 4), "le client prend le tagset du moniteur d'arrivée");
    assert_eq!(c.geom, Rect::new(1280, 18, 1920, 1062));
    assert_eq!(wm.selected_monitor().unwrap().id, a, "le focus reste sur le moniteur de départ");
    assert_eq!(wm.selected(), Some(x));
    assert_eq!(geom(&wm, x), AREA);
    assert_eq!(wm.monitor(b).unwrap().sel, Some(y));
}

#[test]
fn tagmon_keeps_floating_client_inside_target() {
    let (mut wm, _, b) = wm2();
    let x = add(&mut wm, "x");
    act(&mut wm, Action::ToggleFloating);
    wm.mouse_resize(x, 300, 200);
    wm.mouse_move(x, 500, 400);
    act(&mut wm, Action::TagMon(1));
    assert_eq!(wm.client(x).unwrap().mon, b);
    assert_eq!(geom(&wm, x), Rect::new(1780, 400, 300, 200), "même position relative à la zone des fenêtres");
}

#[test]
fn select_monitor_follows_the_pointer() {
    let (mut wm, a, b) = wm2();
    let x = add(&mut wm, "x");
    wm.select_monitor(b);
    assert_eq!(wm.selected_monitor().unwrap().id, b);
    assert_eq!(wm.selected(), None);
    wm.select_monitor(a);
    assert_eq!(wm.selected(), Some(x));
    wm.select_monitor(999);
    assert_eq!(wm.selected_monitor().unwrap().id, a);
    wm.check_invariants().unwrap();
}

#[test]
fn focus_client_on_other_monitor_selects_it() {
    let (mut wm, _, b) = wm2();
    let x = add(&mut wm, "x");
    act(&mut wm, Action::TagMon(1));
    assert_eq!(wm.selected(), None);
    wm.focus_client(x);
    assert_eq!(wm.selected_monitor().unwrap().id, b);
    assert_eq!(wm.selected(), Some(x));
}

#[test]
fn removing_a_monitor_rehomes_its_clients() {
    let (mut wm, a, b) = wm2();
    let x = add(&mut wm, "x");
    act(&mut wm, Action::FocusMon(1));
    act(&mut wm, Action::View(1 << 2));
    let y = add(&mut wm, "y");
    let z = add(&mut wm, "z");
    assert!(wm.remove_monitor(b));
    wm.check_invariants().unwrap();
    assert_eq!(wm.monitors().len(), 1);
    assert_eq!(wm.selected_monitor().unwrap().id, a);
    assert_eq!(wm.monitor(a).unwrap().clients, vec![z, y, x]);
    assert_eq!(wm.client(y).unwrap().tags, 1 << 2, "les tags sont conservés");
    assert_eq!(wm.selected(), Some(x));
    act(&mut wm, Action::View(1 << 2));
    assert_eq!(wm.selected(), Some(z));
    assert_eq!(geom(&wm, z), Rect::new(0, 18, 704, 702));

    assert!(!wm.remove_monitor(a), "le dernier moniteur est conservé");
    assert!(!wm.remove_monitor(999));
    assert_eq!(wm.monitors().len(), 1);
    wm.rename_monitor(a, "NOUVEAU");
    assert_eq!(wm.monitor(a).unwrap().name, "NOUVEAU");
}

#[test]
fn removing_the_first_monitor_keeps_selection_valid() {
    let (mut wm, a, b) = wm2();
    let x = add(&mut wm, "x");
    act(&mut wm, Action::ToggleFloating);
    wm.mouse_resize(x, 300, 200);
    wm.mouse_move(x, 900, 400);
    act(&mut wm, Action::FocusMon(1));
    assert!(wm.remove_monitor(a));
    wm.check_invariants().unwrap();
    assert_eq!(wm.selected_monitor().unwrap().id, b);
    assert_eq!(wm.selected(), Some(x));
    assert_eq!(geom(&wm, x), Rect::new(2180, 400, 300, 200));
}

// -------------------------------------------------------------------- barre

#[test]
fn bar_view_reflects_the_model() {
    let (mut wm, a, b) = wm2();
    let x = add(&mut wm, "x");
    wm.set_title(x, "Éditeur");
    act(&mut wm, Action::View(1 << 1));
    let y = add(&mut wm, "y");
    act(&mut wm, Action::ToggleTag(1 << 4));
    wm.set_urgent(x, true);
    wm.set_status("12:00");

    let v = wm.bar_view(a).unwrap();
    assert_eq!(v.tags.len(), 9);
    assert_eq!(
        v.tags.iter().map(|t| t.selected).collect::<Vec<_>>(),
        [false, true, false, false, false, false, false, false, false]
    );
    assert_eq!(
        v.tags.iter().map(|t| t.occupied).collect::<Vec<_>>(),
        [true, true, false, false, true, false, false, false, false]
    );
    assert_eq!(
        v.tags.iter().map(|t| t.filled).collect::<Vec<_>>(),
        [false, true, false, false, true, false, false, false, false]
    );
    assert!(v.tags[0].urgent && !v.tags[1].urgent);
    assert_eq!(v.layout_symbol, "[]=");
    assert_eq!(v.title.as_deref(), Some("y"));
    assert_eq!(v.status.as_deref(), Some("12:00"));
    assert!(v.is_selected_monitor && !v.title_floating);

    let vb = wm.bar_view(b).unwrap();
    assert_eq!((vb.title, vb.status, vb.is_selected_monitor), (None, None, false));
    assert!(vb.tags.iter().all(|t| !t.occupied && !t.filled));
    assert_eq!(wm.bar_view(999), None);

    wm.focus_client(y);
    act(&mut wm, Action::ToggleFloating);
    assert!(wm.bar_view(a).unwrap().title_floating);
}

#[test]
fn default_status_is_the_version() {
    assert_eq!(wm().status(), concat!("rwdwm-", env!("CARGO_PKG_VERSION")));
}

#[test]
fn urgency_is_cleared_by_focus_and_refused_on_selected() {
    let mut wm = wm();
    let a = add(&mut wm, "a");
    let b = add(&mut wm, "b");
    wm.set_urgent(b, true);
    assert!(!wm.client(b).unwrap().is_urgent, "le client sélectionné ne devient pas urgent");
    wm.set_urgent(a, true);
    assert!(wm.client(a).unwrap().is_urgent);
    wm.focus_client(a);
    assert!(!wm.client(a).unwrap().is_urgent);
    // Urgence sur un tag masqué, effacée en y allant.
    act(&mut wm, Action::Tag(1 << 3));
    wm.set_urgent(a, true);
    assert!(wm.bar_view(mon(&wm).id).unwrap().tags[3].urgent);
    act(&mut wm, Action::View(1 << 3));
    assert!(!wm.client(a).unwrap().is_urgent);
}

// --------------------------------------------------------------- empilement

#[test]
fn stacking_orders_by_category_then_focus() {
    let mut wm = wm();
    let a = add(&mut wm, "a");
    let b = add(&mut wm, "b");
    let c = add(&mut wm, "c");
    let d = add(&mut wm, "d");
    let id = mon(&wm).id;
    wm.focus_client(b);
    act(&mut wm, Action::ToggleFloating);
    wm.focus_client(c);
    act(&mut wm, Action::ToggleFloating);
    wm.focus_client(a);
    let s = wm.stacking(id);
    // Pile de focus : a, c, b, d → du fond vers l'avant : d, b, c, a.
    assert_eq!(s.tiled, vec![d, a]);
    assert_eq!(s.floating, vec![b, c]);
    assert!(s.fullscreen.is_empty());
    act(&mut wm, Action::ToggleFullscreen);
    let s = wm.stacking(id);
    assert_eq!((s.tiled, s.fullscreen), (vec![d], vec![a]));
    act(&mut wm, Action::ToggleFullscreen);
    wm.focus_client(d);
    act(&mut wm, Action::Tag(1 << 1));
    let s = wm.stacking(id);
    assert_eq!(s.tiled, vec![a], "les clients masqués ne sont pas empilés");
    assert_eq!(wm.stacking(999), Stacking::default());
}

// ---------------------------------------------------------------- robustesse

/// Générateur pseudo-aléatoire déterministe (xorshift64*), sans dépendance.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: u64) -> u64 {
        (self.next() >> 33) % n
    }
}

#[test]
fn random_operations_preserve_invariants() {
    for seed in 1..=40u64 {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let mut cfg = Config::default();
        cfg.rules = vec![
            Rule { app_id: Some("float"), is_floating: true, ..Default::default() },
            Rule { app_id: Some("tag5"), tags: 1 << 4, monitor: Some(1), ..Default::default() },
        ];
        let mut wm = Wm::new(cfg);
        let mut mons = vec![wm.add_monitor("M0", SCREEN)];
        let mut next_x = 1280;
        let mut log: Vec<String> = Vec::new();

        for step in 0..600 {
            let ids: Vec<ClientId> = {
                let mut v: Vec<_> = wm.client_ids().collect();
                v.sort_unstable();
                v
            };
            let pick = |rng: &mut Rng| (!ids.is_empty()).then(|| ids[rng.below(ids.len() as u64) as usize]);
            let bit = 1u32 << rng.below(11);
            let dir = if rng.below(2) == 0 { 1 } else { -1 };
            let op = rng.below(30);
            let desc = match op {
                0..=3 => {
                    let app = ["term", "float", "tag5", "x"][rng.below(4) as usize];
                    let fixed = rng.below(8) == 0;
                    let size = if fixed { (200, 150) } else { (0, 0) };
                    let parent = if rng.below(6) == 0 { pick(&mut rng) } else { None };
                    wm.manage(NewClient {
                        app_id: app.into(),
                        title: app.into(),
                        parent,
                        min_size: size,
                        max_size: size,
                    });
                    format!("manage {app} fixed={fixed} parent={parent:?}")
                }
                4 | 5 => {
                    if let Some(id) = pick(&mut rng) {
                        wm.unmanage(id);
                    }
                    "unmanage".into()
                }
                6 => apply(&mut wm, Action::View(bit)),
                7 => apply(&mut wm, Action::View(0)),
                8 => apply(&mut wm, Action::ToggleView(bit)),
                9 => apply(&mut wm, Action::Tag(bit)),
                10 => apply(&mut wm, Action::ToggleTag(bit)),
                11 => apply(&mut wm, Action::FocusStack(dir)),
                12 => apply(&mut wm, Action::Zoom),
                13 => apply(
                    &mut wm,
                    Action::SetLayout(match rng.below(5) {
                        4 => None,
                        n => Some(n as usize),
                    }),
                ),
                14 => apply(&mut wm, Action::ToggleFloating),
                15 => apply(&mut wm, Action::ToggleFullscreen),
                16 => apply(&mut wm, Action::IncNMaster(dir)),
                17 => apply(&mut wm, Action::SetMFact(dir as f32 * 0.05)),
                18 => apply(&mut wm, Action::FocusMon(dir)),
                19 => apply(&mut wm, Action::TagMon(dir)),
                20 => apply(&mut wm, Action::ToggleBar),
                21 => {
                    if let Some(id) = pick(&mut rng) {
                        wm.focus_client(id);
                    }
                    "focus_client".into()
                }
                22 => {
                    if let Some(id) = pick(&mut rng) {
                        wm.mouse_move(id, rng.below(3000) as i32 - 200, rng.below(1200) as i32 - 100);
                        wm.mouse_release(id);
                    }
                    "mouse_move".into()
                }
                23 => {
                    if let Some(id) = pick(&mut rng) {
                        wm.mouse_resize(id, rng.below(1500) as i32 - 100, rng.below(1000) as i32 - 100);
                        wm.mouse_release(id);
                    }
                    "mouse_resize".into()
                }
                24 => {
                    if let Some(id) = pick(&mut rng) {
                        wm.set_fullscreen(id, rng.below(2) == 0);
                        wm.set_urgent(id, rng.below(2) == 0);
                        wm.set_natural_size(id, rng.below(900) as i32, rng.below(700) as i32);
                    }
                    "set_fullscreen/urgent/natural".into()
                }
                25 => {
                    if mons.len() < 3 {
                        mons.push(wm.add_monitor(&format!("M{}", mons.len()), Rect::new(next_x, 0, 1024, 768)));
                        next_x += 1024;
                    }
                    "add_monitor".into()
                }
                26 => {
                    let i = rng.below(mons.len() as u64) as usize;
                    if wm.remove_monitor(mons[i]) {
                        mons.remove(i);
                    }
                    "remove_monitor".into()
                }
                27 => {
                    let id = mons[rng.below(mons.len() as u64) as usize];
                    let s = wm.monitor(id).unwrap().screen;
                    let top = rng.below(40) as i32;
                    let s2 = Rect::new(s.x, s.y, 800 + rng.below(800) as i32, 600 + rng.below(400) as i32);
                    wm.set_monitor_geometry(id, s2, Rect::new(s2.x, s2.y + top, s2.w, s2.h - top));
                    "set_monitor_geometry".into()
                }
                28 => {
                    wm.select_monitor(mons[rng.below(mons.len() as u64) as usize]);
                    "select_monitor".into()
                }
                _ => apply(&mut wm, Action::KillClient),
            };
            log.push(desc);
            if let Err(e) = wm.check_invariants() {
                let tail = &log[log.len().saturating_sub(8)..];
                panic!("graine {seed}, étape {step} : {e}\ndernières opérations : {tail:#?}");
            }
            // Les lectures ne doivent jamais paniquer et rester cohérentes.
            for m in wm.monitors() {
                let s = wm.stacking(m.id);
                let visible = m.clients.iter().filter(|&&c| wm.is_visible(c)).count();
                assert_eq!(s.tiled.len() + s.floating.len() + s.fullscreen.len(), visible);
                let view = wm.bar_view(m.id).unwrap();
                assert_eq!(view.tags.iter().filter(|t| t.selected).count() as u32, m.current_tagset().count_ones());
                for id in s.tiled {
                    let g = wm.client(id).unwrap().geom;
                    let lt = wm.config().layouts[m.current_layout()].kind;
                    if lt != LayoutKind::Floating {
                        assert_eq!(
                            g.intersection_area(&m.window_area),
                            g.w as i64 * g.h as i64,
                            "client tuilé hors zone"
                        );
                    }
                }
            }
        }
    }
}

fn apply(wm: &mut Wm, action: Action) -> String {
    wm.apply(&action);
    format!("{action:?}")
}
