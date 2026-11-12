//! Lens Blur: a shallow depth of field added after the fact.
//!
//! Without a depth map, the scene has two depths: what is in focus (the photo's subject, or the
//! selection of the mask [`LensBlur::focus_mask`](craft_develop_model::LensBlur) names) and
//! everything else. A pixel's circle of confusion is `R · (1 − focus)`, `R` set by Blur Amount
//! relative to the long edge, so previews and exports match.
//!
//! The blur is gathered from [`LEVELS`] disc blurs of growing radius, blended per pixel by its
//! circle of confusion. Each disc blur runs on a grid coarse enough that the disc spans about
//! [`DISC_CELLS`] cells (its cost doesn't grow with the radius) and is a normalized convolution
//! weighted by how far out of focus each pixel is, so the sharp subject doesn't bleed into the
//! blurred background around it (no halo). Boost brightens highlights before they are spread, so
//! they turn into bright discs (bokeh), as a lens does with scene-linear light.
//!
//! Brush Refinement paints the focus by hand: Focus brush strokes stay sharp, Blur brush strokes
//! are blurred whatever the focus says.
//!
//! A mask can blur too (Lens Blur among its local adjustments): there the mask's strength is the
//! circle of confusion, so what it fully selects gets the whole blur, its feathered edge less, and
//! what it leaves out stays sharp and doesn't bleed into the blur.
//!
//! Amount is capped at 3.5% of the output's long edge. The disc convolution runs on a grid whose
//! disc radius is at most [`DISC_CELLS`] cells and whose size is capped by [`MAX_CELLS`] per level,
//! so increasing the output resolution or blur radius does not make the convolution kernel grow.
//! The full-resolution passes remain linear in the output pixel count.
//!
//! It runs on the scene-linear image (white-balanced and denoised, before exposure: a blur is
//! linear, so exposure commutes with it), on the CPU, for both renderers.

use crate::raster::{Plane, Rgb32f};
use craft_develop_model::{BrushStroke, DevelopSettings, Mask, MaskShape};

use crate::geometry::Frame;
use crate::masks::{self, Mattes};

/// The largest circle of confusion (Blur Amount 100), as a fraction of the long edge.
pub const MAX_RADIUS: f64 = 0.035;
/// How far from the focus the blur reaches its full size at Transition 100, as a fraction of the
/// long edge (a stand-in for depth: what is near the subject in the picture is usually near it in
/// the scene too, as a lens renders a floor or a wall running away from it).
pub const MAX_TRANSITION: f64 = 0.15;
/// The blur right at the focus's edge, as a share of the full blur (with a transition): the
/// background around a subject is a little soft at once, not cut out sharp.
const EDGE_BLUR: f32 = 0.1;
/// The feather (0..100) of a fixed focus shape's edge ([`craft_develop_model::FocusShape`]).
const SHAPE_FEATHER: f64 = 40.0;
/// Most cells along the long edge of the grid the distance to the focus is measured on.
const DISTANCE_CELLS: usize = 384;
/// Disc blurs the per-pixel circle of confusion is interpolated between.
const LEVELS: usize = 4;
/// Radius, in grid cells, a level's disc blur works at.
const DISC_CELLS: f32 = 8.0;
/// Most cells a level's grid holds (bounds the memory of a full-size export).
const MAX_CELLS: usize = 3_000_000;
/// Weight of in-focus pixels in the blurred levels: enough to keep a level defined where only
/// in-focus pixels are near, too little to bleed them into the background.
const FOCUS_WEIGHT: f32 = 1e-4;
/// The brightest highlights grow this much brighter at Boost 100 before they are spread.
const BOOST_GAIN: f32 = 4.0;

/// Whether `s` renders a lens blur (switched on, with some amount, and its section's eye open).
pub fn active(s: &DevelopSettings) -> bool {
    let b = &s.lens_blur;
    b.enabled && b.amount.is_finite() && b.amount > 0.0 && s.section_enabled("lensBlur")
}

/// Whether [`apply`] changes anything: the global lens blur, or a mask's.
pub fn renders(s: &DevelopSettings) -> bool {
    active(s) || blurring_masks(s).next().is_some()
}

/// The largest circle of confusion (radius in output pixels) at `px_per_long` output pixels per
/// long edge; 0 when [`active`] is false.
pub fn radius_px(s: &DevelopSettings, px_per_long: f64) -> f32 {
    if !active(s) {
        return 0.0;
    }
    let r = s.lens_blur.amount.clamp(0.0, 100.0) / 100.0 * MAX_RADIUS * px_per_long;
    if r.is_finite() { r.max(0.0) as f32 } else { 0.0 }
}

/// The masks that blur what they select, or what they leave out (the Lens Blur block among a
/// mask's local adjustments): shown, with components, the block on and some blur.
fn blurring_masks(s: &DevelopSettings) -> impl Iterator<Item = &Mask> {
    s.masks
        .iter()
        .filter(|m| m.visible && !m.components.is_empty() && !m.adjust.lens_blur_off && m.adjust.lens_blur.is_finite() && m.adjust.lens_blur > 0.0)
}

/// The circle of confusion (radius in output pixels) where mask `m` fully selects.
fn mask_radius_px(m: &Mask, px_per_long: f64) -> f32 {
    let r = m.adjust.lens_blur.clamp(0.0, 100.0) / 100.0 * MAX_RADIUS * px_per_long;
    if r.is_finite() { r.max(0.0) as f32 } else { 0.0 }
}

/// The distance from the focus (output pixels) at which the blur is full; 0: a sharp cut.
pub fn transition_px(s: &DevelopSettings, px_per_long: f64) -> f32 {
    if !active(s) {
        return 0.0;
    }
    transition_of(s.lens_blur.transition, px_per_long)
}

/// [`transition_px`] for a Transition slider value `t` (0..100).
fn transition_of(t: f64, px_per_long: f64) -> f32 {
    if !t.is_finite() || t <= 0.0 {
        return 0.0;
    }
    let d = t.min(100.0) / 100.0 * MAX_TRANSITION * px_per_long;
    if d.is_finite() { d.max(0.0) as f32 } else { 0.0 }
}

/// A Boost slider value (0..100) as 0..1.
fn boost_of(b: f64) -> f32 {
    if b.is_finite() { (b.clamp(0.0, 100.0) / 100.0) as f32 } else { 0.0 }
}

/// The farthest any lens blur of `s` reads (output pixels): the global one's (its blur, and how far
/// it looks for the focus) or a mask's.
pub fn reach_px(s: &DevelopSettings, px_per_long: f64) -> f32 {
    let global = radius_px(s, px_per_long) + transition_px(s, px_per_long);
    blurring_masks(s).map(|m| mask_radius_px(m, px_per_long) + transition_of(m.adjust.lens_blur_transition, px_per_long)).fold(global, f32::max)
}

/// `win` (a window of the `full_w × full_h` output) grown by the blur's reach, so a zoomed view
/// blurs its edges with the pixels around them, as the whole render does (the render is cut back
/// to `win`, see [`crate::Plan::keep`]).
pub fn window_for_reads(s: &DevelopSettings, full_w: usize, full_h: usize, px_per_long: f64, win: crate::PixelWindow) -> crate::PixelWindow {
    let r = reach_px(s, px_per_long);
    if r <= 0.0 {
        return win;
    }
    let m = (r * 1.25).ceil() as usize + 2;
    let (x0, y0) = (win.x.saturating_sub(m), win.y.saturating_sub(m));
    let x1 = win.x.saturating_add(win.w).saturating_add(m).min(full_w.max(1));
    let y1 = win.y.saturating_add(win.h).saturating_add(m).min(full_h.max(1));
    crate::PixelWindow { x: x0, y: y0, w: x1.saturating_sub(x0).max(1), h: y1.saturating_sub(y0).max(1) }
}

