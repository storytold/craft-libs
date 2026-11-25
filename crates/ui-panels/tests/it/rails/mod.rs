//! The rail's own logic: which tool a slot shows, what the rail remembers, where a drop goes, the flyout's hold and
//! click, a press dragged away, the flyout's tear-off bar and the window a torn-off group floats in.

pub(crate) mod drawer;
pub(crate) mod locate;

use craft_behavior::{GesturePhase, Mixed};
use craft_ui::contract::overlay::{PlacementChange, PlacementKind};
use craft_ui::contract::{Props as _, Role, Show as _};
use craft_ui::docking::{FloatingGroup, GroupDrag};
use craft_ui::rails::{FlyoutButton, RailTool};
use craft_ui::runtime::commands::EditCommands;
use craft_ui::runtime::drag;
use craft_ui::runtime::frame::take_output;
use craft_ui::testing::pointer::{moved, press, secondary};
use craft_ui::testing::{Fixture, Target, trial};
use egui::{Key, Modifiers, Pos2, Rect, Vec2};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

const UNDO: u8 = 1;
const REDO: u8 = 2;
const CUT: u8 = 3;
/// More than the 0.5 s a press must be held to open the flyout.
const HELD: f64 = 0.7;

fn tools(ids: &[u8]) -> Vec<RailTool<u8>> {
    let command = |id: u8| match id {
        UNDO => EditCommands::Undo,
        REDO => EditCommands::Redo,
        _ => EditCommands::Cut,
    };
    ids.iter().map(|id| RailTool::new(*id, command(*id))).collect()
}

struct Rig {
    fixture: Fixture,
    selected: Rc<RefCell<Vec<u8>>>,
    doubles: Rc<Cell<u32>>,
}

/// A button over the three tools that shows the tool last selected, with `click` as its click command.
fn build(hold_selects: bool, click: Option<EditCommands>) -> Rig {
    let selected = Rc::new(RefCell::new(Vec::new()));
    let doubles = Rc::new(Cell::new(0_u32));
    let shown = Rc::new(Cell::new(UNDO));
    let (seen, counted, current) = (selected.clone(), doubles.clone(), shown);
    let mut fixture = Fixture::new(move |ui| {
        let all = tools(&[UNDO, REDO, CUT]);
        let at = current.get();
        let mut button = FlyoutButton::new(&all, &at).hold_selects(hold_selects);
        if let Some(command) = click {
            button = button.click(command);
        }
        let out = button.show(ui);
        if let Some(tool) = out.select {
            current.set(tool);
            seen.borrow_mut().push(tool);
        }
        if out.double_clicked {
            counted.set(counted.get() + 1);
        }
    });
    fixture.settle();
    Rig { fixture, selected, doubles }
}

fn rig(hold_selects: bool) -> Rig {
    build(hold_selects, None)
}

/// How many commands fired since the last call.
fn fired(rig: &Rig) -> usize {
    take_output(rig.fixture.ctx()).fired.len()
}

fn button(rig: &Rig) -> craft_ui::testing::Target {
    rig.fixture.by_role(Role::BUTTON).into_iter().next().expect("the flyout button shows")
}

#[trial]
fn a_click_selects_the_shown_tool_and_holding_opens_the_flyout() {
    let mut clicked_rig = rig(true);
    let target = button(&clicked_rig);
    clicked_rig.fixture.click(&target);
    clicked_rig.fixture.idle(3);
    assert_eq!(*clicked_rig.selected.borrow(), [UNDO], "a click selects the shown tool");
    assert!(clicked_rig.fixture.by_role(Role::MENU).is_empty(), "a click opens no flyout");

    let mut held = rig(true);
    let target = button(&held);
    held.fixture.press(&target);
    held.fixture.advance(HELD);
    held.fixture.idle(3);
    assert!(!held.fixture.by_role(Role::MENU).is_empty(), "a held press opens the flyout");
}

