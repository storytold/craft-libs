//! The rail's own logic: which tool a slot shows, what the rail remembers, where a drop goes, and the flyout's hold
//! and click.

mod drawer;

use craft_ui::contract::{Role, Show as _};
use craft_ui::rails::{FlyoutButton, RailTool};
use craft_ui::runtime::commands::EditCommands;
use craft_ui::testing::{Fixture, trial};
use std::cell::RefCell;
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
}

fn rig(hold_selects: bool) -> Rig {
    let selected = Rc::new(RefCell::new(Vec::new()));
    let seen = selected.clone();
    let mut fixture = Fixture::new(move |ui| {
        let all = tools(&[UNDO, REDO, CUT]);
        let out = FlyoutButton::new(&all, &UNDO).hold_selects(hold_selects).show(ui);
        if let Some(tool) = out.select {
            seen.borrow_mut().push(tool);
        }
    });
    fixture.settle();
    Rig { fixture, selected }
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
