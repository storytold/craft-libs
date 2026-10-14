//! The Coated FOGRA39 built-in profile: accuracy against Fogra's FOGRA39 characterisation data
//! (`tests/data/FOGRA39.txt`), conversions to sRGB against reference values, and its generator:
//!
//! ```sh
//! CRAFT_CMS_REGEN_PROFILES=1 cargo test -p craft-cms --release --test fogra39
//! ```
//!
//! The generator fits a Yule–Nielsen modified Neugebauer model (free 16 overprints, per-ink
//! piecewise-linear tone curves, free n) to all 1617 patches by least squares, corrects the
//! remaining error with a Gaussian-kernel ridge regression of the Lab residuals over CMYK, and
//! samples the result into the AToB table. The BToA tables invert that table
//! ([`synth::cmyk_profile_from_model`]).

use craft_cms::math::{self, delta_e2000};
use craft_cms::synth::{self, CmykParams, Forward};
use craft_cms::{Builtin, Intent, Profile, Transform};

const DATA: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/FOGRA39.txt");
const PROFILE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/profiles/craft-coated-fogra39.icc");

#[derive(Clone)]
struct Patch {
    cmyk: [f64; 4],
    /// Absolute Lab (D50, 2°) as measured.
    lab: [f64; 3],
}

/// The CGATS patches: `SAMPLE_ID CMYK_C CMYK_M CMYK_Y CMYK_K XYZ_X XYZ_Y XYZ_Z LAB_L LAB_A LAB_B`.
fn patches() -> Vec<Patch> {
    let text = std::fs::read_to_string(DATA).unwrap();
    let mut lines = text.lines().map(str::trim);
    let fields: Vec<&str> = lines.by_ref().skip_while(|l| *l != "BEGIN_DATA_FORMAT").nth(1).unwrap().split_whitespace().collect();
    let col = |name: &str| fields.iter().position(|f| *f == name).unwrap();
    let cols = ["CMYK_C", "CMYK_M", "CMYK_Y", "CMYK_K", "LAB_L", "LAB_A", "LAB_B"].map(col);
    let out: Vec<Patch> = lines
        .skip_while(|l| *l != "BEGIN_DATA")
        .skip(1)
        .take_while(|l| *l != "END_DATA")
        .map(|l| {
            let v: Vec<f64> = l.split_whitespace().map(|t| t.parse().unwrap()).collect();
            Patch { cmyk: std::array::from_fn(|i| v[cols[i]] / 100.0), lab: std::array::from_fn(|i| v[cols[4 + i]]) }
        })
        .collect();
    assert_eq!(out.len(), 1617);
    out
}

/// Absolute XYZ of the paper (the 0/0/0/0 patch).
fn paper(p: &[Patch]) -> [f64; 3] {
    let w = p.iter().find(|p| p.cmyk == [0.0; 4]).unwrap();
    math::lab_to_xyz(w.lab, math::D50)
}

/// ICC-relative Lab (paper → L* 100) of absolute XYZ.
fn relative(xyz: [f64; 3], paper: [f64; 3]) -> [f64; 3] {
    math::xyz_to_lab(std::array::from_fn(|i| xyz[i] * math::D50[i] / paper[i]), math::D50)
}

/// The measured colours as ICC-relative Lab.
fn relative_targets(p: &[Patch]) -> Vec<[f64; 3]> {
    let w = paper(p);
    p.iter().map(|p| relative(math::lab_to_xyz(p.lab, math::D50), w)).collect()
}

struct Stats {
    mean: f64,
    p95: f64,
    max: f64,
    worst: [f64; 4],
}

fn stats(p: &[Patch], lab_of: impl Fn([f64; 4]) -> [f64; 3]) -> Stats {
    let want = relative_targets(p);
    let mut de: Vec<(f64, [f64; 4])> = p.iter().zip(&want).map(|(p, w)| (delta_e2000(lab_of(p.cmyk), *w), p.cmyk)).collect();
    let mean = de.iter().map(|d| d.0).sum::<f64>() / de.len() as f64;
    de.sort_by(|a, b| a.0.total_cmp(&b.0));
    let p95 = de[(de.len() * 95).div_ceil(100) - 1].0;
    let (max, worst) = de[de.len() - 1];
    Stats { mean, p95, max, worst }
}

