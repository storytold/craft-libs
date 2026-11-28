//! Annotation geometry.

use craft_ui::testing::trial;
use kurbo::{Point, Rect};

use crate::annotate::{
    Annotation, AnnotationHandle, AnnotationHit, AnnotationId, AnnotationKind, CALLOUT_KNEE_GAP, HandleRule, ShiftLock, callout_reshaped, handles,
    hit, marker_rect, pick_at,
};

/// The icon side, in page units, the hit tests are given.
const ICON: f64 = 16.0;

fn annotation(id: u64, kind: AnnotationKind, rect: Rect) -> Annotation {
    Annotation {
        id: AnnotationId(id),
        page: 0,
        kind,
        rect,
        points: Vec::new(),
        strokes: Vec::new(),
        color: [255, 0, 0],
        opacity: 1.0,
        stroke_width: 2.0,
        text: String::new(),
        locked: false,
        aspect: None,
        handle_rule: HandleRule::Free,
    }
}

fn square(id: u64, kind: AnnotationKind) -> Annotation {
    annotation(id, kind, Rect::new(10.0, 10.0, 50.0, 30.0))
}

/// The four corners of `r`, as a markup's quad.
fn quad(r: Rect) -> Vec<Point> {
    vec![Point::new(r.x0, r.y0), Point::new(r.x1, r.y0), Point::new(r.x1, r.y1), Point::new(r.x0, r.y1)]
}

/// A highlight over `r` with a comment.
fn commented_highlight(r: Rect) -> Annotation {
    let mut a = annotation(1, AnnotationKind::Highlight, r);
    a.points = quad(r);
    a.text = "Check this".to_owned();
    a
}

/// Whether `r` lies inside `page`, give or take rounding.
fn within(r: Rect, page: Rect) -> bool {
    const SLACK: f64 = 1e-9;
    r.x0 >= page.x0 - SLACK && r.x1 <= page.x1 + SLACK && r.y0 >= page.y0 - SLACK && r.y1 <= page.y1 + SLACK
}

// handles by kind; none for markups and locked ones.

// hit order: selected handles, then last-drawn bodies.
#[trial]
fn hit_prefers_handles_then_the_topmost_body() {
    let (below, above) = (square(1, AnnotationKind::Rectangle), square(2, AnnotationKind::Rectangle));
    let all = [below, above];
    let inside = Point::new(30.0, 20.0);
    assert_eq!(hit(&all, &[], inside, 2.0, ICON), Some(AnnotationHit::Body(AnnotationId(2))));
    let corner = Point::new(11.0, 11.0);
    assert_eq!(hit(&all, &[AnnotationId(1)], corner, 2.0, ICON), Some(AnnotationHit::Handle(AnnotationId(1), AnnotationHandle::Box(-1, -1))));
    assert_eq!(hit(&all, &[], Point::new(200.0, 200.0), 2.0, ICON), None);

    let mut line = square(3, AnnotationKind::Line);
    line.points = vec![Point::new(100.0, 100.0), Point::new(200.0, 100.0)];
    assert_eq!(hit(&[line.clone()], &[], Point::new(150.0, 101.0), 2.0, ICON), Some(AnnotationHit::Body(AnnotationId(3))));
    assert_eq!(hit(&[line], &[], Point::new(150.0, 110.0), 2.0, ICON), None);
}

