use super::colour::file_local_look;
use super::hue_sat::HueSat;
use super::look_fit::{look_proxies, with_dro_off_tone, without_clipped};
use super::pairs::{
    EDGE_CONTRAST, MIN_IMPROVEMENT, PARTIAL_MIN_RANK_CORRELATION, Pairs, collect_pairs, displayed, fit_tone_with_shadows, local_contrast, luma,
    rank_correlation_ties,
};
use super::proxy::LensGeometry;
use super::tone::{HighlightCheck, ToneFit, fit_tone, highlights, highlights_hold};
use super::{CameraLook, CameraTransform, D50, D65, HsvTable, Mat3, PROPHOTO, REC2020, RawImage, Rgb32f, ToneMap, bradford, luminance_2020};
#[allow(unused_imports)]
use crate::sensor::prelude::*;

/// A camera JPEG pixel's channels may differ by this much (linear) and still count as grey.
const GREY_SPREAD: f32 = 0.002;

/// Whether the camera JPEG is black and white: a monochrome picture style (Canon Monochrome, Sony B/W, Nikon
/// Monochrome), shot from a full-colour raw. All but one proxy pixel in a thousand are grey within
/// [`GREY_SPREAD`]. A toned monochrome (sepia) has colour, so it isn't one.
pub(super) fn monochrome(reference: &Rgb32f) -> bool {
    let coloured = reference
        .data
        .iter()
        .filter(|p| {
            let (min, max) = p.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), v| (lo.min(*v), hi.max(*v)));
            !p.iter().all(|v| v.is_finite()) || max - min > GREY_SPREAD
        })
        .count();
    reference.data.len() >= 256 && coloured.saturating_mul(1000) <= reference.data.len()
}

/// The look of a raw whose camera JPEG is black and white ([`monochrome`]; issue storytold/lightcraft#577) and gave no look with a camera
/// profile's colour in [`fit_preview`] (or whose model has no profile). Such a JPEG has no colour to fit, so the photo
/// would start from the spectral matrices or the neutral fallback with the default tone curve, about 1.3 EV darker
/// and flatter than the same body's photos shot in colour. But the JPEG still shows the camera's tone curve.
/// `transform` is the colour the photo gets otherwise (the spectral matrices or the fallback's); the look keeps it and
/// adds the tone curve alone ([`fit_tone_only_with`]), as [`fit_look_with`] tries the photo's own colour after a profile's. The
/// chroma curve stays neutral: the JPEG has no colourfulness to learn. `embedded` as for [`fit_preview`].
pub(super) fn monochrome_look(
    raw: &RawImage,
    bytes: &[u8],
    transform: &CameraTransform,
    lens: Option<&LensGeometry>,
    embedded: Option<&craft_develop_model::EmbeddedLens>,
) -> Option<CameraLook> {
    if !file_local_look(raw.format) {
        return None;
    }
    let (sensor, reference, clipped) = look_proxies(raw, bytes, transform, lens, embedded)?;
    if !monochrome(&reference) {
        return None;
    }
    // Sony's Dynamic Range Optimizer brightens a B/W JPEG as much as a colour one
    let dro = craft_raw::embedded_preview_dynamic_range_optimized(bytes) == Some(true);
    let unclipped = without_clipped(&sensor, &clipped);
    let bright = HighlightCheck { pairs: highlights(&sensor, &reference), dro, grey: true };
    let look = fit_tone_only_with(unclipped.as_ref().unwrap_or(&sensor), &reference, None, &bright)?;
    let look = if dro { with_dro_off_tone(look) } else { look };
    if crate::profiling() {
        eprintln!(
            "[profile] {:?} camera look from a black-and-white JPEG: tone {:?}, {} colour, DRO tone replaced {dro}",
            raw.format,
            look.tone,
            if transform.matrix_is_fallback { "fallback" } else { "spectral" }
        );
    }
    Some(look)
}

