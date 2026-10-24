//! Complex regions rasterized as ink images (plan tasks C3 and C4), and [`flatten_page`], which
//! flattens one page of a PDF.
//!
//! A region's image is its items written alone as a small PDF ([`page_pdf`]) and separated with
//! [`plates`], overprint on: one plane per ink, so spot inks stay spot inks and overprint applies
//! per plate as on press. The planes map to the list's inks by name and come back as an
//! [`ImageSpace::Inks`] image, which the writer clips to the region and writes as DeviceN.
//!
//! Backdrop: the sub-list is [`ComplexRegion::items`], in paint order, and nothing else. Those
//! are the owners of every shape covering the region, and a flattened group holds every item
//! whose box touches one of its members ([`geom::overlap_groups`](super::geom::overlap_groups)),
//! up to its last transparent item. Kept items are either in other groups, so their boxes are
//! clear of the region, or painted after it. So no kept item is under a complex region, and the
//! items listed are the region's whole backdrop.
//!
//! Seams:
//! - The image is snapped outward to the device pixel grid at the raster resolution (counted
//!   from the page box's corner) and reaches one more device pixel on every side. The clip stays
//!   the exact region path, so the image's edge pixels are never seen: no paper shows where the
//!   clip edge falls between pixel centers, and anti-aliased edges of the image don't exist.
//! - Items render unclipped in the sub-list, so pixels along the region edge hold the full color
//!   of the art on both sides of the edge, not a mix with paper.
//! - Neighboring complex regions are merged before rasterizing (`compose`), so their shared
//!   edges fall inside one image.
//! - Regions larger than a plate render in tiles that abut on the device grid and are joined
//!   into one image before it is written.
//!
//! Whole groups the region sweep failed on
//! ([`ComplexReason::Degenerate`](super::ComplexReason::Degenerate)) are clipped to their
//! members' boxes, which hold pixels nothing paints. Their image is opaque like any other: the
//! boxes are clear of every other group's items, so the paper is the only backdrop there, and
//! the plates of the items over paper print the same as the items would. A soft mask would leave
//! transparency on the page, which PDF/X-1a forbids.
//!
//! Ported from VectorCraft (MIT OR Apache-2.0) at commit c2b15982: `render_raster` in
//! `crates/engine/src/cmd/flatten.rs` (the image reaching a pixel past its area). VectorCraft
//! renders RGB over white; here the plates give each ink's tint directly.

use craft_pdf::OpenError;
use craft_pdf::interpret::color::CmykConversion;
use craft_pdf::write::backend::SerializeSettings;
use kurbo::{Affine, Rect, Shape as _};

use super::document::{FlattenDocumentError, FlattenDocumentOptions, flatten_document};
use super::pdf::{ReadError, ReadOptions, ReadReport};
use super::write::{WriteError, page_pdf};
use super::{
    BlendMode, ComplexRegion, DisplayList, FillItem, FillRule, FlattenError, FlattenOptions, FlattenReport, GroupItem, ImageItem, ImageSpace, Item,
    Mask, MaskKind, Paint,
};
use crate::ink::{Ink, InkColor, InkId, Overprint, Process};
use crate::plates::{Fallback, MAX_SPOT_INKS, Options, plates};

/// Pixels per complex region's image. A larger region rasterizes at a lower resolution;
/// [`RasterReport::lowest_dpi`] says how low.
pub const MAX_RASTER_PIXELS: u64 = 64 << 20;

/// Tile side in device pixels: tiles stay below [`crate::MAX_PLATE_PIXELS`], so `plates` renders
/// them at the resolution asked for.
const TILE: i64 = 1536;

/// What [`Rasterizer`] made.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct RasterReport {
    /// Images returned.
    pub images: usize,
    /// Device pixels of all images.
    pub pixels: u64,
    /// The lowest resolution used, when a region past [`MAX_RASTER_PIXELS`] needed less than
    /// the one asked for.
    pub lowest_dpi: Option<f32>,
}

/// Rasterizes the complex regions of a flattened [`DisplayList`]; [`Self::region`] is the
/// callback [`write_flattened`](super::write::write_flattened) takes.
pub struct Rasterizer<'a> {
    list: &'a DisplayList,
    settings: SerializeSettings,
    printing: Fallback,
    cms: &'a dyn CmykConversion,
    dpi: f32,
    report: RasterReport,
}

impl<'a> Rasterizer<'a> {
    /// `settings` and `cms` are the output's, and `printing` is its printing condition: the
    /// sub-lists are written without the output intent, which `plates` would decode and parse
    /// for every region, and separated with `printing` as the fallback.
    pub fn new(list: &'a DisplayList, settings: SerializeSettings, printing: Fallback, cms: &'a dyn CmykConversion, dpi: f32) -> Self {
        let settings = SerializeSettings { output_intent: None, ..settings };
        Self { list, settings, printing, cms, dpi, report: RasterReport::default() }
    }

