//! La barre d'état : modèle déclaratif, découpage en zones, dessin, clics.
//!
//! Transposition de `drawbar()` et de la partie « barre » de `buttonpress()`
//! de dwm. Le découpage en zones ([`segments`]) est partagé par le dessin
//! ([`draw`]) et le test de clic ([`click_at`]).
//!
//! Toutes les coordonnées sont en pixels logiques, relatives au coin supérieur
//! gauche de la barre.

use crate::config::{Colors, Scheme};
use crate::font::{self, FONT_H, FONT_W};

/// Hauteur de la barre : hauteur de police + 2 (`bh` de dwm).
pub const BAR_HEIGHT: i32 = FONT_H + 2;
/// Marge horizontale totale d'un texte (`lrpad` de dwm).
pub const LRPAD: i32 = FONT_H;
/// Décalage de l'indicateur d'occupation (`boxs` de dwm).
pub const BOX_OFFSET: i32 = FONT_H / 9;
/// Côté de l'indicateur d'occupation (`boxw` de dwm).
pub const BOX_SIZE: i32 = FONT_H / 6 + 2;

/// État d'un tag dans la barre.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TagView {
    /// Nom du tag.
    pub label: String,
    /// Le tag fait partie du tagset affiché.
    pub selected: bool,
    /// Au moins un client porte ce tag.
    pub occupied: bool,
    /// Un client urgent porte ce tag.
    pub urgent: bool,
    /// Le client sélectionné (du moniteur sélectionné) porte ce tag.
    pub filled: bool,
}

/// Contenu d'une barre. Deux vues égales produisent le même dessin : le
/// compositeur ne redessine que lorsque la vue change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BarView {
    /// Les tags, dans l'ordre.
    pub tags: Vec<TagView>,
    /// Symbole de la disposition courante.
    pub layout_symbol: String,
    /// Titre du client sélectionné du moniteur.
    pub title: Option<String>,
    /// Le client sélectionné est flottant.
    pub title_floating: bool,
    /// Le client sélectionné est de taille fixe.
    pub title_fixed: bool,
    /// La barre est celle du moniteur sélectionné.
    pub is_selected_monitor: bool,
    /// Texte de statut (moniteur sélectionné uniquement).
    pub status: Option<String>,
}

/// Nature d'une zone de la barre.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SegmentKind {
    /// Le tag d'indice donné.
    Tag(usize),
    /// Le symbole de disposition.
    LayoutSymbol,
    /// Le titre de la fenêtre.
    Title,
    /// Le texte de statut.
    Status,
}

/// Zone horizontale de la barre.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Segment {
    /// Nature.
    pub kind: SegmentKind,
    /// Abscisse de début.
    pub x: i32,
    /// Largeur.
    pub w: i32,
}

/// Cible d'un clic sur la barre.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BarClick {
    /// Le tag d'indice donné.
    Tag(usize),
    /// Le symbole de disposition.
    LayoutSymbol,
    /// Le titre de la fenêtre.
    Title,
    /// Le texte de statut.
    Status,
}

/// Largeur d'un texte avec ses marges (`TEXTW` de dwm).
pub fn textw(s: &str) -> i32 {
    font::text_width(s) + LRPAD
}

/// Largeur de la zone de statut : texte + 2 px de marge droite.
fn status_width(view: &BarView) -> i32 {
    view.status.as_deref().map_or(0, |s| font::text_width(s) + 2)
}

/// Découpe une barre de largeur `width` en zones, de gauche à droite.
///
/// La zone de titre n'existe que s'il reste plus de [`BAR_HEIGHT`] pixels,
/// comme dans dwm.
pub fn segments(view: &BarView, width: i32) -> Vec<Segment> {
    let mut out = Vec::with_capacity(view.tags.len() + 3);
    let mut x = 0;
    for (i, tag) in view.tags.iter().enumerate() {
        let w = textw(&tag.label);
        out.push(Segment { kind: SegmentKind::Tag(i), x, w });
        x += w;
    }
    let w = textw(&view.layout_symbol);
    out.push(Segment { kind: SegmentKind::LayoutSymbol, x, w });
    x += w;
    let tw = status_width(view).min((width - x).max(0));
    let title_w = width - tw - x;
    if title_w > BAR_HEIGHT {
        out.push(Segment { kind: SegmentKind::Title, x, w: title_w });
    }
    if tw > 0 {
        out.push(Segment { kind: SegmentKind::Status, x: width - tw, w: tw });
    }
    out
}

