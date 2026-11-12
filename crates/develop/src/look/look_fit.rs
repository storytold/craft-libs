use super::colour::{PROXY, file_local_look};
use super::mono::{fit_tone_only_with, monochrome};
use super::pairs::{Attempt, TONE_FITS, fit_attempt, fit_partial, search_ordered};
use super::proxy::{CLIP_TOLERANCE, LensGeometry, MIN_PREVIEW_FRACTION, proxies, structure_agrees};
use super::tone::{HighlightCheck, highlights};
use super::{CameraLook, CameraTone, CameraTransform, HsvTable, Mat3, RawImage, Rgb32f, ToneMap, luminance_2020};
#[allow(unused_imports)]
use crate::sensor::prelude::*;

/// The look fitted to the file's camera JPEG, and whether that JPEG is black and white ([`monochrome`]).
/// `embedded`: the file's embedded lens correction, whose vignetting the render applies ([`look_proxies`]).
pub(crate) fn fit_preview(
    raw: &RawImage,
    bytes: &[u8],
    transform: &CameraTransform,
    lens: Option<&LensGeometry>,
    embedded: Option<&craft_develop_model::EmbeddedLens>,
) -> (Option<CameraLook>, bool) {
    if !transform.matrix_is_fallback || !file_local_look(raw.format) {
        return (None, false);
    }
    let Some((sensor, reference, clipped)) = look_proxies(raw, bytes, transform, lens, embedded) else { return (None, false) };
    let black_and_white = monochrome(&reference);
    // A camera profile pooled from many photos knows colours this photo shows too little of;
    // try its colour first and fit tone/chroma per photo (picture styles vary).
    let profile = raw.camera.model.as_deref().and_then(crate::look::profiles::get);
    let colour = profile.as_ref().and_then(|p| Some((p.matrix().mul(&transform.matrix.inverse()?), p.hue_sat.clone())));
    // Sony's Dynamic Range Optimizer (on by default) brightens the camera JPEG's darker regions,
    // not the raw: keep the colour fitted to the JPEG, take the tone curve without DRO.
    let dro = craft_raw::embedded_preview_dynamic_range_optimized(bytes) == Some(true);
    let look = match fit_look_with(&sensor, &reference, &clipped, colour.clone(), dro) {
        Some(look) => look,
        // A black-and-white JPEG has no colour to fit: the profile's colour and the JPEG's tone curve
        // (`monochrome_look` does it without a profile).
        None if colour.is_some() && black_and_white => {
            let unclipped = without_clipped(&sensor, &clipped);
            let bright = HighlightCheck { pairs: highlights(&sensor, &reference), dro, grey: true };
            let Some(look) = fit_tone_only_with(unclipped.as_ref().unwrap_or(&sensor), &reference, colour, &bright) else { return (None, true) };
            look
        }
        None => return (None, black_and_white),
    };
    let look = if dro { with_dro_off_tone(look) } else { look };
    if crate::profiling() {
        eprintln!(
            "[profile] {:?} camera look: {:?}, {:?}, hue/sat table {}, camera profile available {}, DRO tone replaced {dro}",
            raw.format,
            look.matrix.0,
            look.tone,
            look.hue_sat.is_some(),
            profile.is_some()
        );
    }
    (Some(look), black_and_white)
}

/// Sensor level (normalised, white = 1) at or above which a raw channel counts as clipped, as in
/// the render's highlight reconstruction.
pub(super) const SENSOR_CLIP: f32 = 0.99;

/// [`fit_look_with`] for a photo without Sony's DRO.
#[cfg(test)]
pub(super) fn fit_look(sensor: &Rgb32f, reference: &Rgb32f, clipped: &[bool], colour: Option<(Mat3, Option<HsvTable>)>) -> Option<CameraLook> {
    fit_look_with(sensor, reference, clipped, colour, false)
}