/// Relative Lab through a profile's colorimetric AToB.
fn profile_lab(profile: &Profile) -> impl Fn([f64; 4]) -> [f64; 3] {
    let t = Transform::new(profile, Builtin::LabD50.profile(), Intent::RelativeColorimetric, false).unwrap();
    move |c| {
        let mut o = [0.0f32; 3];
        t.eval(&c.map(|v| v as f32), &mut o).unwrap();
        [o[0] as f64 * 100.0, o[1] as f64 * 255.0 - 128.0, o[2] as f64 * 255.0 - 128.0]
    }
}

// --- Fit ------------------------------------------------------------------------------------

/// Tone curve knots at 10 %, 20 % … 90 % coverage.
const KNOTS: usize = 9;
const PARAMS: usize = 1 + 48 + 4 * KNOTS;

/// Yule–Nielsen modified Neugebauer model: XYZ = (Σ Demichel weight · Pᵢ^(1/n))ⁿ over the 16
/// overprints, with effective coverages from per-ink piecewise-linear tone curves.
#[derive(Clone)]
struct Model {
    n: f64,
    /// Overprint XYZ^(1/n), indexed by ink bit mask (C=1, M=2, Y=4, K=8).
    prim_n: [[f64; 3]; 16],
    tone: [[f64; KNOTS]; 4],
}

impl Model {
    fn initial(p: &[Patch]) -> Model {
        let n = 2.0;
        let prim_n = std::array::from_fn(|m| {
            let cmyk: [f64; 4] = std::array::from_fn(|i| if m & (1 << i) != 0 { 1.0 } else { 0.0 });
            let solid = p.iter().find(|p| p.cmyk == cmyk).unwrap();
            math::lab_to_xyz(solid.lab, math::D50).map(|v| v.powf(1.0 / n))
        });
        let tone = std::array::from_fn(|_| {
            std::array::from_fn(|k| {
                let v = (k + 1) as f64 / 10.0;
                v + 0.6 * v * (1.0 - v)
            })
        });
        Model { n, prim_n, tone }
    }

    fn coverage(t: &[f64; KNOTS], v: f64) -> f64 {
        let p = v.clamp(0.0, 1.0) * (KNOTS + 1) as f64;
        let i = (p.floor() as usize).min(KNOTS);
        let at = |k: usize| match k {
            0 => 0.0,
            k if k > KNOTS => 1.0,
            k => t[k - 1],
        };
        at(i) + (at(i + 1) - at(i)) * (p - i as f64)
    }

    fn xyz(&self, cmyk: [f64; 4]) -> [f64; 3] {
        let a: [f64; 4] = std::array::from_fn(|i| Model::coverage(&self.tone[i], cmyk[i]));
        let mut s = [0.0; 3];
        for (m, q) in self.prim_n.iter().enumerate() {
            let w: f64 = (0..4).map(|i| if m & (1 << i) != 0 { a[i] } else { 1.0 - a[i] }).product();
            for (acc, v) in s.iter_mut().zip(q) {
                *acc += w * v;
            }
        }
        s.map(|v| v.max(0.0).powf(self.n))
    }

    fn params(&self) -> Vec<f64> {
        let mut v = vec![self.n];
        v.extend(self.prim_n.iter().flatten());
        v.extend(self.tone.iter().flatten());
        v
    }

    fn from_params(v: &[f64]) -> Model {
        Model {
            n: v[0],
            prim_n: std::array::from_fn(|m| std::array::from_fn(|i| v[1 + m * 3 + i])),
            tone: std::array::from_fn(|c| std::array::from_fn(|k| v[49 + c * KNOTS + k])),
        }
    }
}

