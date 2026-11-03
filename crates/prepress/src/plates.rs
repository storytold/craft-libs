//! A PDF page separated into plates, one gray film per ink.
//!
//! A plate is the page rendered with hayro-interpret's plate mode
//! (`InterpreterSettings::plate`): every paint draws its tint of one ink as gray, and a paint
//! that overprints without naming the ink leaves the film as it is (ISO 32000-2 §8.6.7).
//! Clipping, anti-aliasing, soft masks, transparency groups and blending stay the renderer's
//! own. `Separation` and `DeviceN` colorants paint their own plates, DeviceCMYK its four
//! components, DeviceGray black only, and other process color converts to CMYK through the
//! printing condition (see [`Conversion`]).
//!
//! Ported from PdfCraft #2770 by Guillem Hermida (`crates/render/src/separations.rs` at
//! 6fcf2a4, MIT OR Apache-2.0), with craft-cms in place of moxcms.

use std::collections::HashSet;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex};

use craft_pdf::interpret::InterpreterSettings;
use craft_pdf::interpret::color::{CmykSimulation, Ink as PdfInk, InkSinkFn, Plate};
use craft_pdf::raster::{self, MAX_SIDE, RenderCache};
use craft_pdf::syntax::Pdf;
use craft_pdf::syntax::object::dict::keys::{DEST_OUTPUT_PROFILE, OUTPUT_CONDITION, OUTPUT_CONDITION_IDENTIFIER, OUTPUT_INTENTS, S};
use craft_pdf::syntax::object::{Array, Dict, Name, Stream, String as PdfText};
use craft_pdf::{OpenError, open_slice};

use crate::cms::{CmsConversion, ProfileError, srgb8_to_lab};
use crate::ink::{Alternate, InkId, InkSet, Spot};

/// Spot inks separated per page, after the four process inks. More than any press runs: a
/// `DeviceN` space has at most 32 colorants, and every plate costs a render and a byte per
/// pixel. Further spot inks are listed in [`Plates::unseparated`].
pub const MAX_SPOT_INKS: usize = 32;

/// Device pixels per plate (4 MP, 2000 × 2000), so a page with many inks stays within a few
/// hundred megabytes. The resolution is reduced to fit; [`Plates::dpi`] says what was used.
pub const MAX_PLATE_PIXELS: f32 = 4.0e6;

/// The resolution, in pixels per point, inks are discovered at before the plates render.
const DISCOVERY_SCALE: f32 = 0.1;

/// Spot ink names recorded while discovering, past which further names are dropped.
const MAX_LISTED_SPOTS: usize = MAX_SPOT_INKS * 4;

/// Output intents looked at per page: each can cost a profile's decoding and parsing.
const MAX_OUTPUT_INTENTS: usize = 16;

/// Decoding limit for an output intent's profile.
const MAX_PROFILE_BYTES: usize = 16 << 20;

/// The longest output condition kept, in characters.
const MAX_CONDITION: usize = 160;

/// The printing condition process color converts to when the page has no usable output
/// intent.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub enum Fallback {
    /// craft-cms's Coated FOGRA39 profile.
    #[default]
    Fogra39,
    /// The caller's CMYK ICC profile.
    Profile(Arc<[u8]>),
    /// The ISO 32000-2 §10.4.2.4 device formulas, black generation and undercolor removal the
    /// identity: neutral RGB goes to black alone, ICCBased CMYK separates as it is.
    DeviceFormulas,
}

/// How [`plates`] separates.
#[derive(Clone, Debug)]
pub struct Options {
    /// Honor `OP`, `op` and `OPM` as a separating device does. Without it every paint knocks
    /// out what is under it.
    pub overprint: bool,
    /// The printing condition when the page has no output intent with a usable CMYK profile.
    pub fallback: Fallback,
}

impl Default for Options {
    fn default() -> Self {
        Self { overprint: true, fallback: Fallback::default() }
    }
}

/// How process color that isn't CMYK already became CMYK.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Conversion {
    /// Through the CMYK profile of the page's or document's output intent (ISO 32000-2
    /// §14.11.5).
    OutputIntent {
        /// The profile's description, else the intent's output condition.
        profile: String,
        /// The intent's `/OutputConditionIdentifier`, such as `FOGRA39`.
        condition: Option<String>,
    },
    /// Through [`Fallback::Fogra39`] or [`Fallback::Profile`]; the profile's description.
    Fallback { profile: String },
    /// By [`Fallback::DeviceFormulas`].
    DeviceFormulas,
}

