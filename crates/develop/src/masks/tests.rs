use super::*;
use craft_develop_model::{DevelopSettings, MaskComponent};

fn frame(w: usize, h: usize) -> Frame {
    Frame::new(w, h, &DevelopSettings::default(), true)
}

#[test]
fn linear_gradient_ramps() {
    let f = frame(100, 50);
    let img = Rgb32f::new(100, 50);
    let l = Plane::new(100, 50);
    let a = shape_alpha(&MaskShape::Linear { start: Point::new(0.0, 0.0), end: Point::new(1.0, 0.0) }, &f, 100, 50, &img, &l, 0.0, None);
    assert!(a.get(1, 25) > 0.99 && a.get(98, 25) < 0.01);
    assert!((a.get(50, 25) - 0.5).abs() < 0.05);
}

#[test]
fn stored_segmentations_render_at_any_size_and_follow_the_crop() {
    // the left half selected, on a 16² logit grid
    let side = 16;
    let logits: Vec<f32> = (0..side * side).map(|i| if i % side < side / 2 { 12.0 } else { -12.0 }).collect();
    let seg = SegMask::from_logits(side, &logits);
    for shape in [
        MaskShape::Object { hint: vec![Point::new(0.25, 0.5)], exclude: vec![], seg: Some(seg.clone()), detail: vec![], edge: 0.0 },
        MaskShape::Prompt { text: "left".into(), seg: Some(seg.clone()), detail: vec![], edge: 0.0 },
    ] {
        for (w, h) in [(40, 20), (400, 200)] {
            let a = shape_alpha(&shape, &frame(w, h), w, h, &Rgb32f::new(w, h), &Plane::new(w, h), 0.0, None);
            assert!(a.get(1, h / 2) > 0.99 && a.get(w - 2, h / 2) < 0.01, "{w}×{h}");
            if w >= 400 {
                let soft = (0..w).filter(|x| (0.05..0.95).contains(&a.get(*x, h / 2))).count();
                assert!((2..30).contains(&soft), "an anti-aliased edge, not a ramp: {soft} px");
            }
        }
    }
    // cropped to the right half: nothing left selected
    let mut d = DevelopSettings::default();
    d.crop.geometry.rect = craft_geom::Rect::from_center(Point::new(0.75, 0.5), 0.5, 1.0);
    let f = Frame::new(40, 20, &d, true);
    let shape = MaskShape::Prompt { text: "left".into(), seg: Some(seg), detail: vec![], edge: 0.0 };
    let a = shape_alpha(&shape, &f, 20, 20, &Rgb32f::new(20, 20), &Plane::new(20, 20), 0.0, None);
    assert!(a.data.iter().all(|v| *v < 0.05));
}

#[test]
fn detail_patches_replace_the_coarse_mask_inside_their_rectangle() {
    // coarse: nothing; a patch over the right half selects its own left half (x 0.5..0.75)
    let coarse = SegMask::from_logits(4, &[-12.0; 16]);
    let side = 16;
    let l: Vec<f32> = (0..side * side).map(|i| if i % side < side / 2 { 12.0 } else { -12.0 }).collect();
    let patch = SegMask::from_logits_in(side, &l, [0.5, 0.0, 1.0, 1.0]);
    let shape = MaskShape::Object { hint: vec![Point::new(0.6, 0.5)], exclude: vec![], seg: Some(coarse), detail: vec![patch], edge: 0.0 };
    let (w, h) = (200, 100);
    let a = shape_alpha(&shape, &frame(w, h), w, h, &Rgb32f::new(w, h), &Plane::new(w, h), 0.0, None);
    assert!(a.get(20, 50) < 0.01, "outside the patch: the coarse mask");
    assert!(a.get(120, 50) > 0.99, "inside the patch, its selected half");
    assert!(a.get(190, 50) < 0.01, "inside the patch, its unselected half");
}

#[test]
fn a_hostile_number_of_detail_patches_is_capped() {
    // built in memory (a deserialized document is capped already): only the first
    // MAX_DETAIL are decoded and sampled, so this needs 4 × 4 MB, not 20 000 × 4 MB
    let side = 1024;
    let l: Vec<f32> = vec![12.0; side * side];
    let patch = SegMask::from_logits_in(side, &l, [0.0, 0.0, 0.5, 1.0]);
    let shape = MaskShape::Object { hint: vec![Point::new(0.2, 0.5)], exclude: vec![], seg: None, detail: vec![patch; 20_000], edge: 0.0 };
    let (w, h) = (64, 32);
    let t = std::time::Instant::now();
    let a = shape_alpha(&shape, &frame(w, h), w, h, &Rgb32f::new(w, h), &Plane::new(w, h), 0.0, None);
    assert!(a.get(5, 16) > 0.99 && a.get(60, 16) < 0.01);
    assert!(t.elapsed() < std::time::Duration::from_secs(20), "{:?}", t.elapsed());
}

