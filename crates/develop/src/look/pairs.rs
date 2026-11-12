use super::hue_sat::{HueSat, fit_chroma, fit_hue_sat};
use super::proxy::DARK_FLOOR;
#[cfg(test)]
use super::tone::highlights;
use super::tone::{HighlightCheck, ToneFit, highlights_hold};
use super::{CameraLook, CameraTone, HsvTable, Mat3, Rgb32f, ToneMap, luminance_2020};
#[allow(unused_imports)]
use crate::sensor::prelude::*;

/// A fit must cut the held-out squared error to below this share of the fallback's.
pub(super) const MIN_IMPROVEMENT: f64 = 0.7;

/// ...and stay within this per-channel RMS of the camera JPEG (linear display values). On public
/// raw.pixls.us samples (eight Sony bodies) good fits that still beat the fallback 1.5–3× landed
/// at 0.065–0.093 (camera local tone, vignetting and lens processing that a global matrix + curve
/// cannot follow): visibly better renders that 0.055 rejected.
const MAX_HOLDOUT_RMS: f64 = 0.10;

pub(super) fn luma(p: [f64; 3]) -> f64 {
    p[0] * 0.2627 + p[1] * 0.6780 + p[2] * 0.0593
}

/// The finish stage's tone map and chroma curve (`crate::finish`).
pub(super) fn displayed(scene: [f64; 3], tone: &ToneMap) -> [f64; 3] {
    let scene = scene.map(|v| v.max(0.0));
    let y = luma(scene);
    if y <= 0.0 {
        return [0.0; 3];
    }
    let o = f64::from(tone.apply(y as f32));
    let k = f64::from(tone.chroma_scale(o as f32));
    scene.map(|v| o + (v * o / y - o) * k)
}

#[cfg(test)]
pub(super) fn fit_pairs(sensor: &Rgb32f, reference: &Rgb32f) -> Option<CameraLook> {
    fit_pairs_with(sensor, reference, None)
}

/// Largest luminance ratio within the 3×3 neighbourhood of pixel `i` (1 on flat areas, large on edges).
pub(super) fn local_contrast(image: &Rgb32f, i: usize) -> f32 {
    let (w, h) = (image.width, image.height);
    if w == 0 {
        return f32::INFINITY;
    }
    let (x, y) = (i % w, i / w);
    let (mut lo, mut hi) = (f32::INFINITY, 0f32);
    for row in y.saturating_sub(1)..(y + 2).min(h) {
        for col in x.saturating_sub(1)..(x + 2).min(w) {
            let l = image.data.get(row * w + col).map_or(0.0, |p| luminance_2020(*p));
            lo = lo.min(l);
            hi = hi.max(l);
        }
    }
    if hi.is_finite() { hi / lo.max(1e-4) } else { f32::INFINITY }
}

/// Training pairs (sensor → JPEG, unclipped midtones) and the wider set including highlights
/// (for the chroma curve); `None` when too few, or the photo has too little colour. With
/// `edge_limit`, pixels whose 3×3 neighbourhood in either proxy spans a larger luminance ratio
/// are left out: there a small geometric mismatch between the raw and its JPEG pairs unrelated
/// colours (see [`EDGE_CONTRAST`]).
pub(super) type Pair = ([f64; 3], [f64; 3]);

pub(super) type Pairs = Vec<Pair>;