/// The photo's look, refitted without the proxy pixels whose sensor values are clipped.
///
/// A clipped raw pixel no longer records the scene's colour: with one channel held at the clip
/// level while the others keep rising, a white overcast sky comes out magenta or blue through the
/// camera matrix, while the camera JPEG shows it white. As correspondences such pixels teach the
/// colour matrix to pull colours toward grey, and the chroma curve to remove colour at their
/// display luminance (on a Z 6II forest under a clipped sky the curve cut colourfulness above
/// display luminance 0.4 to 13–19%, and every one of its samples there was clipped). The render
/// rebuilds clipped highlights separately (`crate::sensor::highlight`), so the look is fitted on
/// what the sensor measured.
///
/// The search on all pixels decides, as before, whether the photo gets a look and which attempt
/// of [`search_ordered`] gives it; only that attempt is refitted without the clipped pixels, and
/// when the refit misses the acceptance gates the look on all pixels is kept. So no photo loses
/// its look and none switches attempt (a refit that just clears the gate on all pixels must not
/// replace a look fitted away from edges). A photo that had no look gets one when the search
/// without the clipped pixels finds it, and otherwise the [`fit_partial`] look when that is clearly
/// closer to the camera JPEG than the neutral fallback.
pub(super) fn fit_look_with(
    sensor: &Rgb32f,
    reference: &Rgb32f,
    clipped: &[bool],
    colour: Option<(Mat3, Option<HsvTable>)>,
    dro: bool,
) -> Option<CameraLook> {
    let unclipped = without_clipped(sensor, clipped);
    // from the proxies as they are: the retries' filters (clipped pixels, edges) don't remove them
    let bright = HighlightCheck { pairs: highlights(sensor, reference), dro, grey: false };
    let Some((look, attempt)) = search_ordered(sensor, reference, colour.clone(), TONE_FITS, &bright) else {
        return unclipped
            .as_ref()
            .and_then(|u| search_ordered(u, reference, colour, TONE_FITS, &bright))
            .map(|(look, _)| look)
            .or_else(|| fit_partial(unclipped.as_ref().unwrap_or(sensor), reference, &bright));
    };
    let Some(unclipped) = unclipped else { return Some(look) };
    let refit = fit_attempt(&unclipped, reference, &colour, attempt, &bright);
    if refit.is_none() && crate::profiling() {
        eprintln!("[profile] camera look: no fit without the clipped pixels on the accepted attempt, keeping the fit on all pixels");
    }
    Some(refit.unwrap_or(look))
}

/// `sensor` with its clipped pixels set to NaN (left out of every training pair), or `None` when
/// none is clipped.
pub(super) fn without_clipped(sensor: &Rgb32f, clipped: &[bool]) -> Option<Rgb32f> {
    if clipped.len() != sensor.data.len() || !clipped.iter().any(|c| *c) {
        return None;
    }
    let mut out = sensor.clone();
    for (p, c) in out.data.iter_mut().zip(clipped) {
        if *c {
            *p = [f32::NAN; 3];
        }
    }
    if crate::profiling() {
        eprintln!(
            "[profile] camera look: {} of {} proxy pixels have a clipped sensor channel",
            clipped.iter().filter(|c| **c).count(),
            clipped.len()
        );
    }
    Some(out)
}

/// 1 where any channel of a camera-RGB pixel is at or above [`SENSOR_CLIP`], else 0.
pub(super) fn clip_mask(camera: &Rgb32f) -> Rgb32f {
    let mut mask = camera.clone();
    mask.map_in_place(|p| if p.iter().any(|v| *v >= SENSOR_CLIP) { [1.0; 3] } else { [0.0; 3] });
    mask
}

/// Sony's global tone curve with the Dynamic Range Optimizer off (Standard creative style), pooled
/// from 12 CC0 raw.pixls.us ARWs of 10 interchangeable-lens bodies and their camera JPEGs; the file
/// lists them and how the curve was derived (`assets/ATTRIBUTION.md`). Pooled the same way from 55
/// private ILCE-7CR photos with DRO off, the curve differs by at most 1.2 L* between scene luminance
/// 0.01 and 0.5. Issue storytold/lightcraft#244, `docs/camera-preview-colour.md`.
const SONY_DRO_OFF_TONE: &str = include_str!("../../assets/camera-tone/sony-dro-off.json");

/// [`SONY_DRO_OFF_TONE`]'s curve (`None` only if the built-in file were invalid; a test checks it).
pub(super) fn sony_dro_off_tone() -> Option<CameraTone> {
    #[derive(serde::Deserialize)]
    struct File {
        knots: [[f32; 2]; 32],
    }
    static TONE: std::sync::OnceLock<Option<CameraTone>> = std::sync::OnceLock::new();
    *TONE.get_or_init(|| serde_json::from_str::<File>(SONY_DRO_OFF_TONE).ok().and_then(|f| CameraTone::new(f.knots)))
}

/// `look`, fitted to a camera JPEG brightened by Sony's Dynamic Range Optimizer, with its tone
/// curve lowered to Sony's without DRO ([`SONY_DRO_OFF_TONE`]) wherever that is darker. DRO only
/// brightens, so the fitted curve bounds the curve without it from above, everywhere: where Sony's
/// curve is brighter than the photo's own (the 1″ compacts map raw values darker), the photo keeps
/// its own. Its colour (matrix, hue/saturation table and chroma curve) stays as fitted.
pub(super) fn with_dro_off_tone(look: CameraLook) -> CameraLook {
    CameraLook { tone: dro_off(&look.tone), ..look }
}

