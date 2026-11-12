//! Fujifilm's per-shot lens correction tables from the RAF raw IFD, expressed as DNG opcodes for the shared optics
//! pipeline (the same path as Sony `arw_lens` and Panasonic `rw2`).
//!
//! Tag identity: the ExifTool FujiFilm tag-name documentation names `0xf00b` GeometricDistortionParams and
//! `0xf010` VignettingParams (signed rationals). Layout, units, radius normalisation and geometry were derived from
//! the CC0 samples and the cameras' own JPEGs; see docs/fuji-lens-corrections.md. No lens database and no external
//! decoder code. The lateral chromatic aberration table (`0xf00f`) is deliberately not applied: LightCraft's
//! demosaic already registers the colour planes, so the stored shift over-corrects (see the document).
//!
//! Layout (`n` knots): `[scale, knot × n, value × n]`. `scale × n` is the half diagonal of the recorded image in
//! pixels. Knots are radii in half diagonals, evenly spaced in `r` (`0, 0.1 … 1`) or in `r²` (`√(k/8)`) depending
//! on the body; the corner is at 1. The nine-knot tables run one knot past it, to `√(9/8)` (on the X-T4 the ninth
//! repeats the eighth's value).
//!
//! - distortion: percent, the bracket `1 + D/100` multiplies the radius; the camera's JPEG samples the raw at
//!   `source = z · r · (1 + D(source)/100)`, radii in half diagonals, with the zoom `z` that keeps the output
//!   rectangle inside the source (`1 / max` of the bracket over the frame's border radii).
//! - vignette: relative illumination in percent at the source radius; the correction is the gain `100 / V`.

use crate::{Opcode, Rect};
use craft_tiff::Ifd;

const DISTORTION: u16 = 0xf00b;
const VIGNETTING: u16 = 0xf010;
const SAMPLES: usize = 1024;
/// Largest allowed deviation of the warp polynomial from the table, in half diagonals (median over the 118 sample
/// tables 0.00012, largest 0.0007; about 1 px at 1440 px).
const MAX_WARP_ERROR: f64 = 0.0008;
/// Largest allowed relative deviation of the gain polynomial from the table (median 0.4 %, largest 1.9 %).
const MAX_GAIN_ERROR: f64 = 0.025;
/// Reweighting rounds of the minimax polynomial fit.
const FIT_ROUNDS: usize = 40;
/// `scale × n` must be the recorded image's half diagonal to this relative tolerance (other normalisations are not
/// understood).
const SCALE_TOLERANCE: f64 = 0.005;

/// Dimensions and half diagonals of the 26.1 MP (`6240 × 4160`) and 40 MP (`7728 × 5152`) active areas in pixels.
/// The X-S20 writes `scale × n = 4643` (the 40 MP half diagonal) on its uncropped 26.1 MP frame (`3749` px
/// half diagonal), while indexing the 9 knots `√(k/8)` over the 26.1 MP active area.
const XTRANS4_26MP_WIDTH: usize = 6240;
const XTRANS4_26MP_HEIGHT: usize = 4160;
const XTRANS4_26MP_HALF_DIAGONAL: f64 = 3749.0;
const XTRANS5_40MP_HALF_DIAGONAL: f64 = 4643.0;

/// Camera models whose tables were checked against the camera's own JPEG (distortion within 0.01 of the best free
/// radial fit's correlation on at least one lens, see docs/fuji-lens-corrections.md). Other bodies stay uncorrected.
/// The nine-knot bodies other than the X-T4 were checked with their tables read at `knot_last · r`; the plain radius
/// they are now read at was measured on the X-T4 only ("Knot radius" in that doc).
const VALIDATED: &[&str] = &[
    "FinePix F770EXR",
    "FinePix HS30EXR",
    "FinePix HS33EXR",
    "FinePix HS50EXR",
    "FinePix X100",
    "GFX 100",
    "GFX 50R",
    "GFX 50S",
    "GFX100 II",
    "GFX100RF",
    "GFX100S",
    "GFX100S II",
    "GFX50S II",
    "X-A1",
    "X-A10",
    "X-A2",
    "X-A3",
    "X-A5",
    "X-A7",
    "X-E1",
    "X-E2",
    "X-E2S",
    "X-E3",
    "X-E4",
    "X-E5",
    "X-H1",
    "X-H2",
    "X-H2S",
    "X-M1",
    "X-M5",
    "X-Pro1",
    "X-Pro2",
    "X-Pro3",
    "X-S1",
    "X-S10",
    "X-S20",
    "X-T1",
    "X-T10",
    "X-T100",
    "X-T2",
    "X-T20",
    "X-T200",
    "X-T3",
    "X-T30",
    "X-T30 II",
    "X-T30 III",
    "X-T4",
    "X-T5",
    "X-T50",
    "X100F",
    "X100S",
    "X100T",
    "X100V",
    "X100VI",
    "X20",
    "X30",
    "X70",
    "XF10",
    "XQ2",
];

