//! Correlated color temperature, tint and white balance.
//!
//! The Planckian locus is the cubic approximation of Kim et al., "Design of Advanced Color
//! Temperature Control System for HDTV Applications", Journal of the Korean Physical Society 41
//! (2002), over 1667 K to 25000 K. Outside that range the locus continues as a straight line in
//! mired (10⁶ / K) from the end of the approximation. Temperatures are clamped to
//! [`MIN_TEMPERATURE`] to [`MAX_TEMPERATURE`].
//!
//! Tint is a signed distance from the locus in CIE 1960 (u, v), along the locus normal, in units
//! of 1/[`TINT_SCALE`] of Duv. A positive tint is a white above the locus (towards green), so the
//! correction that removes it adds magenta; a negative tint is a white below the locus.

use crate::{Mat3, RgbSpace, Xy, bradford};

/// Tint units per unit of Duv.
pub const TINT_SCALE: f64 = 3000.0;
/// The lowest temperature the approximation covers, in kelvin.
pub const KIM_MIN: f64 = 1667.0;
/// The highest temperature the approximation covers, in kelvin.
pub const KIM_MAX: f64 = 25000.0;
/// The lowest temperature the functions here take.
pub const MIN_TEMPERATURE: f64 = 1000.0;
/// The highest temperature the functions here take.
pub const MAX_TEMPERATURE: f64 = 100_000.0;

fn kim_xy(t: f64) -> Xy {
    let t = t.clamp(KIM_MIN, KIM_MAX);
    let (t2, t3) = (t * t, t * t * t);
    let x = if t <= 4000.0 {
        -0.2661239e9 / t3 - 0.2343589e6 / t2 + 0.8776956e3 / t + 0.179910
    } else {
        -3.0258469e9 / t3 + 2.1070379e6 / t2 + 0.2226347e3 / t + 0.240390
    };
    let (x2, x3) = (x * x, x * x * x);
    let y = if t <= 2222.0 {
        -1.1063814 * x3 - 1.34811020 * x2 + 2.18555832 * x - 0.20219683
    } else if t <= 4000.0 {
        -0.9549476 * x3 - 1.37418593 * x2 + 2.09137015 * x - 0.16748867
    } else {
        3.0817580 * x3 - 5.87338670 * x2 + 3.75112997 * x - 0.37001483
    };
    Xy::new(x, y)
}

/// CIE 1931 xy to CIE 1960 (u, v).
pub fn xy_to_uv(p: Xy) -> (f64, f64) {
    let d = -2.0 * p.x + 12.0 * p.y + 3.0;
    (4.0 * p.x / d, 6.0 * p.y / d)
}

/// CIE 1960 (u, v) to CIE 1931 xy.
pub fn uv_to_xy(u: f64, v: f64) -> Xy {
    let d = 2.0 * u - 8.0 * v + 4.0;
    Xy::new(3.0 * u / d, 2.0 * v / d)
}

/// A temperature in range: NaN reads as 6500 K.
fn sane_temperature(t: f64) -> f64 {
    if t.is_nan() { 6500.0 } else { t.clamp(MIN_TEMPERATURE, MAX_TEMPERATURE) }
}

/// The locus in (u, v) at `t` kelvin.
fn locus_uv(t: f64) -> (f64, f64) {
    if (KIM_MIN..=KIM_MAX).contains(&t) {
        return xy_to_uv(kim_xy(t));
    }
    let (a, b) = if t < KIM_MIN { (KIM_MIN, KIM_MIN + 50.0) } else { (KIM_MAX - 500.0, KIM_MAX) };
    let (ua, va) = xy_to_uv(kim_xy(a));
    let (ub, vb) = xy_to_uv(kim_xy(b));
    let (ma, mb, m) = (1e6 / a, 1e6 / b, 1e6 / t);
    let s = (m - ma) / (mb - ma);
    (ua + (ub - ua) * s, va + (vb - va) * s)
}

/// The unit normal of the locus at `t`, pointing towards green (increasing v).
fn locus_normal(t: f64) -> (f64, f64) {
    let m = 1e6 / t;
    let (u0, v0) = locus_uv(1e6 / (m + 1.0));
    let (u1, v1) = locus_uv(1e6 / (m - 1.0).max(1.0));
    let (du, dv) = (u1 - u0, v1 - v0);
    let l = du.hypot(dv).max(1e-12);
    let (nx, ny) = (-dv / l, du / l);
    if ny < 0.0 { (-nx, -ny) } else { (nx, ny) }
}