/// Dragging a press away from the button opens the flyout, and releasing that press over a row picks the row's tool.
#[trial]
fn dragging_a_press_away_opens_the_flyout_and_releasing_over_a_row_picks_it() {
    let mut dragged = rig(true);
    let target = button(&dragged);
    let away = egui::Vec2::new(target.rect.width() * 1.5, 0.0);
    dragged.fixture.drag_hold_at(target.center(), away);
    dragged.fixture.idle(2);
    assert!(!dragged.fixture.by_role(Role::MENU).is_empty(), "a press dragged off the button opens the flyout");
    let row = dragged.fixture.by_label("Redo").expect("the flyout lists Redo");
    dragged.fixture.frame(vec![craft_ui::testing::pointer::moved(row.center())]);
    dragged.fixture.release_at(row.center()).idle(2);
    assert_eq!(*dragged.selected.borrow(), [REDO], "releasing over Redo picks it and nothing else");
}

/// A flyout of a tool low on the rail, too tall for the room under the tool's top, slides up to stay in the window
/// as PhotoCraft's tool flyouts do, rather than opening upward from the tool as a submenu does.
#[trial]
fn the_flyout_of_a_low_tool_slides_up_to_stay_in_the_window() {
    let all: Vec<RailTool<u8>> = (0..12u8).map(|id| RailTool::new(id, EditCommands::Cut)).collect();
    let mut fixture = Fixture::new(move |ui| {
        ui.add_space(440.0);
        FlyoutButton::new(&all, &0).show(ui);
    });
    fixture.settle();
    let target = fixture.by_role(Role::BUTTON).into_iter().next().expect("the flyout button shows");
    fixture.press(&target);
    fixture.advance(HELD);
    fixture.idle(3);
    let flyout = fixture.by_role(Role::MENU).first().expect("a held press opens the flyout").rect;
    let window = fixture.ctx().content_rect();
    assert!(target.rect.top() + flyout.height() > window.bottom(), "the flyout does not fit under the tool's top");
    assert!(window.contains_rect(flyout), "the flyout {flyout:?} stays in the window {window:?}");
    assert!(flyout.bottom() > target.rect.bottom() + 1.0, "it slides: its bottom {} is below the tool's {}", flyout.bottom(), target.rect.bottom());
}

/// A rail button tells assistive technology which tool is the current one: the current tool's button is pressed and
/// every other tool's button is not, as it paints.
#[trial]
fn the_current_tools_button_reports_pressed_and_the_others_not() {
    let (undo, cut) = (tools(&[UNDO]), tools(&[CUT]));
    let mut fixture = Fixture::new(move |ui| {
        FlyoutButton::new(&undo, &UNDO).id_salt("undo").selected(true).show(ui);
        FlyoutButton::new(&cut, &CUT).id_salt("cut").selected(false).show(ui);
    });
    fixture.settle();
    let pressed = |label: &str| {
        let button = fixture.by_label(label).unwrap_or_else(|| panic!("the {label} button shows"));
        fixture.tree().toggled_of(button.node)
    };
    assert_eq!(pressed("Undo"), Some(Mixed::Same(true)), "the current tool's button is pressed");
    assert_eq!(pressed("Cut"), Some(Mixed::Same(false)), "another tool's button reports that it is not");
}

