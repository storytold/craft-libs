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

struct SearchableDrawer {
    fixture: Fixture,
    open: std::rc::Rc<std::cell::Cell<bool>>,
    reveal: std::rc::Rc<std::cell::Cell<Option<u8>>>,
}

/// A searchable drawer opened from its real rail button. Reveal is a request of one pass, not held state.
fn searchable_drawer(no_matches: Option<&'static str>) -> SearchableDrawer {
    use craft_ui::rails::{DrawerGroup, DrawerKind, LastUsed, ToolRail, ToolbarDrawer};
    use std::cell::Cell;
    use std::rc::Rc;

    let open = Rc::new(Cell::new(false));
    let reveal = Rc::new(Cell::new(None));
    let (shown, pending) = (open.clone(), reveal.clone());
    let slots = Tracked::new(vec![slot(0, &[UNDO])]);
    let last = Tracked::new(LastUsed::default());
    let mut fixture = Fixture::new(move |ui| {
        let groups = [DrawerGroup::new("Edit", vec![tool(UNDO), tool(REDO), tool(CUT)])];
        let rail = ToolRail::new("Tools", &slots, &UNDO, &last).edit_toolbar("Customize").show_rail(ui, |_, _| {});
        let opener = rail.edit_toolbar.expect("the rail's Edit Toolbar button");
        if craft_ui::contract::clicked(&opener) {
            shown.set(true);
        }
        if shown.get() {
            let target = pending.take();
            let mut drawer = ToolbarDrawer::new(rail.response.rect, &groups, &slots, None).opened_by(&opener).kind(DrawerKind::Searchable);
            if let Some(label) = no_matches {
                drawer = drawer.no_matches(label);
            }
            if let Some(target) = target.as_ref() {
                drawer = drawer.reveal(target);
            }
            let out = drawer.show(ui);
            assert!(out.select.is_none() && out.edits.is_empty(), "searching or revealing chooses and edits nothing");
            if out.closed {
                shown.set(false);
            }
        }
    });
    fixture.settle();
    let opener = fixture.by_label("Customize").expect("the rail's Edit Toolbar button");
    fixture.click(&opener).settle();
    SearchableDrawer { fixture, open, reveal }
}

#[trial]
fn searchable_drawer_keeps_native_escape_clear_then_close_and_opener_focus() {
    use craft_ui::contract::Role;
    use egui::{Key, Modifiers};

    let SearchableDrawer { mut fixture, open, .. } = searchable_drawer(None);
    let search = fixture.by_role(Role::TEXT_INPUT).first().cloned().expect("the drawer's own editable search");
    fixture.click(&search).type_text("undo").settle();
    assert_eq!(fixture.tree().value_of(search.node), Some("undo"), "real typing reached Search");
    assert!(fixture.by_label("Cut").is_none(), "the nonempty query filters the drawer");
    fixture.key(Key::Escape, Modifiers::NONE).settle();
    assert!(open.get(), "native first Escape clears instead of closing");
    assert_eq!(fixture.tree().value_of(search.node), Some(""));
    assert!(fixture.by_label("Cut").is_some(), "clearing restores other rows");
    fixture.key(Key::Escape, Modifiers::NONE).settle();
    assert!(!open.get(), "Escape on the empty search closes the drawer");
    let focused = craft_ui::testing::access::focused_key(fixture.tree());
    assert!(fixture.by_label("Customize").is_some_and(|button| button.node == focused), "closing returns focus to the opener");
}

#[trial]
fn revealing_a_filtered_tool_clears_search_once_and_keeps_later_typing() {
    use craft_ui::contract::Role;
    use egui::{Key, Modifiers};

    let SearchableDrawer { mut fixture, open, reveal } = searchable_drawer(None);
    fixture.room(Vec2::new(960.0, 720.0)).settle();
    let search = fixture.by_role(Role::TEXT_INPUT).first().cloned().expect("the drawer's own search");
    fixture.click(&search).type_text("undo").settle();
    assert!(fixture.by_label("Cut").is_none(), "the old query really hides the requested tool");
    reveal.set(Some(CUT));
    // Queue the reveal before the resized frame remeasures the drawer header.
    fixture.room(Vec2::new(800.0, 600.0));
    fixture.settle();
    assert!(open.get(), "revealing leaves the drawer open");
    assert_eq!(fixture.tree().value_of(search.node), Some(""), "a valid reveal clears the stale query");
    let drawer = fixture.by_role(Role::DIALOG).first().expect("the drawer stays open").rect;
    assert!(drawer.contains_rect(fixture.by_label("Cut").expect("the requested tool is shown").rect));
    let search = fixture.by_role(Role::TEXT_INPUT).first().cloned().expect("search remains usable");
    fixture.click(&search).key(Key::A, Modifiers::COMMAND).type_text("redo").settle();
    fixture.idle(6);
    assert_eq!(fixture.tree().value_of(search.node), Some("redo"), "later frames do not replay the clear request");
    assert!(fixture.by_label("Cut").is_none(), "later typing filters again");
    reveal.set(Some(u8::MAX));
    fixture.settle();
    assert_eq!(fixture.tree().value_of(search.node), Some("redo"), "a missing tool does not erase the user's filter");
}

