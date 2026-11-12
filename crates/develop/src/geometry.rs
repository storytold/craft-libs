//! Output framing: user orientation, lens corrections and perspective ([`Warp`]), crop + straighten,
//! flips — and the mapping between output pixels and normalized coordinates (used by masks, spots and
//! on-canvas tools).
//!
//! Normalized coordinates (masks, spots, crop) refer to the *transformed* image: the oriented source after
//! lens correction and perspective, at the source's size. Everything is sampled in one resample.

use crate::raster::resample::{Filter, resize};
use crate::raster::{Rgb32f, par_rows};
use craft_develop_model::{DevelopSettings, EmbeddedLens};
use craft_geom::{Affine, CropGeometry, Homography, Orientation, Point, Rect};

use crate::optics::{Warp, reorient_lens};

/// Linear value of areas outside the source after a warp (a light neutral, like an empty canvas).
pub const BLANK: [f32; 3] = [0.55, 0.55, 0.55];

/// The geometric frame of a render.
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    /// Source (EXIF-oriented) dimensions in pixels.
    pub src_w: usize,
    pub src_h: usize,
    /// User orientation (Rotate Left/Right, Flip) on top of the source.
    pub orient: Orientation,
    /// Oriented dimensions.
    pub ow: f64,
    pub oh: f64,
    pub crop: CropGeometry,
    pub flip_h: bool,
    pub flip_v: bool,
    /// Lens corrections / perspective (None = identity).
    pub warp: Option<Warp>,
    /// Set when this frame draws only a window of a larger output (see [`Frame::window`]).
    pub view: Option<ViewWindow>,
    /// Mask coordinates are on the photo before the warp ([`Frame::for_mask`],
    /// `craft_develop_model::Mask::on_photo`); otherwise they are normalized coordinates of this
    /// frame's transformed image, like spots and the crop.
    pub masks_on_photo: bool,
}

/// Where a windowed [`Frame`]'s pixels sit in the full output: its offset and the full output's
/// size, for effects defined by the whole frame (post-crop vignette).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewWindow {
    pub x: f64,
    pub y: f64,
    pub full_w: f64,
    pub full_h: f64,
}

impl Frame {
    pub fn new(src_w: usize, src_h: usize, s: &DevelopSettings, apply_crop: bool) -> Frame {
        Frame::with_lens(src_w, src_h, s, apply_crop, None)
    }

    /// Like [`Frame::new`], with the source's embedded lens corrections (relative to the EXIF-oriented source).
    pub fn with_lens(src_w: usize, src_h: usize, s: &DevelopSettings, apply_crop: bool, lens: Option<&EmbeddedLens>) -> Frame {
        let orient = s.orientation;
        let (ow, oh) = if orient.swaps_axes() { (src_h as f64, src_w as f64) } else { (src_w as f64, src_h as f64) };
        // bounded: a crop from a sidecar or an agent can reach far past the photo (issue storytold/lightcraft#2555)
        let crop = if apply_crop { s.crop.geometry.bounded(ow, oh) } else { CropGeometry::default() };
        let lens = lens.map(|l| reorient_lens(l, orient, src_w as f64, src_h as f64));
        let mut warp = Warp::from_settings(ow, oh, s, lens.as_ref());
        let persp = perspective(s, ow, oh);
        if persp != Homography::IDENTITY
            && let Some(inv) = persp.inverse()
        {
            warp.persp = persp;
            warp.persp_inv = inv;
        }
        let mut f = Frame {
            src_w,
            src_h,
            orient,
            ow,
            oh,
            crop,
            // a flip is not part of the crop: the crop tool's uncropped view shows it too
            flip_h: s.crop.flip_h,
            flip_v: s.crop.flip_v,
            warp: (!warp.is_identity()).then_some(warp),
            view: None,
            masks_on_photo: false,
        };
        if apply_crop {
            f.constrain_to_image(s.geometry.constrain_crop);
        }
        f
    }

