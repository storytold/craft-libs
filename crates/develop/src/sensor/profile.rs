//! DNG camera-profile look data carried by the file itself (Adobe DNG Specification 1.6/1.7,
//! chapter 6 and the `ProfileHueSatMap*`, `ProfileLookTable*`, `ProfileToneCurve` and
//! `ProfileGainTableMap*` tags), applied: the tables themselves are
//! [`craft_raw::profile`]'s data.
//!
//! - A hue/saturation/value table holds, for a grid of (value, hue, saturation) inputs, a hue shift
//!   in degrees, a saturation scale and a value scale (value-major, hue-middle, saturation-minor).
//!   Hue samples are spread evenly around the circle with wrap-around, saturation and value samples
//!   evenly over 0..=1; a dimension with one division is constant. Lookups are tri-linear.
//! - Tables are applied in HSV of linear ProPhoto RGB (D50). With encoding 1 ("sRGB") the value
//!   axis is indexed through the sRGB transfer curve (3D tables only).
//! - `ProfileHueSatMapData1/2` follow the colour matrices (interpolated by the same illuminant
//!   weight); `ProfileLookTableData` comes after exposure compensation and before any tone curve.
//! - `ProfileToneCurve`: (input, output) pairs in linear gamma from (0, 0) to (1, 1).
//! - `ProfileGainTableMap*` ([`super::gaintable`]) is read and kept (a DNG export writes it back)
//!   but not rendered by default (the photo's "Camera local tone mapping" option renders it, in the
//!   pipeline): Lightroom Classic renders Apple ProRAW without it (measured on the iPhone
//!   12 Pro corpus file against Lightroom Classic 15.6: with the map the default render is far
//!   brighter than Lightroom's, mean ΔE00 19; without it Lightroom's output is a tight function of
//!   the hue/saturation map, exposure compensation and tone curve alone).
//!
//! The data is read from the user's own file at run time; nothing here ships profile data.
//! Values above 1.0 (highlight headroom) keep their headroom: the table's value scale only ever
//! lowers them, so a lookup can't clip what the scene-referred pipeline still needs.

use crate::color::transfer::{linear_to_srgb, srgb_to_linear};
use crate::color::{D50, D65, Mat3, PROPHOTO, REC2020, bradford};
pub use craft_raw::profile::{HsvTable, ProfileLook, ToneCurve};

/// Looking a hue/saturation/value table up and applying it.
pub trait HsvTableExt {
    /// The entry-wise blend `g · a + (1 − g) · b` of two tables of the same shape (`None` otherwise).
    fn blend(a: &HsvTable, b: &HsvTable, g: f64) -> Option<HsvTable>;
    /// Tri-linear lookup at hue `h` (degrees), saturation `s` and value `v` (table coordinates).
    fn lookup(&self, h: f32, s: f32, v: f32) -> [f32; 3];
    /// Apply the table to one linear ProPhoto RGB colour.
    fn apply(&self, rgb: [f32; 3]) -> [f32; 3];
    /// [`Self::apply`] to `N` colours at once (`rgb[channel][lane]`), bit for bit: the same operations, with the
    /// branches on the data (largest channel, hue sector, wrap-around) turned into selects so the compiler
    /// vectorises them. The table reads, `powf` and the rare `rem_euclid` run lane by lane.
    fn apply_lanes<const N: usize>(&self, rgb: [[f32; N]; 3]) -> [[f32; N]; 3];
}

impl HsvTableExt for HsvTable {
    /// The entry-wise blend `g · a + (1 − g) · b` of two tables of the same shape (`None` otherwise).
    fn blend(a: &HsvTable, b: &HsvTable, g: f64) -> Option<HsvTable> {
        if (a.hue_divisions, a.sat_divisions, a.val_divisions) != (b.hue_divisions, b.sat_divisions, b.val_divisions) {
            return None;
        }
        let g = g.clamp(0.0, 1.0) as f32;
        let data = a.data.iter().zip(&b.data).map(|(x, y)| std::array::from_fn(|i| g * x[i] + (1.0 - g) * y[i])).collect();
        Some(HsvTable { data, ..a.clone() })
    }

