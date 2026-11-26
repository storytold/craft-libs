//! [`CanvasTool::keeps_press_on_escape`] (8.3): the Escape of a drag whose tool keeps its press is that tool's key,
//! not the framework's cancel. The press stays open, its moves and its release keep coming, and the tool ends the
//! drag its own way or goes on with it. A tool that does not keep its press is cancelled exactly as before, and its
//! Escape reaches `key` as nothing. A key-owning layer open over the canvas outranks the claim, and an Escape that
//! arrives in the same frame as the release reaches nobody.

use craft_behavior::kurbo::{Point, Size};
use craft_behavior::{CancelReason, CanvasView, GesturePhase};
use craft_ui::buttons::Button;
use craft_ui::canvas::{Canvas, CanvasTool, ToolAction, ToolContext, ToolHost, ToolKey, ToolPath, ToolPointer, tool_pass};
use craft_ui::contract::overlay::{Dismiss, Outside};
use craft_ui::contract::{Bind, Show as _, ShowContainer as _};
use craft_ui::popover::Popover;
use craft_ui::primitives::Draw;
use craft_ui::runtime::access::Role;
use craft_ui::testing::input::key_press;
use craft_ui::testing::{Fixture, ToolFixture, pointer, trial};
use egui::{Key, Modifiers, Pos2, Vec2};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

const DOCUMENT: Size = Size::new(1200.0, 900.0);

/// The drag's length: four steps of its own from the press.
const BY: Vec2 = Vec2::new(60.0, 40.0);

/// What the host told a [`BoxTool`], in order.
#[derive(Clone, Debug, PartialEq)]
enum Seen {
    Press,
    Drag,
    Release,
    Cancelled(CancelReason),
    Key(ToolKey),
}

/// A box sized by a drag, as the Rectangular Marquee and the reference application's Red Eye box are: every press it
/// takes is a drag until the release. `keeps` says whether the tool keeps its press on Escape, `takes` whether it
/// takes the key the host offers it.
#[derive(Default)]
struct BoxTool {
    keeps: bool,
    takes: bool,
    seen: Vec<Seen>,
}

impl BoxTool {
    /// What the tool was told.
    fn seen(&self) -> Vec<Seen> {
        self.seen.clone()
    }

    /// How many of what the tool was told match `want`.
    fn count(&self, want: impl Fn(&Seen) -> bool) -> usize {
        self.seen.iter().filter(|seen| want(seen)).count()
    }

    /// How many times the Escape reached it as its `Cancel` key.
    fn cancels(&self) -> usize {
        self.count(|seen| matches!(seen, Seen::Key(ToolKey::Cancel)))
    }
}

impl CanvasTool for BoxTool {
    fn keeps_press_on_escape(&self) -> bool {
        self.keeps
    }

    fn press(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) -> ToolAction {
        self.seen.push(Seen::Press);
        ToolAction::Claimed
    }

