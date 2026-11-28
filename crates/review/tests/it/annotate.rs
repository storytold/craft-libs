//! The annotation tool on a canvas: the Eraser, drawing, placing, the slips and the selection rules.

use craft_behavior::{CancelReason, CanvasView, EditEvent};
use craft_keymap::Mods;
use craft_review::annotate::{Annotation, AnnotationEdit, AnnotationId, AnnotationKind, AnnotationTool, HandleRule, SLIP_MOVE, ShiftLock};
use craft_review::ui::annotate::{
    DrawOptions, EmptyPress, EraseMode, HandlesWhileDrawing, InkStrokes, MarkupPress, OpenOn, SelectOptions, ToggleMods,
};
use craft_review::ui::{AnnotateTool, AnnotateWords};
use craft_ui::testing::{ToolFixture, pointer, trial};
use egui::epaint::ColorMode;
use egui::{Color32, Event, Key, Modifiers, PointerButton, Pos2, Rect as ScreenRect, Shape, vec2};
use kurbo::{Point, Rect, Vec2};

/// Frames past the double-click time (0.3 s at 1/60 s a frame), so the next press is a click of its own.
const PAST_DOUBLE_CLICK: usize = 20;

fn view(room: ScreenRect) -> CanvasView {
    CanvasView::new(room, Point::new(f64::from(room.width()) / 2.0, f64::from(room.height()) / 2.0), 1.0)
}

/// `tool` on a canvas whose page (0, 0) is the canvas's top left corner at zoom 1.
fn rig(tool: AnnotateTool) -> ToolFixture<AnnotateTool> {
    ToolFixture::with_view("Document", tool, view, |_, _| {})
}

/// The screen position of page point `at`.
fn on_screen(rig: &ToolFixture<AnnotateTool>, at: Point) -> Pos2 {
    rig.canvas().rect.min + vec2(at.x as f32, at.y as f32)
}

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
        stroke_width: 1.0,
        text: String::new(),
        locked: false,
        aspect: None,
        handle_rule: HandleRule::default(),
    }
}

fn ink(id: u64, xs: &[f64]) -> Annotation {
    let mut ink = annotation(id, AnnotationKind::Ink, Rect::new(0.0, 0.0, 300.0, 200.0));
    ink.strokes = xs.iter().map(|&x| vec![Point::new(x, 0.0), Point::new(x, 100.0)]).collect();
    ink
}

/// The edits' values, in order.
fn values(rig: &ToolFixture<AnnotateTool>) -> Vec<AnnotationEdit> {
    rig.tool_mut().take_edits().iter().filter_map(|edit| edit.value().cloned()).collect()
}

/// The annotations the edits create, in order.
fn created(rig: &ToolFixture<AnnotateTool>) -> Vec<Annotation> {
    values(rig)
        .into_iter()
        .filter_map(|edit| match edit {
            AnnotationEdit::Create(annotation) => Some(annotation),
            _ => None,
        })
        .collect()
}

/// Whether `a` is within a hundredth of a point of `b`: screen positions pass through `f32`.
fn near(a: Point, b: Point) -> bool {
    (a - b).hypot() < 0.01
}

#[trial]
fn a_callout_box_and_leader_drag_report_distinct_edits() {
    let mut callout = annotation(1, AnnotationKind::Callout, Rect::new(150.0, 100.0, 250.0, 140.0));
    callout.points = vec![Point::new(60.0, 60.0), Point::new(120.0, 120.0)];
    let make = || {
        let mut tool = AnnotateTool::new(AnnotationTool::Select, 0);
        tool.annotations = vec![callout.clone()];
        tool.selected = vec![callout.id];
        rig(tool)
    };
    let mut box_rig = make();
    drag(&mut box_rig, Point::new(200.0, 120.0), Point::new(220.0, 130.0));
    assert!(
        values(&box_rig)
            .iter()
            .any(|edit| matches!(edit, AnnotationEdit::Resize { id, rect } if *id == callout.id && *rect == Rect::new(170.0, 110.0, 270.0, 150.0))),
        "a box drag moves the box without its arrow"
    );
    let mut leader_rig = make();
    drag(&mut leader_rig, Point::new(90.0, 90.0), Point::new(100.0, 90.0));
    assert!(
        values(&leader_rig)
            .iter()
            .any(|edit| matches!(edit, AnnotationEdit::Move { ids, delta } if ids == &[callout.id] && *delta == Vec2::new(10.0, 0.0))),
        "a leader drag moves the whole callout"
    );
}

fn button(at: Pos2, pressed: bool, modifiers: Modifiers) -> Event {
    Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers }
}

/// Drags the tool from page point `from` to page point `to`.
fn drag(rig: &mut ToolFixture<AnnotateTool>, from: Point, to: Point) {
    let (start, end) = (on_screen(rig, from), on_screen(rig, to));
    rig.drag_from(start, end - start);
}

/// Drags from page point `from` through `via` to `to` with `modifiers` held, then waits past the double-click time.
fn drag_through(rig: &mut ToolFixture<AnnotateTool>, from: Point, via: Point, to: Point, modifiers: Modifiers) {
    let (from, via, to) = (on_screen(rig, from), on_screen(rig, via), on_screen(rig, to));
    let fixture = rig.fixture();
    fixture.hold(modifiers);
    fixture.frame(vec![pointer::moved(from)]);
    fixture.frame(vec![button(from, true, modifiers)]);
    for step in 1..=4 {
        fixture.frame(vec![pointer::moved(from + (via - from) * (step as f32 / 4.0))]);
    }
    fixture.frame(vec![pointer::moved(to)]);
    fixture.frame(vec![button(to, false, modifiers)]);
    fixture.hold(Modifiers::NONE);
    fixture.idle(PAST_DOUBLE_CLICK);
}

/// Drags from page point `from` to `to` with `modifiers` held.
fn drag_holding(rig: &mut ToolFixture<AnnotateTool>, from: Point, to: Point, modifiers: Modifiers) {
    drag_through(rig, from, to, to, modifiers);
}