/// A page separated into plates.
#[derive(Clone, Debug)]
pub struct Plates {
    /// The process inks, then the spot inks the page paints, in the order first painted. Spots
    /// carry a Lab alternate measured from their sRGB appearance at 100%.
    pub inks: InkSet,
    /// Spot inks past [`MAX_SPOT_INKS`]: listed, not separated.
    pub unseparated: Vec<String>,
    pub width: u32,
    pub height: u32,
    /// The resolution the plates rendered at, after [`MAX_PLATE_PIXELS`].
    pub dpi: f32,
    pub conversion: Conversion,
    /// One plane per ink, in [`InkSet::iter`] order: the tint per pixel, row-major, 0 for no
    /// ink and 255 for 100%.
    pub planes: Vec<Vec<u8>>,
}

impl Plates {
    /// Ink `id`'s plane.
    pub fn plane(&self, id: InkId) -> Option<&[u8]> {
        self.planes.get(id.index()).map(Vec::as_slice)
    }

    /// Ink `id`'s tint at pixel (`x`, `y`), 0–255.
    pub fn tint(&self, id: InkId, x: u32, y: u32) -> Option<u8> {
        self.plane(id)?.get(self.index_of(x, y)?).copied()
    }
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    #[error(transparent)]
    Open(#[from] OpenError),
    #[error("page {0} does not exist")]
    NoPage(usize),
    #[error("the resolution must be positive and finite")]
    InvalidDpi,
    #[error("the fallback profile can't be a printing condition: {0}")]
    Profile(#[from] ProfileError),
    /// After a [`raster::Error::BackendPanic`] the document is dropped; nothing else is kept.
    #[error(transparent)]
    Render(#[from] raster::Error),
}

/// Separates page `page` (from 0) of `pdf` at `dpi`, reduced to fit [`MAX_PLATE_PIXELS`] and
/// the renderer's side limit: one plate per ink, the four process inks and up to
/// [`MAX_SPOT_INKS`] spot inks the page paints. Spot inks only soft masks use are left out
/// (masks don't print). Never panics.
pub fn plates(pdf: &[u8], page: usize, dpi: f32, opts: &Options) -> Result<Plates, Error> {
    if !dpi.is_finite() || dpi <= 0.0 {
        return Err(Error::InvalidDpi);
    }
    let doc = open_slice(pdf, None)?;
    let (simulation, conversion) = match catch_unwind(AssertUnwindSafe(|| output_intent(&doc, page))) {
        Ok(Some(found)) => (Some(found.0), found.1),
        Ok(None) | Err(_) => fallback(&opts.fallback)?,
    };
    let pages = doc.pages();
    let p = pages.get(page).ok_or(Error::NoPage(page))?;
    let (w, h) = p.render_dimensions();
    let scale = plate_scale(w, h, dpi / 72.0);
    let cache = RenderCache::new();

    let log = Arc::new(Mutex::new(SpotLog::default()));
    let discover = InterpreterSettings { ink_sink: Some(spot_sink(&log)), ..Default::default() };
    render(p, &cache, &discover, DISCOVERY_SCALE.min(scale))?;
    let spots = std::mem::take(&mut log.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).order);

    let mut inks = InkSet::new();
    let mut separated: Vec<PdfInk> = PdfInk::PROCESS.to_vec();
    let mut unseparated = Vec::new();
    for (ink, swatch) in spots {
        if separated.len() >= 4 + MAX_SPOT_INKS {
            unseparated.push(ink.name().to_string());
            continue;
        }
        let alternate = Alternate::Lab(srgb8_to_lab(swatch).unwrap_or([0.0; 3]));
        let spot = Spot { name: ink.name().to_string(), alternate, solidity: None };
        // hayro hands over names with their `#xx` escapes decoded, so `#` here is a literal
        // character. Two names that read the same after lossy UTF-8 decoding share one plate.
        match inks.add_decoded(spot) {
            Some(id) if id.index() == separated.len() => separated.push(ink),
            _ => unseparated.push(ink.name().to_string()),
        }
    }

    let mut size = None;
    let mut planes = Vec::with_capacity(separated.len());
    for ink in separated {
        let plate = Plate { ink, overprint: opts.overprint, simulation: simulation.clone(), spots_as_process: false };
        let settings = InterpreterSettings { plate: Some(plate), ..Default::default() };
        let film = render(p, &cache, &settings, scale)?;
        size = Some((film.width, film.height));
        // The film is gray on white: its red channel is the paper left uncovered.
        planes.push(film.bytes().as_chunks::<4>().0.iter().map(|[red, ..]| 255 - red).collect());
    }
    let (width, height) = size.unwrap_or((0, 0));
    Ok(Plates { inks, unseparated, width, height, dpi: scale * 72.0, conversion, planes })
}

fn render<'a>(
    page: &'a craft_pdf::syntax::page::Page<'a>,
    cache: &RenderCache<'a>,
    settings: &InterpreterSettings,
    scale: f32,
) -> Result<raster::Raster, Error> {
    // The plate's gray film must keep its tints: no color replacement.
    let options = raster::Options { scale: [scale, scale], background: [255; 4], color_replacement: None, ..Default::default() };
    Ok(raster::render(page, cache, settings, &options)?)
}

/// The scale a `w` × `h` pt page separates at: `scale`, within [`MAX_PLATE_PIXELS`] and
/// [`MAX_SIDE`].
fn plate_scale(w: f32, h: f32, scale: f32) -> f32 {
    let (w, h) = (w.max(1.0), h.max(1.0));
    let by_area = (MAX_PLATE_PIXELS / (w * h)).sqrt();
    let by_side = MAX_SIDE as f32 / w.max(h);
    let s = scale.min(by_area).min(by_side);
    if s.is_finite() && s > 0.0 { s } else { f32::MIN_POSITIVE }
}

pub(crate) fn fallback(fallback: &Fallback) -> Result<(Option<CmykSimulation>, Conversion), Error> {
    let (conversion, icc): (CmsConversion, &[u8]) = match fallback {
        Fallback::DeviceFormulas => return Ok((None, Conversion::DeviceFormulas)),
        Fallback::Fogra39 => (CmsConversion::fogra39(), craft_cms::builtin::COATED_FOGRA39_ICC),
        Fallback::Profile(icc) => (CmsConversion::new(icc)?, icc),
    };
    let simulation = conversion.simulation(icc);
    let profile = simulation.description().unwrap_or("unnamed CMYK profile").to_string();
    Ok((Some(simulation), Conversion::Fallback { profile }))
}

/// Spot inks reported while a page renders, in the order first painted, each with its sRGB
/// appearance at 100%.
#[derive(Default)]
struct SpotLog {
    order: Vec<(PdfInk, [u8; 3])>,
    seen: HashSet<PdfInk>,
}

fn spot_sink(log: &Arc<Mutex<SpotLog>>) -> InkSinkFn {
    let log = log.clone();
    Arc::new(move |ink: &PdfInk, appearance: &dyn Fn(f32) -> [u8; 3]| {
        let mut log = log.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if log.seen.contains(ink) || log.order.len() >= MAX_LISTED_SPOTS {
            return;
        }
        log.seen.insert(ink.clone());
        log.order.push((ink.clone(), appearance(1.0)));
    })
}

/// A text string entry of `dict`, without control characters and at most [`MAX_CONDITION`]
/// characters long; `None` when missing or blank.
fn text_entry(dict: &Dict<'_>, key: &[u8]) -> Option<String> {
    let s = dict.get::<PdfText<'_>>(key)?;
    let text: String = pdf_text(s.as_bytes()).chars().filter(|c| !c.is_control()).take(MAX_CONDITION).collect();
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

/// A PDF text string (ISO 32000-2 §7.9.2.2): UTF-16BE or UTF-8 after its byte order mark, else
/// PDFDocEncoding, read here as Latin-1 (they differ only in rarely used punctuation).
pub(crate) fn pdf_text(bytes: &[u8]) -> String {
    if let Some(utf16) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        let units: Vec<u16> = utf16.as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes(*c)).collect();
        return String::from_utf16_lossy(&units);
    }
    if let Some(utf8) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(utf8).into_owned();
    }
    bytes.iter().map(|b| char::from(*b)).collect()
}

