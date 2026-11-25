//! The gesture sources a dock area reports (`DockAreaOutput::sources`): one per tab, group bar, window grip and divider
//! it drew, named by what it acts on, and none for what it did not draw.

use super::Numbered;
use craft_behavior::Axis;
use craft_behavior::dock::{Column, DockEdge, Floating, Group, Layout, Node, SplitSize, StackEntry, Workspace};
use craft_ui::contract::{Bind, BodyCx, Role, Tracked};
use craft_ui::docking::{DividerKind, DockConfig, DockPanels, DockSource, HidePanels, PanelKey, SourceKind};
use craft_ui::patterns::dock::DockArea;
use craft_ui::runtime::env::{BodyMode, ShellCx, ShellEntry, WindowKey};
use craft_ui::testing::access::NodeKey;
use craft_ui::testing::{Fixture, trial};
use egui::{Pos2, Rect, Ui, Vec2};
use std::cell::RefCell;
use std::collections::{BTreeSet, HashSet};
use std::rc::Rc;
use std::sync::Arc;

/// The start dock's column: a group of two tabs over a group of one.
const UPPER: usize = 1;
const UPPER_TWO: usize = 2;
const LOWER: usize = 3;
/// The middle split: a group of two tabs beside a group of one.
const LEFT: usize = 4;
const LEFT_TWO: usize = 5;
const RIGHT: usize = 6;
/// The end dock's column, collapsed to icons.
const ICONIC: usize = 7;
/// The floating window's panel.
const FLOATING: usize = 8;

struct Bodies;