    pub fn report(&self) -> &RasterReport {
        &self.report
    }

    /// The image of `region`, or `None` when it prints no ink (nothing is under a complex
    /// region to knock out).
    pub fn region(&mut self, region: &ComplexRegion) -> Result<Option<ImageItem>, WriteError> {
        let page = self.list.page;
        let bounds = region.path.bounding_box();
        if !(bounds.is_finite() && page.is_finite() && self.dpi.is_finite() && self.dpi > 0.0) {
            return Ok(None);
        }
        let mut scale = f64::from(self.dpi) / 72.0;
        let mut grid = Grid::new(bounds, page, scale);
        let pixels = grid.pixels();
        if pixels > MAX_RASTER_PIXELS {
            scale *= (MAX_RASTER_PIXELS as f64 / pixels as f64).sqrt() * 0.999;
            grid = Grid::new(bounds, page, scale);
            let dpi = (scale * 72.0) as f32;
            self.report.lowest_dpi = Some(self.report.lowest_dpi.map_or(dpi, |d| d.min(dpi)));
        }
        if grid.width() <= 0 || grid.height() <= 0 {
            return Ok(None);
        }

        let items: Vec<Item> =
            region.items.iter().filter_map(|&i| self.list.items.get(i)).map(|i| self.alpha_masks(i, &grid)).collect::<Result<_, _>>()?;
        let planes = self.separate(&items, &grid, true)?;

        let mut ids = Vec::new();
        let mut used = Vec::new();
        for ((id, _), plane) in self.list.inks.iter().zip(planes) {
            if let Some(plane) = plane.filter(|p| p.iter().any(|v| *v > 0)) {
                ids.push(id);
                used.push(plane);
            }
        }
        if ids.is_empty() {
            return Ok(None);
        }
        let n = grid.len();
        let mut samples = Vec::with_capacity(n * used.len());
        for px in 0..n {
            samples.extend(used.iter().map(|p| p[px]));
        }
        self.report.images += 1;
        self.report.pixels += n as u64;
        let r = grid.rect();
        Ok(Some(ImageItem {
            space: ImageSpace::Inks(ids),
            size: (grid.width() as u32, grid.height() as u32),
            samples: samples.into(),
            transform: Affine::new([r.width(), 0.0, 0.0, r.height(), r.x0, r.y0]),
            smask: None,
            overprint: Overprint::default(),
            jpeg: None,
        }))
    }

    /// `item` with its luminosity masks, and those of the groups in it, made alpha masks over
    /// `grid` (see [`Self::luminosity`]).
    fn alpha_masks(&self, item: &Item, grid: &Grid) -> Result<Item, WriteError> {
        let Item::Group(g) = item else { return Ok(item.clone()) };
        let mask = match g.mask.as_deref() {
            Some(m) if m.kind == MaskKind::Luminosity => Some(Box::new(self.luminosity(m, grid)?)),
            other => other.cloned().map(Box::new),
        };
        let children = g.children.iter().map(|c| self.alpha_masks(c, grid)).collect::<Result<_, _>>()?;
        Ok(Item::Group(GroupItem {
            opacity: g.opacity,
            blend: g.blend,
            isolated: g.isolated,
            knockout: g.knockout,
            clip: g.clip.clone(),
            mask,
            actual_text: g.actual_text.clone(),
            children,
        }))
    }

