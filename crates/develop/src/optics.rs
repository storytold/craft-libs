//! Lens corrections (Optics) and the geometric warp.
//!
//! [`Warp`] is the complete inverse map from the *transformed* image (what the user sees before the crop:
//! after lens correction and perspective) back to the source pixels, evaluated once per output pixel inside the
//! single geometry resample:
//!
//! transformed px → (inverse perspective) → lens-corrected px → (manual distortion, embedded DNG warp,
//! per-channel lateral CA scale) → source px
//!
//! plus a radial vignetting gain evaluated at the source position. Every parameter is relative (normalized radii,
//! dimensionless scales), so a 400 px preview and a full-size export are warped identically.
//!
//! Also here: automatic lateral chromatic aberration estimation ([`estimate_lateral_ca`]) and [`defringe`].
//! Profile corrections use lens data embedded in the files (DNG `WarpRectilinear` / `WarpRectilinear2` /
//! `FixVignetteRadial`, and the raw readers' equivalent of a camera's own correction, e.g. Panasonic / Leica RW2
//! distortion), or a lens database profile in the same form (`lightcraft_lensdb`).

use std::sync::Mutex;

use crate::color::perceptual::{oklab_from_2020, oklab_to_2020};
use crate::raster::{Plane, Rgb32f};
use craft_develop_model::{DevelopSettings, EmbeddedLens, EmbeddedVignette, EmbeddedWarp, EmbeddedWarpStage};
use craft_geom::{Affine, Homography, Interval, Orientation, Point, Real};

use crate::for_rows;

/// Manual "Distortion" ±100 → radial coefficient (r normalized to the half diagonal).
const DISTORTION_K: f64 = 0.2;
/// Manual vignetting ±100 → stops of gain at the corners.
const VIGNETTE_STOPS: f64 = 1.5;
/// Manual CA ±100 → radial scale difference of a colour plane.
const CA_SCALE: f64 = 0.003;

/// The inverse map transformed → source, in pixels of the (user-)oriented source image `w × h`.
#[derive(Clone, Debug, PartialEq)]
pub struct Warp {
    pub w: f64,
    pub h: f64,
    /// Forward perspective (lens-corrected → transformed), in centred coordinates `(p − centre) / (long edge / 2)`.
    pub persp: Homography,
    /// Its inverse (transformed → lens-corrected).
    pub persp_inv: Homography,
    /// Manual radial distortion: source radius = r·(1 + k1·r²), r relative to the half diagonal.
    pub k1: f64,
    /// Manual vignetting: gain = 2^(amount·r^power) stops at source radius r (half-diagonal units).
    pub vig_stops: f64,
    pub vig_power: f64,
    /// Lateral CA: per-channel radial scale about the image centre (R, G, B).
    pub ca: [f64; 3],
    /// Embedded DNG lens corrections (already in this oriented frame) and their strengths (1 = as recorded).
    pub lens: Option<EmbeddedLens>,
    pub lens_dist: f64,
    pub lens_vig: f64,
}

impl Warp {
    pub fn identity(w: f64, h: f64) -> Warp {
        Warp {
            w,
            h,
            persp: Homography::IDENTITY,
            persp_inv: Homography::IDENTITY,
            k1: 0.0,
            vig_stops: 0.0,
            vig_power: 4.0,
            ca: [0.0; 3],
            lens: None,
            lens_dist: 0.0,
            lens_vig: 0.0,
        }
    }

    /// The lens part of the warp from the settings (optics section) and optional embedded lens data.
    pub fn from_settings(w: f64, h: f64, s: &DevelopSettings, lens: Option<&EmbeddedLens>) -> Warp {
        let mut wp = Warp::identity(w, h);
        if !s.section_enabled("optics") {
            return wp;
        }
        let o = &s.optics;
        wp.k1 = -DISTORTION_K * o.distortion / 100.0;
        wp.vig_stops = VIGNETTE_STOPS * o.vignetting / 100.0;
        wp.vig_power = 1.0 + 6.0 * (o.vignetting_midpoint / 100.0).clamp(0.0, 1.0);
        wp.ca = [CA_SCALE * o.ca_red / 100.0, 0.0, CA_SCALE * o.ca_blue / 100.0];
        if o.lens_profile
            && let Some(l) = lens
        {
            wp.lens = Some(*l);
            wp.lens_dist = (o.profile_distortion / 100.0).clamp(0.0, 2.0);
            wp.lens_vig = (o.profile_vignetting / 100.0).clamp(0.0, 2.0);
        }
        wp
    }

    fn half_long(&self) -> f64 {
        self.w.max(self.h) / 2.0
    }

