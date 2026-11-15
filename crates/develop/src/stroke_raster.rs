//! A brush stroke's coverage built up as it is painted.
//!
//! The committed stroke is rasterized from scratch by [`crate::masks`] each time a mask renders:
//! every dab stamped again and, for an Auto Mask stroke, a guided filter of the whole frame
//! ([`masks::guided_cross`]). While the stroke is being painted that work grows with its length and
//! is repeated for each pointer move. [`StrokeRaster`] keeps the stroke's coverage and stamps only
//! the dabs added since the last update. An Auto Mask stroke's edge refinement is a guided filter of
//! boxes of finite support (three box blurs, twice: the window statistics, then the averaged
//! coefficients), so new dabs change the refined alpha only within twice that support of the
//! pixels they touched: the filter runs on that window alone, with its inputs read from twice as
//! far again, and the result is the whole-frame filter's up to float rounding of its running sums.
//!
//! The planes are kept over the stroke's bounding box (with some slack so a growing stroke rarely
//! reallocates), not the whole frame.
//!
//! The render path keeps rasters across renders in a [`BrushCache`] ([`crate::StageCache`] holds
//! one), keyed by the source image and the dab list's prefix, so a re-render while a stroke grows
//! (the next pointer move) updates its raster instead of starting again; a UI can drive a
//! [`StrokeRaster`] itself to draw the stroke as it is painted.

use std::ops::Range;
use std::sync::{Arc, Mutex};

use craft_geom::Point;

use crate::masks::{self, BrushDabs, accumulate, auto_refine, auto_similarity, chromaticity, dab_alpha, dab_pixel};
use crate::raster::{Plane, Rgb32f};

/// A rectangle of output pixels, `x0..x1 × y0..y1` (exclusive ends).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PixRect {
    pub x0: usize,
    pub y0: usize,
    pub x1: usize,
    pub y1: usize,
}

impl PixRect {
    pub fn is_empty(&self) -> bool {
        self.x0 >= self.x1 || self.y0 >= self.y1
    }

    pub fn width(&self) -> usize {
        self.x1.saturating_sub(self.x0)
    }

    pub fn height(&self) -> usize {
        self.y1.saturating_sub(self.y0)
    }

    /// The smallest rectangle holding both (an empty one adds nothing).
    pub fn union(self, o: PixRect) -> PixRect {
        if self.is_empty() {
            return o;
        }
        if o.is_empty() {
            return self;
        }
        PixRect { x0: self.x0.min(o.x0), y0: self.y0.min(o.y0), x1: self.x1.max(o.x1), y1: self.y1.max(o.y1) }
    }

    /// Grown by `m` pixels on every side, clamped to a `w × h` output.
    pub fn grow(self, m: usize, w: usize, h: usize) -> PixRect {
        if self.is_empty() {
            return self;
        }
        PixRect {
            x0: self.x0.saturating_sub(m),
            y0: self.y0.saturating_sub(m),
            x1: self.x1.saturating_add(m).min(w),
            y1: self.y1.saturating_add(m).min(h),
        }
    }

    pub fn contains(&self, o: &PixRect) -> bool {
        o.is_empty() || (o.x0 >= self.x0 && o.y0 >= self.y0 && o.x1 <= self.x1 && o.y1 <= self.y1)
    }

    fn area(&self) -> usize {
        self.width().saturating_mul(self.height())
    }
}

/// The output pixels a dab at `d` of radius `r` can paint in a `w × h` output (empty when it lies
/// outside).
pub fn dab_rect(d: Point, r: f64, w: usize, h: usize) -> PixRect {
    let (x0, x1) = (((d.x - r).floor().max(0.0)) as usize, ((d.x + r).ceil().max(0.0) as usize).min(w));
    let (y0, y1) = (((d.y - r).floor().max(0.0)) as usize, ((d.y + r).ceil().max(0.0) as usize).min(h));
    PixRect { x0, y0, x1, y1 }
}

/// How far (pixels) the guided filter's output moves when its input changes at one pixel: two
/// passes of the three box blurs [`crate::raster::blur::gaussian`] makes for `sigma` (the window
/// statistics, then the coefficients), each reaching the sum of its box radii.
pub(crate) fn guided_reach(sigma: f32, w: usize, h: usize) -> usize {
    crate::raster::blur::radii_for(sigma, w, h).map_or(0, |r| 2 * r.iter().sum::<usize>())
}

/// Stamps dabs `range` of `bd` into `buf`, which covers `area` of a `w × h` output row by row
/// (pixels of a dab outside `area` are skipped). Auto Mask dabs weight their paint by how like the
/// pixel under their centre each pixel is (`img`, `log_l`: the output's colours and log
/// luminance). Each pixel takes its dabs in order, so stamping a stroke in parts gives exactly what
/// stamping it at once does.
#[allow(clippy::too_many_arguments)]
pub(crate) fn stamp(bd: &BrushDabs, range: Range<usize>, w: usize, h: usize, img: &Rgb32f, log_l: &Plane, buf: &mut [f32], area: PixRect) {
    if w == 0 || h == 0 || buf.len() < area.area() {
        return;
    }
    let auto = bd.auto && img.data.len() == w * h && log_l.data.len() == w * h;
    let bw = area.width();
    for k in range {
        let Some(d) = bd.dabs.get(k).copied() else { break };
        let (r, hard, flow) = bd.dab(k);
        let refc = auto.then(|| {
            let j = dab_pixel(d, w, h);
            (log_l.data[j], chromaticity(img.data[j]))
        });
        let rect = dab_rect(d, r, w, h);
        let (x0, x1) = (rect.x0.max(area.x0), rect.x1.min(area.x1));
        let (y0, y1) = (rect.y0.max(area.y0), rect.y1.min(area.y1));
        for y in y0..y1 {
            for x in x0..x1 {
                let dd = Point::new(x as f64 + 0.5, y as f64 + 0.5).dist(d);
                if dd > r {
                    continue;
                }
                let mut a = dab_alpha(dd, r, hard);
                if let Some((rl, rch)) = refc {
                    let i = y * w + x;
                    a *= auto_similarity(log_l.data[i], chromaticity(img.data[i]), rl, rch);
                }
                let v = &mut buf[(y - area.y0) * bw + (x - area.x0)];
                // flow accumulates within a stroke up to density
                *v = accumulate(*v, a, flow, bd.density);
            }
        }
    }
}

/// How an Auto Mask stroke decides what to paint (by the settings' process,
/// [`AutoMaskModel::for_process`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AutoMaskModel {
    /// Processes 1 to 8: each pixel compared with the one pixel under the dab's centre, then the
    /// paint's gaps along edges filled by a guided filter on log luminance.
    #[default]
    PerPixel,
    /// Process 9: each dab grows its paint from its centre over a grid of cells in proportion to
    /// the brush, stopping where the path would cross an edge (a change of luminance beyond
    /// shading, or of colour) rather than where a pixel differs from the centre by some amount;
    /// pixels then take it from the like, uniform cells beside them, so the paint ends on the
    /// photo's edge. It paints solidly to three quarters of the radius, closes small holes, and
    /// the guided filter on the smoothed luminance fills only what the dabs found to belong.
    /// Fills noisy, textured and shaded surfaces solidly to their edges, and keeps off a
    /// neighbour of the same brightness but another colour.
    Smoothed,
}

impl AutoMaskModel {
    /// The model process `p` renders Auto Mask strokes with.
    pub fn for_process(p: craft_develop_model::Process) -> AutoMaskModel {
        if p >= craft_develop_model::Process::V9 { AutoMaskModel::Smoothed } else { AutoMaskModel::PerPixel }
    }
}

/// How far past its own pixels a process 9 Auto Mask stroke reads the photo, in radii: a dab's
/// reach, the centres before it its reference and growth start from, and the smoothing and
/// refinement around them.
const AUTO_READS_RADII: f64 = 5.0;