/// Presses and releases at page point `at`, then waits past the double-click time.
fn click(rig: &mut ToolFixture<AnnotateTool>, at: Point) {
    let at = on_screen(rig, at);
    let fixture = rig.fixture();
    fixture.frame(vec![pointer::moved(at)]);
    fixture.frame(vec![pointer::press(at)]);
    fixture.frame(vec![pointer::release(at)]);
    fixture.idle(PAST_DOUBLE_CLICK);
}

/// The Eraser reports each annotation once, even when its strokes are crossed with another annotation's in
/// between: an app removing strokes by index would otherwise delete the wrong ones.
#[trial]
fn eraser_reports_one_erase_per_annotation_crossing_a_b_a() {
    let mut tool = AnnotateTool::new(AnnotationTool::Eraser, 0);
    tool.annotations = vec![ink(1, &[50.0, 250.0]), ink(2, &[150.0])];
    let mut rig = rig(tool);
    drag(&mut rig, Point::new(10.0, 50.0), Point::new(280.0, 50.0));
    assert_eq!(
        values(&rig),
        vec![AnnotationEdit::Erase { id: AnnotationId(1), strokes: vec![0, 1] }, AnnotationEdit::Erase { id: AnnotationId(2), strokes: vec![0] }]
    );
}

/// One annotation per stroke: each release reports its own `Create` of the pen's kind, and a click with no drag is a
/// one-point dot, which the smallest-shape rule does not drop. Gathered strokes (the default) report none until the
/// drawing ends.
#[trial]
fn per_stroke_ink_reports_a_create_per_release_and_a_dot_on_a_click() {
    for kind in [AnnotationKind::Ink, AnnotationKind::FreehandHighlight] {
        let options = DrawOptions { ink: InkStrokes::PerStroke, ..DrawOptions::default() };
        let mut rig = rig(AnnotateTool::new(AnnotationTool::Draw(kind), 0).draw_options(options));
        drag(&mut rig, Point::new(20.0, 20.0), Point::new(120.0, 60.0));
        drag(&mut rig, Point::new(20.0, 100.0), Point::new(120.0, 140.0));
        click(&mut rig, Point::new(200.0, 200.0));
        let made = created(&rig);
        assert_eq!(made.len(), 3, "{kind:?}: one annotation per release, the click's included: {made:?}");
        assert!(made.iter().all(|a| a.kind == kind && a.strokes.len() == 1), "{kind:?}: each holds its one stroke: {made:?}");
        let dot = &made[2].strokes[0];
        assert!(dot.len() == 1 && near(dot[0], Point::new(200.0, 200.0)), "{kind:?}: a click is a dot where it was clicked: {dot:?}");

        let mut gathered = rig_for(kind);
        drag(&mut gathered, Point::new(20.0, 20.0), Point::new(120.0, 60.0));
        drag(&mut gathered, Point::new(20.0, 100.0), Point::new(120.0, 140.0));
        assert!(created(&gathered).is_empty(), "{kind:?}: gathered strokes wait for the drawing to end");
    }
}

fn rig_for(kind: AnnotationKind) -> ToolFixture<AnnotateTool> {
    rig(AnnotateTool::new(AnnotationTool::Draw(kind), 0))
}

/// A polygon closes on a click at its first point once it has three, without adding that click; a click within two
/// points of the last adds none; Backspace takes back the last point. A polyline never closes, and taking back its
/// last point ends the drawing with nothing made.
#[trial]
fn a_polygon_closes_on_its_first_point_and_backspace_takes_back_the_last() {
    let [a, b, c, d] = [Point::new(50.0, 50.0), Point::new(150.0, 50.0), Point::new(150.0, 150.0), Point::new(50.0, 150.0)];
    let mut rig = rig_for(AnnotationKind::Polygon);
    click(&mut rig, a);
    click(&mut rig, b);
    click(&mut rig, Point::new(b.x + 1.0, b.y));
    assert_eq!(rig.tool().drawing().len(), 2, "a click within two points of the last adds none");
    click(&mut rig, c);
    click(&mut rig, Point::new(170.0, 170.0));
    rig.fixture().key(Key::Backspace, Modifiers::NONE);
    let drawn = rig.tool().drawing().to_vec();
    assert!(drawn.len() == 3 && near(drawn[2], c), "Backspace takes back the last point: {drawn:?}");
    click(&mut rig, d);
    assert!(created(&rig).is_empty(), "nothing is made while the polygon is open");
    click(&mut rig, Point::new(a.x + 1.0, a.y + 1.0));
    let made = created(&rig);
    assert_eq!(made.len(), 1, "a click on the first point closes the polygon: {made:?}");
    assert_eq!(made[0].kind, AnnotationKind::Polygon);
    assert!(made[0].points.iter().zip([a, b, c, d]).all(|(got, want)| near(*got, want)) && made[0].points.len() == 4, "{:?}", made[0].points);
    assert!(rig.tool().drawing().is_empty(), "closing ends the drawing");

    let mut line = rig_for(AnnotationKind::PolyLine);
    for at in [a, b, c, a] {
        click(&mut line, at);
    }
    assert_eq!(line.tool().drawing().len(), 4, "a polyline takes its first point again as a point");
    for _ in 0..4 {
        line.fixture().key(Key::Backspace, Modifiers::NONE);
    }
    assert!(line.tool().drawing().is_empty(), "taking back every point ends the drawing");
    assert!(created(&line).is_empty(), "with nothing made");
}

