//! `DockArea`: the docking pattern. It shows a workspace's docked tree, dock columns, flyouts and floating groups from
//! the parts of `docking`, takes the drops of tab and group drags, and applies their actions to the workspace.

use super::hide::{self, Shown};
use crate::contract::overlay::{PlacementChange, PlacementKind};
use crate::contract::{
    Bind, BindMut, BodyCx, Cx, Entry, Lazy, NoValue, Outcome, Pad, Pointer as _, Props as _, ShowContainer as _, Sizing, WordSetCx as _,
};
use crate::docking::SplitFramesCx as _;
use crate::docking::stack::{StackSection, StackSections, section_heights};
use crate::docking::{Arrangement, close_requests, cross_window, end_replaced_drags, float_drop, native, open_requests};
use crate::docking::{
    ColumnArrow, ColumnBar, DockColumn, DockCommands, DockConfig, DockDropTarget, DockDropped, DockEvent, DockGroup, DockPanels, DockRoot, DockScene,
    DockSizes, DockSpot, DockTargets, DockTo, DockWords, DragUnit, FloatSpace, FloatingGroup, FlyoutResize, GroupAt, GroupDrag, HidePanels,
    IconicColumn, IconicSizes, PanelDrag, PanelKey, PanelSpec, PanelTitle, Placing, SplitArea, WindowColumn, actions, column_cells, dock_states,
    frame, move_actions, stepped, with_area_rules,
};
use crate::menu::{GroupItem, MenuWords, PanelMenu, PanelRow};
use crate::runtime::commands::{CommandRef, CommandSet as _, CommandState, Commands as _, focus_within};
use crate::runtime::drag;
use crate::runtime::env::{Env, Label, RevealEdge, ShellCx, ShellEntry};
use crate::runtime::memory::Remember as _;
use crate::runtime::windows::{ResizeEdge, WindowHost};
use crate::sections::HeaderAction;
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

#[cfg(feature = "testing")]
mod stories;

/// A layout the dock cannot show: the workspace failed `Workspace::validate`. The area draws nothing for it.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct InvalidGeometry {
    /// Why the model refused the workspace.
    pub error: DockError,
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
    /// The panel whose tab the user clicked or picked this pass.
    pub picked: Option<P>,
    /// Why nothing was drawn, when the workspace is invalid.
    pub error: Option<InvalidGeometry>,
    /// The edits of the whole workspace this pass, in order: a split divider's size, a floating window's move, and the
    /// applied `actions` as one undo step. A workspace bound by reference takes them itself; a given one needs the host
    /// to apply them.
    pub edit: EditEvents<Workspace<P>>,
    /// Where commands float a panel, group or column: the area's float bounds and the sizes of what it drew.
    pub float_space: Option<FloatSpace<P>>,
    /// Where each dock column was placed this pass, named by a panel of its first group, in the area's layer space.
    /// Hosts that anchor their own surfaces to a column read it.
    pub columns: Vec<(P, Rect)>,
    /// The OS windows the floating windows showed in this pass, each named by a panel of the window (the first).
    pub windows: Vec<(P, ViewportId)>,
    /// The files dropped on the area's windows this pass, for the host to open.
    pub window_files: Vec<drag::OsFile>,
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
///   through its binding, with the panels' [`DockPanels::permissions`] (narrowed when `locked`). The output lists the
///   actions the workspace took, divider sizes and window moves included. A given workspace is not written.
/// - Requests: while the pass runs, a body changes the workspace through
///   [`request_actions`](crate::docking::request_actions); the output returns those actions unapplied in `requested`.
/// - Hiding: [`hide`](DockArea::hide), the shell's panel visibility and [`maximized`](DockArea::maximized) prune
///   what shows; a pruned panel keeps its place for restore.
/// - Drops: the area is the drop target of tab and group drags. Ctrl or Cmd held at the release floats the drop, and
///   so does a release no spot takes, with `DockConfig::float_on_empty_drop`.
/// - State: the open flyout (unless the host binds it with [`flyout`](DockArea::flyout)) lives in the area's root
///   container.
/// - Replacement: a tab or group drag records the workspace revision it started under; when another arrangement
///   replaces the workspace ([`Workspace::replace`]), the drag ends without a drop.
#[derive(crate::contract::Pattern, crate::contract::Props)]
#[pattern(probe(DockArea<'static, usize>))]
pub struct DockArea<'a, P: Clone + Eq + Hash + Debug + Send + Sync + 'static> {
    label: Label,
    workspace: Bind<'a, Workspace<P>>,
    panels: &'a mut dyn DockPanels<P>,
    config: DockConfig,
    /// Which panels the host hides.
    #[prop]
    hide: HidePanels<'a, P>,
    /// The panel whose frame shows alone.
    #[prop]
    maximized: Option<P>,
    /// Auto-Collapse Iconic Panels: a flyout closes when the focus leaves it.
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
}