    /// Tri-linear lookup at hue `h` (degrees), saturation `s` and value `v` (table coordinates).
    fn lookup(&self, h: f32, s: f32, v: f32) -> [f32; 3] {
        let hf = (wrap(h, 360.0) / 360.0 * self.hue_divisions as f32).clamp(0.0, self.hue_divisions as f32);
        let h0 = (hf as usize).min(self.hue_divisions - 1);
        let h1 = (h0 + 1) % self.hue_divisions;
        let th = (hf - h0 as f32).clamp(0.0, 1.0);
        let axis = |x: f32, n: usize| {
            let f = x.clamp(0.0, 1.0) * (n - 1) as f32;
            let i0 = (f as usize).min(n.saturating_sub(2));
            let i1 = (i0 + 1).min(n - 1);
            (i0, i1, (f - i0 as f32).clamp(0.0, 1.0))
        };
        let (s0, s1, ts) = axis(s, self.sat_divisions);
        let (v0, v1, tv) = if self.val_divisions > 1 { axis(v, self.val_divisions) } else { (0, 0, 0.0) };
        let lerp = |a: [f32; 3], b: [f32; 3], t: f32| -> [f32; 3] { std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t) };
        let plane = |vi: usize| {
            let lo = lerp(entry(self, vi, h0, s0), entry(self, vi, h0, s1), ts);
            let hi = lerp(entry(self, vi, h1, s0), entry(self, vi, h1, s1), ts);
            lerp(lo, hi, th)
        };
        if v0 == v1 { plane(v0) } else { lerp(plane(v0), plane(v1), tv) }
    }

    /// Apply the table to one linear ProPhoto RGB colour.
    fn apply(&self, rgb: [f32; 3]) -> [f32; 3] {
        let max = rgb[0].max(rgb[1]).max(rgb[2]);
        if max.is_nan() || max <= 0.0 || !max.is_finite() {
            return rgb;
        }
        let min = rgb[0].min(rgb[1]).min(rgb[2]);
        let d = max - min;
        let hue = hue_of(rgb, max, d);
        let sat = d / max;
        let v_index = if self.srgb_value { linear_to_srgb(max.min(1.0)) } else { max };
        let [shift, ss, vs] = self.lookup(hue, sat, v_index);
        // saturation and value: scaled and clipped to 1.0 (DNG chapter 6), except that inputs
        // already above 1 (out-of-ProPhoto colours, highlight headroom) are only ever lowered.
        let s2 = limited(sat, ss);
        let v2 = if self.srgb_value && max <= 1.0 {
            // (a value scale of 1 — every hue/saturation-only table — leaves the value as it is:
            // no round trip through the sRGB curve, two `powf` per pixel)
            if vs == 1.0 { max } else { srgb_to_linear((v_index * vs).min(1.0)) }
        } else if self.srgb_value {
            max * srgb_to_linear(vs.min(1.0)).min(1.0)
        } else {
            limited(max, vs)
        };
        hsv_to_rgb(hue + shift, s2, v2)
    }

    /// [`Self::apply`] to `N` colours at once (`rgb[channel][lane]`), bit for bit: the same operations, with the
    /// branches on the data (largest channel, hue sector, wrap-around) turned into selects so the compiler
    /// vectorises them. The table reads, `powf` and the rare `rem_euclid` run lane by lane.
    fn apply_lanes<const N: usize>(&self, rgb: [[f32; N]; 3]) -> [[f32; N]; 3] {
        let [r, g, b] = rgb;
        if !fits_lanes(self) {
            let mut out = rgb;
            for l in 0..N {
                [out[0][l], out[1][l], out[2][l]] = self.apply([r[l], g[l], b[l]]);
            }
            return out;
        }
        // `active`: the colours the table applies to (`apply` returns black, NaN and infinite ones as they are)
        let (mut max, mut active, mut first) = ([0.0f32; N], [true; N], [false; N]);
        let (mut d, mut ratio, mut second, mut third) = ([0.0f32; N], [0.0f32; N], [0.0f32; N], [0.0f32; N]);
        for l in 0..N {
            let m = r[l].max(g[l]).max(b[l]);
            active[l] = !(m.is_nan() | (m <= 0.0) | !m.is_finite());
            d[l] = m - r[l].min(g[l]).min(b[l]);
            // hue_of, for each largest channel
            ratio[l] = (g[l] - b[l]) / d[l];
            second[l] = (b[l] - r[l]) / d[l] + 2.0;
            third[l] = (r[l] - g[l]) / d[l] + 4.0;
            first[l] = active[l] & (d[l] > 0.0) & (m == r[l]);
            max[l] = m;
        }
        let ratio = wrap_lanes(ratio, 6.0, first);
        let (mut hue, mut sat) = ([0.0f32; N], [0.0f32; N]);
        for l in 0..N {
            let h = if max[l] == r[l] {
                ratio[l]
            } else if max[l] == g[l] {
                second[l]
            } else {
                third[l]
            };
            let h = h * 60.0;
            hue[l] = if d[l] <= 0.0 { 0.0 } else { h };
            sat[l] = d[l] / max[l];
        }
        let mut v_index = max;
        if self.srgb_value {
            for l in 0..N {
                if active[l] {
                    v_index[l] = linear_to_srgb(max[l].min(1.0));
                }
            }
        }
        let [shift, ss, vs] = lookup_lanes(self, &hue, &sat, &v_index, active);
        let (mut s2, mut v2, mut curve) = ([0.0f32; N], [0.0f32; N], [false; N]);
        for l in 0..N {
            s2[l] = limited_select(sat[l], ss[l]);
            v2[l] = if self.srgb_value { max[l] } else { limited_select(max[l], vs[l]) };
            curve[l] = self.srgb_value & active[l] & !((max[l] <= 1.0) & (vs[l] == 1.0));
        }
        for l in 0..N {
            if curve[l] {
                v2[l] = if max[l] <= 1.0 { srgb_to_linear((v_index[l] * vs[l]).min(1.0)) } else { max[l] * srgb_to_linear(vs[l].min(1.0)).min(1.0) };
            }
        }
        // hsv_to_rgb
        let hue = wrap_lanes(std::array::from_fn(|l| hue[l] + shift[l]), 360.0, active);
        let mut out = rgb;
        for l in 0..N {
            let hp = hue[l] / 60.0;
            // `(hp as u32).min(5) as f32` for every `hp`, NaN included
            let step = |k: f32| if hp >= k { 1.0 } else { 0.0 };
            let sector = step(1.0) + step(2.0) + step(3.0) + step(4.0) + step(5.0);
            let f = hp - sector;
            let (s, v) = (s2[l], v2[l]);
            let (p, q, t) = (v * (1.0 - s), v * (1.0 - s * f), v * (1.0 - s * (1.0 - f)));
            let red = if (sector == 0.0) | (sector == 5.0) {
                v
            } else if sector == 1.0 {
                q
            } else if sector == 4.0 {
                t
            } else {
                p
            };
            let green = if sector == 0.0 {
                t
            } else if sector <= 2.0 {
                v
            } else if sector == 3.0 {
                q
            } else {
                p
            };
            let blue = if sector <= 1.0 {
                p
            } else if sector == 2.0 {
                t
            } else if sector == 5.0 {
                q
            } else {
                v
            };
            out[0][l] = if active[l] { red } else { r[l] };
            out[1][l] = if active[l] { green } else { g[l] };
            out[2][l] = if active[l] { blue } else { b[l] };
        }
        out
    }
}

