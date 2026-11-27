//! `CanvasTool`, `ToolHost` and `tool_pick` (8.3): a drag is one `Begin`...`Commit`, Escape cancels it once, a press
//! on a canvas control never reaches the tool, and a group's letter picks and cycles its tools.

use craft_behavior::geometry::{SnapKind, SnapLine, SnapOptions, SnapTarget, SnapTargets};
use craft_behavior::kurbo::{Point, Size};
use craft_behavior::{Axis, CancelReason, CanvasView, Cursor, CursorIcon, GesturePhase, PointerButton, PointerKind, PointerSample};
use craft_keymap::Mods;
use craft_ui::canvas::{
    Autoscroll, Canvas, CanvasScrollbars, CanvasTool, ContextRequest, CropOverlay, HandTool, Momentary, MomentaryCommands, MomentaryTool,
    NavigationOptions, Snap, StandInKey, ToolAction, ToolClaims, ToolContext, ToolHost, ToolInput, ToolKey, ToolPath, ToolPointer, ToolRegion,
    canvas_space, tool_pass,
};
use craft_ui::contract::tooltip::DELAY;
use craft_ui::contract::{
    Bind, Common, Component, Cx, DragPreview, DropMark, DropTarget, MeasureCx, MemoProps, MemoView, Pointer as _, Props as _, Sampler as _,
    Show as _, ShowContainer as _,
};
use craft_ui::dialog::Dialog;
use craft_ui::display::Text;
use craft_ui::primitives::Draw;
use craft_ui::runtime::access::Role;
use craft_ui::runtime::commands::{CommandGate, CommandSet as _};
use craft_ui::runtime::drag;
use craft_ui::runtime::frame;
use craft_ui::runtime::windows::WindowHost;
use craft_ui::style::{DataColor, StyleCx};
use craft_ui::testing::focus::key_release;
use craft_ui::testing::input::key_press;
use craft_ui::testing::layout::in_region;
use craft_ui::testing::{Fixture, MemoRun, ToolFixture, last_frame_memo_runs, pointer, trial};
use craft_ui::text::NumberField;
use craft_ui_test_kit::contract::PlainStyle;
use egui::{Color32, Event, Id, Key, Modifiers, Pos2, Rect, Response, Shape, Vec2};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

const DOCUMENT: Size = Size::new(1200.0, 900.0);

#[derive(Clone, Copy, Debug, PartialEq)]
enum Call {
    Press,
    Drag,
    Release,
    Cancel(CancelReason),
}

/// A tool that claims every press it takes and records what the host called.
#[derive(Default)]
struct Recorder {
    calls: Vec<Call>,
}

impl Recorder {
    fn count(&self, call: Call) -> usize {
        self.calls.iter().filter(|seen| **seen == call).count()
    }
}

impl CanvasTool for Recorder {
    fn claims(&self) -> ToolClaims {
        ToolClaims::NONE.deletes()
    }

    fn press(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) -> ToolAction {
        self.calls.push(Call::Press);
        ToolAction::Claimed
    }

