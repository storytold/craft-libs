//! Floating windows of a `DockArea`: a drop over a window lands on it.

use craft_behavior::Axis;
use craft_behavior::dock::{Action, Floating, Layout, Node, Permissions, SplitSize, Workspace};
use craft_ui::contract::{Bind, BodyCx, Role, Show as _, ShowContainer as _};
use craft_ui::display::Text;
use craft_ui::docking::{DockConfig, DockPanels, GroupDrag, PanelSpec};
use craft_ui::layout_components::Stack;
use craft_ui::patterns::dock::DockArea;
use craft_ui::runtime::drag::{self, DropMark};
use craft_ui::testing::pointer::{moved, press, release};
use craft_ui::testing::{Fixture, trial};
use egui::{Modifiers, Pos2, Rect, Ui, Vec2};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

const DOCKED: usize = 1;
const WINDOW: usize = 2;
const SECOND: usize = 3;
/// A panel whose content is two lines of text.
const SHORT: usize = 4;
/// A panel whose content fills its body.
const FILL: usize = 5;
/// Inside the fixture's room, so the area's bounds clamp neither the window nor the moves and resizes made of it.
const WINDOW_AT: Rect = Rect { min: Pos2::new(130.0, 90.0), max: Pos2::new(330.0, 210.0) };
const SLACK: f32 = 1.5;

fn title(panel: usize) -> String {
    format!("Panel {panel}")
}

struct Panels {
    rights: fn(&usize) -> Permissions,
}

impl DockPanels<usize> for Panels {
    fn spec(&self, panel: &usize) -> PanelSpec {
        PanelSpec::new(title(*panel))
    }

    fn permissions(&self, panel: &usize) -> Permissions {
        (self.rights)(panel)
    }

    fn body(&mut self, panel: &usize, ui: &mut Ui, _: &mut BodyCx<'_>) {
        match *panel {
            SHORT => {
                Text::new("One").show(ui);
                Text::new("Two").show(ui);
            }
            FILL => {
                Stack::new().fill().show(ui, |ui, _| {
                    Text::new("Fills").show(ui);
                });
            }
            _ => {}
        }
    }
}

/// A dock area over a bound workspace; the area applies its actions. A second area over its own workspace shows
/// while `second` is set.
struct Bed {
    fixture: Fixture,
    workspace: Rc<RefCell<Workspace<usize>>>,
    actions: Rc<RefCell<Vec<Action<usize>>>>,
    second: Rc<Cell<bool>>,
    /// The tallest a floated panel opens (`FloatSpace::default_size`), as the area reported it.
    float_height: Rc<Cell<f32>>,
}

fn bed(layout: Layout<usize>, rights: fn(&usize) -> Permissions) -> Bed {
    let workspace = Rc::new(RefCell::new(Workspace::new(layout)));
    let actions: Rc<RefCell<Vec<Action<usize>>>> = Rc::default();
    let second = Rc::new(Cell::new(false));
    let float_height = Rc::new(Cell::new(0.0));
    let (held, seen_actions, shows_second, seen_height) = (Rc::clone(&workspace), Rc::clone(&actions), Rc::clone(&second), Rc::clone(&float_height));
    let mut other = Workspace::new(Layout::new(Some(Node::tabs(vec![SECOND], 0)), Vec::new()));
    let mut fixture = Fixture::new(move |ui| {
        let mut workspace = held.borrow().clone();
        let mut panels = Panels { rights };
        let out = DockArea::new("Floating dock", Bind::bound(&mut workspace), &mut panels, DockConfig::default()).show(ui);
        *held.borrow_mut() = workspace;
        seen_actions.borrow_mut().extend(out.actions);
        if let Some(space) = out.float_space {
            seen_height.set(space.default_size.y);
        }
        if shows_second.get() {
            DockArea::new("Second dock", Bind::bound(&mut other), &mut panels, DockConfig::default()).show(ui);
        }
    });
    fixture.settle();
    Bed { fixture, workspace, actions, second, float_height }
}

/// A point on the floating window's title row.
fn window_title() -> Pos2 {
    Pos2::new(WINDOW_AT.center().x, WINDOW_AT.min.y + 12.0)
}

fn dragging(bed: &Bed) -> bool {
    drag::active::<GroupDrag<usize>>(bed.fixture.ctx()).is_some()
}

fn with_window(root: Option<Node<usize>>) -> Layout<usize> {
    Layout::new(root, vec![Floating::single(vec![WINDOW], 0, WINDOW_AT)])
}

