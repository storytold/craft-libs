//! The accessible names of the handles that move a group or a dock column (#410).

use super::*;
use crate::layout::{SplitAxis, SplitSize};
use egui::accesskit::Role;
use egui::{Id, vec2};
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};

const GROUP: &str = "Move the {name} tab group";
const COLUMN: &str = "Move the {name} panel column";

fn tabs(panels: &[u32], active: usize) -> Node<u32> {
    Node::Tabs { panels: panels.to_vec(), active }
}

/// Two tab groups side by side, showing panels 2 and 3.
fn two_groups() -> Layout<u32> {
    Layout {
        root: Some(Node::Split {
            axis: SplitAxis::Horizontal,
            size: SplitSize::FixedSecond(300.0),
            first: Box::new(tabs(&[1, 2], 1)),
            second: Box::new(tabs(&[3], 0)),
        }),
        floating: vec![],
    }
}

/// A left dock of two columns beside canvas 0: [1, 2] over [3], and [4]; iconic when `iconic`.
fn two_columns(iconic: bool) -> Layout<u32> {
    let mut first = Column::new(vec![Group::new(vec![1, 2], 1), Group::new(vec![3], 0)], 200.0);
    first.iconic = iconic;
    let second = Column::new(vec![Group::new(vec![4], 0)], 200.0);
    let dock = Node::Dock { side: Side::Start, columns: vec![first, second] };
    Layout {
        root: Some(Node::Split {
            axis: SplitAxis::Horizontal,
            size: SplitSize::FixedFirst(400.0),
            first: Box::new(dock),
            second: Box::new(tabs(&[0], 0)),
        }),
        floating: vec![],
    }
}

fn titled(panel: &u32) -> String {
    format!("Panel {panel}")
}

/// Panels 3 and 4 have no title; panel 1 only spaces.
fn partly_untitled(panel: &u32) -> String {
    match panel {
        3 | 4 => String::new(),
        1 => "  ".into(),
        _ => titled(panel),
    }
}

/// Renders `layout` with the move labels set to `group` and `column` (the defaults when `None`).
/// Untitled tabs have no name, so kittest's check for nameless widgets is off when a panel has
/// none.
fn harness(layout: Layout<u32>, group: Option<&'static str>, column: Option<&'static str>, label: fn(&u32) -> String) -> Harness<'static> {
    let check = (1..=4).all(|panel| !label(&panel).trim().is_empty());
    let mut h = Harness::builder().with_size(vec2(900.0, 500.0)).with_accessibility_check(check).build_ui(move |ui| {
        let mut style = DockStyle::from_ui(ui);
        if let Some(text) = group {
            style.move_group_label = text.into();
        }
        if let Some(text) = column {
            style.move_column_label = text.into();
        }
        DockArea::new(Id::unique("handle-names")).show(
            ui,
            &layout,
            &style,
            label,
            |_| Permissions::default(),
            |ui: &mut egui::Ui, _: &u32| {
                ui.label("Body");
            },
        );
    });
    h.run_steps(3);
    h
}

fn buttons(h: &Harness<'_>, label: &str) -> usize {
    h.query_all_by_role_and_label(Role::Button, label).count()
}

/// The names of every button whose name starts with "Move", sorted.
fn move_handles(h: &Harness<'_>) -> Vec<String> {
    let mut names: Vec<String> =
        h.query_all_by_role(Role::Button).filter_map(|node| node.accesskit_node().label()).filter(|name| name.starts_with("Move")).collect();
    names.sort();
    names
}

#[test]
/// Each group has one move handle, named after the panel it shows, so two groups' handles differ.
fn move_group_handles_are_named_after_the_panel_their_group_shows() {
    let h = harness(two_groups(), Some(GROUP), None, titled);
    assert_eq!(move_handles(&h), ["Move the Panel 2 tab group", "Move the Panel 3 tab group"]);
}

#[test]
/// Dock groups and column bars are named after what they move: a column after the panels its
/// groups show. No two move handles share a name.
fn dock_group_and_column_handles_are_named_after_what_they_move() {
    let h = harness(two_columns(false), Some(GROUP), Some(COLUMN), titled);
    assert_eq!(
        move_handles(&h),
        [
            "Move the Panel 0 tab group",
            "Move the Panel 2 tab group",
            "Move the Panel 2, Panel 3 panel column",
            "Move the Panel 3 tab group",
            "Move the Panel 4 panel column",
            "Move the Panel 4 tab group",
        ]
    );
}

#[test]
/// The grip above each icon group of an iconic column is named after the group's active panel.
fn icon_group_grips_are_named_after_their_active_panel() {
    let h = harness(two_columns(true), Some(GROUP), Some(COLUMN), titled);
    assert_eq!(buttons(&h, "Move the Panel 2 tab group"), 1);
    assert_eq!(buttons(&h, "Move the Panel 3 tab group"), 1);
    assert_eq!(buttons(&h, "Move the Panel 2, Panel 3 panel column"), 1);
    assert_eq!(buttons(&h, GROUP), 0);
}