fn entry(table: &HsvTable, v: usize, h: usize, s: usize) -> [f32; 3] {
    table.data.get((v * table.hue_divisions + h) * table.sat_divisions + s).copied().unwrap_or([0.0, 1.0, 1.0])
}
/// Whether [`Self::lookup_lanes`] can do this table's index arithmetic in `f32`: every index below 2²³.
fn fits_lanes(table: &HsvTable) -> bool {
    let entries = table.hue_divisions.checked_mul(table.sat_divisions).and_then(|n| n.checked_mul(table.val_divisions.max(1)));
    table.hue_divisions >= 1 && table.sat_divisions >= 1 && entries.is_some_and(|n| n <= 1 << 23)
}
/// [`Self::lookup`] at `N` points (`(shift, saturation scale, value scale)[lane]`), bit for bit for the lanes
/// `wanted`, on a table that [`Self::fits_lanes`]. The indices are computed as `f32` (exact integers below 2²³:
/// `x as usize` is a saturating conversion that doesn't vectorise).
fn lookup_lanes<const N: usize>(table: &HsvTable, h: &[f32; N], s: &[f32; N], v: &[f32; N], wanted: [bool; N]) -> [[f32; N]; 3] {
    const EXACT: f32 = 8_388_608.0; // 2²³: ulp 1 from here
    // `x as usize` for `0 ≤ x < 2²³`: rounded to an integer, minus 1 if it rounded up; NaN is 0
    let floor = |x: f32| {
        let nearest = (x + EXACT) - EXACT;
        let floor = if nearest > x { nearest - 1.0 } else { nearest };
        if x.is_nan() { 0.0 } else { floor }
    };
    let (hn, sn, vn) = (table.hue_divisions as f32, table.sat_divisions as f32, table.val_divisions as f32);
    let axis = |x: f32, n: f32| {
        let f = x.clamp(0.0, 1.0) * (n - 1.0);
        let i0 = floor(f).min((n - 2.0).max(0.0));
        let i1 = (i0 + 1.0).min(n - 1.0);
        (i0, i1, (f - i0).clamp(0.0, 1.0))
    };
    let h = wrap_lanes(*h, 360.0, wanted);
    // the corners' indices (v, h, s), s fastest, and the weights between them
    let (mut at, mut t) = ([[0u32; N]; 8], [[0.0f32; N]; 3]);
    for l in 0..N {
        let hf = (h[l] / 360.0 * hn).clamp(0.0, hn);
        let h0 = floor(hf).min(hn - 1.0);
        // (h0 + 1) % hn
        let h1 = if h0 + 1.0 == hn { 0.0 } else { h0 + 1.0 };
        let (s0, s1, ts) = axis(s[l], sn);
        let (v0, v1, tv) = axis(v[l], vn);
        let (v0, v1, tv) = if table.val_divisions > 1 { (v0, v1, tv) } else { (0.0, 0.0, 0.0) };
        for (k, (vi, hi)) in [(v0, h0), (v0, h1), (v1, h0), (v1, h1)].into_iter().enumerate() {
            let row = (vi * hn + hi) * sn;
            // an exact integer below 2²³ plus 2²³ has it as its mantissa
            at[2 * k][l] = (row + s0 + EXACT).to_bits().wrapping_sub(EXACT.to_bits());
            at[2 * k + 1][l] = (row + s1 + EXACT).to_bits().wrapping_sub(EXACT.to_bits());
        }
        [t[0][l], t[1][l], t[2][l]] = [(hf - h0).clamp(0.0, 1.0), ts, tv];
    }
    let mut corner = [[[0.0f32; N]; 3]; 8];
    for l in 0..N {
        for (corner, at) in corner.iter_mut().zip(&at) {
            [corner[0][l], corner[1][l], corner[2][l]] = table.data.get(at[l] as usize).copied().unwrap_or([0.0, 1.0, 1.0]);
        }
    }
    let lerp = |a: &[f32; N], b: &[f32; N], t: &[f32; N]| {
        let mut out = [0.0f32; N];
        for l in 0..N {
            out[l] = a[l] + (b[l] - a[l]) * t[l];
        }
        out
    };
    let [th, ts, tv] = &t;
    let mut out = [[0.0f32; N]; 3];
    for (c, out) in out.iter_mut().enumerate() {
        let plane = |k: usize| lerp(&lerp(&corner[k][c], &corner[k + 1][c], ts), &lerp(&corner[k + 2][c], &corner[k + 3][c], ts), th);
        // `lookup`'s `v0 == v1` for every lane: `axis` gives `v1 = v0 + 1` whenever there are two values or more
        *out = if table.val_divisions > 1 { lerp(&plane(0), &plane(4), tv) } else { plane(0) };
    }
    out
}

