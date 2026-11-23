//! OkLab and OkLCh, as Björn Ottosson defined them in "A perceptual color space for image
//! processing" (2020).
//!
//! [`from_linear_srgb`] and [`to_linear_srgb`] use the published linear sRGB matrices.
//! [`OkLab::for_space`] builds the conversion for any [`RgbSpace`] from the published XYZ
//! matrix, adapting the space's white to D65 first (Bradford).

use crate::{D65, Mat3, RgbSpace, bradford};

/// XYZ (D65) to approximate cone responses (the published M1).
pub const XYZ_TO_LMS: Mat3 =
    Mat3([[0.8189330101, 0.3618667424, -0.1288597137], [0.0329845436, 0.9293118715, 0.0361456387], [0.0482003018, 0.2643662691, 0.6338517070]]);
/// Cube-rooted cone responses to Lab (the published M2).
pub const LMS_TO_LAB: Mat3 =
    Mat3([[0.2104542553, 0.7936177850, -0.0040720468], [1.9779984951, -2.4285922050, 0.4505937099], [0.0259040371, 0.7827717662, -0.8086757660]]);

/// Linear sRGB to cone responses (published with the linear sRGB conversion).
pub const SRGB_TO_LMS: Mat3 =
    Mat3([[0.4122214708, 0.5363325363, 0.0514459929], [0.2119034982, 0.6806995451, 0.1073969566], [0.0883024619, 0.2817188376, 0.6299787005]]);
/// Cone responses to linear sRGB (published with the linear sRGB conversion).
pub const LMS_TO_SRGB: Mat3 =
    Mat3([[4.0767416621, -3.3077115913, 0.2309699292], [-1.2684380046, 2.6097574011, -0.3413193965], [-0.0041960863, -0.7034186147, 1.7076147010]]);
/// Lab to cube-rooted cone responses (published with the linear sRGB conversion).
pub const LAB_TO_LMS: Mat3 = Mat3([[1.0, 0.3963377774, 0.2158037573], [1.0, -0.1055613458, -0.0638541728], [1.0, -0.0894841775, -1.2914855480]]);

const fn f32s(m: &Mat3) -> [[f32; 3]; 3] {
    let m = &m.0;
    [
        [m[0][0] as f32, m[0][1] as f32, m[0][2] as f32],
        [m[1][0] as f32, m[1][1] as f32, m[1][2] as f32],
        [m[2][0] as f32, m[2][1] as f32, m[2][2] as f32],
    ]
}

const SRGB_LMS: [[f32; 3]; 3] = f32s(&SRGB_TO_LMS);
const LMS_SRGB: [[f32; 3]; 3] = f32s(&LMS_TO_SRGB);
const LAB: [[f32; 3]; 3] = f32s(&LMS_TO_LAB);
const LAB_INV: [[f32; 3]; 3] = f32s(&LAB_TO_LMS);

#[inline]
fn mul(m: &[[f32; 3]; 3], v: [f32; 3]) -> [f32; 3] {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}

/// Linear sRGB (BT.709 primaries, D65) to OkLab `[L, a, b]`.
#[inline]
pub fn from_linear_srgb(rgb: [f32; 3]) -> [f32; 3] {
    mul(&LAB, mul(&SRGB_LMS, rgb).map(f32::cbrt))
}

/// OkLab to linear sRGB.
#[inline]
pub fn to_linear_srgb(lab: [f32; 3]) -> [f32; 3] {
    mul(&LMS_SRGB, mul(&LAB_INV, lab).map(|v| v * v * v))
}

/// XYZ (D65, white Y = 1) to OkLab.
pub fn from_xyz(xyz: [f64; 3]) -> [f64; 3] {
    LMS_TO_LAB.apply(XYZ_TO_LMS.apply(xyz).map(f64::cbrt))
}

/// The `f64` matrices [`OkLab::for_space`] starts from: linear RGB in `space` to cone responses
/// (the white adapted to D65 by Bradford), and back. `None` when the first has no inverse. Code
/// that builds its matrices once per render in `f64` uses these with [`LMS_TO_LAB`] and
/// [`LAB_TO_LMS`].
pub fn lms_matrices(space: &RgbSpace) -> Option<(Mat3, Mat3)> {
    let to_lms = XYZ_TO_LMS.mul(&bradford(space.white, D65)).mul(&space.to_xyz());
    to_lms.inverse().map(|from_lms| (to_lms, from_lms))
}