/// The placements of the place kinds, by the tool's input: Note, Attach and Caret report `Place` at a click (a caret
/// at its snapped tip, with its side); a text box drag under 6 screen points on a side is a click; a callout drag
/// under 8 page units places nothing, and a longer one has its tip at the press and its box at the release.
#[trial]
fn placing_kinds_report_a_place_and_a_short_text_box_drag_is_a_click() {
    let placed = |rig: &ToolFixture<AnnotateTool>| -> Vec<(AnnotationKind, Point, Option<Vec2>, Option<Point>)> {
        let edits = values(rig);
        assert!(!edits.iter().any(|edit| matches!(edit, AnnotationEdit::Create(_))), "a placement creates nothing itself: {edits:?}");
        edits
            .into_iter()
            .filter_map(|edit| match edit {
                AnnotationEdit::Place(place) => Some((place.kind, place.at, place.size, place.from)),
                _ => None,
            })
            .collect()
    };
    let at = Point::new(100.0, 100.0);
    for kind in [AnnotationKind::Note, AnnotationKind::Attach, AnnotationKind::Caret] {
        let mut rig = rig_for(kind);
        click(&mut rig, at);
        let got = placed(&rig);
        assert!(matches!(got[..], [(k, p, None, None)] if k == kind && near(p, at)), "{kind:?} places at the click: {got:?}");
    }

    let snapped = AnnotateTool::new(AnnotationTool::Draw(AnnotationKind::Caret), 0).caret_snap(|p| Some((Point::new(p.x, 80.0), 12.0)));
    let mut caret = rig(snapped);
    click(&mut caret, at);
    let got = placed(&caret);
    assert!(
        matches!(got[..], [(_, p, Some(size), None)] if near(p, Point::new(100.0, 80.0)) && size == Vec2::new(12.0, 12.0)),
        "a snapped caret takes the snap's tip and side: {got:?}"
    );

    let mut text_box = rig_for(AnnotationKind::TextBox);
    drag(&mut text_box, Point::new(50.0, 50.0), Point::new(150.0, 120.0));
    let got = placed(&text_box);
    assert!(
        matches!(got[..], [(_, p, Some(size), None)] if near(p, Point::new(50.0, 50.0)) && (size - Vec2::new(100.0, 70.0)).hypot() < 0.01),
        "a text box drag places a box of its size: {got:?}"
    );
    drag(&mut text_box, Point::new(50.0, 50.0), Point::new(54.0, 90.0));
    let got = placed(&text_box);
    assert!(matches!(got[..], [(_, p, None, None)] if near(p, Point::new(50.0, 50.0))), "a drag 4 points wide is a click: {got:?}");

    let mut callout = rig_for(AnnotationKind::Callout);
    drag(&mut callout, Point::new(50.0, 50.0), Point::new(55.0, 53.0));
    assert!(placed(&callout).is_empty(), "a leader under 8 units places nothing");
    drag(&mut callout, Point::new(50.0, 50.0), Point::new(150.0, 100.0));
    let got = placed(&callout);
    assert!(
        matches!(got[..], [(_, p, None, Some(tip))] if near(p, Point::new(150.0, 100.0)) && near(tip, Point::new(50.0, 50.0))),
        "the tip is at the press and the box at the release: {got:?}"
    );
}

/// The path eraser reports one `EraseAlong` on release with its path and its radius in page units, over the unlocked
/// Ink and freehand highlights whose box its path reaches, and no stroke-by-stroke `Erase`.
#[trial]
fn the_path_eraser_reports_its_path_and_reach() {
    let options = DrawOptions { erase: EraseMode::Path, eraser_radius: 6.0, ..DrawOptions::default() };
    let mut tool = AnnotateTool::new(AnnotationTool::Eraser, 0).draw_options(options);
    let boxed = |id: u64, kind: AnnotationKind, rect: Rect| {
        let mut a = annotation(id, kind, rect);
        a.strokes = vec![vec![Point::new(rect.center().x, rect.y0), Point::new(rect.center().x, rect.y1)]];
        a
    };
    let mut locked = boxed(3, AnnotationKind::Ink, Rect::new(95.0, 0.0, 105.0, 100.0));
    locked.locked = true;
    tool.annotations = vec![
        boxed(1, AnnotationKind::Ink, Rect::new(45.0, 0.0, 55.0, 100.0)),
        boxed(2, AnnotationKind::FreehandHighlight, Rect::new(145.0, 0.0, 155.0, 100.0)),
        locked,
        // Under the path only within its radius: the box's top is 4 units below it.
        boxed(4, AnnotationKind::Ink, Rect::new(245.0, 54.0, 255.0, 150.0)),
        // Out of reach.
        boxed(5, AnnotationKind::Ink, Rect::new(45.0, 300.0, 55.0, 400.0)),
        annotation(6, AnnotationKind::Rectangle, Rect::new(195.0, 0.0, 205.0, 100.0)),
    ];
    let mut rig = rig(tool);
    drag(&mut rig, Point::new(40.0, 50.0), Point::new(260.0, 50.0));
    let edits = values(&rig);
    assert!(!edits.iter().any(|edit| matches!(edit, AnnotationEdit::Erase { .. })), "the path eraser erases no whole stroke: {edits:?}");
    let along: Vec<_> = edits
        .into_iter()
        .filter_map(|edit| match edit {
            AnnotationEdit::EraseAlong { ids, path, radius } => Some((ids, path, radius)),
            _ => None,
        })
        .collect();
    assert_eq!(along.len(), 1, "one report on release: {along:?}");
    let (ids, path, radius) = &along[0];
    assert_eq!(ids, &vec![AnnotationId(1), AnnotationId(2), AnnotationId(4)], "the unlocked pens the path reaches");
    assert!((radius - 6.0).abs() < 1e-9, "6 screen points at zoom 1 are 6 page units: {radius}");
    assert!(
        path.len() >= 2 && near(path[0], Point::new(40.0, 50.0)) && near(path[path.len() - 1], Point::new(260.0, 50.0)),
        "the path runs from the press to the release: {path:?}"
    );
}

