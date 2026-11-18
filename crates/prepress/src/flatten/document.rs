//! [`flatten_document`]: every page of a PDF flattened into one PDF.
//!
//! - Output intent: written once, the first flattened page's as [`flatten_page`] picks it (the
//!   page's own `/OutputIntents`, PDF 2.0, else the catalog's), else [`ReadOptions::fallback`]'s
//!   profile, else FOGRA39. Each page is read and its complex regions separated with its own
//!   intent; krilla writes no page-level `/OutputIntents`, so pages whose intent differs from the
//!   one written are listed in [`FlattenDocumentReport::other_intents`].
//! - Page boxes: the page is the source's visible area (CropBox within MediaBox, rotated), written
//!   as the MediaBox. TrimBox, BleedBox and ArtBox carry over, mapped onto it and clipped to it.
//! - Metadata: title, author, creation date and modification date carry over; the producer is
//!   the caller's.
//! - Version: [`FlattenDocumentOptions::version`], [`FlattenDocumentOptions::pdfx_version`]
//!   and [`FlattenDocumentOptions::pdfx_conformance`] set the header, `pdf:PDFVersion` and the
//!   PDF/X version and conformance keys. krilla has no PDF/X validator, so a caller claiming
//!   PDF/X runs its own conformance checks.
//! - Fonts: prepared per page ([`FontCache`](super::font::FontCache)); pages share a krilla font
//!   only when the font bytes are equal ([`FontMap`]).
//! - The source is opened once, and each output intent and printing condition parsed once.

use std::cell::Cell;
use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;
use std::sync::{Arc, OnceLock};

use craft_pdf::interpret::TransformExt as _;
use craft_pdf::syntax::Pdf;
use craft_pdf::syntax::object::dict::keys::{ART_BOX, BLEED_BOX, OUTPUT_INTENTS, TRIM_BOX};
use craft_pdf::syntax::object::{Array, DateTime as PdfDate, Dict, Rect as PdfRect};
use craft_pdf::syntax::page::Page;
use craft_pdf::write::backend::SerializeSettings;
use craft_pdf::write::backend::configure::PdfVersion;
use craft_pdf::write::backend::geom::{Rect as KRect, Size, Transform};
use craft_pdf::write::backend::metadata::DateTime;
use craft_pdf::write::backend::page::PageSettings;
use craft_pdf::write::backend::pdf::PdfDocument;
use craft_pdf::write::print::{self as print_settings, PrintColor};
use craft_pdf::write::{self as pdf_write, Metadata, Writer};
use craft_pdf::{OpenError, open_slice};
use kurbo::Rect;

use super::pdf::{ReadColor, ReadError, ReadOptions, read_color, read_page_in};
use super::raster::{FlattenPageReport, Rasterizer};
use super::write::{FontMap, WriteError, write_flattened_with_fonts};
use super::{DisplayList, FlattenError, FlattenOptions, flatten};
use crate::cms::CmsConversion;
use crate::plates::{self, Fallback, pdf_text};

/// How [`flatten_document`] reads, flattens and writes.
#[derive(Clone, Debug, Default)]
pub struct FlattenDocumentOptions {
    pub read: ReadOptions,
    pub flatten: FlattenOptions,
    /// The pages to flatten, from 0; all when `None`.
    pub pages: Option<Range<usize>>,
    /// Written as `/Producer`.
    pub producer: String,
    pub unreadable: Unreadable,
    /// The output's PDF version. Below 1.6, DeviceN spaces get no NChannel attributes; at 1.4,
    /// krilla also enforces the 1.4 implementation limits (reals, array and dictionary sizes),
    /// and a DeviceN space of more than 8 colorants is a [`WriteError::Colorants`].
    pub version: PdfVersion,
    /// The PDF/X version claimed: `/GTS_PDFXVersion` in the information dictionary and
    /// `pdfxid:GTS_PDFXVersion` in XMP. For PDF/X-1a:2003 it is `PDF/X-1:2003`. Not checked.
    pub pdfx_version: Option<String>,
    /// The PDF/X conformance level claimed, `/GTS_PDFXConformance` in the information
    /// dictionary: `PDF/X-1a:2003` for PDF/X-1a:2003. Not checked.
    pub pdfx_conformance: Option<String>,
}

/// What [`flatten_document`] does with a page the PDF interpreter fails on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum Unreadable {
    /// Fail with [`FlattenDocumentError::Read`].
    #[default]
    Fail,
    /// Copy the page unflattened, as a form XObject, and list it in
    /// [`FlattenDocumentReport::unflattened`]. Its transparency stays.
    Keep,
}

