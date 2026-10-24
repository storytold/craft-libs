//! Small colour-science helpers: 3×3 matrices, CIE XYZ ↔ L*a*b*, chromatic adaptation.
//!
//! All formulas are from public standards: CIE 15 (L*a*b*), ICC.1:2010 Annex A (PCS, D50),
//! and the Bradford cone response matrix published by Lam (1985).

/// The ICC profile connection space illuminant (D50), as encoded in s15Fixed16 in every
/// ICC header: X = 0.9642, Y = 1.0, Z = 0.8249.
pub const D50: [f64; 3] = [0.9642, 1.0, 0.8249];

/// CIE D65 white point as XYZ (Y = 1), from the chromaticity (0.3127, 0.3290) used by sRGB,
/// Display P3 and Adobe RGB-compatible spaces.
pub const D65_XY: [f64; 2] = [0.3127, 0.3290];

/// Row-major 3×3 matrix.
pub type Mat3 = [[f64; 3]; 3];

pub const IDENTITY: Mat3 = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

pub fn mul(a: &Mat3, b: &Mat3) -> Mat3 {
    let mut o = [[0.0; 3]; 3];
    for (i, row) in o.iter_mut().enumerate() {
        for (j, v) in row.iter_mut().enumerate() {
            *v = a[i][0] * b[0][j] + a[i][1] * b[1][j] + a[i][2] * b[2][j];
        }
    }
    o
}

pub const fn apply(m: &Mat3, v: [f64; 3]) -> [f64; 3] {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}

pub const fn det(m: &Mat3) -> f64 {
    m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
}

/// Inverse, or `None` when (nearly) singular.
pub const fn invert(m: &Mat3) -> Option<Mat3> {
    let d = det(m);
    if !d.is_finite() || d.abs() < 1e-12 {
        return None;
    }
    let inv = 1.0 / d;
    Some([
        [(m[1][1] * m[2][2] - m[1][2] * m[2][1]) * inv, (m[0][2] * m[2][1] - m[0][1] * m[2][2]) * inv, (m[0][1] * m[1][2] - m[0][2] * m[1][1]) * inv],
        [(m[1][2] * m[2][0] - m[1][0] * m[2][2]) * inv, (m[0][0] * m[2][2] - m[0][2] * m[2][0]) * inv, (m[0][2] * m[1][0] - m[0][0] * m[1][2]) * inv],
        [(m[1][0] * m[2][1] - m[1][1] * m[2][0]) * inv, (m[0][1] * m[2][0] - m[0][0] * m[2][1]) * inv, (m[0][0] * m[1][1] - m[0][1] * m[1][0]) * inv],
    ])
}

/// XYZ (Y = 1) of a chromaticity `(x, y)`.
pub const fn xy_to_xyz(xy: [f64; 2]) -> [f64; 3] {
    let [x, y] = xy;
    [x / y, 1.0, (1.0 - x - y) / y]
}

/// Bradford cone response matrix (Lam 1985).
const BRADFORD: Mat3 = [[0.8951, 0.2664, -0.1614], [-0.7502, 1.7135, 0.0367], [0.0389, -0.0685, 1.0296]];

/// Inverse of [`BRADFORD`] (determinant ~1.7), computed at compile time.
const BRADFORD_INV: Mat3 = match invert(&BRADFORD) {
    Some(m) => m,
    None => panic!("the Bradford matrix is invertible"),
};

/// Bradford chromatic adaptation matrix from white `src` to white `dst` (both XYZ).
pub fn bradford(src: [f64; 3], dst: [f64; 3]) -> Mat3 {
    let inv = BRADFORD_INV;
    let s = apply(&BRADFORD, src);
    let d = apply(&BRADFORD, dst);
    let scale = [[d[0] / s[0], 0.0, 0.0], [0.0, d[1] / s[1], 0.0], [0.0, 0.0, d[2] / s[2]]];
    mul(&inv, &mul(&scale, &BRADFORD))
}

/// RGB → XYZ matrix (relative to `white`, Y = 1) for primaries given as chromaticities, or
/// `None` when the primaries are not independent.
pub const fn rgb_to_xyz_matrix(r: [f64; 2], g: [f64; 2], b: [f64; 2], white: [f64; 3]) -> Option<Mat3> {
    let (xr, xg, xb) = (xy_to_xyz(r), xy_to_xyz(g), xy_to_xyz(b));
    let m = [[xr[0], xg[0], xb[0]], [xr[1], xg[1], xb[1]], [xr[2], xg[2], xb[2]]];
    let Some(inv) = invert(&m) else { return None };
    let s = apply(&inv, white);
    Some([
        [m[0][0] * s[0], m[0][1] * s[1], m[0][2] * s[2]],
        [m[1][0] * s[0], m[1][1] * s[1], m[1][2] * s[2]],
        [m[2][0] * s[0], m[2][1] * s[1], m[2][2] * s[2]],
    ])
}

const EPS: f64 = 216.0 / 24389.0; // (6/29)^3
const KAPPA: f64 = 24389.0 / 27.0; // (29/3)^3

