//! Region rendering (issue storytold/lightcraft#323): a window of the virtual full-size output is rendered on its own
//! and must match the same pixels of the full render, so zoomed views can be sharp at any zoom
//! without materializing the whole frame.

use crate::raster::{Rgb32f, Rgba8};
use craft_develop_model::{DevelopSettings, controls};
use craft_geom::{Orientation, Rect};

use crate::{OutputDepth, PixelWindow, RenderRequest, SourceInfo, render};

const W: usize = 960;
const H: usize = 640;
/// Context rendered around the pixels a caller keeps (spatial filters read their neighbours).
const HALO: usize = 64;

fn scene() -> Rgb32f {
    crate::scenes::demo_library()[0].render(480, 320)
}

fn full(s: &DevelopSettings) -> Rgba8 {
    render(&scene(), &SourceInfo::default(), s, &RenderRequest::fit(W, H)).image
}

fn window(s: &DevelopSettings, win: PixelWindow) -> Rgba8 {
    let req = RenderRequest { window: Some(win), ..RenderRequest::fit(W, H) };
    render(&scene(), &SourceInfo::default(), s, &req).image
}

/// Largest per-channel difference and mean absolute difference between `win` (rendered for
/// `at`, kept `HALO` px in from its edges) and the same pixels of `full`.
fn compare(full: &Rgba8, win: &Rgba8, at: PixelWindow) -> (u8, f64) {
    compare_inset(full, win, at, HALO)
}

/// [`compare`] keeping `inset` px in from the window's edges.
fn compare_inset(full: &Rgba8, win: &Rgba8, at: PixelWindow, inset: usize) -> (u8, f64) {
    let (mut max, mut sum, mut n) = (0u8, 0f64, 0f64);
    for y in inset..at.h - inset {
        for x in inset..at.w - inset {
            let (a, b) = (full.get(at.x + x, at.y + y), win.get(x, y));
            for k in 0..3 {
                let d = a[k].abs_diff(b[k]);
                max = max.max(d);
                sum += d as f64;
                n += 1.0;
            }
        }
    }
    (max, sum / n)
}

fn check(name: &str, s: &DevelopSettings, win: PixelWindow, max_ok: u8, mean_ok: f64) {
    let (f, w) = (full(s), window(s, win));
    assert_eq!((w.width, w.height), (win.w, win.h), "{name}: the window's own size");
    let (max, mean) = compare(&f, &w, win);
    assert!(max <= max_ok && mean <= mean_ok, "{name}: max {max} (≤ {max_ok}), mean {mean:.3} (≤ {mean_ok})");
}

const MID: PixelWindow = PixelWindow { x: 300, y: 180, w: 400, h: 300 };

// Given the window is the whole output, then it is the full render, byte for byte
#[test]
fn the_whole_frame_as_a_window_is_the_full_render() {
    let s = DevelopSettings::default();
    let f = full(&s);
    let w = window(&s, PixelWindow { x: 0, y: 0, w: f.width, h: f.height });
    assert_eq!(f.data, w.data);
}

// Given default settings, a window in the middle matches the full render
#[test]
fn a_window_matches_the_full_render() {
    check("default", &DevelopSettings::default(), MID, 1, 0.1);
}

// Given a window at each corner, it matches too (edges of the frame are not special)
#[test]
fn windows_at_the_corners_match() {
    let s = DevelopSettings::default();
    for (x, y) in [(0, 0), (W - 400, 0), (0, H - 300), (W - 400, H - 300)] {
        check("corner", &s, PixelWindow { x, y, w: 400, h: 300 }, 1, 0.1);
    }
}

// Given crop, straighten and flip, the window still lands on the same pixels
#[test]
fn a_cropped_straightened_flipped_view_matches() {
    let mut s = DevelopSettings::default();
    s.crop.geometry.rect = Rect { x0: 0.1, y0: 0.15, x1: 0.85, y1: 0.9 };
    s.crop.geometry.angle = 4.0;
    s.crop.flip_h = true;
    check("crop+angle+flipH", &s, MID, 3, 0.3);
    s.crop.flip_h = false;
    s.crop.flip_v = true;
    check("crop+angle+flipV", &s, MID, 3, 0.3);
}

