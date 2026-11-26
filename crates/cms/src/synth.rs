//! Synthetic CMYK output profiles.
//!
//! Adobe's CMYK profiles (U.S. Web Coated SWOP, Coated FOGRA39 …) are proprietary, and the
//! freely downloadable characterisation-based profiles come with redistribution conditions, so
//! this crate ships a CMYK profile it generates itself from a documented printer model:
//!
//! * **Forward model** (CMYK → XYZ): Yule–Nielsen modified Neugebauer (n = 2) over the 16
//!   ink overprints, with Demichel coverage weights and a parabolic tone value increase (dot
//!   gain) per ink. The eight K-free primaries use the published ISO 12647-2 coated-paper Lab
//!   aim values (paper, C, M, Y, the three two-colour overprints) and an estimated three-colour
//!   black; overprints with black are derived multiplicatively with a first-surface reflection
//!   floor.
//! * **AToB** (all intents): 11⁴ `lut16` table of the forward model, relative colorimetric
//!   (paper → L* 100).
//! * **BToA1** (colorimetric): for every Lab node, black generation (GCR) from the gray
//!   component of the K-free solution, then a bounded least-squares solve for C, M, Y with the
//!   total ink limit; out-of-gamut colours land on the closest reachable colour (clipping).
//! * **BToA0/BToA2** (perceptual/saturation): the target is first compressed into the gamut —
//!   L* is scaled onto the device black, chroma is softly compressed against a gamut boundary
//!   descriptor sampled from the forward model — then solved as above.
//!
//! That is [`GamutMapping::Legacy`]. [`GamutMapping::Oklab`] maps out-of-gamut colours in Oklab
//! instead and gives saturation its own table; the Coated FOGRA39 built-in uses it.
//!
//! The model is plausible for coated offset, not a measured characterisation: use a real
//! profile from your print provider for production print.
//!
//! [`cmyk_profile_from_model`] builds the same tables from any [`Forward`] model, inverting its
//! sampled AToB table; the Coated FOGRA39 built-in is made that way from a model fitted to
//! Fogra's measurements (`tests/fogra39.rs`).

use crate::clut::Clut;
use crate::curve::Curve;
use crate::math;
use crate::pipeline::Stage;
use crate::profile::{ColorSpace, Lut, LutKind, Pcs, Profile, ProfileClass};
use crate::{CmsError, Intent, invalid_arg};

/// Profile description of the synthetic coated CMYK profile.
pub const DESCRIPTION: &str = "PhotoCraft Coated CMYK (synthetic, 300% TAC, medium GCR)";

/// Parameters of the synthetic profile.
#[derive(Clone, Debug)]
pub struct CmykParams {
    /// Profile description text.
    pub description: String,
    /// Copyright tag text.
    pub copyright: String,
    /// Total area coverage limit (3.0 = 300 %).
    pub tac: f64,
    /// Gray component (min of C, M, Y of the K-free solution) where black generation starts.
    pub gcr_start: f64,
    /// Maximum black.
    pub k_max: f64,
    /// Exponent of the black generation curve above `gcr_start`.
    pub gcr_gamma: f64,
    /// Black generation as a curve of `[gray component, black]` points with increasing gray
    /// components in `[0, 1]`, interpolated linearly and held flat beyond the ends. Empty by
    /// default: black is then `k_max` times the ramp of `gcr_start` and `gcr_gamma` (the curve
    /// of the shipped Photocraft profile).
    pub gcr_curve: Vec<[f64; 2]>,
    /// Tone value increase at 50 % for C, M, Y, K.
    pub tvi: [f64; 4],
    /// Grid points of the AToB (4D) and BToA (3D) tables.
    pub grid_a2b: usize,
    #[allow(missing_docs)]
    pub grid_b2a: usize,
    /// Map paper and PCS white onto each other exactly: the AToB maps no ink to L* 100,
    /// a* = b* = 0, and the BToA input curves put that white on a grid node (`grid_b2a` must
    /// be odd), which holds no ink. Without them white falls between nodes of the legacy Lab
    /// encoding and separates to a trace of ink. Off by default, which keeps the bytes of the
    /// shipped Photocraft profile.
    pub white_on_node: bool,
    /// How the BToA tables bring colours into the gamut. [`GamutMapping::Legacy`] by default,
    /// which keeps the bytes of the shipped Photocraft profile.
    pub mapping: GamutMapping,
}

/// How the BToA tables map colours the printer cannot reach.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GamutMapping {
    /// The shipped Photocraft profile: out-of-gamut colours land on the closest reachable
    /// colour (ΔE76); perceptual scales L* onto the device black and compresses Lab chroma;
    /// saturation shares the perceptual table.
    Legacy,
    /// Gamut mapping in Oklab, whose hue lines stay perceptually constant (a CIELAB hue line
    /// from a saturated blue toward grey drifts toward purple; an Oklab one does not):
    ///
    /// * **Colorimetric**: in-gamut colours separate exactly. An out-of-gamut colour takes the
    ///   closest colour the printer reaches in Oklab, lightness and hue weighted above chroma,
    ///   and more so the further outside it is; black is generated from the clipped colour, so
    ///   inks the clip drives to 100 % stay there.
    /// * **Perceptual** and **saturation**: the ICC v4 perceptual reference black is mapped
    ///   onto the device black linearly in XYZ (as black point compensation does) for neutrals,
    ///   fading out with Oklab chroma, so greys separate as with colorimetric + BPC and colourful
    ///   colours keep their lightness. Perceptual then compresses Oklab chroma above `knee` ×
    ///   the gamut boundary into the boundary; saturation scales Oklab chroma by `saturation`
    ///   and clips as colorimetric does.
    Oklab { knee: f64, saturation: f64 },
}