/// Solves `a x = b` for symmetric positive definite `a` (row-major `n × n`) and `cols`
/// right-hand sides (`b` row-major `n × cols`), in place in `b`.
fn cholesky_solve(a: &mut [f64], n: usize, b: &mut [f64], cols: usize) {
    for j in 0..n {
        let mut d = a[j * n + j];
        for k in 0..j {
            d -= a[j * n + k] * a[j * n + k];
        }
        assert!(d > 0.0, "matrix is not positive definite");
        let d = d.sqrt();
        a[j * n + j] = d;
        for i in j + 1..n {
            let mut s = a[i * n + j];
            for k in 0..j {
                s -= a[i * n + k] * a[j * n + k];
            }
            a[i * n + j] = s / d;
        }
    }
    for c in 0..cols {
        for i in 0..n {
            let mut s = b[i * cols + c];
            for k in 0..i {
                s -= a[i * n + k] * b[k * cols + c];
            }
            b[i * cols + c] = s / a[i * n + i];
        }
        for i in (0..n).rev() {
            let mut s = b[i * cols + c];
            for k in i + 1..n {
                s -= a[k * n + i] * b[k * cols + c];
            }
            b[i * cols + c] = s / a[i * n + i];
        }
    }
}

/// Levenberg–Marquardt on the absolute Lab error of every patch.
fn fit_model(p: &[Patch]) -> Model {
    let resid = |x: &[f64]| -> Vec<f64> {
        let m = Model::from_params(x);
        p.iter()
            .flat_map(|p| {
                let l = math::xyz_to_lab(m.xyz(p.cmyk), math::D50);
                [l[0] - p.lab[0], l[1] - p.lab[1], l[2] - p.lab[2]]
            })
            .collect()
    };
    let cost = |r: &[f64]| r.iter().map(|v| v * v).sum::<f64>();
    let mut x = Model::initial(p).params();
    let mut r = resid(&x);
    let mut f = cost(&r);
    let mut lambda = 1e-3;
    for _ in 0..200 {
        let jac: Vec<Vec<f64>> = (0..PARAMS)
            .map(|j| {
                let h = 1e-6 * x[j].abs().max(1e-2);
                let mut xp = x.clone();
                xp[j] += h;
                resid(&xp).iter().zip(&r).map(|(a, b)| (a - b) / h).collect()
            })
            .collect();
        let mut jtj = vec![0.0; PARAMS * PARAMS];
        let mut g = vec![0.0; PARAMS];
        for a in 0..PARAMS {
            for b in a..PARAMS {
                let s: f64 = jac[a].iter().zip(&jac[b]).map(|(u, v)| u * v).sum();
                jtj[a * PARAMS + b] = s;
                jtj[b * PARAMS + a] = s;
            }
            g[a] = -jac[a].iter().zip(&r).map(|(u, v)| u * v).sum::<f64>();
        }
        let mut improved = false;
        for _ in 0..10 {
            let mut m = jtj.clone();
            for d in 0..PARAMS {
                m[d * PARAMS + d] += lambda * (1.0 + jtj[d * PARAMS + d]);
            }
            let mut step = g.clone();
            cholesky_solve(&mut m, PARAMS, &mut step, 1);
            let xn: Vec<f64> = x.iter().zip(&step).map(|(a, b)| a + b).collect();
            let rn = resid(&xn);
            let fnew = cost(&rn);
            if fnew < f {
                improved = (f - fnew) > 1e-9 * f;
                (x, r, f) = (xn, rn, fnew);
                lambda = (lambda * 0.3).max(1e-12);
                break;
            }
            lambda *= 10.0;
        }
        if !improved {
            break;
        }
    }
    Model::from_params(&x)
}

/// Kernel ridge regression of Lab residuals over CMYK with a Gaussian kernel.
struct Correction {
    centers: Vec<[f64; 4]>,
    weights: Vec<[f64; 3]>,
    inv_2s2: f64,
}

impl Correction {
    fn fit(centers: Vec<[f64; 4]>, residuals: &[[f64; 3]], sigma: f64, lambda: f64) -> Correction {
        let n = centers.len();
        let inv_2s2 = 1.0 / (2.0 * sigma * sigma);
        let mut k = vec![0.0; n * n];
        for i in 0..n {
            for j in 0..n {
                k[i * n + j] = kernel(centers[i], centers[j], inv_2s2) + if i == j { lambda } else { 0.0 };
            }
        }
        let mut b: Vec<f64> = residuals.iter().flatten().copied().collect();
        cholesky_solve(&mut k, n, &mut b, 3);
        let weights = b.chunks(3).map(|c| [c[0], c[1], c[2]]).collect();
        Correction { centers, weights, inv_2s2 }
    }

