//! Dock columns: room sharing between groups, and a dock side's total width shared between its columns.

use craft_behavior::Axis;
use craft_behavior::dock::{Action, Column, DockEdge, Floating, Group, Layout, Node, Permissions, SplitSize, Workspace, node_at};
use craft_theme::{AppSizes, SizeValue};
use craft_ui::contract::{Bind, BodyCx, PlainData, Role, Show as _, SizeLimits, Tracked};
use craft_ui::display::Text;
use craft_ui::docking::{
    ColumnCards, DockConfig, DockGrid, DockHeights, DockPanels, DockSizes, GroupAt, GroupSize, IconicMode, PanelSpec, PanelSpecs, group_heights,
};
use craft_ui::patterns::dock::DockArea;
use craft_ui::runtime::env::{Direction, RootEnv};
use craft_ui::testing::{Fixture, pointer, trial};
use egui::{Pos2, Rect, Ui, Vec2};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

const STRIP: f32 = 20.0;
const MIN: f32 = 60.0;
const ROOM: f32 = 400.0;

fn open(min: f32) -> GroupSize {
    GroupSize::new(min)
}

fn sum(heights: &[f32]) -> f32 {
    heights.iter().sum()
}

#[trial]
fn default_heights_give_way_to_the_group_that_fills_before_it_gets_less() {
    let mut roomy = open(MIN);
    roomy.content = Some(DockHeights::new(250.0).compact(100.0));
    let mut filling = open(MIN);
    filling.content = Some(DockHeights::new(0.0).fill(200.0));
    let heights = group_heights(&[roomy, filling], STRIP, ROOM);
    assert_eq!(heights[1], 200.0, "the filling group gets its fill height");
    assert_eq!(heights[0], ROOM - 200.0, "the other group gave way from its default of 250");
    assert!(sum(&heights) <= ROOM, "the groups fit the room");
}

#[trial]
fn only_the_last_open_group_fills_the_column() {
    let asks = || {
        let mut group = open(MIN);
        group.content = Some(DockHeights::new(100.0).compact(80.0).fill(150.0));
        group
    };
    let mut minimized = asks();
    minimized.minimized = true;
    let heights = group_heights(&[asks(), asks(), minimized], STRIP, ROOM);
    assert_eq!(heights[0], 100.0, "an earlier group keeps its default although it asks for a fill");
    assert!(heights[1] > heights[0], "the last open group takes the rest");
    assert_eq!(sum(&heights), ROOM, "the column is filled");
}

/// Canvas 1 beside an end dock: column A holds panel 2 (200 to 400 wide), column B panel 3 (at least 150), and
/// column C, iconic, panel 4.
fn end_dock_layout() -> Layout<u32> {
    let mut iconic = Column::new(vec![Group::new(vec![4], 0)]).with_width(300.0);
    iconic.iconic = true;
    let columns =
        vec![Column::new(vec![Group::new(vec![2], 0)]).with_width(250.0), Column::new(vec![Group::new(vec![3], 0)]).with_width(250.0), iconic];
    Layout::new(Some(Node::split(Axis::X, SplitSize::Ratio(0.7), Node::tabs(vec![1], 0), Node::dock(DockEdge::End, columns))), Vec::new())
}

fn spec(panel: &u32) -> PanelSpec {
    let limits = match panel {
        2 => SizeLimits::new(Vec2::new(200.0, 0.0), Vec2::new(400.0, 10_000.0)),
        3 => SizeLimits::at_least(Vec2::new(150.0, 0.0)),
        _ => SizeLimits::default(),
    };
    PanelSpec::new("Panel").limits(limits)
}

fn end_columns(layout: &Layout<u32>) -> Vec<Column<u32>> {
    let path = layout.dock_path(DockEdge::End).expect("the layout has an end dock");
    match node_at(layout.root.as_ref().unwrap(), &path) {
        Some(Node::Dock { columns, .. }) => columns.clone(),
        other => panic!("{other:?}"),
    }
}

#[trial]
fn a_dock_sides_total_width_scales_its_columns_within_their_bounds() {
    let sizes = super::sizes(4.0, 100.0, STRIP);
    let spec: &dyn Fn(&u32) -> PanelSpec = &spec;
    let mut live = Workspace::new(end_dock_layout());
    let widths = |live: &Workspace<u32>| end_columns(live.layout()).iter().map(|column| sizes.width(column, spec)).collect::<Vec<f32>>();
    let within = |live: &Workspace<u32>| {
        end_columns(live.layout()).iter().zip(widths(live)).all(|(column, width)| {
            let (lo, hi) = sizes.shown_bounds(column, spec);
            (lo - 0.01..=hi + 0.01).contains(&width)
        })
    };
    let start = widths(&live);

    for total in [sum(&start) + 120.0, sum(&start) - 60.0] {
        let actions = sizes.resize_side(live.layout(), DockEdge::End, spec, total);
        live.apply_all(actions, true, |_| Permissions::ALL).expect("resizing works while the workspace is locked");
        assert!((sum(&widths(&live)) - total).abs() < 0.5, "the columns add up to {total}: {:?}", widths(&live));
        assert!(within(&live), "every column stays within its bounds: {:?}", widths(&live));
    }

    let actions = sizes.resize_side(live.layout(), DockEdge::End, spec, 10.0);
    live.apply_all(actions, false, |_| Permissions::ALL).unwrap();
    let least: f32 = end_columns(live.layout()).iter().map(|column| sizes.shown_bounds(column, spec).0).sum();
    assert!((sum(&widths(&live)) - least).abs() < 0.5, "a total below the bounds stops at their sum");
    let iconic = &end_columns(live.layout())[2];
    assert_eq!(iconic.width, Some(300.0), "an iconic column keeps the width it expands to");
    assert!(sizes.resize_side(live.layout(), DockEdge::End, spec, least).is_empty(), "the width it has asks for nothing");
    assert!(sizes.resize_side(live.layout(), DockEdge::Start, spec, 500.0).is_empty(), "a side without a dock asks for nothing");
}

