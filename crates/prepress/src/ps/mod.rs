//! PostScript and EPS from a print PDF: [`ps_document`] and [`eps_document`].
//!
//! The source is the print PDF an app already writes, because placements, flattening, each
//! placement's output intent, overprint and ink aliases only exist in those bytes. Each page is
//! read and flattened as [`flatten_document`](crate::flatten::flatten_document) does (the same
//! reader, flattener and rasterizer, with the page's own printing condition), then written as
//! PostScript instead of PDF:
//!
//! - **Composite** ([`PsColor::Composite`]): one file. Process colors are DeviceCMYK, a spot
//!   ink alone is a `Separation` space with its CMYK alternate, mixes are `DeviceN`
//!   (LanguageLevel 3). Overprint is `setoverprint`; an overprinting process color that names
//!   fewer than four inks is a `Separation` or `DeviceN` color of just those inks, as the PDF
//!   writer writes it.
//! - **Separations** ([`PsColor::Separations`]): one file per plate, each painting that plate's
//!   tint in DeviceGray. Overprint is decided here, per plate, as the
//!   [plates](crate::plates()) of the Separations Preview decide it: a paint that overprints
//!   and doesn't name the plate's ink leaves the plate alone, and every other paint knocks out.
//!   The plates are the four process inks and each spot ink that prints on its own plate.
//!
//! Every page keeps its full visible area (marks and bleed included) as the PostScript page,
//! and its TrimBox and BleedBox as `/PAGE` pdfmarks. Text in TrueType fonts stays text, as
//! Type 42 fonts ([`PsFonts::TrueType42`]) or CIDFontType 2 fonts under `Identity-H`
//! ([`PsFonts::Cid`]); text in other fonts, and text painted with a gradient, is drawn from its
//! outlines and counted in [`PsReport::outlined_runs`]. JPEG images in DeviceGray or DeviceCMYK
//! keep their DCT data in a composite; other images are written as 8-bit samples, Flate at
//! LanguageLevel 3 and RunLength at LanguageLevel 2.
//!
//! LanguageLevel 2 has no `DeviceN` spaces and no smooth shadings: a mix of inks that includes a
//! spot is written as its CMYK appearance ([`PsReport::converted_to_process`]), an overprinting
//! color of two or three process inks as DeviceCMYK with `setoverprint`
//! ([`PsReport::approximated_overprints`]), and gradients are rasterized at
//! [`FlattenOptions::raster_dpi`] ([`PsReport::rasterized_gradients`]).
//!
//! Not written: CFF and Type 1 fonts as fonts (they are drawn from outlines), 1-bit CCITT image
//! data (the reader gives 8-bit samples), PICT previews, and fonts shared across pages (each
//! page defines its own inside its `save`, so pages stay independent).

use std::ops::Range;
use std::rc::Rc;

use craft_pdf::syntax::object::dict::keys::{BLEED_BOX, TRIM_BOX};
use craft_pdf::{OpenError, open_slice};
use kurbo::Rect;

use crate::flatten::document::{Colors, PageColor, page_box, totals};
use crate::flatten::pdf::{ReadError, ReadOptions, read_page_in};
use crate::flatten::raster::{FlattenPageReport, Rasterizer};
use crate::flatten::write::WriteError;
use crate::flatten::{
    BlendMode, ComplexRegion, DisplayList, FillRule, FlattenDocumentError, FlattenError, FlattenOptions, FlattenTotals, GroupItem, ImageItem, Item,
    flatten,
};
use crate::ink::{Ink, Overprint, Process};
use crate::plates::pdf_text;

mod data;
mod emit;
mod font;
mod preview;
#[cfg(test)]
mod tests;

use emit::{Counts, Target};
use font::FontSet;

/// The PostScript LanguageLevel written.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PsLevel {
    Level2,
    #[default]
    Level3,
}

