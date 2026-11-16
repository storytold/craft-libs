use super::*;
use crate::geometry::Frame;
use crate::masks::{brush_dabs, shape_alpha};
use craft_develop_model::{BrushStroke, DevelopSettings, MaskShape};

/// A scene with a hard vertical edge at x = `edge` (dark, noisy left; bright, bluish right) and a
/// soft diagonal one, its colours and log luminance.
fn scene(w: usize, h: usize, edge: usize) -> (Rgb32f, Plane) {
    let img = Rgb32f::from_fn(w, h, |x, y| {
        let n = (((x * 7919 + y * 104_729) % 97) as f32 / 97.0 - 0.5) * 0.02;
        let soft = ((x as f32 + y as f32 - 1.2 * w as f32) / 12.0).clamp(-1.0, 1.0) * 0.03;
        if x < edge { [0.08 + n + soft, 0.07 + n, 0.05 + n] } else { [0.4 + n, 0.45 + n + soft, 0.6 + n] }
    });
    let l = img.map(crate::local::log_lum);
    (img, l)
}

fn frame(w: usize, h: usize) -> Frame {
    Frame::new(w, h, &DevelopSettings::default(), true)
}

/// A stroke wandering across the edge, `n` points.
fn path(n: usize) -> Vec<Point> {
    (0..n).map(|i| Point::new(0.15 + 0.004 * i as f64, 0.3 + 0.25 * ((i as f64) * 0.05).sin())).collect()
}

fn stroke(points: Vec<Point>, auto_mask: bool, erase: bool) -> BrushStroke {
    BrushStroke { points, size: 0.04, feather: 60.0, flow: 70.0, auto_mask, erase, ..BrushStroke::default() }
}

/// The stroke painted one point at a time, as a pointer drag adds them.
fn painted(s: &BrushStroke, f: &Frame, w: usize, h: usize, img: &Rgb32f, l: &Plane) -> StrokeRaster {
    let mut r = StrokeRaster::new(w, h);
    for n in 1..=s.points.len() {
        let part = BrushStroke { points: s.points[..n].to_vec(), ..s.clone() };
        r.update(&brush_dabs(&part, f, w, h), img, l);
    }
    r
}

fn max_diff(a: &Plane, b: &Plane) -> f32 {
    a.data.iter().zip(&b.data).map(|(x, y)| (x - y).abs()).fold(0.0, f32::max)
}

#[test]
fn a_plain_stroke_painted_point_by_point_is_the_committed_stroke_exactly() {
    let (w, h) = (240, 160);
    let (img, l) = scene(w, h, 120);
    let f = frame(w, h);
    for erase in [false, true] {
        let s = stroke(path(90), false, erase);
        let live = painted(&s, &f, w, h, &img, &l);
        let mut base = Plane::filled(w, h, if erase { 1.0 } else { 0.0 });
        let mut committed = base.clone();
        live.apply_to(&mut base);
        // the renderer's own rasterization, on the same starting alpha
        let full = shape_alpha(&MaskShape::Brush { strokes: vec![s.clone()] }, &f, w, h, &img, &l, 0.0, None);
        for (o, a) in committed.data.iter_mut().zip(&full.data) {
            *o = if erase { 1.0 - (1.0 - a) } else { *a };
        }
        if !erase {
            assert_eq!(base.data, committed.data, "bit for bit");
        } else {
            // erasing from 1: what is left is 1 − coverage, coverage the same stroke's
            let plain = shape_alpha(&MaskShape::Brush { strokes: vec![stroke(path(90), false, false)] }, &f, w, h, &img, &l, 0.0, None);
            let left = Plane::from_fn(w, h, |x, y| 1.0 - plain.get(x, y));
            assert_eq!(base.data, left.data);
        }
    }
}

#[test]
fn an_auto_mask_stroke_painted_point_by_point_matches_the_committed_stroke() {
    let (w, h) = (320, 200);
    let (img, l) = scene(w, h, 150);
    let f = frame(w, h);
    let s = stroke(path(120), true, false);
    let live = painted(&s, &f, w, h, &img, &l).to_plane();
    let committed = shape_alpha(&MaskShape::Brush { strokes: vec![s] }, &f, w, h, &img, &l, 0.0, None);
    let d = max_diff(&live, &committed);
    eprintln!("live vs committed Auto Mask stroke: max difference {d:e}");
    assert!(d < 1e-4, "live and committed differ by {d}");
    // and it is an Auto Mask result: a stroke along the dark side of the edge, its rim reaching
    // over it, leaves the bright side clear where the plain brush paints it
    let along: Vec<Point> = (0..40).map(|i| Point::new(141.0 / w as f64, 0.1 + 0.02 * i as f64)).collect();
    let live = painted(&stroke(along.clone(), true, false), &f, w, h, &img, &l).to_plane();
    let plain = shape_alpha(&MaskShape::Brush { strokes: vec![stroke(along, false, false)] }, &f, w, h, &img, &l, 0.0, None);
    let spill = |p: &Plane| (150..156).flat_map(|x| (20..180).map(move |y| (x, y))).map(|(x, y)| p.get(x, y)).sum::<f32>();
    assert!(spill(&plain) > 10.0 * spill(&live).max(0.1), "plain {} vs auto {}", spill(&plain), spill(&live));
}

