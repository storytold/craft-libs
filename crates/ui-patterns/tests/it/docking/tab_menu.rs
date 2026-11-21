//! The tab menu of `DockConfig::tab_context_menu`: a right click on a docked group's tab opens the panel's short
//! menu, and a right click on a docked iconic column's icon opens the same menu for that panel; everything the
//! option does not cover keeps the group's panel menu.

use craft_behavior::Axis;
use craft_behavior::Mixed;
use craft_behavior::dock::{Column, DockEdge, Floating, Group, Layout, Node, Permissions, SplitSize, StackEntry, Workspace};
use craft_ui::contract::{Bind, BodyCx, PlainData, Role, Tracked};
use craft_ui::docking::{DockConfig, DockEvent, DockPanels, PanelSpec, PanelSpecs};
use craft_ui::menu::PanelRow;
use craft_ui::patterns::dock::DockArea;
use craft_ui::runtime::env::Label;
use craft_ui::testing::{Fixture, pointer, trial};
use egui::{Pos2, Rect, Ui, Vec2};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

/// The host's tab-menu rows carry this label, so the tests can see where the dock's rows end.
const HOST_ROW: &str = "Host Tab Rows";

/// The panel specs: each panel named by its number, with one panel that may not close.
#[derive(PlainData, Default)]
struct Specs {
    /// A panel that may not close: its menu rows show greyed.
    uncloseable: Option<usize>,
}

impl PanelSpecs<usize> for Specs {
    fn spec(&self, panel: &usize) -> PanelSpec {
        PanelSpec::new(format!("Panel {panel}"))
    }

    fn permissions(&self, panel: &usize) -> Permissions {
        if self.uncloseable == Some(*panel) { Permissions::ALL.closable(false) } else { Permissions::ALL }
    }
}

/// The host's views: empty bodies, and the menu rows that give the group its panel menu button.
struct Panels;

