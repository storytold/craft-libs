//! Masks stay on the image content under every geometry change: a mask
//! drawn around a mark in the photo still brightens that mark after Straighten, a crop, a flip or
//! a lens / perspective warp, rendered fresh or through a view's stage cache.

use std::sync::Arc;

use crate::raster::{Rgb32f, Rgba8};
use craft_develop_model::{BrushStroke, DevelopSettings, LocalAdjustments, Mask, MaskComponent, MaskOp, MaskShape, SegMask};
use craft_geom::{CropGeometry, Point, Rect};

use crate::{RenderRequest, SourceInfo, StageCache, render, render_cached};

const W: usize = 240;
const H: usize = 160;
/// Where the mark sits, normalized.
const MARK: (f64, f64) = (0.3, 0.32);

/// Mid grey with a small bright mark at [`MARK`].
fn marked(mark: bool) -> Rgb32f {
    let (mx, my) = (MARK.0 * W as f64, MARK.1 * H as f64);
    Rgb32f::from_fn(W, H, |x, y| {
        let d = ((x as f64 + 0.5 - mx).powi(2) + (y as f64 + 0.5 - my).powi(2)).sqrt();
        if d < 2.5 && mark { [0.9; 3] } else { [0.05; 3] }
    })
}

fn with_mask(shape: MaskShape) -> DevelopSettings {
    let mut s = DevelopSettings::default();
    s.masks.push(Mask {
        id: 1,
        components: vec![MaskComponent { name: None, op: MaskOp::Add, invert: false, shape }],
        adjust: LocalAdjustments { exposure: 2.0, ..Default::default() },
        ..Default::default()
    });
    s
}

fn radial() -> MaskShape {
    let l = W.max(H) as f64;
    // (radii in long-edge units)
    MaskShape::Radial { center: Point::new(MARK.0, MARK.1), rx: 12.0 / l, ry: 12.0 / l, angle: 0.0, feather: 40.0, invert: false }
}

/// An AI object selection (a stored segmentation) covering the mark.
fn subject() -> MaskShape {
    const SIDE: usize = 64;
    let logits: Vec<f32> = (0..SIDE * SIDE)
        .map(|i| {
            let (x, y) = ((i % SIDE) as f64 + 0.5, (i / SIDE) as f64 + 0.5);
            let (dx, dy) = ((x / SIDE as f64 - MARK.0) * W as f64, (y / SIDE as f64 - MARK.1) * H as f64);
            if (dx * dx + dy * dy).sqrt() < 10.0 { 8.0 } else { -8.0 }
        })
        .collect();
    MaskShape::Object {
        hint: vec![Point::new(MARK.0, MARK.1)],
        exclude: Vec::new(),
        seg: Some(SegMask::from_logits(SIDE, &logits)),
        detail: Vec::new(),
        edge: 0.0,
    }
}

fn luma(img: &Rgba8, i: usize) -> f64 {
    let p = img.data[i];
    (p[0] as f64 + p[1] as f64 + p[2] as f64) / 3.0
}

/// The mark's centre in a render with `s` (where the marked photo differs from an unmarked one:
/// a warp leaves light blank canvas around the photo).
fn mark_at(s: &DevelopSettings) -> (f64, f64) {
    let req = RenderRequest::fit(W, H);
    let info = SourceInfo::default();
    let (a, b) = (render(&marked(true), &info, s, &req).image, render(&marked(false), &info, s, &req).image);
    centroid(a.width, (0..a.data.len()).map(|i| above_dither(luma(&a, i) - luma(&b, i))))
}

/// A difference between two renders, without the one level of 8-bit dither any pixel can differ by.
fn above_dither(d: f64) -> f64 {
    (d - 1.5).max(0.0)
}

fn centroid(w: usize, weights: impl Iterator<Item = f64>) -> (f64, f64) {
    let (mut sx, mut sy, mut n) = (0.0, 0.0, 0.0);
    for (i, v) in weights.enumerate() {
        sx += v * ((i % w) as f64 + 0.5);
        sy += v * ((i / w) as f64 + 0.5);
        n += v;
    }
    assert!(n > 0.0, "nothing found");
    (sx / n, sy / n)
}

/// Where the mask brightened the render (`plain` is the same render without the mask).
fn mask_at(masked: &Rgba8, plain: &Rgba8) -> (f64, f64) {
    centroid(masked.width, (0..masked.data.len()).map(|i| above_dither(luma(masked, i) - luma(plain, i))))
}

/// The geometry changes a user makes after drawing a mask.
#[allow(clippy::type_complexity)] // the type is spelled out where it is used
pub(crate) fn geometries() -> Vec<(&'static str, Box<dyn Fn(&mut DevelopSettings)>)> {
    vec![
        ("straighten", Box::new(|s: &mut DevelopSettings| s.crop.geometry = CropGeometry { rect: Rect::new(0.08, 0.1, 0.92, 0.9), angle: 7.0 })),
        (
            "straighten the other way",
            Box::new(|s: &mut DevelopSettings| s.crop.geometry = CropGeometry { rect: Rect::new(0.08, 0.1, 0.92, 0.9), angle: -9.0 }),
        ),
        ("crop", Box::new(|s: &mut DevelopSettings| s.crop.geometry.rect = Rect::new(0.1, 0.15, 0.7, 0.8))),
        ("flip", Box::new(|s: &mut DevelopSettings| s.crop.flip_h = true)),
        ("flip vertical", Box::new(|s: &mut DevelopSettings| s.crop.flip_v = true)),
        (
            "perspective",
            Box::new(|s: &mut DevelopSettings| {
                s.geometry.vertical = 25.0;
                s.geometry.rotate = 4.0;
            }),
        ),
        ("distortion", Box::new(|s: &mut DevelopSettings| s.optics.distortion = 40.0)),
        (
            // Rotate Right: the photo and its masks turn together (`photo.rotateRight`)
            "quarter turn",
            Box::new(|s: &mut DevelopSettings| {
                let turn = craft_geom::Orientation::Normal.rotated(true);
                s.orientation = s.orientation.rotated(true);
                for m in &mut s.masks {
                    crate::mask_space::turn(m, turn);
                }
            }),
        ),
    ]
}