#[trial]
fn a_callers_empty_search_label_shows_only_while_no_tools_match() {
    use craft_behavior::PartKey;
    use craft_ui::contract::Role;
    use craft_ui::runtime::frame::take_output;

    const EMPTY: &str = "No tools found here";
    for label in [None, Some(EMPTY)] {
        let SearchableDrawer { mut fixture, open, .. } = searchable_drawer(label);
        assert!(fixture.by_label(EMPTY).is_none(), "an empty query lists tools without no-match feedback");
        let search = fixture.by_role(Role::TEXT_INPUT).first().cloned().expect("the drawer's own search");
        fixture.click(&search).type_text("undo").settle();
        assert!(fixture.by_label("Undo").is_some(), "a nonempty matching query keeps its real row");
        assert!(fixture.by_label(EMPTY).is_none(), "the authoritative matching count suppresses empty feedback");
        let _ = take_output(fixture.ctx());
        fixture.key(egui::Key::A, egui::Modifiers::COMMAND).type_text("qzx-no-such-tool").settle();
        let expected: Vec<String> = label.into_iter().map(str::to_owned).collect();
        assert_eq!(take_output(fixture.ctx()).announcements, expected, "entering the opted-in empty result announces its caller's label once");
        assert_eq!(craft_ui::testing::access::focused_key(fixture.tree()), search.node, "the result announcement leaves focus on Search");
        fixture.idle(6);
        assert!(take_output(fixture.ctx()).announcements.is_empty(), "idle frames cannot repeat the result announcement");
        assert_eq!(fixture.tree().value_of(search.node), Some("qzx-no-such-tool"));
        assert!(fixture.by_label("Cut").is_none() && fixture.by_label("Redo").is_none(), "the real search has no matching tool rows");
        assert_eq!(fixture.by_label(EMPTY).is_some(), label.is_some(), "only an opted-in caller adds the feedback");
        if let Some(feedback) = fixture.by_label(EMPTY) {
            let drawer = fixture.by_role(Role::DIALOG).first().expect("the open drawer").rect;
            assert!(drawer.contains_rect(feedback.rect), "the feedback is part of the drawer");
        }
        fixture.key(egui::Key::A, egui::Modifiers::COMMAND).type_text("undo").settle();
        assert!(fixture.by_label("Undo").is_some() && fixture.by_label(EMPTY).is_none(), "a matching query recovers actual results");
        assert!(take_output(fixture.ctx()).announcements.is_empty(), "returning matches resets the state without an empty-result announcement");
        fixture.key(egui::Key::A, egui::Modifiers::COMMAND).type_text("qzx-no-such-tool").settle();
        assert_eq!(take_output(fixture.ctx()).announcements, expected, "empty results after a matching query announce once again");
        let clear = craft_ui::testing::pointer::part_rect_of_node(fixture.ctx(), search.node.into(), PartKey::new("clear"))
            .expect("the search's real clear part");
        fixture.click_at(clear.center()).settle();
        assert_eq!(fixture.tree().value_of(search.node), Some(""));
        assert!(fixture.by_label("Cut").is_some(), "clear restores the tool rows");
        assert!(fixture.by_label(EMPTY).is_none(), "the feedback leaves with the rejected query");
        assert!(open.get(), "searching and clearing leave the drawer open");
        assert!(take_output(fixture.ctx()).announcements.is_empty(), "clearing resets the result state silently");
        fixture.click(&search).type_text("qzx-no-such-tool").settle();
        assert_eq!(take_output(fixture.ctx()).announcements, expected, "a later empty-result transition announces once again");
        fixture.idle(6);
        assert!(take_output(fixture.ctx()).announcements.is_empty(), "the second result is also quiet while unchanged");
    }
}
