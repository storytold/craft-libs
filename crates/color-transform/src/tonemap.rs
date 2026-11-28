//! PQ tone mapping with the reference EETF of Report ITU-R BT.2408-8, Annex 5 ("Displaying
//! PQ: calculating the EETF"), which Report ITU-R BT.2390 described before it.
//!
//! The EETF works on PQ (SMPTE ST 2084) values: it keeps the range below a knee 1:1, rolls off
//! the range above it with a Hermite spline so the source peak lands on the target peak, and
//! lifts the target black with a tapered offset. [`ToneMap`] applies it to RGB by max(R, G, B),
//! the "maxRGB" option of the Annex, as a ratio, so hue and saturation are kept.

use craft_color::{REFERENCE_WHITE_NITS, pq_eotf, pq_inverse_eotf};

/// PQ code value (0..1) of `nits`.
fn pq(nits: f64) -> f64 {
    f64::from(pq_inverse_eotf((nits / 10_000.0).clamp(0.0, 1.0) as f32))
}

/// Luminance in nits of the PQ value `e`.
fn nits(e: f64) -> f64 {
    f64::from(pq_eotf(e.clamp(0.0, 1.0) as f32)) * 10_000.0
}

/// A luminance in nits: NaN, negative values and infinity read as 0, 0 and 10 000.
fn sane(n: f64) -> f64 {
    if n.is_nan() { 0.0 } else { n.clamp(0.0, 10_000.0) }
}

/// The reference EETF from a mastering display (black `src_black`, peak `src_peak`) to a
/// target display (black `dst_black`, peak `dst_peak`), all in nits.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Eetf {
    pb: f64,
    span: f64,
    min_lum: f64,
    max_lum: f64,
    ks: f64,
    /// The spline's slope at the knee: 1 within the Annex's range.
    slope: f64,
    /// The black lift's exponent: 4 within the Annex's range.
    lift: f64,
    /// The E2 at which the black lift reaches 0: 1 within the Annex's range.
    reach: f64,
}

impl Eetf {
    /// Luminances are clamped to 0..10 000 nits. A target black below the source black adds no
    /// lift. When the source range is empty the EETF passes values through.
    ///
    /// When the target peak sits below a third of the source range in PQ, the Annex's knee
    /// (KS = 1.5 maxLum − 0.5) is negative. The knee is then 0 and the spline starts with a
    /// slope of 3 maxLum, the steepest that keeps it rising.
    ///
    /// The Annex's black lift, minLum × (1 − E2)⁴, makes the output fall near black when minLum
    /// is above 0.25, and lifts the peak well above the target peak. The lift here is
    /// minLum × (1 − E2 / r)ᵏ with r = max(T, min(1, 2 − 4 minLum)) and k = min(4, r / minLum),
    /// where T, the top of the output, is the smaller of maxLum and 1. Up to minLum 0.25 this is
    /// the Annex's lift. Above it, k is the largest exponent that keeps the output rising, r moves
    /// continuously from 1 down to T, and from minLum 0.5 the lift reaches 0 at the top of the
    /// output. A target black above the source peak reads as the source peak.
    ///
    /// A target peak at or below the source black would put every output at the source black
    /// or above it; the source black then reads as 0. A target black above the target peak
    /// reads as the target peak. A target peak that is NaN or not above 0 names no display, and
    /// the EETF then passes values through.
    pub fn new(src_black: f64, src_peak: f64, dst_black: f64, dst_peak: f64) -> Eetf {
        let pb = pq(sane(src_black));
        let pass = Eetf { pb, span: 0.0, min_lum: 0.0, max_lum: 1.0, ks: 1.0, slope: 1.0, lift: 4.0, reach: 1.0 };
        if dst_peak.is_nan() || dst_peak <= 0.0 {
            return pass;
        }
        let (dst_peak, pw) = (sane(dst_peak), pq(sane(src_peak)));
        let dst_black = sane(dst_black).min(dst_peak);
        if pw <= pb {
            return pass;
        }
        let pb = if pq(dst_peak) <= pb { 0.0 } else { pb };
        let span = pw - pb;
        // Step 2.
        let min_lum = ((pq(dst_black) - pb) / span).max(0.0);
        let max_lum = ((pq(dst_peak) - pb) / span).max(0.0);
        let ks = 1.5 * max_lum - 0.5;
        let (ks, slope) = if ks >= 0.0 { (ks, 1.0) } else { (0.0, 3.0 * max_lum) };
        // E2 ends at `top`. The output's slope at E2 = 0 is 1 − lift × minLum / reach, so the
        // exponent is at most reach / minLum. Up to minLum 0.25 this gives the Annex's (1, 4); both
        // change continuously above it, and reach is `top` from minLum 0.5. A target black above
        // the source peak (minLum above `top`) reads as the source peak.
        let top = max_lum.min(1.0);
        let min_lum = min_lum.min(top);
        let reach = top.max((2.0 - 4.0 * min_lum).min(1.0));
        let lift = if min_lum > 0.0 { (reach / min_lum).min(4.0) } else { 4.0 };
        Eetf { pb, span, min_lum, max_lum, ks, slope, lift, reach }
    }