    fn centre(&self) -> Point {
        Point::new(self.w / 2.0, self.h / 2.0)
    }

    /// Whether pixels move at all.
    pub fn moves_pixels(&self) -> bool {
        self.persp != Homography::IDENTITY
            || self.k1 != 0.0
            || self.ca.iter().any(|c| *c != 0.0)
            || (self.lens_dist != 0.0 && self.lens.is_some_and(|l| l.warp.is_some() || l.has_stages()))
    }

    /// Whether the embedded lens warp is a `WarpRectilinear2` chain, whose degree-14 polynomials are ill-conditioned
    /// in `f32` (terms of 1e5 cancelling to 1): a renderer that works in `f32` must sample on the CPU.
    pub fn needs_f64(&self) -> bool {
        self.lens_dist != 0.0 && self.lens.is_some_and(|l| l.has_stages())
    }

    /// Whether the colour planes are sampled at different positions.
    pub fn per_channel(&self) -> bool {
        self.ca[0] != self.ca[1]
            || self.ca[1] != self.ca[2]
            || (self.lens_dist != 0.0 && self.lens.and_then(|l| l.warp).is_some_and(|w| w.planes[0] != w.planes[1] || w.planes[1] != w.planes[2]))
            || (self.lens_dist != 0.0
                && self.lens.is_some_and(|l| l.stages.iter().flatten().any(|s| s.planes[0] != s.planes[1] || s.planes[1] != s.planes[2])))
    }

    pub fn has_gain(&self) -> bool {
        self.vig_stops != 0.0 || (self.lens_vig != 0.0 && self.lens.is_some_and(|l| l.vignette.is_some()))
    }

    pub fn is_identity(&self) -> bool {
        !self.moves_pixels() && !self.has_gain()
    }

    /// Transformed px → lens-corrected px (inverse perspective).
    pub fn to_corrected(&self, p: Point) -> Point {
        let (x, y) = self.to_corrected_real(p.x, p.y);
        Point::new(x, y)
    }

    /// [`Warp::to_corrected`] for any [`Real`].
    fn to_corrected_real<T: Real>(&self, x: T, y: T) -> (T, T) {
        if self.persp == Homography::IDENTITY {
            return (x, y);
        }
        let (c, l) = (self.centre(), self.half_long());
        let (qx, qy) = self.persp_inv.apply_real((x - c.x) / l, (y - c.y) / l);
        (qx * l + c.x, qy * l + c.y)
    }

    /// Lens-corrected px → transformed px (forward perspective).
    pub fn from_corrected(&self, p: Point) -> Point {
        if self.persp == Homography::IDENTITY {
            return p;
        }
        let (c, l) = (self.centre(), self.half_long());
        let q = self.persp.apply(Point::new((p.x - c.x) / l, (p.y - c.y) / l));
        Point::new(c.x + q.x * l, c.y + q.y * l)
    }

    /// Lens-corrected px → source px for colour plane `ch` (0 = R, 1 = G, 2 = B).
    pub fn corrected_to_source(&self, p: Point, ch: usize) -> Point {
        let (x, y) = self.corrected_to_source_real(p.x, p.y, ch);
        Point::new(x, y)
    }

    /// [`Warp::corrected_to_source`] for any [`Real`].
    fn corrected_to_source_real<T: Real>(&self, x: T, y: T, ch: usize) -> (T, T) {
        let c = self.centre();
        let hd = (self.w.hypot(self.h) / 2.0).max(1e-9);
        let (mut qx, mut qy) = (x, y);
        if self.k1 != 0.0 {
            let (dx, dy) = (qx - c.x, qy - c.y);
            let r2 = (dx * dx + dy * dy) / (hd * hd);
            let f = r2 * self.k1 + 1.0;
            (qx, qy) = (dx * f + c.x, dy * f + c.y);
        }
        if self.lens_dist != 0.0
            && let Some(wp) = self.lens.and_then(|l| l.warp)
        {
            let (sx, sy) = embedded_warp_real(&wp, qx, qy, ch, self.w, self.h);
            (qx, qy) = (qx + (sx - qx) * self.lens_dist, qy + (sy - qy) * self.lens_dist);
        }
        if self.lens_dist != 0.0
            && let Some(l) = self.lens.filter(|l| l.has_stages())
        {
            // each opcode warps the image the one before it made, so the last one maps the output position first
            let (mut sx, mut sy) = (qx, qy);
            for st in l.stages.iter().rev().flatten() {
                (sx, sy) = embedded_stage_real(st, sx, sy, ch, self.w, self.h);
            }
            (qx, qy) = (qx + (sx - qx) * self.lens_dist, qy + (sy - qy) * self.lens_dist);
        }
        let k = self.ca[ch];
        if k != 0.0 {
            (qx, qy) = ((qx - c.x) * (1.0 + k) + c.x, (qy - c.y) * (1.0 + k) + c.y);
        }
        (qx, qy)
    }

