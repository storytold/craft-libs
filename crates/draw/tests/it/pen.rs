//! The Pen and the Curvature Pen: anchors, handles, keys, Auto Add/Delete, the close cue and the tool's access
//! node, driven through `ToolFixture`.

use craft_behavior::kurbo::Point;
use craft_behavior::{CanvasView, EditEvent};
use craft_draw::ui::pen::{EditSubpath, Knot, PenDraft, PenEnd, PenEvent, PenMode, PenTool};
use craft_ui::testing::{ToolFixture, trial};
use egui::{Event, Modifiers, PointerButton, Pos2, Rect, vec2};

/// The document point drawn at the middle of the canvas: the canvas shows (0, 0) to (400, 300) at zoom 1.
const MIDDLE: Point = Point::new(200.0, 150.0);

fn view_of(room: Rect) -> CanvasView {
    CanvasView::new(room, MIDDLE, 1.0)
}

/// A pen on a canvas, with the events it reported so far.
struct Pen {
    fixture: ToolFixture<PenTool>,
    view: fn(Rect) -> CanvasView,
    log: Vec<PenEvent>,
}

fn pen() -> Pen {
    pen_with(PenTool::new(), view_of)
}

fn pen_with(tool: PenTool, view: fn(Rect) -> CanvasView) -> Pen {
    Pen { fixture: ToolFixture::with_view("Pen", tool, view, |_, _| {}), view, log: Vec::new() }
}

fn button(at: Pos2, pressed: bool, modifiers: Modifiers) -> Event {
    Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers }
}

fn doc(x: f64, y: f64) -> Point {
    Point::new(x, y)
}

impl Pen {
    fn sync(&mut self) {
        let events = self.fixture.tool_mut().take_events();
        self.log.extend(events);
    }

    /// The screen point of the document point `(x, y)`.
    fn at(&self, x: f64, y: f64) -> Pos2 {
        (self.view)(self.fixture.canvas().rect).to_screen(doc(x, y))
    }

    fn press(&mut self, at: Pos2, modifiers: Modifiers) {
        self.fixture.fixture().hold(modifiers);
        self.fixture.fixture().frame(vec![Event::PointerMoved(at)]);
        self.fixture.fixture().frame(vec![button(at, true, modifiers)]);
        self.sync();
    }

    fn move_to(&mut self, at: Pos2, modifiers: Modifiers) {
        self.fixture.fixture().hold(modifiers);
        self.fixture.fixture().frame(vec![Event::PointerMoved(at)]);
        self.sync();
    }

    fn release(&mut self, at: Pos2, modifiers: Modifiers) {
        self.fixture.fixture().frame(vec![button(at, false, modifiers)]);
        self.fixture.fixture().hold(Modifiers::NONE);
        self.fixture.fixture().frame(Vec::new());
        self.sync();
    }

    /// Space goes down or up.
    fn space(&mut self, down: bool) {
        let key = Event::Key { key: egui::Key::Space, physical_key: None, pressed: down, repeat: false, modifiers: Modifiers::NONE };
        self.fixture.fixture().frame(vec![key]);
        self.sync();
    }

    fn click(&mut self, at: Pos2, modifiers: Modifiers) {
        self.press(at, modifiers);
        self.release(at, modifiers);
    }

    fn click_doc(&mut self, x: f64, y: f64) {
        let at = self.at(x, y);
        self.click(at, Modifiers::NONE);
    }

    /// A drag over several frames: press, two moves, release.
    fn drag(&mut self, from: Pos2, to: Pos2, modifiers: Modifiers) {
        self.press(from, modifiers);
        self.move_to(from.lerp(to, 0.5), modifiers);
        self.move_to(to, modifiers);
        self.release(to, modifiers);
    }

    /// The draft as the host holds it: the last value the pen reported, empty once it finished.
    fn draft(&self) -> EditSubpath {
        let mut now = EditSubpath::default();
        for event in &self.log {
            match event {
                PenEvent::Edit(EditEvent::Update { value, .. }) => now = value.clone(),
                PenEvent::Edit(EditEvent::Commit { after, .. }) => now = after.clone(),
                PenEvent::Finished(_) => now = EditSubpath::default(),
                _ => {}
            }
        }
        now
    }