/// One pick serves the comment part, the hover outline and the marker: a marker before any body, the last listed
/// marker first; then the topmost body, a markup by its quads and a note by its icon and its rect.
#[trial]
fn pick_at_takes_the_marker_before_any_body_and_the_topmost_body_after() {
    // The markup's quads lie away from its rect, so only the quads can pick it.
    let mut markup = annotation(1, AnnotationKind::Highlight, Rect::new(10.0, 60.0, 50.0, 80.0));
    markup.points = quad(Rect::new(200.0, 10.0, 240.0, 30.0));
    let boxed = square(2, AnnotationKind::Rectangle);
    // A note's rect smaller than its icon: the icon's extent picks it.
    let note = annotation(3, AnnotationKind::Note, Rect::new(100.0, 100.0, 104.0, 104.0));
    // A note's rect larger than its icon: the rect beyond the icon picks it too.
    let wide = annotation(4, AnnotationKind::Note, Rect::new(300.0, 300.0, 340.0, 340.0));
    let all = [markup, boxed, note, wide];
    let over_box = Point::new(25.0, 20.0);
    let markers = [(AnnotationId(1), Rect::new(20.0, 15.0, 32.0, 27.0)), (AnnotationId(3), Rect::new(22.0, 17.0, 30.0, 25.0))];

    assert_eq!(pick_at(&all, over_box, 0.0, ICON, &markers[..1]), Some(AnnotationId(1)), "the marker over the rectangle");
    assert_eq!(pick_at(&all, over_box, 0.0, ICON, &markers), Some(AnnotationId(3)), "the last listed marker");
    assert_eq!(pick_at(&all, over_box, 0.0, ICON, &[]), Some(AnnotationId(2)), "the topmost body");
    assert_eq!(pick_at(&all, Point::new(220.0, 20.0), 0.0, ICON, &[]), Some(AnnotationId(1)), "the markup by its quad");
    assert_eq!(pick_at(&all, Point::new(30.0, 70.0), 0.0, ICON, &[]), None, "a markup's rect outside its quads");
    assert_eq!(pick_at(&all, Point::new(112.0, 112.0), 0.0, ICON, &[]), Some(AnnotationId(3)), "the note by its icon");
    assert_eq!(pick_at(&all, Point::new(120.0, 120.0), 0.0, ICON, &[]), None);
    assert_eq!(pick_at(&all, Point::new(335.0, 335.0), 0.0, ICON, &[]), Some(AnnotationId(4)), "the note by its rect beyond its icon");
}

/// The marker is clamped inside the page whatever its bounds: at the page's right edge, with inverted or fractional
/// bounds; a page smaller than the marker and a markup off the page get none, as do plain markups and other kinds.
#[trial]
fn marker_rect_stays_inside_the_page_on_rounded_or_inverted_bounds() {
    let page = Rect::new(0.0, 0.0, 100.0, 200.0);
    let at_edge = commented_highlight(Rect::new(90.0, 10.0, 100.0, 20.0));
    let marker = marker_rect(&at_edge, page, ICON);
    assert!(marker.is_some_and(|m| within(m, page) && m.width() == ICON && m.height() == ICON), "{marker:?}");
    assert_eq!(marker_rect(&at_edge, Rect::new(100.0, 200.0, 0.0, 0.0), ICON), marker, "inverted bounds name the same page");
    let rounded = Rect::new(0.3, 0.7, 99.7, 199.3);
    let marker = marker_rect(&at_edge, rounded, ICON);
    assert!(marker.is_some_and(|m| within(m, rounded)), "{marker:?}");

    let open = commented_highlight(Rect::new(20.0, 50.0, 60.0, 60.0));
    let marker = marker_rect(&open, page, ICON);
    assert!(marker.is_some_and(|m| m.x0 > 60.0 && m.y1 < 50.0), "beside the quad's top right: {marker:?}");

    let tiny = Rect::new(0.0, 0.0, 10.0, 10.0);
    assert_eq!(marker_rect(&commented_highlight(Rect::new(2.0, 2.0, 8.0, 8.0)), tiny, ICON), None, "a page smaller than the marker");
    assert_eq!(marker_rect(&commented_highlight(Rect::new(300.0, 300.0, 320.0, 310.0)), page, ICON), None, "a markup off the page");
    let mut plain = at_edge.clone();
    plain.text = "  ".to_owned();
    assert_eq!(marker_rect(&plain, page, ICON), None, "a markup with no comment");
    let mut boxed = square(2, AnnotationKind::Rectangle);
    boxed.text = "Check this".to_owned();
    assert_eq!(marker_rect(&boxed, page, ICON), None, "a shape");
}

