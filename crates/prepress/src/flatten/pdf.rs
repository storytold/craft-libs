//! The PDF-in adapter: a page read through hayro-interpret into a [`DisplayList`].
//!
//! - Coordinates are page space (`Page::initial_transform(true)`): rotation and the crop box
//!   are applied.
//! - Colors stay inks, as plate mode separates them (`InterpreterSettings::plate`):
//!   `Separation` and `DeviceN` colorants are their inks, DeviceCMYK its four process tints,
//!   DeviceGray black only (C, M and Y at 0), and RGB, Lab and ICC colors convert to CMYK through
//!   the page's output intent, else [`ReadOptions::fallback`]. Spot inks join
//!   [`DisplayList::inks`] with their alternate: CMYK when the space's alternate is DeviceCMYK,
//!   else Lab measured from the sRGB appearance.
//! - Overprint: an item's [`Overprint::fill`] and [`Overprint::stroke`] both hold the flag that
//!   governed its paint (`op` for fills, glyphs and images, `OP` for strokes).
//!   [`Overprint::nonzero`] is `OPM 1` for colors set in DeviceCMYK only, where it applies.
//! - Constant opacity, blend modes and soft masks wrap each paint in a group of its own; a text
//!   run shares one wrapper.
//! - Images keep their own space where it is inks: DeviceGray, DeviceCMYK, and the colorants of
//!   `Separation` and `DeviceN` (Indexed images take their base's). Other spaces convert to CMYK
//!   as colors do. `/Decode` and `/Matte` are applied; transfer functions are not, as on a plate.
//! - Stencil masks become fills traced from the mask, or images when the mask is too detailed.
//! - Axial and radial shadings become [`Paint::Gradient`]s with stops sampled in ink space.
//!   Function-based and mesh shadings, and tiling patterns past [`MAX_TILING_CELLS`] cells or
//!   [`MAX_TILING_ITEMS`] items, become images at [`ReadOptions::raster_dpi`]; other tiling patterns expand into their cells.

use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use craft_pdf::interpret::color::{CmykSimulation, ColorSpace, Ink as PdfInk, InkPaint, InkSpace, SpotAlternate};
use craft_pdf::interpret::font::{Glyph, OutlineGlyph};
use craft_pdf::interpret::hayro_cmap::BfString;
use craft_pdf::interpret::pattern::{Pattern, ShadingPattern, TilingPattern};
use craft_pdf::interpret::shading::ShadingType;
use craft_pdf::interpret::{
    BlendMode as PdfBlend, CacheKey, ClipPath, Context, Device, FillRule as PdfFillRule, GlyphDrawMode, Image, InterpreterCache, InterpreterSettings,
    MaskType, Paint as PdfPaint, PathDrawMode, RasterImage, SoftMask, StencilImage, StrokeProps, TransformExt, interpret_page,
};
use craft_pdf::syntax::Filter;
use craft_pdf::syntax::Pdf;
use craft_pdf::syntax::object::dict::keys::{ANNOTS, AP, BITS_PER_COMPONENT, COLORSPACE, DECODE, DECODE_PARMS, F, HEIGHT, N, WIDTH};
use craft_pdf::syntax::object::{Array, Dict, Name, Stream};
use craft_pdf::syntax::page::Page;
use craft_pdf::{OpenError, open_slice};
use kurbo::{Affine, BezPath, PathEl, Point, Rect, Shape, Stroke, StrokeOpts};

use super::font::FontCache;
use super::shade::{self, Canvas, StopInks};
use super::{
    BlendMode, DisplayList, FillItem, FillRule, GradientGeometry, GroupItem, ImageItem, ImageSpace, Item, Mask, MaskKind, Paint, PlacedGlyph,
    TextFont, TextItem,
};
use crate::cms::{ProfileError, srgb8_to_lab};
use crate::ink::{Alternate, Ink, InkColor, InkId, InkSet, Overprint, Process, Spot};
use crate::plates::{self, Conversion, Fallback};

/// Curve flattening tolerance of outlined strokes, in points.
const STROKE_TOLERANCE: f64 = 0.01;

/// The width of a `0 w` stroke, the thinnest line a device can draw, in points.
const HAIRLINE: f64 = 0.1;

/// Dashes one stroke may be cut into; past this it is stroked solid, as hayro draws it.
const MAX_DASHES_PER_STROKE: f64 = 250_000.0;

/// Tiling cells one pattern fill expands into; past this the fill is read as an image.
pub const MAX_TILING_CELLS: usize = 10_000;

/// Items one pattern fill expands into, counting those inside groups and nested pattern fills;
/// past this the fill is read as an image. A hundred per cell at [`MAX_TILING_CELLS`].
pub const MAX_TILING_ITEMS: usize = 1_000_000;

/// Rectangles a stencil mask may trace into; past this it is read as an image.
const MAX_STENCIL_RECTS: usize = 4_096;

/// Pixels one image read from a shading or tiling pattern may hold; a larger area is sampled
/// at a lower resolution.
const MAX_RASTER_PIXELS: f64 = 16_777_216.0;

/// How much finer than the image a tiling cell is drawn, so that small cells keep their shapes.
const CELL_OVERSAMPLE: f64 = 4.0;

/// Pixels the drawing of one tiling cell may hold.
const MAX_CELL_PIXELS: f64 = 4_194_304.0;

/// Cell samples one image read from a tiling pattern may take; a pattern whose cells overlap is
/// sampled at a lower resolution. Four per pixel at [`MAX_RASTER_PIXELS`], because a cell no
/// larger than its step meets three others at its corners.
const MAX_TILING_SAMPLES: f64 = 4.0 * MAX_RASTER_PIXELS;

/// Inks a tiling pattern's canvases hold at their full pixel limits. Each pixel holds a value
/// per ink of the page, so a page with more inks is sampled at a lower resolution, keeping the
/// canvases (and their sampling) at the memory and work of CMYK ones.
const CANVAS_INKS: f64 = 4.0;

/// How [`read_page`] reads.
#[derive(Clone, Debug)]
pub struct ReadOptions {
    /// The printing condition RGB, Lab and ICC colors convert into when the page has no output
    /// intent with a usable CMYK profile.
    pub fallback: Fallback,
    /// Read the page's annotation appearances too. When `false`, the print-flagged ones with an
    /// appearance are left out and counted ([`ReadReport::dropped_annotations`]).
    pub annotations: bool,
    /// The resolution, in pixels per inch, of images read from function-based and mesh
    /// shadings and from tiling patterns past [`MAX_TILING_CELLS`] or [`MAX_TILING_ITEMS`].
    pub raster_dpi: f32,
}

impl Default for ReadOptions {
    fn default() -> Self {
        Self { fallback: Fallback::default(), annotations: false, raster_dpi: 300.0 }
    }
}