impl Default for CmykParams {
    fn default() -> Self {
        CmykParams {
            description: DESCRIPTION.into(),
            copyright: crate::builtin::COPYRIGHT.into(),
            tac: 3.0,
            gcr_start: 0.2,
            k_max: 0.95,
            gcr_gamma: 1.1,
            gcr_curve: Vec::new(),
            tvi: [0.14, 0.14, 0.14, 0.17],
            grid_a2b: 11,
            grid_b2a: 21,
            white_on_node: false,
            mapping: GamutMapping::Legacy,
        }
    }
}

/// The printer model.
#[derive(Clone, Debug)]
pub struct CmykModel {
    /// Absolute XYZ (D50) of the 16 overprints, indexed by ink bit mask (C=1, M=2, Y=4, K=8),
    /// stored as `X^(1/n)` for the Yule–Nielsen sum.
    prim_n: [[f64; 3]; 16],
    /// Absolute XYZ of the paper.
    pub paper: [f64; 3],
    tvi: [f64; 4],
}

const YN: f64 = 2.0;

impl CmykModel {
    /// Model of a coated offset press (ISO 12647-2 paper type 1/2 aims) with the given tone value
    /// increase at 50 % for C, M, Y, K.
    pub fn coated(tvi: [f64; 4]) -> Self {
        let lab = |l: f64, a: f64, b: f64| math::lab_to_xyz([l, a, b], math::D50);
        // ISO 12647-2 (coated, paper type 1/2) Lab aims, D50/2°; CMY overprint estimated.
        let w = lab(95.0, 0.0, -2.0);
        let base: [[f64; 3]; 8] = [
            w,                       // paper
            lab(55.0, -37.0, -50.0), // C
            lab(48.0, 74.0, -3.0),   // M
            lab(24.0, 22.0, -46.0),  // C+M (blue)
            lab(89.0, -5.0, 93.0),   // Y
            lab(50.0, -65.0, 27.0),  // C+Y (green)
            lab(47.0, 68.0, 48.0),   // M+Y (red)
            lab(23.0, 0.0, 0.0),     // C+M+Y
        ];
        let k = lab(16.0, 0.0, 0.0);
        // Overprints with black: multiplicative in reflectance above a first-surface floor.
        let floor = 0.006;
        let mut prim = [[0.0; 3]; 16];
        for (m, slot) in prim.iter_mut().enumerate() {
            let p = base[m & 7];
            *slot = if m & 8 == 0 {
                p
            } else {
                std::array::from_fn(|i| {
                    let s = floor * w[i];
                    s + (p[i] - s) * (k[i] - s) / (w[i] - s)
                })
            };
        }
        let prim_n = prim.map(|p| p.map(|v| v.max(0.0).powf(1.0 / YN)));
        CmykModel { prim_n, paper: w, tvi }
    }

    /// Absolute XYZ of an ink combination (each coverage in `[0, 1]`).
    pub fn xyz(&self, cmyk: [f64; 4]) -> [f64; 3] {
        let a: [f64; 4] = std::array::from_fn(|i| {
            let v = cmyk[i].clamp(0.0, 1.0);
            (v + 4.0 * self.tvi[i] * v * (1.0 - v)).clamp(0.0, 1.0)
        });
        let mut s = [0.0; 3];
        for (m, prim) in self.prim_n.iter().enumerate() {
            let mut w = 1.0;
            for (i, ai) in a.iter().enumerate() {
                w *= if m & (1 << i) != 0 { *ai } else { 1.0 - ai };
            }
            if w == 0.0 {
                continue;
            }
            for (acc, p) in s.iter_mut().zip(prim) {
                *acc += w * p;
            }
        }
        s.map(|v| v.powf(YN))
    }

    /// ICC-relative Lab (paper = L* 100).
    pub fn lab(&self, cmyk: [f64; 4]) -> [f64; 3] {
        let x = self.xyz(cmyk);
        let rel = [x[0] * math::D50[0] / self.paper[0], x[1] * math::D50[1] / self.paper[1], x[2] * math::D50[2] / self.paper[2]];
        math::xyz_to_lab(rel, math::D50)
    }
}

/// A printer model: CMYK coverages (each in `[0, 1]`) → ICC-relative Lab (paper = L* 100).
/// The separation solves and the gamut boundary invert it.
pub trait Forward: Sync {
    /// ICC-relative Lab for the given ink coverages.
    fn lab(&self, cmyk: [f64; 4]) -> [f64; 3];
}

impl Forward for CmykModel {
    fn lab(&self, cmyk: [f64; 4]) -> [f64; 3] {
        CmykModel::lab(self, cmyk)
    }
}

/// A forward model read from a sampled AToB table: 4 inputs, legacy (`lut16`) Lab outputs, as
/// [`sample_a2b`] makes it.
struct TableForward(Clut);

impl Forward for TableForward {
    fn lab(&self, cmyk: [f64; 4]) -> [f64; 3] {
        let mut o = [0.0f32; 3];
        self.0.interp(&cmyk.map(|v| v.clamp(0.0, 1.0) as f32), &mut o);
        decode_v2(o.map(f64::from))
    }
}

/// Bounded least-squares solve for C, M, Y given K, minimising ΔE76 to `target`, subject to
/// `C + M + Y ≤ limit` (projected Levenberg–Marquardt with a numerical Jacobian).
fn solve_cmy<F: Forward + ?Sized>(model: &F, target: [f64; 3], k: f64, limit: f64, seeds: &[[f64; 3]]) -> ([f64; 3], f64) {
    solve_cmy_by(model, k, limit, seeds, |l| [l[0] - target[0], l[1] - target[1], l[2] - target[2]])
}

