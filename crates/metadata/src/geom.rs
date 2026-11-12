//! The geometry metadata is stated in: a point, an axis-aligned rectangle, and the eight Exif orientations.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

/// Axis-aligned rectangle `[x0, x1) × [y0, y1)`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

impl Rect {
    pub const UNIT: Rect = Rect { x0: 0.0, y0: 0.0, x1: 1.0, y1: 1.0 };
    pub const fn new(x0: f64, y0: f64, x1: f64, y1: f64) -> Self {
        Self { x0, y0, x1, y1 }
    }
    pub fn from_xywh(x: f64, y: f64, w: f64, h: f64) -> Self {
        Self::new(x, y, x + w, y + h)
    }
    pub fn from_center(c: Point, w: f64, h: f64) -> Self {
        Self::new(c.x - w / 2.0, c.y - h / 2.0, c.x + w / 2.0, c.y + h / 2.0)
    }
    pub fn from_points(a: Point, b: Point) -> Self {
        Self::new(a.x.min(b.x), a.y.min(b.y), a.x.max(b.x), a.y.max(b.y))
    }
    pub fn width(&self) -> f64 {
        self.x1 - self.x0
    }
    pub fn height(&self) -> f64 {
        self.y1 - self.y0
    }
    pub fn area(&self) -> f64 {
        self.width().max(0.0) * self.height().max(0.0)
    }
    pub fn center(&self) -> Point {
        Point::new((self.x0 + self.x1) / 2.0, (self.y0 + self.y1) / 2.0)
    }
    pub fn aspect(&self) -> f64 {
        if self.height() != 0.0 { self.width() / self.height() } else { 0.0 }
    }
    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.x0 && p.x < self.x1 && p.y >= self.y0 && p.y < self.y1
    }
    pub fn is_empty(&self) -> bool {
        self.x1 <= self.x0 || self.y1 <= self.y0
    }
    pub fn intersect(&self, o: &Rect) -> Rect {
        Rect::new(self.x0.max(o.x0), self.y0.max(o.y0), self.x1.min(o.x1), self.y1.min(o.y1))
    }
}

/// EXIF orientation (1..=8) as a transform of the stored pixel grid.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Orientation {
    #[default]
    Normal,
    FlipH,
    Rotate180,
    FlipV,
    Transpose,
    Rotate90,
    Transverse,
    Rotate270,
}

impl Orientation {
    pub fn from_exif(v: u16) -> Orientation {
        match v {
            2 => Orientation::FlipH,
            3 => Orientation::Rotate180,
            4 => Orientation::FlipV,
            5 => Orientation::Transpose,
            6 => Orientation::Rotate90,
            7 => Orientation::Transverse,
            8 => Orientation::Rotate270,
            _ => Orientation::Normal,
        }
    }
    pub fn to_exif(self) -> u16 {
        match self {
            Orientation::Normal => 1,
            Orientation::FlipH => 2,
            Orientation::Rotate180 => 3,
            Orientation::FlipV => 4,
            Orientation::Transpose => 5,
            Orientation::Rotate90 => 6,
            Orientation::Transverse => 7,
            Orientation::Rotate270 => 8,
        }
    }
    /// Whether width and height swap.
    pub fn swaps_axes(self) -> bool {
        matches!(self, Orientation::Transpose | Orientation::Rotate90 | Orientation::Transverse | Orientation::Rotate270)
    }
    /// Decompose into (clockwise quarter turns applied after an optional horizontal flip).
    pub fn to_parts(self) -> (bool, u8) {
        match self {
            Orientation::Normal => (false, 0),
            Orientation::Rotate90 => (false, 1),
            Orientation::Rotate180 => (false, 2),
            Orientation::Rotate270 => (false, 3),
            Orientation::FlipH => (true, 0),
            Orientation::Transverse => (true, 1),
            Orientation::FlipV => (true, 2),
            Orientation::Transpose => (true, 3),
        }
    }
    pub fn from_parts(flip: bool, quarter_turns_cw: u8) -> Orientation {
        match (flip, quarter_turns_cw % 4) {
            (false, 0) => Orientation::Normal,
            (false, 1) => Orientation::Rotate90,
            (false, 2) => Orientation::Rotate180,
            (false, _) => Orientation::Rotate270,
            (true, 0) => Orientation::FlipH,
            (true, 1) => Orientation::Transverse,
            (true, 2) => Orientation::FlipV,
            (true, _) => Orientation::Transpose,
        }
    }
    /// Rotate the (already oriented) image a further quarter turn clockwise (`cw`) or counter-clockwise.
    pub fn rotated(self, cw: bool) -> Orientation {
        let (f, t) = self.to_parts();
        Orientation::from_parts(f, if cw { t + 1 } else { t + 3 })
    }
    /// Flip the displayed image horizontally.
    pub fn flipped_h(self) -> Orientation {
        let (f, t) = self.to_parts();
        // flipH ∘ rot(t) = rot(-t) ∘ flipH
        Orientation::from_parts(!f, (4 - t) % 4)
    }
    /// The orientation that undoes this one (`o.inverse().map(o.map(p)) == p`). Flips and half turns
    /// are their own inverses; quarter turns swap.
    pub fn inverse(self) -> Orientation {
        match self {
            Orientation::Rotate90 => Orientation::Rotate270,
            Orientation::Rotate270 => Orientation::Rotate90,
            o => o,
        }
    }
    /// Map a rectangle in normalized (0..1) coordinates of the stored image to normalized coordinates
    /// of the oriented one (e.g. a face region given on the upright photo → the frame after the user's
    /// Rotate Left/Right).
    pub fn map_norm_rect(self, r: Rect) -> Rect {
        let (ax, ay) = self.map(r.x0, r.y0, 1.0, 1.0);
        let (bx, by) = self.map(r.x1, r.y1, 1.0, 1.0);
        Rect::from_points(Point::new(ax, ay), Point::new(bx, by))
    }
    /// Map a point in stored pixel space (w×h stored dims) to oriented space.
    pub fn map(self, x: f64, y: f64, w: f64, h: f64) -> (f64, f64) {
        let (flip, t) = self.to_parts();
        let (mut x, y2, mut w2, mut h2) = (x, y, w, h);
        if flip {
            x = w2 - x;
        }
        let mut y = y2;
        for _ in 0..t {
            // rotate 90° cw: (x, y) -> (h - y, x)
            let nx = h2 - y;
            let ny = x;
            x = nx;
            y = ny;
            std::mem::swap(&mut w2, &mut h2);
        }
        (x, y)
    }
}
