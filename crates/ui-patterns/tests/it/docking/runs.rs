//! The runs of a settled docked layout. Each group adds the same runs whatever its tab count, and a closed menu, the
//! group's header controls and its tabs add none. The dock's chrome (each group's tab row, each column's bar) and the
//! host's memoized bodies are memoized views: at rest they replay, and a change runs only the views it reaches.

use craft_behavior::Axis;
use craft_behavior::dock::{Action, Floating, Layout, Node, Permissions, SplitSize, Workspace};
use craft_behavior::dock::{Column, DockEdge, Group};
use craft_ui::buttons::Button;
use craft_ui::contract::{Bind, BodyCx, MemoProps, MemoView, PlainData, Role, Show as _, Tracked};
use craft_ui::docking::{DockConfig, DockPanels, PanelDrag, PanelSpec, PanelSpecs};
use craft_ui::menu::PanelRow;
use craft_ui::patterns::dock::DockArea;
use craft_ui::runtime::drag;
use craft_ui::testing::pointer::{moved, press};
use craft_ui::testing::{
    Fixture, MemoRun, last_frame_derived_builds, last_frame_memo_runs, last_pass_derived_builds, last_pass_memo_key_bytes, last_pass_memo_runs,
    last_pass_runs, trial,
};
use craft_ui::testing::{audit_replays, in_audit_run};
use egui::{Pos2, Rect, Ui, Vec2};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

/// Panels named by their number, which count the specs the dock reads.
#[derive(PlainData)]
struct Named;

impl PanelSpecs<usize> for Named {
    fn spec(&self, panel: &usize) -> PanelSpec {
        SPEC_READS.with(|reads| reads.set(reads.get() + 1));
        PanelSpec::new(format!("Panel {panel}"))
    }
}

struct Panels;

