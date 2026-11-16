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
    /// Process 9: each dab grows over a grid of cells in proportion to the brush from its centre,
    /// stopping where the path would cross an edge (a change of luminance beyond shading, or of
    /// colour, or a cell straddling one) rather than where a pixel differs from the centre by some
    /// amount. What the stroke's dabs found, small holes and seams filled, is a coarse paint whose
    /// edge lies within a cell of the photo's; a guided filter on the smoothed photo's luminance
    /// and colour carries it onto the photo's own edge, anti-aliased as the photo is, and the
    /// brush's feathered footprint (solid to three quarters of the radius) shapes it where the
    /// stroke ends inside the object. Fills noisy, textured and shaded surfaces solidly to their
    /// edges, and keeps off a neighbour of the same brightness but another colour.
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

/// [`AutoMaskModel::Smoothed`]'s scales for a brush of radius `r` px, all in
/// proportion to the brush (so a preview and the full-size export paint the same shape).
#[derive(Clone, Copy, Debug)]
struct Smoothing {
    /// The edge-preserving smoothing of the photo's luminance the paint's edge snaps to: window
    /// radius (px) and regularization (EV²: differences well under its root are flattened, edges
    /// well over it kept).
    radius: usize,
    eps: f32,
    /// The chromaticity is averaged over squares this many px from their centre.
    chroma_box: usize,
    /// How far (cells) [`CellMap::filled`] looks for cells that belong around a hole.
    hole: usize,
    /// The guided filter that carries the cells' coarse paint onto the photo's edges: its window
    /// radius (px).
    snap: usize,
}

/// [`Smoothing::eps`]: luminance texture and noise up to about a third of a stop are flattened.
const SMOOTH_EPS: f32 = 0.1;

impl Smoothing {
    fn of(r: f64) -> Smoothing {
        let size = (r / CELLS_PER_RADIUS).round().max(1.0);
        Smoothing {
            radius: (0.2 * r).round().max(1.0) as usize,
            eps: SMOOTH_EPS,
            chroma_box: (r / (4.0 * CELLS_PER_RADIUS)).round() as usize,
            hole: HOLE_CELLS,
            snap: (SNAP_CELLS * size).round().max(2.0) as usize,
        }
    }
}

/// The photo as [`AutoMaskModel::Smoothed`] snaps the paint's edge to, over a window, each part
/// smoothed without crossing edges: log luminance, and two of the chromaticity's three parts (red
/// and blue over luminance; the third follows from them).
struct Smoothed {
    l: Vec<f32>,
    c0: Vec<f32>,
    c2: Vec<f32>,
}

/// The chromaticity's smoothing flattens differences well under this (texture, noise) and keeps
/// edges well over it (a warm white against a cooler grey wall: 0.08).
const SMOOTH_CHROMA: f32 = 0.015;