pub(super) fn collect_pairs(sensor: &Rgb32f, reference: &Rgb32f, min_chroma: f32, edge_limit: Option<f32>, dark: bool) -> Option<(Pairs, Pairs)> {
    if (sensor.width, sensor.height) != (reference.width, reference.height) || sensor.data.len() != reference.data.len() {
        return None;
    }
    let mut pairs = Vec::new();
    // Highlights too (camera JPEGs bleach colours toward white there), for the chroma curve only.
    let mut bright = Vec::new();
    let mut colour = 0;
    for (i, (input, output)) in sensor.data.iter().zip(&reference.data).enumerate() {
        if edge_limit.is_some_and(|limit| local_contrast(sensor, i) > limit || local_contrast(reference, i) > limit) {
            continue;
        }
        let y = luminance_2020(*output);
        if !input.iter().all(|v| v.is_finite() && *v > 0.001 && *v < 1.5) || !output.iter().all(|v| v.is_finite() && *v >= 0.0) {
            continue;
        }
        if (0.015..=1.0).contains(&y) {
            bright.push((input.map(f64::from), output.map(f64::from)));
        }
        let floors = if dark { DARK_FLOOR } else { (0.015, 0.004) };
        if !colour_pair_above(*input, *output, floors) {
            continue;
        }
        let min = output.iter().copied().fold(f32::INFINITY, f32::min);
        let max = output.iter().copied().fold(0.0, f32::max);
        colour += usize::from(max - min > min_chroma);
        pairs.push((input.map(f64::from), output.map(f64::from)));
    }
    (pairs.len() >= 256 && colour >= pairs.len() / 20).then_some((pairs, bright))
}

/// The original colour-pair eligibility, also used to preserve its held-out split in tone fitting.
pub(super) fn colour_pair(input: [f32; 3], output: [f32; 3]) -> bool {
    colour_pair_above(input, output, (0.015, 0.004))
}

/// [`colour_pair`] with its lowest preview luminance and channel value given (`floors`: the usual 0.015 and 0.004, or
/// [`DARK_FLOOR`] for the tone-only fit of a DNG).
fn colour_pair_above(input: [f32; 3], output: [f32; 3], (ylo, clo): (f32, f32)) -> bool {
    input.iter().all(|v| v.is_finite() && *v > 0.001 && *v < 1.5)
        && output.iter().all(|v| *v > clo && *v < 0.98)
        && (ylo..0.85).contains(&luminance_2020(output))
}

/// Fit tone independently of colour eligibility when enough dark JPEG signal is available.
/// The original pairs and their split still own the colour fit and its acceptance gates.
/// Luminance needs no channel ratios, so the binned proxy can use a lower sensor floor and
/// keep JPEG pixels with a channel near black. Quantile medians limit the effect of noise.
pub(super) fn fit_tone_with_shadows(
    fit: ToneFit,
    pairs: &[([f64; 3], [f64; 3])],
    sensor: &Rgb32f,
    reference: &Rgb32f,
    edge_limit: Option<f32>,
    colour: impl Fn([f64; 3]) -> [f64; 3],
) -> Option<CameraTone> {
    let luminances = |x: [f64; 3], y: [f64; 3]| (luma(colour(x).map(|v| v.max(0.0))), luma(y));
    let original = fit.fit(pairs.iter().enumerate().filter(|(i, _)| i % 3 != 0).map(|(_, (x, y))| luminances(*x, *y)).collect());
    let mut eligible = Vec::new();
    let (mut colour_index, mut extra_index) = (0, 0);
    for (i, (input, output)) in sensor.data.iter().zip(&reference.data).enumerate() {
        if edge_limit.is_some_and(|limit| local_contrast(sensor, i) > limit || local_contrast(reference, i) > limit) {
            continue;
        }
        // About one 8-bit sRGB code above black: don't learn a flat black segment from
        // JPEG compression. The sensor proxy averages many samples before this test.
        if !input.iter().all(|v| v.is_finite() && *v > 0.0001 && *v < 1.5)
            || !output.iter().all(|v| v.is_finite() && *v >= 0.0)
            || !(0.0003..0.85).contains(&luminance_2020(*output))
        {
            continue;
        }
        // A colour holdout must never enter tone training just because the wider sample set
        // changes its index. Split the extra samples separately, keeping every colour index.
        let index = if colour_pair(*input, *output) { &mut colour_index } else { &mut extra_index };
        let held = *index % 3 == 0;
        *index += 1;
        let (x, y) = luminances(input.map(f64::from), output.map(f64::from));
        if x.is_finite() && x > 0.0 {
            eligible.push((x, y, held));
        }
    }
    let dark = eligible.iter().filter(|(_, y, _)| *y < 0.015).count();
    let held: Pairs = eligible.iter().filter(|(_, y, held)| *held && *y < 0.015).map(|(x, y, _)| ([*x; 3], [*y; 3])).collect();
    // Sparse or uncorrelated shadow noise cannot teach a toe. Tied JPEG levels share a rank,
    // so a single quantized grey also keeps the original curve.
    if dark < 96 || held.len() < 32 || rank_correlation_ties(&held) < PARTIAL_MIN_RANK_CORRELATION {
        return original;
    }
    let training = eligible.iter().filter(|(_, _, held)| !held).map(|(x, y, _)| (*x, *y)).collect();
    let Some(candidate) = fit.fit(training) else { return original };
    let error = |tone: &CameraTone| -> f64 {
        held.iter().map(|(x, y)| (f64::from(tone.apply(x[0] as f32)).powf(1.0 / 2.2) - y[0].powf(1.0 / 2.2)).powi(2)).sum()
    };
    // A wider histogram must also help independent dark pixels, not just redistribute the
    // knots. The usual held-out gates still check the completed look on the colour pairs.
    if let Some(old) = original {
        let (before, after) = (error(&old), error(&candidate));
        if crate::profiling() {
            eprintln!("[profile] camera toe: {dark} dark pairs, held-out error {before:.5} -> {after:.5}");
        }
        if !after.is_finite() || after >= before * 0.99 {
            return original;
        }
    }
    Some(candidate)
}