    /// Whether the EETF passes every value through unchanged.
    pub fn is_identity(&self) -> bool {
        self.span <= 0.0 || (self.max_lum >= 1.0 && self.min_lum <= 0.0)
    }

    /// The EETF on a PQ value (0..1): steps 1 to 5 of the Annex. Unless the EETF is the
    /// identity, values above the source peak map as the peak does.
    pub fn apply_pq(&self, e: f64) -> f64 {
        if self.is_identity() || e.is_nan() {
            return e;
        }
        // Step 1.
        let e1 = ((e - self.pb) / self.span).clamp(0.0, 1.0);
        // Steps 3 and 4: 1:1 below the knee, the Hermite spline above it.
        let e2 = if e1 < self.ks || self.max_lum >= 1.0 {
            e1
        } else {
            let t = (e1 - self.ks) / (1.0 - self.ks);
            let (t2, t3) = (t * t, t * t * t);
            (2.0 * t3 - 3.0 * t2 + 1.0) * self.ks + (t3 - 2.0 * t2 + t) * (1.0 - self.ks) * self.slope + (-2.0 * t3 + 3.0 * t2) * self.max_lum
        };
        let e3 = if self.reach == 1.0 && self.lift == 4.0 {
            e2 + self.min_lum * (1.0 - e2).powi(4)
        } else {
            e2 + self.min_lum * (1.0 - e2 / self.reach).max(0.0).powf(self.lift)
        };
        // Step 5.
        e3 * self.span + self.pb
    }

    /// The EETF on a luminance in nits.
    pub fn apply_nits(&self, n: f64) -> f64 {
        nits(self.apply_pq(pq(sane(n))))
    }
}

/// The BT.2408 EETF from a display of peak `src_peak` to one of peak `dst_peak` (nits), with
/// black at 0.
pub fn eetf(n: f64, src_peak: f64, dst_peak: f64) -> f64 {
    Eetf::new(0.0, src_peak, 0.0, dst_peak).apply_nits(n)
}

/// Entries in [`ToneMap`]'s table.
const TABLE: usize = 4096;

/// Tone mapping of linear RGB, where 1.0 is reference white, from a source peak to a target
/// peak. It applies the EETF (black at 0) to max(R, G, B) and scales the three channels by the
/// same ratio. The ratios are tabulated on a PQ axis once, so [`apply`](Self::apply) does not
/// allocate.
#[derive(Clone, Debug, PartialEq)]
pub struct ToneMap {
    src_peak: f32,
    dst_peak: f32,
    white: f32,
    identity: bool,
    /// Output nits over input nits, at PQ values `i / (TABLE − 1)`.
    ratio: Vec<f32>,
}

impl ToneMap {
    /// Tone mapping with reference white at `craft_color::REFERENCE_WHITE_NITS` (203 nits,
    /// Report ITU-R BT.2408).
    pub fn new(src_peak_nits: f64, dst_peak_nits: f64) -> ToneMap {
        ToneMap::with_reference_white(src_peak_nits, dst_peak_nits, REFERENCE_WHITE_NITS)
    }