impl Smoothed {
    /// The smoothed photo over `win`, each smoothing read as far beyond it as it reaches (so a
    /// window and the whole frame agree).
    fn of(win: PixRect, img: &Rgb32f, log_l: &Plane, k: &Smoothing) -> Smoothed {
        let (w, h) = (log_l.width, log_l.height);
        let (ww, wh) = (win.width(), win.height());
        let cut = |v: Vec<f32>, from: PixRect| -> Vec<f32> {
            let fw = from.width();
            let mut o = Vec::with_capacity(ww * wh);
            for y in win.y0..win.y1 {
                let s = (y - from.y0) * fw + (win.x0 - from.x0);
                o.extend_from_slice(v.get(s..s + ww).unwrap_or_default());
            }
            o
        };
        // (a self-guided filter of box windows: its output reads twice its radius around)
        let lwin = win.grow(2 * k.radius, w, h);
        let (lw, lh) = (lwin.width(), lwin.height());
        let l = log_l.crop(lwin.x0, lwin.y0, lw, lh);
        let l = || cut(smooth_self(&l.data, lw, lh, k.radius, k.eps), lwin);
        // (the chromaticity over a small square only: smoothed as far as the luminance, the
        // corners of an edge only the colour marks would round off)
        let r = k.chroma_box.max(1);
        let cwin = win.grow(2 * r, w, h);
        let (cw, chh) = (cwin.width(), cwin.height());
        let ch = img.crop(cwin.x0, cwin.y0, cw, chh).map(chromaticity);
        let part = |q: usize| {
            let v: Vec<f32> = ch.data.iter().map(|c| c[q]).collect();
            cut(smooth_self(&v, cw, chh, r, SMOOTH_CHROMA * SMOOTH_CHROMA), cwin)
        };
        let (l, (c0, c2)) = rayon::join(l, || rayon::join(|| part(0), || part(2)));
        Smoothed { l, c0, c2 }
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

/// What a path pays to pass through a cell that straddles an edge (its pixels spread far more
/// than the cells around: [`grow`]), more than any path may cost: a path may end on such a cell
/// but not go on from it. Without this, a path walked along a slanting edge from cell to cell,
/// each holding a little more of the other side, and crossed it a free step at a time.
const BARRIER: f32 = 1.0;

/// A cell's pixels' spread ([`Cells::cell_stats`]) is measured over at least this many pixels
/// across, the cell and the pixels around it: a photo's edges are a few pixels soft.
const SPREAD_PX: usize = 6;

/// A cell straddles an edge of colour, as far as [`BARRIER`] goes, when its pixels' chromaticity
/// spreads this much, or twice as much as noise does, or half again as much as the typical cell in
/// reach (a textured surface's cells all spread); twice that, fully. A cell holding a sliver of a
/// neighbour of another colour spreads a hundredth or two.
const BARRIER_CHROMA: f32 = 0.008;
const BARRIER_TYPICAL: f32 = 1.5;

/// Holes in what a stroke grew over (a freckle, a nostril, a closed eye's crease, a speck on a cake
/// board, the seam between two things painted) up to about twice this many cells across are
/// filled ([`CellMap::filled`]): four tenths of the brush's radius.
const HOLE_CELLS: usize = 4;

/// A cell some dab found to belong this much or more is painted fully, this little or less not
/// at all ([`CellMap::filled`]): a dab finds a cell partly its own where the way there costs a
/// little more than is free, which says the cell is likely the object's, not that it is half
/// painted. Unfirmed, the guided filter carried such halves into the paint as a haze or a
/// pinhole along a seam.
const MAP_NONE: f32 = 0.2;
const MAP_FULL: f32 = 0.6;

/// The window radius of the guided filter that carries the cells' coarse paint onto the photo's
/// edges, in cells: the coarse edge lies within about a cell of the photo's.
const SNAP_CELLS: f64 = 1.5;

/// The guided filter's guide: smoothed log luminance over this many EV, chromaticity over this
/// much, a unit each; and its regularization (in those units squared): differences of colour or
/// luminance much under its root (texture, noise) are not edges the paint follows.
const SNAP_EV: f32 = 1.0;
const SNAP_CHROMA: f32 = 0.04;
const SNAP_EPS: f32 = 0.02;

/// The guided filter's output is taken from this much to its complement as none to all: a paint's
/// edge as crisp as the photo's (the filter's own ramp is as wide as its window where the photo's
/// colour drifts beside the edge), and a filter's faint spill past an edge none.
const SNAP_SOFT: f32 = 0.1;

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
        // how far the pixels spread is measured over at least SPREAD_PX across: a cell smaller
        // than an edge is soft could sit wholly on the edge's ramp and look uniform
        let e = SPREAD_PX.saturating_sub(self.size).div_ceil(2);
        let (ex0, ey0, ex1, ey1) = (x0.saturating_sub(e), y0.saturating_sub(e), (x1 + e).min(self.w), (y1 + e).min(self.h));
        // (the pixels' offsets from the middle of the pixels measured: a rectangle of them, so
        // the two directions' slopes fit apart)
        let (cx, cy) = ((ex0 + ex1 - 1) as f32 / 2.0, (ey0 + ey1 - 1) as f32 / 2.0);
        let (mut l, mut ch, mut n) = (0f32, [0f32; 3], 0f32);
        let (mut lo, mut cho, mut no) = (0f32, [0f32; 3], 0f32);
        let (mut lu, mut lv, mut uu, mut vv) = (0f32, 0f32, 0f32, 0f32);
        for y in ey0..ey1 {
            let v = y as f32 - cy;
            for x in ex0..ex1 {
                let u = x as f32 - cx;
                let k = y * self.w + x;
                let lk = *log_l.data.get(k)?;
                let c = masks::chromaticity(*img.data.get(k)?);
                l += lk;
                lu += lk * u;
                lv += lk * v;
                for q in 0..3 {
                    ch[q] += c[q];
                }
                uu += u * u;
                vv += v * v;
                n += 1.0;
                if x >= x0 && x < x1 && y >= y0 && y < y1 {
                    lo += lk;
                    for q in 0..3 {
                        cho[q] += c[q];
                    }
                    no += 1.0;
                }
            }
        }
        // (the cell's own colour is its own pixels')
        let mean = (lo / no, cho.map(|v| v / no));
        let around = (l / n, ch.map(|v| v / n));
        let slope = |s: f32, d: f32| if d > 0.0 { s / d } else { 0.0 };
        let (gx, gy) = (slope(lu, uu), slope(lv, vv));
        // the luminance's spread about the plane through the cell (its shading taken out: a
        // steeply shaded cell of one surface is still uniform; how steep is judged apart), the
        // colour's about its mean (shading leaves the colour, so a cell whose colour changes
        // across it straddles an edge)
        let (mut vl, mut vc) = (0f32, 0f32);
        for y in ey0..ey1 {
            let v = y as f32 - cy;
            for x in ex0..ex1 {
                let u = x as f32 - cx;
                let k = y * self.w + x;
                vl += (log_l.data.get(k)? - around.0 - gx * u - gy * v).powi(2);
                vc += chroma_distance(masks::chromaticity(*img.data.get(k)?), around.1).powi(2);
            }
        }
        // (the fits took degrees of freedom from the pixels: what is left spreads by the rest,
        // so a small cell's spread is not understated)
        let (vl, vc) = (vl / (n - 3.0).max(1.0), vc / (n - 1.0).max(1.0));
        Some(CellStats { mean, spread: (vl.sqrt(), vc.sqrt()), change: gx.hypot(gy) * self.size as f32 })
    }
}

