//! 3×3 matrices, chromaticities, RGB spaces and Bradford chromatic adaptation.

use craft_cms::math;
use craft_color::Primaries;
use serde::{Deserialize, Serialize};

/// A row-major 3×3 matrix.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Mat3(pub [[f64; 3]; 3]);

impl Mat3 {
    pub const IDENTITY: Mat3 = Mat3([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]);

    pub const fn diag(a: f64, b: f64, c: f64) -> Mat3 {
        Mat3([[a, 0.0, 0.0], [0.0, b, 0.0], [0.0, 0.0, c]])
    }

    /// `self × o`: applying the product applies `o` first ([`math::mul`]).
    pub fn mul(&self, o: &Mat3) -> Mat3 {
        Mat3(math::mul(&self.0, &o.0))
    }

    /// [`math::apply`].
    pub fn apply(&self, v: [f64; 3]) -> [f64; 3] {
        math::apply(&self.0, v)
    }

    /// [`math::det`].
    pub fn determinant(&self) -> f64 {
        math::det(&self.0)
    }

    /// The inverse, or `None` when the matrix holds a non-finite value, is singular to within
    /// rounding (the determinant is at most 1e-12 times the cube of the largest entry), or has an
    /// inverse with an entry beyond the range of `f64` ([`math::inverse`]).
    pub fn inverse(&self) -> Option<Mat3> {
        math::inverse(&self.0).map(Mat3)
    }

    pub fn transpose(&self) -> Mat3 {
        let m = &self.0;
        Mat3([[m[0][0], m[1][0], m[2][0]], [m[0][1], m[1][1], m[2][1]], [m[0][2], m[1][2], m[2][2]]])
    }

    /// The matrix as `f32`, row-major, for per-pixel code and GPU uniforms.
    pub fn to_f32(&self) -> [[f32; 3]; 3] {
        self.0.map(|r| r.map(|v| v as f32))
    }
}

/// A row-major array, the form `craft_cms::math::Mat3` takes.
impl From<[[f64; 3]; 3]> for Mat3 {
    fn from(m: [[f64; 3]; 3]) -> Mat3 {
        Mat3(m)
    }
}

impl From<Mat3> for [[f64; 3]; 3] {
    fn from(m: Mat3) -> [[f64; 3]; 3] {
        m.0
    }
}

/// A CIE 1931 xy chromaticity.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Xy {
    pub x: f64,
    pub y: f64,
}

impl Xy {
    pub const fn new(x: f64, y: f64) -> Xy {
        Xy { x, y }
    }

    /// XYZ with Y = 1, or `None` when `y` is not above 0, a coordinate is not finite, or `y` is
    /// so small that X or Z overflows.
    pub fn to_xyz(self) -> Option<[f64; 3]> {
        if !(self.x.is_finite() && self.y.is_finite() && self.y > 0.0) {
            return None;
        }
        let xyz = math::xy_to_xyz([self.x, self.y]);
        xyz.iter().all(|v| v.is_finite()).then_some(xyz)
    }

    /// The chromaticity of an XYZ triple, or `None` when X + Y + Z is not above 0.
    pub fn from_xyz(v: [f64; 3]) -> Option<Xy> {
        let s = v[0] + v[1] + v[2];
        if !(s.is_finite() && s > 0.0) {
            return None;
        }
        Some(Xy::new(v[0] / s, v[1] / s))
    }
}

/// CIE standard illuminant D65, as ITU-R BT.709 and BT.2020 give it.
pub const D65: Xy = Xy::new(0.3127, 0.3290);
/// CIE illuminant D50, the ICC profile connection space white.
pub const D50: Xy = Xy::new(0.3457, 0.3585);

/// An RGB color space: three primaries and a white point. A space read at run time (from a
/// file's primaries or a stream's mastering metadata) is built with a struct literal.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct RgbSpace {
    pub r: Xy,
    pub g: Xy,
    pub b: Xy,
    pub white: Xy,
}

/// The ITU-R BT.709 primaries, which sRGB (IEC 61966-2-1) shares.
pub const SRGB: RgbSpace = RgbSpace { r: Xy::new(0.64, 0.33), g: Xy::new(0.30, 0.60), b: Xy::new(0.15, 0.06), white: D65 };
/// ITU-R BT.601, 625-line systems.
pub const REC601_625: RgbSpace = RgbSpace { r: Xy::new(0.64, 0.33), g: Xy::new(0.29, 0.60), b: Xy::new(0.15, 0.06), white: D65 };
/// ITU-R BT.601, 525-line systems.
pub const REC601_525: RgbSpace = RgbSpace { r: Xy::new(0.630, 0.340), g: Xy::new(0.310, 0.595), b: Xy::new(0.155, 0.070), white: D65 };
/// ITU-R BT.2020 (and BT.2100).
pub const REC2020: RgbSpace = RgbSpace { r: Xy::new(0.708, 0.292), g: Xy::new(0.170, 0.797), b: Xy::new(0.131, 0.046), white: D65 };
/// The DCI-P3 primaries with a D65 white (SMPTE EG 432-1).
pub const DISPLAY_P3: RgbSpace = RgbSpace { r: Xy::new(0.680, 0.320), g: Xy::new(0.265, 0.690), b: Xy::new(0.150, 0.060), white: D65 };

