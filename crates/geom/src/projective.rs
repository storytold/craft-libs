//! Projective (perspective) transforms of the plane: 3×3 homographies.
//!
//! The Perspective Grid maps each grid plane to the page with one, objects in perspective move by
//! conjugating plane-space affine maps with it, and type and symbol instances in perspective keep
//! one that projects their flat art.

use kurbo::{Affine, Point, Rect};

/// A 3×3 projective transform (row-major), acting on `(x, y, 1)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Homography(pub [[f64; 3]; 3]);

impl Homography {
    pub const IDENTITY: Self = Self([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]);

    /// From the images of (1,0,0), (0,1,0) and (0,0,1).
    pub fn from_cols(c1: [f64; 3], c2: [f64; 3], c3: [f64; 3]) -> Self {
        Self([[c1[0], c2[0], c3[0]], [c1[1], c2[1], c3[1]], [c1[2], c2[2], c3[2]]])
    }

    /// The affine map `a` as a homography.
    pub fn from_affine(a: Affine) -> Self {
        let [m0, m1, m2, m3, m4, m5] = a.as_coeffs();
        Self([[m0, m2, m4], [m1, m3, m5], [0.0, 0.0, 1.0]])
    }

    /// From nine row-major numbers (as stored in documents); `None` unless all are finite and the
    /// map is invertible. When the determinant is not finite, this is the same map divided by the
    /// largest magnitude in its `w` row (so that row's largest entry is 1), and `None` when that
    /// copy has an entry that is not finite or is not invertible.
    pub fn from_array(v: [f64; 9]) -> Option<Self> {
        if v.iter().any(|x| !x.is_finite()) {
            return None;
        }
        let h = Self([[v[0], v[1], v[2]], [v[3], v[4], v[5]], [v[6], v[7], v[8]]]);
        if h.det().is_finite() {
            return h.is_invertible().then_some(h);
        }
        let k = h.0[2].iter().fold(0.0_f64, |k, v| k.max(v.abs()));
        if k <= 0.0 || !h.is_invertible() {
            return None;
        }
        let c = Self(h.0.map(|row| row.map(|v| v / k)));
        (c.is_finite() && c.is_invertible()).then_some(c)
    }

    /// The nine row-major numbers.
    pub fn to_array(&self) -> [f64; 9] {
        let m = &self.0;
        [m[0][0], m[0][1], m[0][2], m[1][0], m[1][1], m[1][2], m[2][0], m[2][1], m[2][2]]
    }

    /// Map a point; `None` when it lands on or beyond the horizon (w at most 1e-9 times the largest
    /// magnitude in the map's `w` row, so the answer is the same at any scale of the map), or when
    /// it does not map to a finite page point. The point is mapped through [`Self::upscaled`].
    pub fn apply(&self, p: Point) -> Option<Point> {
        let m = self.upscaled(p);
        let x = m[0][0] * p.x + m[0][1] * p.y + m[0][2];
        let y = m[1][0] * p.x + m[1][1] * p.y + m[1][2];
        let w = m[2][0] * p.x + m[2][1] * p.y + m[2][2];
        let k = m[2].iter().fold(0.0_f64, |k, v| k.max(v.abs()));
        if w <= 1e-9 * k || !x.is_finite() || !y.is_finite() {
            return None;
        }
        let q = Point::new(x / w, y / w);
        q.is_finite().then_some(q)
    }

    /// The entries for mapping `p`, multiplied by a power of two of at least 1 (an exact step that
    /// keeps a map at a tiny scale from losing precision to subnormal products). When every entry
    /// is below 1/2 in magnitude, the power brings the largest to at least 1/2 and below 1, divided
    /// by the power of two of `p`'s larger coordinate magnitude when that is 1 or more, so that no
    /// product of an entry and a coordinate reaches 1. Otherwise the entries are unchanged.
    fn upscaled(&self, p: Point) -> [[f64; 3]; 3] {
        let largest = self.0.iter().flatten().fold(0.0_f64, |k, v| k.max(v.abs()));
        let e_p = frexp(p.x.abs().max(p.y.abs())).1.max(0);
        let e = (frexp(largest).1.min(0) + e_p).min(0);
        self.0.map(|row| row.map(|v| ldexp(v, -e)))
    }

    pub fn det(&self) -> f64 {
        let m = &self.0;
        m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
    }

    /// Whether the map is invertible: [`Self::has_significant_det`], or, when the determinant is
    /// not finite, [`Self::inverse`] gives one.
    fn is_invertible(&self) -> bool {
        if self.det().is_finite() {
            return self.has_significant_det();
        }
        self.inverse().is_some()
    }

    /// Whether every entry is finite and [`significant_det`] holds for the map or for its
    /// adjugate (the inverse times the determinant). Both tests give the same answer at any scale
    /// of the map. The adjugate of the adjugate is the map times a number, so, apart from
    /// rounding, a map passes when its inverse passes and fails when its inverse fails.
    fn has_significant_det(&self) -> bool {
        if !self.is_finite() {
            return false;
        }
        significant_det(self.det_terms()) || significant_det(split_det_terms(&self.adjugate()))
    }

    /// The six signed products the determinant sums, each as a mantissa product and an exponent
    /// sum.
    fn det_terms(&self) -> [(f64, i32); 6] {
        split_det_terms(&self.0.map(|row| row.map(frexp)))
    }

    /// The adjugate (the transposed matrix of cofactors), each entry as a mantissa and a power of
    /// two. Each cofactor's two products are formed exactly before they are subtracted, so an
    /// entry is within a few units in the last place of the exact cofactor of the stored numbers.
    /// The entries must be finite.
    fn adjugate(&self) -> [[(f64, i32); 3]; 3] {
        let f = self.0.map(|row| row.map(frexp));
        let c = |r0: usize, c0: usize, r1: usize, c1: usize| split_minor(f[r0][c0], f[r1][c1], f[r0][c1], f[r1][c0]);
        let neg = |(m, e): (f64, i32)| (-m, e);
        [
            [c(1, 1, 2, 2), neg(c(0, 1, 2, 2)), c(0, 1, 1, 2)],
            [neg(c(1, 0, 2, 2)), c(0, 0, 2, 2), neg(c(0, 0, 1, 2))],
            [c(1, 0, 2, 1), neg(c(0, 0, 2, 1)), c(0, 0, 1, 1)],
        ]
    }