/// Ridge-regularised 3×3 chromaticity matrix (luminance-normalised RGB) on the training pairs.
pub(super) fn fit_matrix(pairs: &[([f64; 3], [f64; 3])]) -> Option<Mat3> {
    let mut gram = [[0.0; 3]; 3];
    let mut cross = [[0.0; 3]; 3];
    for (i, (x, y)) in pairs.iter().enumerate() {
        if i % 3 == 0 {
            continue;
        }
        let (lx, ly) = (luma(*x), luma(*y));
        if lx <= 0.0 || ly <= 0.0 {
            continue;
        }
        for row in 0..3 {
            for col in 0..3 {
                // Normalising by luminance prevents a camera S-curve from corrupting colour.
                gram[row][col] += x[row] * x[col] / (lx * lx);
                cross[row][col] += y[row] * x[col] / (ly * lx);
            }
        }
    }
    let trace: f64 = (0..3).map(|i| gram[i][i]).sum();
    let inverse = Mat3(gram).inverse()?;
    let condition = trace * (0..3).map(|i| inverse.0[i][i].abs()).sum::<f64>();
    if trace <= 0.0 || !condition.is_finite() || condition > 1e6 {
        return None;
    }
    let regularization = trace * 1e-4;
    for i in 0..3 {
        gram[i][i] += regularization;
        cross[i][i] += regularization;
    }
    let matrix = Mat3(cross).mul(&Mat3(gram).inverse()?);
    matrix.0.iter().flatten().all(|v| v.is_finite() && v.abs() < 8.0).then_some(matrix)
}

/// Neighbourhood luminance ratio above which a pixel counts as an edge for the second attempt
/// of [`search_ordered`]. Camera JPEGs are often lens-corrected (Sony "Distortion Comp.: Auto",
/// compacts and kit zooms): on a 51 mm ILCE-7RM2 shot of a glass façade the JPEG is up to 15 px
/// of 1440 (about one proxy pixel) off the raw, and the fit failed the gate on the mismatched
/// window frames alone (held-out RMS 0.111; 0.059 for the same shot with distortion correction
/// off). Away from edges: 0.049. Issue storytold/lightcraft#232.
pub(super) const EDGE_CONTRAST: f32 = 3.0;

/// The look on all pixels (the search of [`search_ordered`] with [`TONE_FITS`]), as before
/// clipped pixels were left out.
#[cfg(test)]
pub(super) fn fit_pairs_with(sensor: &Rgb32f, reference: &Rgb32f, colour: Option<(Mat3, Option<HsvTable>)>) -> Option<CameraLook> {
    fit_pairs_ordered(sensor, reference, colour, TONE_FITS)
}