/// [`solve_cmy`] minimising the norm of `residual(lab)` instead of ΔE76.
fn solve_cmy_by<F: Forward + ?Sized>(model: &F, k: f64, limit: f64, seeds: &[[f64; 3]], residual: impl Fn([f64; 3]) -> [f64; 3]) -> ([f64; 3], f64) {
    let project = |mut x: [f64; 3]| {
        for _ in 0..4 {
            for v in &mut x {
                *v = v.clamp(0.0, 1.0);
            }
            let s: f64 = x.iter().sum();
            if s <= limit + 1e-12 {
                break;
            }
            let active = x.iter().filter(|v| **v > 0.0).count().max(1) as f64;
            let d = (s - limit) / active;
            for v in &mut x {
                if *v > 0.0 {
                    *v -= d;
                }
            }
        }
        x
    };
    let err = |x: [f64; 3]| residual(model.lab([x[0], x[1], x[2], k]));
    let norm2 = |e: [f64; 3]| e[0] * e[0] + e[1] * e[1] + e[2] * e[2];
    let mut best = ([0.0; 3], f64::INFINITY);
    for seed in seeds {
        let mut x = project(*seed);
        let mut e = err(x);
        let mut f = norm2(e);
        let mut lambda = 1e-3;
        for _ in 0..60 {
            // Jacobian (3×3) by forward differences.
            let h = 1e-5;
            let mut j = [[0.0; 3]; 3];
            for c in 0..3 {
                let mut xp = x;
                xp[c] += h;
                let ep = err(xp);
                for r in 0..3 {
                    j[r][c] = (ep[r] - e[r]) / h;
                }
            }
            // (JᵀJ + λ diag) δ = −Jᵀe
            let mut a = [[0.0; 3]; 3];
            let mut g = [0.0; 3];
            for r in 0..3 {
                for c in 0..3 {
                    a[r][c] = (0..3).map(|k| j[k][r] * j[k][c]).sum();
                }
                g[r] = -(0..3).map(|k| j[k][r] * e[k]).sum::<f64>();
            }
            let mut improved = false;
            for _ in 0..8 {
                let mut al = a;
                for (d, row) in al.iter_mut().enumerate() {
                    row[d] += lambda * (1.0 + a[d][d]);
                }
                let Some(inv) = math::invert(&al) else {
                    lambda *= 10.0;
                    continue;
                };
                let step = math::apply(&inv, g);
                let xn = project([x[0] + step[0], x[1] + step[1], x[2] + step[2]]);
                let en = err(xn);
                let fnew = norm2(en);
                if fnew < f {
                    let moved = (0..3).map(|i| (xn[i] - x[i]).abs()).fold(0.0, f64::max);
                    x = xn;
                    e = en;
                    f = fnew;
                    lambda = (lambda * 0.3).max(1e-9);
                    improved = moved > 1e-9;
                    break;
                }
                lambda *= 10.0;
            }
            if !improved || f < 1e-10 {
                break;
            }
        }
        if f < best.1 {
            best = (x, f);
        }
        if best.1 < 1e-6 {
            break;
        }
    }
    (best.0, best.1.sqrt())
}

const SEEDS: [[f64; 3]; 5] = [[0.3, 0.3, 0.3], [0.05, 0.05, 0.05], [0.8, 0.8, 0.8], [0.9, 0.2, 0.2], [0.2, 0.9, 0.9]];

/// Black for a gray component `g` (the smallest of C, M, Y without black), at most the total ink
/// limit: the CMY solve gets `tac - k`, so black above `tac` would break the limit on its own.
fn gcr(p: &CmykParams, g: f64) -> f64 {
    let k = match p.gcr_curve.first() {
        // The Photocraft profile's curve.
        None => {
            if g <= p.gcr_start {
                0.0
            } else {
                p.k_max * ((g - p.gcr_start) / (1.0 - p.gcr_start)).powf(p.gcr_gamma)
            }
        }
        Some(first) => {
            let mut k = first[1];
            for w in p.gcr_curve.windows(2) {
                let ([g0, k0], [g1, k1]) = (w[0], w[1]);
                if g >= g1 {
                    k = k1;
                } else if g > g0 {
                    k = k0 + (k1 - k0) * (g - g0) / (g1 - g0);
                    break;
                }
            }
            k
        }
    };
    k.clamp(0.0, 1.0).min(p.tac.max(0.0))
}

/// Colorimetric separation of a relative Lab target: GCR black generation then a CMY solve.
pub fn separate<F: Forward + ?Sized>(model: &F, p: &CmykParams, target: [f64; 3], warm: Option<[f64; 4]>) -> ([f64; 4], f64) {
    let mut seeds: Vec<[f64; 3]> = warm.map(|w| vec![[w[0], w[1], w[2]]]).unwrap_or_default();
    seeds.extend_from_slice(&SEEDS);
    let (cmy0, _) = solve_cmy(model, target, 0.0, 3.0, &seeds);
    let g = cmy0[0].min(cmy0[1]).min(cmy0[2]);
    let k = gcr(p, g);
    let mut seeds2 = vec![[(cmy0[0] - k).max(0.0), (cmy0[1] - k).max(0.0), (cmy0[2] - k).max(0.0)], cmy0];
    seeds2.extend_from_slice(&seeds);
    let (cmy, de) = solve_cmy(model, target, k, (p.tac - k).max(0.0), &seeds2);
    ([cmy[0], cmy[1], cmy[2], k], de)
}

/// Gamut boundary descriptor: maximum chroma per (L*, hue) bin, sampled from the model.
struct Gbd {
    l_bins: usize,
    h_bins: usize,
    cmax: Vec<f64>,
    black_l: f64,
}