    fn drag(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {
        self.calls.push(Call::Drag);
    }

    fn release(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {
        self.calls.push(Call::Release);
    }

    fn cancel(&mut self, reason: CancelReason, _: &mut ToolContext<'_>) {
        self.calls.push(Call::Cancel(reason));
    }

    fn key(&mut self, _: &ToolKey, _: &mut ToolContext<'_>) -> bool {
        true
    }

    fn paint(&self, _: &Draw<'_>, _: &ToolContext<'_>) {}

    /// The primary button, and the secondary while Alt is held, as a brush takes them.
    fn takes(&self, button: PointerButton, mods: Mods) -> bool {
        button == PointerButton::Primary || (button == PointerButton::Secondary && mods.alt)
    }

    fn command_form(&self) -> ToolPath {
        ToolPath::Command("the Select Area... command")
    }
}

const BY: Vec2 = Vec2::new(60.0, 40.0);

#[trial]
fn a_drag_reaches_the_tool_as_one_press_drags_and_one_release() {
    let mut fixture = ToolFixture::new("Document", Recorder::default());
    fixture.drag(BY);
    let tool = fixture.tool();
    assert_eq!(tool.calls.first(), Some(&Call::Press), "a drag starts with the press: {:?}", tool.calls);
    assert_eq!(tool.calls.last(), Some(&Call::Release), "a drag ends with the release: {:?}", tool.calls);
    assert_eq!(tool.count(Call::Press), 1, "one press: {:?}", tool.calls);
    assert_eq!(tool.count(Call::Release), 1, "one release: {:?}", tool.calls);
    assert!(tool.count(Call::Drag) >= 1, "the moves reach the tool: {:?}", tool.calls);
    drop(tool);
    let phases = fixture.gestures();
    assert_eq!(phases.first(), Some(&GesturePhase::Begin), "{phases:?}");
    assert_eq!(phases.last(), Some(&GesturePhase::Commit), "{phases:?}");
    assert_eq!(phases.iter().filter(|phase| **phase == GesturePhase::Begin).count(), 1, "one Begin: {phases:?}");
    assert_eq!(phases.iter().filter(|phase| **phase == GesturePhase::Commit).count(), 1, "one Commit: {phases:?}");
}

#[trial]
fn escape_mid_drag_cancels_the_tool_once_and_its_release_reaches_nothing() {
    let mut fixture = ToolFixture::new("Document", Recorder::default());
    let from = fixture.canvas().rect.center();
    fixture.drag_and_escape_from(from, BY);
    let tool = fixture.tool();
    let calls = &tool.calls;
    assert_eq!(calls.first(), Some(&Call::Press), "the drag starts with the press: {calls:?}");
    assert_eq!(tool.count(Call::Cancel(CancelReason::Escape)), 1, "Escape cancels the gesture once: {calls:?}");
    assert_eq!(calls.iter().filter(|call| matches!(call, Call::Cancel(_))).count(), 1, "one cancel, with its reason: {calls:?}");
    let cancel = calls.iter().position(|call| matches!(call, Call::Cancel(_))).expect("the tool is cancelled");
    assert!(calls[cancel + 1..].is_empty(), "no drag or release reaches the tool after the cancel: {calls:?}");
    assert_eq!(tool.count(Call::Release), 0, "the release after Escape reaches no release: {calls:?}");
}

#[trial]
fn a_release_before_escape_in_one_frame_commits_the_drag() {
    let mut fixture = ToolFixture::new("Document", Recorder::default());
    let from = fixture.canvas().rect.center();
    let to = from + BY;
    fixture.press_at(from).move_to(to);
    fixture.fixture().frame(vec![pointer::release(to), key_press(Key::Escape, Modifiers::NONE)]).idle(2);
    let tool = fixture.tool();
    assert_eq!(tool.count(Call::Release), 1, "the earlier release commits: {:?}", tool.calls);
    assert_eq!(tool.count(Call::Cancel(CancelReason::Escape)), 0, "Escape does not cancel an ended drag: {:?}", tool.calls);
}

#[trial]
fn escape_before_a_release_in_one_frame_cancels_the_drag() {
    let mut fixture = ToolFixture::new("Document", Recorder::default());
    let from = fixture.canvas().rect.center();
    let to = from + BY;
    fixture.press_at(from).move_to(to);
    fixture.fixture().frame(vec![key_press(Key::Escape, Modifiers::NONE), pointer::release(to)]).idle(2);
    let tool = fixture.tool();
    assert_eq!(tool.count(Call::Cancel(CancelReason::Escape)), 1, "the earlier Escape cancels: {:?}", tool.calls);
    assert_eq!(tool.count(Call::Release), 0, "the later release does not commit: {:?}", tool.calls);
}

#[trial]
fn a_press_on_a_canvas_control_never_reaches_the_tool_and_one_away_from_it_does() {
    let crop = Rc::new(RefCell::new(craft_behavior::geometry::CropBox::upright(craft_behavior::kurbo::Rect::new(20.0, 20.0, 100.0, 80.0))));
    let shown = Rc::clone(&crop);
    let mut fixture = ToolFixture::with_body("Document", Recorder::default(), move |ui, body| {
        // The overlay takes the canvas's top left only: its whole-frame and turn parts cover nothing past it, where the
        // tool owns the press.
        let room = Rect::from_min_size(body.rect.min, Vec2::new(160.0, 120.0));
        let _ = CropOverlay::new("Crop", Bind::bound(&mut *shown.borrow_mut())).show_at(ui, room);
    });
    let corner = fixture.fixture().by_label("Crop corner, top left").expect("the crop shows its corners");
    fixture.fixture().click(&corner);
    assert!(fixture.tool().calls.is_empty(), "a click on a corner is the crop's: {:?}", fixture.tool().calls);

    let away = fixture.canvas().rect.right_bottom() - Vec2::splat(20.0);
    fixture.press_at(away).release_at(away);
    assert_eq!(fixture.tool().count(Call::Press), 1, "a press away from the corners reaches the tool: {:?}", fixture.tool().calls);
}

#[trial]
fn controls_pressing_reads_a_canvas_control_drag_not_the_tool() {
    let crop = Rc::new(RefCell::new(craft_behavior::geometry::CropBox::upright(craft_behavior::kurbo::Rect::new(20.0, 20.0, 100.0, 80.0))));
    let shown = Rc::clone(&crop);
    let mut fixture = ToolFixture::with_body("Document", Recorder::default(), move |ui, body| {
        let room = Rect::from_min_size(body.rect.min, Vec2::new(160.0, 120.0));
        let _ = CropOverlay::new("Crop", Bind::bound(&mut *shown.borrow_mut())).show_at(ui, room);
    });
    let canvas = fixture.canvas_id();
    let pressing = |fixture: &mut ToolFixture<Recorder>| ToolHost::controls_pressing(fixture.fixture().ctx(), canvas);
    let tool_held = |fixture: &mut ToolFixture<Recorder>| ToolHost::in_gesture(fixture.fixture().ctx(), canvas);

    let corner = fixture.fixture().by_label("Crop corner, top left").expect("the crop shows its corners").rect.center();
    fixture.press_at(corner).move_to(corner + Vec2::new(15.0, 10.0)).move_to(corner + Vec2::new(30.0, 20.0));
    assert!(pressing(&mut fixture), "a corner drag is a control's gesture");
    assert!(!tool_held(&mut fixture), "the tool holds no gesture under a control's drag");
    fixture.release_at(corner + Vec2::new(30.0, 20.0));
    assert!(!pressing(&mut fixture), "the release ends the control's gesture");

    let away = fixture.canvas().rect.right_bottom() - Vec2::splat(20.0);
    fixture.press_at(away).move_to(away - Vec2::new(30.0, 20.0));
    assert!(tool_held(&mut fixture), "a press away from the overlay is the tool's");
    assert!(!pressing(&mut fixture), "the tool's gesture is no control's");
    fixture.release_at(away - Vec2::new(30.0, 20.0));
}

/// A tool that claims `regions` over the canvas controls and counts the presses it gets.
#[derive(Default)]
struct Claimer {
    regions: Vec<ToolRegion>,
    presses: usize,
}

impl CanvasTool for Claimer {
    fn regions(&self) -> Vec<ToolRegion> {
        self.regions.clone()
    }

    fn press(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) -> ToolAction {
        self.presses += 1;
        ToolAction::Claimed
    }

    fn drag(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {}

    fn release(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {}

    fn cancel(&mut self, _: CancelReason, _: &mut ToolContext<'_>) {}

    fn paint(&self, _: &Draw<'_>, _: &ToolContext<'_>) {}

    fn command_form(&self) -> ToolPath {
        ToolPath::Command("the tool's form")
    }
}

#[trial]
fn a_region_the_tool_claims_takes_a_press_over_a_canvas_control_and_the_control_keeps_the_rest() {
    let start = craft_behavior::kurbo::Rect::new(100.0, 80.0, 300.0, 220.0);
    let crop = Rc::new(RefCell::new(craft_behavior::geometry::CropBox::upright(start)));
    let shown = Rc::clone(&crop);
    // The region covers the frame's top left corner, in document units.
    let corner = vec![Point::new(85.0, 65.0), Point::new(115.0, 65.0), Point::new(115.0, 95.0), Point::new(85.0, 95.0)];
    let tool = Claimer { regions: vec![ToolRegion::polygon(corner)], presses: 0 };
    let view = |rect| CanvasView::new(rect, Point::new(200.0, 150.0), 1.0);
    let mut fixture = ToolFixture::with_view("Document", tool, view, move |ui, body| {
        let _ = CropOverlay::new("Crop", Bind::bound(&mut *shown.borrow_mut())).show_at(ui, body.rect);
    });
    let by = Vec2::new(20.0, 15.0);

    let claimed = fixture.fixture().by_label("Crop corner, top left").expect("the crop shows its corners").rect.center();
    fixture.press_at(claimed).move_to(claimed + by * 0.5).move_to(claimed + by).release_at(claimed + by);
    assert_eq!(fixture.tool().presses, 1, "a press in the claimed region is the tool's background press");
    assert_eq!(crop.borrow().rect, start, "the corner under the region does not move");

    let free = fixture.fixture().by_label("Crop corner, top right").expect("the crop shows its corners").rect.center();
    fixture.press_at(free).move_to(free + by * 0.5).move_to(free + by).release_at(free + by);
    assert_eq!(fixture.tool().presses, 1, "a press outside the region is the control's: the tool gets none");
    assert_ne!(crop.borrow().rect, start, "the corner outside the region moves");
}

#[trial]
fn a_gesture_cancelled_by_the_app_starts_no_new_one_while_the_button_stays_down() {
    let mut fixture = ToolFixture::new("Document", Recorder::default());
    let from = fixture.canvas().center();
    fixture.press_at(from);
    let (ctx, id) = (fixture.fixture().ctx().clone(), fixture.canvas_id());
    assert!(ToolHost::new().cancel(&ctx, id, &mut *fixture.tool_mut(), CancelReason::Dismissed), "a gesture was running");

    fixture.move_to(from + BY).move_to(from + BY * 2.0).release_at(from + BY * 2.0);
    assert_eq!(fixture.tool().calls, [Call::Press, Call::Cancel(CancelReason::Dismissed)], "the held button is not the tool's any more");

    fixture.press_at(from).release_at(from);
    assert_eq!(fixture.tool().count(Call::Press), 2, "the next press starts a gesture again: {:?}", fixture.tool().calls);
}

#[trial]
fn a_body_hides_the_pointer_over_the_canvas_over_the_tools_cursor_while_it_asks() {
    let hide = Rc::new(Cell::new(false));
    let asks = Rc::clone(&hide);
    let room = Rc::new(Cell::new(Rect::NOTHING));
    let shown_room = Rc::clone(&room);
    let tool = RefCell::new(Recorder::default());
    let mut fixture = Fixture::new(move |ui| {
        let room = ui.available_rect_before_wrap();
        shown_room.set(room);
        let view = CanvasView::new(room, Point::new(600.0, 450.0), 1.0);
        Canvas::new("Document").document(DOCUMENT).view(view).show_at(ui, room.shrink(40.0), |ui, body| {
            if let Some(pass) = tool_pass(&body.env) {
                let _ = ToolHost::new().run(ui, &pass, &mut *tool.borrow_mut());
                if asks.get() {
                    pass.hide_pointer(ui);
                }
            }
        });
    });
    fixture.settle();
    let hidden =
        |fixture: &Fixture| craft_ui::runtime::cursor::current(fixture.ctx()) == Some(Cursor::Painted(craft_behavior::PaintedCursor::Hidden));
    let (over, beside) = (room.get().center(), room.get().min + Vec2::splat(10.0));
    fixture.frame(vec![pointer::moved(over)]);
    fixture.frame(Vec::new());
    assert!(!hidden(&fixture), "precondition: the tool's cursor shows over the canvas");
    hide.set(true);
    fixture.frame(Vec::new());
    assert!(hidden(&fixture), "the body's request hides the pointer over the canvas");
    fixture.frame(vec![pointer::moved(beside)]);
    fixture.frame(Vec::new());
    assert!(!hidden(&fixture), "beside the canvas the pointer shows");
    fixture.frame(vec![pointer::moved(over)]);
    hide.set(false);
    fixture.frame(Vec::new());
    assert!(!hidden(&fixture), "once the body stops asking, the pointer shows again");
}

/// A tool with a rubber band that follows the hovering pointer, which asks for the hover autoscroll when `scrolls`.
struct Band {
    scrolls: bool,
    hovers: usize,
}

impl CanvasTool for Band {
    fn press(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) -> ToolAction {
        ToolAction::Ignored
    }

    fn drag(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {}

    fn release(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {}

    fn hover(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {
        self.hovers += 1;
    }

    fn cancel(&mut self, _: CancelReason, _: &mut ToolContext<'_>) {}

    fn autoscrolls_on_hover(&self) -> bool {
        self.scrolls
    }

    fn paint(&self, _: &Draw<'_>, _: &ToolContext<'_>) {}

    fn command_form(&self) -> ToolPath {
        ToolPath::Freehand("the band only follows the pointer")
    }
}

type BandCanvas = (Fixture, Rc<RefCell<Band>>, Rc<Cell<Option<CanvasView>>>);

/// A canvas inset in the room, so the window has room outside it, shown as an app runs it: the document is zoomed in
/// past the canvas, and the app moves its view by the host's `scroll`. The view starts turned by `rotation` degrees.
/// Returns the fixture and the app's view.
fn scroll_canvas<T: CanvasTool + 'static>(tool: Rc<RefCell<T>>, rotation: f64) -> (Fixture, Rc<Cell<Option<CanvasView>>>) {
    let view = Rc::new(Cell::new(None::<CanvasView>));
    let shown_view = Rc::clone(&view);
    let mut fixture = Fixture::new(move |ui| {
        let room = ui.available_rect_before_wrap();
        let current = shown_view.get().unwrap_or_else(|| {
            let mut start = CanvasView::new(room, Point::new(600.0, 450.0), 4.0);
            start.rotation = rotation;
            start
        });
        let mut reply = None;
        let shown = Canvas::new("Document").document(DOCUMENT).view(current).show_at(ui, room.shrink(40.0), |ui, body| {
            if let Some(pass) = tool_pass(&body.env) {
                reply = Some(ToolHost::new().run(ui, &pass, &mut *tool.borrow_mut()));
            }
        });
        let reply = reply.unwrap_or_default();
        let mut next = reply.view.or(shown.output.view).unwrap_or(current);
        if let Some(step) = reply.scroll
            && Autoscroll::scroll(&mut next, step, DOCUMENT).is_some()
        {
            ui.ctx().request_repaint();
        }
        shown_view.set(Some(next));
    });
    fixture.settle();
    (fixture, view)
}

/// The hover autoscroll rig: a `Band` tool. Returns the fixture, the tool and the app's view.
fn band_canvas(scrolls: bool) -> BandCanvas {
    let tool = Rc::new(RefCell::new(Band { scrolls, hovers: 0 }));
    let (fixture, view) = scroll_canvas(Rc::clone(&tool), 0.0);
    (fixture, tool, view)
}

#[trial]
fn a_tool_hovering_near_the_canvas_edge_scrolls_the_view_while_the_pointer_rests() {
    let (mut fixture, tool, view) = band_canvas(true);
    let canvas = fixture.by_role(Role::CANVAS)[0].rect;
    let centre = |view: &Rc<Cell<Option<CanvasView>>>| view.get().expect("the canvas ran").center;
    let edge = |dx: f32| Pos2::new(canvas.right() + dx, canvas.center().y);

    let before = centre(&view);
    fixture.frame(vec![pointer::moved(canvas.center())]).idle(3);
    assert_eq!(centre(&view), before, "a hover away from the edge scrolls nothing");

    fixture.frame(vec![pointer::moved(edge(-4.0))]);
    let (entered, hovers) = (centre(&view), tool.borrow().hovers);
    fixture.idle(3);
    let rested = centre(&view);
    assert!(rested.x > entered.x, "resting at the right edge scrolls the view right: {entered:?} -> {rested:?}");
    assert_eq!(rested.y, entered.y, "a scroll toward the right edge keeps the height");
    assert!(tool.borrow().hovers >= hovers + 3, "the tool follows the view as it scrolls under the resting pointer");
    let in_band = rested.x - entered.x;

    // A pointer outside the canvas still hovers with its position and scrolls at full speed.
    let (mut fixture, tool, view) = band_canvas(true);
    fixture.frame(vec![pointer::moved(edge(30.0))]);
    let (entered, hovers) = (centre(&view), tool.borrow().hovers);
    fixture.idle(3);
    let outside = centre(&view).x - entered.x;
    assert!(outside > 0.0 && outside >= in_band, "outside the canvas the view scrolls at least as fast as in the band: {outside} vs {in_band}");
    assert!(tool.borrow().hovers >= hovers + 3, "the tool hovers while the pointer rests outside the canvas");

    // A button down scrolls nothing from a hover.
    let (mut fixture, _, view) = band_canvas(true);
    fixture.frame(vec![pointer::press(edge(-4.0))]);
    let entered = centre(&view);
    fixture.idle(3);
    assert_eq!(centre(&view), entered, "a pointer with a button down is no hover");

    let (mut fixture, _, view) = band_canvas(false);
    fixture.frame(vec![pointer::moved(edge(-4.0))]);
    let entered = centre(&view);
    fixture.idle(3);
    assert_eq!(centre(&view), entered, "a tool that does not autoscroll on hover leaves the view");

    a_hover_past_the_edge_scrolls_nothing_under_a_momentary_tool_a_modal_or_another_canvas();
    an_open_polygon_keeps_its_vertices_while_the_view_scrolls_under_a_resting_pointer();
}

/// A tool that keeps the points it was clicked at, as a polygon tool keeps its vertices, and autoscrolls on hover.
#[derive(Default)]
struct Lasso {
    vertices: Vec<Pos2>,
}

impl CanvasTool for Lasso {
    fn press(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) -> ToolAction {
        ToolAction::Claimed
    }

    fn drag(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {}

    fn release(&mut self, pointer: &ToolPointer, _: &mut ToolContext<'_>) {
        self.vertices.push(pointer.pos);
    }

    fn hover(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {}

    fn cancel(&mut self, _: CancelReason, _: &mut ToolContext<'_>) {}

    fn autoscrolls_on_hover(&self) -> bool {
        true
    }

    fn paint(&self, _: &Draw<'_>, _: &ToolContext<'_>) {}

    fn command_form(&self) -> ToolPath {
        ToolPath::Freehand("the lasso only keeps its clicks")
    }
}

/// What stands between a resting pointer and the hover autoscroll.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Crowd {
    Nothing,
    MomentaryHand,
    SecondCanvas,
    Modal,
}

/// A `Band` canvas inset in the room with one `Crowd` around it. Returns the fixture, the app's view and the canvas's
/// rect.
fn crowded_canvas(crowd: Crowd) -> CanvasRig<Rect> {
    let view = Rc::new(Cell::new(None::<CanvasView>));
    let canvas = Rc::new(Cell::new(Rect::NOTHING));
    let (shown_view, shown_canvas) = (Rc::clone(&view), Rc::clone(&canvas));
    let mut tool = Band { scrolls: true, hovers: 0 };
    let mut momentary = Momentary::new(NavigationOptions::default());
    let mut fixture = Fixture::new(move |ui| {
        let room = ui.available_rect_before_wrap();
        let current = shown_view.get().unwrap_or_else(|| CanvasView::new(room, Point::new(600.0, 450.0), 4.0));
        let area = room.shrink(40.0);
        let (first, second) = if crowd == Crowd::SecondCanvas {
            let middle = area.center().x;
            (
                Rect::from_min_max(area.min, Pos2::new(middle - 2.0, area.max.y)),
                Some(Rect::from_min_max(Pos2::new(middle + 2.0, area.min.y), area.max)),
            )
        } else {
            (area, None)
        };
        shown_canvas.set(first);
        let mut reply = None;
        let shown = Canvas::new("Document").document(DOCUMENT).view(current).show_at(ui, first, |ui, body| {
            if let Some(pass) = tool_pass(&body.env) {
                let mut host = ToolHost::new();
                if crowd == Crowd::MomentaryHand {
                    host = host.momentary(&mut momentary);
                }
                reply = Some(host.run(ui, &pass, &mut tool));
            }
        });
        if let Some(second) = second {
            Canvas::new("Other").document(DOCUMENT).view(CanvasView::new(second, Point::new(600.0, 450.0), 4.0)).show_at(ui, second, |_, _| {});
        }
        if crowd == Crowd::Modal {
            let mut open = true;
            Dialog::new("Over the canvas").opened(Bind::bound(&mut open)).show(ui, |ui, _| {
                Text::new("Modal").show(ui);
            });
        }
        let reply = reply.unwrap_or_default();
        let mut next = reply.view.or(shown.output.view).unwrap_or(current);
        if let Some(step) = reply.scroll
            && Autoscroll::scroll(&mut next, step, DOCUMENT).is_some()
        {
            ui.ctx().request_repaint();
        }
        shown_view.set(Some(next));
    });
    fixture.settle();
    (fixture, view, canvas)
}

fn a_hover_past_the_edge_scrolls_nothing_under_a_momentary_tool_a_modal_or_another_canvas() {
    for crowd in [Crowd::Nothing, Crowd::MomentaryHand, Crowd::SecondCanvas, Crowd::Modal] {
        let (mut fixture, view, canvas) = crowded_canvas(crowd);
        let canvas = canvas.get();
        // Over the second canvas the pointer is past the first's edge and inside the other's background.
        let at = Pos2::new(canvas.right() + if crowd == Crowd::SecondCanvas { 10.0 } else { 30.0 }, canvas.center().y);
        let mut events = vec![pointer::moved(at)];
        if crowd == Crowd::MomentaryHand {
            events.push(key_press(Key::Space, Modifiers::NONE));
        }
        fixture.frame(events);
        let entered = view.get().expect("the canvas ran").center;
        fixture.idle(3);
        let rested = view.get().expect("the canvas ran").center;
        if crowd == Crowd::Nothing {
            assert!(rested.x > entered.x, "the rig scrolls when nothing is in the way: {entered:?} -> {rested:?}");
        } else {
            assert_eq!(rested, entered, "{crowd:?}: a pointer resting past the edge scrolls nothing");
        }
    }
}

fn an_open_polygon_keeps_its_vertices_while_the_view_scrolls_under_a_resting_pointer() {
    let tool = Rc::new(RefCell::new(Lasso::default()));
    let (mut fixture, view) = scroll_canvas(Rc::clone(&tool), 0.0);
    let canvas = fixture.by_role(Role::CANVAS)[0].rect;
    let inside = [canvas.center(), canvas.center() + Vec2::new(30.0, 10.0), canvas.center() + Vec2::new(10.0, 40.0)];
    for at in inside {
        fixture.frame(vec![pointer::press(at)]).frame(vec![pointer::release(at)]);
    }
    let before = view.get().expect("the canvas ran").center;
    fixture.frame(vec![pointer::moved(Pos2::new(canvas.right() + 30.0, canvas.center().y))]).idle(4);
    let after = view.get().expect("the canvas ran").center;
    assert!(after.x > before.x, "the view scrolls while the polygon is open: {before:?} -> {after:?}");
    assert_eq!(tool.borrow().vertices, inside, "the vertices stay where they were clicked");
}

/// A fixture with the view the canvas reported and the last value the body saw.
type CanvasRig<Seen> = (Fixture, Rc<Cell<Option<CanvasView>>>, Rc<Cell<Seen>>);

/// `CanvasRig` with the context requests the canvas raised.
type MenuRig = (Fixture, Rc<RefCell<Vec<ContextRequest>>>, Rc<Cell<Option<CanvasView>>>, Rc<Cell<Rect>>);

type Log = Rc<RefCell<Vec<String>>>;

/// A tool that logs its calls under its name. It refuses a press while its `refuses` key is held, answers it `Ignored`
/// while its `ignores` key is held, and takes the secondary button when `secondary`.
struct Probe {
    name: &'static str,
    refuses: Option<StandInKey>,
    ignores: Option<StandInKey>,
    secondary: bool,
    every_move: bool,
    snaps: bool,
    drags: Vec<(Pos2, f64)>,
    /// Whether Space was held at each drag.
    space_drags: Vec<bool>,
    pass_time: f64,
    log: Log,
}

impl Probe {
    fn new(name: &'static str, log: &Log) -> Self {
        Probe {
            name,
            refuses: None,
            ignores: None,
            secondary: false,
            every_move: false,
            snaps: false,
            drags: Vec::new(),
            space_drags: Vec::new(),
            pass_time: 0.0,
            log: Rc::clone(log),
        }
    }

    fn note(&self, call: &str) {
        self.log.borrow_mut().push(format!("{} {call}", self.name));
    }
}

fn holds(key: Option<StandInKey>, mods: Mods) -> bool {
    match key {
        Some(StandInKey::Command) => mods.cmd,
        Some(StandInKey::Alt) => mods.alt,
        _ => false,
    }
}

impl CanvasTool for Probe {
    fn each_pass(&mut self, cx: &mut ToolContext<'_>) {
        self.pass_time = cx.time;
        if self.snaps {
            let _ = cx.snap_rect(craft_behavior::kurbo::Rect::new(98.0, 40.0, 198.0, 90.0));
        }
    }

    fn press(&mut self, pointer: &ToolPointer, _: &mut ToolContext<'_>) -> ToolAction {
        self.note("press");
        if holds(self.ignores, pointer.mods) { ToolAction::Ignored } else { ToolAction::Claimed }
    }

    fn drag(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        self.note("drag");
        self.drags.push((pointer.pos, pointer.time));
        self.space_drags.push(cx.held(craft_keymap::Key::Space));
    }

    fn release(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {
        self.note("release");
    }

    fn hover(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {
        self.note("hover");
    }

    fn cancel(&mut self, _: CancelReason, _: &mut ToolContext<'_>) {
        self.note("cancel");
    }

    fn takes(&self, button: PointerButton, mods: Mods) -> bool {
        (button == PointerButton::Primary || (self.secondary && button == PointerButton::Secondary)) && !holds(self.refuses, mods)
    }

    fn takes_every_move(&self) -> bool {
        self.every_move
    }

    fn paint(&self, _: &Draw<'_>, _: &ToolContext<'_>) {}

    fn cursor(&self, _: &ToolContext<'_>) -> Cursor {
        Cursor::System(match self.name {
            "command" => CursorIcon::Crosshair,
            "alt" => CursorIcon::Wait,
            _ => CursorIcon::Default,
        })
    }

    fn command_form(&self) -> ToolPath {
        ToolPath::Freehand("the probe only logs")
    }
}

/// What a `Rig` adds to the host: the momentary tools, a colour for smart guides.
#[derive(Default)]
struct Setup {
    momentary: Option<Momentary>,
    smart_color: Option<DataColor>,
    /// Set by a trial: the next pass is discarded after the host ran, so egui runs it again in the same frame.
    rerun: Rc<Cell<bool>>,
}

/// A canvas at 100 % with a `Probe` as the selected tool and two stand-in probes, `command` and `alt`.
struct Rig {
    fixture: Fixture,
    main: Rc<RefCell<Probe>>,
    canvas: Rc<Cell<Id>>,
    log: Log,
    contexts: Rc<RefCell<Vec<ContextRequest>>>,
    snapped: Rc<RefCell<Vec<SnapLine>>>,
    cursor: Rc<RefCell<Option<Cursor>>>,
    command: Rc<RefCell<Probe>>,
    alt: Rc<RefCell<Probe>>,
}

impl Rig {
    fn new(main: Probe, log: &Log, snap: Option<Snap>) -> Self {
        Self::with(main, log, snap, Setup::default())
    }

    fn with(main: Probe, log: &Log, snap: Option<Snap>, mut setup: Setup) -> Self {
        let main = Rc::new(RefCell::new(main));
        let (canvas, contexts, snapped) = (Rc::new(Cell::new(Id::NULL)), Rc::new(RefCell::new(Vec::new())), Rc::new(RefCell::new(Vec::new())));
        let cursor = Rc::new(RefCell::new(None));
        let (shown_main, shown_canvas, shown_contexts, shown_snapped, shown_cursor) =
            (Rc::clone(&main), Rc::clone(&canvas), Rc::clone(&contexts), Rc::clone(&snapped), Rc::clone(&cursor));
        let (command, alt) = (Rc::new(RefCell::new(Probe::new("command", log))), Rc::new(RefCell::new(Probe::new("alt", log))));
        let (shown_command, shown_alt) = (Rc::clone(&command), Rc::clone(&alt));
        let mut fixture = Fixture::new(move |ui| {
            let room = ui.available_rect_before_wrap();
            let mut shown = Canvas::new("Document").document(DOCUMENT).view(CanvasView::new(room, Point::new(600.0, 450.0), 1.0));
            if let Some(snap) = &snap {
                shown = shown.snap(snap.clone());
            }
            let mut reply = None;
            let shown = shown.show(ui, |ui, body| {
                if let Some(pass) = tool_pass(&body.env) {
                    let (mut command, mut alt) = (shown_command.borrow_mut(), shown_alt.borrow_mut());
                    let mut host = ToolHost::new()
                        .stand_in(StandInKey::Command, &mut *command)
                        .stand_in(StandInKey::Alt, &mut *alt)
                        .smart_guide_color(setup.smart_color);
                    if let Some(momentary) = setup.momentary.as_mut() {
                        host = host.momentary(momentary);
                    }
                    reply = Some(host.run(ui, &pass, &mut *shown_main.borrow_mut()));
                }
            });
            shown_canvas.set(shown.output.response.id);
            let reply = reply.unwrap_or_default();
            shown_contexts.borrow_mut().extend(reply.context);
            shown_snapped.borrow_mut().extend(reply.snapped.iter().copied());
            if setup.rerun.replace(false) && frame::discardable(ui.ctx()) {
                frame::discard_pass(ui.ctx(), "a trial reruns the pass");
            }
            if reply.cursor.is_some() {
                *shown_cursor.borrow_mut() = reply.cursor.clone();
            }
        });
        fixture.settle();
        Rig { fixture, main, command, alt, canvas, log: Rc::clone(log), contexts, snapped, cursor }
    }

    fn centre(&mut self) -> Pos2 {
        self.fixture.by_role(Role::CANVAS)[0].rect.center()
    }

    fn calls(&self) -> Vec<String> {
        self.log.borrow().clone()
    }
}

#[trial]
fn a_held_command_or_alt_runs_its_stand_in_for_one_gesture() {
    let log = Log::default();
    let mut main = Probe::new("main", &log);
    main.refuses = Some(StandInKey::Command);
    let mut rig = Rig::new(main, &log, None);
    let at = rig.centre();

    // Command at the press, up mid-drag: the stand-in keeps the gesture to the release.
    rig.fixture.hold(Modifiers::COMMAND).frame_held(vec![pointer::press(at)]).frame(vec![pointer::moved(at + BY)]);
    rig.fixture.hold(Modifiers::NONE).frame(vec![pointer::moved(at + BY * 2.0)]).frame(vec![pointer::release(at + BY * 2.0)]);
    let calls = rig.calls();
    assert_eq!(calls.first().map(String::as_str), Some("command press"), "{calls:?}");
    assert_eq!(calls.last().map(String::as_str), Some("command release"), "{calls:?}");
    assert!(calls.iter().all(|call| call.starts_with("command")), "the selected tool saw none of it: {calls:?}");

    // The next plain press is the selected tool's again.
    rig.log.borrow_mut().clear();
    rig.fixture.frame(vec![pointer::press(at)]).frame(vec![pointer::release(at)]);
    assert_eq!(rig.calls().first().map(String::as_str), Some("main press"), "{:?}", rig.calls());

    // With the key held and no gesture, hover is the stand-in's.
    rig.log.borrow_mut().clear();
    rig.fixture.hold(Modifiers::COMMAND).frame(vec![pointer::moved(at + BY)]);
    assert!(rig.calls().iter().any(|call| call == "command hover"), "{:?}", rig.calls());
    rig.fixture.hold(Modifiers::NONE);

    // A press injected through the host is substituted from its own modifiers.
    rig.log.borrow_mut().clear();
    let (ctx, canvas) = (rig.fixture.ctx().clone(), rig.canvas.get());
    ToolHost::inject(&ctx, canvas, ToolInput::down(PointerButton::Primary, at).mods(Mods::CMD));
    ToolHost::inject(&ctx, canvas, ToolInput::up(at));
    rig.fixture.run_injected();
    // The press's move to `at` hovers first, with Command held.
    let calls: Vec<String> = rig.calls().into_iter().filter(|call| !call.ends_with("hover")).collect();
    assert_eq!(calls.first().map(String::as_str), Some("command press"), "{calls:?}");
    assert_eq!(calls.last().map(String::as_str), Some("command release"), "{calls:?}");

    // A selection tool's Ignored Command press reaches the stand-in.
    let log = Log::default();
    let mut main = Probe::new("main", &log);
    main.ignores = Some(StandInKey::Command);
    let mut rig = Rig::new(main, &log, None);
    let at = rig.centre();
    rig.fixture
        .hold(Modifiers::COMMAND)
        .frame_held(vec![pointer::press(at)])
        .frame(vec![pointer::moved(at + BY)])
        .frame(vec![pointer::release(at + BY)]);
    let calls = rig.calls();
    assert_eq!(&calls[..2], ["main press", "command press"], "{calls:?}");
    assert_eq!(calls.last().map(String::as_str), Some("command release"), "{calls:?}");

    // Alt runs the Alt stand-in.
    let log = Log::default();
    let mut main = Probe::new("main", &log);
    main.refuses = Some(StandInKey::Alt);
    let mut rig = Rig::new(main, &log, None);
    let at = rig.centre();
    rig.fixture.hold(Modifiers::ALT).frame_held(vec![pointer::press(at)]).frame(vec![pointer::release(at)]);
    assert_eq!(rig.calls().first().map(String::as_str), Some("alt press"), "{:?}", rig.calls());

    // The hover cursor is the stand-in's while its key is held, no gesture runs and the selected tool does not take
    // the press.
    let log = Log::default();
    let mut main = Probe::new("main", &log);
    main.refuses = Some(StandInKey::Command);
    let mut rig = Rig::new(main, &log, None);
    let at = rig.centre();
    rig.fixture.hold(Modifiers::COMMAND).frame(vec![pointer::moved(at + BY)]);
    assert_eq!(*rig.cursor.borrow(), Some(Cursor::System(CursorIcon::Crosshair)), "the Command stand-in's cursor");
    rig.fixture.hold(Modifiers::NONE).frame(vec![pointer::moved(at + BY * 2.0)]);
    assert_eq!(*rig.cursor.borrow(), Some(Cursor::System(CursorIcon::Default)), "the selected tool's cursor again");

    // Space's momentary Hand wins over both stand-ins: Command held with Space is the momentary tool's press.
    let log = Log::default();
    let mut main = Probe::new("main", &log);
    main.refuses = Some(StandInKey::Command);
    let mut rig = Rig::with(main, &log, None, Setup { momentary: Some(Momentary::new(NavigationOptions::default())), ..Setup::default() });
    let at = rig.centre();
    rig.fixture.hold(Modifiers::COMMAND).frame(vec![pointer::moved(at), key_press(Key::Space, Modifiers::COMMAND)]);
    rig.fixture.frame_held(vec![pointer::press(at)]).frame(vec![pointer::moved(at + BY)]).frame(vec![pointer::release(at + BY)]);
    assert!(rig.calls().is_empty(), "neither the selected tool nor a stand-in sees a press under a momentary tool: {:?}", rig.calls());

    // With the Hand selected, Cmd at a press is Zoom In, not the Command stand-in.
    let log = Log::default();
    let mut main = Probe::new("main", &log);
    main.refuses = Some(StandInKey::Command);
    let mut momentary = Momentary::new(NavigationOptions::default());
    momentary.hand_selected(true);
    let mut rig = Rig::with(main, &log, None, Setup { momentary: Some(momentary), ..Setup::default() });
    let at = rig.centre();
    rig.fixture.hold(Modifiers::COMMAND).frame(vec![pointer::moved(at)]).frame_held(vec![pointer::press(at)]).frame(vec![pointer::release(at)]);
    assert!(!rig.calls().iter().any(|call| call == "command press"), "Cmd over the selected Hand is Zoom In: {:?}", rig.calls());
}

/// The app's cancel ends a gesture a stand-in holds on that stand-in: the host built with the pass's stand-ins picks the
/// holder, so the selected tool is not cancelled for a gesture it never had.
#[trial]
fn a_gesture_a_stand_in_holds_is_cancelled_on_the_stand_in() {
    let log = Log::default();
    let mut main = Probe::new("main", &log);
    main.refuses = Some(StandInKey::Command);
    let mut rig = Rig::new(main, &log, None);
    let at = rig.centre();
    rig.fixture.hold(Modifiers::COMMAND).frame_held(vec![pointer::press(at)]).frame(vec![pointer::moved(at + BY)]);
    rig.log.borrow_mut().clear();
    let (ctx, canvas) = (rig.fixture.ctx().clone(), rig.canvas.get());
    let cancelled = {
        let (mut command, mut alt, mut main) = (rig.command.borrow_mut(), rig.alt.borrow_mut(), rig.main.borrow_mut());
        ToolHost::new().stand_in(StandInKey::Command, &mut *command).stand_in(StandInKey::Alt, &mut *alt).cancel(
            &ctx,
            canvas,
            &mut *main,
            CancelReason::HostChanged,
        )
    };
    assert!(cancelled, "the Command stand-in's gesture was running");
    assert_eq!(rig.calls(), ["command cancel"], "the stand-in that holds the gesture is cancelled, the selected tool is not");
}

/// A key injected with the pointer events is down for exactly the events injected after it until it is let go.
#[trial]
fn a_queued_space_covers_the_moves_queued_after_it_and_not_those_after_its_release() {
    let log = Log::default();
    let mut rig = Rig::new(Probe::new("main", &log), &log, None);
    let at = rig.centre();
    let (ctx, canvas) = (rig.fixture.ctx().clone(), rig.canvas.get());
    let space = craft_keymap::Key::Space;
    // Short steps keep every move clear of the edge band, where a resting pointer gets a drag in each pass.
    let step = BY / 2.0;
    for input in [
        ToolInput::down(PointerButton::Primary, at),
        ToolInput::moved(at + step),
        ToolInput::hold(space),
        ToolInput::moved(at + step * 2.0),
        ToolInput::moved(at + step * 3.0),
        ToolInput::let_go(space),
        ToolInput::moved(at + step * 4.0),
        ToolInput::up(at + step * 4.0),
    ] {
        ToolHost::inject(&ctx, canvas, input);
    }
    rig.fixture.run_injected();
    assert_eq!(rig.main.borrow().space_drags, [false, true, true, false], "Space is held for the two moves queued while it is down");
}

/// A held key whose release never comes ends where the window host drops injected input: after a cancel, the drag of
/// a later gesture runs without it.
#[trial]
fn a_held_space_ends_when_the_injected_input_is_cancelled() {
    let log = Log::default();
    let mut rig = Rig::new(Probe::new("main", &log), &log, None);
    let at = rig.centre();
    let (ctx, canvas) = (rig.fixture.ctx().clone(), rig.canvas.get());
    for input in [ToolInput::hold(craft_keymap::Key::Space), ToolInput::down(PointerButton::Primary, at), ToolInput::moved(at + BY)] {
        ToolHost::inject(&ctx, canvas, input);
    }
    rig.fixture.run_injected();
    WindowHost::of(&ctx).cancel_injected();
    for input in [ToolInput::up(at + BY), ToolInput::down(PointerButton::Primary, at), ToolInput::moved(at + BY), ToolInput::up(at + BY)] {
        ToolHost::inject(&ctx, canvas, input);
    }
    rig.fixture.run_injected();
    assert_eq!(rig.main.borrow().space_drags, [true, false], "Space is held for the first drag and not after the cancel");
}

/// A tool that records the pen pressure of its press and drags, and the click count of each press.
#[derive(Default)]
struct PenProbe {
    pressures: Vec<f32>,
    kinds: Vec<PointerKind>,
    clicks: Vec<u32>,
}

impl CanvasTool for PenProbe {
    fn press(&mut self, pointer: &ToolPointer, _: &mut ToolContext<'_>) -> ToolAction {
        self.clicks.push(pointer.clicks);
        self.pressures.push(pointer.sample.pressure);
        self.kinds.push(pointer.kind);
        ToolAction::Claimed
    }

    fn drag(&mut self, pointer: &ToolPointer, _: &mut ToolContext<'_>) {
        self.pressures.push(pointer.sample.pressure);
        self.kinds.push(pointer.kind);
    }

    fn release(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {}

    fn cancel(&mut self, _: CancelReason, _: &mut ToolContext<'_>) {}

    fn paint(&self, _: &Draw<'_>, _: &ToolContext<'_>) {}

    fn command_form(&self) -> ToolPath {
        ToolPath::Freehand("the probe only records")
    }
}

/// A canvas showing `probe` as its tool; the canvas's id is in the returned cell after the first pass.
fn probe_fixture(probe: &Rc<RefCell<PenProbe>>) -> (Fixture, Rc<Cell<Id>>) {
    let (shown_probe, canvas) = (Rc::clone(probe), Rc::new(Cell::new(Id::NULL)));
    let shown_canvas = Rc::clone(&canvas);
    let mut fixture = Fixture::new(move |ui| {
        let room = ui.available_rect_before_wrap();
        let shown = Canvas::new("Document").document(DOCUMENT).view(CanvasView::new(room, Point::new(600.0, 450.0), 1.0)).show(ui, |ui, body| {
            if let Some(pass) = tool_pass(&body.env) {
                let _ = ToolHost::new().run(ui, &pass, &mut *shown_probe.borrow_mut());
            }
        });
        shown_canvas.set(shown.output.response.id);
    });
    fixture.settle();
    (fixture, canvas)
}

/// Injected clicks in a row reach the tool as one press each, counted as a double and then a triple click.
#[trial]
fn injected_clicks_reach_the_tool_as_a_multi_click() {
    let probe = Rc::new(RefCell::new(PenProbe::default()));
    let (mut fixture, canvas) = probe_fixture(&probe);
    let at = fixture.by_role(Role::CANVAS)[0].rect.center();
    ToolHost::inject(&fixture.ctx().clone(), canvas.get(), ToolInput::clicks(PointerButton::Primary, at, 3));
    fixture.run_injected();
    let clicks = probe.borrow().clicks.clone();
    assert!(clicks.len() == 3 && clicks.windows(2).all(|pair| pair[1] > pair[0]), "each press counts one more: {clicks:?}");
    assert_eq!(clicks.last(), Some(&3), "the third press is a triple click");
}

/// Two clicks whose presses and releases all came in one frame (a stalled window delivers a double click at once)
/// reach the tool as two presses in the frames the runtime asks for (one press per pass), the second counted as a
/// double click.
#[trial]
fn two_clicks_in_one_frame_reach_the_tool_as_a_double_click() {
    let probe = Rc::new(RefCell::new(PenProbe::default()));
    let (mut fixture, _) = probe_fixture(&probe);
    let at = fixture.by_role(Role::CANVAS)[0].rect.center();
    fixture.frame(vec![pointer::moved(at)]);
    fixture.frame(vec![pointer::press(at), pointer::release(at), pointer::press(at), pointer::release(at)]).settle();
    let clicks = probe.borrow().clicks.clone();
    assert!(clicks.len() == 2 && clicks[1] > clicks[0], "each click reaches the tool, the second counted on: {clicks:?}");
}

/// An injected pen reading reaches the tool as a tablet's does: the press and the drag carry the pressure their steps
/// were injected with, and the pointer kind of a pen.
#[trial]
fn an_injected_pen_reading_reaches_the_tool() {
    let probe = Rc::new(RefCell::new(PenProbe::default()));
    let (mut fixture, canvas) = probe_fixture(&probe);
    let at = fixture.by_role(Role::CANVAS)[0].rect.center();
    let ctx = fixture.ctx().clone();
    for input in [
        ToolInput::down(PointerButton::Primary, at).sample(PointerSample::new(0.3)),
        ToolInput::moved(at + BY).sample(PointerSample::new(0.7)),
        ToolInput::up(at + BY),
    ] {
        ToolHost::inject(&ctx, canvas.get(), input);
    }
    fixture.run_injected();
    let pressures = probe.borrow().pressures.clone();
    assert!(pressures.len() == 2 && (pressures[0] - 0.3).abs() < 1e-4 && (pressures[1] - 0.7).abs() < 1e-4, "{pressures:?}");
    let kinds = probe.borrow().kinds.clone();
    assert_eq!(kinds, [PointerKind::Pen, PointerKind::Pen], "the press and the drag of a pen reach the tool as a pen's");
}

/// A pass whose only news is a key or a modifier change hovers a still pointer and drags a still press once, in the
/// frame's first pass; the rerun pass of the same frame, which has no input, does neither.
#[trial]
fn a_pass_with_only_a_key_or_modifier_change_re_hovers_a_still_pointer_and_re_drags_a_still_press_once() {
    let log = Log::default();
    let rerun = Rc::new(Cell::new(false));
    let mut rig = Rig::with(Probe::new("main", &log), &log, None, Setup { rerun: Rc::clone(&rerun), ..Setup::default() });
    let at = rig.centre();
    let count = |rig: &Rig, call: &str| rig.calls().iter().filter(|logged| logged.as_str() == call).count();

    rig.fixture.frame(vec![pointer::moved(at)]).idle(2);
    rig.log.borrow_mut().clear();
    rig.fixture.idle(1);
    assert_eq!(count(&rig, "main hover"), 0, "a still pointer is not hovered again by a pass with no news: {:?}", rig.calls());

    rerun.set(true);
    rig.fixture.frame(vec![key_press(Key::A, Modifiers::NONE)]);
    assert_eq!(count(&rig, "main hover"), 1, "a key press hovers the still pointer once, the rerun pass not again: {:?}", rig.calls());

    rig.log.borrow_mut().clear();
    rerun.set(true);
    rig.fixture.hold(Modifiers::SHIFT).frame(vec![Event::ModifiersChanged(Modifiers::SHIFT)]);
    assert_eq!(count(&rig, "main hover"), 1, "Shift going down hovers the still pointer once: {:?}", rig.calls());
    rig.fixture.hold(Modifiers::NONE).frame(vec![Event::ModifiersChanged(Modifiers::NONE)]);

    rig.fixture.frame(vec![pointer::press(at)]).frame(vec![pointer::moved(at + BY)]).idle(1);
    rig.log.borrow_mut().clear();
    rig.fixture.idle(1);
    assert_eq!(count(&rig, "main drag"), 0, "a still press is not dragged by a pass with no news: {:?}", rig.calls());
    rerun.set(true);
    rig.fixture.frame(vec![key_press(Key::B, Modifiers::NONE)]);
    assert_eq!(count(&rig, "main drag"), 1, "a key press drags the still press once, the rerun pass not again: {:?}", rig.calls());
    rig.fixture.frame(vec![pointer::release(at + BY)]);
}

/// What the drag session of `DropperTool` carries.
#[derive(Clone, Debug, PartialEq)]
struct Parcel(u32);

/// Takes every `Parcel` dropped on it.
struct TakesParcel;

impl DropTarget<Parcel> for TakesParcel {
    type Drop = u32;

    fn accept(&self, payload: &Parcel, _: Pos2, _: craft_behavior::DropOperation) -> Option<u32> {
        Some(payload.0)
    }

    fn mark(&self, _: &u32) -> DropMark {
        DropMark::region(TARGET)
    }
}

/// Where the drop target lies in the fixture's room: the room's top left corner.
const TARGET: Rect = Rect { min: Pos2::new(0.0, 0.0), max: Pos2::new(100.0, 50.0) };

/// A drop target for `Parcel`s that records what it took.
#[derive(Component)]
struct DropSlot<'a> {
    #[common]
    common: Common,
    taken: &'a RefCell<Vec<u32>>,
}

impl Component for DropSlot<'_> {
    const ROLE: Role = Role::GROUP;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: DropSlotStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        TARGET.size()
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        if let Some(parcel) = cx.drop_target(&TakesParcel) {
            self.taken.borrow_mut().push(parcel);
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

/// A tool that hands a `Parcel` to a drag session in every drag and notes whether a target would take it.
struct DropperTool {
    seen: Rc<RefCell<Vec<(&'static str, bool)>>>,
}

impl CanvasTool for DropperTool {
    fn press(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) -> ToolAction {
        ToolAction::Claimed
    }

    fn drag(&mut self, _: &ToolPointer, cx: &mut ToolContext<'_>) {
        cx.start_drag(Parcel(7), DragPreview::None);
        self.seen.borrow_mut().push(("drag", cx.over_drop_target()));
    }

    fn release(&mut self, _: &ToolPointer, cx: &mut ToolContext<'_>) {
        self.seen.borrow_mut().push(("release", cx.over_drop_target()));
    }

    fn cancel(&mut self, _: CancelReason, _: &mut ToolContext<'_>) {}

    fn paint(&self, _: &Draw<'_>, _: &ToolContext<'_>) {}

    fn command_form(&self) -> ToolPath {
        ToolPath::Freehand("the probe only records")
    }
}

/// A tool's `start_drag` starts one session under the canvas, however many `drag` calls repeat it; a target takes the
/// drop, `over_drop_target` is true in `release` over the target and false off it, and the session ends with the
/// gesture.
#[trial]
fn a_tools_start_drag_session_reaches_a_target_and_the_tool_reads_over_drop_target_at_its_release() {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let taken = Rc::new(RefCell::new(Vec::new()));
    let tool = Rc::new(RefCell::new(DropperTool { seen: Rc::clone(&seen) }));
    let (shown_tool, shown_taken) = (Rc::clone(&tool), Rc::clone(&taken));
    let canvas = Rc::new(Cell::new(Id::NULL));
    let shown_canvas = Rc::clone(&canvas);
    let mut fixture = Fixture::new(move |ui| {
        let room = ui.available_rect_before_wrap();
        let shown = Canvas::new("Document").document(DOCUMENT).view(CanvasView::new(room, Point::new(600.0, 450.0), 1.0)).show(ui, |ui, body| {
            if let Some(pass) = tool_pass(&body.env) {
                let _ = ToolHost::new().run(ui, &pass, &mut *shown_tool.borrow_mut());
            }
        });
        shown_canvas.set(shown.output.response.id);
        DropSlot { common: Common::new(), taken: &shown_taken }.id_salt("slot").show_at(ui, TARGET.translate(room.min.to_vec2()));
    });
    fixture.settle();
    let room = fixture.by_role(Role::CANVAS)[0].rect;
    let (at, slot) = (room.center(), TARGET.translate(room.min.to_vec2()).center());
    let ctx = fixture.ctx().clone();
    let before = drag::started(&ctx);

    fixture.frame(vec![pointer::moved(at)]).frame(vec![pointer::press(at)]);
    for step in 1..=4 {
        fixture.frame(vec![pointer::moved(at.lerp(slot, step as f32 / 4.0))]);
    }
    fixture.idle(1);
    assert_eq!(drag::started(&ctx) - before, 1, "several drag calls start one session");
    assert_eq!(seen.borrow().first(), Some(&("drag", false)), "no target takes the drag before the session runs: {:?}", seen.borrow());
    fixture.frame(vec![pointer::release(slot)]).idle(1);
    assert_eq!(seen.borrow().last(), Some(&("release", true)), "the release over the target reads it: {:?}", seen.borrow());
    assert_eq!(taken.borrow().clone(), vec![7], "the target took the drop");
    assert!(!ToolHost::in_gesture(&ctx, canvas.get()) && drag::mark(&ctx).is_none(), "the session ends with the gesture");

    let away = at + BY;
    fixture.frame(vec![pointer::moved(at)]).frame(vec![pointer::press(at)]).frame(vec![pointer::moved(away)]).idle(1);
    fixture.frame(vec![pointer::release(away)]).idle(1);
    assert_eq!(seen.borrow().last(), Some(&("release", false)), "a release off the target reads none: {:?}", seen.borrow());
    assert_eq!(taken.borrow().clone(), vec![7], "a release off the target drops nothing");
}

/// A tool that asks for a repaint in every drag, as a brush that previews its stroke does.
#[derive(Default)]
struct Painter {
    drags: usize,
}

impl CanvasTool for Painter {
    fn press(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) -> ToolAction {
        ToolAction::Claimed
    }

    fn drag(&mut self, _: &ToolPointer, cx: &mut ToolContext<'_>) {
        self.drags += 1;
        cx.request_repaint();
    }

    fn release(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {}

    fn cancel(&mut self, _: CancelReason, _: &mut ToolContext<'_>) {}

    fn paint(&self, _: &Draw<'_>, _: &ToolContext<'_>) {}

    fn command_form(&self) -> ToolPath {
        ToolPath::Freehand("the painter only asks for repaints")
    }
}

/// A panel beside the canvas, showing its name.
#[derive(MemoProps)]
struct SidePanel {
    name: &'static str,
}

fn side_panel(props: &mut SidePanel, ui: &mut egui::Ui) {
    Text::new(props.name).show(ui);
}

/// A stroke on the canvas, whose tool asks for a repaint in every drag, runs neither of the panels beside it (each a
/// memoized view), from its press to its release, and each of its move frames runs one pass.
#[trial]
fn a_stroke_runs_no_settled_panel_beside_the_canvas_and_one_pass_per_move() {
    let painter = Rc::new(RefCell::new(Painter::default()));
    let canvas = Rc::new(Cell::new(Id::NULL));
    let (shown_painter, shown_canvas) = (Rc::clone(&painter), Rc::clone(&canvas));
    let mut fixture = Fixture::new(move |ui| {
        let room = ui.available_rect_before_wrap();
        let (side, document) = room.split_left_right_at_x(room.left() + 160.0);
        in_region(ui, side, |ui| {
            for name in ["Layers", "Navigator"] {
                MemoView::new(name, SidePanel { name }, side_panel).show(ui);
            }
        });
        in_region(ui, document, |ui| {
            let view = CanvasView::new(document, Point::new(600.0, 450.0), 1.0);
            let shown = Canvas::new("Document").document(DOCUMENT).view(view).show(ui, |ui, body| {
                if let Some(pass) = tool_pass(&body.env) {
                    let _ = ToolHost::new().run(ui, &pass, &mut *shown_painter.borrow_mut());
                }
            });
            shown_canvas.set(shown.output.response.id);
        });
    });
    fixture.settle();
    let at = fixture.by_role(Role::CANVAS)[0].rect.center();
    let ctx = fixture.ctx().clone();
    ToolHost::inject(&ctx, canvas.get(), ToolInput::down(PointerButton::Primary, at));
    for step in 1..=6 {
        ToolHost::inject(&ctx, canvas.get(), ToolInput::moved(at + Vec2::new(8.0 * step as f32, 4.0 * step as f32)));
    }
    ToolHost::inject(&ctx, canvas.get(), ToolInput::up(at + Vec2::new(48.0, 24.0)));
    let host = WindowHost::of(&ctx);
    let mut frames = Vec::new();
    while !host.injected_idle() && frames.len() < 64 {
        fixture.frame(Vec::new());
        let runs = last_frame_memo_runs(fixture.ctx()).iter().filter(|(_, _, run)| *run == MemoRun::Ran).count();
        frames.push((fixture.passes(), runs));
    }
    assert!(painter.borrow().drags > 0, "the moves reach the tool, which asks for repaints");
    assert!(frames.iter().all(|(_, runs)| *runs == 0), "no panel runs during the stroke: {frames:?}");
    let moves = &frames[1..frames.len() - 1];
    assert!(moves.iter().all(|(passes, _)| *passes == 1), "each move frame runs one pass: {frames:?}");
}

/// A queued Space starts the momentary Hand as the real key does: the press it is held over pans the view, and the
/// press after its release runs the selected tool.
/// A canvas with the momentary set and `main` as the selected tool, zoomed in, so the document is larger than the
/// canvas and the Hand has room to pan: returns the fixture, the app's view and the canvas's id.
fn momentary_canvas(main: Probe) -> CanvasRig<Id> {
    momentary_canvas_with(main, |_, _| {})
}

/// `momentary_canvas` with `body` as the canvas's body.
fn momentary_canvas_with(main: Probe, body: impl FnMut(&mut egui::Ui, &mut craft_ui::contract::BodyCx<'_>) + 'static) -> CanvasRig<Id> {
    momentary_canvas_under(NavigationOptions::default(), Rc::default(), main, body)
}

/// `momentary_canvas_with` under the navigation choices `options`, its host told in each pass whether the pointer
/// `navigates`.
fn momentary_canvas_under(
    options: NavigationOptions,
    navigates: Rc<Cell<bool>>,
    main: Probe,
    mut body: impl FnMut(&mut egui::Ui, &mut craft_ui::contract::BodyCx<'_>) + 'static,
) -> CanvasRig<Id> {
    let main = RefCell::new(main);
    let (view, canvas) = (Rc::new(Cell::new(None::<CanvasView>)), Rc::new(Cell::new(Id::NULL)));
    let (shown_view, shown_canvas) = (Rc::clone(&view), Rc::clone(&canvas));
    let mut set = Momentary::new(options);
    let mut fixture = Fixture::new(move |ui| {
        let room = ui.available_rect_before_wrap();
        let current = shown_view.get().unwrap_or_else(|| CanvasView::new(room, Point::new(600.0, 450.0), 4.0));
        let mut reply = None;
        let shown = Canvas::new("Document").document(DOCUMENT).view(current).show(ui, |ui, cx| {
            body(ui, cx);
            if let Some(pass) = tool_pass(&cx.env) {
                reply = Some(ToolHost::new().momentary(&mut set).navigates(navigates.get()).run(ui, &pass, &mut *main.borrow_mut()));
            }
        });
        shown_canvas.set(shown.output.response.id);
        let reply = reply.unwrap_or_default();
        shown_view.set(Some(reply.view.or(shown.output.view).unwrap_or(current)));
    });
    fixture.settle();
    (fixture, view, canvas)
}

#[trial]
fn a_queued_space_held_over_a_press_pans_with_the_hand_and_the_press_after_its_release_runs_the_selected_tool() {
    let log = Log::default();
    let (mut fixture, view, canvas) = momentary_canvas(Probe::new("main", &log));
    let at = fixture.by_role(Role::CANVAS)[0].rect.center();
    let before = view.get().expect("the canvas ran").center;
    let space = craft_keymap::Key::Space;
    for input in [
        ToolInput::hold(space),
        ToolInput::down(PointerButton::Primary, at),
        ToolInput::moved(at + BY),
        ToolInput::up(at + BY),
        ToolInput::let_go(space),
        ToolInput::down(PointerButton::Primary, at),
        ToolInput::up(at),
    ] {
        ToolHost::inject(fixture.ctx(), canvas.get(), input);
    }
    fixture.run_injected();
    assert_ne!(view.get().expect("the canvas ran").center, before, "the press under the queued Space pans with the Hand");
    let calls = log.borrow().clone();
    assert_eq!(calls.iter().filter(|call| *call == "main press").count(), 1, "only the press after the release is the selected tool's: {calls:?}");
    assert!(!calls.iter().any(|call| call == "main drag"), "the Hand's drag never reaches the selected tool: {calls:?}");
}

/// Palm rejection: a press the host says navigates (a finger once a pen has been seen) pans with the Hand to its
/// release, even once the host stops saying so mid-drag, and never reaches the selected tool; the next press that does
/// not navigate is the tool's.
#[trial]
fn a_press_the_host_says_navigates_pans_with_the_hand_to_its_release() {
    let log = Log::default();
    let navigates = Rc::new(Cell::new(true));
    let (mut fixture, view, _) = momentary_canvas_under(NavigationOptions::default(), Rc::clone(&navigates), Probe::new("main", &log), |_, _| {});
    let at = fixture.by_role(Role::CANVAS)[0].rect.center();
    let before = view.get().expect("the canvas ran").center;
    fixture.frame(vec![pointer::moved(at)]).frame(vec![pointer::press(at)]).frame(vec![pointer::moved(at + BY * 0.5)]);
    navigates.set(false);
    fixture.frame(vec![pointer::moved(at + BY)]).frame(vec![pointer::release(at + BY)]).settle();
    assert_ne!(view.get().expect("the canvas ran").center, before, "the navigating press pans the view");
    let calls = log.borrow().clone();
    let pressed = |call: &String| ["main press", "main drag", "main release"].contains(&call.as_str());
    assert!(!calls.iter().any(pressed), "the navigating press never reaches the selected tool: {calls:?}");
    fixture.frame(vec![pointer::press(at)]).frame(vec![pointer::release(at)]).settle();
    let calls = log.borrow().clone();
    assert_eq!(calls.iter().filter(|call| *call == "main press").count(), 1, "the next press is the selected tool's: {calls:?}");
}

/// The zoom rectangle of a momentary Zoom that Cmd+Space started keeps being sized by the drag once Cmd is let go, and
/// moves with Space once Space is let go and pressed again, as a marquee does: the release zooms to the moved
/// rectangle. Space held since the press sizes it until let go.
#[trial]
fn space_pressed_again_moves_a_cmd_space_zoom_rectangle() {
    let log = Log::default();
    let mut options = NavigationOptions::default();
    options.scrubby_zoom = false;
    options.animated_zoom = false;
    let (mut fixture, view, _) = momentary_canvas_under(options, Rc::default(), Probe::new("main", &log), |_, _| {});
    let room = fixture.by_role(Role::CANVAS)[0].rect;
    let at = room.center() - Vec2::new(180.0, 90.0);
    let before = view.get().expect("the canvas ran").zoom;
    let (none, by) = (Modifiers::NONE, |x: f32| at + Vec2::new(x, x / 2.0));
    fixture.hold(Modifiers::COMMAND).frame(vec![pointer::moved(at), key_press(Key::Space, Modifiers::COMMAND)]);
    fixture.frame(vec![pointer::press(at)]).frame(vec![pointer::moved(by(100.0))]);
    fixture.hold(none).frame(vec![pointer::moved(by(200.0))]);
    fixture.frame(vec![key_release(Key::Space, none)]);
    fixture.frame(vec![key_press(Key::Space, none), pointer::moved(by(240.0))]).frame(vec![pointer::moved(by(280.0))]);
    fixture.frame(vec![key_release(Key::Space, none), pointer::moved(by(360.0))]).frame(vec![pointer::release(by(360.0))]).settle();
    let zoomed = view.get().expect("the canvas ran").zoom / before;
    // Moved by Space, the rectangle spans 280 x 140 points; never moved, it would span 360 x 180.
    let fit = |w: f32, h: f32| f64::from((room.width() / w).min(room.height() / h));
    assert!((zoomed - fit(280.0, 140.0)).abs() < (zoomed - fit(360.0, 180.0)).abs(), "zoomed by {zoomed}: to the moved rectangle");
}

/// The momentary Zoom's direction comes from the keys that chose it: a click with Alt and Space held zooms out, and the
/// Alt that turned the Hand into Zoom Out does not flip it back to Zoom In.
#[trial]
fn a_click_under_alt_space_zooms_out() {
    let log = Log::default();
    let (mut fixture, view, canvas) = momentary_canvas(Probe::new("main", &log));
    let at = fixture.by_role(Role::CANVAS)[0].rect.center();
    let before = view.get().expect("the canvas ran").zoom;
    let space = craft_keymap::Key::Space;
    for input in [
        ToolInput::hold(space),
        ToolInput::down(PointerButton::Primary, at).mods(Mods::ALT),
        ToolInput::up(at).mods(Mods::ALT),
        ToolInput::let_go(space),
    ] {
        ToolHost::inject(fixture.ctx(), canvas.get(), input);
    }
    fixture.run_injected();
    let after = view.get().expect("the canvas ran").zoom;
    assert!(after < before, "Alt+Space and a click zoom out: {before} -> {after}");
    assert!(log.borrow().is_empty(), "the selected tool sees nothing: {:?}", log.borrow());
}

/// A held Space pans over a session member on the canvas's body: a drag on a crop corner pans the view with the Hand
/// and leaves the crop as it was; without Space the same drag moves the corner and leaves the view.
#[trial]
fn a_held_space_pans_over_a_canvas_control_and_leaves_it_unchanged() {
    // Around the document's centre, which the zoomed-in view shows.
    let start = craft_behavior::geometry::CropBox::upright(craft_behavior::kurbo::Rect::new(580.0, 430.0, 620.0, 470.0));
    let crop = Rc::new(RefCell::new(start));
    let shown = Rc::clone(&crop);
    let log = Log::default();
    let (mut fixture, view, canvas) = momentary_canvas_with(Probe::new("main", &log), move |ui, body| {
        let _ = CropOverlay::new("Crop", Bind::bound(&mut *shown.borrow_mut())).show_at(ui, body.rect);
    });
    let corner = fixture.by_label("Crop corner, top left").expect("the crop shows its corners").rect.center();
    let before = view.get().expect("the canvas ran").center;
    let space = craft_keymap::Key::Space;
    for input in [
        ToolInput::hold(space),
        ToolInput::down(PointerButton::Primary, corner),
        ToolInput::moved(corner + BY),
        ToolInput::up(corner + BY),
        ToolInput::let_go(space),
    ] {
        ToolHost::inject(fixture.ctx(), canvas.get(), input);
    }
    fixture.run_injected();
    let panned = view.get().expect("the canvas ran").center;
    assert_ne!(panned, before, "the press under the held Space pans with the Hand");
    assert_eq!(*crop.borrow(), start, "the crop stays as it was under the Hand's drag");
    let calls = log.borrow().clone();
    assert!(!calls.iter().any(|call| call == "main press" || call == "main drag"), "the Hand's drag never reaches the selected tool: {calls:?}");

    let corner = fixture.by_label("Crop corner, top left").expect("the crop shows its corners").rect.center();
    for input in [ToolInput::down(PointerButton::Primary, corner), ToolInput::moved(corner + BY), ToolInput::up(corner + BY)] {
        ToolHost::inject(fixture.ctx(), canvas.get(), input);
    }
    fixture.run_injected();
    assert_ne!(*crop.borrow(), start, "without Space the drag moves the corner");
    assert_eq!(view.get().expect("the canvas ran").center, panned, "the corner's drag leaves the view");
}

/// A queued Space starts the Hand only where the real key would fire it: with the app's gate refusing the Hand, the
/// press it is held over runs the selected tool.
#[trial]
fn a_queued_space_whose_hand_the_gate_refuses_leaves_the_press_to_the_selected_tool() {
    let log = Log::default();
    let (mut fixture, view, canvas) = momentary_canvas(Probe::new("main", &log));
    fixture.gate(CommandGate::new((), |_: &(), command| !matches!(MomentaryCommands::from_ref(command), Some(MomentaryCommands::Hand)))).settle();
    let at = fixture.by_role(Role::CANVAS)[0].rect.center();
    let before = view.get().expect("the canvas ran").center;
    let space = craft_keymap::Key::Space;
    for input in [
        ToolInput::hold(space),
        ToolInput::down(PointerButton::Primary, at),
        ToolInput::moved(at + BY),
        ToolInput::up(at + BY),
        ToolInput::let_go(space),
    ] {
        ToolHost::inject(fixture.ctx(), canvas.get(), input);
    }
    fixture.run_injected();
    assert_eq!(view.get().expect("the canvas ran").center, before, "the refused Hand pans nothing");
    let calls = log.borrow().clone();
    assert!(calls.iter().any(|call| call == "main drag"), "the press under the queued Space is the selected tool's: {calls:?}");
}

/// A canvas under a dialog that keeps the canvas: returns the fixture, the app's view and the canvas's rect.
fn kept_canvas<T: CanvasTool + 'static>(tool: Rc<RefCell<T>>) -> CanvasRig<Rect> {
    let view = Rc::new(Cell::new(None::<CanvasView>));
    let rect = Rc::new(Cell::new(Rect::NOTHING));
    let (shown_view, shown_rect) = (Rc::clone(&view), Rc::clone(&rect));
    let mut fixture = Fixture::new(move |ui| {
        let room = ui.available_rect_before_wrap();
        let current = shown_view.get().unwrap_or_else(|| CanvasView::new(room, Point::new(600.0, 450.0), 1.0));
        let mut reply = None;
        let shown = Canvas::new("Document").document(DOCUMENT).view(current).show_at(ui, room, |ui, body| {
            if let Some(pass) = tool_pass(&body.env) {
                reply = Some(ToolHost::new().run(ui, &pass, &mut *tool.borrow_mut()));
            }
        });
        shown_rect.set(room);
        let id = shown.output.response.id;
        let mut open = true;
        Dialog::new("Keeps the canvas").opened(Bind::bound(&mut open)).show(ui, |ui, cx| {
            cx.keep_view_open(id);
            Text::new("Pick").show(ui);
        });
        let reply = reply.unwrap_or_default();
        shown_view.set(Some(reply.view.or(shown.output.view).unwrap_or(current)));
    });
    fixture.settle();
    (fixture, view, rect)
}

#[trial]
fn a_view_only_tool_drags_under_a_modal_that_keeps_the_canvas() {
    let hand = Rc::new(RefCell::new(HandTool::new(NavigationOptions::default())));
    let (mut fixture, view, rect) = kept_canvas(hand);
    let at = rect.get().center();
    let before = view.get().expect("the canvas ran").center;
    fixture.frame(vec![pointer::press(at)]).frame(vec![pointer::moved(at + BY)]).frame(vec![pointer::moved(at + BY * 2.0)]);
    fixture.frame(vec![pointer::release(at + BY * 2.0)]);
    assert_ne!(view.get().expect("the canvas ran").center, before, "a Hand drag pans under the modal");

    let tool = Rc::new(RefCell::new(Recorder::default()));
    let (mut fixture, _, rect) = kept_canvas(Rc::clone(&tool));
    let at = rect.get().center();
    fixture.frame(vec![pointer::press(at)]).frame(vec![pointer::moved(at + BY)]).frame(vec![pointer::release(at + BY)]);
    assert_eq!(tool.borrow().count(Call::Press), 0, "a drawing tool's press stays blocked: {:?}", tool.borrow().calls);
}

/// Space over the canvas a dialog keeps open is the canvas's while a field in the dialog has focus (production pans
/// under a dialog whatever has focus): its momentary Hand pans the press held over it, and the selected tool sees none.
#[trial]
fn space_over_a_kept_canvas_pans_while_a_field_in_the_dialog_has_focus() {
    let log = Log::default();
    let main = RefCell::new(Probe::new("main", &log));
    let (view, rect) = (Rc::new(Cell::new(None::<CanvasView>)), Rc::new(Cell::new(Rect::NOTHING)));
    let (shown_view, shown_rect) = (Rc::clone(&view), Rc::clone(&rect));
    let mut set = Momentary::new(NavigationOptions::default());
    let mut amount = Some(5.0);
    let mut fixture = Fixture::new(move |ui| {
        let room = ui.available_rect_before_wrap();
        let current = shown_view.get().unwrap_or_else(|| CanvasView::new(room, Point::new(600.0, 450.0), 4.0));
        let mut reply = None;
        let shown = Canvas::new("Document").document(DOCUMENT).view(current).show_at(ui, room, |ui, body| {
            if let Some(pass) = tool_pass(&body.env) {
                reply = Some(ToolHost::new().momentary(&mut set).run(ui, &pass, &mut *main.borrow_mut()));
            }
        });
        shown_rect.set(room);
        let id = shown.output.response.id;
        let mut open = true;
        Dialog::new("Keeps the canvas").opened(Bind::bound(&mut open)).show(ui, |ui, cx| {
            cx.keep_view_open(id);
            NumberField::new("Amount", Bind::bound(&mut amount)).show(ui);
        });
        let reply = reply.unwrap_or_default();
        shown_view.set(Some(reply.view.or(shown.output.view).unwrap_or(current)));
    });
    fixture.settle();
    assert!(fixture.focused().is_some(), "the dialog's field has focus");
    // Near the canvas's corner, clear of the centred dialog.
    let at = rect.get().min + Vec2::splat(40.0);
    let before = view.get().expect("the canvas ran").center;
    fixture.frame(vec![pointer::moved(at)]).frame(vec![key_press(Key::Space, Modifiers::NONE)]);
    fixture.frame(vec![pointer::press(at)]).frame(vec![pointer::moved(at + BY)]).frame(vec![pointer::moved(at + BY * 2.0)]);
    fixture.frame(vec![pointer::release(at + BY * 2.0)]);
    assert_ne!(view.get().expect("the canvas ran").center, before, "Space's Hand pans under the dialog");
    assert!(!log.borrow().iter().any(|call| call == "main press"), "the selected tool sees no press: {:?}", log.borrow());
}

#[trial]
fn matched_smart_lines_are_drawn_and_reported() {
    let shapes = |smart: bool, color: Option<DataColor>| {
        let log = Log::default();
        let mut targets = SnapTargets::new();
        targets.add(SnapTarget::new(Axis::Y, 100.0, SnapKind::LayerEdge));
        let mut options = SnapOptions::default();
        options.smart_guides = smart;
        let mut main = Probe::new("main", &log);
        main.snaps = true;
        let mut rig = Rig::with(main, &log, Some(Snap::new(Arc::new(targets), options)), Setup { smart_color: color, ..Setup::default() });
        rig.fixture.idle(2);
        let reported = rig.snapped.borrow().len();
        (rig.fixture.frame_shapes(), reported)
    };
    let (on, reported_on) = shapes(true, None);
    let (off, reported_off) = shapes(false, None);
    let (on, off) = (on.len(), off.len());
    assert!(reported_on > 0 && reported_off > 0, "the lines reach the response whatever the option: {reported_on}, {reported_off}");
    assert!(on > off, "smart kinds are painted only with Smart Guides on: {on} shapes vs {off}");

    let color = Color32::from_rgb(12, 200, 34);
    let (coloured, _) = shapes(true, Some(DataColor(color)));
    assert!(coloured.iter().any(|shape| strokes_in(shape, color)), "a smart line is drawn in `smart_guide_color` when it is set");
    let (plain, _) = shapes(true, None);
    assert!(!plain.iter().any(|shape| strokes_in(shape, color)), "and not otherwise");
}

/// Whether `shape` or a shape inside it is a line or path stroked in `color`.
fn strokes_in(shape: &Shape, color: Color32) -> bool {
    match shape {
        Shape::Vec(shapes) => shapes.iter().any(|shape| strokes_in(shape, color)),
        Shape::LineSegment { stroke, .. } => stroke.color == color,
        Shape::Path(path) => path.stroke.color == egui::epaint::ColorMode::Solid(color),
        _ => false,
    }
}

#[trial]
fn a_tool_that_takes_every_move_gets_each_move_of_a_pass_in_order() {
    for every in [true, false] {
        let log = Log::default();
        let mut main = Probe::new("main", &log);
        main.every_move = every;
        let rerun = Rc::new(Cell::new(false));
        let mut rig = Rig::with(main, &log, None, Setup { rerun: Rc::clone(&rerun), ..Setup::default() });
        let at = rig.centre();
        let path = [at + Vec2::new(20.0, 0.0), at + Vec2::new(40.0, 5.0), at + Vec2::new(60.0, 10.0)];
        rig.fixture.frame(vec![pointer::press(at)]);
        // The frame's pass runs twice: the moves reach the tool once.
        rerun.set(true);
        rig.fixture.frame(path.iter().map(|to| pointer::moved(*to)).collect());
        let main = rig.main.borrow();
        if every {
            let seen: Vec<Pos2> = main.drags.iter().map(|(pos, _)| *pos).collect();
            assert_eq!(seen, path, "each move of the pass reaches the tool, in order");
            assert!(main.drags.windows(2).all(|pair| pair[0].1 <= pair[1].1), "times rise: {:?}", main.drags);
            assert_eq!(main.drags.last().map(|(_, time)| *time), Some(main.pass_time), "the last move is at the pass time");
        } else {
            assert_eq!(main.drags.len(), 1, "a tool that does not ask gets one call: {:?}", main.drags);
            assert_eq!(main.drags[0].0, path[2], "at the last position");
        }
    }
}

/// A tool that takes every move gets the moves of the press inside the click distance, one pass each; any other tool
/// gets none of them and its press stays a click.
#[trial]
fn moves_inside_the_click_distance_reach_a_tool_that_takes_every_move() {
    for every in [true, false] {
        let log = Log::default();
        let mut main = Probe::new("main", &log);
        main.every_move = every;
        let mut rig = Rig::with(main, &log, None, Setup::default());
        let at = rig.centre();
        let path = [at + Vec2::new(1.0, 0.0), at + Vec2::new(2.0, 0.0), at + Vec2::new(3.0, 0.0)];
        rig.fixture.frame(vec![pointer::press(at)]);
        for to in path {
            rig.fixture.frame(vec![pointer::moved(to)]);
        }
        rig.fixture.frame(vec![pointer::release(path[2])]);
        let seen: Vec<Pos2> = rig.main.borrow().drags.iter().map(|(pos, _)| *pos).collect();
        let calls = log.borrow().clone();
        if every {
            assert_eq!(seen, path, "each move from the press reaches the tool, in order");
        } else {
            assert!(seen.is_empty(), "a tool that does not take every move gets no drag inside the click distance: {seen:?}");
        }
        assert!(calls.iter().any(|call| call == "main release"), "the gesture ends with the release: {calls:?}");
    }
}

/// A right-click during a drag asks for the menu where it came up and leaves the drag to the tool: no release and no
/// cancel until the primary button comes up (a dragged comment keeps moving while its page menu opens).
#[trial]
fn a_secondary_click_during_a_drag_asks_for_the_menu_and_the_drag_goes_on() {
    let log = Log::default();
    let mut rig = Rig::new(Probe::new("main", &log), &log, None);
    let at = rig.centre();
    let menu = at + BY;
    rig.fixture.frame(vec![pointer::press(at)]).frame(vec![pointer::moved(menu)]);
    rig.fixture.frame(vec![pointer::secondary(menu, true)]).frame(vec![pointer::secondary(menu, false)]);
    let asked = rig.contexts.borrow().clone();
    assert_eq!(asked.iter().map(|request| request.at).collect::<Vec<_>>(), [menu], "the secondary release asks once, where it came up");
    let calls = rig.calls();
    assert!(!calls.iter().any(|call| call.ends_with("release") || call.ends_with("cancel")), "the drag goes on: {calls:?}");
    rig.fixture.frame(vec![pointer::moved(menu + BY)]).frame(vec![pointer::release(menu + BY)]);
    let calls = rig.calls();
    assert_eq!(calls.iter().filter(|call| call.ends_with("press")).count(), 1, "one gesture: {calls:?}");
    assert_eq!(calls.last().map(String::as_str), Some("main release"), "it ends on the primary release: {calls:?}");
    assert_eq!(rig.contexts.borrow().len(), 1, "and asks no more");
}

#[trial]
fn a_secondary_click_the_tool_did_not_take_asks_for_the_menu() {
    let log = Log::default();
    let mut rig = Rig::new(Probe::new("main", &log), &log, None);
    let at = rig.centre();
    rig.fixture.hold(Modifiers::SHIFT).frame_held(vec![pointer::secondary(at, true)]);
    rig.fixture.hold(Modifiers::NONE).frame(vec![pointer::secondary(at, false)]);
    let asked = rig.contexts.borrow().clone();
    assert_eq!(asked.len(), 1, "{asked:?}");
    assert_eq!(asked[0].at, at);
    assert!(asked[0].mods.shift, "the modifiers are those held at the press: {:?}", asked[0]);
    assert_eq!(asked[0].momentary, None);

    // The Menu key asks at the view's centre.
    rig.contexts.borrow_mut().clear();
    rig.fixture.frame(vec![pointer::press(at)]).frame(vec![pointer::release(at)]);
    rig.fixture.key(Key::F10, Modifiers::SHIFT).settle();
    let asked = rig.contexts.borrow().clone();
    assert_eq!(asked.first().map(|request| request.at), Some(at), "{asked:?}");

    // A press the tool took asks for nothing.
    let log = Log::default();
    let mut taker = Probe::new("main", &log);
    taker.secondary = true;
    let mut rig = Rig::new(taker, &log, None);
    let at = rig.centre();
    rig.fixture.frame(vec![pointer::secondary(at, true)]).frame(vec![pointer::secondary(at, false)]);
    assert!(rig.contexts.borrow().is_empty(), "{:?}", rig.contexts.borrow());

    // A press on the scroll bars reports none.
    let (mut fixture, contexts, _, rect) = menu_canvas(false, true);
    let on_bar = Pos2::new(rect.get().center().x, rect.get().bottom() - 3.0);
    fixture.frame(vec![pointer::secondary(on_bar, true)]).frame(vec![pointer::secondary(on_bar, false)]);
    assert!(contexts.borrow().is_empty(), "{:?}", contexts.borrow());

    // With Space held a secondary drag pans with the momentary Hand and reports none.
    let (mut fixture, contexts, view, rect) = menu_canvas(true, false);
    let at = rect.get().center();
    let before = view.get().expect("the canvas ran").center;
    fixture.frame(vec![pointer::moved(at), key_press(Key::Space, Modifiers::NONE)]);
    fixture.frame(vec![pointer::secondary(at, true)]).frame(vec![pointer::moved(at + BY)]).frame(vec![pointer::moved(at + BY * 2.0)]);
    fixture.frame(vec![pointer::secondary(at + BY * 2.0, false)]);
    assert_ne!(view.get().expect("the canvas ran").center, before, "the secondary drag pans");
    assert!(contexts.borrow().is_empty(), "{:?}", contexts.borrow());

    // The selected Hand does not take the secondary button: its click asks for the menu.
    let (mut fixture, contexts, _, rect) = menu_canvas(false, false);
    let at = rect.get().center();
    fixture.frame(vec![pointer::secondary(at, true)]).frame(vec![pointer::secondary(at, false)]);
    assert_eq!(contexts.borrow().len(), 1, "{:?}", contexts.borrow());
}

/// With Space held a secondary click asks for the menu with the momentary Hand, whose secondary drag pans.
#[trial]
fn a_secondary_click_with_the_momentary_hand_asks_for_its_menu() {
    let (mut fixture, contexts, _, rect) = menu_canvas(true, false);
    let at = rect.get().center();
    fixture.frame(vec![pointer::moved(at), key_press(Key::Space, Modifiers::NONE)]);
    fixture.frame(vec![pointer::secondary(at, true)]).frame(vec![pointer::secondary(at, false)]);
    let asked = contexts.borrow().clone();
    assert_eq!(asked.len(), 1, "{asked:?}");
    assert_eq!(asked[0].at, at);
    assert_eq!(asked[0].momentary, Some(MomentaryTool::Hand), "the request names the momentary Hand: {asked:?}");
}

/// A canvas with the Hand selected, with the momentary set when `momentary` and scroll bars in its body when `bars`.
/// Returns the fixture, the menu requests, the app's view and the canvas's rect.
fn menu_canvas(momentary: bool, bars: bool) -> MenuRig {
    let (contexts, view, rect) = (Rc::new(RefCell::new(Vec::new())), Rc::new(Cell::new(None::<CanvasView>)), Rc::new(Cell::new(Rect::NOTHING)));
    let (shown_contexts, shown_view, shown_rect) = (Rc::clone(&contexts), Rc::clone(&view), Rc::clone(&rect));
    let mut hand = HandTool::new(NavigationOptions::default());
    let mut set = Momentary::new(NavigationOptions::default());
    let mut fixture = Fixture::new(move |ui| {
        let room = ui.available_rect_before_wrap();
        let current = shown_view.get().unwrap_or_else(|| CanvasView::new(room, Point::new(600.0, 450.0), 4.0));
        shown_rect.set(room);
        let mut reply = None;
        let shown = Canvas::new("Document").document(DOCUMENT).view(current).show_at(ui, room, |ui, body| {
            if bars {
                let mut bar_view = current;
                CanvasScrollbars::new(
                    "Scroll bars",
                    Bind::bound(&mut bar_view),
                    craft_behavior::kurbo::Rect::from_origin_size(Point::ZERO, DOCUMENT),
                )
                .show_at(ui, body.rect);
            }
            if let Some(pass) = tool_pass(&body.env) {
                let mut host = ToolHost::new();
                if momentary {
                    host = host.momentary(&mut set);
                }
                reply = Some(host.run(ui, &pass, &mut hand));
            }
        });
        let reply = reply.unwrap_or_default();
        shown_contexts.borrow_mut().extend(reply.context);
        shown_view.set(Some(reply.view.or(shown.output.view).unwrap_or(current)));
    });
    fixture.settle();
    (fixture, contexts, view, rect)
}

#[trial]
fn a_drag_past_the_canvas_edge_scrolls_the_view_and_a_turned_view_scrolls_on_the_clipped_axis() {
    for (rotation, upright) in [(0.0, true), (90.0, false)] {
        let tool = Rc::new(RefCell::new(Recorder::default()));
        let (mut fixture, view) = scroll_canvas(Rc::clone(&tool), rotation);
        let canvas = fixture.by_role(Role::CANVAS)[0].rect;
        let start = canvas.center();
        let before = view.get().expect("the canvas ran").center;
        fixture.frame(vec![pointer::press(start)]);
        fixture.frame(vec![pointer::moved(Pos2::new(canvas.right() + 25.0, start.y))]).idle(3);
        let after = view.get().expect("the canvas ran").center;
        if upright {
            assert!(after.x > before.x && (after.y - before.y).abs() < 1e-6, "scrolls along x: {before:?} -> {after:?}");
        } else {
            assert!(after.y != before.y && (after.x - before.x).abs() < 1e-6, "a view turned 90 degrees scrolls along y: {before:?} -> {after:?}");
        }
        fixture.frame(vec![pointer::release(Pos2::new(canvas.right() + 25.0, start.y))]);
        assert_eq!(tool.borrow().count(Call::Press), 1, "{:?}", tool.borrow().calls);
    }
}

/// A drag held still in the band inside the canvas edge scrolls the view: a canvas that fills the screen leaves the
/// pointer no room past its edge. A drag held away from the edge leaves the view.
#[trial]
fn a_drag_held_in_the_band_inside_the_canvas_edge_scrolls_the_view() {
    let (mut fixture, view) = scroll_canvas(Rc::new(RefCell::new(Recorder::default())), 0.0);
    let canvas = fixture.by_role(Role::CANVAS)[0].rect;
    let centre = || view.get().expect("the canvas ran").center;
    let start = canvas.center();
    fixture.frame(vec![pointer::press(start)]);
    fixture.frame(vec![pointer::moved(start + Vec2::new(30.0, 0.0))]);
    let held = centre();
    fixture.idle(3);
    assert_eq!(centre(), held, "a drag held away from the edge leaves the view");

    let band = Pos2::new(canvas.right() - 4.0, start.y);
    fixture.frame(vec![pointer::moved(band)]);
    let entered = centre();
    fixture.idle(3);
    let rested = centre();
    assert!(rested.x > entered.x && rested.y == entered.y, "resting in the band scrolls toward the edge: {entered:?} -> {rested:?}");
    fixture.frame(vec![pointer::release(band)]);
}

/// A canvas whose app reads the window's intents after the tool host, last in its UI: returns the fixture and how many
/// Cancels the window took.
fn window_reading_canvas(tool: impl CanvasTool + 'static) -> (Fixture, Rc<Cell<u32>>) {
    let tool = RefCell::new(tool);
    let cancels = Rc::new(Cell::new(0));
    let seen = Rc::clone(&cancels);
    let mut fixture = Fixture::new(move |ui| {
        let room = ui.available_rect_before_wrap();
        let _ = Canvas::new("Document").document(DOCUMENT).view(CanvasView::new(room, Point::new(600.0, 450.0), 1.0)).show(ui, |ui, body| {
            if let Some(pass) = tool_pass(&body.env) {
                let _ = ToolHost::new().run(ui, &pass, &mut *tool.borrow_mut());
            }
        });
        craft_ui::runtime::keys::with_window_intents(ui.ctx(), false, |window| {
            let took = window.take(craft_behavior::Intent::Cancel);
            seen.set(seen.get() + u32::from(took));
            took
        });
    });
    fixture.settle();
    (fixture, cancels)
}

/// On a canvas focused by a click, an Escape the tool does not take reaches the window's intents, and one the tool
/// takes does not.
#[trial]
fn a_focused_canvas_leaves_the_cancel_no_tool_took_to_the_window() {
    for takes in [false, true] {
        let (mut fixture, cancels) =
            if takes { window_reading_canvas(Recorder::default()) } else { window_reading_canvas(Probe::new("main", &Log::default())) };
        let canvas = fixture.by_role(Role::CANVAS).remove(0);
        fixture.click(&canvas).settle();
        cancels.set(0);
        fixture.frame(vec![key_press(Key::Escape, Modifiers::NONE)]).settle();
        let want = if takes { 0 } else { 1 };
        assert_eq!(cancels.get(), want, "the window sees the Escape only when the tool leaves it (tool takes it: {takes})");
    }
}

/// A tool that records whether its context shows the precise cursor, in each pass.
struct PreciseProbe(Rc<Cell<Option<bool>>>);

impl CanvasTool for PreciseProbe {
    fn each_pass(&mut self, cx: &mut ToolContext<'_>) {
        self.0.set(Some(cx.precise()));
    }

    fn press(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) -> ToolAction {
        ToolAction::Ignored
    }

    fn drag(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {}

    fn release(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {}

    fn cancel(&mut self, _: CancelReason, _: &mut ToolContext<'_>) {}

    fn paint(&self, _: &Draw<'_>, _: &ToolContext<'_>) {}

    fn command_form(&self) -> ToolPath {
        ToolPath::Command("no command")
    }
}

/// The precise cursor has one source, the canvas: its Precise preference, toggled by Caps Lock, reaches the tool and the
/// body's controls alike.
#[trial]
fn the_tool_and_the_canvas_controls_read_one_precise_cursor_from_the_canvas() {
    for preference in [false, true] {
        let (tool, body) = (Rc::new(Cell::new(None)), Rc::new(Cell::new(None)));
        let (tool_seen, body_seen) = (Rc::clone(&tool), Rc::clone(&body));
        let mut fixture = Fixture::new(move |ui| {
            let view = CanvasView::new(ui.available_rect_before_wrap(), Point::new(600.0, 450.0), 1.0);
            let _ = Canvas::new("Document").document(DOCUMENT).view(view).precise_cursors(preference).show(ui, |ui, cx| {
                body_seen.set(canvas_space(&cx.env).map(|space| space.precise()));
                if let Some(pass) = tool_pass(&cx.env) {
                    let _ = ToolHost::new().run(ui, &pass, &mut PreciseProbe(Rc::clone(&tool_seen)));
                }
            });
        });
        let ctx = fixture.ctx().clone();
        for caps_lock in [false, true] {
            WindowHost::of(&ctx).set_caps_lock(Some(caps_lock));
            fixture.frame(Vec::new());
            let precise = preference != caps_lock;
            assert_eq!(tool.get(), Some(precise), "the tool: Precise {preference}, Caps Lock {caps_lock}");
            assert_eq!(body.get(), Some(precise), "the controls: Precise {preference}, Caps Lock {caps_lock}");
        }
    }
}

/// A tool that asks for a readout, or a hud, on every drag step.
struct Measuring {
    hud: bool,
}

impl CanvasTool for Measuring {
    fn press(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) -> ToolAction {
        ToolAction::Claimed
    }

    fn drag(&mut self, _: &ToolPointer, cx: &mut ToolContext<'_>) {
        let rows = vec![(craft_ui::runtime::env::Label::from("W"), "123 px".to_owned())];
        if self.hud { cx.hud(rows) } else { cx.readout(rows) }
    }

    fn release(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {}

    fn cancel(&mut self, _: CancelReason, _: &mut ToolContext<'_>) {}

    fn paint(&self, _: &Draw<'_>, _: &ToolContext<'_>) {}

    fn command_form(&self) -> ToolPath {
        ToolPath::Command("the Measure command")
    }
}

/// Holds a drag of a `Measuring` tool under Show Transformation Values `shown`: whether the value painted and whether
/// the host reported the rows.
fn readout_during_a_drag(hud: bool, shown: Option<bool>) -> (bool, bool) {
    let reported = Rc::new(Cell::new(false));
    let seen = Rc::clone(&reported);
    let tool = RefCell::new(Measuring { hud });
    let mut fixture = Fixture::new(move |ui| {
        let room = ui.available_rect_before_wrap();
        let mut reply = None;
        let _ = Canvas::new("Document").document(DOCUMENT).view(CanvasView::new(room, Point::new(600.0, 450.0), 0.5)).show(ui, |ui, body| {
            if let Some(pass) = tool_pass(&body.env) {
                reply = Some(ToolHost::new().run(ui, &pass, &mut *tool.borrow_mut()));
            }
        });
        let reply = reply.unwrap_or_default();
        seen.set(reply.readout.is_some_and(|readout| readout.rows.iter().any(|(_, value)| value == "123 px")));
    });
    if let Some(shown) = shown {
        fixture.root_value(craft_ui::canvas::ShowReadouts(shown));
    }
    fixture.settle();
    let centre = fixture.by_role(Role::CANVAS).into_iter().next().expect("a canvas").center();
    // The last frame is a move, so the tool's drag asks for its rows in the frame the texts are read from.
    fixture.frame(vec![pointer::moved(centre)]);
    fixture.frame(vec![pointer::press(centre)]);
    for step in 1..=4 {
        fixture.frame(vec![pointer::moved(centre + BY * (step as f32 / 4.0))]);
    }
    let painted = fixture.texts().iter().any(|run| run.text.contains("123 px"));
    (painted, reported.get())
}

#[trial]
fn a_drags_readout_is_not_reported_in_the_pass_its_release_ends_the_gesture() {
    let reported = Rc::new(Cell::new(false));
    let seen = Rc::clone(&reported);
    let tool = RefCell::new(Measuring { hud: false });
    let mut fixture = Fixture::new(move |ui| {
        let room = ui.available_rect_before_wrap();
        let mut reply = None;
        let _ = Canvas::new("Document").document(DOCUMENT).view(CanvasView::new(room, Point::new(600.0, 450.0), 0.5)).show(ui, |ui, body| {
            if let Some(pass) = tool_pass(&body.env) {
                reply = Some(ToolHost::new().run(ui, &pass, &mut *tool.borrow_mut()));
            }
        });
        seen.set(reply.unwrap_or_default().readout.is_some());
    });
    fixture.settle();
    let centre = fixture.by_role(Role::CANVAS).into_iter().next().expect("a canvas").center();
    fixture.frame(vec![pointer::moved(centre)]);
    fixture.frame(vec![pointer::press(centre)]);
    for step in 1..=4 {
        fixture.frame(vec![pointer::moved(centre + BY * (step as f32 / 4.0))]);
    }
    assert!(reported.get(), "the drag reports its readout");
    fixture.frame(vec![pointer::release(centre + BY)]);
    assert!(!reported.get(), "the release pass reports none");
}

/// A tool whose drag asks for a readout while `asks` holds and for none once it is cleared.
struct Pausing {
    asks: Rc<Cell<bool>>,
}

impl CanvasTool for Pausing {
    fn press(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) -> ToolAction {
        ToolAction::Claimed
    }

    fn drag(&mut self, _: &ToolPointer, cx: &mut ToolContext<'_>) {
        if self.asks.get() {
            cx.readout(vec![(craft_ui::runtime::env::Label::from("W"), "123 px".to_owned())]);
        }
    }

    fn release(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {}

    fn cancel(&mut self, _: CancelReason, _: &mut ToolContext<'_>) {}

    fn paint(&self, _: &Draw<'_>, _: &ToolContext<'_>) {}

    fn command_form(&self) -> ToolPath {
        ToolPath::Command("the Pause command")
    }
}

#[trial]
fn a_drags_readout_stands_in_the_passes_between_its_moves() {
    let reported = Rc::new(Cell::new(false));
    let seen = Rc::clone(&reported);
    let asks = Rc::new(Cell::new(true));
    let tool = RefCell::new(Pausing { asks: Rc::clone(&asks) });
    let mut fixture = Fixture::new(move |ui| {
        let room = ui.available_rect_before_wrap();
        let mut reply = None;
        let _ = Canvas::new("Document").document(DOCUMENT).view(CanvasView::new(room, Point::new(600.0, 450.0), 0.5)).show(ui, |ui, body| {
            if let Some(pass) = tool_pass(&body.env) {
                reply = Some(ToolHost::new().run(ui, &pass, &mut *tool.borrow_mut()));
            }
        });
        seen.set(reply.unwrap_or_default().readout.is_some());
    });
    fixture.settle();
    let centre = fixture.by_role(Role::CANVAS).into_iter().next().expect("a canvas").center();
    fixture.frame(vec![pointer::moved(centre)]);
    fixture.frame(vec![pointer::press(centre)]);
    for step in 1..=4 {
        fixture.frame(vec![pointer::moved(centre + BY * (step as f32 / 4.0))]);
    }
    // A pass with no move (the pointer rests, the ants or anything else repaint) gives the tool no drag call.
    for _ in 0..3 {
        fixture.frame(Vec::new());
        assert!(reported.get(), "the readout stays while the pointer rests mid-drag");
        assert!(fixture.texts().iter().any(|run| run.text.contains("123 px")), "and it still paints");
    }
    asks.set(false);
    fixture.frame(vec![pointer::moved(centre + BY * 1.25)]);
    assert!(!reported.get(), "a drag that asks for none drops it");
    fixture.frame(Vec::new());
    assert!(!reported.get(), "and it does not come back while the pointer rests");
    asks.set(true);
    fixture.frame(vec![pointer::moved(centre + BY * 1.5)]);
    fixture.frame(vec![pointer::release(centre + BY * 1.5)]);
    fixture.frame(Vec::new());
    assert!(!reported.get(), "the readout ends with its gesture");
}

#[trial]
fn show_transformation_values_gates_the_readouts_paint_and_never_the_hud() {
    assert_eq!(readout_during_a_drag(false, None), (true, true), "by default a tool's readout paints and is reported");
    assert_eq!(
        readout_during_a_drag(false, Some(false)),
        (false, true),
        "Show Transformation Values off: nothing paints, the rows are still reported"
    );
    assert_eq!(readout_during_a_drag(true, Some(false)), (true, true), "a hud paints with Show Transformation Values off");
}

/// The tip [`Tipping`] asks for.
const TIP: &str = "Follow the link";

/// A tool that asks for a tip on every hover and claims every press.
struct Tipping;

impl CanvasTool for Tipping {
    fn hover(&mut self, _: &ToolPointer, cx: &mut ToolContext<'_>) {
        cx.tip(TIP);
    }

    fn press(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) -> ToolAction {
        ToolAction::Claimed
    }

    fn drag(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {}

    fn release(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {}

    fn cancel(&mut self, _: CancelReason, _: &mut ToolContext<'_>) {}

    fn paint(&self, _: &Draw<'_>, _: &ToolContext<'_>) {}

    fn command_form(&self) -> ToolPath {
        ToolPath::Command("the Follow command")
    }
}

/// Guards the host's tip rule: the tool answers only when the pointer moves, so the host must keep its tip while the
/// pointer rests through the tooltip delay, and drop it at a press.
#[trial]
fn a_hovering_tools_tip_shows_at_the_pointer_and_a_press_hides_it() {
    let reported = Rc::new(Cell::new(false));
    let seen = Rc::clone(&reported);
    let tool = RefCell::new(Tipping);
    let mut fixture = Fixture::new(move |ui| {
        let room = ui.available_rect_before_wrap();
        let mut reply = None;
        let _ = Canvas::new("Document").document(DOCUMENT).view(CanvasView::new(room, Point::new(600.0, 450.0), 0.5)).show(ui, |ui, body| {
            if let Some(pass) = tool_pass(&body.env) {
                reply = Some(ToolHost::new().run(ui, &pass, &mut *tool.borrow_mut()));
            }
        });
        seen.set(reply.unwrap_or_default().tip.is_some());
    });
    fixture.settle();
    let centre = fixture.by_role(Role::CANVAS).into_iter().next().expect("a canvas").center();
    fixture.frame(vec![pointer::moved(centre)]);
    fixture.advance(DELAY);
    fixture.settle();
    let tip = fixture.texts().iter().find(|run| run.text == TIP).map(|run| run.rect);
    let tip = tip.expect("the tip shows once the pointer rested for the tooltip delay");
    assert!(tip.min.y >= centre.y, "the tip hangs below the pointer: {tip:?} for {centre:?}");
    assert!(reported.get(), "the host reports the standing tip");
    fixture.frame(vec![pointer::press(centre)]);
    fixture.settle();
    assert!(!fixture.texts().iter().any(|run| run.text == TIP), "a press hides the tip");
    assert!(!reported.get(), "a press drops the tip");
}

/// After a handled press comes up, a pointer that rests where it was gets the tool's tip again, as the WordCraft
/// canvas showed a link's tip again after a Ctrl+click on it. The press drops the tip and the tool answers only on a
/// hover, so the host hovers once at the resting pointer when a press ends. Before that, a still pointer never got
/// the tip back.
#[trial]
fn a_pointer_resting_after_a_handled_press_gets_the_tip_again() {
    let reported = Rc::new(Cell::new(false));
    let seen = Rc::clone(&reported);
    let tool = RefCell::new(Tipping);
    let mut fixture = Fixture::new(move |ui| {
        let room = ui.available_rect_before_wrap();
        let mut reply = None;
        let _ = Canvas::new("Document").document(DOCUMENT).view(CanvasView::new(room, Point::new(600.0, 450.0), 0.5)).show(ui, |ui, body| {
            if let Some(pass) = tool_pass(&body.env) {
                reply = Some(ToolHost::new().run(ui, &pass, &mut *tool.borrow_mut()));
            }
        });
        seen.set(reply.unwrap_or_default().tip.is_some());
    });
    fixture.settle();
    let centre = fixture.by_role(Role::CANVAS).into_iter().next().expect("a canvas").center();
    fixture.frame(vec![pointer::moved(centre)]);
    fixture.advance(DELAY);
    fixture.settle();
    assert!(reported.get(), "the hover asks for the tip");
    fixture.frame(vec![pointer::press(centre)]);
    fixture.settle();
    assert!(!reported.get(), "the press drops the tip");
    fixture.frame(vec![pointer::release(centre)]);
    fixture.settle();
    fixture.advance(DELAY);
    fixture.settle();
    assert!(reported.get(), "the host reports the tip again with the pointer resting where the press was");
    assert!(fixture.texts().iter().any(|run| run.text == TIP), "the tip shows again once the pointer rested for the tooltip delay");
}

/// A tool that keeps the points of its gesture: the press and every drag.
#[derive(Default)]
struct Trail {
    points: Vec<Pos2>,
}

impl CanvasTool for Trail {
    fn press(&mut self, pointer: &ToolPointer, _: &mut ToolContext<'_>) -> ToolAction {
        self.points.push(pointer.pos);
        ToolAction::Claimed
    }

    fn drag(&mut self, pointer: &ToolPointer, _: &mut ToolContext<'_>) {
        self.points.push(pointer.pos);
    }

    fn release(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {}

    fn cancel(&mut self, _: CancelReason, _: &mut ToolContext<'_>) {}

    fn paint(&self, _: &Draw<'_>, _: &ToolContext<'_>) {}

    fn command_form(&self) -> ToolPath {
        ToolPath::Freehand("a test tool")
    }
}

/// Input, tool, picture: the host runs in the canvas's body before the picture, so with one pass a frame the picture
/// of each frame already holds that frame's press and moves.
#[trial]
fn the_picture_of_a_pass_shows_that_passes_press_and_moves() {
    let tool = Rc::new(RefCell::new(Trail::default()));
    // How many of the tool's points the picture of the last pass drew.
    let drawn = Rc::new(Cell::new(0_usize));
    let (held, picture) = (Rc::clone(&tool), Rc::clone(&drawn));
    let mut fixture = Fixture::new(move |ui| {
        let _ = Canvas::new("Document").document(DOCUMENT).show(ui, |ui, body| {
            if let Some(pass) = tool_pass(&body.env) {
                let _ = ToolHost::new().run(ui, &pass, &mut *held.borrow_mut());
            }
            picture.set(held.borrow().points.len());
        });
    });
    fixture.settle();
    fixture.ctx().options_mut(|options| options.max_passes = std::num::NonZeroUsize::MIN);
    let centre = fixture.by_role(Role::CANVAS).into_iter().next().expect("a canvas").center();
    fixture.frame(vec![pointer::moved(centre)]);
    let mut frames = vec![vec![pointer::press(centre)]];
    frames.extend((1..=4).map(|step| vec![pointer::moved(centre + BY * (step as f32 / 4.0))]));
    for events in frames {
        let before = tool.borrow().points.len();
        fixture.frame(events);
        assert_eq!(fixture.passes(), 1, "one pass a frame");
        let points = tool.borrow().points.len();
        assert!(points > before, "the frame's press or move reached the tool ({before} -> {points})");
        assert_eq!(drawn.get(), points, "the picture of the frame holds every point the tool took in it");
    }
}
