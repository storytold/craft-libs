//! Projective transforms (Upright, guided perspective, panorama alignment).

use serde::{Deserialize, Serialize};

use crate::{Affine, Point, Real};

/// Row-major 3×3 matrix mapping homogeneous points: p' = H·p.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Homography(pub [f64; 9]);

impl Default for Homography {
    fn default() -> Self {
        Homography::IDENTITY
    }
}

impl Homography {
    pub const IDENTITY: Homography = Homography([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]);

    pub fn from_affine(a: &Affine) -> Homography {
        let [a0, b, c, d, e, f] = a.0;
        Homography([a0, c, e, b, d, f, 0.0, 0.0, 1.0])
    }

    pub fn apply(&self, p: Point) -> Point {
        let (x, y) = self.apply_real(p.x, p.y);
        Point::new(x, y)
    }

    /// [`Homography::apply`] for any [`Real`] (e.g. [`Interval`](crate::Interval) bounds over many points).
    pub fn apply_real<T: Real>(&self, x: T, y: T) -> (T, T) {
        let m = &self.0;
        let w = (x * m[6] + y * m[7] + m[8]).clamp_tiny(1e-300);
        ((x * m[0] + y * m[1] + m[2]) / w, (x * m[3] + y * m[4] + m[5]) / w)
    }

    pub fn mul(&self, o: &Homography) -> Homography {
        let (a, b) = (&self.0, &o.0);
        let mut r = [0.0; 9];
        for i in 0..3 {
            for j in 0..3 {
                r[i * 3 + j] = (0..3).map(|k| a[i * 3 + k] * b[k * 3 + j]).sum();
            }
        }
        Homography(r)
    }

    pub fn inverse(&self) -> Option<Homography> {
        let m = &self.0;
        let c00 = m[4] * m[8] - m[5] * m[7];
        let c01 = m[5] * m[6] - m[3] * m[8];
        let c02 = m[3] * m[7] - m[4] * m[6];
        let det = m[0] * c00 + m[1] * c01 + m[2] * c02;
        if !det.is_finite() || det.abs() < 1e-300 {
            return None;
        }
        let inv = 1.0 / det;
        let coefficients = [
            c00 * inv,
            (m[2] * m[7] - m[1] * m[8]) * inv,
            (m[1] * m[5] - m[2] * m[4]) * inv,
            c01 * inv,
            (m[0] * m[8] - m[2] * m[6]) * inv,
            (m[2] * m[3] - m[0] * m[5]) * inv,
            c02 * inv,
            (m[1] * m[6] - m[0] * m[7]) * inv,
            (m[0] * m[4] - m[1] * m[3]) * inv,
        ];
        coefficients.iter().all(|v| v.is_finite()).then_some(Homography(coefficients))
    }

    /// The homography mapping `src[i]` → `dst[i]` (4 correspondences, DLT with h33 = 1).
    ///
    /// h33 = 1 cannot express a mapping that sends the origin to infinity (h33 = 0), though the quads themselves
    /// may be finite and well away from it. When the solve fails it is tried again with the origin moved to the
    /// middle of `src`, which a convex quad maps to a finite point; the result then has whatever h33 it needs.
    pub fn from_quads(src: &[Point; 4], dst: &[Point; 4]) -> Option<Homography> {
        // Failed feature matches and invalid perspective parameters can contain
        // non-finite coordinates. Do not return a transform that poisons rendering.
        if src.iter().chain(dst.iter()).any(|p| !p.x.is_finite() || !p.y.is_finite()) {
            return None;
        }
        Self::solve_h33_one(src, dst)
            .or_else(|| {
                let (cx, cy) = (src.iter().map(|p| p.x).sum::<f64>() / 4.0, src.iter().map(|p| p.y).sum::<f64>() / 4.0);
                let moved = src.map(|p| Point::new(p.x - cx, p.y - cy));
                let h = Self::solve_h33_one(&moved, dst)?.mul(&Homography([1.0, 0.0, -cx, 0.0, 1.0, -cy, 0.0, 0.0, 1.0]));
                h.0.iter().all(|v| v.is_finite()).then_some(h)
            })
            // A destination with corners on top of each other or in a line can still solve, to a matrix that sends a
            // corner to infinity or flattens the plane. Such a matrix does not map the quads and has no inverse.
            .filter(|h| {
                let scale = dst.iter().fold(1.0f64, |m, p| m.max(p.x.abs()).max(p.y.abs()));
                h.inverse().is_some() && src.iter().zip(dst).all(|(s, d)| h.apply(*s).dist(*d) <= 1e-6 * scale)
            })
    }