impl Gbd {
    fn new<F: Forward + ?Sized>(model: &F, p: &CmykParams) -> Self {
        let (l_bins, h_bins) = (41usize, 72usize);
        let mut cmax = vec![0.0f64; l_bins * h_bins];
        let n = 21;
        let mut black_l = 100.0f64;
        for ci in 0..n {
            for mi in 0..n {
                for yi in 0..n {
                    for ki in 0..11 {
                        let v = [ci as f64 / (n - 1) as f64, mi as f64 / (n - 1) as f64, yi as f64 / (n - 1) as f64, ki as f64 / 10.0];
                        if v.iter().sum::<f64>() > p.tac + 1e-9 {
                            continue;
                        }
                        let lab = model.lab(v);
                        black_l = black_l.min(lab[0]);
                        let c = lab[1].hypot(lab[2]);
                        let h = lab[2].atan2(lab[1]).rem_euclid(std::f64::consts::TAU);
                        let li = ((lab[0] / 100.0) * (l_bins - 1) as f64).round().clamp(0.0, (l_bins - 1) as f64) as usize;
                        let hi = ((h / std::f64::consts::TAU) * h_bins as f64).round() as usize % h_bins;
                        let e = &mut cmax[li * h_bins + hi];
                        *e = e.max(c);
                    }
                }
            }
        }
        // Fill holes and smooth: max over a 3×3 neighbourhood, then a box blur.
        let get = |v: &Vec<f64>, l: isize, h: isize| {
            let l = l.clamp(0, l_bins as isize - 1) as usize;
            let h = h.rem_euclid(h_bins as isize) as usize;
            v[l * h_bins + h]
        };
        let mut filled = cmax.clone();
        for l in 0..l_bins as isize {
            for h in 0..h_bins as isize {
                let mut m = 0.0f64;
                for dl in -1..=1 {
                    for dh in -1..=1 {
                        m = m.max(get(&cmax, l + dl, h + dh));
                    }
                }
                filled[l as usize * h_bins + h as usize] = m;
            }
        }
        let mut smooth = filled.clone();
        for l in 0..l_bins as isize {
            for h in 0..h_bins as isize {
                let mut s = 0.0;
                for dl in -1..=1 {
                    for dh in -1..=1 {
                        s += get(&filled, l + dl, h + dh);
                    }
                }
                smooth[l as usize * h_bins + h as usize] = s / 9.0;
            }
        }
        Gbd { l_bins, h_bins, cmax: smooth, black_l }
    }

    fn max_chroma(&self, l: f64, h: f64) -> f64 {
        let lp = (l / 100.0).clamp(0.0, 1.0) * (self.l_bins - 1) as f64;
        let hp = (h.rem_euclid(std::f64::consts::TAU) / std::f64::consts::TAU) * self.h_bins as f64;
        let (l0, h0) = (lp.floor() as usize, hp.floor() as usize);
        let (fl, fh) = (lp - l0 as f64, hp - h0 as f64);
        let l1 = (l0 + 1).min(self.l_bins - 1);
        let (h0m, h1m) = (h0 % self.h_bins, (h0 + 1) % self.h_bins);
        let v = |l: usize, h: usize| self.cmax[l * self.h_bins + h];
        let a = v(l0, h0m) + (v(l0, h1m) - v(l0, h0m)) * fh;
        let b = v(l1, h0m) + (v(l1, h1m) - v(l1, h0m)) * fh;
        a + (b - a) * fl
    }

    /// Perceptual gamut compression of a relative Lab colour.
    fn compress(&self, lab: [f64; 3]) -> [f64; 3] {
        let l = self.black_l + lab[0].clamp(0.0, 100.0) * (100.0 - self.black_l) / 100.0;
        let c = lab[1].hypot(lab[2]);
        if c < 1e-9 {
            return [l, 0.0, 0.0];
        }
        let h = lab[2].atan2(lab[1]);
        let cm = self.max_chroma(l, h).max(1e-3);
        let knee = 0.8 * cm;
        let c2 = if c <= knee { c } else { knee + (cm - knee) * ((c - knee) / (cm - knee)).tanh() };
        [l, c2 * h.cos(), c2 * h.sin()]
    }
}

/// ΔE76 under which a separation counts as reaching its target.
const REACH: f64 = 1.5;
/// Oklab chroma beyond any printable colour: the top of the boundary search.
const OKLAB_CHROMA_MAX: f64 = 0.5;
/// Oklab chroma above which the perceptual and saturation tables no longer lift black: they lift
/// neutrals fully, and less the more colourful a colour is.
const LIFT_CHROMA: f64 = 0.1;
/// Weight of lightness and hue against chroma when a colour just outside the gamut is clipped.
const CLIP_WEIGHT: f64 = 1.5;
/// Growth of that weight with the squared share of chroma the clip loses.
const CLIP_WEIGHT_GROWTH: f64 = 200.0;

// Oklab (Björn Ottosson, 2020): XYZ (D65) → cone response → cube root → Lab-like axes.
const OKLAB_M1: math::Mat3 =
    [[0.8189330101, 0.3618667424, -0.1288597137], [0.0329845436, 0.9293118715, 0.0361456387], [0.0482003018, 0.2643662691, 0.6338517070]];
const OKLAB_M2: math::Mat3 =
    [[0.2104542553, 0.7936177850, -0.0040720468], [1.9779984951, -2.4285922050, 0.4505937099], [0.0259040371, 0.7827717662, -0.8086757660]];
const OKLAB_M1_INV: math::Mat3 =
    [[1.2270138511, -0.5577999807, 0.2812561490], [-0.0405801784, 1.1122568696, -0.0716766787], [-0.0763812845, -0.4214819784, 1.5861632204]];
const OKLAB_M2_INV: math::Mat3 = [[1.0, 0.3963377774, 0.2158037573], [1.0, -0.1055613458, -0.0638541728], [1.0, -0.0894841775, -1.2914855480]];