    /// Shrink the crop about its centre until it contains no area outside the warped source. Lens
    /// corrections (profile, embedded camera warp, manual distortion, lateral CA) always do this, as
    /// a corrected lens never shows the blank past the photo's edge (issue storytold/lightcraft#2361); perspective only
    /// with "Constrain Crop" (`with_persp`), so a keystone fix can keep its corners.
    pub(crate) fn constrain_to_image(&mut self, with_persp: bool) {
        let Some(full) = self.warp.as_ref().filter(|w| w.moves_pixels()) else { return };
        let optics;
        let wp = if with_persp || full.persp == Homography::IDENTITY {
            full
        } else {
            let mut o = full.clone();
            o.persp = Homography::IDENTITY;
            o.persp_inv = Homography::IDENTITY;
            optics = o;
            &optics
        };
        if !wp.moves_pixels() {
            return;
        }
        let planes: &[usize] = if wp.per_channel() { &[0, 1, 2] } else { &[1] };
        let a = self.crop.output_to_source(self.ow, self.oh, 1.0, 1.0);
        let (w, h) = (self.ow, self.oh);
        let fits = |t: f64| {
            (0..=48).all(|i| {
                let u = i as f64 / 48.0;
                [(u, 0.0), (u, 1.0), (0.0, u), (1.0, u)].iter().all(|&(x, y)| {
                    let p = Point::new(0.5 + t * (x - 0.5), 0.5 + t * (y - 0.5));
                    planes.iter().all(|&ch| {
                        let s = wp.to_source(a.apply(p), ch);
                        s.x >= -1e-6 * w && s.y >= -1e-6 * h && s.x <= w * (1.0 + 1e-6) && s.y <= h * (1.0 + 1e-6)
                    })
                })
            })
        };
        if fits(1.0) {
            return;
        }
        let (mut lo, mut hi) = (0.0, 1.0);
        for _ in 0..30 {
            let m = (lo + hi) / 2.0;
            if fits(m) { lo = m } else { hi = m }
        }
        let t = lo.max(0.02);
        let r = self.crop.rect;
        self.crop.rect = Rect::from_center(r.center(), r.width() * t, r.height() * t);
    }

    /// Long edge, in pixels, of the whole output this frame draws (its own size unless it is a
    /// window of a larger one): what size-relative effects (noise reduction, edge refinement)
    /// must scale with, so a window matches the same pixels of the whole render.
    pub fn output_long(&self, w: usize, h: usize) -> usize {
        self.view.map_or(w.max(h), |v| v.full_w.max(v.full_h) as usize)
    }

    /// This frame narrowed to `win` of its `full_w × full_h` output: the same mapping, but the
    /// output is only that window (the crop rectangle shrinks to it; flips are as drawn).
    pub fn window(&self, full_w: usize, full_h: usize, win: crate::PixelWindow) -> Frame {
        let (fw, fh) = (full_w.max(1) as f64, full_h.max(1) as f64);
        let (x0, x1) = (win.x as f64 / fw, (win.x + win.w) as f64 / fw);
        let (y0, y1) = (win.y as f64 / fh, (win.y + win.h) as f64 / fh);
        // flips happen on the output before it maps onto the crop: undo them for the range
        let (x0, x1) = if self.flip_h { (1.0 - x1, 1.0 - x0) } else { (x0, x1) };
        let (y0, y1) = if self.flip_v { (1.0 - y1, 1.0 - y0) } else { (y0, y1) };
        let r = self.crop.rect;
        let (rw, rh) = (r.width(), r.height());
        let mut f = self.clone();
        f.crop.rect = Rect { x0: r.x0 + x0 * rw, y0: r.y0 + y0 * rh, x1: r.x0 + x1 * rw, y1: r.y0 + y1 * rh };
        f.view = Some(ViewWindow { x: win.x as f64, y: win.y as f64, full_w: fw, full_h: fh });
        f
    }

    /// Add automatically estimated lateral CA (`[α_R, α_B]`, see [`crate::optics::estimate_lateral_ca`]).
    pub fn add_lateral_ca(&mut self, ca: [f64; 2]) {
        if ca == [0.0; 2] {
            return;
        }
        let w = self.warp.get_or_insert_with(|| Warp::identity(self.ow, self.oh));
        w.ca[0] += ca[0];
        w.ca[2] += ca[1];
    }

