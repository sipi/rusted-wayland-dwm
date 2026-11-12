//! **La configuration de rwdwm** — l'équivalent de `config.h`.
//!
//! Comme dwm, rwdwm se configure en modifiant ce fichier puis en recompilant
//! (`cargo build --release`). Tout est typé : une erreur de configuration est
//! une erreur de compilation.
//!
//! Les valeurs ci-dessous reprennent `config.def.h` de dwm 6.5 ; les écarts
//! (terminal, lanceur, plein écran) sont signalés.

use rwdwm_core::action::Action;
use rwdwm_core::config::{
    Button, Click, Color, Colors, Config, Key, Keyboard, Layout, LayoutKind, Mods, MonitorRule, Rule, Scheme, Touchpad,
    Trigger, BTN_LEFT, BTN_MIDDLE, BTN_RIGHT,
};
use rwdwm_core::keysyms::{digit_keycode, evdev, sym, RETURN, SPACE, TAB};
use rwdwm_core::model::TagMask;

/// Le modificateur principal. `Mods::ALT` comme dwm ; `Mods::LOGO` pour la
/// touche « Super ».
pub const MODKEY: Mods = Mods::ALT;

/// Terminal (`st` dans dwm ; `foot` est son équivalent Wayland).
const TERMINAL: &[&str] = &["foot"];
/// Lanceur (`dmenu_run` dans dwm ; `wmenu-run` est son équivalent Wayland).
const MENU: &[&str] = &["wmenu-run"];

// Touches physiques situées à droite de `N` sur un clavier QWERTY (`,` et
// `.`) : liées par code pour rester au même endroit sur AZERTY.
const KEY_COMMA: u32 = evdev(51);
const KEY_DOT: u32 = evdev(52);

/// Les quatre raccourcis d'un tag (`TAGKEYS` de dwm), liés à la touche
/// **physique** : sur AZERTY, la rangée de chiffres fonctionne sans Maj.
fn tag_keys(keys: &mut Vec<Key>, digit: u32, tag: usize) {
    let mask: TagMask = 1 << tag;
    let code = Trigger::Code(digit_keycode(digit));
    keys.push(Key { mods: MODKEY, trigger: code, action: Action::View(mask) });
    keys.push(Key { mods: MODKEY.or(Mods::CTRL), trigger: code, action: Action::ToggleView(mask) });
    keys.push(Key { mods: MODKEY.or(Mods::SHIFT), trigger: code, action: Action::Tag(mask) });
    keys.push(Key { mods: MODKEY.or(Mods::CTRL).or(Mods::SHIFT), trigger: code, action: Action::ToggleTag(mask) });
}