/// What [`read_page`] read, converted or left out.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ReadReport {
    /// How process color that isn't CMYK became CMYK.
    pub conversion: Conversion,
    /// Raster images read.
    pub images: usize,
    /// Of [`Self::images`], those whose samples were converted into CMYK (RGB, Lab, ICC and
    /// CIE-based spaces).
    pub converted_images: usize,
    /// Images whose `/SMask` or `/Mask` had another size than the image, resampled to it.
    pub resampled_masks: usize,
    /// Images and stencil masks that didn't decode, left out.
    pub unreadable_images: usize,
    /// Stencil masks (`/ImageMask`) read.
    pub stencils: usize,
    /// Axial and radial shadings read as gradients.
    pub gradients: usize,
    /// Function-based and mesh shadings read as images.
    pub rasterized_shadings: usize,
    /// Tiling pattern cells expanded.
    pub tiling_cells: usize,
    /// Tiling pattern fills past [`MAX_TILING_CELLS`] cells or [`MAX_TILING_ITEMS`] items, read
    /// as images.
    pub rasterized_tilings: usize,
    /// Tiling pattern fills left out: cells stacked too deep to sample, or canvases too large.
    pub dropped_tilings: usize,
    /// Glyphs in text rendering mode 3, dropped.
    pub invisible_glyphs: usize,
    /// Spot inks past the ink set's capacity, whose tints were dropped.
    pub dropped_inks: usize,
    /// Annotations that would print (Print flag set, not Hidden, with a normal appearance), left
    /// out because [`ReadOptions::annotations`] is `false`.
    pub dropped_annotations: usize,
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ReadError {
    #[error(transparent)]
    Open(#[from] OpenError),
    #[error("page {0} does not exist")]
    NoPage(usize),
    #[error("the fallback profile can't be a printing condition: {0}")]
    Profile(#[from] ProfileError),
    #[error("the PDF interpreter failed on the page")]
    Interpret,
}

/// Reads page `page` (from 0) of `pdf` into a display list. Never panics.
pub fn read_page(pdf: &[u8], page: usize, opts: &ReadOptions) -> Result<(DisplayList, ReadReport), ReadError> {
    let doc = open_slice(pdf, None)?;
    let color = read_color(&doc, page, &opts.fallback)?;
    read_page_in(&doc, page, opts, color)
}

/// How a page's process color converts: its output intent's simulation, else `fallback`'s.
pub(crate) type ReadColor = (Option<CmykSimulation>, Conversion);

/// The [`ReadColor`] of page `page`; parsing the profile is the slow part, so a document
/// reads it once per output intent.
pub(crate) fn read_color(doc: &Pdf, page: usize, fallback: &Fallback) -> Result<ReadColor, ReadError> {
    match catch_unwind(AssertUnwindSafe(|| plates::output_intent(doc, page))) {
        Ok(Some(found)) => Ok((Some(found.0), found.1)),
        Ok(None) | Err(_) => plates::fallback(fallback).map_err(|e| match e {
            plates::Error::Profile(p) => ReadError::Profile(p),
            _ => ReadError::Interpret,
        }),
    }
}

/// [`read_page`] of an open document, with the page's color already read.
pub(crate) fn read_page_in(doc: &Pdf, page: usize, opts: &ReadOptions, color: ReadColor) -> Result<(DisplayList, ReadReport), ReadError> {
    let (simulation, conversion) = color;
    let pages = doc.pages();
    let p = pages.get(page).ok_or(ReadError::NoPage(page))?;
    let (w, h) = p.render_dimensions();
    let bbox = Rect::new(0.0, 0.0, f64::from(w), f64::from(h));
    let report = ReadReport {
        conversion,
        images: 0,
        converted_images: 0,
        resampled_masks: 0,
        unreadable_images: 0,
        stencils: 0,
        gradients: 0,
        rasterized_shadings: 0,
        tiling_cells: 0,
        rasterized_tilings: 0,
        dropped_tilings: 0,
        invisible_glyphs: 0,
        dropped_inks: 0,
        dropped_annotations: if opts.annotations { 0 } else { catch_unwind(AssertUnwindSafe(|| printing_annotations(p))).unwrap_or(0) },
    };
    let dpi = if opts.raster_dpi.is_finite() && opts.raster_dpi > 0.0 { opts.raster_dpi } else { 300.0 };
    let mut shared = Shared {
        simulation,
        inks: InkSet::new(),
        spots: HashMap::new(),
        report,
        masks: HashMap::new(),
        fonts: FontCache::new(),
        scale: f64::from(dpi) / 72.0,
    };

    let items = catch_unwind(AssertUnwindSafe(|| {
        let cache = InterpreterCache::new();
        let settings = InterpreterSettings { render_annotations: opts.annotations, ..Default::default() };
        let mut context = Context::new(p.initial_transform(true).to_kurbo(), bbox, &cache, p.xref(), settings);
        let mut reader = Reader::new(&mut shared, false, bbox);
        interpret_page(p, &mut context, &mut reader);
        reader.finish()
    }))
    .map_err(|_| ReadError::Interpret)?;

    let Shared { inks, report, fonts, .. } = shared;
    let fonts = fonts.finish();
    let mut items = items;
    set_fonts(&mut items, &fonts);
    Ok((DisplayList { page: bbox, inks, items }, report))
}

/// The annotations of `page` a printer would paint: Print flag (bit 3) set, Hidden (bit 2)
/// clear, with a normal appearance (`/AP /N`).
fn printing_annotations(page: &Page<'_>) -> usize {
    let Some(annots) = page.raw().get::<Array<'_>>(ANNOTS) else { return 0 };
    annots
        .iter::<Dict<'_>>()
        .filter(|a| a.get::<u32>(F).unwrap_or(0) & (4 | 2) == 4 && a.get::<Dict<'_>>(AP).is_some_and(|ap| ap.contains_key(N)))
        .count()
}

/// State shared by the page's reader and the readers of its soft masks and Type 3 glyphs.
struct Shared {
    simulation: Option<CmykSimulation>,
    inks: InkSet,
    spots: HashMap<PdfInk, Option<InkId>>,
    report: ReadReport,
    /// Soft masks read so far, by hayro's cache key.
    masks: HashMap<u128, Option<Mask>>,
    fonts: FontCache,
    /// Raster pixels per point.
    scale: f64,
}

/// How a paint composites: what wraps it in a group of its own.
#[derive(Clone)]
struct Look<'a> {
    opacity: f32,
    blend: BlendMode,
    mask: Option<SoftMask<'a>>,
}

impl Look<'_> {
    fn same(&self, other: &Self) -> bool {
        self.opacity == other.opacity
            && self.blend == other.blend
            && self.mask.as_ref().map(CacheKey::cache_key) == other.mask.as_ref().map(CacheKey::cache_key)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FrameKind {
    Root,
    Clip,
    Group,
}

struct Frame {
    kind: FrameKind,
    group: GroupItem,
}

/// A text run being collected.
struct Run<'a> {
    /// The source font's key; `None` for Type 3 glyphs.
    font: Option<u128>,
    /// The linear part of glyph space (one unit per em) to page space.
    linear: Affine,
    look: Look<'a>,
    item: TextItem,
}

/// What a paint paints with.
enum Painted<'a> {
    /// A paint an item carries.
    Flat(Paint, Look<'a>, Overprint),
    /// A pattern read over the painted area ([`Reader::area_fill`]).
    Area,
}

/// The hayro-interpret device. `glyph` readers read a Type 3 glyph procedure: they collect
/// paints without their own transparency, which the glyph's run carries.
struct Reader<'s, 'a> {
    shared: &'s mut Shared,
    glyph: bool,
    /// Where the content can show, before clips: the page, or unbounded for a tiling cell,
    /// whose copies land elsewhere.
    bounds: Rect,
    frames: Vec<Frame>,
    soft_mask: Option<SoftMask<'a>>,
    blend: BlendMode,
    /// `op`, `OP` and `OPM`.
    overprint: (bool, bool, u8),
    run: Option<Run<'a>>,
}

fn plain_group(clip: Option<BezPath>) -> GroupItem {
    GroupItem { opacity: 1.0, blend: BlendMode::Normal, isolated: false, knockout: false, clip, mask: None, actual_text: None, children: Vec::new() }
}

impl<'s, 'a> Reader<'s, 'a> {
    fn new(shared: &'s mut Shared, glyph: bool, bounds: Rect) -> Self {
        Self {
            shared,
            glyph,
            bounds,
            frames: vec![Frame { kind: FrameKind::Root, group: plain_group(None) }],
            soft_mask: None,
            blend: BlendMode::Normal,
            overprint: (false, false, 0),
            run: None,
        }
    }

    /// The items read, with every open clip and group closed.
    fn finish(mut self) -> Vec<Item> {
        self.flush();
        while self.frames.len() > 1 {
            self.close_top();
        }
        self.frames.pop().map(|f| f.group.children).unwrap_or_default()
    }

    fn add(&mut self, item: Item) {
        if let Some(frame) = self.frames.last_mut() {
            frame.group.children.push(item);
        }
    }

    fn close_top(&mut self) {
        let Some(frame) = self.frames.pop_if(|f| f.kind != FrameKind::Root) else { return };
        if frame.group.children.is_empty() {
            return;
        }
        // A group that changes nothing (hayro wraps every image in one) adds its children.
        let g = &frame.group;
        let plain = g.opacity == 1.0
            && g.blend == BlendMode::Normal
            && !g.isolated
            && !g.knockout
            && g.clip.is_none()
            && g.mask.is_none()
            && g.actual_text.is_none();
        if plain {
            for item in frame.group.children {
                self.add(item);
            }
        } else {
            self.add(Item::Group(frame.group));
        }
    }

    /// The page area content can show in, through every open clip.
    fn clip_bounds(&self) -> Rect {
        let mut r = self.bounds;
        for frame in &self.frames {
            if let Some(clip) = &frame.group.clip {
                r = r.intersect(clip.bounding_box());
            }
        }
        r
    }

    /// Raster pixels per point for an image over `area`, within [`MAX_RASTER_PIXELS`].
    fn raster_scale(&self, area: Rect) -> f64 {
        limit_scale(self.shared.scale, area, MAX_RASTER_PIXELS)
    }

    /// Closes the innermost frame of `kind`, and the frames opened inside it.
    fn close(&mut self, kind: FrameKind) {
        self.flush();
        let Some(at) = self.frames.iter().rposition(|f| f.kind == kind) else { return };
        while self.frames.len() > at {
            self.close_top();
        }
    }

    fn look(&self, opacity: f32) -> Look<'a> {
        if self.glyph {
            return Look { opacity: 1.0, blend: BlendMode::Normal, mask: None };
        }
        Look { opacity, blend: self.blend, mask: self.soft_mask.clone() }
    }

    /// Adds `item`, in a group of its own when `look` is transparent.
    fn push(&mut self, item: Item, look: &Look<'a>) {
        let mask = look.mask.as_ref().and_then(|m| self.mask(m));
        let item = if look.opacity < 1.0 || look.blend != BlendMode::Normal || mask.is_some() {
            Item::Group(GroupItem { opacity: look.opacity, blend: look.blend, mask: mask.map(Box::new), children: vec![item], ..plain_group(None) })
        } else {
            item
        };
        self.add(item);
    }

    fn flush(&mut self) {
        let Some(mut run) = self.run.take() else { return };
        if run.item.text.as_deref() == Some("") {
            run.item.text = None;
        }
        self.push(Item::Text(run.item), &run.look);
    }

    fn mask(&mut self, mask: &SoftMask<'a>) -> Option<Mask> {
        let key = mask.cache_key();
        if let Some(read) = self.shared.masks.get(&key) {
            return read.clone();
        }
        // A mask that refers to itself reads as no mask.
        self.shared.masks.insert(key, None);
        let mut reader = Reader::new(&mut *self.shared, false, self.bounds);
        mask.interpret(&mut reader);
        let children = reader.finish();
        let kind = match mask.mask_type() {
            MaskType::Luminosity => MaskKind::Luminosity,
            MaskType::Alpha => MaskKind::Alpha,
        };
        let backdrop = match kind {
            MaskKind::Luminosity => self.ink_color(&mask.background_color()),
            MaskKind::Alpha => None,
        };
        let transfer = mask.transfer_function().map(|tf| (0..=255_u8).map(|v| (tf.apply(f32::from(v) / 255.0) * 255.0).round() as u8).collect());
        let group = GroupItem { isolated: true, children, ..plain_group(None) };
        let read = Mask { kind, group, backdrop, transfer };
        self.shared.masks.insert(key, Some(read.clone()));
        Some(read)
    }

    fn ink(&mut self, ink: &PdfInk, alternate: impl FnOnce() -> Option<SpotAlternate>) -> Option<InkId> {
        let process = match ink {
            PdfInk::Cyan => Process::Cyan,
            PdfInk::Magenta => Process::Magenta,
            PdfInk::Yellow => Process::Yellow,
            PdfInk::Black => Process::Black,
            PdfInk::Spot(name) => {
                if let Some(id) = self.shared.spots.get(ink) {
                    return *id;
                }
                let alternate = match alternate() {
                    Some(SpotAlternate::Cmyk(cmyk)) => Alternate::Cmyk(cmyk),
                    Some(SpotAlternate::Srgb(rgb)) => Alternate::Lab(srgb8_to_lab(rgb).unwrap_or([0.0; 3])),
                    None => Alternate::Lab([0.0; 3]),
                };
                let id = self.shared.inks.add(Ink::Spot(Spot { name: name.to_string(), alternate, solidity: None }));
                self.shared.spots.insert(ink.clone(), id);
                return id;
            }
        };
        Some(InkId::process(process))
    }

    /// The color as inks; `None` when it paints nothing (the `None` colorant).
    fn ink_color(&mut self, color: &craft_pdf::interpret::color::Color) -> Option<InkColor> {
        match color.ink_paint(self.shared.simulation.as_ref())? {
            InkPaint::Tints(tints) => {
                let mut out = Vec::with_capacity(tints.len());
                for (ink, tint) in tints {
                    match self.ink(&ink, || color.spot_alternate(&ink)) {
                        Some(id) => out.push((id, tint)),
                        None => self.shared.report.dropped_inks += 1,
                    }
                }
                Some(InkColor::Tints(out))
            }
            InkPaint::All(tint) => Some(InkColor::All(tint)),
            InkPaint::None => None,
        }
    }

    /// How values of `space` (colors of `cs`) become samples or stops: the inks, and which
    /// values to keep. `None` when the space paints nothing.
    fn space_inks(&mut self, space: &InkSpace, cs: &ColorSpace) -> Option<(Inks, Channels)> {
        let process = || Process::ALL.iter().map(|p| InkId::process(*p)).collect();
        Some(match space {
            InkSpace::None => return None,
            InkSpace::Gray => (Inks::Gray, Channels::All),
            InkSpace::Cmyk { .. } => (Inks::Cmyk(process()), Channels::All),
            InkSpace::All => (Inks::List(self.shared.inks.iter().map(|(id, _)| id).collect()), Channels::Spread(self.shared.inks.len())),
            InkSpace::Inks(list) => {
                let mut ids = Vec::with_capacity(list.len());
                let mut keep = Vec::with_capacity(list.len());
                for ink in list {
                    let id = self.ink(ink, || cs.spot_alternate(ink));
                    if id.is_none() {
                        self.shared.report.dropped_inks += 1;
                    }
                    keep.push(id.is_some());
                    ids.extend(id);
                }
                if ids.is_empty() {
                    return None;
                }
                let channels = if keep.iter().all(|k| *k) { Channels::All } else { Channels::Keep(keep) };
                (Inks::List(ids), channels)
            }
        })
    }

    /// The paint and how it paints; `None` when it paints nothing.
    fn paint(&mut self, paint: &PdfPaint<'a>, stroke: bool) -> Option<Painted<'a>> {
        let (op, big_op, mode) = self.overprint;
        let flag = if stroke { big_op } else { op };
        let mut overprint = Overprint { fill: flag, stroke: flag, nonzero: false };
        match paint {
            PdfPaint::Color(color) => {
                overprint.nonzero = mode == 1 && color.is_device_cmyk();
                let ink = self.ink_color(color)?;
                Some(Painted::Flat(Paint::Solid(ink), self.look(color.opacity()), overprint))
            }
            PdfPaint::Pattern(pattern) => match pattern.as_ref() {
                // A gradient that needs a clip (a bounding box, or an end that doesn't extend) or a
                // background is read over its area.
                Pattern::Shading(s)
                    if s.shading.clip_path.is_none()
                        && s.shading.background.is_none()
                        && matches!(s.shading.shading_type.as_ref(), ShadingType::RadialAxial { extend: [true, true], .. }) =>
                {
                    overprint.nonzero = mode == 1 && s.shading.color_space.is_device_cmyk();
                    let (gradient, _) = self.gradient(s)?;
                    Some(Painted::Flat(gradient, self.look(s.opacity), overprint))
                }
                Pattern::Shading(_) | Pattern::Tiling(_) => Some(Painted::Area),
            },
        }
    }

    /// An axial or radial shading as a gradient extended at both ends, with the clip to its own
    /// extent when the shading doesn't extend (see [`shade::gradient_extent`]); `None` for other
    /// types, or when it paints nothing.
    fn gradient(&mut self, sp: &ShadingPattern) -> Option<(Paint, Option<BezPath>)> {
        let ShadingType::RadialAxial { coords, domain, function, extend, axial } = sp.shading.shading_type.as_ref() else { return None };
        let cs = &sp.shading.color_space;
        let simulation = self.shared.simulation.clone();
        let space = cs.ink_space(simulation.as_ref());
        let (inks, channels) = self.space_inks(&space, cs)?;
        let stops = shade::sample_stops(space.len(), |s| {
            let t = domain[0] + s * (domain[1] - domain[0]);
            let components = shade::eval(function, &[t])?;
            let mut values = Vec::new();
            cs.ink_values(&components, simulation.as_ref(), &mut values);
            Some(values)
        });
        let kind = inks.stop_inks();
        let stops = stops.into_iter().map(|(offset, values)| (offset, shade::stop_color(&kind, &channels.apply_f32(&values)))).collect();
        let c = coords.map(f64::from);
        let geometry = if *axial {
            GradientGeometry::Axial { start: Point::new(c[0], c[1]), end: Point::new(c[2], c[3]), extend: [true, true], transform: sp.matrix }
        } else {
            GradientGeometry::Radial {
                start: Point::new(c[0], c[1]),
                start_radius: c[2],
                end: Point::new(c[3], c[4]),
                end_radius: c[5],
                extend: [true, true],
                transform: sp.matrix,
            }
        };
        let extent = shade::gradient_extent(&geometry, *extend, self.clip_bounds());
        self.shared.report.gradients += 1;
        Some((Paint::Gradient { stops, geometry }, extent))
    }

    fn fill(&mut self, path: BezPath, rule: FillRule, paint: &PdfPaint<'a>, stroke: bool) {
        match self.paint(paint, stroke) {
            Some(Painted::Flat(paint, look, overprint)) => {
                self.flush();
                self.push(Item::Fill(FillItem { path, rule, paint, overprint }), &look);
            }
            Some(Painted::Area) => self.area_fill(path, rule, paint, stroke),
            None => {}
        }
    }

    /// A shading or tiling pattern read over `path`: its items or an image, clipped to the
    /// path.
    fn area_fill(&mut self, path: BezPath, rule: FillRule, paint: &PdfPaint<'a>, stroke: bool) {
        let PdfPaint::Pattern(pattern) = paint else { return };
        self.flush();
        let clip = nonzero(&path, rule);
        let (op, big_op, _) = self.overprint;
        let flag = if stroke { big_op } else { op };
        match pattern.as_ref() {
            Pattern::Shading(sp) => self.shading_area(clip, sp, flag),
            Pattern::Tiling(t) => self.tiling_area(clip, t, flag, stroke),
        }
    }

    fn shading_area(&mut self, clip: BezPath, sp: &ShadingPattern, flag: bool) {
        let look = self.look(sp.opacity);
        let cs = &sp.shading.color_space;
        // Nonzero overprint mode is for colors set in DeviceCMYK, not shadings or images
        // (ISO 32000-2 §8.6.7).
        let overprint = Overprint { fill: flag, stroke: flag, nonzero: false };
        let simulation = self.shared.simulation.clone();
        let mut children = Vec::new();
        if let Some(bg) = &sp.shading.background {
            let mut values = Vec::new();
            cs.ink_values(bg, simulation.as_ref(), &mut values);
            let space = cs.ink_space(simulation.as_ref());
            if let Some((inks, channels)) = self.space_inks(&space, cs) {
                let kind = inks.stop_inks();
                let paint = Paint::Solid(shade::stop_color(&kind, &channels.apply_f32(&values)));
                children.push(Item::Fill(FillItem { path: clip.clone(), rule: FillRule::NonZero, paint, overprint }));
            }
        }
        let shaded = if matches!(sp.shading.shading_type.as_ref(), ShadingType::RadialAxial { .. }) {
            self.gradient(sp).map(|(paint, extent)| {
                let fill = Item::Fill(FillItem { path: clip.clone(), rule: FillRule::NonZero, paint, overprint });
                match extent {
                    Some(extent) => Item::Group(GroupItem { children: vec![fill], ..plain_group(Some(extent)) }),
                    None => fill,
                }
            })
        } else {
            self.shading_image(&clip, sp, overprint).map(Item::Image)
        };
        if let Some(item) = shaded {
            match &sp.shading.clip_path {
                Some(bbox) => children.push(Item::Group(GroupItem { children: vec![item], ..plain_group(Some(bbox.clone())) })),
                None => children.push(item),
            }
        }
        if !children.is_empty() {
            self.push(Item::Group(GroupItem { children, ..plain_group(Some(clip)) }), &look);
        }
    }

    /// A function-based or mesh shading sampled over the visible part of `clip`.
    fn shading_image(&mut self, clip: &BezPath, sp: &ShadingPattern, overprint: Overprint) -> Option<ImageItem> {
        let mut area = clip.bounding_box().intersect(self.clip_bounds());
        if let Some(bbox) = &sp.shading.clip_path {
            area = area.intersect(bbox.bounding_box());
        }
        if !(area.width() > 0.0 && area.height() > 0.0) {
            return None;
        }
        let scale = self.raster_scale(area);
        let simulation = self.shared.simulation.clone();
        // The background is its own fill, under the image.
        let raster = shade::rasterize_shading(sp, area, scale, false, simulation.as_ref())?;
        let cs = &sp.shading.color_space;
        let space = cs.ink_space(simulation.as_ref());
        let (inks, channels) = self.space_inks(&space, cs)?;
        let samples: Vec<u8> = raster.values.iter().map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8).collect();
        let samples = channels.apply(samples, space.len());
        let (w, h) = (raster.width as usize, raster.height as usize);
        let smask = (!raster.alpha.iter().all(|a| *a == 255)).then(|| raster.alpha.into());
        self.shared.report.rasterized_shadings += 1;
        Some(ImageItem {
            space: inks.image_space(),
            size: (raster.width, raster.height),
            samples: samples.into(),
            transform: shade::area_transform(area, scale, (w, h)),
            smask,
            overprint,
            jpeg: None,
        })
    }

    /// A tiling pattern over `clip`: its cells, each read like page content, or an image past
    /// [`MAX_TILING_CELLS`] or [`MAX_TILING_ITEMS`].
    fn tiling_area(&mut self, clip: BezPath, t: &TilingPattern<'a>, flag: bool, stroke: bool) {
        let area = clip.bounding_box().intersect(self.clip_bounds());
        if !(area.width() > 0.0 && area.height() > 0.0) {
            return;
        }
        let m = t.matrix;
        let inv = m.inverse();
        let (xs, ys) = (f64::from(t.x_step), f64::from(t.y_step));
        let b = t.bbox;
        let pa = inv.transform_rect_bbox(area);
        let (Some(is), Some(js)) = (cells(pa.x0, pa.x1, b.x0, b.x1, xs), cells(pa.y0, pa.y1, b.y0, b.y1, ys)) else { return };
        let count = (is.1 - is.0 + 1) as u128 * (js.1 - js.0 + 1) as u128;

        let mut reader = Reader::new(&mut *self.shared, false, Rect::new(-1e9, -1e9, 1e9, 1e9));
        reader.overprint = self.overprint;
        t.interpret(&mut reader, m, stroke);
        let cell = reader.finish();
        if cell.is_empty() {
            return;
        }
        let look = self.look(1.0);
        let offset = |i: i64, j: i64| m * Point::new(i as f64 * xs, j as f64 * ys) - m * Point::ORIGIN;
        let items = count.saturating_mul(item_count(&cell) as u128);
        let child = if count <= MAX_TILING_CELLS as u128 && items <= MAX_TILING_ITEMS as u128 {
            let mut children = Vec::with_capacity(count as usize * cell.len());
            for j in js.0..=js.1 {
                for i in is.0..=is.1 {
                    let mut copy = cell.clone();
                    shade::translate(&mut copy, offset(i, j));
                    children.extend(copy);
                }
            }
            self.shared.report.tiling_cells += count as usize;
            children
        } else {
            // Every pixel samples each cell over it: one, or more where cells are larger than
            // their step and overlap. Cells stacked deeper than MAX_TILING_CELLS paint nothing,
            // like fills past the range of `cells`.
            let per_pixel = ((b.width().abs() / xs.abs()).floor() + 1.0) * ((b.height().abs() / ys.abs()).floor() + 1.0);
            if !per_pixel.is_finite() || per_pixel > MAX_TILING_CELLS as f64 {
                self.shared.report.dropped_tilings += 1;
                return;
            }
            let inks = &self.shared.inks;
            let cell_box = m.transform_rect_bbox(b);
            let (scale, cell_scale) = tiling_scales(self.shared.scale, area, cell_box, per_pixel, inks.len());
            let (Some(mut drawn), Some(mut canvas)) = (Canvas::new(cell_box, cell_scale, inks), Canvas::new(area, scale, inks)) else {
                self.shared.report.dropped_tilings += 1;
                return;
            };
            drawn.draw(&cell);
            let (w, h) = canvas.size();
            for y in 0..h {
                for x in 0..w {
                    let p = canvas.center(x, y);
                    let q = inv * p;
                    let (Some(ci), Some(cj)) = (cells(q.x, q.x, b.x0, b.x1, xs), cells(q.y, q.y, b.y0, b.y1, ys)) else { continue };
                    for j in cj.0..=cj.1 {
                        for i in ci.0..=ci.1 {
                            if let Some((value, a)) = drawn.sample(p - offset(i, j))
                                && a > 0.0
                            {
                                let value = value.to_vec();
                                canvas.over(x, y, &value, a);
                            }
                        }
                    }
                }
            }
            let image = canvas.into_image(Overprint { fill: flag, stroke: flag, nonzero: false });
            self.shared.report.rasterized_tilings += 1;
            vec![Item::Image(image)]
        };
        self.push(Item::Group(GroupItem { children: child, ..plain_group(Some(clip)) }), &look);
    }

    fn raster_image(&mut self, image: &RasterImage<'_>, transform: Affine) {
        let mut decoded = None;
        image.with_inks(self.shared.simulation.as_ref(), |im| decoded = Some(im));
        let Some(im) = decoded else {
            self.shared.report.unreadable_images += 1;
            return;
        };
        let Some((inks, channels)) = self.space_inks(&im.space, &im.color_space) else { return };
        self.shared.report.images += 1;
        if matches!(im.space, InkSpace::Cmyk { converted: true }) {
            self.shared.report.converted_images += 1;
        }
        let (w, h) = (im.width as usize, im.height as usize);
        let samples = channels.apply(im.samples, im.space.len());
        let smask = im.alpha.map(|a| {
            if (a.width as usize, a.height as usize) == (w, h) {
                a.data
            } else {
                self.shared.report.resampled_masks += 1;
                shade::resample(&a.data, (a.width as usize, a.height as usize), (w, h))
            }
        });
        let smask = smask.filter(|a| !a.iter().all(|v| *v == 255)).map(Arc::from);
        let (sx, sy) = im.scale_factors;
        let transform = transform * Affine::scale_non_uniform(w as f64 * f64::from(sx), h as f64 * f64::from(sy));
        let (op, _, _) = self.overprint;
        let overprint = Overprint { fill: op, stroke: op, nonzero: false };
        self.flush();
        let device = matches!(im.space, InkSpace::Gray | InkSpace::Cmyk { converted: false });
        let jpeg = if device && smask.is_none() { dct_passthrough(image.stream(), im.width, im.height) } else { None };
        self.add(Item::Image(ImageItem {
            space: inks.image_space(),
            size: (im.width, im.height),
            samples: samples.into(),
            transform,
            smask,
            overprint,
            jpeg,
        }));
    }

    /// A stencil mask: a fill traced from the mask, or past [`MAX_STENCIL_RECTS`] an image of
    /// the paint with the mask as its alpha.
    fn stencil(&mut self, image: &StencilImage<'a, '_>, transform: Affine) {
        let mut read = None;
        image.with_stencil(|luma, paint| read = Some((luma, paint.clone())), None);
        let Some((luma, paint)) = read else {
            self.shared.report.unreadable_images += 1;
            return;
        };
        self.shared.report.stencils += 1;
        let (w, h) = (luma.width as usize, luma.height as usize);
        let (sx, sy) = luma.scale_factors;
        let to_page = transform * Affine::scale_non_uniform(f64::from(sx), f64::from(sy));
        if let Some(rects) = shade::trace_stencil(&luma.data, w, h, MAX_STENCIL_RECTS) {
            let mut path = BezPath::new();
            for r in rects {
                path.extend(shade::rect_path(r, to_page).iter());
            }
            if !path.elements().is_empty() {
                self.fill(path, FillRule::NonZero, &paint, false);
            }
            return;
        }
        let unit = to_page * Affine::scale_non_uniform(w as f64, h as f64);
        let size = (luma.width, luma.height);
        match self.paint(&paint, false) {
            None => {}
            Some(Painted::Flat(Paint::Solid(color), look, overprint)) => {
                let (space, value): (ImageSpace, Vec<u8>) = match &color {
                    InkColor::Tints(tints) => (ImageSpace::Inks(tints.iter().map(|t| t.0).collect()), tints.iter().map(|t| to_byte(t.1)).collect()),
                    InkColor::All(t) => {
                        (ImageSpace::Inks(self.shared.inks.iter().map(|(id, _)| id).collect()), vec![to_byte(*t); self.shared.inks.len()])
                    }
                    InkColor::None => return,
                };
                let samples: Vec<u8> = value.iter().copied().cycle().take(value.len() * w * h).collect();
                self.flush();
                self.push(
                    Item::Image(ImageItem {
                        space,
                        size,
                        samples: samples.into(),
                        transform: unit,
                        smask: Some(luma.data.into()),
                        overprint,
                        jpeg: None,
                    }),
                    &look,
                );
            }
            Some(_) => {
                // A pattern paint: the paint over the image's square, masked by the stencil.
                let shape = ImageItem {
                    space: ImageSpace::Gray,
                    size,
                    samples: vec![0; w * h].into(),
                    transform: unit,
                    smask: Some(luma.data.into()),
                    overprint: Overprint::default(),
                    jpeg: None,
                };
                let group = GroupItem { isolated: true, children: vec![Item::Image(shape)], ..plain_group(None) };
                let mask = Mask { kind: MaskKind::Alpha, group, backdrop: None, transfer: None };
                self.flush();
                self.frames.push(Frame { kind: FrameKind::Group, group: GroupItem { mask: Some(Box::new(mask)), ..plain_group(None) } });
                self.fill(shade::rect_path(Rect::new(0.0, 0.0, 1.0, 1.0), unit), FillRule::NonZero, &paint, false);
                self.close(FrameKind::Group);
            }
        }
    }

    fn outline_glyph(&mut self, glyph: &OutlineGlyph, matrix: Affine, pdf_paint: &PdfPaint<'a>) {
        let (paint, look, overprint) = match self.paint(pdf_paint, false) {
            Some(Painted::Flat(paint, look, overprint)) => (paint, look, overprint),
            Some(Painted::Area) => return self.area_fill(matrix * glyph.outline(), FillRule::NonZero, pdf_paint, false),
            None => return,
        };
        let key = self.shared.fonts.note(glyph);
        let unicode = glyph.as_unicode().map(unicode);
        self.add_glyph(Some(key), glyph.glyph_id().to_u32(), matrix, matrix * glyph.outline(), unicode, paint, look, overprint);
    }

    #[allow(clippy::too_many_arguments)]
    fn add_glyph(
        &mut self,
        font: Option<u128>,
        id: u32,
        matrix: Affine,
        outline: BezPath,
        unicode: Option<String>,
        paint: Paint,
        look: Look<'a>,
        overprint: Overprint,
    ) {
        let linear = matrix * Affine::scale(1000.0);
        let continues = self.run.as_ref().is_some_and(|run| {
            run.font == font && same_linear(run.linear, linear) && run.item.paint == paint && run.item.overprint == overprint && run.look.same(&look)
        });
        if !continues {
            self.flush();
            let size = linear.determinant().abs().sqrt();
            if !size.is_finite() || size <= f64::EPSILON {
                return;
            }
            let [a, b, c, d, ..] = linear.as_coeffs();
            let origin = matrix.translation();
            let transform = Affine::new([a / size, b / size, c / size, d / size, origin.x, origin.y]);
            // A placeholder font carrying the key; `set_fonts` replaces it.
            let placeholder = font.map(|key| Arc::new(TextFont { data: Arc::new(Vec::<u8>::new()), index: 0, key }));
            let item = TextItem {
                font: placeholder,
                size: size as f32,
                transform,
                glyphs: Vec::new(),
                text: Some(String::new()),
                outlines: BezPath::new(),
                paint,
                overprint,
            };
            self.run = Some(Run { font, linear, look, item });
        }
        let Some(run) = self.run.as_mut() else { return };
        let item = &mut run.item;
        let text = item.text.get_or_insert_with(String::new);
        let start = text.len();
        text.push_str(unicode.as_deref().unwrap_or(""));
        let origin = item.transform.inverse() * matrix.translation().to_point();
        item.glyphs.push(PlacedGlyph { id, origin, text: start..text.len() });
        item.outlines.extend(outline.iter());
    }

    fn type3_glyph(&mut self, glyph: &craft_pdf::interpret::font::Type3Glyph<'a>, transform: Affine, glyph_transform: Affine, paint: &PdfPaint<'a>) {
        let (run_paint, look, overprint) = match self.paint(paint, false) {
            Some(Painted::Flat(paint, look, overprint)) => (Some(paint), look, overprint),
            Some(Painted::Area) => (None, self.look(1.0), Overprint::default()),
            None => return,
        };
        let mut reader = Reader::new(&mut *self.shared, true, self.bounds);
        reader.overprint = self.overprint;
        glyph.interpret(&mut reader, transform, glyph_transform, paint);
        let items = reader.finish();
        let unicode = glyph.as_unicode().map(unicode);
        // A glyph that only fills in the text's paint (`d1`, or `d0` painting that color) is a
        // shape: it joins the run.
        let shape = items.iter().all(|item| matches!(item, Item::Fill(f) if Some(&f.paint) == run_paint.as_ref()));
        if let (true, Some(run_paint)) = (shape, run_paint) {
            let mut outline = BezPath::new();
            for item in &items {
                if let Item::Fill(f) = item {
                    outline.extend(nonzero(&f.path, f.rule).iter());
                }
            }
            self.add_glyph(None, 0, transform * glyph_transform, outline, unicode, run_paint, look, overprint);
        } else if !items.is_empty() {
            self.flush();
            let group = GroupItem { actual_text: unicode, children: items, ..plain_group(None) };
            self.push(Item::Group(group), &look);
        }
    }
}

impl<'a> Device<'a> for Reader<'_, 'a> {
    fn set_soft_mask(&mut self, mask: Option<SoftMask<'a>>) {
        self.soft_mask = mask;
    }

    fn set_blend_mode(&mut self, blend_mode: PdfBlend) {
        self.blend = blend(blend_mode);
    }

    fn set_overprint(&mut self, fill: bool, stroke: bool, mode: u8) {
        self.overprint = (fill, stroke, mode);
    }

    fn draw_path(&mut self, path: &BezPath, transform: Affine, paint: &PdfPaint<'a>, draw_mode: &PathDrawMode) {
        match draw_mode {
            PathDrawMode::Fill(rule) => self.fill(transform * path.clone(), fill_rule(*rule), paint, false),
            PathDrawMode::Stroke(props) => self.fill(outline_stroke(path, transform, props), FillRule::NonZero, paint, true),
        }
    }

    fn push_clip_path(&mut self, clip_path: &ClipPath) {
        self.flush();
        let clip = nonzero(&clip_path.path, fill_rule(clip_path.fill));
        self.frames.push(Frame { kind: FrameKind::Clip, group: plain_group(Some(clip)) });
    }

    fn push_transparency_group(&mut self, opacity: f32, mask: Option<SoftMask<'a>>, blend_mode: PdfBlend) {
        self.flush();
        let mask = if self.glyph { None } else { mask.and_then(|m| self.mask(&m)) };
        let (opacity, blend_mode) = if self.glyph { (1.0, BlendMode::Normal) } else { (opacity, blend(blend_mode)) };
        let group = GroupItem { opacity, blend: blend_mode, mask: mask.map(Box::new), ..plain_group(None) };
        self.frames.push(Frame { kind: FrameKind::Group, group });
    }

    fn set_knockout(&mut self) {
        if let Some(frame) = self.frames.last_mut().filter(|f| f.kind == FrameKind::Group) {
            frame.group.knockout = true;
        }
    }

    fn set_isolated(&mut self) {
        if let Some(frame) = self.frames.last_mut().filter(|f| f.kind == FrameKind::Group) {
            frame.group.isolated = true;
        }
    }

    fn draw_glyph(&mut self, glyph: &Glyph<'a>, transform: Affine, glyph_transform: Affine, paint: &PdfPaint<'a>, draw_mode: &GlyphDrawMode) {
        match (glyph, draw_mode) {
            (_, GlyphDrawMode::Invisible) => self.shared.report.invisible_glyphs += 1,
            // Clip-only text: the interpreter installs its clip, and the glyph paints nothing.
            (_, GlyphDrawMode::Clip) => {}
            (Glyph::Outline(o), GlyphDrawMode::Fill) => self.outline_glyph(o, transform * glyph_transform, paint),
            (Glyph::Outline(o), GlyphDrawMode::Stroke(props)) => {
                self.fill(outline_stroke(&(glyph_transform * o.outline()), transform, props), FillRule::NonZero, paint, true);
            }
            (Glyph::Type3(t), _) => self.type3_glyph(t, transform, glyph_transform, paint),
        }
    }

    fn draw_image(&mut self, image: Image<'a, '_>, transform: Affine) {
        match image {
            Image::Raster(r) => self.raster_image(&r, transform),
            Image::Stencil(s) => self.stencil(&s, transform),
        }
    }

    fn pop_clip_path(&mut self) {
        self.close(FrameKind::Clip);
    }

    fn pop_transparency_group(&mut self) {
        self.close(FrameKind::Group);
    }
}

/// The inks of samples or stops.
enum Inks {
    /// A gray level.
    Gray,
    /// The four process inks.
    Cmyk(Vec<InkId>),
    List(Vec<InkId>),
}

impl Inks {
    fn stop_inks(self) -> StopInks {
        match self {
            Self::Gray => StopInks::Gray,
            Self::Cmyk(ids) | Self::List(ids) => StopInks::Inks(ids),
        }
    }

    fn image_space(self) -> ImageSpace {
        match self {
            Self::Gray => ImageSpace::Gray,
            Self::Cmyk(_) => ImageSpace::Cmyk,
            Self::List(ids) => ImageSpace::Inks(ids),
        }
    }
}

/// Which of a space's values become samples.
enum Channels {
    All,
    /// Values of inks the set has room for.
    Keep(Vec<bool>),
    /// One value (the `All` colorant) for each of this many inks.
    Spread(usize),
}

impl Channels {
    fn apply(&self, samples: Vec<u8>, width: usize) -> Vec<u8> {
        match self {
            Self::All => samples,
            Self::Keep(keep) => samples.chunks_exact(width.max(1)).flat_map(|s| s.iter().zip(keep).filter(|(_, k)| **k).map(|(v, _)| *v)).collect(),
            Self::Spread(n) => samples.iter().flat_map(|v| std::iter::repeat_n(*v, *n)).collect(),
        }
    }

    fn apply_f32(&self, values: &[f32]) -> Vec<f32> {
        match self {
            Self::All => values.to_vec(),
            Self::Spread(n) => values.iter().flat_map(|v| std::iter::repeat_n(*v, *n)).collect(),
            Self::Keep(keep) => values.iter().zip(keep).filter(|(_, k)| **k).map(|(v, _)| *v).collect(),
        }
    }
}

/// The JPEG data of a `width` × `height` image that krilla's JPEG passthrough writes with the
/// same meaning: DCTDecode alone without parameters, 8 bits, DeviceGray with the default
/// `/Decode`, or DeviceCMYK with the inverted `/Decode` of Adobe CMYK JPEGs, which krilla
/// always writes for CMYK.
fn dct_passthrough(stream: &Stream<'_>, width: u32, height: u32) -> Option<Arc<[u8]>> {
    let dict = stream.dict();
    let only_dct = stream.filters().as_slice() == [Filter::DctDecode] && !dict.contains_key(DECODE_PARMS);
    let size = dict.get::<u32>(WIDTH) == Some(width) && dict.get::<u32>(HEIGHT) == Some(height);
    if !(only_dct && size && dict.get::<u8>(BITS_PER_COMPONENT) == Some(8)) {
        return None;
    }
    let decode: Option<Vec<f32>> = dict.get::<Array<'_>>(DECODE).map(|a| a.iter::<f32>().collect());
    let ok = match dict.get::<Name<'_>>(COLORSPACE)?.as_ref() {
        b"DeviceGray" => decode.is_none_or(|d| d == [0.0, 1.0]),
        b"DeviceCMYK" => decode.is_some_and(|d| d == [1.0, 0.0].repeat(4)),
        _ => false,
    };
    ok.then(|| Arc::from(stream.raw_data().as_ref()))
}