    /// The path the last `Commit` filed.
    fn filed(&self) -> Option<EditSubpath> {
        self.log.iter().rev().find_map(|event| match event {
            PenEvent::Edit(EditEvent::Commit { after, .. }) => Some(after.clone()),
            _ => None,
        })
    }

    fn kinds(&self) -> Vec<&'static str> {
        self.log
            .iter()
            .map(|event| match event {
                PenEvent::Edit(EditEvent::Begin { .. }) => "begin",
                PenEvent::Edit(EditEvent::Update { .. }) => "update",
                PenEvent::Edit(EditEvent::Commit { .. }) => "commit",
                PenEvent::Edit(EditEvent::Cancel { .. }) => "cancel",
                PenEvent::Edit(_) => "other",
                PenEvent::Finished(_) => "finished",
                PenEvent::Resumed(..) => "resumed",
            })
            .collect()
    }

    /// A draft of three corner anchors at (60, 60), (160, 60), (160, 160).
    fn three() -> Pen {
        let mut pen = pen();
        for (x, y) in [(60.0, 60.0), (160.0, 60.0), (160.0, 160.0)] {
            pen.click_doc(x, y);
        }
        pen
    }
}

#[trial]
fn a_press_on_the_first_anchor_closes_the_path_on_release_with_or_without_a_drag() {
    let mut pen = Pen::three();
    let first = pen.at(60.0, 60.0);
    pen.press(first, Modifiers::NONE);
    assert!(!pen.draft().closed, "closes on release");
    pen.release(first, Modifiers::NONE);
    let done = pen.filed().expect("commit");
    assert!(pen.kinds().ends_with(&["commit", "finished"]), "{:?}", pen.kinds());
    assert!(done.closed && done.knots.len() == 3 && done.knots[0] == Knot::corner(doc(60.0, 60.0)));

    let mut pen = Pen::three();
    let first = pen.at(60.0, 60.0);
    pen.drag(first, first + vec2(-30.0, 20.0), Modifiers::NONE);
    let done = pen.filed().expect("commit");
    assert!(done.closed);
    assert_eq!(done.knots[0], Knot::smooth(doc(60.0, 60.0), doc(30.0, 80.0)));
    assert_eq!(done.knots.len(), 3, "no anchor added");
}

// Over the first anchor of an open draft the Pen's cursor carries the close badge (#4596); over the last anchor and
// over empty canvas it does not.
#[trial]
fn the_cursor_has_the_close_badge_only_over_the_first_anchor() {
    use craft_behavior::{Cursor, CursorBadge, PaintedCursor};
    let mut pen = Pen::three();
    let mut badge_at = |x: f64, y: f64| {
        let at = pen.at(x, y);
        pen.move_to(at, Modifiers::NONE);
        pen.fixture.fixture().frame(Vec::new());
        match craft_ui::runtime::cursor::current(pen.fixture.fixture().ctx()) {
            Some(Cursor::Painted(PaintedCursor::Tool { badge, .. })) => badge,
            other => panic!("the Pen shows its own cursor: {other:?}"),
        }
    };
    assert_eq!(badge_at(60.0, 60.0), Some(CursorBadge::Close), "over the first anchor");
    assert_eq!(badge_at(300.0, 250.0), None, "over empty canvas");
    assert_eq!(badge_at(160.0, 160.0), None, "the last anchor does not close");
    assert_eq!(badge_at(60.0, 60.0), Some(CursorBadge::Close), "back on the first anchor");
}

#[trial]
fn remove_last_waits_for_a_press_that_is_still_down() {
    let mut pen = Pen::three();
    let at = pen.at(260.0, 160.0);
    pen.press(at, Modifiers::NONE);
    assert!(!pen.fixture.tool_mut().remove_last(), "the press is held");
    pen.release(at, Modifiers::NONE);
    assert_eq!(pen.draft().knots.len(), 4);
    assert!(pen.fixture.tool_mut().remove_last());
    pen.sync();
    assert_eq!(pen.draft().knots.len(), 3, "one request removes one anchor");
}