    /// Map a point of the transformed image (normalized) to the lens-corrected, pre-perspective image (normalized).
    pub fn transformed_to_corrected(&self, p: Point) -> Point {
        match &self.warp {
            Some(w) => {
                let q = w.to_corrected(Point::new(p.x * self.ow, p.y * self.oh));
                Point::new(q.x / self.ow, q.y / self.oh)
            }
            None => p,
        }
    }

    /// Inverse of [`Frame::transformed_to_corrected`].
    pub fn corrected_to_transformed(&self, p: Point) -> Point {
        match &self.warp {
            Some(w) => {
                let q = w.from_corrected(Point::new(p.x * self.ow, p.y * self.oh));
                Point::new(q.x / self.ow, q.y / self.oh)
            }
            None => p,
        }
    }

    /// Aspect ratio (w/h) of the output. A crop narrower than a source pixel (or of no size, from
    /// an agent or a sidecar) counts each side as one pixel, so the aspect stays finite: 0/0 made
    /// it NaN, and the loupe laid the photo out at a NaN size.
    pub fn aspect(&self) -> f64 {
        let r = self.crop.rect_px(self.ow, self.oh);
        r.width().max(1.0) / r.height().max(1.0)
    }

    /// Size of the (cropped) output at the source's own resolution, in pixels.
    pub fn native_size(&self) -> (f64, f64) {
        let r = self.crop.rect_px(self.ow, self.oh);
        (r.width(), r.height())
    }

    pub fn fit(&self, max_w: usize, max_h: usize) -> (usize, usize) {
        let a = self.aspect();
        let (mw, mh) = (max_w.max(1) as f64, max_h.max(1) as f64);
        // a box that is already this aspect up to rounding (an export size) is the size: fitting
        // into it again rounds twice and can lose a pixel of the edge that was asked for.
        if (mw - 0.5) / a <= mh + 0.5 && (mw + 0.5) / a >= mh - 0.5 {
            return (max_w.max(1), max_h.max(1));
        }
        let (w, h) = if mw / mh > a { (mh * a, mh) } else { (mw, mw / a) };
        ((w.round() as usize).max(1), (h.round() as usize).max(1))
    }

    /// Output pixels per unit of the oriented image's long edge.
    pub fn px_per_long(&self, out_w: usize) -> f64 {
        let r = self.crop.rect_px(self.ow, self.oh);
        out_w as f64 / r.width() * self.ow.max(self.oh)
    }

    /// Affine from output pixel coordinates to oriented-image pixel coordinates.
    pub fn out_to_oriented(&self, out_w: usize, out_h: usize) -> Affine {
        let (w, h) = (out_w as f64, out_h as f64);
        let mut flip = Affine::IDENTITY;
        if self.flip_h {
            flip = Affine([-1.0, 0.0, 0.0, 1.0, w, 0.0]) * flip;
        }
        if self.flip_v {
            flip = Affine([1.0, 0.0, 0.0, -1.0, 0.0, h]) * flip;
        }
        self.crop.output_to_source(self.ow, self.oh, w, h) * flip
    }

    /// Map an output pixel to normalized oriented-image coordinates (0..1).
    pub fn out_to_norm(&self, out_w: usize, out_h: usize) -> Affine {
        Affine::scale(1.0 / self.ow, 1.0 / self.oh) * self.out_to_oriented(out_w, out_h)
    }

    /// Normalized oriented coords → output pixel (inverse of `out_to_norm`).
    pub fn norm_to_out(&self, out_w: usize, out_h: usize) -> Affine {
        self.out_to_norm(out_w, out_h).inverse().unwrap_or(Affine::IDENTITY)
    }