#[test]
fn the_edge_setting_hardens_or_feathers_the_transition() {
    let side = 16;
    let l: Vec<f32> = (0..side * side).map(|i| ((i % side) as f32 - 7.5) * -0.6).collect(); // a gentle ramp
    let seg = SegMask::from_logits(side, &l);
    let (w, h) = (400, 40);
    let soft_px = |edge: f64| {
        let shape = MaskShape::Prompt { text: "x".into(), seg: Some(seg.clone()), detail: vec![], edge };
        let a = shape_alpha(&shape, &frame(w, h), w, h, &Rgb32f::new(w, h), &Plane::new(w, h), 0.0, None);
        assert!(a.get(2, h / 2) > 0.9 && a.get(w - 3, h / 2) < 0.1, "still the same selection at {edge}");
        (0..w).filter(|x| (0.1..0.9).contains(&a.get(*x, h / 2))).count()
    };
    let (hard, plain, soft) = (soft_px(-100.0), soft_px(0.0), soft_px(100.0));
    assert!(hard < plain && plain < soft, "transition widths {hard} < {plain} < {soft}");
}

/// A dark disc (radius 0.3 of the frame) on a light background, `n × n`: the photo and its log
/// luminance, and a model's view of it on a `side²` grid (each cell's logit from how much of the
/// cell the disc covers, as a low-resolution segmenter sees it).
fn disc_scene(n: usize, side: usize) -> (Rgb32f, Plane, SegMask) {
    const R: f64 = 0.3;
    let dark = |x: f64, y: f64| (x - 0.5).hypot(y - 0.5) < R;
    let lum = |x: usize, y: usize| if dark((x as f64 + 0.5) / n as f64, (y as f64 + 0.5) / n as f64) { 0.05f32 } else { 0.6 };
    let img = Rgb32f::from_fn(n, n, |x, y| [lum(x, y); 3]);
    let log_l = img.map(crate::local::log_lum);
    let logits: Vec<f32> = (0..side * side)
        .map(|i| {
            let (cx, cy) = (i % side, i / side);
            let sub = 8;
            let cover = (0..sub * sub)
                .filter(|k| {
                    dark((cx as f64 + (k % sub) as f64 / sub as f64) / side as f64, (cy as f64 + (k / sub) as f64 / sub as f64) / side as f64)
                })
                .count() as f32
                / (sub * sub) as f32;
            12.0 * (2.0 * cover - 1.0)
        })
        .collect();
    (img, log_l, SegMask::from_logits(side, &logits))
}

/// Alpha along the disc's horizontal radius, `d` px outside (> 0) or inside (< 0) its edge.
fn across_disc_edge(a: &Plane, d: i32) -> f32 {
    let n = a.width;
    let edge = (0.8 * n as f64).round() as i32; // first background pixel on the right
    let x = if d > 0 { edge + d - 1 } else { edge + d };
    a.get(x as usize, n / 2)
}

/// Issue storytold/lightcraft#3272: a mask from a low-resolution model grid spilled 1–2 px onto the background (a bright
/// outline once its Exposure was raised). Enlarged along the photo's edges, it is ≈ 0 one and two
/// pixels outside the subject, ≈ 1 inside, and exactly what it was away from the edge.
#[test]
fn ai_masks_follow_the_photos_edge_when_enlarged() {
    let (n, side) = (512, 32); // 16 px cells, as a 288² grid on a ~4600 px export
    let (img, log_l, seg) = disc_scene(n, side);
    let shape = MaskShape::Prompt { text: "disc".into(), seg: Some(seg), detail: vec![], edge: 0.0 };
    let f = frame(n, n);
    let a = shape_alpha(&shape, &f, n, n, &img, &log_l, 0.0, None);
    for d in [1, 2] {
        assert!(across_disc_edge(&a, d) < 0.05, "{d} px outside: {}", across_disc_edge(&a, d));
        assert!(across_disc_edge(&a, -d) > 0.95, "{d} px inside: {}", across_disc_edge(&a, -d));
    }
    // all around the disc, not only along one row
    let r_px = 0.3 * n as f64;
    for k in 0..64 {
        let t = k as f64 / 64.0 * std::f64::consts::TAU;
        let at = |r: f64| a.get((256.0 + r * t.cos()) as usize, (256.0 + r * t.sin()) as usize);
        assert!(at(r_px + 2.5) < 0.05 && at(r_px - 2.5) > 0.95, "at {t:.2} rad: {} / {}", at(r_px + 2.5), at(r_px - 2.5));
    }
    // away from the edge the selection is the plain sample of the grid, to the bit
    let flat = Plane::new(n, n);
    let plain = shape_alpha(&shape, &f, n, n, &img, &flat, 0.0, None);
    for (x, y) in [(256, 256), (200, 300), (10, 10), (500, 256), (256, 505)] {
        assert_eq!(a.get(x, y), plain.get(x, y), "({x}, {y})");
    }
}

