//! Turning a document into a format a printer takes: the app's [`DocumentConverter`] and the
//! rules the [`Registry`](crate::Registry) applies around it.
//!
//! This crate has no renderer. A PDF becomes PWG raster (or XPS, or anything else) only because the
//! app that owns the PDF renderer says how, by registering a converter
//! ([`Registry::set_converter`](crate::Registry::set_converter)). The registry owns the **decision**:
//!
//! 1. The document's format is accepted by the printer: it is sent as it is and the converter is
//!    never called.
//! 2. Otherwise the converter lists what it can make from that format
//!    ([`DocumentConverter::targets`], best first). The registry picks the first format of the
//!    [`ConversionPolicy::preference`] order that both the converter makes and the printer accepts,
//!    falling back to the converter's own order for formats the preference does not name.
//! 3. The resolution for the converter is the caller's [`JobOptions::resolution`](crate::JobOptions)
//!    when set, else the printer's listed resolution nearest [`ConversionPolicy::target_dpi`]
//!    ([`choose_resolution`]), else that target dpi square.
//! 4. The converter runs ([`DocumentConverter::convert`]) and its result is sent. For a PWG raster
//!    target the chosen resolution also goes into the job's options, so the ticket and the raster
//!    agree.
//! 5. When there is no way (no converter, nothing in common, or the converter failed), the error is
//!    typed: [`ErrorKind::UnsupportedFormat`](crate::ErrorKind) for "no format in common" (the
//!    message lists what the document is, what the converter offers and what the printer accepts)
//!    and the converter's own error, marked as raised before anything was sent, otherwise.
//!
//! Which printers' formats count is the backend's answer: [`crate::PrinterDetails::formats`] for a backend
//! whose [`formats_per_printer`](crate::backends::Backend::formats_per_printer) is `true` (one
//! read-only details request per print, skipped when the caller passes details it already has to
//! [`Registry::submit_job_with_details`](crate::Registry::submit_job_with_details)), else its
//! [`accepted_formats`](crate::backends::Backend::accepted_formats).

use crate::backend::{Document, DocumentFormat, PrintError};
use crate::options::{ColourMode, Orientation, Resolution};
use crate::pwg::RasterType;
use crate::{CancelToken, Duplex};

/// A document that owns its bytes: what a [`DocumentConverter`] returns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnedDocument {
    pub format: DocumentFormat,
    pub bytes: Vec<u8>,
}

impl OwnedDocument {
    pub fn new(format: DocumentFormat, bytes: Vec<u8>) -> Self {
        Self { format, bytes }
    }

    /// The borrowed [`Document`] a request carries.
    pub fn as_document(&self) -> Document<'_> {
        Document::new(self.format, &self.bytes)
    }
}

/// What the registry tells a [`DocumentConverter`] about the print, so it can render the right
/// pixels. Everything is a resolved value, not a request: the resolution is always set (see the
/// [module documentation](self) for the rule) and the colour flag already accounts for the printer.
/// Fields may be added over time, so a converter reads them and a test builds one with
/// [`Self::new`].
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct ConvertContext {
    /// The printer id as the caller gave it (namespaced), when there was one.
    pub printer: Option<String>,
    /// The resolution to render raster output at. A vector target may ignore it.
    pub resolution: Resolution,
    /// Render in colour: the caller did not ask for monochrome (neither [`ColourMode::Mono`] nor,
    /// with [`ColourMode::Auto`], `Job::grayscale`) and the printer does not say it is monochrome.
    pub colour: bool,
    /// The PWG media name the caller asked for, if any.
    pub media: Option<String>,
    pub orientation: Option<Orientation>,
    pub duplex: Option<Duplex>,
    /// How many pages the caller says the document has, when it said.
    pub expected_pages: Option<u32>,
    /// The pixel format to write when the target is PWG raster: the first of the converter's own list
    /// ([`DocumentConverter::raster_types`]) that the printer lists (or [`RasterType::BASELINE`] when it lists none),
    /// grey for a monochrome job and colour otherwise. A converter may ignore it for other targets.
    pub raster: RasterType,
    /// The request's cancellation token: a long conversion should check it between pages.
    pub cancel: CancelToken,
}

impl ConvertContext {
    /// A context for `resolution` and colour, with nothing else set.
    pub fn new(resolution: Resolution, colour: bool) -> Self {
        let raster = if colour { RasterType::Srgb8 } else { RasterType::Sgray8 };
        Self {
            printer: None,
            resolution,
            colour,
            media: None,
            orientation: None,
            duplex: None,
            expected_pages: None,
            raster,
            cancel: CancelToken::new(),
        }
    }
}

