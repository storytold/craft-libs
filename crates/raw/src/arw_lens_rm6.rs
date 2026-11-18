//! Sony ARW lens corrections from the tables in the raw IFD, as the equivalent DNG `OpcodeList3` opcodes, so they
//! reach the same "profile corrections" (`optics.lens_profile`) as a DNG's embedded warp.
//!
//! These are the ILCE-7RM6 conventions, measured against the camera JPEG. [`crate::arw_lens`] holds the ILCE-7RM4A
//! conventions, measured against Sony's Imaging Edge exports (unit 16384, the table indexed at the output radius).
//! Each is selected by camera model; no single unit fits every lens under either. See both docs:
//! docs/sony-ilce7rm6-lens-corrections.md (→ Two conventions) and docs/sony-lens-corrections.md.
//!
//! Sony records three count-prefixed signed 16-bit tables in the raw IFD, each with an in-camera on/off tag (ExifTool
//! Sony tag names): distortion [`DISTORTION_PARAMS`] (flag [`DISTORTION_FLAG`]), vignetting [`VIGNETTING_PARAMS`]
//! ([`VIGNETTING_FLAG`]) and lateral chromatic aberration [`CA_PARAMS`] ([`CA_FLAG`]). The first value is the knot
//! count: 16 / 16 / 32 in full-frame files of the ILCE-7RM6, 11 / 11 / 22 in its APS-C crop mode, the rest padding;
//! the CA table holds red's knots, then blue's.
//!
//! Their meaning was measured by registering blocks of the camera JPEG in each file on LightCraft's uncorrected render
//! (and raw data, for CA) over 54 ILCE-7RM6 photos with four lenses, 41 of them shot with in-camera distortion
//! correction on; nothing else (clean room). The conventions, with their worst residuals at a 1440 px long edge:
//! - knot `i` sits at sensor radius `i · s` from the default-crop centre, `s` = the body's full-frame default-crop
//!   half diagonal / 15 (the same pixel spacing in APS-C mode: the knots are fixed on the sensor); linear in between,
//!   the last segment extended beyond the last knot.
//! - distortion: a corrected radius `r` (px) reads the source radius `S(r) = r · (1 + v(r) / 15000)` (positive:
//!   pincushion), and the camera then zooms (output radius = `z` · corrected radius) so the output fills the default
//!   crop and reads only inside it: the largest `S(ρ/z)/ρ` over the crop border radii `ρ` is 1. 0.596 px over the 34
//!   measurable corrected photos (budget 0.75 px; corners and edge midpoints 0.570 px).
//! - `WarpRectilinear`'s 4-term odd polynomial fits that map within 2.35·10⁻⁴ of the DNG radius (0.220 px) on every
//!   photo, at its 65 fit samples. A table it fits worse than 0.25 px (0.5 px on an APS-C body) at the samples the crop shows is refused (the
//!   photo stays uncorrected rather than wrongly warped). Between samples the error is somewhat larger: 0.071 against
//!   0.049 px for the full-frame 16–35 mm example in the tests.
//! - vignetting: read but never applied. The camera JPEGs keep the uncorrected render's radial falloff (median
//!   corner/centre ratio 0.996 over 39 photos) and no radial gain model fits the tables.
//! - lateral CA: red's (blue's) source radius is `g · (1 + c(g) / 3.36·10⁶)` with `g` the green source radius and `c`
//!   the red (blue) half on the same knots. The unit is **provisional**: the measurement is borderline (0.399 px
//!   worst against a 0.3 px budget; per-photo best units scatter from 1.5·10⁶ to 7.2·10⁶, about ±30 % scene bias),
//!   but it cuts the rms raw-CFA CA over 46 photos from 0.079 to 0.052 px. The table is parsed and [`warp_opcode`]
//!   converts it to the warp's R and B planes, but they are **not emitted by default**: [`lens_opcodes`] passes
//!   `lateral_ca: false`, because applied after demosaic they made the render's CA worse (numbers there). To turn
//!   them on in code, pass `true` there. When on, a CA value beyond [`CA_MAX_RELATIVE`] or a colour plane that fails
//!   the fit gate above drops both colour planes, and G alone is applied to all three channels.
//!
//! The correction is produced whatever the in-camera flag says. Only bodies listed in [`FULL_FRAME_CROP`] or
//! [`APSC_BODIES`] get it: the unit and the knot spacing are verified on the ILCE-7RM6 and, for distortion, on the
//! ILCE-6700 (APS-C, 11 knots spanning its own crop, the same unit 15000: 0.73 px worst over 27 photos with three
//! lenses as rendered, 1.38 px at the 10–18 mm's 10 mm end). Another body is added by repeating the measurement on its photos shot
//! with distortion correction on and listing its full-frame default crop (or, for an APS-C body, its model). The method
//! and the numbers: docs/sony-ilce7rm6-lens-corrections.md.

use crate::opcodes::max_radius;
use crate::shared::fit_radial_warp;
use crate::{Opcode, Rect};
use craft_tiff::Ifd;
use std::ops::RangeInclusive;

pub(crate) const DISTORTION_PARAMS: u16 = 0x7037;
pub(crate) const VIGNETTING_PARAMS: u16 = 0x7032;
pub(crate) const CA_PARAMS: u16 = 0x7035;
pub(crate) const DISTORTION_FLAG: u16 = 0x7036;
pub(crate) const VIGNETTING_FLAG: u16 = 0x7031;
pub(crate) const CA_FLAG: u16 = 0x7034;

/// The bodies whose tables are verified (Exif `Model`), with their full-frame default crop (width, height) in px.
const FULL_FRAME_CROP: &[(&str, (f64, f64))] = &[("ILCE-7RM6", (9984.0, 6656.0))];
/// The verified APS-C bodies (Exif `Model`): their knots span the file's own default crop, the last knot at its corner.
/// Measured on the ILCE-6700 (docs/sony-ilce7rm6-lens-corrections.md → ILCE-6700).
const APSC_BODIES: &[&str] = &["ILCE-6700"];

