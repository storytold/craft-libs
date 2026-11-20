//! The tab menu of `DockConfig::tab_context_menu`: a right click on a docked group's tab opens the panel's short
//! menu, and a right click on a docked iconic column's icon opens the same menu for that panel; everything the
//! option does not cover keeps the group's panel menu.

use craft_behavior::Axis;
use craft_behavior::Mixed;
use craft_behavior::dock::{Column, DockEdge, Group, Layout, Node, Permissions, SplitSize, Workspace};
use craft_ui::contract::{Bind, BodyCx};
use craft_ui::docking::{DockConfig, DockEvent, DockPanels, PanelSpec};
use craft_ui::menu::PanelRow;
use craft_ui::patterns::dock::DockArea;
use craft_ui::runtime::env::Label;
use craft_ui::testing::{Fixture, pointer, trial};
use egui::{Pos2, Ui, Vec2};
use std::cell::RefCell;
use std::rc::Rc;

/// The host's tab-menu rows carry this label, so the tests can see where the dock's rows end.
const HOST_ROW: &str = "Host Tab Rows";

#[derive(Clone, Copy, Default)]
struct Panels {
    /// A panel that may not close: its menu rows show greyed.
    uncloseable: Option<usize>,
}

impl DockPanels<usize> for Panels {
    fn spec(&self, panel: &usize) -> PanelSpec {
        PanelSpec::new(format!("Panel {panel}"))
    }