#[test]
fn a_stroke_that_does_not_continue_the_raster_starts_it_again() {
    let (w, h) = (200, 120);
    let (img, l) = scene(w, h, 100);
    let f = frame(w, h);
    let a = stroke(path(40), true, false);
    let mut r = painted(&a, &f, w, h, &img, &l);
    // another brush size: not a continuation
    let b = BrushStroke { size: 0.02, ..stroke(path(20), true, false) };
    let bd = brush_dabs(&b, &f, w, h);
    assert!(!r.continues(&bd));
    r.update(&bd, &img, &l);
    let fresh = painted(&b, &f, w, h, &img, &l);
    assert_eq!(r.dabs(), bd.dabs.len());
    assert!(max_diff(&r.to_plane(), &fresh.to_plane()) < 1e-3);
    // the same dabs again change nothing
    assert_eq!(r.update(&bd, &img, &l), None);
}

#[test]
fn hostile_sizes_paint_nothing_and_never_panic() {
    let f = frame(64, 64);
    let s = stroke(path(10), true, false);
    let bd = brush_dabs(&s, &f, 64, 64);
    // colours of another size: an Auto Mask stroke has nothing to compare to
    let mut r = StrokeRaster::new(64, 64);
    r.update(&bd, &Rgb32f::new(3, 3), &Plane::new(3, 3));
    assert!(r.to_plane().data.iter().all(|v| *v == 0.0));
    // an empty output
    let mut r = StrokeRaster::new(0, 0);
    r.update(&brush_dabs(&s, &f, 0, 0), &Rgb32f::new(0, 0), &Plane::new(0, 0));
    assert_eq!(r.alpha_at(5, 5), 0.0);
    let mut p = Plane::new(10, 10);
    r.apply_to(&mut p);
}

#[test]
fn the_brush_cache_reuses_the_raster_of_a_growing_stroke() {
    let (w, h) = (240, 160);
    let (img, l) = scene(w, h, 120);
    let f = frame(w, h);
    let cache = BrushCache::default();
    let src = Arc::new(img.clone());
    let at = BrushCacheRef { cache: &cache, src: &src, key: 1 };
    let mask = |n: usize| craft_develop_model::Mask {
        id: 1,
        components: vec![craft_develop_model::MaskComponent {
            name: None,
            op: craft_develop_model::MaskOp::Add,
            invert: false,
            shape: MaskShape::Brush { strokes: vec![stroke(path(n), true, false)] },
        }],
        ..Default::default()
    };
    for n in [5, 30, 60] {
        let cached = crate::masks::evaluate_one_cached(&mask(n), &f, w, h, &img, &l, 0.0, None, AutoMaskModel::PerPixel, Some(at));
        let fresh = crate::masks::evaluate_one(&mask(n), &f, w, h, &img, &l, 0.0, None);
        assert!(max_diff(&cached, &fresh) < 1e-3, "{n} points");
        assert_eq!(cache.len(), 1, "one raster, grown");
    }
    // another source: not this raster
    let other = Arc::new(img.clone());
    let bd = brush_dabs(&stroke(path(60), true, false), &f, w, h);
    assert_eq!(cache.take(&BrushCacheRef { key: 1, src: &other, cache: &cache }, &bd, w, h, AutoMaskModel::PerPixel).dabs(), 0);
    assert_eq!(cache.take(&BrushCacheRef { key: 2, ..at }, &bd, w, h, AutoMaskModel::PerPixel).dabs(), 0);
    assert_eq!(cache.take(&at, &bd, w, h, AutoMaskModel::PerPixel).dabs(), bd.dabs.len());
}

