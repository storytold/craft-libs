//! `CanvasTool`, `ToolHost` and `tool_pick` (8.3): a drag is one `Begin`...`Commit`, Escape cancels it once, a press
//! on a canvas control never reaches the tool, and a group's letter picks and cycles its tools.

use craft_behavior::kurbo::{Point, Size};
use craft_behavior::{CancelReason, CanvasView, GesturePhase, PointerButton};
use craft_keymap::Mods;
use craft_ui::canvas::{Autoscroll, Canvas, CanvasTool, CropOverlay, ToolAction, ToolClaims, ToolContext, ToolHost, ToolKey, ToolPath, ToolPointer};
use craft_ui::contract::{Bind, Show as _, ShowContainer as _};
use craft_ui::primitives::Draw;
use craft_ui::runtime::access::Role;
use craft_ui::testing::{Fixture, ToolFixture, pointer, trial};
use egui::{Pos2, Rect, Vec2};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

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
fn a_gesture_cancelled_by_the_app_starts_no_new_one_while_the_button_stays_down() {
    let mut fixture = ToolFixture::new("Document", Recorder::default());
    let from = fixture.canvas().center();
    fixture.press_at(from);
    let (ctx, id) = (fixture.fixture().ctx().clone(), fixture.canvas_id());
    assert!(ToolHost::cancel(&ctx, id, &mut *fixture.tool_mut(), CancelReason::Dismissed), "a gesture was running");

    fixture.move_to(from + BY).move_to(from + BY * 2.0).release_at(from + BY * 2.0);
    assert_eq!(fixture.tool().calls, [Call::Press, Call::Cancel(CancelReason::Dismissed)], "the held button is not the tool's any more");

    fixture.press_at(from).release_at(from);
    assert_eq!(fixture.tool().count(Call::Press), 2, "the next press starts a gesture again: {:?}", fixture.tool().calls);
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

/// The hover autoscroll as an app runs it: the canvas shows a document zoomed in past the room, and the app scrolls
/// its view from the host's `past_edge`. Returns the fixture, the tool and the app's view.
fn band_canvas(scrolls: bool) -> (Fixture, Rc<RefCell<Band>>, Rc<Cell<Option<CanvasView>>>) {
    const DOCUMENT: Size = Size::new(1200.0, 900.0);
    let tool = Rc::new(RefCell::new(Band { scrolls, hovers: 0 }));
    let view = Rc::new(Cell::new(None::<CanvasView>));
    let (shown_tool, shown_view) = (Rc::clone(&tool), Rc::clone(&view));
    let mut fixture = Fixture::new(move |ui| {
        let room = ui.available_rect_before_wrap();
        let current = shown_view.get().unwrap_or_else(|| CanvasView::new(room, Point::new(600.0, 450.0), 4.0));
        let shown = Canvas::new("Document").document(DOCUMENT).view(current).show(ui, |_, _| {});
        let reply = ToolHost::new().run(ui, &shown.output, &mut *shown_tool.borrow_mut());
        let mut next = reply.view.unwrap_or(current);
        if let Some(past) = reply.past_edge
            && Autoscroll::scroll(&mut next, Autoscroll::step(past, ui.input(|input| input.stable_dt)), DOCUMENT).is_some()
        {
            ui.ctx().request_repaint();
        }
        shown_view.set(Some(next));
    });
    fixture.settle();
    (fixture, tool, view)
}

#[trial]
fn a_tool_hovering_near_the_canvas_edge_scrolls_the_view_while_the_pointer_rests() {
    let (mut fixture, tool, view) = band_canvas(true);
    let canvas = fixture.by_role(Role::CANVAS)[0].rect;
    let centre = |view: &Rc<Cell<Option<CanvasView>>>| view.get().expect("the canvas ran").center;

    let before = centre(&view);
    fixture.frame(vec![pointer::moved(canvas.center())]).idle(3);
    assert_eq!(centre(&view), before, "a hover away from the edge scrolls nothing");

    fixture.frame(vec![pointer::moved(Pos2::new(canvas.right() - 4.0, canvas.center().y))]);
    let (entered, hovers) = (centre(&view), tool.borrow().hovers);
    fixture.idle(3);
    let rested = centre(&view);
    assert!(rested.x > entered.x, "resting at the right edge scrolls the view right: {entered:?} -> {rested:?}");
    assert_eq!(rested.y, entered.y, "a scroll toward the right edge keeps the height");
    assert!(tool.borrow().hovers >= hovers + 3, "the tool follows the view as it scrolls under the resting pointer");

    let (mut fixture, _, view) = band_canvas(false);
    fixture.frame(vec![pointer::moved(Pos2::new(canvas.right() - 4.0, canvas.center().y))]);
    let entered = centre(&view);
    fixture.idle(3);
    assert_eq!(centre(&view), entered, "a tool that does not autoscroll on hover leaves the view");
}