    /// Transformed px → source px for plane `ch`.
    pub fn to_source(&self, p: Point, ch: usize) -> Point {
        self.corrected_to_source(self.to_corrected(p), ch)
    }

    /// Source px → transformed px (green plane): the inverse of [`Warp::to_source`], by Newton's
    /// method on it (the warps are smooth and close to the identity, so a few steps reach a
    /// hundredth of a pixel). `None` when it doesn't converge (a point far outside a strong warp).
    pub fn from_source(&self, s: Point) -> Option<Point> {
        if !(s.x.is_finite() && s.y.is_finite()) {
            return None;
        }
        let f = |p: Point| self.to_source(p, 1);
        // start from where the perspective alone puts it (the lens part moves pixels little)
        let mut t = self.from_corrected(s);
        let h = 1e-3 * self.w.max(self.h).max(1.0);
        for _ in 0..24 {
            let v = f(t);
            let (ex, ey) = (v.x - s.x, v.y - s.y);
            if !(ex.is_finite() && ey.is_finite()) {
                return None;
            }
            if ex.abs() < 1e-4 && ey.abs() < 1e-4 {
                return Some(t);
            }
            let (dx, dy) = (f(Point::new(t.x + h, t.y)), f(Point::new(t.x, t.y + h)));
            let (a, c) = ((dx.x - v.x) / h, (dx.y - v.y) / h);
            let (b, d) = ((dy.x - v.x) / h, (dy.y - v.y) / h);
            let det = a * d - b * c;
            if !det.is_finite() || det.abs() < 1e-12 {
                return None;
            }
            t = Point::new(t.x - (d * ex - b * ey) / det, t.y - (a * ey - c * ex) / det);
        }
        let v = f(t);
        ((v.x - s.x).abs() < 1e-2 && (v.y - s.y).abs() < 1e-2).then_some(t)
    }

    /// Vignetting compensation gain at source position `src` (px).
    pub fn gain(&self, src: Point) -> f32 {
        let mut g = 1.0;
        if self.vig_stops != 0.0 {
            let hd = (self.w.hypot(self.h) / 2.0).max(1e-9);
            let r = (src.dist(self.centre()) / hd).min(1.5);
            g *= (self.vig_stops * r.powf(self.vig_power)).exp2();
        }
        if self.lens_vig != 0.0
            && let Some(v) = self.lens.and_then(|l| l.vignette)
        {
            g *= 1.0 + (embedded_vignette_gain(&v, src, self.w, self.h) - 1.0) * self.lens_vig;
        }
        g as f32
    }

    /// Whether a source position lies on the image (within [`edges`](Self::edges)).
    fn inside(&self, s: Point) -> bool {
        let (lo, hx, hy) = self.edges();
        s.x >= lo && s.y >= lo && s.x <= hx && s.y <= hy
    }

    /// The source area that frames output pixels, `lo..=hx × lo..=hy`: the image with a half-pixel margin.
    fn edges(&self) -> (f64, f64, f64) {
        (-0.5, self.w + 0.5, self.h + 0.5)
    }

    /// The framing chain of output position `(x, y)` (`o2t`: output px → transformed px): its lens-corrected
    /// position and its green-plane source position.
    fn framing<T: Real>(&self, o2t: &Affine, x: T, y: T) -> ((T, T), (T, T)) {
        let (tx, ty) = o2t.apply_real(x, y);
        let (cx, cy) = self.to_corrected_real(tx, ty);
        ((cx, cy), self.corrected_to_source_real(cx, cy, 1))
    }

    /// The reference framing of output pixel `(x, y)`: its lens-corrected and green-plane source positions if
    /// the latter is [`inside`](Self::inside) the image, else `None` (the pixel is blank canvas).
    pub fn frame(&self, o2t: &Affine, x: usize, y: usize) -> Option<(Point, Point)> {
        let ((cx, cy), (gx, gy)) = self.framing(o2t, x as f64 + 0.5, y as f64 + 0.5);
        let g = Point::new(gx, gy);
        self.inside(g).then(|| (Point::new(cx, cy), g))
    }

    /// Whether output pixel `(x, y)` shows the image ([`frame`](Self::frame) is `Some`).
    pub fn covers(&self, o2t: &Affine, x: usize, y: usize) -> bool {
        self.frame(o2t, x, y).is_some()
    }

