//! Enter and Escape reach a tool holding a pending edit before the canvas has focus (Photoshop commits a transform
//! with Enter without a click on the canvas first); a focused field beside the canvas keeps its own Enter. On the
//! focused canvas or with nothing focused, a tool takes the deletes it claims in its state at the end of the last pass: the plain ones, and
//! with Alt held while it keeps the keyboard (Photoshop's polygonal lasso removes its last point while Alt holds the
//! freehand switch); a modal open over the canvas keeps them from it. Unclaimed deletes are the app's (Edit > Clear, Fill). Enter and Escape reach a tool as `Submit`
//! and `Cancel` with Alt carried, and Shift+Delete is no delete (Photoshop opens Fill, which the app binds). A
//! session control in the canvas body (a crop frame) takes the arrows, Enter and Escape before the tool, with the
//! canvas focused or nothing focused; with none shown, the arrows reach the tool with nothing focused. A tool that
//! claims the nudges declares Increment and Decrement on its node, and an AccessKit step moves what it holds as the
//! arrow toward or away from the reading end does. A key the tool claims, pressed under a held pointer, drags again.

use craft_behavior::geometry::CropBox;
use craft_behavior::kurbo::{Point, Rect};
use craft_behavior::{CancelReason, CanvasView};
use craft_keymap::{Chord, Key as ChordKey, Mods};
use craft_ui::buttons::Button;
use craft_ui::canvas::{CanvasTool, CropOverlay, ToolAccess, ToolAction, ToolClaims, ToolContext, ToolKey, ToolPath, ToolPointer};
use craft_ui::contract::{Bind, Show as _, ShowContainer as _};
use craft_ui::dialog::Dialog;
use craft_ui::display::Text;
use craft_ui::popover::Popover;
use craft_ui::primitives::Draw;
use craft_ui::runtime::commands::{self, CommandSet as _, EditCommands, FireSource};
use craft_ui::runtime::frame::take_output;
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

/// With nothing focused, the deletes a tool claims are its own, as the arrows, Enter and Escape are: they never reach
/// the app's Edit > Clear while the tool has something to delete.
#[trial]
fn a_tool_takes_the_deletes_it_claims_with_nothing_focused() {
    let mut canvas = ToolFixture::new("Document", Outline::new(true));
    canvas.fixture().settle();
    assert_eq!(canvas.fixture().focused(), None, "nothing has focus");
    canvas.fixture().key(Key::Delete, Modifiers::NONE).key(Key::Backspace, Modifiers::NONE).settle();
    assert_eq!(canvas.tool().seen, [Seen::Delete { forward: true }, Seen::Delete { forward: false }]);
}

/// A modal layer open over the canvas keeps the deletes a tool claims from it while nothing is focused, as it keeps
/// the arrows: they are the modal's or the app's. Once the modal closes, the tool takes them again.
#[trial]
fn a_modal_keeps_the_deletes_a_tool_claims_with_nothing_focused() {
    let modal = Rc::new(Cell::new(true));
    let shown = Rc::clone(&modal);
    let mut canvas = ToolFixture::with_body("Document", Outline::new(true), move |ui, _| {
        let mut open = shown.get();
        if open {
            Dialog::new("Over the canvas").opened(Bind::bound(&mut open)).show(ui, |ui, _| {
                Text::new("Modal").show(ui);
            });
        }
    });
    canvas.fixture().settle();
    assert_eq!(canvas.fixture().focused(), None, "nothing has focus under the modal");
    canvas.fixture().key(Key::Delete, Modifiers::NONE).settle();
    assert_eq!(canvas.tool().seen, [], "the modal keeps the claimed Delete from the tool");
    modal.set(false);
    canvas.fixture().settle();
    canvas.fixture().key(Key::Delete, Modifiers::NONE).settle();
    assert_eq!(canvas.tool().seen, [Seen::Delete { forward: true }], "with the modal closed the tool takes it");
}

/// Edit ▸ Delete from a menu or the palette, while the focused canvas's tool claims Delete, reaches the tool as that
/// key, as the key does, and runs no app command; a tool that claims no delete gets none.
#[trial]
fn a_fired_command_whose_key_the_tool_claims_reaches_the_tool_as_that_key() {
    let mut canvas = ToolFixture::new("Document", Outline::new(true));
    canvas.fixture().settle();
    canvas.fixture().key(Key::Tab, Modifiers::NONE).settle();
    assert_eq!(canvas.fixture().focused(), Some(canvas.canvas_id()), "Tab focuses the canvas");
    take_output(canvas.fixture().ctx());
    commands::fire(canvas.fixture().ctx(), EditCommands::Delete, FireSource::Menu);
    canvas.fixture().settle();
    assert_eq!(canvas.tool().seen, [Seen::Delete { forward: true }], "the tool took Edit ▸ Delete as its Delete");
    assert!(take_output(canvas.fixture().ctx()).fired.is_empty(), "no app command ran");
    canvas.tool_mut().keeps = false;
    canvas.fixture().settle();
    commands::fire(canvas.fixture().ctx(), EditCommands::Delete, FireSource::Menu);
    canvas.fixture().settle();
    assert_eq!(canvas.tool().seen.len(), 1, "a tool that claims no delete gets none");
}

