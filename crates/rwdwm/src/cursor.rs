//! Images de curseur : thème XCursor, avec une flèche intégrée en repli.

use std::collections::HashMap;
use std::io::Read;

use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::memory::MemoryRenderBuffer;
use smithay::input::pointer::CursorIcon;
use smithay::utils::Transform;
use tracing::debug;

/// Image de curseur prête à être composée.
#[derive(Debug)]
pub struct CursorImage {
    /// Pixels.
    pub buffer: MemoryRenderBuffer,
    /// Point chaud, en pixels logiques.
    pub hotspot: (i32, i32),
}

/// Thème de curseurs, avec cache par (forme, échelle).
#[derive(Debug)]
pub struct CursorTheme {
    name: String,
    size: u32,
    cache: HashMap<(CursorIcon, i32), CursorImage>,
}

/// Flèche de repli : `X` = contour noir, `.` = intérieur blanc.
const FALLBACK: [&str; 19] = [
    "X           ",
    "XX          ",
    "X.X         ",
    "X..X        ",
    "X...X       ",
    "X....X      ",
    "X.....X     ",
    "X......X    ",
    "X.......X   ",
    "X........X  ",
    "X.........X ",
    "X......XXXXX",
    "X...X..X    ",
    "X..XX..X    ",
    "X.X  X..X   ",
    "XX   X..X   ",
    "X     X..X  ",
    "      X..X  ",
    "       XX   ",
];

impl CursorTheme {
    /// Lit `XCURSOR_THEME` et `XCURSOR_SIZE` (défauts : `default`, 24).
    pub fn load() -> CursorTheme {
        let name = std::env::var("XCURSOR_THEME").unwrap_or_else(|_| "default".into());
        let size = std::env::var("XCURSOR_SIZE").ok().and_then(|s| s.parse().ok()).unwrap_or(24);
        CursorTheme { name, size, cache: HashMap::new() }
    }

    /// L'image d'une forme de curseur à l'échelle donnée.
    pub fn image(&mut self, icon: CursorIcon, scale: i32) -> &CursorImage {
        let scale = scale.max(1);
        let (name, size) = (&self.name, self.size);
        self.cache.entry((icon, scale)).or_insert_with(|| {
            load_xcursor(name, icon, size * scale as u32, scale)
                .or_else(|| load_xcursor(name, CursorIcon::Default, size * scale as u32, scale))
                .unwrap_or_else(|| {
                    debug!(theme = name, ?icon, "curseur introuvable, flèche intégrée");
                    fallback(scale)
                })
        })
    }
}

/// Cherche une forme (sous son nom puis ses alias) dans un thème XCursor.
fn load_xcursor(theme: &str, icon: CursorIcon, size: u32, scale: i32) -> Option<CursorImage> {
    let theme = xcursor::CursorTheme::load(theme);
    let path = std::iter::once(icon.name()).chain(icon.alt_names().iter().copied()).find_map(|n| theme.load_icon(n))?;
    let mut data = Vec::new();
    std::fs::File::open(path).ok()?.read_to_end(&mut data).ok()?;
    let images = xcursor::parser::parse_xcursor(&data)?;
    // Première image de la taille la plus proche (curseurs animés : image fixe).
    let image = images.iter().min_by_key(|i| (i.size as i64 - size as i64).abs())?;
    let buffer = MemoryRenderBuffer::from_slice(
        &image.pixels_rgba,
        Fourcc::Abgr8888,
        (image.width as i32, image.height as i32),
        scale,
        Transform::Normal,
        None,
    );
    Some(CursorImage { buffer, hotspot: (image.xhot as i32 / scale, image.yhot as i32 / scale) })
}

/// Construit la flèche intégrée.
fn fallback(scale: i32) -> CursorImage {
    let (w, h) = (FALLBACK[0].len() as i32, FALLBACK.len() as i32);
    let mut data = Vec::with_capacity((w * h * scale * scale * 4) as usize);
    for row in FALLBACK {
        for _ in 0..scale {
            for cell in row.bytes() {
                let pixel: [u8; 4] = match cell {
                    b'X' => [0, 0, 0, 255],
                    b'.' => [255, 255, 255, 255],
                    _ => [0, 0, 0, 0],
                };
                for _ in 0..scale {
                    data.extend_from_slice(&pixel);
                }
            }
        }
    }
    let buffer =
        MemoryRenderBuffer::from_slice(&data, Fourcc::Abgr8888, (w * scale, h * scale), scale, Transform::Normal, None);
    CursorImage { buffer, hotspot: (0, 0) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_rows_have_the_same_width() {
        assert!(FALLBACK.iter().all(|row| row.len() == FALLBACK[0].len()));
    }

    #[test]
    fn missing_theme_falls_back_to_the_arrow() {
        let mut theme = CursorTheme { name: "thème-inexistant".into(), size: 24, cache: HashMap::new() };
        assert_eq!(theme.image(CursorIcon::Default, 1).hotspot, (0, 0));
        assert_eq!(theme.image(CursorIcon::Text, 2).hotspot, (0, 0));
        assert_eq!(theme.cache.len(), 2);
    }
}