    /// [`covers`](Self::covers) for every pixel of the output block `x0..x1 × y0..y1` at once, when that is
    /// certain: `Some(true)` if all are covered, `Some(false)` if none is, `None` if the block may straddle the
    /// image edge (or can't be bounded) and each pixel must be decided alone.
    ///
    /// Why the answer equals the per-pixel decision: the block's pixel centres `x + 0.5` are exact `f64` values in
    /// `[x0 + 0.5, x1 − 0.5]` (likewise y). [`framing`](Self::framing) evaluated with [`Interval`]s runs the very
    /// same operations as its `f64` evaluation in [`frame`](Self::frame), each rounded outward, so its result
    /// bounds the `f64` source position of every pixel of the block — the rounded values, not only the exact
    /// ones (see [`Interval`]). Divisions by a range that may contain zero, the homography's denominator clamp
    /// and overflow make the bounds undecided (NaN), and NaN fails every test below. So when the bounds lie
    /// within [`edges`](Self::edges) every pixel's position does, and when they lie beyond one edge every
    /// pixel's position does: the same `f64` comparisons [`inside`](Self::inside) makes, no margin needed.
    pub fn block_coverage(&self, o2t: &Affine, x0: usize, x1: usize, y0: usize, y1: usize) -> Option<bool> {
        if x1 <= x0 || y1 <= y0 {
            return None;
        }
        let px = Interval::new(x0 as f64 + 0.5, (x1 - 1) as f64 + 0.5);
        let py = Interval::new(y0 as f64 + 0.5, (y1 - 1) as f64 + 0.5);
        let (_, (sx, sy)) = self.framing(o2t, px, py);
        let (lo, hx, hy) = self.edges();
        if sx.lo() >= lo && sy.lo() >= lo && sx.hi() <= hx && sy.hi() <= hy {
            Some(true)
        } else if sx.hi() < lo || sy.hi() < lo || sx.lo() > hx || sy.lo() > hy {
            Some(false)
        } else {
            None
        }
    }
}

/// DNG `WarpRectilinear` source position for corrected position `p` (px) and plane `ch`.
pub fn embedded_warp(wp: &EmbeddedWarp, p: Point, ch: usize, w: f64, h: f64) -> Point {
    let (x, y) = embedded_warp_real(wp, p.x, p.y, ch, w, h);
    Point::new(x, y)
}

/// [`embedded_warp`] for any [`Real`].
fn embedded_warp_real<T: Real>(wp: &EmbeddedWarp, x: T, y: T, ch: usize, w: f64, h: f64) -> (T, T) {
    let (cx, cy) = (wp.center.x * w, wp.center.y * h);
    let m = (wp.radius * w.max(h)).max(1e-9);
    let [kr0, kr1, kr2, kr3, kt0, kt1] = wp.planes[ch.min(2)];
    let dx = (x - cx) / m;
    let dy = (y - cy) / m;
    let r2 = dx * dx + dy * dy;
    let f = (((r2 * kr3 + kr2) * r2 + kr1) * r2) + kr0;
    let sx = dx * f + dx * (kt0 * 2.0) * dy + (r2 + dx * 2.0 * dx) * kt1;
    let sy = dy * f + dx * (kt1 * 2.0) * dy + (r2 + dy * 2.0 * dy) * kt0;
    (sx * m + cx, sy * m + cy)
}

/// DNG `WarpRectilinear2` source position for corrected position `p` (px) and plane `ch`.
pub fn embedded_stage(st: &EmbeddedWarpStage, p: Point, ch: usize, w: f64, h: f64) -> Point {
    let (x, y) = embedded_stage_real(st, p.x, p.y, ch, w, h);
    Point::new(x, y)
}

/// [`embedded_stage`] for any [`Real`]: DNG specification 1.6.0.0, "WarpRectilinear2" — the radial term is
/// `f(clamp(r, min_valid_radius, max_valid_radius))` with `f(r) = Σ krₚ rᵖ` (p = 0..14), used as `d·f` or, with
/// `reciprocalRadial`, `d/f`; the tangential terms are those of `WarpRectilinear` (with the unclamped `r`).
fn embedded_stage_real<T: Real>(st: &EmbeddedWarpStage, x: T, y: T, ch: usize, w: f64, h: f64) -> (T, T) {
    let (cx, cy) = (st.center.x * w, st.center.y * h);
    let m = (st.radius * w.max(h)).max(1e-9);
    let k = &st.planes[ch.min(2)];
    let dx = (x - cx) / m;
    let dy = (y - cy) / m;
    let r2 = dx * dx + dy * dy;
    let r = r2.sqrt_nonneg().clamp_range(k[17], k[18]);
    let mut f = r * k[14] + k[13];
    for &c in k[..13].iter().rev() {
        f = f * r + c;
    }
    let (rx, ry) = if st.reciprocal { (dx / f, dy / f) } else { (dx * f, dy * f) };
    let sx = rx + dx * (k[15] * 2.0) * dy + (r2 + dx * 2.0 * dx) * k[16];
    let sy = ry + dx * (k[16] * 2.0) * dy + (r2 + dy * 2.0 * dy) * k[15];
    (sx * m + cx, sy * m + cy)
}