    /// The same map with every entry divided by one power of two, chosen so that the largest of the
    /// six products its determinant sums is at least 1/8 and below 4. `None` when every one of
    /// those products is 0 or an entry is not finite.
    fn scaled(&self) -> Option<Self> {
        if !self.is_finite() {
            return None;
        }
        let f = self.0.map(|row| row.map(frexp));
        // The columns each of the six products takes from rows 0, 1 and 2.
        let cols = [(0, 1, 2), (0, 2, 1), (1, 0, 2), (1, 2, 0), (2, 0, 1), (2, 1, 0)];
        let e = cols
            .iter()
            .map(|&(a, b, c)| (f[0][a], f[1][b], f[2][c]))
            .filter(|(a, b, c)| a.0 * b.0 * c.0 != 0.0)
            .map(|(a, b, c)| a.1 + b.1 + c.1)
            .max()?;
        let e = e.div_euclid(3);
        Some(Self(self.0.map(|row| row.map(|v| ldexp(v, -e)))))
    }

    /// Inverse; `None` when [`Self::has_significant_det`] counts the map as not invertible. The
    /// inverse is formed from the map itself when its determinant is finite, and is kept when every
    /// entry is finite and its own determinant is not 0. Otherwise it is the inverse of
    /// [`Self::scaled`] (the exact inverse multiplied by one power of two, which is the same map),
    /// and `None` when that one fails the same checks.
    pub fn inverse(&self) -> Option<Self> {
        if !self.has_significant_det() {
            return None;
        }
        if self.det().is_finite()
            && let Some(inv) = self.adjugate_inverse()
        {
            return Some(inv);
        }
        self.scaled()?.adjugate_inverse()
    }

    /// The adjugate divided by the determinant; `None` when an entry is not finite, the
    /// determinant is 0, or the result has an entry that is not finite or fails
    /// [`Self::has_significant_det`] (as when a row underflows to 0). Every cofactor and the
    /// determinant are held as a mantissa and a power of two, so an entry of the result is lost
    /// only when that entry itself is beyond the range of `f64`.
    fn adjugate_inverse(&self) -> Option<Self> {
        if !self.is_finite() {
            return None;
        }
        let f = self.0.map(|row| row.map(frexp));
        let mul = |a: (f64, i32), b: (f64, i32)| (a.0 * b.0, a.1 + b.1);
        let adj = self.adjugate();
        // The determinant expanded along the first row, as `det` sums it.
        let (dm, de) = split_sum(&[mul(f[0][0], adj[0][0]), mul(f[0][1], adj[1][0]), mul(f[0][2], adj[2][0])]);
        if dm == 0.0 {
            return None;
        }
        let mut out = [[0.0; 3]; 3];
        for (r, row) in adj.iter().enumerate() {
            for (k, (m, e)) in row.iter().enumerate() {
                out[r][k] = ldexp(m / dm, e - de);
            }
        }
        let out = Self(out);
        // H⁻¹·(x, y, 1) = (u, v, 1)/w_H, so points in front of the camera keep w > 0.
        out.has_significant_det().then_some(out)
    }

    /// `self ∘ other`: `other` first.
    pub fn then_after(&self, other: &Self) -> Self {
        let (a, b) = (&self.0, &other.0);
        let mut out = [[0.0; 3]; 3];
        for (r, row) in out.iter_mut().enumerate() {
            for (k, v) in row.iter_mut().enumerate() {
                *v = a[r][0] * b[0][k] + a[r][1] * b[1][k] + a[r][2] * b[2][k];
            }
        }
        Self(out)
    }

    /// [`Self::then_after`]; `None` when an entry of the product is not finite (an entry
    /// overflows, or a factor has one that is not finite).
    pub fn checked_then_after(&self, other: &Self) -> Option<Self> {
        let c = self.then_after(other);
        c.is_finite().then_some(c)
    }

    /// Whether every entry of `a` is finite and the determinant of its linear part has a magnitude
    /// more than 1e-15 times the larger of the two products it sums (a test that gives the same
    /// answer at any scale of `a`).
    pub fn affine_is_invertible(a: Affine) -> bool {
        Self::from_affine(a).has_significant_det()
    }

    /// `self` seen through `a`: the map `a ∘ self ∘ a⁻¹` (what `self` becomes when everything it
    /// maps from and to is moved by `a`). When the product overflows, or underflows to a map that
    /// is not invertible, every entry is formed with its powers of two kept apart, and the product
    /// is returned divided by the largest magnitude in its `w` row (the same map, with a `w` row
    /// whose largest entry is 1). `None` when `a` isn't invertible
    /// ([`Self::affine_is_invertible`]), or when the result at that scale has an entry that is not
    /// finite or is not invertible. `a⁻¹` is
    /// [`Affine::inverse`] when `a`'s determinant and that inverse are finite, and
    /// [`Self::inverse`] of `a` otherwise.
    pub fn conjugated(&self, a: Affine) -> Option<Self> {
        if !Self::affine_is_invertible(a) {
            return None;
        }
        let to = Self::from_affine(a);
        let inv = a.inverse();
        let from = if a.determinant().is_finite() && inv.is_finite() { Self::from_affine(inv) } else { to.inverse()? };
        let c = to.then_after(self).then_after(&from);
        if c.is_finite() && c.has_significant_det() {
            return Some(c);
        }
        // Each entry of the product is a sum of nine terms to[r][i]·self[i][j]·from[j][k]. Every
        // factor is split into a mantissa and a power of two, so each term is a mantissa product
        // and an exponent sum, and the whole product is scaled by one power of two at the end.
        let split = |h: &Self| h.0.map(|row| row.map(frexp));
        let (t, s, f) = (split(&to), split(self), split(&from));
        let term = |r: usize, i: usize, j: usize, k: usize| {
            let (a, b, c) = (t[r][i], s[i][j], f[j][k]);
            (a.0 * b.0 * c.0, a.1 + b.1 + c.1)
        };
        let ijk = (0..27).map(|n| (n / 9, n / 3 % 3, n % 3));
        let e = ijk.map(|(i, j, k)| term(2, i, j, k)).filter(|(m, _)| *m != 0.0).map(|(_, e)| e).max()?;
        let mut c = [[0.0; 3]; 3];
        for (r, row) in c.iter_mut().enumerate() {
            for (k, v) in row.iter_mut().enumerate() {
                *v = (0..9).map(|n| term(r, n / 3, n % 3, k)).map(|(m, x)| ldexp(m, x - e)).sum();
            }
        }
        let k = c[2].iter().fold(0.0_f64, |k, v| k.max(v.abs()));
        if k <= 0.0 {
            return None;
        }
        let c = Self(c.map(|row| row.map(|v| v / k)));
        (c.is_finite() && c.is_invertible()).then_some(c)
    }