/// The panel that lays its content in a grid, as a toolbar does.
const TOOLS: u32 = 5;

fn grid() -> DockGrid {
    DockGrid::new(40.0, 32.0, 6)
}

/// A stepped column keeps its width, which only its steps set, while the side's other columns scale; a width that
/// its edge is dragged to takes the most whole steps that fit it.
#[trial]
fn a_stepped_column_keeps_its_width_while_the_others_scale() {
    let sizes = super::sizes(4.0, 100.0, STRIP);
    let with_tools = |panel: &u32| if *panel == TOOLS { PanelSpec::new("Tools").grid(grid()) } else { spec(panel) };
    let spec: &dyn Fn(&u32) -> PanelSpec = &with_tools;
    let columns = vec![Column::new(vec![Group::new(vec![2], 0)]).with_width(250.0), Column::new(vec![Group::new(vec![TOOLS], 0)]).with_steps(2)];
    let root = Node::split(Axis::X, SplitSize::Ratio(0.7), Node::tabs(vec![1], 0), Node::dock(DockEdge::End, columns));
    let mut live = Workspace::new(Layout::new(Some(root), Vec::new()));
    let widths = |live: &Workspace<u32>| end_columns(live.layout()).iter().map(|column| sizes.width(column, spec)).collect::<Vec<f32>>();
    let before = widths(&live);
    assert_eq!(before[1], grid().width(2), "a stepped column is as wide as its steps");
    let actions = sizes.resize_side(live.layout(), DockEdge::End, spec, sum(&before) + 80.0);
    let touches_tools = |action: &Action<u32>| matches!(action, Action::ResizeColumn { panel: TOOLS, .. } | Action::SetColumnSteps { .. });
    assert!(!actions.iter().any(touches_tools), "the side's width leaves the stepped column alone: {actions:?}");
    live.apply_all(actions, false, |_| Permissions::ALL).expect("the side resizes");
    let after = widths(&live);
    assert_eq!(after[1], before[1], "the stepped column keeps its width");
    assert!(after[0] > before[0], "the other column took the room: {after:?}");

    let grid = grid();
    assert_eq!(grid.steps_within(grid.width(3) - 1.0), 2, "a width short of three steps takes two");
    assert_eq!(grid.steps_within(grid.width(3)), 3, "the width of three steps takes three");
    assert_eq!(grid.steps_within(0.0), 1, "a column keeps one step");
    assert_eq!(grid.steps_within(f32::MAX), grid.most, "no more steps than the grid has columns");
}

/// A tool rail's grid is sized by the dock's own style: a column of two rail steps is a rail button and its padding
/// plus one more button and gap wide, and the side's width leaves it alone.
#[trial]
fn a_rail_column_steps_in_the_docks_rail_cells() {
    let sizes = super::sizes(4.0, 100.0, STRIP);
    let with_rail = |panel: &u32| if *panel == TOOLS { PanelSpec::new("Tools").grid(DockGrid::rail(6)) } else { spec(panel) };
    let spec: &dyn Fn(&u32) -> PanelSpec = &with_rail;
    let columns = vec![Column::new(vec![Group::new(vec![2], 0)]).with_width(250.0), Column::new(vec![Group::new(vec![TOOLS], 0)]).with_steps(2)];
    let root = Node::split(Axis::X, SplitSize::Ratio(0.7), Node::tabs(vec![1], 0), Node::dock(DockEdge::End, columns));
    let live = Workspace::new(Layout::new(Some(root), Vec::new()));
    let rail = end_columns(live.layout())[1].clone();
    assert_eq!(sizes.width(&rail, spec), sizes.rail_first + sizes.rail_step, "two rail steps in the dock's sizes");
    let actions = sizes.resize_side(live.layout(), DockEdge::End, spec, sum(&[250.0, sizes.rail_first + sizes.rail_step]) + 80.0);
    let touches_tools = |action: &Action<u32>| matches!(action, Action::ResizeColumn { panel: TOOLS, .. } | Action::SetColumnSteps { .. });
    assert!(!actions.is_empty() && !actions.iter().any(touches_tools), "the side's width leaves the rail column alone: {actions:?}");
}

/// A toolbar: a panel whose two lines of content lay out in a grid and never collapse.
#[derive(PlainData)]
struct Tools;

impl PanelSpecs<u32> for Tools {
    fn spec(&self, panel: &u32) -> PanelSpec {
        if *panel == TOOLS { PanelSpec::new("Tools").grid(grid()) } else { PanelSpec::new("Canvas") }
    }

    fn permissions(&self, panel: &u32) -> Permissions {
        if *panel == TOOLS { Permissions::ALL.collapses(false) } else { Permissions::ALL }
    }
}

impl DockPanels<u32> for Tools {
    fn body(&mut self, panel: &u32, ui: &mut Ui, _: &mut BodyCx<'_>) {
        if *panel == TOOLS {
            Text::new("One").show(ui);
            Text::new("Two").show(ui);
        }
    }
}