    /// This frame for evaluating mask `m`: whether its coordinates are on the photo before the
    /// warp ([`craft_develop_model::Mask::on_photo`]).
    pub fn for_mask(&self, on_photo: bool) -> std::borrow::Cow<'_, Frame> {
        if self.masks_on_photo == on_photo {
            std::borrow::Cow::Borrowed(self)
        } else {
            std::borrow::Cow::Owned(Frame { masks_on_photo: on_photo, ..self.clone() })
        }
    }

    /// The warp mask coordinates go through: the lens / perspective warp when masks are on the
    /// photo and the warp moves pixels, else none (mask coordinates are then this frame's
    /// normalized coordinates).
    pub fn mask_warp(&self) -> Option<&Warp> {
        self.warp.as_ref().filter(|w| self.masks_on_photo && w.moves_pixels())
    }

    /// Output pixel → mask coordinates (see [`Frame::masks_on_photo`]).
    pub fn out_to_mask(&self, out_w: usize, out_h: usize) -> MaskMap {
        MaskMap {
            o2t: self.out_to_oriented(out_w, out_h),
            to_norm: self.out_to_norm(out_w, out_h),
            ow: self.ow,
            oh: self.oh,
            warp: self.mask_warp().cloned(),
        }
    }

    /// Mask coordinates → output pixel (the inverse of [`Frame::out_to_mask`]); `None` for a
    /// point the warp can't invert (far outside a strong one), which has no place in the output.
    pub fn mask_to_out(&self, out_w: usize, out_h: usize) -> impl Fn(Point) -> Option<Point> + use<> {
        let to_out = self.norm_to_out(out_w, out_h);
        let (warp, ow, oh) = (self.mask_warp().cloned(), self.ow, self.oh);
        move |n: Point| match &warp {
            Some(w) => w.from_source(Point::new(n.x * ow, n.y * oh)).map(|t| to_out.apply(Point::new(t.x / ow, t.y / oh))),
            None => Some(to_out.apply(n)),
        }
    }

    /// Normalized coordinates of this frame's transformed image → mask coordinates.
    pub fn norm_to_mask(&self, n: Point) -> Point {
        match self.mask_warp() {
            Some(w) => {
                let s = w.to_source(Point::new(n.x * self.ow, n.y * self.oh), 1);
                Point::new(s.x / self.ow, s.y / self.oh)
            }
            None => n,
        }
    }

    /// Mask coordinates → normalized coordinates of this frame's transformed image.
    pub fn mask_to_norm(&self, n: Point) -> Point {
        mask_to_norm(self.mask_warp(), self.ow, self.oh, n)
    }

    /// Convert normalized oriented coordinates to "long-edge units" (isotropic), used by mask shapes.
    pub fn norm_to_long(&self, p: Point) -> Point {
        let l = self.ow.max(self.oh);
        Point::new(p.x * self.ow / l, p.y * self.oh / l)
    }

    /// How [`Frame::sample`] resamples a (EXIF-oriented) source of `src_w × src_h` into `w × h`.
    pub fn sample_plan(&self, src_w: usize, src_h: usize, w: usize, h: usize) -> SamplePlan {
        let (ow, oh) = if self.orient.swaps_axes() { (src_h, src_w) } else { (src_w, src_h) };
        let crop_px = self.crop.rect_px(self.ow, self.oh);
        let k = crop_px.width() / w as f64;
        let prefilter = (k > 1.25).then(|| (((ow as f64 / k).round() as usize).max(1), ((oh as f64 / k).round() as usize).max(1)));
        let (bw, bh) = prefilter.unwrap_or((ow, oh));
        let (sx, sy) = (bw as f64 / ow as f64, bh as f64 / oh as f64);
        let mode = if self.warp.is_some() {
            SampleMode::Warp(self.out_to_oriented(w, h))
        } else if self.crop.is_identity() && !self.flip_h && !self.flip_v && bw == w && bh == h {
            SampleMode::Copy
        } else {
            SampleMode::Affine(Affine::scale(sx, sy) * self.out_to_oriented(w, h))
        };
        SamplePlan { ow, oh, prefilter, sx, sy, mode }
    }

    /// Sample the source into a `w × h` output buffer: orientation, crop, rotation, flips — one
    /// bilinear resample from a pre-filtered (area-downscaled) copy, so minification never aliases.
    pub fn sample(&self, src: &Rgb32f, w: usize, h: usize) -> Rgb32f {
        let oriented = if self.orient == Orientation::Normal { None } else { Some(src.oriented(self.orient)) };
        let o = oriented.as_ref().unwrap_or(src);
        let plan = self.sample_plan(src.width, src.height, w, h);
        let base = match plan.prefilter {
            Some((nw, nh)) => std::borrow::Cow::Owned(resize(o, nw, nh, Filter::Mitchell)),
            None => std::borrow::Cow::Borrowed(o),
        };
        self.sample_base(base, plan, w, h)
    }

    /// [`Frame::sample`] of each of `windows` of this frame at its `full_w × full_h` output size: the
    /// same pixels, with the source oriented and prefiltered once for all of them rather than per window.
    pub fn sample_windows(&self, src: &Rgb32f, full_w: usize, full_h: usize, windows: &[crate::PixelWindow]) -> Vec<Rgb32f> {
        let oriented = if self.orient == Orientation::Normal { None } else { Some(src.oriented(self.orient)) };
        let o = oriented.as_ref().unwrap_or(src);
        let plans: Vec<(Frame, SamplePlan)> = windows
            .iter()
            .map(|t| {
                let frame = if (t.w, t.h) == (full_w, full_h) { self.clone() } else { self.window(full_w, full_h, *t) };
                let plan = frame.sample_plan(src.width, src.height, t.w, t.h);
                (frame, plan)
            })
            .collect();
        let mut bases: Vec<((usize, usize), Rgb32f)> = Vec::new();
        for dims in plans.iter().filter_map(|p| p.1.prefilter) {
            if !bases.iter().any(|b| b.0 == dims) {
                bases.push((dims, resize(o, dims.0, dims.1, Filter::Mitchell)));
            }
        }
        plans
            .into_iter()
            .zip(windows)
            .map(|((frame, plan), t)| {
                let base = match plan.prefilter.and_then(|dims| bases.iter().find(|b| b.0 == dims)) {
                    Some(b) => std::borrow::Cow::Borrowed(&b.1),
                    None => std::borrow::Cow::Borrowed(o),
                };
                frame.sample_base(base, plan, t.w, t.h)
            })
            .collect()
    }

    /// The sampling of [`Frame::sample`] from the oriented, prefiltered source `base`.
    fn sample_base(&self, base: std::borrow::Cow<'_, Rgb32f>, plan: SamplePlan, w: usize, h: usize) -> Rgb32f {
        let affine = |xf: Affine| {
            let mut out = Rgb32f::new(w, h);
            par_rows(&mut out.data, w, |y, row| {
                for (x, px) in row.iter_mut().enumerate() {
                    let p = xf.apply(Point::new(x as f64 + 0.5, y as f64 + 0.5));
                    *px = base.sample_bilinear(p.x as f32, p.y as f32);
                }
            });
            out
        };
        match (plan.mode, self.warp.as_ref()) {
            (SampleMode::Copy, _) => base.into_owned(),
            (SampleMode::Warp(o2t), Some(warp)) => sample_warped(&base, plan.sx, plan.sy, warp, o2t, w, h),
            // `sample_plan` plans a warp only when there is one; without it the same mapping is affine
            (SampleMode::Warp(o2t), None) => affine(Affine::scale(plan.sx, plan.sy) * o2t),
            (SampleMode::Affine(xf), _) => affine(xf),
        }
    }

    /// The crop rectangle as a quad in normalized oriented coordinates (for overlays).
    pub fn crop_quad_norm(&self) -> [Point; 4] {
        let r = self.crop.rect_px(self.ow, self.oh);
        let back = Affine::rotate_about(-self.crop.angle.to_radians(), Point::new(self.ow / 2.0, self.oh / 2.0));
        r.corners().map(|p| {
            let q = back.apply(p);
            Point::new(q.x / self.ow, q.y / self.oh)
        })
    }
}