impl<'a, P: Clone + Eq + Hash + Debug + Send + Sync + 'static> DockArea<'a, P> {
    /// A dock area named `label` over `workspace`, with its panels from `panels` and its behaviour from `config`. The
    /// root's id comes from the label under the parent's id, and inside an `AppShell` from the window too, so two
    /// windows' docks never share state.
    pub fn new(label: impl Into<Label>, workspace: Bind<'a, Workspace<P>>, panels: &'a mut dyn DockPanels<P>, config: DockConfig) -> Self {
        DockArea {
            label: label.into(),
            workspace,
            panels,
            config,
            hide: HidePanels::None,
            maximized: None,
            auto_collapse: false,
            locked: false,
            flyout: None,
        }
    }

    /// A dock area over a layout that is only shown: a given workspace without hidden panels, locked, so nothing
    /// moves, floats or closes.
    pub fn read_only(label: impl Into<Label>, layout: &Layout<P>, panels: &'a mut dyn DockPanels<P>, config: DockConfig) -> Self {
        DockArea { locked: true, ..Self::new(label, Bind::given(Workspace::new(layout.clone())), panels, config) }
    }
}

/// What the area keeps in its root container.
#[derive(Clone)]
struct AreaMemory<P> {
    /// The open flyout when the host binds none.
    open: Option<P>,
    /// The running move or resize of a floating window.
    moving: Option<Moving<P>>,
}

/// A floating window's move or resize as one workspace edit: its gesture and the workspace when it began.
#[derive(Clone)]
struct Moving<P> {
    id: GestureId,
    before: Workspace<P>,
}

impl<P> Default for AreaMemory<P> {
    fn default() -> Self {
        AreaMemory { open: None, moving: None }
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
    hide: HidePanels<'a, P>,
    maximized: Option<&'a P>,
    rtl: bool,
}

impl<P: Clone + Eq + Hash + Debug + Send + Sync + 'static> Drops<'_, P> {
    /// Takes a group or tab drag released on the root.
    fn take(&self, cx: &mut Cx<'_, DockRoot<'_, DockDropped<P>>>) -> Option<DockDropped<P>> {
        let ctx = cx.core.ctx.clone();
        let sizes = *cx.style.sizes();
        let shell = cx.env.get::<ShellEntry>().0;
        let bounds = shell.as_ref().map_or(cx.rect, ShellCx::float_bounds);
        let space = float_space(bounds, &sizes, self.config, self.specs);
        let spec = |panel: &P| spec_in(self.specs, panel);
        let protected = |panel: &P| (self.permissions)(panel).is_protected();
        let (hide, _) = hide::resolved(self.hide, shell.as_ref());
        let mut target = DockDropTarget::new(&ctx, self.dock, self.layout, self.permissions);
        target.rtl = self.rtl;
        target.space = Some(&space);
        target.float_on_empty = self.config.float_on_empty_drop;
        target.title_bar_docks = self.config.title_bar_docks;
        target.placing = Some(Placing::new(sizes, &spec, &protected).shown(hide, self.maximized));
        cx.drop_target::<GroupDrag<P>, _>(&target).or_else(|| cx.drop_target::<PanelDrag, _>(&target))
    }
}

/// What the area collects while it shows its parts.
struct Report<P> {
    actions: Vec<Action<P>>,
    events: Vec<DockEvent<P>>,
    focused: Option<P>,
    picked: Option<P>,
    flyout: Vec<EditEvent<Option<P>>>,
    placed: Vec<(P, Rect)>,
    /// The OS windows the floating windows showed in, by their first panel.
    windows: Vec<(P, ViewportId)>,
    /// The files dropped on the floating windows that no target there took.
    files: Vec<drag::OsFile>,
}

/// What every part of one pass reads.
struct Scene<'s, P> {
    dock: Id,
    sizes: DockSizes,
    config: &'s DockConfig,
    layout: &'s Layout<P>,
    specs: &'s HashMap<P, PanelSpec>,
    permissions: &'s HashMap<P, Permissions>,
    keys: &'s HashMap<PanelKey, P>,
    space: &'s FloatSpace<P>,
    bounds: Rect,
    auto_collapse: bool,
    rtl: bool,
    /// The docked places floating panels left, for Dock Panel.
    floated: &'s [(P, Location<P>)],
    /// The key of what every region of the chrome shows of the pass beyond its own room ([`chrome_key`]); `None` when
    /// the host gives no [`DockPanels::spec_version`], and the chrome runs every pass.
    chrome: Option<u64>,
}

/// The spec of `panel` in `specs`, or an unnamed one.
fn spec_in<P: Eq + Hash>(specs: &HashMap<P, PanelSpec>, panel: &P) -> PanelSpec {
    specs.get(panel).cloned().unwrap_or_else(|| PanelSpec::new(Label::default()))
}

impl<P: Clone + Eq + Hash> Scene<'_, P> {
    fn spec(&self, panel: &P) -> PanelSpec {
        spec_in(self.specs, panel)
    }

    fn permissions(&self, panel: &P) -> Permissions {
        self.permissions.get(panel).copied().unwrap_or(Permissions::ALL)
    }

    fn panel(&self, key: PanelKey) -> Option<&P> {
        self.keys.get(&key)
    }
}