// Delete and the host's Undo agree: both take back the last anchor of the selected tool's draft, and both leave a
// draft kept for another tool (`PenDraft::Foreign`) to the app, whether it was handed in kept or became another
// tool's while open.
#[trial]
fn delete_and_undo_agree_on_whose_anchors_they_take_back() {
    let mut drawn = Pen::three();
    drawn.fixture.fixture().key(egui::Key::Delete, Modifiers::NONE);
    drawn.sync();
    assert_eq!(drawn.draft().knots.len(), 2);
    let open = drawn.draft();
    drawn.fixture.tool_mut().draft(Some(PenDraft::Foreign(&open)));
    drawn.fixture.fixture().key(egui::Key::Delete, Modifiers::NONE);
    assert!(!drawn.fixture.tool_mut().remove_last(), "Undo is the app's");
    drawn.sync();
    assert_eq!(drawn.draft().knots.len(), 2, "the key is the app's");

    let mut pen = pen();
    let kept = EditSubpath::new(vec![Knot::corner(doc(60.0, 60.0)), Knot::corner(doc(160.0, 60.0)), Knot::corner(doc(160.0, 160.0))], false);
    pen.fixture.tool_mut().draft(Some(PenDraft::Foreign(&kept)));
    pen.fixture.fixture().settle();
    pen.fixture.fixture().key(egui::Key::Delete, Modifiers::NONE);
    assert!(!pen.fixture.tool_mut().remove_last());
    pen.sync();
    assert_eq!(pen.kinds(), ["begin"], "a kept draft loses no anchor to either");
    pen.fixture.tool_mut().draft(Some(PenDraft::Own(&kept)));
    pen.click_doc(260.0, 160.0);
    pen.fixture.fixture().key(egui::Key::Delete, Modifiers::NONE);
    pen.sync();
    assert_eq!(pen.draft(), kept, "back with its tool, Delete takes back the added anchor");
    pen.click_doc(260.0, 160.0);
    assert!(pen.fixture.tool_mut().remove_last());
    pen.sync();
    assert_eq!(pen.draft(), kept, "and so does Undo");
}

#[trial]
fn a_quick_drag_in_one_frame_pulls_the_handles() {
    let mut pen = pen();
    pen.click_doc(60.0, 60.0);
    let (from, to) = (pen.at(160.0, 60.0), pen.at(160.0, 60.0) + vec2(40.0, 30.0));
    pen.fixture.fixture().frame_through(vec![
        Event::PointerMoved(from),
        button(from, true, Modifiers::NONE),
        Event::PointerMoved(to),
        button(to, false, Modifiers::NONE),
    ]);
    pen.sync();
    assert_eq!(pen.draft().knots[1], Knot::smooth(doc(160.0, 60.0), doc(200.0, 90.0)));
}

#[trial]
fn the_pen_is_a_tool_every_canvas_accepts() {
    let mut fixture = ToolFixture::with_view("Pen", PenTool::new(), view_of, |_, _| {});
    fixture.check();
}

// With Space held mid-drag the anchor and both handles move by the pointer's travel; released, the handle follows
// the pointer again. A pass that repeats the pointer moves nothing.
#[trial]
fn space_moves_the_anchor_with_its_handles() {
    let mut pen = pen();
    pen.click_doc(60.0, 60.0);
    let from = pen.at(160.0, 60.0);
    pen.press(from, Modifiers::NONE);
    pen.move_to(from + vec2(40.0, 30.0), Modifiers::NONE);
    assert_eq!(pen.draft().knots[1], Knot::smooth(doc(160.0, 60.0), doc(200.0, 90.0)));

    pen.space(true);
    pen.move_to(from + vec2(60.0, 30.0), Modifiers::NONE);
    assert_eq!(pen.draft().knots[1], Knot::smooth(doc(180.0, 60.0), doc(220.0, 90.0)), "the anchor took the 20 points of travel");
    pen.move_to(from + vec2(60.0, 30.0), Modifiers::NONE);
    assert_eq!(pen.draft().knots[1].anchor, doc(180.0, 60.0), "a repeated pointer moves nothing");

    pen.space(false);
    pen.move_to(from + vec2(60.0, 50.0), Modifiers::NONE);
    let knot = pen.draft().knots[1];
    assert_eq!(knot.anchor, doc(180.0, 60.0));
    assert_eq!(knot.out_ctrl, doc(220.0, 110.0), "the handle follows the pointer again");
    pen.release(from + vec2(60.0, 50.0), Modifiers::NONE);
}

