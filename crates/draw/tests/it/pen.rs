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

// ---- Curvature Pen: Delete on a selected anchor ----

// The measured rows of photocraft #12696: a click on an anchor of the draft selects it, and Delete or Backspace
// removes that anchor with the curve re-solved through the remaining ones, as for a path drawn through them.
// A case the issue does not measure keeps today's behaviour, pinned below.

/// The five anchors of the issue's measured example, halved to fit the test canvas: A, B, C, D, E.
const CURVE5: [(f64, f64); 5] = [(50.0, 150.0), (125.0, 75.0), (200.0, 150.0), (275.0, 75.0), (350.0, 150.0)];

/// The four left when C is deleted from CURVE5, in drawing order: A, B, D, E.
const CURVE4: [(f64, f64); 4] = [CURVE5[0], CURVE5[1], CURVE5[3], CURVE5[4]];

/// A Curvature Pen with the anchors of `points` clicked in drawing order.
fn curvature_drawn(points: &[(f64, f64)]) -> Pen {
    let mut pen = pen();
    pen.fixture.tool_mut().mode(PenMode::Curvature);
    for &(x, y) in points {
        pen.click_doc(x, y);
    }
    pen
}

impl Pen {
    /// Delete or Backspace, and the events it reported.
    fn delete_key(&mut self, key: egui::Key) {
        self.fixture.fixture().key(key, Modifiers::NONE);
        self.sync();
    }

    /// The whole-draft values of the `Update`s reported so far.
    fn updates(&self) -> Vec<EditSubpath> {
        self.log
            .iter()
            .filter_map(|event| match event {
                PenEvent::Edit(EditEvent::Update { value, .. }) => Some(value.clone()),
                _ => None,
            })
            .collect()
    }
}

#[trial]
fn a_selected_curvature_anchor_deletes_and_the_curve_resolves_through_the_neighbours() {
    let mut pen = curvature_drawn(&CURVE5);
    let five = pen.draft();
    assert_eq!(five.knots.len(), 5, "the draft A, B, C, D, E is open");
    pen.click_doc(CURVE5[2].0, CURVE5[2].1);
    assert_eq!(pen.draft(), five, "a click on an anchor changes nothing but the selection");
    pen.delete_key(egui::Key::Delete);
    let after = pen.draft();
    assert_eq!(after.knots.len(), 4, "C is gone");
    let drawn = curvature_drawn(&CURVE4);
    assert_eq!(after, drawn.draft(), "every handle re-solved as for a path drawn through A, B, D, E");
    assert_eq!(pen.kinds().iter().filter(|kind| **kind == "begin").count(), 1, "the open gesture stays one history step");
    assert!(pen.kinds().ends_with(&["update"]), "the removal is one Update: {:?}", pen.kinds());
    assert!(pen.log.iter().all(|event| !matches!(event, PenEvent::Finished(_))), "the draft stays open");
}

// The app hands its held draft back in before every pass, as `apply_pen` does: the selection survives that round
// trip, so a Delete in a later pass still removes the anchor clicked before it.
#[trial]
fn the_curvature_selection_survives_the_host_handing_the_draft_back() {
    let mut pen = curvature_drawn(&CURVE5);
    let five = pen.draft();
    pen.click_doc(CURVE5[2].0, CURVE5[2].1);
    {
        let mut tool = pen.fixture.tool_mut();
        tool.mode(PenMode::Curvature).draft(Some(PenDraft::Own(&five))).kept(0);
    }
    pen.fixture.fixture().settle();
    pen.sync();
    assert_eq!(pen.draft(), five, "the round trip leaves the draft");
    pen.delete_key(egui::Key::Delete);
    let drawn = curvature_drawn(&CURVE4);
    assert_eq!(pen.draft(), drawn.draft(), "the selected anchor is the one removed");
}

#[trial]
fn backspace_deletes_the_selected_curvature_anchor_like_delete() {
    let mut pen = curvature_drawn(&CURVE5);
    pen.click_doc(CURVE5[2].0, CURVE5[2].1);
    pen.delete_key(egui::Key::Backspace);
    let drawn = curvature_drawn(&CURVE4);
    assert_eq!(pen.draft(), drawn.draft(), "Backspace does the same as Delete");
}

// Deleting the last anchor stays today's removal: right after placing it, and after clicking it. The click on D
// keeps the click on E a single click.
#[trial]
fn delete_right_after_placing_the_last_anchor_removes_it_as_today() {
    let mut pen = curvature_drawn(&CURVE5);
    pen.delete_key(egui::Key::Delete);
    let drawn = curvature_drawn(&CURVE5[..4]);
    assert_eq!(pen.draft(), drawn.draft(), "the last anchor goes, as today");
}