/// How image data is carried.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PsDataFormat {
    /// ASCII85, so the file is 7-bit clean.
    #[default]
    Ascii,
    /// Bytes as they are, marked with `%%BeginData` comments.
    Binary,
}

/// How TrueType fonts are written.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PsFonts {
    /// Type 42 fonts of at most 256 glyphs each, sharing one `sfnts` array per font.
    #[default]
    TrueType42,
    /// CIDFontType 2 fonts in an `Identity-H` composite font, glyphs addressed by id. Needs
    /// LanguageLevel 3.
    Cid,
}

/// Composite, or one file per plate.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PsColor {
    #[default]
    Composite,
    Separations,
}

/// A TIFF preview in the EPS's binary header, rendered from the source page.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PsPreview {
    /// Pixels per inch.
    pub dpi: f32,
}

impl Default for PsPreview {
    fn default() -> Self {
        Self { dpi: 72.0 }
    }
}

/// How [`ps_document`] and [`eps_document`] read, flatten and write.
#[derive(Clone, Debug, Default)]
pub struct PsOptions {
    pub level: PsLevel,
    pub data_format: PsDataFormat,
    pub fonts: PsFonts,
    pub color: PsColor,
    /// [`eps_document`] only.
    pub preview: Option<PsPreview>,
    pub read: ReadOptions,
    /// [`FlattenOptions::raster_dpi`] is also the resolution of gradients rasterized at
    /// LanguageLevel 2.
    pub flatten: FlattenOptions,
    /// The pages to write, from 0; all when `None`.
    pub pages: Option<Range<usize>>,
    /// Written as `%%Creator`.
    pub creator: String,
}

/// One file: the composite (`plate` is `None`) or one plate, named as its ink is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PsOutput {
    pub plate: Option<String>,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct PsTotals {
    pub pages: usize,
    /// Plate files written; 0 for a composite.
    pub plates: usize,
}