/// The look from [`search_ordered`] without its attempt.
#[cfg(test)]
pub(super) fn fit_pairs_ordered(
    sensor: &Rgb32f,
    reference: &Rgb32f,
    colour: Option<(Mat3, Option<HsvTable>)>,
    order: &[ToneFit],
) -> Option<CameraLook> {
    search_ordered(sensor, reference, colour, order, &HighlightCheck { pairs: highlights(sensor, reference), dro: false, grey: false })
        .map(|(look, _)| look)
}

/// The tone fits the search tries, in order: the first whose look passes the acceptance gates is
/// used. Quantile matching comes first because it cannot produce flat steps; the
/// conditional-median fit follows because on a few photos (compacts, early Micro Four Thirds) its
/// slightly lower held-out error is what clears the gates, and a photo must not lose its look to
/// the change of fit.
pub(super) const TONE_FITS: &[ToneFit] = &[ToneFit::Quantile, ToneFit::ConditionalMedian];

/// One attempt of the search: tone fit, the camera profile's colour or the photo's own, all pixels
/// or only those away from edges.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct Attempt {
    pub(super) tone: ToneFit,
    pub(super) profile: bool,
    pub(super) away_from_edges: bool,
    /// Train on the dark end of the preview too ([`DARK_FLOOR`]): the tone-only fit of a DNG.
    pub(super) dark: bool,
}

/// The photo's look and the attempt that produced it: its own matrix (with and without a
/// hue/saturation table), or the given colour model (a camera profile's, in the sensor proxy's
/// space), each completed with a tone and chroma curve fitted to this photo. Per tone fit in
/// `order`: the camera profile's colour (when there is one), then the photo's own, as a different
/// picture style can make a camera profile fail the gates; each fitted on all pixels and, when that
/// fails the gates, once more on the pixels away from edges (same gates): there colour pairs stay
/// valid when the camera JPEG's geometry differs slightly from the raw's (issue storytold/lightcraft#232). The whole
/// search runs with one tone fit before the next is tried, so the conditional-median fallback
/// accepts exactly what the previous releases accepted.
pub(super) fn search_ordered(
    sensor: &Rgb32f,
    reference: &Rgb32f,
    colour: Option<(Mat3, Option<HsvTable>)>,
    order: &[ToneFit],
    bright: &HighlightCheck,
) -> Option<(CameraLook, Attempt)> {
    let profiles: &[bool] = if colour.is_some() { &[true, false] } else { &[false] };
    order.iter().find_map(|&tone| {
        profiles.iter().find_map(|&profile| {
            [false, true].into_iter().find_map(|away_from_edges| {
                let attempt = Attempt { tone, profile, away_from_edges, dark: false };
                fit_attempt(sensor, reference, &colour, attempt, bright).map(|look| (look, attempt))
            })
        })
    })
}

pub(super) fn fit_attempt(
    sensor: &Rgb32f,
    reference: &Rgb32f,
    colour: &Option<(Mat3, Option<HsvTable>)>,
    attempt: Attempt,
    bright: &HighlightCheck,
) -> Option<CameraLook> {
    let colour = if attempt.profile { colour.clone() } else { None };
    fit_pairs_on(sensor, reference, colour, attempt.away_from_edges.then_some(EDGE_CONTRAST), attempt.tone, attempt.dark, bright)
}