// Given a rotated photo, the same
#[test]
fn rotated_and_mirrored_orientations_match() {
    for o in [Orientation::Rotate90, Orientation::Rotate270, Orientation::FlipH, Orientation::Transpose] {
        let s = DevelopSettings { orientation: o, ..Default::default() };
        let f = full(&s);
        let win = PixelWindow { x: f.width / 4, y: f.height / 4, w: f.width / 2, h: f.height / 2 };
        check(&format!("{o:?}"), &s, win, 3, 0.3);
    }
}

// Given a post-crop vignette, the window shows the vignette of the whole frame, not its own
#[test]
fn the_vignette_belongs_to_the_frame_not_the_window() {
    let mut s = DevelopSettings::default();
    controls::set(&mut s, "vignette.amount", -90.0);
    controls::set(&mut s, "vignette.midpoint", 10.0);
    // the vignette is really there: the corner is much darker than without it
    let (with, without) = (full(&s), full(&DevelopSettings::default()));
    let corner = |i: &Rgba8| i.get(W - 20, H - 20)[1] as i32;
    assert!(corner(&without) - corner(&with) > 30, "{} vs {}", corner(&without), corner(&with));
    // …and a window over that corner shows it, at the same place
    check("vignette", &s, PixelWindow { x: W - 400, y: H - 300, w: 400, h: 300 }, 2, 0.2);
    check("vignette middle", &s, MID, 2, 0.2);
}

// Given spatial adjustments, the window agrees away from its halo
#[test]
fn spatial_adjustments_agree_inside_the_halo() {
    for (id, v) in
        [("effects.clarity", 70.0), ("effects.dehaze", 40.0), ("detail.sharpenAmount", 80.0), ("light.shadows", 60.0), ("effects.texture", 60.0)]
    {
        let mut s = DevelopSettings::default();
        controls::set(&mut s, id, v);
        check(id, &s, MID, 6, 0.8);
    }
}

// Given a huge virtual frame (48000 px: an 800 % zoom), only the window is rendered
#[test]
fn a_window_of_a_huge_frame_is_cheap() {
    let req = RenderRequest { window: Some(PixelWindow { x: 20_000, y: 12_000, w: 256, h: 192 }), ..RenderRequest::fit(48_000, 32_000) };
    let r = render(&scene(), &SourceInfo::default(), &DevelopSettings::default(), &req);
    assert_eq!((r.image.width, r.image.height), (256, 192));
}

// Hostile windows (from UI state) are clamped into the frame, never a panic or a huge buffer
#[test]
fn windows_outside_or_degenerate_are_clamped() {
    let s = DevelopSettings::default();
    for win in [
        PixelWindow { x: W + 500, y: 5, w: 100, h: 100 },
        PixelWindow { x: 0, y: 0, w: 0, h: 0 },
        PixelWindow { x: usize::MAX, y: usize::MAX, w: usize::MAX, h: usize::MAX },
        PixelWindow { x: 900, y: 600, w: 5000, h: 5000 },
    ] {
        let r = window(&s, win);
        assert!(r.width >= 1 && r.height >= 1 && r.width <= W && r.height <= H, "{win:?}: {}×{}", r.width, r.height);
    }
}

/// A model's segmentation on a `side²` grid: an ellipse in the middle of the photo.
fn middle_seg(side: usize) -> craft_develop_model::SegMask {
    let logits: Vec<f32> = (0..side * side)
        .map(|i| {
            let (x, y) = (((i % side) as f32 + 0.5) / side as f32, ((i / side) as f32 + 0.5) / side as f32);
            let r = (((x - 0.5) / 0.3).powi(2) + ((y - 0.5) / 0.25).powi(2)).sqrt();
            (12.0 * (1.0 - r) * 4.0).clamp(-12.0, 12.0)
        })
        .collect();
    craft_develop_model::SegMask::from_logits(side, &logits)
}

/// A textured, noisy source: what noise reduction and edge refinement act on.
fn noisy_scene() -> Rgb32f {
    let base = scene();
    Rgb32f::from_fn(base.width, base.height, |x, y| {
        let mut v = (x as u32).wrapping_mul(0x8da6_b343) ^ (y as u32).wrapping_mul(0xd816_3841);
        v ^= v >> 13;
        v = v.wrapping_mul(0x5bd1_e995);
        v ^= v >> 15;
        let n = 1.0 + 0.25 * ((v & 0xffff) as f32 / 32768.0 - 1.0);
        base.data[y * base.width + x].map(|c| c * n)
    })
}

