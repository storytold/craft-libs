//! A small three-component `f32` vector for the lighting math. The operations evaluate in the same
//! order as glam's `Vec3`, so results match the apps' code that used it.

use std::ops::{Add, AddAssign, Div, Mul, Sub};

use crate::MAX_RADIANCE;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct V3 {
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) z: f32,
}

impl V3 {
    pub(crate) const ZERO: V3 = V3::new(0.0, 0.0, 0.0);
    pub(crate) const X: V3 = V3::new(1.0, 0.0, 0.0);
    pub(crate) const Y: V3 = V3::new(0.0, 1.0, 0.0);

    pub(crate) const fn new(x: f32, y: f32, z: f32) -> V3 {
        V3 { x, y, z }
    }

    pub(crate) const fn splat(v: f32) -> V3 {
        V3::new(v, v, v)
    }

    pub(crate) const fn from_array(a: [f32; 3]) -> V3 {
        V3::new(a[0], a[1], a[2])
    }

    pub(crate) const fn to_array(self) -> [f32; 3] {
        [self.x, self.y, self.z]
    }

    pub(crate) fn dot(self, o: V3) -> f32 {
        (self.x * o.x) + (self.y * o.y) + (self.z * o.z)
    }

    pub(crate) fn cross(self, o: V3) -> V3 {
        V3::new(self.y * o.z - o.y * self.z, self.z * o.x - o.z * self.x, self.x * o.y - o.x * self.y)
    }

    pub(crate) fn length(self) -> f32 {
        self.dot(self).sqrt()
    }

    pub(crate) fn normalize(self) -> V3 {
        self * self.length().recip()
    }

    pub(crate) fn lerp(self, o: V3, s: f32) -> V3 {
        self * (1.0 - s) + o * s
    }

    pub(crate) fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }

    /// The components of a non-negative average of radiance, kept at most [`MAX_RADIANCE`] when
    /// rounding takes them a little above it.
    pub(crate) fn to_radiance_array(self) -> [f32; 3] {
        [self.x.min(MAX_RADIANCE), self.y.min(MAX_RADIANCE), self.z.min(MAX_RADIANCE)]
    }
}

impl Div<f32> for V3 {
    type Output = V3;
    fn div(self, s: f32) -> V3 {
        V3::new(self.x / s, self.y / s, self.z / s)
    }
}

impl Add for V3 {
    type Output = V3;
    fn add(self, o: V3) -> V3 {
        V3::new(self.x + o.x, self.y + o.y, self.z + o.z)
    }
}

impl AddAssign for V3 {
    fn add_assign(&mut self, o: V3) {
        *self = *self + o;
    }
}

impl Sub for V3 {
    type Output = V3;
    fn sub(self, o: V3) -> V3 {
        V3::new(self.x - o.x, self.y - o.y, self.z - o.z)
    }
}

impl Mul<f32> for V3 {
    type Output = V3;
    fn mul(self, s: f32) -> V3 {
        V3::new(self.x * s, self.y * s, self.z * s)
    }
}

/// A unit vector along `d`, or `None` when `d` has a non-finite component or zero length.
pub(crate) fn unit(d: [f32; 3]) -> Option<V3> {
    let v = V3::from_array(d);
    let big = v.x.abs().max(v.y.abs()).max(v.z.abs());
    if !v.is_finite() || big == 0.0 {
        return None;
    }
    // Scaling by the largest component first keeps the squared length in range.
    let v = v / big;
    let n = v * (1.0 / v.length());
    n.is_finite().then_some(n)
}