/// DNG `FixVignetteRadial` gain at source position `p` (px).
pub fn embedded_vignette_gain(v: &EmbeddedVignette, p: Point, w: f64, h: f64) -> f64 {
    let (cx, cy) = (v.center.x * w, v.center.y * h);
    let m = (v.radius * w.max(h)).max(1e-9);
    let r2 = ((p.x - cx).powi(2) + (p.y - cy).powi(2)) / (m * m);
    let mut g = 1.0;
    let mut rp = r2;
    for k in v.k {
        g += k * rp;
        rp *= r2;
    }
    g
}

/// Re-express embedded lens data (relative to a `w × h` image) for the image after orientation `o`.
pub fn reorient_lens(lens: &EmbeddedLens, o: Orientation, w: f64, h: f64) -> EmbeddedLens {
    if o == Orientation::Normal {
        return *lens;
    }
    let (ow, oh) = if o.swaps_axes() { (h, w) } else { (w, h) };
    let long_ratio = w.max(h) / ow.max(oh); // 1 (rotations keep the long edge)
    let map_pt = |c: Point| {
        let (x, y) = o.map(c.x * w, c.y * h, w, h);
        Point::new(x / ow, y / oh)
    };
    let warp = lens.warp.map(|wp| {
        let c = Point::new(wp.center.x * w, wp.center.y * h);
        let c2 = o.map(c.x, c.y, w, h);
        let planes = wp.planes.map(|[a, b, cc, d, kt0, kt1]| {
            // tangential coefficients form the vector (kt1, kt0), which rotates/reflects with the image
            let e = o.map(c.x + kt1, c.y + kt0, w, h);
            [a, b, cc, d, e.1 - c2.1, e.0 - c2.0]
        });
        EmbeddedWarp { planes, center: map_pt(wp.center), radius: wp.radius * long_ratio }
    });
    let vignette = lens.vignette.map(|v| EmbeddedVignette { k: v.k, center: map_pt(v.center), radius: v.radius * long_ratio });
    let stages = lens.stages.map(|st| {
        st.map(|st| {
            let c = Point::new(st.center.x * w, st.center.y * h);
            let c2 = o.map(c.x, c.y, w, h);
            let planes = st.planes.map(|mut k| {
                // same rotation of the tangential vector as for `WarpRectilinear`
                let e = o.map(c.x + k[16], c.y + k[15], w, h);
                (k[15], k[16]) = (e.1 - c2.1, e.0 - c2.0);
                k
            });
            EmbeddedWarpStage { planes, center: map_pt(st.center), radius: st.radius * long_ratio, reciprocal: st.reciprocal }
        })
    });
    EmbeddedLens { warp, vignette, stages, source: lens.source }
}

// ------------------------------------------------------------------------------------------ lateral CA

/// Estimate lateral chromatic aberration: the radial magnification of the red and blue planes relative to green
/// (`[α_R, α_B]`, dimensionless; red edges sit at `r·(1 + α_R)`). Strong, radially oriented green edges are
/// located in each plane with sub-pixel 1-D matching of normalized edge profiles; a weighted, outlier-trimmed
/// least-squares fit of displacement = α·r gives the scale. Results are cached per image.
pub fn estimate_lateral_ca(img: &Rgb32f) -> [f64; 2] {
    static CACHE: Mutex<Vec<(u64, [f64; 2])>> = Mutex::new(Vec::new());
    let key = fingerprint(img);
    if let Ok(c) = CACHE.lock()
        && let Some((_, v)) = c.iter().find(|(k, _)| *k == key)
    {
        return *v;
    }
    let v = estimate_lateral_ca_uncached(img);
    if let Ok(mut c) = CACHE.lock() {
        c.push((key, v));
        if c.len() > 16 {
            c.remove(0);
        }
    }
    v
}

/// Cheap content fingerprint of an image (dimensions + 512 sampled pixels), for analysis caches.
pub(crate) fn fingerprint(img: &Rgb32f) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325 ^ ((img.width as u64) << 32 | img.height as u64);
    let n = img.data.len();
    for i in 0..512usize {
        // (an empty image has no pixels to sample: its dimensions are the fingerprint)
        let Some(p) = img.data.get((i.wrapping_mul(2_654_435_761) ^ i) % n.max(1)) else { break };
        for c in *p {
            h ^= c.to_bits() as u64;
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    }
    h
}