/// What [`flatten_document`] did.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct FlattenDocumentReport {
    /// Each flattened page (from 0) and its report.
    pub pages: Vec<(usize, FlattenPageReport)>,
    /// Pages copied unflattened ([`Unreadable::Keep`]) and why they didn't read.
    pub unflattened: Vec<(usize, String)>,
    /// Pages read and separated with an output intent other than the one written.
    pub other_intents: Vec<usize>,
    pub totals: FlattenTotals,
}

/// [`FlattenDocumentReport::pages`] summed.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct FlattenTotals {
    /// Top-level items cut into regions.
    pub flattened: usize,
    pub complex_regions: usize,
    /// Groups rasterized whole.
    pub complex_groups: usize,
    pub raster_images: usize,
    pub raster_pixels: u64,
    /// The lowest resolution a complex region was rasterized at, when one needed less than
    /// [`FlattenOptions::raster_dpi`].
    pub lowest_dpi: Option<f32>,
    /// Raster images read.
    pub images: usize,
    pub unreadable_images: usize,
    pub dropped_tilings: usize,
    pub dropped_inks: usize,
    pub dropped_annotations: usize,
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum FlattenDocumentError {
    #[error(transparent)]
    Open(#[from] OpenError),
    #[error("pages {start}..{end} are not pages of the document's {count}")]
    PageRange { start: usize, end: usize, count: usize },
    #[error("page {page} (from 0): {source}")]
    Read { page: usize, source: ReadError },
    #[error("page {page} (from 0): {source}")]
    Flatten { page: usize, source: FlattenError },
    #[error("page {page} (from 0): {source}")]
    Write { page: usize, source: WriteError },
    /// Serializing the document failed.
    #[error(transparent)]
    Serialize(#[from] pdf_write::WriteError),
}

/// `pdf` with the transparency of every page in [`FlattenDocumentOptions::pages`] flattened, as
/// one PDF. See the [module docs](self) for what carries over.
pub fn flatten_document(pdf: &[u8], opts: &FlattenDocumentOptions) -> Result<(Vec<u8>, FlattenDocumentReport), FlattenDocumentError> {
    flatten_document_with_progress(pdf, opts, &mut |_, _| {})
}

/// [`flatten_document`], calling `progress(done, total)` after each page.
pub fn flatten_document_with_progress(
    pdf: &[u8],
    opts: &FlattenDocumentOptions,
    progress: &mut dyn FnMut(usize, usize),
) -> Result<(Vec<u8>, FlattenDocumentReport), FlattenDocumentError> {
    let doc = Arc::new(open_slice(pdf, None)?);
    let count = doc.pages().len();
    let range = opts.pages.clone().unwrap_or(0..count);
    if range.is_empty() || range.end > count {
        return Err(FlattenDocumentError::PageRange { start: range.start, end: range.end, count });
    }
    let mut colors = Colors { doc: &doc, fallback: &opts.read.fallback, cache: HashMap::new() };
    let first = colors.get(range.start)?;
    let mut report = FlattenDocumentReport::default();
    // The page being written, to name it when krilla panics.
    let current = Cell::new(None);
    let written = pdf_write::guard(|| {
        let settings = SerializeSettings {
            configuration: pdf_write::configuration(opts.version, None)?,
            pdfx_version: opts.pdfx_version.clone(),
            pdfx_conformance: opts.pdfx_conformance.clone(),
            ..first.settings.clone()
        };
        let mut writer = Writer::new(settings, metadata(&doc, &opts.producer));
        let mut fonts = FontMap::new();
        let mut source = None;
        let total = range.len();
        for (done, page) in range.clone().enumerate() {
            current.set(Some(page));
            let color = colors.get(page)?;
            if !Rc::ptr_eq(&color, &first) && !same_condition(&color.condition, &first.condition) {
                report.other_intents.push(page);
            }
            let p = doc.pages().get(page).ok_or(FlattenDocumentError::Read { page, source: ReadError::NoPage(page) })?;
            match read_page_in(&doc, page, &opts.read, color.read.clone()) {
                Ok((list, read)) => {
                    let flat = flatten(&list, &opts.flatten).map_err(|source| FlattenDocumentError::Flatten { page, source })?;
                    let mut raster = Rasterizer::new(&list, color.settings.clone(), color.condition.clone(), &color.cms, opts.flatten.raster_dpi);
                    let at = |source| FlattenDocumentError::Write { page, source };
                    let settings = page_settings(p, list.page).ok_or(at(WriteError::Page))?;
                    write_page(&mut writer, settings, &list, |surface| {
                        write_flattened_with_fonts(surface, &list, &flat, &color.cms, opts.version, &mut fonts, &mut |r| raster.region(r))
                    })
                    .map_err(at)?;
                    report.pages.push((page, FlattenPageReport { read, flatten: flat.report, raster: raster.report().clone() }));
                }
                Err(e) if opts.unreadable == Unreadable::Keep => {
                    let (w, h) = p.render_dimensions();
                    let bbox = Rect::new(0.0, 0.0, f64::from(w), f64::from(h));
                    let at = |source| FlattenDocumentError::Write { page, source };
                    let settings = page_settings(p, bbox).ok_or(at(WriteError::Page))?;
                    let size = Size::from_wh(w, h).ok_or(at(WriteError::Page))?;
                    let source = source.get_or_insert_with(|| PdfDocument::new(doc.clone()));
                    let mut page_builder = writer.document_mut().start_page_with(settings);
                    let mut surface = page_builder.surface();
                    surface.draw_pdf_page(source, size, page);
                    surface.finish();
                    page_builder.finish();
                    report.unflattened.push((page, e.to_string()));
                }
                Err(source) => return Err(FlattenDocumentError::Read { page, source }),
            }
            progress(done + 1, total);
        }
        current.set(None);
        Ok(writer.finish()?)
    });
    let bytes = written.map_err(|e| match (e, current.get()) {
        (FlattenDocumentError::Serialize(source), Some(page)) => FlattenDocumentError::Write { page, source: WriteError::Pdf(source) },
        (e, _) => e,
    })?;
    report.totals = totals(&report.pages);
    Ok((bytes, report))
}

/// Draws one page of `list.page`'s size with `draw`, in page space.
fn write_page(
    writer: &mut Writer,
    settings: PageSettings,
    list: &DisplayList,
    draw: impl FnOnce(&mut craft_pdf::write::backend::surface::Surface<'_>) -> Result<(), WriteError>,
) -> Result<(), WriteError> {
    let box_ = list.page;
    let mut page_builder = writer.document_mut().start_page_with(settings);
    let mut surface = page_builder.surface();
    let shifted = box_.x0 != 0.0 || box_.y0 != 0.0;
    if shifted {
        surface.push_transform(&Transform::from_translate(-box_.x0 as f32, -box_.y0 as f32));
    }
    let drawn = draw(&mut surface);
    if shifted {
        surface.pop();
    }
    surface.finish();
    page_builder.finish();
    drawn
}

/// A page of `bbox`'s size (page space) with the source page's TrimBox, BleedBox and ArtBox,
/// mapped into page space and clipped to `bbox`.
fn page_settings(p: &Page<'_>, bbox: Rect) -> Option<PageSettings> {
    let size = PageSettings::from_wh(bbox.width() as f32, bbox.height() as f32).filter(|_| bbox.is_finite())?;
    let to_page = p.initial_transform(true).to_kurbo();
    let mapped = |key: &[u8]| {
        let r = p.raw().get::<PdfRect>(key)?;
        let r = to_page.transform_rect_bbox(Rect::new(r.x0, r.y0, r.x1, r.y1)).intersect(bbox);
        if !(r.is_finite() && r.width() > 0.0 && r.height() > 0.0) {
            return None;
        }
        KRect::from_ltrb((r.x0 - bbox.x0) as f32, (r.y0 - bbox.y0) as f32, (r.x1 - bbox.x0) as f32, (r.y1 - bbox.y0) as f32)
    };
    Some(size.with_trim_box(mapped(TRIM_BOX)).with_bleed_box(mapped(BLEED_BOX)).with_art_box(mapped(ART_BOX)))
}

/// The source's title, author, creation and modification dates, and `producer`.
fn metadata(doc: &Pdf, producer: &str) -> Metadata {
    let info = doc.metadata();
    let text = |bytes: &Option<Vec<u8>>| bytes.as_deref().map(|b| pdf_text(b).trim().to_string()).filter(|s| !s.is_empty());
    Metadata {
        producer: producer.to_string(),
        title: text(&info.title),
        authors: text(&info.author).into_iter().collect(),
        created: info.creation_date.as_ref().map(date),
        modified: info.modification_date.as_ref().map(date),
        ..Metadata::default()
    }
}

fn date(d: &PdfDate) -> DateTime {
    DateTime::new(d.year)
        .month(d.month)
        .day(d.day)
        .hour(d.hour)
        .minute(d.minute)
        .second(d.second)
        .utc_offset_hour(d.utc_offset_hour)
        .utc_offset_minute(d.utc_offset_minute)
}

fn totals(pages: &[(usize, FlattenPageReport)]) -> FlattenTotals {
    let mut t = FlattenTotals::default();
    for (_, r) in pages {
        t.flattened += r.flatten.flattened;
        t.complex_regions += r.flatten.complex_regions;
        t.complex_groups += r.flatten.complex_groups.len();
        t.raster_images += r.raster.images;
        t.raster_pixels += r.raster.pixels;
        t.lowest_dpi = match (t.lowest_dpi, r.raster.lowest_dpi) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        t.images += r.read.images;
        t.unreadable_images += r.read.unreadable_images;
        t.dropped_tilings += r.read.dropped_tilings;
        t.dropped_inks += r.read.dropped_inks;
        t.dropped_annotations += r.read.dropped_annotations;
    }
    t
}

/// A page's printing condition, for reading, separating and writing.
struct PageColor {
    /// With the output intent.
    settings: SerializeSettings,
    cms: CmsConversion,
    condition: Fallback,
    read: ReadColor,
}

/// The [`PageColor`]s of a document: one for the pages without their own output intents, one per
/// page with them.
struct Colors<'d> {
    doc: &'d Pdf,
    fallback: &'d Fallback,
    cache: HashMap<Option<usize>, Rc<PageColor>>,
}

impl Colors<'_> {
    fn get(&mut self, page: usize) -> Result<Rc<PageColor>, FlattenDocumentError> {
        let own =
            self.doc.pages().get(page).and_then(|p| p.raw().get::<Array<'_>>(OUTPUT_INTENTS)).is_some_and(|a| a.iter::<Dict<'_>>().next().is_some());
        let key = own.then_some(page);
        if let Some(color) = self.cache.get(&key) {
            return Ok(color.clone());
        }
        let read = read_color(self.doc, page, self.fallback).map_err(|source| FlattenDocumentError::Read { page, source })?;
        let (settings, cms, condition) = printing(self.doc, page, self.fallback).map_err(|source| FlattenDocumentError::Write { page, source })?;
        let color = Rc::new(PageColor { settings, cms, condition, read });
        self.cache.insert(key, color.clone());
        Ok(color)
    }
}

fn same_condition(a: &Fallback, b: &Fallback) -> bool {
    match (a, b) {
        (Fallback::Profile(a), Fallback::Profile(b)) => a == b,
        _ => std::mem::discriminant(a) == std::mem::discriminant(b),
    }
}

/// The output's serialize settings and printing condition for page `page`: its output intent
/// written again, else `fallback`'s profile, else FOGRA39.
fn printing(doc: &Pdf, page: usize, fallback: &Fallback) -> Result<(SerializeSettings, CmsConversion, Fallback), WriteError> {
    let fogra39 = || -> Result<_, WriteError> {
        // Reading the profile is the slow part of writing a small page; it is the usual one.
        static SETTINGS: OnceLock<Option<SerializeSettings>> = OnceLock::new();
        let settings = SETTINGS.get_or_init(|| print_settings::settings(PrintColor::fogra39(craft_cms::builtin::COATED_FOGRA39_ICC)).ok());
        let settings = settings.clone().ok_or(WriteError::Pdf(pdf_write::WriteError::InvalidProfile))?;
        Ok((settings, CmsConversion::fogra39(), Fallback::Fogra39))
    };
    let custom = |icc: &[u8], color: PrintColor<'_>| {
        if icc == craft_cms::builtin::COATED_FOGRA39_ICC {
            return fogra39().ok();
        }
        let cms = CmsConversion::new(icc).ok()?;
        let settings = print_settings::settings(color).ok()?;
        Some((settings, cms, Fallback::Profile(icc.into())))
    };
    let found = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| plates::output_intent_profile(doc, page))).ok().flatten();
    if let Some(intent) = found {
        let color = PrintColor {
            profile: &intent.icc,
            condition_identifier: intent.identifier.as_deref().unwrap_or("Custom"),
            condition: intent.condition.as_deref(),
            registry_name: intent.registry_name.as_deref(),
            info: intent.info.as_deref().or(intent.condition.as_deref()),
        };
        if let Some(found) = custom(&intent.icc, color) {
            return Ok(found);
        }
    }
    if let Fallback::Profile(icc) = fallback {
        let color = PrintColor {
            profile: icc,
            condition_identifier: "Custom",
            condition: None,
            registry_name: None,
            info: Some("Custom CMYK printing condition"),
        };
        if let Some(found) = custom(icc, color) {
            return Ok(found);
        }
    }
    fogra39()
}
