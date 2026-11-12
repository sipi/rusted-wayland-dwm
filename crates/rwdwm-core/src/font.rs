//! Police bitmap embarquée : **Spleen 8×16** (© Frédéric Cambus, BSD-2-Clause,
//! voir `assets/LICENSE-spleen.txt`).
//!
//! La table est produite par `tools/gen_font.py` : un en-tête `u32` (nombre de
//! glyphes) suivi d'entrées de 20 octets `{ u32 point de code ; 16 lignes }`
//! triées par point de code. Dans chaque ligne, le bit 7 est le pixel de gauche.

/// Largeur d'un glyphe, en pixels.
pub const FONT_W: i32 = 8;
/// Hauteur d'un glyphe, en pixels.
pub const FONT_H: i32 = 16;

static DATA: &[u8] = include_bytes!("../assets/spleen-8x16.bin");
const ENTRY: usize = 4 + FONT_H as usize;

fn count() -> usize {
    u32::from_le_bytes([DATA[0], DATA[1], DATA[2], DATA[3]]) as usize
}

fn codepoint_at(i: usize) -> u32 {
    let o = 4 + i * ENTRY;
    u32::from_le_bytes([DATA[o], DATA[o + 1], DATA[o + 2], DATA[o + 3]])
}

fn lookup(cp: u32) -> Option<&'static [u8]> {
    let (mut lo, mut hi) = (0usize, count());
    while lo < hi {
        let mid = (lo + hi) / 2;
        match codepoint_at(mid).cmp(&cp) {
            std::cmp::Ordering::Equal => {
                let o = 4 + mid * ENTRY + 4;
                return Some(&DATA[o..o + FONT_H as usize]);
            }
            std::cmp::Ordering::Less => lo = mid + 1,
            std::cmp::Ordering::Greater => hi = mid,
        }
    }
    None
}

/// La police possède-t-elle un glyphe pour `c` ?
pub fn has_glyph(c: char) -> bool {
    lookup(c as u32).is_some()
}

/// Les 16 lignes du glyphe de `c` ; `?` si la police ne le possède pas.
pub fn glyph(c: char) -> &'static [u8] {
    lookup(c as u32).or_else(|| lookup('?' as u32)).expect("la police embarquée contient '?'")
}

/// Largeur en pixels d'un texte (chasse fixe : un glyphe par caractère).
pub fn text_width(s: &str) -> i32 {
    s.chars().count() as i32 * FONT_W
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_is_well_formed_and_sorted() {
        let n = count();
        assert!(n > 900);
        assert_eq!(DATA.len(), 4 + n * ENTRY);
        for i in 1..n {
            assert!(codepoint_at(i - 1) < codepoint_at(i));
        }
    }

    #[test]
    fn covers_ascii_and_french() {
        for c in (0x20u8..0x7f).map(char::from) {
            assert!(has_glyph(c), "glyphe ASCII manquant : {c:?}");
        }
        for c in "àâäçéèêëîïôöùûüÿœÀÉÈÊÇŒ«»€°".chars() {
            assert!(has_glyph(c), "glyphe manquant : {c:?}");
        }
    }

    #[test]
    fn glyph_shapes() {
        assert!(glyph(' ').iter().all(|&row| row == 0));
        assert!(glyph('A').iter().any(|&row| row != 0));
        // Barre verticale : même colonne allumée sur plusieurs lignes.
        let bar = glyph('|');
        assert!(bar.iter().filter(|&&row| row != 0).count() >= 8);
    }

    #[test]
    fn unknown_falls_back_to_question_mark() {
        assert!(!has_glyph('\u{1F600}'));
        assert_eq!(glyph('\u{1F600}'), glyph('?'));
    }

    #[test]
    fn width_counts_chars_not_bytes() {
        assert_eq!(text_width("été"), 24);
        assert_eq!(text_width(""), 0);
    }
}