/// What the area reports from one pass.
struct Passed<P> {
    /// The divider sizes and window moves of `edit`, as actions on the workspace the pass started with.
    geometry: Vec<Action<P>>,
    /// The pass's other actions: as `show_area` proposes them, then the ones the workspace took.
    actions: Vec<Action<P>>,
    events: Vec<DockEvent<P>>,
    focused: Option<P>,
    picked: Option<P>,
    edit: EditEvents<Workspace<P>>,
    space: Option<FloatSpace<P>>,
    columns: Vec<(P, Rect)>,
    windows: Vec<(P, ViewportId)>,
    files: Vec<drag::OsFile>,
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
    titles: Vec<PanelTitle>,
    menu: Vec<PanelRow>,
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
}

/// Where and how a column shows: its rect; the edge it keeps to, a dock's window edge or the float area's edge a
/// floating window keeps to; where its bar is; the rect its inner edge and flyouts stay within; and the open flyout.
/// Its flyouts open away from its edge: towards the canvas for a dock column, towards the float area's room for a
/// floating one.
struct ColumnAt<P> {
    cell: Rect,
    edge: DockEdge,
    bar: ColumnBar,
    within: Rect,
    open: Option<P>,
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
        let DockArea { label, mut workspace, panels, config, hide, maximized, auto_collapse, locked, mut flyout } = self;
        let ctx = ui.ctx().clone();
        let (rtl, window) = {
            let env = Env::of(ui);
            (env.direction.is_rtl(), env.get::<ShellEntry>().0.map(|shell| shell.window().clone()))
        };
        let id = ui.scope_id().with(("dock area", label.as_str(), window));
        let held = match workspace.reborrow().get() {
            Some(Mixed::Same(held)) => held,
            _ => Workspace::default(),
        };
        let (current, error) = match held.validate() {
            Ok(()) => (held, None),
            Err(error) => (Workspace::default(), Some(InvalidGeometry { error })),
        };
        let rights: HashMap<P, Permissions> = current
            .layout
            .panels()
            .into_iter()
            .map(|panel| {
                let rights = panels.permissions(panel);
                (panel.clone(), if locked { rights.locked() } else { rights })
            })
            .collect();
        let policy = |panel: &P| rights.get(panel).copied().unwrap_or(Permissions::ALL);
        let specs: HashMap<P, PanelSpec> = current.layout.panels().into_iter().map(|panel| (panel.clone(), panels.spec(panel))).collect();
        let drops = Drops {
            dock: id,
            layout: &current.layout,
            permissions: &policy,
            config: &config,
            specs: &specs,
            hide,
            maximized: maximized.as_ref(),
            rtl,
        };
        let shows = Arrangement { dock: id, revision: current.revision() };
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
            let Some(sizes) = body.style::<DockRoot<'_, DockDropped<P>>>().map(|style| *style.sizes()) else { return };
            let input = Input {
                id,
                ctx: &ctx,
                current: &current,
                config: &config,
                specs: &specs,
                hide,
                maximized: maximized.as_ref(),
                auto_collapse,
                locked,
                sizes,
                rtl,
            };
            let mut result = show_area(ui, body, &input, &rights, &mut *panels, flyout.as_mut());
            for edit in &result.edit {
                workspace.reborrow().write(body, edit);
            }
            result.actions.extend(dropped.take());
            if !result.actions.is_empty() {
                let proposed = std::mem::take(&mut result.actions);
                let applied = apply_actions(body, workspace.reborrow(), &current, &*panels, locked, &label, proposed);
                result.actions = applied.actions;
                result.edit.extend(applied.edit);
            }
            passed = Some(result);
        });
        let requested = close_requests::<P>(&ctx);
        let Passed { geometry, actions, events, focused, picked, edit, space, columns, windows, files } = passed.unwrap_or_else(|| Passed {
            geometry: Vec::new(),
            actions: Vec::new(),
            events: Vec::new(),
            focused: None,
            picked: None,
            edit: EditEvents::new(),
            space: None,
            columns: Vec::new(),
            windows: Vec::new(),
            files: Vec::new(),
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
            picked,
            error,
            edit,
            float_space: space,
            columns,
            windows,
            window_files: files,
            response: shown.output.response,
        }
    }
}

/// What the workspace took of a pass's actions.
struct Applied<P> {
    actions: Vec<Action<P>>,
    edit: Option<EditEvent<Workspace<P>>>,
}

/// Applies `proposed` in order to the workspace the binding holds, with the panels' permissions (narrowed when
/// `locked`). A refused action changes nothing and is left out of the result. The accepted ones are written as one
/// edit, which a given workspace does not take. `current` is the workspace as the pass started.
fn apply_actions<P: Clone + Eq + Hash + Debug + Send + Sync + 'static>(
    body: &mut BodyCx<'_>,
    mut workspace: BindMut<'_, Workspace<P>>,
    current: &Workspace<P>,
    panels: &dyn DockPanels<P>,
    locked: bool,
    label: &Label,
    proposed: Vec<Action<P>>,
) -> Applied<P> {
    let before = match workspace.read(body) {
        Some(Mixed::Same(held)) => held,
        _ => current.clone(),
    };
    let permissions = |panel: &P| panels.permissions(panel);
    let mut next = before.clone();
    let mut actions = Vec::new();
    for action in proposed {
        if next.apply(action.clone(), locked, permissions).is_ok() {
            actions.push(action);
        }
    }
    if actions.is_empty() {
        return Applied { actions, edit: None };
    }
    let edit = EditEvent::set(Mixed::Same(before), next, Arc::from(label.as_str()), None);
    workspace.write(body, &edit);
    Applied { actions, edit: Some(edit) }
}

