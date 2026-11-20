//! The runs of a settled docked layout: each group adds the same runs whatever its tab count, a closed menu, the
//! group's header controls and its tabs add none, and a panel body with a content version runs only when it changes.
//! With a host spec version the groups' chrome settles too: it runs only for the pointer, an arrangement or spec
//! change, or a body without a version.

use craft_behavior::Axis;
use craft_behavior::dock::{Action, Floating, Layout, Node, Permissions, SplitSize, Workspace};
use craft_ui::contract::{Bind, BodyCx, Role};
use craft_ui::docking::{DockConfig, DockPanels, GroupAt, PanelDrag, PanelSpec};
use craft_ui::menu::PanelRow;
use craft_ui::patterns::dock::DockArea;
use craft_ui::runtime::drag;
use craft_ui::testing::pointer::{moved, press};
use craft_ui::testing::{Fixture, last_pass_runs, trial};
use egui::{Pos2, Rect, Ui, Vec2};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

struct Panels;

impl DockPanels<usize> for Panels {
    fn spec(&self, panel: &usize) -> PanelSpec {
        PanelSpec::new(format!("Panel {panel}"))
    }

    fn body(&mut self, _: &usize, _: &mut Ui, _: &mut BodyCx<'_>) {}

    /// A host row, so each group shows its panel menu button and its menu.
    fn panel_rows(&mut self, _: &usize) -> Vec<PanelRow> {
        vec![PanelRow::Separator]
    }
}

/// The runs by type of a settled pass of a dock of `groups` groups side by side, `tabs` panels each.
fn settled_runs(groups: usize, tabs: usize) -> Vec<(&'static str, u32)> {
    let group = |at: usize| Node::tabs((tabs * at..tabs * (at + 1)).collect(), 0);
    let root = (1..groups).fold(group(0), |left, at| Node::split(Axis::X, SplitSize::Ratio(0.5), left, group(at)));
    let mut workspace = Workspace::new(Layout::new(Some(root), Vec::new()));
    let mut fixture = Fixture::new(move |ui| {
        DockArea::new("Dock", Bind::bound(&mut workspace), &mut Panels, DockConfig::default()).show(ui);
    });
    fixture.room(Vec2::new(1200.0, 600.0)).settle().idle(1);
    last_pass_runs(fixture.ctx())
}

fn total(runs: &[(&'static str, u32)]) -> u32 {
    runs.iter().map(|(_, runs)| runs).sum()
}

/// A settled dock runs no closed menu (the panel menus and the tabs' context menus), and each group it shows adds
/// the same runs.
#[trial]
fn a_settled_dock_runs_no_closed_menu_and_the_same_runs_per_group() {
    let runs: Vec<_> = (1..=3).map(|groups| settled_runs(groups, 2)).collect();
    for shown in &runs {
        let menus: Vec<_> = shown.iter().filter(|(ty, _)| ty.split('<').next().is_some_and(|name| name.ends_with("::Menu"))).collect();
        assert!(menus.is_empty(), "a settled dock ran closed menus: {menus:?}");
    }
    let (one, two, three) = (total(&runs[0]), total(&runs[1]), total(&runs[2]));
    assert_eq!(three - two, two - one, "the runs per group grow: {one}, {two}, {three} runs for 1, 2 and 3 groups");
}

/// The name of a run's type, without its path and its type arguments.
fn short(ty: &str) -> &str {
    let name = ty.split('<').next().unwrap_or(ty);
    name.rsplit("::").next().unwrap_or(name)
}

/// A group adds the same runs whether it shows one tab or three: its tabs are members of the strip's run, and its drag
/// band and panel menu button are its own parts. No `Row`, `Tab`, `IconButton` or `GroupGrip` runs.
#[trial]
fn a_settled_dock_group_runs_the_same_whatever_its_tab_count() {
    let step = |tabs: usize| total(&settled_runs(2, tabs)) - total(&settled_runs(1, tabs));
    assert_eq!(step(1), step(3), "a group's runs grow with its tabs");
    let shown = settled_runs(2, 3);
    let parts: Vec<_> = shown.iter().filter(|(ty, _)| matches!(short(ty), "Row" | "Tab" | "IconButton" | "GroupGrip")).collect();
    assert!(parts.is_empty(), "a settled dock ran header parts as runs: {parts:?}");
}

/// Two panels in two groups: panel 0 gives the content version `version`, panel 1 none. Each body counts its runs.
struct Versioned {
    runs: Rc<[Cell<usize>; 2]>,
    version: Rc<Cell<u64>>,
}

impl DockPanels<usize> for Versioned {
    fn spec(&self, panel: &usize) -> PanelSpec {
        PanelSpec::new(format!("Panel {panel}"))
    }

    fn body(&mut self, panel: &usize, _: &mut Ui, _: &mut BodyCx<'_>) {
        self.runs[*panel].set(self.runs[*panel].get() + 1);
    }

    fn content(&self, panel: &usize) -> Option<u64> {
        (*panel == 0).then(|| self.version.get())
    }
}

/// A panel body with a content version runs in the frame its version changes and settles otherwise; a body without one
/// runs every frame.
#[trial]
fn a_panel_body_with_a_content_version_runs_only_when_its_version_changes() {
    let (runs, version): (Rc<[Cell<usize>; 2]>, _) = (Rc::default(), Rc::new(Cell::new(1)));
    let root = Node::split(Axis::X, SplitSize::Ratio(0.5), Node::tabs(vec![0], 0), Node::tabs(vec![1], 0));
    let mut workspace = Workspace::new(Layout::new(Some(root), Vec::new()));
    let mut panels = Versioned { runs: Rc::clone(&runs), version: Rc::clone(&version) };
    let mut fixture = Fixture::new(move |ui| {
        DockArea::new("Dock", Bind::bound(&mut workspace), &mut panels, DockConfig::default()).show(ui);
    });
    fixture.room(Vec2::new(1200.0, 600.0)).settle();
    let mut frame = || {
        runs.iter().for_each(|runs| runs.set(0));
        fixture.frame(Vec::new());
        [runs[0].get(), runs[1].get()]
    };
    let [versioned, plain] = frame();
    assert!(versioned == 0 && plain > 0, "the versioned body settles, the other runs: {versioned}, {plain}");
    version.set(2);
    assert!(frame()[0] > 0, "a new version runs the versioned body");
    assert_eq!(frame()[0], 0, "and it settles at it");
}

/// Panel 0's group is the host's view and panel 1's the dock's; both panels give the content version `version`. The
/// view counts its runs where it shows, the bodies theirs.
struct Hosted {
    views: Rc<Cell<usize>>,
    bodies: Rc<Cell<usize>>,
    version: Rc<Cell<u64>>,
}

impl DockPanels<usize> for Hosted {
    fn spec(&self, panel: &usize) -> PanelSpec {
        PanelSpec::new(format!("Panel {panel}"))
    }

    fn body(&mut self, _: &usize, _: &mut Ui, _: &mut BodyCx<'_>) {
        self.bodies.set(self.bodies.get() + 1);
    }

    fn content(&self, _: &usize) -> Option<u64> {
        Some(self.version.get())
    }

    fn group_view(&mut self, panels: &[usize], _: usize, _: &mut Ui, at: GroupAt, _: &mut BodyCx<'_>) -> Option<f32> {
        if !panels.contains(&0) {
            return None;
        }
        match at {
            GroupAt::Measure(_) => Some(0.0),
            GroupAt::Show(rect) => {
                self.views.set(self.views.get() + 1);
                Some(rect.height())
            }
        }
    }
}

/// A host view settles as a panel body does: it runs in the frame its panel's version changes or the pointer comes
/// over it, and not in a settled frame. The group the host leaves to the dock keeps showing beside it.
#[trial]
fn a_host_view_with_a_content_version_runs_only_when_its_version_changes() {
    let (views, bodies, version) = (Rc::new(Cell::new(0)), Rc::new(Cell::new(0)), Rc::new(Cell::new(1)));
    let root = Node::split(Axis::X, SplitSize::Ratio(0.5), Node::tabs(vec![0], 0), Node::tabs(vec![1], 0));
    let mut workspace = Workspace::new(Layout::new(Some(root), Vec::new()));
    let mut panels = Hosted { views: Rc::clone(&views), bodies: Rc::clone(&bodies), version: Rc::clone(&version) };
    let mut fixture = Fixture::new(move |ui| {
        DockArea::new("Dock", Bind::bound(&mut workspace), &mut panels, DockConfig::default()).show(ui);
    });
    fixture.room(Vec2::new(1200.0, 600.0)).settle();
    let mut frame = |events: Vec<egui::Event>| {
        views.set(0);
        bodies.set(0);
        fixture.frame(events);
        (views.get(), bodies.get())
    };
    assert_eq!(frame(Vec::new()), (0, 0), "a settled frame runs neither the view nor the versioned body");
    assert_eq!(frame(Vec::new()), (0, 0), "nor the next one");
    version.set(2);
    let (viewed, bodied) = frame(Vec::new());
    assert!(viewed > 0 && bodied > 0, "a new version runs the view and the body: {viewed}, {bodied}");
    assert_eq!(frame(Vec::new()), (0, 0), "both settle at it");
    assert!(frame(vec![moved(Pos2::new(300.0, 300.0))]).0 > 0, "the pointer over the view runs it");
    assert!(fixture.by_label("Panel 1").is_some(), "the dock's group shows beside the view");
}

/// Panels whose specs and bodies all have versions, so the dock's chrome settles; `plain` is a panel whose body has
/// none.
struct Settling {
    spec: Rc<Cell<u64>>,
    plain: Option<usize>,
}

impl DockPanels<usize> for Settling {
    fn spec(&self, panel: &usize) -> PanelSpec {
        PanelSpec::new(format!("Panel {panel}"))
    }

    fn body(&mut self, _: &usize, _: &mut Ui, _: &mut BodyCx<'_>) {}

    fn content(&self, panel: &usize) -> Option<u64> {
        (self.plain != Some(*panel)).then_some(1)
    }

    fn spec_version(&self) -> Option<u64> {
        Some(self.spec.get())
    }

    fn panel_rows(&mut self, _: &usize) -> Vec<PanelRow> {
        vec![PanelRow::Separator]
    }
}

/// A settled dock whose panels are [`Settling`].
struct SettledDock {
    fixture: Fixture,
    /// The spec version the panels report.
    spec: Rc<Cell<u64>>,
    /// The workspace the dock shows.
    workspace: Rc<RefCell<Workspace<usize>>>,
}

/// A settled dock over `layout` whose panels are [`Settling`].
fn settling(layout: Layout<usize>, plain: Option<usize>) -> SettledDock {
    let spec = Rc::new(Cell::new(1));
    let workspace = Rc::new(RefCell::new(Workspace::new(layout)));
    let mut panels = Settling { spec: Rc::clone(&spec), plain };
    let shown = Rc::clone(&workspace);
    let mut fixture = Fixture::new(move |ui| {
        DockArea::new("Dock", Bind::bound(&mut *shown.borrow_mut()), &mut panels, DockConfig::default()).show(ui);
    });
    fixture.room(Vec2::new(1200.0, 600.0)).settle().idle(1);
    SettledDock { fixture, spec, workspace }
}

/// `groups` groups side by side, two tabs each.
fn side_by_side(groups: usize) -> Layout<usize> {
    let group = |at: usize| Node::tabs(vec![2 * at, 2 * at + 1], 0);
    let root = (1..groups).fold(group(0), |left, at| Node::split(Axis::X, SplitSize::Ratio(0.5), left, group(at)));
    Layout::new(Some(root), Vec::new())
}

/// How many runs of the types named `names` the last pass made.
fn runs_of(fixture: &Fixture, names: &[&str]) -> u32 {
    last_pass_runs(fixture.ctx()).iter().filter(|(ty, _)| names.contains(&short(ty))).map(|(_, runs)| runs).sum()
}

const CHROME: &[&str] = &["DockGroup", "TabStrip", "Tab", "GroupGrip"];

/// At rest a dock whose host versions its specs and bodies runs none of its groups' chrome, however many groups it
/// shows, and still paints their tabs.
#[trial]
fn a_settled_dock_runs_no_group_chrome_at_rest() {
    for groups in 1..=3 {
        let SettledDock { mut fixture, .. } = settling(side_by_side(groups), None);
        fixture.frame(Vec::new());
        assert_eq!(runs_of(&fixture, CHROME), 0, "{groups} settled groups ran chrome: {:?}", last_pass_runs(fixture.ctx()));
        let painted = |title: &str| fixture.texts().iter().any(|text| text.text == title);
        assert!((0..2 * groups).all(|panel| painted(&format!("Panel {panel}"))), "the replayed tabs paint their titles");
    }
}

/// The pointer over one group's tab runs that group alone; the others replay.
#[trial]
fn hovering_a_tab_runs_only_its_group() {
    let SettledDock { mut fixture, .. } = settling(side_by_side(3), None);
    let tab = fixture.by_label("Panel 4").expect("the third group's tab");
    fixture.frame(vec![egui::Event::PointerMoved(tab.center())]);
    assert_eq!(runs_of(&fixture, &["DockGroup"]), 1, "{:?}", last_pass_runs(fixture.ctx()));
}

/// An edit of the arrangement (a tab picked from outside the dock) runs every group (its measuring runs included), and
/// a change of the host's spec version does too; the frame after settles again.
#[trial]
fn an_arrangement_or_spec_change_runs_every_group() {
    let SettledDock { mut fixture, spec, workspace } = settling(side_by_side(3), None);
    workspace.borrow_mut().apply(Action::activate(1), false, |_| Permissions::ALL).expect("panel 1 is shown");
    fixture.frame(Vec::new());
    assert!(runs_of(&fixture, &["DockGroup"]) >= 3, "the edit runs every group: {:?}", last_pass_runs(fixture.ctx()));
    fixture.frame(Vec::new());
    assert_eq!(runs_of(&fixture, &["DockGroup"]), 0, "and they settle at it");
    spec.set(2);
    fixture.frame(Vec::new());
    assert!(runs_of(&fixture, &["DockGroup"]) >= 3, "a new spec version runs every group: {:?}", last_pass_runs(fixture.ctx()));
    fixture.frame(Vec::new());
    assert_eq!(runs_of(&fixture, &["DockGroup"]), 0, "and they settle at it");
}

/// A group whose shown body has no content version runs every pass, as its body does; the others settle.
#[trial]
fn a_group_whose_body_has_no_version_keeps_running() {
    let SettledDock { mut fixture, .. } = settling(side_by_side(2), Some(0));
    fixture.frame(Vec::new());
    assert_eq!(runs_of(&fixture, &["DockGroup"]), 1, "{:?}", last_pass_runs(fixture.ctx()));
}

/// A floating window's groups settle as the docked ones do.
#[trial]
fn a_settled_floating_window_runs_no_group_chrome() {
    let window = Floating::single(vec![9], 0, Rect::from_min_size(Pos2::new(600.0, 100.0), Vec2::new(300.0, 300.0)));
    let SettledDock { mut fixture, .. } = settling(Layout::new(Some(Node::tabs(vec![0], 0)), vec![window]), None);
    fixture.frame(Vec::new());
    assert_eq!(runs_of(&fixture, CHROME), 0, "{:?}", last_pass_runs(fixture.ctx()));
}

/// A tab dragged straight onto the strip of a group that replayed its last run finds the group's drop spot in the
/// frame it arrives: the replay registered the spots again.
#[trial]
fn a_tab_dropped_on_a_settled_group_lands_there() {
    let SettledDock { mut fixture, workspace, .. } = settling(side_by_side(3), None);
    let (from, to) = (fixture.by_label("Panel 0").expect("a tab").center(), fixture.by_label("Panel 4").expect("a tab").center());
    fixture.frame(vec![moved(from)]).frame(vec![press(from)]).frame(vec![moved(from + Vec2::new(0.0, 12.0))]);
    assert!(drag::active::<PanelDrag>(fixture.ctx()).is_some(), "the tab drag started");
    fixture.frame(vec![moved(to)]);
    assert!(drag::mark(fixture.ctx()).is_some(), "the settled group's strip takes the drop in the frame the pointer arrives");
    fixture.release_at(to).idle(1);
    let layout = &workspace.borrow().layout;
    let group = layout.place(&4).and_then(|place| layout.node_of(&place));
    assert!(matches!(group, Some(Node::Tabs { panels, .. }) if panels.contains(&0)), "the tab joined the third group: {group:?}");
}

/// Close in a settled group's panel menu closes the panel in the frame after the pick, with the pointer gone from the
/// group: the command the menu queued for the group reaches it.
#[trial]
fn close_in_a_settled_groups_panel_menu_closes_the_panel() {
    let SettledDock { mut fixture, workspace, .. } = settling(side_by_side(2), None);
    let buttons: Vec<_> =
        fixture.by_role(Role::BUTTON).into_iter().filter(|button| fixture.tree().name_of(button.node) == Some("Panel Menu")).collect();
    assert_eq!(buttons.len(), 2, "each group shows its panel menu button");
    fixture.click(&buttons[1]).idle(2);
    let close = fixture.by_label("Close").expect("the menu's Close row");
    fixture.click(&close);
    fixture.frame(vec![egui::Event::PointerGone]);
    assert!(workspace.borrow().layout.place(&2).is_none(), "Close closed the second group's panel");
}

/// Panels without versions whose bodies note the id of the `Ui` they run in.
struct Noted {
    ids: Rc<RefCell<Vec<(usize, egui::Id)>>>,
}

impl DockPanels<usize> for Noted {
    fn spec(&self, panel: &usize) -> PanelSpec {
        PanelSpec::new(format!("Panel {panel}"))
    }

    fn body(&mut self, panel: &usize, ui: &mut Ui, _: &mut BodyCx<'_>) {
        self.ids.borrow_mut().push((*panel, ui.scope_id()));
    }
}

/// Moving another tab to the front of a group keeps the shown body's ids, and with them its scroll, collapsing and
/// text state.
#[trial]
fn a_tab_moved_to_the_front_keeps_the_shown_bodys_ids() {
    let ids: Rc<RefCell<Vec<(usize, egui::Id)>>> = Rc::default();
    let workspace = Rc::new(RefCell::new(Workspace::new(Layout::new(Some(Node::tabs(vec![0, 1], 0)), Vec::new()))));
    let (shown, mut panels) = (Rc::clone(&workspace), Noted { ids: Rc::clone(&ids) });
    let mut fixture = Fixture::new(move |ui| {
        DockArea::new("Dock", Bind::bound(&mut *shown.borrow_mut()), &mut panels, DockConfig::default()).show(ui);
    });
    fixture.room(Vec2::new(800.0, 600.0)).settle();
    let body_id = |ids: &RefCell<Vec<(usize, egui::Id)>>| ids.borrow().iter().rev().find(|(panel, _)| *panel == 0).map(|(_, id)| *id);
    let before = body_id(&ids).expect("panel 0's body ran");
    workspace.borrow_mut().layout.root = Some(Node::tabs(vec![1, 0], 1));
    ids.borrow_mut().clear();
    fixture.frame(Vec::new());
    assert_eq!(body_id(&ids), Some(before), "the shown body keeps its id when another tab moves to the front");
}