/// `win` (a zoomed view's pixels of a `full_w × full_h` output) grown by what process 9's Auto
/// Mask strokes in `s` read around it, so the view paints them as the whole frame does (the
/// window renders that much more and is cut back). `win` itself without such strokes.
pub fn window_for_reads(
    s: &craft_develop_model::DevelopSettings,
    full_w: usize,
    full_h: usize,
    px_per_long: f64,
    win: crate::PixelWindow,
) -> crate::PixelWindow {
    if AutoMaskModel::for_process(s.process.process()) != AutoMaskModel::Smoothed {
        return win;
    }
    let r = s
        .masks
        .iter()
        .flat_map(|m| &m.components)
        .filter_map(|c| match &c.shape {
            craft_develop_model::MaskShape::Brush { strokes } => Some(strokes),
            _ => None,
        })
        .flatten()
        .filter(|st| st.auto_mask && !st.points.is_empty())
        .map(|st| st.size * px_per_long)
        .filter(|r| r.is_finite())
        .fold(0.0f64, f64::max);
    if r <= 0.0 {
        return win;
    }
    let m = (r * AUTO_READS_RADII).ceil().min(1e9) as usize + 4;
    let (x0, y0) = (win.x.saturating_sub(m), win.y.saturating_sub(m));
    let x1 = win.x.saturating_add(win.w).saturating_add(m).min(full_w.max(1));
    let y1 = win.y.saturating_add(win.h).saturating_add(m).min(full_h.max(1));
    crate::PixelWindow { x: x0, y: y0, w: x1.saturating_sub(x0).max(1), h: y1.saturating_sub(y0).max(1) }
}

/// [`AutoMaskModel::Smoothed`]'s scales for a brush of radius `r` px in a `w × h` output, all in
/// proportion to the brush (so a preview and the full-size export paint the same shape).
#[derive(Clone, Copy, Debug)]
struct Smoothing {
    /// The edge-preserving smoothing of the photo: window sigma (px) and regularization (EV²:
    /// luminance differences well under its root are flattened, edges well over it kept).
    sigma: f32,
    eps: f32,
    /// How far the smoothing reaches (px).
    reach: usize,
    /// The chromaticity is averaged over squares this many px from their centre.
    chroma_box: usize,
    /// Holes in the paint up to about twice this (px) across are closed.
    close: usize,
    /// The final guided filter of the paint on the smoothed luminance.
    refine_sigma: f32,
    refine_eps: f32,
    refine_reach: usize,
}

/// [`Smoothing::eps`]: luminance texture and noise up to about a third of a stop are flattened.
const SMOOTH_EPS: f32 = 0.1;

impl Smoothing {
    fn of(r: f64, w: usize, h: usize) -> Smoothing {
        let sigma = (0.12 * r).max(1.0) as f32;
        let (refine_sigma, refine_eps) = auto_refine(r);
        Smoothing {
            sigma,
            eps: SMOOTH_EPS,
            reach: guided_reach(sigma, w, h),
            chroma_box: (r / (4.0 * CELLS_PER_RADIUS)).round() as usize,
            close: (r / 6.0).round().max(1.0) as usize,
            refine_sigma,
            refine_eps: refine_eps * 0.5,
            refine_reach: guided_reach(refine_sigma, w, h),
        }
    }
}

/// The photo as [`AutoMaskModel::Smoothed`] compares it: luminance smoothed without crossing
/// edges over `win`, chromaticity over the smaller `cwin` (only where dabs paint and sample).
struct Smoothed {
    win: PixRect,
    l: Plane,
    cwin: PixRect,
    ch: Rgb32f,
}

/// The colour difference [`chroma_mean`] averages up to, at least, and how many times the
/// typical difference to the pixels around (their lower quartile: noise) it averages up to more.
const CHROMA_MEAN_RANGE: f32 = 0.04;
const CHROMA_MEAN_NOISE: f32 = 4.0;

/// The chromaticity `ch` averaged over a square `r` px from each pixel (25 samples across it,
/// edges clamped), taking only the samples of much its colour: noise is averaged away, but a
/// pixel beside an edge only the colour marks (a warm white against a grey wall) keeps its own
/// side's colour rather than taking some of its neighbour's.
fn chroma_mean(ch: &Rgb32f, r: usize) -> Rgb32f {
    if r == 0 {
        return ch.clone();
    }
    let (w, h) = (ch.width, ch.height);
    let mut offs: Vec<i64> = [-(r as i64), -(r as i64 / 2), 0, r as i64 / 2, r as i64].to_vec();
    offs.dedup();
    let mut out = Rgb32f::new(w, h);
    let mut d = Vec::with_capacity(offs.len() * offs.len());
    for y in 0..h {
        for x in 0..w {
            let Some(c) = ch.data.get(y * w + x).copied() else { continue };
            d.clear();
            for &oy in &offs {
                let sy = (y as i64 + oy).clamp(0, h as i64 - 1) as usize;
                for &ox in &offs {
                    let sx = (x as i64 + ox).clamp(0, w as i64 - 1) as usize;
                    if let Some(s) = ch.data.get(sy * w + sx) {
                        d.push((chroma_distance(*s, c), *s));
                    }
                }
            }
            let mut ds: Vec<f32> = d.iter().map(|e| e.0).collect();
            ds.sort_by(f32::total_cmp);
            let q = ds.get(ds.len() / 4).copied().unwrap_or(0.0);
            let t = CHROMA_MEAN_RANGE.max(CHROMA_MEAN_NOISE * q);
            let (mut acc, mut wsum) = ([0f32; 3], 0f32);
            for (dist, s) in &d {
                let wt = 1.0 - masks_smooth(t, 2.0 * t, *dist);
                for (a, v) in acc.iter_mut().zip(s) {
                    *a += wt * v;
                }
                wsum += wt;
            }
            if let Some(o) = out.data.get_mut(y * w + x) {
                *o = if wsum > 0.0 { acc.map(|a| a / wsum) } else { c };
            }
        }
    }
    out
}

/// Index of output pixel `(x, y)` in a plane over `r`.
fn index_in(r: &PixRect, x: usize, y: usize) -> Option<usize> {
    if x < r.x0 || y < r.y0 || x >= r.x1 || y >= r.y1 {
        return None;
    }
    (y - r.y0).checked_mul(r.width())?.checked_add(x - r.x0)
}

impl Smoothed {
    fn of(win: PixRect, cwin: PixRect, img: &Rgb32f, log_l: &Plane, k: &Smoothing) -> Smoothed {
        let (ww, wh) = (win.width(), win.height());
        let l = log_l.crop(win.x0, win.y0, ww, wh);
        let ls = masks::guided_self(&l, k.sigma, k.eps);
        // chromaticity averaged over a small square only: smoothing it as far as the luminance,
        // steered by the luminance, would carry one surface's colour over an edge only the colour
        // marks (a warm white against a grey wall as bright), a few pixels into its neighbour
        let (cw, chh) = (cwin.width(), cwin.height());
        let ch = img.crop(cwin.x0, cwin.y0, cw, chh).map(chromaticity);
        Smoothed { win, l: ls, cwin, ch: chroma_mean(&ch, k.chroma_box) }
    }

    /// The smoothed (luminance, chromaticity) at output pixel `(x, y)` (inside the windows).
    fn at(&self, x: usize, y: usize) -> Option<(f32, [f32; 3])> {
        Some((*self.l.data.get(index_in(&self.win, x, y)?)?, *self.ch.data.get(index_in(&self.cwin, x, y)?)?))
    }
}

/// How many dab centres [`AutoMaskModel::Smoothed`] takes its reference from: the dab's own and
/// the ones before it, their median, so one dab whose centre strays over an edge (a wobble of the
/// hand) keeps the side the stroke is on, while a stroke that moves over stays there.
const REFERENCE_DABS: usize = 7;