/// The [`GamutMapping::Oklab`] separations of one forward model.
struct OklabMapper<'a, F: Forward + ?Sized> {
    model: &'a F,
    p: &'a CmykParams,
    knee: f64,
    saturation: f64,
    /// XYZ scale that maps the v4 perceptual reference black onto the device black (the white
    /// stays fixed): `v' = a·v + w·(1 − a)`.
    black_scale: f64,
    to_d65: math::Mat3,
    to_d50: math::Mat3,
}

impl<'a, F: Forward + ?Sized> OklabMapper<'a, F> {
    fn new(model: &'a F, p: &'a CmykParams, knee: f64, saturation: f64) -> Self {
        let d65 = math::xy_to_xyz(math::D65_XY);
        // The darkest neutral the printer reaches.
        let (black, _) = separate(model, p, [0.0, 0.0, 0.0], None);
        let device = math::lab_to_xyz([model.lab(black)[0], 0.0, 0.0], math::D50)[1];
        let reference = crate::transform::PERCEPTUAL_BLACK[1];
        OklabMapper {
            model,
            p,
            knee,
            saturation,
            black_scale: (1.0 - device) / (1.0 - reference),
            to_d65: math::bradford(math::D50, d65),
            to_d50: math::bradford(d65, math::D50),
        }
    }

    fn oklab(&self, lab: [f64; 3]) -> [f64; 3] {
        let xyz = math::apply(&self.to_d65, math::lab_to_xyz(lab, math::D50));
        math::apply(&OKLAB_M2, math::apply(&OKLAB_M1, xyz).map(f64::cbrt))
    }

    fn lab(&self, ok: [f64; 3]) -> [f64; 3] {
        let lms = math::apply(&OKLAB_M2_INV, ok).map(|v| v * v * v);
        math::xyz_to_lab(math::apply(&self.to_d50, math::apply(&OKLAB_M1_INV, lms)), math::D50)
    }

    /// The most chroma (scale of `ok`'s chroma in `[0, top]`) the printer reaches at `ok`'s
    /// lightness and hue, and its separation; `None` when even the neutral is out of reach.
    fn boundary(&self, ok: [f64; 3], top: f64, warm: Option<[f64; 4]>) -> Option<(f64, [f64; 4])> {
        let (grey, d) = separate(self.model, self.p, self.lab([ok[0], 0.0, 0.0]), warm);
        if d > REACH {
            return None;
        }
        let (mut lo, mut hi, mut best) = (0.0, top, grey);
        for _ in 0..14 {
            let mid = 0.5 * (lo + hi);
            let (c, d) = separate(self.model, self.p, self.lab([ok[0], ok[1] * mid, ok[2] * mid]), Some(best));
            if d <= REACH {
                lo = mid;
                best = c;
            } else {
                hi = mid;
            }
        }
        Some((lo, best))
    }

    /// Colorimetric: exact in gamut; outside, the closest colour the printer reaches in Oklab,
    /// lightness and hue weighted above chroma, and more so the further outside the colour is.
    fn clip(&self, lab: [f64; 3], warm: Option<[f64; 4]>) -> [f64; 4] {
        let (cmyk, de) = separate(self.model, self.p, lab, warm);
        if de <= REACH {
            return cmyk;
        }
        let ok = self.oklab(lab);
        let c0 = ok[1].hypot(ok[2]);
        // Unit vectors along the colour's chroma and hue directions.
        let (u, v) = if c0 > 1e-9 { ([ok[1] / c0, ok[2] / c0], [-ok[2] / c0, ok[1] / c0]) } else { ([1.0, 0.0], [0.0, 1.0]) };
        // Lightness and hue errors weighted by `w`, chroma error by 1; scaled by 100 so the
        // solver's step tolerances suit it as they suit ΔE76.
        let residual = |w: f64| {
            move |l: [f64; 3]| {
                let o = self.oklab(l);
                let (da, db) = (o[1] - ok[1], o[2] - ok[2]);
                [100.0 * w * (o[0] - ok[0]), 100.0 * (da * u[0] + db * u[1]), 100.0 * w * (da * v[0] + db * v[1])]
            }
        };
        let solve = |w: f64, k: f64, from: [f64; 3]| {
            let mut seeds = vec![from];
            seeds.extend_from_slice(&SEEDS);
            solve_cmy_by(self.model, k, (self.p.tac - k).max(0.0), &seeds, residual(w)).0
        };
        // A first clip without black measures how far outside the colour is. A colour just
        // outside may give up a little lightness and hue to keep its chroma (a red reaches the
        // full magenta and yellow edge); one far outside keeps its lightness and hue and loses
        // chroma (a saturated blue does not slide along the gamut surface).
        let first = solve(CLIP_WEIGHT, 0.0, [cmyk[0], cmyk[1], cmyk[2]]);
        let o = self.oklab(self.model.lab([first[0], first[1], first[2], 0.0]));
        let loss = if c0 > 1e-9 { (1.0 - o[1].hypot(o[2]) / c0).clamp(0.0, 1.0) } else { 0.0 };
        let w = CLIP_WEIGHT * (1.0 + CLIP_WEIGHT_GROWTH * loss * loss);
        let free = solve(w, 0.0, first);
        let norm = |c: [f64; 4]| residual(w)(self.model.lab(c)).iter().map(|v| v * v).sum::<f64>();
        // Candidates, the closest kept: the separation itself (a colour it nearly reaches stays
        // continuous with its in-gamut neighbours), the clip with the separation's black, and
        // the clip with black generated from the black-free clip's gray component (inks the
        // clip drives to 100 % stay there and black replaces the rest).
        let mut best = (norm(cmyk), cmyk);
        for k in [cmyk[3], gcr(self.p, free[0].min(free[1]).min(free[2]))] {
            for from in [[cmyk[0], cmyk[1], cmyk[2]], free] {
                let cmy = solve(w, k, from);
                let c = [cmy[0], cmy[1], cmy[2], k];
                let n = norm(c);
                if n < best.0 {
                    best = (n, c);
                }
            }
        }
        // Near paper white the gamut narrows to a point: the separation's closest colour keeps
        // light tints' chroma where holding lightness would drop them toward bare paper, and
        // the table nodes there carry the in-gamut tints between them. The inks blend from the
        // clip at L* 85 to the separation at L* 95.
        let clip_share = ((95.0 - lab[0]) / 10.0).clamp(0.0, 1.0);
        std::array::from_fn(|i| clip_share * best.1[i] + (1.0 - clip_share) * cmyk[i])
    }

