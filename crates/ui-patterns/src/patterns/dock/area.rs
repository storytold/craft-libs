//! `DockArea`: the docking pattern. It shows a workspace's docked tree, dock columns, flyouts and floating groups from
//! the parts of `docking`, takes the drops of tab and group drags, and applies their actions to the workspace.

use super::hide::{self, Shown};
use crate::contract::overlay::PlacementChange;
use crate::contract::{Bind, BindMut, BodyCx, Cx, Entry, NoValue, Outcome, Pointer as _, Props as _, ShowContainer as _, Sizing, WordSetCx as _};
use crate::docking::SplitFramesCx as _;
use crate::docking::stack::{StackSection, StackSections, section_heights};
use crate::docking::tree::FrameRules;
use crate::docking::{Arrangement, cross_window, end_replaced_drags, float_drop, native};
use crate::docking::{
    DockColumn, DockCommands, DockConfig, DockDropTarget, DockDropped, DockEvent, DockGroup, DockPanels, DockRoot, DockScene, DockSizes, DockSpot,
    DockTargets, DockTo, DockWords, DragUnit, FloatSpace, FloatingGroup, GroupAt, GroupDrag, HidePanels, IconicColumn, PanelDrag, PanelKey,
    PanelSpec, PanelTitle, SplitArea, actions, dock_states, fit_widths, frame, move_actions,
};
use crate::menu::{GroupItem, MenuWords, PanelMenu, PanelRow};
use crate::runtime::commands::{CommandRef, CommandSet as _, Commands as _, focus_within};
use crate::runtime::env::{Env, Label, RevealEdge, ShellCx, ShellEntry};
use crate::runtime::memory::Remember as _;
use crate::runtime::windows::WindowHost;
use crate::sections::HeaderAction;
use craft_behavior::dock::{Action, Column, DockEdge, DockError, Group, Layout, Node, PanelPlace, Permissions, Workspace, node_at};
use craft_behavior::{CancelReason, DockSide, EditEvent, EditEvents, Flow, GestureId, GesturePhase, Mixed, Proposal};
use egui::{Context, Id, Pos2, Rect, Response, Ui, Vec2};
use std::cell::{Cell, RefCell};
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
    /// The changes the user made through the area that the workspace took, in order: tab picks, closes, moves, drops,
    /// floats and menu rows. An action the workspace refuses (a protected panel, a locked area) is left out. A given
    /// workspace is never written: these are the actions it would take, for the host to apply.
    pub actions: Vec<Action<P>>,
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
///   flyouts; a group is a `DockGroup`; a floating window is a `FloatingGroup`. The host's `DockPanels` supplies the
///   panels.
/// - Edits: a divider drag and every [`Action`] of the pass (menu rows, drags, drops) are written to the workspace
///   through its binding, with the panels' [`DockPanels::permissions`] (narrowed when `locked`). The output lists the
///   actions the workspace took. A given workspace is not written.
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
    /// The panel open in an iconic column's flyout, owned by the host. Without it the area keeps the value.
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

/// What the root's drop taker reads: the dock, the layout it shows, the rights and where a floating drop lands.
struct Drops<'a, P> {
    dock: Id,
    layout: &'a Layout<P>,
    permissions: &'a dyn Fn(&P) -> Permissions,
    config: &'a DockConfig,
    specs: &'a HashMap<P, PanelSpec>,
    rtl: bool,
}

impl<P: Clone + Eq + Hash + Debug + Send + Sync + 'static> Drops<'_, P> {
    /// Takes a group or tab drag released on the root.
    fn take(&self, cx: &mut Cx<'_, DockRoot<'_, DockDropped<P>>>) -> Option<DockDropped<P>> {
        let ctx = cx.core.ctx.clone();
        let sizes = *cx.style.sizes();
        let bounds = cx.env.get::<ShellEntry>().0.as_ref().map_or(cx.rect, ShellCx::float_bounds);
        let space = float_space(bounds, &sizes, self.config, self.specs);
        let mut target = DockDropTarget::new(&ctx, self.dock, self.layout, self.permissions);
        target.rtl = self.rtl;
        target.space = Some(&space);
        target.float_on_empty = self.config.float_on_empty_drop;
        target.title_bar_docks = self.config.title_bar_docks;
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
}

impl<P: Clone + Eq + Hash> Scene<'_, P> {
    fn spec(&self, panel: &P) -> PanelSpec {
        self.specs.get(panel).cloned().unwrap_or_else(|| PanelSpec::new(Label::default()))
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
    actions: Vec<Action<P>>,
    events: Vec<DockEvent<P>>,
    focused: Option<P>,
    picked: Option<P>,
    edit: EditEvents<Workspace<P>>,
    space: Option<FloatSpace<P>>,
    columns: Vec<(P, Rect)>,
}

/// How a group is shown: minimized, and whether it is an accordion section (with its open state).
#[derive(Clone, Copy)]
struct Look {
    minimized: bool,
    section: Option<bool>,
}

/// A dock to show: its window edge, its columns and its rect.
struct DockPlan<'p, P> {
    edge: DockEdge,
    columns: &'p [Column<P>],
    rect: Rect,
    flow: Flow,
    open: Option<P>,
}