/// Refine Edges on top of an enlarged AI mask: the second filter finds the edge already in place, so
/// there is no second halo and the edge doesn't move inwards.
#[test]
fn refine_edges_on_an_enlarged_ai_mask_neither_halos_nor_tightens() {
    let (n, side) = (512, 32);
    let (img, log_l, seg) = disc_scene(n, side);
    let f = frame(n, n);
    let mask = |refine: f64| Mask {
        components: vec![MaskComponent {
            name: None,
            op: MaskOp::Add,
            invert: false,
            shape: MaskShape::Prompt { text: "disc".into(), seg: Some(seg.clone()), detail: vec![], edge: 0.0 },
        }],
        refine,
        ..Default::default()
    };
    let crossing = |a: &Plane| (0..n).rev().find(|x| a.get(*x, n / 2) > 0.5).unwrap_or(0);
    let plain = evaluate_one(&mask(0.0), &f, n, n, &img, &log_l, 0.0, None);
    for refine in [50.0, 100.0] {
        let a = evaluate_one(&mask(refine), &f, n, n, &img, &log_l, 0.0, None);
        for d in [1, 2] {
            assert!(across_disc_edge(&a, d) < 0.05, "refine {refine}, {d} px outside: {}", across_disc_edge(&a, d));
            assert!(across_disc_edge(&a, -d) > 0.95, "refine {refine}, {d} px inside: {}", across_disc_edge(&a, -d));
        }
        assert!(crossing(&a).abs_diff(crossing(&plain)) <= 1, "refine {refine}: edge at {} vs {}", crossing(&a), crossing(&plain));
    }
}

/// The window follows the grid, not the output: a preview and an export put the edge in the same
/// place of the photo (and on its edge).
#[test]
fn enlarged_ai_masks_put_the_edge_in_the_same_place_at_any_size() {
    let side = 32;
    let mut edges = Vec::new();
    for n in [256, 512, 1024] {
        let (img, log_l, seg) = disc_scene(n, side);
        let shape = MaskShape::Prompt { text: "disc".into(), seg: Some(seg), detail: vec![], edge: 0.0 };
        let a = shape_alpha(&shape, &frame(n, n), n, n, &img, &log_l, 0.0, None);
        // the edge's sub-pixel position along the row: the integral of alpha past the centre
        let covered: f32 = (n / 2..n).map(|x| a.get(x, n / 2)).sum();
        edges.push((n / 2) as f64 + covered as f64);
        let x = edges.last().copied().unwrap_or(0.0) / n as f64;
        assert!((x - 0.8).abs() < 1.0 / n as f64, "{n} px: edge at {x:.4} of the width, the disc's at 0.8");
    }
}

#[test]
fn ai_shapes_without_a_segmentation_select_nothing() {
    let f = frame(30, 30);
    let (img, l) = (Rgb32f::new(30, 30), Plane::new(30, 30));
    for shape in [
        MaskShape::Object { hint: vec![], exclude: vec![], seg: None, detail: vec![], edge: 0.0 },
        MaskShape::Prompt { text: "sky".into(), seg: None, detail: vec![], edge: 0.0 },
        MaskShape::Prompt {
            text: "sky".into(),
            seg: Some(SegMask { side: 4, data: "damaged!".into(), rect: None, grid: None }),
            detail: vec![],
            edge: 0.0,
        },
    ] {
        assert!(shape_alpha(&shape, &f, 30, 30, &img, &l, 0.0, None).data.iter().all(|v| *v == 0.0));
    }
}

#[test]
fn radial_inside_and_invert() {
    let f = frame(100, 100);
    let img = Rgb32f::new(100, 100);
    let l = Plane::new(100, 100);
    let shape = MaskShape::Radial { center: Point::new(0.5, 0.5), rx: 0.2, ry: 0.2, angle: 0.0, feather: 20.0, invert: false };
    let a = shape_alpha(&shape, &f, 100, 100, &img, &l, 0.0, None);
    assert!(a.get(50, 50) > 0.99 && a.get(5, 5) < 0.01);
    let m = Mask { components: vec![MaskComponent { name: None, op: MaskOp::Add, invert: true, shape }], ..Default::default() };
    let e = evaluate(&[m], &f, 100, 100, &img, &l, 0.0, None);
    assert!(e[0].alpha.get(50, 50) < 0.01);
}

/// storytold/lightcraft#1520: a caller that will not look past its own threshold builds to that threshold, not to
/// the stroke budget. Each segment here is admissible on its own and together they want far
/// more than any threshold the callers apply, which is the aggregate the issue describes
/// rather than one hostile segment.
#[test]
fn a_stroke_is_built_only_as_far_as_the_caller_will_look() {
    let f = frame(4000, 4000);
    // Twenty long sweeps across the output: tens of thousands of dabs between them.
    let points: Vec<Point> =
        (0..20).map(|i| if i % 2 == 0 { Point::new(0.02, 0.02 + i as f64 * 0.04) } else { Point::new(0.98, 0.02 + i as f64 * 0.04) }).collect();
    let s = BrushStroke { points, size: 0.002, feather: 50.0, flow: 100.0, density: 100.0, ..Default::default() };

    let whole = brush_dabs(&s, &f, 4000, 4000);
    assert!(whole.dabs.len() > 20_000, "the stroke really is large: {}", whole.dabs.len());
    assert!(whole.dabs.len() <= MAX_STROKE_DABS);

    // Each caller gets what it asked for and no more, however large the stroke is. 4097 and
    // 200_001 are what the GPU path and the Auto Mask spot window now ask for.
    for max in [1usize, 2, 4097, 200_001] {
        let got = brush_dabs_within(&s, &f, 4000, 4000, max);
        assert!(got.dabs.len() <= max, "asked for at most {max}, built {}", got.dabs.len());
        assert_eq!(got.r, whole.r, "the brush itself is unchanged");
    }

    // A budget the stroke never reaches changes nothing: it only ever truncates.
    let generous = brush_dabs_within(&s, &f, 4000, 4000, whole.dabs.len() + 1000);
    assert_eq!(generous.dabs, whole.dabs, "an unreached budget leaves the stroke alone");

    // The guarantee the callers rely on: a result short of the budget was never truncated,
    // so it is the stroke itself. Both callers ask for one more than they will accept, so
    // anything they keep is identical to what they drew before this change, and anything
    // truncated is something they were going to reject anyway.
    //
    // A truncated one is not a prefix: running out of room makes a segment lay its remaining
    // dabs further apart rather than stop mid-segment, which is how `push_dabs_counted`
    // already behaved at MAX_STROKE_DABS. That only ever reaches a caller that rejects it.
    let cut = brush_dabs_within(&s, &f, 4000, 4000, 500);
    assert_eq!(cut.dabs.len(), 500, "a stroke past the budget fills it exactly");
    for max in [700usize, 5_000, whole.dabs.len()] {
        let got = brush_dabs_within(&s, &f, 4000, 4000, max);
        if got.dabs.len() < max {
            assert_eq!(got.dabs, whole.dabs, "a result short of its budget is the whole stroke");
        }
    }
}