/// A move within `SLIP_MOVE` and a resize within `SLIP_RESIZE` that went out and came back end as a cancel
/// (`Unchanged`) rather than a commit, and a drawn shape under `MIN_SHAPE` makes nothing; past them, each commits.
#[trial]
fn a_move_a_resize_and_a_shape_under_their_slips_commit_nothing() {
    let ended = |rig: &ToolFixture<AnnotateTool>| -> Vec<EditEvent<AnnotationEdit>> {
        rig.tool_mut().take_edits().into_iter().filter(|edit| matches!(edit, EditEvent::Commit { .. } | EditEvent::Cancel { .. })).collect()
    };
    let unchanged = |events: &[EditEvent<AnnotationEdit>]| matches!(events, [EditEvent::Cancel { reason: CancelReason::Unchanged, .. }]);
    let mut tool = AnnotateTool::new(AnnotationTool::Select, 0);
    tool.annotations = vec![annotation(1, AnnotationKind::Rectangle, Rect::new(100.0, 100.0, 200.0, 160.0))];
    let mut rig = rig(tool);

    let grab = Point::new(150.0, 130.0);
    drag_through(&mut rig, grab, Point::new(180.0, 130.0), Point::new(151.0, 130.0), Modifiers::NONE);
    let events = ended(&rig);
    assert!(unchanged(&events), "a move that came back within its slip is cancelled: {events:?}");
    drag_holding(&mut rig, grab, Point::new(160.0, 130.0), Modifiers::NONE);
    let events = ended(&rig);
    assert!(
        matches!(&events[..], [EditEvent::Commit { after: AnnotationEdit::Move { delta, .. }, .. }] if delta.x > SLIP_MOVE),
        "a move past its slip commits: {events:?}"
    );

    let corner = Point::new(200.0, 160.0);
    drag_through(&mut rig, corner, Point::new(230.0, 190.0), Point::new(201.0, 161.0), Modifiers::NONE);
    let events = ended(&rig);
    assert!(unchanged(&events), "a resize that came back within its slip is cancelled: {events:?}");
    drag_holding(&mut rig, corner, Point::new(220.0, 180.0), Modifiers::NONE);
    let events = ended(&rig);
    assert!(matches!(&events[..], [EditEvent::Commit { after: AnnotationEdit::Resize { .. }, .. }]), "a resize past its slip commits: {events:?}");

    let mut shapes = rig_for(AnnotationKind::Rectangle);
    drag(&mut shapes, Point::new(300.0, 300.0), Point::new(301.5, 340.0));
    assert!(created(&shapes).is_empty(), "a box 1.5 units wide is under the smallest shape");
    drag(&mut shapes, Point::new(300.0, 300.0), Point::new(340.0, 340.0));
    assert_eq!(created(&shapes).len(), 1, "a box past the smallest shape is made");
}

/// With `EmptyPress::PassThrough` and `MarkupPress::ClickOnly`, a press on empty page and on a highlight is the
/// host's: the tool begins no gesture and reports nothing, so the drag reaches the host's text selection. The
/// default options take both presses.
#[trial]
fn a_pass_through_tool_ignores_empty_and_markup_presses() {
    let mut highlight = annotation(1, AnnotationKind::Highlight, Rect::new(100.0, 100.0, 200.0, 120.0));
    highlight.points = vec![Point::new(100.0, 100.0), Point::new(200.0, 100.0), Point::new(200.0, 120.0), Point::new(100.0, 120.0)];
    let options = SelectOptions { empty_press: EmptyPress::PassThrough, markup_press: MarkupPress::ClickOnly, ..SelectOptions::default() };
    for (passes, tool) in
        [(true, AnnotateTool::new(AnnotationTool::Select, 0).select_options(options)), (false, AnnotateTool::new(AnnotationTool::Select, 0))]
    {
        let mut tool = tool;
        tool.annotations = vec![highlight.clone()];
        let mut rig = rig(tool);
        for (what, from, to) in
            [("empty page", Point::new(300.0, 300.0), Point::new(360.0, 340.0)), ("a highlight", Point::new(150.0, 110.0), Point::new(250.0, 160.0))]
        {
            rig.forget();
            drag(&mut rig, from, to);
            let (began, edits) = (!rig.gestures().is_empty(), values(&rig));
            if passes {
                assert!(!began && edits.is_empty(), "a press on {what} passes through: {:?} {edits:?}", rig.gestures());
            } else {
                assert!(began && !edits.is_empty(), "by default the tool takes a press on {what}: {edits:?}");
            }
        }
    }
}

/// A corner resize of an annotation with its own aspect keeps it on all four corners; Shift keeps the box's own
/// ratio; a flat box resizes freely with Shift held.
#[trial]
fn a_fixed_aspect_holds_on_every_corner_and_shift_takes_the_boxs_own() {
    let mut square = annotation(1, AnnotationKind::Rectangle, Rect::new(100.0, 100.0, 200.0, 150.0));
    square.aspect = Some(1.0);
    let wide = annotation(2, AnnotationKind::Rectangle, Rect::new(300.0, 100.0, 400.0, 150.0));
    let flat = annotation(3, AnnotationKind::Rectangle, Rect::new(300.0, 300.0, 400.0, 300.0));
    let mut tool = AnnotateTool::new(AnnotationTool::Select, 0);
    tool.annotations = vec![square.clone(), wide.clone(), flat.clone()];
    let mut rig = rig(tool);
    let resize = |rig: &mut ToolFixture<AnnotateTool>, a: &Annotation, (hx, hy): (f64, f64), by: Vec2, modifiers: Modifiers| -> Rect {
        rig.tool_mut().selected = vec![a.id];
        let corner = Point::new(if hx < 0.0 { a.rect.x0 } else { a.rect.x1 }, if hy < 0.0 { a.rect.y0 } else { a.rect.y1 });
        drag_holding(rig, corner, corner + by, modifiers);
        let committed = rig.tool_mut().take_edits().into_iter().find_map(|edit| match edit {
            EditEvent::Commit { after: AnnotationEdit::Resize { rect, .. }, .. } => Some(rect),
            _ => None,
        });
        committed.unwrap_or_else(|| panic!("the corner ({hx}, {hy}) of {:?} resizes", a.id))
    };
    for (hx, hy) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
        let rect = resize(&mut rig, &square, (hx, hy), Vec2::new(hx * 40.0, hy * 10.0), Modifiers::NONE);
        assert!((rect.width() / rect.height() - 1.0).abs() < 1e-6, "the corner ({hx}, {hy}) keeps the annotation's aspect: {rect:?}");
    }
    let held = resize(&mut rig, &wide, (1.0, 1.0), Vec2::new(40.0, 5.0), Modifiers::SHIFT);
    assert!((held.width() / held.height() - 2.0).abs() < 1e-6, "Shift keeps the box's own 2:1: {held:?}");
    let free = resize(&mut rig, &wide, (1.0, 1.0), Vec2::new(40.0, 5.0), Modifiers::NONE);
    assert!((free.width() / free.height() - 2.0).abs() > 0.1, "without Shift the corner is free: {free:?}");
    let flat_rect = resize(&mut rig, &flat, (1.0, 1.0), Vec2::new(20.0, 30.0), Modifiers::SHIFT);
    assert!(flat_rect.width() > flat.rect.width() && flat_rect.height() > 0.0, "a flat box resizes freely: {flat_rect:?}");

    rig.tool_mut().selected = vec![wide.id, square.id];
    drag_holding(&mut rig, Point::new(200.0, 150.0), Point::new(220.0, 155.0), Modifiers::NONE);
    let resized_ids: Vec<AnnotationId> = rig
        .tool_mut()
        .take_edits()
        .into_iter()
        .filter_map(|edit| match edit {
            EditEvent::Commit { after: AnnotationEdit::Resize { id, .. }, .. } => Some(id),
            _ => None,
        })
        .collect();
    assert_eq!(resized_ids, vec![square.id], "the handles are the last selected annotation's");
}

