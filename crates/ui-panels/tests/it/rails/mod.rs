//! The rail's own logic: which tool a slot shows, what the rail remembers, where a drop goes, and the flyout's hold
//! and click, and a press dragged away.

pub(crate) mod drawer;

use craft_ui::contract::{Role, Show as _};
use craft_ui::rails::{FlyoutButton, RailTool};
use craft_ui::runtime::commands::EditCommands;
use craft_ui::runtime::frame::take_output;
use craft_ui::testing::pointer::{moved, secondary};
use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers};
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