/// Issue storytold/lightcraft#517: dabs along a long segment (a fast stroke: two pointer samples far apart) are at
/// most [`dab_spacing`] apart, painting dab by dab (the live stroke) or the whole path at once
/// (the committed one).
#[test]
fn a_long_segment_gets_dabs_at_most_the_spacing_apart() {
    let r = 12.0;
    let pts = [Point::new(10.0, 20.0), Point::new(610.0, 260.0), Point::new(611.0, 260.0), Point::new(40.0, 400.0)];
    let mut live = Vec::new();
    let mut prev = None;
    for q in pts {
        if push_dabs(&mut live, prev, q, r) {
            prev = Some(q);
        }
    }
    assert!(live.len() > 3 * 600 / 12, "{} dabs", live.len());
    assert!(live.windows(2).all(|w| w[0].dist(w[1]) <= dab_spacing(r) + 1e-9));
    assert_eq!(live.first(), pts.first());
    assert_eq!(live.last(), pts.last());
    // the committed stroke places the same dabs (in output pixels; this frame maps 1:1 scaled)
    let (w, h) = (1000, 500);
    let f = frame(w, h);
    let stroke = BrushStroke { points: pts.map(|p| Point::new(p.x / w as f64, p.y / h as f64)).to_vec(), ..Default::default() };
    let committed = brush_dabs(&stroke, &f, w, h);
    let mut again = Vec::new();
    let mut prev = None;
    for p in &stroke.points {
        let q = f.norm_to_out(w, h).apply(*p);
        if push_dabs(&mut again, prev, q, committed.r) {
            prev = Some(q);
        }
    }
    assert_eq!(committed.dabs, again);
    assert!(committed.dabs.windows(2).all(|w| w[0].dist(w[1]) <= dab_spacing(committed.r) + 1e-9));
    // a huge segment lays only its end point, so many of them add one dab each instead of
    // filling the stroke budget with intermediate dabs
    let mut big = Vec::new();
    let mut at = Point::new(0.0, 0.0);
    for i in 0..5 {
        let q = Point::new(if i % 2 == 0 { 1e12 } else { 0.0 }, 0.0);
        if push_dabs(&mut big, Some(at), q, 1.0) {
            at = q;
        }
    }
    assert_eq!(big.len(), 5, "an extreme segment lays no intermediate dabs");
    let mut spent = MAX_STROKE_DABS;
    assert!(!push_dabs_counted(&mut big, &mut spent, Some(at), Point::new(5.0, 5.0), 1.0), "a full stroke takes no more");
    // a hostile segment places no in-between dabs
    let mut v = Vec::new();
    assert!(!push_dabs(&mut v, Some(Point::new(f64::NAN, 0.0)), Point::new(1.0, 1.0), r));
    assert!(!push_dabs(&mut v, None, Point::new(f64::INFINITY, 1.0), r));
    push_dabs(&mut v, Some(Point::new(0.0, 0.0)), Point::new(1e300, 1.0), 0.0);
    assert!(v.len() <= 1_000_002, "{}", v.len());
}

#[test]
fn oversized_brush_segments_skip_intermediate_dabs() {
    let mut dabs = vec![Point::new(10.0, 10.0)];
    let far = Point::new(10_000_000.0, 10.0);
    assert!(push_dabs(&mut dabs, Some(Point::new(10.0, 10.0)), far, 2.0));
    assert_eq!(dabs.len(), 2, "an extreme segment must not allocate a million intermediate dabs");
    assert_eq!(dabs.last(), Some(&far));

    // Ordinary feathered-brush paths still get overlapping dabs and their endpoint.
    let mut regular = vec![Point::new(10.0, 10.0)];
    let end = Point::new(30.0, 10.0);
    assert!(push_dabs(&mut regular, Some(Point::new(10.0, 10.0)), end, 4.0));
    assert_eq!(regular.len(), 21);
    assert_eq!(regular.last(), Some(&end));
    assert!(regular.windows(2).all(|pair| pair[0].dist(pair[1]) <= dab_spacing(4.0)));
}

