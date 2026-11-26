//! The global tone map: scene luminance → display-linear luminance.
//!
//! Built in the log domain around middle grey (0.18): contrast scales log-exposure about grey,
//! whites move the shoulder (white point), blacks move the toe. The shoulder is an extended
//! Reinhard curve so highlights roll off smoothly instead of clipping.
//!
//! Rendered (display-referred) sources such as JPEGs use [`ToneMap::display`] instead: identity at
//! neutral settings (an unedited JPEG renders exactly as the file), with contrast/whites/blacks as
//! S-curve adjustments in a gamma-2.2 perceptual domain and a short shoulder above 0.95.

pub const GREY: f32 = 0.18;
/// The tone LUT spans `LUT_MIN_EV..LUT_MAX_EV` around grey in `LUT_N` steps.
pub const LUT_MIN_EV: f32 = -14.0;
pub const LUT_MAX_EV: f32 = 10.0;
pub const LUT_N: usize = 4096;
/// Nodes of a camera chroma curve, evenly spaced over display luminance 0..=1.
pub const CHROMA_N: usize = 8;
const NO_CHROMA: [f32; CHROMA_N] = [1.0; CHROMA_N];

/// A file-local camera look, fitted independently of the scene-linear colour transform.
/// Knots are scene/display-linear luminance pairs. Keeping this in the finish stage preserves
/// RAW exposure and highlight headroom; it is never baked into the decoded sensor pixels.
/// `chroma` scales colourfulness by display luminance after the curve (a camera's per-channel
/// curve saturates shadows and bleaches highlights toward white, which a luminance curve can't).
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
pub struct CameraTone {
    knots: [[f32; 2]; 32],
    chroma: [f32; CHROMA_N],
}

impl<'de> serde::Deserialize<'de> for CameraTone {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(serde::Deserialize)]
        struct Wire {
            knots: [[f32; 2]; 32],
            // absent in smart previews written before the chroma curve existed
            #[serde(default)]
            chroma: Option<[f32; CHROMA_N]>,
        }
        let w = Wire::deserialize(d)?;
        let tone = Self::new(w.knots).ok_or_else(|| serde::de::Error::custom("invalid camera tone curve"))?;
        match w.chroma {
            Some(c) => tone.with_chroma(c).ok_or_else(|| serde::de::Error::custom("invalid camera chroma curve")),
            None => Ok(tone),
        }
    }
}

impl CameraTone {
    pub fn new(knots: [[f32; 2]; 32]) -> Option<Self> {
        let mut previous = [0.0, 0.0];
        for p in knots {
            if !p.iter().all(|v| v.is_finite()) || p[0] <= previous[0] || p[1] < previous[1] || p[1] >= 1.0 {
                return None;
            }
            previous = p;
        }
        Some(Self { knots, chroma: NO_CHROMA })
    }

    /// The curve with chroma scales at display luminance 0, 1/7 … 1 (each finite, 0..=4).
    pub fn with_chroma(self, chroma: [f32; CHROMA_N]) -> Option<Self> {
        chroma.iter().all(|k| k.is_finite() && (0.0..=4.0).contains(k)).then_some(Self { chroma, ..self })
    }

    pub fn chroma(&self) -> &[f32; CHROMA_N] {
        &self.chroma
    }

    /// The (scene, display) luminance knots, in increasing order.
    pub fn knots(&self) -> &[[f32; 2]; 32] {
        &self.knots
    }

    pub fn apply(&self, y: f32) -> f32 {
        if !y.is_finite() || y <= 0.0 {
            return 0.0;
        }
        let mut previous = [0.0, 0.0];
        for p in self.knots {
            if y <= p[0] {
                let t = (y - previous[0]) / (p[0] - previous[0]);
                return previous[1] + t * (p[1] - previous[1]);
            }
            previous = p;
        }
        let a = self.knots[30];
        let b = self.knots[31];
        // Extend beyond observed (unclipped) highlights with a continuous, bounded shoulder.
        let slope = ((b[1] - a[1]) / (b[0] - a[0])).clamp(0.1, 16.0);
        1.0 - (1.0 - b[1]) * (-(y - b[0]) * slope / (1.0 - b[1]).max(0.01)).exp()
    }