fn fit_pairs_on(
    sensor: &Rgb32f,
    reference: &Rgb32f,
    colour: Option<(Mat3, Option<HsvTable>)>,
    edge_limit: Option<f32>,
    tone_fit: ToneFit,
    dark: bool,
    highlights: &HighlightCheck,
) -> Option<CameraLook> {
    // A known colour model needs enough signal for tone fitting, not a scene rich enough to
    // learn a new colour matrix. Still reject monochrome references.
    let (pairs, bright) = collect_pairs(sensor, reference, if colour.is_some() { 0.005 } else { 0.05 }, edge_limit, dark)?;
    let candidates = match colour {
        Some(given) => vec![given],
        None => {
            let matrix = fit_matrix(&pairs)?;
            vec![(matrix, fit_hue_sat(&pairs, &matrix)), (matrix, None)]
        }
    };
    // Matrix + table when the table helps the held-out pixels, else the matrix alone. Chosen by
    // error in gamma-encoded display values (closer to what is seen: in linear values a slightly
    // missed bright rock outweighs a clearly wrong dark shirt); the acceptance gates below stay linear.
    let mut best: Option<(f64, f64, usize, CameraLook)> = None;
    for (matrix, hue_sat) in candidates {
        let correction = hue_sat.as_ref().and_then(HueSat::new);
        let colour = |x: [f64; 3]| {
            let p = matrix.apply(x);
            correction.as_ref().map_or(p, |c| c.apply(p.map(|v| v as f32)).map(f64::from))
        };
        let Some(curve) = fit_tone_with_shadows(tone_fit, &pairs, sensor, reference, edge_limit, colour) else { continue };
        let mut look = CameraLook { matrix, tone: curve, hue_sat: hue_sat.clone() };
        if let Some(tone) = fit_chroma(&bright, &look) {
            look.tone = tone;
        }
        let tone = ToneMap::camera(&look.tone, 0.0, 0.0, 0.0);
        if !highlights_hold(&look.tone, highlights, colour) {
            continue;
        }
        let (mut linear, mut perceptual, mut samples) = (0.0, 0.0, 0);
        for (x, target) in pairs.iter().step_by(3) {
            let corrected = displayed(colour(*x), &tone);
            for c in 0..3 {
                linear += (corrected[c] - target[c]).powi(2);
                perceptual += (corrected[c].max(0.0).powf(1.0 / 2.2) - target[c].max(0.0).powf(1.0 / 2.2)).powi(2);
                samples += 1;
            }
        }
        if linear.is_finite() && perceptual.is_finite() && best.as_ref().is_none_or(|b| perceptual < b.0) {
            best = Some((perceptual, linear, samples, look));
        }
    }
    let (_, after, samples, look) = best?;
    let original_tone = ToneMap::new(0.0, 0.0, 0.0);
    let before: f64 = pairs
        .iter()
        .step_by(3)
        .map(|(x, target)| {
            let original = displayed(*x, &original_tone);
            (0..3).map(|c| (original[c] - target[c]).powi(2)).sum::<f64>()
        })
        .sum();
    if crate::profiling() {
        eprintln!(
            "[profile] camera look holdout RMS {:.5} -> {:.5} ({samples} channels{}{})",
            (before / samples as f64).sqrt(),
            (after / samples as f64).sqrt(),
            if edge_limit.is_some() { ", away from edges" } else { "" },
            if tone_fit == ToneFit::ConditionalMedian { ", conditional-median tone" } else { "" }
        );
    }
    if samples == 0 || after >= before * MIN_IMPROVEMENT || after / samples as f64 > MAX_HOLDOUT_RMS.powi(2) {
        return None;
    }
    Some(look)
}

/// Weight of the photo's own colour matrix in a [`fit_partial`] look; the rest is the identity (the
/// neutral fallback's generic camera ≈ sRGB colour). On the CC0 archive's proxies, with the
/// matrix at full weight 2 of the 23 colourful photos without a look came out more than 2° further
/// from the camera's hues than the neutral fallback, at half weight none (median hue error 9.6° with
/// the generic colour, which the tone and chroma curves make visible, 5.1° at half weight).
const PARTIAL_MATRIX_WEIGHT: f64 = 0.5;

