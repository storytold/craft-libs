//! The drawer's own logic: how an edit changes the toolbar and what a drop on the rail asks for, and which of its tools
//! drag, where the rail leaves group gaps and which side the drawer opens on.

use craft_ui::contract::Tracked;
use craft_ui::rails::{DropAt, RailTool, ToolSlot, ToolbarEdit, apply_toolbar_edit};
use craft_ui::runtime::commands::EditCommands;
use craft_ui::runtime::drag;
use craft_ui::testing::{Fixture, Registry, trial};
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
    let slots = Tracked::new(vec![slot(0, &[UNDO]), slot(1, &[REDO])]);
    let last = Tracked::new(LastUsed::default());
    let mut fixture = Fixture::new(move |ui| {
        let groups = [DrawerGroup::new("Edit", vec![tool(UNDO), tool(REDO), tool(CUT)])];
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

/// Whether `label`'s row of the drawer `story` (opened from the rail's Edit Toolbar button) starts a drag, for each label.
fn drawer_drags(story: &str, labels: &[&str]) -> Vec<bool> {
    let registry = Registry::of(&[craft_ui::testing::register]);
    let entry = registry.stories().iter().find(|entry| entry.story.name == story).expect("the drawer story");
    let mut fixture = Fixture::story(&entry.story);
    fixture.settle();
    let opener = fixture.by_label("Edit Toolbar").expect("the rail's Edit Toolbar button");
    fixture.click(&opener).settle();
    let drags = |fixture: &mut Fixture, label: &str| {
        let row = fixture.by_label(label).unwrap_or_else(|| panic!("the {label} row"));
        let before = drag::started(fixture.ctx());
        fixture.drag_hold_at(row.center(), Vec2::new(0.0, 30.0));
        let started = drag::started(fixture.ctx()) > before;
        fixture.release_at(row.center() + Vec2::new(0.0, 30.0)).settle();
        started
    };
    labels.iter().map(|label| drags(&mut fixture, label)).collect()
}

/// By default a tool already on the toolbar, grouped behind another tool too, is dimmed and does not drag; a tool that
/// is not on it drags to the rail.
#[trial]
fn only_a_tool_off_the_toolbar_drags() {
    // Pencil is on the toolbar behind Brush.
    assert_eq!(drawer_drags("toolbar_drawer", &["Pencil", "Eraser"]), [false, true], "Pencil is on the toolbar, Eraser is not");
}

/// With `grouped_tools_drag` (PdfCraft's drawer) a grouped tool can also be dragged from the drawer into its own slot.
#[trial]
fn a_grouped_tool_also_drags_from_the_drawer() {
    // Pencil is on the toolbar behind Brush, so its only button is the drawer's row.
    assert_eq!(
        drawer_drags("toolbar_drawer_with_grouped_tools_dragging", &["Pencil", "Eraser"]),
        [true, true],
        "Pencil can also have its own slot; Eraser is off the toolbar"
    );
}

#[trial]
fn adding_a_grouped_tool_keeps_the_group_entry() {
    let slots = [slot(0, &[UNDO, REDO])];
    let edit = ToolbarEdit::add(tool(REDO), DropAt::new_slot(1));
    let result = apply_toolbar_edit(&slots, &edit, &[]);
    assert_eq!(result.len(), 2);
    assert_eq!(result[0].tools.len(), 2);
    assert_eq!(result[1].tools[0].id, REDO);
    let removed = apply_toolbar_edit(&result, &ToolbarEdit::remove_from(result[1].id, 0), &[]);
    assert_eq!(layout(&removed), layout(&slots), "removing the separate occurrence preserves the group");
    let moved = apply_toolbar_edit(&result, &ToolbarEdit::move_from(result[1].id, 0, DropAt::new_slot(0)), &[]);
    assert_eq!(moved[1].tools.len(), 2, "moving the separate occurrence preserves the group");
}

/// A click on a tool reports it as the one to select and leaves no row selected: the drawer's rows pick, they do not
/// hold a selection.
#[trial]
fn a_click_on_a_tool_selects_it_and_no_row_stays_selected() {
    use craft_ui::rails::{DrawerGroup, ToolbarDrawer};
    use std::cell::Cell;
    use std::rc::Rc;
    let picked = Rc::new(Cell::new(None::<u8>));
    let seen = picked.clone();
    let mut fixture = Fixture::new(move |ui| {
        let groups = [DrawerGroup::new("Edit", vec![tool(UNDO), tool(CUT), tool(4)])];
        let toolbar = [slot(0, &[UNDO])];
        let out = ToolbarDrawer::new(ui.max_rect(), &groups, &toolbar, None).show(ui);
        if let Some(id) = out.select {
            seen.set(Some(id));
        }
    });
    fixture.settle();
    let row = fixture.by_label("Cut").expect("the Cut row");
    fixture.click_at(row.center()).settle();
    assert_eq!(picked.get(), Some(CUT), "the click selects the tool");
    let selected = fixture.tree().selected_of(row.node).unwrap_or(false);
    assert!(!selected, "the row keeps no selection");
}

/// A slot that ends a group leaves a wider gap before the next slot than the slots inside a group leave.
#[trial]
fn a_slot_that_ends_a_group_leaves_a_wider_gap_than_slots_within_a_group() {
    use craft_ui::rails::{LastUsed, ToolRail};
    let slots = Tracked::new(vec![slot(0, &[UNDO]), ToolSlot::ending_group(1, vec![tool(REDO)]), slot(2, &[CUT])]);
    let last = Tracked::new(LastUsed::default());
    let mut fixture = Fixture::new(move |ui| {
        ToolRail::new("Tools", &slots, &UNDO, &last).show_rail(ui, |_, _| {});
    });
    fixture.room(Vec2::new(60.0, 300.0)).settle();
    let rect = |label: &str| fixture.by_label(label).unwrap_or_else(|| panic!("the rail shows {label}")).rect;
    let (undo, redo, cut) = (rect("Undo"), rect("Redo"), rect("Cut"));
    let (within, between) = (redo.top() - undo.bottom(), cut.top() - redo.bottom());
    assert!(between > within, "the gap after the group ({between}) is wider than the gap inside it ({within})");
}

/// The drawer opens on the side of the rail it is told to, for either side.
#[trial]
fn the_drawer_opens_on_the_side_it_is_given() {
    use craft_behavior::overlay::Side;
    use craft_ui::rails::{DrawerGroup, ToolbarDrawer};
    use egui::{Pos2, Rect};
    const ROOM: Vec2 = Vec2::new(480.0, 320.0);
    for opens in [Side::End, Side::Start] {
        let x = if opens == Side::Start { ROOM.x - 48.0 } else { 0.0 };
        let rail = Rect::from_min_size(Pos2::new(x, 0.0), Vec2::new(48.0, ROOM.y));
        let mut fixture = Fixture::new(move |ui| {
            let toolbar = [slot(0, &[UNDO])];
            let groups = [DrawerGroup::new("Edit", vec![tool(UNDO), tool(REDO), tool(CUT)])];
            ToolbarDrawer::new(rail, &groups, &toolbar, None).opens(opens).show(ui);
        });
        fixture.room(ROOM).settle();
        let row = fixture.by_label("Cut").expect("the drawer lists Cut").rect;
        let away = if opens == Side::Start { row.right() <= rail.left() } else { row.left() >= rail.right() };
        assert!(away, "{opens:?}: the drawer row {row:?} lies on that side of the rail {rail:?}");
    }
}

/// With two groups of tools the list holds heading rows between the tools, so a tool's key is not its flat index: a
/// revealed tool far down the list scrolls into the drawer's room, and only a marked tool wears the mark.
#[trial]
fn a_revealed_tool_scrolls_into_view_and_wears_the_mark() {
    use craft_ui::contract::Role;
    use craft_ui::display::TargetMark;
    use craft_ui::rails::{DrawerGroup, ToolbarDrawer};
    use egui::{Pos2, Rect};
    use std::cell::Cell;
    use std::rc::Rc;
    const ROOM: Vec2 = Vec2::new(480.0, 240.0);
    let shown = |marked: bool| {
        let pending = Rc::new(Cell::new(true));
        let mut fixture = Fixture::new(move |ui| {
            let ids = |from: u8, to: u8| (from..to).map(tool).collect::<Vec<_>>();
            let mut second = ids(30, 49);
            second.push(tool(CUT));
            let groups = [DrawerGroup::new("First", ids(10, 30)), DrawerGroup::new("Second", second)];
            let toolbar = [slot(0, &[UNDO])];
            let rail = Rect::from_min_size(Pos2::ZERO, Vec2::new(48.0, ROOM.y));
            let target = CUT;
            let mut drawer = ToolbarDrawer::new(rail, &groups, &toolbar, None);
            if pending.replace(false) {
                drawer = drawer.reveal(&target);
            }
            if marked {
                drawer = drawer.mark(&target);
            }
            drawer.show(ui);
        });
        fixture.room(ROOM).idle(60);
        // The drawer is a popover over the window, which is larger than the room: its list scrolls inside it.
        let drawer = fixture.by_role(Role::DIALOG).first().expect("the drawer is open").rect;
        let row = fixture.by_label("Cut").expect("the revealed tool's row is shown").rect;
        (drawer, row, !fixture.styles_of(std::any::type_name::<TargetMark>()).is_empty())
    };
    let (drawer, row, marked) = shown(true);
    assert!(drawer.contains_rect(row), "the revealed row {row:?} lies inside the drawer {drawer:?}");
    assert!(marked, "the marked tool wears the mark");
    let (_, _, unmarked) = shown(false);
    assert!(!unmarked, "without a mark no ring is drawn");
}

// The Edit Toolbar button takes the app's icon (production shows a settings gear, not an ellipsis) and stays one named,
// working button.
#[trial]
fn the_edit_toolbar_button_takes_the_apps_icon() {
    use craft_ui::rails::{LastUsed, ToolRail};
    let slots = Tracked::new(vec![slot(0, &[UNDO])]);
    let last = Tracked::new(LastUsed::default());
    let mut fixture = Fixture::new(move |ui| {
        let _ = ToolRail::new("Tools", &slots, &UNDO, &last)
            .edit_toolbar("Customize")
            .edit_icon(craft_behavior::IconId::new("settings"))
            .show_rail(ui, |_, _| {});
    });
    fixture.idle(12);
    assert!(fixture.by_label("Customize").is_some(), "the button keeps its name with another icon");
}