/// A temporary output buffer must spend the same stroke budget as a retained one.
#[test]
fn incremental_dab_buffers_share_the_stroke_budget() {
    let points = [Point::new(20.0, 20.0), Point::new(80.0, 20.0), Point::new(200.0, 80.0), Point::new(21.0, 20.0)];
    let mut retained = Vec::new();
    let mut streamed = Vec::new();
    let mut retained_count = MAX_STROKE_DABS - 3;
    let mut streamed_count = retained_count;
    let mut prev = None;
    for q in points {
        let mut chunk = Vec::new();
        let accepted = push_dabs_counted(&mut retained, &mut retained_count, prev, q, 10.0);
        assert_eq!(accepted, push_dabs_counted(&mut chunk, &mut streamed_count, prev, q, 10.0));
        streamed.extend(chunk);
        if accepted {
            prev = Some(q);
        }
    }
    assert_eq!(retained, streamed);
    assert_eq!(retained.len(), 3);
    assert_eq!(retained_count, MAX_STROKE_DABS);
    assert_eq!(streamed_count, retained_count);
    assert_eq!(prev, Some(points[1]));
    assert!(!retained.contains(&points[2]));
    assert!(!retained.contains(&points[3]));
}

/// Pen pressure scales each dab's radius and flow; a stroke without pressure data paints as before, and hostile
/// pressures (NaN, out of range, a short list) neither panic nor paint more than full pressure.
#[test]
fn pen_pressure_scales_dab_size_and_flow() {
    let (w, h) = (400, 100);
    let f = frame(w, h);
    let img = Rgb32f::new(w, h);
    let l = Plane::new(w, h);
    let line = vec![Point::new(0.1, 0.5), Point::new(0.9, 0.5)];
    let alpha = |pressure: Vec<f32>| {
        let s = BrushStroke { points: line.clone(), size: 0.05, feather: 0.0, flow: 100.0, pressure, ..Default::default() };
        shape_alpha(&MaskShape::Brush { strokes: vec![s] }, &f, w, h, &img, &l, 0.0, None)
    };
    let full = alpha(vec![]);
    assert_eq!(full.data, alpha(vec![1.0, 1.0]).data, "full pressure is no pressure");
    // light at the start, full at the end: thin and faint on the left, as wide as the brush on the right
    let ramp = alpha(vec![0.1, 1.0]);
    let width = |a: &Plane, x: usize| (0..h).filter(|y| a.get(x, *y) > 0.01).count();
    assert!(width(&ramp, 60) < width(&full, 60) / 2, "{} vs {}", width(&ramp, 60), width(&full, 60));
    assert!(width(&ramp, 350) + 2 >= width(&full, 350));
    assert!(ramp.get(60, 50) < full.get(60, 50) + 1e-6);
    let faint = alpha(vec![0.2]);
    assert!(faint.data.iter().zip(&full.data).all(|(a, b)| *a <= *b + 1e-6), "never more than full pressure");
    assert!(faint.get(200, 50) > 0.0 && faint.get(200, 50) < full.get(200, 50));
    for hostile in [vec![f32::NAN, f32::INFINITY], vec![-3.0], vec![0.5; 10_000]] {
        let a = alpha(hostile);
        assert!(a.data.iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v)));
    }
    assert_eq!(pressure_radius(10.0, f32::NAN), 10.0);
    assert_eq!(pressure_radius(10.0, 0.0), 1.0);
    assert_eq!(pressure_flow(0.8, 2.0), 0.8);
}

#[test]
fn brush_and_subtract() {
    let f = frame(200, 100);
    let img = Rgb32f::new(200, 100);
    let l = Plane::new(200, 100);
    let stroke = BrushStroke { points: vec![Point::new(0.1, 0.5), Point::new(0.9, 0.5)], size: 0.05, ..Default::default() };
    let erase = BrushStroke { points: vec![Point::new(0.5, 0.5)], size: 0.05, erase: true, feather: 0.0, ..Default::default() };
    let m = Mask {
        components: vec![MaskComponent { name: None, op: MaskOp::Add, invert: false, shape: MaskShape::Brush { strokes: vec![stroke, erase] } }],
        ..Default::default()
    };
    let e = evaluate(&[m], &f, 200, 100, &img, &l, 0.0, None);
    let a = &e[0].alpha;
    assert!(a.get(40, 50) > 0.9, "{}", a.get(40, 50));
    assert!(a.get(100, 50) < 0.05, "erased centre {}", a.get(100, 50));
    assert!(a.get(40, 5) < 0.01);
}

