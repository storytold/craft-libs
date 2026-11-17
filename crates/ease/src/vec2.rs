//! A 2D point or vector, for ease curve points and motion paths.

use std::ops::{Add, Mul, Neg, Sub};

/// A 2D point or vector. In files it is `{"x": …, "y": …}`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Vec2 {
    pub x: f64,
    pub y: f64,
}

impl Vec2 {
    pub const ZERO: Vec2 = Vec2 { x: 0.0, y: 0.0 };

    pub const fn new(x: f64, y: f64) -> Self {
        Vec2 { x, y }
    }

    pub fn dot(self, o: Vec2) -> f64 {
        self.x * o.x + self.y * o.y
    }

    /// The length, without overflow for large components.
    pub fn length(self) -> f64 {
        self.x.hypot(self.y)
    }

    /// The vector scaled to length 1, or zero for a zero (or non-finite) vector.
    pub fn normalized(self) -> Vec2 {
        let l = self.length();
        if l > 0.0 && l.is_finite() { Vec2::new(self.x / l, self.y / l) } else { Vec2::ZERO }
    }

    pub fn lerp(self, o: Vec2, t: f64) -> Vec2 {
        Vec2::new(crate::lerp(self.x, o.x, t), crate::lerp(self.y, o.y, t))
    }

    pub fn distance(self, o: Vec2) -> f64 {
        (o - self).length()
    }

    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }
}

impl Add for Vec2 {
    type Output = Vec2;
    fn add(self, o: Vec2) -> Vec2 {
        Vec2::new(self.x + o.x, self.y + o.y)
    }
}

impl Sub for Vec2 {
    type Output = Vec2;
    fn sub(self, o: Vec2) -> Vec2 {
        Vec2::new(self.x - o.x, self.y - o.y)
    }
}

impl Mul<f64> for Vec2 {
    type Output = Vec2;
    fn mul(self, s: f64) -> Vec2 {
        Vec2::new(self.x * s, self.y * s)
    }
}

impl Neg for Vec2 {
    type Output = Vec2;
    fn neg(self) -> Vec2 {
        Vec2::new(-self.x, -self.y)
    }
}