/// One parsed table: knot positions and the value array of the same length.
struct Table {
    knots: Vec<f64>,
    values: Vec<f64>,
}

/// Whether `scale × n` matches the recorded image's `half_diagonal` (or, on an uncropped 26.1 MP X-S20 frame,
/// the 40 MP reference half diagonal that its firmware writes).
fn scale_matches(model: &str, scale_times_n: f64, half_diagonal: f64, w: usize, h: usize) -> bool {
    if (scale_times_n / half_diagonal - 1.0).abs() <= SCALE_TOLERANCE {
        return true;
    }
    model == "X-S20"
        && w == XTRANS4_26MP_WIDTH
        && h == XTRANS4_26MP_HEIGHT
        && (half_diagonal / XTRANS4_26MP_HALF_DIAGONAL - 1.0).abs() <= SCALE_TOLERANCE
        && (scale_times_n / XTRANS5_40MP_HALF_DIAGONAL - 1.0).abs() <= SCALE_TOLERANCE
}

/// Read and sanity-check a table. `half_diagonal` is the recorded image's half diagonal in pixels, which
/// `scale × n` must reproduce.
fn read(model: &str, ifd: &Ifd, tag: u16, half_diagonal: f64, w: usize, h: usize) -> Option<Table> {
    let v = ifd.f64s(tag)?;
    if v.is_empty() || v.iter().any(|x| !x.is_finite()) {
        return None;
    }
    let n = (v.len() - 1) / 2;
    if !(3..=40).contains(&n) || v.len() != 1 + 2 * n {
        return None;
    }
    let scale = v[0];
    if scale <= 0.0 || !scale_matches(model, scale * n as f64, half_diagonal, w, h) {
        return None;
    }
    let knots = v[1..1 + n].to_vec();
    if knots[0] < 0.0 || knots.windows(2).any(|w| w[1] <= w[0]) || !(0.5..=2.0).contains(&knots[n - 1]) {
        return None;
    }
    Some(Table { knots, values: v[1 + n..].to_vec() })
}

impl Table {
    /// Value `which` at knot position `x`: linear between knots, flat outside, anchored at `x = 0` to `at_zero`
    /// when the first knot lies above it (distortion is 0 and illumination 100 % on the axis).
    fn at(&self, x: f64, at_zero: Option<f64>) -> f64 {
        let (k, y) = (&self.knots, &self.values);
        if let (Some(z), true) = (at_zero, k[0] > 0.0)
            && x < k[0]
        {
            return z + (y[0] - z) * (x / k[0]).max(0.0);
        }
        if x <= k[0] {
            return y[0];
        }
        match k.iter().position(|&p| p >= x) {
            None => y[y.len() - 1],
            Some(i) => y[i - 1] + (y[i] - y[i - 1]) * (x - k[i - 1]) / (k[i] - k[i - 1]),
        }
    }
}

/// Source radius of the output radius `r` (both in half diagonals) with the distortion zoom.
struct Geometry<'a> {
    table: Option<&'a Table>,
    zoom: f64,
}