/// OkLab to OkLCh `[L, C, h]`, with the hue in radians from −π to π.
#[inline]
pub fn lab_to_lch(lab: [f32; 3]) -> [f32; 3] {
    [lab[0], lab[1].hypot(lab[2]), lab[2].atan2(lab[1])]
}

/// OkLCh to OkLab.
#[inline]
pub fn lch_to_lab(lch: [f32; 3]) -> [f32; 3] {
    let (s, c) = lch[2].sin_cos();
    [lch[0], lch[1] * c, lch[1] * s]
}

/// The OkLab conversion for one RGB space, with its matrices precomputed in `f32`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OkLab {
    to_lms: [[f32; 3]; 3],
    from_lms: [[f32; 3]; 3],
}

impl OkLab {
    /// The conversion for linear RGB in `space`.
    pub fn for_space(space: &RgbSpace) -> OkLab {
        let pair = lms_matrices(space).map(|(to_lms, from_lms)| (to_lms.to_f32(), from_lms.to_f32()));
        let finite = |m: &[[f32; 3]; 3]| m.iter().flatten().all(|v| v.is_finite());
        match pair {
            Some((to_lms, from_lms)) if finite(&to_lms) && finite(&from_lms) => OkLab { to_lms, from_lms },
            // A matrix with no inverse or with an entry beyond f32 falls back to XYZ_TO_LMS
            // alone, the matrix a space gets when its primaries and its white are both rejected.
            _ => OkLab { to_lms: f32s(&XYZ_TO_LMS), from_lms: XYZ_TO_LMS.inverse().unwrap_or(Mat3::IDENTITY).to_f32() },
        }
    }

    /// The `f32` matrices, row-major: linear RGB to cone responses, and back. GPU passes that
    /// convert to OkLab use these with [`LMS_TO_LAB`].
    pub fn matrices(&self) -> ([[f32; 3]; 3], [[f32; 3]; 3]) {
        (self.to_lms, self.from_lms)
    }

    /// Linear RGB to OkLab.
    #[inline]
    pub fn from_rgb(&self, rgb: [f32; 3]) -> [f32; 3] {
        mul(&LAB, mul(&self.to_lms, rgb).map(f32::cbrt))
    }

