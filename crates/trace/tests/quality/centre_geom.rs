//! Point arithmetic for the code-drawn centre-line scenes.

use std::ops::{Add, Mul, Neg, Sub};

/// A point or vector in pixel coordinates (pixel `(x, y)` covers `[x, x+1] × [y, y+1]`, y down).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct P {
    pub x: f64,
    pub y: f64,
}

impl P {
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
    pub fn dot(self, o: P) -> f64 {
        self.x * o.x + self.y * o.y
    }
    pub fn len(self) -> f64 {
        self.x.hypot(self.y)
    }
    pub fn dist(self, o: P) -> f64 {
        (self - o).len()
    }
    /// Unit vector, or `None` for a (near) zero vector.
    pub fn unit(self) -> Option<P> {
        let l = self.len();
        (l > 1e-12 && l.is_finite()).then(|| P::new(self.x / l, self.y / l))
    }
    pub fn lerp(self, o: P, t: f64) -> P {
        self + (o - self) * t
    }
}

impl Add for P {
    type Output = P;
    fn add(self, o: P) -> P {
        P::new(self.x + o.x, self.y + o.y)
    }
}
impl Sub for P {
    type Output = P;
    fn sub(self, o: P) -> P {
        P::new(self.x - o.x, self.y - o.y)
    }
}
impl Mul<f64> for P {
    type Output = P;
    fn mul(self, k: f64) -> P {
        P::new(self.x * k, self.y * k)
    }
}
impl Neg for P {
    type Output = P;
    fn neg(self) -> P {
        P::new(-self.x, -self.y)
    }
}

/// Distance from `q` to segment `ab`, and the parameter of the nearest point.
pub fn seg_dist(q: P, a: P, b: P) -> (f64, f64) {
    let ab = b - a;
    let l2 = ab.dot(ab);
    if l2 < 1e-18 {
        return (q.dist(a), 0.0);
    }
    let t = ((q - a).dot(ab) / l2).clamp(0.0, 1.0);
    (q.dist(a + ab * t), t)
}