    /// Tone mapping where 1.0 is `white_nits`. A white that is not above 0 or not finite reads
    /// as 203 nits, and one above 10 000 nits as 10 000.
    pub fn with_reference_white(src_peak_nits: f64, dst_peak_nits: f64, white_nits: f64) -> ToneMap {
        let (s, d) = (sane(src_peak_nits), sane(dst_peak_nits));
        let white = if white_nits.is_finite() && white_nits > 0.0 { white_nits.min(10_000.0) } else { REFERENCE_WHITE_NITS };
        let curve = Eetf::new(0.0, s, 0.0, d);
        let identity = curve.is_identity();
        let ratio = if identity {
            Vec::new()
        } else {
            (0..TABLE)
                .map(|i| {
                    let e = i as f64 / (TABLE - 1) as f64;
                    let n = nits(e);
                    if n <= 1e-6 { 1.0 } else { (curve.apply_nits(n) / n) as f32 }
                })
                .collect()
        };
        ToneMap { src_peak: s as f32, dst_peak: d as f32, white: white as f32, identity, ratio }
    }

    /// Whether [`apply`](Self::apply) returns its input (the target peak is at or above the
    /// source peak).
    pub fn is_identity(&self) -> bool {
        self.identity
    }

    /// One pixel of linear RGB, 1.0 = reference white.
    #[inline]
    pub fn apply(&self, c: [f32; 3]) -> [f32; 3] {
        if self.identity {
            return c;
        }
        let m = c[0].max(c[1]).max(c[2]);
        // NaN (all three channels NaN) and values at or below 0 pass through.
        if m.is_nan() || m <= 0.0 {
            return c;
        }
        let n = m * self.white;
        if n >= self.src_peak {
            // The brightest channel lands on the target peak. Dividing by m first keeps an
            // infinite channel (or an n that overflows) from giving infinity times 0.
            let peak = self.dst_peak / self.white;
            return c.map(|v| if v >= m { peak } else { v / m * peak });
        }
        let k = self.lookup(pq_inverse_eotf(n / 10_000.0));
        c.map(|v| v * k)
    }