/// With a click command a click, Enter or Space fires it and selects nothing; a hold, a drag away and a right click
/// list the tools, and the release that ends a hold fires nothing.
#[trial]
fn a_click_command_cycles_and_a_hold_lists_the_tools_without_firing_it() {
    let mut clicked = build(false, Some(EditCommands::Paste));
    let target = button(&clicked);
    clicked.fixture.click(&target);
    clicked.fixture.idle(3);
    assert_eq!(fired(&clicked), 1, "a click fires the click command");
    // A pointer click need not leave focus on the button; the keys act on the focused one.
    if clicked.fixture.focused_stop().is_none() {
        clicked.fixture.key(Key::Tab, Modifiers::NONE);
    }
    assert!(clicked.fixture.focused_stop().is_some(), "the button has focus for the keys");
    clicked.fixture.key(Key::Enter, Modifiers::NONE);
    assert_eq!(fired(&clicked), 1, "Enter fires it");
    clicked.fixture.key(Key::Space, Modifiers::NONE);
    assert_eq!(fired(&clicked), 1, "Space fires it");
    assert!(clicked.selected.borrow().is_empty(), "the click command selects no tool");
    assert!(clicked.fixture.by_role(Role::MENU).is_empty(), "a click opens no list");

    let mut held = build(false, Some(EditCommands::Paste));
    let target = button(&held);
    held.fixture.press(&target);
    held.fixture.advance(HELD);
    held.fixture.idle(3);
    assert!(!held.fixture.by_role(Role::MENU).is_empty(), "a hold lists the tools");
    held.fixture.release(&target);
    held.fixture.idle(3);
    assert_eq!(fired(&held), 0, "the release ending a hold fires nothing");
    assert!(held.selected.borrow().is_empty(), "the release ending a hold selects nothing");

    let mut dragged = build(false, Some(EditCommands::Paste));
    let target = button(&dragged);
    let away = egui::Vec2::new(target.rect.width() * 1.5, 0.0);
    dragged.fixture.drag_hold_at(target.center(), away);
    dragged.fixture.idle(2);
    assert!(!dragged.fixture.by_role(Role::MENU).is_empty(), "a drag away lists the tools");
    // Released on the button itself, not over a flyout row: a row release picks that row's tool by design, which
    // the drag trial above covers.
    dragged.fixture.release_at(target.center()).idle(2);
    assert_eq!(fired(&dragged), 0, "the release of a drag fires nothing");

    let mut context = build(false, Some(EditCommands::Paste));
    let at = button(&context).center();
    context.fixture.frame(vec![moved(at)]);
    context.fixture.frame(vec![secondary(at, true)]);
    context.fixture.frame(vec![secondary(at, false)]);
    context.fixture.idle(3);
    assert!(!context.fixture.by_role(Role::MENU).is_empty(), "a right click lists the tools");
    assert_eq!(fired(&context), 0, "a right click fires nothing");
}

/// Alt-click steps the slot to the next enabled tool and wraps; a modified click is never a double click, and a plain
/// click after one starts a new run.
#[trial]
fn alt_click_steps_a_slot_and_a_modified_click_is_never_a_double_click() {
    let mut rig = rig(true);
    let target = button(&rig);
    rig.fixture.hold(Modifiers::ALT);
    for _ in 0..3 {
        rig.fixture.click(&target);
        rig.fixture.idle(2);
    }
    assert_eq!(*rig.selected.borrow(), [REDO, CUT, UNDO], "each Alt-click picks the next tool, wrapping");
    assert!(rig.fixture.by_role(Role::MENU).is_empty(), "an Alt-click opens no flyout");
    assert_eq!(rig.doubles.get(), 0, "quick Alt-clicks are no double click");

    rig.fixture.hold(Modifiers::NONE);
    rig.fixture.click(&target);
    assert_eq!(rig.doubles.get(), 0, "a plain click after an Alt-click starts a new run");
    rig.fixture.click(&target);
    assert_eq!(rig.doubles.get(), 1, "two plain clicks are a double click");
}

/// The name of the flyout's tear-off bar.
const TEAR_OFF: &str = "Tear Off Group";

/// A button over the three tools, with the tear-off bar when `tear_off` holds, and the pointers it reported torn off
/// and the tools it selected.
struct TearRig {
    fixture: Fixture,
    torn: Rc<RefCell<Vec<Pos2>>>,
    selected: Rc<RefCell<Vec<u8>>>,
}

fn tear_rig(tear_off: bool) -> TearRig {
    let (torn, selected) = (Rc::new(RefCell::new(Vec::new())), Rc::new(RefCell::new(Vec::new())));
    let (tears, picks) = (Rc::clone(&torn), Rc::clone(&selected));
    let mut fixture = Fixture::new(move |ui| {
        let all = tools(&[UNDO, REDO, CUT]);
        let mut button = FlyoutButton::new(&all, &UNDO);
        if tear_off {
            button = button.tear_off();
        }
        let out = button.show(ui);
        tears.borrow_mut().extend(out.torn_off);
        picks.borrow_mut().extend(out.select);
    });
    fixture.settle();
    TearRig { fixture, torn, selected }
}