/// A cell's colour as [`AutoMaskModel::Smoothed`] grows over it: its mean, how far its pixels
/// spread (log luminance about the plane through it, chromaticity about its mean) and how far the
/// plane's luminance changes across the cell (EV).
#[derive(Clone, Copy, Debug)]
struct CellStats {
    mean: Feature,
    spread: (f32, f32),
    change: f32,
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
/// colour. A path may jump over a thin line ([`JUMP_CELLS`]) and may not go on from a cell
/// straddling an edge ([`BARRIER`]).
struct Grown {
    i0: usize,
    j0: usize,
    n: usize,
    m: usize,
    member: Vec<f32>,
    #[cfg(test)]
    cost: Vec<f32>,
    #[cfg(test)]
    free: (f32, f32),
}

impl Grown {
    /// How much cell `(i, j)` is the dab's (0 outside its grid). A cell straddling the object's
    /// edge may count as the dab's whole: where on it the edge lies is the photo's to say
    /// ([`snap`]).
    fn vouch(&self, i: usize, j: usize) -> f32 {
        if i < self.i0 || j < self.j0 || i >= self.i0 + self.n || j >= self.j0 + self.m {
            return 0.0;
        }
        let k = (j - self.j0) * self.n + (i - self.i0);
        self.member.get(k).copied().unwrap_or(0.0)
    }
}

/// Cells' [`CellStats`] already measured: neighbouring dabs reach mostly the same cells.
#[derive(Default)]
struct CellCache(std::collections::HashMap<(usize, usize), Option<CellStats>>);

/// How many cells a [`CellCache`] holds before it starts again (a bound on its memory).
const CELL_CACHE_MAX: usize = 1 << 20;

impl CellCache {
    fn get(&mut self, cells: &Cells, i: usize, j: usize, img: &Rgb32f, log_l: &Plane) -> Option<CellStats> {
        if let Some(v) = self.0.get(&(i, j)) {
            return *v;
        }
        if self.0.len() >= CELL_CACHE_MAX {
            self.0.clear();
        }
        let v = cells.cell_stats(i, j, img, log_l);
        self.0.insert((i, j), v);
        v
    }
}

#[allow(clippy::too_many_arguments)]
fn grow(bd: &BrushDabs, k: usize, r: f64, cells: &Cells, img: &Rgb32f, log_l: &Plane, refc: (f32, [f32; 3]), cache: &mut CellCache) -> Option<Grown> {
    use std::cmp::Reverse;
    use std::collections::BinaryHeap;
    let d = *bd.dabs.get(k)?;
    let (i0, j0) = cells.at(d.x - r, d.y - r);
    let (i1, j1) = cells.at(d.x + r, d.y + r);
    let (n, m) = (i1 + 1 - i0, j1 + 1 - j0);
    let mut feature = Vec::with_capacity(n * m);
    let mut spread = Vec::with_capacity(n * m);
    let mut change = Vec::with_capacity(n * m);
    let mut inside = Vec::with_capacity(n * m);
    for j in j0..=j1 {
        for i in i0..=i1 {
            // (a cell past the output's edge is no way through: a zoomed window and the whole
            // frame must agree)
            let got = cache.get(cells, i, j, img, log_l);
            inside.push(got.is_some());
            let st = got.unwrap_or(CellStats { mean: refc, spread: (f32::INFINITY, f32::INFINITY), change: 0.0 });
            feature.push(st.mean);
            spread.push(st.spread);
            change.push(st.change);
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
    // which cells a path may go on from: not those whose colour spreads past noise and the
    // typical cell in reach (a textured surface's cells all spread), straddling an edge of colour.
    // (Luminance alone is no such sign: a crease, a freckle and steep shading spread a cell as
    // much, and paths cross those, or jump them.)
    let median = |mut v: Vec<f32>| {
        v.retain(|x| x.is_finite());
        v.sort_by(f32::total_cmp);
        v.get(v.len() / 2).copied().unwrap_or(0.0)
    };
    let bc = BARRIER_CHROMA.max(2.0 * qc).max(BARRIER_TYPICAL * median(spread.iter().map(|s| s.1).collect()));
    let barrier: Vec<f32> = spread.iter().map(|&(_, sc)| BARRIER * masks_smooth(bc, 2.0 * bc, sc)).collect();
    let mut seeds = vec![false; n * m];
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
        let s0 = (step_cost(refc, feature[at]) - PATH_FULL).max(0.0);
        if let Some(sd) = seeds.get_mut(at) {
            *sd = true;
        }
        if s0 < cost[at] {
            cost[at] = s0;
            parts[at] = (0.0, s0);
            heap.push(Reverse((key(s0), at)));
        }
    }
    while let Some(Reverse((kc, at))) = heap.pop() {
        let c = cost[at];
        if key(c) < kc || c > PATH_NONE {
            continue;
        }
        let (pl, pc) = parts[at];
        // (a centre's own cell is where the path starts, whatever it straddles)
        let pc = if seeds[at] { pc } else { pc + barrier[at] };
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
        i0,
        j0,
        n,
        m,
        member,
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

/// The share of a radius an Auto Mask dab of process 9 paints fully before it fades (at least its
/// own hard core): what belongs to the object under it is painted solidly to its edges, the
/// feather only softening where the brush ends inside it.
const AUTO_HARD: f64 = 0.75;

/// What a process 9 Auto Mask stroke found to belong over a rectangle of [`Cells`]: for each cell,
/// the most any dab vouched for it ([`Grown::vouch`]). The most, so every dab's finding stands
/// whichever order they came in: a stroke painted live and the same stroke rendered at once find
/// the same.
#[derive(Clone, Debug, Default)]
struct CellMap {
    i0: usize,
    j0: usize,
    n: usize,
    m: usize,
    v: Vec<f32>,
}

impl CellMap {
    /// An empty map over the cells holding output pixels `r`.
    fn over(cells: &Cells, r: PixRect) -> CellMap {
        if r.is_empty() {
            return CellMap::default();
        }
        let (i0, j0) = cells.of_pixel(r.x0, r.y0);
        let (i1, j1) = cells.of_pixel(r.x1 - 1, r.y1 - 1);
        let (n, m) = (i1 + 1 - i0, j1 + 1 - j0);
        CellMap { i0, j0, n, m, v: vec![0.0; n * m] }
    }

    fn index(&self, i: usize, j: usize) -> Option<usize> {
        (i >= self.i0 && j >= self.j0 && i < self.i0 + self.n && j < self.j0 + self.m).then(|| (j - self.j0) * self.n + (i - self.i0))
    }

    /// Cell `(i, j)` (0 outside the map, and for a cell past the grid's edge).
    fn get(&self, i: i64, j: i64) -> f32 {
        if i < 0 || j < 0 {
            return 0.0;
        }
        self.index(i as usize, j as usize).and_then(|k| self.v.get(k)).copied().unwrap_or(0.0)
    }

    /// The same map over `to`'s cells (cells of both kept, the rest 0).
    fn moved(&self, to: CellMap) -> CellMap {
        let mut to = to;
        for j in 0..self.m {
            for i in 0..self.n {
                if let (Some(k), Some(v)) = (to.index(self.i0 + i, self.j0 + j), self.v.get(j * self.n + i))
                    && let Some(o) = to.v.get_mut(k)
                {
                    *o = *v;
                }
            }
        }
        to
    }

    /// Cell `(i, j)` firmed ([`MAP_FULL`]) with the holes filled ([`HOLE_CELLS`]): on each of
    /// eight lines through it, a sixteenth of a turn apart, the less of the most a cell up to
    /// `hole` away on either side holds; the cell takes the third least of the eight if more than
    /// its own (two lines may run along a gap). A speck, or a narrow line in any direction (a
    /// crease, a fold, the seam between two things painted) between cells that belong, is filled;
    /// a notch in the object's edge open a quarter turn or more, or the edge itself, is not.
    fn filled(&self, i: i64, j: i64, hole: usize) -> f32 {
        let firm = |i: i64, j: i64| masks_smooth(MAP_NONE, MAP_FULL, self.get(i, j));
        let v = firm(i, j);
        if hole == 0 || v >= 1.0 {
            return v;
        }
        let most = |(dx, dy): (f64, f64)| {
            (1..=hole).map(|t| firm(i + (dx * t as f64).round() as i64, j + (dy * t as f64).round() as i64)).fold(0f32, f32::max)
        };
        let mut lines = LINES.map(|(dx, dy)| most((dx, dy)).min(most((-dx, -dy))));
        lines.sort_by(f32::total_cmp);
        v.max(lines[2])
    }
}

/// The eight directions [`CellMap::filled`] looks along (unit steps, a sixteenth of a turn apart).
const LINES: [(f64, f64); 8] = [
    (1.0, 0.0),
    (0.923_879_5, 0.382_683_43),
    (std::f64::consts::FRAC_1_SQRT_2, std::f64::consts::FRAC_1_SQRT_2),
    (0.382_683_43, 0.923_879_5),
    (0.0, 1.0),
    (-0.382_683_43, 0.923_879_5),
    (-std::f64::consts::FRAC_1_SQRT_2, std::f64::consts::FRAC_1_SQRT_2),
    (-0.923_879_5, 0.382_683_43),
];

/// [`stamp`] for [`AutoMaskModel::Smoothed`]: each dab of `range` paints its feathered circle into
/// `foot` (over `area`, as a plain brush does but solid to [`AUTO_HARD`] of its radius) and adds
/// what it found to belong ([`grow`]) to `map`.
#[allow(clippy::too_many_arguments)]
fn stamp_smoothed(
    bd: &BrushDabs,
    range: Range<usize>,
    w: usize,
    h: usize,
    img: &Rgb32f,
    log_l: &Plane,
    cells: &Cells,
    foot: &mut [f32],
    map: &mut CellMap,
    area: PixRect,
) {
    if w == 0 || h == 0 || foot.len() < area.area() || img.data.len() != w * h || log_l.data.len() != w * h {
        return;
    }
    let mut cache = CellCache::default();
    let bw = area.width();
    for k in range {
        let Some(d) = bd.dabs.get(k).copied() else { break };
        let (r, hard, flow) = bd.dab(k);
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
                if let Some(v) = foot.get_mut((y - area.y0) * bw + (x - area.x0)) {
                    *v = accumulate(*v, dab_alpha(dd, r, hard), flow, bd.density);
                }
            }
        }
        let Some(refc) = reference(bd, k, cells, img, log_l) else { continue };
        let Some(g) = grow(bd, k, r, cells, img, log_l, refc, &mut cache) else { continue };
        for j in g.j0..g.j0 + g.m {
            for i in g.i0..g.i0 + g.n {
                let v = g.vouch(i, j);
                if let Some(o) = map.index(i, j).and_then(|q| map.v.get_mut(q)) {
                    *o = o.max(v);
                }
            }
        }
    }
}

/// The mean of `p` (`w × h`, row by row) over squares `r` px from each pixel, clipped to the plane.
fn box_mean(p: &[f32], w: usize, h: usize, r: usize) -> Vec<f32> {
    let n = w * h;
    if p.len() != n || n == 0 {
        return vec![0.0; n];
    }
    // across: each row's running sums
    let mut t = vec![0f32; n];
    let mut acc = vec![0f64; w + 1];
    for (row, out) in p.chunks_exact(w).zip(t.chunks_exact_mut(w)) {
        for (x, v) in row.iter().enumerate() {
            acc[x + 1] = acc[x] + f64::from(*v);
        }
        for (x, o) in out.iter_mut().enumerate() {
            let (a, b) = (x.saturating_sub(r), (x + r + 1).min(w));
            *o = ((acc[b] - acc[a]) / (b - a) as f64) as f32;
        }
    }
    // down: running sums of whole rows (row by row, as the plane lies in memory)
    let mut cum = vec![0f64; (h + 1) * w];
    for y in 0..h {
        for x in 0..w {
            cum[(y + 1) * w + x] = cum[y * w + x] + f64::from(t[y * w + x]);
        }
    }
    let mut o = vec![0f32; n];
    for y in 0..h {
        let (a, b) = (y.saturating_sub(r), (y + r + 1).min(h));
        let k = (b - a) as f64;
        for x in 0..w {
            o[y * w + x] = ((cum[b * w + x] - cum[a * w + x]) / k) as f32;
        }
    }
    o
}

/// The self-guided filter (He, Sun and Tang) of `p` (`w × h`) over squares `r` px from each pixel:
/// differences well under `eps.sqrt()` flattened, edges well over it kept.
fn smooth_self(p: &[f32], w: usize, h: usize, r: usize, eps: f32) -> Vec<f32> {
    let n = w * h;
    if p.len() != n {
        return p.to_vec();
    }
    // (about the plane's first value: the variance is a difference of squares, of small numbers
    // precise)
    let o = p.first().copied().unwrap_or(0.0);
    let q: Vec<f32> = p.iter().map(|v| v - o).collect();
    let m = box_mean(&q, w, h, r);
    let m2 = box_mean(&q.iter().map(|v| v * v).collect::<Vec<f32>>(), w, h, r);
    let (mut a, mut b) = (vec![0f32; n], vec![0f32; n]);
    for i in 0..n {
        let var = (m2[i] - m[i] * m[i]).max(0.0);
        let k = var / (var + eps);
        a[i] = k;
        b[i] = m[i] - k * m[i];
    }
    let (a, b) = (box_mean(&a, w, h, r), box_mean(&b, w, h, r));
    (0..n).map(|i| a[i] * q[i] + b[i] + o).collect()
}

/// The guided filter (He, Sun and Tang) of `p` steered by a guide of three channels `g`, all
/// `w × h`, over squares `r` px from each pixel, `eps` the regularization: within each window the
/// paint is taken as a linear function of the guide's colour, so where the window holds the
/// photo's edge the paint's edge falls on it, as soft as the photo's own, and where the paint is
/// flat it stays flat (no specks, no pinholes). Clamped to 0..1.
fn snap(g: [&[f32]; 3], p: &[f32], w: usize, h: usize, r: usize, eps: f32) -> Vec<f32> {
    let n = w * h;
    if g.iter().any(|c| c.len() != n) || p.len() != n {
        return vec![0.0; n];
    }
    use rayon::prelude::*;
    let prod = |a: &[f32], b: &[f32]| a.iter().zip(b).map(|(x, y)| x * y).collect::<Vec<f32>>();
    // the window means of the guide, the paint, their products and the guide's own (its
    // covariance: xx, xy, xz, yy, yz, zz), each apart
    let pairs = [(0, 0), (0, 1), (0, 2), (1, 1), (1, 2), (2, 2)];
    let mut inputs: Vec<Vec<f32>> = vec![g[0].to_vec(), g[1].to_vec(), g[2].to_vec(), p.to_vec()];
    inputs.extend((0..3).map(|c| prod(g[c], p)));
    inputs.extend(pairs.iter().map(|&(a, b)| prod(g[a], g[b])));
    let mut means: Vec<Vec<f32>> = inputs.par_iter().map(|v| box_mean(v, w, h, r)).collect();
    if means.len() != 13 {
        return vec![0.0; n];
    }
    let cg = means.split_off(7);
    let cp = means.split_off(4);
    let mp = means.pop().unwrap_or_default();
    let mg = means;
    let eps = f64::from(eps);
    let solve = |i: usize| -> [f32; 4] {
        // (in f64: the covariances are differences of squares, and the system near singular where
        // the window's colours lie along a line; f32 would let a stroke painted live and rendered
        // at once differ)
        let f = |v: &Vec<f32>| v.get(i).map_or(0.0, |x| f64::from(*x));
        let mu = [f(&mg[0]), f(&mg[1]), f(&mg[2])];
        let mpi = f(&mp);
        let c = |k: usize, a: usize, b: usize| f(&cg[k]) - mu[a] * mu[b];
        let (sxx, sxy, sxz, syy, syz, szz) = (c(0, 0, 0) + eps, c(1, 0, 1), c(2, 0, 2), c(3, 1, 1) + eps, c(4, 1, 2), c(5, 2, 2) + eps);
        let v = [f(&cp[0]) - mu[0] * mpi, f(&cp[1]) - mu[1] * mpi, f(&cp[2]) - mu[2] * mpi];
        // (the 3 × 3 system by its adjugate: positive definite, eps on its diagonal)
        let (ixx, ixy, ixz) = (syy * szz - syz * syz, sxz * syz - sxy * szz, sxy * syz - sxz * syy);
        let (iyy, iyz, izz) = (sxx * szz - sxz * sxz, sxy * sxz - sxx * syz, sxx * syy - sxy * sxy);
        let det = sxx * ixx + sxy * ixy + sxz * ixz;
        if !(det.is_finite() && det > 0.0) {
            return [0.0, 0.0, 0.0, mpi as f32];
        }
        let k = [
            (ixx * v[0] + ixy * v[1] + ixz * v[2]) / det,
            (ixy * v[0] + iyy * v[1] + iyz * v[2]) / det,
            (ixz * v[0] + iyz * v[1] + izz * v[2]) / det,
        ];
        [k[0] as f32, k[1] as f32, k[2] as f32, (mpi - k[0] * mu[0] - k[1] * mu[1] - k[2] * mu[2]) as f32]
    };
    let coef: Vec<[f32; 4]> = (0..n).into_par_iter().map(solve).collect();
    let part = |q: usize| coef.iter().map(|c| c[q]).collect::<Vec<f32>>();
    let (a0, a1, a2, b) = (part(0), part(1), part(2), part(3));
    let ab: Vec<Vec<f32>> = [a0, a1, a2, b].par_iter().map(|v| box_mean(v, w, h, r)).collect();
    let (Some(a0), Some(a1), Some(a2), Some(b)) = (ab.first(), ab.get(1), ab.get(2), ab.get(3)) else {
        return vec![0.0; n];
    };
    (0..n).map(|i| (a0[i] * g[0][i] + a1[i] * g[1][i] + a2[i] * g[2][i] + b[i]).clamp(0.0, 1.0)).collect()
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
    /// [`AutoMaskModel::Smoothed`]: the grid the dabs grow over (set by the first dab) and what
    /// they found to belong on it, over the cells of `bounds`.
    cells: Option<Cells>,
    map: CellMap,
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
            cells: None,
            map: CellMap::default(),
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
        (self.raw.len() + self.refined.len() + self.map.v.len()) * 4 + self.dabs.len() * 16 + self.pressure.len() * 4
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
            let k = Smoothing::of(brush.r);
            let cells = *self.cells.get_or_insert_with(|| Cells::of(bd.r, bd.dabs.first().copied(), w, h));
            // what the dabs found changes over the cells their circles reach (up to a cell past
            // `touched`), the coarse paint `hole` cells and one more (blending between cells'
            // centres) beyond, the guided filter's output twice its window beyond that; it reads
            // the coarse paint and the photo twice its window further again
            let out = touched.grow(cells.size * (k.hole + 2) + 2 * k.snap, w, h);
            self.cover(out);
            let bounds = self.bounds;
            stamp_smoothed(bd, new, w, h, img, log_l, &cells, &mut self.raw, &mut self.map, bounds);
            self.refine_smoothed(out, &cells, img, log_l, &k);
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
        for y in self.bounds.y0..self.bounds.y1 {
            let src = (y - self.bounds.y0) * bw;
            let dst = (y - next.y0) * nw + (self.bounds.x0 - next.x0);
            if let (Some(s), Some(d)) = (self.raw.get(src..src + bw), raw.get_mut(dst..dst + bw)) {
                d.copy_from_slice(s);
            }
            if let (Some(s), Some(d)) = (self.refined.get(src..src + bw), refined.get_mut(dst..dst + bw)) {
                d.copy_from_slice(s);
            }
        }
        self.bounds = next;
        self.raw = raw;
        self.refined = refined;
        if let Some(cells) = self.cells {
            self.map = self.map.moved(CellMap::over(&cells, next));
        }
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

    /// [`AutoMaskModel::Smoothed`]'s coverage over `out`: what the dabs found to belong, holes
    /// filled and blended between the cells' centres (a coarse paint whose edge lies within about
    /// a cell of the photo's), carried onto the photo's edges by the guided filter on the smoothed
    /// photo ([`snap`]), times the brush's feathered footprint.
    fn refine_smoothed(&mut self, out: PixRect, cells: &Cells, img: &Rgb32f, log_l: &Plane, k: &Smoothing) {
        let (w, h) = (self.w, self.h);
        let win = out.grow(2 * k.snap, w, h);
        let (ww, wh) = (win.width(), win.height());
        if ww == 0 || wh == 0 {
            return;
        }
        // the cells' findings, holes filled, over the cells the window's pixels blend between
        let cs = cells.size as f32;
        let (ox, oy) = (cells.ox as f32, cells.oy as f32);
        let at = |v: usize, o: f32| (v as f32 + 0.5 + o) / cs - 0.5;
        let (ci0, cj0) = (at(win.x0, ox).floor() as i64, at(win.y0, oy).floor() as i64);
        let (ci1, cj1) = (at(win.x1 - 1, ox).floor() as i64 + 1, at(win.y1 - 1, oy).floor() as i64 + 1);
        let (cn, cm) = ((ci1 - ci0 + 1) as usize, (cj1 - cj0 + 1) as usize);
        let (last_i, last_j) = (cells.cols() as i64 - 1, cells.rows() as i64 - 1);
        let mut fill = vec![0f32; cn * cm];
        for j in 0..cm {
            for i in 0..cn {
                // (past the grid's edge, the edge cell: the paint runs on to the frame's edge)
                let (ci, cj) = ((ci0 + i as i64).clamp(0, last_i.max(0)), (cj0 + j as i64).clamp(0, last_j.max(0)));
                fill[j * cn + i] = self.map.filled(ci, cj, k.hole);
            }
        }
        let mut coarse = vec![0f32; ww * wh];
        for y in 0..wh {
            let v = at(win.y0 + y, oy);
            let (j, fv) = (v.floor() as i64, v - v.floor());
            let jj = (j - cj0).clamp(0, cm as i64 - 2).max(0) as usize;
            for x in 0..ww {
                let u = at(win.x0 + x, ox);
                let (i, fu) = (u.floor() as i64, u - u.floor());
                let ii = (i - ci0).clamp(0, cn as i64 - 2).max(0) as usize;
                let c = |di: usize, dj: usize| fill.get((jj + dj).min(cm - 1) * cn + (ii + di).min(cn - 1)).copied().unwrap_or(0.0);
                coarse[y * ww + x] = (c(0, 0) * (1.0 - fu) + c(1, 0) * fu) * (1.0 - fv) + (c(0, 1) * (1.0 - fu) + c(1, 1) * fu) * fv;
            }
        }
        let sm = Smoothed::of(win, img, log_l, k);
        // (the guide taken about the photo under the stroke's first dab: the filter's variances
        // are differences of squares, and of small numbers they keep their precision, so a stroke
        // painted live and rendered at once agree)
        let at0 = self.dabs.first().map_or(0, |d| masks::dab_pixel(*d, w, h));
        let (l0, c0) = (log_l.data.get(at0).copied().unwrap_or(0.0), img.data.get(at0).map_or([0.0; 3], |c| chromaticity(*c)));
        let g0: Vec<f32> = sm.l.iter().map(|v| (v - l0) / SNAP_EV).collect();
        let g1: Vec<f32> = sm.c0.iter().map(|c| (c - c0[0]) / SNAP_CHROMA).collect();
        let g2: Vec<f32> = sm.c2.iter().map(|c| (c - c0[2]) / SNAP_CHROMA).collect();
        let q = snap([&g0, &g1, &g2], &coarse, ww, wh, k.snap, SNAP_EPS);
        let b = self.bounds;
        for y in out.y0..out.y1 {
            for x in out.x0..out.x1 {
                let i = (y - b.y0) * b.width() + (x - b.x0);
                let wi = (y - win.y0) * ww + (x - win.x0);
                if let (Some(f), Some(v), Some(o)) = (self.raw.get(i), q.get(wi), self.refined.get_mut(i)) {
                    *o = f * masks_smooth(SNAP_SOFT, 1.0 - SNAP_SOFT, *v);
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
