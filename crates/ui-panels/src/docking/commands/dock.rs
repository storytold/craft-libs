//! The panel commands of a docked or floating panel and the dock actions they turn into.

use crate::docking::drop::{beside, members, moving_panels, takes, to_target};
use crate::docking::{DockEvent, DragUnit, FloatSpace};
use crate::runtime::commands::CommandState;
use craft_behavior::DockSide;
use craft_behavior::dock::{Action, Column, DockEdge, DockError, DockPlacement, Layout, Location, Node, PanelPlace, Permissions, Target, node_at};
use egui::{Rect, Vec2};
use std::hash::Hash;

/// The commands of a docked or floating panel. A panel command acts on one panel or on its tab group: the shown
/// panel of the dock group whose scope instance takes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, craft_ui_macros::CommandSet)]
#[command(prefix = "dock", scope = "dockGroup")]
#[non_exhaustive]
pub enum DockCommands {
    /// Float the panel into its own frame.
    #[command(label = "Float")]
    FloatPanel,
    /// Float the panel's tab group into its own frame.
    #[command(label = "Float Tab Group")]
    FloatGroup,
    /// Dock the floating panel: back where it was docked, else as a tab of the first docked group that takes it.
    #[command(label = "Dock Panel")]
    DockPanel,
    /// Ask for a group to move the panel to; the host lists [`Layout::move_targets`] and calls [`move_actions`].
    #[command(label = "Move Panel To…")]
    MovePanelTo,
    /// Ask for a group to move the panel's tab group to.
    #[command(label = "Move Group To…")]
    MoveGroupTo,
    /// Close the panel.
    #[command(label = "Close")]
    ClosePanel,
    /// Close the other panels of the panel's group.
    #[command(label = "Close Other Panels in Group")]
    CloseOthers,
    /// Close the panel's tab group.
    #[command(label = "Close Tab Group")]
    CloseGroup,
    /// Move the panel's group up its column.
    #[command(label = "Move Group Up")]
    StepGroupUp,
    /// Move the panel's group down its column.
    #[command(label = "Move Group Down")]
    StepGroupDown,
    /// Minimize the panel's group to its tab strip, or expand it.
    #[command(label = "Minimize", kind = "toggle")]
    ToggleMinimized,
    /// Collapse the panel's column to icons, or expand it.
    #[command(label = "Collapse to Icons", kind = "toggle")]
    CollapseToIcons,
    /// Maximize the panel's frame, or restore it.
    #[command(label = "Maximize Frame", kind = "toggle")]
    ToggleMaximize,
    /// Collapse iconic panels when focus leaves them, or stop doing so.
    #[command(label = "Auto-Collapse Iconic Panels", kind = "toggle")]
    AutoCollapse,
}

impl DockCommands {
    /// The unit a command that asks for a destination moves, `None` for the other commands.
    pub fn moves<P>(self, panel: &P) -> Option<DragUnit<P>>
    where
        P: Clone,
    {
        match self {
            DockCommands::MovePanelTo => Some(DragUnit::Panel(panel.clone())),
            DockCommands::MoveGroupTo => Some(DragUnit::Group(panel.clone())),
            _ => None,
        }
    }

    /// The event a command that only the host can carry out reports: Maximize Frame and Auto-Collapse.
    pub fn event<P>(self, panel: &P) -> Option<DockEvent<P>>
    where
        P: Clone,
    {
        match self {
            DockCommands::ToggleMaximize => Some(DockEvent::ToggleMaximize { panel: panel.clone() }),
            DockCommands::AutoCollapse => Some(DockEvent::ToggleAutoCollapse),
            _ => None,
        }
    }
}

/// The layout a command reads, with what it needs to decide what a drag would also decide: the float space, whether
/// the workspace is locked and what each panel allows.
pub struct DockScene<'a, P> {
    layout: &'a Layout<P>,
    space: &'a FloatSpace<P>,
    source: Option<Vec2>,
    locked: bool,
    permissions: &'a dyn Fn(&P) -> Permissions,
    floated: &'a [(P, Location<P>)],
}

impl<'a, P> DockScene<'a, P> {
    /// A scene over `layout`. `locked` is `WorkspaceSet::locked`; `permissions` is what the host passes to
    /// `Workspace::apply_all`.
    pub fn new(layout: &'a Layout<P>, space: &'a FloatSpace<P>, locked: bool, permissions: &'a dyn Fn(&P) -> Permissions) -> Self {
        DockScene { layout, space, source: None, locked, permissions, floated: &[] }
    }