/// A docked stepped column takes the dock's height, as the references' docked toolbar keeps its surface down to the
/// dock's foot (photocraft#16461), and the dock's divider beside it is the tight gap, as thin as the references'; the
/// double arrow of a column that does not collapse switches it between one and two steps, and is named for the steps
/// it switches to.
#[trial]
fn a_docked_stepped_column_takes_the_docks_height_and_its_double_arrow_switches_one_and_two_steps() {
    let dock = Node::dock(DockEdge::Start, vec![Column::new(vec![Group::new(vec![TOOLS], 0)]).with_steps(1)]);
    let root = Node::split(Axis::X, SplitSize::FixedFirst(120.0), dock, Node::tabs(vec![1], 0));
    let workspace = Rc::new(RefCell::new(Arc::new(Workspace::new(Layout::new(Some(root), Vec::new())))));
    let placed: Rc<RefCell<Vec<(u32, Rect)>>> = Rc::default();
    let gaps: Rc<RefCell<Option<(f32, f32)>>> = Rc::default();
    let (held, seen, specs, gaps_seen) = (Rc::clone(&workspace), Rc::clone(&placed), Tracked::new(Tools), Rc::clone(&gaps));
    let mut fixture = Fixture::new(move |ui| {
        let tight = craft_ui::runtime::env::Env::of(ui).tokens.space.gap.tight;
        *gaps_seen.borrow_mut() = Some((DockSizes::of_scope(ui).gap, tight));
        let mut workspace = held.borrow().clone();
        let out = DockArea::new("Stepped dock", Bind::bound(&mut workspace), &specs, &mut Tools, DockConfig::default()).show(ui);
        *held.borrow_mut() = workspace;
        *seen.borrow_mut() = out.columns;
    });
    fixture.settle();
    let tools = placed.borrow().iter().find(|(panel, _)| *panel == TOOLS).map(|(_, rect)| *rect).expect("the stepped column shows");
    let canvas =
        fixture.by_role(Role::TAB_PANEL).into_iter().map(|group| group.rect).find(|rect| !tools.contains_rect(*rect)).expect("the canvas shows");
    assert!(
        (tools.bottom() - canvas.bottom()).abs() <= 1.0,
        "the column reaches the dock's foot, as the canvas beside it: {tools:?}, canvas {canvas:?}"
    );
    let (gap, tight) = gaps.borrow().expect("the dock showed");
    assert_eq!(gap, tight, "the dock's dividers are the tight gap");

    let steps = |workspace: &Workspace<u32>| match &workspace.layout().root {
        Some(Node::Split { first, .. }) => match first.as_ref() {
            Node::Dock { columns, .. } => columns[0].steps,
            other => panic!("{other:?}"),
        },
        other => panic!("{other:?}"),
    };
    for (name, expected) in [("Show in two columns", 2), ("Show in one column", 1)] {
        let tree = fixture.tree();
        let arrow = fixture
            .by_role(Role::BUTTON)
            .into_iter()
            .find(|button| craft_ui::testing::access::name(tree, button.node) == Some(name))
            .expect("the column bar shows its double arrow, named for the steps it switches to");
        fixture.click(&arrow).settle();
        assert_eq!(steps(&workspace.borrow()), Some(expected), "the double arrow switches the steps");
    }
}

/// A dock area showing only a floating window at `at` with the toolbar in a column of one step.
fn floating_tools(at: Rect) -> (Fixture, Rc<RefCell<Arc<Workspace<u32>>>>) {
    let window = Floating::new(100, Column::new(vec![Group::new(vec![TOOLS], 0)]).with_steps(1), at);
    let workspace = Rc::new(RefCell::new(Arc::new(Workspace::new(Layout::new(None, vec![window])))));
    let (held, specs) = (Rc::clone(&workspace), Tracked::new(Tools));
    let mut fixture = Fixture::new(move |ui| {
        let mut workspace = held.borrow().clone();
        DockArea::new("Floating tools", Bind::bound(&mut workspace), &specs, &mut Tools, DockConfig::default()).show(ui);
        *held.borrow_mut() = workspace;
    });
    fixture.settle();
    (fixture, workspace)
}

/// The rect of the floating toolbar's group.
fn shown_tools(fixture: &Fixture) -> Rect {
    fixture.by_role(Role::TAB_PANEL).first().map(|group| group.rect).expect("the window shows its group")
}

/// A floating stepped window keeps the edge opposite the grip a drag holds: a drag of its left edge widens it in
/// whole steps to the left with its right edge where it was, and a drag of its top edge keeps its bottom edge.
#[trial]
fn a_floating_stepped_windows_left_and_top_edge_drags_keep_the_opposite_edge() {
    let at = Rect::from_min_size(Pos2::new(200.0, 80.0), Vec2::new(100.0, 100.0));
    let (mut fixture, workspace) = floating_tools(at);
    let before = shown_tools(&fixture);
    // The left grip straddles the window's left edge, which is its rect's.
    fixture.drag_at(Pos2::new(at.left(), before.center().y), Vec2::new(-1.5 * (grid().width(2) - grid().width(1)), 0.0)).settle();
    let after = shown_tools(&fixture);
    let steps = workspace.borrow().layout().floating[0].column.steps;
    assert!(steps.is_some_and(|steps| steps > 1), "the drag widened the window by whole steps: {steps:?}");
    assert!((after.right() - before.right()).abs() <= 1.5, "the right edge stays: {before:?}, then {after:?}");
    assert!(after.left() < before.left(), "the window grew to the left: {before:?}, then {after:?}");

    // Raised by its top edge, the content still fits one step: the window keeps its size and its bottom edge.
    let (mut fixture, _) = floating_tools(at);
    let before = shown_tools(&fixture);
    fixture.drag_at(Pos2::new(before.center().x, at.top()), Vec2::new(0.0, -30.0)).settle();
    let after = shown_tools(&fixture);
    assert!((after.bottom() - before.bottom()).abs() <= 1.5, "the bottom edge stays: {before:?}, then {after:?}");
}

/// Two side panels in one group, named and with a line of content each.
#[derive(PlainData)]
struct Sides;

impl PanelSpecs<u32> for Sides {
    fn spec(&self, panel: &u32) -> PanelSpec {
        PanelSpec::new(match panel {
            10 => "Alpha",
            11 => "Beta",
            _ => "Canvas",
        })
    }
}