// A draft the host hands in as a resumed path (`kept` its anchors) gains one anchor; the first removal updates the
// draft, the next goes back to the handed-in path and ends the draft as discarded, never as an empty commit. A draft
// handed in with nothing kept (the path Undo gave back) loses its anchors one by one.
#[trial]
fn removing_a_resumed_drafts_own_anchor_restores_the_path_and_ends_the_draft() {
    let mut pen = pen();
    let own = EditSubpath::new(vec![Knot::corner(doc(60.0, 60.0)), Knot::corner(doc(160.0, 60.0))], false);
    pen.fixture.tool_mut().draft(Some(PenDraft::Own(&own))).kept(2);
    pen.fixture.fixture().settle();
    pen.sync();
    assert_eq!(pen.kinds(), ["begin"], "the handed-in draft begins its gesture at once");

    pen.click_doc(260.0, 160.0);
    assert_eq!(pen.draft().knots.len(), 3);
    assert!(pen.fixture.tool_mut().remove_last());
    pen.sync();
    assert_eq!(pen.draft(), own, "the added anchor is gone");
    assert!(pen.fixture.tool_mut().remove_last());
    pen.sync();
    assert_eq!(pen.log.last(), Some(&PenEvent::Finished(PenEnd::Discarded)));
    assert!(pen.kinds().ends_with(&["cancel", "finished"]), "{:?}", pen.kinds());
    assert!(!pen.kinds().contains(&"commit"), "no empty commit removes the path: {:?}", pen.kinds());
    assert_eq!(pen.draft(), EditSubpath::default());
}

#[trial]
fn a_handed_in_draft_with_nothing_kept_loses_its_anchors_to_undo() {
    let mut pen = pen();
    let given_back = EditSubpath::new(vec![Knot::corner(doc(60.0, 60.0)), Knot::corner(doc(160.0, 60.0)), Knot::corner(doc(160.0, 160.0))], false);
    pen.fixture.tool_mut().draft(Some(PenDraft::Own(&given_back)));
    pen.fixture.fixture().settle();
    assert!(pen.fixture.tool_mut().remove_last());
    pen.sync();
    assert_eq!(pen.draft().knots.len(), 2, "the handed-in anchors are the selected tool's to take back");
    assert!(pen.log.iter().all(|e| !matches!(e, PenEvent::Finished(_))), "the draft stays open");
}

// ---- Curvature mode ----

// Curvature mode with `kept(2)`: a handed-in draft resumes, a new click keeps knots 0 and 1's handles but bends knot
// 1's out handle, and `replace` is one `Update`, refused while a press is down.
#[trial]
fn a_curvature_resume_keeps_the_carried_knots_and_replace_updates_the_draft() {
    let mut pen = pen();
    let carried = EditSubpath::new(vec![Knot::smooth(doc(60.0, 60.0), doc(80.0, 70.0)), Knot::smooth(doc(160.0, 60.0), doc(180.0, 40.0))], false);
    {
        let mut tool = pen.fixture.tool_mut();
        tool.mode(PenMode::Curvature);
        tool.kept(2);
        tool.draft(Some(PenDraft::Own(&carried)));
    }
    pen.fixture.fixture().settle();
    pen.sync();
    assert_eq!(pen.kinds(), ["begin"]);

    pen.click_doc(260.0, 160.0);
    let draft = pen.draft();
    assert_eq!(draft.knots.len(), 3);
    assert_eq!(draft.knots[0], carried.knots[0], "the first knot keeps its handles");
    assert_eq!(draft.knots[1].anchor, carried.knots[1].anchor);
    assert_eq!(draft.knots[1].in_ctrl, carried.knots[1].in_ctrl, "the last kept knot keeps its in handle");
    assert_eq!(draft.knots[1].out_ctrl, carried.knots[1].anchor, "the last kept knot's out handle starts the new run");

    let before = pen.log.len();
    assert!(pen.fixture.tool_mut().replace(&carried));
    pen.sync();
    assert_eq!(pen.log.len(), before + 1, "one event");
    assert!(matches!(pen.log.last(), Some(PenEvent::Edit(EditEvent::Update { value, .. })) if *value == carried));

    let at = pen.at(300.0, 100.0);
    pen.press(at, Modifiers::NONE);
    let during = pen.log.len();
    assert!(!pen.fixture.tool_mut().replace(&carried), "refused while the press is down");
    pen.sync();
    assert_eq!(pen.log.len(), during);
    pen.release(at, Modifiers::NONE);
}