    fn drag(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {
        self.seen.push(Seen::Drag);
    }

    fn release(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {
        self.seen.push(Seen::Release);
    }

    fn cancel(&mut self, reason: CancelReason, _: &mut ToolContext<'_>) {
        self.seen.push(Seen::Cancelled(reason));
    }

    fn key(&mut self, key: &ToolKey, _: &mut ToolContext<'_>) -> bool {
        self.seen.push(Seen::Key(key.clone()));
        self.takes
    }

    fn paint(&self, _: &Draw<'_>, _: &ToolContext<'_>) {}

    fn command_form(&self) -> ToolPath {
        ToolPath::Command("the Box... command")
    }
}

/// Presses at the canvas's centre, moves halfway, presses Escape, moves on and releases.
fn drag_around_escape(canvas: &mut ToolFixture<BoxTool>) {
    let from = canvas.canvas().center();
    canvas.press_at(from);
    canvas.move_to(from + BY * 0.25);
    canvas.move_to(from + BY * 0.5);
    canvas.escape();
    canvas.move_to(from + BY * 0.75);
    canvas.move_to(from + BY);
    canvas.release_at(from + BY);
}

/// A tool that keeps its press on Escape: the Escape is its `Cancel` key, the drag goes on, and the release commits.
#[trial]
fn the_escape_of_a_keeping_tools_drag_is_its_cancel_key_and_its_release_still_commits() {
    let mut canvas = ToolFixture::new("Document", BoxTool { keeps: true, takes: true, seen: Vec::new() });
    drag_around_escape(&mut canvas);
    let seen = canvas.tool().seen();
    assert_eq!(canvas.tool().cancels(), 1, "the Escape reaches the tool as its Cancel key once: {seen:?}");
    assert_eq!(canvas.tool().count(|seen| matches!(seen, Seen::Cancelled(_))), 0, "the host cancels nothing: {seen:?}");
    assert_eq!(canvas.tool().count(|seen| matches!(seen, Seen::Release)), 1, "the release commits: {seen:?}");
    let at = seen.iter().position(|seen| matches!(seen, Seen::Key(ToolKey::Cancel))).expect("the tool read its Cancel");
    assert!(seen[at + 1..].iter().all(|seen| matches!(seen, Seen::Drag | Seen::Release)), "the drag goes on after the Escape: {seen:?}");
    assert!(seen[at + 1..].iter().any(|seen| matches!(seen, Seen::Drag)), "its moves still reach the tool: {seen:?}");
    let phases = canvas.gestures();
    assert_eq!(phases.iter().filter(|phase| **phase == GesturePhase::Begin).count(), 1, "one Begin: {phases:?}");
    assert_eq!(phases.iter().filter(|phase| **phase == GesturePhase::Cancel).count(), 0, "no Cancel phase: {phases:?}");
    assert_eq!(phases.last(), Some(&GesturePhase::Commit), "the gesture commits: {phases:?}");
}

/// The same drag on a tool that does not keep its press: Escape cancels it once, no key reaches the tool, and the
/// release after the Escape reaches nothing. This is the behaviour of every tool today, pinned unchanged.
#[trial]
fn a_tool_that_does_not_keep_its_press_on_escape_is_cancelled_as_before() {
    let mut canvas = ToolFixture::new("Document", BoxTool { keeps: false, takes: true, seen: Vec::new() });
    drag_around_escape(&mut canvas);
    let seen = canvas.tool().seen();
    assert_eq!(canvas.tool().count(|seen| matches!(seen, Seen::Key(_))), 0, "no key reaches the tool: {seen:?}");
    assert_eq!(canvas.tool().count(|seen| matches!(seen, Seen::Cancelled(CancelReason::Escape))), 1, "Escape cancels once: {seen:?}");
    assert_eq!(canvas.tool().count(|seen| matches!(seen, Seen::Release)), 0, "the release reaches nothing: {seen:?}");
    let at = seen.iter().position(|seen| matches!(seen, Seen::Cancelled(_))).expect("the tool is cancelled");
    assert!(seen[at + 1..].is_empty(), "nothing follows the cancel: {seen:?}");
    let phases = canvas.gestures();
    assert_eq!(phases.iter().filter(|phase| **phase == GesturePhase::Cancel).count(), 1, "one Cancel phase: {phases:?}");
    assert_eq!(phases.iter().filter(|phase| **phase == GesturePhase::Commit).count(), 0, "no Commit: {phases:?}");
}

/// The boundary of the rule: the host holds the plain Escape only, so an Escape with Shift held cancels a keeping
/// tool's drag exactly as it cancels every other tool's (the measured case is the plain key).
#[trial]
fn a_keeping_tools_drag_is_cancelled_by_an_escape_with_shift_held() {
    let mut canvas = ToolFixture::new("Document", BoxTool { keeps: true, takes: true, seen: Vec::new() });
    let from = canvas.canvas().center();
    canvas.press_at(from);
    canvas.move_to(from + BY * 0.5);
    canvas.fixture().key(Key::Escape, Modifiers::SHIFT).settle();
    canvas.move_to(from + BY);
    canvas.release_at(from + BY);
    let seen = canvas.tool().seen();
    assert_eq!(canvas.tool().count(|seen| matches!(seen, Seen::Key(_))), 0, "no key reaches the tool: {seen:?}");
    assert_eq!(canvas.tool().count(|seen| matches!(seen, Seen::Cancelled(CancelReason::Escape))), 1, "Shift+Escape cancels: {seen:?}");
    assert_eq!(canvas.tool().count(|seen| matches!(seen, Seen::Release)), 0, "the release reaches nothing: {seen:?}");
    assert_eq!(canvas.gestures().iter().filter(|phase| **phase == GesturePhase::Cancel).count(), 1, "one Cancel phase: {:?}", canvas.gestures());
}

/// A canvas whose app reads the window's intents after the tool host, last in its UI: the fixture, how many Cancels
/// the window took, and the tool it runs.
fn window_reading_canvas(tool: BoxTool) -> (Fixture, Rc<Cell<u32>>, Rc<RefCell<BoxTool>>) {
    let tool = Rc::new(RefCell::new(tool));
    let cancels = Rc::new(Cell::new(0));
    let (shown, seen) = (Rc::clone(&tool), Rc::clone(&cancels));
    let mut fixture = Fixture::new(move |ui| {
        let room = ui.available_rect_before_wrap();
        let _ = Canvas::new("Document").document(DOCUMENT).view(CanvasView::new(room, Point::new(600.0, 450.0), 1.0)).show(ui, |ui, body| {
            if let Some(pass) = tool_pass(&body.env) {
                let _ = ToolHost::new().run(ui, &pass, &mut *shown.borrow_mut());
            }
        });
        craft_ui::runtime::keys::with_window_intents(ui.ctx(), false, |window| {
            let took = window.take(craft_behavior::Intent::Cancel);
            seen.set(seen.get() + u32::from(took));
            took
        });
    });
    fixture.settle();
    (fixture, cancels, tool)
}

/// The keeping tool that leaves the `Cancel` it is offered still keeps its drag, and the Escape went to the tool
/// alone: the window's intents take no Cancel to clear or close anything with, and the release commits.
#[trial]
fn a_keeping_tool_that_leaves_the_escape_reaches_no_other_taker_and_still_commits() {
    let (mut fixture, cancels, tool) = window_reading_canvas(BoxTool { keeps: true, takes: false, seen: Vec::new() });
    let from: Pos2 = fixture.by_role(Role::CANVAS).remove(0).center();
    fixture.frame(vec![pointer::moved(from)]);
    fixture.frame(vec![pointer::press(from)]);
    fixture.frame(vec![pointer::moved(from + BY * 0.5)]);
    cancels.set(0);
    fixture.frame(vec![key_press(Key::Escape, Modifiers::NONE)]).settle();
    assert_eq!(cancels.get(), 0, "the window takes no Cancel from the keeping tool's Escape");
    assert_eq!(tool.borrow().cancels(), 1, "the tool was still offered its Cancel: {:?}", tool.borrow().seen());
    fixture.frame(vec![pointer::moved(from + BY)]);
    fixture.frame(vec![pointer::release(from + BY)]).settle();
    let seen = tool.borrow().seen();
    assert_eq!(tool.borrow().count(|seen| matches!(seen, Seen::Cancelled(_))), 0, "nothing cancelled the drag: {seen:?}");
    assert_eq!(seen.last(), Some(&Seen::Release), "the release commits: {seen:?}");
}

/// A layer that owns Escape without holding the focus outranks the claim: the host holds no Escape for a gesture
/// under an open key-owning layer, so the Escape is the press's again and the drag is cancelled as it is for every
/// other tool. The layer keeps its own Escape rule, which is to stay while a press is captured.
#[trial]
fn a_layer_keeping_escape_over_the_canvas_ends_the_keeping_tools_claim() {
    let open = Rc::new(Cell::new(false));
    let shown = Rc::clone(&open);
    let mut canvas = ToolFixture::with_body("Document", BoxTool { keeps: true, takes: true, seen: Vec::new() }, move |ui, _| {
        // The button sits in the body's corner and the popover hangs below it, clear of the drag at the centre.
        let opener = Button::new("Options").show(ui);
        let mut is_open = shown.get();
        Popover::new("Kept open", Bind::bound(&mut is_open)).dismiss(Dismiss::new().outside(Outside::Keep)).opened_by(&opener).show(ui, |ui, _| {
            let _ = Button::new("Row").show(ui);
        });
        shown.set(is_open);
    });
    canvas.fixture().settle();
    open.set(true);
    canvas.fixture().idle(4);
    assert!(open.get(), "the popover stands by itself");
    drag_around_escape(&mut canvas);
    let seen = canvas.tool().seen();
    assert!(open.get(), "the popover keeps its own Escape rule under a captured press: {seen:?}");
    assert_eq!(canvas.tool().cancels(), 0, "no Cancel key for the tool under the layer: {seen:?}");
    assert_eq!(canvas.tool().count(|seen| matches!(seen, Seen::Cancelled(CancelReason::Escape))), 1, "the Escape is the press's: {seen:?}");
    assert_eq!(canvas.tool().count(|seen| matches!(seen, Seen::Release)), 0, "the release reaches nothing: {seen:?}");
    let phases = canvas.gestures();
    assert_eq!(phases.iter().filter(|phase| **phase == GesturePhase::Cancel).count(), 1, "one Cancel phase: {phases:?}");
    assert_eq!(phases.iter().filter(|phase| **phase == GesturePhase::Commit).count(), 0, "no Commit: {phases:?}");
}

/// An Escape carried in the same frame as the release: the release ends the gesture, the claim ends with it, and the
/// Escape reaches nobody - not the tool as a key, and not the framework as a cancel.
#[trial]
fn an_escape_in_the_same_frame_as_the_release_of_a_keeping_drag_reaches_nobody() {
    let mut canvas = ToolFixture::new("Document", BoxTool { keeps: true, takes: true, seen: Vec::new() });
    let from = canvas.canvas().center();
    let to = from + BY;
    canvas.press_at(from);
    canvas.move_to(from + BY * 0.5);
    canvas.fixture().frame(vec![key_press(Key::Escape, Modifiers::NONE), pointer::release(to)]).idle(2);
    let seen = canvas.tool().seen();
    assert_eq!(canvas.tool().cancels(), 0, "no Cancel key: {seen:?}");
    assert_eq!(canvas.tool().count(|seen| matches!(seen, Seen::Cancelled(_))), 0, "not a cancel: {seen:?}");
    assert_eq!(canvas.tool().count(|seen| matches!(seen, Seen::Release)), 1, "the release commits: {seen:?}");
    let phases = canvas.gestures();
    assert_eq!(phases.iter().filter(|phase| **phase == GesturePhase::Commit).count(), 1, "one Commit: {phases:?}");
    assert_eq!(phases.iter().filter(|phase| **phase == GesturePhase::Cancel).count(), 0, "no Cancel phase: {phases:?}");
}