/// A lens whose tables follow its body's convention with another knot spacing and unit, at the focal lengths up to
/// `max_focal_mm`; measured like the body, against the camera JPEG.
struct LensException {
    /// Exif `Model` of the body.
    body: &'static str,
    /// Exif `LensModel`, as the body records it.
    lens: &'static str,
    max_focal_mm: f64,
    knot_spacing: f64,
    unit: f64,
}

/// The measured lens exceptions (docs/sony-ilce7rm6-lens-corrections.md → ILCE-6700).
const LENS_EXCEPTIONS: &[LensException] = &[
    // Sigma 10-18 mm F2.8 DC DN at its wide end: 97 photos at 10 mm and 2 at 10.4-10.5 mm. The body's rule leaves
    // 1.36-1.38 px there; from 11.7 mm on the body's rule fits better. `lens` is the only LensModel in 336 files of
    // this lens (native E-mount, no adapter); the 116 measured copies carry body firmware "ILCE-6700 v2.00"
    LensException { body: "ILCE-6700", lens: "10-18mm F2.8 DC DN | Contemporary 023", max_focal_mm: 10.5, knot_spacing: 416.0, unit: 14250.0 },
];

/// The exception for the body `model` with the lens `lens` at `focal_mm`, if one is measured.
fn lens_exception(model: &str, lens: Option<&str>, focal_mm: Option<f64>) -> Option<&'static LensException> {
    let (lens, focal) = (lens?.trim(), focal_mm.filter(|f| f.is_finite() && *f > 0.0)?);
    LENS_EXCEPTIONS.iter().find(|e| e.body == model.trim() && e.lens == lens && focal <= e.max_focal_mm)
}
/// The distortion table's value of 1 (relative radius); a global least-squares fit over the 34 photos gives 15000.7.
const DISTORTION_UNIT: f64 = 15000.0;
/// The CA table's value of 1 (relative to the green source radius). Provisional: the median best unit of the 20
/// cleanest photos; per-photo units scatter from 1.5·10⁶ to 7.2·10⁶ (the measurement's scene bias is about ±30 %).
const CA_UNIT: f64 = 3.36e6;
/// Largest plausible CA value relative to the green source radius (|c| / [`CA_UNIT`]): about 12 px at the full-frame
/// corner; the largest value in the 54 measured photos' tables is 1724 (FE 24–70 mm), 1724 / 3.36·10⁶ ≈ 5.1·10⁻⁴.
/// A table beyond it is refused.
const CA_MAX_RELATIVE: f64 = 2e-3;
/// Crop border radii scanned by the frame-filling zoom.
const ZOOM_SCAN: usize = 4096;
/// Largest polynomial fit error accepted at the fit samples the crop shows, px at a 1440 px long edge.
const FIT_ERROR_PX_AT_1440: f64 = 0.25;
/// The fit gate of [`APSC_BODIES`]: their 11 knots over a smaller crop bend more sharply than the polynomial follows
/// (worst 0.44 px over the 30 measured ILCE-6700 tables; 13 of them exceed 0.25 px), while the fitted polynomial
/// matches the camera JPEG as well as the exact table (p95 0.22–0.73 px against 0.23–0.68 px, 10 mm aside).
const APSC_FIT_ERROR_PX_AT_1440: f64 = 0.5;

/// A file's lens tables: the first `count` values of each (CA: red, blue); `None` when absent or malformed.
/// `vignetting` and `flags` are parsed for the tests and the docs only: don't build an opcode from them (the camera
/// applies no vignetting correction to its JPEGs, and the flags don't decide anything).
#[derive(Debug)]
// vignetting and the flags never become an opcode
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct LensTables {
    pub distortion: Option<Vec<f64>>,
    pub vignetting: Option<Vec<f64>>,
    pub ca: Option<(Vec<f64>, Vec<f64>)>,
    /// In-camera on/off of distortion, vignetting and CA correction, as recorded.
    pub flags: [Option<i64>; 3],
}

/// The values of a count-prefixed integer table whose count is in `counts` and fits the array.
fn table(raw: &Ifd, tag: u16, counts: RangeInclusive<usize>) -> Option<Vec<f64>> {
    let v = raw.value(tag)?;
    let count = usize::try_from(v.get_i64(0)?).ok().filter(|n| counts.contains(n) && *n < v.count())?;
    (1..=count).map(|i| v.get_i64(i).map(|x| x as f64)).collect()
}

/// The lens tables of the raw IFD `raw`.
pub(crate) fn read_tables(raw: &Ifd) -> LensTables {
    let ca = table(raw, CA_PARAMS, 4..=32).filter(|v| v.len().is_multiple_of(2)).map(|v| {
        let (red, blue) = v.split_at(v.len() / 2);
        (red.to_vec(), blue.to_vec())
    });
    LensTables {
        distortion: table(raw, DISTORTION_PARAMS, 2..=16),
        vignetting: table(raw, VIGNETTING_PARAMS, 2..=16),
        ca,
        flags: [raw.i64(DISTORTION_FLAG), raw.i64(VIGNETTING_FLAG), raw.i64(CA_FLAG)],
    }
}

/// The knot spacing (px) of a verified body: a full-frame body's full-frame default-crop half diagonal / 15 (fixed on
/// the sensor, so in its APS-C crop mode too); an APS-C body's default-crop `crop` half diagonal / (`knots` − 1). Both
/// put the last knot at the corner of the crop the table was written for (the ILCE-7RM6's APS-C mode, 11 knots, gives
/// 392 px that way, which fits its files as well as the sensor-fixed 400 px). `None` for an unverified body, or an
/// APS-C body with fewer than two knots or a crop that isn't 3:2 (the only shape measured; empty crops included).
fn knot_spacing(model: &str, knots: usize, crop: Rect) -> Option<f64> {
    let model = model.trim();
    if let Some(&(_, (w, h))) = FULL_FRAME_CROP.iter().find(|(m, _)| *m == model) {
        return Some(w.hypot(h) / 30.0);
    }
    let gaps = knots.checked_sub(1).filter(|&n| n > 0)?;
    let half_diagonal = (crop.width as f64).hypot(crop.height as f64) / 2.0;
    (APSC_BODIES.contains(&model) && crate::arw_lens::is_3_2(crop)).then(|| half_diagonal / gaps as f64)
}