    /// The docked places floating panels left (`Workspace::floated`), which Dock Panel returns them to.
    #[must_use]
    pub fn floated(mut self, records: &'a [(P, Location<P>)]) -> Self {
        self.floated = records;
        self
    }

    /// The size of the group or window the command runs from, which a float of it takes.
    #[must_use]
    pub fn source(mut self, size: Vec2) -> Self {
        self.source = Some(size);
        self
    }

    fn policy(&self, panel: &P) -> Permissions {
        let permissions = (self.permissions)(panel);
        if self.locked { permissions.locked() } else { permissions }
    }
}

/// Where a moved unit goes.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DockTo<P> {
    /// Into the group of `anchor` as tabs, before `before`, or at the end.
    #[non_exhaustive]
    Tabs {
        /// A panel of the destination group.
        anchor: P,
        /// The tab to go before.
        before: Option<P>,
    },
    /// Beside the group of `anchor`; [`DockSide::Center`] joins its tabs.
    #[non_exhaustive]
    Beside {
        /// A panel of the destination group.
        anchor: P,
        /// The side of the group.
        side: DockSide,
    },
    /// At the outer edge of the docked area on this side.
    Edge(DockSide),
    /// As a new last column of the dock on this window edge, or in a new dock there.
    Dock(DockEdge),
    /// Back to the docked layout: tabs of the first group outside the unit that takes them, as its rights and the
    /// unit's allow ([`Permissions::accepts_tabs`], [`Permissions::shares_column`]); dock columns come before the
    /// docked tree. Nothing for a docked unit.
    Docked,
}

impl<P> DockTo<P> {
    /// Into the group of `anchor` as tabs, before `before` or at the end.
    pub fn tabs(anchor: P, before: Option<P>) -> Self {
        DockTo::Tabs { anchor, before }
    }

    /// Beside the group of `anchor`, on `side`.
    pub fn beside(anchor: P, side: DockSide) -> Self {
        DockTo::Beside { anchor, side }
    }
}

/// The actions that run a panel command, empty for a command the host carries out ([`DockCommands::event`],
/// [`DockCommands::moves`]).
///
/// # Errors
///
/// A [`DockError`] when the panel is not in the layout or the command does not apply to where it is.
pub fn actions<P: Clone + Eq + Hash>(scene: &DockScene<'_, P>, command: DockCommands, panel: &P) -> Result<Vec<Action<P>>, DockError> {
    let layout = scene.layout;
    let shown = || if layout.contains(panel) { Ok(()) } else { Err(DockError::MissingPanel) };
    let one = |action: Action<P>| Ok(vec![action]);
    match command {
        DockCommands::FloatPanel => {
            shown()?;
            Ok(float_actions(scene, &DragUnit::Panel(panel.clone())))
        }
        DockCommands::FloatGroup => {
            shown()?;
            Ok(float_actions(scene, &DragUnit::Group(panel.clone())))
        }
        DockCommands::DockPanel => {
            shown()?;
            let back = scene.floated.iter().find(|(p, _)| p == panel).and_then(|(_, location)| {
                let anchor = location.anchor.clone()?;
                let docked = layout.place(&anchor).is_some_and(|place| !matches!(place, PanelPlace::Floating { .. }));
                docked.then(|| Action::restore(panel.clone(), location.clone()))
            });
            match back {
                Some(action) => one(action),
                None => move_actions(scene, &DragUnit::Panel(panel.clone()), DockTo::Docked),
            }
        }
        DockCommands::ClosePanel => one(Action::close(panel.clone())),
        DockCommands::CloseOthers => one(Action::close_others(panel.clone())),
        DockCommands::CloseGroup => one(Action::close_group(panel.clone())),
        DockCommands::StepGroupUp => one(Action::step_group(panel.clone(), true)),
        DockCommands::StepGroupDown => one(Action::step_group(panel.clone(), false)),
        DockCommands::ToggleMinimized => {
            shown()?;
            one(Action::set_minimized(panel.clone(), !minimized(layout, panel)))
        }
        DockCommands::CollapseToIcons => {
            let place = layout.place(panel).ok_or(DockError::MissingPanel)?;
            let column = layout.column(&place).ok_or(DockError::InvalidPlacement)?;
            if !column.iconic && !collapses(scene, column) {
                return Err(DockError::NotAllowed);
            }
            one(Action::set_iconic(panel.clone(), !column.iconic))
        }
        DockCommands::MovePanelTo | DockCommands::MoveGroupTo | DockCommands::ToggleMaximize | DockCommands::AutoCollapse => Ok(Vec::new()),
    }
}

