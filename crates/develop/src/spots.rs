//! Remove / Heal / Clone spots, rendered at output resolution in scene-linear light.
//!
//! - **Clone** copies the source patch with a feathered edge.
//! - **Heal** copies the source's *detail* and keeps the target's *tone*: result = src + blur(target − src)
//!   (frequency separation — a fast, stable approximation of gradient-domain healing).
//! - **Remove** is Heal with an automatically chosen source: candidate offsets on rings around the
//!   spot are scored by how well the source's surrounding annulus matches the target's.
//! - **Fill** (content-aware) rebuilds the spot from patches of its surroundings instead of one
//!   source: PatchMatch-based completion ([`crate::inpaint::complete`]), coarse to fine, so the
//!   structure is chosen at a scale set by the spot's size (the same at preview and export size).
//!   Deterministic; the source offset is ignored.

use crate::raster::Rgb32f;
use craft_develop_model::{Spot, SpotMode};
use craft_geom::Point;

use crate::geometry::Frame;

#[inline]
fn smooth(e0: f32, e1: f32, x: f32) -> f32 {
    // Zero feather is a hard edge, including when the pixel lies exactly on the radius.
    // Dividing 0 by 0 here would give that pixel a NaN alpha, even at zero opacity.
    if e0 >= e1 {
        return if x < e1 { 0.0 } else { 1.0 };
    }
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Most a spot's size counts for, in long edges: a size read from a sidecar isn't held to the slider's range, and one
/// far past the picture (or infinite) made the radius, the blur it heals with and the patch it reads from unbounded
/// (issue storytold/lightcraft#1519). Two long edges cover the whole frame from anywhere in it.
const MAX_SPOT_SIZE: f64 = 2.0;

/// A spot's radius in output pixels, `ppl` output pixels to the long edge: at least a pixel (a size that is not a
/// number too), at most [`MAX_SPOT_SIZE`] long edges.
fn radius_px(size: f64, ppl: f64) -> f64 {
    let size = if size.is_nan() { 0.0 } else { size.min(MAX_SPOT_SIZE) };
    (size * ppl).max(1.0)
}

/// Directions (cos, sin) of the 24 spokes [`ring_stats`] samples along.
pub fn ring_dirs() -> [(f32, f32); 24] {
    std::array::from_fn(|i| {
        let a = i as f32 / 24.0 * std::f32::consts::TAU;
        (a.cos(), a.sin())
    })
}

/// The radii (inner, middle, outer) of the rings a source is scored on for a spot of radius `r`:
/// the surroundings (compared between target and source) and the inside of the source.
pub fn score_radii(r: f32) -> [[f32; 3]; 2] {
    let ring = |r0: f32, r1: f32| [r0, (r0 + r1) / 2.0, r1];
    [ring(r * 1.05, r * 1.6), ring(r * 0.2, r * 0.9)]
}

/// Samples of `img` along the spokes at three radii (spoke by spoke); `None` when one falls
/// outside the image.
fn ring_stats(img: &Rgb32f, c: (f32, f32), radii: [f32; 3]) -> Option<Vec<[f32; 3]>> {
    let mut out = Vec::new();
    for (cos, sin) in ring_dirs() {
        for rr in radii {
            let (x, y) = (c.0 + cos * rr, c.1 + sin * rr);
            if x < 0.0 || y < 0.0 || x >= img.width as f32 || y >= img.height as f32 {
                return None;
            }
            out.push(img.sample_bilinear(x, y));
        }
    }
    Some(out)
}

fn score(img: &Rgb32f, target: (f32, f32), source: (f32, f32), r: f32) -> f32 {
    let [around, within] = score_radii(r);
    let (Some(a), Some(b)) = (ring_stats(img, target, around), ring_stats(img, source, around)) else { return f32::MAX };
    // also compare the inside of the source against the target's surroundings (texture continuity)
    let inside = ring_stats(img, source, within);
    let mut s = 0.0;
    for (p, q) in a.iter().zip(&b) {
        for c in 0..3 {
            let (u, v) = (p[c].max(0.0).sqrt(), q[c].max(0.0).sqrt());
            s += (u - v) * (u - v);
        }
    }
    if let Some(ins) = inside {
        let mean_a: f32 = a.iter().map(|p| p[1].max(0.0).sqrt()).sum::<f32>() / a.len() as f32;
        let var: f32 = ins.iter().map(|p| (p[1].max(0.0).sqrt() - mean_a).powi(2)).sum::<f32>() / ins.len() as f32;
        s += var * a.len() as f32 * 0.5;
    }
    s
}

/// Candidate source offsets (output pixels) for a spot of radius `r` at `target`, best first
/// (candidates reaching outside the image are left out).
pub fn ranked_sources(img: &Rgb32f, target: (f32, f32), r: f32) -> Vec<(f32, f32)> {
    let scores: Vec<f32> = candidates(r).iter().map(|d| score(img, target, (target.0 + d.0, target.1 + d.1), r)).collect();
    rank(&candidates(r), &scores)
}

/// The source offsets [`ranked_sources`] tries for a spot of radius `r`, in order: 16 directions
/// on each of three rings.
pub fn candidates(r: f32) -> Vec<(f32, f32)> {
    let mut c = Vec::with_capacity(48);
    for ring in [2.4f32, 3.2, 4.2] {
        for i in 0..16 {
            let a = i as f32 / 16.0 * std::f32::consts::TAU;
            c.push((a.cos() * r * ring, a.sin() * r * ring));
        }
    }
    c
}

/// `candidates` ordered by `scores` (best first, ties in candidate order), leaving out those
/// that scored `f32::MAX` (they reach outside the image).
pub fn rank(candidates: &[(f32, f32)], scores: &[f32]) -> Vec<(f32, f32)> {
    let mut c: Vec<(f32, (f32, f32))> = scores.iter().zip(candidates).filter(|(s, _)| **s < f32::MAX).map(|(s, d)| (*s, *d)).collect();
    c.sort_by(|a, b| a.0.total_cmp(&b.0));
    c.into_iter().map(|(_, d)| d).collect()
}

/// The offset used when no candidate fits in the image.
pub fn fallback_source(r: f32) -> (f32, f32) {
    (r * 2.5, 0.0)
}

/// Pick a source offset (in output pixels) for a remove/heal spot.
pub fn auto_source(img: &Rgb32f, target: (f32, f32), r: f32) -> (f32, f32) {
    ranked_sources(img, target, r).first().copied().unwrap_or_else(|| fallback_source(r))
}

/// A source offset (normalized, as [`Spot::source_offset`]) for `spot` on `src` developed with
/// `s`: the best match, or with `avoid` (the current offset) the best one at least a spot radius
/// away from it ("refresh source"). `None` when no candidate fits in the image.
pub fn pick_source(
    src: &Rgb32f,
    info: &crate::SourceInfo,
    s: &craft_develop_model::DevelopSettings,
    spot: &Spot,
    avoid: Option<Point>,
) -> Option<Point> {
    let frame = crate::frame_for(src, info, s, true);
    let (w, h) = frame.fit(512, 512);
    if w == 0 || h == 0 {
        return None;
    }
    let img = frame.sample(src, w, h);
    let (to_out, to_norm) = (frame.norm_to_out(w, h), frame.out_to_norm(w, h));
    let t = to_out.apply(*spot.points.first()?);
    let r = radius_px(spot.size, frame.px_per_long(w)) as f32;
    let avoid = avoid.map(|o| {
        let (a, b) = (to_out.apply(Point::new(0.0, 0.0)), to_out.apply(o));
        ((b.x - a.x) as f32, (b.y - a.y) as f32)
    });
    let far = |d: &(f32, f32)| avoid.is_none_or(|a| ((d.0 - a.0).powi(2) + (d.1 - a.1).powi(2)).sqrt() > r);
    let d = ranked_sources(&img, (t.x as f32, t.y as f32), r).into_iter().find(far)?;
    let (n0, n1) = (to_norm.apply(t), to_norm.apply(Point::new(t.x + d.0 as f64, t.y + d.1 as f64)));
    Some(Point::new(n1.x - n0.x, n1.y - n0.y))
}

/// The most a window grows to take in the spots that touch it.
const MAX_SPOT_SPAN: usize = if cfg!(target_arch = "wasm32") { 4096 } else { 8192 };
/// Strokes with more dabs than this are not examined dab by dab (the render would be too slow anyway).
const MAX_DABS_CONSIDERED: usize = 200_000;

/// `win` (a window of the `full_w × full_h` output of `frame`) grown to hold, for every spot (or
/// Auto Mask stroke) that reaches into it, the whole spot and the patch it reads from: [`apply`] works on the pixels it
/// is given, so a spot cut by the window's edge, or whose source lies outside it, would come out
/// different from the same spot in the whole render. Spots with an automatic source (`None`)
/// search up to 4.2 radii around themselves. Growth that would pass [`MAX_SPOT_SPAN`] pixels is not
/// done (the window comes back as asked; see [`window_for_reads_checked`] to know).
pub fn window_for_reads(
    s: &craft_develop_model::DevelopSettings,
    frame: &Frame,
    full_w: usize,
    full_h: usize,
    ppl: f64,
    win: crate::PixelWindow,
) -> crate::PixelWindow {
    window_for_reads_checked(s, frame, full_w, full_h, ppl, win).unwrap_or(win)
}

/// [`window_for_reads`], or `None` when holding everything the window reads would take more than
/// [`MAX_SPOT_SPAN`] pixels along an axis: a caller that can show something else (the loupe's
/// whole-frame render) must, because the window alone would come out wrong.
pub fn window_for_reads_checked(
    s: &craft_develop_model::DevelopSettings,
    frame: &Frame,
    full_w: usize,
    full_h: usize,
    ppl: f64,
    win: crate::PixelWindow,
) -> Option<crate::PixelWindow> {
    let spots = &s.spots;
    // (with whether its mask's coordinates are on the photo before the warp)
    let autos: Vec<(bool, &craft_develop_model::BrushStroke)> = s
        .masks
        .iter()
        .filter(|m| m.visible)
        .flat_map(|m| m.components.iter().map(move |c| (m.on_photo, c)))
        .filter_map(|(on, c)| match &c.shape {
            craft_develop_model::MaskShape::Brush { strokes } => Some(strokes.iter().map(move |st| (on, st))),
            _ => None,
        })
        .flatten()
        .filter(|(_, st)| st.auto_mask)
        .collect();
    if spots.is_empty() && autos.is_empty() {
        return Some(win);
    }
    let to_out = frame.norm_to_out(full_w, full_h);
    let origin = to_out.apply(Point::new(0.0, 0.0));
    // (target box, source box) of each spot, in output pixels: [x0, y0, x1, y1]
    let boxes: Vec<([f64; 4], [f64; 4])> = spots
        .iter()
        .filter_map(|spot| {
            let r = radius_px(spot.size, ppl);
            let pts: Vec<_> = spot.points.iter().map(|p| to_out.apply(*p)).collect();
            let first = pts.first()?;
            let (mut x0, mut y0, mut x1, mut y1) = (first.x, first.y, first.x, first.y);
            for p in &pts {
                (x0, y0, x1, y1) = (x0.min(p.x), y0.min(p.y), x1.max(p.x), y1.max(p.y));
            }
            // the box `apply` works in: the stroke plus a radius, plus a radius of padding
            let t = [x0 - 2.0 * r, y0 - 2.0 * r, x1 + 2.0 * r, y1 + 2.0 * r];
            let s = match spot.source_offset {
                Some(o) => {
                    let b = to_out.apply(o);
                    let (dx, dy) = (b.x - origin.x, b.y - origin.y);
                    [t[0] + dx, t[1] + dy, t[2] + dx, t[3] + dy]
                }
                None => [t[0] - 4.2 * r, t[1] - 4.2 * r, t[2] + 4.2 * r, t[3] + 4.2 * r],
            };
            Some((t, s)).filter(|(t, s)| t.iter().chain(s).all(|v| v.is_finite()))
        })
        .collect();
    // Auto Mask strokes weigh each dab by its similarity to the pixel under the dab's centre and
    // refine the stroke over a couple of radii: each dab matters to pixels within 2.5 radii
    let mut dab_boxes: Vec<[f64; 4]> = Vec::new();
    for (on_photo, st) in autos {
        // One more than the window takes, so a stroke past it is still recognised as past it
        // without the other 1.8 million dabs being built first (storytold/lightcraft#1520).
        let dabs = crate::masks::brush_dabs_within(st, &frame.for_mask(on_photo), full_w, full_h, MAX_DABS_CONSIDERED + 1);
        let r = dabs.r;
        if dabs.dabs.len() > MAX_DABS_CONSIDERED {
            continue;
        }
        for d in &dabs.dabs {
            let b = [d.x - 2.5 * r, d.y - 2.5 * r, d.x + 2.5 * r, d.y + 2.5 * r];
            if b.iter().all(|v| v.is_finite()) {
                dab_boxes.push(b);
            }
        }
    }
    let (fw, fh) = (full_w as f64, full_h as f64);
    let mut r = [win.x as f64, win.y as f64, (win.x + win.w) as f64, (win.y + win.h) as f64];
    let touches = |r: &[f64; 4], b: &[f64; 4]| b[0] < r[2] && b[2] > r[0] && b[1] < r[3] && b[3] > r[1];
    // a dab reaching the requested window needs its surroundings, once: the pixels that brings in
    // are only read, not drawn on (unlike spots, whose patches other spots may read)
    let asked = r;
    for b in dab_boxes.iter().filter(|b| touches(&asked, b)) {
        r = [r[0].min(b[0]), r[1].min(b[1]), r[2].max(b[2]), r[3].max(b[3])];
    }
    r = [r[0].max(0.0), r[1].max(0.0), r[2].min(fw), r[3].min(fh)];
    if r[2] - r[0] > MAX_SPOT_SPAN as f64 || r[3] - r[1] > MAX_SPOT_SPAN as f64 {
        return None;
    }
    for _ in 0..=boxes.len() {
        let mut grown = r;
        for (t, s) in &boxes {
            if touches(&r, t) {
                for b in [t, s] {
                    grown = [grown[0].min(b[0]), grown[1].min(b[1]), grown[2].max(b[2]), grown[3].max(b[3])];
                }
            }
        }
        let grown = [grown[0].max(0.0), grown[1].max(0.0), grown[2].min(fw), grown[3].min(fh)];
        if grown == r {
            break;
        }
        if grown[2] - grown[0] > MAX_SPOT_SPAN as f64 || grown[3] - grown[1] > MAX_SPOT_SPAN as f64 {
            return None;
        }
        r = grown;
    }
    let (x, y) = (r[0].floor() as usize, r[1].floor() as usize);
    Some(
        crate::PixelWindow { x, y, w: (r[2].ceil() as usize).saturating_sub(x).max(1), h: (r[3].ceil() as usize).saturating_sub(y).max(1) }
            .clamped(full_w, full_h),
    )
}

/// One spot as both renderers lay it out, in output pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct SpotLayout {
    /// Radius.
    pub r: f32,
    /// The stroke's points.
    pub pts: Vec<(f32, f32)>,
    /// Where the patch is read from, relative to the target; `None`: chosen automatically
    /// ([`auto_source`] around the first point, on the image as the earlier spots left it).
    pub offset: Option<(f32, f32)>,
    /// The box the spot works in: left, top, width, height (inside the image, never empty).
    pub bx: usize,
    pub by: usize,
    pub bw: usize,
    pub bh: usize,
    /// Alpha at distance d from the stroke: `(1 − smoothstep(inner, r, d)) · opacity`.
    pub inner: f32,
    pub opacity: f32,
    /// Heal / Remove (tone from the surroundings) rather than Clone or Fill.
    pub heal: bool,
    /// Content-aware Fill: the stroke is rebuilt from the box around it (CPU only; the GPU
    /// renderer leaves photos with a Fill spot to the CPU).
    pub fill: bool,
}