/// [`AutoMaskModel::Smoothed`] grows each dab's paint from its centre over a grid of cells (the
/// photo averaged over squares of [`Cells::size`] px), stopping where it would cross an edge: a
/// cell belongs to the dab when the path to it from the centre crosses little change. Cells are
/// in proportion to the brush (about this many across its radius), so a preview and the full-size
/// export find the same edges, and averaging over them is what quiets noise.
const CELLS_PER_RADIUS: f64 = 10.0;

/// Steps between neighbouring cells change luminance up to this much (EV) for free: shading over
/// a curved surface, however steep (3 EV over a radius). A larger change costs what it exceeds
/// over [`STEP_EV`]. (More where the cells are noisier: [`Grown`] raises it to a multiple of the
/// typical step.) A path's luminance cost is its steepest step's, not the sum of its steps': the
/// shading and soft creases of a face add up to no edge, however many there are on the way.
const STEP_FREE_EV: f32 = 0.3;
const STEP_EV: f32 = 0.5;

/// Chromaticity changes up to this much between neighbouring cells are free, and more costs what
/// it exceeds over [`STEP_CHROMA`], adding up along the path (a gradual drift of colour is free,
/// a step of it is not). Much stricter than luminance: shading darkens a surface but
/// leaves its colour, so where the colour changes is where one surface meets another (a warm,
/// shaded white against a neutral grey wall of the same brightness).
const STEP_FREE_CHROMA: f32 = 0.03;
const STEP_CHROMA: f32 = 0.03;

/// How many cells a path may jump over to a cell like the one it leaves (a thin line across a
/// surface: a crease, a fold, the shadow under a ledge), when what it jumps over is no further
/// than [`JUMP_CHROMA`] in colour from either side.
const JUMP_CELLS: usize = 2;
const JUMP_CHROMA: f32 = 0.15;

/// How many times the difference noise alone makes between neighbouring cells' means is still
/// free (that noise measured inside the flattest cells around a dab, [`grow`]).
const STEP_NOISE: f32 = 3.0;

/// A cell whose chromaticity is up to this far from the reference's belongs (as far as colour
/// goes); twice as far, not at all (more where the cells are noisier: [`STEP_NOISE`] times the
/// typical step): however gently the colour changes on the way, a surface of a quite different
/// colour is another surface. Loose, as light changes a surface's colour too (a white lit by a
/// gold board beside it); the edges between surfaces are what [`grow`] stops at.
const REF_CHROMA: f32 = 0.15;

/// A cell whose pixels spread about their mean by up to this much (EV, chromaticity; more where
/// the cells around are noisier, as [`STEP_NOISE`] raises the free step) is one surface; at twice
/// it, it straddles an edge and vouches for no pixel ([`pixel_member`]).
const UNIFORM_EV: f32 = 0.1;
const UNIFORM_CHROMA: f32 = 0.02;

/// A cell spreading up to this many times as far as the typical cell the dab grew over is uniform
/// too (a textured surface's cells all spread).
const UNIFORM_TYPICAL: f32 = 1.5;

/// Chromaticity distance at which a pixel is no longer like a cell ([`pixel_member`]); at least
/// this, more in noisier cells.
const PIXEL_CHROMA: f32 = 0.08;

/// A cell is wholly the dab's up to this accumulated path cost, not at all past
/// [`PATH_NONE`].
const PATH_FULL: f32 = 0.35;
const PATH_NONE: f32 = 0.6;

/// However smooth the way, a cell this far ([`feature_distance`]) from the reference is not the
/// dab's (2.5 to 3.75 EV): a long, gentle gradient into a different surface.
const FAR_FULL: f32 = 5.0;
const FAR_NONE: f32 = 7.5;

/// A colour as Auto Mask compares it: log luminance and chromaticity.
type Feature = (f32, [f32; 3]);

/// A cell's slope of log luminance (EV per px across and down) and its centre (output pixels).
type CellPlane = ((f32, f32), (f32, f32));

/// The distance between two chromaticities.
#[inline]
fn chroma_distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

/// How different two colours are, in Auto Mask tolerances: log luminance over
/// [`masks::AUTO_TOL_EV`], chromaticity over [`masks::AUTO_TOL_CHROMA`] (1: the old per-pixel
/// model's edge of full paint is 0.5, of none 1).
#[inline]
fn feature_distance(a: (f32, [f32; 3]), b: (f32, [f32; 3])) -> f32 {
    let dl = (a.0 - b.0) / masks::AUTO_TOL_EV;
    let dc = ((a.1[0] - b.1[0]).powi(2) + (a.1[1] - b.1[1]).powi(2) + (a.1[2] - b.1[2]).powi(2)).sqrt() / masks::AUTO_TOL_CHROMA;
    (dl * dl + dc * dc).sqrt()
}

/// The grid [`AutoMaskModel::Smoothed`] grows paint over: square cells of `size` px, one starting
/// at the pixel under the stroke's first dab (so a stroke painted in parts and at once, and a
/// zoomed window of the frame and the whole frame, read the same cells of the photo). Cell `i`
/// covers pixels `i·size − ox ..` across (`ox`, `oy` in `0..size`).
#[derive(Clone, Copy, Debug)]
struct Cells {
    size: usize,
    ox: usize,
    oy: usize,
    w: usize,
    h: usize,
}

impl Cells {
    /// The grid for a brush of radius `r` px whose stroke starts at `first`, in a `w × h` output.
    fn of(r: f64, first: Option<Point>, w: usize, h: usize) -> Cells {
        let size = (r / CELLS_PER_RADIUS).round().max(1.0) as usize;
        let off = |v: f64| if v.is_finite() { (-(v.floor() as i64)).rem_euclid(size as i64) as usize } else { 0 };
        let (ox, oy) = first.map_or((0, 0), |p| (off(p.x), off(p.y)));
        Cells { size, ox, oy, w, h }
    }

    fn cols(&self) -> usize {
        (self.w + self.ox).div_ceil(self.size)
    }

    fn rows(&self) -> usize {
        (self.h + self.oy).div_ceil(self.size)
    }

    /// The cell holding output pixel `(x, y)`.
    fn of_pixel(&self, x: usize, y: usize) -> (usize, usize) {
        ((x + self.ox) / self.size, (y + self.oy) / self.size)
    }

    /// Whether output point `(x, y)` is in the output.
    fn holds(&self, x: f64, y: f64) -> bool {
        x >= 0.0 && y >= 0.0 && x < self.w as f64 && y < self.h as f64
    }

    /// The cell holding output point `(x, y)` (clamped into the output).
    fn at(&self, x: f64, y: f64) -> (usize, usize) {
        let c = |v: f64, o: usize, n: usize| ((v.max(0.0) as usize + o) / self.size).min(n.saturating_sub(1));
        (c(x, self.ox, self.cols()), c(y, self.oy, self.rows()))
    }

    /// Cell `(i, j)`'s mean log luminance and chromaticity, from the photo itself (`None` outside).
    fn feature(&self, i: usize, j: usize, img: &Rgb32f, log_l: &Plane) -> Option<(f32, [f32; 3])> {
        self.feature_spread(i, j, img, log_l).map(|(f, _)| f)
    }

    /// [`Self::feature`] and how far its pixels spread about it (standard deviations of log
    /// luminance and chromaticity): a cell straddling an edge spreads far.
    fn feature_spread(&self, i: usize, j: usize, img: &Rgb32f, log_l: &Plane) -> Option<(Feature, (f32, f32))> {
        self.cell_stats(i, j, img, log_l).map(|c| (c.mean, c.spread))
    }