/// storytold/lightcraft#518: holding the brush still adds no coverage. A press plus any number of stationary
/// pointer samples paints exactly the single dab of the press, whatever the frame rate.
#[test]
fn holding_the_brush_still_adds_no_coverage() {
    let (w, h) = (200, 100);
    let f = frame(w, h);
    let img = Rgb32f::new(w, h);
    let l = Plane::new(w, h);
    let p = Point::new(0.5, 0.5);
    let spot = |samples: usize, flow: f64| {
        let s = BrushStroke { points: vec![p; samples], size: 0.1, feather: 80.0, flow, ..Default::default() };
        shape_alpha(&MaskShape::Brush { strokes: vec![s] }, &f, w, h, &img, &l, 0.0, None)
    };
    for flow in [100.0, 40.0] {
        let press = spot(1, flow);
        // the feathered edge is soft to begin with
        let edge = press.get(100 + 14, 50);
        assert!(edge > 0.05 && edge < 0.6, "flow {flow}: edge {edge}");
        for n in [2, 5, 60, 600] {
            let held = spot(n, flow);
            assert!(held.data == press.data, "flow {flow}: {n} stationary samples hardened the edge {edge} → {}", held.get(100 + 14, 50));
        }
    }
    let s = BrushStroke { points: vec![p; 60], size: 0.1, ..Default::default() };
    assert_eq!(brush_dabs(&s, &f, w, h).dabs.len(), 1, "one dab for the press");
    // a stroke that pauses partway lays the same dabs as one that doesn't
    let (a, b) = (Point::new(0.2, 0.5), Point::new(0.8, 0.5));
    let paused = BrushStroke { points: vec![a, p, p, p, p, b], flow: 30.0, ..Default::default() };
    let smooth = BrushStroke { points: vec![a, p, b], flow: 30.0, ..Default::default() };
    assert_eq!(brush_dabs(&paused, &f, w, h).dabs, brush_dabs(&smooth, &f, w, h).dabs);
}

/// Flow builds up by passes: going back over an area in the same stroke adds coverage.
#[test]
fn a_second_pass_builds_up_with_low_flow() {
    let (w, h) = (200, 100);
    let f = frame(w, h);
    let img = Rgb32f::new(w, h);
    let l = Plane::new(w, h);
    let (a, b) = (Point::new(0.2, 0.5), Point::new(0.8, 0.5));
    let paint = |points: Vec<Point>| {
        let s = BrushStroke { points, size: 0.05, feather: 50.0, flow: 20.0, ..Default::default() };
        shape_alpha(&MaskShape::Brush { strokes: vec![s] }, &f, w, h, &img, &l, 0.0, None)
    };
    let once = paint(vec![a, b]);
    let twice = paint(vec![a, b, a]);
    let thrice = paint(vec![a, b, a, b]);
    let (o, t, th) = (once.get(100, 50), twice.get(100, 50), thrice.get(100, 50));
    assert!(o > 0.05 && o < 0.99, "one pass {o}");
    assert!(t > o + 0.02 && th > t + 0.01, "passes build up: {o} → {t} → {th}");
}

#[test]
fn auto_mask_stops_at_edges() {
    // dark left half, bright right half; a stroke along the boundary, centred on the dark side
    let (w, h) = (200, 100);
    let f = frame(w, h);
    let img = Rgb32f::from_fn(w, h, |x, _| if x < 100 { [0.03; 3] } else { [0.6, 0.5, 0.4] });
    let l = img.map(crate::local::log_lum);
    let stroke = |auto_mask| BrushStroke {
        points: vec![Point::new(0.47, 0.2), Point::new(0.47, 0.8)],
        size: 0.08,
        feather: 0.0,
        auto_mask,
        ..Default::default()
    };
    let comp = |auto| MaskShape::Brush { strokes: vec![stroke(auto)] };
    let plain = shape_alpha(&comp(false), &f, w, h, &img, &l, 0.0, None);
    let auto = shape_alpha(&comp(true), &f, w, h, &img, &l, 0.0, None);
    // without Auto Mask the brush spills over the edge; with it, it stays on the dark side
    assert!(plain.get(105, 50) > 0.9, "{}", plain.get(105, 50));
    assert!(auto.get(105, 50) < 0.05, "spill {}", auto.get(105, 50));
    assert!(auto.get(110, 50) < 0.02);
    assert!(auto.get(90, 50) > 0.9, "painted side {}", auto.get(90, 50));
    assert!(auto.get(99, 50) > 0.8, "up to the edge {}", auto.get(99, 50));
    // on a flat area Auto Mask paints like the plain brush
    let flat = Rgb32f::from_fn(w, h, |_, _| [0.2; 3]);
    let fl = flat.map(crate::local::log_lum);
    let a = shape_alpha(&comp(true), &f, w, h, &flat, &fl, 0.0, None);
    let b = shape_alpha(&comp(false), &f, w, h, &flat, &fl, 0.0, None);
    assert!((a.get(94, 50) - b.get(94, 50)).abs() < 0.02, "{} vs {}", a.get(94, 50), b.get(94, 50));
}

/// Refine Edges: a soft mask over a hard edge in the photo snaps to that edge.
#[test]
fn refine_edges_follows_the_photo() {
    let (w, h) = (120usize, 80usize);
    // left half dark, right half bright
    let img = Rgb32f::from_fn(w, h, |x, _| if x < 60 { [0.05; 3] } else { [0.6; 3] });
    let log_l = Plane::from_fn(w, h, |x, _| if x < 60 { (0.05f32).log2() } else { (0.6f32).log2() });
    let f = Frame::new(w, h, &Default::default(), true);
    let soft = Mask {
        components: vec![MaskComponent {
            name: None,
            op: MaskOp::Add,
            invert: false,
            shape: MaskShape::Linear { start: Point::new(0.75, 0.5), end: Point::new(0.25, 0.5) },
        }],
        ..Default::default()
    };
    let refined = Mask { refine: 100.0, ..soft.clone() };
    let step = |m: &Mask| {
        let a = evaluate_one(m, &f, w, h, &img, &log_l, 0.0, None);
        a.data[40 * w + 63] - a.data[40 * w + 56]
    };
    let (plain, sharp) = (step(&soft), step(&refined));
    assert!(sharp > plain * 1.5 && sharp > 0.1, "the edge in the mask follows the photo's: {plain} → {sharp}");
}