    /// [`CameraTone::apply`] as process V5 extends it past the last knot (issue #1632). The curve is
    /// fitted only to camera JPEG tones below 0.85, so its last knot lies 1 to 3 stops below the
    /// sensor's white on most photos and everything above it is extrapolated. [`CameraTone::apply`]
    /// closes the gap to white exponentially in linear light: it reaches 8-bit 255 within about a
    /// stop past the sensor's white, and Exposure +1 turns the top stop the sensor recorded white.
    /// Here the gap shrinks in stops instead, `gap / (1 + c·(2^(k·t) − 1))` at `t` stops past the
    /// knot, with `c` matching the curve's slope there (so the join is smooth) and the gap halving
    /// every [`ROLLOFF_STOPS`] stops in the end. Same value as [`CameraTone::apply`] up to the last
    /// knot, monotone, finite and at most 1. In real numbers it only approaches white; in `f32` the
    /// gap can round to 0 far up, and within a stop or two for a last knot a few ulps below white.
    pub fn apply_rolloff(&self, y: f32) -> f32 {
        let (a, b) = (self.knots[30], self.knots[31]);
        if !y.is_finite() || y <= b[0] {
            return self.apply(y);
        }
        // the last segment's own slope, so the join is smooth even when it is flat (`new` keeps the
        // knots non-decreasing, so it is at least 0)
        let slope = ((b[1] - a[1]) / (b[0] - a[0])).max(0.0);
        // `CameraTone::new` keeps the last knot below 1, so the gap is positive (no floor: one
        // would step the curve down past a knot within it of white)
        rolloff_past(y, b, slope, CAMERA_MIN_SLOPE)
    }
}

/// The pace [`CameraTone::apply_rolloff`] closes on white at least at, as display per unit of scene
/// luminance at the last knot (the floor [`CameraTone::apply`] puts on its slope there). A last
/// segment flatter than this still joins with its own slope; [`rolloff_past`] then adds a term that
/// has no slope at the knot and reaches this pace further up, so a flat curve does not stay below
/// white.
const CAMERA_MIN_SLOPE: f32 = 0.1;

/// Stops in which the gap between a V5 curve and white halves, far past where its shoulder starts
/// ([`CameraTone::apply_rolloff`], [`ToneMap::rolloff_hdr`]). With 1, the sensor's white renders at
/// 8-bit 239–250 on the CC0 corpus files with a camera curve (Sony, Nikon, Fujifilm) at Exposure 0
/// and below 254 up to Exposure +2, where the extension of [`CameraTone::apply`] reaches 254–255 by
/// Exposure +1 (issue #1632).
pub const ROLLOFF_STOPS: f32 = 1.0;

/// Process V5's shoulder past a knee at scene luminance `x` and display `o` (below 1), where the
/// curve rises at `slope` (display per unit of scene luminance): the gap to white is
/// `gap / (1 + c·(2^(k·t) − 1))` at `t = log2(y / x)` stops past the knee, `k = 1 / ROLLOFF_STOPS`,
/// so it halves every [`ROLLOFF_STOPS`] stops in the end. `c` gives it the curve's slope at the
/// knee: d(display)/d(stop) there is `slope · x · ln 2` on the curve and `gap · c · k · ln 2` on the
/// shoulder. When `slope` is below `min_slope`, `d · (u − ln(1 + u))` (with `u = 2^(k·t) − 1`)
/// joins the denominator: it adds no slope at the knee, so the join keeps the curve's own slope, and
/// far up it grows like `d · u`, so the shoulder closes on white at `min_slope`'s pace (the gap still
/// halves every [`ROLLOFF_STOPS`] stops in the end). A term growing faster than `u` would close a
/// flat curve on white faster than a steeper one far up, and a darker curve would end up above it.
/// Monotone for `y ≥ x`, finite and at most 1; in real numbers it only approaches white, while in
/// `f32` the gap rounds to 0 eventually.
fn rolloff_past(y: f32, [x, o]: [f32; 2], slope: f32, min_slope: f32) -> f32 {
    let gap = 1.0 - o;
    let k = 1.0 / ROLLOFF_STOPS;
    // bounded, so `c · 0` right past the knee stays 0 for a gap of a few ulps
    let per = |s: f32| (s * x / (gap * k)).min(1e30);
    let c = per(slope);
    let d = (per(min_slope) - c).max(0.0);
    let u = (k * (y / x).log2()).exp2() - 1.0;
    // each term only when its coefficient is positive: `0 · ∞` is NaN far up
    let linear = if c > 0.0 { c * u } else { 0.0 };
    let floor = if d > 0.0 { d * past_join(u) } else { 0.0 };
    1.0 - gap / (1.0 + linear + floor)
}

/// `u − ln(1 + u)` for `u ≥ 0`: 0 with no slope at 0, then rising like `u` (infinite for infinite
/// `u`). In `f64`, where the difference keeps its digits down to tiny `u`.
fn past_join(u: f32) -> f32 {
    if u.is_infinite() {
        return u;
    }
    let u = f64::from(u);
    (u - u.ln_1p()).max(0.0) as f32
}

/// Stops below its white point where process V5's generic raw curve ([`ToneMap::rolloff_hdr`])
/// leaves the extended Reinhard curve for the [`ROLLOFF_STOPS`] shoulder, instead of clipping at
/// the white point. With ½, the sensor's white of a raw without a camera curve (scene 1.0, about
/// ¼ stop below that white point at default settings) renders at about 8-bit 235, 248 and 252 at
/// Exposure 0, +1 and +2 where V1 to V4 render 239, 255 and 255 (issue #1632).
pub const GENERIC_KNEE_STOPS: f32 = 0.5;

/// Scene EV over grey where [`ToneMap::hdr`] starts lifting highlights above the SDR curve.
const HDR_FROM_EV: f32 = 1.5;