#[test]
/// A label without `{name}`, the default or an app's own, is used as it is: once per group (one
/// handle each, as in the issue's strip of a column bar over one group) and once per column.
fn a_label_without_the_placeholder_is_kept_for_every_handle() {
    let h = harness(two_columns(false), None, None, titled);
    assert_eq!(buttons(&h, "Move tab group"), 4);
    assert_eq!(buttons(&h, "Move panel column"), 2);
    let h = harness(two_groups(), Some("Déplacer le groupe"), Some("Déplacer la colonne"), titled);
    assert_eq!(move_handles(&h), Vec::<String>::new());
    assert_eq!(buttons(&h, "Déplacer le groupe"), 2);
}

#[test]
/// An untitled panel leaves the placeholder out rather than an empty name; a column names only
/// its titled panels.
fn untitled_panels_leave_the_name_out() {
    let h = harness(two_columns(false), Some(GROUP), Some(COLUMN), partly_untitled);
    assert_eq!(
        move_handles(&h),
        [
            "Move the Panel 0 tab group",
            "Move the Panel 2 panel column",
            "Move the Panel 2 tab group",
            "Move the panel column",
            "Move the tab group",
            "Move the tab group",
        ]
    );
    // The hidden tab's spaces do not count either: group [1, 2] shows 1 here.
    let mut layout = two_groups();
    if let Some(Node::Split { first, .. }) = &mut layout.root {
        **first = tabs(&[1, 2], 0);
    }
    let h = harness(layout, Some("Move tab group: {name}"), None, partly_untitled);
    assert_eq!(move_handles(&h), ["Move tab group", "Move tab group"]);
}

#[test]
/// A label that is only the placeholder, with no name to put in, falls back to the default.
fn a_label_that_is_only_the_placeholder_falls_back_to_the_default() {
    assert_eq!(labels::named("{name}", labels::MOVE_GROUP, String::new), "Move tab group");
    assert_eq!(labels::named("{name}", labels::MOVE_GROUP, || "Layers".into()), "Layers");
    assert_eq!(labels::named("Move the {name} column", labels::MOVE_COLUMN, || " \t".into()), "Move the column");
    // A digit is enough to keep the label's own text.
    assert_eq!(labels::named("{name} 2", labels::MOVE_GROUP, String::new), "2");
    assert_eq!(labels::named("({name})", labels::MOVE_GROUP, String::new), "Move tab group");
}

#[test]
/// A blank name takes the brackets, quotes and separator that only made sense around it, and
/// adds no space where the placeholder had none beside it.
fn a_blank_name_leaves_no_stray_punctuation() {
    for (label, bare) in [
        ("Move tab group ({name})", "Move tab group"),
        ("Déplacer le groupe « {name} »", "Déplacer le groupe"),
        ("Move “{name}” group", "Move group"),
        ("Move [{name}]", "Move"),
        ("Move the {name}, tab group", "Move the tab group"),
        ("移动{name}标签组", "移动标签组"),
        ("「{name}」タブグループを移動", "タブグループを移動"),
        ("Move the {name} tab group", "Move the tab group"),
        ("Move tab group: {name}", "Move tab group"),
        ("Move tab group — {name}", "Move tab group"),
        ("Move {name} and {name} group", "Move and group"),
        ("“{name}”", "Move tab group"),
    ] {
        assert_eq!(labels::named(label, labels::MOVE_GROUP, String::new), bare, "{label}");
    }
}

#[test]
/// A strip whose tabs leave no room for a drag bar still has one announced, focusable move
/// handle per group: its top border band.
fn without_a_drag_bar_the_border_band_is_the_one_move_handle() {
    let area = Id::unique("no-drag-bar");
    let layout = Layout { root: Some(tabs(&[1], 0)), floating: vec![] };
    let mut h = Harness::builder().with_size(vec2(300.0, 200.0)).build_ui(move |ui| {
        let mut style = DockStyle::from_ui(ui);
        style.drag_bar_width = 0.0;
        style.move_group_label = GROUP.into();
        DockArea::new(area).show(
            ui,
            &layout,
            &style,
            |panel| format!("Panel {panel} with a title far wider than its strip, so its tab takes the whole strip"),
            |_| Permissions::default(),
            |ui: &mut egui::Ui, _: &u32| {
                ui.label("Body");
            },
        );
    });
    h.run_steps(3);
    assert!(h.ctx.read_response(area.with(("drag-bar", Some(&1_u32)))).is_none(), "no drag bar is drawn");
    let border = h.ctx.read_response(area.with(("drag-border", Some(&1_u32)))).expect("the border band is drawn");
    assert!(border.sense.is_focusable());
    let handles: Vec<_> =
        h.query_all_by_role(Role::Button).filter(|node| node.accesskit_node().label().is_some_and(|l| l.starts_with("Move"))).collect();
    assert_eq!(handles.len(), 1);
    assert_eq!(handles[0].rect(), border.rect, "the band is the announced handle");
    assert!(handles[0].accesskit_node().label().is_some_and(|l| l.starts_with("Move the Panel 1 with a title")));
}