/// Zone de la barre située à l'abscisse `x` (`buttonpress` de dwm).
pub fn click_at(view: &BarView, width: i32, x: i32) -> BarClick {
    let mut end = 0;
    for (i, tag) in view.tags.iter().enumerate() {
        end += textw(&tag.label);
        if x < end {
            return BarClick::Tag(i);
        }
    }
    if x < end + textw(&view.layout_symbol) {
        BarClick::LayoutSymbol
    } else if view.status.is_some() && x >= width - status_width(view) {
        BarClick::Status
    } else {
        BarClick::Title
    }
}

/// Image de la barre, en ARGB8888 (octets `B, G, R, A`), lignes de haut en bas.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BarPixmap {
    /// Largeur en pixels physiques.
    pub width: i32,
    /// Hauteur en pixels physiques.
    pub height: i32,
    /// `width * height * 4` octets.
    pub data: Vec<u8>,
}

impl BarPixmap {
    /// Couleur `0xAARRGGBB` du pixel `(x, y)`.
    pub fn pixel(&self, x: i32, y: i32) -> u32 {
        let o = ((y * self.width + x) * 4) as usize;
        u32::from_le_bytes([self.data[o], self.data[o + 1], self.data[o + 2], self.data[o + 3]])
    }
}

/// Surface de dessin en pixels logiques.
struct Canvas {
    w: i32,
    h: i32,
    px: Vec<u32>,
}

impl Canvas {
    fn rect(&mut self, x: i32, y: i32, w: i32, h: i32, color: u32) {
        for yy in y.max(0)..(y + h).min(self.h) {
            let row = (yy * self.w) as usize;
            for xx in x.max(0)..(x + w).min(self.w) {
                self.px[row + xx as usize] = color;
            }
        }
    }

    fn outline(&mut self, x: i32, y: i32, w: i32, h: i32, color: u32) {
        self.rect(x, y, w, 1, color);
        self.rect(x, y + h - 1, w, 1, color);
        self.rect(x, y, 1, h, color);
        self.rect(x + w - 1, y, 1, h, color);
    }

    /// `drw_rect` de dwm : carré plein ou contour.
    fn indicator(&mut self, x: i32, filled: bool, color: u32) {
        if filled {
            self.rect(x + BOX_OFFSET, BOX_OFFSET, BOX_SIZE, BOX_SIZE, color);
        } else {
            self.outline(x + BOX_OFFSET, BOX_OFFSET, BOX_SIZE, BOX_SIZE, color);
        }
    }

    fn glyph(&mut self, x: i32, y: i32, c: char, color: u32) {
        for (row, bits) in font::glyph(c).iter().enumerate() {
            let yy = y + row as i32;
            if yy < 0 || yy >= self.h {
                continue;
            }
            for col in 0..FONT_W {
                let xx = x + col;
                if bits & (0x80 >> col) != 0 && xx >= 0 && xx < self.w {
                    self.px[(yy * self.w + xx) as usize] = color;
                }
            }
        }
    }

    /// `drw_text` de dwm : fond, puis texte décalé de `lpad`, tronqué avec
    /// `...` s'il dépasse la zone `[x, x + w)`.
    fn text(&mut self, x: i32, w: i32, lpad: i32, text: &str, fg: u32, bg: u32) {
        self.rect(x, 0, w, self.h, bg);
        let max_chars = ((w - lpad).max(0) / FONT_W) as usize;
        let count = text.chars().count();
        let (shown, ellipsis) = if count <= max_chars {
            (count, 0)
        } else if max_chars > 3 {
            (max_chars - 3, 3)
        } else {
            (max_chars, 0)
        };
        let y = (self.h - FONT_H) / 2;
        let mut cx = x + lpad;
        for c in text.chars().take(shown).chain(std::iter::repeat_n('.', ellipsis)) {
            self.glyph(cx, y, c, fg);
            cx += FONT_W;
        }
    }
}

