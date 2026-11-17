//! Floating windows of a `DockArea`: a drop over a window lands on it.

use craft_behavior::Axis;
use craft_behavior::dock::{Action, Column, DockEdge, Floating, Group, Layout, Node, Permissions, SplitSize, Workspace};
use craft_ui::contract::{Bind, BodyCx, Role, Show as _, ShowContainer as _};
use craft_ui::display::Text;
use craft_ui::docking::{DockConfig, DockPanels, GroupDrag, PanelSpec, request_actions};
use craft_ui::layout_components::Stack;
use craft_ui::patterns::dock::DockArea;
use craft_ui::runtime::drag::{self, DropMark};
use craft_ui::testing::pointer::{moved, press, release, secondary};
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
    /// The height of a floating window's title row (`FloatSpace::title_row`), as the area reported it.
    title_row: Rc<Cell<f32>>,
}

fn bed(layout: Layout<usize>, rights: fn(&usize) -> Permissions) -> Bed {
    let workspace = Rc::new(RefCell::new(Workspace::new(layout)));
    let actions: Rc<RefCell<Vec<Action<usize>>>> = Rc::default();
    let second = Rc::new(Cell::new(false));
    let float_height = Rc::new(Cell::new(0.0));
    let title_row = Rc::new(Cell::new(0.0));
    let (held, seen_actions, shows_second, seen_height) = (Rc::clone(&workspace), Rc::clone(&actions), Rc::clone(&second), Rc::clone(&float_height));
    let seen_title_row = Rc::clone(&title_row);
    let mut other = Workspace::new(Layout::new(Some(Node::tabs(vec![SECOND], 0)), Vec::new()));
    let mut fixture = Fixture::new(move |ui| {
        let mut workspace = held.borrow().clone();
        let mut panels = Panels { rights };
        let out = DockArea::new("Floating dock", Bind::bound(&mut workspace), &mut panels, DockConfig::default()).show(ui);
        *held.borrow_mut() = workspace;
        seen_actions.borrow_mut().extend(out.actions);
        if let Some(space) = out.float_space {
            seen_height.set(space.default_size.y);
            seen_title_row.set(space.title_row);
        }
        if shows_second.get() {
            DockArea::new("Second dock", Bind::bound(&mut other), &mut panels, DockConfig::default()).show(ui);
        }
    });
    fixture.settle();
    Bed { fixture, workspace, actions, second, float_height, title_row }
}

/// A point on the floating window's title row, on its band: clear of the actions and the double arrow at its end.
fn window_title() -> Pos2 {
    Pos2::new(WINDOW_AT.min.x + 40.0, WINDOW_AT.min.y + 12.0)
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
        DropMark::Into(rect) | DropMark::Region { area: rect, .. } => vec![rect.center()],
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
    let dropped = bed.actions.borrow().iter().any(|action| !matches!(action, Action::MoveFloating { .. }));
    assert!(!dropped, "nothing dropped, only the window moved: {:?}", bed.actions.borrow());
}

/// A body that asks the running pass to bring its own panel forward, every pass.
struct Requesting;

impl DockPanels<usize> for Requesting {
    fn spec(&self, panel: &usize) -> PanelSpec {
        PanelSpec::new(title(*panel))
    }

    fn body(&mut self, panel: &usize, ui: &mut Ui, _: &mut BodyCx<'_>) {
        assert!(request_actions(ui.ctx(), vec![Action::activate(*panel)]).is_ok(), "the running pass takes the request");
    }
}