/// The chromaticity of a white at temperature `t` (kelvin) and `tint`. A NaN tint reads as 0.
pub fn temp_tint_to_xy(t: f64, tint: f64) -> Xy {
    let t = sane_temperature(t);
    let tint = if tint.is_finite() { tint } else { 0.0 };
    let (u, v) = locus_uv(t);
    let (nu, nv) = locus_normal(t);
    let duv = tint / TINT_SCALE;
    uv_to_xy(u + nu * duv, v + nv * duv)
}

/// The temperature and tint of a white: the nearest point on the locus, searched in mired, and
/// the signed distance from it. The inverse of [`temp_tint_to_xy`] over its range. A
/// chromaticity with no (u, v) coordinates (NaN, or on the line where their denominator is 0)
/// reads as 6500 K with no tint.
pub fn xy_to_temp_tint(p: Xy) -> (f64, f64) {
    let (u, v) = xy_to_uv(p);
    if !(u.is_finite() && v.is_finite()) {
        return (6500.0, 0.0);
    }
    let dist = |m: f64| {
        let (lu, lv) = locus_uv(1e6 / m);
        (u - lu).powi(2) + (v - lv).powi(2)
    };
    let (lo, hi) = (1e6 / MAX_TEMPERATURE, 1e6 / MIN_TEMPERATURE);
    // A scan every 5 mired, then a golden-section search around the best step.
    let mut best = lo;
    let mut best_d = f64::MAX;
    let mut m = lo;
    while m <= hi {
        let d = dist(m);
        if d < best_d {
            best_d = d;
            best = m;
        }
        m += 5.0;
    }
    let (mut a, mut b) = ((best - 5.0).max(lo), (best + 5.0).min(hi));
    let g = 0.618_033_988_75;
    for _ in 0..60 {
        let c = b - g * (b - a);
        let d = a + g * (b - a);
        if dist(c) < dist(d) { b = d } else { a = c }
    }
    let t = 1e6 / ((a + b) / 2.0);
    let (lu, lv) = locus_uv(t);
    let (nu, nv) = locus_normal(t);
    (t, ((u - lu) * nu + (v - lv) * nv) * TINT_SCALE)
}

/// The matrix (linear RGB in `space` to the same space) that makes a scene lit by a white at
/// `src` look as if it were lit by the space's own white (Bradford adaptation).
pub fn wb_matrix(space: &RgbSpace, src: Xy) -> Mat3 {
    space.from_xyz().mul(&bradford(src, space.white)).mul(&space.to_xyz())
}