    /// A luminosity mask as an alpha mask: an image whose alpha is the mask's luminosity over
    /// `grid`, its transfer function applied.
    ///
    /// The mask's content arrives in inks, and a CMYK blending space has no exact black for the
    /// renderer: K 100% (DeviceGray black, read as ink) has a luminosity of about 12% through
    /// the printing condition. So the content is separated and its luminosity computed from the
    /// inks with the ISO 32000-2 §10.4.2.2 formula, `1 - min(1, 0.3c + 0.59m + 0.11y + k)`,
    /// spot inks through their CMYK equivalents.
    fn luminosity(&self, m: &Mask, grid: &Grid) -> Result<Mask, WriteError> {
        let r = grid.rect();
        // Where nothing paints the mask is the backdrop's luminosity, black by default.
        let backdrop = m.backdrop.clone().unwrap_or_else(|| InkColor::Tints(vec![(InkId::process(Process::Black), 1.0)]));
        let content = [
            Item::Fill(FillItem { path: r.to_path(0.1), rule: FillRule::NonZero, paint: Paint::Solid(backdrop), overprint: Overprint::default() }),
            self.alpha_masks(&Item::Group(m.group.clone()), grid)?,
        ];
        let planes = self.separate(&content, grid, false)?;
        let weights: Vec<[f32; 4]> = self
            .list
            .inks
            .iter()
            .map(|(id, _)| match Process::ALL.get(id.index()) {
                Some(p) => {
                    let mut w = [0.0; 4];
                    w[p.index()] = 1.0;
                    w
                }
                None => self.list.inks.spot_cmyk(id, self.cms).unwrap_or([0.0, 0.0, 0.0, 1.0]),
            })
            .collect();
        let used: Vec<(&[u8], [f32; 4])> = planes.iter().zip(&weights).filter_map(|(p, w)| Some((p.as_deref()?, *w))).collect();
        let alpha: Vec<u8> = (0..grid.len())
            .map(|px| {
                let mut cmyk = [0.0_f32; 4];
                for (plane, w) in &used {
                    let t = f32::from(plane[px]) / 255.0;
                    for (v, w) in cmyk.iter_mut().zip(w) {
                        *v = 1.0 - (1.0 - *v) * (1.0 - t * w);
                    }
                }
                let [c, m_, y, k] = cmyk;
                let gray = 1.0 - (0.3 * c + 0.59 * m_ + 0.11 * y + k).min(1.0);
                let v = (gray * 255.0).round().clamp(0.0, 255.0) as u8;
                m.transfer.as_ref().and_then(|t| t.get(usize::from(v)).copied()).unwrap_or(v)
            })
            .collect();
        let n = alpha.len();
        let image = ImageItem {
            space: ImageSpace::Gray,
            size: (grid.width() as u32, grid.height() as u32),
            samples: vec![0; n].into(),
            transform: Affine::new([r.width(), 0.0, 0.0, r.height(), r.x0, r.y0]),
            smask: Some(alpha.into()),
            overprint: Overprint::default(),
            jpeg: None,
        };
        let group = GroupItem {
            opacity: 1.0,
            blend: BlendMode::Normal,
            isolated: false,
            knockout: false,
            clip: None,
            mask: None,
            actual_text: None,
            children: vec![Item::Image(image)],
        };
        Ok(Mask { kind: MaskKind::Alpha, group, backdrop: None, transfer: None })
    }

    /// `items` separated over `grid`, tile by tile: per ink of the list, its plane, `None` for
    /// inks the items don't print.
    fn separate(&self, items: &[Item], grid: &Grid, overprint: bool) -> Result<Vec<Option<Vec<u8>>>, WriteError> {
        let (w, h) = (grid.width() as usize, grid.height() as usize);
        let mut planes: Vec<Option<Vec<u8>>> = vec![None; self.list.inks.len()];
        let dpi = (grid.scale * 72.0) as f32;
        // The renderer makes a page at least 1 pt on a side: thinner tiles are rendered at that
        // size and cropped.
        let min = grid.scale.ceil() as i64;
        let mut ty = 0;
        while ty < grid.height() {
            let th = TILE.min(grid.height() - ty);
            let mut tx = 0;
            while tx < grid.width() {
                let tw = TILE.min(grid.width() - tx);
                let (rw, rh) = (tw.max(min), th.max(min));
                // A quarter pixel more than the tile: the renderer floors the page size.
                let (x0, y0) = grid.point(grid.x0 + tx, grid.y0 + ty);
                let extra = 0.25 / grid.scale;
                let tile = Rect::new(x0, y0, x0 + rw as f64 / grid.scale + extra, y0 + rh as f64 / grid.scale + extra);
                let sub = DisplayList { page: tile, inks: self.list.inks.clone(), items: items.to_vec() };
                let pdf = page_pdf(&sub, self.settings.clone(), self.cms)?;
                let p = plates(&pdf, 0, dpi, &Options { overprint, fallback: self.printing.clone() })
                    .map_err(|e| WriteError::Region(format!("separating a complex region: {e}")))?;
                if !p.unseparated.is_empty() {
                    let spots = p.inks.iter().filter(|(_, ink)| matches!(ink, Ink::Spot(_))).count();
                    return Err(WriteError::SpotInks { inks: spots + p.unseparated.len(), max: MAX_SPOT_INKS, unseparated: p.unseparated });
                }
                if (p.width as i64, p.height as i64) != (rw, rh) {
                    return Err(WriteError::Region(format!("a tile of {rw} × {rh} px separated at {} × {} px", p.width, p.height)));
                }
                for (pid, ink) in p.inks.iter() {
                    let Some(film) = p.plane(pid).filter(|f| f.iter().any(|v| *v > 0)) else { continue };
                    let id = self.list.inks.lookup(ink.name()).ok_or_else(|| WriteError::Region(format!("ink {} is not in the list", ink.name())))?;
                    let Some(plane) = planes.get_mut(id.index()) else { continue };
                    let plane = plane.get_or_insert_with(|| vec![0; w * h]);
                    for (row, src) in film.chunks_exact(rw as usize).take(th as usize).enumerate() {
                        let at = (ty as usize + row) * w + tx as usize;
                        if let (Some(dst), Some(src)) = (plane.get_mut(at..at + tw as usize), src.get(..tw as usize)) {
                            dst.copy_from_slice(src);
                        }
                    }
                }
                tx += tw;
            }
            ty += th;
        }
        Ok(planes)
    }
}