/// The knot table at radius `r` px: knot `i` at `i · spacing`, linear in between, the last segment extended beyond
/// the last knot. NaN with fewer than two knots.
fn interpolate(knots: &[f64], spacing: f64, r: f64) -> f64 {
    let t = r / spacing;
    // `as` saturates, so a huge `t` lands on the last segment; NaN fails `t > 0`
    let i = if t > 0.0 { (t as usize).min(knots.len().saturating_sub(2)) } else { 0 };
    match (knots.get(i), knots.get(i + 1)) {
        (Some(a), Some(b)) => a + (b - a) * (t - i as f64),
        _ => f64::NAN,
    }
}

/// The camera's frame-filling zoom `z` for the source radius map `source` and a crop of half size `x × y`: the
/// largest `source(ρ/z)/ρ` over the border radii `ρ` in [min(x, y), hypot(x, y)] is 1, so the output crop reads
/// only inside the source crop and touches it (at the corner for pincushion, the short-edge midpoint for barrel).
/// Bisection over [0.5, 2]; `None` when that range holds no solution or a ratio is not finite.
fn fill_zoom(source: &dyn Fn(f64) -> f64, x: f64, y: f64) -> Option<f64> {
    let (near, far) = (x.min(y), x.hypot(y));
    let overshoots = |z: f64| -> Option<bool> {
        let mut worst = f64::NEG_INFINITY;
        for k in 0..=ZOOM_SCAN {
            let rb = near + k as f64 * (far - near) / ZOOM_SCAN as f64;
            let ratio = source(rb / z) / rb;
            if !ratio.is_finite() {
                return None;
            }
            worst = worst.max(ratio);
        }
        Some(worst > 1.0)
    };
    let (mut lo, mut hi) = (0.5, 2.0);
    if !overshoots(lo)? || overshoots(hi)? {
        return None;
    }
    for _ in 0..100 {
        let z = 0.5 * (lo + hi);
        if overshoots(z)? { lo = z } else { hi = z }
    }
    Some(0.5 * (lo + hi))
}

/// The distortion and CA tables as a `WarpRectilinear` relative to the active area `active` (centred on the default
/// crop `crop`, as the camera corrects), with knots `knot_spacing` px apart: planes R, G, B with `lateral_ca`, or G
/// alone (applied to all three) without it, or when the CA table is absent, implausible or a colour plane's fit
/// fails. `None` when the distortion table is absent or
/// all zero (CA alone is not applied: it is relative to green), the zoom or the G fit fails. A fit fails when it is
/// worse than 0.25 px at a 1440 px long edge at one of the fit samples within the crop (a padded active area, as in
/// lossless files, reaches radii that are never shown).
pub(crate) fn warp_opcode(t: &LensTables, knot_spacing: f64, active: Rect, crop: Rect, lateral_ca: bool) -> Option<Opcode> {
    gated_warp_opcode(t, knot_spacing, DISTORTION_UNIT, active, crop, lateral_ca, FIT_ERROR_PX_AT_1440)
}

/// [`warp_opcode`] with the distortion unit `unit` instead of 15000 and the fit gate `fit_error_px` (px at a 1440 px
/// long edge) instead of 0.25 px.
fn gated_warp_opcode(t: &LensTables, knot_spacing: f64, unit: f64, active: Rect, crop: Rect, lateral_ca: bool, fit_error_px: f64) -> Option<Opcode> {
    let v = t.distortion.as_deref().filter(|v| v.iter().any(|&x| x != 0.0))?;
    let (aw, ah) = (active.width as f64, active.height as f64);
    if aw < 2.0 || ah < 2.0 || crop.width == 0 || crop.height == 0 {
        return None;
    }
    let (cw, ch) = (crop.width as f64, crop.height as f64);
    let source = |r: f64| r * (1.0 + interpolate(v, knot_spacing, r) / unit);
    let z = fill_zoom(&source, cw / 2.0, ch / 2.0)?;
    // DNG's normalisation: pixel-index units, radius 1 = the farthest active-area corner
    let (cx, cy) = (crop.x as f64 + (cw - 1.0) / 2.0, crop.y as f64 + (ch - 1.0) / 2.0);
    let m = max_radius(cx, cy, aw - 1.0, ah - 1.0);
    let max_fit_error = fit_error_px * (cw.max(ch) / 1440.0) / m;
    let shown = (cw / 2.0).hypot(ch / 2.0) / m;
    // output radius (px) → green source radius
    let green = |r: f64| source(r / z);
    let plane = |map: &dyn Fn(f64) -> f64| {
        let [k0, k1, k2, k3] = fit_radial_warp(&|u| map(u * m) / m, max_fit_error, shown)?;
        Some([k0, k1, k2, k3, 0.0, 0.0])
    };
    let g = plane(&green)?;
    // red and blue: relative to the green source radius, at that radius
    let colour = |c: &[f64]| {
        plane(&|r| {
            let s = green(r);
            s * (1.0 + interpolate(c, knot_spacing, s) / CA_UNIT)
        })
    };
    // a NaN fails the comparison: refused too
    let plausible = |c: &[f64]| c.iter().all(|v| v.abs() <= CA_MAX_RELATIVE * CA_UNIT);
    let rgb = t.ca.as_ref().filter(|(red, blue)| lateral_ca && plausible(red) && plausible(blue));
    let rgb = rgb.and_then(|(red, blue)| Some(vec![colour(red)?, g, colour(blue)?]));
    Some(Opcode::WarpRectilinear { planes: rgb.unwrap_or_else(|| vec![g]), center: [cx / (aw - 1.0), cy / (ah - 1.0)] })
}