/// Mask coordinates `n` (on the photo before `warp`, normalized to `ow × oh`) → normalized
/// transformed coordinates. A point the warp can't invert (far outside a strong one) stays put.
fn mask_to_norm(warp: Option<&Warp>, ow: f64, oh: f64, n: Point) -> Point {
    match warp.and_then(|w| w.from_source(Point::new(n.x * ow, n.y * oh))) {
        Some(t) => Point::new(t.x / ow, t.y / oh),
        None => n,
    }
}

/// Output pixel → mask coordinates of a frame ([`Frame::out_to_mask`]).
#[derive(Clone, Debug)]
pub struct MaskMap {
    /// Output px → transformed px, and → normalized transformed coordinates.
    o2t: Affine,
    to_norm: Affine,
    ow: f64,
    oh: f64,
    warp: Option<Warp>,
}

impl MaskMap {
    /// The mask coordinates at output position `p` (px; pixel centres at `x + 0.5`).
    #[inline]
    pub fn apply(&self, p: Point) -> Point {
        match &self.warp {
            Some(w) => {
                let s = w.to_source(self.o2t.apply(p), 1);
                Point::new(s.x / self.ow, s.y / self.oh)
            }
            None => self.to_norm.apply(p),
        }
    }
}

/// How a render resamples its source (see [`Frame::sample_plan`]).
#[derive(Clone, Debug)]
pub struct SamplePlan {
    /// Oriented source size.
    pub ow: usize,
    pub oh: usize,
    /// Size of the Mitchell prefilter (area downscale before the bilinear resample), if any.
    pub prefilter: Option<(usize, usize)>,
    /// Oriented-source px → prefiltered px.
    pub sx: f64,
    pub sy: f64,
    pub mode: SampleMode,
}