    /// Maps the v4 perceptual reference black onto the device black, linearly in XYZ, for
    /// neutrals; colourful colours keep their lightness.
    fn lift_black(&self, lab: [f64; 3]) -> [f64; 3] {
        let ok = self.oklab(lab);
        let neutral = (1.0 - ok[1].hypot(ok[2]) / LIFT_CHROMA).clamp(0.0, 1.0);
        let a = 1.0 - neutral * (1.0 - self.black_scale);
        let xyz = math::lab_to_xyz(lab, math::D50);
        math::xyz_to_lab(std::array::from_fn(|i| a * xyz[i] + math::D50[i] * (1.0 - a)), math::D50)
    }

    fn perceptual(&self, lab: [f64; 3], warm: Option<[f64; 4]>) -> [f64; 4] {
        let lab = self.lift_black(lab);
        let ok = self.oklab(lab);
        let c = ok[1].hypot(ok[2]);
        if c < 1e-9 {
            return self.clip(lab, warm);
        }
        let Some((s, edge)) = self.boundary(ok, OKLAB_CHROMA_MAX / c, warm) else {
            return self.clip(lab, warm);
        };
        // Chroma above the knee is compressed smoothly into the boundary chroma `cb`.
        let cb = s * c;
        let knee = self.knee * cb;
        if c <= knee {
            return self.clip(lab, warm);
        }
        let c2 = knee + (cb - knee) * ((c - knee) / (cb - knee).max(1e-9)).tanh();
        let f = c2 / c;
        if f >= s {
            return edge;
        }
        self.clip(self.lab([ok[0], ok[1] * f, ok[2] * f]), Some(edge))
    }

    fn saturation(&self, lab: [f64; 3], warm: Option<[f64; 4]>) -> [f64; 4] {
        let ok = self.oklab(self.lift_black(lab));
        self.clip(self.lab([ok[0], ok[1] * self.saturation, ok[2] * self.saturation]), warm)
    }
}

fn lab_node_legacy(i: usize, n: usize) -> f64 {
    i as f64 / (n - 1) as f64
}

/// Decodes a legacy (lut16) Lab node coordinate triple.
fn decode_v2(v: [f64; 3]) -> [f64; 3] {
    [v[0] * 100.0 * 65535.0 / 65280.0, v[1] * 65535.0 / 256.0 - 128.0, v[2] * 65535.0 / 256.0 - 128.0]
}

fn encode_v2(lab: [f64; 3]) -> [f32; 3] {
    [
        (lab[0] / (100.0 * 65535.0 / 65280.0)).clamp(0.0, 1.0) as f32,
        ((lab[1] + 128.0) * 256.0 / 65535.0).clamp(0.0, 1.0) as f32,
        ((lab[2] + 128.0) * 256.0 / 65535.0).clamp(0.0, 1.0) as f32,
    ]
}

#[cfg(not(target_arch = "wasm32"))]
fn par_map<T: Send>(n: usize, f: impl Fn(usize) -> T + Sync + Send) -> Vec<T> {
    use rayon::prelude::*;
    (0..n).into_par_iter().map(f).collect()
}

#[cfg(target_arch = "wasm32")]
fn par_map<T: Send>(n: usize, f: impl Fn(usize) -> T + Sync + Send) -> Vec<T> {
    (0..n).map(f).collect()
}

/// Grid coordinate of a legacy-encoded PCS value through the [`CmykParams::white_on_node`]
/// input curves: L* 0–100 spans the L axis (higher L* clips to the top node) and a*/b* −128 to
/// +128 span theirs, so L* 100, a* = b* = 0 is the top centre node of an odd grid.
fn white_node_curve(l_axis: bool, x: f64) -> f64 {
    if l_axis { (x * 65535.0 / 65280.0).min(1.0) } else { x * 65535.0 / 65536.0 }
}

fn b2a_table<F: Forward + ?Sized>(model: &F, p: &CmykParams, gbd: Option<&Gbd>) -> Result<Clut, CmsError> {
    b2a_table_with(p, |lab, warm| {
        let target = match gbd {
            Some(g) => g.compress(lab),
            None => lab,
        };
        separate(model, p, target, warm).0
    })
}

/// Samples a BToA table: `node` separates each relative Lab node, warm-started from the
/// previous node along b.
fn b2a_table_with(p: &CmykParams, node: impl Fn([f64; 3], Option<[f64; 4]>) -> [f64; 4] + Sync) -> Result<Clut, CmsError> {
    let n = p.grid_b2a.max(2);
    let nodes = n * n * n;
    let step = (n - 1) as f64;
    let vals = par_map(n * n, |row| {
        // One (L, a) row along b, warm-starting each solve from its neighbour.
        let (li, ai) = (row / n, row % n);
        let mut warm = None;
        let mut out = Vec::with_capacity(n * 4);
        for bi in 0..n {
            let lab = if p.white_on_node {
                // The inverse of `white_node_curve` at the node.
                [100.0 * li as f64 / step, 256.0 * ai as f64 / step - 128.0, 256.0 * bi as f64 / step - 128.0]
            } else {
                decode_v2([lab_node_legacy(li, n), lab_node_legacy(ai, n), lab_node_legacy(bi, n)])
            };
            let cmyk = if p.white_on_node && lab == [100.0, 0.0, 0.0] { [0.0; 4] } else { node(lab, warm) };
            warm = Some(cmyk);
            out.extend(cmyk.map(|v| v as f32));
        }
        out
    });
    let data: Vec<f32> = vals.into_iter().flatten().collect();
    debug_assert_eq!(data.len(), nodes * 4);
    Clut::new(vec![n; 3], 4, data)
}

