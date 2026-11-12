//! Quelques keysyms XKB usuels (valeurs normalisées de `xkbcommon-keysyms.h`).
//!
//! Les lettres et chiffres ASCII ont pour keysym leur code ASCII : utiliser
//! [`sym`] pour les obtenir (`sym('p')`).

#![allow(missing_docs)]

pub const RETURN: u32 = 0xff0d;
pub const TAB: u32 = 0xff09;
pub const ESCAPE: u32 = 0xff1b;
pub const SPACE: u32 = 0x0020;
pub const COMMA: u32 = 0x002c;
pub const PERIOD: u32 = 0x002e;
pub const F1: u32 = 0xffbe;
pub const PRINT: u32 = 0xff61;
/// `XF86Switch_VT_1` ; les suivants sont consécutifs jusqu'à `XF86Switch_VT_12`.
pub const XF86_SWITCH_VT_1: u32 = 0x1008_fe01;

/// Keysym d'un caractère ASCII imprimable (lettre minuscule, chiffre, ponctuation).
pub const fn sym(c: char) -> u32 {
    c as u32
}

/// Keycode XKB d'une touche physique à partir de son code evdev
/// (`KEY_*` de `linux/input-event-codes.h`) : XKB = evdev + 8.
pub const fn evdev(code: u32) -> u32 {
    code + 8
}

/// Codes evdev de la rangée de chiffres : `KEY_1` (2) … `KEY_9` (10), `KEY_0` (11).
pub const fn digit_keycode(digit: u32) -> u32 {
    evdev(if digit == 0 { 11 } else { digit + 1 })
}
