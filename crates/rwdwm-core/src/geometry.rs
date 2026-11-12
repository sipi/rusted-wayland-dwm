//! Géométrie entière en coordonnées logiques.

/// Rectangle `(x, y, largeur, hauteur)` en pixels logiques.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Rect {
    /// Abscisse du coin supérieur gauche.
    pub x: i32,
    /// Ordonnée du coin supérieur gauche.
    pub y: i32,
    /// Largeur.
    pub w: i32,
    /// Hauteur.
    pub h: i32,
}

impl Rect {
    /// Construit un rectangle.
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }

    /// Abscisse du bord droit (exclu).
    pub const fn right(&self) -> i32 {
        self.x + self.w
    }

    /// Ordonnée du bord bas (exclu).
    pub const fn bottom(&self) -> i32 {
        self.y + self.h
    }

    /// Le point `(px, py)` est-il dans le rectangle ?
    pub const fn contains(&self, px: i32, py: i32) -> bool {
        px >= self.x && px < self.right() && py >= self.y && py < self.bottom()
    }

    /// Le rectangle est-il vide ?
    pub const fn is_empty(&self) -> bool {
        self.w <= 0 || self.h <= 0
    }

    /// Aire de l'intersection avec `other` (0 si disjoints) — `INTERSECT` de dwm.
    pub fn intersection_area(&self, other: &Rect) -> i64 {
        let w = (self.right().min(other.right()) - self.x.max(other.x)).max(0) as i64;
        let h = (self.bottom().min(other.bottom()) - self.y.max(other.y)).max(0) as i64;
        w * h
    }

    /// Rectangle rétréci de `n` pixels sur chaque bord (jamais négatif).
    pub fn shrink(&self, n: i32) -> Rect {
        Rect::new(self.x + n, self.y + n, (self.w - 2 * n).max(0), (self.h - 2 * n).max(0))
    }

    /// Rectangle de taille `(w, h)` centré dans `self` ; s'il déborde, il est
    /// calé sur le coin supérieur gauche.
    pub fn centered(&self, w: i32, h: i32) -> Rect {
        let x = self.x + ((self.w - w) / 2).max(0);
        let y = self.y + ((self.h - h) / 2).max(0);
        Rect::new(x, y, w, h)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contains_and_edges() {
        let r = Rect::new(10, 20, 100, 50);
        assert!(r.contains(10, 20));
        assert!(r.contains(109, 69));
        assert!(!r.contains(110, 20));
        assert!(!r.contains(10, 70));
        assert_eq!((r.right(), r.bottom()), (110, 70));
    }

    #[test]
    fn intersection() {
        let a = Rect::new(0, 0, 100, 100);
        assert_eq!(a.intersection_area(&Rect::new(50, 50, 100, 100)), 2500);
        assert_eq!(a.intersection_area(&Rect::new(100, 0, 10, 10)), 0);
    }

    #[test]
    fn shrink_and_center() {
        assert_eq!(Rect::new(0, 0, 10, 10).shrink(1), Rect::new(1, 1, 8, 8));
        assert_eq!(Rect::new(0, 0, 1, 1).shrink(2), Rect::new(2, 2, 0, 0));
        assert_eq!(Rect::new(0, 18, 1000, 700).centered(200, 100), Rect::new(400, 318, 200, 100));
        assert_eq!(Rect::new(5, 5, 100, 100).centered(300, 300), Rect::new(5, 5, 300, 300));
    }
}
