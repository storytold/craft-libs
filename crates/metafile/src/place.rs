//! Placing a picture on a target (a page or a device): the fitted transform, paths as kurbo paths in
//! target units and bitmaps as target rectangles. The rasteriser and the PDF writer only paint the result.

use std::sync::Arc;

use kurbo::{Affine, BezPath, Rect, Shape};

use std::collections::HashMap;

use crate::{ClipPath, Item, LineStyle, Picture, Seg};

/// Largest bitmap side the renderers draw: within the PDF image limit and the rasteriser's 16-bit sides.
pub const MAX_BITMAP_SIDE: u32 = 30_000;
/// Placed coordinates past this (target units) are far off any page; such items are left out.
const MAX_COORD: f64 = 1e6;
/// The most a crop side pads the source: a hundred times its size.
const MIN_CROP: f32 = -100.0;
/// Smallest placed bitmap side (target units), so the stretch onto it is never singular.
const MIN_BITMAP_SIDE: f64 = 1e-6;
/// Maximum aggregate path elements retained in placed output.
const MAX_PLACED_ELEMENTS: usize = 500_000;
/// The least of a source axis a crop keeps (0.5%). Files crop pictures to slivers (a header bar
/// that is 2.7% of a page-sized metafile). A frame of at most 4000 units a side then spans at most
/// 800,000 units for the whole source: what it covers stays inside `MAX_COORD` and is never left
/// out as far off the page.
const MIN_VISIBLE: f64 = 0.005;

/// The part of a picture's source that `crop` (left, top, right, bottom fractions of the source)
/// keeps, as fractions of the source: [left, top, width, height], bitmap or metafile alike. Each
/// side is taken as 0–1 (junk as 0). A negative side pads the source instead of cutting it (down
/// to -100), as a negative `a:srcRect` side does in Office files and a picture fitted whole into
/// a frame of another shape needs: the window then starts before the source or ends past it.
/// When the two sides of an axis leave less than 0.5% of it, the left (top) side stands and 0.5%
/// shows.
pub fn crop_window(crop: [f32; 4]) -> [f64; 4] {
    let [l, t, r, b] = crop.map(|v| if v.is_finite() { f64::from(v.clamp(MIN_CROP, 1.0)) } else { 0.0 });
    let axis = |lo: f64, hi: f64| {
        let lo = lo.min(1.0 - MIN_VISIBLE);
        (lo, (1.0 - lo - hi).max(MIN_VISIBLE))
    };
    let ((x, w), (y, h)) = (axis(l, r), axis(t, b));
    [x, y, w, h]
}

/// One item of a placed picture, in target units. `region` is the device-context clip region and
/// `clips` the clip paths: an item shows inside the region and inside every clip path.
pub enum PlacedItem {
    /// A filled and/or stroked path. The stroke width is in target units; 0 is a cosmetic hairline that
    /// the renderer draws at its minimum width. `line` has its dash lengths in target units.
    Path {
        path: BezPath,
        fill: Option<[u8; 4]>,
        stroke: Option<([u8; 4], f64)>,
        line: LineStyle,
        even_odd: bool,
        region: Option<Arc<BezPath>>,
        clips: Vec<Arc<PlacedClip>>,
    },
    /// `Picture::items[index]`, a bitmap, stretched into `rect`. `transform` maps the unit square
    /// (the whole bitmap) to target units: the same place when the bitmap is upright, and the
    /// turned or skewed place when it is not.
    Bitmap { index: usize, rect: Rect, transform: Affine, region: Option<Arc<BezPath>>, clips: Vec<Arc<PlacedClip>> },
    /// `Picture::items[index]`, a pattern fill: `path` (target units) filled with the item's tile
    /// repeated; `tile` maps tile pixels to target units.
    Pattern { index: usize, path: BezPath, even_odd: bool, tile: Affine, region: Option<Arc<BezPath>>, clips: Vec<Arc<PlacedClip>> },
    /// `Picture::items[index]`, a text string: `transform` maps its text space to target units;
    /// `clip` (target units) bounds what it draws.
    Text { index: usize, transform: Affine, clip: Option<Rect>, region: Option<Arc<BezPath>>, clips: Vec<Arc<PlacedClip>> },
}

/// A placed clip region: `Some(None)` is the default clip, `None` a region that does not meet the page.
type PlacedRegion = Option<Option<Arc<BezPath>>>;

/// A clip path in target units. Items under the same clip share it.
pub struct PlacedClip {
    pub path: BezPath,
    pub even_odd: bool,
}

impl Picture {
    /// Whether the picture has a finite, positive size, so it can be fitted into a frame.
    pub fn has_size(&self) -> bool {
        self.width.is_finite() && self.height.is_finite() && self.width > 0.0 && self.height > 0.0
    }