/// The tone curve of a black-and-white camera JPEG, on top of `colour` (a camera profile's, in the sensor proxy's
/// space) or, without one, of the sensor proxy's own colour. Fitted like [`fit_partial`]'s, by quantile matching of
/// the luminances, on all pixels and then away from edges, and used only when the JPEG shows the same picture
/// ([`PARTIAL_MIN_RANK_CORRELATION`]) and the look cuts the held-out squared luminance error to below
/// [`MIN_IMPROVEMENT`] of the default curve's, in linear and in gamma-encoded display values. A curve narrower than
/// [`NARROW_TONE_RANGE`] must also keep the JPEG's bright areas ([`highlights_hold`], by luminance).
pub(super) fn fit_tone_only_with(
    sensor: &Rgb32f,
    reference: &Rgb32f,
    colour: Option<(Mat3, Option<HsvTable>)>,
    highlights: &HighlightCheck,
) -> Option<CameraLook> {
    [None, Some(EDGE_CONTRAST)].into_iter().find_map(|edge_limit| fit_tone_only_on(sensor, reference, &colour, edge_limit, highlights))
}

/// [`fit_tone_only_with`] for a photo without Sony's DRO, checked against its own bright areas.
#[cfg(test)]
pub(super) fn fit_tone_only(sensor: &Rgb32f, reference: &Rgb32f, colour: Option<(Mat3, Option<HsvTable>)>) -> Option<CameraLook> {
    fit_tone_only_with(sensor, reference, colour, &HighlightCheck { pairs: highlights(sensor, reference), dro: false, grey: true })
}

fn fit_tone_only_on(
    sensor: &Rgb32f,
    reference: &Rgb32f,
    colour: &Option<(Mat3, Option<HsvTable>)>,
    edge_limit: Option<f32>,
    highlights: &HighlightCheck,
) -> Option<CameraLook> {
    // every pair counts: the JPEG has no colour to require
    let (pairs, _) = collect_pairs(sensor, reference, f32::NEG_INFINITY, edge_limit, false)?;
    fit_tone_only_pairs(sensor, reference, colour, edge_limit, &pairs, false, highlights)
}

/// A near-black frame may have enough luminance signal for tone but fewer than 256 colour pairs.
/// Try only after every existing look fails. Keep the colour of the source transform,
/// exclude clipped pixels from training, and keep their luminance at least as close to the JPEG
/// as the default curve does. Bright JPEGs need the existing fits to constrain their highlights.
pub(super) fn fit_dark_tone(sensor: &Rgb32f, reference: &Rgb32f, clipped: &[bool]) -> Option<CameraLook> {
    if (sensor.width, sensor.height) != (reference.width, reference.height)
        || sensor.data.len() != reference.data.len()
        || clipped.len() != sensor.data.len()
        || sensor.data.is_empty()
        || sensor.data.iter().any(|p| !p.iter().all(|v| v.is_finite()))
        || reference.data.iter().any(|p| !p.iter().all(|v| v.is_finite() && *v >= 0.0) || luminance_2020(*p) >= 0.85)
    {
        return None;
    }
    let dark = reference.data.iter().filter(|p| luminance_2020(**p) < 0.015).count();
    if (dark as f64) < reference.data.len() as f64 * 0.9 {
        return None;
    }
    // no JPEG pixel reaches 0.85 here, so the bright-area check has nothing to hold
    let bright = HighlightCheck { pairs: highlights(sensor, reference), dro: false, grey: true };
    [None, Some(EDGE_CONTRAST)].into_iter().find_map(|edge_limit| {
        let pairs: Pairs = sensor
            .data
            .iter()
            .zip(&reference.data)
            .enumerate()
            .filter(|(i, (input, output))| {
                // The same signal floors as the camera toe, without colour ratios or counts.
                !clipped.get(*i).copied().unwrap_or(true)
                    && input.iter().all(|v| *v > 0.0001 && *v < 1.5)
                    && (0.0003..0.85).contains(&luminance_2020(**output))
                    && edge_limit.is_none_or(|limit| local_contrast(sensor, *i) <= limit && local_contrast(reference, *i) <= limit)
            })
            .map(|(_, (input, output))| (input.map(f64::from), output.map(f64::from)))
            .collect();
        if pairs.len() < 256 {
            return None;
        }
        let look = fit_tone_only_pairs(sensor, reference, &None, edge_limit, &pairs, true, &bright)?;
        let (tone, neutral) = (ToneMap::camera(&look.tone, 0.0, 0.0, 0.0), ToneMap::new(0.0, 0.0, 0.0));
        for ((input, target), clipped) in sensor.data.iter().zip(&reference.data).zip(clipped) {
            if *clipped {
                let target = luma(target.map(f64::from));
                let before = luma(displayed(input.map(f64::from), &neutral));
                let after = luma(displayed(input.map(f64::from), &tone));
                if !after.is_finite() || (after - target).abs() > (before - target).abs() {
                    return None;
                }
            }
        }
        Some(look)
    })
}