/// While a stroke grows (one render per pointer move), the cached render and the mask-only alpha
/// show what an uncached render of the same settings does.
#[test]
fn renders_of_a_growing_stroke_match_uncached_ones() {
    use crate::{MaskView, Overlay, RenderRequest, SourceInfo, StageCache, mask_alpha_cached, render, render_cached};
    let (w, h) = (240, 160);
    let (img, _) = scene(w, h, 120);
    let src = Arc::new(img);
    let info = SourceInfo::default();
    let (pic, overlay) = (StageCache::default(), StageCache::default());
    let mut s = DevelopSettings::default();
    let req = RenderRequest::exact(w, h);
    let oreq = RenderRequest { overlay: Overlay::Mask { id: 1, view: MaskView::WhiteOnBlack, color: [255; 3], opacity: 100 }, ..req };
    for n in [3, 20, 45, 70] {
        s.masks = vec![craft_develop_model::Mask {
            id: 1,
            adjust: craft_develop_model::LocalAdjustments { exposure: 1.5, ..Default::default() },
            components: vec![craft_develop_model::MaskComponent {
                name: None,
                op: craft_develop_model::MaskOp::Add,
                invert: false,
                shape: MaskShape::Brush { strokes: vec![stroke(path(n), true, false)] },
            }],
            ..Default::default()
        }];
        let fresh = render(&src, &info, &s, &req).image;
        let cached = render_cached(&src, &info, &s, &req, &pic).image;
        let worst = fresh.data.iter().zip(&cached.data).flat_map(|(a, b)| (0..3).map(move |c| a[c].abs_diff(b[c]))).max().unwrap_or(0);
        assert!(worst <= 1, "{n} points: the cached render is off by {worst}");
        let shown = render(&src, &info, &s, &oreq).image;
        let alpha = mask_alpha_cached(&src, &info, &s, &oreq, &overlay, 1).unwrap();
        assert_eq!((alpha.width, alpha.height), (w, h));
        let worst = shown.data.iter().zip(&alpha.data).map(|(p, a)| p[0].abs_diff((255.0 * a.clamp(0.0, 1.0)).round() as u8)).max().unwrap_or(0);
        assert!(worst <= 1, "{n} points: the mask-only alpha is off by {worst}");
    }
    assert!(mask_alpha_cached(&src, &info, &s, &oreq, &overlay, 9).is_none(), "no such mask");
}

/// Process 9's Auto Mask painted point by point is the committed stroke too, and processes 1 to 8
/// keep the per-pixel model (their renders unchanged).
#[test]
fn the_smoothed_model_painted_live_matches_the_committed_stroke_and_is_process_9s() {
    use craft_develop_model::Process;
    let (w, h) = (320, 200);
    let (img, l) = scene(w, h, 150);
    let f = frame(w, h);
    let s = stroke(path(120), true, false);
    let mut live = StrokeRaster::with_model(w, h, AutoMaskModel::Smoothed);
    for n in 1..=s.points.len() {
        let part = BrushStroke { points: s.points[..n].to_vec(), ..s.clone() };
        live.update(&brush_dabs(&part, &f, w, h), &img, &l);
    }
    let committed = crate::masks::shape_alpha_cached(
        &MaskShape::Brush { strokes: vec![s.clone()] },
        &f,
        w,
        h,
        &img,
        &l,
        0.0,
        None,
        AutoMaskModel::Smoothed,
        None,
    );
    let d = max_diff(&live.to_plane(), &committed);
    eprintln!("live vs committed smoothed Auto Mask stroke: max difference {d:e}");
    assert!(d < 1e-4, "live and committed differ by {d}");
    assert_eq!(AutoMaskModel::for_process(Process::V8), AutoMaskModel::PerPixel);
    assert_eq!(AutoMaskModel::for_process(Process::V9), AutoMaskModel::Smoothed);
    assert_eq!(AutoMaskModel::for_process(Process::LATEST), AutoMaskModel::Smoothed);
    // a V8 photo's Auto Mask renders exactly as before
    let old = shape_alpha(&MaskShape::Brush { strokes: vec![s.clone()] }, &f, w, h, &img, &l, 0.0, None);
    let v8 = crate::masks::shape_alpha_cached(
        &MaskShape::Brush { strokes: vec![s] },
        &f,
        w,
        h,
        &img,
        &l,
        0.0,
        None,
        AutoMaskModel::for_process(Process::V8),
        None,
    );
    assert_eq!(old.data, v8.data);
}

/// Process 9 fills a narrow crack between cells that belong, in any direction (a seam, a closed
/// eye's crease), and a speck; it leaves the object's edge and a wide notch in it alone.
#[test]
fn the_cell_map_fills_cracks_and_specks_but_not_notches_or_edges() {
    let (n, m) = (40usize, 40usize);
    let mut map = CellMap { i0: 0, j0: 0, n, m, v: vec![0.0; n * m] };
    // the object: the left 30 columns; a crack two cells wide slanting across it at about 60°,
    // a speck, and a notch open to the right a quarter turn wide
    for j in 0..m {
        for i in 0..30 {
            let (x, y) = (i as f64, j as f64);
            let crack = (y - 1.7 * (x - 5.0)).abs() < 1.97 && j < 30 && j > 2;
            let speck = (i, j) == (8, 34);
            let notch = (x - 30.0).abs() < (6.0 - (y - 34.0).abs()).max(0.0) && j > 28;
            map.v[j * n + i] = if crack || speck || notch { 0.0 } else { 1.0 };
        }
    }
    let f = |i: usize, j: usize| map.filled(i as i64, j as i64, HOLE_CELLS);
    // (a cell well inside the crack, the speck)
    assert_eq!(f(10, 9), 1.0, "crack");
    assert_eq!(f(8, 34), 1.0, "speck");
    // (the wall past the edge, the notch's mouth)
    assert_eq!(f(33, 10), 0.0, "past the edge");
    assert_eq!(f(29, 34), 0.0, "notch");
}
