use super::*;

#[test]
fn ruler_rect_calculations_are_accurate() {
    let full = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
    let rects = ruler_rects(full, RULER_THICKNESS);

    assert_eq!(rects.corner, Rect::from_min_size(Pos2::ZERO, vec2(16.0, 16.0)));
    assert_eq!(rects.top, Rect::from_min_max(pos2(16.0, 0.0), pos2(800.0, 16.0)));
    assert_eq!(rects.left, Rect::from_min_max(pos2(0.0, 16.0), pos2(16.0, 600.0)));
}

#[test]
fn view_rect_bounds_account_for_scrollbars_and_rulers() {
    let outer = Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 800.0));
    let (full, content) = view_rects(outer, 12.0, 16.0, true, true, 1.0);

    assert_eq!(full, Rect::from_min_max(pos2(0.0, 0.0), pos2(988.0, 788.0)));
    assert_eq!(content, Rect::from_min_max(pos2(16.0, 16.0), pos2(988.0, 788.0)));

    let (full_no_scroll, content_no_scroll) = view_rects(outer, 12.0, 16.0, false, true, 1.0);
    assert_eq!(full_no_scroll, outer);
    assert_eq!(content_no_scroll, Rect::from_min_max(pos2(16.0, 16.0), pos2(1000.0, 800.0)));
}

#[test]
fn ruler_scale_step_and_minor_calculations() {
    // Points (unit_points = 1.0)
    let scale_pts = RulerScale::document(1.0, Pos2::ZERO, false);
    let step_1x = scale_pts.ruler_step(1.0);
    assert!(step_1x >= 50.0);
    let minor_1x = scale_pts.ruler_minor(step_1x);
    assert_eq!(minor_1x, step_1x / 10.0);

    // Inches (halves = true)
    let scale_in = RulerScale::document(72.0, Pos2::ZERO, true);
    let step_in = scale_in.ruler_step(1.0);
    let minor_in = scale_in.ruler_minor(step_in);
    assert_eq!(minor_in, step_in / 8.0);
}

#[test]
fn layout_ticks_generates_bounded_ticks() {
    let scale = RulerScale::document(1.0, Pos2::ZERO, false);
    let from_doc = pos2(0.0, 0.0);
    let to_doc = pos2(200.0, 0.0);
    let axis = Vec2::X;
    let zoom = 1.0;

    let mut ticks = Vec::new();
    layout_ticks(from_doc, to_doc, axis, zoom, &scale, 16.0, |t| {
        ticks.push(t);
    });

    assert!(!ticks.is_empty());
    assert!(ticks.iter().any(|t| t.is_major && t.label.is_some()));
    assert!(ticks.iter().any(|t| !t.is_major));
    assert!(ticks.iter().all(|t| t.distance >= 0.0 && t.distance <= 200.0));
}

#[test]
fn layout_ticks_strictly_respects_segment_bounds() {
    let scale = RulerScale::document(1.0, Pos2::ZERO, false);
    let from_doc = pos2(1.0, 0.0);
    let to_doc = pos2(4.0, 0.0);
    let axis = Vec2::X;
    let zoom = 1.0;

    let mut ticks = Vec::new();
    layout_ticks(from_doc, to_doc, axis, zoom, &scale, 16.0, |t| {
        ticks.push(t);
    });

    // Segment is [1.0, 4.0]: no tick at 0.0 or distance < 0.0
    for t in &ticks {
        assert!(t.distance >= 0.0, "tick distance {} should be >= 0", t.distance);
        assert!(t.value >= 1.0, "tick value {} should be >= 1.0", t.value);
        assert!(t.value <= 4.0, "tick value {} should be <= 4.0", t.value);
    }

    // Segment [0.0, 49.99]: no tick at 50.0
    let mut ticks2 = Vec::new();
    layout_ticks(pos2(0.0, 0.0), pos2(49.99, 0.0), axis, zoom, &scale, 16.0, |t| {
        ticks2.push(t);
    });
    for t in &ticks2 {
        assert!(t.value <= 49.99 + 1e-4, "tick value {} should be <= 49.99", t.value);
    }
}

#[test]
fn segment_clipping_handles_inside_outside_and_crossings() {
    let bounds = Rect::from_min_max(pos2(10.0, 10.0), pos2(100.0, 100.0));

    // Fully inside
    let (c1, c2) = clip_segment(pos2(20.0, 20.0), pos2(50.0, 50.0), bounds).unwrap();
    assert_eq!(c1, pos2(20.0, 20.0));
    assert_eq!(c2, pos2(50.0, 50.0));

    // Fully outside
    assert!(clip_segment(pos2(0.0, 0.0), pos2(5.0, 5.0), bounds).is_none());

    // Crossing
    let (c1, c2) = clip_segment(pos2(0.0, 50.0), pos2(200.0, 50.0), bounds).unwrap();
    assert_eq!(c1.x, 10.0);
    assert_eq!(c2.x, 100.0);
    assert_eq!(c1.y, 50.0);
    assert_eq!(c2.y, 50.0);
}