/// Under `HandlesWhileDrawing::Edit` a drawing tool's press on the selected box's corner resizes it, but a press on a
/// selected line's end draws a new line from there: the drawing tools do not take end and corner points.
#[trial]
fn while_drawing_edit_takes_resize_handles_but_not_point_handles() {
    let options = SelectOptions { handles_while_drawing: HandlesWhileDrawing::Edit, ..SelectOptions::default() };
    let commits = |rig: &ToolFixture<AnnotateTool>| -> Vec<AnnotationEdit> {
        rig.tool_mut()
            .take_edits()
            .into_iter()
            .filter_map(|edit| match edit {
                EditEvent::Commit { after, .. } => Some(after),
                EditEvent::Set { after, .. } => Some(after),
                _ => None,
            })
            .collect()
    };

    let boxed = annotation(1, AnnotationKind::Rectangle, Rect::new(100.0, 100.0, 200.0, 160.0));
    let mut tool = AnnotateTool::new(AnnotationTool::Draw(AnnotationKind::Rectangle), 0).select_options(options);
    tool.annotations = vec![boxed.clone()];
    tool.selected = vec![boxed.id];
    let mut shapes = rig(tool);
    drag_holding(&mut shapes, Point::new(200.0, 160.0), Point::new(240.0, 200.0), Modifiers::NONE);
    let edits = commits(&shapes);
    assert!(edits.iter().any(|edit| matches!(edit, AnnotationEdit::Resize { .. })), "the corner resizes the box: {edits:?}");

    let mut line = annotation(2, AnnotationKind::Line, Rect::new(100.0, 100.0, 200.0, 100.0));
    line.points = vec![Point::new(100.0, 100.0), Point::new(200.0, 100.0)];
    let mut tool = AnnotateTool::new(AnnotationTool::Draw(AnnotationKind::Line), 0).select_options(options);
    tool.annotations = vec![line.clone()];
    tool.selected = vec![line.id];
    let mut lines = rig(tool);
    drag_holding(&mut lines, Point::new(200.0, 100.0), Point::new(260.0, 140.0), Modifiers::NONE);
    let edits = commits(&lines);
    assert!(!edits.iter().any(|edit| matches!(edit, AnnotationEdit::Reshape { .. })), "the line's end is not dragged: {edits:?}");
    assert!(
        edits.iter().any(|edit| matches!(edit, AnnotationEdit::Create(made) if made.kind == AnnotationKind::Line)),
        "a new line is drawn from the end: {edits:?}"
    );
}

/// `AnnotateTool::click`, the host's part click: a plain click selects only the comment (on one already selected it
/// keeps the selection and makes that one the primary, whose handles show), the toggle modifiers toggle it (Command only under `ToggleMods::ShiftOrCommand`), a double click opens a note or text box (any comment under
/// `OpenOn::Every`), and the edits carry the tool's undo labels. A double press on the page runs the same rule.
#[trial]
fn a_click_on_a_hosts_part_selects_toggles_and_opens_by_the_tools_rules() {
    let words = AnnotateWords::default();
    let comments = vec![
        annotation(1, AnnotationKind::Rectangle, Rect::new(100.0, 100.0, 200.0, 150.0)),
        annotation(2, AnnotationKind::Note, Rect::new(300.0, 100.0, 316.0, 116.0)),
    ];
    let mut tool = AnnotateTool::new(AnnotationTool::Select, 0);
    tool.annotations = comments.clone();
    tool.selected = vec![AnnotationId(1)];
    let labelled = |tool: &mut AnnotateTool| -> Vec<(AnnotationEdit, String)> {
        tool.take_edits()
            .into_iter()
            .filter_map(|edit| match edit {
                EditEvent::Set { after, label, .. } => Some((after, label.to_string())),
                _ => None,
            })
            .collect()
    };

    tool.click(AnnotationId(2), Mods::NONE, 1, &words);
    assert_eq!(tool.selected, vec![AnnotationId(2)], "a plain click selects only the comment");
    tool.click(AnnotationId(1), Mods::SHIFT, 1, &words);
    assert_eq!(tool.selected, vec![AnnotationId(2), AnnotationId(1)], "Shift adds it");
    tool.click(AnnotationId(1), Mods::CMD, 1, &words);
    assert_eq!(tool.selected, vec![AnnotationId(2), AnnotationId(1)], "Command does not toggle by default");
    let edits = labelled(&mut tool);
    assert!(
        edits.len() == 2 && edits.iter().all(|(edit, label)| matches!(edit, AnnotationEdit::Selection(_)) && label == words.edit_select.as_str()),
        "each change is a selection step under its label: {edits:?}"
    );
    tool.click(AnnotationId(2), Mods::NONE, 1, &words);
    assert_eq!(
        tool.selected,
        vec![AnnotationId(1), AnnotationId(2)],
        "a plain click on a selected comment keeps the others and makes it the primary"
    );

    let mut options = SelectOptions { toggle: ToggleMods::ShiftOrCommand, ..SelectOptions::default() };
    let mut tool = tool.select_options(options);
    tool.click(AnnotationId(1), Mods::CMD, 1, &words);
    assert_eq!(tool.selected, vec![AnnotationId(2)], "Command toggles under ShiftOrCommand");
    tool.click(AnnotationId(2), Mods::NONE, 2, &words);
    tool.click(AnnotationId(1), Mods::NONE, 2, &words);
    let edits = labelled(&mut tool);
    assert!(
        edits.iter().any(|(edit, label)| *edit == AnnotationEdit::Open(AnnotationId(2)) && label == words.edit_open.as_str()),
        "a double click opens a note: {edits:?}"
    );
    assert!(!edits.iter().any(|(edit, _)| *edit == AnnotationEdit::Open(AnnotationId(1))), "but not a rectangle: {edits:?}");
    assert_eq!(tool.selected, vec![AnnotationId(1)], "the rectangle's double click selects it");

    options.open = OpenOn::Every;
    let mut tool = tool.select_options(options);
    tool.click(AnnotationId(1), Mods::NONE, 2, &words);
    assert!(labelled(&mut tool).iter().any(|(edit, _)| *edit == AnnotationEdit::Open(AnnotationId(1))), "under Every, any comment opens");

    let mut page = AnnotateTool::new(AnnotationTool::Select, 0);
    page.annotations = comments;
    let mut rig = rig(page);
    let note = on_screen(&rig, Point::new(304.0, 104.0));
    rig.double_press_at(note);
    let edits = values(&rig);
    assert!(edits.contains(&AnnotationEdit::Open(AnnotationId(2))), "a double press on the note opens it by the same rule: {edits:?}");
}