// The carried anchors of a resumed path keep their handles: a double click on one that is not the carried end changes
// nothing, and a press on one adds an anchor instead of moving it.
#[trial]
fn carried_anchors_are_neither_toggled_nor_grabbed() {
    let mut pen = pen();
    let carried = EditSubpath::new(
        vec![Knot::corner(doc(60.0, 60.0)), Knot::smooth(doc(160.0, 60.0), doc(180.0, 40.0)), Knot::corner(doc(160.0, 160.0))],
        false,
    );
    {
        let mut tool = pen.fixture.tool_mut();
        tool.mode(PenMode::Curvature);
        tool.kept(3);
        tool.draft(Some(PenDraft::Own(&carried)));
    }
    pen.fixture.fixture().settle();
    // The second anchor: a press on the first of a draft of three or more closes it instead.
    pen.click_doc(160.0, 60.0);
    pen.click_doc(160.0, 60.0);
    let draft = pen.draft();
    assert_eq!(draft.knots[..3], carried.knots[..], "the carried anchors keep their handles and places");
    assert!(draft.knots.len() > 3, "a press on a carried anchor adds an anchor");
}

// With a subpath to carry on, a Curvature press away from every end point reports the resume and adds its anchor to
// the carried subpath, whose anchors keep their handles.
#[trial]
fn a_curvature_press_carries_on_the_given_subpath() {
    let mut pen = pen();
    pen.fixture.tool_mut().mode(PenMode::Curvature);
    let carried = EditSubpath::new(vec![Knot::corner(doc(60.0, 60.0)), Knot::corner(doc(160.0, 60.0))], false);
    pen.fixture.tool_mut().carry_on(Some((7, craft_draw::ui::pen::KnotRef::new(0, 1), &carried)));
    pen.click_doc(300.0, 200.0);
    assert_eq!(pen.kinds().first(), Some(&"resumed"), "{:?}", pen.kinds());
    let draft = pen.draft();
    assert_eq!(draft.knots.len(), 3, "the carried anchors and the new one");
    assert_eq!((draft.knots[0], draft.knots[1].anchor), (carried.knots[0], carried.knots[1].anchor), "the carried anchors keep their places");
    assert_eq!(draft.knots[2].anchor, doc(300.0, 200.0));
}

/// Three clicks of the Curvature Pen far apart give three anchors, each click an `Update`: on its own, and with the
/// host handing the draft it holds back in before every frame, as an app's per-pass configuration does.
#[trial]
fn three_curvature_clicks_give_three_anchors() {
    for round_trip in [false, true] {
        let mut pen = pen();
        pen.fixture.tool_mut().mode(PenMode::Curvature);
        let points = [(60.0, 160.0), (160.0, 60.0), (260.0, 160.0)];
        for (n, (x, y)) in points.into_iter().enumerate() {
            let at = pen.at(x, y);
            let updates = pen.kinds().iter().filter(|kind| **kind == "update").count();
            for event in [Event::PointerMoved(at), button(at, true, Modifiers::NONE), button(at, false, Modifiers::NONE)] {
                if round_trip {
                    let held = pen.draft();
                    let mut tool = pen.fixture.tool_mut();
                    tool.mode(PenMode::Curvature).draft((!held.knots.is_empty()).then_some(PenDraft::Own(&held))).kept(0);
                }
                pen.fixture.fixture().frame(vec![event]);
                pen.sync();
            }
            let what = format!("round trip {round_trip}, click {}", n + 1);
            assert_eq!(pen.draft().knots.len(), n + 1, "{what}: {:?}", pen.kinds());
            assert!(pen.kinds().iter().filter(|kind| **kind == "update").count() > updates, "{what}: an Update");
        }
    }
}

// ---- Path modes and Path Options ----