fn check_noisy(name: &str, s: &DevelopSettings, win: PixelWindow, max_ok: u8, mean_ok: f64) {
    let src = noisy_scene();
    let f = render(&src, &SourceInfo::default(), s, &RenderRequest::fit(W, H)).image;
    let w = render(&src, &SourceInfo::default(), s, &RenderRequest { window: Some(win), ..RenderRequest::fit(W, H) }).image;
    let (max, mean) = compare(&f, &w, win);
    assert!(max <= max_ok && mean <= mean_ok, "{name}: max {max} (≤ {max_ok}), mean {mean:.3} (≤ {mean_ok})");
}

// Given noise reduction, its strength follows the whole frame's size, not the window's
#[test]
fn noise_reduction_does_not_depend_on_the_window() {
    for (id, v) in [("detail.nrLuminance", 80.0), ("detail.nrColor", 80.0)] {
        let mut s = DevelopSettings::default();
        controls::set(&mut s, id, v);
        check_noisy(id, &s, MID, 3, 0.15);
    }
}

// Given a mask with Refine Edges, the refinement width follows the whole frame's size
#[test]
fn mask_edge_refinement_does_not_depend_on_the_window() {
    use craft_develop_model::{LocalAdjustments, Mask, MaskComponent, MaskOp, MaskShape};
    use craft_geom::Point;
    let shape = MaskShape::Linear { start: Point::new(0.45, 0.5), end: Point::new(0.55, 0.5) };
    let mut s = DevelopSettings::default();
    s.masks.push(Mask {
        id: 1,
        components: vec![MaskComponent { name: None, op: MaskOp::Add, invert: false, shape }],
        adjust: LocalAdjustments { exposure: 1.5, ..Default::default() },
        refine: 100.0,
        ..Default::default()
    });
    check_noisy("refine", &s, MID, 3, 0.2);
}

// Given a mask overlay (the Masking panel's view of the selected mask), a window of it matches the
// same pixels of the whole render with that overlay, for every view and for drawn, AI and
// refined shapes: the zoomed loupe can show the overlay as a window (issue storytold/lightcraft#547)
#[test]
fn a_mask_overlay_window_matches_the_whole_render() {
    use crate::{MaskView, Overlay};
    use craft_develop_model::{LocalAdjustments, Mask, MaskComponent, MaskOp, MaskShape};
    use craft_geom::Point;
    let shapes = [
        ("radial", MaskShape::Radial { center: Point::new(0.5, 0.5), rx: 0.2, ry: 0.15, angle: 20.0, feather: 50.0, invert: false }, 0.0),
        ("linear refined", MaskShape::Linear { start: Point::new(0.45, 0.5), end: Point::new(0.55, 0.5) }, 100.0),
        ("sky", MaskShape::Sky, 0.0),
        // a model's grid, enlarged along the photo's edges (issue storytold/lightcraft#3272)
        ("prompt", MaskShape::Prompt { text: "middle".into(), seg: Some(middle_seg(32)), detail: vec![], edge: 0.0 }, 0.0),
    ];
    for (name, shape, refine) in shapes {
        let mut s = DevelopSettings::default();
        s.masks.push(Mask {
            id: 3,
            components: vec![MaskComponent { name: None, op: MaskOp::Add, invert: false, shape }],
            adjust: LocalAdjustments { exposure: 1.0, ..Default::default() },
            refine,
            ..Default::default()
        });
        for view in [MaskView::Color, MaskView::WhiteOnBlack, MaskView::ImageOnBlack] {
            let overlay = Overlay::Mask { id: 3, view, color: [255, 0, 0], opacity: 70 };
            let f = render(&noisy_scene(), &SourceInfo::default(), &s, &RenderRequest { overlay, ..RenderRequest::fit(W, H) }).image;
            let w =
                render(&noisy_scene(), &SourceInfo::default(), &s, &RenderRequest { overlay, window: Some(MID), ..RenderRequest::fit(W, H) }).image;
            let (max, mean) = compare(&f, &w, MID);
            // (refine edges differs by a rounding step, as in `mask_edge_refinement_does_not_depend_on_the_window`)
            assert!(max <= 3 && mean <= 0.25, "{name} {view:?}: max {max} (≤ 3), mean {mean:.3} (≤ 0.25)");
        }
    }
}