/// The matrix (linear RGB in `space` to the same space) that makes a scene lit by a white at
/// temperature and tint `from` look as if it were lit by the white at `to`. Equal settings give
/// the identity. A lower `from` temperature than `to` cools the picture.
pub fn white_balance(space: &RgbSpace, from: (f64, f64), to: (f64, f64)) -> Mat3 {
    let (src, dst) = (temp_tint_to_xy(from.0, from.1), temp_tint_to_xy(to.0, to.1));
    if src == dst {
        return Mat3::IDENTITY;
    }
    space.from_xyz().mul(&bradford(src, dst)).mul(&space.to_xyz())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{D65, REC2020, SRGB};

    /// CIE 15:2004 gives illuminant A (a Planckian radiator at 2856 K) as x = 0.44757,
    /// y = 0.40745. The cubic approximation lands within 0.001 Duv of it.
    #[test]
    fn illuminant_a_lies_on_the_locus() {
        let (u, v) = xy_to_uv(temp_tint_to_xy(2856.0, 0.0));
        let (ua, va) = xy_to_uv(Xy::new(0.44757, 0.40745));
        assert!((u - ua).hypot(v - va) < 1e-3, "{u} {v}");
        let (t, _) = xy_to_temp_tint(Xy::new(0.44757, 0.40745));
        assert!((t - 2856.0).abs() < 15.0, "{t}");
    }

    /// CIE 15:2004 gives D65 a correlated color temperature of 6504 K; it lies above the locus
    /// by a Duv of about 0.0032.
    #[test]
    fn d65_reads_as_6504_k_above_the_locus() {
        let (t, tint) = xy_to_temp_tint(D65);
        assert!((t - 6504.0).abs() < 15.0, "{t}");
        assert!((tint / TINT_SCALE - 0.0032).abs() < 3e-4, "{tint}");
    }

    /// D65 in CIE 1960 UCS is (0.1978, 0.3122).
    #[test]
    fn uv_matches_cie_1960_for_d65() {
        let (u, v) = xy_to_uv(D65);
        assert!((u - 0.1978).abs() < 1e-4 && (v - 0.3122).abs() < 1e-4, "{u} {v}");
        let back = uv_to_xy(u, v);
        assert!((back.x - D65.x).abs() < 1e-12 && (back.y - D65.y).abs() < 1e-12);
    }

    #[test]
    fn temperature_and_tint_round_trip() {
        for t in [1200.0, 2000.0, 2856.0, 4000.0, 5003.0, 6500.0, 9000.0, 15000.0, 30000.0, 50000.0] {
            for tint in [-100.0, -20.0, 0.0, 35.0, 150.0] {
                let (t2, tint2) = xy_to_temp_tint(temp_tint_to_xy(t, tint));
                assert!((t2 - t).abs() / t < 0.01, "{t} {tint} -> {t2}");
                assert!((tint2 - tint).abs() < 1.0, "{t} {tint} -> {tint2}");
            }
        }
    }

    #[test]
    fn warmer_whites_are_redder_and_positive_tint_is_greener() {
        assert!(temp_tint_to_xy(3000.0, 0.0).x > temp_tint_to_xy(9000.0, 0.0).x);
        let (_, v0) = xy_to_uv(temp_tint_to_xy(5000.0, 0.0));
        let (_, v1) = xy_to_uv(temp_tint_to_xy(5000.0, 50.0));
        assert!(v1 > v0);
    }

    #[test]
    fn wb_matrix_maps_the_source_white_to_neutral() {
        let src = temp_tint_to_xy(3200.0, 10.0);
        let m = wb_matrix(&REC2020, src);
        let out = m.apply(REC2020.from_xyz().apply(src.to_xyz().unwrap()));
        assert!((out[0] - out[1]).abs() < 1e-9 && (out[1] - out[2]).abs() < 1e-9, "{out:?}");
    }

    #[test]
    fn white_balance_is_the_identity_for_equal_settings_and_cools_a_warm_light() {
        assert_eq!(white_balance(&SRGB, (6500.0, 0.0), (6500.0, 0.0)), Mat3::IDENTITY);
        // Light at 3200 K made to look like 6500 K: a white surface lit by it turns neutral.
        let m = white_balance(&SRGB, (3200.0, 0.0), (6500.0, 0.0));
        let gray = m.apply([1.0, 1.0, 1.0]);
        assert!(gray[2] > gray[0], "{gray:?}");
        let lit = SRGB.from_xyz().apply(temp_tint_to_xy(3200.0, 0.0).to_xyz().unwrap());
        let lit6500 = SRGB.from_xyz().apply(temp_tint_to_xy(6500.0, 0.0).to_xyz().unwrap());
        let out = m.apply(lit);
        for i in 0..3 {
            assert!((out[i] - lit6500[i]).abs() < 1e-9, "{out:?} {lit6500:?}");
        }
    }

    #[test]
    fn hostile_temperatures_stay_finite() {
        for t in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 0.0, -5.0, 1e300] {
            for tint in [f64::NAN, f64::INFINITY, 0.0] {
                let p = temp_tint_to_xy(t, tint);
                assert!(p.x.is_finite() && p.y.is_finite(), "{t} {tint} {p:?}");
                let m = white_balance(&SRGB, (t, tint), (6500.0, 0.0));
                assert!(m.0.iter().flatten().all(|v| v.is_finite()), "{t} {tint}");
            }
        }
        // -2x + 12y + 3 = 0 at (1.5, 0): the (u, v) denominator.
        for p in [Xy::new(f64::NAN, f64::NAN), Xy::new(1.5, 0.0), Xy::new(f64::INFINITY, 0.3)] {
            assert_eq!(xy_to_temp_tint(p), (6500.0, 0.0), "{p:?}");
        }
    }

    /// Pins the current behavior for warm, strongly green illuminants (e.g. 2000–3500 K with high
    /// positive tint) where the source white's Bradford blue cone response is `<= 0.0`. The
    /// checked Bradford path currently returns `Mat3::IDENTITY` there (so `wb_matrix` is
    /// `REC2020.from_xyz() * REC2020.to_xyz()`, i.e. identity within `f64` rounding) rather than
    /// inverting blue; final `#587` process-version policy for this region remains open.
    #[test]
    fn warm_green_wb_with_non_positive_cone_response_returns_identity() {
        for (t, tint) in [(2000.0, 30.0), (2500.0, 60.0), (3000.0, 100.0), (3500.0, 125.0)] {
            let src = temp_tint_to_xy(t, tint);
            assert_eq!(crate::bradford(src, REC2020.white), Mat3::IDENTITY, "{t} K tint {tint}");
            let m = wb_matrix(&REC2020, src);
            for (i, row) in m.0.iter().enumerate() {
                for (j, &val) in row.iter().enumerate() {
                    let want = if i == j { 1.0 } else { 0.0 };
                    assert!((val - want).abs() < 1e-12, "{t} K tint {tint} [{i}][{j}] = {val}");
                }
            }
        }
    }
}