fn press_and_move(fixture: &mut Fixture, from: Pos2, by: Vec2) {
    fixture.frame(vec![moved(from)]);
    fixture.frame(vec![press(from)]);
    for step in 1..=4 {
        fixture.frame(vec![moved(from + by * (step as f32 / 4.0))]);
    }
}

#[trial]
fn a_tab_dragged_over_a_floating_window_marks_and_joins_the_window() {
    let mut bed = bed(with_window(Some(Node::tabs(vec![DOCKED, SECOND], 0))), |_| Permissions::ALL);
    let tabs = bed.fixture.by_role(Role::TAB);
    let mut docked: Vec<_> = tabs.iter().filter(|tab| !WINDOW_AT.contains(tab.center())).copied().collect();
    docked.sort_by(|a, b| a.rect.min.x.total_cmp(&b.rect.min.x));
    let second = *docked.last().expect("the second docked tab shows");
    let window_tab = tabs.iter().find(|tab| WINDOW_AT.contains(tab.center())).copied().expect("the window's tab shows");
    let over = Pos2::new(window_tab.rect.right() + 20.0, window_tab.center().y);
    press_and_move(&mut bed.fixture, second.center(), over - second.center());
    bed.fixture.frame(Vec::new());
    let mark = drag::mark(bed.fixture.ctx()).expect("the window takes the drop");
    let points = match mark {
        DropMark::Line { from, to, .. } => vec![from, to],
        DropMark::Into(rect) | DropMark::Split(rect) => vec![rect.center()],
        _ => Vec::new(),
    };
    assert!(!points.is_empty() && points.iter().all(|point| WINDOW_AT.expand(SLACK).contains(*point)), "the mark is the window's: {mark:?}");
    bed.fixture.frame(vec![release(over)]);
    bed.fixture.settle();
    let layout = bed.workspace.borrow().layout.clone();
    let window = layout.floating.first().expect("the window stays");
    assert!(window.panels().any(|panel| *panel == SECOND), "the tab joined the window: {:?}", bed.actions.borrow());
}

/// A window's move is the area's own edit, written each pass, and keeps its drag; a host that replaces the workspace
/// ends the drag, and the release drops nothing.
#[trial]
fn a_replaced_workspace_ends_the_drag_that_the_areas_own_edits_keep() {
    let mut bed = bed(with_window(Some(Node::tabs(vec![DOCKED], 0))), |_| Permissions::ALL);
    let by = Vec2::new(-40.0, 30.0);
    press_and_move(&mut bed.fixture, window_title(), by);
    let moved_to = bed.workspace.borrow().layout.floating[0].rect;
    assert!(dragging(&bed) && moved_to != WINDOW_AT, "the window moves under its drag: {moved_to:?}");
    bed.workspace.borrow_mut().replace(Workspace::new(with_window(Some(Node::tabs(vec![DOCKED, SECOND], 0)))));
    bed.fixture.frame(vec![moved(window_title() + by * 1.25)]);
    assert!(!dragging(&bed), "the replacement ended the drag");
    bed.fixture.frame(vec![release(window_title() + by * 1.25)]);
    bed.fixture.settle();
    assert!(bed.actions.borrow().is_empty(), "nothing dropped: {:?}", bed.actions.borrow());
}

/// A dock area's first pass leaves a drag running in another area alone.
#[trial]
fn an_areas_first_pass_keeps_a_running_drag() {
    let mut bed = bed(with_window(Some(Node::tabs(vec![DOCKED], 0))), |_| Permissions::ALL);
    press_and_move(&mut bed.fixture, window_title(), Vec2::new(-40.0, 30.0));
    assert!(dragging(&bed), "the window drag runs");
    bed.second.set(true);
    bed.fixture.frame(Vec::new());
    assert!(dragging(&bed), "the second area's first pass kept the drag");
}

/// A floating window takes no room in the area: the docked tree reaches as far down as it does without the window.
#[trial]
fn a_floating_window_leaves_the_docked_tree_its_room() {
    let docked_bottom = |layout: Layout<usize>| {
        let bed = bed(layout, |_| Permissions::ALL);
        let panels = bed.fixture.by_role(Role::TAB_PANEL);
        let docked: Vec<f32> =
            panels.iter().filter(|panel| !WINDOW_AT.expand(SLACK).contains_rect(panel.rect)).map(|panel| panel.rect.bottom()).collect();
        assert_eq!(docked.len(), 1, "one docked group shows: {panels:?}");
        docked[0]
    };
    let alone = docked_bottom(Layout::new(Some(Node::tabs(vec![DOCKED], 0)), Vec::new()));
    let beside = docked_bottom(with_window(Some(Node::tabs(vec![DOCKED], 0))));
    assert!((beside - alone).abs() <= SLACK, "the docked group ends at {beside} beside the window, at {alone} without it");
}