/// Smallest rank correlation of scene and camera JPEG luminance over a [`fit_partial`] look's pairs:
/// the JPEG must show the same picture. A look that only matches the brightness distribution of an
/// unrelated picture can still beat the dark neutral fallback pixel for pixel. On the CC0 archive's
/// proxies the accepted full looks have 0.55 or more, lens-corrected compacts without one 0.53–0.77,
/// three old Coolpix files whose raw decodes wrongly 0.07–0.27.
pub(super) const PARTIAL_MIN_RANK_CORRELATION: f64 = 0.5;

/// Spearman rank correlation of the scene's and the JPEG's luminance over `pairs`.
fn rank_correlation(pairs: &[([f64; 3], [f64; 3])]) -> f64 {
    let ranks = |values: Vec<f64>| {
        let mut order: Vec<usize> = (0..values.len()).collect();
        order.sort_by(|a, b| values[*a].total_cmp(&values[*b]));
        let mut ranks = vec![0.0; values.len()];
        for (rank, i) in order.into_iter().enumerate() {
            ranks[i] = rank as f64;
        }
        ranks
    };
    let scene = ranks(pairs.iter().map(|(x, _)| luma(*x)).collect());
    let jpeg = ranks(pairs.iter().map(|(_, y)| luma(*y)).collect());
    let mean = (pairs.len() as f64 - 1.0) / 2.0;
    let (covariance, variance) =
        scene.iter().zip(&jpeg).fold((0.0, 0.0), |(c, v), (a, b)| (c + (a - mean) * (b - mean), v + (a - mean) * (a - mean)));
    if variance > 0.0 { covariance / variance } else { 0.0 }
}

/// [`rank_correlation`] with tied luminances sharing their mean rank, and 0 when either side has no spread. Without
/// colour to require, a JPEG may be flat (one grey for every pixel): [`rank_correlation`] ranks ties in pixel order,
/// so a flat JPEG of a scene whose pixels are stored in brightness order looks perfectly related.
pub(super) fn rank_correlation_ties(pairs: &[([f64; 3], [f64; 3])]) -> f64 {
    let ranks = |values: Vec<f64>| {
        let mut order: Vec<usize> = (0..values.len()).collect();
        order.sort_by(|a, b| values[*a].total_cmp(&values[*b]));
        let mut ranks = vec![0.0; values.len()];
        let mut start = 0;
        while start < order.len() {
            let mut end = start + 1;
            while end < order.len() && values[order[end]] == values[order[start]] {
                end += 1;
            }
            let mean = (start + end - 1) as f64 / 2.0;
            for &i in &order[start..end] {
                ranks[i] = mean;
            }
            start = end;
        }
        ranks
    };
    let scene = ranks(pairs.iter().map(|(x, _)| luma(*x)).collect());
    let jpeg = ranks(pairs.iter().map(|(_, y)| luma(*y)).collect());
    let mean = (pairs.len() as f64 - 1.0) / 2.0;
    let (covariance, scene_variance, jpeg_variance) = scene
        .iter()
        .zip(&jpeg)
        .fold((0.0, 0.0, 0.0), |(c, va, vb), (a, b)| (c + (a - mean) * (b - mean), va + (a - mean) * (a - mean), vb + (b - mean) * (b - mean)));
    if scene_variance > 0.0 && jpeg_variance > 0.0 { covariance / (scene_variance * jpeg_variance).sqrt() } else { 0.0 }
}

/// A partial look for a photo whose full look misses the acceptance gates (lens-corrected JPEGs of
/// compacts, dull or nearly colourless scenes): the tone curve by quantile matching, which needs
/// no pixel-exact registration and was the largest part of the neutral fallback's error (rendered
/// 9–22 L* too dark), the chroma curve (the fallback kept about a third of the camera's
/// colourfulness), and, when the scene has enough colour to learn one (the full look's condition),
/// the photo's own colour matrix at [`PARTIAL_MATRIX_WEIGHT`]. No hue/saturation table. Tried on
/// all pixels and then, like the full look, on the pixels away from edges; used only when it cuts
/// the held-out squared error to below [`MIN_IMPROVEMENT`] of the neutral fallback's, in linear
/// and in gamma-encoded display values, and when the JPEG shows the same picture
/// ([`PARTIAL_MIN_RANK_CORRELATION`]). There is no limit on its own error: it only has to be clearly
/// closer to the camera JPEG than what the photo would get otherwise.
pub(super) fn fit_partial(sensor: &Rgb32f, reference: &Rgb32f, highlights: &HighlightCheck) -> Option<CameraLook> {
    [None, Some(EDGE_CONTRAST)].into_iter().find_map(|edge_limit| fit_partial_on(sensor, reference, edge_limit, highlights))
}