#[derive(Clone, Debug)]
pub struct ToneMap {
    lut: Vec<f32>,
    chroma: [f32; CHROMA_N],
}

impl ToneMap {
    pub fn camera(curve: &CameraTone, contrast: f64, whites: f64, blacks: f64) -> ToneMap {
        ToneMap::camera_hdr(curve, contrast, whites, blacks, 1.0)
    }

    /// [`ToneMap::camera`] for an HDR render reaching `peak` (linear, SDR white = 1; 1 = SDR
    /// exactly): the camera's look up to 1.5 EV over grey, so midtones match the SDR render (and a
    /// gain map only lifts highlights), then the highlights gain up to `peak − 1` more as in
    /// [`ToneMap::hdr`].
    pub fn camera_hdr(curve: &CameraTone, contrast: f64, whites: f64, blacks: f64, peak: f32) -> ToneMap {
        ToneMap::camera_with(|y| curve.apply(y), curve.chroma, contrast, whites, blacks, peak)
    }

    /// [`ToneMap::camera_hdr`] of process V5: past its last knot the camera curve rolls off in stops
    /// ([`CameraTone::apply_rolloff`]) instead of closing on white within a stop of the sensor's.
    pub fn camera_rolloff_hdr(curve: &CameraTone, contrast: f64, whites: f64, blacks: f64, peak: f32) -> ToneMap {
        ToneMap::camera_with(|y| curve.apply_rolloff(y), curve.chroma, contrast, whites, blacks, peak)
    }

    fn camera_with(curve: impl Fn(f32) -> f32, chroma: [f32; CHROMA_N], contrast: f64, whites: f64, blacks: f64, peak: f32) -> ToneMap {
        let peak = if peak.is_finite() { peak.max(1.0) } else { 1.0 };
        let adjustment = Self::display(contrast, whites, blacks);
        let neutral = contrast == 0.0 && whites == 0.0 && blacks == 0.0;
        let top = (2.9 - 1.6 * (whites as f32 / 100.0) + peak.log2() + 1.0).max(HDR_FROM_EV + 0.5);
        let lut = (0..LUT_N)
            .map(|i| {
                let ev = LUT_MIN_EV + (LUT_MAX_EV - LUT_MIN_EV) * i as f32 / (LUT_N - 1) as f32;
                let y = curve(GREY * 2f32.powf(ev));
                let o = if neutral { y } else { adjustment.apply(y) };
                if peak > 1.0 { (o + (peak - 1.0) * smooth(HDR_FROM_EV, top, ev)).clamp(0.0, peak) } else { o }
            })
            .collect();
        ToneMap { lut, chroma }
    }
    /// `contrast`, `whites`, `blacks` in −100..100 (Lightroom slider units).
    pub fn new(contrast: f64, whites: f64, blacks: f64) -> ToneMap {
        ToneMap::hdr(contrast, whites, blacks, 1.0)
    }

    /// [`ToneMap::new`] for an HDR render reaching `peak` (linear, SDR white = 1; 1 = SDR exactly).
    /// Below about half of SDR white the curve is the SDR one; above, the highlights gain up to
    /// `peak − 1` more along a smoothstep in scene EV, from 1.5 EV over grey to one stop past the
    /// white point moved up by the headroom, so the result stays monotone, never exceeds `peak`,
    /// and reaches it there (within the table's range).
    pub fn hdr(contrast: f64, whites: f64, blacks: f64, peak: f32) -> ToneMap {
        ToneMap::generic(contrast, whites, blacks, peak, false)
    }

    /// [`ToneMap::hdr`] of process V5 (issue #1632): from [`GENERIC_KNEE_STOPS`] below the white
    /// point the curve rolls off into white in stops, with the same value and slope at the join,
    /// instead of reaching white about ¼ stop past the sensor's white and clipping there. The SDR
    /// curve stays below 1 (SDR white) and is the same as [`ToneMap::hdr`]'s below the join; for
    /// `peak` above 1 the HDR lift then takes highlights up to `peak`, as in [`ToneMap::hdr`].
    pub fn rolloff_hdr(contrast: f64, whites: f64, blacks: f64, peak: f32) -> ToneMap {
        ToneMap::generic(contrast, whites, blacks, peak, true)
    }