/// The curve of [`with_dro_off_tone`]: `own` lowered to Sony's without DRO wherever that is darker,
/// or `own` unchanged when it can't be.
pub(super) fn dro_off(own_tone: &CameraTone) -> CameraTone {
    let Some(dro_off) = sony_dro_off_tone() else { return *own_tone };
    let own = own_tone.knots();
    let mut knots = own.map(|[x, y]| [x, y.min(dro_off.apply(x))]);
    // Past its last knot a curve goes on as a shoulder whose rate comes from its last two knots
    // (`CameraTone::apply`): lowering the second-last knot more than the last steepens it, and the
    // extended highlights would end up above the fitted curve. Raise the second-last knot (never
    // above the fitted curve) until the rate is at most the fitted curve's. Process V5's shoulder
    // (`CameraTone::apply_rolloff`) is not part of this bound or of `at_most`: V1 to V4 read the
    // same curve, and a V5 bound here would change their renders. It stays below the fitted
    // curve's too when the last segment's slope over the squared gap to white is at most the fitted
    // curve's; a lowered curve with a nearly flat last segment can end up above it far past the last
    // knot under V5 (issue storytold/lightcraft#1632).
    let ([a, b], last) = ([own[30], own[31]], knots[31]);
    let rate = ((b[1] - a[1]) / (b[0] - a[0])).clamp(0.1, 16.0) / (1.0 - b[1]).max(0.01);
    let floor = last[1] - rate * (1.0 - last[1]).max(0.01) * (b[0] - a[0]);
    knots[30][1] = knots[30][1].max(floor).min(last[1]);
    let Some(tone) = CameraTone::new(knots).and_then(|tone| tone.with_chroma(*own_tone.chroma())) else { return *own_tone };
    if !at_most(&tone, own_tone) {
        return *own_tone;
    }
    tone
}

/// Whether curve `a` is nowhere brighter than `b` over the scene luminances the finish stage's tone
/// table covers ([`ToneMap::camera`]).
pub(super) fn at_most(a: &CameraTone, b: &CameraTone) -> bool {
    use crate::tone::{GREY, LUT_MAX_EV, LUT_MIN_EV, LUT_N};
    (0..LUT_N).all(|i| {
        let x = GREY * 2f32.powf(LUT_MIN_EV + (LUT_MAX_EV - LUT_MIN_EV) * i as f32 / (LUT_N - 1) as f32);
        a.apply(x) <= b.apply(x) + 1e-5
    })
}

/// Pixel cap for the embedded JPEG decoded for the fit. The codec checks it before decoding and
/// decodes at full size, so a 66.5 MP full-frame ILCE-7RM6 JPEG transiently needs ~200 MB (3 B/px);
/// 128 MP also admits 100 MP medium-format previews.
pub(super) const FIT_PREVIEW_MAX_PIXELS: u64 = 128_000_000;