impl Geometry<'_> {
    /// The radius bracket `1 + D/100` at source radius `source` (half diagonals, the table's own unit); 1 without a table.
    fn bracket(&self, source: f64) -> f64 {
        self.table.map_or(1.0, |t| 1.0 + t.at(source, Some(0.0)) / 100.0)
    }
    fn source(&self, r: f64) -> f64 {
        let mut s = self.zoom * r;
        for _ in 0..24 {
            s = self.zoom * r * self.bracket(s);
        }
        s
    }
}

/// The frame-filling zoom: the largest radial scale for which every border point of the `w × h` output samples
/// inside the source (the bracket is largest at the nearest border radius or beyond).
fn zoom(table: &Table, nearest: f64) -> Option<f64> {
    let mut g = Geometry { table: Some(table), zoom: 1.0 };
    for _ in 0..8 {
        let m = (0..=512).map(|i| nearest + (1.0 - nearest) * i as f64 / 512.0).map(|r| g.bracket(g.source(r))).fold(f64::MIN, f64::max);
        if !(0.5..2.0).contains(&m) {
            return None;
        }
        g.zoom = 1.0 / m;
    }
    Some(g.zoom)
}

/// Fit `target(r) ≈ Σ c_i basis_i(r)` on `r ∈ [0, 1]` minimising the largest deviation (iteratively reweighted least
/// squares), returning the coefficients and that deviation.
fn minimax<const N: usize>(basis: impl Fn(f64) -> [f64; N], target: impl Fn(f64) -> f64) -> Option<([f64; N], f64)> {
    let rows: Vec<([f64; N], f64)> = (0..=SAMPLES).map(|j| j as f64 / SAMPLES as f64).map(|r| (basis(r), target(r))).collect();
    let mut weight = vec![1.0; rows.len()];
    let mut best: Option<([f64; N], f64)> = None;
    for _ in 0..FIT_ROUNDS {
        let mut matrix = [[0.0; N]; N];
        let mut rhs = [0.0; N];
        for ((terms, y), w) in rows.iter().zip(&weight) {
            for (i, &a) in terms.iter().enumerate() {
                rhs[i] += w * a * y;
                for (k, &b) in terms.iter().enumerate() {
                    matrix[i][k] += w * a * b;
                }
            }
        }
        let c = solve(matrix, rhs)?;
        let errors: Vec<f64> = rows.iter().map(|(terms, y)| (terms.iter().zip(c).map(|(a, b)| a * b).sum::<f64>() - y).abs()).collect();
        let worst = errors.iter().copied().fold(0.0, f64::max);
        if best.as_ref().is_none_or(|b| worst < b.1) {
            best = Some((c, worst));
        }
        let mean = weight.iter().sum::<f64>() / weight.len() as f64;
        for (w, e) in weight.iter_mut().zip(&errors) {
            *w *= 0.3 + e / worst.max(1e-15);
            *w /= mean;
        }
    }
    best
}

/// `source(r) = r (k0 + k1 r² + k2 r⁴ + k3 r⁶)`, rejected when it strays from the table or folds.
fn polynomial(source: impl Fn(f64) -> f64) -> Option<[f64; 4]> {
    let mut previous = -1.0;
    for j in 0..=SAMPLES {
        let s = source(j as f64 / SAMPLES as f64);
        if !s.is_finite() || s <= previous {
            return None;
        }
        previous = s;
    }
    let basis = |r: f64| {
        let r2 = r * r;
        [r, r * r2, r * r2 * r2, r * r2 * r2 * r2]
    };
    let (k, error) = minimax(basis, &source)?;
    let folds = (0..=SAMPLES).any(|j| {
        let r2 = (j as f64 / SAMPLES as f64).powi(2);
        k[0] + r2 * (3.0 * k[1] + r2 * (5.0 * k[2] + r2 * 7.0 * k[3])) <= 0.0
    });
    (error <= MAX_WARP_ERROR && !folds).then_some(k)
}