    #[inline]
    fn lookup(&self, e: f32) -> f32 {
        let last = self.ratio.len().saturating_sub(1);
        let p = e.clamp(0.0, 1.0) * last as f32;
        let i = (p as usize).min(last.saturating_sub(1));
        let (Some(a), Some(b)) = (self.ratio.get(i), self.ratio.get(i + 1)) else { return 1.0 };
        a + (b - a) * (p - i as f32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Report ITU-R BT.2408 lists 203 nits (HDR reference white) at 58 % PQ and 1000 nits at
    /// 75 %; these check the PQ functions the EETF stands on.
    #[test]
    fn pq_reference_levels() {
        assert!((pq(203.0) - 0.58).abs() < 0.005, "{}", pq(203.0));
        assert!((pq(1000.0) - 0.75).abs() < 0.005, "{}", pq(1000.0));
        assert!((nits(pq(100.0)) - 100.0).abs() < 0.05);
    }

    /// Values computed from the Annex 5 equations and the ST 2084 constants by a separate
    /// implementation, 1000 nits to 100 nits.
    #[test]
    fn matches_an_independent_evaluation_of_the_annex() {
        let curve = Eetf::new(0.0, 1000.0, 0.0, 100.0);
        for (input, want) in ORACLE_1000_TO_100 {
            let got = curve.apply_nits(input);
            assert!((got - want).abs() <= want * 2e-4 + 1e-4, "{input} nits: {got} vs {want}");
        }
        let lifted = Eetf::new(0.0, 1000.0, 0.1, 100.0);
        for (input, want) in ORACLE_1000_TO_100_BLACK_01 {
            let got = lifted.apply_nits(input);
            assert!((got - want).abs() <= want * 2e-4 + 1e-4, "{input} nits: {got} vs {want}");
        }
    }

    const ORACLE_1000_TO_100: [(f64, f64); 9] = [
        (0.0, 0.000000),
        (1.0, 1.000000),
        (10.0, 10.000000),
        (50.0, 46.142955),
        (100.0, 69.454403),
        (203.0, 88.243641),
        (400.0, 97.598409),
        (700.0, 99.854805),
        (1000.0, 100.000000),
    ];
    const ORACLE_1000_TO_100_BLACK_01: [(f64, f64); 9] = [
        (0.0, 0.100000),
        (1.0, 1.608558),
        (10.0, 11.080978),
        (50.0, 47.142334),
        (100.0, 70.309850),
        (203.0, 88.992667),
        (400.0, 98.298831),
        (700.0, 100.543945),
        (1000.0, 100.688420),
    ];

    #[test]
    fn the_knee_keeps_low_values_and_lands_the_peak_on_the_target() {
        let curve = Eetf::new(0.0, 1000.0, 0.0, 100.0);
        assert!((curve.apply_nits(1000.0) - 100.0).abs() < 0.05);
        assert!((curve.apply_nits(5000.0) - 100.0).abs() < 0.05, "above the source peak");
        // Below the knee (KS = 1.5 maxLum − 0.5) the EETF is 1:1.
        let below = nits(curve.ks * pq(1000.0) * 0.99);
        assert!((curve.apply_nits(below) - below).abs() < below * 1e-4, "{below}");
        let mut prev = 0.0;
        for i in 0..=1000 {
            let out = curve.apply_pq(f64::from(i) / 1000.0 * pq(1000.0));
            assert!(out >= prev - 1e-12, "monotone");
            prev = out;
        }
        assert!(Eetf::new(0.0, 100.0, 0.0, 1000.0).is_identity());
        assert_eq!(eetf(400.0, 100.0, 1000.0) as f32, nits(pq(400.0)) as f32);
    }

    /// 10 000 nits to 1 nit puts the target peak below a third of the source range: the curve
    /// still rises from 0 to the target peak and keeps shadows apart.
    #[test]
    fn a_deep_reduction_keeps_rising_without_crushing_shadows() {
        let curve = Eetf::new(0.0, 10_000.0, 0.0, 1.0);
        let mut prev = -1.0;
        for i in 0..=2000 {
            let out = curve.apply_pq(f64::from(i) / 2000.0);
            assert!(out > prev || i == 0, "{i}: {out} after {prev}");
            assert!(out >= 0.0);
            prev = out;
        }
        assert!((curve.apply_nits(10_000.0) - 1.0).abs() < 1e-3);
        assert!(curve.apply_nits(0.01) > 0.0 && curve.apply_nits(0.01) < curve.apply_nits(0.1));
    }

    #[test]
    fn the_black_lift_raises_black_to_the_target_black() {
        let curve = Eetf::new(0.0, 1000.0, 0.1, 100.0);
        assert!((curve.apply_nits(0.0) - 0.1).abs() < 1e-3, "{}", curve.apply_nits(0.0));
        // The taper (1 − E2)⁴ is small but not 0 at the target peak, so the peak rises a little.
        let peak = curve.apply_nits(1000.0);
        assert!(peak > 100.0 && peak < 101.0, "{peak}");
    }

    #[test]
    fn a_high_target_black_keeps_the_output_rising_and_lands_the_peak() {
        let cases = [(100.0, 100.0), (50.0, 100.0), (10.0, 100.0), (1.0, 100.0), (0.1, 1000.0), (40.0, 4000.0), (2000.0, 4000.0)];
        for (black, peak) in cases {
            let curve = Eetf::new(0.0, 1000.0, black, peak);
            // A target black above the 1000-nit source peak reads as the source peak.
            let want = f64::min(black, 1000.0);
            assert!((curve.apply_nits(0.0) - want).abs() < 1e-2 * want.max(1.0), "{black} {peak}: {}", curve.apply_nits(0.0));
            // From minLum 0.5 the lift reaches 0 at the top of the output.
            let top = curve.apply_nits(1000.0);
            if curve.min_lum >= 0.5 {
                assert!((top - f64::min(peak, 1000.0)).abs() < 1e-2 * peak, "{black} {peak}: {top}");
            }
            let mut last = curve.apply_pq(0.0);
            for i in 1..=1000 {
                let out = curve.apply_pq(f64::from(i) / 1000.0);
                assert!(out >= last - 1e-12, "{black} {peak} at {i}: {out} < {last}");
                last = out;
            }
        }
    }

    #[test]
    fn the_output_changes_continuously_with_the_target_black() {
        for peak in [100.0, 300.0, 4000.0] {
            let at = |black: f64, n: f64| Eetf::new(0.0, 1000.0, black, peak).apply_pq(pq(n));
            // Target blacks 1/20 000 of the target peak's PQ value apart.
            let black = |i: u32| nits(f64::from(i) / 20_000.0 * pq(peak));
            for n in [0.0, 1.0, 100.0, 1000.0] {
                let mut last = at(0.0, n);
                for i in 1..=20_000 {
                    let out = at(black(i), n);
                    assert!((out - last).abs() < 1e-3, "peak {peak}, input {n}, black {}: {last} to {out}", black(i));
                    last = out;
                }
            }
        }
    }

    #[test]
    fn tone_map_keeps_hue_and_maps_the_peak() {
        let tm = ToneMap::new(1000.0, 203.0);
        let peak = 1000.0 / 203.0;
        let out = tm.apply([peak, peak * 0.5, peak * 0.25]);
        assert!((out[0] - 1.0).abs() < 1e-3, "{out:?}");
        assert!((out[1] / out[0] - 0.5).abs() < 1e-6 && (out[2] / out[0] - 0.25).abs() < 1e-6);
        let low = tm.apply([0.02, 0.01, 0.0]);
        assert!((low[0] - 0.02).abs() < 1e-4, "{low:?}");
        assert_eq!(tm.apply([0.0, 0.0, 0.0]), [0.0, 0.0, 0.0]);
        assert_eq!(tm.apply([-1.0, -2.0, -0.5]), [-1.0, -2.0, -0.5]);
    }

    /// With a target peak above the source peak nothing changes, highlights above the source
    /// peak included (they are not raised to the target peak).
    #[test]
    fn a_brighter_target_leaves_colors_alone() {
        let tm = ToneMap::new(1000.0, 4000.0);
        assert!(tm.is_identity());
        let hot = [1200.0 / 203.0, 0.5, 0.1];
        assert_eq!(tm.apply(hot), hot);
    }

    #[test]
    fn hostile_values_do_not_panic_or_spread() {
        let tm = ToneMap::new(f64::NAN, f64::INFINITY);
        assert!(tm.is_identity());
        let huge = ToneMap::with_reference_white(1000.0, 100.0, 1e100);
        assert!((huge.apply([1.0, 1.0, 1.0])[0] - 100.0 / 10_000.0).abs() < 1e-4, "{:?}", huge.apply([1.0, 1.0, 1.0]));
        let tm = ToneMap::with_reference_white(1000.0, 100.0, -3.0);
        // max() skips NaN, so the other channels are mapped by their own maximum.
        assert!(tm.apply([f32::NAN, 0.5, 0.5]).iter().skip(1).all(|v| v.is_finite()));
        assert_eq!(tm.apply([f32::INFINITY, 0.5, 0.0]), [100.0 / 203.0, 0.0, 0.0]);
        assert_eq!(tm.apply([f32::MAX, f32::MAX, 0.0]), [100.0 / 203.0, 100.0 / 203.0, 0.0]);
        // A target peak below the source black: the peak still lands on the target peak.
        let dim = Eetf::new(100.0, 1000.0, 0.0, 50.0);
        assert!((dim.apply_nits(1000.0) - 50.0).abs() < 0.05, "{}", dim.apply_nits(1000.0));
        assert!(Eetf::new(0.0, 1000.0, 500.0, 100.0).apply_nits(0.0) <= 100.0 + 0.05);
        let e = Eetf::new(500.0, 100.0, 0.0, 50.0);
        assert!(e.is_identity());
        assert_eq!(e.apply_pq(0.3), 0.3);
        assert!(Eetf::new(0.0, 1000.0, 0.0, 100.0).apply_pq(f64::NAN).is_nan());
    }

    /// A target peak that is NaN, 0 or negative passes the picture through instead of turning it
    /// black.
    #[test]
    fn a_missing_target_peak_passes_values_through() {
        for peak in [f64::NAN, 0.0, -100.0, f64::NEG_INFINITY] {
            let tm = ToneMap::new(1000.0, peak);
            assert!(tm.is_identity(), "{peak}");
            assert_eq!(tm.apply([0.8, 0.5, 0.1]), [0.8, 0.5, 0.1], "{peak}");
            assert!(Eetf::new(0.0, 1000.0, 0.0, peak).is_identity(), "{peak}");
        }
    }
}