/// Construit la configuration.
pub fn config() -> Config {
    // ---------------------------------------------------------- apparence
    let gray1 = Color(0x222222);
    let gray2 = Color(0x444444);
    let gray3 = Color(0xbbbbbb);
    let gray4 = Color(0xeeeeee);
    let cyan = Color(0x005577);
    let colors = Colors {
        //                 texte      fond       bordure
        norm: Scheme { fg: gray3, bg: gray1, border: gray2 },
        sel: Scheme { fg: gray4, bg: cyan, border: cyan },
        urgent_border: Color(0xff0000),
        root: gray1,
    };

    // --------------------------------------------------------------- tags
    let tags = vec!["1", "2", "3", "4", "5", "6", "7", "8", "9"];

    // ------------------------------------------------------------- règles
    // Les critères sont des sous-chaînes de l'app_id et du titre
    // (`wlrctl toplevel list` ou les journaux en mode `-v` les affichent).
    let rules = vec![
        Rule { app_id: Some("Gimp"), is_floating: true, ..Rule::default() },
        Rule { app_id: Some("firefox"), tags: 1 << 8, ..Rule::default() },
    ];

    // ------------------------------------------------------- dispositions
    let layouts = vec![
        Layout { symbol: "[]=", kind: LayoutKind::Tile }, // la première est celle par défaut
        Layout { symbol: "><>", kind: LayoutKind::Floating },
        Layout { symbol: "[M]", kind: LayoutKind::Monocle },
    ];

    // ---------------------------------------------------------- moniteurs
    // Exemple : écran HiDPI intégré à l'échelle 2.
    //   MonitorRule { name: Some("eDP-1"), scale: Some(2), ..MonitorRule::default() }
    let monitor_rules: Vec<MonitorRule> = vec![];

    // ------------------------------------------------------------ touches
    let shift = MODKEY.or(Mods::SHIFT);
    let key = |mods: Mods, s: u32, action: Action| Key { mods, trigger: Trigger::Sym(s), action };
    let code = |mods: Mods, c: u32, action: Action| Key { mods, trigger: Trigger::Code(c), action };
    let mut keys = vec![
        key(MODKEY, sym('p'), Action::spawn(MENU)),
        key(shift, RETURN, Action::spawn(TERMINAL)),
        key(MODKEY, sym('b'), Action::ToggleBar),
        key(MODKEY, sym('j'), Action::FocusStack(1)),
        key(MODKEY, sym('k'), Action::FocusStack(-1)),
        key(MODKEY, sym('i'), Action::IncNMaster(1)),
        key(MODKEY, sym('d'), Action::IncNMaster(-1)),
        key(MODKEY, sym('h'), Action::SetMFact(-0.05)),
        key(MODKEY, sym('l'), Action::SetMFact(0.05)),
        key(MODKEY, RETURN, Action::Zoom),
        key(MODKEY, TAB, Action::View(0)),
        key(shift, sym('c'), Action::KillClient),
        key(MODKEY, sym('t'), Action::SetLayout(Some(0))),
        key(MODKEY, sym('f'), Action::SetLayout(Some(1))),
        key(MODKEY, sym('m'), Action::SetLayout(Some(2))),
        key(MODKEY, SPACE, Action::SetLayout(None)),
        key(shift, SPACE, Action::ToggleFloating),
        key(MODKEY, sym('e'), Action::ToggleFullscreen), // absent de dwm
        code(MODKEY, digit_keycode(0), Action::View(!0)),
        code(shift, digit_keycode(0), Action::Tag(!0)),
        code(MODKEY, KEY_COMMA, Action::FocusMon(-1)),
        code(MODKEY, KEY_DOT, Action::FocusMon(1)),
        code(shift, KEY_COMMA, Action::TagMon(-1)),
        code(shift, KEY_DOT, Action::TagMon(1)),
        key(shift, sym('q'), Action::Quit),
    ];
    for tag in 0..tags.len().min(9) {
        tag_keys(&mut keys, tag as u32 + 1, tag);
    }

    // ------------------------------------------------------------ boutons
    // Pour `Click::TagBar`, un masque nul désigne le tag cliqué.
    let button = |click: Click, mods: Mods, button: u32, action: Action| Button { click, mods, button, action };
    let buttons = vec![
        button(Click::LtSymbol, Mods::NONE, BTN_LEFT, Action::SetLayout(None)),
        button(Click::LtSymbol, Mods::NONE, BTN_RIGHT, Action::SetLayout(Some(2))),
        button(Click::WinTitle, Mods::NONE, BTN_MIDDLE, Action::Zoom),
        button(Click::StatusText, Mods::NONE, BTN_MIDDLE, Action::spawn(TERMINAL)),
        button(Click::ClientWin, MODKEY, BTN_LEFT, Action::MoveMouse),
        button(Click::ClientWin, MODKEY, BTN_MIDDLE, Action::ToggleFloating),
        button(Click::ClientWin, MODKEY, BTN_RIGHT, Action::ResizeMouse),
        button(Click::TagBar, Mods::NONE, BTN_LEFT, Action::View(0)),
        button(Click::TagBar, Mods::NONE, BTN_RIGHT, Action::ToggleView(0)),
        button(Click::TagBar, MODKEY, BTN_LEFT, Action::Tag(0)),
        button(Click::TagBar, MODKEY, BTN_RIGHT, Action::ToggleTag(0)),
    ];

    Config {
        border_px: 1,   // largeur des bordures
        snap: 32,       // seuil d'aimantation, en pixels
        show_bar: true, // false : pas de barre
        top_bar: true,  // false : barre en bas
        colors,
        tags,
        rules,
        mfact: 0.55,           // part de la zone maître [0.05..0.95]
        nmaster: 1,            // nombre de clients en zone maître
        lock_fullscreen: true, // interdit de changer de focus en plein écran
        sloppy_focus: true,    // le focus suit la souris
        layouts,
        monitor_rules,
        keys,
        buttons,
        // Chaînes vides : variables XKB_DEFAULT_* de l'environnement, puis
        // disposition par défaut du système. Exemple : layout: "fr".
        keyboard: Keyboard {
            rules: "",
            model: "",
            layout: "",
            variant: "",
            options: None,
            repeat_rate: 25,
            repeat_delay: 600,
        },
        touchpad: Touchpad { tap_to_click: true, natural_scrolling: false, disable_while_typing: true },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rwdwm_core::model::Wm;

    #[test]
    fn configuration_is_consistent() {
        let cfg = config();
        assert!((1..=31).contains(&cfg.tags.len()));
        assert!(!cfg.layouts.is_empty());
        assert!((0.05..=0.95).contains(&cfg.mfact));
        assert!(cfg.nmaster >= 0 && cfg.border_px >= 0);
        for key in &cfg.keys {
            if let Action::SetLayout(Some(i)) = key.action {
                assert!(i < cfg.layouts.len());
            }
        }
        // Le noyau accepte la configuration.
        let mut wm = Wm::new(cfg);
        wm.add_monitor("test", rwdwm_core::Rect::new(0, 0, 800, 600));
        wm.check_invariants().unwrap();
    }

    #[test]
    fn no_duplicate_key_bindings() {
        let cfg = config();
        for (i, a) in cfg.keys.iter().enumerate() {
            for b in &cfg.keys[i + 1..] {
                assert!(!(a.mods == b.mods && a.trigger == b.trigger), "raccourci en double : {a:?} / {b:?}");
            }
        }
    }

    #[test]
    fn every_tag_has_its_four_bindings() {
        let cfg = config();
        for tag in 0..9u32 {
            let code = Trigger::Code(digit_keycode(tag + 1));
            let actions: Vec<_> = cfg.keys.iter().filter(|k| k.trigger == code).map(|k| k.action.clone()).collect();
            let mask = 1 << tag;
            assert_eq!(
                actions,
                [Action::View(mask), Action::ToggleView(mask), Action::Tag(mask), Action::ToggleTag(mask)]
            );
        }
        // KEY_1 = 2 en evdev, soit 10 en XKB.
        assert_eq!(digit_keycode(1), 10);
        assert_eq!(digit_keycode(0), 19);
    }
}
