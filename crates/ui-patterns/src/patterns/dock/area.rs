//! `DockArea`: the docking pattern. It shows a workspace's docked tree, dock columns, flyouts and floating groups from
//! the parts of `docking`, takes the drops of tab and group drags, and applies their actions to the workspace.

use super::hide::{self, PruneKey};
use crate::contract::Revision;
use crate::contract::overlay::{PlacementChange, PlacementKind};
use crate::contract::{
    Bind, BindMut, BodyCx, Cx, Entry, Gap, MemoKey, MemoProps, MemoView, NoValue, Outbox, Outcome, Pad, Pointer as _, Props as _, Show as _,
    ShowContainer as _, Sizing, Tracked, WordSetCx as _,
};
use crate::docking::SplitFramesCx as _;
use crate::docking::stack::{StackSection, StackSections, section_heights};
use crate::docking::tree::FrameRules;
use crate::docking::{Arrangement, CardInset, close_requests, cross_window, end_replaced_drags, float_drop, native, open_requests};
use crate::docking::{
    ColumnArrow, ColumnBar, DividerKind, DockColumn, DockCommands, DockConfig, DockDropTarget, DockDropped, DockEvent, DockGroup, DockPanels,
    DockRoot, DockScene, DockSizes, DockSource, DockSpot, DockTargets, DockTo, DockWords, DragUnit, FloatSpace, FloatingGroup, FlyoutResize, GroupAt,
    GroupDrag, GroupGrip, HidePanels, IconicColumn, IconicMode, IconicSizes, PanelDrag, PanelKey, PanelSpec, PanelSpecs, PanelTitle, Placing,
    SourceKind, SpecSource, SplitArea, Titles, WindowColumn, actions, dock_states, fit_widths, frame, move_actions,
};
use crate::layout_components::Stack;
use crate::menu::{GroupItem, MenuWords, PanelMenu, PanelRow};
use crate::primitives::HostStyle;
use crate::runtime::Shift;
use crate::runtime::commands::{CommandRef, CommandSet as _, CommandState, Commands as _, focus_within};
use crate::runtime::derived::derived;
use crate::runtime::drag;
use crate::runtime::env::{Env, Label, RevealEdge, ShellCx, ShellEntry, in_room};
use crate::runtime::memory::Remember as _;
use crate::runtime::windows::{ResizeEdge, WindowHost};
use crate::sections::{HeaderAction, Panel};
use craft_behavior::dock::{
    Action, Column, DockEdge, DockError, Group, Layout, Location, Node, PanelPlace, Permissions, SplitSize, Workspace, node_at,
};
use craft_behavior::overlay::Side;
use craft_behavior::{CancelReason, DockSide, EditEvent, EditEvents, Flow, GestureId, GesturePhase, Mixed, Proposal};
use craft_host::target::WindowKind;
use egui::{Context, Id, Pos2, Rect, Response, Ui, UiBuilder, Vec2, ViewportId};
use std::cell::{Cell, OnceCell, RefCell};
use std::collections::HashMap;
use std::fmt::Debug;
use std::hash::Hash;
use std::sync::Arc;

#[cfg(feature = "suite")]
pub(crate) mod stories;

/// A layout the dock cannot show: the workspace failed `Workspace::validate`. The area draws nothing for it.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct InvalidGeometry {
    /// Why the model refused the workspace.
    pub error: DockError,
}

/// Files dropped on a floating OS window of the dock: the shown panel of the group under the drop (the first group's for
/// a drop on the title row, or when the pointer was not known), and the files.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct WindowFiles<P> {
    /// The shown panel of the group the files were dropped on.
    pub panel: P,
    /// The files, in the order the OS lists them.
    pub files: Vec<drag::OsFile>,
}

/// What a dock area reports in a pass: the actions it applied to the workspace and the events for the host.
#[derive(Debug)]
#[non_exhaustive]
pub struct DockAreaOutput<P> {
    /// The changes the user made through the area that the workspace took, in order: split divider sizes and a
    /// floating window's move or resize (also in `edit`), then tab picks, closes, moves, drops, floats and menu rows.
    /// An action the workspace refuses (a protected panel, a locked area) is left out. A given workspace is never
    /// written: these are every change of the pass, for the host to apply.
    pub actions: Vec<Action<P>>,
    /// The actions the panel bodies requested through [`request_actions`](crate::docking::request_actions) while the
    /// pass ran, in the order they came. The area applies none of them, to either binding: the host applies them
    /// after `actions`.
    pub requested: Vec<Action<P>>,
    /// Requests for host state and reports of where things ended up.
    pub events: Vec<DockEvent<P>>,
    /// The shown panel of the group that holds key focus: focus is on the group or on a control inside it.
    pub focused: Option<P>,
    /// The shown panel of the group whose body the primary press of this pass went down in, whatever took the press: a
    /// framework control, the group's empty body or the app's own egui widget in a host body
    /// ([`PanelSpec::host_body`](crate::docking::PanelSpec::host_body)), which keeps its presses and focus, so
    /// `focused` does not name its group. A host that keeps a focused panel of its own makes it this one.
    pub pressed: Option<P>,
    /// The panel whose tab the user clicked or picked this pass; a pick of the host's `DockArea::focus` is not the
    /// user's and shows only in `actions`.
    pub picked: Option<P>,
    /// Why nothing was drawn, when the workspace is invalid.
    pub error: Option<InvalidGeometry>,
    /// The edits of the whole workspace this pass, in order: a split divider's size, a floating window's move, and the
    /// applied `actions` as one undo step. A workspace bound by reference takes them itself; a given one needs the host
    /// to apply them.
    pub edit: EditEvents<Arc<Workspace<P>>>,
    /// Where commands float a panel, group or column: the area's float bounds and the sizes of what it drew.
    pub float_space: Option<FloatSpace<P>>,
    /// Where each dock column was placed this pass, named by a panel of its first group, in the area's layer space.
    /// Hosts that anchor their own surfaces to a column read it.
    pub columns: Vec<(P, Rect)>,
    /// The OS windows the floating windows showed in this pass, each named by a panel of the window (the first).
    pub windows: Vec<(P, ViewportId)>,
    /// The files dropped on the area's floating OS windows this pass that no target there took, one entry per drop,
    /// each with the panel it was dropped on, for the host to open.
    pub window_files: Vec<WindowFiles<P>>,
    /// Every place the area drew this pass where a press starts a dock gesture: each tab, each group's drag bar, each
    /// floating window's grip and resize grips, and each divider, with its rect, its OS window and the id of its
    /// accessibility node. Nothing the area did not draw reports: a hidden panel, the groups of a column collapsed to
    /// icons, a minimized window's resize grips.
    pub sources: Vec<DockSource<P>>,
    /// The area's response.
    pub response: Response,
}

impl<P> Outcome for DockAreaOutput<P> {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The docking pattern over a workspace.
///
/// - Shape: the docked tree is a `SplitArea`; a dock's columns are `DockColumn`s, or `IconicColumn`s with their
///   flyouts; a group is a `DockGroup`; a floating window is a `FloatingGroup` holding one column, shown as a dock's
///   column is, whose bar is the window's title row. The host's `DockPanels` supplies the panels.
/// - Edits: a divider drag and every [`Action`] of the pass (menu rows, drags, drops) are written to the workspace
///   through its binding, with the panels' [`SpecSource::permissions`](crate::docking::SpecSource::permissions) (narrowed when `locked`). The output lists the
///   actions the workspace took, divider sizes and window moves included. A given workspace is not written.
/// - Requests: while the pass runs, a body changes the workspace through
///   [`request_actions`](crate::docking::request_actions); the output returns those actions unapplied in `requested`.
///   The host moves key focus to a panel's group with [`focus`](DockArea::focus).
/// - Hiding: [`hide`](DockArea::hide), the shell's panel visibility and [`maximized`](DockArea::maximized) prune
///   what shows; a pruned panel keeps its place for restore.
/// - Drops: the area is the drop target of tab and group drags. Ctrl or Cmd held at the release floats the drop, and
///   so does a release no spot takes, with `DockConfig::float_on_empty_drop`.
/// - State: the open flyout (unless the host binds it with [`flyout`](DockArea::flyout)) lives in the area's root
///   container.
/// - Replacement: a tab or group drag records the workspace generation it started under; when another arrangement
///   replaces the workspace ([`Workspace::replace`]), the drag ends without a drop.
/// - Specs: the panels' specs and permissions come from the host's [`PanelSpecs`] in a `Tracked`. The area reads them
///   only when that revision, the workspace's revision or the lock changed ([`derived`]), and keeps what it read
///   otherwise: an idle pass reads no spec.
#[derive(crate::contract::Pattern, crate::contract::Props)]
#[pattern(probe(DockArea<'static, usize>))]
pub struct DockArea<'a, P: Clone + Eq + Hash + Debug + Send + Sync + 'static> {
    label: Label,
    workspace: Bind<'a, Arc<Workspace<P>>>,
    specs: &'a dyn SpecSource<P>,
    panels: &'a mut dyn DockPanels<P>,
    config: DockConfig,
    /// Which panels the host hides.
    #[prop]
    hide: HidePanels<'a>,
    /// The panel whose frame shows alone.
    #[prop]
    maximized: Option<P>,
    /// Auto-Collapse Iconic Panels: a flyout closes on a press outside it and when the focus leaves it.
    #[prop(flag)]
    auto_collapse: bool,
    /// The workspace is locked: nothing moves, floats or closes.
    #[prop(flag)]
    locked: bool,
    /// The panel open in an iconic column's flyout, owned by the host. Without it the area keeps the value. A host
    /// that opens a panel whose column is collapsed to icons ([`Layout::opens_in_flyout`]), from its Window menu
    /// ([`WindowMenuRowsOutput::flyout`](crate::docking::WindowMenuRowsOutput::flyout)) or a command, writes this
    /// binding: the panel shows in its flyout, and the layout does not change.
    #[prop]
    flyout: Option<Bind<'a, Option<P>>>,
    /// The floating window holding the panel takes over the held primary press in this pass, with the pointer at the
    /// grab point inside the window ([`FloatingGroup::continue_move`]): the window follows the pointer and docks where it
    /// would dock on release. A request of the pass: a host that floats a panel from a rail's `torn_off` keeps it for
    /// the one pass after, when the window shows. Where the dock opens OS windows (`DockConfig::native_windows`), the
    /// window shows in the app window until the press ends, and then opens its OS window where it was released. Without
    /// a held press, when no window holds the panel, or when the window has shown in an OS window, it does nothing.
    #[prop(quantity = "panel and grab point")]
    continue_move: Option<(P, Vec2)>,
    /// Moves key focus to the group that shows the panel, as a press on the group's body does, and picks its tab, as a
    /// click on the tab does (which leaves key focus where it was): the group's stop takes focus in this pass, and the
    /// pick is one of the pass's `actions`. A floating window that holds the panel comes to the front of the windows it
    /// overlaps (`FloatingGroup::raise`); that order is the view's, and the workspace does not keep it. A request of
    /// the pass: a host that focuses a panel itself (it opened something in it) sets it for one pass. A panel the area
    /// does not show in this pass (hidden, pruned by a maximize, in a closed flyout, or in a group the host's
    /// `group_view` draws) takes neither; `DockAreaOutput::focused` tells the host where focus is.
    #[prop]
    focus: Option<P>,
    /// The floating window holding the panel comes to the front in this pass: an OS window gets
    /// `WindowAction::Focus`, an in-app window goes to the top of the floating windows. A request of the pass: a host
    /// that opens or activates a floating panel (its Window menu, a command) passes the panel in the pass after. When
    /// no floating window holds the panel, or the window opens in this pass (a new window opens in front), it does
    /// nothing.
    #[prop]
    raise: Option<P>,
    /// Adds the host's fixed trailing controls to the docked rail containing `panel`. The panel identifies the
    /// column as it moves between dock sides. The host draws the controls through [`DockPanels::rail_footer`]; its
    /// icons scroll above them when the rail is short. Has no effect in flyout mode or on floating columns.
    #[prop]
    rail_footer: Option<P>,
    top_inset: f32,
}

impl<'a, P: Clone + Eq + Hash + Debug + Send + Sync + 'static> DockArea<'a, P> {
    /// A dock area named `label` over `workspace`, with its panels' specs from `specs`, their views from `panels` and
    /// its behaviour from `config`. The root's id comes from the label under the parent's id, and inside an `AppShell`
    /// from the window too, so two windows' docks never share state. Keep one `Arc` across frames and clone the `Arc`,
    /// not the workspace: a new workspace value each frame is a new revision, which reads every spec each frame.
    pub fn new<S: PanelSpecs<P> + 'a>(
        label: impl Into<Label>,
        workspace: Bind<'a, Arc<Workspace<P>>>,
        specs: &'a Tracked<S>,
        panels: &'a mut dyn DockPanels<P>,
        config: DockConfig,
    ) -> Self {
        DockArea {
            label: label.into(),
            workspace,
            specs,
            panels,
            config,
            hide: HidePanels::None,
            maximized: None,
            auto_collapse: false,
            locked: false,
            flyout: None,
            continue_move: None,
            focus: None,
            raise: None,
            rail_footer: None,
            top_inset: 0.0,
        }
    }

    /// A dock area that only shows `workspace`: given, and locked, so nothing moves, floats or closes. It keys what it
    /// derives on the workspace's revision, so an idle pass reads no spec.
    pub fn read_only<S: PanelSpecs<P> + 'a>(
        label: impl Into<Label>,
        workspace: &Arc<Workspace<P>>,
        specs: &'a Tracked<S>,
        panels: &'a mut dyn DockPanels<P>,
        config: DockConfig,
    ) -> Self {
        DockArea { locked: true, ..Self::new(label, Bind::given(Arc::clone(workspace)), specs, panels, config) }
    }

    /// Keeps floating windows below the top `inset` points of the window: the height of the bars (notices) the
    /// windows float over. A window's place is clamped to the float bounds, so one at the top of the area sits just
    /// below the bars and moves with them as they show and close. A negative or non-finite inset is none.
    #[must_use]
    // design: allow[props/point-size](the inset is measured content, the height of the app's bars, not a size the area chooses)
    pub fn top_inset(mut self, inset: f32) -> Self {
        self.top_inset = if inset.is_finite() { inset.max(0.0) } else { 0.0 };
        self
    }
}

