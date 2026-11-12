//! Enter and Escape reach a tool holding a pending edit before the canvas has focus (Photoshop commits a transform
//! with Enter without a click on the canvas first); a focused field beside the canvas keeps its own Enter. On the
//! focused canvas, a tool takes the deletes it claims in its state at the end of the last pass: the plain ones, and
//! with Alt held while it keeps the keyboard (Photoshop's polygonal lasso removes its last point while Alt holds the
//! freehand switch). Unclaimed deletes are the app's (Edit > Clear, Fill). Enter and Escape reach a tool as `Submit`
//! and `Cancel` with Alt carried, and Shift+Delete is no delete (Photoshop opens Fill, which the app binds). A
//! session control in the canvas body (a crop frame) takes the arrows, Enter and Escape before the tool, with the
//! canvas focused or nothing focused; with none shown, the arrows reach the tool with nothing focused. A tool that
//! claims the nudges declares Increment and Decrement on its node, and an AccessKit step moves what it holds as the
//! arrow toward or away from the reading end does.

use craft_behavior::geometry::CropBox;
use craft_behavior::kurbo::{Point, Rect};
use craft_behavior::{CancelReason, CanvasView};
use craft_ui::canvas::{CanvasTool, CropOverlay, ToolAccess, ToolAction, ToolClaims, ToolContext, ToolKey, ToolPath, ToolPointer};
use craft_ui::contract::{Bind, Show as _};
use craft_ui::primitives::Draw;
use craft_ui::testing::access::Declared;
use craft_ui::testing::focus::{AccessActionKind, key_release};
use craft_ui::testing::input::key_press;
use craft_ui::testing::{ToolFixture, pointer, trial};
use egui::{Key, Modifiers};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

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

/// A tool that takes every key the host offers it.
#[derive(Default)]
struct Taker {
    seen: Vec<ToolKey>,
}

impl CanvasTool for Taker {
    fn press(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) -> ToolAction {
        ToolAction::Ignored
    }

