//! The runs of a settled docked layout: each group adds the same runs whatever its tab count, and a closed menu, the
//! group's header controls and its tabs add none.

use craft_behavior::Axis;
use craft_behavior::dock::{Layout, Node, SplitSize, Workspace};
use craft_ui::contract::{Bind, BodyCx};
use craft_ui::docking::{DockConfig, DockPanels, PanelSpec};
use craft_ui::menu::PanelRow;
use craft_ui::patterns::dock::DockArea;
use craft_ui::testing::{Fixture, last_pass_runs, trial};
use egui::{Ui, Vec2};

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