    /// Cell `(i, j)`'s [`CellStats`] (`None` outside the output).
    fn cell_stats(&self, i: usize, j: usize, img: &Rgb32f, log_l: &Plane) -> Option<CellStats> {
        let (sx, sy) = (i.checked_mul(self.size)?, j.checked_mul(self.size)?);
        let (x0, y0) = (sx.saturating_sub(self.ox), sy.saturating_sub(self.oy));
        let (x1, y1) = ((sx + self.size).saturating_sub(self.ox).min(self.w), (sy + self.size).saturating_sub(self.oy).min(self.h));
        if x0 >= x1 || y0 >= y1 {
            return None;
        }
        // (the pixels' offsets from the middle of the cell's pixels: a rectangle of them, so the
        // two directions' slopes fit apart)
        let (cx, cy) = ((x0 + x1 - 1) as f32 / 2.0, (y0 + y1 - 1) as f32 / 2.0);
        let (mut l, mut ch, mut n) = (0f32, [0f32; 3], 0f32);
        let (mut lu, mut lv, mut cu, mut cv, mut uu, mut vv) = (0f32, 0f32, [0f32; 3], [0f32; 3], 0f32, 0f32);
        for y in y0..y1 {
            let v = y as f32 - cy;
            for x in x0..x1 {
                let u = x as f32 - cx;
                let k = y * self.w + x;
                let lk = *log_l.data.get(k)?;
                l += lk;
                lu += lk * u;
                lv += lk * v;
                let c = masks::chromaticity(*img.data.get(k)?);
                for q in 0..3 {
                    ch[q] += c[q];
                    cu[q] += c[q] * u;
                    cv[q] += c[q] * v;
                }
                uu += u * u;
                vv += v * v;
                n += 1.0;
            }
        }
        let mean = (l / n, ch.map(|v| v / n));
        let slope = |s: f32, d: f32| if d > 0.0 { s / d } else { 0.0 };
        let (gx, gy) = (slope(lu, uu), slope(lv, vv));
        // the luminance's spread about the plane through the cell (its shading taken out: a
        // steeply shaded cell of one surface is still uniform; how steep is judged apart), the
        // colour's about its mean (shading leaves the colour, so a cell whose colour changes
        // across it straddles an edge)
        let (mut vl, mut vc) = (0f32, 0f32);
        for y in y0..y1 {
            let v = y as f32 - cy;
            for x in x0..x1 {
                let u = x as f32 - cx;
                let k = y * self.w + x;
                vl += (log_l.data.get(k)? - mean.0 - gx * u - gy * v).powi(2);
                vc += chroma_distance(masks::chromaticity(*img.data.get(k)?), mean.1).powi(2);
            }
        }
        // (the fits took degrees of freedom from the pixels: what is left spreads by the rest,
        // so a small cell's spread is not understated)
        let (vl, vc) = (vl / (n - 3.0).max(1.0), vc / (n - 1.0).max(1.0));
        // (the slope as far as it stands out of the noise: a small, noisy cell's fitted slope is
        // mostly noise, and carried a cell on it would be a guess)
        let shrink = |g: f32, d: f32| {
            let var = if d > 0.0 { vl / d } else { f32::INFINITY };
            if g * g + var > 0.0 && var.is_finite() { g * g * g / (g * g + var) } else { 0.0 }
        };
        Some(CellStats {
            mean,
            spread: (vl.sqrt(), vc.sqrt()),
            change: gx.hypot(gy) * self.size as f32,
            grad: (shrink(gx, uu), shrink(gy, vv)),
            centre: (cx + 0.5, cy + 0.5),
        })
    }
}

/// A cell's colour as [`AutoMaskModel::Smoothed`] grows over it: its mean, how far its pixels
/// spread (log luminance about the plane through it, chromaticity about its mean), how far the
/// plane's luminance changes across the cell (EV), the plane's slope (EV per px across and down,
/// as far as it stands out of the noise) and the cell's centre (output pixels).
#[derive(Clone, Copy, Debug)]
struct CellStats {
    mean: Feature,
    spread: (f32, f32),
    change: f32,
    grad: (f32, f32),
    centre: (f32, f32),
}

/// The reference of dab `k` of `bd`: the median, channel by channel, of the cells under its centre
/// and the [`REFERENCE_DABS`] − 1 centres before it.
fn reference(bd: &BrushDabs, k: usize, cells: &Cells, img: &Rgb32f, log_l: &Plane) -> Option<(f32, [f32; 3])> {
    let mut got: Vec<(f32, [f32; 3])> = Vec::with_capacity(REFERENCE_DABS);
    for j in k.saturating_sub(REFERENCE_DABS - 1)..=k {
        let d = bd.dabs.get(j)?;
        // (a centre past the output's edge has nothing under it here)
        if !cells.holds(d.x, d.y) {
            continue;
        }
        let (i, jj) = cells.at(d.x, d.y);
        if let Some(v) = cells.feature(i, jj, img, log_l) {
            got.push(v);
        }
    }
    let median = |mut v: Vec<f32>| {
        v.sort_by(f32::total_cmp);
        v.get(v.len() / 2).copied()
    };
    let l = median(got.iter().map(|g| g.0).collect())?;
    let ch = [0, 1, 2].map(|c| median(got.iter().map(|g| g.1[c]).collect()).unwrap_or(0.0));
    Some((l, ch))
}

/// Dab `k`'s cells: which belong to it (0..1) over the cells its circle reaches, grown from the
/// cells under its centre and the centres before it (each starting at the cost of how unlike the
/// reference it is, so a centre that strays over an edge seeds nothing) by the cheapest path,
/// a step costing the change across it beyond what is free ([`STEP_FREE_EV`],
/// [`STEP_FREE_CHROMA`]): the path's steepest step in luminance plus the sum of its steps in
/// colour. A path may jump over a thin line ([`JUMP_CELLS`]).
struct Grown {
    /// [`pixel_member`]'s chromaticity tolerance here.
    chroma_tol: f32,
    i0: usize,
    j0: usize,
    n: usize,
    m: usize,
    member: Vec<f32>,
    feature: Vec<(f32, [f32; 3])>,
    /// Each cell's slope of log luminance and centre ([`CellStats`]).
    plane: Vec<CellPlane>,
    /// How uniform each cell is ([`Cells::cell_stats`]).
    uniform: Vec<f32>,
    #[cfg(test)]
    cost: Vec<f32>,
    #[cfg(test)]
    free: (f32, f32),
}

impl Grown {
    /// Cell `(i, j)`: how much it belongs, weighted by how uniform it is, its colour, and its
    /// luminance's slope and centre.
    fn get(&self, i: usize, j: usize) -> Option<(f32, Feature, CellPlane)> {
        if i < self.i0 || j < self.j0 || i >= self.i0 + self.n || j >= self.j0 + self.m {
            return None;
        }
        let k = (j - self.j0) * self.n + (i - self.i0);
        Some((*self.member.get(k)? * *self.uniform.get(k)?, *self.feature.get(k)?, *self.plane.get(k)?))
    }
}