/// With nothing focused the window's canvas is the key target, as for its nudges: Edit ▸ Delete reaches the tool that
/// claims Delete (a Select All took the focus away), and no app command runs.
#[trial]
fn a_fired_command_whose_key_the_tool_claims_reaches_it_with_nothing_focused() {
    let mut canvas = ToolFixture::new("Document", Outline::new(true));
    canvas.fixture().settle();
    assert_eq!(canvas.fixture().focused(), None, "nothing is focused");
    take_output(canvas.fixture().ctx());
    commands::fire(canvas.fixture().ctx(), EditCommands::Delete, FireSource::Menu);
    canvas.fixture().settle();
    assert_eq!(canvas.tool().seen, [Seen::Delete { forward: true }], "the tool took Edit ▸ Delete as its Delete");
    assert!(take_output(canvas.fixture().ctx()).fired.is_empty(), "no app command ran");
}

/// A command a button fires runs as the button's command, though the tool claims its key: only a menu's or the
/// palette's stands for the key (a toolbar's tool button finishes a gesture the tool holds the keyboard for).
#[trial]
fn a_pointer_fired_command_whose_key_the_tool_claims_runs_as_a_command() {
    let mut canvas = ToolFixture::new("Document", Outline::new(true));
    canvas.fixture().settle();
    assert_eq!(canvas.fixture().focused(), None, "nothing is focused");
    take_output(canvas.fixture().ctx());
    assert!(commands::fire(canvas.fixture().ctx(), EditCommands::Delete, FireSource::Pointer), "Delete runs");
    canvas.fixture().settle();
    assert_eq!(canvas.tool().seen, [], "the tool got no Delete");
    let fired = take_output(canvas.fixture().ctx()).fired;
    assert!(fired.iter().any(|fired| fired.command.id == EditCommands::Delete.id()), "the app's Delete ran: {fired:?}");
}

/// A tool that takes every key the host offers it, and the lone Alt tap while `taps`.
#[derive(Default)]
struct Taker {
    seen: Vec<ToolKey>,
    taps: bool,
}

impl CanvasTool for Taker {
    fn claims(&self) -> ToolClaims {
        if self.taps { ToolClaims::NONE.alt_tap() } else { ToolClaims::NONE }
    }

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
        r.set((applied || out.apply.is_some(), discarded || out.discard));
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

/// A nudge carries the keys held when its arrow went down, in the order of the frame's events: a key let go after the
/// arrow in the same frame was held with it, and one pressed after it was not.
#[trial]
fn a_nudge_carries_the_keys_held_at_its_arrows_press() {
    let (mut canvas, ..) = session(false);
    canvas.fixture().settle();
    let none = Modifiers::NONE;
    canvas.fixture().frame(vec![key_press(Key::Backtick, none)]).settle();
    canvas.fixture().frame(vec![key_press(Key::ArrowRight, none), key_release(Key::Backtick, none), key_release(Key::ArrowRight, none)]).settle();
    canvas.fixture().frame(vec![key_press(Key::ArrowRight, none), key_press(Key::Backtick, none), key_release(Key::ArrowRight, none)]).settle();
    canvas.fixture().frame(vec![key_release(Key::Backtick, none)]).settle();
    let backtick: Vec<bool> = canvas
        .tool()
        .seen
        .iter()
        .filter_map(|key| match key {
            ToolKey::Nudge { held, .. } => Some(held.contains(&ChordKey::Backtick)),
            _ => None,
        })
        .collect();
    assert_eq!(backtick, [true, false], "Backtick is held with the first arrow and not with the second");
}

#[trial]
fn a_tool_that_claims_the_alt_tap_gets_it() {
    let (mut canvas, ..) = session(false);
    canvas.fixture().settle();
    let tap = |canvas: &mut ToolFixture<Taker>| {
        canvas
            .fixture()
            .frame(vec![egui::Event::ModifiersChanged(Modifiers::ALT)])
            .frame(vec![egui::Event::ModifiersChanged(Modifiers::NONE)])
            .settle();
    };
    tap(&mut canvas);
    assert!(canvas.tool().seen.is_empty(), "a tool that does not claim the tap gets none: {:?}", canvas.tool().seen);
    canvas.tool_mut().taps = true;
    canvas.fixture().settle();
    tap(&mut canvas);
    let seen = &canvas.tool().seen;
    assert!(matches!(seen[..], [ToolKey::AltTap]), "{seen:?}");
}

/// Escape reaches the active tool as `Cancel` with nothing focused and with the canvas focused, so a tool steps back
/// out of what it holds (a comment or fill tool leaves for Select) without a gesture to cancel.
#[trial]
fn escape_reaches_the_tool_as_cancel_with_nothing_or_the_canvas_focused() {
    let (mut canvas, ..) = session(false);
    canvas.fixture().settle();
    canvas.fixture().key(Key::Escape, Modifiers::NONE).settle();
    assert_eq!(canvas.tool().seen, [ToolKey::Cancel], "nothing focused: the window's Escape is the tool's");
    canvas.fixture().key(Key::Tab, Modifiers::NONE).settle();
    assert_eq!(canvas.fixture().focused(), Some(canvas.canvas_id()), "Tab focuses the canvas");
    canvas.fixture().key(Key::Escape, Modifiers::NONE).settle();
    assert_eq!(canvas.tool().seen, [ToolKey::Cancel, ToolKey::Cancel], "the focused canvas's Escape is the tool's");
}

/// The chords `Claimer` takes through `ToolClaims::keys`.
fn enter_and_escape() -> &'static [Chord] {
    static KEYS: std::sync::OnceLock<[Chord; 2]> = std::sync::OnceLock::new();
    KEYS.get_or_init(|| [Chord::new(Mods::NONE, ChordKey::Enter), Chord::new(Mods::NONE, ChordKey::Escape)])
}