/// What a column reported.
struct ColumnWork<P> {
    iconic: Option<bool>,
    picked: Option<GroupItem>,
    resized: Vec<(P, f32)>,
    /// The expanded width the user gave an expanded column, or the iconic width of an iconic one.
    width: Option<f32>,
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
        let drops = Drops { dock: id, layout: &current.layout, permissions: &policy, config: &config, specs: &specs, rtl };
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
        let Passed { actions, events, focused, picked, edit, space, columns } = passed.unwrap_or_else(|| Passed {
            actions: Vec::new(),
            events: Vec::new(),
            focused: None,
            picked: None,
            edit: EditEvents::new(),
            space: None,
            columns: Vec::new(),
        });
        if shown.output.dropped.is_some() {
            cross_window::accept_cross_window(&ctx);
        }
        DockAreaOutput { actions, events, focused, picked, error, edit, float_space: space, columns, response: shown.output.response }
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
    let Input { id, ctx, current, config, specs, hide, maximized, auto_collapse, sizes, rtl } = *input;
    let shell = body.shell();
    let mut memory = body.memory::<AreaMemory<P>>().clone();
    let keys: HashMap<PanelKey, P> = current.layout.panels().into_iter().map(|panel| (PanelKey::of(panel), panel.clone())).collect();
    let protected = |panel: &P| rights.get(panel).is_some_and(|rights| rights.is_protected());
    let Shown { view, floating } = hide::shown(&current.layout, maximized, hide, shell.as_ref(), &protected);
    let layout: &Layout<P> = &view.layout;
    let bounds = shell.as_ref().map_or(body.rect, ShellCx::float_bounds);
    let space = float_space(bounds, &sizes, config, specs);
    let scene = Scene { dock: id, sizes, config, layout, specs, permissions: rights, keys: &keys, space: &space, bounds, auto_collapse, rtl };
    let open = open_of(&mut flyout, body, &memory);
    let mut pass = Pass {
        scene: &scene,
        panels,
        report: Report { actions: Vec::new(), events: Vec::new(), focused: None, picked: None, flyout: Vec::new(), placed: Vec::new() },
        dock: id,
    };

    let spec_of = |panel: &P| scene.spec(panel);
    let limits = |panel: &P| scene.spec(panel).limits;
    let dock_width = |columns: &[Column<P>]| scene.sizes.natural(columns, &spec_of).iter().map(|(width, _)| *width).sum::<f32>();
    let stripless = |panel: &P| scene.spec(panel).canvas;
    let rules = FrameRules::new(&limits).dock_width(&dock_width).stripless(&stripless);
    let label = Label::from(body.words::<DockWords>().panels.as_str().to_owned());
    let mut leaves: Vec<(Vec<bool>, Rect)> = Vec::new();
    let area = SplitArea::new(label, Bind::given(layout.clone()), rules).show(ui, |ui, split| {
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
    });
    let mut edit = area.output.edit.events.map(|edited| {
        let mut next = current.clone();
        view.write_back(&edited, &mut next.layout);
        next
    });
    register_bands(&scene, ctx, body.rect, &leaves);
    let moved = if floating { pass.windows(ui, body) } else { None };
    let mut report = pass.report;
    let label: Arc<str> = Arc::from(body.words::<DockWords>().move_window.as_str());
    edit.extend(moved.and_then(|(anchor, change)| window_edit(ctx, current, rights, &mut memory.moving, &anchor, &change, label)));

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
        actions: report.actions,
        events: report.events,
        focused: report.focused,
        picked: report.picked,
        edit,
        space: Some(space),
        columns: report.placed,
    }
}