/// The `OpcodeList3` lens corrections of a Sony raw IFD for the body `model` (Exif `Model`); empty for a body whose
/// tables are not verified.
///
/// Distortion only: the lateral CA planes are off (`lateral_ca: false`). Our demosaic already hides about 70 % of the
/// lens CA (the render shows 0.29 × the CA measured on the raw CFA), while the warp, which runs after demosaic, moves
/// R and B by 0.84 × the table. Over the 43 verified photos with enough corner detail, the render's corner R/G and B/G
/// CA went from rms 0.032 / max 0.125 px (off) to 0.052 / 0.219 px (on) at a 1440 px long edge; only the 16–35 mm
/// improved. Follow-up: warp the CFA colour planes before demosaic, then turn the planes on here.
///
/// `lens` (Exif `LensModel`) and `focal_mm` (Exif `FocalLength`) select a measured lens exception's spacing and unit.
pub(crate) fn lens_opcodes(raw: &Ifd, model: &str, lens: Option<&str>, focal_mm: Option<f64>, active: Rect, crop: Rect) -> Vec<Opcode> {
    let tables = read_tables(raw);
    let knots = tables.distortion.as_ref().map_or(0, Vec::len);
    let Some(spacing) = knot_spacing(model, knots, crop) else { return Vec::new() };
    let op = if let Some(e) = lens_exception(model, lens, focal_mm) {
        gated_warp_opcode(&tables, e.knot_spacing, e.unit, active, crop, false, APSC_FIT_ERROR_PX_AT_1440)
    } else if APSC_BODIES.contains(&model.trim()) {
        gated_warp_opcode(&tables, spacing, DISTORTION_UNIT, active, crop, false, APSC_FIT_ERROR_PX_AT_1440)
    } else {
        warp_opcode(&tables, spacing, active, crop, false)
    };
    op.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft_tiff::{Entry, Value};

    /// The FF 16–35 mm example (corpus `arw-sony-ilce7rm6-ff-land-lossless.arw`, docs/sony-ilce7rm6-lens-corrections.md).
    const FF_DISTORTION: [i16; 17] = [16, 0, 2, 7, 16, 28, 43, 61, 83, 109, 139, 175, 217, 267, 325, 395, 479];
    const FF_VIGNETTING: [i16; 17] = [16, 0, 80, 336, 672, 1120, 1648, 2240, 2880, 3600, 4336, 5104, 5872, 6640, 7392, 8160, 8976];
    const FF_CA: [i16; 33] = [
        32, 242, 234, 224, 212, 196, 176, 154, 132, 108, 86, 66, 46, 30, 16, 8, 4, -276, -258, -238, -212, -184, -148, -114, -74, -40, -6, 20, 42,
        56, 60, 54, 32,
    ];
    /// The CA table of corpus `arw6-sony-ilce7rm6-ff-land-hq.arw` (the same scene; same distortion table).
    const FF_HQ_CA: [i16; 33] = [
        32, 242, 234, 222, 208, 192, 172, 152, 128, 106, 82, 62, 42, 26, 12, 4, 2, -270, -254, -234, -208, -180, -146, -112, -72, -38, -4, 22, 44,
        58, 64, 56, 36,
    ];
    /// The APS-C example (corpus `arw6-sony-ilce7rm6-apsc-land-hq.arw`): counts 11 / 11 / 22, then padding.
    const APSC_DISTORTION: [i16; 17] = [11, 0, -10, -36, -78, -134, -202, -277, -356, -435, -512, -583, -583, -583, -583, -583, -583];
    const APSC_VIGNETTING: [i16; 17] = [11, 0, 48, 208, 496, 880, 1360, 1952, 2624, 3568, 4832, 6176, 0, 0, 0, 0, 0];
    const APSC_CA: [i16; 33] = [
        22, 790, 764, 734, 702, 666, 624, 582, 544, 510, 484, 470, -432, -404, -372, -330, -286, -230, -174, -114, -58, -6, 34, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0,
    ];
    /// The APS-C lossless corpus file `arw-sony-ilce7rm6-apsc-land-lossless.arw` (same scene; its tables differ slightly).
    const APSC_LOSSLESS_DISTORTION: [i16; 17] = [11, 0, -10, -36, -78, -134, -202, -277, -356, -435, -511, -583, -583, -583, -583, -583, -583];
    const APSC_LOSSLESS_CA: [i16; 33] = [
        22, 792, 766, 738, 706, 670, 630, 588, 548, 514, 488, 476, -436, -408, -376, -334, -290, -234, -178, -118, -62, -12, 30, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0,
    ];
    /// ARW6 geometry (the decoder's raw image size and default crop).
    const FF_ACTIVE: Rect = Rect { x: 0, y: 0, width: 10016, height: 6672 };
    const FF_CROP: Rect = Rect { x: 12, y: 8, width: 9984, height: 6656 };
    const APSC_ACTIVE: Rect = Rect { x: 0, y: 0, width: 6592, height: 4372 };
    const APSC_CROP: Rect = Rect { x: 16, y: 10, width: 6528, height: 4352 };
    /// The G planes of the examples on their ARW6 geometry, and the R and B planes with each file's CA table
    /// (docs/sony-ilce7rm6-lens-corrections.md).
    const FF_G: [f64; 4] = [0.9714495823, 0.022508032, 0.0006090085237, 0.005683284548];
    const FF_HQ_R: [f64; 4] = [0.9715153129, 0.02236266369, 0.0007060224765, 0.005666341563];
    const FF_HQ_B: [f64; 4] = [0.97137946, 0.02277016389, 0.0003830082563, 0.005727820598];
    const APSC_G: [f64; 4] = [1.01627551, -0.0618606692, 0.02768349621, -0.005595798468];
    const APSC_R: [f64; 4] = [1.016503991, -0.06206460153, 0.0278447625, -0.005644118171];
    const APSC_B: [f64; 4] = [1.016155702, -0.06162477627, 0.02753315326, -0.005554915901];

    fn knots() -> f64 {
        knot_spacing("ILCE-7RM6", 16, FF_CROP).unwrap()
    }

    fn ifd(tags: Vec<(u16, Value)>) -> Ifd {
        let mut entries: Vec<Entry> = tags.into_iter().map(|(tag, value)| Entry { tag, value, offset: 0 }).collect();
        entries.sort_by_key(|e| e.tag);
        Ifd { entries, ..Default::default() }
    }

    fn sony_lens_ifd(distortion: &[i16], vignetting: &[i16], ca: &[i16], flags: [u16; 3]) -> Ifd {
        ifd(vec![
            (DISTORTION_PARAMS, Value::SShort(distortion.to_vec())),
            (VIGNETTING_PARAMS, Value::SShort(vignetting.to_vec())),
            (CA_PARAMS, Value::SShort(ca.to_vec())),
            (DISTORTION_FLAG, Value::Short(vec![flags[0]])),
            (VIGNETTING_FLAG, Value::Short(vec![flags[1]])),
            (CA_FLAG, Value::Short(vec![flags[2]])),
        ])
    }

    fn tables_with_distortion(v: &[f64]) -> LensTables {
        LensTables { distortion: Some(v.to_vec()), vignetting: None, ca: None, flags: [None; 3] }
    }

    fn values(table: &[i16]) -> Vec<f64> {
        table[1..=table[0] as usize].iter().map(|&v| v as f64).collect()
    }

    /// The warp's planes and centre against the recipe's numbers (docs/sony-ilce7rm6-lens-corrections.md); returns the planes.
    fn assert_warp(op: Option<Opcode>, center: [f64; 2], k: &[[f64; 4]]) -> Vec<[f64; 6]> {
        let Some(Opcode::WarpRectilinear { planes, center: c }) = op else { panic!("no warp: {op:?}") };
        assert_eq!(planes.len(), k.len());
        assert!((c[0] - center[0]).abs() < 1e-9 && (c[1] - center[1]).abs() < 1e-9, "centre {c:?}, want {center:?}");
        for (p, k) in planes.iter().zip(k) {
            for (i, want) in k.iter().enumerate() {
                assert!((p[i] - want).abs() < 1e-6, "k{i} {}, want {want} ({p:?})", p[i]);
            }
            assert_eq!((p[4], p[5]), (0.0, 0.0), "radial only");
        }
        planes
    }

    #[test]
    fn reads_the_three_tables_and_flags() {
        let t = read_tables(&sony_lens_ifd(&FF_DISTORTION, &FF_VIGNETTING, &FF_CA, [0, 2, 1]));
        let d = t.distortion.unwrap();
        assert_eq!((d.len(), d[0], d[15]), (16, 0.0, 479.0));
        let v = t.vignetting.unwrap();
        assert_eq!((v.len(), v[1], v[15]), (16, 80.0, 8976.0));
        let (red, blue) = t.ca.unwrap();
        assert_eq!((red.len(), red[0], red[15]), (16, 242.0, 4.0));
        assert_eq!((blue.len(), blue[0], blue[15]), (16, -276.0, 32.0));
        assert_eq!(t.flags, [Some(0), Some(2), Some(1)]);
    }

    #[test]
    fn apsc_tables_use_only_the_first_count_values() {
        let t = read_tables(&sony_lens_ifd(&APSC_DISTORTION, &APSC_VIGNETTING, &APSC_CA, [0, 2, 1]));
        let d = t.distortion.unwrap();
        assert_eq!((d.len(), d[10]), (11, -583.0));
        assert_eq!(t.vignetting.unwrap().len(), 11);
        let (red, blue) = t.ca.unwrap();
        assert_eq!((red.len(), red[10]), (11, 470.0));
        assert_eq!((blue.len(), blue[0], blue[10]), (11, -432.0, 34.0));
    }

    #[test]
    fn missing_or_malformed_tables_yield_no_opcode() {
        let mut short = FF_DISTORTION.to_vec();
        short.pop();
        let mut count_17 = FF_DISTORTION.to_vec();
        count_17[0] = 17;
        count_17.push(500);
        let mut too_large = FF_DISTORTION.to_vec();
        too_large[0] = 600;
        for (what, table) in [
            ("count 15 with 3 values", vec![15, 0, 2, 7]),
            ("count 17", count_17),
            ("empty", vec![]),
            ("count 16 with 15 values", short),
            ("count 1", vec![1, 5]),
            ("count 0", vec![0, 5, 6]),
            ("negative count", vec![-16, 0, 2, 7]),
            ("count larger than the array", too_large),
        ] {
            let ifd = ifd(vec![(DISTORTION_PARAMS, Value::SShort(table))]);
            let t = read_tables(&ifd);
            assert!(t.distortion.is_none(), "{what}");
            assert!(warp_opcode(&t, knots(), FF_ACTIVE, FF_CROP, false).is_none(), "{what}");
            assert!(lens_opcodes(&ifd, "ILCE-7RM6", None, None, FF_ACTIVE, FF_CROP).is_empty(), "{what}");
        }
        // not an integer array, or absent
        for value in [Value::Ascii("16 0 2 7".into()), Value::Double(vec![2.0, 0.0, 1.0])] {
            assert!(read_tables(&ifd(vec![(DISTORTION_PARAMS, value)])).distortion.is_none());
        }
        let none = read_tables(&ifd(vec![]));
        assert!(none.distortion.is_none() && none.vignetting.is_none() && none.ca.is_none() && none.flags == [None; 3]);
        // the CA table needs an even count of 4..=32
        for ca in [vec![3, 1, 2, 3], vec![34; 35], vec![2, 1, 2]] {
            assert!(read_tables(&ifd(vec![(CA_PARAMS, Value::SShort(ca))])).ca.is_none());
        }
    }

    #[test]
    fn hostile_tables_are_rejected() {
        let folded = [0.0, 2000.0, 1000.0, 3000.0, 2000.0, 4000.0, 3000.0, 5000.0, 4000.0, 6000.0, 5000.0, 7000.0, 6000.0, 8000.0, 7000.0, 9000.0];
        for (what, table) in [
            ("huge", vec![(i64::MAX / 4) as f64; 16]),
            ("most negative", vec![i64::MIN as f64; 16]),
            ("zig-zag", folded.to_vec()),
            ("all zero", vec![0.0; 16]),
            ("one knot", vec![100.0]),
            ("no knots", vec![]),
            ("not finite", vec![0.0, f64::NAN, 20.0]),
            ("infinite", vec![0.0, f64::INFINITY, 20.0]),
        ] {
            assert!(warp_opcode(&tables_with_distortion(&table), knots(), FF_ACTIVE, FF_CROP, false).is_none(), "{what}");
        }
        // the same through the IFD, as 64-bit values
        for v in [i64::MAX / 4, i64::MIN, i64::MAX] {
            let mut table = vec![v; 17];
            table[0] = 16;
            let ifd = ifd(vec![(DISTORTION_PARAMS, Value::SLong8(table))]);
            assert!(lens_opcodes(&ifd, "ILCE-7RM6", None, None, FF_ACTIVE, FF_CROP).is_empty(), "{v}");
        }
        // degenerate geometry
        let ff = tables_with_distortion(&values(&FF_DISTORTION));
        for (what, knots, active, crop) in [
            ("empty crop", knots(), FF_ACTIVE, Rect::new(12, 8, 0, 6656)),
            ("1 × 1 active area", knots(), Rect::new(0, 0, 1, 1), Rect::new(0, 0, 1, 1)),
            ("knot spacing NaN", f64::NAN, FF_ACTIVE, FF_CROP),
            ("knot spacing 0", 0.0, FF_ACTIVE, FF_CROP),
        ] {
            assert!(warp_opcode(&ff, knots, active, crop, false).is_none(), "{what}");
        }
    }

    #[test]
    fn ff_example_table_becomes_the_measured_warp() {
        let t = tables_with_distortion(&values(&FF_DISTORTION));
        assert_warp(warp_opcode(&t, knots(), FF_ACTIVE, FF_CROP, false), [0.4996005991, 0.5], &[FF_G]);
    }

    #[test]
    fn apsc_example_table_becomes_the_measured_warp() {
        let t = tables_with_distortion(&values(&APSC_DISTORTION));
        let op = warp_opcode(&t, knots(), APSC_ACTIVE, APSC_CROP, false);
        assert_warp(op, [0.4975724473, 0.5], &[APSC_G]);
    }

    #[test]
    fn ca_table_shifts_red_and_blue_planes() {
        let t = read_tables(&sony_lens_ifd(&FF_DISTORTION, &FF_VIGNETTING, &FF_HQ_CA, [0, 2, 1]));
        let planes = assert_warp(warp_opcode(&t, knots(), FF_ACTIVE, FF_CROP, true), [0.4996005991, 0.5], &[FF_HQ_R, FF_G, FF_HQ_B]);
        assert_ne!(planes[0][0], planes[1][0]);
        // G is the distortion plane alone
        let g = warp_opcode(&tables_with_distortion(&values(&FF_DISTORTION)), knots(), FF_ACTIVE, FF_CROP, false);
        assert!(matches!(g, Some(Opcode::WarpRectilinear { planes: ref p, .. }) if p == &[planes[1]]), "{g:?}");
    }

    #[test]
    fn apsc_ca_planes_match_the_measured_warp() {
        let t = read_tables(&sony_lens_ifd(&APSC_DISTORTION, &APSC_VIGNETTING, &APSC_CA, [0, 2, 1]));
        assert_warp(warp_opcode(&t, knots(), APSC_ACTIVE, APSC_CROP, true), [0.4975724473, 0.5], &[APSC_R, APSC_G, APSC_B]);
    }

    #[test]
    fn ca_without_distortion_is_not_applied() {
        let ifd = ifd(vec![(CA_PARAMS, Value::SShort(FF_HQ_CA.to_vec()))]);
        let t = read_tables(&ifd);
        assert!(t.ca.is_some() && t.distortion.is_none());
        assert!(warp_opcode(&t, knots(), FF_ACTIVE, FF_CROP, true).is_none());
        assert!(lens_opcodes(&ifd, "ILCE-7RM6", None, None, FF_ACTIVE, FF_CROP).is_empty());
        // an all-zero distortion table counts as absent
        let zero = LensTables { ca: t.ca, ..tables_with_distortion(&[0.0; 16]) };
        assert!(warp_opcode(&zero, knots(), FF_ACTIVE, FF_CROP, true).is_none());
    }

    #[test]
    fn rejected_ca_keeps_the_single_green_plane() {
        let green = assert_warp(
            warp_opcode(&tables_with_distortion(&values(&FF_DISTORTION)), knots(), FF_ACTIVE, FF_CROP, false),
            [0.4996005991, 0.5],
            &[FF_G],
        );
        let ok = values(&FF_HQ_CA);
        let huge = vec![(i64::MAX / 4) as f64; 16];
        for (what, red, blue) in [
            ("huge red", huge.clone(), ok[16..].to_vec()),
            ("huge blue", ok[..16].to_vec(), huge),
            ("most negative blue", ok[..16].to_vec(), vec![i64::MIN as f64; 16]),
            ("not finite red", vec![0.0, f64::NAN, 20.0], ok[16..].to_vec()),
            ("one knot", vec![100.0], vec![100.0]),
        ] {
            let t = LensTables { ca: Some((red, blue)), ..tables_with_distortion(&values(&FF_DISTORTION)) };
            let Some(Opcode::WarpRectilinear { planes, .. }) = warp_opcode(&t, knots(), FF_ACTIVE, FF_CROP, true) else { panic!("{what}: no warp") };
            assert_eq!(planes, green, "{what}");
        }
        // through the IFD: an odd count, or values that overflow the fit
        let mut odd = FF_HQ_CA.to_vec();
        odd[0] = 31;
        let mut huge_ifd = vec![i64::MAX / 4; 33];
        huge_ifd[0] = 32;
        // a constant 32767 (the SShort maximum) fits as well as G does and would shift red ~59 px at the full-res
        // corner: refused as implausible (bound 2·10⁻³ relative = 6720)
        let mut max = vec![32767; 33];
        max[0] = 32;
        for (what, ca) in [("odd count", Value::SShort(odd)), ("huge", Value::SLong8(huge_ifd)), ("constant 32767", Value::SShort(max))] {
            let ifd = ifd(vec![(DISTORTION_PARAMS, Value::SShort(FF_DISTORTION.to_vec())), (CA_PARAMS, ca)]);
            let op = warp_opcode(&read_tables(&ifd), knots(), FF_ACTIVE, FF_CROP, true);
            assert!(matches!(op, Some(Opcode::WarpRectilinear { ref planes, .. }) if planes == &green), "{what}: {op:?}");
        }
        // values up to the bound are kept
        let mut at_bound = vec![6720; 33];
        at_bound[0] = 32;
        let ifd = ifd(vec![(DISTORTION_PARAMS, Value::SShort(FF_DISTORTION.to_vec())), (CA_PARAMS, Value::SShort(at_bound))]);
        let op = warp_opcode(&read_tables(&ifd), knots(), FF_ACTIVE, FF_CROP, true);
        assert!(matches!(op, Some(Opcode::WarpRectilinear { ref planes, .. }) if planes.len() == 3), "{op:?}");
    }

    /// The default: a valid CA table is read and converts (above), but `lens_opcodes` emits the G plane alone.
    #[test]
    fn lens_opcodes_emit_the_green_plane_only() {
        let ifd = sony_lens_ifd(&FF_DISTORTION, &FF_VIGNETTING, &FF_HQ_CA, [1, 2, 1]);
        let with_ca = warp_opcode(&read_tables(&ifd), knots(), FF_ACTIVE, FF_CROP, true);
        assert!(matches!(with_ca, Some(Opcode::WarpRectilinear { ref planes, .. }) if planes.len() == 3), "{with_ca:?}");
        let ops = lens_opcodes(&ifd, "ILCE-7RM6", None, None, FF_ACTIVE, FF_CROP);
        let [Opcode::WarpRectilinear { planes, .. }] = ops.as_slice() else { panic!("{ops:?}") };
        assert_eq!(planes.len(), 1);
        let green = warp_opcode(&tables_with_distortion(&values(&FF_DISTORTION)), knots(), FF_ACTIVE, FF_CROP, false);
        assert_eq!(ops.first(), green.as_ref());
    }

    /// The corpus lossless files' own tables on their decoded geometry (padded active areas; the corpus test checks
    /// the G plane from the files): R, G, B with the CA planes enabled.
    #[test]
    fn lossless_examples_ca_planes_match_the_measured_warp() {
        let ff = read_tables(&sony_lens_ifd(&FF_DISTORTION, &FF_VIGNETTING, &FF_CA, [0, 2, 1]));
        let ff_active = Rect { x: 0, y: 0, width: 10240, height: 7168 };
        assert_warp(
            warp_opcode(&ff, knots(), ff_active, FF_CROP, true),
            [0.488670768629749, 0.4653969582810102],
            &[
                [0.971770511193381, 0.023073568897462152, 0.00879158563716919, 0.0030468782543633132],
                [0.9717033132764651, 0.023249297636569695, 0.008638979168841135, 0.003091041831374855],
                [0.9716312798157171, 0.02356676356656341, 0.00830169937204417, 0.003182927930635947],
            ],
        );
        let apsc = read_tables(&sony_lens_ifd(&APSC_LOSSLESS_DISTORTION, &APSC_VIGNETTING, &APSC_LOSSLESS_CA, [0, 2, 1]));
        let apsc_active = Rect { x: 0, y: 0, width: 6656, height: 4608 };
        assert_warp(
            warp_opcode(&apsc, knots(), apsc_active, APSC_CROP, true),
            [0.49278737791134486, 0.4743868026915563],
            &[
                [1.0165959888027158, -0.0690974265041942, 0.036328960724338874, -0.009486844437228997],
                [1.016366854640611, -0.0688824189782517, 0.03615745253291194, -0.009438825010476538],
                [1.0162457267369314, -0.0686225599869027, 0.035974238433727206, -0.009384155126102897],
            ],
        );
    }

    /// The lossless files' padded active area (10240 × 7168) puts the farthest active-area corner (DNG radius 1) well
    /// beyond the crop's (u 0.925): the gate looks only at the radii the crop shows. The FF example × 1.5 fits within
    /// 0.12 px at 1440 at the samples there, 0.27 px at all 65.
    #[test]
    fn padded_active_area_gates_only_the_displayed_radii() {
        let strong: Vec<f64> = values(&FF_DISTORTION).iter().map(|v| v * 1.5).collect();
        let padded = Rect { x: 0, y: 0, width: 10240, height: 7168 };
        let op = warp_opcode(&tables_with_distortion(&strong), knots(), padded, FF_CROP, false);
        assert!(matches!(op, Some(Opcode::WarpRectilinear { .. })), "{op:?}");
    }

    /// An ILCE-6700 file (Sigma 10–18 mm at 10 mm): 11 knots, the strongest table of the measured set.
    const A6700_DISTORTION: [i16; 17] = [11, 90, 11, -90, -205, -345, -500, -676, -852, -1031, -1190, -1333, -1333, -1333, -1333, -1333, -1333];
    const A6700_CROP: Rect = Rect { x: 0, y: 0, width: 6192, height: 4128 };
    /// An ILCE-6700 file (Tamron 17–70 mm at 32 mm), on the decoded geometry: the polynomial fits it within 0.44 px.
    const A6700_32MM_DISTORTION: [i16; 17] = [11, 7, 0, 7, 29, 58, 101, 158, 230, 323, 446, 611, 611, 611, 611, 611, 611];
    const A6700_ACTIVE: Rect = Rect { x: 0, y: 0, width: 6656, height: 4608 };
    const A6700_DECODED_CROP: Rect = Rect { x: 26, y: 20, width: 6192, height: 4128 };

    #[test]
    fn apsc_body_knots_span_its_own_crop() {
        // half diagonal 3720.9 px / 10
        let s = knot_spacing("ILCE-6700", 11, A6700_CROP).unwrap();
        assert!((s - 372.0929).abs() < 1e-3, "{s}");
        // the full-frame body keeps its sensor-fixed spacing whatever the crop and count
        assert_eq!(knot_spacing("ILCE-7RM6", 11, APSC_CROP), knot_spacing("ILCE-7RM6", 16, FF_CROP));
        // degenerate counts and crops give no spacing instead of a division by zero
        assert_eq!(knot_spacing("ILCE-6700", 1, A6700_CROP), None);
        assert_eq!(knot_spacing("ILCE-6700", 0, A6700_CROP), None);
        assert_eq!(knot_spacing("ILCE-6700", 11, Rect { x: 0, y: 0, width: 0, height: 0 }), None);
        // the spacing comes from the crop, so a crop of another shape than the measured 3:2 gets no correction
        let square = Rect { width: A6700_CROP.height, ..A6700_CROP };
        assert_eq!(knot_spacing("ILCE-6700", 11, square), None);
        let ifd = sony_lens_ifd(&A6700_DISTORTION, &APSC_VIGNETTING, &APSC_CA, [1, 2, 1]);
        let ops = lens_opcodes(&ifd, "ILCE-6700", None, None, A6700_CROP, A6700_CROP);
        let gated = gated_warp_opcode(&read_tables(&ifd), s, DISTORTION_UNIT, A6700_CROP, A6700_CROP, false, APSC_FIT_ERROR_PX_AT_1440);
        assert_eq!(ops, gated.into_iter().collect::<Vec<_>>());
        assert_eq!(ops.len(), 1);
        // other APS-C bodies stay unverified
        assert!(lens_opcodes(&ifd, "ILCE-6600", None, None, A6700_CROP, A6700_CROP).is_empty());
    }

    /// The APS-C fit gate: a table the polynomial fits within 0.44 px is applied on the ILCE-6700, while the
    /// full-frame 0.25 px gate refuses it.
    #[test]
    fn apsc_body_uses_its_wider_fit_gate() {
        let ifd = sony_lens_ifd(&A6700_32MM_DISTORTION, &APSC_VIGNETTING, &APSC_CA, [1, 2, 1]);
        let t = read_tables(&ifd);
        let s = knot_spacing("ILCE-6700", 11, A6700_DECODED_CROP).unwrap();
        assert_eq!(warp_opcode(&t, s, A6700_ACTIVE, A6700_DECODED_CROP, false), None);
        let ops = lens_opcodes(&ifd, "ILCE-6700", None, None, A6700_ACTIVE, A6700_DECODED_CROP);
        assert!(matches!(ops.as_slice(), [Opcode::WarpRectilinear { planes, .. }] if planes.len() == 1), "{ops:?}");
    }

    const SIGMA_10_18: &str = "10-18mm F2.8 DC DN | Contemporary 023";

    /// The Sigma 10-18 mm exception: its spacing and unit at 10-10.5 mm, the body's rule from 11.7 mm (and without a
    /// lens or focal length, on another body or another lens).
    #[test]
    fn lens_exception_applies_only_to_its_lens_body_and_focal_range() {
        let ifd = sony_lens_ifd(&A6700_DISTORTION, &APSC_VIGNETTING, &APSC_CA, [1, 2, 1]);
        let t = read_tables(&ifd);
        let geometry = (A6700_ACTIVE, A6700_DECODED_CROP);
        let exception = gated_warp_opcode(&t, 416.0, 14250.0, geometry.0, geometry.1, false, APSC_FIT_ERROR_PX_AT_1440);
        let body = lens_opcodes(&ifd, "ILCE-6700", None, None, geometry.0, geometry.1);
        assert!(exception.is_some() && body.len() == 1);
        assert_ne!(body.first(), exception.as_ref());
        for focal in [10.0, 10.5] {
            let ops = lens_opcodes(&ifd, "ILCE-6700", Some(SIGMA_10_18), Some(focal), geometry.0, geometry.1);
            assert_eq!(ops.first(), exception.as_ref(), "{focal} mm");
        }
        for (lens, focal) in [(Some(SIGMA_10_18), Some(11.7)), (Some(SIGMA_10_18), None), (None, Some(10.0)), (Some("E 10-20mm F4 PZ G"), Some(10.0))]
        {
            let ops = lens_opcodes(&ifd, "ILCE-6700", lens, focal, geometry.0, geometry.1);
            assert_eq!(ops, body, "{lens:?} {focal:?}");
        }
        // hostile focal lengths fall back to the body's rule
        for focal in [f64::NAN, f64::INFINITY, -10.0, 0.0] {
            assert_eq!(lens_opcodes(&ifd, "ILCE-6700", Some(SIGMA_10_18), Some(focal), geometry.0, geometry.1), body);
        }
        assert_eq!(lens_exception("ILCE-6600", Some(SIGMA_10_18), Some(10.0)).map(|e| e.unit), None);
    }

    #[test]
    fn unknown_model_gets_no_opcodes() {
        let ifd = sony_lens_ifd(&FF_DISTORTION, &FF_VIGNETTING, &FF_CA, [0, 2, 1]);
        assert!(lens_opcodes(&ifd, "ILCE-7M4", None, None, FF_ACTIVE, FF_CROP).is_empty());
        assert!(lens_opcodes(&ifd, "", None, None, FF_ACTIVE, FF_CROP).is_empty());
        // the camera's in-camera flag (0 = off here) doesn't matter: the table is applied
        let ops = lens_opcodes(&ifd, " ILCE-7RM6 ", None, None, FF_ACTIVE, FF_CROP);
        assert_eq!(ops, warp_opcode(&read_tables(&ifd), knots(), FF_ACTIVE, FF_CROP, false).into_iter().collect::<Vec<_>>());
        assert_eq!(ops.len(), 1);
    }
}