/// What the pattern gives one pass.
struct Input<'i, P> {
    id: Id,
    ctx: &'i Context,
    current: &'i Workspace<P>,
    config: &'i DockConfig,
    specs: &'i HashMap<P, PanelSpec>,
    hide: HidePanels<'i, P>,
    maximized: Option<&'i P>,
    auto_collapse: bool,
    locked: bool,
    sizes: DockSizes,
    rtl: bool,
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
    let Input { id, ctx, current, config, specs, hide, maximized, auto_collapse, locked, sizes, rtl } = *input;
    let shell = body.shell();
    let mut memory = body.memory::<AreaMemory<P>>().clone();
    let keys: HashMap<PanelKey, P> = current.layout.panels().into_iter().map(|panel| (PanelKey::of(panel), panel.clone())).collect();
    let protected = |panel: &P| rights.get(panel).is_some_and(|rights| rights.is_protected());
    let Shown { view, floating } = hide::shown(&current.layout, maximized, hide, shell.as_ref(), &protected);
    let layout: &Layout<P> = &view.layout;
    let bounds = shell.as_ref().map_or(body.rect, ShellCx::float_bounds);
    let space = float_space(bounds, &sizes, config, specs);
    let chrome = panels.spec_version().map(|version| chrome_key(layout, version, config, locked, auto_collapse));
    let scene = Scene {
        dock: id,
        sizes,
        config,
        layout,
        specs,
        permissions: rights,
        keys: &keys,
        space: &space,
        bounds,
        auto_collapse,
        rtl,
        floated: &current.floated,
        chrome,
    };
    let open = open_of(&mut flyout, body, &memory);
    let mut pass = Pass {
        scene: &scene,
        panels,
        report: Report {
            actions: Vec::new(),
            events: Vec::new(),
            focused: None,
            picked: None,
            flyout: Vec::new(),
            placed: Vec::new(),
            windows: Vec::new(),
            files: Vec::new(),
        },
        dock: id,
    };

    let spec_of = |panel: &P| scene.spec(panel);
    let label = Label::from(body.words::<DockWords>().panels.as_str().to_owned());
    let mut leaves: Vec<(Vec<bool>, Rect)> = Vec::new();
    let area = with_area_rules(&scene.sizes, &spec_of, |rules| {
        SplitArea::new(label, Bind::given(layout.clone()), rules).show(ui, |ui, split| {
            DockTargets::<P>::register_room(ctx, id, split.rect);
            for (anchor, rect) in split.group_frames::<P>() {
                let Some(place) = scene.layout.place(&anchor) else { continue };
                let Some(node) = scene.layout.node_of(&place) else { continue };
                if let PanelPlace::Node(path) = &place {
                    leaves.push((path.clone(), rect));
                }
                pass.leaf(ui, split, node, rect);
            }
            let flow = Flow::new(split.rect, rtl);
            for (edge, rect) in split.dock_frames() {
                let Some(path) = scene.layout.dock_path(edge) else { continue };
                let Some(Node::Dock { columns, .. }) = scene.layout.root.as_ref().and_then(|root| node_at(root, &path)) else { continue };
                leaves.push((path.clone(), rect));
                if let Some(shell) = shell.as_ref().filter(|shell| shell.revealed() && crate::runtime::measure::effect(ctx, "shell::reveal_rect")) {
                    shell.reveal_rect(if edge == DockEdge::Start { RevealEdge::Start } else { RevealEdge::End }, rect);
                }
                pass.dock(ui, split, DockPlan { edge, columns, rect, flow, open: open.clone() });
            }
        })
    });
    let mut edit = area.output.edit.events.map(|edited| {
        let mut next = current.clone();
        view.write_back(&edited, &mut next.layout);
        next
    });
    let mut geometry: Vec<Action<P>> = match (current.layout.root.as_ref(), edit.landed().and_then(|landed| landed.layout.root.as_ref())) {
        (Some(from), Some(to)) => split_changes(from, to).into_iter().map(|(path, size)| Action::resize_split(path, size)).collect(),
        _ => Vec::new(),
    };
    register_bands(&scene, ctx, body.rect, &leaves);
    let started = memory.moving.as_ref().map(|moving| &moving.before.layout);
    let moved = if floating { pass.windows(ui, body, &open, started) } else { None };
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
            event.landed().and_then(|landed| place_in(&landed.layout)).filter(|place| Some(*place) != place_in(&current.layout))
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
    Passed {
        geometry,
        actions: report.actions,
        events: report.events,
        focused: report.focused,
        picked: report.picked,
        edit,
        space: Some(space),
        columns: report.placed,
        windows: report.windows,
        files: report.files,
    }
}