/// The printing condition of page `page` (ISO 32000-2 §14.11.5): of the page's own
/// `/OutputIntents` (PDF 2.0), else the catalog's, the first whose `/DestOutputProfile` is a
/// CMYK profile craft-cms converts into, a `GTS_PDFX` intent before the others.
pub(crate) fn output_intent(pdf: &Pdf, page: usize) -> Option<(CmykSimulation, Conversion)> {
    output_intents(pdf, page).iter().find_map(|intent| {
        let icc = intent.get::<Stream<'_>>(DEST_OUTPUT_PROFILE)?.decoded_within(MAX_PROFILE_BYTES).ok()?;
        let simulation = CmsConversion::new(&icc).ok()?.simulation(&icc);
        let condition = text_entry(intent, OUTPUT_CONDITION_IDENTIFIER);
        let profile = simulation
            .description()
            .map(str::to_string)
            .or_else(|| text_entry(intent, OUTPUT_CONDITION))
            .or_else(|| condition.clone())
            .unwrap_or_else(|| "unnamed CMYK profile".to_string());
        Some((simulation, Conversion::OutputIntent { profile, condition }))
    })
}

/// The output intent [`output_intent`] picks, as written in the PDF.
#[cfg(feature = "flatten")]
pub(crate) struct IntentProfile {
    pub icc: Vec<u8>,
    pub identifier: Option<String>,
    pub condition: Option<String>,
    pub registry_name: Option<String>,
    pub info: Option<String>,
}