    fn generic(contrast: f64, whites: f64, blacks: f64, peak: f32, rolloff: bool) -> ToneMap {
        let peak = if peak.is_finite() { peak.max(1.0) } else { 1.0 };
        let c = (contrast / 100.0) as f32;
        let slope = if c >= 0.0 { 1.0 + 0.55 * c } else { 1.0 + 0.4 * c };
        // White point: scene luminance (after contrast) that maps to display 1.0.
        let white_ev = 2.9 - 1.6 * (whites as f32 / 100.0);
        let wl = GREY * 2f32.powf(white_ev);
        let pre = 1.0 + GREY / wl; // keep grey near grey
        let b = (blacks / 100.0) as f32;
        // extended Reinhard with white point wl: y(1 + y/wl²)/(1 + y), reaching 1 at wl
        let reinhard = |y: f32| y * (1.0 + y / (wl * wl)) / (1.0 + y);
        // V5's knee, below the white point, and the curve's slope there: (1 + y(2 + y)/wl²)/(1 + y)²
        let knee = wl * (-GENERIC_KNEE_STOPS).exp2();
        let knee_slope = (1.0 + knee * (2.0 + knee) / (wl * wl)) / ((1.0 + knee) * (1.0 + knee));
        let knee = [knee, reinhard(knee)];
        let lut = (0..LUT_N)
            .map(|i| {
                let ev = LUT_MIN_EV + (LUT_MAX_EV - LUT_MIN_EV) * i as f32 / (LUT_N - 1) as f32;
                let y = GREY * 2f32.powf(ev * slope) * pre;
                let mut o = if rolloff && y > knee[0] && knee[1] < 1.0 { rolloff_past(y, knee, knee_slope, 0.0) } else { reinhard(y) };
                o = o.min(1.0);
                // Toe: blacks < 0 crushes, > 0 lifts.
                if b < 0.0 {
                    // Smooth max(0, o − k) (a soft knee), renormalized so 1 stays 1.
                    let k = -b * 0.035;
                    let e = 0.004;
                    let soft = |v: f32| ((v - k) + ((v - k) * (v - k) + e * e).sqrt()) * 0.5;
                    o = (soft(o) - soft(0.0)) / (soft(1.0) - soft(0.0));
                } else if b > 0.0 {
                    let k = b * 0.03;
                    o = k + (1.0 - k) * o;
                }
                if peak > 1.0 {
                    let top = (white_ev + peak.log2() + 1.0).max(HDR_FROM_EV + 0.5);
                    o += (peak - 1.0) * smooth(HDR_FROM_EV, top, ev * slope);
                }
                o.clamp(0.0, peak)
            })
            .collect();
        ToneMap { lut, chroma: NO_CHROMA }
    }

    /// Tone map for display-referred sources: identity at neutral settings.
    pub fn display(contrast: f64, whites: f64, blacks: f64) -> ToneMap {
        ToneMap::display_hdr(contrast, whites, blacks, 1.0)
    }

    /// [`ToneMap::display`] for an HDR render reaching `peak`: values above SDR white (an HDR
    /// source, e.g. a decoded gain-map image) pass through and roll off softly towards `peak`
    /// instead of clipping at 1; SDR sources (≤ 1) render as with [`ToneMap::display`].
    pub fn display_hdr(contrast: f64, whites: f64, blacks: f64, peak: f32) -> ToneMap {
        let peak = if peak.is_finite() { peak.max(1.0) } else { 1.0 };
        let c = (contrast / 100.0) as f32;
        let w = (whites / 100.0) as f32;
        let b = (blacks / 100.0) as f32;
        let m = GREY.powf(1.0 / 2.2);
        // Neutral sliders are the identity up to white (issue #1945): the SDR shoulder below would
        // pull 1.0 down to 0.9875 (8-bit 255 to 254), so it only shapes an edited curve.
        let neutral = c == 0.0 && w == 0.0 && b == 0.0;
        let lut = (0..LUT_N)
            .map(|i| {
                let ev = LUT_MIN_EV + (LUT_MAX_EV - LUT_MIN_EV) * i as f32 / (LUT_N - 1) as f32;
                let y = GREY * 2f32.powf(ev);
                let mut p = y.powf(1.0 / 2.2);
                if p <= 1.0 {
                    // S-curve anchored at 0, grey and 1
                    p += c * 0.35 * (p - m) * (1.0 - (2.0 * p - 1.0).powi(2));
                    // whites: lift/lower the upper tones; blacks: the lower tones
                    let up = smooth(0.45, 1.0, p);
                    p += w * 0.12 * up * (1.0 - p * 0.5);
                    let lo = 1.0 - smooth(0.0, 0.45, p);
                    p += b * 0.07 * lo * (p * 2.0).min(1.0);
                } else {
                    p += w * 0.12 * 0.5;
                }
                let mut o = p.max(0.0).powf(2.2);
                // short shoulder (SDR, edited curves): slope 1 at 0.95, reaching 1.0 at 1.05
                if peak > 1.0 && o > 0.95 {
                    // exponential shoulder: slope 1 at 0.95, asymptote at `peak`
                    let r = peak - 0.95;
                    o = 0.95 + r * (1.0 - (-(o - 0.95) / r).exp());
                } else if o > 0.95 && !neutral {
                    let d = (o - 0.95).min(0.1);
                    o = 0.95 + d - d * d / 0.2;
                }
                o.clamp(0.0, peak)
            })
            .collect();
        ToneMap { lut, chroma: NO_CHROMA }
    }

    /// The table (`LUT_N` entries, see [`ToneMap::apply`]).
    pub fn lut(&self) -> &[f32] {
        &self.lut
    }