/// What the area keeps in its root container.
#[derive(Clone)]
struct AreaMemory<P> {
    /// The open flyout when the host binds none.
    open: Option<P>,
    /// The running resize or keyboard move of a floating window.
    moving: Option<Moving<P>>,
    /// The least window id above every id the workspaces this area showed held: an undo that brings back an older
    /// workspace keeps its new windows above it ([`Workspace::raise_window_floor`]).
    window_floor: u64,
}

/// A floating window's move or resize as one workspace edit: its gesture and the workspace when it began.
#[derive(Clone)]
struct Moving<P> {
    id: GestureId,
    before: Arc<Workspace<P>>,
}

impl<P> Default for AreaMemory<P> {
    fn default() -> Self {
        AreaMemory { open: None, moving: None, window_floor: 0 }
    }
}

/// What the root's drop taker reads: the dock, the layout it shows, the rights, where a floating drop lands and what
/// the host hides.
struct Drops<'a, P> {
    dock: Id,
    layout: &'a Layout<P>,
    permissions: &'a dyn Fn(&P) -> Permissions,
    config: &'a DockConfig,
    specs: &'a HashMap<P, PanelSpec>,
    unnamed: &'a PanelSpec,
    hide: HidePanels<'a>,
    maximized: Option<&'a P>,
    rtl: bool,
    top_inset: f32,
}

impl<P: Clone + Eq + Hash + Debug + Send + Sync + 'static> Drops<'_, P> {
    /// Takes a group or tab drag released on the root.
    fn take(&self, cx: &mut Cx<'_, DockRoot<'_, DockDropped<P>>>) -> Option<DockDropped<P>> {
        let ctx = cx.core.ctx.clone();
        let sizes = area_sizes(*cx.style.sizes(), self.config);
        let shell = cx.env.get::<ShellEntry>().0;
        let bounds = inset_top(shell.as_ref().map_or(cx.rect, ShellCx::float_bounds), self.top_inset);
        let space = float_space(&ctx, bounds, &sizes, self.config, self.specs);
        let spec = |panel: &P| spec_in(self.specs, self.unnamed, panel).clone();
        let protected = |panel: &P| (self.permissions)(panel).is_protected();
        let prune = hide::Prune::new(self.layout, self.hide, shell.as_ref());
        let mut target = DockDropTarget::new(&ctx, self.dock, self.layout, self.permissions);
        target.rtl = self.rtl;
        target.space = Some(&space);
        target.float_on_empty = self.config.float_on_empty_drop;
        target.title_bar_docks = self.config.title_bar_docks;
        target.placing = Some(Placing::new(sizes, &spec, &protected).shown(prune.hide(), self.maximized));
        cx.drop_target::<GroupDrag<P>, _>(&target).or_else(|| cx.drop_target::<PanelDrag, _>(&target))
    }
}

/// The sizes a dock area lays out with: the dock's style, with its docked columns as cards when `config` shows them
/// ([`DockConfig::column_cards`]) and no gap beside a flush dock ([`DockConfig::flush_docks`]).
fn area_sizes(mut sizes: DockSizes, config: &DockConfig) -> DockSizes {
    sizes.column_cards = config.column_cards;
    if config.flush_docks {
        sizes.dock_gap = 0.0;
    }
    sizes
}

/// What the area collects while it shows its parts.
struct Report<P> {
    actions: Vec<Action<P>>,
    events: Vec<DockEvent<P>>,
    pressed: Option<P>,
    picked: Option<P>,
    flyout: Vec<EditEvent<Option<P>>>,
    placed: Vec<Placed<P>>,
    /// The OS windows the floating windows showed in, by their first panel.
    windows: Vec<(P, ViewportId)>,
    /// The files dropped on the floating windows that no target there took.
    files: Vec<WindowFiles<P>>,
    /// The gesture sources the parts drew, in the order they showed.
    sources: Vec<DockSource<P>>,
    /// The groups shown this pass, in the order they showed.
    shown: Vec<ShownGroup<P>>,
    /// The tab each `DockGroup` picked this pass (a click, or the host's focus request), by the group's id.
    picks: Vec<(Id, P)>,
    /// The divider edits of the docked tree this pass.
    split: EditEvents<Layout<P>>,
}

/// A group the area showed: its shown panel, its rect and, where it is a `DockGroup` (not a host's view), its id.
#[derive(Clone, Debug, PartialEq)]
struct ShownGroup<P> {
    panel: P,
    rect: Rect,
    id: Option<Id>,
}

impl<P: Clone> Shift for ShownGroup<P> {
    fn shifted(&self, by: &egui::RecordShift) -> Self {
        ShownGroup { rect: self.rect.shifted(by), ..self.clone() }
    }

    fn snapped(&self, to: &Self, within: f32) -> Self {
        ShownGroup { rect: self.rect.snapped(&to.rect, within), ..self.clone() }
    }
}

/// Where a dock column was placed, named by a panel of its first group.
#[derive(Clone, Debug, PartialEq)]
struct Placed<P> {
    panel: P,
    rect: Rect,
}

impl<P: Clone> Shift for Placed<P> {
    fn shifted(&self, by: &egui::RecordShift) -> Self {
        Placed { rect: self.rect.shifted(by), ..self.clone() }
    }

    fn snapped(&self, to: &Self, within: f32) -> Self {
        Placed { rect: self.rect.snapped(&to.rect, within), ..self.clone() }
    }
}

/// What a column or the docked tree drew in its run that the area reports in every pass it shows: the gesture
/// sources, the groups and the placed dock columns. A view's replay returns its run's, moved with the record.
#[derive(Clone, Debug, PartialEq)]
struct Drawn<P> {
    sources: Vec<DockSource<P>>,
    shown: Vec<ShownGroup<P>>,
    placed: Vec<Placed<P>>,
}

impl<P: Clone> Shift for Drawn<P> {
    fn shifted(&self, by: &egui::RecordShift) -> Self {
        Drawn { sources: self.sources.shifted(by), shown: self.shown.shifted(by), placed: self.placed.shifted(by) }
    }

    fn snapped(&self, to: &Self, within: f32) -> Self {
        Drawn {
            sources: self.sources.snapped(&to.sources, within),
            shown: self.shown.snapped(&to.shown, within),
            placed: self.placed.snapped(&to.placed, within),
        }
    }
}

/// How far the report had grown when a view's run began ([`Pass::drawn_since`]).
#[derive(Clone, Copy)]
struct Mark {
    sources: usize,
    shown: usize,
    placed: usize,
}

/// The inputs of the workspace's derived values: the workspace, the panels' specs and the lock.
#[derive(Clone, Copy, crate::contract::MemoProps)]
pub(super) struct ModelKey {
    workspace: Revision,
    specs: Revision,
    locked: bool,
}

/// What every part of one pass reads.
struct Scene<'s, P> {
    dock: Id,
    /// The key the pruned layout was derived under: what every group's derived values follow.
    view: u128,
    sizes: DockSizes,
    config: &'s DockConfig,
    layout: &'s Layout<P>,
    specs: &'s HashMap<P, PanelSpec>,
    /// The spec of a panel the specs do not name.
    unnamed: &'s PanelSpec,
    permissions: &'s HashMap<P, Permissions>,
    keys: &'s HashMap<PanelKey, P>,
    space: &'s FloatSpace<P>,
    bounds: Rect,
    auto_collapse: bool,
    /// The panel whose window takes over the held press, with its grab point (`DockArea::continue_move`).
    continue_move: Option<(&'s P, Vec2)>,
    /// The panel whose group takes key focus in this pass (`DockArea::focus`).
    focus: Option<&'s P>,
    /// The panel whose window comes to the front (`DockArea::raise`).
    raise: Option<&'s P>,
    rtl: bool,
    /// The docked places floating panels left, for Dock Panel.
    floated: &'s [(P, Location<P>)],
    rail_footer: Option<&'s P>,
    /// The style of the `Ui` the area was shown in, which the bodies of `PanelSpec::host_body` panels lay out with.
    host_style: &'s HostStyle,
}

/// The spec of `panel` in `specs`, or `unnamed`.
fn spec_in<'s, P: Eq + Hash>(specs: &'s HashMap<P, PanelSpec>, unnamed: &'s PanelSpec, panel: &P) -> &'s PanelSpec {
    specs.get(panel).unwrap_or(unnamed)
}

/// What an area derives from its workspace, its panels' specs and its lock, kept while none of them changes: whether
/// the workspace is valid, and for each panel in its layout the spec, the permissions (narrowed when locked) and the
/// key.
struct DockModel<P> {
    error: Option<InvalidGeometry>,
    specs: HashMap<P, PanelSpec>,
    /// The spec of a panel `specs` does not name.
    unnamed: PanelSpec,
    rights: HashMap<P, Permissions>,
    keys: HashMap<PanelKey, P>,
}

impl<P: Clone + Eq + Hash> DockModel<P> {
    fn of(workspace: &Workspace<P>, specs: &dyn SpecSource<P>, locked: bool) -> Self {
        let error = workspace.validate().err().map(|error| InvalidGeometry { error });
        let panels = if error.is_none() { workspace.layout().panels() } else { Vec::new() };
        let rights = panels
            .iter()
            .map(|panel| {
                let rights = specs.permissions(panel);
                ((*panel).clone(), if locked { rights.locked() } else { rights })
            })
            .collect();
        DockModel {
            error,
            specs: panels.iter().map(|panel| ((*panel).clone(), specs.spec(panel))).collect(),
            unnamed: PanelSpec::new(Label::default()),
            rights,
            keys: panels.iter().map(|panel| (PanelKey::of(*panel), (*panel).clone())).collect(),
        }
    }
}

/// The parts of a group that show the host's code (`Scene::hosted`).
#[derive(Clone, Copy, Debug, Hash)]
enum HostPart {
    Body,
    Footer,
    Header,
    RailFooter,
    View,
}

impl<'s, P: Clone + Eq + Hash> Scene<'s, P> {
    /// Runs `run`, which shows host code of `panel` as its `part`, on `ui`. Where the panel's body is the app's own egui
    /// code (`PanelSpec::host_body`), it runs with the style of the `Ui` the area was shown in, and only in the pass's
    /// real run: a measuring run takes what its last real run took (`HostStyle::show`).
    fn hosted<R: Clone + Default + Send + Sync + 'static>(&self, panel: &P, part: HostPart, ui: &mut Ui, run: impl FnOnce(&mut Ui) -> R) -> R {
        if !self.host_body(panel) {
            return run(ui);
        }
        self.host_style.show(ui, self.dock.with(("host", part, PanelKey::of(panel))), run)
    }

    /// Runs `run`, which measures `panel`'s host view at a width (`GroupAt::Measure`), on `ui`: with the style of the
    /// `Ui` the area was shown in where the panel's body is the app's own egui code.
    fn hosted_measure<R>(&self, panel: &P, ui: &mut Ui, run: impl FnOnce(&mut Ui) -> R) -> R {
        if !self.host_body(panel) {
            return run(ui);
        }
        self.host_style.run(ui, run)
    }

    fn host_body(&self, panel: &P) -> bool {
        self.specs.get(panel).is_some_and(|spec| spec.host_body)
    }

    fn spec(&self, panel: &P) -> &'s PanelSpec {
        spec_in(self.specs, self.unnamed, panel)
    }

    fn permissions(&self, panel: &P) -> Permissions {
        self.permissions.get(panel).copied().unwrap_or(Permissions::ALL)
    }

    fn panel(&self, key: PanelKey) -> Option<&P> {
        self.keys.get(&key)
    }

    /// Whether the docked `column` shows a rail of its icons beside it ([`IconicMode::Rail`]): an expanded column
    /// that collapses to icons.
    fn rail_beside(&self, column: &Column<P>) -> bool {
        let spec = |panel: &P| self.spec(panel).clone();
        let collapses = column.panels().all(|panel| self.permissions(panel).collapses);
        self.config.iconic_mode == IconicMode::Rail
            && !column.iconic
            && ColumnArrow::of(column, &self.sizes, &spec, collapses) == ColumnArrow::Collapse
    }

    /// The `(width, floor)` of each docked column of a dock, for [`fit_widths`]: a rail beside an expanded column adds
    /// its width to both.
    fn natural(&self, columns: &[Column<P>]) -> Vec<(f32, Option<f32>)> {
        let spec = |panel: &P| self.spec(panel).clone();
        let natural = self.sizes.natural(columns, &spec);
        columns
            .iter()
            .zip(natural)
            .map(|(column, (width, floor))| {
                if !self.rail_beside(column) {
                    return (width, floor);
                }
                let rail = self.sizes.iconic_width(column, &spec);
                (width + rail, floor.map(|floor| floor + rail))
            })
            .collect()
    }
}

/// What the docks' geometry and the docked tree's frames follow besides their room: the pruned layout (with the specs
/// and the lock), the sizes and the config.
#[derive(crate::contract::MemoProps)]
struct GeometryKey<'k> {
    view: u128,
    sizes: DockSizes,
    #[memo_props(hash = "the config derives `Hash` over every field")]
    config: &'k DockConfig,
}

/// The docks' columns as they lay out before the room shrinks them, derived per [`GeometryKey`]: for each dock, by its
/// window edge, each column's `(width, floor)` ([`Scene::natural`]) and its card's inset ([`DockSizes::card_place`]).
struct DockGeometry {
    docks: Vec<(DockEdge, Vec<ColumnExtent>, Vec<Option<CardInset>>)>,
}

/// A dock column's `(width, floor)` before the room shrinks it ([`Scene::natural`]).
type ColumnExtent = (f32, Option<f32>);