/// Issue storytold/lightcraft#157: a colour-range sample of one flat patch must not select a clearly different
/// patch. The two patches are the ones from the report (sRGB 120,150,155 and 200,198,195),
/// sampled the way `mask.sampleColor` does (3×3 mean of the selection-space OkLab).
#[test]
fn color_range_keeps_to_the_sampled_colour() {
    use crate::color::{REC2020, SRGB, transfer::decode_srgb8};
    let (w, h) = (120usize, 40usize);
    let m = SRGB.to_space(&REC2020);
    let patch = |rgb: [u8; 3]| m.apply_f32(rgb.map(decode_srgb8));
    let (left, right) = (patch([120, 150, 155]), patch([200, 198, 195]));
    let img = Rgb32f::from_fn(w, h, |x, _| if x < 60 { left } else { right });
    let log_l = img.map(crate::local::log_lum);
    let f = Frame::new(w, h, &Default::default(), true);
    let mut sample = [0f64; 3];
    for y in 19..=21 {
        for x in 29..=31 {
            let lab = crate::color::perceptual::oklab_from_2020(tonemap_for_select(img.data[y * w + x]));
            for k in 0..3 {
                sample[k] += lab[k] as f64 / 9.0;
            }
        }
    }
    let alpha = |refine| {
        let shape = MaskShape::ColorRange { samples: vec![sample], refine };
        let a = shape_alpha(&shape, &f, w, h, &img, &log_l, 0.0, None);
        (a.get(30, 20), a.get(90, 20))
    };
    for refine in [0.0, 50.0, 85.0] {
        let (l, r) = alpha(refine);
        assert!(l > 0.95, "refine {refine}: sampled patch {l}");
        assert!(r < 0.05, "refine {refine}: other patch {r}");
    }
    // Refine 100 reaches furthest (the grey is ~0.063 away, the fade ends at 0.075), but the
    // other patch stays mostly out
    let (l, r) = alpha(100.0);
    assert!(l > 0.95 && r < 0.3, "refine 100: {l} vs {r}");
}

/// A half-resolution matte selecting the left `frac` of a `w × h` source.
fn left_matte(w: usize, h: usize, frac: f64) -> Image<u8> {
    Image::from_fn(w / 2, h / 2, |x, _| if (x as f64 + 0.5) < frac * (w / 2) as f64 { 255 } else { 0 })
}

/// The Sky mask follows the source's sky matte where it has one (here: the bottom-left of a
/// uniformly bright blue photo, where the heuristic sees no sky, and not its top right, where
/// it does), through user orientation, at any output size; without one it keeps the heuristic.
#[test]
fn sky_uses_the_sources_matte() {
    let (w, h) = (200, 100);
    let img = Rgb32f::filled(w, h, [0.5, 0.7, 1.4]);
    let l = img.map(crate::local::log_lum);
    let mut mattes = Mattes::default();
    mattes.push(MatteKind::Sky, left_matte(w, h, 0.5));
    let f = frame(w, h);
    let heuristic = shape_alpha(&MaskShape::Sky, &f, w, h, &img, &l, 0.0, None);
    let matte = shape_alpha(&MaskShape::Sky, &f, w, h, &img, &l, 0.0, Some(&mattes));
    assert!(heuristic.get(150, 5) > 0.9 && heuristic.get(25, 90) < 0.1, "{} {}", heuristic.get(150, 5), heuristic.get(25, 90));
    assert!(matte.get(150, 5) < 0.01 && matte.get(25, 90) > 0.99, "{} {}", matte.get(150, 5), matte.get(25, 90));
    // mattes of other kinds leave the heuristic alone
    let mut hair = Mattes::default();
    hair.push(MatteKind::Hair, left_matte(w, h, 0.5));
    assert_eq!(shape_alpha(&MaskShape::Sky, &f, w, h, &img, &l, 0.0, Some(&hair)), heuristic);
    // rotated 90° clockwise, the source's left half is the top half; a preview and a larger
    // render agree
    let s = craft_develop_model::DevelopSettings { orientation: Orientation::Rotate90, ..Default::default() };
    let f = Frame::new(w, h, &s, true);
    for (ow, oh) in [(100, 200), (25, 50)] {
        let small = Rgb32f::filled(ow, oh, [0.5, 0.7, 1.4]);
        let a = shape_alpha(&MaskShape::Sky, &f, ow, oh, &small, &small.map(crate::local::log_lum), 0.0, Some(&mattes));
        let at = |x: f64, y: f64| a.get((x * ow as f64) as usize, (y * oh as f64) as usize);
        assert!(at(0.5, 0.2) > 0.99 && at(0.5, 0.8) < 0.01, "{ow}×{oh}: {} {}", at(0.5, 0.2), at(0.5, 0.8));
    }
}