    fn eval(&self, x: [f64; 4]) -> [f64; 3] {
        let mut s = [0.0; 3];
        for (c, w) in self.centers.iter().zip(&self.weights) {
            let k = kernel(*c, x, self.inv_2s2);
            for i in 0..3 {
                s[i] += k * w[i];
            }
        }
        s
    }
}

fn kernel(a: [f64; 4], b: [f64; 4], inv_2s2: f64) -> f64 {
    let d2: f64 = (0..4).map(|i| (a[i] - b[i]).powi(2)).sum();
    (-d2 * inv_2s2).exp()
}

/// The fitted forward model: Neugebauer model plus residual correction, ICC-relative Lab.
struct Fitted {
    model: Model,
    paper: [f64; 3],
    correction: Correction,
}

impl Forward for Fitted {
    fn lab(&self, cmyk: [f64; 4]) -> [f64; 3] {
        let l = relative(self.model.xyz(cmyk), self.paper);
        let c = self.correction.eval(cmyk);
        [l[0] + c[0], l[1] + c[1], l[2] + c[2]]
    }
}

/// Fits the model and its correction (σ = 0.2, λ = 0.01) to the patches `p`, which include the
/// paper patch.
fn fitted(p: &[Patch]) -> Fitted {
    let model = fit_model(p);
    let paper = model.xyz([0.0; 4]);
    let want = relative_targets(p);
    let residuals: Vec<[f64; 3]> = p
        .iter()
        .zip(&want)
        .map(|(p, w)| {
            let l = relative(model.xyz(p.cmyk), paper);
            [w[0] - l[0], w[1] - l[1], w[2] - l[2]]
        })
        .collect();
    let correction = Correction::fit(p.iter().map(|p| p.cmyk).collect(), &residuals, 0.2, 0.01);
    Fitted { model, paper, correction }
}

#[test]
fn regen_coated_fogra39() {
    if std::env::var_os("CRAFT_CMS_REGEN_PROFILES").is_none() {
        return;
    }
    let t = std::time::Instant::now();
    let p = patches();
    let f = fitted(&p);
    let s = stats(&p, |c| relative(f.model.xyz(c), f.paper));
    eprintln!("model n {:.3}: mean ΔE00 {:.2} p95 {:.2} max {:.2} at {:?}", f.model.n, s.mean, s.p95, s.max, s.worst);
    let params = CmykParams {
        description: Builtin::CoatedFogra39.description().into(),
        copyright: craft_cms::builtin::COATED_FOGRA39_COPYRIGHT.into(),
        tac: 3.3,
        grid_b2a: 33,
        white_on_node: true,
        ..CmykParams::default()
    };
    let profile = synth::cmyk_profile_from_model(&f, paper(&p), &params).unwrap();
    let bytes = profile.to_bytes();
    std::fs::write(PROFILE, &*bytes).unwrap();
    eprintln!("wrote {} bytes in {:?}", bytes.len(), t.elapsed());
}

fn fogra39() -> &'static Profile {
    let p = Builtin::CoatedFogra39.profile();
    assert_eq!(p.description, Builtin::CoatedFogra39.description(), "the shipped profile parses");
    p
}