// Given the Show All Masks view (issue storytold/lightcraft#1712), a window of it matches the same pixels of the
// whole render with that overlay: the union is composed from the same mask alphas, so the zoomed
// loupe can show it as a window like a single mask's overlay (issue storytold/lightcraft#547)
#[test]
fn an_all_masks_overlay_window_matches_the_whole_render() {
    use crate::{MaskView, Overlay};
    use craft_develop_model::{LocalAdjustments, Mask, MaskComponent, MaskOp, MaskShape};
    use craft_geom::Point;
    let shapes = [
        (3, MaskShape::Radial { center: Point::new(0.45, 0.5), rx: 0.2, ry: 0.15, angle: 20.0, feather: 50.0, invert: false }, 0.0),
        (4, MaskShape::Linear { start: Point::new(0.4, 0.55), end: Point::new(0.6, 0.5) }, 100.0),
    ];
    let mut s = DevelopSettings::default();
    for (id, shape, refine) in shapes {
        s.masks.push(Mask {
            id,
            components: vec![MaskComponent { name: None, op: MaskOp::Add, invert: false, shape }],
            adjust: LocalAdjustments { exposure: 1.0, ..Default::default() },
            refine,
            ..Default::default()
        });
    }
    for view in [MaskView::Color, MaskView::WhiteOnBlack, MaskView::ImageOnBlack] {
        let overlay = Overlay::AllMasks { view, color: [255, 0, 0], opacity: 70 };
        let f = render(&noisy_scene(), &SourceInfo::default(), &s, &RenderRequest { overlay, ..RenderRequest::fit(W, H) }).image;
        let w = render(&noisy_scene(), &SourceInfo::default(), &s, &RenderRequest { overlay, window: Some(MID), ..RenderRequest::fit(W, H) }).image;
        let (max, mean) = compare(&f, &w, MID);
        // (refine edges differs by a rounding step, as in `mask_edge_refinement_does_not_depend_on_the_window`)
        assert!(max <= 3 && mean <= 0.25, "{view:?}: max {max} (≤ 3), mean {mean:.3} (≤ 0.25)");
    }
}

// ---- Deep zoom: the wide stages' kernels are far wider than any margin -------------------------

/// Hard-edged blocks of very different brightness at several scales: the wide stages' edge-aware
/// filters and the dark channel have something to be wrong about.
fn blocks() -> Rgb32f {
    Rgb32f::from_fn(600, 400, |x, y| {
        let cell = |size: usize, k: usize| ((x / size * 7 + y / size * 13 + k) % 5) as f32 / 4.0;
        let l = 0.02 + 0.5 * cell(97, 1) + 0.25 * cell(31, 2) + 0.1 * cell(11, 3);
        [l * 1.1, l, l * 0.8]
    })
}

const BIG_W: usize = 7680;
const BIG_H: usize = 5120;

/// A window of a 7680 × 5120 frame against the same pixels of the whole render. The wide stages
/// (dehaze, clarity, highlights/shadows base) have XX/// can't contain them, so they come from the whole frame at reduced size.
fn check_big(name: &str, s: &DevelopSettings, max_ok: u8, mean_ok: f64) {
    let src = blocks();
    let f = render(&src, &SourceInfo::default(), s, &RenderRequest::fit(BIG_W, BIG_H)).image;
    let win = PixelWindow { x: 3000, y: 1800, w: 512, h: 384 };
    let w = render(&src, &SourceInfo::default(), s, &RenderRequest { window: Some(win), ..RenderRequest::fit(BIG_W, BIG_H) }).image;
    let (max, mean) = compare(&f, &w, win);
    assert!(max <= max_ok && mean <= mean_ok, "{name}: max {max} (≤ {max_ok}), mean {mean:.3} (≤ {mean_ok})");
}

#[test]
fn dehaze_at_depth_matches_the_whole_render() {
    let mut s = DevelopSettings::default();
    controls::set(&mut s, "effects.dehaze", 60.0);
    check_big("dehaze", &s, 6, 0.6);
}

#[test]
fn clarity_at_depth_matches_the_whole_render() {
    let mut s = DevelopSettings::default();
    controls::set(&mut s, "effects.clarity", 80.0);
    check_big("clarity", &s, 6, 0.6);
}