fn grow(bd: &BrushDabs, k: usize, r: f64, cells: &Cells, img: &Rgb32f, log_l: &Plane, refc: (f32, [f32; 3])) -> Option<Grown> {
    use std::cmp::Reverse;
    use std::collections::BinaryHeap;
    let d = *bd.dabs.get(k)?;
    let (i0, j0) = cells.at(d.x - r, d.y - r);
    let (i1, j1) = cells.at(d.x + r, d.y + r);
    let (n, m) = (i1 + 1 - i0, j1 + 1 - j0);
    let mut feature = Vec::with_capacity(n * m);
    let mut spread = Vec::with_capacity(n * m);
    let mut plane = Vec::with_capacity(n * m);
    let mut change = Vec::with_capacity(n * m);
    let mut inside = Vec::with_capacity(n * m);
    for j in j0..=j1 {
        for i in i0..=i1 {
            // (a cell past the output's edge is no way through: a zoomed window and the whole
            // frame must agree)
            let got = cells.cell_stats(i, j, img, log_l);
            inside.push(got.is_some());
            let st =
                got.unwrap_or(CellStats { mean: refc, spread: (f32::INFINITY, f32::INFINITY), change: 0.0, grad: (0.0, 0.0), centre: (0.0, 0.0) });
            feature.push(st.mean);
            spread.push(st.spread);
            change.push(st.change);
            plane.push((st.grad, st.centre));
        }
    }
    // the noise: how far pixels spread inside the flattest cells around (a low percentile, so
    // the edges and texture of other surfaces in reach, however many, don't count), and from it
    // how far two neighbouring cells' means differ by noise alone
    let low = |mut v: Vec<f32>| {
        v.sort_by(f32::total_cmp);
        v.get(v.len() / 10).copied().filter(|x| x.is_finite()).unwrap_or(0.0)
    };
    let (ql, qc) = (low(spread.iter().map(|s| s.0).collect()), low(spread.iter().map(|s| s.1).collect()));
    let per_step = std::f32::consts::SQRT_2 / cells.size as f32;
    let (nl, nc) = (ql * per_step, qc * per_step);
    let (free_l, free_c) = (STEP_FREE_EV.max(STEP_NOISE * nl), STEP_FREE_CHROMA.max(STEP_NOISE * nc));
    // (a cell whose luminance changes across it by more than a free step holds an edge, however
    // well a plane fits it: its spread counts that)
    for (s, c) in spread.iter_mut().zip(&change) {
        s.0 = s.0.hypot((c - free_l).max(0.0));
    }
    let ref_c = REF_CHROMA.max(STEP_NOISE * nc);
    // a step's cost in luminance and in colour, apart: a path's luminance cost is its steepest
    // step's, its colour cost the sum of its steps' ([`grow`])
    let step_lc = |a: (f32, [f32; 3]), b: (f32, [f32; 3])| {
        (((a.0 - b.0).abs() - free_l).max(0.0) / STEP_EV, (chroma_distance(a.1, b.1) - free_c).max(0.0) / STEP_CHROMA)
    };
    let step_cost = |a: (f32, [f32; 3]), b: (f32, [f32; 3])| {
        let (l, c) = step_lc(a, b);
        l + c
    };
    let mut cost = vec![f32::INFINITY; n * m];
    // (the path's luminance and colour costs so far, apart)
    let mut parts = vec![(f32::INFINITY, f32::INFINITY); n * m];
    // (costs as ordered integers: millionths)
    let key = |c: f32| (c * 1e6) as u64;
    let mut heap = BinaryHeap::new();
    for j in k.saturating_sub(REFERENCE_DABS - 1)..=k {
        let Some(c) = bd.dabs.get(j).filter(|c| cells.holds(c.x, c.y)) else { continue };
        let (ci, cj) = cells.at(c.x, c.y);
        if ci < i0 || cj < j0 || ci > i1 || cj > j1 {
            continue;
        }
        let at = (cj - j0) * n + (ci - i0);
        // (a centre on another surface than the reference's costs what stepping onto it would,
        // less what a full member may cost: a centre on texture still seeds its surface)
        let seed = (step_cost(refc, feature[at]) - PATH_FULL).max(0.0);
        if seed < cost[at] {
            cost[at] = seed;
            parts[at] = (0.0, seed);
            heap.push(Reverse((key(seed), at)));
        }
    }
    while let Some(Reverse((kc, at))) = heap.pop() {
        let c = cost[at];
        if key(c) < kc || c > PATH_NONE {
            continue;
        }
        let (pl, pc) = parts[at];
        let (x, y) = (at % n, at / n);
        for (dx, dy) in [(-1i64, 0i64), (1, 0), (0, -1), (0, 1), (-1, -1), (1, -1), (-1, 1), (1, 1)] {
            // a step to the next cell, or a jump over one or two to a cell like this one: over a
            // thin line (a crease, a fold, the shadow under a ledge) of much the same colour
            for len in 1..=1 + JUMP_CELLS {
                let (nx, ny) = (x as i64 + dx * len as i64, y as i64 + dy * len as i64);
                if nx < 0 || ny < 0 || nx >= n as i64 || ny >= m as i64 {
                    break;
                }
                let to = ny as usize * n + nx as usize;
                if !inside[to] {
                    break;
                }
                let (sl, sc) = if len == 1 {
                    step_lc(feature[at], feature[to])
                } else {
                    let (a, b) = (feature[at], feature[to]);
                    // (what is jumped over is a line on the surface, not another surface: its
                    // colour close to both sides'; and the two sides alike as neighbours are, or
                    // a jump over a cell straddling an edge would cross it as freely as shading)
                    let mid = (1..len).map(|t| feature[(y as i64 + dy * t as i64) as usize * n + (x as i64 + dx * t as i64) as usize]);
                    if mid.into_iter().any(|f| chroma_distance(f.1, a.1).min(chroma_distance(f.1, b.1)) > JUMP_CHROMA) {
                        continue;
                    }
                    step_lc(a, b)
                };
                let (nl, nc) = (pl.max(sl), pc + sc);
                let next = nl + nc;
                if next < cost[to] {
                    cost[to] = next;
                    parts[to] = (nl, nc);
                    heap.push(Reverse((key(next), to)));
                }
            }
        }
    }
    // a cell is uniform for the surface the dab grew over: its texture and noise spread every
    // cell of it alike (the typical spread of the cells reached), an edge through one further
    let typical = |f: &dyn Fn(&(f32, f32)) -> f32| {
        let mut v: Vec<f32> = spread.iter().zip(&cost).filter(|(_, c)| **c <= PATH_FULL).map(|(s, _)| f(s)).filter(|x| x.is_finite()).collect();
        v.sort_by(f32::total_cmp);
        v.get(v.len() / 2).copied().unwrap_or(0.0)
    };
    let (tl, tc) = (typical(&|s| s.0), typical(&|s| s.1));
    let (ul, uc) = (UNIFORM_EV.max(2.0 * ql).max(UNIFORM_TYPICAL * tl), UNIFORM_CHROMA.max(2.0 * qc).max(UNIFORM_TYPICAL * tc));
    let uniform: Vec<f32> = spread.iter().map(|&(sl, sc)| (1.0 - masks_smooth(ul, 2.0 * ul, sl)) * (1.0 - masks_smooth(uc, 2.0 * uc, sc))).collect();
    let member = cost
        .iter()
        .zip(&feature)
        .map(|(c, f)| {
            (1.0 - masks_smooth(PATH_FULL, PATH_NONE, *c))
                * (1.0 - masks_smooth(FAR_FULL, FAR_NONE, feature_distance(*f, refc)))
                * (1.0 - masks_smooth(ref_c, 2.0 * ref_c, chroma_distance(f.1, refc.1)))
        })
        .collect();
    Some(Grown {
        chroma_tol: PIXEL_CHROMA.max(2.0 * free_c),
        i0,
        j0,
        n,
        m,
        member,
        feature,
        plane,
        uniform,
        #[cfg(test)]
        cost,
        #[cfg(test)]
        free: (free_l, free_c),
    })
}