/// Pixels per point of the image a tiling pattern over `area` is read as, from `scale`, and of
/// the drawing of its cell over `cell_box`, for `per_pixel` cells over each pixel and `inks`
/// values per pixel.
fn tiling_scales(scale: f64, area: Rect, cell_box: Rect, per_pixel: f64, inks: usize) -> (f64, f64) {
    let ink_share = (CANVAS_INKS / inks.max(1) as f64).min(1.0);
    let scale = limit_scale(scale, area, MAX_RASTER_PIXELS.min(MAX_TILING_SAMPLES / per_pixel) * ink_share);
    (scale, limit_scale(scale * CELL_OVERSAMPLE, cell_box, MAX_CELL_PIXELS * ink_share))
}

/// `scale` (pixels per point), lowered so that `area` holds at most `pixels` pixels.
fn limit_scale(scale: f64, area: Rect, pixels: f64) -> f64 {
    let have = area.area() * scale * scale;
    if have > pixels { scale * (pixels / have).sqrt() } else { scale }
}

/// `items` and every item inside their groups and masks.
fn item_count(items: &[Item]) -> usize {
    items.iter().fold(0, |sum, item| {
        let inner = match item {
            Item::Group(g) => item_count(&g.children).saturating_add(g.mask.as_ref().map_or(0, |m| item_count(&m.group.children))),
            Item::Fill(_) | Item::Text(_) | Item::Image(_) => 0,
        };
        sum.saturating_add(1).saturating_add(inner)
    })
}