#[inline]
fn lab_f(t: f64) -> f64 {
    if t > EPS { t.cbrt() } else { (KAPPA * t + 16.0) / 116.0 }
}

#[inline]
fn lab_finv(f: f64) -> f64 {
    let f3 = f * f * f;
    if f3 > EPS { f3 } else { (116.0 * f - 16.0) / KAPPA }
}

/// CIE XYZ → L*a*b* relative to `white` (CIE 15).
pub fn xyz_to_lab(xyz: [f64; 3], white: [f64; 3]) -> [f64; 3] {
    let fx = lab_f(xyz[0] / white[0]);
    let fy = lab_f(xyz[1] / white[1]);
    let fz = lab_f(xyz[2] / white[2]);
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}

/// CIE L*a*b* → XYZ relative to `white`.
pub fn lab_to_xyz(lab: [f64; 3], white: [f64; 3]) -> [f64; 3] {
    let fy = (lab[0] + 16.0) / 116.0;
    let fx = fy + lab[1] / 500.0;
    let fz = fy - lab[2] / 200.0;
    [lab_finv(fx) * white[0], lab_finv(fy) * white[1], lab_finv(fz) * white[2]]
}

/// f32 versions for the hot paths (D50 PCS only).
#[inline]
pub fn xyz_to_lab_f32(xyz: [f32; 3]) -> [f32; 3] {
    const W: [f32; 3] = [0.9642, 1.0, 0.8249];
    const E: f32 = 216.0 / 24389.0;
    const K: f32 = 24389.0 / 27.0;
    let f = |t: f32| if t > E { t.cbrt() } else { (K * t + 16.0) / 116.0 };
    let fx = f(xyz[0] / W[0]);
    let fy = f(xyz[1] / W[1]);
    let fz = f(xyz[2] / W[2]);
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}

#[inline]
pub fn lab_to_xyz_f32(lab: [f32; 3]) -> [f32; 3] {
    const W: [f32; 3] = [0.9642, 1.0, 0.8249];
    const E: f32 = 216.0 / 24389.0;
    const K: f32 = 24389.0 / 27.0;
    let fy = (lab[0] + 16.0) / 116.0;
    let fx = fy + lab[1] / 500.0;
    let fz = fy - lab[2] / 200.0;
    let finv = |f: f32| {
        let f3 = f * f * f;
        if f3 > E { f3 } else { (116.0 * f - 16.0) / K }
    };
    [finv(fx) * W[0], finv(fy) * W[1], finv(fz) * W[2]]
}

/// CIE 1976 colour difference.
pub fn delta_e76(a: [f64; 3], b: [f64; 3]) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

/// CIEDE2000 colour difference (CIE 142-2001, kL = kC = kH = 1).
pub fn delta_e2000(a: [f64; 3], b: [f64; 3]) -> f64 {
    use std::f64::consts::PI;
    let (l1, a1, b1) = (a[0], a[1], a[2]);
    let (l2, a2, b2) = (b[0], b[1], b[2]);
    let c_bar = (a1.hypot(b1) + a2.hypot(b2)) / 2.0;
    let c7 = c_bar.powi(7);
    let g = 0.5 * (1.0 - (c7 / (c7 + 25f64.powi(7))).sqrt());
    let (a1p, a2p) = (a1 * (1.0 + g), a2 * (1.0 + g));
    let (c1p, c2p) = (a1p.hypot(b1), a2p.hypot(b2));
    let hue = |b: f64, a: f64| if b == 0.0 && a == 0.0 { 0.0 } else { b.atan2(a).rem_euclid(2.0 * PI) };
    let (h1p, h2p) = (hue(b1, a1p), hue(b2, a2p));
    let dl = l2 - l1;
    let dc = c2p - c1p;
    let dh = if c1p * c2p == 0.0 {
        0.0
    } else {
        let d = h2p - h1p;
        if d > PI {
            d - 2.0 * PI
        } else if d < -PI {
            d + 2.0 * PI
        } else {
            d
        }
    };
    let dhh = 2.0 * (c1p * c2p).sqrt() * (dh / 2.0).sin();
    let l_bar = (l1 + l2) / 2.0;
    let cp_bar = (c1p + c2p) / 2.0;
    let hp_bar = if c1p * c2p == 0.0 {
        h1p + h2p
    } else if (h1p - h2p).abs() <= PI {
        (h1p + h2p) / 2.0
    } else if h1p + h2p < 2.0 * PI {
        (h1p + h2p + 2.0 * PI) / 2.0
    } else {
        (h1p + h2p - 2.0 * PI) / 2.0
    };
    let t = 1.0 - 0.17 * (hp_bar - PI / 6.0).cos() + 0.24 * (2.0 * hp_bar).cos() + 0.32 * (3.0 * hp_bar + PI / 30.0).cos()
        - 0.20 * (4.0 * hp_bar - 63f64.to_radians()).cos();
    let d_theta = 30f64.to_radians() * (-((hp_bar.to_degrees() - 275.0) / 25.0).powi(2)).exp();
    let cp7 = cp_bar.powi(7);
    let rc = 2.0 * (cp7 / (cp7 + 25f64.powi(7))).sqrt();
    let l50 = (l_bar - 50.0).powi(2);
    let sl = 1.0 + 0.015 * l50 / (20.0 + l50).sqrt();
    let sc = 1.0 + 0.045 * cp_bar;
    let sh = 1.0 + 0.015 * cp_bar * t;
    let rt = -(2.0 * d_theta).sin() * rc;
    let (x, y, z) = (dl / sl, dc / sc, dhh / sh);
    (x * x + y * y + z * z + rt * y * z).sqrt()
}