#[inline]
fn masks_smooth(e0: f32, e1: f32, x: f32) -> f32 {
    if !x.is_finite() {
        return if x > 0.0 { 1.0 } else { 0.0 };
    }
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// How much output pixel `(x, y)` (its smoothed colour `p`) belongs to grown cells `g`: as much as
/// the most it is like a nearby cell (up to two away) that belongs and is uniform, so the boundary
/// falls on the pixels where the photo changes, not on the cells', and a cell straddling the edge
/// (its mean between the two sides) lends nothing to the other side.
fn pixel_member(g: &Grown, cells: &Cells, x: usize, y: usize, p: (f32, [f32; 3])) -> f32 {
    let (ci, cj) = cells.of_pixel(x, y);
    let cs = cells.size as f32;
    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
    let mut best = 0f32;
    for j in cj.saturating_sub(2)..=cj + 2 {
        for i in ci.saturating_sub(2)..=ci + 2 {
            if let Some((m, f, ((gx, gy), (cx, cy)))) = g.get(i, j)
                && m > best
            {
                // (nearer cells count for more: a pixel's membership blends between the cells'
                // centres rather than stepping at their borders)
                let d = (cx - px).hypot(cy - py) / cs;
                let near = 1.0 - masks_smooth(1.0, 2.5, d);
                // (the cell's luminance carried on along its slope, up to a cell past its centre:
                // shading continues past it, but no further than it was measured)
                let at = f.0 + gx * (px - cx).clamp(-cs, cs) + gy * (py - cy).clamp(-cs, cs);
                let dl = (p.0 - at) / masks::AUTO_TOL_EV;
                let dc = chroma_distance(p.1, f.1) / g.chroma_tol;
                best = best.max(m * near * (1.0 - masks_smooth(0.5, 1.0, (dl * dl + dc * dc).sqrt())));
            }
        }
    }
    best
}

/// The share of a radius an Auto Mask dab of process 9 paints fully before it fades (at least its
/// own hard core): what belongs to the object under it is painted solidly to its edges, the
/// feather only softening where the brush ends inside it.
const AUTO_HARD: f64 = 0.75;

/// [`stamp`] for [`AutoMaskModel::Smoothed`]: each dab paints the pixels its centre's surface
/// reaches without crossing an edge ([`grow`], [`pixel_member`]), solidly to [`AUTO_HARD`] of
/// its radius. `elig` (over `area` too) keeps, for each pixel, the most any dab found it to
/// belong, feather aside: what the refinement may fill up to.
#[allow(clippy::too_many_arguments)]
fn stamp_smoothed(
    bd: &BrushDabs,
    range: Range<usize>,
    w: usize,
    h: usize,
    img: &Rgb32f,
    log_l: &Plane,
    sm: &Smoothed,
    buf: &mut [f32],
    elig: &mut [f32],
    area: PixRect,
) {
    if w == 0 || h == 0 || buf.len() < area.area() || elig.len() < area.area() || img.data.len() != w * h || log_l.data.len() != w * h {
        return;
    }
    let cells = Cells::of(bd.r, bd.dabs.first().copied(), w, h);
    let bw = area.width();
    for k in range {
        let Some(d) = bd.dabs.get(k).copied() else { break };
        let (r, hard, flow) = bd.dab(k);
        let Some(refc) = reference(bd, k, &cells, img, log_l) else { continue };
        let Some(g) = grow(bd, k, r, &cells, img, log_l, refc) else { continue };
        let hard = hard.max(AUTO_HARD * r);
        let rect = dab_rect(d, r, w, h);
        let (x0, x1) = (rect.x0.max(area.x0), rect.x1.min(area.x1));
        let (y0, y1) = (rect.y0.max(area.y0), rect.y1.min(area.y1));
        for y in y0..y1 {
            for x in x0..x1 {
                let dd = Point::new(x as f64 + 0.5, y as f64 + 0.5).dist(d);
                if dd > r {
                    continue;
                }
                let Some(p) = sm.at(x, y) else { continue };
                let mb = pixel_member(&g, &cells, x, y, p);
                if mb <= 0.0 {
                    continue;
                }
                let i = (y - area.y0) * bw + (x - area.x0);
                let a = dab_alpha(dd, r, hard) * mb;
                if let Some(v) = buf.get_mut(i) {
                    *v = accumulate(*v, a, flow, bd.density);
                }
                if let Some(e) = elig.get_mut(i) {
                    *e = e.max(mb);
                }
            }
        }
    }
}

/// A square max (`max` true) or min filter of radius `r` over `p`, separable, edges clamped.
fn rank_filter(p: &Plane, r: usize, max: bool) -> Plane {
    let (w, h) = (p.width, p.height);
    let pick = |a: f32, b: f32| if max { a.max(b) } else { a.min(b) };
    let mut t = Plane::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let (a, b) = (x.saturating_sub(r), (x + r).min(w.saturating_sub(1)));
            let mut v = p.data[y * w + a];
            for xi in a + 1..=b {
                v = pick(v, p.data[y * w + xi]);
            }
            t.data[y * w + x] = v;
        }
    }
    let mut o = Plane::new(w, h);
    for y in 0..h {
        let (a, b) = (y.saturating_sub(r), (y + r).min(h.saturating_sub(1)));
        for x in 0..w {
            let mut v = t.data[a * w + x];
            for yi in a + 1..=b {
                v = pick(v, t.data[yi * w + x]);
            }
            o.data[y * w + x] = v;
        }
    }
    o
}

/// What a stroke's dabs are painted with; a raster only takes more dabs painted the same way.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Brush {
    r: f64,
    hard: f64,
    flow: f32,
    density: f32,
    erase: bool,
    auto: bool,
}

impl Brush {
    fn of(bd: &BrushDabs) -> Brush {
        Brush { r: bd.r, hard: bd.hard, flow: bd.flow, density: bd.density, erase: bd.erase, auto: bd.auto }
    }
}

/// One brush stroke's coverage in a `w × h` output, updated dab by dab as the stroke grows.
///
/// [`Self::update`] takes the stroke's dabs so far ([`masks::brush_dabs`]); when they continue the
/// ones already painted (the same brush, the painted dabs a prefix of them) only the new ones are
/// stamped, otherwise the raster starts again. [`Self::alpha_at`] / [`Self::to_plane`] then give
/// what [`masks::shape_alpha`] would for this stroke alone (exactly for a plain stroke; an Auto
/// Mask stroke's refinement up to float rounding, well under 1e-3).
#[derive(Clone, Debug)]
pub struct StrokeRaster {
    w: usize,
    h: usize,
    model: AutoMaskModel,
    brush: Option<Brush>,
    dabs: Vec<Point>,
    pressure: Vec<f32>,
    /// The output pixels the planes cover.
    bounds: PixRect,
    /// The dabs' accumulated paint over `bounds`.
    raw: Vec<f32>,
    /// The refined coverage over `bounds` (Auto Mask strokes; plain ones use `raw`).
    refined: Vec<f32>,
    /// How much each pixel over `bounds` belongs to what the dabs grew over, feather aside
    /// ([`AutoMaskModel::Smoothed`]): the refinement fills no further.
    elig: Vec<f32>,
}

impl StrokeRaster {
    /// An empty raster for a `w × h` output, Auto Mask strokes painted the way processes 1 to 8
    /// do ([`AutoMaskModel::PerPixel`]; [`Self::with_model`] for another).
    pub fn new(w: usize, h: usize) -> StrokeRaster {
        StrokeRaster::with_model(w, h, AutoMaskModel::PerPixel)
    }

    /// An empty raster for a `w × h` output, Auto Mask strokes painted by `model`.
    pub fn with_model(w: usize, h: usize, model: AutoMaskModel) -> StrokeRaster {
        StrokeRaster {
            w,
            h,
            model,
            brush: None,
            dabs: Vec::new(),
            pressure: Vec::new(),
            bounds: PixRect::default(),
            raw: Vec::new(),
            refined: Vec::new(),
            elig: Vec::new(),
        }
    }

    /// How Auto Mask strokes are painted.
    pub fn model(&self) -> AutoMaskModel {
        self.model
    }

    /// The output size.
    pub fn size(&self) -> (usize, usize) {
        (self.w, self.h)
    }

    /// Dabs painted so far.
    pub fn dabs(&self) -> usize {
        self.dabs.len()
    }

    /// Whether this is an erasing stroke.
    pub fn erases(&self) -> bool {
        self.brush.is_some_and(|b| b.erase)
    }