/// A floating window's drag bar shows no title: its tab is the one place the panel's name shows in the window.
#[trial]
fn a_floating_windows_drag_bar_shows_no_title() {
    let bed = bed(with_window(Some(Node::tabs(vec![DOCKED], 0))), |_| Permissions::ALL);
    let named = bed.fixture.texts().iter().filter(|text| text.text == title(WINDOW) && WINDOW_AT.contains_rect(text.rect)).count();
    assert_eq!(named, 1, "only the tab names the panel: {:?}", bed.fixture.texts());
}

/// A tab dragged out of the window floats at the width its group had where it was docked, not a default size.
#[trial]
fn a_floated_panel_fits_its_content_up_to_the_float_height() {
    let floated = |bed: &Bed, panel: usize| {
        let layout = bed.workspace.borrow().layout.clone();
        let window = layout.floating.iter().find(|window| window.contains(&panel)).map(|window| window.rect);
        window.unwrap_or_else(|| panic!("panel {panel} floats: {:?}", bed.actions.borrow()))
    };

    // Torn out of the window: the filling panel floats at the group's width and the cap, which a measure of its
    // content alone would not reach.
    let mut torn = bed(Layout::new(Some(Node::tabs(vec![FILL, SHORT], 0)), Vec::new()), |_| Permissions::ALL);
    let group = torn.fixture.by_role(Role::TAB_PANEL).first().expect("the docked group shows").rect;
    let tab = torn.fixture.by_role(Role::TAB).into_iter().min_by(|a, b| a.rect.min.x.total_cmp(&b.rect.min.x)).expect("FILL's tab shows").rect;
    let outside = Pos2::new(group.center().x, -40.0);
    press_and_move(&mut torn.fixture, tab.center(), outside - tab.center());
    torn.fixture.frame(vec![release(outside)]);
    torn.fixture.settle();
    let fill = floated(&torn, FILL);
    let cap = torn.float_height.get();
    assert!(cap < group.height() - SLACK, "the cap {cap} is below the docked group {group:?}");
    assert!((fill.width() - group.width()).abs() <= SLACK, "floated {fill:?}, docked group {group:?}");
    assert!((fill.height() - cap).abs() <= SLACK, "filling content floats at the cap: {fill:?}, cap {cap}");

    // The group dragged by its strip's empty room into its own body and released with the key that floats: the
    // drag carries the measure the area added in the pass it started, and the short panel floats at its content's
    // height.
    let root = Node::split(Axis::X, SplitSize::Ratio(0.5), Node::tabs(vec![SHORT], 0), Node::tabs(vec![DOCKED], 0));
    let mut dragged = bed(Layout::new(Some(root), Vec::new()), |_| Permissions::ALL);
    let tab = dragged.fixture.by_role(Role::TAB).into_iter().min_by(|a, b| a.rect.min.x.total_cmp(&b.rect.min.x)).expect("the tab shows").rect;
    let group = dragged.fixture.by_role(Role::TAB_PANEL).into_iter().find(|group| group.rect.contains(tab.center())).expect("its group shows").rect;
    assert!(cap < group.height() - SLACK, "the cap {cap} is below the docked group {group:?}");
    let grip = Pos2::new(group.center().x + group.width() / 4.0, tab.center().y);
    let inside = group.center();
    press_and_move(&mut dragged.fixture, grip, inside - grip);
    assert!(dragging(&dragged), "the strip's empty room drags the group");
    // The key goes down while the pointer still moves, so the drop target marks the float before the release.
    dragged.fixture.hold(Modifiers::COMMAND);
    dragged.fixture.frame(vec![moved(inside)]);
    dragged.fixture.frame(vec![release(inside)]);
    dragged.fixture.hold(Modifiers::NONE);
    dragged.fixture.settle();
    let short = floated(&dragged, SHORT);
    assert!((short.width() - group.width()).abs() <= SLACK, "floated {short:?}, docked group {group:?}");
    assert!(short.height() < cap - SLACK, "short content floats at its height: {short:?}, cap {cap}");
}
