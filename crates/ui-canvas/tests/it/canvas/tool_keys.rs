//! Enter and Escape reach a tool holding a pending edit before the canvas has focus (Photoshop commits a transform
//! with Enter without a click on the canvas first); a focused field beside the canvas keeps its own Enter. On the
//! focused canvas, a tool takes the deletes it claims in its state at the end of the last pass: the plain ones, and
//! with Alt held while it keeps the keyboard (Photoshop's polygonal lasso removes its last point while Alt holds the
//! freehand switch). Unclaimed deletes are the app's (Edit > Clear, Fill). Enter and Escape reach a tool as `Submit`
//! and `Cancel` with Alt carried, and Shift+Delete is no delete (Photoshop opens Fill, which the app binds).

use craft_behavior::CancelReason;
use craft_ui::canvas::{CanvasTool, ToolAction, ToolClaims, ToolContext, ToolKey, ToolPath, ToolPointer};
use craft_ui::primitives::Draw;
use craft_ui::testing::focus::key_release;
use craft_ui::testing::input::key_press;
use craft_ui::testing::{ToolFixture, pointer, trial};
use egui::{Key, Modifiers};

#[derive(Clone, Copy, Debug, PartialEq)]
enum Seen {
    Delete { forward: bool },
    Submit { alt: bool },
    Cancel { alt: bool },
    Kept,
}

/// A polygonal outline being drawn: it takes every key the host offers, and claims the keyboard and the deletes while
/// `keeps` (its outline is open). A press opens the outline when its release comes, as a lasso's first click does.
struct Outline {
    keeps: bool,
    seen: Vec<Seen>,
}

impl Outline {
    fn new(keeps: bool) -> Self {
        Outline { keeps, seen: Vec::new() }
    }
}

impl CanvasTool for Outline {
    fn claims(&self) -> ToolClaims {
        if self.keeps { ToolClaims::NONE.keyboard().deletes() } else { ToolClaims::NONE }
    }

    fn press(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) -> ToolAction {
        ToolAction::Claimed
    }

    fn drag(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {}

    fn release(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {
        self.keeps = true;
    }

    fn cancel(&mut self, _: CancelReason, _: &mut ToolContext<'_>) {}

    fn key(&mut self, key: &ToolKey, cx: &mut ToolContext<'_>) -> bool {
        let seen = match key {
            ToolKey::Delete { forward, .. } => Seen::Delete { forward: *forward },
            ToolKey::Submit => Seen::Submit { alt: cx.mods.alt },
            ToolKey::Cancel => Seen::Cancel { alt: cx.mods.alt },
            ToolKey::Kept(_) => Seen::Kept,
            _ => return false,
        };
        self.seen.push(seen);
        true
    }

    fn paint(&self, _: &Draw<'_>, _: &ToolContext<'_>) {}

    fn command_form(&self) -> ToolPath {
        ToolPath::Command("the Polygonal Lasso's selection form")
    }
}

#[trial]
fn a_tool_whose_release_opens_an_outline_takes_alt_backspace_in_the_next_frame() {
    let mut canvas = ToolFixture::new("Document", Outline::new(false));
    canvas.fixture().settle();
    let at = canvas.canvas().center();
    canvas.press_at(at);
    // The key comes in the frame right after the release: the claim is made from the tool's state after it.
    canvas.fixture().frame(vec![pointer::release(at)]).hold(Modifiers::ALT);
    canvas.fixture().frame(vec![key_press(Key::Backspace, Modifiers::ALT)]).frame(vec![key_release(Key::Backspace, Modifiers::ALT)]);
    canvas.fixture().hold(Modifiers::NONE).settle();
    assert_eq!(canvas.tool().seen, [Seen::Delete { forward: false }], "the outline the release opened takes Alt+Backspace");
}