    /// The output pixels the coverage may be non-zero in (the planes' extent).
    pub fn bounds(&self) -> PixRect {
        self.bounds
    }

    /// Bytes held.
    pub fn bytes(&self) -> usize {
        (self.raw.len() + self.refined.len() + self.elig.len()) * 4 + self.dabs.len() * 16 + self.pressure.len() * 4
    }

    /// Whether `bd` continues the stroke painted so far: the same brush, and the dabs painted so
    /// far (with their pressures) the start of `bd`'s.
    pub fn continues(&self, bd: &BrushDabs) -> bool {
        let n = self.dabs.len();
        match self.brush {
            None => true,
            Some(b) => {
                b == Brush::of(bd)
                    && bd.dabs.get(..n).is_some_and(|p| p == self.dabs.as_slice())
                    && if bd.pressure.is_empty() {
                        self.pressure.is_empty()
                    } else {
                        self.pressure.len() == n && bd.pressure.get(..n).is_some_and(|p| p == self.pressure.as_slice())
                    }
            }
        }
    }

    /// Paints the dabs of `bd` (the whole stroke so far, as [`masks::brush_dabs`] makes it for the
    /// `w × h` output) not painted yet, starting again when `bd` doesn't continue what was painted.
    /// `img` and `log_l` are the output's colours and log luminance (what an Auto Mask stroke
    /// compares and snaps to; they must be `w × h`, or an Auto Mask stroke paints nothing). Returns
    /// the rectangle of output pixels whose coverage may have changed (`None`: nothing did).
    pub fn update(&mut self, bd: &BrushDabs, img: &Rgb32f, log_l: &Plane) -> Option<PixRect> {
        let (w, h) = (self.w, self.h);
        let mut changed = PixRect::default();
        if !self.continues(bd) {
            changed = self.bounds;
            *self = StrokeRaster::with_model(w, h, self.model);
        }
        let brush = Brush::of(bd);
        self.brush = Some(brush);
        if brush.auto && (img.width, img.height, log_l.width, log_l.height) != (w, h, w, h) {
            // nothing to compare against: a stroke that paints nothing rather than a wrong one
            self.dabs = bd.dabs.clone();
            self.pressure = bd.pressure.clone();
            return (!changed.is_empty()).then_some(changed);
        }
        let start = self.dabs.len();
        let new = start..bd.dabs.len();
        let touched = new.clone().fold(PixRect::default(), |acc, k| {
            let (r, _, _) = bd.dab(k);
            acc.union(dab_rect(bd.dabs[k], r, w, h))
        });
        self.dabs.extend_from_slice(&bd.dabs[new.clone()]);
        if !bd.pressure.is_empty() {
            self.pressure.extend_from_slice(bd.pressure.get(start..).unwrap_or_default());
        }
        if touched.is_empty() {
            return (!changed.is_empty()).then_some(changed);
        }
        if brush.auto && self.model == AutoMaskModel::Smoothed {
            let k = Smoothing::of(brush.r, w, h);
            // the paint changes in `touched`, its closing up to `close` twice beyond, the guided
            // filter's output its reach beyond that; the inputs of all that come from `win`
            let out = touched.grow(2 * k.close + k.refine_reach, w, h);
            // (and the centres of the dabs before the new ones, which their references read)
            let before = start.saturating_sub(REFERENCE_DABS - 1)..start;
            let read = before.fold(touched, |acc, j| acc.union(dab_rect(bd.dabs[j], 1.0, w, h)));
            // the smoothed luminance steers the refinement over `out` and so is read its reach
            // beyond; the chromaticity only where dabs paint and sample; each smoothing reads
            // its own reach beyond what it gives
            let win = read.grow(2 * k.close + 2 * k.refine_reach + k.reach, w, h);
            let cwin = read.grow(k.reach, w, h);
            self.cover(out);
            let sm = Smoothed::of(win, cwin, img, log_l, &k);
            let bounds = self.bounds;
            stamp_smoothed(bd, new, w, h, img, log_l, &sm, &mut self.raw, &mut self.elig, bounds);
            self.refine_smoothed(out, &sm, &k);
            return Some(changed.union(out));
        }
        let (sigma, eps) = auto_refine(brush.r);
        let reach = if brush.auto { guided_reach(sigma, w, h) } else { 0 };
        let out = touched.grow(reach, w, h);
        self.cover(out);
        let bounds = self.bounds;
        stamp(bd, new, w, h, img, log_l, &mut self.raw, bounds);
        if brush.auto {
            self.refine(touched, out, reach, sigma, eps, log_l);
        }
        Some(changed.union(out))
    }

    /// Grows the planes to cover `r` (with slack, so a stroke growing a little at a time doesn't
    /// copy its planes on every update).
    fn cover(&mut self, r: PixRect) {
        if self.bounds.contains(&r) && !self.bounds.is_empty() {
            return;
        }
        let want = self.bounds.union(r);
        let slack = (want.width().max(want.height()) / 4).max(32);
        let next = want.grow(slack, self.w, self.h);
        let auto = self.brush.is_some_and(|b| b.auto);
        let (bw, nw) = (self.bounds.width(), next.width());
        let mut raw = vec![0.0; next.area()];
        let mut refined = if auto { vec![0.0; next.area()] } else { Vec::new() };
        let smoothed = auto && self.model == AutoMaskModel::Smoothed;
        let mut elig = if smoothed { vec![0.0; next.area()] } else { Vec::new() };
        for y in self.bounds.y0..self.bounds.y1 {
            let src = (y - self.bounds.y0) * bw;
            let dst = (y - next.y0) * nw + (self.bounds.x0 - next.x0);
            if let (Some(s), Some(d)) = (self.raw.get(src..src + bw), raw.get_mut(dst..dst + bw)) {
                d.copy_from_slice(s);
            }
            if let (Some(s), Some(d)) = (self.refined.get(src..src + bw), refined.get_mut(dst..dst + bw)) {
                d.copy_from_slice(s);
            }
            if let (Some(s), Some(d)) = (self.elig.get(src..src + bw), elig.get_mut(dst..dst + bw)) {
                d.copy_from_slice(s);
            }
        }
        self.bounds = next;
        self.raw = raw;
        self.refined = refined;
        self.elig = elig;
    }

    /// The Auto Mask refinement where the dabs in `touched` changed the paint: the guided filter of
    /// the paint steered by `log_l`, written over `out` (`touched` grown by `reach`, how far the
    /// filter's output moves), computed on `touched` grown by twice that (all its output in `out`
    /// reads).
    fn refine(&mut self, touched: PixRect, out: PixRect, reach: usize, sigma: f32, eps: f32, log_l: &Plane) {
        let (w, h) = (self.w, self.h);
        let win = touched.grow(2 * reach, w, h);
        let (ww, wh) = (win.width(), win.height());
        let b = self.bounds;
        let mut p = Plane::new(ww, wh);
        for y in win.y0..win.y1 {
            if y < b.y0 || y >= b.y1 {
                continue;
            }
            let (x0, x1) = (win.x0.max(b.x0), win.x1.min(b.x1));
            if x0 >= x1 {
                continue;
            }
            let src = (y - b.y0) * b.width() + (x0 - b.x0);
            let dst = (y - win.y0) * ww + (x0 - win.x0);
            if let (Some(s), Some(d)) = (self.raw.get(src..src + (x1 - x0)), p.data.get_mut(dst..dst + (x1 - x0))) {
                d.copy_from_slice(s);
            }
        }
        let guide = log_l.crop(win.x0, win.y0, ww, wh);
        let q = masks::guided_cross(&guide, &p, sigma, eps);
        for y in out.y0..out.y1 {
            for x in out.x0..out.x1 {
                let i = (y - b.y0) * b.width() + (x - b.x0);
                let qi = (y - win.y0) * ww + (x - win.x0);
                if let (Some(r), Some(v), Some(f)) = (self.raw.get(i), q.data.get(qi), self.refined.get_mut(i)) {
                    // the refinement fills gaps along edges; it never takes paint away
                    *f = r.max(*v);
                }
            }
        }
    }