impl DockGeometry {
    fn of<P: Clone + Eq + Hash>(scene: &Scene<'_, P>) -> Self {
        let spec = |panel: &P| scene.spec(panel).clone();
        let docks = [DockEdge::Start, DockEdge::End]
            .into_iter()
            .filter_map(|edge| {
                let path = scene.layout.dock_path(edge)?;
                let Some(Node::Dock { columns, .. }) = node_at(scene.layout.root.as_ref()?, &path) else { return None };
                let cards = columns.iter().enumerate().map(|(at, column)| scene.sizes.card_place(at, column, &spec)).collect();
                Some((edge, scene.natural(columns), cards))
            })
            .collect();
        DockGeometry { docks }
    }

    /// The `(width, floor)` and card place of each column of the dock at `edge`.
    fn dock(&self, edge: DockEdge) -> (&[ColumnExtent], &[Option<CardInset>]) {
        self.docks.iter().find(|(at, _, _)| *at == edge).map_or((&[], &[]), |(_, natural, cards)| (natural.as_slice(), cards.as_slice()))
    }
}

/// What a pass shows the docked tree with: the room, the docks' name, the open flyout, the shell and the docks'
/// geometry.
struct TreeAt<P> {
    room: Rect,
    label: Label,
    open: Option<P>,
    shell: Option<ShellCx>,
    geometry: Arc<DockGeometry>,
    /// The revision of the tree's frames: the layout, the specs, the sizes and the config ([`GeometryKey`]).
    frames: u128,
}

/// The docked tree as a memoized view: it replays while its key holds and no input reaches it.
#[derive(MemoProps)]
struct TreeView<'v, 's, P> {
    /// [`Pass::tree_key`].
    key: u128,
    #[memo_props(hash = "a dock's panels are plain values it keys by their hash")]
    rail_footer: Option<P>,
    #[memo_props(
        covered_by = "key",
        reason = "the key holds the arrangement, the specs, the requests and the versions of every panel the tree shows"
    )]
    pass: &'v mut Pass<'s, P>,
    #[memo_props(covered_by = "key", reason = "the key holds the room, the name, the open flyout, the shell's reveal and the geometry's key")]
    at: TreeAt<P>,
}

fn tree_view<P: Clone + Eq + Hash + Debug + Send + Sync + 'static>(view: &mut TreeView<'_, '_, P>, ui: &mut Ui) -> Drawn<P> {
    let mark = view.pass.mark();
    view.pass.show_tree(ui, &view.at);
    view.pass.drawn_since(mark)
}

/// A column as a memoized view: it replays while its key holds and no input reaches it.
#[derive(MemoProps)]
struct ColumnView<'v, 's, P> {
    /// [`Pass::column_key`].
    key: u128,
    #[memo_props(hash = "a dock's panels are plain values it keys by their hash")]
    first: P,
    #[memo_props(
        covered_by = "key",
        reason = "the key holds the arrangement, the specs, the requests and the versions of the panels the column shows"
    )]
    pass: &'v mut Pass<'s, P>,
    #[memo_props(covered_by = "key", reason = "the arrangement in the key holds the column")]
    column: &'v Column<P>,
    #[memo_props(covered_by = "key", reason = "the key holds the column's place")]
    at: ColumnAt<P>,
    name: Label,
}

fn column_view<P: Clone + Eq + Hash + Debug + Send + Sync + 'static>(view: &mut ColumnView<'_, '_, P>, ui: &mut Ui) -> Drawn<P> {
    let mark = view.pass.mark();
    let cell = view.at.cell;
    view.pass.column(ui, &view.name, view.column, view.at.clone());
    // The column takes no room in flow; the run covers its cell, so the pointer over it runs it again.
    ui.expand_to_include_rect(cell);
    view.pass.drawn_since(mark)
}

/// What the area reports from one pass.
struct Passed<P> {
    /// The divider sizes and window moves of `edit`, as actions on the workspace the pass started with.
    geometry: Vec<Action<P>>,
    /// The pass's other actions: as `show_area` proposes them, then the ones the workspace took.
    actions: Vec<Action<P>>,
    events: Vec<DockEvent<P>>,
    focused: Option<P>,
    pressed: Option<P>,
    picked: Option<P>,
    edit: EditEvents<Arc<Workspace<P>>>,
    space: Option<FloatSpace<P>>,
    columns: Vec<(P, Rect)>,
    windows: Vec<(P, ViewportId)>,
    files: Vec<WindowFiles<P>>,
    sources: Vec<DockSource<P>>,
}

/// How a group is shown: minimized, whether it is an accordion section (with its open state), and whether it is in
/// a floating window, where nothing goes beside it.
#[derive(Clone, Copy)]
struct Look {
    minimized: bool,
    section: Option<bool>,
    in_window: bool,
}

/// What a group's header shows besides its panels' bodies: the tab titles, the panel menu's rows and whether the host
/// gave any, and the panel commands' states.
struct GroupParts {
    titles: Titles,
    hosted: bool,
    states: Vec<(DockCommands, CommandState)>,
}

/// A dock to show: its window edge, its columns and its rect.
struct DockPlan<'p, P> {
    edge: DockEdge,
    columns: &'p [Column<P>],
    rect: Rect,
    flow: Flow,
    open: Option<P>,
    /// Each column's `(width, floor)` and card place ([`DockGeometry::dock`]).
    natural: &'p [(f32, Option<f32>)],
    cards: &'p [Option<CardInset>],
}

/// Where and how a column shows: its rect; the edge it keeps to, a dock's window edge or the float area's edge a
/// floating window keeps to; where its bar is; the rect its inner edge and flyouts stay within; and the open flyout.
/// Its flyouts open away from its edge: towards the canvas for a dock column, towards the float area's room for a
/// floating one.
#[derive(Clone)]
struct ColumnAt<P> {
    cell: Rect,
    edge: DockEdge,
    bar: ColumnBar,
    within: Rect,
    open: Option<P>,
    /// Which outer inset the column's card takes, a docked column's in a dock that shows cards ([`DockSizes::card_place`]).
    card: Option<CardInset>,
}

/// What a column reported.
struct ColumnWork<P> {
    iconic: Option<bool>,
    picked: Option<GroupItem>,
    resized: Vec<(P, f32)>,
    /// The expanded width the user gave an expanded column, or the iconic width of an iconic one.
    width: Option<f32>,
    /// The size the user gave an iconic column's open flyout.
    flyout: Option<FlyoutResize<P>>,
}

/// The parts of one pass with the host's panels and what they reported.
struct Pass<'s, P> {
    scene: &'s Scene<'s, P>,
    panels: &'s mut dyn DockPanels<P>,
    report: Report<P>,
    /// The dock whose registry the groups shown now register their drop spots in: the area's, or a floating window's
    /// own, which takes the drops over that window on the window's layer.
    dock: Id,
}

impl<'a, P: Clone + Eq + Hash + Debug + Send + Sync + 'static> DockArea<'a, P> {
    /// Shows the area across the room offered.
    pub fn show(self, ui: &mut Ui) -> DockAreaOutput<P> {
        let DockArea {
            label,
            mut workspace,
            specs,
            panels,
            config,
            hide,
            maximized,
            auto_collapse,
            locked,
            mut flyout,
            continue_move,
            focus,
            raise,
            rail_footer,
            top_inset,
        } = self;
        let ctx = ui.ctx().clone();
        let host_style = HostStyle::of(ui);
        let (rtl, window) = {
            let env = Env::of(ui);
            (env.direction.is_rtl(), env.get::<ShellEntry>().0.map(|shell| shell.window().clone()))
        };
        let id = ui.scope_id().with(("dock area", label.as_str(), window));
        let held = match workspace.reborrow().get() {
            Some(Mixed::Same(held)) => held,
            _ => Arc::default(),
        };
        let key = ModelKey { workspace: held.revision(), specs: specs.revision(), locked };
        let model = derived(&ctx, id, &key, || DockModel::of(&held, specs, locked));
        let error = model.error.clone();
        let current = if error.is_none() { held } else { Arc::default() };
        let rights = &model.rights;
        let policy = |panel: &P| rights.get(panel).copied().unwrap_or(Permissions::ALL);
        let drops = Drops {
            dock: id,
            layout: current.layout(),
            permissions: &policy,
            config: &config,
            specs: &model.specs,
            unnamed: &model.unnamed,
            hide,
            maximized: maximized.as_ref(),
            rtl,
            top_inset,
        };
        let shows = Arrangement { dock: id, generation: current.generation() };
        end_replaced_drags::<P>(&ctx, shows);
        let dropped: RefCell<Option<Action<P>>> = RefCell::new(None);
        let root = DockRoot::new(|cx| {
            let taken = drops.take(cx);
            *dropped.borrow_mut() = taken.as_ref().map(|taken| taken.action.clone());
            taken
        })
        .id(id)
        .label(label.clone())
        .shows(shows);
        let mut passed = None;
        open_requests::<P>(&ctx);
        let shown = root.show(ui, |ui, body| {
            let Some(sizes) = body.style::<DockRoot<'_, DockDropped<P>>>().map(|style| area_sizes(*style.sizes(), &config)) else { return };
            let memory = body.memory::<AreaMemory<P>>();
            memory.window_floor = memory.window_floor.max(current.layout().window_floor());
            let input = Input {
                id,
                ctx: &ctx,
                current: &current,
                config: &config,
                specs: &model.specs,
                unnamed: &model.unnamed,
                keys: &model.keys,
                hide,
                model: key,
                maximized: maximized.as_ref(),
                auto_collapse,
                continue_move: continue_move.as_ref().map(|(panel, grab)| (panel, *grab)),
                focus: focus.as_ref(),
                raise: raise.as_ref(),
                sizes,
                rtl,
                rail_footer: rail_footer.as_ref(),
                top_inset,
                host_style: &host_style,
            };
            let mut result = show_area(ui, body, &input, rights, &mut *panels, flyout.as_mut());
            for edit in &result.edit {
                workspace.reborrow().write(body, edit);
            }
            result.actions.extend(dropped.take());
            if !result.actions.is_empty() {
                let proposed = std::mem::take(&mut result.actions);
                let applied = apply_actions(body, workspace.reborrow(), &current, specs, locked, &label, proposed);
                result.actions = applied.actions;
                result.edit.extend(applied.edit);
            }
            passed = Some(result);
        });
        let requested = close_requests::<P>(&ctx);
        let Passed { geometry, actions, events, focused, pressed, picked, edit, space, columns, windows, files, sources } =
            passed.unwrap_or_else(|| Passed {
                geometry: Vec::new(),
                actions: Vec::new(),
                events: Vec::new(),
                focused: None,
                pressed: None,
                picked: None,
                edit: EditEvents::new(),
                space: None,
                columns: Vec::new(),
                windows: Vec::new(),
                files: Vec::new(),
                sources: Vec::new(),
            });
        if shown.output.dropped.is_some() {
            cross_window::accept_cross_window(&ctx);
        }
        DockAreaOutput {
            // The edits were written before the actions were judged, so they come first.
            actions: geometry.into_iter().chain(actions).collect(),
            requested,
            events,
            focused,
            pressed,
            picked,
            error,
            edit,
            float_space: space,
            columns,
            windows,
            window_files: files,
            sources,
            response: shown.output.response,
        }
    }
}

/// What the workspace took of a pass's actions.
struct Applied<P> {
    actions: Vec<Action<P>>,
    edit: Option<EditEvent<Arc<Workspace<P>>>>,
}

/// Applies `proposed` in order to the workspace the binding holds, with the panels' permissions from `specs` (narrowed
/// when `locked`; an action may open a panel the layout does not hold). A refused action changes nothing and is left
/// out of the result. The accepted ones are written as one edit, which a given workspace does not take. `current` is
/// the workspace as the pass started. New windows take ids above every id the area has shown (`AreaMemory::window_floor`).
fn apply_actions<P: Clone + Eq + Hash + Debug + Send + Sync + 'static>(
    body: &mut BodyCx<'_>,
    mut workspace: BindMut<'_, Arc<Workspace<P>>>,
    current: &Arc<Workspace<P>>,
    specs: &dyn SpecSource<P>,
    locked: bool,
    label: &Label,
    proposed: Vec<Action<P>>,
) -> Applied<P> {
    let before = match workspace.read(body) {
        Some(Mixed::Same(held)) => held,
        _ => Arc::clone(current),
    };
    let permissions = |panel: &P| specs.permissions(panel);
    let mut next = Workspace::clone(&before);
    next.raise_window_floor(body.memory::<AreaMemory<P>>().window_floor);
    let mut actions = Vec::new();
    for action in proposed {
        if next.apply(action.clone(), locked, permissions).is_ok() {
            actions.push(action);
        }
    }
    if actions.is_empty() {
        return Applied { actions, edit: None };
    }
    let edit = EditEvent::set(Mixed::Same(before), Arc::new(next), Arc::from(label.as_str()), None);
    workspace.write(body, &edit);
    Applied { actions, edit: Some(edit) }
}

/// What the pattern gives one pass.
struct Input<'i, P> {
    id: Id,
    ctx: &'i Context,
    current: &'i Arc<Workspace<P>>,
    config: &'i DockConfig,
    specs: &'i HashMap<P, PanelSpec>,
    unnamed: &'i PanelSpec,
    keys: &'i HashMap<PanelKey, P>,
    hide: HidePanels<'i>,
    model: ModelKey,
    maximized: Option<&'i P>,
    auto_collapse: bool,
    continue_move: Option<(&'i P, Vec2)>,
    focus: Option<&'i P>,
    raise: Option<&'i P>,
    sizes: DockSizes,
    rtl: bool,
    rail_footer: Option<&'i P>,
    top_inset: f32,
    host_style: &'i HostStyle,
}

/// `bounds` without its top `inset` points, never past its bottom.
fn inset_top(bounds: Rect, inset: f32) -> Rect {
    let top = (bounds.min.y + inset).min(bounds.max.y);
    Rect::from_min_max(egui::pos2(bounds.min.x, top), bounds.max)
}