impl DockPanels<usize> for Bodies {
    fn body(&mut self, _: &usize, _: &mut Ui, _: &mut BodyCx<'_>) {}
}

/// A start dock whose column holds two groups, a split of two tab groups, an end dock whose column is collapsed to
/// icons, and one floating window.
fn layout() -> Layout<usize> {
    let start = Node::dock(DockEdge::Start, vec![Column::new(vec![Group::new(vec![UPPER, UPPER_TWO], 0), Group::new(vec![LOWER], 0)])]);
    let mut collapsed = Column::new(vec![Group::new(vec![ICONIC], 0)]);
    collapsed.iconic = true;
    let end = Node::dock(DockEdge::End, vec![collapsed]);
    let middle = Node::split(Axis::X, SplitSize::Ratio(0.5), Node::tabs(vec![LEFT, LEFT_TWO], 0), Node::tabs(vec![RIGHT], 0));
    let root = Node::split(Axis::X, SplitSize::Ratio(0.25), start, Node::split(Axis::X, SplitSize::Ratio(0.85), middle, end));
    let window = Floating::single(FLOATING as u64, vec![FLOATING], 0, Rect::from_min_size(Pos2::new(420.0, 200.0), Vec2::new(220.0, 180.0)));
    Layout::new(Some(root), vec![window])
}

const ROOM: Vec2 = Vec2::new(1000.0, 640.0);

/// A fixture showing the layout under `shell`, keeping only `keep` when it names panels; what the area reported in
/// its last pass goes into the returned cell.
fn fixture(shell: Option<ShellCx>, keep: Option<BTreeSet<usize>>) -> (Fixture, Rc<RefCell<Vec<DockSource<usize>>>>) {
    let reported: Rc<RefCell<Vec<DockSource<usize>>>> = Rc::default();
    let seen = Rc::clone(&reported);
    let mut fixture = Fixture::new(move |ui| {
        let mut workspace = Arc::new(Workspace::new(layout()));
        let specs = Tracked::new(Numbered);
        let kept = Tracked::new(keep.iter().flatten().map(PanelKey::of).collect::<BTreeSet<_>>());
        let mut bodies = Bodies;
        let mut area = DockArea::new("Sources", Bind::bound(&mut workspace), &specs, &mut bodies, DockConfig::default());
        if keep.is_some() {
            area = area.hide(HidePanels::Except(&kept));
        }
        *seen.borrow_mut() = area.show(ui).sources;
    });
    fixture.room(ROOM);
    if let Some(shell) = shell {
        fixture.root_value(ShellEntry(Some(shell)));
    }
    fixture.settle();
    (fixture, reported)
}

/// The panel a source names, for a tab, a group bar or a window's grips.
fn named(source: &DockSource<usize>) -> Option<usize> {
    match &source.kind {
        SourceKind::Tab(panel) | SourceKind::GroupBar(panel) | SourceKind::WindowGrip(panel) | SourceKind::WindowResize(panel, _) => Some(*panel),
        SourceKind::Divider(DividerKind::Groups(panel) | DividerKind::ColumnEdge(panel)) => Some(*panel),
        _ => None,
    }
}

/// The panels named by the sources `pick` takes, in order, with repeats.
fn panels(sources: &[DockSource<usize>], pick: impl Fn(&SourceKind<usize>) -> Option<usize>) -> Vec<usize> {
    let mut picked: Vec<usize> = sources.iter().filter_map(|source| pick(&source.kind)).collect();
    picked.sort_unstable();
    picked
}

#[trial]
fn a_dock_area_reports_one_source_per_tab_bar_grip_and_divider_it_drew_and_the_same_after_an_idle_pass() {
    let (mut fixture, reported) = fixture(None, None);
    let sources = reported.borrow().clone();
    let tabs = panels(&sources, |kind| if let SourceKind::Tab(panel) = kind { Some(*panel) } else { None });
    assert_eq!(tabs, vec![UPPER, UPPER_TWO, LOWER, LEFT, LEFT_TWO, RIGHT, FLOATING], "one tab per panel shown in a tab strip: {sources:#?}");
    let bars = panels(&sources, |kind| if let SourceKind::GroupBar(panel) = kind { Some(*panel) } else { None });
    assert_eq!(bars, vec![UPPER, LOWER, LEFT, RIGHT, FLOATING], "one drag bar per shown group, named by its first panel");
    let grips = panels(&sources, |kind| if let SourceKind::WindowGrip(panel) = kind { Some(*panel) } else { None });
    assert_eq!(grips, vec![FLOATING], "the floating window's grip, named by its panel");
    let edges: Vec<_> =
        sources.iter().filter_map(|source| if let SourceKind::WindowResize(FLOATING, edge) = source.kind { Some(edge) } else { None }).collect();
    assert!(!edges.is_empty(), "the floating window reports its resize grips");
    let distinct = edges.iter().enumerate().all(|(at, edge)| !edges[..at].contains(edge));
    assert!(distinct, "each resize grip names its own edge: {edges:?}");

    let divider = |kind: DividerKind<usize>| sources.iter().filter(|source| source.kind == SourceKind::Divider(kind.clone())).count();
    assert_eq!(divider(DividerKind::Split(vec![true, false])), 1, "the divider of the middle split, by its path");
    assert_eq!(divider(DividerKind::Groups(UPPER)), 1, "the divider under the column's upper group");
    assert_eq!(divider(DividerKind::ColumnEdge(UPPER)), 1, "the inner edge of the start dock's column");
    assert!(sources.iter().all(|source| named(source) != Some(ICONIC)), "a column collapsed to icons reports no tab, bar or divider");

    let room = Rect::from_min_size(Pos2::ZERO, ROOM);
    for source in &sources {
        assert!(source.rect.is_positive() && room.contains_rect(source.rect), "every source lies in the window: {source:?}");
    }
    let window = sources.iter().find(|source| source.kind == SourceKind::WindowGrip(FLOATING)).map(|source| source.rect);
    let tab = sources.iter().find(|source| source.kind == SourceKind::Tab(FLOATING)).map(|source| source.rect);
    assert!(window.zip(tab).is_some_and(|(grip, tab)| grip.bottom() <= tab.top() + 0.5), "the window's grip lies above its tab");
    let ids: HashSet<_> = sources.iter().map(|source| source.id).collect();
    assert_eq!(ids.len(), sources.len(), "every source has its own node id");

    fixture.idle(3);
    assert_eq!(*reported.borrow(), sources, "an idle pass, which replays the settled parts, reports the same sources");

    let left_two = sources.iter().find(|source| source.kind == SourceKind::Tab(LEFT_TWO)).cloned().expect("the tab of the second panel");
    // A click on the tab picks it and leaves key focus; a press on the group's empty body then focuses its strip.
    fixture.click_at(left_two.rect.center()).settle();
    fixture.click_at(Pos2::new(left_two.rect.center().x, left_two.rect.bottom() + 60.0)).settle();
    assert_eq!(
        craft_ui::runtime::access::focused_node(fixture.ctx()),
        Some(left_two.id),
        "the picked tab holds focus under the id its source reports"
    );
}

#[trial]
fn a_panel_the_shell_hides_reports_no_source() {
    let room = Rect::from_min_size(Pos2::ZERO, ROOM);
    let hiding = ShellCx::detached(WindowKey::main(), BodyMode::Normal, room).with_panels_hidden(true);
    let keep = BTreeSet::from([UPPER, UPPER_TWO, LOWER, LEFT, LEFT_TWO, ICONIC]);
    let (_fixture, reported) = fixture(Some(hiding), Some(keep));
    let sources = reported.borrow();
    assert!(sources.iter().any(|source| source.kind == SourceKind::Tab(LEFT)), "a kept panel still reports its tab");
    for hidden in [RIGHT, FLOATING] {
        assert!(sources.iter().all(|source| named(source) != Some(hidden)), "the hidden panel {hidden} reports no tab, bar or grip: {sources:#?}");
    }
}

/// Each section of a stack is a group of one panel whose header holds that panel's tab: an open section and a closed
/// one each report it.
#[trial]
fn each_stack_section_reports_its_panels_tab() {
    let mut closed = StackEntry::new(LOWER);
    closed.open = false;
    let layout = Layout::new(Some(Node::stack(vec![StackEntry::new(UPPER), closed])), Vec::new());
    let reported: Rc<RefCell<Vec<DockSource<usize>>>> = Rc::default();
    let seen = Rc::clone(&reported);
    let mut fixture = Fixture::new(move |ui| {
        let mut workspace = Arc::new(Workspace::new(layout.clone()));
        let specs = Tracked::new(Numbered);
        *seen.borrow_mut() = DockArea::new("Stack", Bind::bound(&mut workspace), &specs, &mut Bodies, DockConfig::default()).show(ui).sources;
    });
    fixture.room(ROOM).settle();
    let tabs = panels(&reported.borrow(), |kind| if let SourceKind::Tab(panel) = kind { Some(*panel) } else { None });
    assert_eq!(tabs, [UPPER, LOWER], "{:?}", reported.borrow());
}

/// A group whose tabs do not fit reports its strip's overflow button, named by the group's first panel, under the id of
/// the button's node and where a click opens the menu of the hidden tabs; groups whose tabs fit report none.
#[trial]
fn a_group_whose_tabs_do_not_fit_reports_its_overflow_button() {
    const PANELS: [usize; 6] = [11, 12, 13, 14, 15, 16];
    let reported: Rc<RefCell<Vec<DockSource<usize>>>> = Rc::default();
    let seen = Rc::clone(&reported);
    let mut narrow = Fixture::new(move |ui| {
        let mut workspace = Arc::new(Workspace::new(Layout::new(Some(Node::tabs(PANELS.to_vec(), 0)), Vec::new())));
        let specs = Tracked::new(Numbered);
        let mut bodies = Bodies;
        *seen.borrow_mut() = DockArea::new("Overflow", Bind::bound(&mut workspace), &specs, &mut bodies, DockConfig::default()).show(ui).sources;
    });
    narrow.room(Vec2::new(160.0, 300.0)).settle();
    let overflow: Vec<DockSource<usize>> =
        reported.borrow().iter().filter(|source| matches!(source.kind, SourceKind::Overflow(_))).cloned().collect();
    let kinds: Vec<SourceKind<usize>> = overflow.iter().map(|source| source.kind.clone()).collect();
    assert_eq!(kinds, vec![SourceKind::Overflow(PANELS[0])], "one overflow button, named by the group's first panel");
    let more = narrow.by_label("More tabs").expect("the strip's overflow button shows");
    assert_eq!(more.node, NodeKey::of(overflow[0].id), "the source's id is the button's node");
    assert!(narrow.by_role(Role::MENU_ITEM).is_empty(), "no menu is open yet");
    narrow.click_at(overflow[0].rect.center()).settle();
    assert!(!narrow.by_role(Role::MENU_ITEM).is_empty(), "a click on the source's rect opens the menu of the hidden tabs");

    let (_wide, wide) = fixture(None, None);
    assert!(wide.borrow().iter().all(|source| !matches!(source.kind, SourceKind::Overflow(_))), "groups whose tabs fit report no overflow button");
}