#[test]
fn highlights_and_shadows_at_depth_match_the_whole_render() {
    let mut s = DevelopSettings::default();
    controls::set(&mut s, "light.shadows", 80.0);
    controls::set(&mut s, "light.highlights", -80.0);
    check_big("shadows+highlights", &s, 6, 0.6);
}

// ---- Spot removal reads outside the spot -------------------------------------------------------

/// Like [`check`], over every pixel of the window (a spot at its edge must be right too).
fn check_whole(name: &str, s: &DevelopSettings, win: PixelWindow, max_ok: u8, mean_ok: f64) {
    let (f, w) = (full(s), window(s, win));
    let (max, mean) = compare_inset(&f, &w, win, 0);
    assert_eq!((w.width, w.height), (win.w, win.h), "{name}: the window's own size");
    assert!(max <= max_ok && mean <= mean_ok, "{name}: max {max} (≤ {max_ok}), mean {mean:.3} (≤ {mean_ok})");
}

fn with_spot(mode: craft_develop_model::SpotMode, at: (f64, f64), offset: Option<(f64, f64)>) -> DevelopSettings {
    use craft_develop_model::Spot;
    use craft_geom::Point;
    let mut s = DevelopSettings::default();
    s.spots.push(Spot {
        mode,
        points: vec![Point::new(at.0, at.1)],
        size: 0.04,
        feather: 30.0,
        opacity: 100.0,
        source_offset: offset.map(|(x, y)| Point::new(x, y)),
    });
    s
}

// Given a spot whose source patch lies outside the window, the window still shows it healed
#[test]
fn a_spot_with_its_source_outside_the_window_matches_the_whole_render() {
    use craft_develop_model::SpotMode;
    // the target is at x ≈ 0.36 of the frame; its source is 0.12 of the width to the right
    for mode in [SpotMode::Clone, SpotMode::Heal] {
        let s = with_spot(mode, (0.40, 0.5), Some((0.12, 0.05)));
        // a window that holds the target (x ≈ 384) but not the source (x ≈ 499..)
        check_whole(&format!("{mode:?}"), &s, PixelWindow { x: 250, y: 200, w: 200, h: 200 }, 1, 0.1);
    }
}

// Given a spot just outside the window whose feathered edge reaches in, the window shows it
#[test]
fn a_spot_overlapping_the_window_edge_matches_the_whole_render() {
    let s = with_spot(craft_develop_model::SpotMode::Heal, (0.30, 0.5), Some((-0.1, 0.0)));
    check_whole("edge", &s, PixelWindow { x: 300, y: 200, w: 240, h: 240 }, 1, 0.1);
}

// Given a spot with an automatic source, two windows over the same spot choose the same source
#[test]
fn an_automatic_spot_source_does_not_depend_on_the_window() {
    let s = with_spot(craft_develop_model::SpotMode::Heal, (0.5, 0.5), None);
    // windows so tight that the candidate sources can't all be inside them
    let (a, b) = (PixelWindow { x: 430, y: 280, w: 110, h: 90 }, PixelWindow { x: 450, y: 290, w: 110, h: 90 });
    let (ia, ib) = (window(&s, a), window(&s, b));
    let (mut max, mut n) = (0u8, 0);
    for y in 0..80 {
        for x in 0..90 {
            let (p, q) = (ia.get(x + 20, y + 10), ib.get(x, y));
            for k in 0..3 {
                max = max.max(p[k].abs_diff(q[k]));
                n += 1;
            }
        }
    }
    assert!(n > 0 && max <= 2, "overlapping windows disagree by up to {max}");
}

// Given Auto Mask strokes that run in and out of the window, the window shows the whole render's mask
#[test]
fn auto_mask_strokes_match_the_whole_render() {
    use craft_develop_model::{BrushStroke, LocalAdjustments, Mask, MaskComponent, MaskOp, MaskShape};
    use craft_geom::Point;
    let st = |pts: &[(f64, f64)], auto_mask, erase| BrushStroke {
        points: pts.iter().map(|p| Point::new(p.0, p.1)).collect(),
        size: 0.06,
        feather: 40.0,
        flow: 80.0,
        auto_mask,
        erase,
        ..Default::default()
    };
    let strokes = vec![st(&[(0.2, 0.55), (0.5, 0.6), (0.8, 0.5)], true, false), st(&[(0.5, 0.58)], true, true)];
    let s = DevelopSettings {
        masks: vec![Mask {
            components: vec![MaskComponent { name: None, op: MaskOp::Add, invert: false, shape: MaskShape::Brush { strokes } }],
            adjust: LocalAdjustments { exposure: 1.0, saturation: -50.0, ..Default::default() },
            ..Default::default()
        }],
        ..Default::default()
    };
    check_noisy("auto mask", &s, PixelWindow { x: 400, y: 300, w: 300, h: 250 }, 2, 0.1);
}