#[trial]
fn a_click_on_the_last_anchor_and_delete_removes_the_last_as_today() {
    let mut pen = curvature_drawn(&CURVE5);
    pen.click_doc(CURVE5[3].0, CURVE5[3].1);
    pen.click_doc(CURVE5[4].0, CURVE5[4].1);
    pen.delete_key(egui::Key::Delete);
    let drawn = curvature_drawn(&CURVE5[..4]);
    assert_eq!(pen.draft(), drawn.draft(), "the last anchor goes, as today");
}

// The selection is spent by the removal: with nothing selected again the key takes back the last anchor, as today.
#[trial]
fn the_delete_after_a_selected_removal_takes_back_the_last_anchor() {
    let mut pen = curvature_drawn(&CURVE5);
    pen.click_doc(CURVE5[2].0, CURVE5[2].1);
    pen.delete_key(egui::Key::Delete);
    pen.delete_key(egui::Key::Backspace);
    let drawn = curvature_drawn(&[CURVE5[0], CURVE5[1], CURVE5[3]]);
    assert_eq!(pen.draft(), drawn.draft(), "with nothing selected again, the last anchor goes, as today");
}

// Deleting at two and one anchor keeps today's edges: one anchor left, the draft open, and the next removal
// ends it as Removed with a cancel. The clicked first anchor of two is the selected one.
#[trial]
fn the_curvature_delete_at_two_anchors_leaves_one_and_the_next_removal_ends_the_draft() {
    let mut pen = curvature_drawn(&CURVE5[..2]);
    pen.click_doc(CURVE5[0].0, CURVE5[0].1);
    pen.delete_key(egui::Key::Delete);
    let left = pen.draft();
    assert_eq!(left.knots.len(), 1, "one anchor is left");
    assert_eq!(left.knots[0].anchor, doc(125.0, 75.0), "the kept one is the unselected B");
    assert!(pen.log.iter().all(|event| !matches!(event, PenEvent::Finished(_))), "the draft stays open");
    pen.delete_key(egui::Key::Delete);
    assert_eq!(pen.log.last(), Some(&PenEvent::Finished(PenEnd::Removed)), "the only anchor's removal ends the draft");
    assert!(pen.kinds().ends_with(&["cancel", "finished"]), "{:?}", pen.kinds());
    assert!(pen.draft().knots.is_empty());
}

#[trial]
fn the_curvature_delete_of_the_only_anchor_discards_the_draft_as_today() {
    let mut pen = curvature_drawn(&CURVE5[..1]);
    pen.delete_key(egui::Key::Delete);
    assert!(pen.kinds().ends_with(&["cancel", "finished"]), "{:?}", pen.kinds());
    assert_eq!(pen.log.last(), Some(&PenEvent::Finished(PenEnd::Removed)));
    assert!(pen.draft().knots.is_empty());
}

// A resumed draft: the removal never eats a carried anchor. The new anchor is the draft's own and goes; the
// carried anchors keep their places and handles, and the draft stays open.
#[trial]
fn a_selected_curvature_deletion_spares_the_carried_anchors_of_a_resumed_draft() {
    let mut pen = pen();
    let carried = EditSubpath::new(vec![Knot::corner(doc(60.0, 60.0)), Knot::corner(doc(160.0, 60.0))], false);
    {
        let mut tool = pen.fixture.tool_mut();
        tool.mode(PenMode::Curvature).kept(2).draft(Some(PenDraft::Own(&carried)));
    }
    pen.fixture.fixture().settle();
    pen.click_doc(260.0, 160.0);
    assert_eq!(pen.draft().knots.len(), 3, "the carried anchors and the new one");
    pen.delete_key(egui::Key::Delete);
    let draft = pen.draft();
    assert_eq!(draft, carried, "the draft's own anchor is gone, the carried ones keep places and handles");
    assert!(pen.log.iter().all(|event| !matches!(event, PenEvent::Finished(_))), "the draft stays open");
}