fn estimate_lateral_ca_uncached(img: &Rgb32f) -> [f64; 2] {
    let (w, h) = (img.width, img.height);
    if w < 32 || h < 32 {
        return [0.0; 2];
    }
    let plane = |c: usize| -> Plane { img.map(|p| p[c]) };
    let planes = [plane(0), plane(1), plane(2)];
    let g = &planes[1];
    let (cx, cy) = (w as f64 / 2.0, h as f64 / 2.0);
    let hd = (w as f64).hypot(h as f64) / 2.0;
    // candidate edge points, best per grid cell (spread over the frame)
    const CELLS: usize = 24;
    const PER_CELL: usize = 12;
    let mut cells: Vec<Vec<(f32, usize, usize)>> = vec![Vec::new(); CELLS * CELLS];
    let at = |x: usize, y: usize| g.data[y * w + x];
    for y in (6..h - 6).step_by(2) {
        for x in (6..w - 6).step_by(2) {
            let gx = at(x + 1, y) - at(x - 1, y);
            let gy = at(x, y + 1) - at(x, y - 1);
            let mag = gx.hypot(gy);
            let mean = (at(x + 1, y) + at(x - 1, y) + at(x, y + 1) + at(x, y - 1)) * 0.25 + 0.02;
            let rel = mag / mean;
            if rel < 0.2 {
                continue;
            }
            let (vx, vy) = (x as f64 - cx, y as f64 - cy);
            let rr = vx.hypot(vy);
            if rr < 0.2 * hd {
                continue;
            }
            let cos = ((gx as f64 * vx + gy as f64 * vy) / (mag as f64 * rr)).abs();
            if cos < 0.85 {
                continue;
            }
            let cell = (y * CELLS / h) * CELLS + x * CELLS / w;
            let v = &mut cells[cell];
            if v.len() < PER_CELL {
                v.push((rel, x, y));
            } else if let Some(min) = v.iter_mut().min_by(|a, b| a.0.total_cmp(&b.0))
                && min.0 < rel
            {
                *min = (rel, x, y);
            }
        }
    }
    let pts: Vec<(f32, usize, usize)> = cells.into_iter().flatten().collect();
    // sub-pixel displacement of R and B against G along the radial direction
    const HALF: i32 = 4; // profile half length (px)
    const SUB: i32 = 20; // samples per px
    const MAXD: i32 = 3; // max displacement (px)
    let sample = |p: &Plane, x: f64, y: f64| -> f32 {
        let fx = x.clamp(0.0, (w - 1) as f64);
        let fy = y.clamp(0.0, (h - 1) as f64);
        let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
        let (tx, ty) = ((fx - x0 as f64) as f32, (fy - y0 as f64) as f32);
        let a = p.data[y0 * w + x0] + (p.data[y0 * w + x1] - p.data[y0 * w + x0]) * tx;
        let b = p.data[y1 * w + x0] + (p.data[y1 * w + x1] - p.data[y1 * w + x0]) * tx;
        a + (b - a) * ty
    };
    let znorm = |v: &mut [f32]| -> bool {
        let n = v.len() as f32;
        let m = v.iter().sum::<f32>() / n;
        let sd = (v.iter().map(|x| (x - m) * (x - m)).sum::<f32>() / n).sqrt();
        if sd < 1e-5 {
            return false;
        }
        v.iter_mut().for_each(|x| *x = (*x - m) / sd);
        true
    };
    let mut obs: [Vec<(f64, f64, f64)>; 2] = [Vec::new(), Vec::new()];
    for &(wt, x, y) in &pts {
        let (vx, vy) = (x as f64 - cx, y as f64 - cy);
        let rr = vx.hypot(vy);
        let (ux, uy) = (vx / rr, vy / rr);
        let (px, py) = (x as f64, y as f64);
        let mut gp: Vec<f32> = (-HALF..=HALF).map(|t| sample(g, px + ux * t as f64, py + uy * t as f64)).collect();
        if !znorm(&mut gp) {
            continue;
        }
        for (k, ch) in [0usize, 2].into_iter().enumerate() {
            let reach = HALF + MAXD;
            let fine: Vec<f32> =
                (-reach * SUB..=reach * SUB).map(|i| sample(&planes[ch], px + ux * i as f64 / SUB as f64, py + uy * i as f64 / SUB as f64)).collect();
            let mut best = (f32::MAX, 0i32);
            for d in -MAXD * SUB..=MAXD * SUB {
                let mut prof: Vec<f32> = (-HALF..=HALF).map(|t| fine[((t + reach) * SUB + d) as usize]).collect();
                if !znorm(&mut prof) {
                    continue;
                }
                let ssd: f32 = prof.iter().zip(&gp).map(|(a, b)| (a - b) * (a - b)).sum();
                if ssd < best.0 {
                    best = (ssd, d);
                }
            }
            // normalized SSD = 2n(1 − corr): require a good match
            let n = (2 * HALF + 1) as f32;
            if best.0 < 0.3 * n && best.1.abs() < MAXD * SUB {
                obs[k].push((rr, best.1 as f64 / SUB as f64, wt as f64));
            }
        }
    }
    let fit = |o: &[(f64, f64, f64)]| -> f64 {
        if o.len() < 12 {
            return 0.0;
        }
        let solve = |o: &[&(f64, f64, f64)]| {
            let (num, den) = o.iter().fold((0.0, 0.0), |(n, d), (r, dd, wt)| (n + wt * dd * r, d + wt * r * r));
            if den > 0.0 { num / den } else { 0.0 }
        };
        let all: Vec<&(f64, f64, f64)> = o.iter().collect();
        let mut a = solve(&all);
        for _ in 0..2 {
            let mut res: Vec<f64> = o.iter().map(|(r, d, _)| (d - a * r).abs()).collect();
            res.sort_by(f64::total_cmp);
            let mad = res[res.len() / 2].max(0.05);
            let keep: Vec<&(f64, f64, f64)> = o.iter().filter(|(r, d, _)| (d - a * r).abs() <= 2.5 * mad).collect();
            if keep.len() < 8 {
                break;
            }
            a = solve(&keep);
        }
        a
    };
    [fit(&obs[0]), fit(&obs[1])]
}

