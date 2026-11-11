//! A group's panel menu: its button is the group's own part, its menu a popup of the group, and its command rows reach
//! that group's command scope.

use craft_behavior::Axis;
use craft_behavior::PartKey;
use craft_behavior::dock::{Layout, Node, SplitSize, Workspace};
use craft_ui::contract::{Bind, BodyCx, Role};
use craft_ui::docking::{DockConfig, DockPanels, PanelSpec};
use craft_ui::menu::PanelRow;
use craft_ui::patterns::dock::DockArea;
use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers, Ui, Vec2};
use std::cell::RefCell;
use std::rc::Rc;

struct Panels;

impl DockPanels<usize> for Panels {
    fn spec(&self, panel: &usize) -> PanelSpec {
        PanelSpec::new(format!("Panel {panel}"))
    }

    fn body(&mut self, _: &usize, _: &mut Ui, _: &mut BodyCx<'_>) {}

    /// A host row, so each group shows its panel menu button.
    fn panel_rows(&mut self, _: &usize) -> Vec<PanelRow> {
        vec![PanelRow::Separator]
    }
}

/// With the first group's tabs focused, Float in the second group's panel menu floats the second group's panel.
#[trial]
fn a_panel_menu_command_reaches_the_group_whose_button_opened_it() {
    let root = Node::split(Axis::X, SplitSize::Ratio(0.5), Node::tabs(vec![0, 1], 0), Node::tabs(vec![2, 3], 0));
    let workspace = Rc::new(RefCell::new(Workspace::new(Layout::new(Some(root), Vec::new()))));
    let held = Rc::clone(&workspace);
    let mut fixture = Fixture::new(move |ui| {
        let mut shown = held.borrow().clone();
        DockArea::new("Dock", Bind::bound(&mut shown), &mut Panels, DockConfig::default()).show(ui);
        *held.borrow_mut() = shown;
    });
    fixture.room(Vec2::new(1200.0, 600.0)).settle();
    let first = fixture.by_label("Panel 0").expect("the first group's tab");
    fixture.click(&first).idle(1);
    let buttons: Vec<_> =
        fixture.by_role(Role::BUTTON).into_iter().filter(|button| fixture.tree().name_of(button.node) == Some("Panel Menu")).collect();
    assert_eq!(buttons.len(), 2, "each group shows its panel menu button");
    fixture.click(&buttons[1]).idle(2);
    let float = fixture.by_label("Float").expect("the menu's Float row");
    fixture.click(&float).idle(2);
    let layout = &workspace.borrow().layout;
    let floated: Vec<usize> = layout.floating.iter().flat_map(|window| window.panels().copied().collect::<Vec<_>>()).collect();
    assert_eq!(floated, vec![2], "Float reaches the group whose menu ran it");
}

/// The group's header parts at its end edge come after its tabs in Tab order, in the order they sit: the drag band,
/// then the panel menu button.
#[trial]
fn tab_reaches_the_tabs_then_the_drag_band_then_the_panel_menu_button() {
    let mut workspace = Workspace::new(Layout::new(Some(Node::tabs(vec![0, 1], 0)), Vec::new()));
    let mut fixture = Fixture::new(move |ui| {
        DockArea::new("Dock", Bind::bound(&mut workspace), &mut Panels, DockConfig::default()).show(ui);
    });
    fixture.room(Vec2::new(800.0, 600.0)).settle();
    let tab = fixture.by_label("Panel 0").expect("the group's tab");
    fixture.click(&tab).idle(1);
    let mut parts = Vec::new();
    for _ in 0..2 {
        fixture.key(Key::Tab, Modifiers::NONE).idle(1);
        parts.push(fixture.focused_stop().and_then(|(_, part)| part).map(|part| part.key));
    }
    assert_eq!(parts, vec![Some(PartKey::new("grip")), Some(PartKey::new("panel_menu"))], "Tab order after the tabs");
}
