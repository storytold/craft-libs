//! The drawer's own logic: how an edit changes the toolbar and what a drop on the rail asks for, and which of its tools
//! drag.

use craft_ui::rails::{DropAt, RailTool, ToolSlot, ToolbarEdit, apply_toolbar_edit};
use craft_ui::runtime::commands::EditCommands;
use craft_ui::runtime::drag;
use craft_ui::testing::{Fixture, stories, trial};
use egui::Vec2;

const UNDO: u8 = 1;
const REDO: u8 = 2;
const CUT: u8 = 3;

fn tool(id: u8) -> RailTool<u8> {
    let command = match id {
        UNDO => EditCommands::Undo,
        REDO => EditCommands::Redo,
        CUT => EditCommands::Cut,
        _ => EditCommands::Copy,
    };
    RailTool::new(id, command)
}

fn slot(id: u32, ids: &[u8]) -> ToolSlot<u8> {
    ToolSlot::new(id, ids.iter().map(|id| tool(*id)).collect())
}

/// The tool ids of each slot, in order.
fn layout(slots: &[ToolSlot<u8>]) -> Vec<(u32, Vec<u8>)> {
    slots.iter().map(|slot| (slot.id, slot.tools.iter().map(|tool| tool.id).collect())).collect()
}

#[trial]
fn a_tool_moved_down_its_own_slot_lands_at_the_index_after_it_leaves() {
    let slots = [slot(0, &[UNDO, REDO, CUT])];
    let edit = ToolbarEdit::move_to(UNDO, DropAt::into_slot(0, 3));
    assert_eq!(layout(&apply_toolbar_edit(&slots, &edit, &[])), [(0, vec![REDO, CUT, UNDO])], "the removal shifts the index");
}

// The drawer opened from the rail's Edit Toolbar button gives the focus back to that button when it closes.
#[trial]
fn closing_the_drawer_returns_the_focus_to_the_rails_edit_toolbar_button() {
    use craft_ui::rails::{DrawerGroup, LastUsed, ToolRail, ToolbarDrawer};
    use craft_ui::testing::Fixture;
    use egui::{Key, Modifiers};
    use std::cell::Cell;
    use std::rc::Rc;

    let open = Rc::new(Cell::new(false));
    let shown = open.clone();
    let mut fixture = Fixture::new(move |ui| {
        let slots = [slot(0, &[UNDO]), slot(1, &[REDO])];
        let groups = [DrawerGroup::new("Edit", vec![tool(UNDO), tool(REDO), tool(CUT)])];
        let last = LastUsed::default();
        let rail = ToolRail::new("Tools", &slots, &UNDO, &last).edit_toolbar("Customize").show_rail(ui, |_, _| {});
        let opener = rail.edit_toolbar.expect("the rail shows its Edit Toolbar button");
        if craft_ui::contract::clicked(&opener) {
            shown.set(true);
        }
        if shown.get() && ToolbarDrawer::new(rail.response.rect, &groups, &slots, None).opened_by(&opener).show(ui).closed {
            shown.set(false);
        }
    });
    fixture.idle(30);
    let customize = fixture.by_label("Customize").expect("the rail shows its Edit Toolbar button");
    fixture.click(&customize).idle(6);
    assert!(open.get(), "the button opened the drawer");
    fixture.key(Key::Escape, Modifiers::NONE).idle(6);
    assert!(!open.get(), "Escape closed the drawer");
    let focused = craft_ui::testing::access::focused_key(fixture.tree());
    assert!(fixture.by_label("Customize").is_some_and(|button| button.node == focused), "the focus is back on Edit Toolbar");
}

/// A tool already on the toolbar is dimmed and does not drag; a tool that is not on it drags to the rail.
#[trial]
fn only_a_tool_off_the_toolbar_drags() {
    let entry = stories().into_iter().find(|entry| entry.story.name == "toolbar_drawer").expect("the drawer story");
    let mut fixture = Fixture::story(&entry.story);
    fixture.settle();
    let drags = |fixture: &mut Fixture, label: &str| {
        let row = fixture.by_label(label).unwrap_or_else(|| panic!("the {label} row"));
        let before = drag::started(fixture.ctx());
        fixture.drag_hold_at(row.center(), Vec2::new(0.0, 30.0));
        let started = drag::started(fixture.ctx()) > before;
        fixture.release_at(row.center() + Vec2::new(0.0, 30.0)).settle();
        started
    };
    assert!(!drags(&mut fixture, "Brush"), "Brush is on the toolbar");
    assert!(drags(&mut fixture, "Eraser"), "Eraser is not");
}