// Given a lens blur, a window (zoomed view) blurs right up to its edges as the whole render does:
// it is rendered with the blur's reach around it
#[test]
fn lens_blur_matches_the_whole_render_up_to_the_window_edges() {
    lens_blur_window_matches(OutputDepth::U8);
    lens_blur_window_matches(OutputDepth::U16);
}

/// A high-bit-depth window grows for the lens blur too, and is cut back to the request.
fn lens_blur_window_matches(depth: OutputDepth) {
    use craft_develop_model::{Mask, MaskComponent, MaskOp, MaskShape};
    use craft_geom::Point;
    let radial = MaskShape::Radial { center: Point::new(0.5, 0.5), rx: 0.15, ry: 0.2, angle: 0.0, feather: 30.0, invert: false };
    let mut s = DevelopSettings {
        masks: vec![Mask {
            id: 4,
            components: vec![MaskComponent { name: None, op: MaskOp::Add, invert: false, shape: radial }],
            ..Default::default()
        }],
        ..Default::default()
    };
    s.lens_blur.enabled = true;
    s.lens_blur.amount = 80.0;
    s.lens_blur.boost = 40.0;
    s.lens_blur.focus_mask = Some(4);
    let deep = |win: Option<PixelWindow>| {
        let r = render(&scene(), &SourceInfo::default(), &s, &RenderRequest { window: win, depth, ..RenderRequest::fit(W, H) });
        if let Some(d) = &r.deep {
            assert_eq!((d.width, d.height), (r.image.width, r.image.height));
        }
        r.image
    };
    let (f, w) = (deep(None), deep(Some(MID)));
    assert_eq!((w.width, w.height), (MID.w, MID.h));
    let (max, mean) = compare_inset(&f, &w, MID, 0);
    assert!(max <= 6 && mean <= 0.4, "max {max}, mean {mean:.3}");
    // and the blur shows: the whole render differs from the one without it
    let mut off = s.clone();
    off.lens_blur.enabled = false;
    let (_, mean) = compare_inset(&full(&off), &f, PixelWindow { x: 0, y: 0, w: f.width, h: f.height }, 0);
    assert!(mean > 1.0, "{mean:.3}");
}

/// With the automatic subject focus (no focus mask, no fixed shape, no person matte) a window
/// focuses where the whole render does: the estimate comes from the whole photo, not from the
/// pixels the window holds.
#[test]
fn lens_blur_on_the_estimated_subject_matches_the_whole_render() {
    let mut s = DevelopSettings::default();
    s.lens_blur.enabled = true;
    s.lens_blur.amount = 80.0;
    // a sharp cut: the window then grows only by the blur's radius, less than the estimate reads
    s.lens_blur.transition = 0.0;
    for depth in [OutputDepth::U8, OutputDepth::U16] {
        let r = |win: Option<PixelWindow>| {
            render(&scene(), &SourceInfo::default(), &s, &RenderRequest { window: win, depth, ..RenderRequest::fit(W, H) }).image
        };
        // a window at the middle and one at an edge, where the estimate's neighbourhood is cut off
        for win in [MID, PixelWindow { x: 0, y: 0, w: 300, h: 220 }] {
            let (f, w) = (r(None), r(Some(win)));
            assert_eq!((w.width, w.height), (win.w, win.h));
            let (max, mean) = compare_inset(&f, &w, win, 0);
            assert!(max <= 6 && mean <= 0.4, "{depth:?} {win:?}: max {max}, mean {mean:.3}");
        }
    }
}