    fn is_finite(&self) -> bool {
        self.0.iter().flatten().all(|v| v.is_finite())
    }

    /// The affine map that matches `self` to first order at `p` (its derivative there), `None`
    /// beyond the horizon or when the derivative is not finite.
    pub fn affine_at(&self, p: Point) -> Option<Affine> {
        let m = self.upscaled(p);
        let w = m[2][0] * p.x + m[2][1] * p.y + m[2][2];
        let q = self.apply(p)?;
        // d(x/w) = (dx − (x/w)·dw) / w, and the same for y.
        let j = |r: usize, c: usize| (m[r][c] - [q.x, q.y][r] * m[2][c]) / w;
        let lin = Affine::new([j(0, 0), j(1, 0), j(0, 1), j(1, 1), 0.0, 0.0]);
        let out = Affine::translate(q.to_vec2()) * lin * Affine::translate(-p.to_vec2());
        out.is_finite().then_some(out)
    }

    /// The bounding box of the image of `r` (lines stay lines: its corners bound it), `None` when
    /// part of it crosses the horizon.
    pub fn map_rect_bbox(&self, r: Rect) -> Option<Rect> {
        let pts = [Point::new(r.x0, r.y0), Point::new(r.x1, r.y0), Point::new(r.x1, r.y1), Point::new(r.x0, r.y1)];
        let mut out: Option<Rect> = None;
        for p in pts {
            let q = self.apply(p)?;
            out = Some(out.map_or(Rect::from_points(q, q), |b| b.union_pt(q)));
        }
        out
    }
}

/// `v` as `m·2^e` with `0.5 ≤ |m| < 1`; 0, infinities and NaN come back as `(v, 0)`.
fn frexp(v: f64) -> (f64, i32) {
    if v == 0.0 || !v.is_finite() {
        return (v, 0);
    }
    // A subnormal is scaled into the normal range first.
    let (v, bias) = if v.abs() < f64::MIN_POSITIVE { (v * pow2(64), -64) } else { (v, 0) };
    let bits = v.to_bits();
    let e = ((bits >> 52) & 0x7ff) as i32 - 1022;
    (f64::from_bits((bits & !(0x7ff << 52)) | (1022 << 52)), e + bias)
}

/// The six signed products the determinant of `f` sums, each as a mantissa product and an exponent
/// sum.
fn split_det_terms(f: &[[(f64, i32); 3]; 3]) -> [(f64, i32); 6] {
    // The columns each of the six products takes from rows 0, 1 and 2, and its sign.
    let cols = [(0, 1, 2, 1.0), (0, 2, 1, -1.0), (1, 0, 2, -1.0), (1, 2, 0, 1.0), (2, 0, 1, 1.0), (2, 1, 0, -1.0)];
    cols.map(|(a, b, c, sign)| {
        let (a, b, c) = (f[0][a], f[1][b], f[2][c]);
        (sign * a.0 * b.0 * c.0, a.1 + b.1 + c.1)
    })
}

/// Whether the sum of the six products in `terms` has a magnitude more than 1e-15 times that of
/// the largest of them. All six are divided by the power of two of the largest before they are
/// summed, so the largest is at least 1/8 and below 1 and none of them overflows. `false` when
/// every product is 0.
fn significant_det(terms: [(f64, i32); 6]) -> bool {
    let Some(e) = terms.iter().filter(|(m, _)| *m != 0.0).map(|(_, e)| *e).max() else {
        return false;
    };
    let terms = terms.map(|(m, x)| ldexp(m, x - e));
    let largest = terms.iter().fold(0.0_f64, |k, v| k.max(v.abs()));
    let d: f64 = terms.iter().sum();
    d.abs() > 1e-15 * largest
}

/// `a·d − b·c` for numbers split as by [`frexp`], as a mantissa `0.5 ≤ |m| < 1` (or 0) and a power
/// of two. Each product is formed exactly as a rounded value and its rounding error, and the four
/// parts are summed with the error of the first subtraction kept, so the result is within a few
/// units in the last place of the exact value unless the two products agree to about 32 digits.
fn split_minor(a: (f64, i32), d: (f64, i32), b: (f64, i32), c: (f64, i32)) -> (f64, i32) {
    let exact = |x: (f64, i32), y: (f64, i32), sign: f64| {
        let hi = x.0 * y.0;
        (sign * hi, sign * x.0.mul_add(y.0, -hi), x.1 + y.1)
    };
    let parts = [exact(a, d, 1.0), exact(b, c, -1.0)];
    let Some(e) = parts.iter().filter(|p| p.0 != 0.0).map(|p| p.2).max() else {
        return (0.0, 0);
    };
    let [(h0, l0), (h1, l1)] = parts.map(|(h, l, x)| (ldexp(h, x - e), ldexp(l, x - e)));
    // Knuth's two-sum: `s + err` is exactly `h0 + h1`.
    let s = h0 + h1;
    let t = s - h0;
    let err = (h0 - (s - t)) + (h1 - t);
    let (m, x) = frexp(s + (err + (l0 + l1)));
    (m, e + x)
}