/// A DNG's tone and chroma curves fitted to its own camera JPEG ([`dng_tone_eligible`]); the file's colour model
/// (matrix, profile tables, white balance) stays as it is: the proxy goes through the file's tables and the given
/// colour of the search is the identity. Same held-out acceptance gates as [`fit_look`]; the preview must not be a
/// thumbnail ([`MIN_PREVIEW_FRACTION`]), must show the same picture ([`structure_agrees`]), and the curve must not
/// clip more highlights than the plain render ([`clips_more`]).
pub(super) fn fit_dng_tone(
    raw: &RawImage,
    bytes: &[u8],
    transform: &CameraTransform,
    lens: Option<&craft_develop_model::EmbeddedLens>,
) -> Option<CameraLook> {
    // Logged under LIGHTCRAFT_PROFILE so a corpus run can count the outcome per gate.
    let refuse = |why: &str| {
        if std::env::var_os("LIGHTCRAFT_PROFILE").is_some() {
            eprintln!("DNGTONE refused {why}");
        }
        None
    };
    let Some((mut sensor, reference, clipped)) = proxies(raw, bytes, transform, PROXY, MIN_PREVIEW_FRACTION, None, None) else {
        return refuse("preview");
    };
    // As the loader renders it: the profile's tables around the baseline exposure.
    let xy = crate::sensor::color::as_shot_white_xy(raw);
    if let Some(tables) = crate::sensor::profile::ProfileTables::new(&raw.color.profile, crate::sensor::color::illuminant_weight(&raw.color, xy)) {
        let gain = 2f32.powf(transform.baseline_exposure as f32);
        sensor.map_in_place(|p| tables.apply(p.map(|v| v / gain), gain));
    }
    correct_vignetting(&mut sensor, lens, crate::sensor::orientation(raw.orientation));
    if (sensor.width, sensor.height) != (reference.width, reference.height) {
        return refuse("preview");
    }
    if !structure_agrees(&sensor, &reference) {
        return refuse("structure");
    }
    let unclipped = without_clipped(&sensor, &clipped);
    let fit_on = unclipped.as_ref().unwrap_or(&sensor);
    let colour = Some((Mat3::IDENTITY, None));
    let bright = HighlightCheck { pairs: highlights(&sensor, &reference), dro: false, grey: false };
    let look = TONE_FITS.iter().find_map(|&tone| {
        [false, true].into_iter().find_map(|away_from_edges| {
            fit_attempt(fit_on, &reference, &colour, Attempt { tone, profile: true, away_from_edges, dark: true }, &bright)
        })
    });
    let Some(look) = look else { return refuse("fit") };
    let plain = match raw.color.profile.tone_curve.as_ref().and_then(crate::load::dng_tone_curve) {
        Some(curve) => ToneMap::camera(&curve, 0.0, 0.0, 0.0),
        None => ToneMap::new(0.0, 0.0, 0.0),
    };
    let fitted = ToneMap::camera(&look.tone, 0.0, 0.0, 0.0);
    if clips_more(&sensor, &reference, &fitted, &plain) {
        return refuse("clip");
    }
    if std::env::var_os("LIGHTCRAFT_PROFILE").is_some() {
        eprintln!("DNGTONE accepted");
    }
    Some(look)
}

/// Whether the fitted tone map puts clearly more of the photo's proxy pixels at or near white than both the plain
/// render and the camera's own preview do ([`CLIP_TOLERANCE`]): a fit that clips what the camera clips too is the
/// camera's look, one that clips what neither does loses highlight detail.
pub(super) fn clips_more(sensor: &Rgb32f, reference: &Rgb32f, fitted: &ToneMap, plain: &ToneMap) -> bool {
    let near_white = |tone: &ToneMap| {
        let n = sensor.data.iter().filter(|p| p.iter().all(|v| v.is_finite()) && tone.apply(luminance_2020(p.map(|v| v.max(0.0)))) >= 0.98).count();
        n as f64 / sensor.data.len().max(1) as f64
    };
    let preview = reference.data.iter().filter(|p| luminance_2020(**p) >= 0.98).count() as f64 / reference.data.len().max(1) as f64;
    near_white(fitted) > near_white(plain).max(preview) + CLIP_TOLERANCE
}

/// The proxies a photo's look is fitted on ([`proxies`] at [`PROXY`]), the sensor's with the embedded vignetting
/// correction the render applies. Without it the tone curve learns the camera JPEG's corrected
/// brightness from the uncorrected sensor, and the render then corrects it again: 21 X-S10 RAFs rendered up to 0.3 EV
/// brighter in their midtones and 0.6 EV in their shadows than their camera JPEGs, about 0.05 EV once corrected.
pub(super) fn look_proxies(
    raw: &RawImage,
    bytes: &[u8],
    transform: &CameraTransform,
    lens: Option<&LensGeometry>,
    embedded: Option<&craft_develop_model::EmbeddedLens>,
) -> Option<(Rgb32f, Rgb32f, Vec<bool>)> {
    proxies(raw, bytes, transform, PROXY, 0.0, lens, embedded)
}

/// Multiply the sensor proxy (sensor orientation, default crop) by the embedded vignetting correction, the gain the
/// pipeline applies to the render (`optics::embedded_vignette_gain`; `lens` is in the oriented, cropped frame): the
/// camera JPEG has it applied, so a tone fitted without it brightens the render's corners twice.
pub(super) fn correct_vignetting(sensor: &mut Rgb32f, lens: Option<&craft_develop_model::EmbeddedLens>, orientation: craft_geom::Orientation) {
    let Some(vignette) = lens.and_then(|l| l.vignette) else { return };
    let (w, h) = (sensor.width as f64, sensor.height as f64);
    let (ow, oh) = if orientation.swaps_axes() { (h, w) } else { (w, h) };
    let width = sensor.width.max(1);
    for (i, p) in sensor.data.iter_mut().enumerate() {
        let (x, y) = orientation.map((i % width) as f64 + 0.5, (i / width) as f64 + 0.5, w, h);
        let gain = crate::optics::embedded_vignette_gain(&vignette, craft_geom::Point::new(x, y), ow, oh) as f32;
        *p = p.map(|v| v * gain);
    }
}