/// `x · k` clipped to 1, for `x ≤ 1`; above 1, only scale factors below 1 apply (continuous at 1).
fn limited(x: f32, k: f32) -> f32 {
    if x <= 1.0 { (x * k).min(1.0) } else { x * k.min(1.0) }
}

/// `x.rem_euclid(p)` (`p > 0`), bit for bit, without the `fmodf` call for the values it nearly always
/// gets here (within one period of `[0, p)`; both subtractions are exact there).
#[inline]
fn wrap(x: f32, p: f32) -> f32 {
    if (0.0..p).contains(&x) {
        x
    } else if x > -p && x < 0.0 {
        x + p
    } else if (p..2.0 * p).contains(&x) {
        x - p
    } else {
        x.rem_euclid(p)
    }
}

/// [`wrap`] of `N` values, bit for bit for the lanes `wanted`: branch-free but for the `rem_euclid` fallback.
#[inline]
fn wrap_lanes<const N: usize>(x: [f32; N], p: f32, wanted: [bool; N]) -> [f32; N] {
    let (mut out, mut slow) = ([0.0f32; N], [false; N]);
    for l in 0..N {
        let (first, below, second) = ((x[l] >= 0.0) & (x[l] < p), (x[l] > -p) & (x[l] < 0.0), (x[l] >= p) & (x[l] < 2.0 * p));
        let (up, down) = (x[l] + p, x[l] - p);
        out[l] = if first {
            x[l]
        } else if below {
            up
        } else {
            down
        };
        slow[l] = wanted[l] & !(first | below | second);
    }
    for l in 0..N {
        if slow[l] {
            out[l] = x[l].rem_euclid(p);
        }
    }
    out
}