/// A callout's knee keeps to its knee line, level beside the box and upright above or below it, and never comes nearer
/// the box than the gap; its tip moves freely, or held to the lock from the knee.
#[trial]
fn a_callout_knee_stays_on_its_line_at_least_the_gap_from_the_box_and_only_the_tip_moves_freely() {
    let r = Rect::new(100.0, 100.0, 200.0, 150.0);
    let gap = CALLOUT_KNEE_GAP;
    let tip = Point::new(10.0, 10.0);
    // (knee, a drag across its line, where that lands, a drag into the box, where that stops)
    let cases = [
        (Point::new(80.0, 125.0), Point::new(60.0, 140.0), Point::new(60.0, 125.0), Point::new(150.0, 125.0), Point::new(r.x0 - gap, 125.0)),
        (Point::new(220.0, 125.0), Point::new(250.0, 90.0), Point::new(250.0, 125.0), Point::new(120.0, 125.0), Point::new(r.x1 + gap, 125.0)),
        (Point::new(150.0, 80.0), Point::new(130.0, 60.0), Point::new(150.0, 60.0), Point::new(150.0, 120.0), Point::new(150.0, r.y0 - gap)),
        (Point::new(150.0, 170.0), Point::new(170.0, 190.0), Point::new(150.0, 190.0), Point::new(150.0, 110.0), Point::new(150.0, r.y1 + gap)),
    ];
    for (knee, across, lands, into, stops) in cases {
        let points = [tip, knee];
        assert_eq!(callout_reshaped(r, &points, 1, across, None), vec![tip, lands], "knee {knee:?} dragged across its line");
        assert_eq!(callout_reshaped(r, &points, 1, into, None), vec![tip, stops], "knee {knee:?} dragged into the box");
    }

    let (tip, knee) = (Point::new(20.0, 60.0), Point::new(80.0, 125.0));
    let points = [tip, knee];
    let free = Point::new(30.0, 120.0);
    assert_eq!(callout_reshaped(r, &points, 0, free, None), vec![free, knee], "the tip lands on the pointer");
    assert_eq!(callout_reshaped(r, &points, 0, free, Some(ShiftLock::LevelOrPlumb)), vec![Point::new(30.0, 125.0), knee], "held level from the knee");
    let held = callout_reshaped(r, &points, 0, Point::new(30.0, 80.0), Some(ShiftLock::Angle45));
    let diagonal = |p: Point| {
        let d = p - knee;
        d.x < 0.0 && d.y < 0.0 && (d.x.abs() - d.y.abs()).abs() < 1e-9
    };
    assert!(matches!(held.as_slice(), [t, k] if *k == knee && diagonal(*t)), "held to 45 degrees from the knee: {held:?}");
}

/// The document-space knee gap corresponds to twelve page points at the standard page scale.
#[trial]
fn a_callout_knee_keeps_the_page_point_gap_in_document_space() {
    let rect = Rect::new(200.0, 100.0, 300.0, 150.0);
    let points = [Point::new(20.0, 125.0), Point::new(160.0, 125.0)];
    let moved = callout_reshaped(rect, &points, 1, Point::new(210.0, 125.0), None);
    assert!((rect.x0 - moved[1].x - 12.0 * 96.0 / 72.0).abs() < 0.001);
}