/// Quantizes to ICC s15Fixed16Number and back (what a profile can actually store).
pub fn s15f16_round(v: f64) -> f64 {
    (v * 65536.0).round() / 65536.0
}

/// The D50 illuminant exactly as an ICC header stores it (s15Fixed16).
pub fn d50_quantized() -> [f64; 3] {
    [s15f16_round(D50[0]), s15f16_round(D50[1]), s15f16_round(D50[2])]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lab_roundtrip() {
        for xyz in [[0.2, 0.3, 0.1], [0.9642, 1.0, 0.8249], [0.001, 0.002, 0.0005]] {
            let back = lab_to_xyz(xyz_to_lab(xyz, D50), D50);
            for i in 0..3 {
                assert!((back[i] - xyz[i]).abs() < 1e-12);
            }
        }
        let w = xyz_to_lab(D50, D50);
        assert!((w[0] - 100.0).abs() < 1e-12 && w[1].abs() < 1e-12 && w[2].abs() < 1e-12);
    }

    #[test]
    fn delta_e2000_matches_sharma_reference_pairs() {
        // Sharma, Wu & Dalal (2005), pairs 1, 7, 9–17, 25 and 29, including hue-wrap boundaries.
        // https://hajim.rochester.edu/ece/sites/gsharma/ciede2000/dataNprograms/ciede2000testdata.txt
        let pairs = [
            ([50.0, 2.6772, -79.7751], [50.0, 0.0, -82.7485], 2.0425),
            ([50.0, 0.0, 0.0], [50.0, -1.0, 2.0], 2.3669),
            ([50.0, 2.49, -0.001], [50.0, -2.49, 0.0009], 7.1792),
            ([50.0, 2.49, -0.001], [50.0, -2.49, 0.001], 7.1792),
            ([50.0, 2.49, -0.001], [50.0, -2.49, 0.0011], 7.2195),
            ([50.0, 2.49, -0.001], [50.0, -2.49, 0.0012], 7.2195),
            ([50.0, -0.001, 2.49], [50.0, 0.0009, -2.49], 4.8045),
            ([50.0, -0.001, 2.49], [50.0, 0.001, -2.49], 4.8045),
            ([50.0, -0.001, 2.49], [50.0, 0.0011, -2.49], 4.7461),
            ([50.0, 2.5, 0.0], [50.0, 0.0, -2.5], 4.3065),
            ([50.0, 2.5, 0.0], [73.0, 25.0, -18.0], 27.1492),
            ([60.2574, -34.0099, 36.2677], [60.4626, -34.1751, 39.4387], 1.2644),
            ([22.7233, 20.0904, -46.6940], [23.0331, 14.9730, -42.5619], 2.0373),
        ];
        for (a, b, want) in pairs {
            assert!((delta_e2000(a, b) - want).abs() < 1e-4, "{a:?} {b:?}: {}", delta_e2000(a, b));
            assert!((delta_e2000(b, a) - want).abs() < 1e-4);
        }
    }

    #[test]
    fn delta_e2000_is_finite_symmetric_and_zero_for_identical_lab() {
        let mut colours = Vec::new();
        for l in [0.0, 50.0, 100.0] {
            for a in [-128.0, 0.0, 127.0] {
                for b in [-128.0, 0.0, 127.0] {
                    colours.push([l, a, b]);
                }
            }
        }
        for &a in &colours {
            assert_eq!(delta_e2000(a, a), 0.0);
            for &b in &colours {
                let d = delta_e2000(a, b);
                assert!(d.is_finite() && d >= 0.0, "{a:?} {b:?}: {d}");
                assert!((d - delta_e2000(b, a)).abs() < 1e-12);
            }
        }
    }

    #[test]
    fn bradford_maps_white() {
        let d65 = xy_to_xyz(D65_XY);
        let m = bradford(d65, D50);
        let w = apply(&m, d65);
        for i in 0..3 {
            assert!((w[i] - D50[i]).abs() < 1e-12);
        }
    }

    #[test]
    fn invert_roundtrip() {
        let m = [[0.4, 0.3, 0.2], [0.2, 0.7, 0.1], [0.0, 0.1, 0.8]];
        let p = mul(&m, &invert(&m).unwrap());
        for (i, row) in p.iter().enumerate() {
            for (j, v) in row.iter().enumerate() {
                assert!((v - if i == j { 1.0 } else { 0.0 }).abs() < 1e-12);
            }
        }
    }
}