/// What [`apply`] reads from `s`, for render cache keys (empty when no lens blur renders).
pub fn key(s: &DevelopSettings) -> String {
    let mut k = String::new();
    if active(s) {
        k = match focus_mask(s) {
            Some(m) => format!("{:?}|{m:?}", s.lens_blur),
            None => format!("{:?}", s.lens_blur),
        };
    }
    for m in blurring_masks(s) {
        k.push_str(&format!("|{m:?}"));
    }
    // a mask's luminance / colour ranges select on the exposed image
    if !k.is_empty() {
        k.push_str(&format!("|{}", s.light.exposure.to_bits()));
    }
    k
}

/// The mask that says what is in focus, if the settings name one that exists and selects something.
/// An AI mask whose selection is still being computed (Find Subject on a slow machine) doesn't
/// count yet: until it is, the subject estimate stays in focus rather than nothing.
fn focus_mask(s: &DevelopSettings) -> Option<&Mask> {
    let id = s.lens_blur.focus_mask?;
    s.masks.iter().find(|m| m.id == id && !m.components.is_empty() && !awaiting_ai(m))
}

/// Whether every component of `m` is an AI selection (Object / Describe) not computed yet.
pub fn awaiting_ai(m: &Mask) -> bool {
    m.components.iter().all(|c| match &c.shape {
        MaskShape::Object { seg, detail, .. } | MaskShape::Prompt { seg, detail, .. } => seg.is_none() && detail.is_empty(),
        _ => false,
    })
}

/// Smoothstep: 0 below `e0`, 1 above `e1`, a smooth S-curve between.
#[inline]
fn smooth(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// What is in focus, 0..1 per pixel of `img` (scene-linear, before exposure, output resolution).
pub fn focus(img: &Rgb32f, s: &DevelopSettings, frame: &Frame, mattes: Option<&Mattes>) -> Plane {
    focus_with(img, s, frame, mattes, None)
}

/// [`focus`], taking the automatic subject estimate from `subject` (made from the whole photo,
/// see [`subject_estimate`]) when there is one, so a window of the output gets the same focus as
/// the whole render.
fn focus_with(img: &Rgb32f, s: &DevelopSettings, frame: &Frame, mattes: Option<&Mattes>, subject: Option<&SubjectEstimate>) -> Plane {
    let (w, h) = (img.width, img.height);
    let log_l = img.map(crate::local::log_lum);
    let mut a = match (focus_mask(s), s.lens_blur.focus_area(s.crop_center())) {
        (Some(m), _) => masks::evaluate_one(m, frame, w, h, img, &log_l, s.light.exposure as f32, mattes),
        // a fixed shape: where it was moved to, else the middle of the crop (in photo
        // coordinates, so it stays put in a zoomed view)
        (None, Some(area)) => {
            let shape = MaskShape::Radial { center: area.center, rx: area.rx, ry: area.ry, angle: 0.0, feather: SHAPE_FEATHER, invert: false };
            masks::shape_alpha(&shape, frame, w, h, img, &log_l, 0.0, mattes)
        }
        (None, None) if let Some(est) = subject => est.sample(frame, w, h),
        (None, None) => {
            let mut a = masks::shape_alpha(&MaskShape::Subject, frame, w, h, img, &log_l, 0.0, mattes);
            // the saliency heuristic gives a flat centre about 0.5: in focus too (person mattes
            // are 0 or 1 and stay so)
            a.data.iter_mut().for_each(|v| *v = smooth(0.25, 0.55, *v));
            a
        }
    };
    // Brush Refinement: the Focus brush adds focus, the Blur brush takes it away (last word)
    let b = &s.lens_blur;
    let brush = |strokes: &[BrushStroke]| {
        (!strokes.is_empty()).then(|| masks::shape_alpha(&MaskShape::Brush { strokes: strokes.to_vec() }, frame, w, h, img, &log_l, 0.0, mattes))
    };
    if let Some(f) = brush(&b.focus_strokes) {
        a.data.iter_mut().zip(&f.data).for_each(|(v, f)| *v = v.max(*f));
    }
    if let Some(bl) = brush(&b.blur_strokes) {
        a.data.iter_mut().zip(&bl.data).for_each(|(v, b)| *v *= 1.0 - b.clamp(0.0, 1.0));
    }
    a.data.iter_mut().for_each(|v| *v = if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 });
    a
}

/// Long edge of the small render [`subject_point`] looks at.
const SUBJECT_EDGE: usize = 256;

/// Long edge of the small render [`subject_estimate`] makes of the whole photo.
const ESTIMATE_EDGE: usize = 384;

/// The automatic subject estimate (the saliency heuristic, for a photo without a person matte)
/// made once from the whole, uncropped photo at [`ESTIMATE_EDGE`]: every render of the photo,
/// whole or a window of it, samples the same estimate, so a zoomed view keeps the focus the
/// whole render has (the heuristic reads its neighbourhood, which a window cuts off).
#[derive(Debug)]
pub struct SubjectEstimate {
    plane: Plane,
    frame: Frame,
}

impl SubjectEstimate {
    /// The estimate at each pixel of a `w × h` output of `frame` (bilinear).
    fn sample(&self, frame: &Frame, w: usize, h: usize) -> Plane {
        let (pw, ph) = (self.plane.width, self.plane.height);
        let mut out = Plane::new(w, h);
        if pw == 0 || ph == 0 || self.plane.data.len() != pw * ph {
            return out;
        }
        let to_norm = frame.out_to_norm(w, h);
        let to_proxy = self.frame.norm_to_out(pw, ph);
        let at = |x: usize, y: usize| self.plane.data.get(y.min(ph - 1) * pw + x.min(pw - 1)).copied().unwrap_or(0.0);
        for (i, v) in out.data.iter_mut().enumerate() {
            let n = to_norm.apply(craft_geom::Point::new((i % w) as f64 + 0.5, (i / w) as f64 + 0.5));
            let p = to_proxy.apply(n);
            let (fx, fy) = ((p.x - 0.5).clamp(0.0, (pw - 1) as f64), (p.y - 0.5).clamp(0.0, (ph - 1) as f64));
            if !(fx.is_finite() && fy.is_finite()) {
                continue;
            }
            let (x0, y0) = (fx as usize, fy as usize);
            let (tx, ty) = ((fx - x0 as f64) as f32, (fy - y0 as f64) as f32);
            let top = at(x0, y0) + (at(x0 + 1, y0) - at(x0, y0)) * tx;
            let bot = at(x0, y0 + 1) + (at(x0 + 1, y0 + 1) - at(x0, y0 + 1)) * tx;
            *v = top + (bot - top) * ty;
        }
        out
    }
}