fn open_of<P: Clone + Send + Sync + 'static + PartialEq + Debug>(
    flyout: &mut Option<&mut Bind<'_, Option<P>>>,
    body: &mut BodyCx<'_>,
    memory: &AreaMemory<P>,
) -> Option<P> {
    match flyout.as_mut() {
        Some(bind) => match bind.reborrow().read(body) {
            Some(Mixed::Same(open)) => open,
            _ => None,
        },
        None => memory.open.clone(),
    }
}

fn show_area<P: Clone + Eq + Hash + Debug + Send + Sync + 'static>(
    ui: &mut Ui,
    body: &mut BodyCx<'_>,
    input: &Input<'_, P>,
    rights: &HashMap<P, Permissions>,
    panels: &mut dyn DockPanels<P>,
    mut flyout: Option<&mut Bind<'_, Option<P>>>,
) -> Passed<P> {
    let Input {
        id,
        ctx,
        current,
        config,
        specs,
        unnamed,
        keys,
        hide,
        model,
        maximized,
        auto_collapse,
        continue_move,
        focus,
        raise,
        sizes,
        rtl,
        rail_footer,
        top_inset,
        host_style,
    } = *input;
    let shell = body.shell();
    let mut memory = body.memory::<AreaMemory<P>>().clone();
    let protected = |panel: &P| rights.get(panel).is_some_and(|rights| rights.is_protected());
    let prune = PruneKey::new(model, hide, shell.as_ref(), maximized.map(PanelKey::of));
    let view = derived(ctx, id.with("pruned"), &prune, || hide::shown(current.layout(), maximized, hide, shell.as_ref(), &protected).into_owned());
    let layout: &Layout<P> = &view.layout;
    let bounds = inset_top(shell.as_ref().map_or(body.rect, ShellCx::float_bounds), top_inset);
    let space = float_space(ctx, bounds, &sizes, config, specs);
    let scene = Scene {
        dock: id,
        view: MemoKey::of(&prune),
        sizes,
        config,
        layout,
        specs,
        unnamed,
        permissions: rights,
        keys,
        space: &space,
        bounds,
        auto_collapse,
        continue_move,
        focus,
        raise,
        rtl,
        floated: current.floated(),
        rail_footer,
        host_style,
    };
    let open = open_of(&mut flyout, body, &memory);
    let mut pass = Pass {
        scene: &scene,
        panels,
        report: Report {
            actions: Vec::new(),
            events: Vec::new(),
            pressed: None,
            picked: None,
            flyout: Vec::new(),
            placed: Vec::new(),
            windows: Vec::new(),
            files: Vec::new(),
            sources: Vec::new(),
            shown: Vec::new(),
            picks: Vec::new(),
            split: EditEvents::new(),
        },
        dock: id,
    };

    let key = GeometryKey { view: scene.view, sizes, config };
    let docks = derived(ctx, id.with("geometry"), &key, || DockGeometry::of(&scene));
    let label = Label::from(body.words::<DockWords>().panels.as_str().to_owned());
    let tree = TreeAt { room: body.rect, label, open: open.clone(), shell: shell.clone(), geometry: docks, frames: MemoKey::of(&key) };
    pass.tree(ui, tree);
    let split = std::mem::take(&mut pass.report.split);
    let mut edit = split.map(|edited| {
        let mut layout = current.layout().clone();
        view.write_back(&edited, &mut layout);
        let resized = match (current.layout().root.as_ref(), layout.root.as_ref()) {
            (Some(from), Some(to)) => split_changes(from, to).into_iter().map(|(path, size)| Action::resize_split(path, size)).collect(),
            _ => Vec::new(),
        };
        let mut next = Workspace::clone(current);
        // A divider drag only resizes splits, which no permission or lock refuses.
        let _ = next.apply_all(resized, false, |_| Permissions::ALL);
        Arc::new(next)
    });
    let mut geometry: Vec<Action<P>> = match (current.layout().root.as_ref(), edit.landed().and_then(|landed| landed.layout().root.as_ref())) {
        (Some(from), Some(to)) => split_changes(from, to).into_iter().map(|(path, size)| Action::resize_split(path, size)).collect(),
        _ => Vec::new(),
    };
    let started = memory.moving.as_ref().map(|moving| moving.before.layout());
    let moved = if prune.floating { pass.windows(ui, body, &open, started) } else { None };
    let mut report = pass.report;
    let label: Arc<str> = Arc::from(body.words::<DockWords>().move_window.as_str());
    if let Some((anchor, change, edge)) = moved
        && let Some(event) = window_edit(
            ctx,
            current,
            rights,
            &mut memory.moving,
            change.phase,
            Action::move_floating_with_edge(anchor.clone(), change.rect, edge),
            label,
        )
    {
        let place_in = |layout: &Layout<P>| layout.floating.iter().find(|window| window.contains(&anchor)).map(|window| (window.rect, window.edge));
        if let Some((rect, edge)) =
            event.landed().and_then(|landed| place_in(landed.layout())).filter(|place| Some(*place) != place_in(current.layout()))
        {
            geometry.push(Action::move_floating_with_edge(anchor.clone(), rect, edge));
        }
        edit.push(event);
    }

    let mut shown = open;
    for event in report.flyout.drain(..) {
        match flyout.as_mut() {
            Some(bind) => {
                bind.reborrow().write(body, &event);
            }
            None => {
                if let Some(value) = event.value() {
                    memory.open = value.clone();
                }
            }
        }
        if let Some(value) = event.value().filter(|value| **value != shown) {
            report.events.push(DockEvent::FlyoutChanged { panel: value.clone() });
            shown = value.clone();
        }
    }
    *body.memory::<AreaMemory<P>>() = memory;
    // The group holding key focus, by the panel it shows, or the one it picked in this pass: a replayed group reports
    // focus as a run does.
    let focused = report
        .shown
        .iter()
        .filter_map(|group| {
            let id = group.id.filter(|id| focus_within(ctx, *id))?;
            Some(report.picks.iter().find(|(picked, _)| *picked == id).map_or(&group.panel, |(_, panel)| panel).clone())
        })
        .last();
    Passed {
        geometry,
        actions: report.actions,
        events: report.events,
        focused,
        pressed: report.pressed,
        picked: report.picked,
        edit,
        space: Some(space),
        columns: report.placed.into_iter().map(|placed| (placed.panel, placed.rect)).collect(),
        windows: report.windows,
        files: report.files,
        sources: report.sources,
    }
}

/// Registers the outer-edge bands and the new-dock bands as the dock's drop spots for the next pass, from the leaves
/// the split area placed in this run (each as its tree path and rect). A new-dock band runs along a start or end edge
/// that holds no dock.
fn register_bands<P: Clone + Eq + Hash + Send + Sync + Debug + 'static>(
    scene: &Scene<'_, P>,
    ctx: &Context,
    area: Rect,
    leaves: &[(Vec<bool>, Rect)],
) {
    if scene.layout.root.is_none() {
        return;
    }
    let canvas = |panel: &P| scene.spec(panel).canvas;
    let bands = frame::edge_bands(scene.layout, leaves, &canvas, scene.config, &scene.sizes, scene.rtl);
    frame::register_edge_bands::<P>(ctx, scene.dock, &bands, scene.sizes.strip_reach);
    if scene.config.new_dock_zones {
        let flow = Flow::new(area, scene.rtl);
        let band = scene.sizes.new_dock_band;
        let start = Rect::from_min_size(area.min, Vec2::new(band, area.height()));
        let end = Rect::from_min_max(Pos2::new(area.right() - band, area.top()), area.max);
        let edge_line = |x: f32| Rect::from_x_y_ranges(x..=x, area.y_range());
        let lines = [edge_line(area.left()), edge_line(area.right())];
        for ((side, edge, rect), line) in [(DockSide::Start, DockEdge::Start, start), (DockSide::End, DockEdge::End, end)].into_iter().zip(lines) {
            if scene.layout.dock_path(edge).is_none() {
                let (rect, line) = (flow.to_physical(rect), flow.to_physical(line));
                DockTargets::<P>::register_band(ctx, scene.dock, DockSpot::NewDock(side), rect, line, scene.sizes.strip_reach);
            }
        }
    }
}

/// Where floats land in an area whose float bounds are `bounds`: each panel at its spec's float size, else the
/// default float size of `sizes`, under the chrome of the windows the dock opens floats in.
fn float_space<P: Clone + PartialEq>(
    ctx: &Context,
    bounds: Rect,
    sizes: &DockSizes,
    config: &DockConfig,
    specs: &HashMap<P, PanelSpec>,
) -> FloatSpace<P> {
    let mut space =
        FloatSpace::new(bounds, sizes).in_os_windows(sizes, native::opens_os_windows(ctx, config)).with_side_edge_zones(config.side_edge_zones);
    for (panel, spec) in specs {
        if let Some(size) = spec.float_size {
            space = space.with_panel_size(panel.clone(), size);
        }
    }
    space
}

/// The rows of the menu of `panel`'s group, and whether the host gave any (only then does the group show a menu
/// button; a context request opens the menu either way): the host's rows, then the panel commands the config shows, with
/// the Move Panel To submenu of the groups the panel can move to.
fn panel_menu_rows<P: Clone + Eq + Hash>(
    scene: &Scene<'_, P>,
    panels: &mut dyn DockPanels<P>,
    words: &MenuWords,
    panel: &P,
) -> (Vec<PanelRow>, bool) {
    let config = scene.config;
    let command = |command: DockCommands| PanelRow::Command(CommandRef::from(command));
    let floating = matches!(scene.layout.place(panel), Some(PanelPlace::Floating { .. }));
    let mut group = vec![command(DockCommands::FloatPanel)];
    if floating {
        group.push(command(DockCommands::DockPanel));
    }
    let targets = scene.layout.move_targets(panel, |other| scene.permissions(other));
    if scene.permissions(panel).movable && !targets.is_empty() {
        let rows = targets.iter().enumerate().map(|(at, target)| PanelRow::Item(Entry::new(GroupItem::MoveTo(at), scene.spec(target).label.clone())));
        let name = Label::from(words.panel_move_to.as_str().to_owned());
        group.push(PanelRow::Submenu(name, rows.collect()));
    }
    if config.close_menu_items {
        group.extend([command(DockCommands::ClosePanel), command(DockCommands::CloseGroup)]);
    }
    if config.group_order_items && matches!(scene.layout.place(panel), Some(PanelPlace::Dock { .. })) {
        group.extend([command(DockCommands::ToggleMinimized), command(DockCommands::StepGroupUp), command(DockCommands::StepGroupDown)]);
    }
    group.push(command(DockCommands::FloatGroup));
    if config.close_others_item {
        group.push(command(DockCommands::CloseOthers));
    }
    if config.maximize_item {
        group.push(command(DockCommands::ToggleMaximize));
    }
    let mut rows = panels.panel_rows(panel);
    let hosted = !rows.is_empty();
    if hosted {
        rows.push(PanelRow::Separator);
    }
    rows.extend(group);
    (rows, hosted)
}

/// Whether `spec` is a document's: its group shows its body edge to edge. Every other docked body fills its group's
/// body, inset from its edges by the panel padding: the group is the body's panel and its tab the body's title, so a
/// docked body pads and frames nothing itself.
fn document(spec: &PanelSpec) -> bool {
    matches!(spec.window_kind, WindowKind::Document)
}

impl<P: Clone + Eq + Hash + Debug + Send + Sync + 'static> Pass<'_, P> {
    /// One group of the docked tree or of a floating window: a tab group, or the sections of an accordion.
    fn leaf(&mut self, ui: &mut Ui, body: &mut BodyCx<'_>, node: &Node<P>, rect: Rect) {
        let scene = self.scene;
        match node {
            Node::Tabs { panels: members, active, .. } => {
                self.group(ui, body, members, *active, Look { minimized: false, section: None, in_window: false }, GroupAt::Show(rect));
            }
            Node::Stack { entries, .. } => {
                let sections: Vec<StackSection> = entries
                    .iter()
                    .map(|entry| {
                        let limits = scene.spec(&entry.panel).limits;
                        StackSection::new(entry.open).height(entry.height).limits(limits.min.y, limits.max.map_or(f32::INFINITY, |max| max.y))
                    })
                    .collect();
                let heights = section_heights(&sections, rect.height(), scene.sizes.strip);
                let shown = StackSections::new(sections).show_at(ui, rect, |ui, inner| {
                    let mut top = rect.top();
                    for (entry, height) in entries.iter().zip(&heights) {
                        let section = Rect::from_min_size(Pos2::new(rect.left(), top), Vec2::new(rect.width(), *height));
                        top += height;
                        let look = Look { minimized: !entry.open, section: Some(entry.open), in_window: false };
                        self.group(ui, inner, std::slice::from_ref(&entry.panel), 0, look, GroupAt::Show(section));
                    }
                });
                for (at, height) in shown.output.resized {
                    if let Some(entry) = entries.get(at) {
                        self.report.actions.push(Action::resize_stack(entry.panel.clone(), Some(height)));
                    }
                }
            }
            _ => {}
        }
    }

    /// The rows of the menu of `panel`'s group, and whether the host gave any ([`panel_menu_rows`]).
    fn menu_rows(&mut self, body: &BodyCx<'_>, panel: &P) -> (Vec<PanelRow>, bool) {
        panel_menu_rows(self.scene, &mut *self.panels, &body.words::<MenuWords>(), panel)
    }

    /// Runs a panel command on `panel`: the event of a command the host carries out, else its actions. `source` is
    /// the size of the group the command runs from, which a float of it takes.
    fn run(&mut self, panel: &P, command: DockCommands, source: Option<Vec2>) {
        let scene = self.scene;
        let permissions = |panel: &P| scene.permissions(panel);
        // The area's permissions are narrowed for a locked workspace already.
        let mut commands = DockScene::new(scene.layout, scene.space, false, &permissions).floated(scene.floated);
        if let Some(size) = source {
            commands = commands.source(size);
        }
        let report = &mut self.report;
        if let Some(event) = command.event(panel) {
            report.events.push(event);
        } else if command.moves(panel).is_some() {
            report.events.push(DockEvent::PanelCommand { panel: panel.clone(), command });
        } else if let Ok(planned) = actions(&commands, command, panel) {
            report.actions.extend(planned);
        }
    }