#[test]
fn fit_predicts_held_out_patches() {
    // Fit to 90 % of the patches, then compare the model with the other 10 %. The 16 solid
    // overprints stay in the fit, which starts from them.
    let all = patches();
    let held = |i: usize, p: &Patch| i % 10 == 3 && !p.cmyk.iter().all(|v| *v == 0.0 || *v == 1.0);
    let train: Vec<Patch> = all.iter().enumerate().filter(|(i, p)| !held(*i, p)).map(|(_, p)| p.clone()).collect();
    let test: Vec<Patch> = all.iter().enumerate().filter(|(i, p)| held(*i, p)).map(|(_, p)| p.clone()).collect();
    let f = fitted(&train);
    let w = paper(&all);
    let mut de: Vec<f64> = test.iter().map(|p| delta_e2000(f.lab(p.cmyk), relative(math::lab_to_xyz(p.lab, math::D50), w))).collect();
    de.sort_by(f64::total_cmp);
    let mean = de.iter().sum::<f64>() / de.len() as f64;
    eprintln!("hold-out ({} patches): mean ΔE00 {mean:.2} p95 {:.2} max {:.2}", de.len(), de[de.len() * 95 / 100], de[de.len() - 1]);
    assert!(mean <= 0.2, "hold-out mean {mean}");
}

#[test]
fn model_profiles_refuse_bad_parameters() {
    let paper = math::lab_to_xyz([95.0, 0.0, -2.0], math::D50);
    for (a2b, b2a) in [(0, 21), (1, 21), (11, 0), (11, 1)] {
        let params = CmykParams { grid_a2b: a2b, grid_b2a: b2a, ..CmykParams::default() };
        assert!(synth::cmyk_profile_from_model(&Flat, paper, &params).is_err(), "{a2b} {b2a}");
    }
    let small = CmykParams { grid_a2b: 2, grid_b2a: 2, ..CmykParams::default() };
    assert!(synth::cmyk_profile_from_model(&Flat, [0.0, 1.0, 1.0], &small).is_err());
    assert!(synth::cmyk_profile_from_model(&Flat, [f64::NAN, 1.0, 1.0], &small).is_err());
    assert!(synth::cmyk_profile_from_model(&Flat, paper, &small).is_ok());
}

/// Paper white everywhere.
struct Flat;

impl Forward for Flat {
    fn lab(&self, _: [f64; 4]) -> [f64; 3] {
        [100.0, 0.0, 0.0]
    }
}

#[test]
fn fits_the_fogra39_data() {
    let p = patches();
    let s = stats(&p, profile_lab(fogra39()));
    eprintln!("Coated FOGRA39 builtin: mean ΔE00 {:.2} p95 {:.2} max {:.2} at {:?}", s.mean, s.p95, s.max, s.worst);
    assert!(s.mean <= 0.15 && s.p95 <= 0.35 && s.max <= 0.8, "mean {} p95 {} max {}", s.mean, s.p95, s.max);
    let s = stats(&p, profile_lab(Builtin::CoatedCmyk.profile()));
    eprintln!("Photocraft coated builtin: mean ΔE00 {:.2} p95 {:.2} max {:.2} at {:?}", s.mean, s.p95, s.max, s.worst);
}

#[test]
fn darkens_with_every_ink() {
    let lab = profile_lab(fogra39());
    for base in [[0.0; 4], [0.2, 0.2, 0.2, 0.0], [0.6, 0.4, 0.4, 0.2], [0.0, 0.5, 1.0, 0.0], [0.8, 0.7, 0.7, 0.5]] {
        for ink in 0..4 {
            let mut prev = f64::INFINITY;
            for i in 0..=40 {
                let mut v = base;
                v[ink] = i as f64 / 40.0;
                let l = lab(v)[0];
                assert!(l < prev + 0.2, "{base:?} ink {ink} at {}: L* {l} after {prev}", v[ink]);
                prev = l;
            }
        }
    }
}