/// Gaussian elimination with partial pivoting for the small normal equations.
fn solve<const N: usize>(mut a: [[f64; N]; N], mut b: [f64; N]) -> Option<[f64; N]> {
    for col in 0..N {
        let pivot = (col..N).max_by(|&i, &j| a[i][col].abs().total_cmp(&a[j][col].abs()))?;
        if !a[pivot][col].is_finite() || a[pivot][col].abs() < 1e-12 {
            return None;
        }
        a.swap(col, pivot);
        b.swap(col, pivot);
        for row in col + 1..N {
            let factor = a[row][col] / a[col][col];
            let (top, bottom) = a.split_at_mut(row);
            for (x, p) in bottom[0][col..].iter_mut().zip(&top[col][col..]) {
                *x -= factor * p;
            }
            b[row] -= factor * b[col];
        }
    }
    let mut x = [0.0; N];
    for i in (0..N).rev() {
        x[i] = (b[i] - (i + 1..N).map(|j| a[i][j] * x[j]).sum::<f64>()) / a[i][i];
    }
    x.iter().all(|v| v.is_finite()).then_some(x)
}

/// The warp (distortion with the frame-filling zoom) for a recorded image of `w × h` pixels.
fn warp(model: &str, ifd: &Ifd, w: usize, h: usize) -> Option<Opcode> {
    let half = (w as f64).hypot(h as f64) / 2.0;
    let table = read(model, ifd, DISTORTION, half, w, h)?;
    // Percent: anything beyond a third is not distortion; an all-zero table has nothing to correct.
    if table.values.iter().any(|d| d.abs() > 30.0) || table.values.iter().all(|d| *d == 0.0) {
        return None;
    }
    let (cw, ch) = (w as f64 - 1.0, h as f64 - 1.0);
    let nearest = cw.min(ch) / cw.hypot(ch);
    let g = Geometry { table: Some(&table), zoom: zoom(&table, nearest)? };
    let k = polynomial(|r| g.source(r))?;
    // The recorded image is the active area and the tables are centred on it: DNG normalises by the farthest
    // corner, which is the half diagonal used here.
    Some(Opcode::WarpRectilinear { planes: vec![[k[0], k[1], k[2], k[3], 0.0, 0.0]], center: [0.5, 0.5] })
}

/// Radial illumination gain `100 / V` as DNG `FixVignetteRadial` (`1 + k0 r² + … + k4 r¹⁰`).
fn vignette(model: &str, ifd: &Ifd, w: usize, h: usize) -> Option<Opcode> {
    let half = (w as f64).hypot(h as f64) / 2.0;
    let t = read(model, ifd, VIGNETTING, half, w, h)?;
    if t.values.iter().any(|v| !(10.0..=130.0).contains(v)) {
        return None;
    }
    let gain = |r: f64| 100.0 / t.at(r, Some(100.0));
    if (0..=SAMPLES).all(|j| (gain(j as f64 / SAMPLES as f64) - 1.0).abs() < 1e-4) {
        return None;
    }
    let basis = |r: f64| {
        let x = r * r;
        [x, x * x, x.powi(3), x.powi(4), x.powi(5)]
    };
    let (k, error) = minimax(basis, |r| gain(r) - 1.0)?;
    // the relative error at the worst point (gain is at least 1 / 1.3 here)
    if error > MAX_GAIN_ERROR || (0..=SAMPLES).any(|j| 1.0 + basis(j as f64 / SAMPLES as f64).iter().zip(k).map(|(a, b)| a * b).sum::<f64>() < 0.5) {
        return None;
    }
    Some(Opcode::FixVignetteRadial { k, center: [0.5, 0.5] })
}