impl DockPanels<usize> for Panels {
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
    let mut workspace = Arc::new(Workspace::new(Layout::new(Some(root), Vec::new())));
    let specs = Tracked::new(Named);
    let mut fixture = Fixture::new(move |ui| {
        DockArea::new("Dock", Bind::bound(&mut workspace), &specs, &mut Panels, DockConfig::default()).show(ui);
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

thread_local! {
    /// The panels whose memoized bodies ran on this trial's thread, in order.
    static BODY_RUNS: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
    /// The panel specs the docks read on this trial's thread.
    static SPEC_READS: Cell<u32> = const { Cell::new(0) };
}

/// Takes the count of specs the docks read since the last call.
fn spec_reads() -> u32 {
    SPEC_READS.with(|reads| reads.replace(0))
}

/// Takes the panels whose bodies ran since the last call.
fn body_runs() -> Vec<usize> {
    BODY_RUNS.with(|runs| std::mem::take(&mut *runs.borrow_mut()))
}

/// What a panel's memoized body shows.
#[derive(MemoProps)]
struct BodyProps {
    panel: usize,
}

fn panel_body(props: &mut BodyProps, ui: &mut Ui) {
    // An audit run is the replay audit's, not the pass's: the trials count none.
    if !in_audit_run(ui.ctx()) {
        BODY_RUNS.with(|runs| runs.borrow_mut().push(props.panel));
    }
    Button::new(format!("Body {}", props.panel)).show(ui);
}

/// The panels' names, by panel; the specs count their reads.
#[derive(PlainData)]
struct Labels(Vec<String>);

impl PanelSpecs<usize> for Labels {
    fn spec(&self, panel: &usize) -> PanelSpec {
        SPEC_READS.with(|reads| reads.set(reads.get() + 1));
        PanelSpec::new(self.0[*panel].clone())
    }
}

/// Panels whose bodies are memoized views; `plain` is a panel whose body is not, which counts its runs in
/// `plain_runs`.
struct Memo {
    plain: Option<usize>,
    plain_runs: Rc<Cell<u32>>,
}

impl DockPanels<usize> for Memo {
    fn body(&mut self, panel: &usize, ui: &mut Ui, _: &mut BodyCx<'_>) {
        if self.plain == Some(*panel) {
            self.plain_runs.set(self.plain_runs.get() + 1);
            Button::new("Plain").show(ui);
            return;
        }
        let _ = MemoView::new(("body", *panel), BodyProps { panel: *panel }, panel_body).show(ui);
    }

    fn panel_rows(&mut self, _: &usize) -> Vec<PanelRow> {
        vec![PanelRow::Separator]
    }
}

/// A settled dock whose panels are [`Memo`].
struct MemoDock {
    fixture: Fixture,
    labels: Rc<RefCell<Tracked<Labels>>>,
    plain_runs: Rc<Cell<u32>>,
    workspace: Rc<RefCell<Arc<Workspace<usize>>>>,
    /// The panel each pass of the dock reported picked.
    picked: Rc<RefCell<Vec<usize>>>,
}

/// A settled dock over `layout` whose panels are [`Memo`], with `plain` a body that is not memoized.
fn memo_dock(layout: Layout<usize>, plain: Option<usize>) -> MemoDock {
    let labels = Rc::new(RefCell::new(Tracked::new(Labels((0..32).map(|panel| format!("Panel {panel}")).collect()))));
    let plain_runs = Rc::new(Cell::new(0));
    let workspace = Rc::new(RefCell::new(Arc::new(Workspace::new(layout))));
    let picked = Rc::new(RefCell::new(Vec::new()));
    let mut panels = Memo { plain, plain_runs: Rc::clone(&plain_runs) };
    let (shown, seen, named) = (Rc::clone(&workspace), Rc::clone(&picked), Rc::clone(&labels));
    let mut fixture = Fixture::new(move |ui| {
        let specs = named.borrow();
        let output = DockArea::new("Dock", Bind::bound(&mut *shown.borrow_mut()), &*specs, &mut panels, DockConfig::default()).show(ui);
        seen.borrow_mut().extend(output.picked);
    });
    fixture.room(Vec2::new(1200.0, 600.0)).settle().idle(1);
    body_runs();
    spec_reads();
    picked.borrow_mut().clear();
    MemoDock { fixture, labels, plain_runs, workspace, picked }
}

/// Idle frames that settle a view after a change: its animations have ended.
const SETTLE: usize = 30;

/// `groups` groups side by side, two tabs each.
fn side_by_side(groups: usize) -> Layout<usize> {
    let group = |at: usize| Node::tabs(vec![2 * at, 2 * at + 1], 0);
    let root = (1..groups).fold(group(0), |left, at| Node::split(Axis::X, SplitSize::Ratio(0.5), left, group(at)));
    Layout::new(Some(root), Vec::new())
}

/// `groups` groups side by side, then a dock at the end edge with a column of panel 20 and an iconic column of panel
/// 21, and a floating window of panel 22.
fn full(groups: usize) -> Layout<usize> {
    let mut iconic = Column::new(vec![Group::new(vec![21], 0)]).with_width(240.0);
    iconic.iconic = true;
    let columns = vec![Column::new(vec![Group::new(vec![20], 0)]).with_width(240.0), iconic];
    let root = side_by_side(groups).root.expect("the groups");
    let root = Node::split(Axis::X, SplitSize::Ratio(0.7), root, Node::dock(DockEdge::End, columns));
    let window = Floating::single(100, vec![22], 0, Rect::from_min_size(Pos2::new(200.0, 300.0), Vec2::new(300.0, 200.0)));
    Layout::new(Some(root), vec![window])
}

/// How the last pass's memoized views took part, by the short name of their props type: `(name, ran, replayed)`.
fn memo_runs(fixture: &Fixture) -> Vec<(&'static str, u32, u32)> {
    tally(last_pass_memo_runs(fixture.ctx()))
}

/// How the memoized views took part in every pass of the last frame, sizing and measuring calls included, by the short
/// name of their props type: `(name, ran, replayed)`.
fn frame_memo_runs(fixture: &Fixture) -> Vec<(&'static str, u32, u32)> {
    tally(last_frame_memo_runs(fixture.ctx()))
}

/// The views of `name` that ran in the last frame, by id, one entry per call that ran.
fn frame_ran_ids(fixture: &Fixture, name: &str) -> Vec<egui::Id> {
    last_frame_memo_runs(fixture.ctx()).into_iter().filter(|(of, _, run)| short(of) == name && *run == MemoRun::Ran).map(|(_, id, _)| id).collect()
}

/// `runs` counted by the short name of their props type: `(name, ran, replayed)`.
fn tally(runs: Vec<(&'static str, egui::Id, MemoRun)>) -> Vec<(&'static str, u32, u32)> {
    let mut by_name: Vec<(&'static str, u32, u32)> = Vec::new();
    for (name, _, run) in runs {
        let name = short(name);
        let at = match by_name.iter().position(|(seen, _, _)| *seen == name) {
            Some(at) => at,
            None => {
                by_name.push((name, 0, 0));
                by_name.len() - 1
            }
        };
        match run {
            MemoRun::Ran => by_name[at].1 += 1,
            MemoRun::Replayed => by_name[at].2 += 1,
            _ => {}
        }
    }
    by_name
}

/// The memoized views of `name` that ran and replayed in the last pass.
fn memo_of(fixture: &Fixture, name: &str) -> (u32, u32) {
    memo_runs(fixture).iter().find(|(seen, _, _)| *seen == name).map_or((0, 0), |(_, ran, replayed)| (*ran, *replayed))
}

/// The memoized views that ran in the last pass.
fn ran(fixture: &Fixture) -> u32 {
    memo_runs(fixture).iter().map(|(_, ran, _)| ran).sum()
}

/// How many runs of the types named `names` the last pass made.
fn runs_of(fixture: &Fixture, names: &[&str]) -> u32 {
    last_pass_runs(fixture.ctx()).iter().filter(|(ty, _)| names.contains(&short(ty))).map(|(_, runs)| runs).sum()
}

/// At rest a dock with groups, a column, an iconic column and a floating window runs none of its memoized views: every
/// tab row, bar and body replays, the replays grow by the same count per group, and each group runs its container
/// once, whatever its panel count. The replayed tabs still paint their titles.
#[trial]
fn an_idle_dock_runs_no_memo_view() {
    let mut replays = Vec::new();
    for groups in 1..=3 {
        let MemoDock { mut fixture, .. } = memo_dock(full(groups), None);
        fixture.frame(Vec::new());
        assert_eq!(ran(&fixture), 0, "{groups} groups ran memoized views: {:?}", memo_runs(&fixture));
        assert_eq!(body_runs(), Vec::<usize>::new(), "no body ran");
        // The side-by-side groups, the column's, the iconic column's flyout is closed, the window's.
        assert_eq!(memo_of(&fixture, "TabRow").1 as usize, groups + 2, "every shown group's tab row replays");
        assert_eq!(memo_of(&fixture, "BarProps").1, 2, "the column's bar and the iconic column's bar replay");
        assert_eq!(runs_of(&fixture, &["DockGroup"]) as usize, groups + 2, "each shown group runs its container once");
        assert_eq!(runs_of(&fixture, &["TabStrip", "Tab", "GroupGrip", "Menu"]), 0, "{:?}", last_pass_runs(fixture.ctx()));
        let painted = |title: &str| fixture.texts().iter().any(|text| text.text == title);
        assert!((0..2 * groups).all(|panel| painted(&format!("Panel {panel}"))), "the replayed tabs paint their titles");
        replays.push(memo_runs(&fixture).iter().map(|(_, _, replayed)| replayed).sum::<u32>());
    }
    assert_eq!(replays[2] - replays[1], replays[1] - replays[0], "the replays per group grow: {replays:?}");
    // A group's container runs once whatever its panel count.
    let three_tabs = Layout::new(Some(Node::tabs(vec![0, 1, 2], 0)), Vec::new());
    let MemoDock { mut fixture, .. } = memo_dock(three_tabs, None);
    fixture.frame(Vec::new());
    assert_eq!(runs_of(&fixture, &["DockGroup"]), 1, "a group of three panels runs its container once");
}

/// The pointer over one panel's body runs that body alone; the tab rows and the other bodies replay.
#[trial]
fn the_pointer_over_a_body_runs_only_that_body() {
    let MemoDock { mut fixture, .. } = memo_dock(side_by_side(3), None);
    let body = fixture.by_label("Body 2").expect("the second group's body").center();
    fixture.frame(vec![moved(body)]);
    assert_eq!(body_runs(), vec![2], "only panel 2's body ran");
    assert_eq!(memo_of(&fixture, "TabRow").0, 0, "no tab row ran: {:?}", memo_runs(&fixture));
}

/// Picking a tab from outside the dock runs that group's tab row and the body it now shows; every other view replays,
/// and the frame after settles.
#[trial]
fn a_tab_switch_runs_only_its_group() {
    let MemoDock { mut fixture, workspace, .. } = memo_dock(side_by_side(3), None);
    Arc::make_mut(&mut workspace.borrow_mut()).apply(Action::activate(3), false, |_| Permissions::ALL).expect("panel 3 is shown");
    fixture.frame(Vec::new());
    let rows_ran = frame_ran_ids(&fixture, "TabRow");
    // The changed row runs in the real run; the discard pass its new size asks for measures it once (no measure of the
    // new content is kept yet) and replays it in its real run.
    assert!(
        rows_ran.len() == 2 && rows_ran[0] == rows_ran[1],
        "only the second group's tab row ran: its real run, then one measuring run in the discard pass's sizing pass: {:?}",
        frame_memo_runs(&fixture)
    );
    assert_eq!(memo_of(&fixture, "TabRow"), (0, 3), "the last pass replays every tab row: {:?}", memo_runs(&fixture));
    assert_eq!(
        body_runs(),
        vec![3, 3],
        "only the newly shown body ran: its real run, then one measuring run in the sizing pass that first measures it"
    );
    // The tab strip's indicator moves to the new tab, which runs the row while it animates.
    fixture.idle(SETTLE).frame(Vec::new());
    assert_eq!(ran(&fixture), 0, "and they settle at it: {:?}", memo_runs(&fixture));
}

/// A new label for one panel runs only its group's tab row, once: the other groups' menus, which name it, are built
/// only while open. The row fits its tabs to the new label's width in that frame: a label too long for the group
/// moves the group's other tab to the overflow menu.
#[trial]
fn a_label_change_runs_only_its_groups_chrome() {
    const RENAMED: &str = "A renamed panel whose title is far too long to share its group's tab row with the other panel";
    let MemoDock { mut fixture, labels, .. } = memo_dock(side_by_side(3), None);
    assert!(fixture.by_label("More tabs").is_none(), "every group's tabs fit before the change");
    labels.borrow_mut().edit(|labels| labels.0[4] = RENAMED.to_owned());
    fixture.frame(Vec::new());
    let rows_ran = frame_ran_ids(&fixture, "TabRow");
    assert_eq!(
        rows_ran.len(),
        1,
        "only the third group's tab row ran, once: it measures the new label before it places its tabs: {:?}",
        frame_memo_runs(&fixture)
    );
    assert_eq!(fixture.passes(), 1, "the frame runs once");
    assert_eq!(memo_of(&fixture, "TabRow"), (1, 2), "the other tab rows replay: {:?}", memo_runs(&fixture));
    assert_eq!(body_runs(), Vec::<usize>::new(), "no body ran");
    assert!(fixture.texts().iter().any(|text| text.text == RENAMED), "the tab shows the new label");
    assert!(fixture.by_label("Panel 5").is_none(), "the group's other tab no longer fits");
    assert!(fixture.by_label("More tabs").is_some(), "the group's overflow menu button shows in the frame of the change");
    fixture.idle(SETTLE).frame(Vec::new());
    assert_eq!(ran(&fixture), 0, "and it settles at it: {:?}", memo_runs(&fixture));
    assert!(fixture.by_label("More tabs").is_some(), "the overflow menu button stays");
}

/// A body the host does not memoize runs every pass while the memoized views replay.
#[trial]
fn an_unconverted_body_runs_every_pass_while_the_others_replay() {
    let MemoDock { mut fixture, plain_runs, .. } = memo_dock(side_by_side(2), Some(0));
    for _ in 0..3 {
        plain_runs.set(0);
        fixture.frame(Vec::new());
        assert_eq!(plain_runs.get(), 1, "the plain body runs once per pass");
        assert_eq!(ran(&fixture), 0, "the memoized views replay: {:?}", memo_runs(&fixture));
    }
}

/// A click on a tab of a tab row that replayed reaches the dock once, through the row's outbox: idle frames that replay
/// the rows report no pick, the click reports one, and the frames after it (the focus stays on the strip, which runs
/// the row) report no other.
#[trial]
fn a_click_on_a_replayed_tab_row_is_picked_once() {
    let MemoDock { mut fixture, picked, workspace, .. } = memo_dock(side_by_side(2), None);
    for _ in 0..3 {
        fixture.frame(Vec::new());
        assert_eq!(memo_of(&fixture, "TabRow"), (0, 2), "both tab rows replay before the click");
    }
    assert_eq!(*picked.borrow(), Vec::<usize>::new(), "an idle replay picks nothing");
    let tab = fixture.by_label("Panel 1").expect("the first group's second tab");
    fixture.click(&tab).idle(3);
    assert_eq!(*picked.borrow(), vec![1], "the click picked panel 1 once");
    fixture.frame(vec![egui::Event::PointerGone]).idle(SETTLE);
    assert_eq!(*picked.borrow(), vec![1], "no frame after the click picks again");
    let shown = workspace.borrow();
    let layout = shown.layout();
    assert!(matches!(layout.root.as_ref(), Some(Node::Split { first, .. }) if matches!(&**first, Node::Tabs { active: 1, .. })), "panel 1 is shown");
}

/// A double click on a replayed column bar collapses the column: the bar's view sends it through its outbox.
#[trial]
fn a_double_click_on_a_replayed_column_bar_collapses_its_column() {
    let columns = vec![Column::new(vec![Group::new(vec![20], 0)]).with_width(240.0)];
    let layout =
        Layout::new(Some(Node::split(Axis::X, SplitSize::Ratio(0.7), Node::tabs(vec![0], 0), Node::dock(DockEdge::End, columns))), Vec::new());
    let MemoDock { mut fixture, workspace, .. } = memo_dock(layout, None);
    fixture.frame(Vec::new());
    assert_eq!(memo_of(&fixture, "BarProps"), (0, 1), "the bar replays before the click");
    let band = fixture.by_label("Move panel column").expect("the column's band").center();
    fixture.click_at(band).click_at(band).settle();
    let shown = workspace.borrow();
    let layout = shown.layout();
    let Some(Node::Split { second, .. }) = layout.root.as_ref() else { panic!("the split") };
    assert!(matches!(&**second, Node::Dock { columns, .. } if columns[0].iconic), "the column collapsed to icons: {layout:?}");
}

/// A tab dragged straight onto the strip of a group that replayed its last run finds the group's drop spot in the
/// frame it arrives: the replay registered the spots again.
#[trial]
fn a_tab_dropped_on_a_settled_group_lands_there() {
    let MemoDock { mut fixture, workspace, .. } = memo_dock(side_by_side(3), None);
    let (from, to) = (fixture.by_label("Panel 0").expect("a tab").center(), fixture.by_label("Panel 4").expect("a tab").center());
    fixture.frame(vec![moved(from)]).frame(vec![press(from)]).frame(vec![moved(from + Vec2::new(0.0, 12.0))]);
    assert!(drag::active::<PanelDrag>(fixture.ctx()).is_some(), "the tab drag started");
    fixture.frame(vec![moved(to)]);
    assert!(drag::mark(fixture.ctx()).is_some(), "the settled group's strip takes the drop in the frame the pointer arrives");
    fixture.release_at(to).idle(1);
    let shown = workspace.borrow();
    let layout = shown.layout();
    let group = layout.place(&4).and_then(|place| layout.node_of(&place));
    assert!(matches!(group, Some(Node::Tabs { panels, .. }) if panels.contains(&0)), "the tab joined the third group: {group:?}");
}

/// Close in a settled group's panel menu closes the panel in the frame after the pick, with the pointer gone from the
/// group: the command the menu queued for the group reaches it.
#[trial]
fn close_in_a_settled_groups_panel_menu_closes_the_panel() {
    let MemoDock { mut fixture, workspace, .. } = memo_dock(side_by_side(2), None);
    let buttons: Vec<_> =
        fixture.by_role(Role::BUTTON).into_iter().filter(|button| fixture.tree().name_of(button.node) == Some("Panel Menu")).collect();
    assert_eq!(buttons.len(), 2, "each group shows its panel menu button");
    fixture.click(&buttons[1]).idle(2);
    let close = fixture.by_label("Close").expect("the menu's Close row");
    fixture.click(&close);
    fixture.frame(vec![egui::Event::PointerGone]);
    assert!(workspace.borrow().layout().place(&2).is_none(), "Close closed the second group's panel");
}

/// Panels without versions whose bodies note the id of the `Ui` they run in.
struct Noted {
    ids: Rc<RefCell<Vec<(usize, egui::Id)>>>,
}

impl DockPanels<usize> for Noted {
    fn body(&mut self, panel: &usize, ui: &mut Ui, _: &mut BodyCx<'_>) {
        self.ids.borrow_mut().push((*panel, ui.scope_id()));
    }
}

/// Moving another tab to the front of a group keeps the shown body's ids, and with them its scroll, collapsing and
/// text state.
#[trial]
fn a_tab_moved_to_the_front_keeps_the_shown_bodys_ids() {
    let ids: Rc<RefCell<Vec<(usize, egui::Id)>>> = Rc::default();
    let workspace = Rc::new(RefCell::new(Arc::new(Workspace::new(Layout::new(Some(Node::tabs(vec![0, 1], 0)), Vec::new())))));
    let (shown, mut panels, specs) = (Rc::clone(&workspace), Noted { ids: Rc::clone(&ids) }, Tracked::new(Named));
    let mut fixture = Fixture::new(move |ui| {
        DockArea::new("Dock", Bind::bound(&mut *shown.borrow_mut()), &specs, &mut panels, DockConfig::default()).show(ui);
    });
    fixture.room(Vec2::new(800.0, 600.0)).settle();
    let body_id = |ids: &RefCell<Vec<(usize, egui::Id)>>| ids.borrow().iter().rev().find(|(panel, _)| *panel == 0).map(|(_, id)| *id);
    let before = body_id(&ids).expect("panel 0's body ran");
    *workspace.borrow_mut() = Arc::new(Workspace::new(Layout::new(Some(Node::tabs(vec![1, 0], 1)), Vec::new())));
    ids.borrow_mut().clear();
    fixture.frame(Vec::new());
    assert_eq!(body_id(&ids), Some(before), "the shown body keeps its id when another tab moves to the front");
}

/// An idle dock reads no panel spec and builds nothing it derives: its specs, permissions and keys are kept while the
/// workspace, the spec model and the lock are unchanged. That holds for a read-only dock too.
#[trial]
fn an_idle_dock_reads_no_spec_and_builds_nothing() {
    let MemoDock { mut fixture, .. } = memo_dock(side_by_side(4), None);
    for _ in 0..3 {
        fixture.frame(Vec::new());
        assert_eq!(last_pass_derived_builds(fixture.ctx()), Vec::new(), "an idle pass of a dock of 8 panels builds nothing");
    }
    assert_eq!(spec_reads(), 0, "and reads no spec");

    let shown = Arc::new(Workspace::new(side_by_side(4)));
    let specs = Tracked::new(Named);
    let mut fixture = Fixture::new(move |ui| {
        DockArea::read_only("Shown", &shown, &specs, &mut Panels, DockConfig::default()).show(ui);
    });
    fixture.room(Vec2::new(1200.0, 600.0)).settle().idle(1);
    spec_reads();
    for _ in 0..3 {
        fixture.frame(Vec::new());
        assert_eq!(last_pass_derived_builds(fixture.ctx()), Vec::new(), "an idle pass of a read-only dock builds nothing");
    }
    assert_eq!(spec_reads(), 0, "and reads no spec");
}

/// One workspace edit reads the specs once, in the frame of the edit, and none after.
#[trial]
fn one_workspace_edit_reads_the_specs_once() {
    let MemoDock { mut fixture, workspace, .. } = memo_dock(side_by_side(4), None);
    Arc::make_mut(&mut workspace.borrow_mut()).apply(Action::activate(3), false, |_| Permissions::ALL).expect("panel 3 is shown");
    fixture.frame(Vec::new());
    assert_eq!(spec_reads(), 8, "the edit reads each of the 8 panels' specs once: one build");
    fixture.frame(Vec::new());
    assert_eq!(last_pass_derived_builds(fixture.ctx()), Vec::new(), "the next pass builds nothing");
    assert_eq!(spec_reads(), 0, "and reads no spec");
}

/// The key bytes of an idle pass of a dock whose single group holds `tabs` panels.
fn idle_key_bytes(tabs: usize) -> usize {
    let MemoDock { mut fixture, .. } = memo_dock(Layout::new(Some(Node::tabs((0..tabs).collect(), 0)), Vec::new()), None);
    fixture.frame(Vec::new());
    assert_eq!(last_pass_derived_builds(fixture.ctx()), Vec::new(), "an idle pass builds no derived value");
    last_pass_memo_key_bytes(fixture.ctx())
}

/// An idle dock feeds the same key bytes whatever the number of its panels.
#[trial]
fn a_docks_idle_key_bytes_do_not_grow_with_its_panels() {
    assert_eq!(idle_key_bytes(4), idle_key_bytes(32), "the tab row's key follows the dock's revisions, not its titles");
}

/// In the pass a click on another tab of the focused group picks its panel, `focused` names that panel, and so do the
/// passes after it. Guards against a dock that named the panel the group drew in that pass, the one before the click.
#[trial]
fn a_tab_picked_in_the_focused_group_is_the_focused_panel() {
    let labels = Tracked::new(Labels((0..32).map(|panel| format!("Panel {panel}")).collect()));
    let workspace = Rc::new(RefCell::new(Arc::new(Workspace::new(side_by_side(2)))));
    let reported = Rc::new(RefCell::new(Vec::new()));
    let (shown, seen) = (Rc::clone(&workspace), Rc::clone(&reported));
    let mut panels = Memo { plain: None, plain_runs: Rc::new(Cell::new(0)) };
    let mut fixture = Fixture::new(move |ui| {
        let output = DockArea::new("Dock", Bind::bound(&mut *shown.borrow_mut()), &labels, &mut panels, DockConfig::default()).show(ui);
        seen.borrow_mut().push((output.focused, output.picked));
    });
    fixture.room(Vec2::new(1200.0, 600.0)).settle().idle(1);
    // A click on a tab leaves key focus where it is: Tab moves it into the first group, whose shown panel is panel 0.
    for _ in 0..4 {
        if reported.borrow().last() == Some(&(Some(0), None)) {
            break;
        }
        fixture.key(egui::Key::Tab, egui::Modifiers::NONE).idle(1);
    }
    assert_eq!(reported.borrow().last(), Some(&(Some(0), None)), "Tab reaches the first group");
    let tab = |fixture: &Fixture, name: &str| fixture.by_label(name).expect("the first group's tab");
    let second = tab(&fixture, "Panel 1");
    fixture.click(&second).idle(3);
    assert_eq!(reported.borrow().last(), Some(&(Some(1), None)), "the clicked tab's group holds the focus");
    reported.borrow_mut().clear();
    let first = tab(&fixture, "Panel 0");
    fixture.click(&first).idle(3);
    let reported = reported.borrow();
    let at = reported.iter().position(|(_, picked)| *picked == Some(0)).expect("the click picks panel 0");
    assert!(reported[at..].iter().all(|(focused, _)| *focused == Some(0)), "from the pick on, panel 0 is focused: {reported:?}");
}

/// Panels whose bodies are memoized views and whose host views have versions ([`DockPanels::content`]), one per
/// panel, in a list a trial moves.
struct Versioned {
    versions: Rc<RefCell<Vec<u64>>>,
}

impl DockPanels<usize> for Versioned {
    fn body(&mut self, panel: &usize, ui: &mut Ui, _: &mut BodyCx<'_>) {
        let _ = MemoView::new(("body", *panel), BodyProps { panel: *panel }, panel_body).show(ui);
    }

    fn content(&self, panel: &usize) -> Option<u64> {
        self.versions.borrow().get(*panel).copied()
    }

    fn panel_rows(&mut self, _: &usize) -> Vec<PanelRow> {
        vec![PanelRow::Separator]
    }
}

/// A settled dock over `layout` whose panels are [`Versioned`], with the versions and what each pass reported focused.
struct VersionedDock {
    fixture: Fixture,
    versions: Rc<RefCell<Vec<u64>>>,
    workspace: Rc<RefCell<Arc<Workspace<usize>>>>,
    focused: Rc<RefCell<Vec<Option<usize>>>>,
    /// How many docked columns and gesture sources each pass reported.
    reported: Rc<RefCell<Vec<(usize, usize)>>>,
}

fn versioned_dock(layout: Layout<usize>) -> VersionedDock {
    let labels = Tracked::new(Labels((0..32).map(|panel| format!("Panel {panel}")).collect()));
    let versions = Rc::new(RefCell::new(vec![0; 32]));
    let workspace = Rc::new(RefCell::new(Arc::new(Workspace::new(layout))));
    let focused = Rc::new(RefCell::new(Vec::new()));
    let reported = Rc::new(RefCell::new(Vec::new()));
    let mut panels = Versioned { versions: Rc::clone(&versions) };
    let (shown, seen, counts) = (Rc::clone(&workspace), Rc::clone(&focused), Rc::clone(&reported));
    let mut fixture = Fixture::new(move |ui| {
        let output = DockArea::new("Dock", Bind::bound(&mut *shown.borrow_mut()), &labels, &mut panels, DockConfig::default()).show(ui);
        seen.borrow_mut().push(output.focused);
        counts.borrow_mut().push((output.columns.len(), output.sources.len()));
    });
    fixture.room(Vec2::new(1200.0, 600.0)).settle().idle(SETTLE);
    body_runs();
    spec_reads();
    focused.borrow_mut().clear();
    VersionedDock { fixture, versions, workspace, focused, reported }
}

/// The dock's containers: what a settled dock whose panels all have versions runs none of.
const CONTAINERS: [&str; 4] = ["SplitArea", "DockColumn", "DockGroup", "IconicColumn"];

/// At rest a dock whose panels all have versions runs none of its containers, whatever its groups and panels: the
/// docked tree replays as one record, and the floating window's column as its own. The replayed tabs still paint.
#[trial]
fn an_idle_versioned_dock_runs_no_dock_container() {
    let twelve = Layout::new(Some(Node::tabs((0..12).collect(), 0)), Vec::new());
    for layout in [full(1), full(2), full(3), twelve] {
        let VersionedDock { mut fixture, .. } = versioned_dock(layout);
        fixture.frame(Vec::new());
        assert_eq!(runs_of(&fixture, &CONTAINERS), 0, "an idle dock ran containers: {:?}", last_pass_runs(fixture.ctx()));
        assert_eq!(memo_of(&fixture, "TreeView"), (0, 1), "the docked tree replays: {:?}", memo_runs(&fixture));
        assert_eq!(body_runs(), Vec::<usize>::new(), "no body ran");
        assert!(fixture.texts().iter().any(|text| text.text == "Panel 0"), "the replayed tabs paint their titles");
    }
    let VersionedDock { mut fixture, .. } = versioned_dock(full(2));
    fixture.frame(Vec::new());
    assert_eq!(memo_of(&fixture, "ColumnView"), (0, 1), "the floating window's column replays: {:?}", memo_runs(&fixture));
}

/// A new version of one docked column's panel runs that column and its group; the iconic column and the floating
/// window's column replay.
#[trial]
fn a_panel_version_change_runs_only_its_column() {
    let VersionedDock { mut fixture, versions, .. } = versioned_dock(full(2));
    versions.borrow_mut()[20] += 1;
    fixture.frame(Vec::new());
    assert_eq!(memo_of(&fixture, "ColumnView"), (1, 2), "only panel 20's column ran: {:?}", memo_runs(&fixture));
    assert_eq!(runs_of(&fixture, &["IconicColumn"]), 0, "the iconic column replays");
    fixture.frame(Vec::new());
    assert_eq!(runs_of(&fixture, &CONTAINERS), 0, "and the dock settles again: {:?}", last_pass_runs(fixture.ctx()));
}

/// The pointer over a docked column's body runs that column and that body; the other columns and bodies replay. The
/// pointer over a group of the docked tree runs no column.
#[trial]
fn the_pointer_over_one_column_runs_only_that_column() {
    let VersionedDock { mut fixture, .. } = versioned_dock(full(2));
    let tree = fixture.by_label("Body 0").expect("a tree group's body").center();
    fixture.frame(vec![moved(tree)]);
    assert_eq!(memo_of(&fixture, "ColumnView").0, 0, "the pointer over the tree ran a column: {:?}", memo_runs(&fixture));
    body_runs();
    let body = fixture.by_label("Body 20").expect("the column's body").center();
    fixture.frame(vec![moved(body)]);
    assert_eq!(memo_of(&fixture, "ColumnView"), (1, 2), "only the column under the pointer ran: {:?}", memo_runs(&fixture));
    // The tree body the pointer left runs once too; of the columns' bodies (panels 20 to 22) only panel 20's runs.
    let columns: Vec<usize> = body_runs().into_iter().filter(|panel| *panel >= 20).collect();
    assert_eq!(columns, vec![20], "only panel 20's body ran of the columns' bodies");
}

/// A pass that runs the docked tree without changing it builds no derived value (the docks' geometry and the tree's
/// frames included); a column resize builds each of them once.
#[trial]
fn the_docks_geometry_builds_once_per_layout_change() {
    let VersionedDock { mut fixture, workspace, .. } = versioned_dock(full(2));
    let body = fixture.by_label("Body 0").expect("a tree group's body").center();
    fixture.frame(vec![moved(body)]);
    assert!(runs_of(&fixture, &["SplitArea"]) > 0, "the pointer over a tree group runs the tree");
    assert_eq!(last_pass_derived_builds(fixture.ctx()), Vec::new(), "an unchanged layout builds nothing");
    Arc::make_mut(&mut workspace.borrow_mut()).apply(Action::resize_column(20, 300.0), false, |_| Permissions::ALL).expect("the column resizes");
    fixture.frame(Vec::new());
    let builds = last_frame_derived_builds(fixture.ctx());
    assert!(!builds.is_empty() && builds.iter().all(|(_, count)| *count == 1), "the resize builds each value once: {builds:?}");
}

/// Key focus resting in a group of a dock that replays is still reported, pass after pass.
#[trial]
fn a_replayed_group_reports_the_focus_it_holds() {
    let VersionedDock { mut fixture, focused, .. } = versioned_dock(side_by_side(2));
    for _ in 0..4 {
        if focused.borrow().last() == Some(&Some(0)) {
            break;
        }
        fixture.key(egui::Key::Tab, egui::Modifiers::NONE).idle(1);
    }
    assert_eq!(focused.borrow().last(), Some(&Some(0)), "Tab reaches the first group");
    focused.borrow_mut().clear();
    fixture.idle(3);
    assert_eq!(runs_of(&fixture, &CONTAINERS), 0, "the focused dock replays");
    assert!(focused.borrow().iter().all(|panel| *panel == Some(0)), "every replay reports the focus: {:?}", focused.borrow());
}

/// The docked tree and its columns report each column and gesture source once in every pass, and replay what they
/// ran: the replay audit runs the tree in place of each replay, at rest and through new rooms, where the docks' split
/// area measures them before it shows them.
#[trial]
fn a_measured_versioned_dock_reports_each_column_and_source_once() {
    let VersionedDock { mut fixture, reported, .. } = versioned_dock(full(2));
    let settled = *reported.borrow().last().expect("the dock reported");
    assert_eq!(settled.0, 2, "the dock's two columns are placed");
    audit_replays(fixture.ctx(), true);
    reported.borrow_mut().clear();
    fixture.idle(4);
    fixture.room(Vec2::new(1100.0, 600.0)).idle(4).room(Vec2::new(1200.0, 640.0)).idle(4);
    audit_replays(fixture.ctx(), false);
    let reported = reported.borrow();
    assert!(reported.iter().all(|counts| *counts == settled), "every pass reports {settled:?}: {reported:?}");
}