/// A given workspace is never written: the host gets every change of the pass as actions, a window's move included,
/// and its bodies' requests unapplied beside them.
#[trial]
fn a_given_workspace_gets_the_window_move_and_the_bodies_requests_as_actions() {
    let workspace = Rc::new(RefCell::new(Workspace::new(with_window(Some(Node::tabs(vec![DOCKED], 0))))));
    let requested: Rc<RefCell<Vec<Action<usize>>>> = Rc::default();
    let (held, seen) = (Rc::clone(&workspace), Rc::clone(&requested));
    let mut fixture = Fixture::new(move |ui| {
        let given = held.borrow().clone();
        let mut panels = Requesting;
        let out = DockArea::new("Given dock", Bind::given(given), &mut panels, DockConfig::default()).show(ui);
        held.borrow_mut().apply_all(out.actions, false, |_| Permissions::ALL).expect("the area's actions apply");
        seen.borrow_mut().extend(out.requested);
    });
    fixture.settle();
    assert!(request_actions(fixture.ctx(), vec![Action::activate(DOCKED)]).is_err(), "no pass runs between frames");
    let asked = requested.borrow().clone();
    let own = |action: &Action<usize>| *action == Action::activate(DOCKED) || *action == Action::activate(WINDOW);
    assert!(!asked.is_empty() && asked.iter().all(own), "each body's request came back: {asked:?}");
    assert_eq!(workspace.borrow().layout.floating[0].rect, WINDOW_AT, "nothing moved yet");
    press_and_move(&mut fixture, window_title(), Vec2::new(-40.0, 30.0));
    let moved_to = workspace.borrow().layout.floating[0].rect;
    assert!(moved_to != WINDOW_AT, "the host's copy took the window's move: {moved_to:?}");
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

/// The title row the area keeps a dropped window's under the pointer and adds to a floated unit's height is the
/// one the window draws: the window's group starts right under it.
#[trial]
fn a_floating_windows_group_starts_under_the_title_row_the_area_sizes_windows_by() {
    let bed = bed(with_window(Some(Node::tabs(vec![DOCKED], 0))), |_| Permissions::ALL);
    let group = bed
        .fixture
        .by_role(Role::TAB_PANEL)
        .into_iter()
        .find(|group| WINDOW_AT.expand(SLACK).contains_rect(group.rect))
        .expect("the window shows its group")
        .rect;
    let title_row = bed.title_row.get();
    assert!(title_row > 0.0, "the area reported its float space");
    assert!(
        (group.top() - WINDOW_AT.top() - title_row).abs() <= SLACK,
        "the group starts at {} in a window from {} with a title row of {title_row}",
        group.top(),
        WINDOW_AT.top()
    );
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

/// A floating window's title row is its column's bar: its double arrow collapses the column to icons, which show in
/// the window in place of the groups, and expands it again; the window keeps its expanded rect throughout.
#[trial]
fn a_floating_windows_double_arrow_collapses_its_column_to_icons_and_expands_it() {
    let mut bed = bed(with_window(Some(Node::tabs(vec![DOCKED], 0))), |_| Permissions::ALL);
    let button = |bed: &Bed, name: &str| {
        let tree = bed.fixture.tree();
        bed.fixture
            .by_role(Role::BUTTON)
            .into_iter()
            .find(|button| WINDOW_AT.expand(SLACK).contains_rect(button.rect) && craft_ui::testing::access::name(tree, button.node) == Some(name))
    };
    let in_window =
        |bed: &Bed| bed.fixture.by_role(Role::TAB_PANEL).into_iter().filter(|panel| WINDOW_AT.expand(SLACK).contains_rect(panel.rect)).count();
    assert_eq!(in_window(&bed), 1, "the expanded window shows its group");
    let collapse = button(&bed, "Collapse column to icons").expect("the title row shows the column's double arrow");
    bed.fixture.click(&collapse);
    bed.fixture.settle();
    let window = bed.workspace.borrow().layout.floating[0].clone();
    assert!(window.column.iconic, "the window's column collapsed: {:?}", bed.actions.borrow());
    assert_eq!(window.rect, WINDOW_AT, "the window keeps its expanded rect");
    assert_eq!(in_window(&bed), 0, "the collapsed window shows no group");
    let icon = button(&bed, &title(WINDOW)).expect("the collapsed window shows the panel's icon");
    assert!(icon.rect.width() < WINDOW_AT.width() / 2.0, "the window shows as a strip of icons: {:?}", icon.rect);
    let expand = button(&bed, "Expand column").expect("the collapsed window's title row shows the double arrow");
    bed.fixture.click(&expand);
    bed.fixture.settle();
    let window = bed.workspace.borrow().layout.floating[0].clone();
    assert!(!window.column.iconic && window.rect == WINDOW_AT, "the column expanded at its rect: {window:?}");
    assert_eq!(in_window(&bed), 1, "the expanded window shows its group again");
}

/// The window of `with_window` at `rect` with its column collapsed to icons, and no docked tree under it.
fn collapsed_at(rect: Rect) -> Layout<usize> {
    let mut window = Floating::single(vec![WINDOW], 0, rect);
    window.column.iconic = true;
    Layout::new(None, vec![window])
}

/// The rect of the button named `name`.
fn button_rect(bed: &Bed, name: &str) -> Option<Rect> {
    let tree = bed.fixture.tree();
    bed.fixture
        .by_role(Role::BUTTON)
        .into_iter()
        .find(|button| craft_ui::testing::access::name(tree, button.node) == Some(name))
        .map(|button| button.rect)
}

/// A collapsed window keeps to the edge it kept when its move began: dragged across the middle of the float area,
/// its strip stays under the pointer, and it stays where it is dropped. So does a strip dropped in the middle band,
/// where the strip is nearer one edge and its window's expanded rect the other.
#[trial]
fn a_collapsed_window_dragged_across_the_middle_keeps_its_strip_under_the_pointer() {
    // Across the middle; then to the strip's centre just past the middle of the 400-point room, on the end side,
    // while the expanded rect hanging from it reaches past the middle to the start side.
    for middle in [None, Some(210.0)] {
        let mut bed = bed(collapsed_at(WINDOW_AT), |_| Permissions::ALL);
        let icon = button_rect(&bed, &title(WINDOW)).expect("the collapsed window shows its icon");
        let by = Vec2::new(middle.map_or(-250.0, |x: f32| x - icon.center().x), 0.0);
        // The title row's band, clear of the double arrow at the end the window keeps to.
        let band = Pos2::new(icon.left() + 4.0, WINDOW_AT.top() + 6.0);
        bed.fixture.drag_hold_at(band, by);
        let held = button_rect(&bed, &title(WINDOW)).expect("the icon shows while the window moves");
        assert!((held.left() - (icon.left() + by.x)).abs() <= SLACK, "by {by:?}: the strip follows the pointer: {icon:?}, then {held:?}");
        bed.fixture.release_at(band + by).settle();
        bed.fixture.idle(3);
        let dropped = button_rect(&bed, &title(WINDOW)).expect("the icon shows after the move");
        assert!((dropped.left() - held.left()).abs() <= SLACK, "by {by:?}: the strip stays where it was dropped: {held:?}, then {dropped:?}");
        let window = bed.workspace.borrow().layout.floating[0].clone();
        assert!(window.column.iconic && window.edge.is_some(), "by {by:?}: the window stays collapsed and keeps its edge: {window:?}");
    }
}

/// An icon dragged out of a docked collapsed column and floated in the middle band of the float area floats
/// collapsed, its strip under the pointer, and stays there.
#[trial]
fn an_icon_floated_in_the_middle_band_lands_collapsed_under_the_pointer() {
    let mut column = Column::new(vec![Group::new(vec![WINDOW], 0)]);
    column.iconic = true;
    let root = Node::split(Axis::X, SplitSize::Ratio(0.8), Node::tabs(vec![DOCKED], 0), Node::dock(DockEdge::End, vec![column]));
    let mut bed = bed(Layout::new(Some(root), Vec::new()), |_| Permissions::ALL);
    let icon = button_rect(&bed, &title(WINDOW)).expect("the docked collapsed column shows its icon");
    // Just past the middle of the 400-point room: the strip is nearer the end edge, a window of the float width
    // around it the start edge.
    let at = Pos2::new(210.0, 150.0);
    press_and_move(&mut bed.fixture, icon.center(), at - icon.center());
    // The key that floats goes down while the pointer still moves, so the drop target marks the float.
    bed.fixture.hold(Modifiers::COMMAND);
    bed.fixture.frame(vec![moved(at)]);
    bed.fixture.frame(vec![release(at)]);
    bed.fixture.hold(Modifiers::NONE);
    bed.fixture.settle();
    let window = bed.workspace.borrow().layout.floating.first().cloned();
    let window = window.unwrap_or_else(|| panic!("the icon floats: {:?}", bed.actions.borrow()));
    assert!(window.column.iconic && window.edge.is_some(), "it floats collapsed, keeping to the edge its strip was dropped near: {window:?}");
    let floated = button_rect(&bed, &title(WINDOW)).expect("the floating window shows its icon");
    assert!(floated.x_range().contains(at.x), "the strip lands under the pointer: {floated:?}, released at {at:?}");
    bed.fixture.idle(3);
    let settled = button_rect(&bed, &title(WINDOW)).expect("the icon still shows");
    assert!((settled.left() - floated.left()).abs() <= SLACK, "the strip stays: {floated:?}, then {settled:?}");
}

/// A floating collapsed column opens its flyouts towards the room of the float area, away from the edge it keeps to.
#[trial]
fn a_floating_collapsed_columns_flyout_opens_towards_the_room() {
    // In the end half of the area, then in the start half.
    for (rect, opens_right) in [(WINDOW_AT, false), (WINDOW_AT.translate(Vec2::new(-110.0, 0.0)), true)] {
        let mut bed = bed(collapsed_at(rect), |_| Permissions::ALL);
        let icon = button_rect(&bed, &title(WINDOW)).expect("the collapsed window shows its icon");
        bed.fixture.click_at(icon.center()).settle();
        let flyout = bed.fixture.by_role(Role::TAB_PANEL).first().map(|group| group.rect).expect("the flyout shows the panel's group");
        let towards = if opens_right { flyout.left() >= icon.right() - SLACK } else { flyout.right() <= icon.left() + SLACK };
        assert!(towards, "window at {rect:?}: the flyout {flyout:?} opens on the side with room of the icon {icon:?}");
    }
}

/// A double click on a floating window's title band collapses its column, as one on a dock column's bar does, and a
/// double click on the collapsed window's band expands it again; the groups keep their state.
#[trial]
fn a_double_click_on_a_floating_windows_band_collapses_and_expands_its_column() {
    let mut bed = bed(with_window(None), |_| Permissions::ALL);
    let band = Pos2::new(WINDOW_AT.left() + 40.0, WINDOW_AT.top() + 6.0);
    bed.fixture.click_at(band).click_at(band).settle();
    let window = bed.workspace.borrow().layout.floating[0].clone();
    assert!(window.column.iconic, "the band's double click collapsed the column: {:?}", bed.actions.borrow());
    assert!(window.groups().iter().all(|group| !group.minimized), "no group minimized");
    let icon = button_rect(&bed, &title(WINDOW)).expect("the collapsed window shows its icon");
    let band = Pos2::new(icon.left() + 4.0, WINDOW_AT.top() + 6.0);
    bed.fixture.click_at(band).click_at(band).settle();
    assert!(!bed.workspace.borrow().layout.floating[0].column.iconic, "a second double click expanded it: {:?}", bed.actions.borrow());
}

/// A floating window's title row is its column's bar, with the double arrow and Close and no menu button: a context
/// request on its band opens the window's menu, collapsed or expanded, and the menu's Close entry closes the window.
#[trial]
fn a_context_request_on_a_windows_band_opens_its_menu_and_close_closes_it() {
    for collapsed in [true, false] {
        let mut bed = if collapsed { bed(collapsed_at(WINDOW_AT), |_| Permissions::ALL) } else { bed(with_window(None), |_| Permissions::ALL) };
        let band = if collapsed {
            let icon = button_rect(&bed, &title(WINDOW)).expect("the collapsed window shows its icon");
            Pos2::new(icon.left() + 4.0, WINDOW_AT.top() + 6.0)
        } else {
            window_title()
        };
        bed.fixture.frame(vec![moved(band)]);
        bed.fixture.frame(vec![secondary(band, true)]);
        bed.fixture.frame(vec![secondary(band, false)]);
        bed.fixture.settle();
        let tree = bed.fixture.tree();
        let items: Vec<_> =
            bed.fixture.by_role(Role::MENU_ITEM).into_iter().map(|item| (craft_ui::testing::access::name(tree, item.node), item)).collect();
        let names: Vec<_> = items.iter().map(|(name, _)| *name).collect();
        let close = items.iter().find(|(name, _)| name.is_some_and(|name| name.starts_with("Close"))).map(|(_, item)| *item);
        let close = close.unwrap_or_else(|| panic!("collapsed {collapsed}: the band's context request opens the window's menu: {names:?}"));
        bed.fixture.click(&close);
        bed.fixture.settle();
        assert!(bed.workspace.borrow().layout.floating.is_empty(), "collapsed {collapsed}: Close closed the window: {:?}", bed.actions.borrow());
    }
}

/// Nothing goes beside a floating window: a tab dragged over the start side of its group marks the join of the
/// group, not a split; and a tab dropped on a collapsed window's icon joins its group, which stays collapsed.
#[trial]
fn a_drag_over_a_floating_window_joins_or_stacks_and_never_splits() {
    let docked_tab = |bed: &Bed| {
        let tabs = bed.fixture.by_role(Role::TAB);
        let mut docked: Vec<_> = tabs.iter().filter(|tab| !WINDOW_AT.contains(tab.center())).map(|tab| tab.rect).collect();
        docked.sort_by(|a, b| a.min.x.total_cmp(&b.min.x));
        docked.last().copied().expect("the second docked tab shows")
    };
    let mut expanded = bed(with_window(Some(Node::tabs(vec![DOCKED, SECOND], 0))), |_| Permissions::ALL);
    let tab = docked_tab(&expanded);
    let side = Pos2::new(WINDOW_AT.left() + 6.0, WINDOW_AT.center().y + 20.0);
    press_and_move(&mut expanded.fixture, tab.center(), side - tab.center());
    expanded.fixture.frame(Vec::new());
    let mark = drag::mark(expanded.fixture.ctx()).expect("the window takes the drop");
    assert!(matches!(mark, DropMark::Region { .. }), "the start side of a floating group joins it: {mark:?}");
    expanded.fixture.frame(vec![release(side)]).settle();

    let mut window = Floating::single(vec![WINDOW], 0, WINDOW_AT);
    window.column.iconic = true;
    let mut collapsed = bed(Layout::new(Some(Node::tabs(vec![DOCKED, SECOND], 0)), vec![window]), |_| Permissions::ALL);
    let tab = docked_tab(&collapsed);
    let icon = button_rect(&collapsed, &title(WINDOW)).expect("the collapsed window shows its icon");
    press_and_move(&mut collapsed.fixture, tab.center(), icon.center() - tab.center());
    collapsed.fixture.frame(Vec::new());
    collapsed.fixture.frame(vec![release(icon.center())]).settle();
    let window = collapsed.workspace.borrow().layout.floating[0].clone();
    assert!(window.groups().len() == 1 && window.contains(&SECOND), "the tab joined the icon's group: {:?}", collapsed.actions.borrow());
    assert!(window.column.iconic, "the window stays collapsed");
}

/// A settled collapsed window runs no more passes than a settled expanded one.
#[trial]
fn a_settled_collapsed_window_runs_no_more_passes_than_an_expanded_one() {
    let passes = |layout: Layout<usize>| {
        let mut bed = bed(layout, |_| Permissions::ALL);
        bed.fixture.idle(3);
        bed.fixture.passes()
    };
    let (collapsed, expanded) = (passes(collapsed_at(WINDOW_AT)), passes(with_window(None)));
    assert!(collapsed <= expanded, "a still frame runs {collapsed} passes collapsed, {expanded} expanded");
}