    /// The chroma curve (`CHROMA_N` entries, see [`ToneMap::chroma_scale`]).
    pub fn chroma_lut(&self) -> &[f32] {
        &self.chroma
    }

    /// Chroma scale at display luminance `o` (exactly 1 everywhere unless a camera look sets it).
    #[inline]
    pub fn chroma_scale(&self, o: f32) -> f32 {
        if !o.is_finite() {
            return 1.0;
        }
        let f = o.clamp(0.0, 1.0) * (CHROMA_N - 1) as f32;
        let i = (f as usize).min(CHROMA_N - 2);
        let t = f - i as f32;
        let (a, b) = (self.chroma.get(i).copied().unwrap_or(1.0), self.chroma.get(i + 1).copied().unwrap_or(1.0));
        if a == b { a } else { a + (b - a) * t }
    }

    /// Scene luminance → display-linear luminance.
    #[inline]
    pub fn apply(&self, y: f32) -> f32 {
        if y <= 0.0 {
            return 0.0;
        }
        let ev = (y / GREY).log2();
        let f = ((ev - LUT_MIN_EV) / (LUT_MAX_EV - LUT_MIN_EV)).clamp(0.0, 1.0) * (LUT_N - 1) as f32;
        let i = (f as usize).min(LUT_N - 2);
        let t = f - i as f32;
        let v = self.lut[i] + (self.lut[i + 1] - self.lut[i]) * t;
        if ev < LUT_MIN_EV { v * (y / (GREY * 2f32.powf(LUT_MIN_EV))) } else { v }
    }
}