    /// Moves `panel` into the group of the move target at `at`, as Move Panel To.
    fn move_to(&mut self, panel: &P, at: usize) {
        let scene = self.scene;
        let permissions = |panel: &P| scene.permissions(panel);
        let Some(target) = scene.layout.move_targets(panel, permissions).get(at).map(|target| (*target).clone()) else { return };
        let commands = DockScene::new(scene.layout, scene.space, false, &permissions).floated(scene.floated);
        if let Ok(planned) = move_actions(&commands, &DragUnit::Panel(panel.clone()), DockTo::tabs(target, None)) {
            self.report.actions.extend(planned);
        }
    }

    /// Runs a dock row of a panel or column menu that is not a command row.
    fn pick(&mut self, panel: &P, item: GroupItem) {
        match item {
            GroupItem::MoveTo(at) => self.move_to(panel, at),
            GroupItem::CollapseToIcons => self.run(panel, DockCommands::CollapseToIcons, None),
            GroupItem::AutoCollapse => self.run(panel, DockCommands::AutoCollapse, None),
        }
    }

    /// The grip strip on top of a tool rail the host shows itself: the dock's grip marks over a band that drags the
    /// group, which floats at `size`, the group's size. A memoized view ([`rail_grip`]) held to the strip, so a frame
    /// whose input is elsewhere replays it, as the column bars and the groups' tab rows replay. Returns the grip's id.
    fn rail_grip(&mut self, ui: &mut Ui, anchor: &P, shown: &P, strip: Rect, size: Vec2) -> Option<Id> {
        let scene = self.scene;
        let label = Label::from(scene.spec(shown).label.as_str().to_owned());
        let mut sent = Vec::new();
        let grip = ui.scope_builder(UiBuilder::new().max_rect(strip), |ui| {
            let props = RailGrip {
                unit: DragUnit::Group(anchor.clone()),
                active: PanelKey::of(shown),
                movable: scene.permissions(shown).movable,
                label,
                sent: Outbox::new(&mut sent),
            };
            // Keyed by the rail's group: unique among the rails a dock area shows.
            MemoView::new(("rail grip", PanelKey::of(anchor)), props, rail_grip::<P>).show(ui)
        });
        // The group's size goes on the drag here, outside the view, whose measuring runs see no size; so does the
        // press, which a floating rail's window keeps under the pointer when it is dropped on empty room.
        if let Some(&press) = sent.first() {
            drag::amend::<GroupDrag<P>>(ui.ctx(), |drag| {
                drag.source = Some(size);
                drag.press = Some(press);
            });
        }
        grip.inner
    }

    /// One group, by the host's `group_view` when it takes the group, else as a `DockGroup` over the host's panels:
    /// shown in a rect, or measured at a width the way it would show, returning its height.
    fn group(&mut self, ui: &mut Ui, body: &mut BodyCx<'_>, members: &[P], active: usize, look: Look, at: GroupAt) -> Option<f32> {
        let scene = self.scene;
        let anchor = members.first()?;
        let active = active.min(members.len() - 1);
        // A tool rail (a panel with a grid) that the host shows itself gets the dock's grip strip on top, under its
        // column bar, as the references' toolbar: the host's view takes the room under it.
        let grip = if members.get(active).is_some_and(|panel| scene.spec(panel).grid.is_some()) { scene.sizes.grip } else { 0.0 };
        if let GroupAt::Measure(_) = at
            && let Some(height) = scene.hosted_measure(&members[active], ui, |ui| self.panels.group_view(members, active, ui, at, body))
        {
            return Some(height + grip);
        }
        let shown = &members[active];
        // The host's focus request, when it names a panel of this group.
        let focus = scene.focus.filter(|panel| members.contains(panel));
        let words = body.words::<MenuWords>();
        let ctx = ui.ctx().clone();
        let panels = RefCell::new(&mut *self.panels);
        // The tab titles, the menu rows and the panel commands' states, built when the group runs.
        // The tab titles, whether the host gives menu rows and the panel commands' states, built once per pass; the
        // menu's rows are built only while it is open.
        let parts: OnceCell<GroupParts> = OnceCell::new();
        let parts_of = || {
            parts.get_or_init(|| {
                let key = (scene.view, PanelKey::of(anchor));
                // Keyed by their content once built, so a group's tab row replays unless its own titles changed.
                let titles = derived(&ctx, scene.dock.with(("titles", key.1)), &key, || {
                    Titles::from_content(
                        members
                            .iter()
                            .map(|panel| {
                                let spec = scene.spec(panel);
                                PanelTitle::new(PanelKey::of(panel), spec.label.clone())
                                    .closable(scene.permissions(panel).closable)
                                    .command(spec.command.clone())
                                    .count(spec.count)
                                    .caption(spec.caption.clone())
                                    .swatch(spec.swatch)
                                    .documents(spec.documents.clone())
                            })
                            .collect(),
                    )
                });
                let titles = Titles::clone(&titles);
                let hosted = !panels.borrow_mut().panel_rows(shown).is_empty();
                let permissions = |panel: &P| scene.permissions(panel);
                let states = dock_states(&DockScene::new(scene.layout, scene.space, false, &permissions).floated(scene.floated), shown);
                GroupParts { titles, hosted, states }
            })
        };
        // The group with `members[at]` shown: as it shows, or as it is measured.
        let make = |at: usize, minimized: bool| {
            let panels = &panels;
            let words = &words;
            let GroupParts { titles, hosted, states } = parts_of();
            let mut group = DockGroup::new(anchor.clone(), titles.clone(), PanelKey::of(&members[at]))
                .extra(move |ui, body| scene.hosted(&members[at], HostPart::Header, ui, |ui| panels.borrow_mut().group_header(members, at, ui, body)))
                .dock(self.dock)
                .zones(scene.config.zone_rule())
                .movable(scene.permissions(shown).movable)
                .menu(move || panel_menu_rows(scene, &mut **panels.borrow_mut(), words, shown).0)
                .panel_menu_shown(*hosted || scene.config.panel_menu_always)
                .panel_menu_icon(scene.config.panel_menu_icon.clone())
                .tab_look(scene.config.tab_look)
                .command_states(states.clone());
            if look.in_window {
                group = group.in_window();
            }
            let spec = scene.spec(&members[at]);
            if minimized {
                group = group.minimized();
            } else if !document(spec) && !spec.host_body {
                // The app's own egui body lays out in the whole body, as it did outside the dock.
                group = group.pad(Pad::Regular);
            }
            if spec.host_body {
                group = group.host_body();
            }
            if spec.footer {
                group = group.footer(move |ui, body| {
                    scene.hosted(&members[at], HostPart::Footer, ui, |ui| panels.borrow_mut().footer(&members[at], ui, body))
                });
            }
            if scene.config.tab_close_buttons {
                group = group.closable();
            }
            if scene.config.titled_headers {
                group = group.titled();
            }
            if scene.config.wheel_selects_tabs {
                group = group.wheel_selects_tabs();
            }
            group
        };
        let rect = match at {
            GroupAt::Measure(width) => {
                let measured =
                    make(active, look.minimized).measure_in(ui, Proposal::Width(width), |ui, body| panel_body(scene, &panels, shown, ui, body));
                return Some(measured.y);
            }
            GroupAt::Show(rect) => rect,
        };
        // A measuring run of the container around the group draws it too; the group reports what it drew from the
        // pass's real run only, once.
        let reports = crate::runtime::measure::effect(ui.ctx(), "dock::group_report");
        let at_shown = self.report.shown.len();
        if reports {
            self.report.shown.push(ShownGroup { panel: shown.clone(), rect, id: None });
        }
        // One child `Ui` held to the group's rect. It takes the parent's next id, so a change of the group's first tab
        // keeps every id inside. The host's view runs first, held to the group's rect and clipped to it: it lays out in
        // the group's room, never in the room the area has left. Where the host leaves the group to the dock, the
        // group shows in its place. Both run every pass; the host memoizes inside its view, the group inside its
        // header and its bodies.
        let pressed = Cell::new(false);
        let (strip, view) = if grip > 0.0 { rect.split_top_bottom_at_y(rect.top() + grip.min(rect.height())) } else { (Rect::NOTHING, rect) };
        let output = ui
            .scope_builder(UiBuilder::new().max_rect(rect), |ui| {
                // The view is a layout room of its own: a view that fills takes the whole group.
                let taken = in_room(ui, view, |ui| {
                    scene.hosted(&members[active], HostPart::View, ui, |ui| {
                        panels.borrow_mut().group_view(members, active, ui, GroupAt::Show(view), body)
                    })
                });
                match taken {
                    Some(_) => None,
                    None => {
                        let group = make(active, look.minimized);
                        let group = if focus.is_some() { group.take_focus() } else { group };
                        Some(
                            group
                                .show_at(ui, rect, |ui, body| {
                                    pressed.set(pressed.get() || body.pressed_within(ui));
                                    panel_body(scene, &panels, shown, ui, body);
                                })
                                .output,
                        )
                    }
                }
            })
            .inner;
        // A host view took the group: a rail's grip drags the group, as the band of a group's header does.
        let Some(output) = output else {
            if strip.is_positive()
                && let Some(node) = self.rail_grip(ui, anchor, shown, strip, rect.size())
                && reports
            {
                self.report.sources.push(DockSource::new(SourceKind::GroupBar(anchor.clone()), strip, node));
            }
            return None;
        };
        // A tab key is a panel's or one of its documents' (`Titles::document_of`).
        let kind_of = |key: PanelKey| match scene.panel(key) {
            Some(panel) => Some(SourceKind::Tab(panel.clone())),
            None => {
                let (panel, document) = parts_of().titles.document_of(key)?;
                Some(SourceKind::Document(scene.panel(panel)?.clone(), document))
            }
        };
        let tabs = output.tabs.iter().filter_map(|(key, rect, node)| Some(DockSource::new(kind_of(*key)?, *rect, *node)));
        if reports {
            self.report.sources.extend(tabs);
            if let Some((rect, node)) = output.overflow {
                self.report.sources.push(DockSource::new(SourceKind::Overflow(anchor.clone()), rect, node));
            }
            if let Some((rect, node)) = output.grip {
                self.report.sources.push(DockSource::new(SourceKind::GroupBar(anchor.clone()), rect, node));
            }
        }
        // The size `panel` floats at out of this group: the group's width, and the height the group measures at with
        // `panel` shown in a floating window's room, so content that fills the body takes that room.
        let fitted = |ui: &Ui, panel: &P| {
            let at = members.iter().position(|member| member == panel).unwrap_or(active);
            let room = Vec2::new(rect.width(), scene.space.content_room());
            let measured = make(at, false).id(output.response.id).measure_in(ui, Proposal::Size(room), |ui, body| {
                panel_body(scene, &panels, &members[at], ui, body);
            });
            Vec2::new(rect.width(), measured.y)
        };
        let floats = output.commands.iter().any(|command| matches!(command, DockCommands::FloatPanel | DockCommands::FloatGroup));
        let source = if floats { fitted(ui, shown) } else { rect.size() };
        if output.drag_started {
            let ctx = ui.ctx().clone();
            let tab = drag::active::<PanelDrag>(&ctx).and_then(|drag| scene.panel(drag.panel).cloned());
            let size = fitted(ui, tab.as_ref().unwrap_or(shown));
            drag::amend::<PanelDrag>(&ctx, |drag| drag.source = Some(size));
            drag::amend::<GroupDrag<P>>(&ctx, |drag| drag.source = Some(size));
        }
        // A tear-off inside the window is a release no spot took: it floats only with `float_on_empty_drop`.
        let torn =
            output.torn_off.as_ref().and_then(|torn| scene.panel(torn.grabbed).map(|panel| (panel, torn.pointer))).filter(|(panel, pointer)| {
                scene.permissions(panel).movable
                    && scene.permissions(panel).floatable
                    && (scene.config.float_on_empty_drop || !ui.ctx().content_rect().contains(*pointer))
            });
        let torn = torn.map(|(panel, pointer)| {
            float_drop(&DragUnit::Panel(panel.clone()), scene.space, pointer, Some(fitted(ui, panel)), None, scene.rtl).action
        });
        // A document's tab shows its panel, as the panel's own tab does, and names the document to the host.
        let document = output.picked_document.and_then(|(key, document)| Some((scene.panel(key)?, document)));
        let clicked = output.picked.and_then(|key| scene.panel(key)).or(document.map(|(panel, _)| panel));
        if let Some(panel) = clicked {
            self.report.picked = Some(panel.clone());
        }
        // The host's focus request picks its panel's tab as a click does; the user's click of the pass wins.
        let picked = clicked.or(focus.filter(|panel| *panel != shown));
        if let Some(panel) = picked {
            self.report.actions.push(Action::activate(panel.clone()));
        }
        if let Some((panel, document)) = document {
            self.report.events.push(DockEvent::DocumentPicked { panel: panel.clone(), document });
        }
        for (panel, document) in output.closed_documents.into_iter().filter_map(|(key, document)| Some((scene.panel(key)?, document))) {
            self.report.events.push(DockEvent::DocumentClosed { panel: panel.clone(), document });
        }
        if pressed.get() {
            self.report.pressed = Some(shown.clone());
        }
        // A tab picked in this pass is the group's shown panel from now on, though this pass drew the one before.
        if reports {
            self.report.shown[at_shown].id = Some(output.response.id);
        }
        if let Some(panel) = picked {
            self.report.picks.push((output.response.id, panel.clone()));
        }
        for panel in output.closed.into_iter().filter_map(|key| scene.panel(key)) {
            self.report.actions.push(Action::close(panel.clone()));
        }
        if let Some(item) = output.item {
            self.pick(shown, item);
        }
        for command in output.commands {
            self.run(shown, command, Some(source));
        }
        if output.toggle_minimized {
            self.report.actions.push(match look.section {
                Some(open) => Action::set_stack_open(shown.clone(), !open),
                None => Action::set_minimized(shown.clone(), !look.minimized),
            });
        }
        self.report.actions.extend(torn);
        None
    }

