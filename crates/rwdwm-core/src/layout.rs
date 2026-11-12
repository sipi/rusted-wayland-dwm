//! Dispositions : fonctions pures qui découpent une zone en rectangles.
//!
//! Les rectangles renvoyés sont les géométries **extérieures** des clients
//! (bordure comprise), dans l'ordre de la liste des clients tuilés.

use crate::geometry::Rect;

/// Disposition tuilée de dwm : zone maître à gauche, pile à droite.
///
/// - `n` : nombre de clients tuilés visibles ;
/// - `nmaster` : nombre de clients en zone maître ;
/// - `mfact` : part de la largeur accordée à la zone maître.
///
/// Reprend `tile()` de `dwm.c` : dans chaque colonne, le client `i` reçoit
/// `(hauteur restante) / (clients restants)` — les pixels du reste de la
/// division vont donc aux derniers clients, et la colonne est toujours
/// remplie exactement.
pub fn tile(area: Rect, n: usize, nmaster: i32, mfact: f32) -> Vec<Rect> {
    let mut out = Vec::with_capacity(n);
    if n == 0 {
        return out;
    }
    let nmaster = nmaster.max(0) as usize;
    let mw = if n > nmaster {
        if nmaster > 0 {
            (area.w as f32 * mfact) as i32
        } else {
            0
        }
    } else {
        area.w
    };
    let in_master = n.min(nmaster);
    let (mut my, mut ty) = (0, 0);
    for i in 0..n {
        if i < nmaster {
            let h = (area.h - my) / (in_master - i) as i32;
            out.push(Rect::new(area.x, area.y + my, mw, h));
            my += h;
        } else {
            let h = (area.h - ty) / (n - i) as i32;
            out.push(Rect::new(area.x + mw, area.y + ty, area.w - mw, h));
            ty += h;
        }
    }
    out
}

/// Disposition monocle : chaque client occupe toute la zone.
pub fn monocle(area: Rect, n: usize) -> Vec<Rect> {
    vec![area; n]
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA: Rect = Rect::new(0, 18, 1280, 702);

    #[test]
    fn empty() {
        assert!(tile(AREA, 0, 1, 0.55).is_empty());
        assert!(monocle(AREA, 0).is_empty());
    }

    #[test]
    fn single_client_fills_area() {
        assert_eq!(tile(AREA, 1, 1, 0.55), vec![AREA]);
    }

    #[test]
    fn master_and_stack() {
        let r = tile(AREA, 3, 1, 0.55);
        assert_eq!(r[0], Rect::new(0, 18, 704, 702));
        assert_eq!(r[1], Rect::new(704, 18, 576, 351));
        assert_eq!(r[2], Rect::new(704, 369, 576, 351));
    }

    #[test]
    fn remainder_goes_to_last_clients() {
        // 702 / 4 = 175 reste 2 : les deux derniers font 176.
        let r = tile(AREA, 5, 1, 0.5);
        let heights: Vec<i32> = r[1..].iter().map(|r| r.h).collect();
        assert_eq!(heights, vec![175, 175, 176, 176]);
        assert_eq!(r.last().unwrap().bottom(), AREA.bottom());
    }

    #[test]
    fn two_masters() {
        let r = tile(AREA, 4, 2, 0.6);
        assert_eq!(r[0], Rect::new(0, 18, 768, 351));
        assert_eq!(r[1], Rect::new(0, 369, 768, 351));
        assert_eq!(r[2], Rect::new(768, 18, 512, 351));
        assert_eq!(r[3], Rect::new(768, 369, 512, 351));
    }

    #[test]
    fn fewer_clients_than_nmaster_use_full_width() {
        let r = tile(AREA, 2, 3, 0.55);
        assert_eq!(r[0], Rect::new(0, 18, 1280, 351));
        assert_eq!(r[1], Rect::new(0, 369, 1280, 351));
    }

    #[test]
    fn nmaster_zero_puts_everything_in_the_stack() {
        let r = tile(AREA, 2, 0, 0.55);
        assert_eq!(r[0], Rect::new(0, 18, 1280, 351));
        assert_eq!(r[1], Rect::new(0, 369, 1280, 351));
    }

    #[test]
    fn columns_always_tile_the_area_exactly() {
        for n in 1..12usize {
            for nmaster in 0..5 {
                for &mfact in &[0.05f32, 0.33, 0.55, 0.95] {
                    let r = tile(AREA, n, nmaster, mfact);
                    assert_eq!(r.len(), n);
                    let area: i64 = r.iter().map(|r| r.w as i64 * r.h as i64).sum();
                    assert_eq!(area, AREA.w as i64 * AREA.h as i64, "n={n} nmaster={nmaster} mfact={mfact}");
                    for (i, a) in r.iter().enumerate() {
                        assert!(a.x >= AREA.x && a.right() <= AREA.right());
                        assert!(a.y >= AREA.y && a.bottom() <= AREA.bottom());
                        for b in &r[i + 1..] {
                            assert_eq!(a.intersection_area(b), 0, "chevauchement n={n} nmaster={nmaster}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn monocle_gives_everyone_the_whole_area() {
        assert_eq!(monocle(AREA, 3), vec![AREA, AREA, AREA]);
    }
}