/// Dessine la barre.
///
/// - `width` : largeur logique ;
/// - `scale` : facteur d'échelle entier (l'image fait `width * scale` pixels
///   de large et `BAR_HEIGHT * scale` de haut).
pub fn draw(view: &BarView, width: i32, scale: i32, colors: &Colors) -> BarPixmap {
    let width = width.max(1);
    let scale = scale.max(1);
    let Colors { norm, sel, .. } = *colors;
    let mut cv = Canvas { w: width, h: BAR_HEIGHT, px: vec![norm.bg.argb(); (width * BAR_HEIGHT) as usize] };

    // Le statut est dessiné en premier : sur une barre trop étroite, les tags
    // le recouvrent (comportement de dwm).
    for seg in segments(view, width).iter().rev() {
        match seg.kind {
            SegmentKind::Status => {
                let s = view.status.as_deref().unwrap_or("");
                cv.text(seg.x, seg.w, 0, s, norm.fg.argb(), norm.bg.argb());
            }
            SegmentKind::Tag(i) => {
                let tag = &view.tags[i];
                let scheme = if tag.selected { sel } else { norm };
                let (fg, bg) = fg_bg(scheme, tag.urgent);
                cv.text(seg.x, seg.w, LRPAD / 2, &tag.label, fg, bg);
                if tag.occupied {
                    cv.indicator(seg.x, tag.filled, fg);
                }
            }
            SegmentKind::LayoutSymbol => {
                cv.text(seg.x, seg.w, LRPAD / 2, &view.layout_symbol, norm.fg.argb(), norm.bg.argb());
            }
            SegmentKind::Title => match &view.title {
                Some(title) => {
                    let scheme = if view.is_selected_monitor { sel } else { norm };
                    let (fg, bg) = fg_bg(scheme, false);
                    cv.text(seg.x, seg.w, LRPAD / 2, title, fg, bg);
                    if view.title_floating {
                        cv.indicator(seg.x, view.title_fixed, fg);
                    }
                }
                None => cv.rect(seg.x, 0, seg.w, BAR_HEIGHT, norm.bg.argb()),
            },
        }
    }

    let (pw, ph) = (width * scale, BAR_HEIGHT * scale);
    let mut data = Vec::with_capacity((pw * ph * 4) as usize);
    for y in 0..ph {
        let row = ((y / scale) * width) as usize;
        for x in 0..pw {
            data.extend_from_slice(&cv.px[row + (x / scale) as usize].to_le_bytes());
        }
    }
    BarPixmap { width: pw, height: ph, data }
}