/// [`limited`] without a branch.
#[inline]
fn limited_select(x: f32, k: f32) -> f32 {
    let (below, above) = ((x * k).min(1.0), x * k.min(1.0));
    if x <= 1.0 { below } else { above }
}

fn hue_of(c: [f32; 3], max: f32, d: f32) -> f32 {
    if d <= 0.0 {
        return 0.0;
    }
    let h = if max == c[0] {
        wrap((c[1] - c[2]) / d, 6.0)
    } else if max == c[1] {
        (c[2] - c[0]) / d + 2.0
    } else {
        (c[0] - c[1]) / d + 4.0
    };
    h * 60.0
}

/// HSV → RGB for any saturation ≥ 0 (above 1 yields negative components, the inverse of
/// [`hue_of`] for out-of-gamut inputs).
fn hsv_to_rgb(h: f32, s: f32, v: f32) -> [f32; 3] {
    let hp = wrap(h, 360.0) / 60.0;
    let sector = (hp as u32).min(5);
    let f = hp - sector as f32;
    let (p, q, t) = (v * (1.0 - s), v * (1.0 - s * f), v * (1.0 - s * (1.0 - f)));
    match sector {
        0 => [v, t, p],
        1 => [q, v, p],
        2 => [p, v, t],
        3 => [p, q, v],
        4 => [t, p, v],
        _ => [v, p, q],
    }
}

/// Evaluating a DNG tone curve.
pub trait ToneCurveExt {
    /// The curve at `x` (0..=1): monotone cubic (Fritsch–Carlson) through the samples.
    fn eval(&self, x: f32) -> f32;
}

impl ToneCurveExt for ToneCurve {
    /// The curve at `x` (0..=1): monotone cubic (Fritsch–Carlson) through the samples.
    fn eval(&self, x: f32) -> f32 {
        let p = &self.points;
        let n = p.len();
        let x = x.clamp(p[0][0], p[n - 1][0]);
        let i = p.partition_point(|q| q[0] <= x).clamp(1, n - 1) - 1;
        let slope = |k: usize| (p[k + 1][1] - p[k][1]) / (p[k + 1][0] - p[k][0]);
        let tangent = |k: usize| -> f32 {
            if k == 0 {
                return slope(0);
            }
            if k == n - 1 {
                return slope(n - 2);
            }
            let (a, b) = (slope(k - 1), slope(k));
            if a * b <= 0.0 { 0.0 } else { 2.0 / (1.0 / a + 1.0 / b) }
        };
        let (x0, x1) = (p[i][0], p[i + 1][0]);
        let h = x1 - x0;
        let t = (x - x0) / h;
        let (t2, t3) = (t * t, t * t * t);
        let y = (2.0 * t3 - 3.0 * t2 + 1.0) * p[i][1]
            + (t3 - 2.0 * t2 + t) * h * tangent(i)
            + (-2.0 * t3 + 3.0 * t2) * p[i + 1][1]
            + (t3 - t2) * h * tangent(i + 1);
        y.clamp(0.0, 1.0)
    }
}

/// The hue/saturation map for one white.
pub trait ProfileLookExt {
    /// The hue/saturation map for calibration-1 weight `g` (one table: used for any white).
    fn hue_sat_for(&self, g: f64) -> Option<HsvTable>;
}

impl ProfileLookExt for ProfileLook {
    /// The hue/saturation map for calibration-1 weight `g` (one table: used for any white).
    fn hue_sat_for(&self, g: f64) -> Option<HsvTable> {
        match &self.hue_sat_map {
            [Some(a), Some(b)] => HsvTable::blend(a, b, g).or_else(|| Some(a.clone())),
            [Some(a), None] | [None, Some(a)] => Some(a.clone()),
            [None, None] => None,
        }
    }
}

/// The tables resolved for one white balance, applied to linear Rec.2020 D65 pixels.
#[derive(Clone, Debug)]
pub struct ProfileTables {
    hue_sat: Option<HsvTable>,
    look: Option<HsvTable>,
    to_prophoto: [[f32; 3]; 3],
    from_prophoto: [[f32; 3]; 3],
}

