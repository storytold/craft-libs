//! A dock drop's mark is the region the dragged panel takes once dropped: the rect the area then gives its group.

use super::Numbered;
use craft_behavior::Axis;
use craft_behavior::dock::{Column, DockEdge, Group, Layout, Node, SplitSize, Workspace};
use craft_ui::contract::{Bind, BodyCx, Role, Tracked};
use craft_ui::docking::{DockConfig, DockPanels, DragUnit, FloatSpace, GroupAt, float_drop};
use craft_ui::patterns::dock::DockArea;
use craft_ui::runtime::drag::{self, DropMark};
use craft_ui::testing::pointer::{moved, press, release};
use craft_ui::testing::{Fixture, trial};
use egui::{Pos2, Rect, Shape, Ui, Vec2};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

const ROOM: Vec2 = Vec2::new(800.0, 500.0);
const SLACK: f32 = 0.5;

/// Each group's rect as the area last showed it, by the group's panels.
type Shown = Rc<RefCell<HashMap<Vec<usize>, Rect>>>;

/// Records the rect of every group the area shows and leaves the group to the dock.
struct Panels {
    shown: Shown,
}

impl DockPanels<usize> for Panels {
    fn body(&mut self, _: &usize, _: &mut Ui, _: &mut BodyCx<'_>) {}

    fn group_view(&mut self, panels: &[usize], _: usize, _: &mut Ui, at: GroupAt, _: &mut BodyCx<'_>) -> Option<f32> {
        if let GroupAt::Show(rect) = at {
            self.shown.borrow_mut().insert(panels.to_vec(), rect);
        }
        None
    }
}

struct Bed {
    fixture: Fixture,
    workspace: Rc<RefCell<Arc<Workspace<usize>>>>,
    shown: Shown,
}

/// `[1, 2]` beside `[3]`.
fn row() -> Node<usize> {
    Node::split(Axis::X, SplitSize::Ratio(0.5), Node::tabs(vec![1, 2], 0), Node::tabs(vec![3], 0))
}

/// The row under `[4]`.
fn bed() -> Bed {
    bed_on(Node::split(Axis::Y, SplitSize::Ratio(0.3), Node::tabs(vec![4], 0), row()))
}

fn bed_on(root: Node<usize>) -> Bed {
    let workspace = Rc::new(RefCell::new(Arc::new(Workspace::new(Layout::new(Some(root), Vec::new())))));
    let shown: Shown = Rc::default();
    let (held, seen, specs) = (Rc::clone(&workspace), Rc::clone(&shown), Tracked::new(Numbered));
    let mut fixture = Fixture::new(move |ui| {
        let mut workspace = held.borrow().clone();
        let mut panels = Panels { shown: Rc::clone(&seen) };
        DockArea::new("Dock", Bind::bound(&mut workspace), &specs, &mut panels, DockConfig::default()).show(ui);
        *held.borrow_mut() = workspace;
    });
    fixture.room(ROOM).settle();
    Bed { fixture, workspace, shown }
}

fn tab(bed: &Bed, panel: usize) -> Rect {
    let tree = bed.fixture.tree();
    let name = format!("Panel {panel}");
    bed.fixture
        .by_role(Role::TAB)
        .into_iter()
        .find(|tab| craft_ui::testing::access::name(tree, tab.node) == Some(name.as_str()))
        .map(|tab| tab.rect)
        .unwrap_or_else(|| panic!("the tab of panel {panel} shows"))
}

/// Drags the tab of `panel` to `to` and returns the mark shown there, then releases it.
fn drop_tab(bed: &mut Bed, panel: usize, to: Pos2) -> DropMark {
    let from = tab(bed, panel).center();
    bed.fixture.frame(vec![moved(from)]);
    bed.fixture.frame(vec![press(from)]);
    for step in 1..=4 {
        bed.fixture.frame(vec![moved(from + (to - from) * (step as f32 / 4.0))]);
    }
    bed.fixture.frame(Vec::new());
    let mark = drag::mark(bed.fixture.ctx()).expect("the dock takes the drop");
    bed.fixture.frame(vec![release(to)]);
    bed.fixture.settle();
    mark
}

/// Presses the tab of `panel` and drags it to `to`, holding the button there.
fn hold_tab_at(bed: &mut Bed, panel: usize, to: Pos2) {
    let from = tab(bed, panel).center();
    bed.fixture.frame(vec![moved(from)]);
    bed.fixture.frame(vec![press(from)]);
    for step in 1..=4 {
        bed.fixture.frame(vec![moved(from + (to - from) * (step as f32 / 4.0))]);
    }
    bed.fixture.frame(Vec::new());
}

/// The rect the area shows the group of `panel` in.
fn group_rect(bed: &Bed, panel: usize) -> Rect {
    let layout = bed.workspace.borrow().layout().clone();
    let place = layout.place(&panel).expect("the panel is docked");
    let Some(Node::Tabs { panels, .. }) = layout.node_of(&place) else { panic!("panel {panel} is in a tab group") };
    *bed.shown.borrow().get(panels).unwrap_or_else(|| panic!("the group {panels:?} was shown"))
}

fn same(a: Rect, b: Rect) -> bool {
    (a.min - b.min).length() <= SLACK && (a.max - b.max).length() <= SLACK
}

#[trial]
fn a_tab_dropped_beside_a_group_marks_the_rect_its_group_takes() {
    let mut bed = bed();
    let right = group_rect(&bed, 3);
    // In the bottom third of the right group's body, clear of the layout's bottom edge band.
    let to = Pos2::new(right.center().x, right.bottom() - right.height() * 0.2);
    let mark = drop_tab(&mut bed, 2, to);
    let DropMark::Region { area, line: None, .. } = mark else { panic!("a side drop marks a region without a line: {mark:?}") };
    let placed = group_rect(&bed, 2);
    assert!(placed.top() > right.top(), "the tab went below the right group: {placed:?}");
    assert!(same(area, placed), "the mark {area:?} is the rect the dropped panel's group took: {placed:?}");
}

#[trial]
fn a_tab_joining_a_group_marks_the_group_and_its_tab_line_and_a_reorder_marks_the_line_only() {
    let mut bed = bed();
    let target = tab(&bed, 3);
    let mark = drop_tab(&mut bed, 2, Pos2::new(target.right() + 20.0, target.center().y));
    let DropMark::Region { area, line: Some(_), .. } = mark else { panic!("a tab join marks a region with its tab line: {mark:?}") };
    let joined = group_rect(&bed, 2);
    assert_eq!(joined, group_rect(&bed, 3), "the tab joined the right group");
    assert!(same(area, joined), "the mark {area:?} is the rect of the group it joined: {joined:?}");

    let after = tab(&bed, 2);
    let mark = drop_tab(&mut bed, 3, Pos2::new(after.right() + 10.0, after.center().y));
    assert!(matches!(mark, DropMark::Line { .. }), "a reorder inside its own strip marks its line only: {mark:?}");
}

/// A tab that joins a group whose rect grows once its own group goes marks the group as it is under the pointer, as
/// production did, not the larger rect the group takes after the drop.
#[trial]
fn a_tab_joining_a_group_marks_the_group_as_shown_now_not_the_rect_it_grows_to() {
    let mut bed = bed();
    let before = group_rect(&bed, 3);
    let target = tab(&bed, 3);
    let mark = drop_tab(&mut bed, 4, Pos2::new(target.right() + 20.0, target.center().y));
    let DropMark::Region { area, line: Some(_), .. } = mark else { panic!("a tab join marks a region with its tab line: {mark:?}") };
    let after = group_rect(&bed, 4);
    assert_eq!(after, group_rect(&bed, 3), "the tab joined the right group");
    assert!(after.height() > before.height() + SLACK, "the group grew once the top group went: {before:?} -> {after:?}");
    assert!(same(area, before), "the mark {area:?} is the group as it was under the pointer: {before:?}");
}

/// A tab drag names its panel inside the region a drop takes, at the region's top, as group drags do; no badge with
/// the name follows the pointer.
#[trial]
fn a_tab_drag_names_its_panel_inside_the_drop_region_and_not_by_the_pointer() {
    let mut bed = bed();
    let own = tab(&bed, 2);
    let right = group_rect(&bed, 3);
    let to = Pos2::new(right.center().x, right.bottom() - right.height() * 0.2);
    hold_tab_at(&mut bed, 2, to);
    let Some(DropMark::Region { area, .. }) = drag::mark(bed.fixture.ctx()) else { panic!("the dock marks the region below the right group") };
    assert!(to.y > area.center().y, "the pointer {to:?} is in the region's lower half: {area:?}");
    let names: Vec<Rect> = bed
        .fixture
        .frame_shapes()
        .iter()
        .filter_map(|shape| match shape {
            Shape::Text(text) if text.galley.text() == "Panel 2" => Some(shape.visual_bounding_rect()),
            _ => None,
        })
        .filter(|rect| !own.intersects(*rect))
        .collect();
    assert!(!names.is_empty(), "the panel's name shows while it is dragged");
    for name in &names {
        // The pointer is in the region's lower half, so a badge beside it would be too.
        assert!(area.contains_rect(*name) && name.bottom() < area.center().y, "the name {name:?} sits at the top of the region {area:?}");
    }
    bed.fixture.frame(vec![release(to)]);
    bed.fixture.settle();
}

/// A strip along the layout's top edge takes tab joins: the band there, where a drop adds a full-width row, reaches
/// into the strip only near the edge.
#[trial]
fn a_tab_dropped_on_a_strip_at_the_layouts_top_edge_joins_its_group_and_the_edge_still_takes_a_row() {
    let mut bed = bed_on(row());
    let target = tab(&bed, 3);
    let mark = drop_tab(&mut bed, 2, Pos2::new(target.right() + 20.0, target.center().y));
    assert!(matches!(mark, DropMark::Region { line: Some(_), .. }), "a drop on the top strip is a tab join: {mark:?}");
    assert_eq!(group_rect(&bed, 2), group_rect(&bed, 3), "the tab joined the top-right group");

    let area = group_rect(&bed, 1).union(group_rect(&bed, 3));
    let mark = drop_tab(&mut bed, 1, Pos2::new(target.right() + 20.0, area.top() + 1.0));
    let DropMark::Region { area: row, line: None, .. } = mark else { panic!("the top edge marks the row a drop adds: {mark:?}") };
    assert!((row.width() - area.width()).abs() <= SLACK, "the edge drop takes the full width {area:?}: {row:?}");
    assert!(group_rect(&bed, 1).width() >= area.width() - SLACK, "the panel went to a full-width row on top");
}

/// A tab dropped at the window edge beside a start dock makes a new outer column there: the band centred on the
/// outermost column's start edge reaches the window edge, so the drop does not need room the dock lacks.
#[trial]
fn a_tab_dropped_at_the_window_edge_beside_a_start_dock_makes_a_new_outer_column() {
    let start = Node::dock(DockEdge::Start, vec![Column::new(vec![Group::new(vec![1], 0)]).with_width(150.0)]);
    let end = Node::dock(DockEdge::End, vec![Column::new(vec![Group::new(vec![3, 4], 0)]).with_width(150.0)]);
    let rest = Node::split(Axis::X, SplitSize::Ratio(0.7), Node::tabs(vec![2], 0), end);
    let mut bed = bed_on(Node::split(Axis::X, SplitSize::FixedFirst(150.0), start, rest));
    // The dock area fills the room from its origin, so x = 3 is 3 pt from the window edge.
    drop_tab(&mut bed, 4, Pos2::new(3.0, ROOM.y * 0.5));
    let layout = bed.workspace.borrow().layout().clone();
    let path = layout.dock_path(DockEdge::Start).expect("the start dock stays");
    let Some(Node::Dock { columns, .. }) = craft_behavior::dock::node_at(layout.root.as_ref().unwrap(), &path) else { panic!("a start dock") };
    let panels: Vec<Vec<usize>> = columns.iter().map(|column| column.groups.iter().flat_map(|group| group.panels.clone()).collect()).collect();
    assert_eq!(panels, vec![vec![4], vec![1]], "the dropped panel is a new column on the window-edge side of the docked one");
    assert!(tab(&bed, 4).right() <= tab(&bed, 1).left() + SLACK, "the new column shows outside the old one");
}

/// A drop that floats a panel in open room marks the region the window takes, as a docked drop marks its group's: one
/// outline with a light fill around the pointer, where the window lands.
#[trial]
fn a_float_drop_marks_the_region_its_window_takes_under_the_pointer() {
    let space = FloatSpace::new(Rect::from_min_size(Pos2::ZERO, ROOM), &super::sizes(4.0, 60.0, 20.0));
    let at = Pos2::new(400.0, 250.0);
    let dropped = float_drop(&DragUnit::Panel(2_usize), &space, at, Some(Vec2::new(52.0, 400.0)), None, false);
    let DropMark::Region { area, line: None, .. } = dropped.mark else { panic!("a float marks a region: {:?}", dropped.mark) };
    assert!(area.contains(at), "the region {area:?} lies under the pointer {at:?}");
}