// ------------------------------------------------------------------------------------------ defringe

/// OkLab hue (degrees) spanned by the purple hue slider (0..100) and the green one.
const PURPLE_HUES: (f32, f32) = (240.0, 360.0);
const GREEN_HUES: (f32, f32) = (90.0, 210.0);

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Membership of hue `h` (deg) in `[lo, hi]` (deg) with soft shoulders.
fn hue_weight(h: f32, lo: f32, hi: f32) -> f32 {
    let feather = 8.0;
    let mid = (lo + hi) / 2.0;
    let half = (hi - lo) / 2.0;
    let mut d = (h - mid).abs() % 360.0;
    if d > 180.0 {
        d = 360.0 - d;
    }
    1.0 - smoothstep(half, half + feather, d)
}

/// Long edge (px) at which defringe widths are specified: fringes are 1–7 px wide on a ~4000 px image.
pub const DEFRINGE_REF_LONG: f64 = 4000.0;

/// Defringe's resolved parameters (the CPU stage below and the GPU kernels both read these).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DefringeParams {
    /// Radius (output px) of the lightness min / max filters that find edges.
    pub radius: usize,
    /// Purple and green OkLab hue ranges (degrees, lo ≤ hi) and strengths (0..1).
    pub purple: (f32, f32),
    pub green: (f32, f32),
    pub purple_strength: f32,
    pub green_strength: f32,
}

/// Defringe's parameters for `s` at `scale` output pixels per reference pixel, or `None` when it
/// does nothing.
pub fn defringe_params(s: &DevelopSettings, scale: f64) -> Option<DefringeParams> {
    let o = &s.optics;
    if !s.section_enabled("optics") || (o.defringe_purple_amount <= 0.0 && o.defringe_green_amount <= 0.0) {
        return None;
    }
    // as far as the sliders go (0–20): an amount from a damaged catalog or sidecar made the
    // radius overflow
    let max_amt = o.defringe_purple_amount.max(o.defringe_green_amount).min(20.0);
    // fringe width in source px grows with the amount (1..7 px), in output px at least one
    let radius = ((1.0 + max_amt * 0.3) * scale).round().max(1.0);
    // (an absurd scale must not ask for a filter wider than any image; the stages clamp it to theirs)
    let radius = if radius.is_finite() { radius.min(1e6) as usize } else { 1 };
    let range = |lo: f64, hi: f64, span: (f32, f32)| {
        let a = span.0 + (span.1 - span.0) * (lo / 100.0) as f32;
        let b = span.0 + (span.1 - span.0) * (hi / 100.0) as f32;
        (a.min(b), a.max(b))
    };
    Some(DefringeParams {
        radius,
        purple: range(o.defringe_purple_hue_lo, o.defringe_purple_hue_hi, PURPLE_HUES),
        green: range(o.defringe_green_hue_lo, o.defringe_green_hue_hi, GREEN_HUES),
        purple_strength: (o.defringe_purple_amount / 8.0).min(1.0) as f32,
        green_strength: (o.defringe_green_amount / 8.0).min(1.0) as f32,
    })
}