/// The lens corrections of a RAF whose camera `model` was validated (anything else, and any table that is not in
/// the understood layout and scale, is left uncorrected). `active` is the recorded image (the raw crop).
pub(super) fn corrections(model: &str, raw: &Ifd, active: Rect) -> Vec<Opcode> {
    let model = model.trim();
    if !VALIDATED.contains(&model) || active.width < 2 || active.height < 2 {
        return Vec::new();
    }
    warp(model, raw, active.width, active.height).into_iter().chain(vignette(model, raw, active.width, active.height)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft_tiff::{Entry, Value};

    pub(crate) const KNOTS9: [f64; 9] = [0.3535211268, 0.5, 0.6126760563, 0.7070422535, 0.7908450704, 0.8661971831, 0.9352112676, 1.0, 1.06056338];
    /// X-T4 with the XF16-55 at 16 mm (3913): the tables as ExifTool prints them.
    pub(crate) const XT4_DIST: [f64; 9] =
        [-1.172515869, -2.354660034, -3.458114624, -4.434677124, -5.274719238, -5.918792725, -6.328369141, -6.426345825, -6.426345825];
    pub(crate) const XT4_VIG: [f64; 9] =
        [98.46142578, 96.87792969, 94.90234375, 90.98144531, 84.73291016, 78.43701172, 72.96044922, 67.45703125, 67.45703125];
    pub(crate) const XT4_AREA: Rect = Rect { x: 0, y: 0, width: 6240, height: 4160 };

    pub(crate) fn rationals(v: &[f64]) -> Value {
        Value::SRational(v.iter().map(|x| ((x * 1.0e6).round() as i32, 1_000_000)).collect())
    }
    pub(crate) fn table(scale: f64, knots: &[f64], values: &[f64]) -> Value {
        let mut v = vec![scale];
        v.extend_from_slice(knots);
        v.extend_from_slice(values);
        rationals(&v)
    }
    pub(crate) fn xt4() -> Ifd {
        let scale = 3749.0 / 9.0;
        Ifd {
            entries: vec![
                Entry { tag: DISTORTION, value: table(scale, &KNOTS9, &XT4_DIST), offset: 0 },
                Entry { tag: VIGNETTING, value: table(scale, &KNOTS9, &XT4_VIG), offset: 0 },
            ],
            ..Default::default()
        }
    }
    fn only(tag: u16, value: Value) -> Ifd {
        Ifd { entries: vec![Entry { tag, value, offset: 0 }], ..Default::default() }
    }
    fn evaluate(k: &[f64; 6], r: f64) -> f64 {
        r * (k[0] + r * r * (k[1] + r * r * (k[2] + r * r * k[3])))
    }

    /// The X-T4's arrays become a one-plane warp with the frame-filling zoom and a vignette gain read at the plain radius.
    #[test]
    fn x_t4_tables_become_a_warp_and_a_vignette_gain() {
        let ops = corrections("X-T4", &xt4(), XT4_AREA);
        let [Opcode::WarpRectilinear { planes, center }, Opcode::FixVignetteRadial { k, center: vc }] = ops.as_slice() else { panic!("{ops:?}") };
        assert_eq!((*center, *vc), ([0.5, 0.5], [0.5, 0.5]));
        assert_eq!(planes.len(), 1, "no colour planes: lateral CA is not applied");
        // barrel distortion: zoomed to fill the frame, the corner samples inside the raw (-6.4 % bracket; the measured
        // free fit below gives 0.9635 there)
        let green = &planes[0];
        assert!((1.02..1.04).contains(&green[0]), "{}", green[0]);
        let corner = evaluate(green, 1.0);
        assert!((corner - 0.9641).abs() < 0.002, "{corner}");
        // vignette: 67.457 % at the corner, 100 % on the axis
        let gain = |r: f64| 1.0 + (0..5).map(|i| k[i] * r.powi(2 * i as i32 + 2)).sum::<f64>();
        assert!((gain(1.0) - 100.0 / 67.45703125).abs() < 0.01, "{}", gain(1.0));
        assert!((gain(0.0) - 1.0).abs() < 1e-12);
        // read at the plain radius: r = 0.5 is the second knot, r = 0.85 lies between the fifth and sixth
        assert!((gain(0.5) - 100.0 / 96.87792969).abs() < 0.01, "{}", gain(0.5));
        let v = 84.73291016 + (78.43701172 - 84.73291016) * (0.85 - 0.7908450704) / (0.8661971831 - 0.7908450704);
        assert!((gain(0.85) - 100.0 / v).abs() < 0.01, "{}", gain(0.85));
    }

    /// The warp follows a free radial fit of the camera's own JPEG of pixls 3913 to within 0.0012 half diagonals.
    #[test]
    fn matches_the_measured_x_t4_geometry() {
        // Free radial fit (data/testing/raf-lens/tools/fit_model.py) of the camera JPEG against LightCraft's uncorrected
        // render of pixls 3913 at 600 px (maximum correlation 0.996 of the high-passed luminance), no table or zoom
        // model: source = s · r · (1 + k1 r² + k2 r⁴ + k3 r⁶), r in half diagonals.
        let measured = [1.0299688, -0.1024252, 0.0195257, 0.0183398];
        let ops = corrections("X-T4", &xt4(), XT4_AREA);
        let Opcode::WarpRectilinear { planes, .. } = &ops[0] else { panic!() };
        for i in 2..=20 {
            let r = i as f64 / 20.0;
            let r2 = r * r;
            let reference = measured[0] * r * (1.0 + measured[1] * r2 + measured[2] * r2 * r2 + measured[3] * r2 * r2 * r2);
            assert!((evaluate(&planes[0], r) - reference).abs() < 0.0012, "{r}: {} vs {reference}", evaluate(&planes[0], r));
        }
    }

    #[test]
    fn only_validated_models_are_corrected() {
        for model in ["", "X-T4 ", "X-T6", "X-Pro3", "FinePix S1", "FinePix F550EXR", "X10", "XF1", "XQ1", "GFX 100S", "x-t4"] {
            let n = corrections(model, &xt4(), XT4_AREA).len();
            assert_eq!(n, usize::from(VALIDATED.contains(&model.trim())) * 2, "{model:?}");
        }
        assert!(corrections("X-T4", &xt4(), Rect::new(0, 0, 1, 1)).is_empty());
    }

    #[test]
    fn rejects_unknown_scale_layout_and_values() {
        let scale = 3749.0 / 9.0;
        let bad = |ifd: Ifd| corrections("X-T4", &ifd, XT4_AREA);
        // another normalisation (ignoring camera exceptions like the X-S20 which writes the 40 MP half diagonal), non-increasing knots, wrong lengths, garbage
        assert!(bad(only(DISTORTION, table(4643.0 / 9.0, &KNOTS9, &XT4_DIST))).is_empty());
        // X-S20 accepts the 40 MP normalization on its uncropped 26 MP sensor, but rejects in-camera crops
        let xs20_ops = corrections("X-S20", &only(DISTORTION, table(4643.0 / 9.0, &KNOTS9, &XT4_DIST)), XT4_AREA);
        assert!(matches!(xs20_ops.as_slice(), [Opcode::WarpRectilinear { .. }]), "{xs20_ops:?}");
        // Nonzero active-frame offsets are permitted as long as dimensions are 6240 × 4160
        let offset_area = Rect::new(32, 20, 6240, 4160);
        let xs20_offset_ops = corrections("X-S20", &only(DISTORTION, table(4643.0 / 9.0, &KNOTS9, &XT4_DIST)), offset_area);
        assert!(matches!(xs20_offset_ops.as_slice(), [Opcode::WarpRectilinear { .. }]), "{xs20_offset_ops:?}");
        let cropped = Rect::new(0, 0, 4992, 3328);
        assert!(corrections("X-S20", &only(DISTORTION, table(4643.0 / 9.0, &KNOTS9, &XT4_DIST)), cropped).is_empty());
        // Differing active dimensions are rejected even if hypothetically close in half-diagonal
        let wrong_dim = Rect::new(0, 0, 6242, 4157);
        assert!(corrections("X-S20", &only(DISTORTION, table(4643.0 / 9.0, &KNOTS9, &XT4_DIST)), wrong_dim).is_empty());
        let mut flat = KNOTS9;
        flat[3] = flat[2];
        assert!(bad(only(DISTORTION, table(scale, &flat, &XT4_DIST))).is_empty());
        assert!(bad(only(DISTORTION, table(scale, &KNOTS9[..8], &XT4_DIST))).is_empty());
        assert!(bad(only(DISTORTION, table(scale, &KNOTS9, &[40.0; 9]))).is_empty());
        assert!(bad(only(DISTORTION, table(scale, &KNOTS9, &[f64::NAN; 9]))).is_empty());
        assert!(bad(only(DISTORTION, Value::Short(vec![19; 19]))).is_empty());
        assert!(bad(only(DISTORTION, table(0.0, &KNOTS9, &XT4_DIST))).is_empty());
        assert!(bad(only(VIGNETTING, table(scale, &KNOTS9, &[5.0; 9]))).is_empty());
        // flat tables (nothing to correct) produce nothing
        let flat_tables = Ifd {
            entries: vec![
                Entry { tag: DISTORTION, value: table(scale, &KNOTS9, &[0.0; 9]), offset: 0 },
                Entry { tag: VIGNETTING, value: table(scale, &KNOTS9, &[100.0; 9]), offset: 0 },
            ],
            ..Default::default()
        };
        assert!(bad(flat_tables).is_empty());
        assert!(bad(Ifd::default()).is_empty());
        // an unusable distortion table leaves the warp off but not the vignette
        let mut ifd = xt4();
        ifd.entries[0].value = table(1.0, &KNOTS9, &XT4_DIST);
        let ops = bad(ifd);
        assert!(matches!(ops.as_slice(), [Opcode::FixVignetteRadial { .. }]), "{ops:?}");
    }

    #[test]
    fn old_layout_with_eleven_knots_and_pincushion_zoom() {
        // X-T2-style: 0 … 1 in tenths, 11 knots, scale = half diagonal / 11; a 3:2 6000 × 4000 frame.
        let knots: Vec<f64> = (0..=10).map(|i| i as f64 / 10.0).collect();
        let dist = [0.0, 0.02, 0.05, 0.1, 0.17, 0.27, 0.39, 0.54, 0.69, 0.84, 0.98];
        let scale = 3605.0 / 11.0;
        let ifd = only(DISTORTION, table(scale, &knots, &dist));
        let area = Rect::new(0, 0, 6000, 4000);
        let ops = corrections("X-T2", &ifd, area);
        let [Opcode::WarpRectilinear { planes, .. }] = ops.as_slice() else { panic!("{ops:?}") };
        assert_eq!(planes.len(), 1);
        // pincushion bracket > 1 everywhere: the zoom shrinks the sampled radius; the corner samples exactly its edge
        assert!(planes[0][0] < 1.0 && planes[0][0] > 0.98);
        let nearest = 5999.0f64.min(3999.0) / 5999.0f64.hypot(3999.0);
        let at_edge = evaluate(&planes[0], nearest) / nearest;
        assert!((at_edge - planes[0][0]).abs() < 0.004);
    }

    #[test]
    fn xs20_40mp_scale_table_produces_warp_and_vignette() {
        let scale = 4643.0 / 9.0;
        let ifd = Ifd {
            entries: vec![
                Entry { tag: DISTORTION, value: table(scale, &KNOTS9, &XT4_DIST), offset: 0 },
                Entry { tag: VIGNETTING, value: table(scale, &KNOTS9, &XT4_VIG), offset: 0 },
            ],
            ..Default::default()
        };
        // X-S20 on its uncropped 26 MP sensor emits both WarpRectilinear and FixVignetteRadial
        let ops = corrections("X-S20", &ifd, XT4_AREA);
        let [Opcode::WarpRectilinear { planes, center: wc }, Opcode::FixVignetteRadial { k, center: vc }] = ops.as_slice() else {
            panic!("expected warp and vignette for X-S20: {ops:?}");
        };
        assert_eq!((*wc, *vc), ([0.5, 0.5], [0.5, 0.5]));
        assert_eq!(planes.len(), 1);
        assert!((planes[0][0] - 1.03).abs() < 0.02);
        assert!((k[0]).abs() > 0.0);
    }

    #[test]
    fn a_vignette_table_works_without_a_distortion_table() {
        let scale = 3749.0 / 9.0;
        let v = corrections("X-T4", &only(VIGNETTING, table(scale, &KNOTS9, &XT4_VIG)), XT4_AREA);
        assert!(matches!(v.as_slice(), [Opcode::FixVignetteRadial { .. }]));
        let d = corrections("X-T4", &only(DISTORTION, table(scale, &KNOTS9, &XT4_DIST)), XT4_AREA);
        assert!(matches!(d.as_slice(), [Opcode::WarpRectilinear { .. }]));
    }
}