/// The actions that move `unit` to `to`.
///
/// # Errors
///
/// A [`DockError`] when the unit's panel is not in the layout, or the destination has no anchor outside the unit.
pub fn move_actions<P: Clone + Eq + Hash>(scene: &DockScene<'_, P>, unit: &DragUnit<P>, to: DockTo<P>) -> Result<Vec<Action<P>>, DockError> {
    let layout = scene.layout;
    let panel = unit.lead();
    if !layout.contains(panel) {
        return Err(DockError::MissingPanel);
    }
    let moved = moving_panels(layout, unit);
    let one = |action: Action<P>| Ok(vec![action]);
    match to {
        DockTo::Tabs { anchor, before } => one(beside(unit, anchor, DockPlacement::tab(before))),
        DockTo::Beside { anchor, side } => {
            let placement = DockPlacement::for_zone(layout, &anchor, side)?;
            one(beside(unit, anchor, placement))
        }
        DockTo::Edge(side) => {
            if side == DockSide::Center {
                return Err(DockError::InvalidPlacement);
            }
            let protected = |p: &P| scene.policy(p).is_protected();
            let skip = |p: &P| moved.contains(p);
            let anchor = layout.edge_anchor(layout.edge_narrows(side, &protected), &protected, &skip).ok_or(DockError::MissingTarget)?;
            one(beside(unit, anchor, DockPlacement::Edge(side)))
        }
        DockTo::Dock(edge) => match last_column(layout, edge) {
            Some(column) => match column.into_iter().find(|p| !moved.contains(p)) {
                Some(anchor) => one(beside(unit, anchor, DockPlacement::column(true))),
                // What moves already is that dock's last column.
                None => Ok(Vec::new()),
            },
            None => {
                let alone = matches!(unit, DragUnit::Panel(_)) && members(layout, panel).len() == 1;
                let unit = if alone { DragUnit::Group(panel.clone()) } else { unit.clone() };
                one(to_target(&unit, Target::new_dock(edge)))
            }
        },
        DockTo::Docked => {
            if !matches!(layout.place(panel), Some(PanelPlace::Floating { .. })) {
                return Ok(Vec::new());
            }
            let panels = layout.panels();
            let free = || panels.iter().copied().filter(|p| !moved.contains(p));
            let docked = free().filter(|p| matches!(layout.place(p), Some(PanelPlace::Dock { .. })));
            let framed = free().filter(|p| matches!(layout.place(p), Some(PanelPlace::Node(_))));
            let mut joins = docked.chain(framed).map(|anchor| beside(unit, anchor.clone(), DockPlacement::tab(None))).peekable();
            if joins.peek().is_none() {
                return Err(DockError::MissingTarget);
            }
            let permissions = |p: &P| scene.policy(p);
            joins.find(|action| takes(layout, action, &permissions)).map_or(Err(DockError::NotAllowed), one)
        }
    }
}

/// Whether the group or accordion section of `panel` is minimized to its tab strip. An accordion entry is minimized
/// while it is closed. A panel the layout does not hold is not.
fn minimized<P: Clone + Eq + Hash>(layout: &Layout<P>, panel: &P) -> bool {
    let Some(place) = layout.place(panel) else { return false };
    match layout.group(&place) {
        Some(group) => group.minimized,
        None => match layout.node_of(&place) {
            Some(Node::Stack { entries, .. }) => entries.iter().any(|entry| &entry.panel == panel && !entry.open),
            _ => false,
        },
    }
}

/// Whether `column` may collapse to icons: every panel in it collapses ([`Permissions::collapses`]).
fn collapses<P: Clone + Eq + Hash>(scene: &DockScene<'_, P>, column: &Column<P>) -> bool {
    column.panels().all(|panel| scene.policy(panel).collapses)
}