    /// A group of a column, docked or in a floating window (`in_window`).
    fn group_of(&mut self, ui: &mut Ui, body: &mut BodyCx<'_>, group: &Group<P>, at: GroupAt, in_window: bool) -> Option<f32> {
        let look = Look { minimized: group.minimized, section: None, in_window };
        self.group(ui, body, &group.panels, group.active, look, at)
    }

    /// The columns of a dock, from the window edge inwards.
    fn dock(&mut self, ui: &mut Ui, body: &mut BodyCx<'_>, plan: DockPlan<'_, P>) {
        let widths = fit_widths(plan.natural, plan.rect.width());
        let name = Label::from(body.words::<DockWords>().panels.as_str().to_owned());
        let reading = plan.flow.to_physical(plan.rect);
        let start = plan.edge == DockEdge::Start;
        let mut cursor = if start { reading.left() } else { reading.right() };
        for (at, (column, width)) in plan.columns.iter().zip(widths).enumerate() {
            let span = if start { (cursor, cursor + width) } else { (cursor - width, cursor) };
            cursor = if start { span.1 } else { span.0 };
            let cell = plan.flow.to_physical(Rect::from_x_y_ranges(span.0..=span.1, reading.y_range()));
            let Some(first) = column.groups.first().and_then(|group| group.panels.first()) else { continue };
            // A docked column takes the dock's height, a stepped one (a tool rail) too: the references' docked
            // toolbar keeps its column's surface down to the dock's foot, with the tools at the top
            // (photocraft#16461). Content taller than the dock scrolls inside. Only a floating stepped window is as
            // tall as its content. A measuring run of the container around the dock places it too; the dock reports
            // its columns from the pass's real run only, once.
            if crate::runtime::measure::effect(ui.ctx(), "dock::placed_report") {
                self.report.placed.push(Placed { panel: first.clone(), rect: cell });
            }
            let card = plan.cards.get(at).copied().flatten();
            let place = ("docked", plan.edge, at);
            let at = ColumnAt { cell, edge: plan.edge, bar: ColumnBar::Own, within: body.rect, open: plan.open.clone(), card };
            self.shown_column(ui, &name, column, at, place);
        }
    }

    /// Shows `column` at `at`: as a memoized view keyed by [`Pass::column_key`] where every panel it shows has a
    /// version, else run in this pass. `place` names the column among the area's: its dock edge and index, or its
    /// window, so the ids inside stay while its tabs move.
    fn shown_column(&mut self, ui: &mut Ui, name: &Label, column: &Column<P>, at: ColumnAt<P>, place: impl Hash + Debug) {
        let (Some(key), Some(first)) = (self.column_key(column, &at), column.panels().next().cloned()) else {
            self.column(ui, name, column, at);
            return;
        };
        let id = self.dock.with(("column", place));
        let cell = at.cell;
        let view = ColumnView { key, first, pass: &mut *self, column, at, name: name.clone() };
        // The view's room is the column's cell, so the pointer over the cell, and nowhere else, runs it.
        let drawn = ui.scope_builder(UiBuilder::new().max_rect(cell), |ui| MemoView::new(id, view, column_view::<P>).show(ui)).inner;
        if let Some(drawn) = drawn {
            self.add_drawn(ui, drawn);
        }
    }

    /// The docked tree, with its docks: as a memoized view keyed by [`Pass::tree_key`] where every panel it shows has
    /// a version, else run in this pass.
    fn tree(&mut self, ui: &mut Ui, at: TreeAt<P>) {
        let Some(key) = self.tree_key(&at) else {
            self.show_tree(ui, &at);
            return;
        };
        let id = self.dock.with("docked tree");
        let rail_footer = self.scene.rail_footer.cloned();
        let view = TreeView { key, rail_footer, pass: &mut *self, at };
        if let Some(drawn) = MemoView::new(id, view, tree_view::<P>).show(ui) {
            self.add_drawn(ui, drawn);
        }
    }

    /// The docked tree in a `SplitArea`: each group at its frame, each dock's columns, and the drop bands along the
    /// area's edges.
    fn show_tree(&mut self, ui: &mut Ui, at: &TreeAt<P>) {
        let scene = self.scene;
        let ctx = ui.ctx().clone();
        let limits = |panel: &P| scene.spec(panel).limits;
        let dock_width = |columns: &[Column<P>]| scene.natural(columns).iter().map(|(width, _)| *width).sum::<f32>();
        let stripless = |panel: &P| scene.spec(panel).canvas;
        let rules = FrameRules::new(&limits).dock_width(&dock_width).stripless(&stripless).dock_gap(scene.sizes.dock_gap);
        let area = SplitArea::new(at.label.clone(), Bind::given(scene.layout.clone()), rules, at.frames).show(ui, |ui, split| {
            DockTargets::<P>::register_room(&ctx, scene.dock, split.rect);
            // The leaves of this run only: a measuring run of the area lays them out too, and registers nothing.
            let mut leaves: Vec<(Vec<bool>, Rect)> = Vec::new();
            for (anchor, rect) in split.group_frames::<P>() {
                let Some(place) = scene.layout.place(&anchor) else { continue };
                let Some(node) = scene.layout.node_of(&place) else { continue };
                if let PanelPlace::Node(path) = &place {
                    leaves.push((path.clone(), rect));
                }
                self.leaf(ui, split, node, rect);
            }
            let flow = Flow::new(split.rect, scene.rtl);
            for (edge, rect) in split.dock_frames() {
                let Some(path) = scene.layout.dock_path(edge) else { continue };
                let Some(Node::Dock { columns, .. }) = scene.layout.root.as_ref().and_then(|root| node_at(root, &path)) else { continue };
                leaves.push((path.clone(), rect));
                if let Some(shell) = at.shell.as_ref().filter(|shell| shell.revealed()) {
                    shell.reveal_rect(&ctx, if edge == DockEdge::Start { RevealEdge::Start } else { RevealEdge::End }, rect);
                }
                let (natural, cards) = at.geometry.dock(edge);
                self.dock(ui, split, DockPlan { edge, columns, rect, flow, open: at.open.clone(), natural, cards });
            }
            register_bands(scene, &ctx, at.room, &leaves);
        });
        let splits = area
            .output
            .dividers
            .iter()
            .map(|(path, rect, node)| DockSource::new(SourceKind::Divider(DividerKind::Split(path.clone())), *rect, *node));
        self.report.sources.extend(splits);
        self.report.split = area.output.edit.events;
    }

    /// A key fed with what every column and the docked tree read of the area: the pruned layout, the dock their spots
    /// register in, the float bounds, the direction, the look and the host's requests.
    fn area_key(&self, open: Option<&P>) -> MemoKey {
        let scene = self.scene;
        let mut key = MemoKey::new();
        scene.view.key(&mut key);
        self.dock.key(&mut key);
        scene.bounds.key(&mut key);
        scene.rtl.key(&mut key);
        scene.auto_collapse.key(&mut key);
        key.feed_hash(scene.config);
        scene.focus.map(PanelKey::of).key(&mut key);
        scene.rail_footer.map(PanelKey::of).key(&mut key);
        open.map(PanelKey::of).key(&mut key);
        key
    }

    /// The key of `column` shown at `at` ([`Pass::area_key`], its place and the versions of the panels it shows), or
    /// `None` where one of those panels has no version ([`DockPanels::content`]).
    fn column_key(&self, column: &Column<P>, at: &ColumnAt<P>) -> Option<u128> {
        let mut key = self.area_key(at.open.as_ref());
        at.cell.key(&mut key);
        at.within.key(&mut key);
        key.feed_hash(&(at.edge, at.bar, at.card));
        self.feed_versions(&mut key, column_shows(column, [at.open.as_ref(), self.scene.rail_footer]))?;
        Some(key.finish())
    }

    /// The key of the docked tree shown at `at` ([`Pass::area_key`], its room, name, the shell's reveal, its frames'
    /// revision and the versions of every panel it shows), or `None` where one of those panels has no version.
    fn tree_key(&self, at: &TreeAt<P>) -> Option<u128> {
        let mut key = self.area_key(at.open.as_ref());
        at.room.key(&mut key);
        at.label.key(&mut key);
        at.shell.as_ref().map(ShellCx::revealed).key(&mut key);
        at.frames.key(&mut key);
        if let Some(root) = self.scene.layout.root.as_ref() {
            self.feed_node(&mut key, root, at.open.as_ref())?;
        }
        Some(key.finish())
    }

    /// Feeds the versions of the panels `node` shows: each group's shown panel, each section and each dock column's
    /// ([`column_shows`]). `None` where one has none.
    fn feed_node(&self, key: &mut MemoKey, node: &Node<P>, open: Option<&P>) -> Option<()> {
        match node {
            Node::Split { first, second, .. } => {
                self.feed_node(key, first, open)?;
                self.feed_node(key, second, open)
            }
            Node::Tabs { panels, active, .. } => self.feed_versions(key, panels.get((*active).min(panels.len().saturating_sub(1))).into_iter()),
            Node::Stack { entries, .. } => self.feed_versions(key, entries.iter().map(|entry| &entry.panel)),
            Node::Dock { columns, .. } => {
                for column in columns {
                    self.feed_versions(key, column_shows(column, [open, self.scene.rail_footer]))?;
                }
                Some(())
            }
            _ => None,
        }
    }