/// `spot`'s layout in a `w × h` output of `frame` (`ppl` = output pixels per long-edge unit);
/// `None` when it has no points or its box misses the image.
pub fn layout(spot: &Spot, frame: &Frame, w: usize, h: usize, ppl: f64) -> Option<SpotLayout> {
    let to_out = frame.norm_to_out(w, h);
    let r = radius_px(spot.size, ppl) as f32;
    let pts: Vec<(f32, f32)> = spot
        .points
        .iter()
        .map(|p| {
            let q = to_out.apply(*p);
            (q.x as f32, q.y as f32)
        })
        .collect();
    pts.first()?;
    let fill = spot.mode == SpotMode::Fill;
    let offset = if fill {
        Some((0.0, 0.0))
    } else {
        spot.source_offset.map(|o| {
            let a = to_out.apply(Point::new(0.0, 0.0));
            let b = to_out.apply(o);
            ((b.x - a.x) as f32, (b.y - a.y) as f32)
        })
    };
    // Bounding box of the stroke.
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for p in &pts {
        x0 = x0.min(p.0 - r);
        y0 = y0.min(p.1 - r);
        x1 = x1.max(p.0 + r);
        y1 = y1.max(p.1 + r);
    }
    // content-aware fill reads the spot's surroundings (the window holds 2 radii around it)
    let pad = if fill { 2.0 * r } else { r };
    let (bx0, by0) = (((x0 - pad).floor().max(0.0)) as usize, ((y0 - pad).floor().max(0.0)) as usize);
    let (bx1, by1) = (((x1 + pad).ceil() as usize).min(w), ((y1 + pad).ceil() as usize).min(h));
    if bx1 <= bx0 || by1 <= by0 {
        return None;
    }
    let feather = (spot.feather / 100.0).clamp(0.0, 1.0) as f32;
    Some(SpotLayout {
        r,
        pts,
        offset,
        bx: bx0,
        by: by0,
        bw: bx1 - bx0,
        bh: by1 - by0,
        inner: r * (1.0 - feather * 0.8),
        opacity: (spot.opacity / 100.0).clamp(0.0, 1.0) as f32,
        heal: matches!(spot.mode, SpotMode::Remove | SpotMode::Heal),
        fill,
    })
}