    fn body(&mut self, _: &usize, _: &mut Ui, _: &mut BodyCx<'_>) {}

    fn permissions(&self, panel: &usize) -> Permissions {
        if self.uncloseable == Some(*panel) { Permissions::ALL.closable(false) } else { Permissions::ALL }
    }

    fn panel_rows(&mut self, _: &usize) -> Vec<PanelRow> {
        vec![PanelRow::Separator]
    }

    fn tab_rows(&mut self, _: &usize) -> Vec<PanelRow> {
        vec![PanelRow::Submenu(Label::from(HOST_ROW), Vec::new())]
    }
}

/// The right-click events at `at`, then the passes the popup needs to open.
fn right_click(fixture: &mut Fixture, at: Pos2) {
    fixture.frame(vec![pointer::moved(at)]);
    fixture.frame(vec![pointer::secondary(at, true)]);
    fixture.frame(vec![pointer::secondary(at, false)]);
    fixture.idle(3);
}

/// The workspace the bed edits, the flyout the area keeps open, and the events it reported.
type Held = Rc<RefCell<Workspace<usize>>>;
type Open = Rc<RefCell<Option<usize>>>;
type Events = Rc<RefCell<Vec<DockEvent<usize>>>>;

/// A dock with `config`: a Start dock column holding the group [0, 1] beside a framed group [2]. Returns the
/// fixture, the workspace it edits, the flyout the area keeps open and the events it reported.
fn bed(config: DockConfig, panels: Panels, iconic: bool, auto: bool) -> (Fixture, Held, Open, Events) {
    let mut column = Column::new(vec![Group::new(vec![0, 1], 0)]);
    column.iconic = iconic;
    let edge = if iconic { SplitSize::FixedFirst(60.0) } else { SplitSize::FixedFirst(260.0) };
    let root = Node::split(Axis::X, edge, Node::dock(DockEdge::Start, vec![column]), Node::tabs(vec![2], 0));
    let workspace = Rc::new(RefCell::new(Workspace::new(Layout::new(Some(root), Vec::new()))));
    let flyout = Rc::new(RefCell::new(None));
    let events = Rc::new(RefCell::new(Vec::new()));
    let (held, open, taken) = (Rc::clone(&workspace), Rc::clone(&flyout), Rc::clone(&events));
    let mut fixture = Fixture::new(move |ui| {
        let mut shown = held.borrow().clone();
        let mut shown_flyout = *open.borrow();
        let mut panels = panels;
        let mut area = DockArea::new("Dock", Bind::bound(&mut shown), &mut panels, config.clone()).flyout(Bind::bound(&mut shown_flyout));
        if auto {
            area = area.auto_collapse();
        }
        let out = area.show(ui);
        *held.borrow_mut() = shown;
        *open.borrow_mut() = shown_flyout;
        taken.borrow_mut().extend(out.events);
    });
    fixture.room(Vec2::new(1200.0, 600.0)).settle();
    (fixture, workspace, flyout, events)
}

/// The dock option on, with the group-order rows the measured host turns on.
fn option_on() -> DockConfig {
    DockConfig::default().tab_context_menu(true).group_order_items(true)
}

/// The row labels of a menu, top to bottom, found by their order on the screen.
fn menu_order(fixture: &Fixture, names: &[&str]) -> Vec<f32> {
    names.iter().map(|name| fixture.by_label(name).unwrap_or_else(|| panic!("the menu shows {name}")).center().y).collect()
}

fn assert_top_to_bottom(ys: &[f32], names: &[&str]) {
    let mut sorted = ys.to_vec();
    sorted.sort_by(f32::total_cmp);
    assert_eq!(ys, sorted.as_slice(), "the menu rows sit top to bottom in the measured order: {names:?}");
}

/// With the option on, a right click on a docked tab shows Close first, then Close Tab Group, Minimize, the
/// column's Collapse to Icons and Auto-Collapse Iconic Panels, and the host's rows last: the Float, Dock Panel,
/// Move Panel To and Float Tab Group rows leave this menu.
#[trial]
fn a_docked_tab_right_click_shows_the_short_tab_menu() {
    let (mut fixture, _workspace, _flyout, _events) = bed(option_on(), Panels::default(), false, false);
    let tab = fixture.by_label("Panel 0").expect("the docked group's tab");
    right_click(&mut fixture, tab.center());
    let names = ["Close", "Close Tab Group", "Minimize", "Collapse to Icons", "Auto-Collapse Iconic Panels", HOST_ROW];
    assert_top_to_bottom(&menu_order(&fixture, &names), &names);
    for gone in ["Float", "Float Tab Group", "Move Panel To"] {
        assert!(fixture.by_label(gone).is_none(), "{gone} leaves the docked tab menu");
    }
}

/// A right-clicked tab's Close row closes the panel, and its group's row stays only in the panel menu.
#[trial]
fn a_tab_menu_close_row_closes_the_panel() {
    let (mut fixture, workspace, _flyout, _events) = bed(option_on(), Panels::default(), false, false);
    let tab = fixture.by_label("Panel 0").expect("the docked group's tab");
    right_click(&mut fixture, tab.center());
    let close = fixture.by_label("Close").expect("the tab menu's Close row");
    fixture.click(&close).idle(3);
    let mut panels: Vec<usize> = workspace.borrow().layout.panels().into_iter().copied().collect();
    panels.sort_unstable();
    assert_eq!(panels, vec![1, 2], "Close closes the panel of the group the tab sits in");
    assert!(fixture.by_label(HOST_ROW).is_none(), "the menu closed with the row");
}

/// The panel menu button keeps the full group menu with the option on: Float is there, and the host's tab rows
/// are not.
#[trial]
fn the_panel_menu_button_keeps_the_group_menu() {
    let (mut fixture, _workspace, _flyout, _events) = bed(option_on(), Panels::default(), false, false);
    let buttons: Vec<_> =
        fixture.by_role(craft_ui::contract::Role::BUTTON).into_iter().filter(|b| fixture.tree().name_of(b.node) == Some("Panel Menu")).collect();
    assert!(!buttons.is_empty(), "the group shows its panel menu button");
    fixture.click(&buttons[0]).idle(2);
    let float = fixture.by_label("Float").expect("the group menu keeps Float");
    assert_eq!(float.center().y, menu_order(&fixture, &["Float"])[0]);
    assert!(fixture.by_label("Close Tab Group").is_some(), "the group menu keeps its own rows");
    assert!(fixture.by_label(HOST_ROW).is_none(), "the host's tab rows stay in the tab menu");
}

/// The option covers docked groups only: the tab of a framed group keeps today's group menu, and without
/// `group_order_items` the tab menu leaves out Minimize.
#[trial]
fn framed_tabs_and_hosts_without_group_order_items_keep_today_s_rows() {
    let (mut fixture, _workspace, _flyout, _events) = bed(option_on(), Panels::default(), false, false);
    let framed = fixture.by_label("Panel 2").expect("the framed group's tab");
    right_click(&mut fixture, framed.center());
    assert!(fixture.by_label("Float").is_some(), "a framed tab keeps today's group menu");
    assert!(fixture.by_label(HOST_ROW).is_none(), "and no tab menu of its own");
    drop(fixture);
    let (mut fixture, _workspace, _flyout, _events) = bed(DockConfig::default().tab_context_menu(true), Panels::default(), false, false);
    let tab = fixture.by_label("Panel 0").expect("the docked group's tab");
    right_click(&mut fixture, tab.center());
    let names = ["Close", "Close Tab Group", "Collapse to Icons", "Auto-Collapse Iconic Panels", HOST_ROW];
    assert_top_to_bottom(&menu_order(&fixture, &names), &names);
    assert!(fixture.by_label("Minimize").is_none(), "Minimize stays with the group-order rows the host turned off");
}

/// With the option off, today's behaviour stands: a right click on a docked tab opens the group menu with Float.
#[trial]
fn without_the_option_a_tab_right_click_still_opens_the_group_menu() {
    let (mut fixture, _workspace, _flyout, _events) = bed(DockConfig::default().group_order_items(true), Panels::default(), false, false);
    let tab = fixture.by_label("Panel 0").expect("the docked group's tab");
    right_click(&mut fixture, tab.center());
    assert!(fixture.by_label("Float").is_some(), "today's tab right click opens the group menu");
    assert!(fixture.by_label("Move Panel To").is_some(), "with its Move Panel To row");
    assert!(fixture.by_label(HOST_ROW).is_none(), "and not the host's tab rows");
}

/// A panel that may not close shows its tab menu's Close rows greyed.
#[trial]
fn the_close_rows_grey_for_a_panel_that_may_not_close() {
    let (mut fixture, _workspace, _flyout, _events) = bed(option_on(), Panels { uncloseable: Some(0) }, false, false);
    let tab = fixture.by_label("Panel 0").expect("the docked group's tab");
    right_click(&mut fixture, tab.center());
    let close = fixture.by_label("Close").expect("the tab menu shows Close");
    assert!(fixture.tree().is_disabled_key(close.node), "Close greys for a panel that may not close");
}

/// With the option on, a right click on a collapsed column's icon shows its menu: Close, Close Tab Group, then
/// Expand Panels and Auto-Collapse Iconic Panels, and the host's rows last. Minimize and the float rows are not
/// there, and the icon's flyout does not open.
#[trial]
fn a_collapsed_icon_right_click_shows_the_icon_menu() {
    let (mut fixture, _workspace, flyout, _events) = bed(option_on(), Panels::default(), true, false);
    let icon = fixture.by_label("Panel 0").expect("the collapsed column's icon");
    right_click(&mut fixture, icon.center());
    let names = ["Close", "Close Tab Group", "Expand Panels", "Auto-Collapse Iconic Panels", HOST_ROW];
    assert_top_to_bottom(&menu_order(&fixture, &names), &names);
    for gone in ["Float", "Minimize", "Collapse to Icons"] {
        assert!(fixture.by_label(gone).is_none(), "{gone} is not in the icon menu");
    }
    assert_eq!(*flyout.borrow(), None, "the right click opens the menu, not the flyout");
}

/// A left click on an icon still opens its flyout with the option on.
#[trial]
fn a_left_click_on_an_icon_still_opens_its_flyout() {
    let (mut fixture, _workspace, flyout, _events) = bed(option_on(), Panels::default(), true, false);
    let icon = fixture.by_label("Panel 0").expect("the collapsed column's icon");
    fixture.click(&icon).idle(3);
    assert_eq!(*flyout.borrow(), Some(0), "the flyout still opens on a click");
}

/// With no rows for the icon (the option off), today's behaviour stands: a right click on an icon opens no menu
/// and leaves the flyout as it was.
#[trial]
fn without_the_option_an_icon_right_click_opens_nothing() {
    let (mut fixture, _workspace, flyout, _events) = bed(DockConfig::default().group_order_items(true), Panels::default(), true, false);
    let icon = fixture.by_label("Panel 0").expect("the collapsed column's icon");
    right_click(&mut fixture, icon.center());
    assert!(fixture.by_label("Expand Panels").is_none(), "the icon has no menu of its own");
    assert!(fixture.by_label("Close").is_none(), "no Close row shows either");
    assert_eq!(*flyout.borrow(), None, "and the flyout stays as it was");
}

/// The icon menu's rows act on the panel of the icon they were opened on, not the group's shown panel: Close on
/// the second icon's menu closes that panel, and Expand Panels expands the column.
#[trial]
fn the_icon_menu_acts_on_the_icon_it_was_opened_on() {
    let (mut fixture, workspace, _flyout, _events) = bed(option_on(), Panels::default(), true, false);
    let icon = fixture.by_label("Panel 1").expect("the collapsed column's second icon");
    right_click(&mut fixture, icon.center());
    let close = fixture.by_label("Close").expect("the icon menu's Close row");
    fixture.click(&close).idle(3);
    let mut panels: Vec<usize> = workspace.borrow().layout.panels().into_iter().copied().collect();
    panels.sort_unstable();
    assert_eq!(panels, vec![0, 2], "Close closes the panel of the icon it was opened on");
    let icon = fixture.by_label("Panel 0").expect("the collapsed column's icon");
    right_click(&mut fixture, icon.center());
    let expand = fixture.by_label("Expand Panels").expect("the icon menu's Expand Panels row");
    fixture.click(&expand).idle(3);
    let held = workspace.borrow();
    let place = held.layout.place(&0).expect("panel 0 is placed");
    let column = held.layout.column(&place).expect("panel 0 is in a column");
    assert!(!column.iconic, "Expand Panels expands the column of the icon it was opened on");
}

/// The Auto-Collapse Iconic Panels row of both menus is checked from the dock's setting, and running it reports
/// the host's toggle event.
#[trial]
fn the_auto_collapse_row_follows_the_setting_and_reports_the_event() {
    let (mut fixture, _workspace, _flyout, events) = bed(option_on(), Panels::default(), false, true);
    let tab = fixture.by_label("Panel 0").expect("the docked group's tab");
    right_click(&mut fixture, tab.center());
    let auto = fixture.by_label("Auto-Collapse Iconic Panels").expect("the Auto-Collapse row");
    assert_eq!(fixture.tree().toggled_of(auto.node), Some(Mixed::Same(true)), "the row is checked from the setting");
    fixture.click(&auto).idle(3);
    assert_eq!(*events.borrow(), vec![DockEvent::ToggleAutoCollapse], "the row reports the host's event");
}