/// The state of each panel command for `panel`: enabled as its permissions and place allow. Collapse to Icons applies
/// to every column, docked or floating, checked while it is iconic, and is off for a column that holds a panel that
/// does not collapse. Maximize Frame applies to a frame of the docked layout, never to a floating one. `DockArea`
/// publishes them in the panel's group scope.
pub fn dock_states<P: Clone + Eq + Hash>(scene: &DockScene<'_, P>, panel: &P) -> Vec<(DockCommands, CommandState)> {
    let layout = scene.layout;
    let policy = |p: &P| scene.policy(p);
    let allowed = scene.policy(panel);
    let place = layout.place(panel);
    let floatable = place.is_some() && allowed.movable && allowed.floatable;
    let column = place.as_ref().and_then(|place| layout.column(place));
    let iconic = column.map(|column| column.iconic);
    let collapsible = column.is_some_and(|column| column.iconic || collapses(scene, column));
    let enabled = |on: bool| CommandState::new().enabled(on);
    let shown = place.is_some();
    let floating = matches!(place, Some(PanelPlace::Floating { .. }));
    let movable = allowed.movable && !layout.move_targets(panel, policy).is_empty();
    vec![
        (DockCommands::FloatPanel, enabled(floatable)),
        (DockCommands::FloatGroup, enabled(floatable)),
        (
            DockCommands::DockPanel,
            enabled(
                floating && allowed.movable && layout.panels().into_iter().any(|p| !matches!(layout.place(p), Some(PanelPlace::Floating { .. }))),
            ),
        ),
        (DockCommands::MovePanelTo, enabled(movable)),
        (DockCommands::MoveGroupTo, enabled(movable)),
        (DockCommands::ClosePanel, enabled(shown && allowed.closable)),
        (DockCommands::CloseOthers, enabled(allowed.closable && members(layout, panel).len() > 1)),
        (DockCommands::CloseGroup, enabled(shown && allowed.closable)),
        (DockCommands::StepGroupUp, enabled(layout.can_step_group(panel, true, policy))),
        (DockCommands::StepGroupDown, enabled(layout.can_step_group(panel, false, policy))),
        (DockCommands::ToggleMinimized, enabled(shown)),
        (DockCommands::CollapseToIcons, CommandState::new().enabled(collapsible).checked(iconic.unwrap_or(false))),
        (DockCommands::ToggleMaximize, enabled(shown && !floating)),
        (DockCommands::AutoCollapse, enabled(true)),
    ]
}

/// The actions that float `unit` at the rect a drag would float it at, or keep its window where it is: a window's
/// only panel, and a window's only group, already float alone.
fn float_actions<P: Clone + Eq + Hash>(scene: &DockScene<'_, P>, unit: &DragUnit<P>) -> Vec<Action<P>> {
    let layout = scene.layout;
    let panel = unit.lead();
    let window = layout.floating.iter().find(|window| window.contains(panel));
    let lone = window.is_some_and(|window| window.panels().count() == 1);
    let whole = window.is_some_and(|window| window.groups().len() == 1);
    let kept = window.map(|window| window.rect);
    let action = match (unit, kept) {
        (DragUnit::Panel(_), Some(rect)) if lone => Action::move_floating(panel.clone(), rect),
        (DragUnit::Group(_), Some(rect)) if whole => to_target(unit, Target::float(rect)),
        (DragUnit::Panel(_), _) => Action::float(panel.clone(), placed(scene, unit)),
        _ => to_target(unit, Target::float(placed(scene, unit))),
    };
    vec![action]
}

/// The rect a command floats `unit` at, inside the float area: the size [`FloatSpace::size_of`] gives it, from the
/// size of the group the command runs from.
fn placed<P: Clone + Eq + Hash>(scene: &DockScene<'_, P>, unit: &DragUnit<P>) -> Rect {
    scene.space.place(scene.space.size_of(unit, scene.source))
}

/// The panels of the last column of the dock on `edge`.
fn last_column<P: Clone + Eq + Hash>(layout: &Layout<P>, edge: DockEdge) -> Option<Vec<P>> {
    let path = layout.dock_path(edge)?;
    match node_at(layout.root.as_ref()?, &path)? {
        Node::Dock { columns, .. } => Some(columns.last()?.groups.iter().flat_map(|group| group.panels.iter().cloned()).collect()),
        _ => None,
    }
}