/// The [`SubjectEstimate`] the lens blur of `s` focuses on, when it focuses on the automatic
/// subject estimate (switched on, no focus mask, no fixed shape) of a photo without a person
/// matte; `None` otherwise (the focus then needs no whole-photo data).
pub fn subject_estimate(src: &Rgb32f, s: &DevelopSettings, mattes: Option<&Mattes>) -> Option<SubjectEstimate> {
    if !active(s) || focus_mask(s).is_some() || s.lens_blur.focus_area(s.crop_center()).is_some() || mattes.is_some_and(Mattes::selects_subject) {
        return None;
    }
    if src.width == 0 || src.height == 0 || src.data.len() != src.width.saturating_mul(src.height) {
        return None;
    }
    let frame = Frame::new(src.width, src.height, s, false);
    let (w, h) = frame.fit(ESTIMATE_EDGE, ESTIMATE_EDGE);
    if w == 0 || h == 0 {
        return None;
    }
    let img = frame.sample(src, w, h);
    let log_l = img.map(crate::local::log_lum);
    let mut plane = masks::shape_alpha(&MaskShape::Subject, &frame, w, h, &img, &log_l, 0.0, None);
    // the saliency heuristic gives a flat centre about 0.5: in focus too
    plane.data.iter_mut().for_each(|v| *v = smooth(0.25, 0.55, *v));
    Some(SubjectEstimate { plane, frame })
}

/// Where the photo's subject most likely is, for a photo whose camera recorded no focus point: the
/// peak of the subject estimate (person mattes, else the saliency heuristic) over the whole,
/// uncropped photo, in normalized mask coordinates. `None` for an empty source.
pub fn subject_point(src: &Rgb32f, s: &DevelopSettings, mattes: Option<&Mattes>) -> Option<craft_geom::Point> {
    if src.width == 0 || src.height == 0 || src.data.len() != src.width.saturating_mul(src.height) {
        return None;
    }
    let frame = Frame::new(src.width, src.height, s, false);
    let (w, h) = frame.fit(SUBJECT_EDGE, SUBJECT_EDGE);
    if w == 0 || h == 0 {
        return None;
    }
    let img = frame.sample(src, w, h);
    let log_l = img.map(crate::local::log_lum);
    let a = masks::shape_alpha(&MaskShape::Subject, &frame, w, h, &img, &log_l, 0.0, mattes);
    let (i, _) = a.data.iter().enumerate().filter(|(_, v)| v.is_finite()).max_by(|x, y| x.1.total_cmp(y.1))?;
    let n = frame.out_to_norm(w, h).apply(craft_geom::Point::new((i % w) as f64 + 0.5, (i / w) as f64 + 0.5));
    (n.x.is_finite() && n.y.is_finite()).then(|| craft_geom::Point::new(n.x.clamp(0.0, 1.0), n.y.clamp(0.0, 1.0)))
}

/// Blur what is out of focus in `img` (scene-linear, before exposure, at output resolution;
/// `frame` and `px_per_long` describe that output), then what each mask with a Lens Blur
/// adjustment selects (in mask order). Does nothing when no lens blur renders.
pub fn apply(img: &mut Rgb32f, s: &DevelopSettings, frame: &Frame, px_per_long: f64, mattes: Option<&Mattes>) {
    apply_with(img, s, frame, px_per_long, mattes, None);
}

/// [`apply`] with the photo's [`SubjectEstimate`] (renders pass [`crate::Plan::lens_subject`]).
pub fn apply_with(
    img: &mut Rgb32f,
    s: &DevelopSettings,
    frame: &Frame,
    px_per_long: f64,
    mattes: Option<&Mattes>,
    subject: Option<&SubjectEstimate>,
) {
    let (w, h) = (img.width, img.height);
    if w == 0 || h == 0 || img.data.len() != w.saturating_mul(h) {
        return;
    }
    // a zoomed view's place in the whole output: its coarse grids line up with the whole render's
    let at = |v: f64| if v.is_finite() && v > 0.0 { v.round() as usize } else { 0 };
    let origin = frame.view.map_or((0, 0), |v| (at(v.x), at(v.y)));
    let r = radius_px(s, px_per_long);
    if r >= 0.5 {
        let focus = crate::timed("lens blur focus", || focus_with(img, s, frame, mattes, subject));
        if focus.data.len() == img.data.len() {
            // the blur grows with the distance from what is in focus, as with depth
            let ramp = crate::timed("lens blur transition", || transition_ramp(&focus, transition_px(s, px_per_long), px_per_long, origin));
            let coc: Vec<f32> = focus.data.iter().enumerate().map(|(i, f)| (1.0 - f) * ramp.as_ref().map_or(1.0, |r| r.at(i))).collect();
            blur(img, &coc, r, boost_of(s.lens_blur.boost), origin);
        }
    }
    let ev = s.light.exposure as f32;
    for m in blurring_masks(s) {
        let r = mask_radius_px(m, px_per_long);
        if r < 0.5 {
            continue;
        }
        // the mask's strength (its Amount included) is how far out of focus a pixel is; blurring
        // the outside, it is how far the mask stays below its Amount
        let log_l = img.map(crate::local::log_lum);
        let alpha = crate::timed("lens blur mask", || masks::evaluate_one(m, frame, w, h, img, &log_l, ev, mattes));
        if alpha.data.len() != img.data.len() {
            continue;
        }
        let a = &m.adjust;
        let top = if a.amount.is_finite() { (a.amount / 100.0).clamp(0.0, 1.0) as f32 } else { 0.0 };
        let mut coc: Vec<f32> = alpha
            .data
            .iter()
            .map(|v| {
                let v = if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
                if a.lens_blur_outside { (top - v).max(0.0) } else { v }
            })
            .collect();
        // the blur grows away from what stays sharp, as the global one's does
        let sharp = Plane { width: w, height: h, data: coc.iter().map(|c| 1.0 - c).collect() };
        if let Some(ramp) = transition_ramp(&sharp, transition_of(a.lens_blur_transition, px_per_long), px_per_long, origin) {
            coc.iter_mut().enumerate().for_each(|(i, c)| *c *= ramp.at(i));
        }
        blur(img, &coc, r, boost_of(a.lens_blur_boost), origin);
    }
}

/// How much of the full blur each pixel gets for its distance from the focus: [`EDGE_BLUR`] at
/// the focus's edge, growing smoothly to all of it `d` pixels away.
struct Ramp {
    /// Distance from the focus, in grid cells.
    dist: Plane,
    /// Pixels per cell.
    g: usize,
    w: usize,
    d: f32,
    /// Where pixel (0, 0) sits in the grid (the grid follows the whole output's cells).
    shift: (usize, usize),
}

impl Ramp {
    /// The share of the full blur pixel `i` (row-major in the window) gets.
    fn at(&self, i: usize) -> f32 {
        let (x, y) = (i % self.w.max(1) + self.shift.0, i / self.w.max(1) + self.shift.1);
        let g = self.g as f32;
        // bilinear in the grid, cell centres at +0.5
        let (fx, fy) = (((x as f32 + 0.5) / g - 0.5).max(0.0), ((y as f32 + 0.5) / g - 0.5).max(0.0));
        let (gw, gh) = (self.dist.width, self.dist.height);
        let (x0, y0) = ((fx as usize).min(gw - 1), (fy as usize).min(gh - 1));
        let (x1, y1) = ((x0 + 1).min(gw - 1), (y0 + 1).min(gh - 1));
        let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
        let v = |x: usize, y: usize| self.dist.data.get(y * gw + x).copied().unwrap_or(f32::MAX);
        let top = v(x0, y0) + (v(x1, y0) - v(x0, y0)) * tx;
        let bot = v(x0, y1) + (v(x1, y1) - v(x0, y1)) * tx;
        let px = (top + (bot - top) * ty) * g;
        EDGE_BLUR + (1.0 - EDGE_BLUR) * smooth(0.0, self.d, px)
    }
}