/// A host that ended the draft itself hands in no draft: the pen drops its run without an event, and the next press
/// starts a new draft rather than carrying on the ended one.
#[trial]
fn a_draft_the_host_ended_is_not_carried_on() {
    let mut pen = pen();
    pen.click_doc(100.0, 100.0);
    pen.click_doc(200.0, 100.0);
    assert_eq!(pen.draft().knots.len(), 2);
    pen.fixture.tool_mut().draft(None);
    assert!(pen.fixture.tool_mut().take_events().is_empty(), "the host ended it: the pen reports nothing");
    pen.log.clear();
    pen.click_doc(100.0, 200.0);
    assert_eq!(pen.draft().knots.len(), 1, "a new draft");
}

// ---- a Shift-click: where the new anchor goes ----

/// The document point the canvas shows at its centre for the numbers measured in #12893: the room of a trial is 400
/// by 300 points, so at zoom 1 this shows the document from (50, 250) to (450, 550) and every measured point of the
/// issue lands on the canvas.
const MEASURED_MIDDLE: Point = Point::new(250.0, 400.0);

fn measured_view(room: Rect) -> CanvasView {
    CanvasView::new(room, MEASURED_MIDDLE, 1.0)
}

fn measured_pen() -> Pen {
    pen_with(PenTool::new(), measured_view)
}

/// A click at the document point `(x, y)` with ⇧ held.
fn shift_click(pen: &mut Pen, x: f64, y: f64) {
    let at = pen.at(x, y);
    pen.click(at, Modifiers::SHIFT);
}

/// A point within `within` document units of `want`.
fn assert_near(at: Point, want: Point, within: f64, what: &str) {
    assert!(at.distance(want) < within, "{what}: got ({}, {}), expected about ({}, {})", at.x, at.y, want.x, want.y);
}

// The three 45 degree rows measured in #12893. The rule before this change turned the click about the previous
// anchor onto the nearest 45° ray and kept its distance, which gave (421.7, 421.7), (182.5, 382.5) and (275.0, 475.0):
// the anchor left the click's shorter axis.
#[trial]
fn a_shift_click_on_a_diagonal_takes_the_shorter_offset_on_both_axes() {
    let mut pen = measured_pen();
    pen.click_doc(300.0, 300.0);
    shift_click(&mut pen, 400.0, 440.0);
    assert_eq!(pen.draft().knots[1].anchor, doc(400.0, 400.0), "from (300, 300), a click at (400, 440)");

    let mut pen = measured_pen();
    pen.click_doc(100.0, 300.0);
    shift_click(&mut pen, 200.0, 360.0);
    assert_eq!(pen.draft().knots[1].anchor, doc(160.0, 360.0), "from (100, 300), a click at (200, 360)");
    shift_click(&mut pen, 240.0, 500.0);
    assert_eq!(pen.draft().knots[2].anchor, doc(240.0, 440.0), "from (160, 360), a click at (240, 500)");
    assert_eq!(pen.draft().knots[0].anchor, doc(100.0, 300.0), "the anchors already placed stay put");
}

// The 0 degree row measured in #12893: the anchor goes onto the axis through the previous one and keeps the click's
// offset along it. Turning onto the ray kept the whole distance and gave (304.0, 300.0).
#[trial]
fn a_shift_click_nearest_an_axis_moves_the_anchor_onto_that_axis() {
    let mut pen = measured_pen();
    pen.click_doc(100.0, 300.0);
    shift_click(&mut pen, 300.0, 340.0);
    assert_eq!(pen.draft().knots[1].anchor, doc(300.0, 300.0), "from (100, 300), a click at (300, 340)");
    assert_eq!(pen.draft().knots[0].anchor, doc(100.0, 300.0), "the first anchor stays put");
}

// Which ray is nearest decides the rule, at half a step (22.5 degrees): a 30 degree click takes the diagonal, and so
// does one just past the half way line, where an axis band of 30 degrees would still have flattened it.
#[trial]
fn a_shift_click_past_the_half_way_line_between_axis_and_diagonal_takes_the_diagonal() {
    let mut pen = measured_pen();
    pen.click_doc(100.0, 300.0);
    shift_click(&mut pen, 220.0, 369.0);
    assert_eq!(pen.draft().knots[1].anchor, doc(169.0, 369.0), "a 30 degree click: the shorter offset (69) on both axes");

    let mut pen = measured_pen();
    pen.click_doc(100.0, 300.0);
    shift_click(&mut pen, 220.0, 351.0);
    assert_eq!(pen.draft().knots[1].anchor, doc(151.0, 351.0), "a 23 degree click is nearer the diagonal than the axis");
}

