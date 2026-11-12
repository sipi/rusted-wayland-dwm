//! Types de la configuration — l'équivalent typé de `config.h`.
//!
//! Les **valeurs** vivent dans `crates/rwdwm/src/config.rs` ; ce module ne
//! définit que la forme des données et des valeurs par défaut minimales
//! (apparence et dispositions de dwm, sans raccourcis) utilisées par les tests.

use crate::action::Action;
use crate::model::TagMask;

/// Couleur opaque `0xRRGGBB`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Color(pub u32);

impl Color {
    /// Valeur ARGB8888 opaque (`0xFFRRGGBB`).
    pub const fn argb(self) -> u32 {
        0xff00_0000 | (self.0 & 0x00ff_ffff)
    }

    /// Composantes `[r, g, b, a]` en flottants `0.0..=1.0`.
    pub fn rgba_f32(self) -> [f32; 4] {
        let c = self.0;
        [((c >> 16) & 0xff) as f32 / 255.0, ((c >> 8) & 0xff) as f32 / 255.0, (c & 0xff) as f32 / 255.0, 1.0]
    }
}

/// Jeu de couleurs (`SchemeNorm` / `SchemeSel` de dwm).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scheme {
    /// Texte.
    pub fg: Color,
    /// Fond.
    pub bg: Color,
    /// Bordure des clients.
    pub border: Color,
}

/// Toutes les couleurs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Colors {
    /// Éléments non sélectionnés.
    pub norm: Scheme,
    /// Éléments sélectionnés.
    pub sel: Scheme,
    /// Bordure d'un client urgent.
    pub urgent_border: Color,
    /// Fond d'écran uni.
    pub root: Color,
}

/// Algorithme de placement d'une disposition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutKind {
    /// Zone maître à gauche, pile à droite.
    Tile,
    /// Aucun placement automatique.
    Floating,
    /// Un client à la fois, sur toute la zone utile.
    Monocle,
}

/// Disposition : symbole affiché dans la barre + algorithme.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    /// Symbole (`[]=`, `><>`, `[M]`).
    pub symbol: &'static str,
    /// Algorithme.
    pub kind: LayoutKind,
}

/// Règle appliquée à l'apparition d'un client (`rules[]` de dwm).
///
/// Les critères sont des **sous-chaînes** ; `None` accepte tout.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Rule {
    /// Sous-chaîne de l'`app_id` (équivalent de la classe X11).
    pub app_id: Option<&'static str>,
    /// Sous-chaîne du titre.
    pub title: Option<&'static str>,
    /// Tags à affecter (`0` = tagset courant du moniteur).
    pub tags: TagMask,
    /// Le client doit-il être flottant ?
    pub is_floating: bool,
    /// Indice du moniteur cible (`None` = moniteur sélectionné).
    pub monitor: Option<usize>,
}

/// Règle appliquée à l'apparition d'un moniteur (`monrules[]` de dwl).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MonitorRule {
    /// Sous-chaîne du nom de la sortie (`eDP-1`, `HDMI-A-1`…) ; `None` = toutes.
    pub name: Option<&'static str>,
    /// `mfact` initial.
    pub mfact: Option<f32>,
    /// `nmaster` initial.
    pub nmaster: Option<i32>,
    /// Indice de la disposition initiale.
    pub layout: Option<usize>,
    /// Facteur d'échelle entier de la sortie.
    pub scale: Option<i32>,
}

/// Masque de modificateurs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Mods(pub u8);

impl Mods {
    /// Aucun modificateur.
    pub const NONE: Mods = Mods(0);
    /// Majuscule.
    pub const SHIFT: Mods = Mods(1);
    /// Contrôle.
    pub const CTRL: Mods = Mods(2);
    /// Alt (`Mod1`).
    pub const ALT: Mods = Mods(4);
    /// Logo / Super (`Mod4`).
    pub const LOGO: Mods = Mods(8);

    /// Union de deux masques (utilisable en contexte `const`).
    pub const fn or(self, other: Mods) -> Mods {
        Mods(self.0 | other.0)
    }

    /// `self` contient-il tous les bits de `other` ?
    pub const fn contains(self, other: Mods) -> bool {
        self.0 & other.0 == other.0
    }
}

impl std::ops::BitOr for Mods {
    type Output = Mods;
    fn bitor(self, rhs: Mods) -> Mods {
        self.or(rhs)
    }
}

/// Ce qui déclenche un raccourci clavier.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trigger {
    /// Keysym de **niveau 0** de la touche (sans Maj) : `sym('c')` correspond
    /// aussi bien à `c` qu'à `Maj+c`.
    Sym(u32),
    /// Keycode XKB (touche physique, indépendante de la disposition).
    Code(u32),
}

/// Raccourci clavier (`keys[]` de dwm).
#[derive(Clone, Debug, PartialEq)]
pub struct Key {
    /// Modificateurs exacts requis.
    pub mods: Mods,
    /// Touche.
    pub trigger: Trigger,
    /// Action.
    pub action: Action,
}

/// Zone cliquée (`Clk*` de dwm).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Click {
    /// Un tag de la barre.
    TagBar,
    /// Le symbole de disposition.
    LtSymbol,
    /// Le texte de statut.
    StatusText,
    /// Le titre de la fenêtre.
    WinTitle,
    /// Un client.
    ClientWin,
    /// Le fond.
    RootWin,
}

/// Bouton gauche (`BTN_LEFT`).
pub const BTN_LEFT: u32 = 0x110;
/// Bouton droit (`BTN_RIGHT`).
pub const BTN_RIGHT: u32 = 0x111;
/// Bouton du milieu (`BTN_MIDDLE`).
pub const BTN_MIDDLE: u32 = 0x112;