fn check(name: &str, shape: MaskShape) {
    let src = Arc::new(marked(true));
    let req = RenderRequest::fit(W, H);
    let info = SourceInfo::default();
    for (what, change) in geometries() {
        let mut s = with_mask(shape.clone());
        // a view that rendered the photo before the change (its stage cache holds the mask)
        let cache = StageCache::default();
        render_cached(&src, &info, &s, &req, &cache);
        change(&mut s);
        let plain = DevelopSettings { masks: Vec::new(), ..s.clone() };
        let p = render(&src, &info, &plain, &req).image;
        let mark = mark_at(&plain);
        for (how, m) in [("fresh", render(&src, &info, &s, &req).image), ("cached", render_cached(&src, &info, &s, &req, &cache).image)] {
            let at = mask_at(&m, &p);
            let d = ((at.0 - mark.0).powi(2) + (at.1 - mark.1).powi(2)).sqrt();
            assert!(d < 2.0, "{name} after {what} ({how}): the mask's effect is at {at:?}, the mark at {mark:?}");
        }
    }
}

#[test]
fn a_drawn_mask_stays_on_the_content_under_every_geometry_change() {
    check("radial", radial());
}

fn brush() -> MaskShape {
    let l = W.max(H) as f64;
    MaskShape::Brush {
        strokes: vec![BrushStroke {
            points: vec![Point::new(MARK.0 - 0.005, MARK.1), Point::new(MARK.0 + 0.005, MARK.1)],
            size: 8.0 / l,
            ..Default::default()
        }],
    }
}

#[test]
fn a_brushed_mask_stays_on_the_content_under_every_geometry_change() {
    check("brush", brush());
}

/// A mask from before masks were on the photo keeps its place on the corrected frame: an old edit
/// renders as it did.
#[test]
fn an_old_mask_stays_where_it_was_drawn_on_the_corrected_frame() {
    let src = Arc::new(marked(true));
    let req = RenderRequest::fit(W, H);
    let info = SourceInfo::default();
    let mut s = with_mask(radial());
    s.masks[0].on_photo = false;
    s.geometry.vertical = 25.0;
    s.geometry.rotate = 6.0;
    let unmasked = DevelopSettings { masks: Vec::new(), ..s.clone() };
    let plain = render(&src, &info, &unmasked, &req).image;
    let at = mask_at(&render(&src, &info, &s, &req).image, &plain);
    let frame = crate::geometry::Frame::new(W, H, &s, true);
    let (w, h) = frame.fit(W, H);
    let want = frame.norm_to_out(w, h).apply(Point::new(MARK.0, MARK.1));
    assert!((at.0 - want.x).hypot(at.1 - want.y) < 2.0, "{at:?} vs {want:?}");
    let mark = mark_at(&unmasked);
    assert!((at.0 - mark.0).hypot(at.1 - mark.1) > 3.0, "and not on the mark, which the warp moved: {at:?} vs {mark:?}");
}

#[test]
fn an_ai_mask_stays_on_the_content_under_every_geometry_change() {
    check("object", subject());
}

/// A brush point the warp can't place lays no dab, rather than one at its unwarped place, which
/// can be far from the stroke (review of storytold/lightcraft#3550).
#[test]
fn a_brush_point_the_warp_cannot_place_lays_no_dab() {
    let mut s = DevelopSettings::default();
    s.geometry.vertical = -100.0;
    s.geometry.horizontal = -100.0;
    s.optics.distortion = 100.0;
    s.geometry.scale = 70.0;
    let frame = crate::geometry::Frame::new(1000, 1500, &s, true).for_mask(true).into_owned();
    let (w, h) = frame.fit(1000, 1500);
    let warp = frame.mask_warp().cloned().expect("a warp");
    // a point on the photo the inverse can't reach (found by search, so the test doesn't depend on
    // the solver's exact limits)
    let lost = (0..200 * 200)
        .map(|i| Point::new((i % 200) as f64 / 199.0, (i / 200) as f64 / 199.0))
        .find(|p| warp.from_source(Point::new(p.x * frame.ow, p.y * frame.oh)).is_none());
    let lost = lost.expect("these settings leave points the inverse can't reach");
    let stroke = BrushStroke { points: vec![lost], size: 0.01, ..Default::default() };
    let dabs = crate::masks::brush_dabs_within(&stroke, &frame, w, h, 1000);
    assert!(dabs.dabs.is_empty(), "{lost:?} laid {:?}", dabs.dabs);
}