impl DockPanels<usize> for Panels {
    fn body(&mut self, _: &usize, _: &mut Ui, _: &mut BodyCx<'_>) {}

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
type Held = Rc<RefCell<Arc<Workspace<usize>>>>;
type Open = Rc<RefCell<Option<usize>>>;
type Events = Rc<RefCell<Vec<DockEvent<usize>>>>;

/// The width of the collapsed column the trials use: no room for the panel names.
const ICONIC_COLUMN: f32 = 60.0;
/// The width of the expanded column the trials use.
const EXPANDED_COLUMN: f32 = 260.0;
/// A collapsed column wider than the label width (`size.rail.width` times 2.4, so 96 points at the Compact size):
/// each cell then shows its panel's name beside its icon, and the name takes most of the row.
const WIDE_ICONIC_COLUMN: f32 = 200.0;

/// As [`bed`], with the Start column given `column` points of the window's room. A collapsed column takes its saved
/// [`iconic_width`](craft_behavior::dock::Column::iconic_width), not that room: pass more than the usual 60 to get a
/// wide one, whose cells show their panel names.
fn bed_at(config: DockConfig, specs: Specs, iconic: bool, column: f32, auto: bool) -> (Fixture, Held, Open, Events) {
    let mut member = Column::new(vec![Group::new(vec![0, 1], 0)]);
    member.iconic = iconic;
    if iconic && column > ICONIC_COLUMN {
        member.width = Some(EXPANDED_COLUMN);
        member.iconic_width = Some(column);
    }
    let root = Node::split(Axis::X, SplitSize::FixedFirst(column), Node::dock(DockEdge::Start, vec![member]), Node::tabs(vec![2], 0));
    bed_over(config, specs, Layout::new(Some(root), Vec::new()), auto)
}

/// A dock with `config`: its specs `specs` over `layout`, the panel host `Panels` and Auto-Collapse on or off.
/// Returns the fixture, the workspace it edits, the flyout the area keeps open and the events it reported.
fn bed_over(config: DockConfig, specs: Specs, layout: Layout<usize>, auto: bool) -> (Fixture, Held, Open, Events) {
    let workspace = Rc::new(RefCell::new(Arc::new(Workspace::new(layout))));
    let flyout = Rc::new(RefCell::new(None));
    let events = Rc::new(RefCell::new(Vec::new()));
    let (held, open, taken) = (Rc::clone(&workspace), Rc::clone(&flyout), Rc::clone(&events));
    let specs = Tracked::new(specs);
    let mut fixture = Fixture::new(move |ui| {
        let mut shown = held.borrow().clone();
        let mut shown_flyout = *open.borrow();
        let mut panels = Panels;
        let mut area = DockArea::new("Dock", Bind::bound(&mut shown), &specs, &mut panels, config.clone()).flyout(Bind::bound(&mut shown_flyout));
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

/// A dock with `config`: a Start dock column holding the group [0, 1] beside a framed group [2], `bed_at`'s usual
/// widths. Returns the fixture, the workspace it edits, the flyout the area keeps open and the events it reported.
fn bed(config: DockConfig, specs: Specs, iconic: bool, auto: bool) -> (Fixture, Held, Open, Events) {
    bed_at(config, specs, iconic, if iconic { ICONIC_COLUMN } else { EXPANDED_COLUMN }, auto)
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
    let (mut fixture, _workspace, _flyout, _events) = bed(option_on(), Specs::default(), false, false);
    let tab = fixture.by_label("Panel 0").expect("the docked group's tab");
    right_click(&mut fixture, tab.center());
    let names = ["Close", "Close Tab Group", "Minimize", "Collapse to Icons", "Auto-Collapse Iconic Panels", HOST_ROW];
    assert_top_to_bottom(&menu_order(&fixture, &names), &names);
    let auto = fixture.by_label("Auto-Collapse Iconic Panels").expect("the Auto-Collapse row");
    // A menu row carries its mark only while it is checked: off, the row has no checked state at all.
    assert_eq!(fixture.tree().toggled_of(auto.node), None, "the row is unchecked while the dock's setting is off");
    for gone in ["Float", "Float Tab Group", "Move Panel To"] {
        assert!(fixture.by_label(gone).is_none(), "{gone} leaves the docked tab menu");
    }
}

/// A right-clicked tab's Close row closes the panel, and its group's row stays only in the panel menu.
#[trial]
fn a_tab_menu_close_row_closes_the_panel() {
    let (mut fixture, workspace, _flyout, _events) = bed(option_on(), Specs::default(), false, false);
    let tab = fixture.by_label("Panel 0").expect("the docked group's tab");
    right_click(&mut fixture, tab.center());
    let close = fixture.by_label("Close").expect("the tab menu's Close row");
    fixture.click(&close).idle(3);
    let mut panels: Vec<usize> = workspace.borrow().layout().panels().into_iter().copied().collect();
    panels.sort_unstable();
    assert_eq!(panels, vec![1, 2], "Close closes the panel of the group the tab sits in");
    assert!(fixture.by_label(HOST_ROW).is_none(), "the menu closed with the row");
}

/// The panel menu button keeps the full group menu with the option on: Float is there, and the host's tab rows
/// are not.
#[trial]
fn the_panel_menu_button_keeps_the_group_menu() {
    let (mut fixture, _workspace, _flyout, _events) = bed(option_on(), Specs::default(), false, false);
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
    let (mut fixture, _workspace, _flyout, _events) = bed(option_on(), Specs::default(), false, false);
    let framed = fixture.by_label("Panel 2").expect("the framed group's tab");
    right_click(&mut fixture, framed.center());
    assert!(fixture.by_label("Float").is_some(), "a framed tab keeps today's group menu");
    assert!(fixture.by_label(HOST_ROW).is_none(), "and no tab menu of its own");
    drop(fixture);
    let (mut fixture, _workspace, _flyout, _events) = bed(DockConfig::default().tab_context_menu(true), Specs::default(), false, false);
    let tab = fixture.by_label("Panel 0").expect("the docked group's tab");
    right_click(&mut fixture, tab.center());
    let names = ["Close", "Close Tab Group", "Collapse to Icons", "Auto-Collapse Iconic Panels", HOST_ROW];
    assert_top_to_bottom(&menu_order(&fixture, &names), &names);
    assert!(fixture.by_label("Minimize").is_none(), "Minimize stays with the group-order rows the host turned off");
}

/// With the option off, today's behaviour stands: a right click on a docked tab opens the group menu with Float.
#[trial]
fn without_the_option_a_tab_right_click_still_opens_the_group_menu() {
    let (mut fixture, _workspace, _flyout, _events) = bed(DockConfig::default().group_order_items(true), Specs::default(), false, false);
    let tab = fixture.by_label("Panel 0").expect("the docked group's tab");
    right_click(&mut fixture, tab.center());
    assert!(fixture.by_label("Float").is_some(), "today's tab right click opens the group menu");
    assert!(fixture.by_label("Move Panel To").is_some(), "with its Move Panel To row");
    assert!(fixture.by_label(HOST_ROW).is_none(), "and not the host's tab rows");
}

/// A panel that may not close shows its tab menu's Close rows greyed.
#[trial]
fn the_close_rows_grey_for_a_panel_that_may_not_close() {
    let (mut fixture, _workspace, _flyout, _events) = bed(option_on(), Specs { uncloseable: Some(0) }, false, false);
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
    let (mut fixture, _workspace, flyout, _events) = bed(option_on(), Specs::default(), true, false);
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
    let (mut fixture, _workspace, flyout, _events) = bed(option_on(), Specs::default(), true, false);
    let icon = fixture.by_label("Panel 0").expect("the collapsed column's icon");
    fixture.click(&icon).idle(3);
    assert_eq!(*flyout.borrow(), Some(0), "the flyout still opens on a click");
}

/// With no rows for the icon (the option off), today's behaviour stands: a right click on an icon opens no menu
/// and leaves the flyout as it was.
#[trial]
fn without_the_option_an_icon_right_click_opens_nothing() {
    let (mut fixture, _workspace, flyout, _events) = bed(DockConfig::default().group_order_items(true), Specs::default(), true, false);
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
    let (mut fixture, workspace, _flyout, _events) = bed(option_on(), Specs::default(), true, false);
    let icon = fixture.by_label("Panel 1").expect("the collapsed column's second icon");
    right_click(&mut fixture, icon.center());
    let close = fixture.by_label("Close").expect("the icon menu's Close row");
    fixture.click(&close).idle(3);
    let mut panels: Vec<usize> = workspace.borrow().layout().panels().into_iter().copied().collect();
    panels.sort_unstable();
    assert_eq!(panels, vec![0, 2], "Close closes the panel of the icon it was opened on");
    let icon = fixture.by_label("Panel 0").expect("the collapsed column's icon");
    right_click(&mut fixture, icon.center());
    let expand = fixture.by_label("Expand Panels").expect("the icon menu's Expand Panels row");
    fixture.click(&expand).idle(3);
    let held = workspace.borrow();
    let place = held.layout().place(&0).expect("panel 0 is placed");
    let column = held.layout().column(&place).expect("panel 0 is in a column");
    assert!(!column.iconic, "Expand Panels expands the column of the icon it was opened on");
}

/// A wide collapsed column gives each cell a name beside its icon: a right click on that name half opens the same
/// icon menu a right click on the icon square does.
#[trial]
fn a_right_click_on_a_wide_column_s_name_half_shows_the_icon_menu() {
    let (mut fixture, _workspace, flyout, _events) = bed_at(option_on(), Specs::default(), true, WIDE_ICONIC_COLUMN, false);
    // The setup: the column is wide enough that the panel's name is painted beside its icon.
    let painted = fixture.texts().iter().any(|run| run.text == "Panel 0");
    assert!(painted, "a column {WIDE_ICONIC_COLUMN} wide shows the panel's name beside its icon");
    let icon = fixture.by_label("Panel 0").expect("the wide column's icon");
    assert!(icon.rect.width() < WIDE_ICONIC_COLUMN, "the icon keeps its square: {:?}", icon.rect);
    right_click(&mut fixture, icon.rect.right_center() + Vec2::new(icon.rect.width(), 0.0));
    let names = ["Close", "Close Tab Group", "Expand Panels", "Auto-Collapse Iconic Panels", HOST_ROW];
    assert_top_to_bottom(&menu_order(&fixture, &names), &names);
    assert_eq!(*flyout.borrow(), None, "the right click opens the menu, not the flyout");
}

/// The gate that gives a tab menu only to the tabs of a dock column leaves three places alone: the tab of a group
/// shown in an iconic column's flyout, the tab of an accordion section, and the tab of a floating window's group all
/// still open today's group menu — Float or Dock Panel there, and no host `tab_rows` row.
#[trial]
fn a_flyout_tab_an_accordion_section_and_a_window_tab_keep_the_group_menu() {
    // The tab inside an iconic column's flyout: the panel is docked, so only the flyout itself keeps the group menu.
    let (mut fixture, _workspace, flyout, _events) = bed(option_on(), Specs::default(), true, false);
    let icon = fixture.by_label("Panel 0").expect("the collapsed column's icon");
    fixture.click(&icon).idle(3);
    assert_eq!(*flyout.borrow(), Some(0), "the click opened the flyout");
    let tab = fixture
        .by_role(Role::TAB)
        .into_iter()
        .find(|tab| fixture.tree().name_of(tab.node) == Some("Panel 1"))
        .expect("the flyout shows the group's second tab");
    right_click(&mut fixture, tab.center());
    assert!(fixture.by_label("Float").is_some(), "the flyout's tab keeps today's group menu");
    assert!(fixture.by_label(HOST_ROW).is_none(), "and no tab menu of its own");
    drop(fixture);
    // An accordion section: a `Stack` node of the docked tree, not a group of a dock column.
    let accordion = Node::split(
        Axis::X,
        SplitSize::FixedFirst(EXPANDED_COLUMN),
        Node::stack(vec![StackEntry::new(0), StackEntry::new(1)]),
        Node::tabs(vec![2], 0),
    );
    let (mut fixture, _workspace, _flyout, _events) = bed_over(option_on(), Specs::default(), Layout::new(Some(accordion), Vec::new()), false);
    let section = fixture
        .by_role(Role::TAB)
        .into_iter()
        .find(|tab| fixture.tree().name_of(tab.node) == Some("Panel 1"))
        .expect("the second section shows its tab");
    right_click(&mut fixture, section.center());
    assert!(fixture.by_label("Float").is_some(), "an accordion section keeps today's group menu");
    assert!(fixture.by_label(HOST_ROW).is_none(), "and no tab menu of its own");
    drop(fixture);
    // A floating window's group: `PanelPlace::Floating`, so its tabs keep the group menu too.
    let window = Floating::single(1, vec![3], 0, Rect::from_min_size(Pos2::new(300.0, 120.0), Vec2::new(320.0, 220.0)));
    let (mut fixture, _workspace, _flyout, _events) =
        bed_over(option_on(), Specs::default(), Layout::new(Some(Node::tabs(vec![2], 0)), vec![window]), false);
    let tab =
        fixture.by_role(Role::TAB).into_iter().find(|tab| fixture.tree().name_of(tab.node) == Some("Panel 3")).expect("the window shows its tab");
    right_click(&mut fixture, tab.center());
    assert!(fixture.by_label("Dock Panel").is_some(), "a window's tab keeps today's group menu, with its Dock Panel row");
    assert!(fixture.by_label(HOST_ROW).is_none(), "and no tab menu of its own");
}

/// The Auto-Collapse Iconic Panels row of both menus is checked from the dock's setting, and running it reports
/// the host's toggle event.
#[trial]
fn the_auto_collapse_row_follows_the_setting_and_reports_the_event() {
    let (mut fixture, _workspace, _flyout, events) = bed(option_on(), Specs::default(), false, true);
    let tab = fixture.by_label("Panel 0").expect("the docked group's tab");
    right_click(&mut fixture, tab.center());
    let auto = fixture.by_label("Auto-Collapse Iconic Panels").expect("the Auto-Collapse row");
    assert_eq!(fixture.tree().toggled_of(auto.node), Some(Mixed::Same(true)), "the row is checked from the setting");
    fixture.click(&auto).idle(3);
    assert_eq!(*events.borrow(), vec![DockEvent::ToggleAutoCollapse], "the row reports the host's event");
}