/// The first button of `fixture`.
fn first_button(fixture: &Fixture) -> Target {
    fixture.by_role(Role::BUTTON).into_iter().next().expect("the flyout button shows")
}

/// Opens the flyout of the button at `at` by a right click, which leaves no press held.
fn open_by_context(fixture: &mut Fixture, at: Pos2) {
    fixture.frame(vec![moved(at)]);
    fixture.frame(vec![secondary(at, true)]);
    fixture.frame(vec![secondary(at, false)]);
    fixture.idle(3);
    assert!(!fixture.by_role(Role::MENU).is_empty(), "a right click opens the flyout");
}

fn bar(fixture: &Fixture) -> Target {
    fixture.by_label(TEAR_OFF).expect("the flyout shows its tear-off bar")
}

/// A press on the bar dragged past the slop, a click on it and the release over it of the hold that opened the flyout
/// each close the flyout and report the pointer once; a release over a row picks the row and tears nothing off; a
/// flyout without `tear_off` has no bar.
#[trial]
fn dragging_the_flyout_bar_clicking_it_or_releasing_a_hold_over_it_reports_the_tear_off_once() {
    let away = Vec2::new(0.0, 40.0);
    let mut dragged = tear_rig(true);
    let at = first_button(&dragged.fixture).center();
    open_by_context(&mut dragged.fixture, at);
    let from = bar(&dragged.fixture).center();
    dragged.fixture.drag_hold_at(from, away);
    dragged.fixture.release_at(from + away);
    let torn = dragged.torn.borrow().clone();
    assert_eq!(torn.len(), 1, "a drag on the bar tears the group off once: {torn:?}");
    assert!(torn[0] != from && torn[0].distance(from) <= away.length(), "where the drag passed the slop: {torn:?} from {from:?}");
    assert!(dragged.fixture.by_role(Role::MENU).is_empty(), "the drag closes the flyout");
    assert!(dragged.selected.borrow().is_empty(), "a tear-off selects no tool");

    let mut clicked = tear_rig(true);
    let at = first_button(&clicked.fixture).center();
    open_by_context(&mut clicked.fixture, at);
    let target = bar(&clicked.fixture);
    clicked.fixture.click(&target);
    clicked.fixture.idle(2);
    assert_eq!(*clicked.torn.borrow(), [target.center()], "a click on the bar tears the group off where it was clicked, once");
    assert!(clicked.fixture.by_role(Role::MENU).is_empty(), "the click closes the flyout");

    let mut held = tear_rig(true);
    let target = first_button(&held.fixture);
    held.fixture.press(&target);
    held.fixture.advance(HELD);
    held.fixture.idle(3);
    let over = bar(&held.fixture).center();
    held.fixture.frame(vec![moved(over)]);
    held.fixture.release_at(over).idle(2);
    assert_eq!(*held.torn.borrow(), [over], "the release of the hold over the bar tears the group off, once");
    assert!(held.selected.borrow().is_empty(), "the release over the bar picks no row");
    assert!(held.fixture.by_role(Role::MENU).is_empty(), "the release closes the flyout");

    let mut row = tear_rig(true);
    let target = first_button(&row.fixture);
    row.fixture.press(&target);
    row.fixture.advance(HELD);
    row.fixture.idle(3);
    let redo = row.fixture.by_label("Redo").expect("the flyout lists Redo").center();
    row.fixture.frame(vec![moved(redo)]);
    row.fixture.release_at(redo).idle(2);
    assert!(row.torn.borrow().is_empty(), "a release over a row tears nothing off");
    assert_eq!(*row.selected.borrow(), [REDO], "it picks the row");

    let mut plain = tear_rig(false);
    let at = first_button(&plain.fixture).center();
    open_by_context(&mut plain.fixture, at);
    assert!(plain.fixture.by_label(TEAR_OFF).is_none(), "a flyout without tear_off has no bar");
}

/// Where the pointer holds the window a group floats in, from its top left: on its title row.
const GRAB: Vec2 = Vec2::new(30.0, 8.0);
/// The size the window floats at.
const FLOAT: Vec2 = Vec2::new(140.0, 90.0);