/// Registers the outer-edge bands and the new-dock bands as the dock's drop spots for the next pass, from the leaves
/// the split area placed this pass (each as its tree path and rect). A new-dock band runs along a start or end edge
/// that holds no dock.
fn register_bands<P: Clone + Eq + Hash + Send + Sync + 'static>(scene: &Scene<'_, P>, ctx: &Context, area: Rect, leaves: &[(Vec<bool>, Rect)]) {
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
/// default float size of `sizes`.
fn float_space<P: Clone + PartialEq>(bounds: Rect, sizes: &DockSizes, config: &DockConfig, specs: &HashMap<P, PanelSpec>) -> FloatSpace<P> {
    let mut space = FloatSpace::new(bounds, sizes).with_side_edge_zones(config.side_edge_zones);
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
        let rows = targets.iter().enumerate().map(|(at, target)| PanelRow::Item(Entry::new(GroupItem::MoveTo(at), scene.spec(target).label)));
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

/// The key of what the chrome's regions show beyond their own room: the host's panel specs (`version`), the config,
/// whether the workspace is locked, Auto-Collapse, and the shown arrangement: which panels each group, column and
/// floating window holds, the shown tab, and the minimized, open, iconic and stepped states, which the panel menus'
/// command states and move targets read across groups. Sizes are left out: a size reaches a region as its room.
fn chrome_key<P: Hash>(layout: &Layout<P>, version: u64, config: &DockConfig, locked: bool, auto_collapse: bool) -> u64 {
    use std::hash::Hasher;
    fn column<P: Hash>(shown: &Column<P>, hasher: &mut impl Hasher) {
        (shown.iconic, shown.steps, shown.groups.len()).hash(hasher);
        for group in &shown.groups {
            (&group.panels, group.active, group.minimized).hash(hasher);
        }
    }
    fn node<P: Hash>(shown: &Node<P>, hasher: &mut impl Hasher) {
        std::mem::discriminant(shown).hash(hasher);
        match shown {
            Node::Split { first, second, .. } => {
                node(first, hasher);
                node(second, hasher);
            }
            Node::Tabs { panels, active, .. } => (panels, active).hash(hasher),
            Node::Stack { entries, .. } => {
                for entry in entries {
                    (&entry.panel, entry.open).hash(hasher);
                }
            }
            Node::Dock { side, columns, .. } => {
                side.hash(hasher);
                columns.iter().for_each(|one| column(one, hasher));
            }
            // `Node` is non_exhaustive: a variant added later hashes only its discriminant until it is listed here.
            _ => {}
        }
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (version, config, locked, auto_collapse).hash(&mut hasher);
    if let Some(root) = &layout.root {
        node(root, &mut hasher);
    }
    for window in &layout.floating {
        (window.id, window.edge).hash(&mut hasher);
        column(&window.column, &mut hasher);
    }
    hasher.finish()
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

    /// One group, by the host's `group_view` when it takes the group, else as a `DockGroup` over the host's panels:
    /// shown in a rect, or measured at a width the way it would show, returning its height.
    fn group(&mut self, ui: &mut Ui, body: &mut BodyCx<'_>, members: &[P], active: usize, look: Look, at: GroupAt) -> Option<f32> {
        let scene = self.scene;
        let anchor = members.first()?;
        let active = active.min(members.len() - 1);
        if let GroupAt::Measure(_) = at
            && let Some(height) = self.panels.group_view(members, active, ui, at, body)
        {
            return Some(height);
        }
        let shown = &members[active];
        let content = self.panels.content(shown);
        // A host view settles as a panel body does, under the shown panel's content version (`DockPanels::content`):
        // it runs again when that version, the group's panels or the shown tab change.
        let view_version = content.map(|content| {
            use std::hash::Hasher;
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            (content, members, active).hash(&mut hasher);
            hasher.finish()
        });
        // The group settles when the host versions its specs and the shown body: it runs again when the pass's chrome
        // key, the body's version or how the group shows changes.
        let version = scene.chrome.zip(content).map(|(chrome, content)| {
            use std::hash::Hasher;
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            (chrome, content, members, active, look.minimized, look.section, look.in_window).hash(&mut hasher);
            hasher.finish()
        });
        let words = body.words::<MenuWords>();
        let panels = RefCell::new(&mut *self.panels);
        // The tab titles, the menu rows and the panel commands' states, built when the group runs.
        let parts: OnceCell<GroupParts> = OnceCell::new();
        let parts_of = || {
            parts.get_or_init(|| {
                let titles = members
                    .iter()
                    .map(|panel| PanelTitle::new(PanelKey::of(panel), scene.spec(panel).label).closable(scene.permissions(panel).closable))
                    .collect();
                let (menu, hosted) = panel_menu_rows(scene, &mut **panels.borrow_mut(), &words, shown);
                let permissions = |panel: &P| scene.permissions(panel);
                let states = dock_states(&DockScene::new(scene.layout, scene.space, false, &permissions).floated(scene.floated), shown);
                GroupParts { titles, menu, hosted, states }
            })
        };
        // The group with `members[at]` shown: as it shows, or as it is measured.
        let make = |at: usize, minimized: bool| {
            let panels = &panels;
            let GroupParts { titles, menu, hosted, states } = parts_of();
            let mut group = DockGroup::new(anchor.clone(), titles, PanelKey::of(&members[at]))
                .extra(move |ui, body| panels.borrow_mut().group_header(members, at, ui, body))
                .dock(self.dock)
                .zones(scene.config.zone_rule())
                .movable(scene.permissions(shown).movable)
                .menu(menu.clone())
                .panel_menu_shown(*hosted)
                .command_states(states.clone());
            if look.in_window {
                group = group.in_window();
            }
            if minimized {
                group = group.minimized();
            } else if !document(&scene.spec(&members[at])) {
                group = group.pad(Pad::Regular);
            }
            if scene.config.tab_close_buttons {
                group = group.closable();
            }
            if scene.config.wheel_selects_tabs {
                group = group.wheel_selects_tabs();
            }
            group
        };
        let rect = match at {
            GroupAt::Measure(width) => {
                let measured = make(active, look.minimized).measure_in(ui, Proposal::Width(width), |ui, body| panel_body(&panels, shown, ui, body));
                return Some(measured.y);
            }
            GroupAt::Show(rect) => rect,
        };
        // One child `Ui` held to the group's rect, whether the group settles or not. It takes the parent's next id, as
        // the group did before it settled, so a change of the group's first tab keeps every id inside; each Lazy is
        // keyed by its `Ui`'s id, unique among the groups of a dock in a scroll area too. The host view runs first and
        // settles; where the host leaves the group to the dock, the group shows in its place and settles under its own
        // version.
        let output = ui
            .scope_builder(UiBuilder::new().max_rect(rect), |ui| {
                let view = Lazy::new(ui.unique_id(), 0.0).settles(view_version.is_some());
                let view = match view_version {
                    Some(version) => view.content(version),
                    None => view,
                };
                view.show_or(
                    ui,
                    |ui| {
                        // The view draws held to the group's rect and clipped to it: it lays out in the group's room,
                        // never in the room the area has left.
                        let taken = ui
                            .scope_builder(UiBuilder::new().max_rect(rect), |ui| {
                                ui.shrink_clip_rect(rect);
                                panels.borrow_mut().group_view(members, active, ui, at, body)
                            })
                            .inner;
                        // The run covers the group, so the pointer over it runs the view again.
                        taken.map(|_| ui.expand_to_include_rect(rect))
                    },
                    |ui| {
                        let lazy = Lazy::new(ui.unique_id(), 0.0).settles(version.is_some());
                        let lazy = match version {
                            Some(version) => lazy.content(version),
                            None => lazy,
                        };
                        lazy.show(ui, |ui| {
                            let output = make(active, look.minimized).show_at(ui, rect, |ui, body| panel_body(&panels, shown, ui, body)).output;
                            // `show_at` takes no room in flow; the run covers the group, so the pointer over it runs it again.
                            ui.expand_to_include_rect(rect);
                            output
                        })
                    },
                )
            })
            .inner;
        // A host view took the group, or a settled view or group replayed its last run: nothing reached it.
        let output = output.and_then(Result::err).flatten()?;
        // The size `panel` floats at out of this group: the group's width, and the height the group measures at with
        // `panel` shown in a floating window's room, so content that fills the body takes that room.
        let fitted = |ui: &Ui, panel: &P| {
            let at = members.iter().position(|member| member == panel).unwrap_or(active);
            let room = Vec2::new(rect.width(), scene.space.content_room());
            let measured = make(at, false).id(output.response.id).measure_in(ui, Proposal::Size(room), |ui, body| {
                panel_body(&panels, &members[at], ui, body);
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
        if let Some(panel) = output.picked.and_then(|key| scene.panel(key)) {
            self.report.actions.push(Action::activate(panel.clone()));
            self.report.picked = Some(panel.clone());
        }
        if focus_within(ui.ctx(), output.response.id) {
            self.report.focused = Some(shown.clone());
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
        let scene = self.scene;
        let spec = |panel: &P| scene.spec(panel);
        let cells = column_cells(plan.columns, plan.rect, plan.edge, plan.flow, &scene.sizes, &spec);
        for (column, mut cell) in plan.columns.iter().zip(cells) {
            let Some(first) = column.groups.first().and_then(|group| group.panels.first()) else { continue };
            // A stepped column is as tall as its bar and content, from the top of the dock; content taller than the
            // dock gets the dock's height and scrolls inside. The rest of its slot is the dock's empty room.
            if let Some(content) = self.stepped_height(ui, body, column, cell.width(), false) {
                cell.max.y = (cell.top() + scene.sizes.bar + content).min(cell.bottom());
            }
            self.report.placed.push((first.clone(), cell));
            let at = ColumnAt { cell, edge: plan.edge, bar: ColumnBar::Own, within: body.rect, open: plan.open.clone() };
            self.column(ui, body, column, at);
        }
    }

    /// The steps a resize `change` of the floating window of the stepped `column`, whose first panel is `first`, sets,
    /// and the window's rect at them: as wide as its steps and as tall as its title row and its content at that width.
    /// `fitted` is the content's size under the title row before the change. A grip on the top or bottom edge sets the
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
        fitted: Vec2,
    ) -> Option<Rect> {
        let scene = self.scene;
        let spec_of = |panel: &P| scene.spec(panel);
        let (grid, steps) = stepped(column, &spec_of)?;
        let by_height = match change.kind {
            PlacementKind::Size(grip) => matches!(grip, ResizeEdge::Top | ResizeEdge::Bottom),
            _ => (change.rect.width() - fitted.x).abs() <= 0.5,
        };
        let to = if by_height {
            let room = change.rect.height() - scene.sizes.title_row;
            grid.steps_for_height(room, |width| self.stepped_height(ui, body, column, width, true).unwrap_or(0.0))
        } else {
            grid.steps_within(change.rect.width())
        };
        if to != steps {
            self.report.actions.push(Action::set_column_steps(first.clone(), to));
        }
        let size = Vec2::new(grid.width(to), scene.sizes.title_row + self.stepped_height(ui, body, column, grid.width(to), true).unwrap_or(0.0));
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
        let spec = |panel: &P| scene.spec(panel);
        if column.iconic || stepped(column, &spec).is_none() {
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
        let spec = |panel: &P| scene.spec(panel);
        ColumnArrow::of(column, &spec, column.panels().all(|panel| scene.permissions(panel).collapses))
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
    fn column(&mut self, ui: &mut Ui, body: &mut BodyCx<'_>, column: &Column<P>, at: ColumnAt<P>) {
        let scene = self.scene;
        let spec = |panel: &P| scene.spec(panel);
        let name = Label::from(body.words::<DockWords>().panels.as_str().to_owned());
        let dock = self.dock;
        let ColumnAt { cell, edge, bar, within, open } = at;
        let in_window = bar == ColumnBar::Window;
        if in_window {
            DockTargets::<P>::register_room(ui.ctx(), dock, cell);
        }
        let opens = if edge == DockEdge::Start { Side::End } else { Side::Start };
        let work = if column.iconic {
            let mut shown = IconicColumn::new(name, dock, column, Bind::given(open), &spec)
                .edge(edge)
                .opens(opens)
                .bar(bar)
                .within(within)
                .auto_collapse(scene.auto_collapse);
            if let Some(chrome) = scene.chrome {
                shown = shown.content(chrome);
            }
            let output = shown.show_flyouts_at(ui, cell, |group, at, ui, body| self.group_of(ui, body, group, at, in_window));
            self.report.flyout.extend(output.edit.events);
            ColumnWork { iconic: output.iconic, picked: output.picked, resized: Vec::new(), width: output.width, flyout: output.flyout }
        } else {
            let mut shown = DockColumn::new(name, dock, column, &spec)
                .edge(edge)
                .bar(bar)
                .arrow(self.arrow_of(column))
                .within(within)
                .auto_collapse(scene.auto_collapse);
            if let Some(chrome) = scene.chrome {
                shown = shown.content(chrome);
            }
            let output = shown.show_groups_at(ui, cell, |group, rect, ui, body| {
                self.group_of(ui, body, group, GroupAt::Show(rect), in_window);
            });
            ColumnWork { iconic: output.iconic, picked: output.picked, resized: output.resized, width: output.width, flyout: None }
        };
        self.column_work(column, work);
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
        let spec = |panel: &P| scene.spec(panel);
        if let Some(width) = work.width {
            match stepped(column, &spec) {
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
    /// ([`WindowColumn::dropped`]); an expanded window's move leaves its edge to its rect. An OS window is measured
    /// where it would sit in the app window ([`native::in_app`]), and the end of each of its moves and resizes records
    /// that rect and edge; its placement keeps its expanded rect while its column is collapsed ([`WindowColumn::kept`]).
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
        let spec_of = |panel: &P| scene.spec(panel);
        let mut moved = None;
        for window in &scene.layout.floating {
            let Some(anchor) = window.panels().next().cloned() else { continue };
            // The window's menu acts on the shown panel of its top group.
            let active = window.groups().first().and_then(Group::active_panel).cloned().unwrap_or_else(|| anchor.clone());
            let specs: Vec<PanelSpec> = window.panels().map(|panel| scene.spec(panel)).collect();
            let rights: Vec<Permissions> = window.panels().map(|panel| scene.permissions(panel)).collect();
            let dock = scene.dock.with(("floating", window.id));
            let group_id = dock.with("window");
            let opening = native::opening(&ctx, scene.config, window, &specs, group_id);
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
                WindowColumn::collapsed(edge, size)
            } else if let Some(content) = self.stepped_height(ui, body, &window.column, width, true) {
                WindowColumn::fitted(edge, Vec2::new(width, content))
            } else {
                WindowColumn::expanded(edge)
            }
            .arrow(self.arrow_of(&window.column));
            let mut group = FloatingGroup::new(spec.label, opening.rect)
                .limits(scene.spec(&anchor).limits)
                .sized(Sizing::Fill)
                .column(shape)
                .actions(actions)
                .window_of(anchor.clone())
                .takes_drops(target, &dropped)
                .command_scope(DockCommands::SCOPE)
                .id(group_id);
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
            let shown = group.show_group(ui, |ui, body| {
                for (command, state) in states.iter().cloned() {
                    body.publish(command, state);
                }
                while let Some((command, _)) = body.take_command::<DockCommands>() {
                    commands.push(command);
                }
                let at = ColumnAt { cell: body.rect, edge, bar: ColumnBar::Window, within: scene.bounds, open: open.clone() };
                self.column(ui, body, &window.column, at);
            });
            self.dock = area_dock;
            let output = shown.output;
            self.report.files.extend(output.files);
            if let Some(dropped) = dropped.into_inner() {
                cross_window::accept_cross_window(&ctx);
                self.report.actions.push(dropped.action);
            }
            // A resize of a window whose column has steps: the steps it sets, and the window's rect at them.
            let resized = output
                .placed
                .filter(|change| !matches!(change.kind, PlacementKind::Move))
                .zip(shape.fitted)
                .and_then(|(change, fitted)| self.stepped_resize(ui, body, &window.column, &anchor, &change, fitted));
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
                        let (rect, keeps) = shape.dropped(rect, scene.bounds, scene.rtl);
                        kept.edge = keeps.unwrap_or(kept.edge);
                        self.report.actions.push(Action::move_floating_with_edge(anchor.clone(), rect, keeps));
                    }
                    let title_row = scene.sizes.title_row;
                    let reported = native::reported(&ctx, window, viewport, |shown| kept.kept(shown, opening.rect, title_row, scene.rtl));
                    self.report.actions.extend(reported);
                }
            } else if let Some(mut change) = output.placed {
                // A collapsed window keeps the edge its move began with until the move ends where its strip was dropped.
                let mut keeps = shape.collapsed.map(|_| edge);
                if change.phase == GesturePhase::Commit {
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
            // The OS close of a native window arrives here as well, also where the title row offers no Close.
            if output.action == Some(HeaderAction::Close) && rights.iter().all(|rights| rights.closable) {
                self.report.actions.push(Action::close_window(anchor.clone()));
            }
            if let Some(item) = picked.get() {
                self.pick(&active, item);
            }
            // The window's commands run from its body, below the title row.
            let body = opening.rect.size() - Vec2::new(0.0, scene.sizes.title_row);
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
/// drag is one undo step. `moving` holds the gesture and the workspace it began on; `rights` are the panels'
/// permissions with the lock applied, which refuse the move of a locked workspace.
fn window_edit<P: Clone + Eq + Hash>(
    ctx: &Context,
    current: &Workspace<P>,
    rights: &HashMap<P, Permissions>,
    moving: &mut Option<Moving<P>>,
    phase: GesturePhase,
    action: Action<P>,
    label: Arc<str>,
) -> Option<EditEvent<Workspace<P>>> {
    let moved = || {
        let mut next = current.clone();
        let policy = |panel: &P| rights.get(panel).copied().unwrap_or(Permissions::ALL);
        next.apply(action.clone(), false, policy).ok().map(|_| next)
    };
    let begin = |moving: &mut Option<Moving<P>>| {
        let id = crate::runtime::edit::next_gesture_id(ctx);
        *moving = Some(Moving { id, before: current.clone() });
        EditEvent::begin(id, Mixed::Same(current.clone()), Arc::clone(&label), None)
    };
    match phase {
        GesturePhase::Begin => Some(begin(moving)),
        GesturePhase::Update => match moving.as_ref() {
            Some(held) => Some(EditEvent::update(held.id, moved()?)),
            None => Some(begin(moving)),
        },
        GesturePhase::Commit => match moving.take() {
            Some(held) => Some(EditEvent::commit(held.id, Mixed::Same(held.before), moved()?)),
            None => Some(EditEvent::set(Mixed::Same(current.clone()), moved()?, label, None)),
        },
        _ => moving.take().map(|held| EditEvent::cancel(held.id, CancelReason::Dismissed, Some(Mixed::Same(held.before)))),
    }
}

/// Shows `panel`'s body in a group's body; a panel with a content version settles (`DockPanels::content`).
fn panel_body<'p, P: Hash>(panels: &RefCell<&mut (dyn DockPanels<P> + 'p)>, panel: &P, ui: &mut Ui, body: &mut BodyCx<'_>) {
    let content = panels.borrow().content(panel);
    match content {
        Some(version) => {
            Lazy::new(("panel", PanelKey::of(panel).0), 0.0).settles(true).content(version).show(ui, |ui| panels.borrow_mut().body(panel, ui, body));
        }
        None => panels.borrow_mut().body(panel, ui, body),
    }
}