#[derive(Clone, Debug)]
pub enum SampleMode {
    /// The (prefiltered) oriented source is the output.
    Copy,
    /// Bilinear through this affine (output px → prefiltered px).
    Affine(Affine),
    /// Through the warp; the affine maps output px → transformed px.
    Warp(Affine),
}

/// The perspective homography of the settings (Upright, then the manual Transform sliders), lens-corrected →
/// transformed, in centred coordinates.
pub fn perspective(s: &DevelopSettings, ow: f64, oh: f64) -> Homography {
    if !s.section_enabled("geometry") {
        return Homography::IDENTITY;
    }
    crate::transform::manual(&s.geometry).mul(&crate::upright::homography(&s.geometry, ow, oh))
}

/// One resample through the warp: `o2t` maps output px → transformed px, the warp maps those to source px
/// (scaled by `sx, sy` into `base`).
fn sample_warped(base: &Rgb32f, sx: f64, sy: f64, wp: &Warp, o2t: Affine, w: usize, h: usize) -> Rgb32f {
    let per_channel = wp.per_channel();
    let gain = wp.has_gain();
    let mut out = Rgb32f::new(w, h);
    par_rows(&mut out.data, w, |y, row| {
        for (x, px) in row.iter_mut().enumerate() {
            let Some((c, g)) = wp.frame(&o2t, x, y) else {
                *px = BLANK;
                continue;
            };
            let mut v = base.sample_bilinear((g.x * sx) as f32, (g.y * sy) as f32);
            if per_channel {
                for ch in [0usize, 2] {
                    let q = wp.corrected_to_source(c, ch);
                    v[ch] = base.sample_bilinear((q.x * sx) as f32, (q.y * sy) as f32)[ch];
                }
            }
            if gain {
                let k = wp.gain(g);
                v = v.map(|c| c * k);
            }
            *px = v;
        }
    });
    out
}

/// Whole-image rectangle in normalized coordinates.
pub const FULL: Rect = Rect::UNIT;

#[cfg(test)]
mod tests {
    use super::*;

    /// Issue storytold/lightcraft#2555: a turned crop rect of 0..1000 (from a sidecar or an agent) made the native size
    /// of a 6000 × 4000 photo ~2.4·10¹³ pixels, which a full-size export allocated.
    #[test]
    fn a_crop_far_past_the_photo_keeps_the_native_size_near_the_source() {
        let mut s = DevelopSettings::default();
        s.crop.geometry = craft_geom::CropGeometry { rect: craft_geom::Rect::new(0.0, 0.0, 1000.0, 1000.0), angle: 5.0 };
        let (w, h) = crate::native_output_size(6000, 4000, &s);
        assert!(w <= 12_000.0 && h <= 8_000.0, "{w} x {h}");
        // an untouched photo is its own size
        assert_eq!(crate::native_output_size(6000, 4000, &DevelopSettings::default()), (6000.0, 4000.0));
    }

    #[test]
    fn default_frame_is_identity() {
        let s = DevelopSettings::default();
        let f = Frame::new(300, 200, &s, true);
        assert_eq!(f.fit(3000, 3000), (3000, 2000));
        let m = f.out_to_norm(300, 200);
        let p = m.apply(Point::new(150.0, 100.0));
        assert!((p.x - 0.5).abs() < 1e-9 && (p.y - 0.5).abs() < 1e-9);
        assert!((f.px_per_long(300) - 300.0).abs() < 1e-9);
    }