/// Couleurs de texte et de fond d'un jeu, éventuellement inversées (urgence).
fn fg_bg(scheme: Scheme, invert: bool) -> (u32, u32) {
    if invert {
        (scheme.bg.argb(), scheme.fg.argb())
    } else {
        (scheme.fg.argb(), scheme.bg.argb())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn view() -> BarView {
        BarView {
            tags: (1..=9)
                .map(|i| TagView {
                    label: i.to_string(),
                    selected: i == 1,
                    occupied: false,
                    urgent: false,
                    filled: false,
                })
                .collect(),
            layout_symbol: "[]=".into(),
            title: Some("foot".into()),
            title_floating: false,
            title_fixed: false,
            is_selected_monitor: true,
            status: Some("rwdwm-0.1.0".into()),
        }
    }

    const NORM_BG: u32 = 0xff222222;
    const NORM_FG: u32 = 0xffbbbbbb;
    const SEL_BG: u32 = 0xff005577;
    const SEL_FG: u32 = 0xffeeeeee;

    fn count(p: &BarPixmap, x0: i32, x1: i32, color: u32) -> usize {
        (0..p.height).flat_map(|y| (x0..x1).map(move |x| (x, y))).filter(|&(x, y)| p.pixel(x, y) == color).count()
    }

    #[test]
    fn dwm_metrics() {
        assert_eq!(BAR_HEIGHT, 18);
        assert_eq!(LRPAD, 16);
        assert_eq!((BOX_OFFSET, BOX_SIZE), (1, 4));
        assert_eq!(textw("1"), 24);
        assert_eq!(textw("[]="), 40);
    }

    #[test]
    fn segments_layout() {
        let segs = segments(&view(), 1280);
        assert_eq!(segs.len(), 12);
        assert_eq!(segs[0], Segment { kind: SegmentKind::Tag(0), x: 0, w: 24 });
        assert_eq!(segs[8], Segment { kind: SegmentKind::Tag(8), x: 192, w: 24 });
        assert_eq!(segs[9], Segment { kind: SegmentKind::LayoutSymbol, x: 216, w: 40 });
        // statut : 11 caractères × 8 + 2
        assert_eq!(segs[11], Segment { kind: SegmentKind::Status, x: 1280 - 90, w: 90 });
        assert_eq!(segs[10], Segment { kind: SegmentKind::Title, x: 256, w: 1280 - 90 - 256 });
        // Les zones pavent exactement la barre.
        assert_eq!(segs.iter().map(|s| s.w).sum::<i32>(), 1280);
    }

    #[test]
    fn no_status_on_unselected_monitor() {
        let mut v = view();
        v.status = None;
        let segs = segments(&v, 800);
        assert!(segs.iter().all(|s| s.kind != SegmentKind::Status));
        assert_eq!(segs.last().unwrap(), &Segment { kind: SegmentKind::Title, x: 256, w: 544 });
    }

    #[test]
    fn narrow_bar_drops_title() {
        let segs = segments(&view(), 300);
        assert!(segs.iter().all(|s| s.kind != SegmentKind::Title));
        let status = segs.iter().find(|s| s.kind == SegmentKind::Status).unwrap();
        assert_eq!((status.x, status.w), (256, 44));
    }

    #[test]
    fn clicks() {
        let v = view();
        assert_eq!(click_at(&v, 1280, 0), BarClick::Tag(0));
        assert_eq!(click_at(&v, 1280, 23), BarClick::Tag(0));
        assert_eq!(click_at(&v, 1280, 24), BarClick::Tag(1));
        assert_eq!(click_at(&v, 1280, 215), BarClick::Tag(8));
        assert_eq!(click_at(&v, 1280, 216), BarClick::LayoutSymbol);
        assert_eq!(click_at(&v, 1280, 255), BarClick::LayoutSymbol);
        assert_eq!(click_at(&v, 1280, 256), BarClick::Title);
        assert_eq!(click_at(&v, 1280, 1189), BarClick::Title);
        assert_eq!(click_at(&v, 1280, 1190), BarClick::Status);
        assert_eq!(click_at(&v, 1280, 1279), BarClick::Status);
    }

    #[test]
    fn clicks_agree_with_segments() {
        let v = view();
        for seg in segments(&v, 1000) {
            let expected = match seg.kind {
                SegmentKind::Tag(i) => BarClick::Tag(i),
                SegmentKind::LayoutSymbol => BarClick::LayoutSymbol,
                SegmentKind::Title => BarClick::Title,
                SegmentKind::Status => BarClick::Status,
            };
            assert_eq!(click_at(&v, 1000, seg.x), expected);
            assert_eq!(click_at(&v, 1000, seg.x + seg.w - 1), expected);
        }
    }

    #[test]
    fn draw_uses_dwm_colors() {
        let colors = Config::default().colors;
        let p = draw(&view(), 1280, 1, &colors);
        assert_eq!((p.width, p.height), (1280, 18));
        assert_eq!(p.data.len(), 1280 * 18 * 4);
        // Tag 1 sélectionné : fond cyan, texte clair.
        assert_eq!(p.pixel(0, 0), SEL_BG);
        assert!(count(&p, 0, 24, SEL_FG) > 0);
        assert_eq!(count(&p, 0, 24, NORM_BG), 0);
        // Tag 2 non sélectionné : fond gris, texte gris clair.
        assert_eq!(p.pixel(24, 0), NORM_BG);
        assert!(count(&p, 24, 48, NORM_FG) > 0);
        // Titre du moniteur sélectionné : fond cyan.
        assert_eq!(p.pixel(256, 0), SEL_BG);
        assert_eq!(p.pixel(1189, 17), SEL_BG);
        // Statut : schéma normal.
        assert_eq!(p.pixel(1190, 0), NORM_BG);
        assert!(count(&p, 1190, 1280, NORM_FG) > 0);
    }

    #[test]
    fn occupied_indicator_outline_and_filled() {
        let colors = Config::default().colors;
        let mut v = view();
        v.tags[1].occupied = true;
        let p = draw(&v, 1280, 1, &colors);
        // Contour 4×4 en (25..29, 1..5) : coins allumés, centre éteint.
        assert_eq!(p.pixel(25, 1), NORM_FG);
        assert_eq!(p.pixel(28, 4), NORM_FG);
        assert_eq!(p.pixel(26, 2), NORM_BG);
        v.tags[1].filled = true;
        let p = draw(&v, 1280, 1, &colors);
        assert_eq!(p.pixel(26, 2), NORM_FG);
        // Tag non occupé : pas d'indicateur.
        assert_eq!(p.pixel(49, 1), NORM_BG);
    }

    #[test]
    fn urgent_tag_is_inverted() {
        let colors = Config::default().colors;
        let mut v = view();
        v.tags[2].urgent = true;
        let p = draw(&v, 1280, 1, &colors);
        assert_eq!(p.pixel(48, 0), NORM_FG);
        assert!(count(&p, 48, 72, NORM_BG) > 0);
    }

    #[test]
    fn title_scheme_and_floating_marker() {
        let colors = Config::default().colors;
        let mut v = view();
        v.is_selected_monitor = false;
        v.status = None;
        let p = draw(&v, 800, 1, &colors);
        assert_eq!(p.pixel(256, 0), NORM_BG);
        v.is_selected_monitor = true;
        v.title_floating = true;
        let p = draw(&v, 800, 1, &colors);
        assert_eq!(p.pixel(257, 1), SEL_FG);
        assert_eq!(p.pixel(258, 2), SEL_BG);
        v.title_fixed = true;
        assert_eq!(draw(&v, 800, 1, &colors).pixel(258, 2), SEL_FG);
        v.title = None;
        assert_eq!(draw(&v, 800, 1, &colors).pixel(256, 0), NORM_BG);
    }

    #[test]
    fn long_title_is_truncated_with_ellipsis() {
        let colors = Config::default().colors;
        let mut v = view();
        v.status = None;
        v.title = Some("x".repeat(500));
        let long = draw(&v, 400, 1, &colors);
        // Zone de titre : 256..400, soit 144 px ; texte à partir de 264 :
        // 17 caractères dont les 3 derniers sont des points.
        v.title = Some(format!("{}...", "x".repeat(14)));
        assert_eq!(long, draw(&v, 400, 1, &colors));
        // Rien n'est dessiné au-delà du dernier caractère.
        assert_eq!(count(&long, 264 + 17 * 8, 400, SEL_FG), 0);
    }

    #[test]
    fn scale_doubles_pixels() {
        let colors = Config::default().colors;
        let p1 = draw(&view(), 300, 1, &colors);
        let p2 = draw(&view(), 300, 2, &colors);
        assert_eq!((p2.width, p2.height), (600, 36));
        for y in 0..18 {
            for x in 0..300 {
                let c = p1.pixel(x, y);
                assert_eq!(p2.pixel(2 * x, 2 * y), c);
                assert_eq!(p2.pixel(2 * x + 1, 2 * y + 1), c);
            }
        }
    }

    #[test]
    fn unknown_glyphs_do_not_panic() {
        let colors = Config::default().colors;
        let mut v = view();
        v.title = Some("日本語 😀 été".into());
        let p = draw(&v, 600, 1, &colors);
        assert!(count(&p, 256, 500, SEL_FG) > 0);
    }

    #[test]
    fn degenerate_widths_do_not_panic() {
        let colors = Config::default().colors;
        for w in [0, 1, 7, 17, 100] {
            let p = draw(&view(), w, 1, &colors);
            assert_eq!(p.width, w.max(1));
        }
    }
}