impl DockPanels<u32> for Sides {
    fn body(&mut self, panel: &u32, ui: &mut Ui, _: &mut BodyCx<'_>) {
        match panel {
            10 => Text::new("Alpha body").show(ui),
            11 => Text::new("Beta body").show(ui),
            _ => return,
        };
    }
}

#[derive(PlainData)]
struct RailWithFooter;

impl PanelSpecs<u32> for RailWithFooter {
    fn spec(&self, panel: &u32) -> PanelSpec {
        PanelSpec::new(format!("Panel {panel}"))
    }
}

impl DockPanels<u32> for RailWithFooter {
    fn body(&mut self, _: &u32, _: &mut Ui, _: &mut BodyCx<'_>) {}

    fn rail_footer(&mut self, _: &u32, ui: &mut Ui, _: &mut BodyCx<'_>) {
        Text::new("Rail controls").show(ui);
        Text::new("More controls").show(ui);
    }
}

/// A rail's host controls stay below its overflowing icons and travel with the dock side.
#[trial]
fn rail_footer_stays_clear_of_icons_on_either_dock_side() {
    for edge in [DockEdge::Start, DockEdge::End] {
        let side = Column::new(vec![Group::new((10..30).collect(), 0)]).with_iconic(true);
        let dock = Node::dock(edge, vec![side]);
        let root = if edge == DockEdge::Start {
            Node::split(Axis::X, SplitSize::Ratio(0.4), dock, Node::tabs(vec![1], 0))
        } else {
            Node::split(Axis::X, SplitSize::Ratio(0.6), Node::tabs(vec![1], 0), dock)
        };
        let workspace = Rc::new(RefCell::new(Arc::new(Workspace::new(Layout::new(Some(root), Vec::new())))));
        let (held, specs) = (Rc::clone(&workspace), Tracked::new(RailWithFooter));
        let actions = Rc::new(RefCell::new(Vec::new()));
        let seen = Rc::clone(&actions);
        let mut fixture = Fixture::new(move |ui| {
            let mut workspace = Arc::clone(&held.borrow());
            let config = DockConfig::default().iconic_mode(IconicMode::Rail);
            let out = DockArea::new("Rail dock", Bind::given(Arc::clone(&workspace)), &specs, &mut RailWithFooter, config).rail_footer(10).show(ui);
            for action in out.actions {
                drop(Arc::make_mut(&mut workspace).apply(action.clone(), false, |_| Permissions::ALL));
                seen.borrow_mut().push(action);
            }
            *held.borrow_mut() = workspace;
        });
        fixture.room(Vec2::new(900.0, 300.0)).settle();
        let footer = fixture.by_label("Rail controls").expect("the rail shows its host controls").rect;
        let more = fixture.by_label("More controls").expect("the rail shows the remaining controls").rect;
        let first = fixture.by_label("Panel 10").expect("the first rail icon shows").rect;
        assert!(first.bottom() <= footer.top(), "icons stay above the fixed controls");
        assert!(footer.bottom() <= more.top(), "the host controls form a column");
        assert!((first.center().x - footer.center().x).abs() < footer.width(), "controls share the rail's side");
        assert!(footer.bottom() <= 300.0, "the controls stay inside the window");
        fixture.frame(vec![pointer::moved(first.center())]);
        for _ in 0..12 {
            fixture.frame(vec![pointer::wheel(-200.0)]);
        }
        fixture.settle();
        let after = fixture.by_label("Rail controls").expect("the controls remain visible").rect;
        let last = fixture.by_label("Panel 29").expect("the last rail icon shows").rect;
        assert!((after.top() - footer.top()).abs() < 1.0, "the controls stay fixed while the icons scroll");
        assert!(last.bottom() <= after.top() && last.top() >= 0.0, "scrolling brings the last icon above the controls: {last:?}, {after:?}");
        let last = fixture.by_label("Panel 29").expect("the last icon can be clicked");
        fixture.click(&last).settle();
        assert!(
            actions.borrow().iter().any(|action| matches!(action, Action::Activate { panel: 29, .. })),
            "the scrolled icon still activates its panel"
        );
        let expanded = fixture.by_label("Rail controls").expect("the controls remain on the expanded rail").rect;
        assert!((expanded.center().x - footer.center().x).abs() < footer.width(), "the controls stay at the same dock edge when expanded");
    }
}

/// With the rail mode, a click on an icon expands its column with that panel in front, and the icons stay beside
/// it, between the panel and the window edge; a click on another icon switches the panel, and a click on the icon of
/// the panel in front collapses the column again.
#[trial]
fn rail_icons_stay_beside_the_expanded_column_and_the_open_panels_icon_collapses_it() {
    let side = Column::new(vec![Group::new(vec![10, 11], 0)]).with_iconic(true);
    let root = Node::split(Axis::X, SplitSize::Ratio(0.6), Node::tabs(vec![1], 0), Node::dock(DockEdge::End, vec![side]));
    let workspace = Rc::new(RefCell::new(Arc::new(Workspace::new(Layout::new(Some(root), Vec::new())))));
    let (held, specs) = (Rc::clone(&workspace), Tracked::new(Sides));
    let mut fixture = Fixture::new(move |ui| {
        let mut workspace = Arc::clone(&held.borrow());
        let config = DockConfig::default().iconic_mode(IconicMode::Rail);
        let out = DockArea::new("Rail dock", Bind::given(Arc::clone(&workspace)), &specs, &mut Sides, config).show(ui);
        for action in out.actions {
            drop(Arc::make_mut(&mut workspace).apply(action, false, |_: &u32| Permissions::ALL));
        }
        *held.borrow_mut() = workspace;
    });
    fixture.room(Vec2::new(900.0, 500.0)).settle();
    let icon = |fixture: &Fixture, name: &str| {
        let tree = fixture.tree();
        fixture
            .by_role(Role::BUTTON)
            .into_iter()
            .find(|button| craft_ui::testing::access::name(tree, button.node) == Some(name))
            .unwrap_or_else(|| panic!("the rail shows the {name} icon"))
    };
    let iconic = || end_columns(workspace.borrow().layout())[0].iconic;
    let shown = |fixture: &Fixture, text: &str| fixture.texts().iter().any(|run| run.text == text);

    let beta = icon(&fixture, "Beta");
    fixture.click(&beta).settle();
    assert!(!iconic(), "a click on an icon expands the column");
    assert!(shown(&fixture, "Beta body") && !shown(&fixture, "Alpha body"), "the clicked panel is in front");
    let (beta, body) = (icon(&fixture, "Beta"), fixture.by_label("Beta body").expect("the panel's content").rect);
    assert!(body.right() <= beta.rect.left(), "the icons stay beside the panel, at the window edge: {body:?}, icon {:?}", beta.rect);

    let alpha = icon(&fixture, "Alpha");
    fixture.click(&alpha).settle();
    assert!(!iconic() && shown(&fixture, "Alpha body") && !shown(&fixture, "Beta body"), "another icon switches the panel in front");

    let alpha = icon(&fixture, "Alpha");
    fixture.click(&alpha).settle();
    assert!(iconic(), "the icon of the panel in front collapses the column");
    assert!(!shown(&fixture, "Alpha body"), "and its panel no longer shows");
}

#[derive(PlainData)]
struct RailWithMenu;

impl PanelSpecs<u32> for RailWithMenu {
    fn spec(&self, panel: &u32) -> PanelSpec {
        PanelSpec::new(format!("Panel {panel}"))
    }
}

impl DockPanels<u32> for RailWithMenu {
    fn body(&mut self, _: &u32, _: &mut Ui, _: &mut BodyCx<'_>) {}

    fn rail_footer(&mut self, _: &u32, ui: &mut Ui, _: &mut BodyCx<'_>) {
        use craft_ui::buttons::IconButton;
        use craft_ui::contract::{PopupKind, Props as _, ShowContainer as _, Trigger};
        use craft_ui::menu::{Menu, MenuItem};
        let opener = IconButton::new(craft_behavior::IconId::new("settings")).label("Rail menu").popup(PopupKind::Menu).show(ui);
        Menu::new("Rail options").opened_by(&opener.response, Trigger::Press).show(ui, |ui, _| {
            MenuItem::new("Rail option").show(ui);
        });
    }
}

/// A popup opened from the fixed rail footer remains interactive at the edge of the window.
#[trial]
fn a_menu_opens_from_the_rail_footer() {
    let side = Column::new(vec![Group::new(vec![10, 11], 0)]).with_iconic(true);
    let root = Node::split(Axis::X, SplitSize::Ratio(0.6), Node::tabs(vec![1], 0), Node::dock(DockEdge::End, vec![side]));
    let workspace = Arc::new(Workspace::new(Layout::new(Some(root), Vec::new())));
    let specs = Tracked::new(RailWithMenu);
    let mut fixture = Fixture::new(move |ui| {
        let config = DockConfig::default().iconic_mode(IconicMode::Rail);
        DockArea::new("Rail dock", Bind::given(Arc::clone(&workspace)), &specs, &mut RailWithMenu, config).rail_footer(10).show(ui);
    });
    fixture.room(Vec2::new(900.0, 500.0)).settle();
    let opener = fixture.by_label("Rail menu").expect("the rail menu button");
    fixture.access(craft_ui::testing::focus::AccessActionKind::Click, &opener).settle();
    assert!(fixture.by_label("Rail option").is_some(), "the footer button opens its menu through accessibility");
    fixture.key(egui::Key::Escape, egui::Modifiers::NONE).settle();
    assert!(fixture.by_label("Rail option").is_none(), "Escape closes the menu");
    fixture.click(&opener).settle();
    assert!(fixture.by_label("Rail option").is_some(), "a pointer press still opens it");
    fixture.key(egui::Key::Escape, egui::Modifiers::NONE).settle();
    fixture.access(craft_ui::testing::focus::AccessActionKind::Focus, &opener).settle();
    fixture.key(egui::Key::Enter, egui::Modifiers::NONE).settle();
    assert!(fixture.by_label("Rail option").is_some(), "Enter opens the focused menu");
    fixture.key(egui::Key::Escape, egui::Modifiers::NONE).settle();
    let at = opener.center();
    fixture.frame(vec![pointer::moved(egui::pos2(100.0, 100.0))]);
    fixture.frame(vec![pointer::moved(at), pointer::press(at), pointer::release(at)]).settle();
    assert!(fixture.by_label("Rail option").is_some(), "a same-frame click opens the menu");
}

/// A toolbar of six rail cells beside a side panel.
#[derive(PlainData)]
struct RailTools;

impl PanelSpecs<u32> for RailTools {
    fn spec(&self, panel: &u32) -> PanelSpec {
        if *panel == TOOLS { PanelSpec::new("Tools").grid(DockGrid::rail(6)) } else { PanelSpec::new("Side") }
    }