fn smooth(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn knots() -> [[f32; 2]; 32] {
        std::array::from_fn(|i| {
            let x = 0.004 * 1.18f32.powi(i as i32);
            [x, 1.0 - (-2.0 * x).exp()]
        })
    }

    #[test]
    fn chroma_curve_is_identity_unless_set_and_survives_serde() {
        let plain = CameraTone::new(knots()).unwrap();
        let map = ToneMap::camera(&plain, 0.0, 0.0, 0.0);
        assert!([0.0, 0.3, 0.77, 1.0, 2.0].iter().all(|o| map.chroma_scale(*o) == 1.0));
        assert!([0.0, 0.5, 1.0].iter().all(|o| ToneMap::new(0.0, 0.0, 0.0).chroma_scale(*o) == 1.0));
        // smart previews written before the chroma curve existed still load (identity)
        let old = serde_json::json!({ "knots": knots() });
        assert_eq!(serde_json::from_value::<CameraTone>(old).unwrap(), plain);
        let tone = plain.with_chroma([1.4, 1.3, 1.1, 1.0, 0.7, 0.4, 0.25, 0.2]).unwrap();
        let back: CameraTone = serde_json::from_value(serde_json::to_value(tone).unwrap()).unwrap();
        assert_eq!(back, tone);
        let map = ToneMap::camera(&tone, 0.0, 0.0, 0.0);
        assert!((map.chroma_scale(0.0) - 1.4).abs() < 1e-6 && (map.chroma_scale(1.0) - 0.2).abs() < 1e-6);
        assert!((map.chroma_scale(0.5 / 7.0) - 1.35).abs() < 1e-5, "interpolates between nodes");
        assert_eq!(map.chroma_scale(f32::NAN), 1.0);
        // hostile values are rejected, also when deserialized
        assert!(plain.with_chroma([f32::NAN; CHROMA_N]).is_none());
        assert!(plain.with_chroma([-1.0; CHROMA_N]).is_none());
        let bad = serde_json::json!({ "knots": knots(), "chroma": [9.0, 1, 1, 1, 1, 1, 1, 1] });
        assert!(serde_json::from_value::<CameraTone>(bad).is_err());
    }

    #[test]
    fn camera_curve_preserves_black_and_extends_headroom() {
        let knots = std::array::from_fn(|i| {
            let x = 0.005 * 1.15f32.powi(i as i32);
            [x, 1.0 - (-3.0 * x).exp()]
        });
        let curve = CameraTone::new(knots).unwrap();
        let base = ToneMap::camera(&curve, 0.0, 0.0, 0.0);
        assert_eq!(base.apply(0.0), 0.0);
        assert!(base.apply(0.1) < base.apply(0.2));
        let mut previous = 0.0;
        for i in 0..2000 {
            let y = 1e-6 * 1.01f32.powi(i);
            let v = base.apply(y);
            assert!((0.0..=1.0).contains(&v));
            assert!(v >= previous - 1e-6);
            previous = v;
        }
        assert!((base.apply(0.1) - curve.apply(0.1)).abs() < 0.001);
        assert!(ToneMap::camera(&curve, 50.0, 0.0, 0.0).apply(0.3) > base.apply(0.3));
        let mut invalid = knots;
        invalid[1][0] = invalid[0][0];
        assert!(CameraTone::new(invalid).is_none());
        assert!(serde_json::from_value::<CameraTone>(serde_json::json!({"knots": invalid})).is_err());
    }

    #[test]
    fn rolloff_joins_the_curve_smoothly_and_never_reaches_white() {
        let curve = CameraTone::new(knots()).unwrap();
        let last = curve.knots()[31];
        // the camera's curve up to the last knot
        for i in 0..200 {
            let y = last[0] * i as f32 / 199.0;
            assert_eq!(curve.apply_rolloff(y), curve.apply(y), "{y}");
        }
        // continuous, with the slope of the last segment, at the knot
        let a = curve.knots()[30];
        let slope = (last[1] - a[1]) / (last[0] - a[0]);
        let h = last[0] * 1e-3;
        let after = (curve.apply_rolloff(last[0] + h) - curve.apply_rolloff(last[0])) / h;
        assert!((after - slope).abs() < 0.02 * slope, "{after} vs {slope}");
        // monotone, below white, and slower to close on it than the V1/V2 extension
        let mut previous = last[1];
        for i in 1..3000 {
            let y = last[0] * 1.005f32.powi(i);
            let o = curve.apply_rolloff(y);
            assert!(o >= previous && o < 1.0, "{y}: {o}");
            assert!(o <= curve.apply(y) + 1e-6, "{y}");
            previous = o;
        }
        assert!(1.0 - curve.apply_rolloff(last[0] * 8.0) > 4.0 * (1.0 - curve.apply(last[0] * 8.0)));
        // a steep last segment joins with its own slope, and a last knot next to white without a step
        for top in [0.9, 0.99995, 1.0 - f32::EPSILON] {
            let mut k = knots();
            k[31] = [k[30][0] * 1.01, top];
            let c = CameraTone::new(k).unwrap();
            let slope = (k[31][1] - k[30][1]) / (k[31][0] - k[30][0]);
            assert!(slope > 16.0 || top < 0.99, "{slope}");
            let mut prev = c.apply_rolloff(k[31][0]);
            assert_eq!(prev, top);
            for i in 1..2000 {
                let o = c.apply_rolloff(k[31][0] * 1.0001f32.powi(i));
                assert!(o.is_finite() && o >= prev && o <= 1.0, "{top} {i}: {o} < {prev}");
                prev = o;
            }
            if top == 0.9 {
                let h = k[31][0] * 1e-5;
                let after = (c.apply_rolloff(k[31][0] + h) - top) / h;
                assert!((after - slope).abs() < 0.02 * slope, "{after} vs {slope}");
            }
        }
        // a flat or shallow last segment joins with its own slope (not the 0.1 floor of the V1/V2
        // extension) and still closes on white further up
        for s in [0.0f32, 0.05] {
            let mut k = knots();
            k[31][1] = k[30][1] + s * (k[31][0] - k[30][0]);
            let c = CameraTone::new(k).unwrap();
            let (x, top) = (k[31][0], k[31][1]);
            let h = x * 1e-4;
            let after = (c.apply_rolloff(x + h) - top) / h;
            assert!((after - s).abs() < 0.01, "{s}: right slope {after}");
            let mut prev = top;
            for i in 1..3000 {
                let o = c.apply_rolloff(x * 1.005f32.powi(i));
                assert!(o.is_finite() && o >= prev && o <= 1.0, "{s} {i}: {o} < {prev}");
                prev = o;
            }
            assert!(1.0 - c.apply_rolloff(x * 64.0) < (1.0 - top) / 4.0, "{s}");
            for y in [f32::MAX, f32::INFINITY] {
                assert!((0.0..=1.0).contains(&c.apply_rolloff(y)), "{s} {y}");
            }
        }
        // hostile input
        for y in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -1.0, 0.0, f32::MAX] {
            let o = curve.apply_rolloff(y);
            assert!(o.is_finite() && (0.0..=1.0).contains(&o), "{y}: {o}");
        }
        // the tone map built from it is monotone and bounded, SDR and HDR
        for peak in [1.0f32, 4.0] {
            let t = ToneMap::camera_rolloff_hdr(&curve, 0.0, 0.0, 0.0, peak);
            let mut prev = -1.0;
            for i in 0..2400 {
                let o = t.apply(1e-5 * 1.01f32.powi(i));
                assert!((0.0..=peak).contains(&o) && o >= prev - 1e-6, "{peak} {i}: {o}");
                prev = o;
            }
        }
    }

    #[test]
    fn generic_rolloff_joins_the_curve_smoothly_and_never_reaches_white() {
        for (c, w, b) in [(0.0, 0.0, 0.0), (100.0, 100.0, -100.0), (-100.0, -100.0, 100.0), (50.0, -30.0, -40.0), (0.0, -100.0, 0.0)] {
            for peak in [1.0f32, 4.0] {
                let (clip, roll) = (ToneMap::hdr(c, w, b, peak), ToneMap::rolloff_hdr(c, w, b, peak));
                let (clip, roll) = (clip.lut(), roll.lut());
                // the generic curve up to the knee, never brighter past it
                let join = roll.iter().zip(clip).position(|(r, c)| r != c).expect("the shoulder starts inside the table");
                assert!(join > 2 && roll[join] < clip[join], "{c} {w} {b} {peak}: {join}");
                for (i, (r, c)) in roll.iter().zip(clip).enumerate() {
                    assert!(*r <= c + 1e-6, "{c} {w} {b} {peak} {i}: {r} > {c}");
                }
                // same value and slope at the join: the step across it is the step before it
                let (before, across) = (roll[join - 1] - roll[join - 2], roll[join] - roll[join - 1]);
                assert!((across / before - 1.0).abs() < 0.02, "{c} {w} {b} {peak}: {before} then {across}");
                // monotone, and below white where the V1/V2 curve clips (SDR) or lifts to `peak` (HDR)
                assert!(roll.windows(2).all(|p| p[1] >= p[0] - 1e-6), "{c} {w} {b} {peak}");
                if peak == 1.0 {
                    assert_eq!(clip[LUT_N - 1], 1.0);
                    // close to it at the top of the table at default settings (10 EV over grey);
                    // Whites −100 moves the white point, and so the shoulder, up 1.6 stops
                    let top = roll[LUT_N - 1];
                    assert!(top < 1.0 && (top > 0.999 || w < 0.0), "{c} {w} {b}: {top}");
                }
            }
        }
        // no kink where V1/V2 clip: there the clipped curve stops rising at once
        let (clip, roll) = (ToneMap::new(0.0, 0.0, 0.0), ToneMap::rolloff_hdr(0.0, 0.0, 0.0, 1.0));
        let kink = |lut: &[f32]| lut.windows(3).filter(|p| p[1] - p[0] > 1e-5).map(|p| 1.0 - (p[2] - p[1]) / (p[1] - p[0])).fold(0.0f32, f32::max);
        assert!(kink(clip.lut()) > 0.9);
        assert!(kink(roll.lut()) < 0.05, "{}", kink(roll.lut()));
        // hostile input (NaN passes through `ToneMap::apply` as on every curve)
        for y in [f32::INFINITY, f32::NEG_INFINITY, -1.0, 0.0, f32::MAX, f32::MIN_POSITIVE] {
            let o = roll.apply(y);
            assert!(o.is_finite() && (0.0..=1.0).contains(&o), "{y}: {o}");
        }
    }

    #[test]
    fn monotone_and_bounded() {
        for (c, w, b) in [(0.0, 0.0, 0.0), (100.0, 100.0, -100.0), (-100.0, -100.0, 100.0), (50.0, -30.0, -40.0)] {
            let t = ToneMap::new(c, w, b);
            let mut prev = -1.0;
            for i in 0..2000 {
                let y = 1e-5 * 1.012f32.powi(i);
                let o = t.apply(y);
                assert!((0.0..=1.0).contains(&o));
                assert!(o >= prev - 1e-6, "{c} {w} {b} at {y}: {o} < {prev}");
                prev = o;
            }
        }
    }

    #[test]
    fn hdr_is_sdr_at_peak_one_and_reaches_peak() {
        for (c, w, b) in [(0.0, 0.0, 0.0), (100.0, 100.0, -100.0), (-100.0, -100.0, 100.0), (50.0, -30.0, -40.0)] {
            assert_eq!(ToneMap::hdr(c, w, b, 1.0).lut(), ToneMap::new(c, w, b).lut());
            for peak in [2.0f32, 8.0, 32.0] {
                let sdr = ToneMap::new(c, w, b);
                let t = ToneMap::hdr(c, w, b, peak);
                let mut prev = -1.0;
                for i in 0..2400 {
                    let y = 1e-5 * 1.01f32.powi(i);
                    let o = t.apply(y);
                    assert!((0.0..=peak).contains(&o), "{c} {w} {b} {peak} at {y}: {o}");
                    assert!(o >= prev - 1e-6, "{c} {w} {b} {peak} at {y}: {o} < {prev}");
                    prev = o;
                    if y <= GREY * 2f32.powf(0.9) {
                        assert!((o - sdr.apply(y)).abs() < 1e-6, "midtones differ at {y}");
                    }
                }
                assert!(t.apply(4.0) > sdr.apply(4.0));
                if (c, w, b) == (0.0, 0.0, 0.0) {
                    assert!(t.apply(GREY * 2f32.powf(LUT_MAX_EV)) > peak * 0.99, "{peak}");
                }
            }
        }
        // a broken peak falls back to SDR
        assert_eq!(ToneMap::hdr(0.0, 0.0, 0.0, f32::NAN).lut(), ToneMap::new(0.0, 0.0, 0.0).lut());
    }

    #[test]
    fn camera_hdr_keeps_the_camera_look_and_lifts_highlights() {
        let curve = CameraTone::new(knots()).unwrap().with_chroma([1.2, 1.1, 1.0, 1.0, 0.9, 0.8, 0.7, 0.6]).unwrap();
        for (c, w, b) in [(0.0, 0.0, 0.0), (40.0, -30.0, -20.0)] {
            let sdr = ToneMap::camera(&curve, c, w, b);
            assert_eq!(ToneMap::camera_hdr(&curve, c, w, b, 1.0).lut(), sdr.lut());
            assert_eq!(ToneMap::camera_hdr(&curve, c, w, b, f32::NAN).lut(), sdr.lut());
            for peak in [2.0f32, 8.0] {
                let t = ToneMap::camera_hdr(&curve, c, w, b, peak);
                assert_eq!(t.chroma_lut(), sdr.chroma_lut());
                let mut prev = -1.0;
                for i in 0..2400 {
                    let y = 1e-5 * 1.01f32.powi(i);
                    let o = t.apply(y);
                    assert!((0.0..=peak).contains(&o), "{peak} at {y}: {o}");
                    assert!(o >= prev - 1e-6, "{peak} at {y}: {o} < {prev}");
                    prev = o;
                    // the camera's midtones, not the generic HDR curve's (issue: dark HDR exports)
                    if y <= GREY * 2f32.powf(HDR_FROM_EV - 0.01) {
                        assert!((o - sdr.apply(y)).abs() < 1e-6, "midtones differ at {y}: {o} vs {}", sdr.apply(y));
                    }
                }
                assert!(t.apply(4.0) > 1.0 && t.apply(4.0) > sdr.apply(4.0) + 0.5, "{peak}: highlights lifted");
            }
        }
    }

    #[test]
    fn display_hdr_passes_hdr_sources_and_keeps_sdr() {
        assert_eq!(ToneMap::display_hdr(20.0, 10.0, -5.0, 1.0).lut(), ToneMap::display(20.0, 10.0, -5.0).lut());
        let t = ToneMap::display_hdr(0.0, 0.0, 0.0, 8.0);
        for i in 1..=90 {
            let y = i as f32 / 100.0;
            assert!((t.apply(y) - y).abs() < 2e-3, "{y} -> {}", t.apply(y));
        }
        let mut prev = 0.0;
        for i in 1..2000 {
            let o = t.apply(i as f32 / 100.0);
            assert!(o >= prev - 1e-6 && o <= 8.0);
            prev = o;
        }
        assert!(t.apply(3.0) > 2.0 && t.apply(3.0) < 3.0);
    }

    #[test]
    fn display_identity_and_monotone() {
        let t = ToneMap::display(0.0, 0.0, 0.0);
        // up to and including white (issue #1945: 1.0 came out as 0.9875, 8-bit 255 as 254); the
        // tolerance is the LUT's interpolation next to the clip at 1.0, far below an 8-bit level
        for i in 1..=100 {
            let y = i as f32 / 100.0;
            assert!((t.apply(y) - y).abs() < 1e-3, "{y} -> {}", t.apply(y));
        }
        assert_eq!(t.apply(4.0), 1.0, "neutral values pushed above white clip");
        for (c, w, b) in [(100.0, 100.0, -100.0), (-100.0, -100.0, 100.0), (60.0, -40.0, 30.0)] {
            let t = ToneMap::display(c, w, b);
            let mut prev = -1.0;
            for i in 0..1000 {
                let o = t.apply(i as f32 / 500.0);
                assert!(o >= prev - 1e-5, "{c} {w} {b}");
                prev = o;
            }
        }
        let c = ToneMap::display(60.0, 0.0, 0.0);
        assert!(c.apply(0.05) < 0.05 && c.apply(0.7) > 0.7);
    }

    #[test]
    fn display_shoulder_stays_on_edited_curves() {
        // The short shoulder is part of the V1 look of edited curves (docs/process-versions.md);
        // only neutral settings are the identity.
        for (c, w, b) in [(20.0, 0.0, 0.0), (0.0, 10.0, 0.0), (0.0, 0.0, -5.0)] {
            let t = ToneMap::display(c, w, b);
            let o = t.apply(1.0);
            assert!(o < 0.999 && o > 0.95, "{c} {w} {b}: {o}");
        }
    }

    #[test]
    fn grey_stays_near_grey_and_highlights_roll_off() {
        let t = ToneMap::new(0.0, 0.0, 0.0);
        let g = t.apply(0.18);
        assert!((0.15..0.24).contains(&g), "{g}");
        assert!(t.apply(1.0) < 0.95 && t.apply(1.0) > 0.6);
        assert!(t.apply(8.0) > 0.97);
    }

    #[test]
    fn sliders_move_the_right_way() {
        let base = ToneMap::new(0.0, 0.0, 0.0);
        let contrast = ToneMap::new(60.0, 0.0, 0.0);
        assert!(contrast.apply(0.05) < base.apply(0.05));
        assert!(contrast.apply(0.8) > base.apply(0.8));
        assert!(ToneMap::new(0.0, 60.0, 0.0).apply(0.8) > base.apply(0.8));
        assert!(ToneMap::new(0.0, 0.0, -60.0).apply(0.01) < base.apply(0.01));
        assert!(ToneMap::new(0.0, 0.0, 60.0).apply(0.01) > base.apply(0.01));
    }
}