/// The app's way of making a document the printer accepts. Register one with
/// [`Registry::set_converter`](crate::Registry::set_converter); see the [module
/// documentation](self) for when the registry calls it.
///
/// Implementations are blocking and `Send + Sync`, like backends, and never panic on bad input
/// (return an error). [`crate::pwg::encode`] packs already-rendered pixel pages into PWG raster.
pub trait DocumentConverter: Send + Sync {
    /// The formats this converter can make from a document in `from`, best first. Empty when it
    /// cannot convert that format at all.
    fn targets(&self, from: DocumentFormat) -> Vec<DocumentFormat>;

    /// The PWG raster pixel formats this converter can write, best first (for a printer that lists
    /// `pwg-raster-document-type-supported`: the first one the printer lists is used, see [`ConvertContext::raster`]).
    /// A converter that writes more than 8-bit sRGB and grey says so here. Default: 8-bit sRGB and grey.
    fn raster_types(&self) -> Vec<RasterType> {
        RasterType::BASELINE.to_vec()
    }

    /// Make `target` (one of [`Self::targets`] for the document's format) from `document`. The
    /// result's format must be `target`. An error is reported as a failure before anything was
    /// sent (nothing reached the printer, so it is safe to try again).
    fn convert(&self, document: Document<'_>, target: DocumentFormat, context: &ConvertContext) -> Result<OwnedDocument, PrintError>;
}

/// The registry's choices around conversion; see [`Registry::set_conversion_policy`](crate::Registry::set_conversion_policy).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConversionPolicy {
    /// Target formats, best first. A format the converter makes and the printer accepts earlier in
    /// this list wins. Default: [`DocumentFormat::DEFAULT_PREFERENCE`].
    pub preference: Vec<DocumentFormat>,
    /// The resolution to approach when the caller named none, in dpi. Default 300: sharp text and
    /// line art without the memory of 600 or 1200 dpi pages.
    pub target_dpi: u32,
}

impl Default for ConversionPolicy {
    fn default() -> Self {
        Self { preference: DocumentFormat::DEFAULT_PREFERENCE.to_vec(), target_dpi: 300 }
    }
}

/// What the registry did to a document: [`Submitted::conversion`](crate::Submitted) is `Some` when
/// the converter ran.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Conversion {
    pub from: DocumentFormat,
    pub to: DocumentFormat,
    /// The resolution the converter was asked to render at.
    pub resolution: Resolution,
    /// The pixel format a PWG raster result was asked to be in; `None` for another target.
    pub raster: Option<RasterType>,
    /// The size of the converted document.
    pub bytes: usize,
}

/// The raster type to ask a converter for: the first of `offered` (the converter's own order, best first) of the right
/// kind (grey for a monochrome job, colour otherwise) that `listed` (the printer's list; empty = not published, taken
/// as [`RasterType::BASELINE`]) contains. When none is in common, a monochrome job takes a colour type the printer
/// lists (the converter writes neutral colour), and failing that the baseline type: every PWG raster printer takes it.
pub fn choose_raster_type(offered: &[RasterType], listed: &[RasterType], colour: bool) -> RasterType {
    let listed = if listed.is_empty() { &RasterType::BASELINE[..] } else { listed };
    let wanted = |t: &&RasterType| t.is_gray() != colour;
    offered
        .iter()
        .filter(wanted)
        .find(|t| listed.contains(t))
        .or_else(|| offered.iter().filter(|t| !colour && !t.is_gray()).find(|t| listed.contains(t)))
        .copied()
        .unwrap_or(if colour { RasterType::Srgb8 } else { RasterType::Sgray8 })
}

/// The listed resolution nearest `target_dpi`. Distance is the sum of the differences on both axes
/// (so 300x300 beats 600x300 for a target of 300); a tie goes to the higher resolution; entries with
/// a zero axis are ignored. With no usable entry the answer is `target_dpi` both ways.
pub fn choose_resolution(available: &[Resolution], target_dpi: u32) -> Resolution {
    available
        .iter()
        .copied()
        .filter(|r| r.x_dpi > 0 && r.y_dpi > 0)
        .min_by_key(|r| {
            (r.x_dpi.abs_diff(target_dpi).saturating_add(r.y_dpi.abs_diff(target_dpi)), std::cmp::Reverse(u64::from(r.x_dpi) + u64::from(r.y_dpi)))
        })
        .unwrap_or_else(|| Resolution::square(target_dpi.max(1)))
}

/// The target to convert to: the first of `preference` that the converter offers and the printer
/// accepts, else the converter's first offer the printer accepts.
pub(crate) fn pick_target(offered: &[DocumentFormat], preference: &[DocumentFormat], accepted: &[DocumentFormat]) -> Option<DocumentFormat> {
    preference
        .iter()
        .copied()
        .find(|f| offered.contains(f) && accepted.contains(f))
        .or_else(|| offered.iter().copied().find(|f| accepted.contains(f)))
}