/// What [`ps_document`] and [`eps_document`] did. The counts are per page written, not per
/// plate file.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct PsReport {
    pub totals: PsTotals,
    /// Each page written (from 0) and how it was read and flattened.
    pub pages: Vec<(usize, FlattenPageReport)>,
    /// [`PsReport::pages`] summed.
    pub flatten: FlattenTotals,
    /// Font programs written (a font used on several pages is written on each).
    pub fonts: usize,
    /// Text runs drawn from their outlines: their font isn't TrueType or can't be read, or they
    /// are painted with a gradient.
    pub outlined_runs: usize,
    /// Gradients rasterized (LanguageLevel 2).
    pub rasterized_gradients: usize,
    /// Colors and images of several inks including a spot, written as their CMYK appearance
    /// (LanguageLevel 2 composite).
    pub converted_to_process: usize,
    /// Overprinting colors and images of two or three process inks, written as DeviceCMYK with
    /// `setoverprint`: how a device treats their zero components is its own (LanguageLevel 2
    /// composite; one ink is an exact `Separation` color).
    pub approximated_overprints: usize,
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PsError {
    #[error(transparent)]
    Open(#[from] OpenError),
    #[error("pages {start}..{end} are not pages of the document's {count}")]
    PageRange { start: usize, end: usize, count: usize },
    #[error("page {page} (from 0): {source}")]
    Read { page: usize, source: ReadError },
    #[error("page {page} (from 0): {source}")]
    Flatten { page: usize, source: FlattenError },
    /// Preparing the page's printing condition or rasterizing a complex region failed.
    #[error("page {page} (from 0): {source}")]
    Raster { page: usize, source: WriteError },
    /// The page has something this writer can't write.
    #[error("page {page} (from 0): {what} can't be written as PostScript")]
    Unsupported { page: usize, what: &'static str },
    /// The options ask for something this writer can't do.
    #[error("{0}")]
    Options(&'static str),
    /// An EPS holds one page.
    #[error("an EPS holds one page, not {0}")]
    EpsPages(usize),
    #[error("preview: {0}")]
    Preview(String),
}

/// `pdf` as PostScript: one composite file, or one file per plate ([`PsColor`]), each holding
/// every page in [`PsOptions::pages`]. See the [module docs](self).
pub fn ps_document(pdf: &[u8], opts: &PsOptions) -> Result<(Vec<PsOutput>, PsReport), PsError> {
    check(opts)?;
    let mut report = PsReport::default();
    let mut counts = Counts::default();
    let doc = open_slice(pdf, None)?;
    let title = doc.metadata().title.as_deref().map(|t| pdf_text(t).trim().to_string()).unwrap_or_default();
    let header = Header { title, creator: opts.creator.clone(), level: opts.level, binary: opts.data_format == PsDataFormat::Binary };
    let outputs = match opts.color {
        PsColor::Composite => {
            let mut pages = Vec::new();
            report.pages = each_page(&doc, opts, &mut |page| {
                let body = emit::page(&page, opts, Target::Composite, &mut counts)?;
                pages.push((page.geometry, body));
                Ok(())
            })?;
            let spots = counts.spots.clone();
            vec![PsOutput { plate: None, bytes: header.document(&pages, None, &spots) }]
        }
        PsColor::Separations => {
            let mut prepared = Vec::new();
            report.pages = each_page(&doc, opts, &mut |page| {
                prepared.push(page);
                Ok(())
            })?;
            let mut plates: Vec<String> = Process::ALL.iter().map(|p| p.name().to_string()).collect();
            for page in &prepared {
                for (id, ink) in page.list.inks.iter() {
                    if let Ink::Spot(spot) = ink
                        && page.list.inks.plate_of(id) == Some(id)
                        && !plates.contains(&spot.name)
                    {
                        plates.push(spot.name.clone());
                    }
                }
            }
            let mut outputs = Vec::with_capacity(plates.len());
            for (n, plate) in plates.iter().enumerate() {
                let mut pages = Vec::with_capacity(prepared.len());
                // Counted once, on the first plate.
                let mut scratch = Counts::default();
                let counts = if n == 0 { &mut counts } else { &mut scratch };
                for page in &prepared {
                    let id = page.list.inks.lookup(plate).filter(|id| page.list.inks.plate_of(*id) == Some(*id));
                    pages.push((page.geometry, emit::page(page, opts, Target::Plate(id), counts)?));
                }
                outputs.push(PsOutput { plate: Some(plate.clone()), bytes: header.document(&pages, Some(plate), &[]) });
            }
            report.totals.plates = outputs.len();
            outputs
        }
    };
    counts.add_to(&mut report);
    report.totals.pages = report.pages.len();
    report.flatten = totals(&report.pages);
    Ok((outputs, report))
}

/// One page of `pdf` as an EPS: header comments with its bounding box, the composite program and,
/// with [`PsOptions::preview`], a TIFF preview behind a binary header. [`PsOptions::pages`]
/// must name one page (or the document must have one), and [`PsOptions::color`] must be
/// composite.
pub fn eps_document(pdf: &[u8], opts: &PsOptions) -> Result<(PsOutput, PsReport), PsError> {
    check(opts)?;
    if opts.color != PsColor::Composite {
        return Err(PsError::Options("an EPS is composite; separate it where it is placed"));
    }
    let doc = open_slice(pdf, None)?;
    let count = doc.pages().len();
    let range = opts.pages.clone().unwrap_or(0..count);
    if range.len() != 1 {
        return Err(PsError::EpsPages(range.len()));
    }
    let mut report = PsReport::default();
    let mut counts = Counts::default();
    let mut body = None;
    report.pages = each_page(&doc, opts, &mut |page| {
        body = Some((page.geometry, emit::page(&page, opts, Target::Composite, &mut counts)?));
        Ok(())
    })?;
    let Some((geometry, body)) = body else { return Err(PsError::EpsPages(0)) };
    let title = doc.metadata().title.as_deref().map(|t| pdf_text(t).trim().to_string()).unwrap_or_default();
    let header = Header { title, creator: opts.creator.clone(), level: opts.level, binary: opts.data_format == PsDataFormat::Binary };
    let spots = counts.spots.clone();
    let program = header.eps(geometry, &body, &spots);
    let bytes = match opts.preview {
        Some(preview) => {
            let tiff = preview::tiff(&doc, range.start, preview.dpi)?;
            craft_eps::wrap(&program, Some(&tiff), None).map_err(|e| PsError::Preview(e.to_string()))?
        }
        None => program,
    };
    counts.add_to(&mut report);
    report.totals.pages = report.pages.len();
    report.flatten = totals(&report.pages);
    Ok((PsOutput { plate: None, bytes }, report))
}

fn check(opts: &PsOptions) -> Result<(), PsError> {
    if opts.fonts == PsFonts::Cid && opts.level == PsLevel::Level2 {
        return Err(PsError::Options("CID-keyed fonts need LanguageLevel 3"));
    }
    Ok(())
}

/// Where a page is: its visible area in page space (y down) and its boxes, clipped to it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Geometry {
    pub page: Rect,
    pub trim: Option<Rect>,
    pub bleed: Option<Rect>,
}

impl Geometry {
    /// `r` in PostScript default user space (y up, origin at the page's bottom left).
    pub fn user_space(&self, r: Rect) -> [f64; 4] {
        [r.x0 - self.page.x0, self.page.y1 - r.y1, r.x1 - self.page.x0, self.page.y1 - r.y0]
    }
}

/// A page read and flattened, with the images of its complex regions in place.
pub(crate) struct Prepared {
    pub index: usize,
    pub geometry: Geometry,
    pub list: DisplayList,
    /// What prints, in order: the flattened items, and each complex region's image clipped to
    /// the region.
    pub items: Vec<Item>,
    pub color: Rc<PageColor>,
    /// The page's fonts, written once for every plate.
    pub fonts: FontSet,
}

/// Reads, flattens and rasterizes each page of [`PsOptions::pages`] and hands it to `f`.
fn each_page(
    doc: &craft_pdf::syntax::Pdf,
    opts: &PsOptions,
    f: &mut dyn FnMut(Prepared) -> Result<(), PsError>,
) -> Result<Vec<(usize, FlattenPageReport)>, PsError> {
    let count = doc.pages().len();
    let range = opts.pages.clone().unwrap_or(0..count);
    if range.is_empty() || range.end > count {
        return Err(PsError::PageRange { start: range.start, end: range.end, count });
    }
    let mut colors = Colors::new(doc, &opts.read.fallback);
    let mut reports = Vec::with_capacity(range.len());
    for index in range {
        let color = colors.get(index).map_err(|e| match e {
            FlattenDocumentError::Read { page, source } => PsError::Read { page, source },
            FlattenDocumentError::Write { page, source } => PsError::Raster { page, source },
            _ => PsError::Read { page: index, source: ReadError::NoPage(index) },
        })?;
        let p = doc.pages().get(index).ok_or(PsError::Read { page: index, source: ReadError::NoPage(index) })?;
        let (list, read) = read_page_in(doc, index, &opts.read, color.read.clone()).map_err(|source| PsError::Read { page: index, source })?;
        let flat = flatten(&list, &opts.flatten).map_err(|source| PsError::Flatten { page: index, source })?;
        let mut raster = Rasterizer::new(&list, color.settings.clone(), color.condition.clone(), &color.cms, opts.flatten.raster_dpi);
        let items = with_regions(&flat.items, &flat.complex, &mut |r| raster.region(r)).map_err(|source| PsError::Raster { page: index, source })?;
        let raster_report = raster.report().clone();
        let geometry = Geometry { page: list.page, trim: page_box(p, list.page, TRIM_BOX), bleed: page_box(p, list.page, BLEED_BOX) };
        reports.push((index, FlattenPageReport { read, flatten: flat.report, raster: raster_report }));
        let fonts = FontSet::new(&items, index, opts.fonts);
        f(Prepared { index, geometry, list, items, color, fonts })?;
    }
    Ok(reports)
}

/// `items` with, before item `at` of each complex region, the image `raster` gives it, clipped
/// to the region and without overprint: the order the PDF writer draws them in.
fn with_regions(
    items: &[Item],
    regions: &[ComplexRegion],
    raster: &mut dyn FnMut(&ComplexRegion) -> Result<Option<ImageItem>, WriteError>,
) -> Result<Vec<Item>, WriteError> {
    let mut sorted: Vec<&ComplexRegion> = regions.iter().collect();
    sorted.sort_by_key(|r| r.at);
    let mut sorted = sorted.into_iter().peekable();
    let mut out = Vec::with_capacity(items.len() + regions.len());
    for at in 0..=items.len() {
        while let Some(region) = sorted.next_if(|r| r.at <= at || at == items.len()) {
            let Some(image) = raster(region)? else { continue };
            out.push(Item::Group(GroupItem {
                opacity: 1.0,
                blend: BlendMode::Normal,
                isolated: false,
                knockout: false,
                clip: Some(region.path.clone()),
                clip_rule: FillRule::NonZero,
                mask: None,
                actual_text: None,
                children: vec![Item::Image(ImageItem { overprint: Overprint::default(), ..image })],
            }));
        }
        if let Some(item) = items.get(at) {
            out.push(item.clone());
        }
    }
    Ok(out)
}

/// The document's header comments and prolog.
struct Header {
    title: String,
    creator: String,
    level: PsLevel,
    binary: bool,
}

/// Bound in the prolog; the page programs use the short names.
const PROLOG: &str = "/CraftPS 40 dict dup begin
/m /moveto load def /l /lineto load def /c /curveto load def /h /closepath load def
/f /fill load def /f* /eofill load def /W { clip newpath } bind def /W* { eoclip newpath } bind def
/q /gsave load def /Q /grestore load def /cm /concat load def
/g /setgray load def /k /setcmykcolor load def /S /show load def
end def
";

impl Header {
    fn level(&self) -> u8 {
        match self.level {
            PsLevel::Level2 => 2,
            PsLevel::Level3 => 3,
        }
    }

    fn comments(&self, out: &mut Vec<u8>, bbox: [f64; 4]) {
        let mut s = String::new();
        if !self.creator.is_empty() {
            s.push_str(&format!("%%Creator: {}\n", dsc_text(&self.creator)));
        }
        if !self.title.is_empty() {
            s.push_str(&format!("%%Title: {}\n", dsc_text(&self.title)));
        }
        s.push_str(&format!(
            "%%BoundingBox: {} {} {} {}\n%%HiResBoundingBox: {} {} {} {}\n",
            bbox[0].floor(),
            bbox[1].floor(),
            bbox[2].ceil(),
            bbox[3].ceil(),
            num(bbox[0]),
            num(bbox[1]),
            num(bbox[2]),
            num(bbox[3]),
        ));
        s.push_str(&format!("%%LanguageLevel: {}\n", self.level()));
        s.push_str(if self.binary { "%%DocumentData: Binary\n" } else { "%%DocumentData: Clean7Bit\n" });
        out.extend_from_slice(s.as_bytes());
    }

    fn prolog(out: &mut Vec<u8>, pdfmark: bool) {
        out.extend_from_slice(b"%%BeginProlog\n%%BeginResource: procset CraftPS 1.0 0\n");
        out.extend_from_slice(PROLOG.as_bytes());
        if pdfmark {
            out.extend_from_slice(b"/pdfmark where { pop } { userdict /pdfmark /cleartomark load put } ifelse\n");
        }
        out.extend_from_slice(b"%%EndResource\n%%EndProlog\n");
    }

    /// A PostScript document of `pages`; `plate` names the plate a separation prints.
    fn document(&self, pages: &[(Geometry, Vec<u8>)], plate: Option<&str>, spots: &[String]) -> Vec<u8> {
        let mut out = b"%!PS-Adobe-3.0\n".to_vec();
        let (w, h) = pages.iter().fold((0.0_f64, 0.0_f64), |(w, h), (g, _)| (w.max(g.page.width()), h.max(g.page.height())));
        self.comments(&mut out, [0.0, 0.0, w, h]);
        out.extend_from_slice(format!("%%Pages: {}\n", pages.len()).as_bytes());
        match plate {
            Some(plate) => out.extend_from_slice(format!("%%PlateColor: {}\n", dsc_text(plate)).as_bytes()),
            None => {
                out.extend_from_slice(b"%%DocumentProcessColors: Cyan Magenta Yellow Black\n");
                if !spots.is_empty() {
                    let names: Vec<String> = spots.iter().map(|s| craft_ps::write::string(s)).collect();
                    out.extend_from_slice(format!("%%DocumentCustomColors: {}\n", names.join(" ")).as_bytes());
                }
            }
        }
        out.extend_from_slice(b"%%EndComments\n");
        Self::prolog(&mut out, true);
        out.extend_from_slice(b"%%BeginSetup\n%%EndSetup\n");
        for (n, (geometry, body)) in pages.iter().enumerate() {
            let (w, h) = (geometry.page.width(), geometry.page.height());
            let mut s = format!("%%Page: {0} {0}\n%%PageBoundingBox: 0 0 {1} {2}\n", n + 1, w.ceil(), h.ceil());
            s.push_str("%%BeginPageSetup\n");
            s.push_str(&format!("/setpagedevice where {{ pop << /PageSize [{} {}] >> setpagedevice }} if\n", num(w), num(h)));
            let mut marks = String::new();
            for (key, r) in [("TrimBox", geometry.trim), ("BleedBox", geometry.bleed)] {
                if let Some(r) = r {
                    let [x0, y0, x1, y1] = geometry.user_space(r);
                    marks.push_str(&format!(" /{key} [{} {} {} {}]", num(x0), num(y0), num(x1), num(y1)));
                }
            }
            if !marks.is_empty() {
                s.push_str(&format!("[{marks} /PAGE pdfmark\n"));
            }
            s.push_str("%%EndPageSetup\n");
            out.extend_from_slice(s.as_bytes());
            out.extend_from_slice(body);
            out.extend_from_slice(b"showpage\n%%PageTrailer\n");
        }
        out.extend_from_slice(b"%%Trailer\n%%EOF\n");
        out
    }

    /// An EPS of one page.
    fn eps(&self, geometry: Geometry, body: &[u8], spots: &[String]) -> Vec<u8> {
        let mut out = b"%!PS-Adobe-3.0 EPSF-3.0\n".to_vec();
        self.comments(&mut out, [0.0, 0.0, geometry.page.width(), geometry.page.height()]);
        out.extend_from_slice(b"%%Pages: 1\n%%DocumentProcessColors: Cyan Magenta Yellow Black\n");
        if !spots.is_empty() {
            let names: Vec<String> = spots.iter().map(|s| craft_ps::write::string(s)).collect();
            out.extend_from_slice(format!("%%DocumentCustomColors: {}\n", names.join(" ")).as_bytes());
        }
        out.extend_from_slice(b"%%EndComments\n");
        Self::prolog(&mut out, false);
        out.extend_from_slice(b"%%Page: 1 1\n");
        out.extend_from_slice(body);
        out.extend_from_slice(b"showpage\n%%Trailer\n%%EOF\n");
        out
    }
}

/// `text` for a DSC comment: printable ASCII, other characters as `?`.
fn dsc_text(text: &str) -> String {
    text.chars().map(|c| if c.is_ascii_graphic() || c == ' ' { c } else { '?' }).collect()
}

fn num(v: f64) -> String {
    craft_ps::write::num(v)
}