/// The output intent [`output_intent`] picks: its profile bytes and entries, to write the same
/// intent again.
#[cfg(feature = "flatten")]
pub(crate) fn output_intent_profile(pdf: &Pdf, page: usize) -> Option<IntentProfile> {
    output_intents(pdf, page).iter().find_map(|intent| {
        let icc = intent.get::<Stream<'_>>(DEST_OUTPUT_PROFILE)?.decoded_within(MAX_PROFILE_BYTES).ok()?;
        if *icc != *craft_cms::builtin::COATED_FOGRA39_ICC && CmsConversion::new(&icc).is_err() {
            return None;
        }
        Some(IntentProfile {
            icc: icc.into_owned(),
            identifier: text_entry(intent, OUTPUT_CONDITION_IDENTIFIER),
            condition: text_entry(intent, OUTPUT_CONDITION),
            registry_name: text_entry(intent, b"RegistryName"),
            info: text_entry(intent, b"Info"),
        })
    })
}

/// The page's own `/OutputIntents` (PDF 2.0), else the catalog's, a `GTS_PDFX` intent first.
fn output_intents(pdf: &Pdf, page: usize) -> Vec<Dict<'_>> {
    fn list<'a>(intents: Option<Array<'a>>) -> Vec<Dict<'a>> {
        intents.iter().flat_map(|a| a.iter::<Dict<'a>>()).take(MAX_OUTPUT_INTENTS).collect()
    }
    let xref = pdf.xref();
    let mut listed = list(pdf.pages().get(page).and_then(|p| p.raw().get::<Array<'_>>(OUTPUT_INTENTS)));
    if listed.is_empty() {
        listed = list(xref.get::<Dict<'_>>(xref.root_id()).and_then(|catalog| catalog.get::<Array<'_>>(OUTPUT_INTENTS)));
    }
    // A stable sort: the PDF/X intent first, the others in their order.
    listed.sort_by_key(|d| d.get::<Name<'_>>(S).is_none_or(|s| s.as_ref() != b"GTS_PDFX"));
    listed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plates_fit_the_pixel_and_side_caps() {
        assert_eq!(plate_scale(40.0, 40.0, 1.0), 1.0);
        // A4 at 2400 dpi would be 22 000 × 31 000 pixels.
        let s = plate_scale(595.0, 842.0, 2400.0 / 72.0);
        assert!((595.0 * s) * (842.0 * s) <= MAX_PLATE_PIXELS * 1.0001);
        // A 1 × 10 000 pt strip fits the area cap but not the renderer's side limit.
        assert!(10_000.0 * plate_scale(1.0, 10_000.0, 1.0) <= MAX_SIDE as f32);
        assert!(plate_scale(0.0, f32::NAN, 1.0) > 0.0);
    }
}