fn fit_tone_only_pairs(
    sensor: &Rgb32f,
    reference: &Rgb32f,
    colour: &Option<(Mat3, Option<HsvTable>)>,
    edge_limit: Option<f32>,
    pairs: &Pairs,
    dark: bool,
    highlights: &HighlightCheck,
) -> Option<CameraLook> {
    let kind = if dark { "near-black scene" } else { "black-and-white JPEG" };
    let related = rank_correlation_ties(pairs);
    if related < PARTIAL_MIN_RANK_CORRELATION {
        if crate::profiling() {
            eprintln!("[profile] camera look from a {kind}: luminance rank correlation {related:.3}, not the same picture");
        }
        return None;
    }
    let (matrix, hue_sat) = colour.clone().unwrap_or((Mat3::IDENTITY, None));
    let correction = hue_sat.as_ref().and_then(HueSat::new);
    let coloured = |x: [f64; 3]| {
        let p = matrix.apply(x);
        correction.as_ref().map_or(p, |c| c.apply(p.map(|v| v as f32)).map(f64::from))
    };
    let tone = if dark {
        // Split this wider set once. The camera toe's separate colour/extra split belongs to
        // the original colour pairs and would put some of these held-out pixels into training.
        fit_tone(
            pairs.iter().enumerate().filter(|(i, _)| i % 3 != 0).map(|(_, (x, y))| (luma(coloured(*x).map(|v| v.max(0.0))), luma(*y))).collect(),
        )?
    } else {
        fit_tone_with_shadows(ToneFit::Quantile, pairs, sensor, reference, edge_limit, coloured)?
    };
    let look = CameraLook { matrix, tone, hue_sat: hue_sat.clone() };
    if !highlights_hold(&look.tone, highlights, coloured) {
        return None;
    }
    let (tone, neutral) = (ToneMap::camera(&look.tone, 0.0, 0.0, 0.0), ToneMap::new(0.0, 0.0, 0.0));
    let encoded = |v: f64| v.max(0.0).powf(1.0 / 2.2);
    let (mut before, mut after, mut before_encoded, mut after_encoded, mut samples) = (0.0, 0.0, 0.0, 0.0, 0);
    for (x, target) in pairs.iter().step_by(3) {
        let (fallback, toned, target) = (luma(displayed(*x, &neutral)), luma(displayed(coloured(*x), &tone)), luma(*target));
        before += (fallback - target).powi(2);
        after += (toned - target).powi(2);
        before_encoded += (encoded(fallback) - encoded(target)).powi(2);
        after_encoded += (encoded(toned) - encoded(target)).powi(2);
        samples += 1;
    }
    if crate::profiling() {
        eprintln!(
            "[profile] camera look from a {kind}: holdout luminance RMS {:.5} -> {:.5} ({samples} pixels{})",
            (before / samples.max(1) as f64).sqrt(),
            (after / samples.max(1) as f64).sqrt(),
            if edge_limit.is_some() { ", away from edges" } else { "" }
        );
    }
    let better =
        after.is_finite() && after_encoded.is_finite() && after < before * MIN_IMPROVEMENT && after_encoded < before_encoded * MIN_IMPROVEMENT;
    (samples > 0 && better).then_some(look)
}

/// Linear Rec.2020 D65 → linear ProPhoto RGB D50, the space DNG hue/saturation tables work in.
pub(super) fn to_prophoto() -> Mat3 {
    PROPHOTO.from_xyz().mul(&bradford(D65, D50)).mul(&REC2020.to_xyz())
}