/// The sum of `m·2^e` over `terms`, in order, as a mantissa `0.5 ≤ |m| < 1` (or 0) and a power of
/// two. Every term is first divided by the power of two of the largest nonzero one, so none of
/// them overflows.
fn split_sum(terms: &[(f64, i32)]) -> (f64, i32) {
    let Some(e) = terms.iter().filter(|(m, _)| *m != 0.0).map(|(_, e)| *e).max() else {
        return (0.0, 0);
    };
    let (m, x) = frexp(terms.iter().map(|&(m, x)| ldexp(m, x - e)).sum());
    (m, e + x)
}

/// `m·2^e`, multiplied in steps of powers of two that are each within the normal range.
fn ldexp(mut m: f64, mut e: i32) -> f64 {
    while e > 1023 && m.is_finite() {
        m *= pow2(1023);
        e -= 1023;
    }
    while e < -1022 && m != 0.0 {
        m *= pow2(-1022);
        e += 1022;
    }
    m * pow2(e.clamp(-1022, 1023))
}

/// `2^e` for `-1022 ≤ e ≤ 1023`.
fn pow2(e: i32) -> f64 {
    f64::from_bits(((e + 1023) as u64) << 52)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h() -> Homography {
        Homography([[1.2, 0.1, 5.0], [-0.2, 0.9, 3.0], [0.001, 0.002, 1.0]])
    }

    #[test]
    fn compose_invert_and_round_trip() {
        let a = h();
        let inv = a.inverse().unwrap();
        let id = a.then_after(&inv);
        for (r, row) in id.0.iter().enumerate() {
            for (k, v) in row.iter().enumerate() {
                assert!((v - if r == k { 1.0 } else { 0.0 }).abs() < 1e-9);
            }
        }
        let p = Point::new(30.0, -12.0);
        assert!(inv.apply(a.apply(p).unwrap()).unwrap().distance(p) < 1e-9);
        assert_eq!(Homography::from_array(a.to_array()), Some(a));
        assert!(Homography::from_array([f64::NAN; 9]).is_none());
        assert!(Homography::from_array([0.0; 9]).is_none());
    }

    #[test]
    fn affine_and_conjugation() {
        let t = Affine::translate((10.0, -4.0)) * Affine::scale(2.0);
        let p = Point::new(3.0, 7.0);
        assert!(Homography::from_affine(t).apply(p).unwrap().distance(t * p) < 1e-12);
        // Conjugating: moving the input and the output by `t` keeps the picture moved by `t`.
        let c = h().conjugated(t).unwrap();
        assert!(c.apply(t * p).unwrap().distance(t * h().apply(p).unwrap()) < 1e-9);
        assert!(h().conjugated(Affine::scale(0.0)).is_none());
    }

    #[test]
    fn affine_at_is_the_derivative() {
        let p = Point::new(12.0, -7.0);
        let a = h().affine_at(p).unwrap();
        assert!(a * p == h().apply(p).unwrap() || (a * p).distance(h().apply(p).unwrap()) < 1e-9);
        let e = 1e-4;
        for d in [kurbo::Vec2::new(e, 0.0), kurbo::Vec2::new(0.0, e)] {
            let exact = h().apply(p + d).unwrap();
            assert!((a * (p + d)).distance(exact) < 1e-7, "{d:?}");
        }
    }

    #[test]
    fn a_point_that_overflows_maps_to_nothing() {
        // w = 1e-8 is in front of the horizon, but 1e305 / 1e-8 overflows.
        let near = Homography([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1e-8]]);
        assert_eq!(near.apply(Point::new(1e305, 0.0)), None);
        assert_eq!(near.apply(Point::new(0.0, -1e305)), None);
        assert_eq!(near.map_rect_bbox(Rect::new(0.0, 0.0, 1e305, 1.0)), None);
        assert!(near.apply(Point::new(2.0, 3.0)).unwrap().distance(Point::new(2e8, 3e8)) < 1e-6);
        // A NaN w is not caught by the horizon test.
        let nan = Homography([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, f64::NAN]]);
        assert_eq!(nan.apply(Point::new(1.0, 1.0)), None);
    }

    #[test]
    fn an_inverse_that_overflows_is_none() {
        let big = Homography([[1e300, 0.0, 0.0], [0.0, 1e300, 0.0], [0.0, 0.0, 1e-310]]);
        assert_eq!(big.inverse(), None);
        assert!(h().inverse().is_some());
    }

    #[test]
    fn a_determinant_that_overflows_is_taken_at_a_smaller_scale() {
        // (x, y) ↦ (x + 1, y) / (x + 2) with every entry multiplied by 1e200: the determinant is
        // inf − inf, though the map is invertible.
        let big = Homography([[1e200, 0.0, 1e200], [0.0, 1e200, 0.0], [1e200, 0.0, 2e200]]);
        assert!(big.det().is_nan());
        // Read back as the same map with a w row whose largest entry is 1.
        let read = Homography::from_array(big.to_array()).unwrap();
        assert_eq!(read, Homography([[0.5, 0.0, 0.5], [0.0, 0.5, 0.0], [0.5, 0.0, 1.0]]));
        assert_eq!(Homography::from_array(read.to_array()), Some(read));
        let inv = big.inverse().unwrap();
        for p in [Point::new(3.0, -2.0), Point::new(-1.5, 4.0)] {
            assert!(read.apply(p).unwrap().distance(big.apply(p).unwrap()) < 1e-12);
            assert!(inv.apply(big.apply(p).unwrap()).unwrap().distance(p) < 1e-9);
        }
        // The identity with every entry 1e308: stored as it is, (2, 2) overflows to 2e308, but the
        // map read back draws it at (2, 2).
        let wide = Homography([[1e308, 0.0, 0.0], [0.0, 1e308, 0.0], [0.0, 0.0, 1e308]]);
        assert_eq!(wide.apply(Point::new(2.0, 2.0)), None);
        let read = Homography::from_array(wide.to_array()).unwrap();
        assert_eq!(read.apply(Point::new(2.0, 2.0)), Some(Point::new(2.0, 2.0)));
        // A large map whose w row is 1e-205 times its largest entry: read back at a w row of 1,
        // its points are in front of the horizon and draw.
        let low = Homography([[1e300, 0.0, 1e305], [0.0, 1e300, 0.0], [1e90, 0.0, 1e100]]);
        assert!(low.det().is_nan());
        let read = Homography::from_array(low.to_array()).unwrap();
        assert_eq!(Homography::from_array(read.to_array()), Some(read));
        let p = Point::new(1.0, 1.0);
        let want = Point::new((1e200 + 1e205) / (1.0 + 1e-10), 1e200 / (1.0 + 1e-10));
        assert!(read.apply(p).unwrap().distance(want) <= 1e-9 * want.to_vec2().hypot());
        // A scale by 1e305 and a move by -3.9e307 with a w row of 1: the determinant is inf, and
        // the inverse is a scale by 1e-305 and a move by 390.
        let moved = Homography([[1e305, 0.0, -3.9e307], [0.0, 1e305, -3.2e307], [0.0, 0.0, 1.0]]);
        assert!(moved.det().is_infinite());
        assert_eq!(Homography::from_array(moved.to_array()), Some(moved));
        let inv = moved.inverse().unwrap();
        for p in [Point::new(400.0, 330.0), Point::new(500.0, 400.0)] {
            assert!(inv.apply(moved.apply(p).unwrap()).unwrap().distance(p) < 1e-9);
        }
        // A scale by 1e500: the determinant is inf, but scaled so that its products are near 1
        // the w row is 0, and no point maps to a finite page point except the origin.
        let flat = Homography([[1e300, 0.0, 0.0], [0.0, 1e300, 0.0], [0.0, 0.0, 1e-200]]);
        assert!(flat.det().is_infinite());
        assert_eq!(Homography::from_array(flat.to_array()), None);
        assert_eq!(flat.inverse(), None);
        // Two equal rows with large entries.
        let twice = Homography([[1e200, 1e200, 0.0], [1e200, 1e200, 0.0], [0.0, 0.0, 1e200]]);
        assert_eq!(Homography::from_array(twice.to_array()), None);
        assert_eq!(twice.inverse(), None);
        // (x, y) ↦ 1e200·(x + 1, y) / (x + 2) seen through a scale by 1e10: the product
        // overflows, and the determinant of the result at a w row of 1 is inf − inf.
        let far = Homography([[1e300, 0.0, 1e300], [0.0, 1e300, 0.0], [1e100, 0.0, 2e100]]);
        let t = Affine::scale(1e10);
        let c = far.conjugated(t).unwrap();
        assert!(c.det().is_nan());
        assert_eq!(Homography::from_array(c.to_array()), Some(c));
        for p in [Point::new(3.0, -2.0), Point::new(0.5, 7.0)] {
            let want = t * far.apply(p).unwrap();
            assert!(c.apply(t * p).unwrap().distance(want) <= 1e-9 * want.to_vec2().hypot(), "{c:?}");
        }
        // A determinant that does not overflow is judged against the size of the map's entries:
        // the identity with every entry 1e-6 is the identity, and two equal rows are not invertible.
        let small = [1e-6, 0.0, 0.0, 0.0, 1e-6, 0.0, 0.0, 0.0, 1e-6];
        let q = Homography::from_array(small).unwrap().apply(Point::new(2.0, 3.0)).unwrap();
        assert!(q.distance(Point::new(2.0, 3.0)) < 1e-12);
        assert!(Homography::from_array([1e-6, 2e-6, 0.0, 1e-6, 2e-6, 0.0, 0.0, 0.0, 1e-6]).is_none());
        assert!(Homography::from_array([f64::NAN; 9]).is_none());
        assert!(Homography::from_array([0.0; 9]).is_none());
    }

    #[test]
    fn the_checks_do_not_depend_on_the_scale_of_the_map() {
        let times = |h: Homography, f: f64| Homography(h.0.map(|row| row.map(|v| v * f)));
        // The same map with its w row's largest entry 1.
        let unit = |h: Homography| {
            let k = h.0[2].iter().fold(0.0_f64, |k, v| k.max(v.abs()));
            Homography(h.0.map(|row| row.map(|v| v / k)))
        };
        // The horizon of h() is 0.001x + 0.002y + 1 = 0: two points in front of it, one with w of
        // about 5e-10 (too close to it to draw), and two beyond it.
        let pts =
            [Point::new(30.0, -12.0), Point::new(-999.99, 0.0), Point::new(-999.9999995, 0.0), Point::new(-2000.0, 0.0), Point::new(0.0, -600.0)];
        let at_unit: Vec<_> = pts.iter().map(|&p| h().apply(p)).collect();
        assert!(at_unit[0].is_some() && at_unit[1].is_some());
        assert!(at_unit[2..].iter().all(Option::is_none));
        let inv = h().inverse().unwrap();
        // Two equal rows (up to a factor of 2), with entries that are exact in binary.
        let flat = Homography([[1.0, 2.0, 3.0], [2.0, 4.0, 6.0], [0.25, 0.5, 1.0]]);
        assert!(!flat.is_invertible());
        assert_eq!(flat.inverse(), None);
        for f in [1e-200, 1e-12, 1e12, 1e200] {
            let s = times(h(), f);
            for (&p, want) in pts.iter().zip(&at_unit) {
                match (s.apply(p), want) {
                    (Some(q), Some(w)) => assert!(q.distance(*w) <= 1e-6 * w.to_vec2().hypot(), "{f} {p:?}"),
                    (None, None) => {}
                    (got, _) => panic!("{f} {p:?}: {got:?}"),
                }
            }
            assert!(s.is_invertible(), "{f}");
            let si = unit(s.inverse().unwrap_or_else(|| panic!("{f}")));
            for (a, b) in si.to_array().iter().zip(unit(inv).to_array()) {
                assert!((a - b).abs() <= 1e-9 * b.abs().max(1.0), "{f}: {si:?}");
            }
            let sf = times(flat, f);
            assert!(!sf.is_invertible(), "{f}");
            assert_eq!(sf.inverse(), None, "{f}");
        }
        // Entries of very different sizes: the determinant is about 1e-7 and every entry of the
        // inverse is finite, though one power of two that brings the products near 1 takes the
        // first entry past the largest finite number.
        let uneven = Homography([[1e307, 0.0, 0.0], [0.0, 1e-157, 0.0], [0.0, 0.0, 1e-157]]);
        assert!(uneven.det().is_finite());
        assert_eq!(Homography::from_array(uneven.to_array()), Some(uneven));
        let inv = uneven.inverse().unwrap();
        for p in [Point::new(1e-300, 2.0), Point::new(-3e-299, -0.5)] {
            let q = uneven.apply(p).unwrap();
            let back = inv.apply(q).unwrap();
            assert!((back.x - p.x).abs() <= 1e-9 * p.x.abs() && (back.y - p.y).abs() <= 1e-9 * p.y.abs(), "{p:?} {back:?}");
        }
        // A normal determinant (1e-200) whose cofactor 1e-200 · 1e-200 is below the smallest
        // positive f64: the inverse still has a w row that is not 0.
        let tiny = Homography([[1e-200, 0.0, 0.0], [0.0, 1e-200, 0.0], [0.0, 0.0, 1e200]]);
        assert_eq!(Homography::from_array(tiny.to_array()), Some(tiny));
        let inv = tiny.inverse().unwrap();
        for p in [Point::new(1e300, -2e299), Point::new(-4e299, 5e300)] {
            let q = tiny.apply(p).unwrap();
            let back = inv.apply(q).unwrap();
            assert!((back.x - p.x).abs() <= 1e-9 * p.x.abs() && (back.y - p.y).abs() <= 1e-9 * p.y.abs(), "{p:?} {back:?}");
        }
        // A normal determinant (1e-300) whose cofactor 1e-300 · 1e-300 is below the smallest
        // positive f64, and where one power of two that brings the products near 1 takes 1e300 past
        // the largest finite number: the inverse is still diag(1e300, 1e-300, 1e300).
        let spread = Homography([[1e-300, 0.0, 0.0], [0.0, 1e300, 0.0], [0.0, 0.0, 1e-300]]);
        assert_eq!(Homography::from_array(spread.to_array()), Some(spread));
        let inv = spread.inverse().unwrap();
        let want = [1e300, 0.0, 0.0, 0.0, 1e-300, 0.0, 0.0, 0.0, 1e300];
        for (v, w) in inv.to_array().iter().zip(want) {
            assert!((v - w).abs() <= 1e-12 * w.abs(), "{inv:?}");
        }
    }

    #[test]
    fn an_invertible_map_has_an_inverse_at_any_scale() {
        let times = |h: Homography, f: f64| Homography(h.0.map(|row| row.map(|v| v * f)));
        // Determinant 1e-17 against a largest product of 1e-9 (and 1e-8 at 1e3 times the map): its
        // inverse is finite, though that inverse's own determinant is near 1e-16 of its largest
        // product.
        let skew = Homography([[-1000.0, -1e-8, -1e-5], [0.0, 1e-4, -1e7], [1e-8, 0.0, 1e-8]]);
        let exact = [1e5, 10.0, 1e16 + 1e8, -1e16, -1e12 + 1e4, -1e27, -1e5, -10.0, -1e16];
        for f in [1.0, 1e3] {
            let s = times(skew, f);
            assert_eq!(Homography::from_array(s.to_array()), Some(s), "{f}");
            let inv = s.inverse().unwrap_or_else(|| panic!("{f}"));
            for (v, w) in inv.to_array().iter().zip(exact) {
                assert!((v * f - w).abs() <= 1e-9 * w.abs(), "{f}: {inv:?}");
            }
            // The map times its inverse is the identity, to within the rounding of the products
            // each entry sums (they cancel by up to 1e23 in the second row).
            let id = s.then_after(&inv);
            for (r, row) in id.0.iter().enumerate() {
                for (k, v) in row.iter().enumerate() {
                    let size: f64 = (0..3).map(|i| (s.0[r][i] * inv.0[i][k]).abs()).sum();
                    let want = if r == k { 1.0 } else { 0.0 };
                    assert!((v - want).abs() <= 1e-12 * size, "{f}: {id:?}");
                }
            }
        }
    }

    #[test]
    fn the_inverse_of_an_invertible_map_is_invertible() {
        // The inverse's determinant is about 1e-16 of its largest product, but its adjugate is
        // the map itself times a number.
        let skew = Homography([[-1000.0, -1e-8, -1e-5], [0.0, 1e-4, -1e7], [1e-8, 0.0, 1e-8]]);
        let inv = skew.inverse().unwrap();
        assert_eq!(Homography::from_array(inv.to_array()), Some(inv));
        let back = inv.inverse().unwrap_or_else(|| panic!("{inv:?}"));
        // The same map as `skew`, to within the rounding of the stored inverse: each entry within
        // 1e-6 of the largest magnitude in its row, and points map to within 1e-6 of where `skew`
        // maps them.
        let k = back.0[2][0] / skew.0[2][0];
        assert!(k > 0.0, "{k}");
        for (r, row) in back.0.iter().enumerate() {
            let size = skew.0[r].iter().fold(0.0_f64, |k, v| k.max(v.abs()));
            for (c, v) in row.iter().enumerate() {
                assert!((v / k - skew.0[r][c]).abs() <= 1e-6 * size, "{back:?}");
            }
        }
        for p in [Point::new(3.0, -2.0), Point::new(-0.5, 7.0)] {
            let q = skew.apply(p).unwrap();
            assert!(back.apply(p).unwrap().distance(q) <= 1e-6 * q.to_vec2().hypot(), "{p:?}");
        }
        // Maps whose determinant is 0 are refused, though the cofactors of the first one cancel
        // to about 2e-12 of their products: its rows are close to parallel and the third is the sum of
        // the first two.
        let two = pow2(40);
        let near =
            Homography([[two + 1.0, two + 3.0, two + 7.0], [two + 2.0, two + 5.0, two + 11.0], [2.0 * two + 3.0, 2.0 * two + 8.0, 2.0 * two + 18.0]]);
        let flat = Homography([[1.0, 2.0, 3.0], [2.0, 4.0, 6.0], [0.25, 0.5, 1.0]]);
        let times = |h: Homography, f: f64| Homography(h.0.map(|row| row.map(|v| v * f)));
        for h in [near, flat] {
            for f in [pow2(-900), pow2(-40), 1.0, pow2(40), pow2(900)] {
                let s = times(h, f);
                assert!(!s.is_invertible(), "{f} {s:?}");
                assert_eq!(s.inverse(), None, "{f}");
                assert_eq!(Homography::from_array(s.to_array()), None, "{f}");
            }
        }
    }

    #[test]
    fn conjugating_does_not_depend_on_the_scale_of_the_affine() {
        let unit = |h: Homography| {
            let k = h.0[2].iter().fold(0.0_f64, |k, v| k.max(v.abs()));
            Homography(h.0.map(|row| row.map(|v| v / k)))
        };
        let m = h().0;
        for s in [1e-200, 1e-8, 1.0, 1e8, 1e200] {
            let t = Affine::scale(s);
            let c = h().conjugated(t).unwrap_or_else(|| panic!("{s}"));
            // The scale times h() times its inverse.
            let want = unit(Homography([[m[0][0], m[0][1], s * m[0][2]], [m[1][0], m[1][1], s * m[1][2]], [m[2][0] / s, m[2][1] / s, m[2][2]]]));
            for (v, w) in unit(c).to_array().iter().zip(want.to_array()) {
                assert!((v - w).abs() <= 1e-12 * w.abs().max(1.0), "{s}: {c:?}");
            }
            // At 1e-200 the w row's first two entries are about 1e197, so these points, whose w is
            // about 1, count as on the horizon (see `apply`).
            if s < 1e-8 {
                continue;
            }
            for p in [Point::new(3.0, 7.0), Point::new(-40.0, 12.0)] {
                let want = t * h().apply(p).unwrap();
                assert!(c.apply(t * p).unwrap().distance(want) <= 1e-9 * want.to_vec2().hypot(), "{s} {p:?}");
            }
        }
        // An affine whose columns are parallel is refused at every scale.
        for s in [1e-200, 1e-8, 1.0, 1e8, 1e200] {
            let flat = Affine::new([s, 2.0 * s, 2.0 * s, 4.0 * s, 5.0, 6.0]);
            assert_eq!(h().conjugated(flat), None, "{s}");
            assert_eq!(h().conjugated(Affine::scale(s) * Affine::scale_non_uniform(1.0, 0.0)), None, "{s}");
        }
        assert_eq!(h().conjugated(Affine::new([f64::NAN, 0.0, 0.0, 1.0, 0.0, 0.0])), None);
        assert_eq!(h().conjugated(Affine::scale(f64::INFINITY)), None);
    }

    #[test]
    fn conjugating_a_map_at_a_tiny_scale_gives_an_invertible_map() {
        // Against these affines the first two rows of the direct product underflow to 0.
        let tiny = Homography(Homography::IDENTITY.0.map(|row| row.map(|v| v * 5e-324)));
        for s in [1e-200, 1e-8] {
            let c = tiny.conjugated(Affine::scale(s)).unwrap_or_else(|| panic!("{s}"));
            assert_eq!(Homography::from_array(c.to_array()), Some(c), "{s}");
            let k = c.0[2][2];
            for (r, row) in c.0.iter().enumerate() {
                for (j, v) in row.iter().enumerate() {
                    let want = if r == j { k } else { 0.0 };
                    assert!(k != 0.0 && (v - want).abs() <= 1e-12 * k.abs(), "{s}: {c:?}");
                }
            }
        }
    }

    #[test]
    fn a_map_at_a_tiny_scale_maps_points_as_at_unit_scale() {
        for f in [5e-324, 1e-300] {
            let h = Homography(Homography::IDENTITY.0.map(|row| row.map(|v| v * f)));
            assert_eq!(Homography::from_array(h.to_array()), Some(h), "{f}");
            for p in [Point::new(0.5, 0.5), Point::new(1e-20, 1e-20)] {
                let q = h.apply(p).unwrap_or_else(|| panic!("{f}: {p:?}"));
                assert!((q.x - p.x).abs() <= 1e-12 * p.x && (q.y - p.y).abs() <= 1e-12 * p.y, "{f}: {p:?} -> {q:?}");
            }
        }
        // A perspective map whose entries are exact at 2^-1060 times their size, so the scaled
        // copy holds the same map.
        let unit = Homography([[1.0, 0.0, 2.0], [0.0, 1.0, 3.0], [0.5, 0.25, 1.0]]);
        let tiny = Homography(unit.0.map(|row| row.map(|v| v * pow2(-1022) * pow2(-38))));
        let p = Point::new(0.3, 0.7);
        let (a, b) = (tiny.affine_at(p).unwrap(), unit.affine_at(p).unwrap());
        for (u, v) in a.as_coeffs().iter().zip(b.as_coeffs()) {
            assert!((u - v).abs() <= 1e-12 * v.abs().max(1.0), "{a:?} {b:?}");
        }
    }

    #[test]
    fn a_map_at_a_small_scale_maps_a_far_point() {
        // Every product of an entry and a coordinate is finite, but the entries brought to 1/2 or
        // more would overflow against this point.
        let h = Homography([[1e-6, 1e-6, 0.0], [0.0, 1e-6, 0.0], [1e-6, 0.0, 1e-6]]);
        let p = Point::new(1.75e308, 1.75e308);
        let q = h.apply(p).unwrap_or_else(|| panic!("{p:?}"));
        assert!((q.x - 2.0).abs() <= 1e-12 && (q.y - 1.0).abs() <= 1e-12, "{q:?}");
        assert!(h.affine_at(p).is_some());
    }

    #[test]
    fn a_product_that_overflows_is_none() {
        // Each factor is finite and invertible, but the product's first entry is 1e400.
        let big = Homography([[1e200, 0.0, 0.0], [0.0, 1e200, 0.0], [0.0, 0.0, 1e200]]);
        assert!(Homography::from_array(big.to_array()).is_some());
        assert_eq!(big.checked_then_after(&big), None);
        let nan = Homography([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, f64::NAN]]);
        assert_eq!(h().checked_then_after(&nan), None);
        assert_eq!(h().checked_then_after(&Homography::IDENTITY), Some(h()));
        let inv = h().inverse().unwrap();
        assert_eq!(h().checked_then_after(&inv), Some(h().then_after(&inv)));
    }

    #[test]
    fn a_derivative_that_overflows_is_none() {
        // The point maps to 1e300, but the derivative there is 1e302 / 1e-8.
        let steep = Homography([[1e302, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1e-8]]);
        let p = Point::new(1e-10, 0.0);
        assert!(steep.apply(p).is_some());
        assert_eq!(steep.affine_at(p), None);
    }

    #[test]
    fn a_conjugate_that_overflows_is_scaled_down() {
        // A move by 10, scaled by 1e307: seen through a scale by 10 it moves by 100, but the
        // product's corner entry is 1e309.
        let big = Homography([[1e307, 0.0, 1e308], [0.0, 1e307, 0.0], [0.0, 0.0, 1e307]]);
        let t = Affine::scale(10.0);
        let c = big.conjugated(t).unwrap();
        assert!(c.is_finite());
        assert_eq!(c.0[2], [0.0, 0.0, 1.0]);
        assert_eq!(Homography::from_array(c.to_array()), Some(c));
        for p in [Point::new(3.0, 7.0), Point::new(-4.0, 1.5)] {
            assert!(c.apply(t * p).unwrap().distance(t * big.apply(p).unwrap()) < 1e-9);
            assert!(c.apply(p).unwrap().distance(p + kurbo::Vec2::new(100.0, 0.0)) < 1e-9);
        }
        // A move by 1 scaled by 1e307, seen through a scale by 1e7: a move by 1e7, whose
        // determinant is 1 at a w row of 1 but 1e-21 at a largest entry of 1.
        let one = Homography([[1e307, 0.0, 1e307], [0.0, 1e307, 0.0], [0.0, 0.0, 1e307]]);
        assert!(Homography::from_array(one.to_array()).is_some());
        let c = one.conjugated(Affine::scale(1e7)).unwrap();
        assert_eq!(Homography::from_array(c.to_array()), Some(c));
        let p = Point::new(3.0, -2.0);
        assert!(c.apply(p).unwrap().distance(p + kurbo::Vec2::new(1e7, 0.0)) < 1e-6);
        // A scale by 1e10 seen through a scale by 1e10: still a scale by 1e10, which every point
        // reaches only when w is not left at 1e-10.
        let wide = Homography([[1e300, 0.0, 0.0], [0.0, 1e300, 0.0], [0.0, 0.0, 1e290]]);
        let c = wide.conjugated(Affine::scale(1e10)).unwrap();
        assert_eq!(Homography::from_array(c.to_array()), Some(c));
        for p in [Point::new(1.0, 2.0), Point::new(-5.0, 0.5)] {
            assert!(c.apply(p).unwrap().distance(Point::new(p.x * 1e10, p.y * 1e10)) < 1e-3);
        }
        // A scale by 2 seen through a move by 1e308: p goes to 2p - 1e308, though the product's
        // corner entry is 2 * -1e308 + 1e308 and the moves scaled to a largest entry of 1 leave
        // their w rows at 1e-308.
        let two = Homography([[2.0, 0.0, 0.0], [0.0, 2.0, 0.0], [0.0, 0.0, 1.0]]);
        let c = two.conjugated(Affine::translate((1e308, 0.0))).unwrap();
        let want = [2.0, 0.0, -1e308, 0.0, 2.0, 0.0, 0.0, 0.0, 1.0];
        for (v, w) in c.to_array().iter().zip(want) {
            assert!((v - w).abs() <= 1e-9 * w.abs().max(1.0), "{c:?}");
        }
        assert_eq!(Homography::from_array(c.to_array()), Some(c));
        // A scale by (1e160, 1e-160) seen through the same scale is itself, though the product's
        // first entry is 1e320 and a scale to a largest entry of 1 takes its second to 1e-480.
        let thin = Homography([[1e160, 0.0, 0.0], [0.0, 1e-160, 0.0], [0.0, 0.0, 1.0]]);
        assert!(Homography::from_array(thin.to_array()).is_some());
        let c = thin.conjugated(Affine::scale_non_uniform(1e160, 1e-160)).unwrap();
        for (v, w) in c.to_array().iter().zip(thin.to_array()) {
            assert!((v - w).abs() <= 1e-9 * w.abs(), "{c:?}");
        }
        assert_eq!(Homography::from_array(c.to_array()), Some(c));
        // A move by 1e200 seen through a scale by 1e150 sends every point to about (1e350, 0),
        // which no invertible map of finite numbers draws.
        let far = Homography([[1.0, 0.0, 1e200], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]);
        assert_eq!(far.conjugated(Affine::scale(1e150)), None);
        assert!(far.conjugated(Affine::scale(2.0)).is_some());
        // A product that does not overflow is kept as it is.
        assert_eq!(far.conjugated(Affine::scale(2.0)).unwrap().0[0][2], 2e200);
    }

    #[test]
    fn rect_bbox_is_the_corners_box() {
        let b = h().map_rect_bbox(Rect::new(0.0, 0.0, 10.0, 20.0)).unwrap();
        for p in [Point::new(0.0, 0.0), Point::new(10.0, 20.0), Point::new(10.0, 0.0), Point::new(5.0, 10.0)] {
            let q = h().apply(p).unwrap();
            assert!(b.inflate(1e-9, 1e-9).contains(q));
        }
    }
}