/// Presses at `from`, drags to `to` with `modifiers` held and leaves the button down, so the next frame paints the
/// live preview.
fn hold_drag(rig: &mut ToolFixture<AnnotateTool>, from: Point, to: Point, modifiers: Modifiers) {
    let (start, end) = (on_screen(rig, from), on_screen(rig, to));
    let fixture = rig.fixture();
    fixture.hold(modifiers);
    fixture.frame(vec![pointer::moved(start)]);
    fixture.frame(vec![button(start, true, modifiers)]);
    for step in 1..=3 {
        fixture.frame(vec![pointer::moved(start + (end - start) * (step as f32 / 3.0))]);
    }
}

/// Releases the drag held by [`hold_drag`] at `to`.
fn release_drag(rig: &mut ToolFixture<AnnotateTool>, to: Point, modifiers: Modifiers) {
    let end = on_screen(rig, to);
    let fixture = rig.fixture();
    fixture.frame(vec![button(end, false, modifiers)]);
    fixture.hold(Modifiers::NONE);
    fixture.idle(PAST_DOUBLE_CLICK);
}

/// Clicks at `at` with `modifiers` held, then waits past the double-click time.
fn click_holding(rig: &mut ToolFixture<AnnotateTool>, at: Point, modifiers: Modifiers) {
    let pos = on_screen(rig, at);
    let fixture = rig.fixture();
    fixture.hold(modifiers);
    fixture.frame(vec![pointer::moved(pos)]);
    fixture.frame(vec![button(pos, true, modifiers)]);
    fixture.frame(vec![button(pos, false, modifiers)]);
    fixture.hold(Modifiers::NONE);
    fixture.idle(PAST_DOUBLE_CLICK);
}

/// The painted path shapes in `shapes` stroked in `color`.
fn colored_paths(shapes: &[Shape], color: Color32) -> Vec<&egui::epaint::PathShape> {
    shapes
        .iter()
        .filter_map(|shape| match shape {
            Shape::Path(path) if path.stroke.color == ColorMode::Solid(color) => Some(path),
            _ => None,
        })
        .collect()
}