// The signs of the pointer's offset are kept in all four quadrants.
#[trial]
fn a_shift_click_keeps_the_signs_of_the_pointer_in_every_quadrant() {
    for (sx, sy) in [(1.0, 1.0), (1.0, -1.0), (-1.0, 1.0), (-1.0, -1.0)] {
        let mut pen = measured_pen();
        pen.click_doc(250.0, 400.0);
        shift_click(&mut pen, 250.0 + 60.0 * sx, 400.0 + 90.0 * sy);
        assert_eq!(pen.draft().knots[1].anchor, doc(250.0 + 60.0 * sx, 400.0 + 60.0 * sy), "pointer offset ({} 60, {} 90)", sx, sy);
    }
}

// Guard tests, passing before and after this change: a click already on a ray keeps its whole offset, and the first
// anchor of a run has no previous anchor to snap from.
#[trial]
fn a_shift_click_exactly_on_a_ray_or_with_no_previous_anchor_places_the_pointer() {
    let mut pen = measured_pen();
    pen.click_doc(100.0, 300.0);
    shift_click(&mut pen, 160.0, 360.0);
    assert_eq!(pen.draft().knots[1].anchor, doc(160.0, 360.0), "exactly 45 degrees");

    let mut pen = measured_pen();
    pen.click_doc(100.0, 300.0);
    shift_click(&mut pen, 200.0, 300.0);
    assert_eq!(pen.draft().knots[1].anchor, doc(200.0, 300.0), "exactly on the horizontal");

    let mut pen = measured_pen();
    shift_click(&mut pen, 200.0, 360.0);
    assert_eq!(pen.draft().knots[0].anchor, doc(200.0, 360.0), "no previous anchor: the click places itself");
}

// Guard test: ⇧ in a drag still turns the pulled handle onto the nearest 45 degree ray, keeping the pointer's
// distance. #12893 lists the handle drag as not measured, so it keeps the rotation.
#[trial]
fn shift_in_a_drag_still_turns_the_handle_onto_the_nearest_45_degree_ray() {
    let mut pen = measured_pen();
    pen.click_doc(200.0, 300.0);
    let start = pen.at(300.0, 300.0);
    pen.press(start, Modifiers::NONE);
    let to = pen.at(360.0, 380.0);
    pen.move_to(to, Modifiers::SHIFT);
    let knot = pen.draft().knots[1];
    assert_eq!(knot.anchor, doc(300.0, 300.0), "the anchor was placed without ⇧");
    // The pointer's offset (60, 80) has length 100: turned to the 45 degree ray it reaches (370.7, 370.7), further
    // out than the shorter offset (60, 60) a Shift-click now takes.
    let turned = doc(300.0 + 100.0 * 45.0f64.to_radians().cos(), 300.0 + 100.0 * 45.0f64.to_radians().sin());
    assert_near(knot.out_ctrl, turned, 0.01, "the out handle keeps the pointer's distance");
    assert_near(knot.in_ctrl, doc(600.0 - turned.x, 600.0 - turned.y), 0.01, "the in handle stays opposite it");
    pen.release(to, Modifiers::SHIFT);
    assert_eq!(pen.draft().knots.len(), 2, "releasing after a shift move adds no anchor");
}

// Guard test: the ⇧ snap applies only to the anchor a press adds. A Shift press on the first anchor of a draft of
// two or more still closes the path and adds nothing.
#[trial]
fn a_shift_press_on_the_first_anchor_still_closes_the_path() {
    let mut pen = measured_pen();
    for (x, y) in [(100.0, 300.0), (220.0, 300.0), (220.0, 420.0)] {
        pen.click_doc(x, y);
    }
    let first = pen.at(100.0, 300.0);
    pen.click(first, Modifiers::SHIFT);
    let done = pen.filed().expect("the close commits");
    assert!(done.closed, "⇧ on the first anchor closes");
    assert_eq!(done.knots.len(), 3, "no snapped anchor is added: {:?}", done.knots);
    assert_eq!(done.knots[0].anchor, doc(100.0, 300.0), "the first anchor keeps its place");
}