/// The [`Ramp`] for `focus` over `d` pixels; `None` for a sharp cut (`d` 0) or nothing in focus.
/// Distances are chamfer distances (steps of 1 and √2 cells) on a grid of about
/// [`DISTANCE_CELLS`] along the photo's long edge (`px_per_long`: the cell size follows the
/// photo's scale, not the rendered window's, and its cells are the whole output's: `origin` is
/// where a window's pixel (0, 0) sits in it, so a zoomed view measures as the whole render does):
/// what is in focus are the cells mostly in focus.
fn transition_ramp(focus: &Plane, d: f32, px_per_long: f64, origin: (usize, usize)) -> Option<Ramp> {
    let (w, h) = (focus.width, focus.height);
    // (a NaN distance is no transition)
    if d.is_nan() || d <= 0.5 || w == 0 || h == 0 || focus.data.len() != w * h {
        return None;
    }
    let per_cell = px_per_long / DISTANCE_CELLS as f64;
    let g = if per_cell.is_finite() { (per_cell.ceil() as usize).clamp(1, w.max(h)) } else { 1 };
    let shift = (origin.0 % g, origin.1 % g);
    // the grid covers the window shifted into the whole output's cells
    let (gw, gh) = ((w + shift.0).div_ceil(g), (h + shift.1).div_ceil(g));
    let mut dist = Plane::new(gw, gh);
    let mut any = false;
    for cy in 0..gh {
        for cx in 0..gw {
            let (mut sum, mut n) = (0.0f32, 0usize);
            for vy in cy * g..((cy + 1) * g).min(h + shift.1) {
                for vx in cx * g..((cx + 1) * g).min(w + shift.0) {
                    let (Some(x), Some(y)) = (vx.checked_sub(shift.0), vy.checked_sub(shift.1)) else { continue };
                    sum += focus.data.get(y * w + x).copied().unwrap_or(0.0);
                    n += 1;
                }
            }
            let inside = n > 0 && sum / n as f32 > 0.5;
            any |= inside;
            if let Some(c) = dist.data.get_mut(cy * gw + cx) {
                *c = if inside { 0.0 } else { f32::MAX };
            }
        }
    }
    if !any {
        return None;
    }
    let diag = std::f32::consts::SQRT_2;
    let at = |d: &Plane, x: isize, y: isize| {
        if x < 0 || y < 0 || x >= gw as isize || y >= gh as isize {
            f32::MAX
        } else {
            d.data.get(y as usize * gw + x as usize).copied().unwrap_or(f32::MAX)
        }
    };
    // forward then backward chamfer passes
    for y in 0..gh as isize {
        for x in 0..gw as isize {
            let c = at(&dist, x, y)
                .min(at(&dist, x - 1, y) + 1.0)
                .min(at(&dist, x, y - 1) + 1.0)
                .min(at(&dist, x - 1, y - 1) + diag)
                .min(at(&dist, x + 1, y - 1) + diag);
            if let Some(v) = dist.data.get_mut(y as usize * gw + x as usize) {
                *v = c;
            }
        }
    }
    for y in (0..gh as isize).rev() {
        for x in (0..gw as isize).rev() {
            let c = at(&dist, x, y)
                .min(at(&dist, x + 1, y) + 1.0)
                .min(at(&dist, x, y + 1) + 1.0)
                .min(at(&dist, x + 1, y + 1) + diag)
                .min(at(&dist, x - 1, y + 1) + diag);
            if let Some(v) = dist.data.get_mut(y as usize * gw + x as usize) {
                *v = c;
            }
        }
    }
    Some(Ramp { dist, g, w, d, shift })
}

/// Blur `img` by a circle of confusion `coc · r` pixels per pixel (`coc` 0..1, one per pixel),
/// highlights brightened by `boost` (0..1) before they are spread.
/// `origin`: where `img`'s pixel (0, 0) sits in the whole output (a zoomed view's window), so the
/// coarse grids line up with the whole render's.
fn blur(img: &mut Rgb32f, coc: &[f32], r: f32, boost: f32, origin: (usize, usize)) {
    let w = img.width;
    // highlights brightened before they are spread (the bokeh), the rest as it is
    let gain = |c: [f32; 3]| 1.0 + BOOST_GAIN * boost * smooth(0.5, 1.0, crate::color::luminance_2020(c));
    let levels: Vec<Level> = (1..=LEVELS).map(|k| crate::timed("lens blur level", || level(img, coc, k, r, &gain, origin))).collect();
    // each pixel between the two levels around its circle of confusion (level 0: the image)
    let src = img.clone();
    crate::for_rows(&mut img.data, w, |y, row| {
        for (x, px) in row.iter_mut().enumerate() {
            let i = y * w + x;
            let t = coc.get(i).copied().unwrap_or(0.0) * LEVELS as f32;
            let k = (t.floor().max(0.0) as usize).min(LEVELS - 1);
            let fr = (t - k as f32).clamp(0.0, 1.0);
            let at = |k: usize| match k {
                0 => src.data.get(i).copied().unwrap_or(*px),
                k => levels.get(k - 1).map_or(*px, |l| l.sample(x, y)),
            };
            let (a, b) = (at(k), at(k + 1));
            *px = std::array::from_fn(|c| a[c] + (b[c] - a[c]) * fr);
        }
    });
}

/// One disc blur, on a grid `f` times coarser than the image.
struct Level {
    img: Rgb32f,
    f: usize,
    /// Where image pixel (0, 0) sits on the grid's pixels (the whole output's cells).
    shift: (usize, usize),
}

impl Level {
    /// The blurred value at image pixel `(x, y)`.
    fn sample(&self, x: usize, y: usize) -> [f32; 3] {
        let f = self.f as f32;
        self.img.sample_bilinear(((x + self.shift.0) as f32 + 0.5) / f, ((y + self.shift.1) as f32 + 0.5) / f)
    }
}

/// Level `k` of [`LEVELS`] (radius `k / LEVELS · r` pixels): the disc blur of the pixels at least
/// that far out of focus.
fn level(img: &Rgb32f, coc: &[f32], k: usize, r: f32, gain: &(impl Fn([f32; 3]) -> f32 + Sync), origin: (usize, usize)) -> Level {
    let (w, h) = (img.width, img.height);
    let radius = r * k as f32 / LEVELS as f32;
    let (f, shift, lw, lh) = level_grid(w, h, radius, origin);
    let (grid_w, grid_h) = (w.saturating_add(shift.0), h.saturating_add(shift.1));
    // a pixel counts in this level when its own circle of confusion reaches it
    let (lo, hi) = ((k - 1) as f32 / LEVELS as f32, k as f32 / LEVELS as f32);
    let mut cells = vec![[0f32; 4]; lw * lh];
    crate::for_rows(&mut cells, lw, |cy, row| {
        for (cx, cell) in row.iter_mut().enumerate() {
            let mut acc = [0f32; 4];
            for vy in cy * f..((cy + 1) * f).min(grid_h) {
                for vx in cx * f..((cx + 1) * f).min(grid_w) {
                    let (Some(x), Some(y)) = (vx.checked_sub(shift.0), vy.checked_sub(shift.1)) else { continue };
                    let i = y * w + x;
                    let (Some(c), Some(d)) = (img.data.get(i), coc.get(i)) else { continue };
                    // (a non-finite pixel would spread over the whole disc: left out)
                    if !c.iter().all(|v| v.is_finite()) {
                        continue;
                    }
                    let wt = FOCUS_WEIGHT + smooth(lo, hi, *d);
                    let g = gain(*c) * wt;
                    acc = [acc[0] + c[0] * g, acc[1] + c[1] * g, acc[2] + c[2] * g, acc[3] + wt];
                }
            }
            *cell = acc;
        }
    });
    Level { img: disc(&cells, lw, lh, radius / f as f32), f, shift }
}