// Given the joint colour NR, a window is filtered with the whole frame's noise model: a quiet part of
// the frame does not make a window of its noisy part (or the other way round) smooth differently
#[test]
fn joint_colour_nr_measures_the_noise_of_the_whole_frame() {
    let mut s = DevelopSettings::default();
    s.detail.nr_color = 25.0;
    s.detail.nr_color_smoothness = 0.0;
    // a frame that is one noise tile, and one of many tiles (`nrc_joint::tiles`)
    for (w, h, win) in [(768, 128, PixelWindow { x: 640, y: 32, w: 64, h: 64 }), (1664, 1536, PixelWindow { x: 1400, y: 700, w: 128, h: 128 })] {
        let src = crate::tests::colour_noise_scene(w, h, w * 2 / 3);
        let req = RenderRequest::fit(w, h);
        let f = render(&src, &SourceInfo::default(), &s, &req).image;
        let part = render(&src, &SourceInfo::default(), &s, &RenderRequest { window: Some(win), ..req }).image;
        let (max, mean) = compare_inset(&f, &part, win, win.w / 4);
        assert!(max <= 2 && mean <= 0.1, "{w}×{h}: max {max}, mean {mean:.3}");
    }
}

// Given a 16-bit or float render of a window with the joint colour NR, then its edges are the whole
// render's too: the window is grown by the filter's reach and cut back
#[test]
fn a_deep_window_with_colour_noise_reduction_matches_the_whole_render() {
    use crate::OutputDepth;
    let (w, h, win) = (768, 128, PixelWindow { x: 600, y: 32, w: 96, h: 64 });
    let src = crate::tests::colour_noise_scene(w, h, w * 2 / 3);
    let mut s = DevelopSettings::default();
    s.detail.nr_color = 25.0;
    s.detail.nr_color_smoothness = 0.0;
    for depth in [OutputDepth::U16, OutputDepth::F32Linear] {
        let req = RenderRequest { depth, ..RenderRequest::fit(w, h) };
        let deep = |req: &RenderRequest| render(&src, &SourceInfo::default(), &s, req).deep.map(|d| d.to_rgba8());
        let (Some(f), Some(part)) = (deep(&req), deep(&RenderRequest { window: Some(win), ..req })) else {
            panic!("{depth:?}: no deep image");
        };
        assert_eq!((part.width, part.height), (win.w, win.h), "{depth:?}: the window's own size");
        let (max, mean) = compare_inset(&f, &part, win, 0);
        assert!(max <= 1 && mean <= 0.01, "{depth:?}: max {max}, mean {mean:.3}");
    }
}

/// The margin the loupe's windows carry around what they show at this size (`region::margin_for`:
/// 4.5 % of the long edge).
const OVERLAY_INSET: usize = W * 45 / 1000;

/// [`check`] for a diagnostic overlay, with the photo as a raw (Visualize HDR needs one), keeping the
/// loupe's margin in from the window's edges.
fn check_overlay(name: &str, s: &DevelopSettings, overlay: crate::Overlay, win: PixelWindow, max_ok: u8, mean_ok: f64) {
    let info = SourceInfo { raw: true, ..Default::default() };
    let full_req = RenderRequest { overlay, ..RenderRequest::fit(W, H) };
    let f = render(&scene(), &info, s, &full_req).image;
    let w = render(&scene(), &info, s, &RenderRequest { window: Some(win), ..full_req }).image;
    assert_eq!((w.width, w.height), (win.w, win.h), "{name}: the window's own size");
    assert_ne!(f.data, render(&scene(), &info, s, &RenderRequest::fit(W, H)).image.data, "{name}: the overlay draws something");
    let (max, mean) = compare_inset(&f, &w, win, OVERLAY_INSET);
    assert!(max <= max_ok && mean <= mean_ok, "{name}: max {max} (≤ {max_ok}), mean {mean:.3} (≤ {mean_ok})");
}