/// A region's pixels on the device grid of the page: `x0..x1` and `y0..y1` count device pixels
/// from the page box's top left corner.
struct Grid {
    page: Rect,
    scale: f64,
    x0: i64,
    y0: i64,
    x1: i64,
    y1: i64,
}

impl Grid {
    /// `bounds` snapped outward and grown by one pixel, within the page's pixels.
    fn new(bounds: Rect, page: Rect, scale: f64) -> Self {
        let px = |v: f64| v.clamp(-1e12, 1e12) as i64;
        let (pw, ph) = (px((page.width() * scale).ceil()), px((page.height() * scale).ceil()));
        Self {
            page,
            scale,
            x0: px(((bounds.x0 - page.x0) * scale).floor() - 1.0).max(0),
            y0: px(((bounds.y0 - page.y0) * scale).floor() - 1.0).max(0),
            x1: px(((bounds.x1 - page.x0) * scale).ceil() + 1.0).min(pw),
            y1: px(((bounds.y1 - page.y0) * scale).ceil() + 1.0).min(ph),
        }
    }

    fn width(&self) -> i64 {
        self.x1 - self.x0
    }

    fn height(&self) -> i64 {
        self.y1 - self.y0
    }

    fn pixels(&self) -> u64 {
        (self.width().max(0) as u64).saturating_mul(self.height().max(0) as u64)
    }

    fn len(&self) -> usize {
        usize::try_from(self.pixels()).unwrap_or(usize::MAX)
    }

    /// Page space of device pixel corner (`x`, `y`).
    fn point(&self, x: i64, y: i64) -> (f64, f64) {
        (self.page.x0 + x as f64 / self.scale, self.page.y0 + y as f64 / self.scale)
    }

    fn rect(&self) -> Rect {
        let (x0, y0) = self.point(self.x0, self.y0);
        let (x1, y1) = self.point(self.x1, self.y1);
        Rect::new(x0, y0, x1, y1)
    }
}

/// How [`flatten_page`] reads and flattens.
#[derive(Clone, Debug, Default)]
pub struct FlattenPageOptions {
    pub read: ReadOptions,
    pub flatten: FlattenOptions,
}

/// What [`flatten_page`] did.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct FlattenPageReport {
    pub read: ReadReport,
    pub flatten: FlattenReport,
    pub raster: RasterReport,
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum FlattenPageError {
    #[error(transparent)]
    Open(#[from] OpenError),
    #[error(transparent)]
    Read(#[from] ReadError),
    #[error(transparent)]
    Flatten(#[from] FlattenError),
    #[error(transparent)]
    Write(#[from] WriteError),
}

/// Page `page` (from 0) of `pdf` with its transparency flattened, as a one-page PDF: the
/// one-page case of [`flatten_document`].
pub fn flatten_page(pdf: &[u8], page: usize, opts: &FlattenPageOptions) -> Result<(Vec<u8>, FlattenPageReport), FlattenPageError> {
    let doc =
        FlattenDocumentOptions { read: opts.read.clone(), flatten: opts.flatten, pages: Some(page..page.saturating_add(1)), ..Default::default() };
    let (bytes, mut report) = flatten_document(pdf, &doc).map_err(|e| match e {
        FlattenDocumentError::Open(e) => FlattenPageError::Open(e),
        FlattenDocumentError::PageRange { .. } => FlattenPageError::Read(ReadError::NoPage(page)),
        FlattenDocumentError::Read { source, .. } => FlattenPageError::Read(source),
        FlattenDocumentError::Flatten { source, .. } => FlattenPageError::Flatten(source),
        FlattenDocumentError::Write { source, .. } => FlattenPageError::Write(source),
        FlattenDocumentError::Serialize(e) => FlattenPageError::Write(WriteError::Pdf(e)),
    })?;
    let (_, report) = report.pages.pop().ok_or(FlattenPageError::Read(ReadError::Interpret))?;
    Ok((bytes, report))
}