    /// The transform that fits the picture into `r` with the crop fractions `crop` (left, top, right,
    /// bottom) removed first: the visible part fills `r`. None for a degenerate size.
    pub fn fit(&self, r: Rect, crop: [f32; 4]) -> Option<Affine> {
        let [cl, ct, vw, vh] = crop_window(crop);
        let (w, h) = (f64::from(self.width), f64::from(self.height));
        let sx = r.width() / (w * vw);
        let sy = r.height() / (h * vh);
        (sx.is_finite() && sy.is_finite() && sx > 0.0 && sy > 0.0)
            .then(|| Affine::translate((r.x0 - cl * w * sx, r.y0 - ct * h * sy)) * Affine::scale_non_uniform(sx, sy))
    }

    /// The items placed into `r` with `crop` applied. Items that are not finite or reach past ±1e6
    /// target units are left out (they are far off any page), as are bitmaps with no area. Stroke
    /// widths are scaled to target units (along the mean axis scale, so a non-uniform fit keeps
    /// strokes even).
    pub fn place(&self, r: Rect, crop: [f32; 4]) -> Vec<PlacedItem> {
        let Some(tf) = self.fit(r, crop) else { return Vec::new() };
        let c = tf.as_coeffs();
        let k = (c[0].abs() + c[3].abs()) / 2.0;
        let mut out = Vec::with_capacity(self.items.len());
        let mut total_elements = 0usize;
        // The last region placed: consecutive items under the same clip share one placed path.
        // Each region expanded counts against the element budget like the paths do.
        let region_elements = std::cell::Cell::new(0usize);
        let mut cached_region: Option<(Option<Vec<[f32; 4]>>, PlacedRegion)> = None;
        let mut get_region = |reg: &Option<Vec<[f32; 4]>>| -> PlacedRegion {
            if let Some((key, res)) = &cached_region
                && key == reg
            {
                return res.clone();
            }
            let res = placed_region(&tf, reg);
            let n = res.as_ref().and_then(Option::as_ref).map_or(0, |p| p.elements().len());
            region_elements.set(region_elements.get().saturating_add(n));
            cached_region = Some((reg.clone(), res.clone()));
            res
        };
        // Whether the paths and regions placed so far are past the budget.
        let over = |paths: usize| paths.saturating_add(region_elements.get()) > MAX_PLACED_ELEMENTS;

        // Clip paths by address: the picture holds them for the whole call. None is a clip that is
        // not finite or reaches past the coordinate limit; what it clips is left out.
        let mut placed_clips: HashMap<*const ClipPath, Option<Arc<PlacedClip>>> = HashMap::new();
        let mut clip_elements = 0usize;
        let mut get_clips = |clips: &[Arc<ClipPath>]| -> Option<Vec<Arc<PlacedClip>>> {
            clips
                .iter()
                .map(|c| {
                    placed_clips
                        .entry(Arc::as_ptr(c))
                        .or_insert_with(|| {
                            let path = tf * bez_path(&c.segs);
                            clip_elements = clip_elements.saturating_add(path.elements().len());
                            (clip_elements <= MAX_PLACED_ELEMENTS && in_range(path.bounding_box()))
                                .then(|| Arc::new(PlacedClip { path, even_odd: c.even_odd }))
                        })
                        .clone()
                })
                .collect()
        };

        for (index, item) in self.items.iter().enumerate() {
            if total_elements.saturating_add(region_elements.get()) >= MAX_PLACED_ELEMENTS {
                break;
            }
            match item {
                Item::Path { segs, fill, stroke, line, even_odd, region, clips } => {
                    let path = tf * bez_path(segs);
                    let Some(region) = get_region(region) else { continue };
                    // This item's own region is expanded by now: it counts before the item is kept.
                    if over(total_elements) {
                        break;
                    }
                    let Some(clips) = get_clips(clips) else { continue };
                    let line = LineStyle { dash: line.dash.iter().map(|d| (f64::from(*d) * k) as f32).collect(), ..line.clone() };
                    let bounds = path.bounding_box();
                    if in_range(bounds) && line.dash.iter().all(|d| d.is_finite()) {
                        total_elements = total_elements.saturating_add(path.elements().len());
                        // A pen wider than twice the distance across the frame and the path
                        // already covers the whole frame: wider adds nothing the frame shows, only
                        // work for the renderer flattening its round caps (the width is file data).
                        let u = bounds.union(r);
                        let widest = 2.0 * u.width().hypot(u.height());
                        let stroke = stroke.map(|(c, w)| (c, (f64::from(w) * k).min(widest)));
                        out.push(PlacedItem::Path { path, fill: *fill, stroke, line, even_odd: *even_odd, region, clips });
                    }
                }
                Item::Pattern { segs, even_odd, tile: t, region, clips } => {
                    let path = tf * bez_path(segs);
                    let Some(region) = get_region(region) else { continue };
                    // This item's own region is expanded by now: it counts before the item is kept.
                    if over(total_elements) {
                        break;
                    }
                    let Some(clips) = get_clips(clips) else { continue };
                    let tile = tf
                        * Affine::translate((f64::from(t.origin[0]), f64::from(t.origin[1])))
                        * Affine::scale_non_uniform(f64::from(t.cell[0]), f64::from(t.cell[1]));
                    let det = tile.determinant();
                    if in_range(path.bounding_box()) && det.is_finite() && det.abs() > 1e-12 {
                        total_elements = total_elements.saturating_add(path.elements().len());
                        out.push(PlacedItem::Pattern { index, path, even_odd: *even_odd, tile, region, clips });
                    }
                }
                Item::Text(t) => {
                    let transform = tf * Affine::new(t.transform);
                    let Some(region) = get_region(&t.region) else { continue };
                    // This item's own region is expanded by now: it counts before the item is kept.
                    if over(total_elements) {
                        break;
                    }
                    let Some(clips) = get_clips(&t.clips) else { continue };
                    let clip =
                        t.clip.map(|[x0, y0, x1, y1]| tf.transform_rect_bbox(Rect::new(f64::from(x0), f64::from(y0), f64::from(x1), f64::from(y1))));
                    let c = transform.as_coeffs();
                    let origin = Rect::new(c[4], c[5], c[4], c[5]);
                    if in_range(origin) && c.iter().all(|v| v.is_finite() && v.abs() <= MAX_COORD) && clip.is_none_or(in_range) {
                        out.push(PlacedItem::Text { index, transform, clip, region, clips });
                    }
                }
                Item::Bitmap { rect: [x, y, w, h], transform, region, clips, .. } => {
                    let (x0, y0) = (f64::from(*x), f64::from(*y));
                    let rect = tf.transform_rect_bbox(Rect::new(x0, y0, x0 + f64::from(*w), y0 + f64::from(*h)));
                    let Some(region) = get_region(region) else { continue };
                    // This item's own region is expanded by now: it counts before the item is kept.
                    if over(total_elements) {
                        break;
                    }
                    let Some(clips) = get_clips(clips) else { continue };
                    let transform = tf * Affine::new(*transform);
                    let finite = transform.as_coeffs().iter().all(|v| v.is_finite() && v.abs() <= MAX_COORD);
                    if in_range(rect) && finite && rect.width() > MIN_BITMAP_SIDE && rect.height() > MIN_BITMAP_SIDE {
                        out.push(PlacedItem::Bitmap { index, rect, transform, region, clips });
                    }
                }
            }
        }
        out
    }
}