impl From<Primaries> for RgbSpace {
    fn from(p: Primaries) -> RgbSpace {
        match p {
            Primaries::Bt709 => SRGB,
            Primaries::Bt601_625 => REC601_625,
            Primaries::Bt601_525 => REC601_525,
            Primaries::Bt2020 => REC2020,
            Primaries::P3D65 => DISPLAY_P3,
        }
    }
}

impl RgbSpace {
    /// Linear RGB in this space to XYZ, with the space's white at Y = 1 (the normalized primary
    /// matrix of SMPTE RP 177). Primaries or a white that do not define a space (collinear
    /// primaries, a `y` of 0, a white on a primary or on the line between two) give the identity.
    pub fn to_xyz(&self) -> Mat3 {
        let (Some(r), Some(g), Some(b), Some(w)) = (self.r.to_xyz(), self.g.to_xyz(), self.b.to_xyz(), self.white.to_xyz()) else {
            return Mat3::IDENTITY;
        };
        let m = Mat3([[r[0], g[0], b[0]], [r[1], g[1], b[1]], [r[2], g[2], b[2]]]);
        let Some(inv) = m.inverse() else { return Mat3::IDENTITY };
        let s = inv.apply(w);
        let npm = m.mul(&Mat3::diag(s[0], s[1], s[2]));
        // A white that gives a primary no weight leaves a matrix with no inverse.
        if npm.inverse().is_none() { Mat3::IDENTITY } else { npm }
    }

    /// XYZ to linear RGB in this space: the inverse of [`to_xyz`](Self::to_xyz).
    pub fn from_xyz(&self) -> Mat3 {
        self.to_xyz().inverse().unwrap_or(Mat3::IDENTITY)
    }

    /// The luminance weights of R, G and B (the Y row of [`to_xyz`](Self::to_xyz)).
    pub fn luma(&self) -> [f64; 3] {
        self.to_xyz().0[1]
    }

    /// Linear RGB in `self` to linear RGB in `dst`, Bradford-adapting between the white points.
    pub fn to_space(&self, dst: &RgbSpace) -> Mat3 {
        dst.from_xyz().mul(&bradford(self.white, dst.white)).mul(&self.to_xyz())
    }
}

/// The Bradford cone response matrix (XYZ to sharpened cone responses).
pub const BRADFORD: Mat3 = Mat3(math::BRADFORD);

/// The Bradford chromatic adaptation matrix (XYZ to XYZ) from white `src` to white `dst`. A
/// white that [`Xy::to_xyz`] rejects gives the identity.
pub fn bradford(src: Xy, dst: Xy) -> Mat3 {
    if src == dst {
        return Mat3::IDENTITY;
    }
    match (src.to_xyz(), dst.to_xyz()) {
        (Some(s), Some(d)) => bradford_xyz(s, d),
        _ => Mat3::IDENTITY,
    }
}