    /// The paint over `win` (0 outside the planes).
    fn raw_in(&self, win: PixRect) -> Plane {
        self.plane_in(&self.raw, win)
    }

    /// `plane` (one of the raster's, over `bounds`) over `win` (0 outside the planes).
    fn plane_in(&self, plane: &[f32], win: PixRect) -> Plane {
        let (ww, wh) = (win.width(), win.height());
        let b = self.bounds;
        let mut p = Plane::new(ww, wh);
        for y in win.y0.max(b.y0)..win.y1.min(b.y1) {
            let (x0, x1) = (win.x0.max(b.x0), win.x1.min(b.x1));
            if x0 >= x1 {
                continue;
            }
            let src = (y - b.y0) * b.width() + (x0 - b.x0);
            let dst = (y - win.y0) * ww + (x0 - win.x0);
            if let (Some(s), Some(d)) = (plane.get(src..src + (x1 - x0)), p.data.get_mut(dst..dst + (x1 - x0))) {
                d.copy_from_slice(s);
            }
        }
        p
    }

    /// [`AutoMaskModel::Smoothed`]'s refinement over `out`: the paint closed (holes up to about
    /// twice `close` filled), then its gaps along edges filled by the guided filter on the
    /// smoothed luminance, neither past what the dabs found to belong (`elig`, closed alike);
    /// never less than the paint.
    fn refine_smoothed(&mut self, out: PixRect, sm: &Smoothed, k: &Smoothing) {
        let win = sm.win;
        let p = self.raw_in(win);
        let closed = rank_filter(&rank_filter(&p, k.close, true), k.close, false);
        let q = masks::guided_cross(&sm.l, &closed, k.refine_sigma, k.refine_eps);
        // neither the closing nor the guided filter takes paint past what the dabs found to belong
        // (its own holes closed alike): across an edge the guided filter would otherwise spread
        // its window's mean, a soft arc of paint over the neighbour
        let e = self.plane_in(&self.elig, win);
        let cap = rank_filter(&rank_filter(&e, k.close, true), k.close, false);
        let (b, ww) = (self.bounds, win.width());
        for y in out.y0..out.y1 {
            for x in out.x0..out.x1 {
                let i = (y - b.y0) * b.width() + (x - b.x0);
                let wi = (y - win.y0) * ww + (x - win.x0);
                if let (Some(r), Some(c), Some(v), Some(cap), Some(f)) =
                    (p.data.get(wi), closed.data.get(wi), q.data.get(wi), cap.data.get(wi), self.refined.get_mut(i))
                {
                    *f = r.max(c.max(*v).min(*cap));
                }
            }
        }
    }

    /// The stroke's coverage at output pixel `(x, y)` (0 outside it).
    pub fn alpha_at(&self, x: usize, y: usize) -> f32 {
        let b = self.bounds;
        if x < b.x0 || x >= b.x1 || y < b.y0 || y >= b.y1 {
            return 0.0;
        }
        let i = (y - b.y0) * b.width() + (x - b.x0);
        let plane = if self.brush.is_some_and(|b| b.auto) { &self.refined } else { &self.raw };
        plane.get(i).copied().unwrap_or(0.0)
    }

    /// The stroke's coverage over `rect` (row by row; 0 outside the stroke).
    pub fn alpha_in(&self, rect: PixRect) -> Vec<f32> {
        let mut out = Vec::with_capacity(rect.area());
        for y in rect.y0..rect.y1 {
            for x in rect.x0..rect.x1 {
                out.push(self.alpha_at(x, y));
            }
        }
        out
    }

    /// The stroke's coverage as a `w × h` plane.
    pub fn to_plane(&self) -> Plane {
        let mut p = Plane::new(self.w, self.h);
        self.apply_to(&mut p);
        p
    }

    /// Combines the stroke into a brush component's alpha `out` (`w × h`) as the renderer does:
    /// the larger of the two, or for an erasing stroke what it leaves.
    pub fn apply_to(&self, out: &mut Plane) {
        if (out.width, out.height) != (self.w, self.h) {
            return;
        }
        let erase = self.erases();
        let b = self.bounds;
        for y in b.y0..b.y1 {
            for x in b.x0..b.x1 {
                let a = self.alpha_at(x, y);
                if let Some(o) = out.data.get_mut(y * self.w + x) {
                    *o = if erase { *o * (1.0 - a) } else { o.max(a) };
                }
            }
        }
    }
}

/// Most bytes of stroke rasters a [`BrushCache`] keeps.
pub const BRUSH_CACHE_BYTES: usize = 96 << 20;

/// Most stroke rasters a [`BrushCache`] keeps.
pub const BRUSH_CACHE_STROKES: usize = 64;

struct Entry {
    src: Arc<Rgb32f>,
    key: u64,
    raster: StrokeRaster,
}

/// Stroke rasters kept between renders of one view, so a stroke that grew since the last render
/// (the next pointer move while painting) stamps only its new dabs ([`StrokeRaster`]). Entries are
/// matched on the source image (the same `Arc`), a key standing for everything else the render
/// read (the output size and frame, the colours the image was rendered to before masking), and
/// the stroke's dabs. The least recently used go first past [`BRUSH_CACHE_BYTES`] or
/// [`BRUSH_CACHE_STROKES`].
#[derive(Default)]
pub struct BrushCache {
    entries: Mutex<Vec<Entry>>,
}

/// A [`BrushCache`] with the source and key of one render.
#[derive(Clone, Copy)]
pub struct BrushCacheRef<'a> {
    pub cache: &'a BrushCache,
    pub src: &'a Arc<Rgb32f>,
    pub key: u64,
}

impl BrushCache {
    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Entry>> {
        self.entries.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Drops every raster.
    pub fn clear(&self) {
        self.lock().clear();
    }

    /// Rasters held.
    pub fn len(&self) -> usize {
        self.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Bytes held.
    pub fn bytes(&self) -> usize {
        self.lock().iter().map(|e| e.raster.bytes()).sum()
    }

    /// The raster of `bd` painted so far for this render's source and key (taken out of the cache:
    /// [`Self::put`] it back), the one that has the most of it; a new one when none fits.
    pub fn take(&self, at: &BrushCacheRef<'_>, bd: &BrushDabs, w: usize, h: usize, model: AutoMaskModel) -> StrokeRaster {
        let mut v = self.lock();
        let best = v
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                e.key == at.key && Arc::ptr_eq(&e.src, at.src) && e.raster.size() == (w, h) && e.raster.model == model && e.raster.continues(bd)
            })
            .max_by_key(|(_, e)| e.raster.dabs())
            .map(|(i, _)| i);
        match best {
            Some(i) => v.remove(i).raster,
            None => StrokeRaster::with_model(w, h, model),
        }
    }

    /// Keeps `raster` (the most recently used), dropping the least recently used past the limits.
    pub fn put(&self, at: &BrushCacheRef<'_>, raster: StrokeRaster) {
        if raster.bytes() > BRUSH_CACHE_BYTES {
            return;
        }
        let mut v = self.lock();
        v.push(Entry { src: at.src.clone(), key: at.key, raster });
        let mut total: usize = v.iter().map(|e| e.raster.bytes()).sum();
        while v.len() > BRUSH_CACHE_STROKES || total > BRUSH_CACHE_BYTES {
            let gone = v.remove(0);
            total = total.saturating_sub(gone.raster.bytes());
        }
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod photo_probe;
#[cfg(test)]
mod quality_tests;
#[cfg(test)]
mod scene_tests;

#[cfg(test)]
mod surface_tests;
