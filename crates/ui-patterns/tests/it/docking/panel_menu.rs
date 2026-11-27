//! A group's panel menu: its button is the group's own part, its menu a popup of the group, and its command rows reach
//! that group's command scope.

use craft_behavior::Axis;
use craft_behavior::PartKey;
use craft_behavior::dock::{Action, Column, DockEdge, Group, Layout, Node, SplitSize, Workspace};
use craft_ui::contract::{Bind, BodyCx, PlainData, Role, ShowContainer as _, Tracked};
use craft_ui::docking::{DockConfig, DockPanels, PanelSpec, PanelSpecs, WindowMenuRows};
use craft_ui::menu::{Menu, PanelRow};
use craft_ui::patterns::dock::DockArea;
use craft_ui::testing::{Fixture, Target, trial};
use egui::{Key, Modifiers, Ui, Vec2};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

/// Panels named by number.
#[derive(PlainData)]
struct Specs;

impl PanelSpecs<usize> for Specs {
    fn spec(&self, panel: &usize) -> PanelSpec {
        PanelSpec::new(format!("Panel {panel}"))
    }
}

struct Panels;

impl DockPanels<usize> for Panels {
    fn body(&mut self, _: &usize, _: &mut Ui, _: &mut BodyCx<'_>) {}

    /// A host row, so each group shows its panel menu button.
    fn panel_rows(&mut self, _: &usize) -> Vec<PanelRow> {
        vec![PanelRow::Separator]
    }
}

/// A host with no rows of its own.
struct Bare;

impl DockPanels<usize> for Bare {
    fn body(&mut self, _: &usize, _: &mut Ui, _: &mut BodyCx<'_>) {}
}

/// The panel menu buttons a dock of two groups shows with `config`, for a host that gives no rows.
fn menu_buttons(config: DockConfig) -> (Fixture, Vec<Target>) {
    let root = Node::split(Axis::X, SplitSize::Ratio(0.5), Node::tabs(vec![0, 1], 0), Node::tabs(vec![2, 3], 0));
    let mut workspace = Arc::new(Workspace::new(Layout::new(Some(root), Vec::new())));
    let specs = Tracked::new(Specs);
    let mut fixture = Fixture::new(move |ui| {
        DockArea::new("Dock", Bind::bound(&mut workspace), &specs, &mut Bare, config.clone()).show(ui);
    });
    fixture.room(Vec2::new(1200.0, 600.0)).settle();
    let buttons = fixture.by_role(Role::BUTTON).into_iter().filter(|button| fixture.tree().name_of(button.node) == Some("Panel Menu")).collect();
    (fixture, buttons)
}

/// A panel with no rows of its own shows no menu button, unless the dock shows the button on every group; that
/// button's menu holds the dock's own rows.
#[trial]
fn a_dock_that_shows_the_panel_menu_always_gives_every_group_the_button() {
    let (_, hidden) = menu_buttons(DockConfig::default());
    assert!(hidden.is_empty(), "no button for panels without rows");
    let (mut fixture, shown) = menu_buttons(DockConfig::default().panel_menu_always(true));
    assert_eq!(shown.len(), 2, "each group shows its panel menu button");
    fixture.click(&shown[0]).idle(2);
    assert!(fixture.by_label("Float").is_some(), "the menu holds the dock's own rows");
}

/// With the first group's tabs focused, Float in the second group's panel menu floats the second group's panel.
#[trial]
fn a_panel_menu_command_reaches_the_group_whose_button_opened_it() {
    let root = Node::split(Axis::X, SplitSize::Ratio(0.5), Node::tabs(vec![0, 1], 0), Node::tabs(vec![2, 3], 0));
    let workspace = Rc::new(RefCell::new(Arc::new(Workspace::new(Layout::new(Some(root), Vec::new())))));
    let (held, specs) = (Rc::clone(&workspace), Tracked::new(Specs));
    let mut fixture = Fixture::new(move |ui| {
        let mut shown = held.borrow().clone();
        DockArea::new("Dock", Bind::bound(&mut shown), &specs, &mut Panels, DockConfig::default()).show(ui);
        *held.borrow_mut() = shown;
    });
    fixture.room(Vec2::new(1200.0, 600.0)).settle();
    // Tab from nothing focuses the first group's tabs; a click on a tab leaves focus where it is.
    fixture.key(Key::Tab, Modifiers::NONE).idle(1);
    assert!(fixture.focused_stop().is_some(), "Tab focused the first group's tabs");
    let buttons: Vec<_> =
        fixture.by_role(Role::BUTTON).into_iter().filter(|button| fixture.tree().name_of(button.node) == Some("Panel Menu")).collect();
    assert_eq!(buttons.len(), 2, "each group shows its panel menu button");
    fixture.click(&buttons[1]).idle(2);
    let float = fixture.by_label("Float").expect("the menu's Float row");
    fixture.click(&float).idle(2);
    let shown = workspace.borrow();
    let layout = shown.layout();
    let floated: Vec<usize> = layout.floating.iter().flat_map(|window| window.panels().copied().collect::<Vec<_>>()).collect();
    assert_eq!(floated, vec![2], "Float reaches the group whose menu ran it");
}

/// The group's header parts at its end edge come after its tabs in Tab order, in the order they sit: the drag band,
/// then the panel menu button.
#[trial]
fn tab_reaches_the_tabs_then_the_drag_band_then_the_panel_menu_button() {
    let mut workspace = Arc::new(Workspace::new(Layout::new(Some(Node::tabs(vec![0, 1], 0)), Vec::new())));
    let specs = Tracked::new(Specs);
    let mut fixture = Fixture::new(move |ui| {
        DockArea::new("Dock", Bind::bound(&mut workspace), &specs, &mut Panels, DockConfig::default()).show(ui);
    });
    fixture.room(Vec2::new(800.0, 600.0)).settle();
    // Tab from nothing reaches the tabs first.
    fixture.key(Key::Tab, Modifiers::NONE).idle(1);
    assert!(fixture.focused_stop().is_some(), "Tab focused the tabs");
    let mut parts = Vec::new();
    for _ in 0..2 {
        fixture.key(Key::Tab, Modifiers::NONE).idle(1);
        parts.push(fixture.focused_stop().and_then(|(_, part)| part).map(|part| part.key));
    }
    assert_eq!(parts, vec![Some(PartKey::new("grip")), Some(PartKey::new("panel_menu"))], "Tab order after the tabs");
}

/// A panel in a column collapsed to icons shows in its flyout: its Window menu row opens the flyout, is checked while
/// the flyout is open, and closes it on a second pick; the row of a panel elsewhere asks for an action instead.
#[trial]
fn the_window_menu_opens_and_closes_the_flyout_of_a_panel_in_a_collapsed_column() {
    let mut iconic = Column::new(vec![Group::new(vec![2], 0)]);
    iconic.iconic = true;
    let dock = Node::dock(DockEdge::Start, vec![iconic]);
    let layout = Layout::new(Some(Node::split(Axis::X, SplitSize::FixedFirst(40.0), dock, Node::tabs(vec![1], 0))), Vec::new());
    let flyout: Rc<RefCell<Option<usize>>> = Rc::default();
    let actions: Rc<RefCell<Vec<Action<usize>>>> = Rc::default();
    let (open, taken) = (Rc::clone(&flyout), Rc::clone(&actions));
    let mut fixture = Fixture::new(move |ui| {
        let shown = *open.borrow();
        let mut menu_open = true;
        Menu::new("Window").open(Bind::bound(&mut menu_open)).show(ui, |ui, _| {
            let out = WindowMenuRows::new(&layout, &Tracked::new(Specs), &[1, 2]).flyout(shown.as_ref()).show(ui);
            if let Some(next) = out.flyout {
                *open.borrow_mut() = next;
            }
            taken.borrow_mut().extend(out.action);
        });
    });
    fixture.settle();
    for expected in [Some(2), None] {
        let row = fixture.by_label("Panel 2").expect("the collapsed panel's row shows");
        fixture.click(&row).settle();
        assert_eq!(*flyout.borrow(), expected, "the row opens, then closes, the panel's flyout");
    }
    assert!(actions.borrow().is_empty(), "the collapsed panel's row changes no layout: {:?}", actions.borrow());
    let row = fixture.by_label("Panel 1").expect("the docked panel's row shows");
    fixture.click(&row).settle();
    assert_eq!(*actions.borrow(), vec![Action::close(1)], "the row of a showing panel closes it");
    assert_eq!(*flyout.borrow(), None, "and leaves the flyout alone");
}

/// Floats panel 2 through the panel menu of the second group.
fn float_second_group(fixture: &mut Fixture) {
    let buttons: Vec<_> =
        fixture.by_role(Role::BUTTON).into_iter().filter(|button| fixture.tree().name_of(button.node) == Some("Panel Menu")).collect();
    fixture.click(&buttons[1]).idle(2);
    let float = fixture.by_label("Float").expect("the menu's Float row");
    fixture.click(&float).idle(2);
}

/// An undo that brings back the workspace from before a float keeps the area's next window off the id the undone
/// window had: the area remembers the ids it has shown.
#[trial]
fn a_window_floated_after_an_undo_takes_a_new_id() {
    let root = Node::split(Axis::X, SplitSize::Ratio(0.5), Node::tabs(vec![0, 1], 0), Node::tabs(vec![2, 3], 0));
    let workspace = Rc::new(RefCell::new(Arc::new(Workspace::new(Layout::new(Some(root), Vec::new())))));
    let (held, specs) = (Rc::clone(&workspace), Tracked::new(Specs));
    let mut fixture = Fixture::new(move |ui| {
        DockArea::new("Dock", Bind::bound(&mut *held.borrow_mut()), &specs, &mut Panels, DockConfig::default()).show(ui);
    });
    fixture.room(Vec2::new(1200.0, 600.0)).settle();
    let older = Arc::clone(&workspace.borrow());
    float_second_group(&mut fixture);
    let undone: Vec<u64> = workspace.borrow().layout().floating.iter().map(|window| window.id).collect();
    assert_eq!(undone.len(), 1, "Float made a window");
    *workspace.borrow_mut() = older;
    fixture.settle();
    float_second_group(&mut fixture);
    let ids: Vec<u64> = workspace.borrow().layout().floating.iter().map(|window| window.id).collect();
    assert_eq!(ids.len(), 1, "Float made a window again");
    assert_ne!(ids, undone, "the window after the undo does not take the undone window's id");
}