    /// [`Self::from_quads`] for finite points, with h33 fixed at 1.
    fn solve_h33_one(src: &[Point; 4], dst: &[Point; 4]) -> Option<Homography> {
        let mut a = [[0.0f64; 9]; 8];
        for i in 0..4 {
            let (x, y) = (src[i].x, src[i].y);
            let (u, v) = (dst[i].x, dst[i].y);
            a[2 * i] = [x, y, 1.0, 0.0, 0.0, 0.0, -u * x, -u * y, u];
            a[2 * i + 1] = [0.0, 0.0, 0.0, x, y, 1.0, -v * x, -v * y, v];
        }
        // Gaussian elimination with partial pivoting on the 8×8 system (last column = rhs).
        for col in 0..8 {
            let piv = (col..8).max_by(|&i, &j| a[i][col].abs().total_cmp(&a[j][col].abs()))?;
            if !a[piv][col].is_finite() || a[piv][col].abs() < 1e-12 {
                return None;
            }
            a.swap(col, piv);
            for row in 0..8 {
                if row != col {
                    let f = a[row][col] / a[col][col];
                    #[allow(clippy::needless_range_loop)] // `k` indexes row `row` and pivot row `col` of `a` alike
                    for k in col..9 {
                        a[row][k] -= f * a[col][k];
                    }
                }
            }
        }
        let h: Vec<f64> = (0..8).map(|i| a[i][8] / a[i][i]).collect();
        h.iter().all(|v| v.is_finite()).then_some(Homography([h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7], 1.0]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// (x, y) → ((x + 1) / (x + y), y / (x + y)) has h33 = 0: the origin goes to infinity, the quad is nowhere near it.
    #[test]
    fn quad_mapping_that_sends_the_origin_to_infinity() {
        let src = [Point::new(1.0, 1.0), Point::new(2.0, 1.0), Point::new(2.0, 2.0), Point::new(1.0, 2.0)];
        let f = |p: Point| Point::new((p.x + 1.0) / (p.x + p.y), p.y / (p.x + p.y));
        let dst = src.map(f);
        let h = Homography::from_quads(&src, &dst).expect("a convex quad to a convex quad has a mapping");
        for p in src.into_iter().chain([Point::new(1.5, 1.25), Point::new(1.1, 1.9)]) {
            let (got, want) = (h.apply(p), f(p));
            assert!((got.x - want.x).abs() < 1e-9 && (got.y - want.y).abs() < 1e-9, "{p:?}: {got:?} != {want:?}");
        }
    }

    #[test]
    fn quad_to_quad() {
        let src = [Point::new(0.0, 0.0), Point::new(1.0, 0.0), Point::new(1.0, 1.0), Point::new(0.0, 1.0)];
        let dst = [Point::new(0.1, 0.0), Point::new(0.9, 0.05), Point::new(1.0, 1.0), Point::new(0.0, 0.95)];
        let h = Homography::from_quads(&src, &dst).unwrap();
        for i in 0..4 {
            assert!(h.apply(src[i]).dist(dst[i]) < 1e-9);
        }
        let inv = h.inverse().unwrap();
        let p = Point::new(0.3, 0.7);
        assert!(inv.apply(h.apply(p)).dist(p) < 1e-9);
        assert!(h.mul(&inv).apply(p).dist(p) < 1e-9);
    }

    /// Two destination corners in one place: the system solves, to a matrix that maps (1, 0) to (0, 0).
    #[test]
    fn degenerate_destination_quad_rejected() {
        let src = [Point::new(0.0, 0.0), Point::new(1.0, 0.0), Point::new(1.0, 1.0), Point::new(0.0, 1.0)];
        let dst = [Point::new(0.0, 0.0), Point::new(1.0, 0.0), Point::new(0.0, 0.0), Point::new(0.0, 1.0)];
        assert!(Homography::from_quads(&src, &dst).is_none());
    }

    #[test]
    fn degenerate_quad_rejected() {
        let src = [Point::new(0.0, 0.0); 4];
        assert!(Homography::from_quads(&src, &src).is_none());
    }

    #[test]
    fn nonfinite_projective_and_affine_transforms_are_rejected() {
        let src = [Point::new(0.0, 0.0), Point::new(1.0, 0.0), Point::new(1.0, 1.0), Point::new(0.0, 1.0)];
        let mut invalid = src;
        invalid[2].x = f64::NAN;
        assert!(Homography::from_quads(&invalid, &src).is_none());
        invalid[2].x = f64::INFINITY;
        assert!(Homography::from_quads(&src, &invalid).is_none());
        assert!(Homography([f64::NAN, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]).inverse().is_none());
        assert!(Homography([f64::INFINITY, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]).inverse().is_none());
        assert!(Affine([f64::NAN, 0.0, 0.0, 1.0, 0.0, 0.0]).inverse().is_none());
        assert!(Affine([f64::INFINITY, 0.0, 0.0, 1.0, 0.0, 0.0]).inverse().is_none());
        assert_eq!(Affine::IDENTITY.inverse(), Some(Affine::IDENTITY));
    }
}