/// Grid scale and dimensions for one disc level, bounded even when a window's grid is offset.
fn level_grid(w: usize, h: usize, radius: f32, origin: (usize, usize)) -> (usize, (usize, usize), usize, usize) {
    let at_least = ((w as f64 * h as f64) / MAX_CELLS as f64).sqrt().ceil() as usize;
    let max_edge = w.max(h).max(1);
    let mut f = ((radius / DISC_CELLS).ceil() as usize).max(at_least).clamp(1, max_edge);
    loop {
        let shift = (origin.0 % f, origin.1 % f);
        let (lw, lh) = (w.saturating_add(shift.0).div_ceil(f), h.saturating_add(shift.1).div_ceil(f));
        if lw.saturating_mul(lh) <= MAX_CELLS || f == max_edge {
            return (f, shift, lw, lh);
        }
        f = f.saturating_add(1);
    }
}

/// The normalized disc blur of radius `rho` cells of weighted sums `cells` (`[r·w, g·w, b·w, w]`,
/// `lw × lh`): per row of the disc, a span read from that row's prefix sums, its two end cells
/// weighted by the fraction of them inside.
fn disc(cells: &[[f32; 4]], lw: usize, lh: usize, rho: f32) -> Rgb32f {
    let stride = lw + 1;
    let mut pre = vec![[0f64; 4]; stride * lh];
    crate::for_rows(&mut pre, stride, |y, p| {
        let Some(row) = cells.get(y * lw..(y + 1) * lw) else { return };
        let mut acc = [0f64; 4];
        for (x, c) in row.iter().enumerate() {
            for ch in 0..4 {
                acc[ch] += c[ch] as f64;
            }
            if let Some(slot) = p.get_mut(x + 1) {
                *slot = acc;
            }
        }
    });
    // (a disc past the grid reads nothing more)
    let rho = if rho.is_finite() { rho.clamp(0.0, lw.max(lh) as f32 + 1.0) } else { 0.0 };
    let n = rho.ceil() as isize;
    let spans: Vec<(isize, isize, f64)> = (-n..=n)
        .filter_map(|dy| {
            let q = rho * rho - (dy * dy) as f32;
            (q >= 0.0).then(|| {
                let hw = q.sqrt();
                (dy, hw.floor() as isize, (hw - hw.floor()) as f64)
            })
        })
        .collect();
    let mut out = Rgb32f::new(lw, lh);
    let (lwi, lhi) = (lw as isize, lh as isize);
    crate::for_rows(&mut out.data, lw, |y, row| {
        for (x, px) in row.iter_mut().enumerate() {
            let x = x as isize;
            let mut acc = [0f64; 4];
            for &(dy, hw, fr) in &spans {
                let yy = y as isize + dy;
                if yy < 0 || yy >= lhi {
                    continue;
                }
                let base = yy as usize * stride;
                let (a, b) = ((x - hw).max(0), (x + hw + 1).min(lwi));
                if a < b
                    && let (Some(pa), Some(pb)) = (pre.get(base + a as usize), pre.get(base + b as usize))
                {
                    for ch in 0..4 {
                        acc[ch] += pb[ch] - pa[ch];
                    }
                }
                if fr > 0.0 {
                    for xx in [x - hw - 1, x + hw + 1] {
                        if xx >= 0
                            && xx < lwi
                            && let Some(c) = cells.get(yy as usize * lw + xx as usize)
                        {
                            for ch in 0..4 {
                                acc[ch] += fr * c[ch] as f64;
                            }
                        }
                    }
                }
            }
            *px = if acc[3] > 0.0 { std::array::from_fn(|c| (acc[c] / acc[3]) as f32) } else { [0.0; 3] };
        }
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft_develop_model::{MaskComponent, MaskOp};
    use craft_geom::Point;

    /// Settings with the lens blur switched on at `amount`.
    fn on(amount: f64) -> DevelopSettings {
        let mut s = DevelopSettings::default();
        s.lens_blur.enabled = true;
        s.lens_blur.amount = amount;
        s
    }

    /// A frame for a `w × h` source rendered at its own size.
    fn frame(s: &DevelopSettings, w: usize, h: usize) -> Frame {
        Frame::new(w, h, s, true)
    }

    /// A radial mask around the centre (radius `r`, long-edge units).
    fn centre_mask(id: u32, r: f64) -> Mask {
        let mut m = Mask { id, ..Mask::default() };
        m.components.push(MaskComponent {
            name: None,
            op: MaskOp::Add,
            invert: false,
            shape: MaskShape::Radial { center: Point::new(0.5, 0.5), rx: r, ry: r, angle: 0.0, feather: 0.0, invert: false },
        });
        m
    }

    /// A black image with one bright point at `(px, py)` and white stripes in the left third.
    fn scene(w: usize, h: usize, px: usize, py: usize) -> Rgb32f {
        let mut img = Rgb32f::from_fn(w, h, |x, _| if x < w / 3 && x % 4 < 2 { [0.6; 3] } else { [0.02; 3] });
        img.set(px, py, [4.0; 3]);
        img
    }

    /// On a 60 MP export, the strongest global and per-mask settings keep a fixed-size disc kernel
    /// and every level's coarse grid within its cell budget, including an offset render window.
    #[test]
    fn large_image_blur_work_is_bounded() {
        let (w, h) = (9500, 6316);
        let long_edge = w.max(h) as f64;
        let s = on(100.0);
        let r = radius_px(&s, long_edge);
        assert!((r - (MAX_RADIUS * long_edge) as f32).abs() < 1e-3);

        let mut mask = Mask::default();
        mask.adjust.lens_blur = 100.0;
        assert!((mask_radius_px(&mask, long_edge) - r).abs() < 1e-3);

        let mut total_cells = 0usize;
        for k in 1..=LEVELS {
            let radius = r * k as f32 / LEVELS as f32;
            let (f, _, lw, lh) = level_grid(w, h, radius, (17, 29));
            let cells = lw.saturating_mul(lh);
            assert!(cells <= MAX_CELLS, "level {k}: {lw} × {lh} = {cells}");
            total_cells = total_cells.saturating_add(cells);
            assert!(radius / f as f32 <= DISC_CELLS);
        }
        assert!(total_cells <= LEVELS * MAX_CELLS);

        let (_, _, lw, lh) = level_grid(w, h, r, (w - 1, h - 1));
        assert!(lw.saturating_mul(lh) <= MAX_CELLS, "offset window: {lw} × {lh}");
    }

    /// Switched off, at Amount 0 or with the section's eye closed, the photo is untouched.
    #[test]
    fn off_amount_zero_or_section_hidden_changes_nothing() {
        let img = scene(64, 48, 50, 10);
        for s in [DevelopSettings::default(), on(0.0), {
            let mut s = on(80.0);
            s.set_section_enabled("lensBlur", false);
            s
        }] {
            let mut out = img.clone();
            apply(&mut out, &s, &frame(&s, 64, 48), 64.0, None);
            assert_eq!(out, img);
            assert_eq!(key(&s), "");
        }
    }

    /// An out-of-focus point of light spreads into a disc of the expected radius and keeps its energy.
    #[test]
    fn out_of_focus_point_becomes_a_disc_and_keeps_its_energy() {
        let (w, h) = (200, 200);
        let mut s = on(100.0);
        // in focus: a small disc in the middle; the point sits far outside it
        s.masks.push(centre_mask(7, 0.05));
        s.lens_blur.focus_mask = Some(7);
        let mut img = Rgb32f::filled(w, h, [0.0; 3]);
        img.set(40, 40, [100.0; 3]);
        let before: f32 = img.data.iter().map(|c| c[1]).sum();
        apply(&mut img, &s, &frame(&s, w, h), w as f64, None);
        let r = radius_px(&s, w as f64);
        assert!((r - 7.0).abs() < 1e-3, "{r}");
        // spread over a disc: inside it lit, well outside it dark, the centre no longer a spike
        let at = |x: usize, y: usize| img.get(x, y)[1];
        assert!(at(40, 40) < 10.0, "{}", at(40, 40));
        for (x, y) in [(44, 40), (40, 36), (37, 43)] {
            assert!(at(x, y) > 0.2, "({x}, {y}) {}", at(x, y));
        }
        for (x, y) in [(52, 40), (40, 28), (50, 50)] {
            assert!(at(x, y) < 0.05, "({x}, {y}) {}", at(x, y));
        }
        let after: f32 = img.data.iter().map(|c| c[1]).sum();
        assert!((after - before).abs() / before < 0.15, "{before} → {after}");
    }

    /// What is in focus stays sharp, and its brightness doesn't bleed into the blurred background as a halo.
    #[test]
    fn the_focus_stays_sharp_and_does_not_bleed_into_the_background() {
        let (w, h) = (160, 160);
        let mut s = on(100.0);
        s.masks.push(centre_mask(3, 0.2));
        s.lens_blur.focus_mask = Some(3);
        // a bright subject filling the focus (a disc of 30 px; the focus is 32 px), grey stripes
        // around it
        let mut img = Rgb32f::from_fn(w, h, |x, y| {
            let d = ((x as f32 + 0.5 - 80.0).powi(2) + (y as f32 + 0.5 - 80.0).powi(2)).sqrt();
            if d < 30.0 {
                [2.0; 3]
            } else if x % 4 < 2 {
                [0.3; 3]
            } else {
                [0.1; 3]
            }
        });
        let src = img.clone();
        apply(&mut img, &s, &frame(&s, w, h), w as f64, None);
        // the subject is untouched
        assert_eq!(img.get(80, 80), src.get(80, 80));
        assert_eq!(img.get(60, 70), src.get(60, 70));
        // the background's stripes are smoothed out…
        let row: Vec<f32> = (4..40).map(|x| img.get(x, 10)[1]).collect();
        let (lo, hi) = row.iter().fold((f32::MAX, f32::MIN), |(a, b), v| (a.min(*v), b.max(*v)));
        assert!(hi - lo < 0.04, "{lo}..{hi}");
        // …to their mean, also 5 px from the bright subject (within the blur's reach): no halo
        let near = img.get(45, 80)[1];
        assert!((near - 0.2).abs() < 0.04, "{near}");
    }

    /// Boost brightens the blurred highlights only: a mid-grey background stays as it is.
    #[test]
    fn boost_brightens_bokeh_only() {
        let (w, h) = (120, 120);
        let mut s = on(60.0);
        s.masks.push(centre_mask(1, 0.05));
        s.lens_blur.focus_mask = Some(1);
        let mut img = Rgb32f::filled(w, h, [0.1; 3]);
        img.set(20, 20, [8.0; 3]);
        let (mut plain, mut boosted) = (img.clone(), img.clone());
        apply(&mut plain, &s, &frame(&s, w, h), w as f64, None);
        s.lens_blur.boost = 100.0;
        apply(&mut boosted, &s, &frame(&s, w, h), w as f64, None);
        assert!(boosted.get(22, 20)[0] > plain.get(22, 20)[0] * 2.0, "{} vs {}", boosted.get(22, 20)[0], plain.get(22, 20)[0]);
        // a mid-grey background stays as it is
        assert!((boosted.get(100, 100)[0] - plain.get(100, 100)[0]).abs() < 1e-4);
    }

    /// A small preview and a full-size export blur alike (the radius is relative to the long edge).
    #[test]
    fn previews_and_exports_blur_alike() {
        // the same photo at two sizes: the blur scales with the long edge
        let s = on(100.0);
        let big = scene(240, 180, 200, 30);
        let small = Rgb32f::from_fn(120, 90, |x, y| {
            let p = |dx, dy| big.get(2 * x + dx, 2 * y + dy);
            let q = [p(0, 0), p(1, 0), p(0, 1), p(1, 1)];
            std::array::from_fn(|c| q.iter().map(|v| v[c]).sum::<f32>() / 4.0)
        });
        let (mut a, mut b) = (big.clone(), small.clone());
        apply(&mut a, &s, &frame(&s, 240, 180), 240.0, None);
        apply(&mut b, &s, &frame(&s, 120, 90), 120.0, None);
        let mut diff = 0.0;
        for y in 0..90 {
            for x in 0..120 {
                let d = a.get(2 * x, 2 * y)[1] - b.get(x, y)[1];
                diff += d.abs();
            }
        }
        let mean = diff / (120.0 * 90.0);
        assert!(mean < 0.02, "{mean}");
    }

    /// Brush Refinement: a Focus stroke keeps what it covers sharp, a Blur stroke blurs what it
    /// covers even inside the focus.
    #[test]
    fn focus_and_blur_brushes_refine_the_focus() {
        let (w, h) = (160, 120);
        let img = Rgb32f::from_fn(w, h, |x, _| if x % 4 < 2 { [0.3; 3] } else { [0.1; 3] });
        let mut s = on(100.0);
        // in focus: the middle
        s.masks.push(centre_mask(5, 0.15));
        s.lens_blur.focus_mask = Some(5);
        let dab = |x: f64, y: f64| BrushStroke { points: vec![Point::new(x, y)], size: 0.08, feather: 0.0, ..BrushStroke::default() };
        // Focus brush on the top-left corner (out of focus otherwise), Blur brush on the centre
        s.lens_blur.focus_strokes.push(dab(0.1, 0.12));
        s.lens_blur.blur_strokes.push(dab(0.5, 0.5));
        let mut out = img.clone();
        apply(&mut out, &s, &frame(&s, w, h), w as f64, None);
        let stripes = |o: &Rgb32f, cx: usize, cy: usize| {
            let v: Vec<f32> = (cx - 4..cx + 4).map(|x| o.get(x, cy)[1]).collect();
            v.iter().fold(f32::MIN, |a, b| a.max(*b)) - v.iter().fold(f32::MAX, |a, b| a.min(*b))
        };
        // the corner the Focus brush painted keeps its stripes; one it didn't is smooth
        assert!(stripes(&out, 16, 14) > 0.15, "{}", stripes(&out, 16, 14));
        assert!(stripes(&out, 140, 100) < 0.05, "{}", stripes(&out, 140, 100));
        // the centre is in focus, but the Blur brush wins
        assert!(stripes(&out, 80, 60) < 0.05, "{}", stripes(&out, 80, 60));
        // without the strokes, the centre stays sharp
        let mut plain = s.clone();
        plain.lens_blur.focus_strokes.clear();
        plain.lens_blur.blur_strokes.clear();
        let mut out = img.clone();
        apply(&mut out, &plain, &frame(&plain, w, h), w as f64, None);
        assert!(stripes(&out, 80, 60) > 0.15);
        assert_ne!(key(&plain), key(&s));
    }

    /// Without a focus mask, a fixed shape keeps a circle, a wide or a tall oval sharp in the
    /// middle: a wide one reaches farther sideways than up, a tall one the other way round.
    #[test]
    fn fixed_focus_shapes() {
        use craft_develop_model::FocusShape;
        let (w, h) = (300, 300);
        let s0 = on(100.0);
        let fr = frame(&s0, w, h);
        let img = Rgb32f::filled(w, h, [0.2; 3]);
        let alpha = |shape: FocusShape| {
            let mut s = s0.clone();
            s.lens_blur.shape = shape;
            focus(&img, &s, &fr, None)
        };
        // sideways and upwards from the centre, 0.2 of the long edge away
        let side = |a: &Plane| a.get(150 + 60, 150);
        let up = |a: &Plane| a.get(150, 150 - 60);
        let (round, wide, tall) = (alpha(FocusShape::Round), alpha(FocusShape::Wide), alpha(FocusShape::Tall));
        for a in [&round, &wide, &tall] {
            assert!(a.get(150, 150) > 0.99, "the middle is in focus");
            assert!(a.get(5, 5) < 0.01, "a corner isn't");
        }
        assert!((side(&round) - up(&round)).abs() < 0.05);
        assert!(side(&wide) > 0.9 && up(&wide) < 0.1, "{} {}", side(&wide), up(&wide));
        assert!(up(&tall) > 0.9 && side(&tall) < 0.1, "{} {}", side(&tall), up(&tall));
        // moved and resized: the area follows; hostile values are tamed
        let mut s = s0.clone();
        s.lens_blur.shape = FocusShape::Round;
        s.lens_blur.area = Some(craft_develop_model::FocusArea { center: Point::new(0.2, 0.2), rx: 0.1, ry: 0.1 });
        let a = focus(&img, &s, &fr, None);
        assert!(a.get(60, 60) > 0.99 && a.get(150, 150) < 0.01, "moved to the top left");
        s.lens_blur.area = Some(craft_develop_model::FocusArea { center: Point::new(f64::NAN, 9e9), rx: f64::INFINITY, ry: -3.0 });
        let a = focus(&img, &s, &fr, None);
        assert!(a.data.iter().all(|v| v.is_finite()));
        // an unknown shape in the JSON reads as Auto (never an error)
        let b: craft_develop_model::LensBlur = serde_json::from_value(serde_json::json!({"shape": "hexagon"})).unwrap();
        assert_eq!(b.shape, FocusShape::Auto);
    }

    /// With a Transition the blur grows with the distance from the focus, as a lens renders what
    /// is near the subject: soft right beside it, full far away. At 0 it is a sharp cut.
    #[test]
    fn the_blur_grows_away_from_the_focus() {
        let (w, h) = (400, 200);
        // wide stripes: how much of them a blur leaves says how large it is
        let img = Rgb32f::from_fn(w, h, |x, _| if (x / 12) % 2 == 0 { [0.3; 3] } else { [0.1; 3] });
        let mut s = on(100.0);
        // in focus: a disc left of centre
        let mut m = centre_mask(2, 0.08);
        if let MaskShape::Radial { center, .. } = &mut m.components[0].shape {
            *center = Point::new(0.2, 0.5);
        }
        s.masks.push(m);
        s.lens_blur.focus_mask = Some(2);
        // how much detail is left (stripe contrast) around x, on the middle row
        let detail = |o: &Rgb32f, x: usize| {
            let v: Vec<f32> = (x - 12..x + 12).map(|x| o.get(x, 100)[1]).collect();
            v.iter().fold(f32::MIN, |a, b| a.max(*b)) - v.iter().fold(f32::MAX, |a, b| a.min(*b))
        };
        let render = |t: f64| {
            let mut s = s.clone();
            s.lens_blur.transition = t;
            let mut o = img.clone();
            apply(&mut o, &s, &frame(&s, w, h), w as f64, None);
            o
        };
        let (soft, cut) = (render(100.0), render(0.0));
        // the focus disc ends at x ≈ 112 (the ramp is 60 px long): just past it, then farther
        let (near, mid, far) = (detail(&soft, 130), detail(&soft, 150), detail(&soft, 330));
        assert!(near > mid + 0.02 && mid > far + 0.02, "{near} > {mid} > {far}");
        // a sharp cut blurs right beside the focus as much as far away
        let cut_near = detail(&cut, 130);
        assert!((cut_near - detail(&cut, 330)).abs() < 0.02 && near > cut_near + 0.05, "{cut_near} vs {near}");
        // the focus itself stays sharp either way
        assert!(detail(&soft, 70) > 0.19 && detail(&cut, 70) > 0.19);
    }

    /// A focus mask whose AI selection isn't computed yet (Find Subject still running) leaves the
    /// subject estimate in focus: the photo is never blurred all over meanwhile.
    #[test]
    fn a_pending_ai_focus_mask_falls_back_to_the_subject_estimate() {
        let (w, h) = (160, 120);
        let img = Rgb32f::from_fn(w, h, |x, _| if x % 4 < 2 { [0.3; 3] } else { [0.1; 3] });
        let mut s = on(100.0);
        let mut m = Mask { id: 9, ..Mask::default() };
        let pending = MaskShape::Object { hint: vec![], exclude: vec![], seg: None, detail: vec![], edge: 0.0 };
        m.components.push(MaskComponent { name: None, op: MaskOp::Add, invert: false, shape: pending });
        assert!(awaiting_ai(&m));
        s.masks.push(m);
        s.lens_blur.focus_mask = Some(9);
        let mut subject = s.clone();
        subject.lens_blur.focus_mask = None;
        let (mut a, mut b) = (img.clone(), img.clone());
        apply(&mut a, &s, &frame(&s, w, h), w as f64, None);
        apply(&mut b, &subject, &frame(&subject, w, h), w as f64, None);
        assert_eq!(a, b, "the same as no focus mask");
        // the middle (the estimate's subject) keeps its stripes
        let v: Vec<f32> = (76..84).map(|x| a.get(x, 60)[1]).collect();
        assert!(v.iter().fold(f32::MIN, |p, q| p.max(*q)) - v.iter().fold(f32::MAX, |p, q| p.min(*q)) > 0.1, "{v:?}");
    }

    /// A mask with Lens Blur blurs what it selects and nothing else; hidden, it blurs nothing.
    #[test]
    fn a_mask_blurs_only_what_it_selects() {
        let (w, h) = (160, 120);
        // vertical stripes everywhere
        let img = Rgb32f::from_fn(w, h, |x, _| if x % 4 < 2 { [0.3; 3] } else { [0.1; 3] });
        let mut s = DevelopSettings::default();
        // the left half (a linear gradient with a short ramp in the middle)
        let mut m = Mask { id: 2, ..Mask::default() };
        m.components.push(MaskComponent {
            name: None,
            op: MaskOp::Add,
            invert: false,
            shape: MaskShape::Linear { start: Point::new(0.45, 0.5), end: Point::new(0.55, 0.5) },
        });
        m.adjust.lens_blur = 100.0;
        s.masks.push(m);
        assert!(renders(&s) && !active(&s) && !key(&s).is_empty());
        let mut out = img.clone();
        apply(&mut out, &s, &frame(&s, w, h), w as f64, None);
        // left: the stripes are smoothed to their mean
        let left: Vec<f32> = (8..50).map(|x| out.get(x, 60)[1]).collect();
        let (lo, hi) = left.iter().fold((f32::MAX, f32::MIN), |(a, b), v| (a.min(*v), b.max(*v)));
        assert!(hi - lo < 0.05 && (lo + hi) / 2.0 > 0.15 && (lo + hi) / 2.0 < 0.25, "{lo}..{hi}");
        // right: untouched
        for x in 100..150 {
            assert_eq!(out.get(x, 60), img.get(x, 60), "x {x}");
        }
        // the window grows by the mask's blur
        let win = crate::PixelWindow { x: 60, y: 40, w: 20, h: 20 };
        assert!(window_for_reads(&s, w, h, w as f64, win).w > 20);
        // hidden or at 0: nothing
        for change in [|m: &mut Mask| m.visible = false, |m: &mut Mask| m.adjust.lens_blur = 0.0, |m: &mut Mask| m.adjust.lens_blur_off = true] {
            let mut t = s.clone();
            change(&mut t.masks[0]);
            let mut out = img.clone();
            apply(&mut out, &t, &frame(&t, w, h), w as f64, None);
            assert_eq!(out, img);
            assert!(!renders(&t) && key(&t).is_empty());
        }
    }

    /// A mask's Lens Blur block can blur what the mask leaves out, and its Transition makes that
    /// blur grow away from what stays sharp.
    #[test]
    fn a_mask_blurs_its_outside_gradually() {
        let (w, h) = (320, 120);
        let img = Rgb32f::from_fn(w, h, |x, _| if x % 4 < 2 { [0.3; 3] } else { [0.1; 3] });
        let mut s = DevelopSettings::default();
        // the left third selected
        let mut m = Mask { id: 2, ..Mask::default() };
        m.components.push(MaskComponent {
            name: None,
            op: MaskOp::Add,
            invert: false,
            shape: MaskShape::Linear { start: Point::new(0.30, 0.5), end: Point::new(0.34, 0.5) },
        });
        m.adjust.lens_blur = 100.0;
        m.adjust.lens_blur_outside = true;
        s.masks.push(m);
        // how much of the stripes is left around column x
        let spread = |out: &Rgb32f, x: usize| {
            let v: Vec<f32> = (x..x + 8).map(|x| out.get(x, 60)[1]).collect();
            v.iter().fold(f32::MIN, |a, b| a.max(*b)) - v.iter().fold(f32::MAX, |a, b| a.min(*b))
        };
        let mut sharp_cut = img.clone();
        apply(&mut sharp_cut, &s, &frame(&s, w, h), w as f64, None);
        // the selection stays sharp, the rest is blurred
        for x in 8..80 {
            assert_eq!(sharp_cut.get(x, 60), img.get(x, 60), "x {x}");
        }
        assert!(spread(&sharp_cut, 150) < 0.05 && spread(&sharp_cut, 290) < 0.05);
        // with a Transition, near the selection keeps more of the stripes than far from it
        s.masks[0].adjust.lens_blur_transition = 100.0;
        s.masks[0].adjust.lens_blur_boost = 50.0;
        let mut gradual = img.clone();
        apply(&mut gradual, &s, &frame(&s, w, h), w as f64, None);
        assert!(spread(&gradual, 112) > spread(&sharp_cut, 112) + 0.02, "{} vs {}", spread(&gradual, 112), spread(&sharp_cut, 112));
        assert!(spread(&gradual, 112) > spread(&gradual, 290), "{} vs {}", spread(&gradual, 112), spread(&gradual, 290));
        // the window reads farther with the transition
        let win = crate::PixelWindow { x: 200, y: 40, w: 20, h: 20 };
        let mut no_t = s.clone();
        no_t.masks[0].adjust.lens_blur_transition = 0.0;
        assert!(window_for_reads(&s, w, h, w as f64, win).w > window_for_reads(&no_t, w, h, w as f64, win).w);
        // at Amount 0 the outside isn't blurred either
        s.masks[0].adjust.amount = 0.0;
        let mut out = img.clone();
        apply(&mut out, &s, &frame(&s, w, h), w as f64, None);
        assert_eq!(out, img);
        // hostile values
        s.masks[0].adjust.amount = f64::NAN;
        s.masks[0].adjust.lens_blur_transition = f64::INFINITY;
        s.masks[0].adjust.lens_blur_boost = f64::NAN;
        let mut out = img.clone();
        apply(&mut out, &s, &frame(&s, w, h), w as f64, None);
        assert!(out.data.iter().all(|p| p.iter().all(|v| v.is_finite())));
    }

    /// The subject point lands on a person matte when the photo carries one, and is always inside
    /// the photo; an empty source has none.
    #[test]
    fn subject_point_finds_the_person_matte() {
        use crate::masks::{MatteKind, Mattes};
        let s = DevelopSettings::default();
        let src = Rgb32f::filled(300, 200, [0.2; 3]);
        // a person in the top-right corner, far from the centre the heuristic favours
        let mut person = crate::raster::Image::<u8>::new(300, 200);
        for y in 10..60 {
            for x in 220..290 {
                person.set(x, y, 255);
            }
        }
        let mut mattes = Mattes::default();
        mattes.push(MatteKind::Person, person);
        let p = subject_point(&src, &s, Some(&mattes)).unwrap();
        assert!(p.x > 0.72 && p.x < 0.97 && p.y > 0.04 && p.y < 0.3, "{p:?}");
        // without mattes: somewhere in the photo (the heuristic's centre-weighted peak)
        let p = subject_point(&src, &s, None).unwrap();
        assert!((0.0..=1.0).contains(&p.x) && (0.0..=1.0).contains(&p.y), "{p:?}");
        assert!(subject_point(&Rgb32f::new(0, 0), &s, None).is_none());
    }

    /// Hostile settings and images: no panic, no NaN.
    #[test]
    fn hostile_input_never_panics() {
        for amount in [f64::NAN, f64::INFINITY, -5.0, 1e9] {
            let mut s = on(amount);
            s.lens_blur.boost = f64::NAN;
            s.lens_blur.focus_mask = Some(99);
            for (w, h) in [(1, 1), (1, 7), (9, 1), (33, 5)] {
                let mut img = Rgb32f::filled(w, h, [0.5, f32::NAN, f32::INFINITY]);
                apply(&mut img, &s, &frame(&s, w, h), 1e6, None);
                let mut img = Rgb32f::filled(w, h, [0.5; 3]);
                apply(&mut img, &s, &frame(&s, w, h), w.max(h) as f64, None);
                assert!(img.data.iter().flatten().all(|v| v.is_finite()), "{amount} {w}×{h}");
            }
        }
        // a mismatched buffer is left alone
        let s = on(50.0);
        let mut img = Rgb32f { width: 10, height: 10, data: vec![[0.5; 3]; 7] };
        apply(&mut img, &s, &frame(&s, 10, 10), 10.0, None);
        assert_eq!(img.data.len(), 7);
    }
}