/// What the host of a torn-off group keeps: the window's rect, and the placement changes it reported.
#[derive(Default)]
struct Floated {
    rect: Cell<Option<Rect>>,
    changes: RefCell<Vec<PlacementChange>>,
}

/// A button over the three tools with its tear-off bar, whose host floats a torn-off group in a window of the dock
/// model: in the pass that reports the tear-off the window continues the press at `GRAB`, a pass that runs twice.
fn floating_rig() -> (Fixture, Rc<Floated>) {
    let floated = Rc::new(Floated::default());
    let host = Rc::clone(&floated);
    let mut fixture = Fixture::new(move |ui| {
        let all = tools(&[UNDO, REDO, CUT]);
        let out = FlyoutButton::new(&all, &UNDO).tear_off().show(ui);
        if let Some(pointer) = out.torn_off {
            host.rect.set(Some(Rect::from_min_size(pointer - GRAB, FLOAT)));
            // The second pass of the frame has no input of its own: the press is the window's already.
            if ui.ctx().current_pass_index() == 0 {
                ui.ctx().request_discard("the tear-off pass runs twice");
            }
        }
        let Some(rect) = host.rect.get() else { return };
        let mut group = FloatingGroup::new("Edit", rect).window_of("Edit");
        if out.torn_off.is_some() {
            group = group.continue_move(GRAB);
        }
        let shown = group.show_group(ui, |_, _| {});
        if let Some(change) = shown.output.placed {
            host.rect.set(Some(change.rect));
            host.changes.borrow_mut().push(change);
        }
    });
    fixture.settle();
    (fixture, floated)
}

/// The window a group torn off by a drag of the bar floats in follows the press still held from its first pass,
/// keeping the pointer at the grab point, reports the move and runs the dock drag that docks it until the release
/// commits the move. A group torn off by a click, with no press held, floats where it was put and does not move.
#[trial]
fn a_window_floated_from_a_torn_off_group_follows_the_held_press_and_docks_on_release() {
    let (mut fixture, floated) = floating_rig();
    let at = first_button(&fixture).center();
    open_by_context(&mut fixture, at);
    let from = bar(&fixture).center();
    fixture.frame(vec![moved(from)]);
    fixture.frame(vec![press(from)]);
    let path = [from + Vec2::new(0.0, 12.0), from + Vec2::new(40.0, 30.0), from + Vec2::new(90.0, 60.0)];
    for pointer in path {
        fixture.frame(vec![moved(pointer)]);
        let rect = floated.rect.get().expect("the torn-off group floats in a window");
        assert!((rect.min + GRAB).distance(pointer) < 1.0, "the window keeps the pointer at its grab point: {rect:?} at {pointer:?}");
    }
    assert!(drag::active::<GroupDrag<&'static str>>(fixture.ctx()).is_some(), "the move runs the window's dock drag");
    let moving = |change: &PlacementChange| change.kind == PlacementKind::Move && change.phase == GesturePhase::Update;
    assert!(floated.changes.borrow().iter().any(moving), "the window reports its move");
    fixture.release_at(path[2]).idle(2);
    let last = floated.changes.borrow().last().copied().expect("the window reported its move");
    assert!(last.kind == PlacementKind::Move && last.phase == GesturePhase::Commit, "the release commits the move: {last:?}");
    assert!(drag::active::<GroupDrag<&'static str>>(fixture.ctx()).is_none(), "the release ends the dock drag");

    let (mut fixture, floated) = floating_rig();
    let at = first_button(&fixture).center();
    open_by_context(&mut fixture, at);
    let target = bar(&fixture);
    fixture.click(&target);
    let put = floated.rect.get().expect("a click on the bar floats the group too");
    fixture.frame(vec![moved(target.center() + Vec2::new(60.0, 40.0))]).idle(2);
    assert!(floated.changes.borrow().is_empty(), "with no press held the window reports no move");
    assert_eq!(floated.rect.get(), Some(put), "with no press held the window stays where it was put");
}