/// [`bradford`] for whites given as XYZ. A white with a cone response that is not above 0 or
/// not finite, or a pair whose response ratios overflow, gives the identity.
pub fn bradford_xyz(src: [f64; 3], dst: [f64; 3]) -> Mat3 {
    math::bradford_checked(src, dst).map_or(Mat3::IDENTITY, Mat3)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: &Mat3, b: &[[f64; 3]; 3], tol: f64) {
        for i in 0..3 {
            for j in 0..3 {
                assert!((a.0[i][j] - b[i][j]).abs() < tol, "[{i}][{j}] {} vs {}", a.0[i][j], b[i][j]);
            }
        }
    }

    /// IEC 61966-2-1 gives the sRGB RGB to XYZ matrix to four decimals.
    #[test]
    fn srgb_matrix_matches_iec_61966_2_1() {
        close(&SRGB.to_xyz(), &[[0.4124, 0.3576, 0.1805], [0.2126, 0.7152, 0.0722], [0.0193, 0.1192, 0.9505]], 1e-4);
    }

    /// ITU-R BT.2020 and BT.709 give the luminance coefficients.
    #[test]
    fn luminance_weights_match_bt709_and_bt2020() {
        for (space, want) in [(SRGB, [0.2126, 0.7152, 0.0722]), (REC2020, [0.2627, 0.6780, 0.0593])] {
            let l = space.luma();
            for i in 0..3 {
                assert!((l[i] - want[i]).abs() < 1e-4, "{space:?} {l:?}");
            }
        }
    }

    /// The Bradford matrix from D65 to D50 as Lindbloom's chromatic adaptation tables print it,
    /// with his reference whites.
    #[test]
    fn bradford_d65_to_d50_matches_the_published_matrix() {
        let m = bradford_xyz([0.95047, 1.0, 1.08883], [0.96422, 1.0, 0.82521]);
        let want = [[1.0478112, 0.0228866, -0.0501270], [0.0295424, 0.9904844, -0.0170491], [-0.0092345, 0.0150436, 0.7521316]];
        close(&m, &want, 1e-6);
    }

    #[test]
    fn adaptation_maps_white_to_white_and_round_trips() {
        let m = bradford(D65, D50);
        let w = m.apply(D65.to_xyz().unwrap());
        let d50 = D50.to_xyz().unwrap();
        for i in 0..3 {
            assert!((w[i] - d50[i]).abs() < 1e-12);
        }
        let back = bradford(D50, D65).mul(&m);
        close(&back, &Mat3::IDENTITY.0, 1e-12);
        let v = SRGB.to_space(&REC2020).apply([1.0, 1.0, 1.0]);
        assert!(v.iter().all(|c| (c - 1.0).abs() < 1e-9), "{v:?}");
    }

    /// The BT.709 to BT.2020 conversion matrix of Report ITU-R BT.2087 (equation 2).
    #[test]
    fn bt709_to_bt2020_matches_bt2087() {
        let want = [[0.6274, 0.3293, 0.0433], [0.0691, 0.9195, 0.0114], [0.0164, 0.0880, 0.8956]];
        close(&SRGB.to_space(&REC2020), &want, 1e-4);
    }

    #[test]
    fn primaries_tags_map_to_spaces() {
        assert_eq!(RgbSpace::from(Primaries::Bt2020), REC2020);
        assert_eq!(RgbSpace::from(Primaries::default()), SRGB);
    }

    #[test]
    fn hostile_whites_and_primaries_do_not_produce_nan() {
        assert_eq!(Xy::new(0.3, 0.0).to_xyz(), None);
        assert_eq!(Xy::new(f64::NAN, 0.3).to_xyz(), None);
        assert_eq!(Xy::new(0.3, 1e-320).to_xyz(), None);
        assert_eq!(bradford(Xy::new(0.3, 1e-320), D65), Mat3::IDENTITY);
        assert_eq!(Xy::from_xyz([0.0, 0.0, 0.0]), None);
        assert_eq!(bradford(Xy::new(0.3, 0.0), D65), Mat3::IDENTITY);
        assert_eq!(bradford(D65, Xy::new(f64::INFINITY, 0.3)), Mat3::IDENTITY);
        assert_eq!(bradford_xyz([-1.0, 1.0, 1.0], [0.95, 1.0, 1.09]), Mat3::IDENTITY);
        let flat = RgbSpace { r: Xy::new(0.2, 0.2), g: Xy::new(0.3, 0.3), b: Xy::new(0.4, 0.4), white: D65 };
        assert_eq!(flat.to_xyz(), Mat3::IDENTITY);
        assert_eq!(Mat3([[f64::NAN; 3]; 3]).inverse(), None);
        // The determinant, 1e-312, has no finite reciprocal; the inverse itself is finite.
        let tiny = Mat3::diag(1e-104, 1e-104, 1e-104).inverse().unwrap();
        close(&tiny, &[[1e104, 0.0, 0.0], [0.0, 1e104, 0.0], [0.0, 0.0, 1e104]], 1e93);
        // A determinant beyond f64 in either direction still leaves a finite inverse.
        let small = Mat3::diag(1e-200, 1e-200, 1e-200).inverse().unwrap();
        close(&small, &Mat3::diag(1e200, 1e200, 1e200).0, 1e186);
        let large = Mat3::diag(1e110, 1e110, 1e110).inverse().unwrap();
        close(&large, &Mat3::diag(1e-110, 1e-110, 1e-110).0, 1e-124);
        // An inverse with entries beyond f64 is no inverse.
        assert_eq!(Mat3::diag(1e-310, 1e-310, 1e-310).inverse(), None);
        assert_eq!(Mat3([[0.0; 3]; 3]).inverse(), None);
        assert_eq!(Mat3::diag(f64::MAX, f64::MAX, f64::MAX).inverse().map(|m| m.0[0][0]), Some(1.0 / f64::MAX));
        // A white on a primary leaves two primaries with no weight: no space.
        let on_red = RgbSpace { white: SRGB.r, ..SRGB };
        assert_eq!(on_red.to_xyz(), Mat3::IDENTITY);
        assert_eq!(on_red.from_xyz(), Mat3::IDENTITY);
        // Cone responses this small give scale ratios beyond f64.
        assert_eq!(bradford_xyz([1e-320; 3], D65.to_xyz().unwrap()), Mat3::IDENTITY);
    }

    /// A space built from primaries read at run time saves and loads, and its matrix converts
    /// to and from a plain array.
    #[test]
    fn spaces_save_and_load() {
        let read = RgbSpace { r: Xy::new(0.7, 0.29), g: Xy::new(0.16, 0.8), b: Xy::new(0.13, 0.05), white: D50 };
        let json = serde_json::to_string(&read).unwrap();
        assert_eq!(serde_json::from_str::<RgbSpace>(&json).unwrap(), read);
        let m: [[f64; 3]; 3] = read.to_xyz().into();
        assert_eq!(Mat3::from(m), read.to_xyz());
    }
}