// Given a diagnostic overlay (Visualize Spots, a Point Color range, Visualize HDR), a window shows
// the same pixels as the whole-frame render, so the loupe can draw them as a zoom window
#[test]
fn diagnostic_overlays_in_a_window_match_the_full_render() {
    use crate::Overlay;
    let mut spots = DevelopSettings::default();
    spots.detail.sharpen_amount = 60.0;
    // a binary threshold: a pixel at the threshold may flip on a one-level difference underneath
    check_overlay("spots", &spots, Overlay::Spots(70), MID, 255, 0.05);
    for (x, y) in [(0, 0), (W - 400, H - 300)] {
        check_overlay("spots at a corner", &spots, Overlay::Spots(70), PixelWindow { x, y, w: 400, h: 300 }, 255, 0.05);
    }
    let mut pc = DevelopSettings::default();
    pc.point_colors.push(craft_develop_model::PointColor { lum: 0.6, chroma: 0.08, hue: 60.0, hue_shift: 40.0, ..Default::default() });
    check_overlay("point colour range", &pc, Overlay::PointColorRange(0), MID, 1, 0.1);
    let mut hdr = DevelopSettings::default();
    hdr.hdr.enabled = true;
    hdr.hdr.max_ev = 3.0;
    hdr.light.exposure = 1.5;
    check_overlay("hdr range", &hdr, Overlay::HdrRange, MID, 1, 0.1);
}

// Given a deep render of a window and a region of it finished alone from the same stage cache (a
// brush stroke growing between them), the region is those pixels of the whole render, sample for
// sample: a shown window patched where a stroke went matches the render that follows the stroke.
#[test]
fn a_region_finished_alone_is_the_whole_renders_pixels() {
    use crate::{DeepSamples, OutputSpace, StageCache, render_cached, render_region_cached};
    use craft_develop_model::{BrushStroke, LocalAdjustments, Mask, MaskComponent, MaskOp, MaskShape};
    use craft_geom::Point;
    use std::sync::Arc;
    let src = Arc::new(noisy_scene());
    let info = SourceInfo::default();
    let with = |pts: &[(f64, f64)], auto_mask: bool| DevelopSettings {
        masks: vec![Mask {
            components: vec![MaskComponent {
                name: None,
                op: MaskOp::Add,
                invert: false,
                shape: MaskShape::Brush {
                    strokes: vec![BrushStroke {
                        points: pts.iter().map(|p| Point::new(p.0, p.1)).collect(),
                        size: 0.05,
                        feather: 50.0,
                        flow: 100.0,
                        auto_mask,
                        ..Default::default()
                    }],
                },
            }],
            adjust: LocalAdjustments { exposure: 1.0, clarity: 30.0, ..Default::default() },
            ..Default::default()
        }],
        ..Default::default()
    };
    let samples = |d: crate::DeepImage| match d.samples {
        DeepSamples::U16(v) => (d.width, d.height, v),
        _ => panic!("16-bit samples"),
    };
    for (window, auto_mask) in [(None, false), (Some(PixelWindow { x: 200, y: 120, w: 500, h: 380 }), true)] {
        let req = RenderRequest { space: OutputSpace::ProPhoto, depth: OutputDepth::U16, window, ..RenderRequest::exact(W, H) };
        let cache = StageCache::default();
        // the stroke so far rendered, then grown: the region round the new dab finished alone
        let _ = render_cached(&src, &info, &with(&[(0.3, 0.4), (0.4, 0.42)], auto_mask), &req, &cache);
        let grown = with(&[(0.3, 0.4), (0.4, 0.42), (0.5, 0.45)], auto_mask);
        let region = PixelWindow { x: 130, y: 60, w: 210, h: 170 };
        let (rw, rh, part) = samples(render_region_cached(&src, &info, &grown, &req, &cache, region).unwrap());
        assert_eq!((rw, rh), (region.w, region.h));
        let (fw, _, whole) = samples(render_cached(&src, &info, &grown, &req, &StageCache::default()).deep.unwrap());
        let mut differ = 0;
        for y in 0..rh {
            for x in 0..rw * 3 {
                let (a, b) = (part[y * rw * 3 + x], whole[(region.y + y) * fw * 3 + region.x * 3 + x]);
                // (an Auto Mask stroke continued from a cached raster matches up to float rounding)
                if a.abs_diff(b) > u16::from(auto_mask) * 2 {
                    differ += 1;
                }
            }
        }
        assert_eq!(differ, 0, "window {window:?}");
        assert!(render_region_cached(&src, &info, &grown, &req, &cache, PixelWindow { x: 0, y: 0, w: 10_000, h: 1 }).is_none(), "outside the render");
    }
    let req8 = RenderRequest { window: None, ..RenderRequest::exact(W, H) };
    assert!(
        render_region_cached(&src, &info, &with(&[(0.3, 0.4)], false), &req8, &StageCache::default(), PixelWindow { x: 0, y: 0, w: 4, h: 4 })
            .is_none(),
        "8-bit"
    );
}
