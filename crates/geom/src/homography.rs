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
    pub fn from_quads(src: &[Point; 4], dst: &[Point; 4]) -> Option<Homography> {
        // Failed feature matches and invalid perspective parameters can contain
        // non-finite coordinates. Do not return a transform that poisons rendering.
        if src.iter().chain(dst.iter()).any(|p| !p.x.is_finite() || !p.y.is_finite()) {
            return None;
        }
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