    fn drag(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {}

    fn release(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {}

    fn cancel(&mut self, _: CancelReason, _: &mut ToolContext<'_>) {}

    fn key(&mut self, key: &ToolKey, _: &mut ToolContext<'_>) -> bool {
        self.seen.push(key.clone());
        true
    }

    fn paint(&self, _: &Draw<'_>, _: &ToolContext<'_>) {}

    fn command_form(&self) -> ToolPath {
        ToolPath::Command("the tool's form")
    }
}

/// What the crop frame reported: applied, discarded.
type Reports = Rc<Cell<(bool, bool)>>;

/// A canvas at 100 % with the document point (200, 150) at its centre, the `Taker` tool and, when `frame`, a crop
/// frame over (100, 80)-(300, 220).
fn session(frame: bool) -> (ToolFixture<Taker>, Rc<RefCell<CropBox>>, Reports) {
    let crop = Rc::new(RefCell::new(CropBox::new(Rect::new(100.0, 80.0, 300.0, 220.0), 0.0)));
    let reports: Reports = Rc::new(Cell::new((false, false)));
    let (c, r) = (Rc::clone(&crop), Rc::clone(&reports));
    let view = |rect| CanvasView::new(rect, Point::new(200.0, 150.0), 1.0);
    let canvas = ToolFixture::with_view("Document", Taker::default(), view, move |ui, body| {
        if !frame {
            return;
        }
        let out = CropOverlay::new("Crop", Bind::bound(&mut *c.borrow_mut())).show_at(ui, body.rect);
        let (applied, discarded) = r.get();
        r.set((applied || out.apply, discarded || out.discard));
    });
    (canvas, crop, reports)
}

#[trial]
fn a_crop_frame_takes_the_arrows_enter_and_escape_before_the_tool() {
    let (mut canvas, crop, reports) = session(true);
    let start = crop.borrow().rect;
    // Nothing focused: the window's arrows and Escape reach the frame.
    canvas.fixture().key(Key::ArrowRight, Modifiers::NONE).settle();
    assert_eq!(crop.borrow().rect, start + craft_behavior::kurbo::Vec2::new(1.0, 0.0), "the arrow moves the whole frame");
    canvas.fixture().key(Key::Escape, Modifiers::NONE).settle();
    assert_eq!(reports.get(), (false, true), "Escape discards");
    // The canvas focused: it is the first Tab stop, the frame the next.
    canvas.fixture().key(Key::Tab, Modifiers::NONE).settle();
    assert_eq!(canvas.fixture().focused(), Some(canvas.canvas_id()), "Tab focuses the canvas");
    canvas.fixture().key(Key::ArrowDown, Modifiers::SHIFT).settle();
    assert_eq!(crop.borrow().rect, start + craft_behavior::kurbo::Vec2::new(1.0, 10.0), "Shift moves the frame by ten");
    canvas.fixture().key(Key::Enter, Modifiers::NONE).settle();
    assert_eq!(reports.get(), (true, true), "Enter applies");
    assert!(canvas.tool().seen.is_empty(), "the tool gets none of the frame's keys: {:?}", canvas.tool().seen);
}

#[trial]
fn with_no_session_control_the_arrows_and_enter_reach_the_tool_with_nothing_focused() {
    let (mut canvas, ..) = session(false);
    canvas.fixture().key(Key::ArrowRight, Modifiers::NONE).key(Key::Enter, Modifiers::NONE).settle();
    let seen = &canvas.tool().seen;
    assert!(matches!(seen[..], [ToolKey::Nudge { .. }, ToolKey::Submit]), "{seen:?}");
}

/// A marker the arrows move one unit along x while `held`; it describes itself as `Marker` at its place.
struct Marker {
    held: bool,
    x: f64,
}

impl CanvasTool for Marker {
    fn claims(&self) -> ToolClaims {
        if self.held { ToolClaims::NONE.nudges() } else { ToolClaims::NONE }
    }

    fn press(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) -> ToolAction {
        ToolAction::Ignored
    }

    fn drag(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {}

    fn release(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {}

    fn cancel(&mut self, _: CancelReason, _: &mut ToolContext<'_>) {}

    fn key(&mut self, key: &ToolKey, _: &mut ToolContext<'_>) -> bool {
        let ToolKey::Nudge { direction, .. } = key else { return false };
        self.x += f64::from(direction.x);
        true
    }

    fn paint(&self, _: &Draw<'_>, _: &ToolContext<'_>) {}

    fn access(&self, _: &ToolContext<'_>) -> Option<ToolAccess> {
        Some(ToolAccess::new("Marker").bounds(Rect::new(self.x, 0.0, self.x + 10.0, 10.0)))
    }

    fn command_form(&self) -> ToolPath {
        ToolPath::Command("the marker's form")
    }
}

#[trial]
fn a_tool_that_claims_the_nudges_takes_increment_and_decrement_on_its_node() {
    let mut canvas = ToolFixture::new("Document", Marker { held: false, x: 0.0 });
    canvas.fixture().settle();
    let node = canvas.fixture().by_label("Marker").expect("the tool's node").node;
    let steps = |canvas: &mut ToolFixture<Marker>| {
        let tree = canvas.fixture().tree();
        (tree.declares_key(node, Declared::Increment), tree.declares_key(node, Declared::Decrement))
    };
    assert_eq!(steps(&mut canvas), (false, false), "a tool that holds nothing to nudge declares no step");
    canvas.tool_mut().held = true;
    canvas.fixture().settle();
    assert_eq!(steps(&mut canvas), (true, true), "a tool that claims the nudges declares both steps on its node");
    let target = canvas.fixture().by_label("Marker").expect("the tool's node");
    canvas.fixture().access(AccessActionKind::Increment, &target).settle();
    assert_eq!(canvas.tool().x, 1.0, "Increment moves the marker toward the reading end");
    canvas.fixture().access(AccessActionKind::Decrement, &target).access(AccessActionKind::Decrement, &target).settle();
    assert_eq!(canvas.tool().x, -1.0, "Decrement moves it back");
}