// The removal is one `Update` of the open gesture, which keeps its single Path-named history step; the `Update`
// before the removal holds the whole draft, so the host handing that value back restores the five anchors.
#[trial]
fn a_selected_curvature_delete_is_one_undoable_update_of_the_open_gesture() {
    let mut pen = curvature_drawn(&CURVE5);
    pen.click_doc(CURVE5[2].0, CURVE5[2].1);
    let five = pen.draft();
    pen.delete_key(egui::Key::Delete);
    let labels: Vec<String> = pen
        .log
        .iter()
        .filter_map(|event| match event {
            PenEvent::Edit(EditEvent::Begin { label, .. }) => Some(label.to_string()),
            _ => None,
        })
        .collect();
    assert_eq!(labels, ["Path"], "one history step, named by the pen's word");
    let updates = pen.updates();
    let (last, earlier) = updates.split_last().expect("the Updates so far");
    let drawn = curvature_drawn(&CURVE4);
    assert_eq!(last, &drawn.draft(), "the removal itself");
    assert_eq!(earlier[earlier.len() - 1], five, "the previous Update holds the whole five-anchor draft");
    assert!(pen.fixture.tool_mut().replace(&five));
    pen.sync();
    assert_eq!(pen.draft(), five, "handing that value back restores the draft: the host's Undo");
}

// The Pen mode is untouched: a click there selects no anchor, and Delete takes back the last anchor - here the
// corner anchor a click on a middle one added.
#[trial]
fn the_pen_still_deletes_the_last_anchor_whatever_was_clicked() {
    let mut pen = pen();
    pen.click_doc(60.0, 60.0);
    pen.click_doc(160.0, 60.0);
    pen.click_doc(160.0, 160.0);
    let three = pen.draft();
    pen.click_doc(160.0, 60.0);
    assert_eq!(pen.draft().knots.len(), 4, "a Pen click on a middle anchor adds one");
    pen.delete_key(egui::Key::Delete);
    assert_eq!(pen.draft(), three, "the Pen takes back the last anchor, as today");
}

// Auto Add/Delete stays off in Curvature mode: a press on a segment of the draft adds its anchor last, as the
// mode always did, and the key takes it back.
#[trial]
fn auto_add_delete_stays_off_for_the_curvature_pen() {
    let mut pen = pen();
    {
        let mut tool = pen.fixture.tool_mut();
        tool.mode(PenMode::Curvature).auto_add_delete(true);
    }
    for (x, y) in [(60.0, 60.0), (160.0, 60.0), (260.0, 160.0)] {
        pen.click_doc(x, y);
    }
    pen.click_doc(110.0, 55.0);
    let draft = pen.draft();
    assert_eq!(draft.knots.len(), 4, "a press on a segment adds a new last anchor, it does not insert there");
    assert_eq!(draft.knots[3].anchor, doc(110.0, 55.0));
    pen.delete_key(egui::Key::Delete);
    assert_eq!(pen.draft().knots.len(), 3, "the key takes it back, as today");
}

// The host's Undo keeps today's rule while another anchor is selected: it takes back the draft's last anchor,
// and with the selection spent the next key takes back the next-to-last as well.
#[trial]
fn remove_last_takes_back_the_last_anchor_even_with_another_one_selected() {
    let mut pen = curvature_drawn(&CURVE5);
    pen.click_doc(CURVE5[2].0, CURVE5[2].1);
    assert!(pen.fixture.tool_mut().remove_last());
    pen.sync();
    let drawn = curvature_drawn(&CURVE5[..4]);
    assert_eq!(pen.draft(), drawn.draft(), "the Undo takes back E, not the selected C");
    pen.delete_key(egui::Key::Delete);
    let drawn = curvature_drawn(&CURVE5[..3]);
    assert_eq!(pen.draft(), drawn.draft(), "and the key now takes back the next-to-last, D");
}

// The double-click toggle is a press on that anchor too: afterwards Delete removes the toggled anchor. Not a
// measured row of the issue; it follows the selected-anchor rule the issue states for a click.
#[trial]
fn a_double_clicked_curvature_anchor_then_deletes_as_the_selected_one() {
    let mut pen = curvature_drawn(&CURVE5);
    pen.click_doc(CURVE5[2].0, CURVE5[2].1);
    pen.click_doc(CURVE5[2].0, CURVE5[2].1);
    let toggled = pen.draft();
    assert_eq!(toggled.knots.len(), 5, "the toggle changes the anchor, not the count");
    assert!(!toggled.knots[2].smooth, "C became a corner");
    pen.delete_key(egui::Key::Delete);
    assert_eq!(pen.draft().knots.len(), 4, "C is gone");
    assert!(pen.draft().knots.iter().all(|k| k.smooth), "the toggled anchor is the one removed, the rest are smooth again");
    let drawn = curvature_drawn(&CURVE4);
    assert_eq!(pen.draft(), drawn.draft(), "the curve re-solves through A, B, D, E as always");
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