#[test]
fn in_gamut_lab_round_trips() {
    let profile = fogra39();
    let lab = Builtin::LabD50.profile();
    let to_cmyk = Transform::new(lab, profile, Intent::RelativeColorimetric, false).unwrap();
    let to_lab = profile_lab(profile);
    // Targets: the profile's own colours of the patches inside the gamut (no solid ink, well
    // within the 330 % ink limit). Gamut-edge colours with black (e.g. C100 K100) come back
    // up to ~7 ΔE00 off, as with the Photocraft profile: black generation fixes K before the
    // CMY solve.
    let mut de: Vec<f64> = patches()
        .iter()
        .filter(|p| p.cmyk.iter().all(|v| *v <= 0.85) && p.cmyk.iter().sum::<f64>() <= 2.8)
        .map(|p| {
            let target = to_lab(p.cmyk);
            let enc = [(target[0] / 100.0) as f32, ((target[1] + 128.0) / 255.0) as f32, ((target[2] + 128.0) / 255.0) as f32];
            let mut c = [0.0f32; 4];
            to_cmyk.eval(&enc, &mut c).unwrap();
            assert!(c.iter().map(|v| *v as f64).sum::<f64>() <= 3.3 + 0.02, "{target:?} -> {c:?}");
            delta_e2000(to_lab(c.map(f64::from)), target)
        })
        .collect();
    de.sort_by(f64::total_cmp);
    let mean = de.iter().sum::<f64>() / de.len() as f64;
    let (p95, max) = (de[de.len() * 95 / 100], de[de.len() - 1]);
    eprintln!("Lab → CMYK → Lab over {} colours: mean ΔE00 {mean:.2} p95 {p95:.2} max {max:.2}", de.len());
    assert!(mean < 0.5 && p95 < 1.5 && max < 3.0, "mean {mean} p95 {p95} max {max}");
}

/// Reference sRGB values for CMYK in Coated FOGRA39 (relative colorimetric with black point
/// compensation).
const REFERENCE_SRGB: [([f64; 4], [u8; 3]); 15] = [
    ([0.0, 0.5, 1.0, 0.0], [243, 146, 0]),
    ([0.0, 0.6, 1.0, 0.0], [239, 125, 0]),
    ([1.0, 0.0, 0.0, 0.0], [0, 159, 227]),
    ([0.0, 1.0, 0.0, 0.0], [229, 0, 125]),
    ([0.0, 0.0, 1.0, 0.0], [255, 237, 0]),
    ([0.0, 0.0, 0.0, 1.0], [29, 29, 27]),
    ([1.0, 1.0, 0.0, 0.0], [49, 39, 131]),
    ([0.0, 1.0, 1.0, 0.0], [227, 6, 19]),
    // Reported DesignCraft orange: shown as 245,144,25 by the uncalibrated formula.
    ([0.0, 0.41, 0.9, 0.04], [239, 161, 35]),
    // Grey, rich black, tint, skin tone, green and a near-neutral.
    ([0.0, 0.0, 0.0, 0.5], [157, 157, 156]),
    ([0.6, 0.4, 0.4, 1.0], [2, 10, 10]),
    ([0.1, 0.05, 0.0, 0.0], [233, 238, 249]),
    ([0.05, 0.3, 0.4, 0.0], [241, 192, 157]),
    ([1.0, 0.0, 1.0, 0.0], [0, 150, 64]),
    ([0.4, 0.3, 0.3, 0.1], [157, 158, 159]),
];

fn to_srgb(profile: &Profile) -> impl Fn([f64; 4]) -> [u8; 3] {
    let t = Transform::new(profile, Builtin::Srgb.profile(), Intent::RelativeColorimetric, true).unwrap();
    move |c| {
        let mut o = [0.0f32; 3];
        t.eval(&c.map(|v| v as f32), &mut o).unwrap();
        o.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
    }
}

fn srgb_lab(rgb: [u8; 3]) -> [f64; 3] {
    let t = Transform::new(Builtin::Srgb.profile(), Builtin::LabD50.profile(), Intent::RelativeColorimetric, false).unwrap();
    let mut o = [0.0f32; 3];
    t.eval(&rgb.map(|v| v as f32 / 255.0), &mut o).unwrap();
    [o[0] as f64 * 100.0, o[1] as f64 * 255.0 - 128.0, o[2] as f64 * 255.0 - 128.0]
}

#[test]
fn matches_reference_srgb_values() {
    let ours = to_srgb(fogra39());
    let photocraft = to_srgb(Builtin::CoatedCmyk.profile());
    let mut worst = 0.0f64;
    for (cmyk, want) in REFERENCE_SRGB {
        let (got, pc) = (ours(cmyk), photocraft(cmyk));
        let de = delta_e2000(srgb_lab(got), srgb_lab(want));
        let de_pc = delta_e2000(srgb_lab(pc), srgb_lab(want));
        eprintln!("{cmyk:?}: reference {want:?} | FOGRA39 {got:?} ΔE00 {de:.2} | Photocraft {pc:?} ΔE00 {de_pc:.2}");
        worst = worst.max(de);
    }
    assert!(worst <= 1.0, "worst ΔE00 {worst}");
}