    /// Feeds the host's version of each of `panels` ([`DockPanels::content`]); `None` where one has none.
    fn feed_versions<'p>(&self, key: &mut MemoKey, panels: impl Iterator<Item = &'p P>) -> Option<()>
    where
        P: 'p,
    {
        for panel in panels {
            self.panels.content(panel)?.key(key);
        }
        Some(())
    }

    /// How far the report has grown, for [`Pass::drawn_since`].
    fn mark(&self) -> Mark {
        Mark { sources: self.report.sources.len(), shown: self.report.shown.len(), placed: self.report.placed.len() }
    }

    /// What the report gained since `mark`, taken out of it: what a view's run drew.
    fn drawn_since(&mut self, mark: Mark) -> Drawn<P> {
        Drawn {
            sources: self.report.sources.split_off(mark.sources),
            shown: self.report.shown.split_off(mark.shown),
            placed: self.report.placed.split_off(mark.placed),
        }
    }

    /// Adds what a view drew, by its run or its replay, to the report: in the pass's real run only. A measuring run's
    /// replay returns the kept real run's report, which the real run adds once.
    fn add_drawn(&mut self, ui: &Ui, drawn: Drawn<P>) {
        if !crate::runtime::measure::effect(ui.ctx(), "dock::drawn_report") {
            return;
        }
        self.report.sources.extend(drawn.sources);
        self.report.shown.extend(drawn.shown);
        self.report.placed.extend(drawn.placed);
    }

    /// The steps a resize `change` of the floating window of the stepped `column`, whose first panel is `first`, sets,
    /// and the window's rect at them: as wide as its steps and as tall as its chrome and its content at that width.
    /// `fitted` is the content's size under the chrome before the change, `chrome` the height above the window's body
    /// (`DockSizes::window_chrome`). A grip on the top or bottom edge sets the
    /// steps by height (the fewest whose content fits), any other grip by width (the most that fit); keyboard Size
    /// mode, which names no grip, by the side it changed. The window keeps its place: a grip on the left keeps the
    /// right edge where it is and one on the top the bottom edge, so the window grows or shrinks that way. Reports
    /// the steps when they change. `None` for a column without steps.
    fn stepped_resize(
        &mut self,
        ui: &mut Ui,
        body: &mut BodyCx<'_>,
        column: &Column<P>,
        first: &P,
        change: &PlacementChange,
        (fitted, chrome): (Vec2, f32),
    ) -> Option<Rect> {
        let scene = self.scene;
        let spec_of = |panel: &P| scene.spec(panel).clone();
        let (grid, steps) = scene.sizes.stepped(column, &spec_of)?;
        let by_height = match change.kind {
            PlacementKind::Size(grip) => matches!(grip, ResizeEdge::Top | ResizeEdge::Bottom),
            _ => (change.rect.width() - fitted.x).abs() <= 0.5,
        };
        let to = if by_height {
            let room = change.rect.height() - chrome;
            grid.steps_for_height(room, |width| self.stepped_height(ui, body, column, width, true).unwrap_or(0.0))
        } else {
            grid.steps_within(change.rect.width())
        };
        if to != steps {
            self.report.actions.push(Action::set_column_steps(first.clone(), to));
        }
        let size = Vec2::new(grid.width(to), chrome + self.stepped_height(ui, body, column, grid.width(to), true).unwrap_or(0.0));
        let left = match change.kind {
            PlacementKind::Size(ResizeEdge::Left | ResizeEdge::TopLeft | ResizeEdge::BottomLeft) => change.rect.right() - size.x,
            _ => change.rect.left(),
        };
        let top = match change.kind {
            PlacementKind::Size(ResizeEdge::Top | ResizeEdge::TopLeft | ResizeEdge::TopRight) => change.rect.bottom() - size.y,
            _ => change.rect.top(),
        };
        Some(Rect::from_min_size(Pos2::new(left, top), size))
    }

    /// The height of the groups of an expanded stepped column ([`stepped`]) measured at `width`, the width of its
    /// steps: what the column shows under its bar. `None` for any other column.
    fn stepped_height(&mut self, ui: &mut Ui, body: &mut BodyCx<'_>, column: &Column<P>, width: f32, in_window: bool) -> Option<f32> {
        let scene = self.scene;
        let spec = |panel: &P| scene.spec(panel).clone();
        if column.iconic || scene.sizes.stepped(column, &spec).is_none() {
            return None;
        }
        let mut height = 0.0;
        for group in &column.groups {
            height += self.group_of(ui, body, group, GroupAt::Measure(width), in_window).unwrap_or(0.0);
        }
        Some(height)
    }

    /// What the double arrow of `column` does ([`ColumnArrow::of`]): it names the arrow and decides what a click asks
    /// for.
    fn arrow_of(&self, column: &Column<P>) -> ColumnArrow {
        let scene = self.scene;
        let spec = |panel: &P| scene.spec(panel).clone();
        ColumnArrow::of(column, &scene.sizes, &spec, column.panels().all(|panel| scene.permissions(panel).collapses))
    }

    /// What a click on the double arrow of `column`, whose first panel is `first`, asks for when it reports
    /// `iconic`: the steps it switches to, or the collapse or expansion of the column; nothing for a column with no
    /// double arrow, whose bar's double click reports the same.
    fn arrow(&self, column: &Column<P>, first: &P, iconic: bool) -> Option<Action<P>> {
        match self.arrow_of(column) {
            ColumnArrow::Fixed => None,
            arrow => Some(match arrow.switched_steps() {
                Some(steps) => Action::set_column_steps(first.clone(), steps),
                None => Action::set_iconic(first.clone(), iconic),
            }),
        }
    }

    /// One column, docked or floating: its icons and their flyouts while it is collapsed, else its stacked groups,
    /// each group shown by `group_of`. What the user did on it becomes actions.
    fn column(&mut self, ui: &mut Ui, name: &Label, column: &Column<P>, at: ColumnAt<P>) {
        let scene = self.scene;
        let spec = |panel: &P| scene.spec(panel).clone();
        let dock = self.dock;
        let ColumnAt { mut cell, edge, bar, within, open, card } = at;
        let in_window = bar == ColumnBar::Window;
        if in_window {
            DockTargets::<P>::register_room(ui.ctx(), dock, cell);
        }
        let opens = if edge == DockEdge::Start { Side::End } else { Side::Start };
        // Only a docked column's icons are a rail; a floating window's open flyouts.
        let mode = if in_window { IconicMode::Flyouts } else { scene.config.iconic_mode };
        // The width the user gives a column by its inner edge leaves out its card's inset.
        let inset = card.map_or(0.0, |place| scene.sizes.inset(place));
        let mut icons = |cell: Rect, open: Option<P>, card: Option<CardInset>| {
            let footer = scene.rail_footer.filter(|panel| mode == IconicMode::Rail && column.panels().any(|item| item == *panel));
            let pass = RefCell::new(&mut *self);
            let draw = |ui: &mut Ui, rect: Rect, open: Option<P>| {
                let shown = IconicColumn::new(name.clone(), dock, column, Bind::given(open), &spec)
                    .edge(edge)
                    .opens(opens)
                    .bar(bar)
                    .within(within)
                    .auto_collapse(scene.auto_collapse)
                    .mode(mode)
                    .tab_look(scene.config.tab_look)
                    .framed(scene.config.flush_docks && !in_window);
                let shown = match card {
                    Some(place) => shown.card(place),
                    None => shown,
                };
                shown.show_flyouts_at(ui, rect, |group, at, ui, body| pass.borrow_mut().group_of(ui, body, group, at, in_window))
            };
            if let Some(panel) = footer {
                let height = IconicSizes::of(ui.ctx()).icons_height(column) + scene.sizes.bar;
                Panel::new(name.clone())
                    .id(dock.with(("rail footer", panel)))
                    .pad(Pad::None)
                    .footer(|ui, _| {
                        // Compact rail controls leave room for the scrolling panel icons in short windows.
                        Stack::new().fill_width().gap(Gap::None).show(ui, |ui, body| {
                            scene.hosted(panel, HostPart::RailFooter, ui, |ui| pass.borrow_mut().panels.rail_footer(panel, ui, body))
                        });
                    })
                    .show_at(ui, cell, |ui, _| {
                        // design: allow[contract/allocate](the rail lays its icons out in a rect of its own: the panel body claims their height, so the host's footer sits under them)
                        let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height), egui::Sense::hover());
                        draw(ui, rect, open.clone())
                    })
                    .inner
            } else {
                Some(draw(ui, cell, open))
            }
        };
        let work = if column.iconic {
            let Some(output) = icons(cell, open, card) else { return };
            self.report.flyout.extend(output.edit.events);
            self.rail_work(output.front);
            let width = output.width.map(|width| width - inset);
            ColumnWork { iconic: output.iconic, picked: output.picked, resized: Vec::new(), width, flyout: output.flyout }
        } else {
            if !in_window && scene.rail_beside(column) {
                // The rail keeps to the dock's window edge and the expanded column shows inward of it.
                let rail = scene.sizes.iconic_width(column, &spec).min(cell.width());
                let at_left = (edge == DockEdge::Start) != scene.rtl;
                let (rail_cell, rest) = if at_left {
                    (cell.with_max_x(cell.left() + rail), cell.with_min_x(cell.left() + rail))
                } else {
                    (cell.with_min_x(cell.right() - rail), cell.with_max_x(cell.right() - rail))
                };
                cell = rest;
                // The rail stands square at the window edge; the expanded column's card is inward of it.
                let Some(output) = icons(rail_cell, None, None) else { return };
                self.rail_work(output.front);
                // The rail's inner edge sets its own width, not the expanded column's.
                self.column_work(column, ColumnWork { iconic: output.iconic, picked: output.picked, resized: Vec::new(), width: None, flyout: None });
                if let (Some(width), Some(first)) = (output.width, column.groups.first().and_then(|group| group.panels.first())) {
                    self.report.actions.push(Action::resize_iconic(first.clone(), Some(width)));
                }
            }
            let shown = DockColumn::new(name.clone(), dock, column, &spec)
                .edge(edge)
                .bar(bar)
                .arrow(self.arrow_of(column))
                .within(within)
                .auto_collapse(scene.auto_collapse)
                .tab_look(scene.config.tab_look);
            let shown = match card {
                Some(place) => shown.card(place),
                None => shown,
            };
            let output = shown.show_groups_at(ui, cell, |group, rect, ui, body| {
                self.group_of(ui, body, group, GroupAt::Show(rect), in_window);
            });
            if crate::runtime::measure::effect(ui.ctx(), "dock::divider_report") {
                let dividers = output.dividers.into_iter().map(|(divider, rect, node)| DockSource::new(SourceKind::Divider(divider), rect, node));
                self.report.sources.extend(dividers);
            }
            let width = output.width.map(|width| width - inset);
            ColumnWork { iconic: output.iconic, picked: output.picked, resized: output.resized, width, flyout: None }
        };
        self.column_work(column, work);
    }

    /// Brings the panel a click on its rail icon chose, `front`, to the front of its group.
    fn rail_work(&mut self, front: Option<P>) {
        self.report.actions.extend(front.map(Action::activate));
    }

    /// Turns what a column reported into actions and drags.
    fn column_work(&mut self, column: &Column<P>, work: ColumnWork<P>) {
        let Some(first) = column.groups.first().and_then(|group| group.panels.first()) else { return };
        if let Some(iconic) = work.iconic {
            let action = self.arrow(column, first, iconic);
            self.report.actions.extend(action);
        }
        if let Some(item) = work.picked {
            self.pick(first, item);
        }
        for (anchor, height) in work.resized {
            self.report.actions.push(Action::resize_group(anchor, Some(height)));
        }
        let scene = self.scene;
        let spec = |panel: &P| scene.spec(panel).clone();
        if let Some(width) = work.width {
            match scene.sizes.stepped(column, &spec) {
                // The edge of a stepped column moves in whole steps: the most that fit where it is dragged.
                Some((grid, steps)) if !column.iconic => {
                    let to = grid.steps_within(width);
                    if to != steps {
                        self.report.actions.push(Action::set_column_steps(first.clone(), to));
                    }
                }
                _ => self.report.actions.push(if column.iconic {
                    Action::resize_iconic(first.clone(), Some(width))
                } else {
                    Action::resize_column(first.clone(), width)
                }),
            }
        }
        if let Some(resize) = work.flyout {
            let panel = resize.panel;
            if let Some(width) = resize.width {
                self.report.actions.push(Action::resize_flyout_width(panel.clone(), Some(width)));
            }
            if let Some(height) = resize.height {
                self.report.actions.push(Action::resize_group(panel.clone(), Some(height)));
            }
            if resize.offset.is_some() {
                self.report.actions.push(Action::move_flyout_top(panel, resize.offset));
            }
        }
    }

    /// The floating windows of the pruned layout, each a `FloatingGroup` holding its one column, drawn as a dock
    /// column is with the window's title row as the column's bar, in an OS window where the config, the platform and
    /// its panels allow one. A window takes the drops over it on its own layer, from the spots its column registers in
    /// its own registry. `open` is the open flyout, which a collapsed window's icons share with the docks'. `started`
    /// is the layout a running move or resize of a window began on. Returns the move or resize of a window in the app
    /// window this pass, with the edge the window keeps to after it, which the area reports as a workspace edit.
    ///
    /// A window keeps to its edge of the float area (`Floating::edge`, else the one nearer it): its title row's
    /// double arrow sits at that end, and its collapsed column shows against that edge of its rect. A move or resize
    /// keeps the edge the window had when it began, so nothing jumps while the window crosses the middle. A collapsed
    /// window's move ends at the edge nearer the strip where it was dropped, with the rect that shows the strip there
    /// ([`WindowColumn::dropped`]); an expanded window's move leaves its edge to its rect. A collapsed window's sides
    /// resize its column's width within [`DockSizes::iconic_bounds`] (`Action::ResizeIconic`, as a docked collapsed
    /// column's inner edge does), and the window keeps its edge and its expanded rect's size. An OS window is measured
    /// where it would sit in the app window ([`native::in_app`]), and the end of each of its moves and resizes records
    /// that rect and edge; its placement keeps its expanded rect while its column is collapsed ([`WindowColumn::kept`]).
    ///
    /// A window whose every group is minimized shows its title row and their strips at its rect's place and width
    /// ([`WindowColumn::minimized`]): its sides resize its width, and its placement keeps its expanded height, so a
    /// restore shows it at that height again.
    ///
    /// A window whose column has steps ([`stepped`]) is as wide as its steps and as tall as its content at that
    /// width, at its rect's place: a drag of its side sets the most steps that fit the width, one of its top or
    /// bottom the fewest whose content fits the height, and the window keeps its place; a drag of its left side keeps
    /// its right edge. An OS window takes that rect at once ([`native::place`]); the app runs its grips, so the OS
    /// sizes it no other way.
    fn windows(
        &mut self,
        ui: &mut Ui,
        body: &mut BodyCx<'_>,
        open: &Option<P>,
        started: Option<&Layout<P>>,
    ) -> Option<(P, PlacementChange, Option<DockEdge>)> {
        let scene = self.scene;
        let ctx = ui.ctx().clone();
        let policy = |panel: &P| scene.permissions(panel);
        let protected = |panel: &P| scene.permissions(panel).is_protected();
        let spec_of = |panel: &P| scene.spec(panel).clone();
        let mut moved = None;
        for window in &scene.layout.floating {
            let Some(anchor) = window.panels().next().cloned() else { continue };
            // The window's menu acts on the shown panel of its top group.
            let active = window.groups().first().and_then(Group::active_panel).cloned().unwrap_or_else(|| anchor.clone());
            let specs: Vec<PanelSpec> = window.panels().map(|panel| scene.spec(panel).clone()).collect();
            let rights: Vec<Permissions> = window.panels().map(|panel| scene.permissions(panel)).collect();
            let dock = scene.dock.with(("floating", window.id));
            let group_id = dock.with("window");
            let continued = scene.continue_move.filter(|(panel, _)| window.panels().any(|held| held == *panel)).map(|(_, grab)| grab);
            let opening = native::opening(&ctx, scene.config, window, &specs, group_id, continued.is_some());
            // An OS window lays out at its own monitor's scale, which sizes its chrome; one in the app window at the
            // main window's.
            let scale = opening
                .spec
                .as_ref()
                .and_then(|_| WindowHost::of(&ctx).viewport_of(group_id))
                .and_then(|viewport| native::pixels_per_point(&ctx, viewport));
            let window_sizes = scale.map_or(scene.sizes, |scale| scene.sizes.at_scale(scale));
            let movable = rights.iter().all(|rights| rights.movable);
            // The title row is the column's bar: its double arrow and Close; the menu opens from a context request on
            // its band.
            let mut actions = Vec::new();
            if scene.config.close_button && rights.iter().all(|rights| rights.closable) {
                actions.push(HeaderAction::Close);
            }
            let (rows, _) = self.menu_rows(body, &active);
            let picked = Cell::new(None);
            let dropped = RefCell::new(None);
            let mut target = DockDropTarget::new(&ctx, dock, scene.layout, &policy);
            target.rtl = scene.rtl;
            target.space = Some(scene.space);
            target.float_on_empty = scene.config.float_on_empty_drop;
            target.title_bar_docks = scene.config.title_bar_docks;
            // The scene's layout is the pruned view the windows show, so the placing hides nothing more.
            target.placing = Some(Placing::new(scene.sizes, &spec_of, &protected).window(window.id));
            // The window is an instance of the group scope: its menu's command rows reach it, and run on its active
            // panel through the same path as a docked group's.
            let states = dock_states(&DockScene::new(scene.layout, scene.space, false, &policy).floated(scene.floated), &active);
            let mut commands = Vec::new();
            let spec = scene.spec(&active);
            let began = started.and_then(|layout| layout.floating.iter().find(|began| began.id == window.id));
            // The window's rect in the app window: for an OS window, where it would sit there.
            let in_app = opening.spec.as_ref().and_then(|_| native::in_app(&ctx, opening.rect)).unwrap_or(opening.rect);
            let (began_rect, began_edge) = began.map_or((in_app, window.edge), |began| (began.rect, began.edge));
            let edge = began_edge.unwrap_or_else(|| WindowColumn::nearer_edge(began_rect, scene.bounds, scene.rtl));
            let width = scene.sizes.expanded_width(&window.column, &spec_of);
            let shape = if window.column.iconic {
                let size = Vec2::new(scene.sizes.width(&window.column, &spec_of), IconicSizes::of(&ctx).icons_height(&window.column));
                WindowColumn::collapsed(edge, size, scene.sizes.iconic_bounds(&window.column, &spec_of))
            } else if let Some(content) = self.stepped_height(ui, body, &window.column, width, true) {
                WindowColumn::fitted(edge, Vec2::new(width, content))
            } else if let Some(strips) = scene.sizes.minimized_height(&window.column) {
                WindowColumn::minimized(edge, strips)
            } else {
                WindowColumn::expanded(edge)
            }
            .arrow(self.arrow_of(&window.column));
            let mut group = FloatingGroup::new(spec.label.clone(), opening.rect)
                .limits(scene.spec(&anchor).limits)
                .sized(Sizing::Fill)
                .column(shape)
                .resizable(specs.iter().all(|spec| spec.allow_float_resize))
                .actions(actions)
                .grip(scene.config.floating_grip)
                .rounded_body(scene.config.floating_rounded_body)
                .window_of(anchor.clone())
                .takes_drops(target, &dropped)
                .command_scope(DockCommands::SCOPE)
                .id(group_id);
            if let Some(scale) = scale {
                group = group.scale(scale);
            }
            // The host's focus request brings the window that holds its panel to the front.
            if scene.focus.is_some_and(|focus| window.panels().any(|held| held == focus)) {
                group = group.raise();
            }
            if let Some(grab) = continued {
                group = group.continue_move(grab);
            }
            if scene.raise.is_some_and(|raised| window.panels().any(|held| held == raised)) {
                group = group.raise();
            }
            group = match opening.spec.clone() {
                Some(window_spec) => group.native(window_spec),
                None => group.bounds(scene.bounds),
            };
            if !rows.is_empty() {
                let picked = &picked;
                group = group.menu(move |ui, _| {
                    if let Some(item) = PanelMenu::new(Vec::new(), rows.clone()).show(ui).picked {
                        picked.set(Some(item));
                    }
                });
            }
            let area_dock = std::mem::replace(&mut self.dock, dock);
            let drawn = self.report.sources.len();
            let first_group = self.report.shown.len();
            let shown = group.show_group(ui, |ui, body| {
                for (command, state) in states.iter().cloned() {
                    body.publish(command, state);
                }
                while let Some((command, _)) = body.take_command::<DockCommands>() {
                    commands.push(command);
                }
                // An OS window's flyouts and other layers take the room around the window (`overlay::layer_room`); an
                // in-app window's stay in the float area.
                let within = if opening.spec.is_some() { Rect::NOTHING } else { scene.bounds };
                let at = ColumnAt { cell: body.rect, edge, bar: ColumnBar::Window, within, open: open.clone(), card: None };
                let name = Label::from(body.words::<DockWords>().panels.as_str().to_owned());
                self.shown_column(ui, &name, &window.column, at, window.id);
            });
            self.dock = area_dock;
            let output = shown.output;
            native::waited(&ctx, &opening, &output);
            // The window's grips, then everything it drew, in the OS window it shows in, if it has one.
            let grips = output.grip.map(|(rect, node)| DockSource::new(SourceKind::WindowGrip(anchor.clone()), rect, node));
            let resize =
                output.resize.iter().map(|(edge, rect, node)| DockSource::new(SourceKind::WindowResize(anchor.clone(), *edge), *rect, *node));
            let inside = self.report.sources.split_off(drawn);
            self.report.sources.extend(grips.into_iter().chain(resize).chain(inside));
            if let Some(viewport) = opening.spec.as_ref().and_then(|_| WindowHost::of(&ctx).viewport_of(output.response.id)) {
                for source in &mut self.report.sources[drawn..] {
                    source.viewport = viewport;
                }
            }
            if !output.files.is_empty() {
                // The group under the drop, else the first group: a drop on the title row, or with no pointer known.
                let groups = &self.report.shown[first_group..];
                let under = output.files_at.and_then(|at| groups.iter().find(|group| group.rect.contains(at)));
                let panel = under.or(groups.first()).map_or_else(|| active.clone(), |group| group.panel.clone());
                self.report.files.push(WindowFiles { panel, files: output.files });
            }
            if let Some(dropped) = dropped.into_inner() {
                cross_window::accept_cross_window(&ctx);
                self.report.actions.push(dropped.action);
            }
            // The height above the window's body at its scale: the title row, the rule under it, and in the app window
            // the band over it.
            let chrome = window_sizes.window_chrome(opening.spec.is_none());
            // A resize of a window whose column has steps: the steps it sets, and the window's rect at them.
            let resized = output
                .placed
                .filter(|change| !matches!(change.kind, PlacementKind::Move))
                .zip(shape.fitted)
                .and_then(|(change, fitted)| self.stepped_resize(ui, body, &window.column, &anchor, &change, (fitted, chrome)));
            if opening.spec.is_some() {
                if let Some(viewport) = WindowHost::of(&ctx).viewport_of(output.response.id) {
                    // The OS window takes the rect of its steps at once, so the place recorded below is that rect.
                    if let Some(rect) = resized {
                        native::send_to(&ctx, viewport, opening.rect, rect);
                    }
                    self.report.windows.push((anchor.clone(), viewport));
                    // A move or resize that ends records where the window would sit in the app window, and the edge it
                    // keeps to: a collapsed window's the edge nearer its strip where it was dropped, as for a window
                    // in the app window ([`WindowColumn::dropped`]). The window's placement keeps the rect the
                    // collapsed column shows against that edge.
                    let mut kept = shape;
                    if movable
                        && let Some(change) = output.placed.filter(|change| change.phase == GesturePhase::Commit)
                        && let Some(rect) = native::in_app(&ctx, change.rect)
                    {
                        // A resize of a collapsed window keeps its edge; a move ends where its strip was dropped.
                        let (rect, keeps) =
                            if output.collapsed_width.is_some() { (rect, Some(edge)) } else { shape.dropped(rect, scene.bounds, scene.rtl) };
                        kept.edge = keeps.unwrap_or(kept.edge);
                        self.report.actions.push(Action::move_floating_with_edge(anchor.clone(), rect, keeps));
                    }
                    let reported = native::reported(&ctx, window, viewport, |shown| kept.kept(shown, opening.rect, chrome, scene.rtl));
                    self.report.actions.extend(reported);
                }
            } else if let Some(mut change) = output.placed.filter(|change| change.kind != PlacementKind::Move || change.phase == GesturePhase::Commit)
            {
                // The layer host shows a window its title row moves under the pointer, so the move reaches the workspace
                // once, when it ends: one edit. A collapsed window keeps its edge until then, and the move ends where its
                // strip was dropped.
                let mut keeps = shape.collapsed.map(|_| edge);
                if change.phase == GesturePhase::Commit && output.collapsed_width.is_none() {
                    (change.rect, keeps) = shape.dropped(change.rect, scene.bounds, scene.rtl);
                }
                // The window keeps the place of its steps; its size is the steps' own, whatever the rect keeps.
                if let Some(rect) = resized {
                    change.rect = Rect::from_min_size(rect.min, opening.rect.size());
                }
                if change.phase == GesturePhase::Commit {
                    self.report.events.push(DockEvent::FloatPlaced { panel: anchor.clone(), rect: change.rect });
                }
                moved = Some((anchor.clone(), change, keeps));
            }
            // The sides of a collapsed window set its column's width, as a docked collapsed column's inner edge does.
            if let Some(width) = output.collapsed_width {
                let work = ColumnWork { iconic: None, picked: None, resized: Vec::new(), width: Some(width), flyout: None };
                self.column_work(&window.column, work);
            }
            // The OS close of a native window arrives here as well, also where the title row offers no Close.
            if output.action == Some(HeaderAction::Close) && rights.iter().all(|rights| rights.closable) {
                self.report.actions.push(Action::close_window(anchor.clone()));
            }
            if let Some(item) = picked.get() {
                self.pick(&active, item);
            }
            // The window's commands run from its body, below the title row.
            let body = opening.rect.size() - Vec2::new(0.0, chrome);
            for command in commands {
                self.run(&active, command, Some(body));
            }
            // The title row is the column's bar: its double arrow collapses or expands the column.
            if let Some(iconic) = output.iconic {
                let action = self.arrow(&window.column, &anchor, iconic);
                self.report.actions.extend(action);
            }
        }
        moved
    }
}