/// Subject, Background and People use person mattes: the whole person is the union of the
/// parts; a People part without a matte keeps the heuristic.
#[test]
fn people_and_subject_use_person_mattes() {
    let (w, h) = (200, 100);
    let img = Rgb32f::filled(w, h, [0.2; 3]);
    let l = img.map(crate::local::log_lum);
    let mut mattes = Mattes::default();
    mattes.push(MatteKind::Skin, left_matte(w, h, 0.25));
    let right: Image<u8> = Image::from_fn(w / 2, h / 2, |x, _| if x >= 75 { 255 } else { 0 });
    mattes.push(MatteKind::Hair, right);
    let f = frame(w, h);
    let eval = |shape: &MaskShape| shape_alpha(shape, &f, w, h, &img, &l, 0.0, Some(&mattes));
    let subject = eval(&MaskShape::Subject);
    assert!(subject.get(10, 50) > 0.99 && subject.get(190, 50) > 0.99 && subject.get(100, 50) < 0.01);
    let background = eval(&MaskShape::Background);
    assert!(background.get(100, 50) > 0.99 && background.get(10, 50) < 0.01);
    let hair = eval(&MaskShape::People { person: 0, parts: vec!["Hair".into()] });
    assert!(hair.get(190, 50) > 0.99 && hair.get(10, 50) < 0.01);
    let lips = MaskShape::People { person: 0, parts: vec!["Lips".into()] };
    assert_eq!(eval(&lips), shape_alpha(&lips, &f, w, h, &img, &l, 0.0, None));
}

/// A thousand two-point strokes, each a sweep far past the photo that lays most of a stroke's
/// own budget: the mask shares one budget across its strokes, so it renders in moments rather
/// than building and visiting most of a billion dabs.
#[test]
fn many_far_flung_strokes_share_one_dab_budget() {
    let (w, h) = (64, 64);
    let s = BrushStroke { points: vec![Point::new(-4000.0, 0.5), Point::new(4000.0, 0.5)], size: 0.05, ..Default::default() };
    assert!(brush_dabs(&s, &frame(w, h), w, h).dabs.len() > 500_000, "each stroke alone is large");
    let strokes = vec![s; 1000];
    let t = std::time::Instant::now();
    let mut out = Plane::new(w, h);
    rasterize_brush(&strokes, &frame(w, h), w, h, &Rgb32f::new(w, h), &Plane::new(w, h), &mut out, AutoMaskModel::PerPixel, None);
    assert!(t.elapsed().as_secs() < 30, "took {:?}", t.elapsed());
    assert!(out.data.iter().any(|&v| v > 0.0), "the strokes crossing the photo still paint it");
}

/// Past the shared dab budget a brush mask is an error for the caller, not a mask missing its later (erase) strokes.
#[test]
fn an_over_budget_brush_mask_is_reported() {
    let (w, h) = (64, 64);
    let far = BrushStroke { points: vec![Point::new(-4000.0, 0.5), Point::new(4000.0, 0.5)], size: 0.05, ..Default::default() };
    let near = BrushStroke { points: vec![Point::new(0.1, 0.1), Point::new(0.9, 0.9)], size: 0.05, ..Default::default() };
    assert!(!brush_over_budget(std::slice::from_ref(&near), &frame(w, h), w, h));
    assert!(brush_over_budget(&vec![far; 10], &frame(w, h), w, h));
}

/// Strokes that fill the budget exactly are not over it; one more dab is.
#[test]
fn a_brush_mask_that_exactly_fills_the_budget_is_not_over_it() {
    let (w, h) = (64, 64);
    let stroke = |x: f64| BrushStroke { points: vec![Point::new(0.0, 0.5), Point::new(x, 0.5)], size: 0.05, ..Default::default() };
    // a stroke whose dab count divides the budget evenly
    let (s, m) = (0..14_000)
        .map(|i| stroke(6.0 + f64::from(i) * 0.001))
        .map(|s| {
            let m = brush_dabs(&s, &frame(w, h), w, h).dabs.len();
            (s, m)
        })
        .find(|(_, m)| *m >= 500 && MAX_STROKE_DABS.is_multiple_of(*m))
        .expect("a stroke whose dabs divide the budget");
    let k = MAX_STROKE_DABS / m;
    assert!(!brush_over_budget(&vec![s.clone(); k], &frame(w, h), w, h), "{k} strokes of {m} dabs fill the budget exactly");
    assert!(brush_over_budget(&vec![s; k + 1], &frame(w, h), w, h), "one more stroke is over it");
}

#[test]
fn intersect_first_component_selects_its_shape() {
    let f = frame(100, 100);
    let img = Rgb32f::new(100, 100);
    let l = Plane::new(100, 100);
    let shape = MaskShape::Radial { center: Point::new(0.5, 0.5), rx: 0.2, ry: 0.2, angle: 0.0, feather: 20.0, invert: false };
    let m = Mask { components: vec![MaskComponent { name: None, op: MaskOp::Intersect, invert: false, shape }], ..Default::default() };
    let a = evaluate_one(&m, &f, 100, 100, &img, &l, 0.0, None);
    assert!(a.get(50, 50) > 0.99 && a.get(5, 5) < 0.01, "intersect first: {} {}", a.get(50, 50), a.get(5, 5));
}
