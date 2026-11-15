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
    /// Process 9: both compared on an edge-preserving smoothing of the photo (noise and fine
    /// texture flattened, edges kept), small holes in the paint closed, then the guided filter on
    /// the smoothed luminance. Fills noisy and textured surfaces solidly and smoothly.
    Smoothed,
}

impl AutoMaskModel {
    /// The model process `p` renders Auto Mask strokes with.
    pub fn for_process(p: craft_develop_model::Process) -> AutoMaskModel {
        if p >= craft_develop_model::Process::V9 { AutoMaskModel::Smoothed } else { AutoMaskModel::PerPixel }
    }
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
            close: (r / 8.0).round().max(1.0) as usize,
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
        // chromaticity smoothed by the same edge-aware windows, steered by the smoothed luminance
        let (cw, chh) = (cwin.width(), cwin.height());
        let guide = ls.crop(cwin.x0.saturating_sub(win.x0), cwin.y0.saturating_sub(win.y0), cw, chh);
        let ch = img.crop(cwin.x0, cwin.y0, cw, chh).map(chromaticity);
        let mut chs = Rgb32f::new(cw, chh);
        if (guide.width, guide.height) == (cw, chh) {
            for c in 0..3 {
                let plane = ch.map(|v| v[c]);
                let s = masks::guided_cross_unclamped(&guide, &plane, k.sigma, k.eps);
                for (o, v) in chs.data.iter_mut().zip(&s.data) {
                    o[c] = *v;
                }
            }
        }
        Smoothed { win, l: ls, cwin, ch: chs }
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

/// The reference colour of dab `k` of `bd`: the median, channel by channel, of the smoothed photo
/// at its centre and at the [`REFERENCE_DABS`] − 1 centres before it.
fn reference(bd: &BrushDabs, k: usize, w: usize, h: usize, sm: &Smoothed) -> Option<(f32, [f32; 3])> {
    let mut got: Vec<(f32, [f32; 3])> = Vec::with_capacity(REFERENCE_DABS);
    for j in k.saturating_sub(REFERENCE_DABS - 1)..=k {
        let c = dab_pixel(*bd.dabs.get(j)?, w, h);
        if let Some(v) = sm.at(c % w, c / w) {
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

/// [`stamp`] for [`AutoMaskModel::Smoothed`]: each dab's paint weighted by how like its reference
/// ([`reference`]) the smoothed photo is.
fn stamp_smoothed(bd: &BrushDabs, range: Range<usize>, w: usize, h: usize, sm: &Smoothed, buf: &mut [f32], area: PixRect) {
    if w == 0 || h == 0 || buf.len() < area.area() {
        return;
    }
    let bw = area.width();
    for k in range {
        let Some(d) = bd.dabs.get(k).copied() else { break };
        let (r, hard, flow) = bd.dab(k);
        let Some((rl, rch)) = reference(bd, k, w, h, sm) else { continue };
        let rect = dab_rect(d, r, w, h);
        let (x0, x1) = (rect.x0.max(area.x0), rect.x1.min(area.x1));
        let (y0, y1) = (rect.y0.max(area.y0), rect.y1.min(area.y1));
        for y in y0..y1 {
            for x in x0..x1 {
                let dd = Point::new(x as f64 + 0.5, y as f64 + 0.5).dist(d);
                if dd > r {
                    continue;
                }
                let Some((l, ch)) = sm.at(x, y) else { continue };
                let a = dab_alpha(dd, r, hard) * auto_similarity(l, ch, rl, rch);
                let v = &mut buf[(y - area.y0) * bw + (x - area.x0)];
                *v = accumulate(*v, a, flow, bd.density);
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
        (self.raw.len() + self.refined.len()) * 4 + self.dabs.len() * 16 + self.pressure.len() * 4
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
            stamp_smoothed(bd, new, w, h, &sm, &mut self.raw, bounds);
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
            if let (Some(s), Some(d)) = (self.raw.get(src..src + (x1 - x0)), p.data.get_mut(dst..dst + (x1 - x0))) {
                d.copy_from_slice(s);
            }
        }
        p
    }

    /// [`AutoMaskModel::Smoothed`]'s refinement over `out`: the paint closed (holes up to about
    /// twice `close` filled), then its gaps along edges filled by the guided filter on the
    /// smoothed luminance; never less than the paint.
    fn refine_smoothed(&mut self, out: PixRect, sm: &Smoothed, k: &Smoothing) {
        let win = sm.win;
        let p = self.raw_in(win);
        let closed = rank_filter(&rank_filter(&p, k.close, true), k.close, false);
        let q = masks::guided_cross(&sm.l, &closed, k.refine_sigma, k.refine_eps);
        let (b, ww) = (self.bounds, win.width());
        for y in out.y0..out.y1 {
            for x in out.x0..out.x1 {
                let i = (y - b.y0) * b.width() + (x - b.x0);
                let wi = (y - win.y0) * ww + (x - win.x0);
                if let (Some(c), Some(v), Some(f)) = (closed.data.get(wi), q.data.get(wi), self.refined.get_mut(i)) {
                    *f = c.max(*v);
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
mod quality_tests;