    fn permissions(&self, panel: &u32) -> Permissions {
        if *panel == TOOLS { Permissions::ALL.collapses(false).shares_column(false) } else { Permissions::ALL }
    }
}

/// Panels that record the rect the toolbar's group gets, the rail's stage.
struct RailStage(Rc<Cell<Option<Rect>>>);

impl DockPanels<u32> for RailStage {
    fn body(&mut self, _: &u32, _: &mut Ui, _: &mut BodyCx<'_>) {}

    fn group_view(&mut self, panels: &[u32], _: usize, _: &mut Ui, at: GroupAt, _: &mut BodyCx<'_>) -> Option<f32> {
        if !panels.contains(&TOOLS) {
            return None;
        }
        Some(match at {
            GroupAt::Measure(_) => 0.0,
            GroupAt::Show(rect) => {
                self.0.set(Some(rect));
                rect.height()
            }
        })
    }
}

/// A start dock in `direction` of a toolbar column of two rail steps, at the window edge, and a side column beside it,
/// with `DockConfig::column_cards` set to `cards`: the toolbar's cell, the side column's cell and the rail's stage.
fn rail_dock(cards: ColumnCards, direction: Direction) -> (Rect, Rect, Rect) {
    let columns = vec![Column::new(vec![Group::new(vec![TOOLS], 0)]).with_steps(2), Column::new(vec![Group::new(vec![2], 0)]).with_width(150.0)];
    let root = Node::split(Axis::X, SplitSize::FixedFirst(300.0), Node::dock(DockEdge::Start, columns), Node::tabs(vec![1], 0));
    let workspace = Rc::new(RefCell::new(Arc::new(Workspace::new(Layout::new(Some(root), Vec::new())))));
    let placed: Rc<RefCell<Vec<(u32, Rect)>>> = Rc::default();
    let stage: Rc<Cell<Option<Rect>>> = Rc::default();
    let (held, seen, shown, specs) = (Rc::clone(&workspace), Rc::clone(&placed), Rc::clone(&stage), Tracked::new(RailTools));
    let mut fixture = Fixture::new(move |ui| {
        let mut workspace = held.borrow().clone();
        let config = DockConfig::default().column_cards(cards);
        let out = DockArea::new("Rail dock", Bind::bound(&mut workspace), &specs, &mut RailStage(Rc::clone(&shown)), config).show(ui);
        *held.borrow_mut() = workspace;
        *seen.borrow_mut() = out.columns;
    });
    fixture.direction(direction).room(Vec2::new(800.0, 500.0)).settle();
    let cell = |panel: u32| placed.borrow().iter().find(|(at, _)| *at == panel).map(|(_, rect)| *rect).expect("the column shows");
    (cell(TOOLS), cell(2), stage.get().expect("the rail's group shows"))
}

/// A docked tool rail shown as a card is inset alike on its outer side, the window edge's, and on its top and bottom,
/// its bar and grip on the card above the rail as without one, and flush with its inner edge; the column beside it starts past its cell; and its
/// rail gets the same width as without the card, so it shows the same steps, its column wider by the inset. Right to
/// left mirrors the outer side.
#[trial]
fn a_rail_card_is_inset_alike_on_three_sides_and_keeps_its_steps() {
    for direction in [Direction::Ltr, Direction::Rtl] {
        let rtl = direction == Direction::Rtl;
        let (plain_cell, _, plain) = rail_dock(ColumnCards::None, direction);
        let (cell, side, card) = rail_dock(ColumnCards::All, direction);
        let (outer, inner) =
            if rtl { (cell.right() - card.right(), card.left() - cell.left()) } else { (card.left() - cell.left(), cell.right() - card.right()) };
        let top = (card.top() - cell.top()) - (plain.top() - plain_cell.top());
        let bottom = (cell.bottom() - card.bottom()) - (plain_cell.bottom() - plain.bottom());
        assert!(outer > 0.5, "{direction:?}: the card is inset from the window edge: {card:?} in {cell:?}");
        assert!((top - outer).abs() <= 0.5 && (bottom - outer).abs() <= 0.5, "{direction:?}: insets outer {outer}, top {top}, bottom {bottom}");
        assert!(inner.abs() <= 0.5, "{direction:?}: the card keeps to the inner edge: {card:?} in {cell:?}");
        let clear = if rtl { side.right() <= cell.left() + 0.5 } else { side.left() >= cell.right() - 0.5 };
        assert!(clear, "{direction:?}: the side column {side:?} overlaps the toolbar {cell:?}");
        assert!((card.width() - plain.width()).abs() <= 0.5, "{direction:?}: the rail's stage {card:?} differs from {plain:?} without the card");
        assert!(
            (cell.width() - plain_cell.width() - outer).abs() <= 0.5,
            "{direction:?}: the column {cell:?} grows by the inset from {plain_cell:?}"
        );
    }
}

/// The panel of the card dock's expanded column, and of its column collapsed to icons.
const SIDE: u32 = 2;
const ICONS: u32 = 4;

/// Panels named for the card dock: an expanded column's panel, and a collapsed column's, whose icon the trial finds by
/// its name.
#[derive(PlainData)]
struct CardPanels;

impl PanelSpecs<u32> for CardPanels {
    fn spec(&self, panel: &u32) -> PanelSpec {
        PanelSpec::new(if *panel == ICONS { "Icons" } else { "Side" })
    }
}

/// Panels that record the rect the expanded column's group gets.
struct CardStage(Rc<Cell<Option<Rect>>>);

impl DockPanels<u32> for CardStage {
    fn body(&mut self, panel: &u32, _: &mut Ui, body: &mut BodyCx<'_>) {
        if *panel == SIDE {
            self.0.set(Some(body.rect));
        }
    }
}

/// What a card dock trial reads: the cells of the expanded column and of the column of icons, the body of the expanded
/// column's panel and the icon's cell.
struct CardDock {
    side: Rect,
    icons: Rect,
    body: Rect,
    icon: Rect,
}

/// An end dock in `direction` beside the canvas: an expanded column at the window edge and, inward of it, a column
/// collapsed to icons, with `DockConfig::column_cards` set to `cards`.
fn card_dock(cards: ColumnCards, direction: Direction) -> CardDock {
    let mut iconic = Column::new(vec![Group::new(vec![ICONS], 0)]);
    iconic.iconic = true;
    let columns = vec![Column::new(vec![Group::new(vec![SIDE], 0)]).with_width(220.0), iconic];
    let root = Node::split(Axis::X, SplitSize::Ratio(0.6), Node::tabs(vec![1], 0), Node::dock(DockEdge::End, columns));
    let workspace = Rc::new(RefCell::new(Arc::new(Workspace::new(Layout::new(Some(root), Vec::new())))));
    let placed: Rc<RefCell<Vec<(u32, Rect)>>> = Rc::default();
    let body: Rc<Cell<Option<Rect>>> = Rc::default();
    let (held, seen, shown, specs) = (Rc::clone(&workspace), Rc::clone(&placed), Rc::clone(&body), Tracked::new(CardPanels));
    let mut fixture = Fixture::new(move |ui| {
        let mut workspace = held.borrow().clone();
        let config = DockConfig::default().column_cards(cards);
        let out = DockArea::new("Card dock", Bind::bound(&mut workspace), &specs, &mut CardStage(Rc::clone(&shown)), config).show(ui);
        *held.borrow_mut() = workspace;
        *seen.borrow_mut() = out.columns;
    });
    fixture.direction(direction).room(Vec2::new(900.0, 500.0)).settle();
    let cell = |panel: u32| placed.borrow().iter().find(|(at, _)| *at == panel).map(|(_, rect)| *rect).expect("the column shows");
    let icon = fixture.by_label("Icons").expect("the icon shows").rect;
    CardDock { side: cell(SIDE), icons: cell(ICONS), body: body.get().expect("the side panel shows"), icon }
}

/// The insets of `inner` in `cell` on the outer side, the window edge's (the right in left to right), and on the inner
/// side.
fn sides(cell: Rect, inner: Rect, rtl: bool) -> (f32, f32) {
    if rtl { (inner.left() - cell.left(), cell.right() - inner.right()) } else { (cell.right() - inner.right(), inner.left() - cell.left()) }
}

/// Every docked column shows as a card: the column at the window edge is inset alike on its outer side and its foot,
/// the column of icons inward of it stands a gutter from its card, narrower than the edge's inset, and both cards keep
/// to their inner edges. The columns do not overlap, and each is wider than without cards by its inset, so its
/// content keeps its width. Right to left mirrors the outer sides.
#[trial]
fn every_docked_column_is_a_card_with_gutters_between_them() {
    for direction in [Direction::Ltr, Direction::Rtl] {
        let rtl = direction == Direction::Rtl;
        let plain = card_dock(ColumnCards::None, direction);
        let cards = card_dock(ColumnCards::All, direction);
        // Each inset is measured against the same column without cards, so a panel's own padding cancels out.
        let inset = |cell: Rect, inner: Rect, plain_cell: Rect, plain_inner: Rect| {
            let ((outer, inward), (plain_outer, plain_inward)) = (sides(cell, inner, rtl), sides(plain_cell, plain_inner, rtl));
            (outer - plain_outer, inward - plain_inward)
        };
        let (edge, side_inner) = inset(cards.side, cards.body, plain.side, plain.body);
        let foot = (cards.side.bottom() - cards.body.bottom()) - (plain.side.bottom() - plain.body.bottom());
        assert!(edge > 0.5, "{direction:?}: the edge column's card is inset from the window edge: {:?} in {:?}", cards.body, cards.side);
        assert!((foot - edge).abs() <= 0.5, "{direction:?}: the edge column's card is inset {edge} at the edge and {foot} at its foot");
        assert!(side_inner.abs() <= 0.5, "{direction:?}: the edge column's card keeps to its inner edge");
        let (gutter, icons_inner) = inset(cards.icons, cards.icon, plain.icons, plain.icon);
        assert!(
            gutter > 0.5 && gutter < edge,
            "{direction:?}: the icons' card stands a gutter {gutter} from its neighbour's, under the edge's {edge}"
        );
        assert!(icons_inner.abs() <= 0.5, "{direction:?}: the icons' card keeps to its inner edge");
        let apart = if rtl { cards.icons.left() >= cards.side.right() - 0.5 } else { cards.icons.right() <= cards.side.left() + 0.5 };
        assert!(apart, "{direction:?}: the icons' column {:?} overlaps the edge column {:?}", cards.icons, cards.side);
        assert!((cards.body.width() - plain.body.width()).abs() <= 0.5, "{direction:?}: the panel keeps its width in its card");
        assert!((cards.side.width() - plain.side.width() - edge).abs() <= 0.5, "{direction:?}: the edge column grows by its inset");
        assert!((cards.icons.width() - plain.icons.width() - gutter).abs() <= 0.5, "{direction:?}: the icons' column grows by its gutter");
    }
}

/// The top of the expanded column's panel body in the card dock, with the column bar `size.bar.column` set to `bar`
/// points when given.
fn body_top(bar: Option<f32>) -> f32 {
    let mut iconic = Column::new(vec![Group::new(vec![ICONS], 0)]);
    iconic.iconic = true;
    let columns = vec![Column::new(vec![Group::new(vec![SIDE], 0)]).with_width(220.0), iconic];
    let root = Node::split(Axis::X, SplitSize::Ratio(0.6), Node::tabs(vec![1], 0), Node::dock(DockEdge::End, columns));
    let workspace = Rc::new(RefCell::new(Arc::new(Workspace::new(Layout::new(Some(root), Vec::new())))));
    let body: Rc<Cell<Option<Rect>>> = Rc::default();
    let (held, shown, specs) = (Rc::clone(&workspace), Rc::clone(&body), Tracked::new(CardPanels));
    let mut fixture = Fixture::new(move |ui| {
        let mut workspace = held.borrow().clone();
        let _ = DockArea::new("Bar dock", Bind::bound(&mut workspace), &specs, &mut CardStage(Rc::clone(&shown)), DockConfig::default()).show(ui);
        *held.borrow_mut() = workspace;
    });
    let mut root = RootEnv::default();
    if let Some(bar) = bar {
        let mut sizes = AppSizes::default();
        sizes.all.insert("size.bar.column".to_owned(), SizeValue::Number(bar));
        root.sizes = Arc::new(sizes);
    }
    fixture.root(root).room(Vec2::new(900.0, 500.0)).settle();
    body.get().expect("the side panel shows").top()
}

/// A dock column's bar is `size.bar.column` high: an app that sets it lower (VectorCraft's 14 points) raises the
/// column's groups by the difference from the built-in small control height.
#[trial]
fn a_columns_bar_is_as_high_as_its_token() {
    let built_in = body_top(None);
    let low = body_top(Some(14.0));
    assert!(low < built_in - 0.5, "a lower column bar raises the column's groups: {low} against {built_in}");
}

/// `ColumnCards::Stepped` (VectorCraft's dock) shows the tool rail as the card `ColumnCards::All` shows, and the panel
/// column beside it flush, as wide as with no cards; neither overlaps the other. Right to left mirrors it.
#[trial]
fn only_the_tool_rail_is_a_card_with_stepped_cards() {
    for direction in [Direction::Ltr, Direction::Rtl] {
        let (_, plain_side, _) = rail_dock(ColumnCards::None, direction);
        let (all_cell, _, all_stage) = rail_dock(ColumnCards::All, direction);
        let (cell, side, stage) = rail_dock(ColumnCards::Stepped, direction);
        assert!(
            (stage.min - all_stage.min).length() <= 0.5 && (stage.size() - all_stage.size()).length() <= 0.5,
            "{direction:?}: the rail's card {stage:?} is the one all cards give, {all_stage:?}"
        );
        assert!((cell.width() - all_cell.width()).abs() <= 0.5, "{direction:?}: the toolbar {cell:?} is as wide as with all cards, {all_cell:?}");
        assert!(
            (side.width() - plain_side.width()).abs() <= 0.5,
            "{direction:?}: the panel column {side:?} is no card: as wide as {plain_side:?} without cards"
        );
        let clear = if direction == Direction::Rtl { side.right() <= cell.left() + 0.5 } else { side.left() >= cell.right() - 0.5 };
        assert!(clear, "{direction:?}: the panel column {side:?} overlaps the toolbar {cell:?}");
    }
}

/// Panels that record the rect of the canvas's group (panel 1), which they draw themselves.
struct CanvasStage(Rc<Cell<Option<Rect>>>);

impl DockPanels<u32> for CanvasStage {
    fn body(&mut self, _: &u32, _: &mut Ui, _: &mut BodyCx<'_>) {}

    fn group_view(&mut self, panels: &[u32], _: usize, _: &mut Ui, at: GroupAt, _: &mut BodyCx<'_>) -> Option<f32> {
        if !panels.contains(&1) {
            return None;
        }
        Some(match at {
            GroupAt::Measure(_) => 0.0,
            GroupAt::Show(rect) => {
                self.0.set(Some(rect));
                rect.height()
            }
        })
    }
}

/// The room between the canvas and the end dock of the card dock's columns, with `DockConfig::flush_docks` set to
/// `flush`.
fn canvas_to_dock(flush: bool, direction: Direction) -> f32 {
    let mut iconic = Column::new(vec![Group::new(vec![ICONS], 0)]);
    iconic.iconic = true;
    let columns = vec![Column::new(vec![Group::new(vec![SIDE], 0)]).with_width(220.0), iconic];
    let root = Node::split(Axis::X, SplitSize::Ratio(0.6), Node::tabs(vec![1], 0), Node::dock(DockEdge::End, columns));
    let workspace = Rc::new(RefCell::new(Arc::new(Workspace::new(Layout::new(Some(root), Vec::new())))));
    let placed: Rc<RefCell<Vec<(u32, Rect)>>> = Rc::default();
    let canvas: Rc<Cell<Option<Rect>>> = Rc::default();
    let (held, seen, shown, specs) = (Rc::clone(&workspace), Rc::clone(&placed), Rc::clone(&canvas), Tracked::new(CardPanels));
    let mut fixture = Fixture::new(move |ui| {
        let mut workspace = held.borrow().clone();
        let config = DockConfig::default().flush_docks(flush);
        let out = DockArea::new("Flush dock", Bind::bound(&mut workspace), &specs, &mut CanvasStage(Rc::clone(&shown)), config).show(ui);
        *held.borrow_mut() = workspace;
        *seen.borrow_mut() = out.columns;
    });
    fixture.direction(direction).room(Vec2::new(900.0, 500.0)).settle();
    let icons = placed.borrow().iter().find(|(at, _)| *at == ICONS).map(|(_, rect)| *rect).expect("the icons' column shows");
    let canvas = canvas.get().expect("the canvas shows");
    if direction == Direction::Rtl { canvas.left() - icons.right() } else { icons.left() - canvas.right() }
}

/// A flush dock (`DockConfig::flush_docks`, VectorCraft's) meets the canvas with no gap and without overlapping it; by
/// default a gap parts them. Right to left mirrors it.
#[trial]
fn a_flush_dock_meets_the_canvas() {
    for direction in [Direction::Ltr, Direction::Rtl] {
        let (gap, flush) = (canvas_to_dock(false, direction), canvas_to_dock(true, direction));
        assert!(gap > 0.5, "{direction:?}: a gap {gap} parts the canvas from the dock by default");
        assert!(flush.abs() <= 0.5, "{direction:?}: the flush dock stands {flush} from the canvas");
    }
}