/// A tool that claims Enter and Escape as single chords and takes every key it is offered.
#[derive(Default)]
struct Claimer {
    seen: Vec<ToolKey>,
}

impl CanvasTool for Claimer {
    fn claims(&self) -> ToolClaims {
        ToolClaims::NONE.keys(enter_and_escape())
    }

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
        ToolPath::Command("the claimer's form")
    }
}

#[trial]
fn an_open_layer_takes_enter_and_escape_before_a_tools_claim() {
    let open = Rc::new(Cell::new(false));
    let shown = Rc::clone(&open);
    let mut canvas = ToolFixture::with_body("Document", Claimer::default(), move |ui, _| {
        let opener = Button::new("Options").show(ui);
        let mut is_open = shown.get();
        Popover::new("Options panel", Bind::bound(&mut is_open)).opened_by(&opener).show(ui, |ui, _| {
            let _ = Button::new("Item").show(ui);
        });
        shown.set(is_open);
    });
    canvas.fixture().settle();
    open.set(true);
    canvas.fixture().idle(4);
    // The layer's keys come first: Enter goes to it, and Escape closes it.
    canvas.fixture().key(Key::Enter, Modifiers::NONE).key(Key::Escape, Modifiers::NONE).settle();
    assert!(canvas.tool().seen.is_empty(), "the open layer took both keys: {:?}", canvas.tool().seen);
    assert!(!open.get(), "Escape closed the layer");
    canvas.fixture().idle(10);
    // Escape returned focus to the opener, whose Enter it is; the tool's chords hold while the canvas has focus, and
    // the canvas is the stop before the opener.
    canvas.fixture().key(Key::Tab, Modifiers::SHIFT).settle();
    assert_eq!(canvas.fixture().focused_stop().map(|(id, _)| id), Some(canvas.canvas_id()), "Shift+Tab focuses the canvas");
    canvas.fixture().key(Key::Enter, Modifiers::NONE).key(Key::Escape, Modifiers::NONE).settle();
    let kept = canvas.tool().seen.iter().filter(|key| matches!(key, ToolKey::Kept(_))).count();
    assert_eq!(kept, 2, "with the layer closed both chords reach the tool as Kept: {:?}", canvas.tool().seen);
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

/// A ring sized by a drag: it claims the keyboard while its press is down, and each drag call records whether
/// Backtick was held (a flare's rings place anew while it is).
#[derive(Default)]
struct Ring {
    busy: bool,
    backtick: Vec<bool>,
}

impl CanvasTool for Ring {
    fn claims(&self) -> ToolClaims {
        if self.busy { ToolClaims::NONE.keyboard() } else { ToolClaims::NONE }
    }

    fn press(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) -> ToolAction {
        self.busy = true;
        ToolAction::Claimed
    }

    fn drag(&mut self, _: &ToolPointer, cx: &mut ToolContext<'_>) {
        self.backtick.push(cx.held(ChordKey::Backtick));
    }

    fn release(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {
        self.busy = false;
    }

    fn cancel(&mut self, _: CancelReason, _: &mut ToolContext<'_>) {}

    fn key(&mut self, key: &ToolKey, _: &mut ToolContext<'_>) -> bool {
        matches!(key, ToolKey::Kept(_))
    }

    fn paint(&self, _: &Draw<'_>, _: &ToolContext<'_>) {}

    fn command_form(&self) -> ToolPath {
        ToolPath::Command("the ring's form")
    }
}

/// A key the tool claims, pressed or let go under a still pointer, changes what the drag reads: the held pointer gets
/// a drag in that pass, as for a modifier change.
#[trial]
fn a_claimed_key_under_a_held_pointer_drags_again() {
    let mut canvas = ToolFixture::new("Document", Ring::default());
    canvas.fixture().settle();
    let at = canvas.canvas().center() + egui::vec2(40.0, 0.0);
    canvas.press_at(canvas.canvas().center()).move_to(at);
    canvas.fixture().frame(vec![key_press(Key::Backtick, Modifiers::NONE)]).frame(Vec::new());
    canvas.fixture().frame(vec![key_release(Key::Backtick, Modifiers::NONE)]).frame(Vec::new());
    let seen = canvas.tool().backtick.clone();
    assert!(seen.ends_with(&[false, true, false]), "the press and the release each drag once: {seen:?}");
}
