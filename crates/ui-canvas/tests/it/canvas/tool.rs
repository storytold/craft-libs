//! `CanvasTool`, `ToolHost` and `tool_pick` (8.3): a drag is one `Begin`...`Commit`, Escape cancels it once, a press
//! on a canvas control never reaches the tool, and a group's letter picks and cycles its tools.

use craft_behavior::{CancelReason, GesturePhase, PointerButton};
use craft_keymap::Mods;
use craft_ui::canvas::{CanvasTool, CropOverlay, ToolAction, ToolClaims, ToolContext, ToolHost, ToolKey, ToolPath, ToolPointer};
use craft_ui::contract::{Bind, Show as _};
use craft_ui::primitives::Draw;
use craft_ui::testing::{ToolFixture, trial};
use egui::{Rect, Vec2};
use std::cell::RefCell;
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