#[test]
fn builtins_by_document_profile_name() {
    for b in Builtin::ALL {
        assert_eq!(Builtin::from_profile_name(b.description()), Some(b), "{b:?}");
        assert_eq!(Builtin::stand_in_for(b.description()), Some(b), "{b:?}");
    }
    assert_eq!(Builtin::from_profile_name(" srgb iec61966-2.1 "), Some(Builtin::Srgb));
    assert_eq!(Builtin::from_profile_name("coated fogra39 (FITTED, craft-cms)"), Some(Builtin::CoatedFogra39));
    for name in FOGRA39_STAND_IN_NAMES.into_iter().chain(["coated  FOGRA39 (iso 12647-2:2004)", "iso coated v2(eci)"]) {
        assert_eq!(Builtin::from_profile_name(name), None, "{name}");
        assert_eq!(Builtin::stand_in_for(name), Some(Builtin::CoatedFogra39), "{name}");
    }
    // The built-in separates to up to 330 % total ink, more than this profile allows.
    for name in ["ISO Coated v2 300% (ECI)", "iso coated  V2 300 % (eci)", "U.S. Web Coated (SWOP) v2", "Coated FOGRA51 (PSO)", "PSO Coated v3", ""] {
        assert_eq!(Builtin::from_profile_name(name), None, "{name}");
        assert_eq!(Builtin::stand_in_for(name), None, "{name}");
    }
}

/// Names [`Builtin::stand_in_for`] maps to [`Builtin::CoatedFogra39`] (all 330 % profiles).
const FOGRA39_STAND_IN_NAMES: [&str; 3] = ["Coated FOGRA39 (ISO 12647-2:2004)", "Coated FOGRA39", "ISO Coated v2 (ECI)"];

/// Total ink of every separation through a stand-in stays within the named profiles' 330 %
/// limit: a 33³ sRGB grid, with BPC, for each intent the profile has tables for.
#[test]
fn stand_in_separations_respect_the_ink_limit() {
    const N: usize = 33;
    const LIMIT: f32 = 3.30 + 0.005;
    let grid: Vec<f32> = (0..N * N * N).flat_map(|i| [i / (N * N), i / N % N, i % N].map(|v| v as f32 / (N - 1) as f32)).collect();
    for name in FOGRA39_STAND_IN_NAMES {
        let cmyk = Builtin::stand_in_for(name).unwrap().profile();
        for intent in [Intent::Perceptual, Intent::RelativeColorimetric, Intent::Saturation] {
            let t = Transform::new(Builtin::Srgb.profile(), cmyk, intent, true).unwrap();
            let mut out = vec![0.0f32; N * N * N * 4];
            t.convert_f32(&grid, 3, &mut out, 4, false).unwrap();
            let (i, max) = out
                .as_chunks::<4>()
                .0
                .iter()
                .map(|c| c.iter().sum::<f32>())
                .enumerate()
                .fold((0, 0.0f32), |m, (i, s)| if s > m.1 { (i, s) } else { m });
            eprintln!("{name}, {intent:?}: max total ink {:.2} % at sRGB {:?}", max * 100.0, &grid[i * 3..i * 3 + 3]);
            assert!(max <= LIMIT, "{name}, {intent:?}: total ink {max} at sRGB {:?}", &grid[i * 3..i * 3 + 3]);

            // sRGB black, the darkest separation (reviewed with perceptual: 329.75 %).
            let mut black = [0.0f32; 4];
            t.eval(&[0.0, 0.0, 0.0], &mut black).unwrap();
            let total: f32 = black.iter().sum();
            eprintln!("{name}, {intent:?}: sRGB black → {black:?}, total {:.2} %", total * 100.0);
            assert!(total <= LIMIT, "{name}, {intent:?}: sRGB black total ink {total}");
        }
    }
}