/// Defringe: desaturate purple / green colour fringes that sit next to high-contrast edges. `scale` is output
/// pixels per reference pixel (output long edge / [`DEFRINGE_REF_LONG`]) so the fringe width is the same
/// fraction of the image at any size.
pub fn defringe(img: &mut Rgb32f, s: &DevelopSettings, scale: f64) {
    let Some(DefringeParams { radius, purple: pr, green: gr, purple_strength: ps, green_strength: gs }) = defringe_params(s, scale) else {
        return;
    };
    let (w, h) = (img.width, img.height);
    // never wider than the picture
    let radius = radius.clamp(1, w.max(h).max(1));
    // perceptual lightness and its local range (dilated contrast)
    let lab: Vec<[f32; 3]> = img.data.iter().map(|p| oklab_from_2020([p[0].max(0.0), p[1].max(0.0), p[2].max(0.0)])).collect();
    let l = Plane { width: w, height: h, data: lab.iter().map(|p| p[0]).collect() };
    let (lo, hi) = (min_filter(&l, radius, false), min_filter(&l, radius, true));
    for_rows(&mut img.data, w, |y, row| {
        for (x, px) in row.iter_mut().enumerate() {
            let i = y * w + x;
            let edge = smoothstep(0.08, 0.25, hi.data[i] - lo.data[i]);
            if edge <= 0.0 {
                continue;
            }
            let [ll, a, b] = lab[i];
            let c = a.hypot(b);
            let chroma_w = smoothstep(0.015, 0.05, c);
            if chroma_w <= 0.0 {
                continue;
            }
            let hue = b.atan2(a).to_degrees().rem_euclid(360.0);
            let k = (ps * hue_weight(hue, pr.0, pr.1)).max(gs * hue_weight(hue, gr.0, gr.1)) * edge * chroma_w;
            if k <= 0.0 {
                continue;
            }
            let f = 1.0 - k.min(1.0);
            *px = oklab_to_2020([ll, a * f, b * f]).map(|v| v.max(0.0));
        }
    });
}

/// Separable square min (or max) filter of radius `r`, linear in the pixels whatever `r` is (a
/// crop can make the radius as wide as the output, and a scan of every neighbourhood was then
/// cubic in the side).
fn min_filter(p: &Plane, r: usize, max: bool) -> Plane {
    let (w, h) = (p.width, p.height);
    let mut tmp = Plane::new(w, h);
    for_rows(&mut tmp.data, w, |y, row| extreme_1d(&p.data[y * w..(y + 1) * w], r, max, row));
    // the vertical pass runs along the rows of the transposed plane
    let mut t = vec![0f32; w * h];
    for (i, v) in tmp.data.iter().enumerate() {
        t[(i % w) * h + i / w] = *v;
    }
    // each buffer goes once the next is built, so no more than three planes are alive at once
    drop(tmp);
    let mut tt = vec![0f32; w * h];
    for_rows(&mut tt, h, |x, col| extreme_1d(&t[x * h..(x + 1) * h], r, max, col));
    drop(t);
    let mut out = Plane::new(w, h);
    for (i, v) in tt.iter().enumerate() {
        out.data[(i % h) * w + i / h] = *v;
    }
    out
}

/// `out[x]` = the min (or max) of `src[x - r ..= x + r]` clipped to the slice, by the van Herk /
/// Gil–Werman blocks: three passes over the line however wide the window.
fn extreme_1d(src: &[f32], r: usize, max: bool, out: &mut [f32]) {
    let n = src.len();
    if n == 0 {
        return;
    }
    // past the line's length a wider window takes in nothing more
    let r = r.min(n);
    let id = if max { f32::NEG_INFINITY } else { f32::INFINITY };
    let pick = |a: f32, b: f32| if max { a.max(b) } else { a.min(b) };
    let (k, m) = (2 * r + 1, n + 2 * r);
    let pad = |i: usize| if i < r || i >= r + n { id } else { src[i - r] };
    let mut g = vec![id; m];
    let mut hh = vec![id; m];
    for i in 0..m {
        g[i] = if i % k == 0 { pad(i) } else { pick(g[i - 1], pad(i)) };
    }
    for i in (0..m).rev() {
        hh[i] = if i % k == k - 1 || i == m - 1 { pad(i) } else { pick(hh[i + 1], pad(i)) };
    }
    for (x, o) in out.iter_mut().enumerate().take(n) {
        *o = pick(hh[x], g[x + k - 1]);
    }
}

#[cfg(test)]
mod tests;