/// The overlay paints an annotation as the in-flight edit leaves it: moved with its points and strokes, resized,
/// reshaped with its box around the new points, and a resized callout's box going while its tip and knee stay.
#[cfg(feature = "ui")]
#[trial]
fn an_edited_annotation_takes_a_live_move_reshape_and_callout_resize() {
    use crate::annotate::AnnotationEdit;
    use kurbo::Vec2;

    let delta = Vec2::new(5.0, 10.0);
    let boxed = square(1, AnnotationKind::Rectangle);
    let moved = boxed.edited(&AnnotationEdit::Move { ids: vec![AnnotationId(1)], delta });
    assert_eq!(moved.map(|a| a.rect), Some(Rect::new(15.0, 20.0, 55.0, 40.0)));
    assert_eq!(boxed.edited(&AnnotationEdit::Move { ids: vec![AnnotationId(9)], delta }), None, "another annotation's move");
    let to = Rect::new(0.0, 0.0, 80.0, 60.0);
    assert_eq!(boxed.edited(&AnnotationEdit::Resize { id: AnnotationId(1), rect: to }).map(|a| a.rect), Some(to));
    assert_eq!(boxed.edited(&AnnotationEdit::Resize { id: AnnotationId(9), rect: to }), None, "another annotation's resize");
    assert_eq!(boxed.edited(&AnnotationEdit::Delete(vec![AnnotationId(1)])), None, "not a live edit");

    let mut line = annotation(2, AnnotationKind::Line, Rect::new(10.0, 10.0, 50.0, 10.0));
    line.points = vec![Point::new(10.0, 10.0), Point::new(50.0, 10.0)];
    let moved = line.edited(&AnnotationEdit::Move { ids: vec![AnnotationId(1), AnnotationId(2)], delta });
    assert_eq!(moved.map(|a| a.points), Some(vec![Point::new(15.0, 20.0), Point::new(55.0, 20.0)]));
    let points = vec![Point::new(10.0, 10.0), Point::new(60.0, 40.0)];
    let reshaped = line.edited(&AnnotationEdit::Reshape { id: AnnotationId(2), points: points.clone() });
    assert_eq!(reshaped.map(|a| (a.points, a.rect)), Some((points, Rect::new(10.0, 10.0, 60.0, 40.0))));

    let mut ink = annotation(3, AnnotationKind::Ink, Rect::new(0.0, 0.0, 10.0, 10.0));
    ink.strokes = vec![vec![Point::new(0.0, 0.0), Point::new(10.0, 10.0)]];
    let moved = ink.edited(&AnnotationEdit::Move { ids: vec![AnnotationId(3)], delta });
    assert_eq!(moved.map(|a| a.strokes), Some(vec![vec![Point::new(5.0, 10.0), Point::new(15.0, 20.0)]]));

    let mut callout = annotation(4, AnnotationKind::Callout, Rect::new(100.0, 100.0, 200.0, 150.0));
    callout.points = vec![Point::new(20.0, 60.0), Point::new(80.0, 125.0)];
    let to = Rect::new(110.0, 90.0, 230.0, 160.0);
    let resized = callout.edited(&AnnotationEdit::Resize { id: AnnotationId(4), rect: to });
    assert_eq!(resized.map(|a| (a.rect, a.points)), Some((to, callout.points.clone())));
}

/// Under `FixedPoints` and `NoResize` a selected annotation offers no handle, pointwise or boxed, and its body still
/// selects; `Free` keeps the kind's handles, a callout's tip and knee among them.
#[trial]
fn a_fixed_points_or_no_resize_annotation_shows_no_handle_and_still_selects() {
    let mut line = square(1, AnnotationKind::Line);
    line.points = vec![Point::new(100.0, 100.0), Point::new(200.0, 100.0)];
    let boxed = square(2, AnnotationKind::Rectangle);
    assert_eq!(handles(&line).len(), 2);
    assert_eq!(handles(&boxed).len(), 8);
    let selected = [AnnotationId(1), AnnotationId(2)];
    for rule in [HandleRule::FixedPoints, HandleRule::NoResize] {
        let (mut line, mut boxed) = (line.clone(), boxed.clone());
        line.handle_rule = rule;
        boxed.handle_rule = rule;
        assert!(handles(&line).is_empty(), "{rule:?} line: {:?}", handles(&line));
        assert!(handles(&boxed).is_empty(), "{rule:?} box: {:?}", handles(&boxed));
        let all = [line, boxed];
        assert_eq!(hit(&all, &selected, Point::new(100.0, 100.0), 2.0, ICON), Some(AnnotationHit::Body(AnnotationId(1))), "{rule:?} line end");
        assert_eq!(hit(&all, &selected, Point::new(11.0, 11.0), 2.0, ICON), Some(AnnotationHit::Body(AnnotationId(2))), "{rule:?} box corner");
    }

    let mut callout = annotation(3, AnnotationKind::Callout, Rect::new(100.0, 100.0, 200.0, 150.0));
    let (tip, knee) = (Point::new(20.0, 60.0), Point::new(80.0, 125.0));
    callout.points = vec![tip, knee];
    let shown = handles(&callout);
    assert_eq!(shown.len(), 10, "{shown:?}");
    assert!(shown.contains(&(AnnotationHandle::Point(0), tip)) && shown.contains(&(AnnotationHandle::Point(1), knee)), "{shown:?}");
}

// reshaped with shift snaps the segment.

// AnnotationGroup::tools order and Eraser in Draw.