/// New Line, Arrow and PolyLine/Polygon drawings honour `SelectOptions::shift_lock`: under `LevelOrPlumb`, Shift holds
/// a line, an arrow, a polygon side and their live previews level or plumb (while hovering a polygon's first corner
/// still previews the closing side); under the default `Angle45`, Shift snaps to 45-degree steps.
#[trial]
fn new_lines_arrows_and_poly_sides_honour_shift_lock() {
    let ortho = SelectOptions { shift_lock: ShiftLock::LevelOrPlumb, ..SelectOptions::default() };
    let color = [0x10, 0x80, 0xE0];
    let stroke_color = Color32::from_rgb(color[0], color[1], color[2]);

    // Line: mostly across -> level, mostly down -> plumb; preview while held is level too.
    let mut tool = AnnotateTool::new(AnnotationTool::Draw(AnnotationKind::Line), 0).select_options(ortho);
    tool.color = color;
    let mut line_rig = rig(tool);
    let origin = Point::new(40.0, 100.0);
    let tilted = Point::new(140.0, 145.0);
    hold_drag(&mut line_rig, origin, tilted, Modifiers::SHIFT);
    let shapes = line_rig.fixture().frame_shapes();
    let paths = colored_paths(&shapes, stroke_color);
    let expected_end = on_screen(&line_rig, Point::new(140.0, 100.0));
    assert!(
        paths.iter().any(|p| p.points.len() == 2 && (p.points[1] - expected_end).length() < 0.5),
        "line preview with LevelOrPlumb stays level: {paths:?}"
    );
    release_drag(&mut line_rig, tilted, Modifiers::SHIFT);
    drag_holding(&mut line_rig, Point::new(200.0, 40.0), Point::new(215.0, 150.0), Modifiers::SHIFT);
    drag_holding(&mut line_rig, Point::new(40.0, 160.0), Point::new(140.0, 190.0), Modifiers::NONE);
    let made = created(&line_rig);
    assert_eq!(made.len(), 3, "{made:?}");
    assert!(near(made[0].points[1], Point::new(140.0, 100.0)), "mostly across stays level: {:?}", made[0].points);
    assert!(near(made[1].points[1], Point::new(200.0, 150.0)), "mostly down stays plumb: {:?}", made[1].points);
    assert!(near(made[2].points[1], Point::new(140.0, 190.0)), "without Shift stays free: {:?}", made[2].points);

    // Arrow honours LevelOrPlumb too.
    let mut arrow_rig = rig(AnnotateTool::new(AnnotationTool::Draw(AnnotationKind::Arrow), 0).select_options(ortho));
    drag_holding(&mut arrow_rig, Point::new(260.0, 180.0), Point::new(150.0, 170.0), Modifiers::SHIFT);
    let made = created(&arrow_rig);
    assert_eq!(made.len(), 1);
    assert!(near(made[0].points[1], Point::new(150.0, 180.0)), "arrow stays level: {:?}", made[0].points);

    // PolyLine: each Shift click is held level or plumb with the corner before; plain click is free.
    let mut poly_rig = rig(AnnotateTool::new(AnnotationTool::Draw(AnnotationKind::PolyLine), 0).select_options(ortho));
    click_holding(&mut poly_rig, Point::new(20.0, 20.0), Modifiers::NONE);
    click_holding(&mut poly_rig, Point::new(90.0, 30.0), Modifiers::SHIFT);
    click_holding(&mut poly_rig, Point::new(95.0, 80.0), Modifiers::SHIFT);
    click_holding(&mut poly_rig, Point::new(130.0, 60.0), Modifiers::NONE);
    poly_rig.fixture().key(Key::Enter, Modifiers::NONE);
    let made = created(&poly_rig);
    assert_eq!(made.len(), 1);
    let pts = &made[0].points;
    assert_eq!(pts.len(), 4, "{pts:?}");
    assert!(
        near(pts[1], Point::new(90.0, 20.0)) && near(pts[2], Point::new(90.0, 80.0)) && near(pts[3], Point::new(130.0, 60.0)),
        "polyline Shift clicks hold level/plumb: {pts:?}"
    );

    // Polygon rubber band: held level/plumb away from the start, but closes directly to the first corner when near it.
    let mut polygon = AnnotateTool::new(AnnotationTool::Draw(AnnotationKind::Polygon), 0).select_options(ortho);
    polygon.color = color;
    let mut polygon_rig = rig(polygon);
    for p in [Point::new(40.0, 40.0), Point::new(140.0, 40.0), Point::new(140.0, 140.0)] {
        click(&mut polygon_rig, p);
    }
    let away = on_screen(&polygon_rig, Point::new(80.0, 155.0));
    polygon_rig.fixture().hold(Modifiers::SHIFT).frame(vec![pointer::moved(away)]);
    let shapes = polygon_rig.fixture().frame_shapes();
    let paths = colored_paths(&shapes, stroke_color);
    let expected_ortho = on_screen(&polygon_rig, Point::new(80.0, 140.0));
    assert!(
        paths.iter().any(|p| p.points.len() == 4 && (p.points[3] - expected_ortho).length() < 0.5),
        "polygon rubber band holds level with Shift: {paths:?}"
    );
    let near_first = on_screen(&polygon_rig, Point::new(41.0, 41.0));
    polygon_rig.fixture().frame(vec![pointer::moved(near_first)]);
    let shapes = polygon_rig.fixture().frame_shapes();
    let paths = colored_paths(&shapes, stroke_color);
    let first_screen = on_screen(&polygon_rig, Point::new(40.0, 40.0));
    assert!(
        paths.iter().any(|p| p.points.len() == 4 && (p.points[3] - first_screen).length() < 0.5),
        "over the first corner, the rubber band closes to it even with Shift: {paths:?}"
    );
    polygon_rig.fixture().hold(Modifiers::NONE);

    // Default Angle45 still snaps a diagonal line to 45 degrees.
    let mut angle45_rig = rig_for(AnnotationKind::Line);
    drag_holding(&mut angle45_rig, Point::new(40.0, 40.0), Point::new(140.0, 120.0), Modifiers::SHIFT);
    let made = created(&angle45_rig);
    assert_eq!(made.len(), 1);
    let d = made[0].points[1] - made[0].points[0];
    assert!((d.x - d.y).abs() < 0.5 && d.x > 50.0, "default Angle45 snaps near-diagonal to 45 degrees: {:?}", made[0].points);
}