/// Content-aware fill of the box: `src` becomes `tgt` with every pixel under the stroke (`alpha > 0`)
/// rebuilt from the rest of the box. Falls back to a smooth membrane fill when the box holds no whole
/// patch to copy from.
fn fill_box(src: &mut [[f32; 3]], tgt: &[[f32; 3]], alpha: &[f32], bw: usize, bh: usize) {
    let hole: Vec<bool> = alpha.iter().map(|a| *a > 0.0).collect();
    let flat: Vec<f32> = tgt.iter().flatten().copied().collect();
    let out = crate::inpaint::complete(bw, bh, 3, &flat, &hole, &crate::inpaint::CompleteParams::default())
        .unwrap_or_else(|| crate::inpaint::membrane_fill(bw, bh, 3, &flat, &hole));
    let (pixels, _) = out.as_chunks::<3>();
    for (d, v) in src.iter_mut().zip(pixels) {
        *d = *v;
    }
}

/// Apply all spots to `img` (output resolution). `ppl` = output pixels per long-edge unit.
pub fn apply(img: &mut Rgb32f, spots: &[Spot], frame: &Frame, ppl: f64) {
    let (w, h) = (img.width, img.height);
    for spot in spots {
        let Some(SpotLayout { r, pts, offset, bx: bx0, by: by0, bw, bh, inner, opacity, heal, fill }) = layout(spot, frame, w, h, ppl) else {
            continue;
        };
        let Some(&first) = pts.first() else { continue };
        let offset = offset.unwrap_or_else(|| auto_source(img, first, r));
        // alpha of the stroke and source patch in the box
        let mut alpha = vec![0.0f32; bw * bh];
        let mut src = vec![[0.0f32; 3]; bw * bh];
        let mut tgt = vec![[0.0f32; 3]; bw * bh];
        for y in 0..bh {
            for x in 0..bw {
                let (px, py) = ((bx0 + x) as f32 + 0.5, (by0 + y) as f32 + 0.5);
                let d = pts.iter().map(|p| ((px - p.0).powi(2) + (py - p.1).powi(2)).sqrt()).fold(f32::MAX, f32::min);
                alpha[y * bw + x] = (1.0 - smooth(inner, r, d)) * opacity;
                src[y * bw + x] = img.sample_bilinear(px + offset.0, py + offset.1);
                tgt[y * bw + x] = img.get(bx0 + x, by0 + y);
            }
        }
        if fill {
            fill_box(&mut src, &tgt, &alpha, bw, bh);
        }
        let low = if heal {
            // Tone correction from the spot's *surroundings* only: normalized convolution of
            // (target − source) over pixels outside the stroke (a membrane-like interpolation).
            let outside: Vec<f32> = alpha.iter().map(|a| if *a < 0.02 { 1.0 } else { 0.0 }).collect();
            let diff = Rgb32f {
                width: bw,
                height: bh,
                data: tgt.iter().zip(&src).zip(&outside).map(|((t, s), m)| [(t[0] - s[0]) * m, (t[1] - s[1]) * m, (t[2] - s[2]) * m]).collect(),
            };
            let wts = crate::raster::Plane { width: bw, height: bh, data: outside };
            let sigma = (r * 0.6).max(1.0);
            let (bd, bm) = (crate::raster::blur::gaussian(&diff, sigma), crate::raster::blur::gaussian(&wts, sigma));
            Some(Rgb32f {
                width: bw,
                height: bh,
                data: bd
                    .data
                    .iter()
                    .zip(&bm.data)
                    .map(|(d, m)| {
                        let m = m.max(1e-4);
                        [d[0] / m, d[1] / m, d[2] / m]
                    })
                    .collect(),
            })
        } else {
            None
        };
        for y in 0..bh {
            for x in 0..bw {
                let i = y * bw + x;
                let a = alpha[i];
                if a <= 0.0 {
                    continue;
                }
                let mut v = src[i];
                if let Some(l) = &low {
                    let d = l.data[i];
                    v = [v[0] + d[0], v[1] + d[1], v[2] + d[2]];
                }
                let t = tgt[i];
                img.set(bx0 + x, by0 + y, [t[0] + (v[0] - t[0]) * a, t[1] + (v[1] - t[1]) * a, t[2] + (v[2] - t[2]) * a].map(|c| c.max(0.0)));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft_develop_model::DevelopSettings;

    #[test]
    fn hard_edge_spots_at_zero_opacity_leave_every_pixel_unchanged() {
        for mode in [SpotMode::Clone, SpotMode::Heal, SpotMode::Remove, SpotMode::Fill] {
            for (cx, cy) in [(32.5, 24.5), (0.5, 0.5)] {
                let original = Rgb32f::from_fn(64, 48, |x, y| [0.2 + x as f32 / 128.0, 0.3 + y as f32 / 128.0, 0.4]);
                let mut img = original.clone();
                let frame = Frame::new(64, 48, &DevelopSettings::default(), true);
                let spot = Spot {
                    mode,
                    points: vec![Point::new(cx / 64.0, cy / 48.0)],
                    size: 5.0 / 64.0,
                    feather: 0.0,
                    opacity: 0.0,
                    source_offset: Some(Point::new(8.0 / 64.0, 0.0)),
                };
                apply(&mut img, &[spot], &frame, frame.px_per_long(64));
                for (i, (actual, expected)) in img.data.iter().zip(&original.data).enumerate() {
                    assert_eq!(actual, expected, "zero-opacity {mode:?} at ({cx}, {cy}), pixel {i}");
                }
            }
        }
    }

    #[test]
    fn hard_edge_clone_copies_inside_and_preserves_pixels_on_the_radius() {
        for (w, h, cx, cy, radius) in [(64, 48, 32.5, 24.5, 5.0), (128, 96, 64.5, 48.5, 10.0)] {
            let original = Rgb32f::from_fn(w, h, |x, y| [0.2 + x as f32 / w as f32, y as f32 / h as f32, 0.4]);
            let mut img = original.clone();
            let frame = Frame::new(w, h, &DevelopSettings::default(), true);
            let spot = Spot {
                mode: SpotMode::Clone,
                points: vec![Point::new(cx / w as f64, cy / h as f64)],
                size: radius / w as f64,
                feather: 0.0,
                opacity: 100.0,
                source_offset: Some(Point::new(0.125, 0.0)),
            };
            apply(&mut img, &[spot], &frame, frame.px_per_long(w));
            let (x, y) = (cx as usize, cy as usize);
            assert_eq!(img.get(x, y), original.get(x + w / 8, y), "inside copies the source at {w}px");
            assert_eq!(img.get(x + radius as usize, y), original.get(x + radius as usize, y), "radius stays unchanged at {w}px");
            assert!(img.data.iter().flatten().all(|v| v.is_finite()));
        }
    }

    #[test]
    fn remove_erases_a_dot() {
        // flat grey with a gentle gradient and a black dot in the middle
        let mut img = Rgb32f::from_fn(120, 80, |x, _| [0.2 + x as f32 * 0.001; 3]);
        for y in 36..44 {
            for x in 56..64 {
                img.set(x, y, [0.0; 3]);
            }
        }
        let frame = Frame::new(120, 80, &DevelopSettings::default(), true);
        let spot = Spot { points: vec![Point::new(0.5, 0.5)], size: 8.0 / 120.0, feather: 30.0, ..Default::default() };
        apply(&mut img, &[spot], &frame, frame.px_per_long(120));
        let c = img.get(60, 40);
        assert!((c[0] - 0.26).abs() < 0.03, "{c:?}");
    }

    /// Vertical stripes (period `period` px) with a black blot over the middle.
    fn striped_with_blot(w: usize, h: usize, period: usize) -> Rgb32f {
        let mut img = Rgb32f::from_fn(w, h, |x, _| if (x / period).is_multiple_of(2) { [0.2; 3] } else { [0.7; 3] });
        let (cx, cy, r) = (w as f32 / 2.0, h as f32 / 2.0, w as f32 * 0.04);
        for y in 0..h {
            for x in 0..w {
                if (x as f32 + 0.5 - cx).hypot(y as f32 + 0.5 - cy) < r {
                    img.set(x, y, [0.0; 3]);
                }
            }
        }
        img
    }

    fn fill_spot() -> Spot {
        // a source offset is ignored by Fill
        Spot {
            mode: SpotMode::Fill,
            points: vec![Point::new(0.5, 0.5)],
            size: 0.06,
            feather: 0.0,
            source_offset: Some(Point::new(0.3, 0.0)),
            ..Default::default()
        }
    }

    #[test]
    fn fill_rebuilds_structure_deterministically() {
        let (w, h) = (200, 160);
        let mut img = striped_with_blot(w, h, 8);
        let frame = Frame::new(w, h, &DevelopSettings::default(), true);
        apply(&mut img, &[fill_spot()], &frame, frame.px_per_long(w));
        // the blot is gone and the stripes continue through it
        let mut err = 0.0f32;
        for x in 90..110 {
            let want = if (x / 8) % 2 == 0 { 0.2 } else { 0.7 };
            err += (img.get(x, 80)[0] - want).abs();
        }
        assert!(err / 20.0 < 0.12, "mean error {}", err / 20.0);
        let mut again = striped_with_blot(w, h, 8);
        apply(&mut again, &[fill_spot()], &frame, frame.px_per_long(w));
        assert_eq!(img.data, again.data);
    }

    #[test]
    fn fill_matches_between_preview_and_export_size() {
        // the same photo at two sizes: the half-size fill matches the full-size one, downsampled
        let (mut big, mut small) = (striped_with_blot(320, 240, 16), striped_with_blot(160, 120, 8));
        let (fb, fs) = (Frame::new(320, 240, &DevelopSettings::default(), true), Frame::new(160, 120, &DevelopSettings::default(), true));
        apply(&mut big, &[fill_spot()], &fb, fb.px_per_long(320));
        apply(&mut small, &[fill_spot()], &fs, fs.px_per_long(160));
        let (mut err, mut n) = (0.0f32, 0);
        for y in 50..70 {
            for x in 70..90 {
                let b: f32 = [(0, 0), (1, 0), (0, 1), (1, 1)].iter().map(|(dx, dy)| big.get(2 * x + dx, 2 * y + dy)[0]).sum::<f32>() / 4.0;
                err += (b - small.get(x, y)[0]).abs();
                n += 1;
            }
        }
        assert!(err / n as f32 <= 0.08, "mean difference {}", err / n as f32);
    }

    #[test]
    fn fill_survives_hostile_spots() {
        let frame = Frame::new(40, 30, &DevelopSettings::default(), true);
        for (p, size) in [((0.0, 0.0), 2.0), ((-5.0, 9.0), 0.1), ((0.5, 0.5), 0.0), ((f64::NAN, 0.5), 0.1), ((0.5, 0.5), f64::INFINITY)] {
            let mut img = Rgb32f::from_fn(40, 30, |x, _| [x as f32 / 40.0; 3]);
            let spot = Spot { mode: SpotMode::Fill, points: vec![Point::new(p.0, p.1)], size, ..Default::default() };
            apply(&mut img, &[spot], &frame, frame.px_per_long(40));
            assert!(img.data.iter().flatten().all(|v| v.is_finite()));
        }
    }

    #[test]
    fn clone_copies_source() {
        let mut img = Rgb32f::from_fn(100, 100, |x, _| if x < 50 { [0.1; 3] } else { [0.9; 3] });
        let frame = Frame::new(100, 100, &DevelopSettings::default(), true);
        let spot = Spot {
            mode: SpotMode::Clone,
            points: vec![Point::new(0.25, 0.5)],
            size: 0.05,
            feather: 0.0,
            source_offset: Some(Point::new(0.5, 0.0)),
            ..Default::default()
        };
        apply(&mut img, &[spot], &frame, frame.px_per_long(100));
        assert!(img.get(25, 50)[0] > 0.8);
        assert!(img.get(5, 50)[0] < 0.2);
    }

    fn with_spots(spots: Vec<Spot>) -> DevelopSettings {
        DevelopSettings { spots, ..Default::default() }
    }

    fn spot_at(x: f64, y: f64, size: f64, offset: Option<Point>) -> Spot {
        Spot { points: vec![Point::new(x, y)], size, source_offset: offset, ..Default::default() }
    }

    #[test]
    fn a_window_grows_to_hold_the_spots_that_touch_it_and_their_sources() {
        use crate::PixelWindow;
        let frame = Frame::new(4000, 3000, &DevelopSettings::default(), true);
        let ppl = frame.px_per_long(4000);
        let win = PixelWindow { x: 1000, y: 1000, w: 400, h: 300 };
        // none: the window is left alone
        assert_eq!(window_for_reads(&with_spots(vec![]), &frame, 4000, 3000, ppl, win), win);
        // a spot far away is no business of this window
        let far = spot_at(0.9, 0.9, 0.01, Some(Point::new(0.02, 0.0)));
        assert_eq!(window_for_reads(&with_spots(vec![far]), &frame, 4000, 3000, ppl, win), win);
        // a spot inside it, reading from 300 px to the right, widens it to the source
        let near = spot_at(0.30, 0.38, 0.01, Some(Point::new(0.075, 0.0)));
        let g = window_for_reads(&with_spots(vec![near]), &frame, 4000, 3000, ppl, win);
        assert!(g.x <= win.x && g.x + g.w >= 1200 + 300 + 80 && g.y <= win.y && g.y + g.h >= win.y + win.h, "{g:?}");
        // a spot that only reaches in with its edge counts too
        let edge = spot_at(0.24, 0.38, 0.01, Some(Point::new(-0.05, 0.0)));
        assert!(window_for_reads(&with_spots(vec![edge]), &frame, 4000, 3000, ppl, win).x < win.x);
    }

    #[test]
    fn spot_windows_are_bounded_and_survive_hostile_spots() {
        use crate::PixelWindow;
        let frame = Frame::new(40_000, 30_000, &DevelopSettings::default(), true);
        let ppl = frame.px_per_long(40_000);
        let win = PixelWindow { x: 1000, y: 1000, w: 400, h: 300 };
        // a spot whose source is a long way off would need an enormous window: not granted
        let wide = spot_at(0.03, 0.04, 0.01, Some(Point::new(0.6, 0.0)));
        let g = window_for_reads(&with_spots(vec![wide]), &frame, 40_000, 30_000, ppl, win);
        assert!(g.w <= MAX_SPOT_SPAN && g.h <= MAX_SPOT_SPAN, "{g:?}");
        for bad in [f64::NAN, f64::INFINITY, 1e300, -1e300] {
            let s = spot_at(bad, bad, bad, Some(Point::new(bad, bad)));
            let g = window_for_reads(&with_spots(vec![s]), &frame, 40_000, 30_000, ppl, win);
            assert_eq!(g, win);
        }
        let empty = Spot { points: vec![], ..Default::default() };
        assert_eq!(window_for_reads(&with_spots(vec![empty]), &frame, 40_000, 30_000, ppl, win), win);
    }

    // Given an Auto Mask stroke reaching into the window, the window grows to hold all of it
    #[test]
    fn a_window_grows_to_hold_an_auto_mask_stroke() {
        use crate::PixelWindow;
        use craft_develop_model::{BrushStroke, Mask, MaskComponent, MaskOp, MaskShape};
        let frame = Frame::new(4000, 3000, &DevelopSettings::default(), true);
        let ppl = frame.px_per_long(4000);
        let win = PixelWindow { x: 1000, y: 1000, w: 400, h: 300 };
        let stroke =
            |auto| BrushStroke { points: vec![Point::new(0.30, 0.38), Point::new(0.5, 0.38)], size: 0.01, auto_mask: auto, ..Default::default() };
        let mask = |auto| Mask {
            components: vec![MaskComponent { name: None, op: MaskOp::Add, invert: false, shape: MaskShape::Brush { strokes: vec![stroke(auto)] } }],
            ..Default::default()
        };
        let plain = DevelopSettings { masks: vec![mask(false)], ..Default::default() };
        assert_eq!(window_for_reads(&plain, &frame, 4000, 3000, ppl, win), win, "a plain stroke reads nothing around it");
        let auto = DevelopSettings { masks: vec![mask(true)], ..Default::default() };
        let g = window_for_reads(&auto, &frame, 4000, 3000, ppl, win);
        // the dabs near the window, not the whole stroke (it runs on to x = 2000)
        assert!(g.x <= 1000 && g.x + g.w > 1400 && g.x + g.w < 1700, "{g:?}");
        // a window at the far end of a long stroke does not grow back along all of it
        let far = window_for_reads(&auto, &frame, 4000, 3000, ppl, PixelWindow { x: 1900, ..win });
        assert!(far.x > 1500, "{far:?}");
        // a hidden mask reads nothing
        let hidden = DevelopSettings { masks: vec![Mask { visible: false, ..auto.masks[0].clone() }], ..Default::default() };
        assert_eq!(window_for_reads(&hidden, &frame, 4000, 3000, ppl, win), win);
    }

    // Given a window that would have to grow past the cap, there is no answer (never a wrong one)
    #[test]
    fn a_window_that_cannot_hold_what_it_reads_is_refused() {
        use crate::PixelWindow;
        let frame = Frame::new(40_000, 30_000, &DevelopSettings::default(), true);
        let ppl = frame.px_per_long(40_000);
        let win = PixelWindow { x: 1000, y: 1000, w: 400, h: 300 };
        let far = with_spots(vec![spot_at(0.03, 0.04, 0.01, Some(Point::new(0.6, 0.0)))]);
        assert_eq!(window_for_reads_checked(&far, &frame, 40_000, 30_000, ppl, win), None);
        let near = with_spots(vec![spot_at(0.03, 0.04, 0.01, Some(Point::new(0.02, 0.0)))]);
        assert!(window_for_reads_checked(&near, &frame, 40_000, 30_000, ppl, win).is_some());
        // a spot that doesn't touch the window is no business of it, however far it reads
        let elsewhere = with_spots(vec![spot_at(0.9, 0.9, 0.01, Some(Point::new(-0.8, 0.0)))]);
        assert_eq!(window_for_reads_checked(&elsewhere, &frame, 40_000, 30_000, ppl, win), Some(win));
    }
}