/// The splits whose size `to` changes from `from`, each with its path from the root, where both trees split alike.
fn split_changes<P>(from: &Node<P>, to: &Node<P>) -> Vec<(Vec<bool>, SplitSize)> {
    fn walk<P>(from: &Node<P>, to: &Node<P>, path: &mut Vec<bool>, out: &mut Vec<(Vec<bool>, SplitSize)>) {
        let (Node::Split { size: was, first: first_was, second: second_was, .. }, Node::Split { size, first, second, .. }) = (from, to) else {
            return;
        };
        if was != size {
            out.push((path.clone(), *size));
        }
        for (second, (was, now)) in [(false, (first_was, first)), (true, (second_was, second))] {
            path.push(second);
            walk(was, now, path, out);
            path.pop();
        }
    }
    let mut out = Vec::new();
    walk(from, to, &mut Vec::new(), &mut out);
    out
}

/// The workspace edit of a floating window's move or resize, the step `phase` of a gesture that `action`
/// (`Action::MoveFloating`) places the window by: the gesture runs from the window's `Begin` to its `Commit`, so one
/// drag is one undo step. A move by the title row arrives only as its `Commit`, which sets the place. `moving` holds
/// the gesture and the workspace it began on; `rights` are the panels' permissions with the lock applied, which refuse
/// the move of a locked workspace.
fn window_edit<P: Clone + Eq + Hash>(
    ctx: &Context,
    current: &Arc<Workspace<P>>,
    rights: &HashMap<P, Permissions>,
    moving: &mut Option<Moving<P>>,
    phase: GesturePhase,
    action: Action<P>,
    label: Arc<str>,
) -> Option<EditEvent<Arc<Workspace<P>>>> {
    let moved = || {
        let mut next = Workspace::clone(current);
        let policy = |panel: &P| rights.get(panel).copied().unwrap_or(Permissions::ALL);
        next.apply(action.clone(), false, policy).ok().map(|_| Arc::new(next))
    };
    let begin = |moving: &mut Option<Moving<P>>| {
        let id = crate::runtime::edit::next_gesture_id(ctx);
        *moving = Some(Moving { id, before: Arc::clone(current) });
        EditEvent::begin(id, Mixed::Same(Arc::clone(current)), Arc::clone(&label), None)
    };
    match phase {
        GesturePhase::Begin => Some(begin(moving)),
        GesturePhase::Update => match moving.as_ref() {
            Some(held) => Some(EditEvent::update(held.id, moved()?)),
            None => Some(begin(moving)),
        },
        GesturePhase::Commit => match moving.take() {
            Some(held) => Some(EditEvent::commit(held.id, Mixed::Same(held.before), moved()?)),
            None => Some(EditEvent::set(Mixed::Same(Arc::clone(current)), moved()?, label, None)),
        },
        _ => moving.take().map(|held| EditEvent::cancel(held.id, CancelReason::Dismissed, Some(Mixed::Same(held.before)))),
    }
}

/// The panels whose host views `column` shows: each group's shown panel, and each of `extra` (the open flyout's, the
/// rail footer's) that the column holds.
fn column_shows<'c, P: PartialEq>(column: &'c Column<P>, extra: [Option<&'c P>; 2]) -> impl Iterator<Item = &'c P> {
    let shown = column.groups.iter().filter_map(Group::active_panel);
    shown.chain(extra.into_iter().flatten().filter(move |panel| column.panels().any(|held| held == *panel)))
}

/// Shows `panel`'s body in a group's body, in the host's style and once per pass where the body is the app's own egui
/// code (`Scene::hosted`). It runs every pass the group shows; the host memoizes inside it.
fn panel_body<'p, P: Clone + Eq + Hash>(
    scene: &Scene<'_, P>,
    panels: &RefCell<&mut (dyn DockPanels<P> + 'p)>,
    panel: &P,
    ui: &mut Ui,
    body: &mut BodyCx<'_>,
) {
    let host = scene.host_body(panel);
    scene.hosted(panel, HostPart::Body, ui, |ui| {
        // The app's own egui body keeps its background: the host's panel fill over the group's surface, as it showed
        // outside the dock.
        if host {
            scene.host_style.paint_panel(ui, ui.max_rect());
        }
        panels.borrow_mut().body(panel, ui, body);
    });
}

/// The props of a tool rail's grip strip: the group it drags, its name, and the outbox the drag's start goes to, with
/// the point its press went down.
#[derive(MemoProps)]
struct RailGrip<'a, P> {
    #[memo_props(hash = "a dock's panels are plain values it keys by their hash")]
    unit: DragUnit<P>,
    active: PanelKey,
    movable: bool,
    label: Label,
    sent: Outbox<'a, Pos2>,
}

/// A tool rail's grip strip: the dock's grip marks over the view's room, which a drag moves the rail's group by.
/// Returns the grip's id.
fn rail_grip<P: Clone + Hash + Send + Sync + 'static>(props: &mut RailGrip<'_, P>, ui: &mut Ui) -> Id {
    let strip = ui.max_rect();
    let held = GroupGrip::new(props.unit.clone(), props.active)
        .ticks(true)
        .movable(props.movable)
        .ghost(props.label.clone())
        .label(props.label.clone())
        .show_at(ui, strip);
    if let Some(origin) = held.drag_origin {
        props.sent.send(origin);
    }
    // The grip takes no room in flow; the run covers the strip, so the pointer over it runs it again.
    ui.expand_to_include_rect(strip);
    held.response.id
}