/// Registers the outer-edge bands and the new-dock bands as the dock's drop spots for the next pass, from the leaves
/// the split area placed this pass (each as its tree path and rect). A new-dock band runs along a start or end edge
/// that holds no dock.
fn register_bands<P: Clone + Eq + Hash + Send + 'static>(scene: &Scene<'_, P>, ctx: &Context, area: Rect, leaves: &[(Vec<bool>, Rect)]) {
    if scene.layout.root.is_none() {
        return;
    }
    let canvas = |panel: &P| scene.spec(panel).canvas;
    let bands = frame::edge_bands(scene.layout, leaves, &canvas, scene.config, &scene.sizes, scene.rtl);
    frame::register_edge_bands::<P>(ctx, scene.dock, &bands);
    if scene.config.new_dock_zones {
        let flow = Flow::new(area, scene.rtl);
        let band = scene.sizes.new_dock_band;
        let start = Rect::from_min_size(area.min, Vec2::new(band, area.height()));
        let end = Rect::from_min_max(Pos2::new(area.right() - band, area.top()), area.max);
        for (side, edge, rect) in [(DockSide::Start, DockEdge::Start, start), (DockSide::End, DockEdge::End, end)] {
            if scene.layout.dock_path(edge).is_none() {
                DockTargets::<P>::register(ctx, scene.dock, DockSpot::NewDock(side), flow.to_physical(rect));
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

impl<P: Clone + Eq + Hash + Debug + Send + Sync + 'static> Pass<'_, P> {
    /// One group of the docked tree or of a floating window: a tab group, or the sections of an accordion.
    fn leaf(&mut self, ui: &mut Ui, body: &mut BodyCx<'_>, node: &Node<P>, rect: Rect) {
        let scene = self.scene;
        match node {
            Node::Tabs { panels: members, active, .. } => {
                self.group(ui, body, members, *active, Look { minimized: false, section: None }, GroupAt::Show(rect));
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
                        let look = Look { minimized: !entry.open, section: Some(entry.open) };
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

    /// The rows of the menu of `panel`'s group, and whether the host gave any (only then does the group show a menu
    /// button; a context request opens the menu either way): the host's rows, then the panel commands the config shows, with
    /// the Move Panel To submenu of the groups the panel can move to.
    fn menu_rows(&mut self, body: &BodyCx<'_>, panel: &P) -> (Vec<PanelRow>, bool) {
        let scene = self.scene;
        let config = scene.config;
        let command = |command: DockCommands| PanelRow::Command(CommandRef::from(command));
        let mut group = vec![command(DockCommands::FloatPanel)];
        let targets = scene.layout.move_targets(panel, |other| scene.permissions(other));
        if scene.permissions(panel).movable && !targets.is_empty() {
            let rows = targets.iter().enumerate().map(|(at, target)| PanelRow::Item(Entry::new(GroupItem::MoveTo(at), scene.spec(target).label)));
            let name = Label::from(body.words::<MenuWords>().panel_move_to.as_str().to_owned());
            group.push(PanelRow::Submenu(name, rows.collect()));
        }
        if config.close_menu_items {
            group.extend([command(DockCommands::ClosePanel), command(DockCommands::CloseGroup)]);
        }
        group.push(command(DockCommands::FloatGroup));
        if config.close_others_item {
            group.push(command(DockCommands::CloseOthers));
        }
        if config.maximize_item {
            group.push(command(DockCommands::ToggleMaximize));
        }
        let mut rows = self.panels.panel_rows(panel);
        let hosted = !rows.is_empty();
        if hosted {
            rows.push(PanelRow::Separator);
        }
        rows.extend(group);
        (rows, hosted)
    }

    /// Runs a panel command on `panel`: the event of a command the host carries out, else its actions.
    fn run(&mut self, panel: &P, command: DockCommands) {
        let scene = self.scene;
        let permissions = |panel: &P| scene.permissions(panel);
        // The area's permissions are narrowed for a locked workspace already.
        let commands = DockScene::new(scene.layout, scene.space, false, &permissions);
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
        let commands = DockScene::new(scene.layout, scene.space, false, &permissions);
        if let Ok(planned) = move_actions(&commands, &DragUnit::Panel(panel.clone()), DockTo::tabs(target, None)) {
            self.report.actions.extend(planned);
        }
    }

    /// Runs a dock row of a panel or column menu that is not a command row.
    fn pick(&mut self, panel: &P, item: GroupItem) {
        match item {
            GroupItem::MoveTo(at) => self.move_to(panel, at),
            GroupItem::CollapseToIcons => self.run(panel, DockCommands::CollapseToIcons),
            GroupItem::AutoCollapse => self.run(panel, DockCommands::AutoCollapse),
        }
    }

    /// One group, by the host's `group_view` when it takes the group, else as a `DockGroup` over the host's panels:
    /// shown in a rect, or measured at a width the way it would show, returning its height.
    fn group(&mut self, ui: &mut Ui, body: &mut BodyCx<'_>, members: &[P], active: usize, look: Look, at: GroupAt) -> Option<f32> {
        let scene = self.scene;
        let anchor = members.first()?;
        let active = active.min(members.len() - 1);
        if let Some(height) = self.panels.group_view(members, active, ui, at, body) {
            return matches!(at, GroupAt::Measure(_)).then_some(height);
        }
        let titles: Vec<PanelTitle> = members
            .iter()
            .map(|panel| PanelTitle::new(PanelKey::of(panel), scene.spec(panel).label).closable(scene.permissions(panel).closable))
            .collect();
        let shown = &members[active];
        let (menu, hosted) = self.menu_rows(body, shown);
        let permissions = |panel: &P| scene.permissions(panel);
        let states = dock_states(&DockScene::new(scene.layout, scene.space, false, &permissions), shown);
        let panels = RefCell::new(&mut *self.panels);
        let mut group = DockGroup::new(anchor.clone(), &titles, PanelKey::of(shown))
            .extra(|ui, body| panels.borrow_mut().group_header(members, active, ui, body))
            .dock(self.dock)
            .zones(scene.config.zone_rule())
            .movable(scene.permissions(shown).movable)
            .menu(menu)
            .panel_menu_shown(hosted)
            .command_states(states);
        if look.minimized {
            group = group.minimized();
        }
        if scene.config.tab_close_buttons {
            group = group.closable();
        }
        let rect = match at {
            GroupAt::Measure(width) => {
                return Some(group.measure_in(ui, Proposal::Width(width), |ui, body| panels.borrow_mut().body(shown, ui, body)).y);
            }
            GroupAt::Show(rect) => rect,
        };
        let output = group.show_at(ui, rect, |ui, body| panels.borrow_mut().body(shown, ui, body)).output;
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
            self.run(shown, command);
        }
        if output.toggle_minimized {
            self.report.actions.push(match look.section {
                Some(open) => Action::set_stack_open(shown.clone(), !open),
                None => Action::set_minimized(shown.clone(), !look.minimized),
            });
        }
        // A tear-off inside the window is a release no spot took: it floats only with `float_on_empty_drop`.
        if let Some(torn) = output.torn_off
            && let Some(panel) = scene.panel(torn.grabbed)
            && scene.permissions(panel).movable
            && scene.permissions(panel).floatable
            && (scene.config.float_on_empty_drop || !ui.ctx().content_rect().contains(torn.pointer))
        {
            self.report.actions.push(float_drop(&DragUnit::Panel(panel.clone()), scene.space, torn.pointer).action);
        }
        None
    }

    fn group_of(&mut self, ui: &mut Ui, body: &mut BodyCx<'_>, group: &Group<P>, at: GroupAt) -> Option<f32> {
        let look = Look { minimized: group.minimized, section: None };
        self.group(ui, body, &group.panels, group.active, look, at)
    }

    /// The columns of a dock, from the window edge inwards.
    fn dock(&mut self, ui: &mut Ui, body: &mut BodyCx<'_>, plan: DockPlan<'_, P>) {
        let scene = self.scene;
        let spec = |panel: &P| scene.spec(panel);
        let widths = fit_widths(&scene.sizes.natural(plan.columns, &spec), plan.rect.width());
        let reading = plan.flow.to_physical(plan.rect);
        let start = plan.edge == DockEdge::Start;
        let mut cursor = if start { reading.left() } else { reading.right() };
        let name = Label::from(body.words::<DockWords>().panels.as_str().to_owned());
        for (column, width) in plan.columns.iter().zip(widths) {
            let span = if start { (cursor, cursor + width) } else { (cursor - width, cursor) };
            cursor = if start { span.1 } else { span.0 };
            let cell = plan.flow.to_physical(Rect::from_x_y_ranges(span.0..=span.1, reading.y_range()));
            let Some(first) = column.groups.first().and_then(|group| group.panels.first()) else { continue };
            self.report.placed.push((first.clone(), cell));
            if column.iconic {
                let open = plan.open.clone();
                let output = IconicColumn::new(name.clone(), scene.dock, column, Bind::given(open.clone()), &spec)
                    .edge(plan.edge)
                    .within(body.rect)
                    .auto_collapse(scene.auto_collapse)
                    .show_flyouts_at(ui, cell, |group, at, ui, body| self.group_of(ui, body, group, at));
                self.report.flyout.extend(output.edit.events);
                let work = ColumnWork { iconic: output.iconic, picked: output.picked, resized: Vec::new(), width: output.width };
                self.column_work(column, work);
            } else {
                let output = DockColumn::new(name.clone(), scene.dock, column, &spec)
                    .edge(plan.edge)
                    .within(body.rect)
                    .auto_collapse(scene.auto_collapse)
                    .show_groups_at(ui, cell, |group, rect, ui, body| {
                        self.group_of(ui, body, group, GroupAt::Show(rect));
                    });
                let work = ColumnWork { iconic: output.iconic, picked: output.picked, resized: output.resized, width: output.width };
                self.column_work(column, work);
            }
        }
    }

    /// Turns what a column reported into actions and drags.
    fn column_work(&mut self, column: &Column<P>, work: ColumnWork<P>) {
        let Some(first) = column.groups.first().and_then(|group| group.panels.first()) else { return };
        if let Some(iconic) = work.iconic {
            self.report.actions.push(Action::set_iconic(first.clone(), iconic));
        }
        if let Some(item) = work.picked {
            self.pick(first, item);
        }
        for (anchor, height) in work.resized {
            self.report.actions.push(Action::resize_group(anchor, Some(height)));
        }
        if let Some(width) = work.width {
            self.report.actions.push(if column.iconic {
                Action::resize_iconic(first.clone(), Some(width))
            } else {
                Action::resize_column(first.clone(), width)
            });
        }
    }

    /// The floating windows of the pruned layout, each a `FloatingGroup` holding its tree's groups, in an OS window
    /// where the config, the platform and its panels allow one. A window takes the drops over it on its own layer,
    /// from the spots its groups register in its own registry. Returns the move or resize of a window in the app window
    /// this pass, which the area reports as a workspace edit.
    fn windows(&mut self, ui: &mut Ui, body: &BodyCx<'_>) -> Option<(P, PlacementChange)> {
        let scene = self.scene;
        let ctx = ui.ctx().clone();
        let policy = |panel: &P| scene.permissions(panel);
        let mut moved = None;
        for window in &scene.layout.floating {
            let Some(anchor) = window.panels().next().cloned() else { continue };
            let active = match &window.root {
                Node::Tabs { panels, active, .. } => panels.get(*active).cloned().unwrap_or_else(|| anchor.clone()),
                _ => anchor.clone(),
            };
            let specs: Vec<PanelSpec> = window.panels().map(|panel| scene.spec(panel)).collect();
            let rights: Vec<Permissions> = window.panels().map(|panel| scene.permissions(panel)).collect();
            let opening = native::opening(&ctx, scene.config, window, &specs);
            let mut actions = vec![HeaderAction::Menu];
            if rights.iter().all(|rights| rights.movable) {
                actions.push(HeaderAction::Float);
            }
            if scene.config.close_button && rights.iter().all(|rights| rights.closable) {
                actions.push(HeaderAction::Close);
            }
            let (rows, _) = self.menu_rows(body, &active);
            let picked = Cell::new(None);
            let dropped = RefCell::new(None);
            let dock = scene.dock.with(("floating", window.id));
            let mut target = DockDropTarget::new(&ctx, dock, scene.layout, &policy);
            target.rtl = scene.rtl;
            target.space = Some(scene.space);
            target.float_on_empty = scene.config.float_on_empty_drop;
            target.title_bar_docks = scene.config.title_bar_docks;
            // The window is an instance of the group scope: its menu's command rows reach it, and run on its active
            // panel through the same path as a docked group's.
            let states = dock_states(&DockScene::new(scene.layout, scene.space, false, &policy), &active);
            let mut commands = Vec::new();
            let spec = scene.spec(&active);
            let mut group = FloatingGroup::new(spec.label, opening.rect)
                .limits(scene.spec(&anchor).limits)
                .sized(Sizing::Fill)
                .minimized(window.minimized)
                .actions(actions)
                .window_of(anchor.clone())
                .takes_drops(target, &dropped)
                .command_scope(DockCommands::SCOPE)
                .id_salt(("floating", window.id));
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
                let limits = |panel: &P| scene.spec(panel).limits;
                let rules = FrameRules::new(&limits);
                let name = Label::from(body.words::<DockWords>().panels.as_str().to_owned());
                let tree = Layout::new(Some(window.root.clone()), Vec::new());
                let area = SplitArea::new(name, Bind::given(tree), rules).show_at(ui, body.rect, |ui, split| {
                    for (leaf, rect) in split.group_frames::<P>() {
                        let Some(place) = scene.layout.place(&leaf) else { continue };
                        let Some(node) = scene.layout.node_of(&place) else { continue };
                        self.leaf(ui, split, node, rect);
                    }
                });
                area.output.edit.events
            });
            self.dock = area_dock;
            if let Some(edited) = shown.inner.as_ref().and_then(EditEvents::value).and_then(|tree| tree.root.as_ref()) {
                self.report.actions.extend(split_resizes(&anchor, &window.root, edited));
            }
            let output = shown.output;
            if let Some(dropped) = dropped.into_inner() {
                cross_window::accept_cross_window(&ctx);
                self.report.actions.push(dropped.action);
            }
            if opening.spec.is_some() {
                if let Some(viewport) = WindowHost::of(&ctx).viewport_of(output.response.id) {
                    self.report.actions.extend(native::reported(&ctx, window, viewport));
                }
            } else if let Some(change) = output.placed {
                if change.phase == GesturePhase::Commit {
                    self.report.events.push(DockEvent::FloatPlaced { panel: anchor.clone(), rect: change.rect });
                }
                moved = Some((anchor.clone(), change));
            }
            match output.action {
                Some(HeaderAction::Close) => self.report.actions.push(Action::close_window(anchor.clone())),
                Some(HeaderAction::Float) => {
                    let commands = DockScene::new(scene.layout, scene.space, false, &policy);
                    if let Ok(docked) = move_actions(&commands, &DragUnit::Window(anchor.clone()), DockTo::Docked) {
                        self.report.actions.extend(docked);
                    }
                }
                _ => {}
            }
            if let Some(item) = picked.get() {
                self.pick(&active, item);
            }
            for command in commands {
                self.run(&active, command);
            }
            if output.minimized_toggled {
                self.report.actions.push(Action::set_minimized(anchor.clone(), !window.minimized));
            }
        }
        moved
    }
}

/// The `ResizeWindowSplit` actions that take the splits of the floating window holding `anchor`, whose tree is
/// `from`, to the sizes `to` has.
fn split_resizes<P: Clone>(anchor: &P, from: &Node<P>, to: &Node<P>) -> Vec<Action<P>> {
    fn walk<P: Clone>(anchor: &P, from: &Node<P>, to: &Node<P>, path: &mut Vec<bool>, out: &mut Vec<Action<P>>) {
        let (Node::Split { size: was, first: first_was, second: second_was, .. }, Node::Split { size, first, second, .. }) = (from, to) else {
            return;
        };
        if was != size {
            out.push(Action::resize_window_split(anchor.clone(), path.clone(), *size));
        }
        for (second, (was, now)) in [(false, (first_was, first)), (true, (second_was, second))] {
            path.push(second);
            walk(anchor, was, now, path, out);
            path.pop();
        }
    }
    let mut out = Vec::new();
    walk(anchor, from, to, &mut Vec::new(), &mut out);
    out
}

/// The workspace edit of a floating window's move or resize: the gesture runs from the window's `Begin` to its
/// `Commit`, so one drag is one undo step. `moving` holds the gesture and the workspace it began on; `rights` are the
/// panels' permissions with the lock applied, which refuse the move of a locked workspace.
fn window_edit<P: Clone + Eq + Hash>(
    ctx: &Context,
    current: &Workspace<P>,
    rights: &HashMap<P, Permissions>,
    moving: &mut Option<Moving<P>>,
    anchor: &P,
    change: &PlacementChange,
    label: Arc<str>,
) -> Option<EditEvent<Workspace<P>>> {
    let moved = || {
        let mut next = current.clone();
        let policy = |panel: &P| rights.get(panel).copied().unwrap_or(Permissions::ALL);
        next.apply(Action::move_floating(anchor.clone(), change.rect), false, policy).ok().map(|_| next)
    };
    let begin = |moving: &mut Option<Moving<P>>| {
        let id = crate::runtime::edit::next_gesture_id(ctx);
        *moving = Some(Moving { id, before: current.clone() });
        EditEvent::begin(id, Mixed::Same(current.clone()), Arc::clone(&label), None)
    };
    match change.phase {
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
