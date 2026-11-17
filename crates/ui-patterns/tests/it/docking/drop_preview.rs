//! A dock drop's mark is the region the dragged panel takes once dropped: the rect the area then gives its group.

use craft_behavior::Axis;
use craft_behavior::dock::{Layout, Node, SplitSize, Workspace};
use craft_ui::contract::{Bind, BodyCx, Role};
use craft_ui::docking::{DockConfig, DockPanels, GroupAt, PanelSpec};
use craft_ui::patterns::dock::DockArea;
use craft_ui::runtime::drag::{self, DropMark};
use craft_ui::testing::pointer::{moved, press, release};
use craft_ui::testing::{Fixture, trial};
use egui::{Pos2, Rect, Ui, Vec2};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

const ROOM: Vec2 = Vec2::new(800.0, 500.0);
const SLACK: f32 = 0.5;

/// Each group's rect as the area last showed it, by the group's panels.
type Shown = Rc<RefCell<HashMap<Vec<usize>, Rect>>>;

/// Records the rect of every group the area shows and leaves the group to the dock.
struct Panels {
    shown: Shown,
}

impl DockPanels<usize> for Panels {
    fn spec(&self, panel: &usize) -> PanelSpec {
        PanelSpec::new(format!("Panel {panel}"))
    }

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
    workspace: Rc<RefCell<Workspace<usize>>>,
    shown: Shown,
}

/// `[1, 2]` beside `[3]`.
fn bed() -> Bed {
    let root = Node::split(Axis::X, SplitSize::Ratio(0.5), Node::tabs(vec![1, 2], 0), Node::tabs(vec![3], 0));
    let workspace = Rc::new(RefCell::new(Workspace::new(Layout::new(Some(root), Vec::new()))));
    let shown: Shown = Rc::default();
    let (held, seen) = (Rc::clone(&workspace), Rc::clone(&shown));
    let mut fixture = Fixture::new(move |ui| {
        let mut workspace = held.borrow().clone();
        let mut panels = Panels { shown: Rc::clone(&seen) };
        DockArea::new("Dock", Bind::bound(&mut workspace), &mut panels, DockConfig::default()).show(ui);
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

/// The rect the area shows the group of `panel` in.
fn group_rect(bed: &Bed, panel: usize) -> Rect {
    let layout = bed.workspace.borrow().layout.clone();
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