/// Liaison de bouton de souris (`buttons[]` de dwm).
#[derive(Clone, Debug, PartialEq)]
pub struct Button {
    /// Zone cliquée.
    pub click: Click,
    /// Modificateurs exacts requis.
    pub mods: Mods,
    /// Code evdev du bouton.
    pub button: u32,
    /// Action. Pour `Click::TagBar`, un masque de tags nul est remplacé par le
    /// tag cliqué.
    pub action: Action,
}

/// Disposition du clavier (noms XKB). Une chaîne vide laisse libxkbcommon
/// utiliser `XKB_DEFAULT_*` puis ses valeurs par défaut.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Keyboard {
    /// Règles (`evdev`).
    pub rules: &'static str,
    /// Modèle (`pc105`).
    pub model: &'static str,
    /// Disposition (`fr`, `us`…).
    pub layout: &'static str,
    /// Variante (`oss`, `bepo`…).
    pub variant: &'static str,
    /// Options (`ctrl:nocaps`…).
    pub options: Option<&'static str>,
    /// Répétitions par seconde.
    pub repeat_rate: i32,
    /// Délai avant répétition, en millisecondes.
    pub repeat_delay: i32,
}

/// Réglages du pavé tactile (libinput).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Touchpad {
    /// Tapoter pour cliquer.
    pub tap_to_click: bool,
    /// Défilement naturel.
    pub natural_scrolling: bool,
    /// Désactiver pendant la frappe.
    pub disable_while_typing: bool,
}

/// La configuration complète.
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    /// Largeur des bordures, en pixels.
    pub border_px: i32,
    /// Seuil d'aimantation / de détachement à la souris, en pixels.
    pub snap: i32,
    /// Barre visible au démarrage.
    pub show_bar: bool,
    /// Barre en haut (`false` : en bas).
    pub top_bar: bool,
    /// Couleurs.
    pub colors: Colors,
    /// Noms des tags (31 au plus).
    pub tags: Vec<&'static str>,
    /// Règles de clients.
    pub rules: Vec<Rule>,
    /// Part de la zone maître, `0.05..=0.95`.
    pub mfact: f32,
    /// Nombre de clients en zone maître.
    pub nmaster: i32,
    /// Interdire `focusstack` quand le client sélectionné est en plein écran.
    pub lock_fullscreen: bool,
    /// Le focus suit la souris.
    pub sloppy_focus: bool,
    /// Dispositions ; la première est celle par défaut, la deuxième la
    /// « précédente » initiale.
    pub layouts: Vec<Layout>,
    /// Règles de moniteurs.
    pub monitor_rules: Vec<MonitorRule>,
    /// Raccourcis clavier.
    pub keys: Vec<Key>,
    /// Liaisons de boutons.
    pub buttons: Vec<Button>,
    /// Clavier.
    pub keyboard: Keyboard,
    /// Pavé tactile.
    pub touchpad: Touchpad,
}

impl Config {
    /// Masque couvrant tous les tags configurés (`TAGMASK` de dwm).
    pub fn tag_mask(&self) -> TagMask {
        let n = self.tags.len().clamp(1, 31) as u32;
        (1u32 << n) - 1
    }
}

impl Default for Config {
    /// Valeurs de `config.def.h` (dwm 6.5), sans raccourcis ni règles.
    fn default() -> Self {
        const GRAY1: Color = Color(0x222222);
        const GRAY2: Color = Color(0x444444);
        const GRAY3: Color = Color(0xbbbbbb);
        const GRAY4: Color = Color(0xeeeeee);
        const CYAN: Color = Color(0x005577);
        Config {
            border_px: 1,
            snap: 32,
            show_bar: true,
            top_bar: true,
            colors: Colors {
                norm: Scheme { fg: GRAY3, bg: GRAY1, border: GRAY2 },
                sel: Scheme { fg: GRAY4, bg: CYAN, border: CYAN },
                urgent_border: Color(0xff0000),
                root: GRAY1,
            },
            tags: vec!["1", "2", "3", "4", "5", "6", "7", "8", "9"],
            rules: Vec::new(),
            mfact: 0.55,
            nmaster: 1,
            lock_fullscreen: true,
            sloppy_focus: true,
            layouts: vec![
                Layout { symbol: "[]=", kind: LayoutKind::Tile },
                Layout { symbol: "><>", kind: LayoutKind::Floating },
                Layout { symbol: "[M]", kind: LayoutKind::Monocle },
            ],
            monitor_rules: Vec::new(),
            keys: Vec::new(),
            buttons: Vec::new(),
            keyboard: Keyboard { repeat_rate: 25, repeat_delay: 600, ..Keyboard::default() },
            touchpad: Touchpad { tap_to_click: true, natural_scrolling: false, disable_while_typing: true },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_mask_matches_tag_count() {
        let mut c = Config::default();
        assert_eq!(c.tag_mask(), 0b1_1111_1111);
        c.tags.truncate(4);
        assert_eq!(c.tag_mask(), 0b1111);
    }

    #[test]
    fn colors_convert() {
        assert_eq!(Color(0x005577).argb(), 0xff005577);
        let [r, g, b, a] = Color(0xff8000).rgba_f32();
        assert!((r - 1.0).abs() < 1e-6 && (g - 128.0 / 255.0).abs() < 1e-6 && b == 0.0 && a == 1.0);
    }

    #[test]
    fn mods_combine() {
        let m = Mods::ALT | Mods::SHIFT;
        assert!(m.contains(Mods::ALT) && m.contains(Mods::SHIFT) && !m.contains(Mods::CTRL));
        assert_eq!(Mods::ALT.or(Mods::SHIFT), m);
    }
}