impl ProfileTables {
    /// `None` when the profile has no hue/saturation map and no look table.
    pub fn new(look: &ProfileLook, illuminant_weight: f64) -> Option<ProfileTables> {
        let hue_sat = look.hue_sat_for(illuminant_weight);
        if hue_sat.is_none() && look.look_table.is_none() {
            return None;
        }
        let to: Mat3 = PROPHOTO.from_xyz().mul(&bradford(D65, D50)).mul(&REC2020.to_xyz());
        let from = to.inverse()?;
        Some(ProfileTables { hue_sat, look: look.look_table.clone(), to_prophoto: to.to_f32(), from_prophoto: from.to_f32() })
    }

    /// Hue/saturation map, then `gain` (exposure compensation), then the look table:
    /// linear Rec.2020 D65 in, linear Rec.2020 D65 out.
    #[inline]
    pub fn apply(&self, rgb: [f32; 3], gain: f32) -> [f32; 3] {
        let m = &self.to_prophoto;
        let mut p: [f32; 3] = std::array::from_fn(|i| m[i][0] * rgb[0] + m[i][1] * rgb[1] + m[i][2] * rgb[2]);
        if let Some(t) = &self.hue_sat {
            p = t.apply(p);
        }
        p = p.map(|v| v * gain);
        if let Some(t) = &self.look {
            p = t.apply(p);
        }
        let m = &self.from_prophoto;
        std::array::from_fn(|i| m[i][0] * p[0] + m[i][1] * p[1] + m[i][2] * p[2])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(h: usize, s: usize, v: usize) -> HsvTable {
        HsvTable { hue_divisions: h, sat_divisions: s, val_divisions: v, data: vec![[0.0, 1.0, 1.0]; h * s * v], srgb_value: false }
    }

    #[test]
    fn wrap_is_rem_euclid_bit_for_bit() {
        for p in [6.0f32, 360.0] {
            let mut x = -3.0 * p;
            while x < 3.0 * p {
                for v in [x, x.next_up(), x.next_down(), -0.0, 0.0, p, -p, 2.0 * p, p.next_down(), (-p).next_up()] {
                    assert_eq!(wrap(v, p).to_bits(), v.rem_euclid(p).to_bits(), "{v} mod {p}");
                }
                x += p / 97.0;
            }
            for v in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 1e30, -1e30] {
                assert_eq!(wrap(v, p).is_nan(), v.rem_euclid(p).is_nan());
            }
        }
    }

    #[test]
    fn unit_value_scale_with_srgb_values_keeps_the_value() {
        let mut t = identity(6, 2, 3);
        t.srgb_value = true;
        for (i, e) in t.data.iter_mut().enumerate() {
            *e = [(i % 7) as f32 - 3.0, 1.1, 1.0];
        }
        for c in [[0.2f32, 0.1, 0.05], [0.9, 0.5, 0.7], [0.01, 0.02, 0.015], [1.5, 0.4, 0.2]] {
            let max = c[0].max(c[1]).max(c[2]);
            let out = t.apply(c);
            let got = out[0].max(out[1]).max(out[2]);
            assert!((got - max).abs() <= 1e-6 * max.max(1.0), "{c:?} -> {out:?}");
        }
    }