fn fit_partial_on(sensor: &Rgb32f, reference: &Rgb32f, edge_limit: Option<f32>, highlights: &HighlightCheck) -> Option<CameraLook> {
    // as for a known colour model: enough signal for the tone, but no monochrome reference
    let (pairs, bright) = collect_pairs(sensor, reference, 0.005, edge_limit, false)?;
    let related = rank_correlation(&pairs);
    if related < PARTIAL_MIN_RANK_CORRELATION {
        if crate::profiling() {
            eprintln!("[profile] camera look partial: luminance rank correlation {related:.3}, not the same picture");
        }
        return None;
    }
    let spread = |y: &[f64; 3]| y.iter().copied().fold(f64::NEG_INFINITY, f64::max) - y.iter().copied().fold(f64::INFINITY, f64::min);
    let colourful = pairs.iter().filter(|(_, y)| spread(y) > 0.05).count() >= pairs.len() / 20;
    let matrix = match colourful.then(|| fit_matrix(&pairs)).flatten() {
        Some(own) => Mat3(std::array::from_fn(|r| {
            std::array::from_fn(|c| Mat3::IDENTITY.0[r][c] + PARTIAL_MATRIX_WEIGHT * (own.0[r][c] - Mat3::IDENTITY.0[r][c]))
        })),
        None => Mat3::IDENTITY,
    };
    let tone = fit_tone_with_shadows(ToneFit::Quantile, &pairs, sensor, reference, edge_limit, |x| matrix.apply(x))?;
    let mut look = CameraLook { matrix, tone, hue_sat: None };
    if let Some(tone) = fit_chroma(&bright, &look) {
        look.tone = tone;
    }
    let (tone, neutral) = (ToneMap::camera(&look.tone, 0.0, 0.0, 0.0), ToneMap::new(0.0, 0.0, 0.0));
    if !highlights_hold(&look.tone, highlights, |x| matrix.apply(x)) {
        return None;
    }
    let encoded = |v: f64| v.max(0.0).powf(1.0 / 2.2);
    let (mut before, mut after, mut before_encoded, mut after_encoded, mut samples) = (0.0, 0.0, 0.0, 0.0, 0);
    for (x, target) in pairs.iter().step_by(3) {
        let (fallback, partial) = (displayed(*x, &neutral), displayed(matrix.apply(*x), &tone));
        for c in 0..3 {
            before += (fallback[c] - target[c]).powi(2);
            after += (partial[c] - target[c]).powi(2);
            before_encoded += (encoded(fallback[c]) - encoded(target[c])).powi(2);
            after_encoded += (encoded(partial[c]) - encoded(target[c])).powi(2);
            samples += 1;
        }
    }
    if crate::profiling() {
        eprintln!(
            "[profile] camera look partial (tone{}{}) holdout RMS {:.5} -> {:.5} ({samples} channels{})",
            if look.tone.chroma().iter().any(|k| *k != 1.0) { ", chroma" } else { "" },
            if matrix != Mat3::IDENTITY { ", damped matrix" } else { "" },
            (before / samples.max(1) as f64).sqrt(),
            (after / samples.max(1) as f64).sqrt(),
            if edge_limit.is_some() { ", away from edges" } else { "" }
        );
    }
    let better =
        after.is_finite() && after_encoded.is_finite() && after < before * MIN_IMPROVEMENT && after_encoded < before_encoded * MIN_IMPROVEMENT;
    (samples > 0 && better).then_some(look)
}