    /// OkLab to linear RGB.
    #[inline]
    pub fn to_rgb(&self, lab: [f32; 3]) -> [f32; 3] {
        mul(&self.from_lms, mul(&LAB_INV, lab).map(|v| v * v * v))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{D50, REC2020, SRGB, Xy};

    /// The table of XYZ values and their OkLab coordinates in Ottosson's definition, rounded
    /// to three decimals.
    #[test]
    fn xyz_matches_the_published_table() {
        let table = [
            ([0.950, 1.000, 1.089], [1.000, 0.000, 0.000]),
            ([1.000, 0.000, 0.000], [0.450, 1.236, -0.019]),
            ([0.000, 1.000, 0.000], [0.922, -0.671, 0.263]),
            ([0.000, 0.000, 1.000], [0.153, -1.415, -0.449]),
        ];
        for (xyz, want) in table {
            let lab = from_xyz(xyz);
            for i in 0..3 {
                assert!((lab[i] - want[i]).abs() < 6e-4, "{xyz:?} -> {lab:?}");
            }
        }
    }

    /// CSS Color Module Level 4 (section 2, "These are exactly equivalent") gives #7654CD as
    /// oklab(54.43237% 0.0681665 -0.1656703).
    #[test]
    fn an_srgb_color_matches_css_color_4() {
        let rgb = [0x76, 0x54, 0xCD].map(|v: u8| craft_color::srgb_to_linear(f32::from(v) / 255.0));
        let lab = from_linear_srgb(rgb);
        let want = [0.5443237, 0.0681665, -0.1656703];
        for i in 0..3 {
            assert!((lab[i] - want[i]).abs() < 2e-4, "{lab:?}");
        }
    }

    #[test]
    fn the_srgb_matrices_and_the_space_built_from_xyz_agree() {
        let srgb = OkLab::for_space(&SRGB);
        for c in [[0.1, 0.5, 0.9], [1.0, 0.0, 0.0], [0.02, 0.03, 0.01], [1.0, 1.0, 1.0]] {
            let a = from_linear_srgb(c);
            let b = srgb.from_rgb(c);
            for i in 0..3 {
                assert!((a[i] - b[i]).abs() < 2e-4, "{c:?} {a:?} {b:?}");
            }
        }
    }

    #[test]
    fn white_is_l1_and_grays_have_no_chroma_in_every_space() {
        let d50 = crate::RgbSpace { white: D50, ..SRGB };
        for space in [SRGB, REC2020, d50] {
            let ok = OkLab::for_space(&space);
            let w = ok.from_rgb([1.0, 1.0, 1.0]);
            assert!((w[0] - 1.0).abs() < 2e-4 && w[1].abs() < 2e-4 && w[2].abs() < 2e-4, "{space:?} {w:?}");
            let g = ok.from_rgb([0.18, 0.18, 0.18]);
            assert!(g[1].abs() < 2e-4 && g[2].abs() < 2e-4, "{space:?} {g:?}");
        }
    }

    #[test]
    fn conversions_round_trip() {
        let ok = OkLab::for_space(&REC2020);
        for c in [[0.1, 0.5, 0.9], [1.0, 0.0, 0.0], [0.02, 0.03, 0.01], [2.0, 1.5, 0.3], [-0.1, 0.4, 0.2]] {
            let a = to_linear_srgb(from_linear_srgb(c));
            let b = ok.to_rgb(ok.from_rgb(c));
            for i in 0..3 {
                assert!((a[i] - c[i]).abs() < 1e-4 * c[i].abs().max(1.0), "{c:?} {a:?}");
                assert!((b[i] - c[i]).abs() < 1e-4 * c[i].abs().max(1.0), "{c:?} {b:?}");
            }
        }
        let lab = lch_to_lab(lab_to_lch([0.5, 0.1, -0.05]));
        assert!((lab[1] - 0.1).abs() < 1e-6 && (lab[2] + 0.05).abs() < 1e-6);
    }

    #[test]
    fn a_space_that_does_not_exist_does_not_produce_nan() {
        let bad = crate::RgbSpace { r: Xy::new(0.3, 0.0), ..SRGB };
        let ok = OkLab::for_space(&bad);
        assert!(ok.from_rgb([0.5, 0.5, 0.5]).iter().all(|v| v.is_finite()));
    }

    #[test]
    fn the_f64_matrices_match_the_published_xyz_conversion() {
        let (to, from) = lms_matrices(&SRGB).unwrap();
        let p = to.mul(&from);
        for i in 0..3 {
            for j in 0..3 {
                assert!((p.0[i][j] - if i == j { 1.0 } else { 0.0 }).abs() < 1e-12, "{p:?}");
            }
        }
        // The published linear sRGB matrix and the one built from XYZ differ by at most 3.1e-4.
        for i in 0..3 {
            for j in 0..3 {
                assert!((to.0[i][j] - SRGB_TO_LMS.0[i][j]).abs() < 5e-4, "{to:?}");
            }
        }
        for c in [[0.1, 0.5, 0.9], [1.0, 1.0, 1.0]] {
            let lab = LMS_TO_LAB.apply(to.apply(c).map(f64::cbrt));
            let f = OkLab::for_space(&SRGB).from_rgb(c.map(|v| v as f32));
            for i in 0..3 {
                assert!((lab[i] - f64::from(f[i])).abs() < 1e-5, "{c:?} {lab:?} {f:?}");
            }
        }
        let bad = crate::RgbSpace { r: Xy::new(0.3, 0.0), ..SRGB };
        assert!(lms_matrices(&bad).is_some_and(|(t, _)| t.0.iter().flatten().all(|v| v.is_finite())));
    }

    /// A white this close to y = 0 gives `f64` matrices with entries beyond `f32`.
    #[test]
    fn a_space_whose_matrices_overflow_f32_does_not_produce_nan() {
        let far = crate::RgbSpace { white: Xy::new(0.5, 1e-40), ..SRGB };
        let ok = OkLab::for_space(&far);
        let (to, from) = ok.matrices();
        assert!(to.iter().chain(from.iter()).flatten().all(|v| v.is_finite()), "{to:?} {from:?}");
        for c in [[0.0; 3], [0.5, 0.5, 0.5], [1.0, 0.0, 0.0]] {
            assert!(ok.from_rgb(c).iter().all(|v| v.is_finite()), "{c:?}");
            assert!(ok.to_rgb(ok.from_rgb(c)).iter().all(|v| v.is_finite()), "{c:?}");
        }
    }
}