    /// Every combination of awkward components (zero, negative, tiny, around 1, huge, NaN, ±infinity), and random
    /// colours from black to above 1: grey, each channel largest, hues either side of the wrap-around at 0°.
    fn awkward_colours() -> Vec<[f32; 3]> {
        let special = [0.0, -0.0, 1e-30, -1e-30, 1e-45, 0.003_130_8, 0.5, -0.5, 1.0, 1.0f32.next_up(), 2.0, 1e30, -1e30];
        let special = [&special[..], &[f32::MAX, f32::INFINITY, f32::NEG_INFINITY, f32::NAN]].concat();
        let mut out = Vec::new();
        for a in &special {
            for b in &special {
                out.extend(special.iter().map(|c| [*a, *b, *c]));
            }
        }
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        for _ in 0..40_000 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let scale = [1.0, 1e-4, 8.0, 1e-38][(seed >> 56) as usize & 3];
            let c: [f32; 3] = std::array::from_fn(|i| (((seed >> (16 * i)) & 0xffff) as f32 / 65535.0 * 1.6 - 0.2) * scale);
            out.push(match seed >> 60 {
                0 => [c[0]; 3],
                1 => [c[0].abs() + 0.1, c[1], c[1].next_up()],
                2 => [c[0].abs() + 0.1, c[1], c[1].next_down()],
                _ => c,
            });
        }
        out
    }

    /// `apply_lanes` against `apply`, bit for bit, `N` colours at a time (the last batch padded).
    fn assert_lanes_match<const N: usize>(t: &HsvTable, colours: &[[f32; 3]]) {
        for chunk in colours.chunks(N) {
            let rgb: [[f32; N]; 3] = std::array::from_fn(|c| std::array::from_fn(|l| chunk.get(l).map_or(0.25, |p| p[c])));
            let out = t.apply_lanes(rgb);
            for (l, p) in chunk.iter().enumerate() {
                let (got, want) = ([out[0][l], out[1][l], out[2][l]], t.apply(*p));
                assert_eq!(got.map(f32::to_bits), want.map(f32::to_bits), "{p:?}: {got:?} vs {want:?}");
            }
        }
    }

    #[test]
    fn lanes_match_one_by_one_bit_for_bit() {
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let mut table = |h: usize, s: usize, v: usize, srgb_value: bool, shift: f32, value_scale: bool| {
            let data = (0..h * s * v)
                .map(|_| {
                    seed ^= seed << 13;
                    seed ^= seed >> 7;
                    seed ^= seed << 17;
                    let x = |k: u64| ((seed >> k) & 0xffff) as f32 / 65535.0;
                    [(x(0) * 2.0 - 1.0) * shift, 0.5 + x(16), if value_scale { 0.5 + x(32) } else { 1.0 }]
                })
                .collect();
            HsvTable { hue_divisions: h, sat_divisions: s, val_divisions: v, data, srgb_value }
        };
        let colours = awkward_colours();
        // a fitted camera look's shape; with value scales; a 2D DNG map turning hues by up to 5 turns; one hue and
        // one saturation
        for t in [
            table(72, 5, 5, true, 30.0, false),
            table(72, 5, 5, true, 30.0, true),
            table(90, 30, 1, false, 1800.0, true),
            table(1, 1, 3, true, 30.0, true),
        ] {
            assert_lanes_match::<16>(&t, &colours);
            assert_lanes_match::<5>(&t, &colours);
        }
        // entries missing (the identity fallback); too big for the lanes' index arithmetic (one by one)
        let short = HsvTable { data: vec![[10.0, 1.5, 0.8]; 7], ..identity(4, 3, 2) };
        let huge = HsvTable { data: Vec::new(), ..identity(4096, 4096, 1) };
        for t in [short, huge] {
            assert_lanes_match::<16>(&t, &colours);
        }
    }

    fn close(a: [f32; 3], b: [f32; 3], tol: f32) -> bool {
        a.iter().zip(&b).all(|(x, y)| (x - y).abs() <= tol)
    }

    #[test]
    fn identity_table_is_identity() {
        let t = identity(6, 3, 1);
        for c in [[0.2, 0.5, 0.1], [0.9, 0.9, 0.9], [0.0, 0.0, 0.4], [3.0, 1.0, 0.5], [0.3, -0.05, 0.2]] {
            assert!(close(t.apply(c), c, 1e-5), "{c:?} → {:?}", t.apply(c));
        }
        assert_eq!(t.apply([0.0; 3]), [0.0; 3]);
        assert_eq!(t.apply([-1.0, -2.0, -0.5]), [-1.0, -2.0, -0.5]);
    }

    #[test]
    fn saturation_scale_follows_the_table_layout() {
        // 4 hues (0°, 90°, 180°, 270°), 2 saturations; saturation-minor: entry (h, s) at h·2 + s.
        let mut t = identity(4, 2, 1);
        t.data[1] = [0.0, 1.5, 1.0]; // hue 0°, saturation 1 → ×1.5
        let red = [0.5, 0.4, 0.4]; // hue 0°, saturation 0.2
        let out = t.apply(red);
        // half-way along the saturation axis is ×1.1 (0.2 → 0.22 sat), value kept
        let sat = (out[0] - out[1].min(out[2])) / out[0];
        assert!((sat - 0.2 * 1.1).abs() < 1e-4, "{out:?}");
        assert!((out[0] - 0.5).abs() < 1e-6);
        // other hues are untouched (hue 180°)
        let cyan = [0.3, 0.4, 0.4];
        assert!(close(t.apply(cyan), cyan, 1e-6));
        // saturation clips to 1.0
        assert!(t.apply([1.0, 0.1, 0.1]).iter().all(|v| *v >= -1e-6));
    }

    #[test]
    fn hue_wraps_and_value_scales() {
        // 2 hues: entries at 0° and 180°, hue shift +20° at 0°, value ×0.5 at 180°.
        let mut t = identity(2, 2, 1);
        t.data = vec![[20.0, 1.0, 1.0], [20.0, 1.0, 1.0], [0.0, 1.0, 0.5], [0.0, 1.0, 0.5]];
        // 350° sits between the last sample (180°) and the first (0° = 360°), 10/180 from 360
        assert!((t.lookup(350.0, 0.5, 0.5)[0] - 20.0 * (170.0 / 180.0)).abs() < 1e-3);
        assert!((t.lookup(180.0, 0.5, 0.5)[2] - 0.5).abs() < 1e-6);
        // headroom above 1 only goes down
        let bright = [2.0, 2.4, 2.4];
        let out = t.apply(bright);
        assert!((out[1] - 1.2).abs() < 1e-5, "{out:?}");
        let mut up = identity(1, 2, 1);
        up.data = vec![[0.0, 1.0, 2.0]; 2];
        assert!((up.apply([0.4, 0.4, 0.4])[0] - 0.8).abs() < 1e-6);
        assert!((up.apply([0.8, 0.8, 0.8])[0] - 1.0).abs() < 1e-6, "clipped to 1");
        assert!((up.apply([3.0, 3.0, 3.0])[0] - 3.0).abs() < 1e-6, "headroom kept");
    }

    #[test]
    fn value_axis_and_srgb_encoding() {
        // 1 hue, 2 sats, 2 values: saturation ×2 at value 1 only
        let mut t = identity(1, 2, 2);
        t.data[3] = [0.0, 2.0, 1.0];
        let sat = |c: [f32; 3]| (c[0] - c[2]) / c[0];
        let dark = [0.25, 0.2, 0.2]; // V 0.25, S 0.2
        // linear: 1/4 of the way → S scale at s=0.2: lerp(1, lerp(1,2,0.2)=1.2, 0.25) = 1.05
        assert!((sat(t.apply(dark)) - 0.2 * 1.05).abs() < 1e-4);
        t.srgb_value = true;
        let k = linear_to_srgb(0.25); // ≈ 0.537
        let want = 0.2 * (1.0 + 0.2 * k);
        assert!((sat(t.apply(dark)) - want).abs() < 1e-4);
    }

    #[test]
    fn blend_and_tables_round_trip_through_prophoto() {
        let a = identity(2, 2, 1);
        let mut b = identity(2, 2, 1);
        b.data[1] = [10.0, 2.0, 1.0];
        let m = HsvTable::blend(&a, &b, 0.25).unwrap();
        assert_eq!(m.data[1], [7.5, 1.75, 1.0]);
        assert!(HsvTable::blend(&a, &identity(3, 2, 1), 0.5).is_none());
        let look = ProfileLook { hue_sat_map: [Some(a.clone()), None], look_table: Some(a), tone_curve: None, gain_table_map: None };
        let t = ProfileTables::new(&look, 0.5).unwrap();
        let c = [0.3, 0.2, 0.1];
        assert!(close(t.apply(c, 2.0), [0.6, 0.4, 0.2], 1e-4), "{:?}", t.apply(c, 2.0));
        assert!(ProfileTables::new(&ProfileLook::default(), 0.5).is_none());
    }

    #[test]
    fn tone_curve_interpolates_monotonically() {
        assert!(ToneCurve::from_tag(&[0.0, 0.0]).is_none());
        assert!(ToneCurve::from_tag(&[0.0, 0.0, 0.5, 0.6, 0.4, 0.7, 1.0, 1.0]).is_none(), "inputs must increase");
        assert!(ToneCurve::from_tag(&[0.0, 0.0, 1.0, 1.5]).is_none());
        let c = ToneCurve::from_tag(&[0.0, 0.0, 0.25, 0.4, 0.5, 0.7, 1.0, 1.0]).unwrap();
        assert_eq!(c.eval(0.0), 0.0);
        assert!((c.eval(0.25) - 0.4).abs() < 1e-6);
        assert!((c.eval(1.0) - 1.0).abs() < 1e-6);
        let mut last = 0.0;
        for i in 0..=100 {
            let y = c.eval(i as f32 / 100.0);
            assert!(y >= last - 1e-6, "monotone at {i}");
            last = y;
        }
        assert!(c.eval(0.1) > 0.1 && c.eval(0.1) < 0.4);
    }
}
