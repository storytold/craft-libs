//! The runs of a settled docked layout: each group adds the same runs whatever its tab count, a closed menu, the
//! group's header controls and its tabs add none, and a panel body with a content version runs only when it changes.

use craft_behavior::Axis;
use craft_behavior::dock::{Layout, Node, SplitSize, Workspace};
use craft_ui::contract::{Bind, BodyCx};
use craft_ui::docking::{DockConfig, DockPanels, PanelSpec};
use craft_ui::menu::PanelRow;
use craft_ui::patterns::dock::DockArea;
use craft_ui::testing::{Fixture, last_pass_runs, trial};
use egui::{Ui, Vec2};
use std::cell::Cell;
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