/// Place drags (TextBox and Callout) and Eraser drags (both Path swath and Strokes marching ants) paint their live
/// preview while the button is held and clear it once released.
#[trial]
fn place_and_erase_drags_paint_their_preview_while_held() {
    let color = [0x10, 0x80, 0xE0];
    let stroke_color = Color32::from_rgb(color[0], color[1], color[2]);

    // TextBox place drag: closed 4-corner outline in the tool's colour while held, gone on release.
    let mut tb = AnnotateTool::new(AnnotationTool::Draw(AnnotationKind::TextBox), 0);
    tb.color = color;
    let mut tb_rig = rig(tb);
    let (from, to) = (Point::new(50.0, 50.0), Point::new(160.0, 120.0));
    hold_drag(&mut tb_rig, from, to, Modifiers::NONE);
    let held_shapes = tb_rig.fixture().frame_shapes();
    let held_paths = colored_paths(&held_shapes, stroke_color);
    assert!(
        held_paths.iter().any(|p| p.points.len() == 5 && (p.points[0] - on_screen(&tb_rig, from)).length() < 0.5),
        "TextBox drag paints its box preview while held: {held_paths:?}"
    );
    release_drag(&mut tb_rig, to, Modifiers::NONE);
    let after_shapes = tb_rig.fixture().frame_shapes();
    assert!(colored_paths(&after_shapes, stroke_color).is_empty(), "TextBox preview clears after release");

    // Callout place drag: leader line in the tool's colour from tip to box while held, gone on release.
    let mut callout = AnnotateTool::new(AnnotationTool::Draw(AnnotationKind::Callout), 0);
    callout.color = color;
    let mut callout_rig = rig(callout);
    hold_drag(&mut callout_rig, from, to, Modifiers::NONE);
    let held_shapes = callout_rig.fixture().frame_shapes();
    let held_paths = colored_paths(&held_shapes, stroke_color);
    assert!(
        held_paths.iter().any(|p| p.points.len() == 2
            && (p.points[0] - on_screen(&callout_rig, from)).length() < 0.5
            && (p.points[1] - on_screen(&callout_rig, to)).length() < 0.5),
        "Callout drag paints its leader preview while held: {held_paths:?}"
    );
    release_drag(&mut callout_rig, to, Modifiers::NONE);
    let after_shapes = callout_rig.fixture().frame_shapes();
    assert!(colored_paths(&after_shapes, stroke_color).is_empty(), "Callout preview clears after release");

    // Path eraser: swath stroke width matches 2 * eraser_radius while held, gone on release.
    for radius in [6.0_f32, 36.0_f32] {
        let options = DrawOptions { erase: EraseMode::Path, eraser_radius: radius, ..DrawOptions::default() };
        let mut eraser_rig = rig(AnnotateTool::new(AnnotationTool::Eraser, 0).draw_options(options));
        let swath_widths = |shapes: &[Shape]| -> Vec<f32> {
            shapes
                .iter()
                .filter_map(|shape| match shape {
                    Shape::Path(path) if path.points.len() >= 2 && (path.stroke.width - 2.0 * radius).abs() < 0.1 => Some(path.stroke.width),
                    _ => None,
                })
                .collect()
        };
        hold_drag(&mut eraser_rig, Point::new(60.0, 100.0), Point::new(220.0, 100.0), Modifiers::NONE);
        let held_shapes = eraser_rig.fixture().frame_shapes();
        assert_eq!(swath_widths(&held_shapes).len(), 1, "radius {radius}: swath painted at twice the radius while held");
        let cap_circles = held_shapes.iter().filter(|shape| matches!(shape, Shape::Circle(c) if (c.radius - radius).abs() < 0.1)).count();
        assert_eq!(cap_circles, 0, "radius {radius}: the swath is a flat stroke, with no dots at its ends");
        let grey = held_shapes.iter().any(|shape| match shape {
            Shape::Path(path) if (path.stroke.width - 2.0 * radius).abs() < 0.1 => match path.stroke.color {
                ColorMode::Solid(c) => c.r().abs_diff(c.b()) < 8 && c.r().abs_diff(c.g()) < 8,
                _ => false,
            },
            _ => false,
        });
        assert!(grey, "radius {radius}: the swath is a flat grey");
        release_drag(&mut eraser_rig, Point::new(220.0, 100.0), Modifiers::NONE);
        let after_shapes = eraser_rig.fixture().frame_shapes();
        assert!(swath_widths(&after_shapes).is_empty(), "radius {radius}: swath clears after release");
    }

    // Strokes eraser: marching ants while held, gone on release.
    let mut strokes_eraser = rig(AnnotateTool::new(AnnotationTool::Eraser, 0));
    let ant_shapes = |shapes: &[Shape]| shapes.iter().filter(|shape| matches!(shape, Shape::Mesh(_) | Shape::LineSegment { .. })).count();
    let baseline_ants = ant_shapes(&strokes_eraser.fixture().frame_shapes());
    hold_drag(&mut strokes_eraser, Point::new(60.0, 100.0), Point::new(220.0, 100.0), Modifiers::NONE);
    let held_ants = ant_shapes(&strokes_eraser.fixture().frame_shapes());
    assert!(held_ants > baseline_ants, "strokes eraser paints marching ants while held: {held_ants} vs {baseline_ants}");
    release_drag(&mut strokes_eraser, Point::new(220.0, 100.0), Modifiers::NONE);
    let after_ants = ant_shapes(&strokes_eraser.fixture().frame_shapes());
    assert_eq!(after_ants, baseline_ants, "strokes eraser preview clears after release");
}

/// A shape being dragged out is previewed in a heavy outline, as the drawing it will become reads on a page.
#[trial]
fn a_dragged_shape_preview_is_a_heavy_outline() {
    let color = [0x10, 0x80, 0xE0];
    let mut tool = AnnotateTool::new(AnnotationTool::Draw(AnnotationKind::Rectangle), 0);
    tool.color = color;
    let mut shape_rig = rig(tool);
    hold_drag(&mut shape_rig, Point::new(50.0, 50.0), Point::new(160.0, 120.0), Modifiers::NONE);
    let shapes = shape_rig.fixture().frame_shapes();
    let paths = colored_paths(&shapes, Color32::from_rgb(color[0], color[1], color[2]));
    assert!(paths.iter().any(|p| p.stroke.width >= 3.0), "the preview outline is at least 3 points wide: {paths:?}");
}

/// The overlay of a selected rectangle: round resize handles. A callout's leader is painted once, in its own colour.
#[trial]
fn the_overlay_draws_round_handles_and_no_second_leader() {
    use craft_review::ui::AnnotationOverlay;
    use craft_ui::contract::Show as _;
    let mut callout = annotation(2, AnnotationKind::Callout, Rect::new(150.0, 100.0, 250.0, 140.0));
    callout.points = vec![Point::new(60.0, 160.0), Point::new(120.0, 120.0)];
    let annotations = vec![annotation(1, AnnotationKind::Rectangle, Rect::new(20.0, 20.0, 100.0, 60.0)), callout];
    let selected = vec![AnnotationId(2)];
    let shown = annotations.clone();
    let mut overlay = ToolFixture::with_view("Document", AnnotateTool::new(AnnotationTool::Select, 0), view, move |ui, cx| {
        AnnotationOverlay::new("Annotations", &shown, &selected).show_at(ui, cx.rect);
    });
    let tip = on_screen(&overlay, Point::new(60.0, 160.0));
    let shapes = overlay.fixture().frame_shapes();
    let leaders: Vec<&egui::epaint::PathShape> = shapes
        .iter()
        .filter_map(|shape| if let Shape::Path(p) = shape { Some(p) } else { None })
        .filter(|p| p.points.first().is_some_and(|q| (*q - tip).length() < 0.5))
        .collect();
    assert!(!leaders.is_empty(), "the leader paints");
    assert!(
        leaders.iter().all(|p| p.stroke.color == ColorMode::Solid(Color32::from_rgb(255, 0, 0))),
        "every leader stroke is the annotation's colour"
    );

    let mut boxed = annotations;
    boxed.truncate(1);
    let selected = vec![AnnotationId(1)];
    let mut overlay = ToolFixture::with_view("Document", AnnotateTool::new(AnnotationTool::Select, 0), view, move |ui, cx| {
        AnnotationOverlay::new("Annotations", &boxed, &selected).show_at(ui, cx.rect);
    });
    let shapes = overlay.fixture().frame_shapes();
    let discs = shapes.iter().filter(|shape| matches!(shape, Shape::Circle(c) if c.radius < 8.0)).count();
    assert!(discs >= 8, "the eight resize handles are round: {discs}");
}