/// Whether the colour flag of a [`ConvertContext`] is on, for the printer's own colour report.
pub(crate) fn wants_colour(mode: ColourMode, legacy_grayscale: bool, printer_colour: Option<bool>) -> bool {
    !mode.is_mono(legacy_grayscale) && printer_colour != Some(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use DocumentFormat::*;

    #[test]
    fn the_resolution_nearest_the_target_wins_and_ties_go_up() {
        let list = [Resolution::square(150), Resolution::square(600), Resolution::square(1200)];
        assert_eq!(choose_resolution(&list, 300), Resolution::square(150), "150 is 300 away in total, 600 is 600");
        assert_eq!(choose_resolution(&[Resolution::square(150), Resolution::new(300, 600)], 300), Resolution::new(300, 600), "a tie goes up");
        assert_eq!(choose_resolution(&[Resolution::square(200), Resolution::square(400)], 300), Resolution::square(400));
        assert_eq!(choose_resolution(&[Resolution::square(600), Resolution::new(300, 300)], 300), Resolution::square(300));
        assert_eq!(choose_resolution(&[Resolution::new(0, 300), Resolution::square(600)], 300), Resolution::square(600));
        assert_eq!(choose_resolution(&[], 300), Resolution::square(300));
        assert_eq!(choose_resolution(&[Resolution::new(0, 0)], 0), Resolution::square(1));
    }

    #[test]
    fn the_preference_decides_and_the_converters_order_is_the_fallback() {
        let offered = [Jpeg, PwgRaster, Tiff];
        assert_eq!(pick_target(&offered, &[Pdf, PwgRaster, Jpeg], &[Jpeg, PwgRaster]), Some(PwgRaster));
        assert_eq!(pick_target(&offered, &[Jpeg, PwgRaster], &[PwgRaster, Jpeg]), Some(Jpeg));
        assert_eq!(pick_target(&offered, &[Pdf], &[Tiff, Jpeg]), Some(Jpeg), "unlisted in the preference: the converter's order");
        assert_eq!(pick_target(&offered, &DocumentFormat::DEFAULT_PREFERENCE, &[Png, Xps]), None);
        assert_eq!(pick_target(&[], &DocumentFormat::DEFAULT_PREFERENCE, &[Pdf]), None);
    }

    #[test]
    fn colour_follows_the_mode_and_the_printer() {
        assert!(wants_colour(ColourMode::Auto, false, None));
        assert!(!wants_colour(ColourMode::Auto, true, None));
        assert!(!wants_colour(ColourMode::Colour, false, Some(false)));
        assert!(wants_colour(ColourMode::Colour, true, Some(true)));
        assert!(!wants_colour(ColourMode::Mono, false, Some(true)));
    }

    #[test]
    fn an_owned_document_lends_its_bytes() {
        let owned = OwnedDocument::new(Png, vec![1, 2, 3]);
        let borrowed = owned.as_document();
        assert_eq!((borrowed.format, borrowed.bytes), (Png, &[1u8, 2, 3][..]));
    }

    #[test]
    fn the_raster_type_is_the_converters_first_choice_that_the_printer_lists() {
        use RasterType::*;
        let offered = [AdobeRgb16, Srgb16, AdobeRgb8, Srgb8, Sgray16, Sgray8];
        // a printer that lists everything gets the converter's best
        assert_eq!(choose_raster_type(&offered, &RasterType::ALL, true), AdobeRgb16);
        // a printer that lists only some gets the first the converter prefers among them
        assert_eq!(choose_raster_type(&offered, &[Srgb8, Srgb16, Sgray8], true), Srgb16);
        assert_eq!(choose_raster_type(&offered, &[Srgb8, Sgray8, AdobeRgb8], true), AdobeRgb8);
        // a monochrome job takes a grey type, the deepest the converter offers first
        assert_eq!(choose_raster_type(&offered, &[Srgb8, Sgray8, Sgray16], false), Sgray16);
        // a printer that lists no grey type gets a colour one for a monochrome job
        assert_eq!(choose_raster_type(&offered, &[Srgb16, Srgb8], false), Srgb16);
        // a printer that publishes nothing is taken to accept the baseline
        assert_eq!(choose_raster_type(&offered, &[], true), Srgb8);
        assert_eq!(choose_raster_type(&offered, &[], false), Sgray8);
        // nothing in common: the baseline type every PWG raster printer takes
        assert_eq!(choose_raster_type(&[AdobeRgb16], &[Srgb8], true), Srgb8);
        assert_eq!(choose_raster_type(&[], &RasterType::ALL, false), Sgray8);
        // the default converter offers the baseline
        struct Plain;
        impl DocumentConverter for Plain {
            fn targets(&self, _: DocumentFormat) -> Vec<DocumentFormat> {
                Vec::new()
            }
            fn convert(&self, _: Document<'_>, _: DocumentFormat, _: &ConvertContext) -> Result<OwnedDocument, PrintError> {
                Err(PrintError::new(crate::ErrorKind::Failed, "unused"))
            }
        }
        assert_eq!(Plain.raster_types(), RasterType::BASELINE.to_vec());
        assert_eq!(ConvertContext::new(Resolution::new(300, 300), false).raster, Sgray8);
    }
}