/// Refuses grids below 2 nodes or above what a [`Clut`] holds, before anything is sampled.
fn check_grids(p: &CmykParams) -> Result<(), CmsError> {
    let fits = |grid: usize, dims: u32, outputs: usize| {
        grid >= 2 && grid.checked_pow(dims).and_then(|n| n.checked_mul(outputs)).is_some_and(|n| n <= crate::clut::MAX_VALUES)
    };
    if !fits(p.grid_a2b, 4, 3) || !fits(p.grid_b2a, 3, 4) {
        return Err(invalid_arg(format!("CMYK profile grids {}⁴/{}³ need at least 2 nodes and must fit a CLUT", p.grid_a2b, p.grid_b2a)));
    }
    if p.white_on_node && p.grid_b2a.is_multiple_of(2) {
        return Err(invalid_arg(format!("white on a node needs an odd BToA grid, not {}", p.grid_b2a)));
    }
    if !(p.tac.is_finite() && p.tac >= 0.0) {
        return Err(invalid_arg(format!("total ink limit {} must be finite and not negative", p.tac)));
    }
    let valid = |v: &[f64; 2]| v.iter().all(|x| (0.0..=1.0).contains(x));
    if !p.gcr_curve.iter().all(valid) || p.gcr_curve.windows(2).any(|w| w[1][0] <= w[0][0]) {
        return Err(invalid_arg(format!("black generation curve {:?} needs points in [0, 1] with increasing gray components", p.gcr_curve)));
    }
    if let GamutMapping::Oklab { knee, saturation } = p.mapping
        && !((0.0..1.0).contains(&knee) && saturation.is_finite() && saturation > 0.0)
    {
        return Err(invalid_arg(format!("Oklab gamut mapping needs a knee in [0, 1) and a positive saturation, not {knee} and {saturation}")));
    }
    Ok(())
}

/// Input curves of a `lut16` table: identity, or the [`white_node_curve`]s for a BToA table.
fn lut16(clut: Clut, white_on_node: bool) -> Lut {
    let (i, o) = (clut.inputs, clut.outputs);
    let input = if white_on_node {
        // mft2's longest input tables; white lies between samples, within a u16 step of its node.
        let table = |l_axis: bool| Curve::Table((0..4096).map(|j| white_node_curve(l_axis, j as f64 / 4095.0) as f32).collect());
        vec![table(true), table(false), table(false)]
    } else {
        vec![Curve::Table(vec![0.0, 1.0]); i]
    };
    Lut {
        kind: LutKind::Lut16,
        inputs: i,
        outputs: o,
        stages: vec![Stage::Curves(input), Stage::Clut(clut), Stage::Curves(vec![Curve::Table(vec![0.0, 1.0]); o])],
    }
}

/// Generates a synthetic CMYK profile. Grids are refused as in [`cmyk_profile_from_model`].
pub fn cmyk_profile(p: &CmykParams) -> Result<Profile, CmsError> {
    check_grids(p)?;
    let model = CmykModel::coated(p.tvi);
    let a2b = sample_a2b(&model, p)?;
    lut_profile(&model, a2b, model.paper, p)
}

/// Samples a forward model into an AToB table with `p.grid_a2b`⁴ nodes and legacy Lab
/// encoding.
fn sample_a2b<F: Forward + ?Sized>(model: &F, p: &CmykParams) -> Result<Clut, CmsError> {
    Clut::sample(vec![p.grid_a2b.max(2); 4], 3, |inp, out| {
        let lab = if p.white_on_node && inp.iter().all(|v| *v == 0.0) {
            [100.0, 0.0, 0.0]
        } else {
            model.lab([inp[0] as f64, inp[1] as f64, inp[2] as f64, inp[3] as f64])
        };
        out.copy_from_slice(&encode_v2(lab));
    })
}

/// A CMYK profile of any forward model on paper with absolute XYZ `paper`: the AToB table
/// samples `model` on `p.grid_a2b`⁴ nodes, and the BToA tables invert that table rather than the
/// model, so Lab → CMYK → Lab round trips through the profile are as close as the separation
/// allows. Grids below 2 nodes or too large for a [`Clut`], and a zero, negative or
/// non-finite `paper`, are refused with [`CmsError::InvalidArgument`].
pub fn cmyk_profile_from_model<F: Forward + ?Sized>(model: &F, paper: [f64; 3], p: &CmykParams) -> Result<Profile, CmsError> {
    check_grids(p)?;
    if !paper.iter().all(|v| v.is_finite() && *v > 0.0) {
        return Err(invalid_arg(format!("paper XYZ {paper:?}")));
    }
    let a2b = sample_a2b(model, p)?;
    let forward = TableForward(a2b.clone());
    lut_profile(&forward, a2b, paper, p)
}