impl Item {
    /// Takes a bitmap's pixels out of the item (its geometry stays) and returns them as (width, height,
    /// RGBA) when a renderer can draw them: both sides in 1..=[`MAX_BITMAP_SIDE`] and exactly that many
    /// pixels. None for paths and for bitmaps that cannot be drawn.
    pub fn take_pixels(&mut self) -> Option<(u32, u32, Vec<u8>)> {
        let Item::Bitmap { width, height, rgba, .. } = self else { return None };
        let (w, h, px) = (*width, *height, std::mem::take(rgba));
        let sides = (1..=MAX_BITMAP_SIDE).contains(&w) && (1..=MAX_BITMAP_SIDE).contains(&h);
        (sides && px.len() == (w as usize).saturating_mul(h as usize).saturating_mul(4)).then_some((w, h, px))
    }
}

/// The device-context clip in target units. `Some(None)` is the default clip. `None` means the
/// region does not meet the page, so the item draws nothing.
fn placed_region(tf: &Affine, region: &Option<Vec<[f32; 4]>>) -> PlacedRegion {
    let Some(rects) = region else { return Some(None) };
    let mut path = BezPath::new();
    for [x0, y0, x1, y1] in rects {
        let r = tf.transform_rect_bbox(Rect::new(f64::from(*x0), f64::from(*y0), f64::from(*x1), f64::from(*y1)));
        if in_range(r) && r.width() > 1e-9 && r.height() > 1e-9 {
            path.extend(r.to_path(0.1));
        }
    }
    (!path.is_empty()).then_some(Some(Arc::new(path)))
}

/// Segments as a kurbo path. A segment after a close opens a new subpath at the last start point.
pub fn bez_path(segs: &[Seg]) -> BezPath {
    let mut p = BezPath::new();
    let mut start = (0.0, 0.0);
    let mut open = false;
    for s in segs {
        match *s {
            Seg::Move(x, y) => {
                start = (f64::from(x), f64::from(y));
                p.move_to(start);
                open = true;
            }
            Seg::Line(x, y) => {
                if !open {
                    p.move_to(start);
                }
                p.line_to((f64::from(x), f64::from(y)));
                open = true;
            }
            Seg::Cubic(a, b, c, d, e, f) => {
                if !open {
                    p.move_to(start);
                }
                p.curve_to((f64::from(a), f64::from(b)), (f64::from(c), f64::from(d)), (f64::from(e), f64::from(f)));
                open = true;
            }
            Seg::Close => {
                if open {
                    p.close_path();
                }
                open = false;
            }
        }
    }
    p
}

/// Whether a placed box is finite and within ±[`MAX_COORD`].
fn in_range(b: Rect) -> bool {
    [b.x0, b.y0, b.x1, b.y1].iter().all(|v| v.is_finite() && v.abs() < MAX_COORD)
}