    #[test]
    fn rotation_swaps_aspect() {
        let s = DevelopSettings { orientation: Orientation::Rotate90, ..Default::default() };
        let f = Frame::new(300, 200, &s, true);
        assert_eq!(f.fit(1000, 1000), (667, 1000));
    }

    #[test]
    fn crop_changes_aspect_and_mapping() {
        let mut s = DevelopSettings::default();
        s.crop.geometry.rect = Rect::new(0.5, 0.0, 1.0, 1.0);
        let f = Frame::new(400, 200, &s, true);
        assert_eq!(f.fit(1000, 1000), (1000, 1000));
        let p = f.out_to_norm(100, 100).apply(Point::new(0.0, 0.0));
        assert!((p.x - 0.5).abs() < 1e-9 && p.y.abs() < 1e-9);
        let back = f.norm_to_out(100, 100).apply(Point::new(0.75, 0.5));
        assert!((back.x - 50.0).abs() < 1e-6 && (back.y - 50.0).abs() < 1e-6);
    }

    /// A crop of no size (or under a pixel wide) gave a NaN or infinite aspect; each side now
    /// counts as at least one source pixel.
    #[test]
    fn degenerate_crops_have_a_finite_aspect() {
        let mut s = DevelopSettings::default();
        for (rect, want) in [
            (Rect::new(0.5, 0.5, 0.5, 0.5), 1.0),
            (Rect::new(0.0, 0.0, 1e-9, 1.0), 1.0 / 200.0),
            (Rect::new(0.0, 0.5, 1.0, 0.5), 400.0),
            (Rect::new(0.25, 0.0, 0.75, 1.0), 1.0),
        ] {
            s.crop.geometry.rect = rect;
            let f = Frame::new(400, 200, &s, true);
            assert!((f.aspect() - want).abs() < 1e-9, "{rect:?}: {}", f.aspect());
            let (w, h) = f.fit(1000, 1000);
            assert!((1..=1000).contains(&w) && (1..=1000).contains(&h), "{rect:?}: {w}×{h}");
        }
    }

    #[test]
    fn flip_mirrors() {
        let mut s = DevelopSettings::default();
        s.crop.flip_h = true;
        let src = Rgb32f::from_fn(4, 1, |x, _| [x as f32, 0.0, 0.0]);
        let f = Frame::new(4, 1, &s, true);
        let out = f.sample(&src, 4, 1);
        assert!((out.get(0, 0)[0] - 3.0).abs() < 1e-5);
        assert!((out.get(3, 0)[0] - 0.0).abs() < 1e-5);
    }

    /// The crop tool's uncropped frame keeps the flips: Flip H / Flip V show while cropping, and
    /// screen points map back through the mirror (the crop box is drawn and dragged right).
    #[test]
    fn uncropped_frame_keeps_the_flips() {
        let mut s = DevelopSettings::default();
        s.crop.flip_h = true;
        s.crop.flip_v = true;
        s.crop.geometry.rect = Rect::new(0.1, 0.2, 0.6, 0.9);
        let src = Rgb32f::from_fn(4, 2, |x, y| [x as f32, y as f32, 0.0]);
        let f = Frame::new(4, 2, &s, false);
        let out = f.sample(&src, 4, 2);
        assert!((out.get(0, 0)[0] - 3.0).abs() < 1e-5 && (out.get(0, 0)[1] - 1.0).abs() < 1e-5, "{:?}", out.get(0, 0));
        assert!((out.get(3, 1)[0] - 0.0).abs() < 1e-5 && (out.get(3, 1)[1] - 0.0).abs() < 1e-5, "{:?}", out.get(3, 1));
        // the whole image, mirrored: output top-left is the image's bottom-right
        let p = f.out_to_norm(400, 200).apply(Point::new(0.0, 0.0));
        assert!((p.x - 1.0).abs() < 1e-9 && (p.y - 1.0).abs() < 1e-9, "{p:?}");
    }
}