fn lut_profile<F: Forward + ?Sized>(model: &F, a2b: Clut, paper: [f64; 3], p: &CmykParams) -> Result<Profile, CmsError> {
    let (perceptual, colorimetric, saturation) = match p.mapping {
        GamutMapping::Legacy => {
            let gbd = Gbd::new(model, p);
            let colorimetric = b2a_table(model, p, None)?;
            let perceptual = b2a_table(model, p, Some(&gbd))?;
            (perceptual.clone(), colorimetric, perceptual)
        }
        GamutMapping::Oklab { knee, saturation } => {
            let m = OklabMapper::new(model, p, knee, saturation);
            let colorimetric = b2a_table_with(p, |lab, warm| m.clip(lab, warm))?;
            let perceptual = b2a_table_with(p, |lab, warm| m.perceptual(lab, warm))?;
            let saturation = b2a_table_with(p, |lab, warm| m.saturation(lab, warm))?;
            (perceptual, colorimetric, saturation)
        }
    };
    let a2b = lut16(a2b, false);
    let (b0, b1, b2) = (lut16(perceptual, p.white_on_node), lut16(colorimetric, p.white_on_node), lut16(saturation, p.white_on_node));
    Ok(Profile {
        version: (4, 0x30),
        class: ProfileClass::Output,
        color_space: ColorSpace::Cmyk,
        pcs: Pcs::Lab,
        rendering_intent: Intent::Perceptual,
        description: p.description.clone(),
        copyright: p.copyright.clone(),
        white_point: paper.map(math::s15f16_round),
        black_point: None,
        chad: None,
        matrix: None,
        trc: None,
        gray_trc: None,
        a2b: [Some(a2b.clone()), Some(a2b.clone()), Some(a2b)],
        b2a: [Some(b0), Some(b1), Some(b2)],
        bytes: None,
        hash: Default::default(),
    }
    .with_encoded_bytes())
}

/// The default coated profile (what `profiles/photocraft-coated-cmyk.icc` contains).
pub fn coated_cmyk() -> Result<Profile, CmsError> {
    cmyk_profile(&CmykParams::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oklab_mapping_refuses_bad_parameters() {
        for (knee, saturation) in [(1.0, 1.0), (-0.1, 1.0), (f64::NAN, 1.0), (0.5, 0.0), (0.5, f64::INFINITY), (0.5, f64::NAN)] {
            let p = CmykParams { grid_a2b: 2, grid_b2a: 2, mapping: GamutMapping::Oklab { knee, saturation }, ..CmykParams::default() };
            assert!(cmyk_profile(&p).is_err(), "knee {knee}, saturation {saturation}");
        }
    }

    #[test]
    fn black_generation_curve() {
        let p = CmykParams { gcr_curve: vec![[0.0, 0.0], [0.5, 0.2], [1.0, 0.9]], ..CmykParams::default() };
        for (g, k) in [(-1.0, 0.0), (0.0, 0.0), (0.25, 0.1), (0.5, 0.2), (0.75, 0.55), (1.0, 0.9), (2.0, 0.9)] {
            assert!((gcr(&p, g) - k).abs() < 1e-12, "g {g}: {} != {k}", gcr(&p, g));
        }
        for curve in [vec![[0.5, 0.2], [0.5, 0.3]], vec![[0.5, 0.2], [0.2, 0.3]], vec![[0.0, 1.5]], vec![[f64::NAN, 0.0]]] {
            let p = CmykParams { grid_a2b: 2, grid_b2a: 2, gcr_curve: curve.clone(), ..CmykParams::default() };
            assert!(cmyk_profile(&p).is_err(), "{curve:?}");
        }
    }

    #[test]
    fn total_ink_limit_must_be_finite_and_not_negative() {
        for tac in [-0.5, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let p = CmykParams { grid_a2b: 2, grid_b2a: 2, tac, ..CmykParams::default() };
            assert!(cmyk_profile(&p).is_err(), "tac {tac}");
        }
        // `separate` is public and does not validate: black is still never negative.
        let p = CmykParams { tac: -0.5, gcr_curve: vec![[0.0, 0.75]], ..CmykParams::default() };
        assert_eq!(gcr(&p, 0.5), 0.0);
    }

    #[test]
    fn curve_black_stays_within_the_ink_limit() {
        // Black from either the profile curve or the default empty-curve fallback never exceeds the ink limit.
        let m = CmykModel::coated(CmykParams::default().tvi);
        for gcr_curve in [vec![[0.0, 0.75]], vec![]] {
            for tac in [0.5, 0.0] {
                let p = CmykParams { tac, gcr_curve: gcr_curve.clone(), ..CmykParams::default() };
                assert!(gcr(&p, 1.0) <= p.tac, "tac {tac} curve {gcr_curve:?}");
                for target in [[50.0, 0.0, 0.0], [90.0, 0.0, 0.0], [30.0, 20.0, -10.0]] {
                    let (cmyk, _) = separate(&m, &p, target, None);
                    assert!(cmyk.iter().sum::<f64>() <= p.tac + 1e-6, "{target:?} -> {cmyk:?}");
                }
            }
        }
    }

    #[test]
    fn oklab_round_trips() {
        let m = CmykModel::coated(CmykParams::default().tvi);
        let p = CmykParams::default();
        let mapper = OklabMapper::new(&m, &p, 0.65, 1.05);
        for lab in [[50.0, 0.0, 0.0], [44.0, 63.7, 47.0], [31.9, 32.9, -76.8], [90.0, -5.0, 80.0]] {
            let back = mapper.lab(mapper.oklab(lab));
            // The published matrices carry 10 digits, so the inverses are exact to about 1e-5 ΔE.
            assert!(math::delta_e76(lab, back) < 1e-3, "{lab:?} -> {back:?}");
        }
    }

    #[test]
    fn oklab_profile_has_three_tables() {
        let p = CmykParams { grid_a2b: 5, grid_b2a: 5, mapping: GamutMapping::Oklab { knee: 0.65, saturation: 1.05 }, ..CmykParams::default() };
        let profile = cmyk_profile(&p).unwrap();
        let [Some(b0), Some(b1), Some(b2)] = &profile.b2a else { panic!("missing BToA tables") };
        assert!(b0 != b1 && b0 != b2, "perceptual shares a table");
    }
}