/// The cell indices `k` whose span `[c0, c1] + k * step` meets `[lo, hi]`.
fn cells(lo: f64, hi: f64, c0: f64, c1: f64, step: f64) -> Option<(i64, i64)> {
    let (a, z) = ((lo - c1) / step, (hi - c0) / step);
    let (a, z) = if step > 0.0 { (a, z) } else { (z, a) };
    let (a, z) = (a.ceil(), z.floor());
    (a.is_finite() && z.is_finite() && a <= z && z - a < 1e9).then_some((a as i64, z as i64))
}

fn to_byte(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// Fills each text item's font from the prepared fonts, replacing the key placeholders.
fn set_fonts(items: &mut [Item], fonts: &HashMap<u128, Option<Arc<TextFont>>>) {
    for item in items {
        match item {
            Item::Text(t) => {
                t.font = t.font.as_ref().and_then(|f| fonts.get(&f.key).cloned().flatten());
            }
            Item::Group(g) => set_group_fonts(g, fonts),
            Item::Fill(_) | Item::Image(_) => {}
        }
    }
}

fn set_group_fonts(group: &mut GroupItem, fonts: &HashMap<u128, Option<Arc<TextFont>>>) {
    if let Some(mask) = group.mask.as_mut() {
        set_group_fonts(&mut mask.group, fonts);
    }
    set_fonts(&mut group.children, fonts);
}

fn unicode(s: BfString) -> String {
    match s {
        BfString::Char(c) => c.to_string(),
        BfString::String(s) => s,
    }
}

fn same_linear(a: Affine, b: Affine) -> bool {
    let (a, b) = (a.as_coeffs(), b.as_coeffs());
    let scale = a[..4].iter().fold(0.0_f64, |m, v| m.max(v.abs()));
    a[..4].iter().zip(&b[..4]).all(|(x, y)| (x - y).abs() <= scale * 1e-6)
}

fn fill_rule(rule: PdfFillRule) -> FillRule {
    match rule {
        PdfFillRule::NonZero => FillRule::NonZero,
        PdfFillRule::EvenOdd => FillRule::EvenOdd,
    }
}

fn blend(mode: PdfBlend) -> BlendMode {
    match mode {
        PdfBlend::Normal => BlendMode::Normal,
        PdfBlend::Multiply => BlendMode::Multiply,
        PdfBlend::Screen => BlendMode::Screen,
        PdfBlend::Overlay => BlendMode::Overlay,
        PdfBlend::Darken => BlendMode::Darken,
        PdfBlend::Lighten => BlendMode::Lighten,
        PdfBlend::ColorDodge => BlendMode::ColorDodge,
        PdfBlend::ColorBurn => BlendMode::ColorBurn,
        PdfBlend::HardLight => BlendMode::HardLight,
        PdfBlend::SoftLight => BlendMode::SoftLight,
        PdfBlend::Difference => BlendMode::Difference,
        PdfBlend::Exclusion => BlendMode::Exclusion,
        PdfBlend::Hue => BlendMode::Hue,
        PdfBlend::Saturation => BlendMode::Saturation,
        PdfBlend::Color => BlendMode::Color,
        PdfBlend::Luminosity => BlendMode::Luminosity,
    }
}

/// `path` (user space) stroked with `props` and taken to page space by `transform`. The stroke
/// is outlined in user space, where its width is given, so a skewed CTM skews it too.
fn outline_stroke(path: &BezPath, transform: Affine, props: &StrokeProps) -> BezPath {
    let scale = transform.determinant().abs().sqrt();
    let scale = if scale.is_finite() && scale > f64::EPSILON { scale } else { 1.0 };
    let width = match f64::from(props.line_width) {
        w if w > 0.0 && w.is_finite() => w,
        _ => HAIRLINE / scale,
    };
    let mut style = Stroke::new(width).with_caps(props.line_cap).with_join(props.line_join).with_miter_limit(f64::from(props.miter_limit));
    if let Some(dashes) = dash_pattern(path, &props.dash_array) {
        style = style.with_dashes(f64::from(props.dash_offset), dashes);
    }
    let outline = kurbo::stroke(path.iter(), &style, &StrokeOpts::default(), STROKE_TOLERANCE / scale);
    transform * outline
}

/// The dash pattern to outline `path` with; `None` (solid) when the pattern is invalid or could
/// cut the path into more than [`MAX_DASHES_PER_STROKE`] pieces. The same estimate as hayro's.
fn dash_pattern(path: &BezPath, dashes: &[f32]) -> Option<Vec<f64>> {
    if dashes.iter().any(|d| !d.is_finite() || *d < 0.0) {
        return None;
    }
    let sum: f64 = dashes.iter().map(|d| f64::from(*d)).sum();
    // kurbo repeats an odd-length pattern once, so a period covers it twice.
    let (period, per_period) = if dashes.len() % 2 == 1 { (2.0 * sum, dashes.len().saturating_mul(2)) } else { (sum, dashes.len()) };
    if !(period.is_finite() && period > 0.0) {
        return None;
    }
    // A curve is no longer than its control polygon. kurbo restarts the pattern at every
    // subpath and close, so each restart can begin a whole period however short its segment.
    let (mut length, mut restarts) = (0.0, 0usize);
    let (mut start, mut last) = (Point::ORIGIN, Point::ORIGIN);
    for el in path.elements() {
        match *el {
            PathEl::MoveTo(p) => {
                (start, last) = (p, p);
                restarts = restarts.saturating_add(1);
            }
            PathEl::LineTo(p) => {
                length += last.distance(p);
                last = p;
            }
            PathEl::QuadTo(c, p) => {
                length += last.distance(c) + c.distance(p);
                last = p;
            }
            PathEl::CurveTo(c1, c2, p) => {
                length += last.distance(c1) + c1.distance(c2) + c2.distance(p);
                last = p;
            }
            PathEl::ClosePath => {
                length += last.distance(start);
                last = start;
                restarts = restarts.saturating_add(1);
            }
        }
    }
    let pieces = (length / period + restarts as f64) * per_period as f64;
    (pieces.is_finite() && pieces <= MAX_DASHES_PER_STROKE).then(|| dashes.iter().map(|d| f64::from(*d)).collect())
}

/// `path` filled by `rule`, as a path filled nonzero. An even-odd path is rebuilt from its
/// boundary; if that fails it is kept as it is.
fn nonzero(path: &BezPath, rule: FillRule) -> BezPath {
    if rule == FillRule::NonZero {
        return path.clone();
    }
    let closed = closed(path);
    match linesweeper::binary_op(&closed, &BezPath::new(), linesweeper::FillRule::EvenOdd, linesweeper::BinaryOp::Union) {
        Ok(contours) => {
            let mut out = BezPath::new();
            for contour in contours.contours() {
                out.extend(contour.path.iter());
            }
            out
        }
        Err(_) => closed,
    }
}

/// `path` with every subpath closed, as a fill closes them.
fn closed(path: &BezPath) -> BezPath {
    let mut out = BezPath::new();
    let mut open = false;
    for el in path.elements() {
        match el {
            PathEl::MoveTo(_) => {
                if open {
                    out.close_path();
                }
                open = true;
            }
            PathEl::ClosePath => open = false,
            _ => {}
        }
        out.push(*el);
    }
    if open {
        out.close_path();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiling_canvases_hold_cmyk_memory_with_many_inks() {
        let (page, cell) = (Rect::new(0.0, 0.0, 600.0, 800.0), Rect::new(0.0, 0.0, 300.0, 300.0));
        let dpi = 300.0 / 72.0;
        let values = |area: Rect, scale: f64, inks: usize| {
            let (w, h) = shade::pixel_size(area, scale).unwrap();
            (w * h * inks) as f64
        };
        // With 1,000 inks the page canvas would hold 8 billion values.
        let (scale, cell_scale) = tiling_scales(dpi, page, cell, 1.0, 1000);
        assert!(values(page, scale, 1000) <= MAX_RASTER_PIXELS * CANVAS_INKS * 1.01);
        assert!(values(cell, cell_scale, 1000) <= MAX_CELL_PIXELS * CANVAS_INKS * 1.01);
        // A CMYK page keeps its resolution.
        assert_eq!(tiling_scales(dpi, page, cell, 1.0, 4).0, dpi);
    }
}
