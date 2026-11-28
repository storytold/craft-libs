//! Locating a tool on the rail: `reveal` opens the flyout of a slot that hides the tool, `mark` dresses it, and
//! neither chooses it.

use super::{CUT, REDO, UNDO, tools};
use craft_ui::contract::{Role, Tracked};
use craft_ui::display::TargetMark;
use craft_ui::rails::{LastUsed, SlotId, ToolRail, ToolSlot};
use craft_ui::testing::{Fixture, trial};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

struct Rig {
    fixture: Fixture,
    selected: Rc<RefCell<Vec<u8>>>,
    flyout: Rc<Cell<Option<SlotId>>>,
}

/// One slot of three tools that shows Undo, with `reveal` and `mark` passed in every pass.
fn rig(reveal: Option<u8>, mark: Option<u8>) -> Rig {
    let selected = Rc::new(RefCell::new(Vec::new()));
    let flyout = Rc::new(Cell::new(None));
    let (seen, opened) = (selected.clone(), flyout.clone());
    let slots = Tracked::new(vec![ToolSlot::new(0, tools(&[UNDO, REDO, CUT]))]);
    let last = Tracked::new(LastUsed::default());
    let mut fixture = Fixture::new(move |ui| {
        let mut rail = ToolRail::new("Tools", &slots, &UNDO, &last);
        if let Some(tool) = &reveal {
            rail = rail.reveal(tool);
        }
        if let Some(tool) = &mark {
            rail = rail.mark(tool);
        }
        let out = rail.show_rail(ui, |_, _| {});
        if let Some(tool) = out.select {
            seen.borrow_mut().push(tool);
        }
        opened.set(out.flyout);
    });
    fixture.settle();
    Rig { fixture, selected, flyout }
}

#[trial]
fn a_revealed_tool_opens_its_group_and_wears_the_mark_without_being_chosen() {
    let hidden = rig(Some(REDO), Some(REDO));
    assert!(hidden.flyout.get().is_some(), "revealing a tool the slot hides opens the slot's flyout");
    assert!(!hidden.fixture.by_role(Role::MENU).is_empty(), "the flyout shows");
    assert!(!hidden.fixture.styles_of(std::any::type_name::<TargetMark>()).is_empty(), "the marked tool wears a mark");
    assert!(hidden.selected.borrow().is_empty(), "revealing and marking choose nothing");

    let shown = rig(None, Some(UNDO));
    assert!(shown.flyout.get().is_none(), "a mark alone opens no flyout");
    assert!(shown.fixture.by_role(Role::MENU).is_empty(), "no flyout shows");
    assert!(!shown.fixture.styles_of(std::any::type_name::<TargetMark>()).is_empty(), "the shown tool's button wears the mark");
    assert!(shown.selected.borrow().is_empty(), "a mark chooses nothing");
}
