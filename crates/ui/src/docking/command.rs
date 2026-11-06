//! The shared docking command vocabulary (feature `serde`).
//!
//! Applications deserialize a control command's JSON params, with its id under `"command"`,
//! into [`DockCommand`] and apply it. craft-ui does not depend on `serde_json`.
//!
//! ```ignore
//! let mut params = params.clone();
//! params["command"] = json!(id);
//! let command: DockCommand<PanelId> = serde_json::from_value(params)?;
//! let space = float_space(ctx, DOCK_ID).unwrap_or_else(|| FloatSpace::new(ctx.content_rect()));
//! command.apply(&mut app.workspace, style.right_to_left, &space, |p| app.permissions(p))?;
//! ```

use std::hash::Hash;

use serde::{Deserialize, Deserializer, Serialize};

use super::{
    Action, DockError, FloatSpace, Layout, Permissions, Placement, Side, Target, Workspace, Zone, dock_model::Place, surface::DEFAULT_FLOAT,
};
use crate::layout::SplitSize;

/// A docking command, tagged by its control id under `"command"`.
///
/// Ids and params (`?` marks optional ones):
///
/// | Id | Params |
/// |---|---|
/// | `window.panel.layout` | `{action}`: a serialized [`Action`] |
/// | `window.panel.move` | `{panel, anchor, zone?, before?, group?, placement?}` |
/// | `window.panel.float` | `{panel, x?, y?, width?, height?, group?, column?}` |
/// | `window.panel.dock` | `{panel, anchor?, zone?, before?, group?, column?, side?}` |
/// | `window.panel.activate`, `.close`, `.closeGroup`, `.closeOthers` | `{panel}` |
/// | `window.panel.minimize` | `{panel, minimized?}`; without `minimized` it toggles |
/// | `window.panel.iconic` | `{panel, iconic?}`; without `iconic` it toggles |
/// | `window.panel.window` | `{panel, native?}` (desktop) |
/// | `window.workspace`, `.new`, `.delete` | `{name}` |
/// | `window.workspace.reset`, `ui.dock.reset` | `{}` |
/// | `window.workspace.lock` | `{locked?}`; without `locked` it toggles |
/// | `ui.dock` | `{action}`, or `{operation, …}`, see [`DockOperation`] |
///
/// `zone` is `tab` (the default for `move` and `dock`), `center`, `left`, `right`, `top`,
/// `bottom`, `above` or `below` (a new group in a dock column, or a split in a floating window). The side
/// zones map onto the anchor's container with [`Placement::for_zone`]. `before`
/// names the tab to insert before; `null` appends. `group` moves the panel's whole group,
/// `column` its dock column. `side` is `Start` or `End`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "command", bound(serialize = "P: Serialize", deserialize = "P: Deserialize<'de>"))]
#[non_exhaustive]
pub enum DockCommand<P> {
    #[serde(rename = "window.panel.layout")]
    Layout { action: Action<P> },
    #[serde(rename = "window.panel.move")]
    Move {
        panel: P,
        anchor: P,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        zone: Option<ZoneName>,
        #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
        before: Option<Option<P>>,
        #[serde(default)]
        group: bool,
        /// A serialized [`Placement`]; it takes precedence over `zone` and `before`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        placement: Option<Placement<P>>,
    },
    #[serde(rename = "window.panel.float")]
    Float {
        panel: P,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        x: Option<f32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        y: Option<f32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        width: Option<f32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        height: Option<f32>,
        #[serde(default)]
        group: bool,
        #[serde(default)]
        column: bool,
    },
    /// To `anchor`; else to a new column of the dock on `side` (a new dock when it has none);
    /// else with a `left`, `right`, `top` or `bottom` zone to a new frame on that outer edge
    /// ([`Placement::Edge`]); else a floating panel joins the first dock group (or frame) as a
    /// tab, and a docked one stays. A hidden panel without `anchor` and `side` is shown at its
    /// last location, then moved to the edge a zone names.
    #[serde(rename = "window.panel.dock")]
    Dock {
        panel: P,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        anchor: Option<P>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        zone: Option<ZoneName>,
        #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
        before: Option<Option<P>>,
        #[serde(default)]
        group: bool,
        #[serde(default)]
        column: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        side: Option<Side>,
    },
    /// Select a panel; a hidden one is shown at its last location first.
    #[serde(rename = "window.panel.activate")]
    Activate { panel: P },
    #[serde(rename = "window.panel.close")]
    Close { panel: P },
    #[serde(rename = "window.panel.closeGroup")]
    CloseGroup { panel: P },
    #[serde(rename = "window.panel.closeOthers")]
    CloseOthers { panel: P },
    #[serde(rename = "window.panel.minimize")]
    Minimize {
        panel: P,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        minimized: Option<bool>,
    },
    #[serde(rename = "window.panel.iconic")]
    Iconic {
        panel: P,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        iconic: Option<bool>,
    },
    /// Move the panel's floating window to its own OS window (`native`, the default) or back.
    /// A docked panel's group floats first.
    #[serde(rename = "window.panel.window")]
    Window {
        panel: P,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        native: Option<bool>,
    },
    /// Switch to a named workspace. The application owns named workspaces.
    #[serde(rename = "window.workspace")]
    Workspace { name: String },
    #[serde(rename = "window.workspace.new")]
    NewWorkspace { name: String },
    #[serde(rename = "window.workspace.delete")]
    DeleteWorkspace { name: String },
    /// Reset the current workspace to its saved or built-in arrangement (application-owned).
    #[serde(rename = "window.workspace.reset", alias = "ui.dock.reset")]
    ResetWorkspace {},
    #[serde(rename = "window.workspace.lock")]
    Lock {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        locked: Option<bool>,
    },
    /// The `ui.dock` shape of LightCraft, PDFCraft, CADCraft and SoundCraft.
    #[serde(rename = "ui.dock")]
    UiDock(UiDock<P>),
}

/// `ui.dock` params: a raw action, or an operation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged, bound(serialize = "P: Serialize", deserialize = "P: Deserialize<'de>"))]
pub enum UiDock<P> {
    Action { action: Action<P> },
    Operation(DockOperation<P>),
}

/// A `ui.dock` operation, tagged by `operation`. `move` takes `target` (the anchor), `zone`
/// (default `center`) and `before`, which only applies to a center move.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "camelCase", bound(serialize = "P: Serialize", deserialize = "P: Deserialize<'de>"))]
#[non_exhaustive]
pub enum DockOperation<P> {
    /// Show a panel at its last location and select it.
    Open {
        panel: P,
    },
    Close {
        panel: P,
    },
    Activate {
        panel: P,
    },
    Float {
        panel: P,
        rect: [f32; 4],
    },
    MoveFloating {
        panel: P,
        rect: [f32; 4],
    },
    Move {
        panel: P,
        target: P,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        zone: Option<ZoneName>,
        #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
        before: Option<Option<P>>,
    },
    SetStackOpen {
        panel: P,
        open: bool,
    },
    /// `height` is required; `null` clears it.
    ResizeStack {
        panel: P,
        #[serde(deserialize_with = "required")]
        height: Option<f32>,
    },
    ResizeSplit {
        path: Vec<bool>,
        size: SplitSize,
    },
}

/// A drop zone by name, as control protocols write it. PascalCase [`Zone`] names also parse.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ZoneName {
    Tab,
    #[serde(alias = "Center")]
    Center,
    #[serde(alias = "Left")]
    Left,
    #[serde(alias = "Right")]
    Right,
    #[serde(alias = "Top")]
    Top,
    #[serde(alias = "Bottom")]
    Bottom,
    Above,
    Below,
}

/// A present field, `null` included: `Some(None)` for `null`, `None` when absent.
fn present<'de, D: Deserializer<'de>, T: Deserialize<'de>>(deserializer: D) -> Result<Option<T>, D::Error> {
    T::deserialize(deserializer).map(Some)
}

/// A required field that may be `null`.
fn required<'de, D: Deserializer<'de>, T: Deserialize<'de>>(deserializer: D) -> Result<T, D::Error> {
    T::deserialize(deserializer)
}

/// Where a command puts what it moves relative to `anchor`: `zone` and `before`, with side
/// zones mapped by [`Placement::for_zone`]. `default` is the zone when none is given.
struct Zoned<'a, P> {
    layout: &'a Layout<P>,
    right_to_left: bool,
}

impl<P: Clone + Eq + Hash> Zoned<'_, P> {
    fn placement(&self, anchor: &P, zone: Option<ZoneName>, before: &Option<Option<P>>, default: ZoneName) -> Result<Placement<P>, DockError> {
        let side = |zone| Placement::for_zone(self.layout, anchor, zone, self.right_to_left);
        Ok(match (zone.unwrap_or(default), before.clone()) {
            (ZoneName::Tab | ZoneName::Center, Some(before)) => Placement::Tab { before },
            (ZoneName::Tab, None) => Placement::Tab { before: None },
            (ZoneName::Center, None) => Placement::Split(Zone::Center),
            (_, Some(_)) => return Err(DockError::InvalidPlacement),
            (ZoneName::Left, None) => side(Zone::Left)?,
            (ZoneName::Right, None) => side(Zone::Right)?,
            (ZoneName::Top, None) => side(Zone::Top)?,
            (ZoneName::Bottom, None) => side(Zone::Bottom)?,
            (ZoneName::Above, None) => Placement::Group { below: false, height: None },
            (ZoneName::Below, None) => Placement::Group { below: true, height: None },
        })
    }
}

/// How much of the layout a command moves.
#[derive(Clone, Copy)]
enum Unit {
    Panel,
    Group,
    Column,
}

impl Unit {
    fn of(group: bool, column: bool) -> Self {
        if column {
            Self::Column
        } else if group {
            Self::Group
        } else {
            Self::Panel
        }
    }

    /// The action that moves this unit of `panel` to `to`. A floating panel's column is its
    /// whole window.
    fn to<P: Clone + Eq + Hash>(self, layout: &Layout<P>, panel: &P, to: Target<P>) -> Result<Action<P>, DockError> {
        let floating = matches!(layout.place(panel), Some(Place::Floating { .. }));
        let panel = panel.clone();
        Ok(match (self, to) {
            (Self::Panel, Target::At { anchor, placement }) => Action::Move { panel, anchor, placement },
            (Self::Panel, to) => Action::MovePanel { panel, to },
            (Self::Group, to) => Action::MoveGroup { panel, to },
            (Self::Column, to) if floating => Action::MoveWindow { panel, to },
            (Self::Column, to) => Action::MoveColumn { panel, to },
        })
    }
}

impl<P> DockCommand<P> {
    /// Every command id, for an application's command registry.
    pub const IDS: &'static [&'static str] = &[
        "window.panel.layout",
        "window.panel.move",
        "window.panel.float",
        "window.panel.dock",
        "window.panel.activate",
        "window.panel.close",
        "window.panel.closeGroup",
        "window.panel.closeOthers",
        "window.panel.minimize",
        "window.panel.iconic",
        "window.panel.window",
        "window.workspace",
        "window.workspace.new",
        "window.workspace.delete",
        "window.workspace.reset",
        "window.workspace.lock",
        "ui.dock",
        "ui.dock.reset",
    ];

    /// Named workspaces and Reset belong to the application, which handles these commands
    /// before [`Self::apply`].
    pub fn is_app_owned(&self) -> bool {
        matches!(self, Self::Workspace { .. } | Self::NewWorkspace { .. } | Self::DeleteWorkspace { .. } | Self::ResetWorkspace {})
    }
}

impl<P: Clone + Eq + Hash> DockCommand<P> {
    /// The layout actions this command makes on `workspace`, to apply all or none.
    /// Showing a hidden panel and Lock change more than the layout, and named workspaces
    /// belong to the application: those return [`DockError::NotAllowed`]; use [`Self::apply`].
    /// A panel or anchor that is not shown returns [`DockError::MissingPanel`] or
    /// [`DockError::MissingTarget`].
    ///
    /// `left`, `right`, `top` and `bottom` zones follow [`Placement::for_zone`]: beside a docked
    /// anchor they make a new column or group, and `right_to_left` is the
    /// [`super::DockStyle::right_to_left`] the layout is drawn with.
    ///
    /// `space` is where `window.panel.float` floats what it moves when it gives no rect: at the
    /// size a drag would float it at, inside the dock area and clear of the app's menu bar. Pass
    /// [`super::float_space`] of the area that draws `workspace`. Given `x`, `y`, `width` and
    /// `height` replace the matching values, and a group or column that is already a whole
    /// floating window keeps its rect.
    ///
    /// An outer edge dock wraps the part between the docks. [`Self::to_actions_with`] also
    /// knows which panels are protected, so it wraps the canvas column where a drag would.
    pub fn to_actions(&self, workspace: &Workspace<P>, right_to_left: bool, space: &FloatSpace<P>) -> Result<Vec<Action<P>>, DockError> {
        self.to_actions_with(workspace, right_to_left, space, |_| Permissions::default())
    }

    /// [`Self::to_actions`] with the `permissions` the actions are applied with. An outer edge
    /// dock wraps what a drag to that edge would: the canvas column where the area narrows its
    /// edge band to it, else the part between the docks. `space` says whether the area draws
    /// with [`super::DockStyle::side_edge_zones`], which never narrows.
    pub fn to_actions_with(
        &self,
        workspace: &Workspace<P>,
        right_to_left: bool,
        space: &FloatSpace<P>,
        permissions: impl Fn(&P) -> Permissions,
    ) -> Result<Vec<Action<P>>, DockError> {
        let layout = &workspace.layout;
        let zoned = Zoned { layout, right_to_left };
        let shown = |panel: &P| if layout.contains(panel) { Ok(()) } else { Err(DockError::MissingPanel) };
        let one = |action: Action<P>| Ok(vec![action]);
        match self {
            Self::Layout { action } | Self::UiDock(UiDock::Action { action }) => one(action.clone()),
            Self::Move { panel, anchor, zone, before, group, placement: given } => {
                shown(panel)?;
                let placement = match given {
                    Some(placement) => placement.clone(),
                    None => zoned.placement(anchor, *zone, before, ZoneName::Tab)?,
                };
                one(Unit::of(*group, false).to(layout, panel, Target::At { anchor: anchor.clone(), placement })?)
            }
            Self::Float { panel, x, y, width, height, group, column } => {
                shown(panel)?;
                let given = [*x, *y, *width, *height];
                let window = layout.floating.iter().find(|w| w.contains(panel)).map(|w| w.rect);
                let rect = |base: [f32; 4]| {
                    let mut rect = base;
                    for (value, given) in rect.iter_mut().zip(given) {
                        *value = given.unwrap_or(*value);
                    }
                    rect
                };
                let any = given.iter().any(Option::is_some);
                let unit = Unit::of(*group, *column);
                let lone = layout.floating.iter().any(|w| w.contains(panel) && w.panels().count() == 1);
                let whole =
                    layout.floating.iter().any(|w| w.contains(panel) && matches!(w.root, super::Node::Tabs { .. } | super::Node::Stack { .. }));
                // The rect a drag would float the unit at, inside the area.
                let placed = || {
                    let docked = layout.location(panel).ok().and_then(|l| l.group).and_then(|g| g.column).map(|c| c.width);
                    let fallback = egui::vec2(docked.filter(|_| !matches!(unit, Unit::Panel)).unwrap_or(DEFAULT_FLOAT.x), DEFAULT_FLOAT.y);
                    let size = match unit {
                        Unit::Panel => space.panel(panel),
                        Unit::Group => space.group(panel),
                        Unit::Column => space.column(panel),
                    };
                    rect(space.place(size.unwrap_or(fallback)))
                };
                match (unit, window) {
                    (Unit::Panel, Some(base)) if lone => one(Action::MoveFloating { panel: panel.clone(), rect: rect(base) }),
                    (Unit::Column, Some(base)) => one(Action::MoveFloating { panel: panel.clone(), rect: rect(base) }),
                    // Without given values the window keeps its rect and native state.
                    (Unit::Group, Some(base)) if whole => one(unit.to(layout, panel, Target::Float { rect: any.then(|| rect(base)) })?),
                    (Unit::Panel, _) => one(Action::Float { panel: panel.clone(), rect: placed() }),
                    (unit, _) => one(unit.to(layout, panel, Target::Float { rect: Some(placed()) })?),
                }
            }
            Self::Dock { panel, anchor, zone, before, group, column, side } => {
                shown(panel)?;
                let unit = Unit::of(*group, *column);
                if let Some(anchor) = anchor {
                    let placement = zoned.placement(anchor, *zone, before, ZoneName::Tab)?;
                    return one(unit.to(layout, panel, Target::At { anchor: anchor.clone(), placement })?);
                }
                if let Some(side) = side {
                    let to = match last_column(layout, *side) {
                        Some(column) => match column.into_iter().find(|p| !members(layout, panel, unit).contains(p)) {
                            Some(anchor) => Target::At { anchor, placement: Placement::Column { after: true } },
                            // What moves already is that dock's last column.
                            None => return Ok(Vec::new()),
                        },
                        None => Target::NewDock { side: *side },
                    };
                    let unit = match (unit, &to) {
                        (Unit::Panel, Target::NewDock { .. }) if group_len(layout, panel) == Some(1) => Unit::Group,
                        (unit, _) => unit,
                    };
                    return one(unit.to(layout, panel, to)?);
                }
                let members = members(layout, panel, unit);
                if let Some(zone) = self.edge() {
                    if before.is_some() {
                        return Err(DockError::InvalidPlacement);
                    }
                    // The model narrows the edge on a protected anchor, as `Workspace::apply` checks it.
                    let protected = |p: &P| {
                        let policy = if workspace.locked { permissions(p).locked() } else { permissions(p) };
                        policy.is_protected()
                    };
                    let narrowed = !space.side_edge_zones && layout.edge_narrows(zone, &protected);
                    let anchor = layout.edge_anchor(narrowed, &protected, &|p| members.contains(p)).ok_or(DockError::MissingTarget)?;
                    return one(unit.to(layout, panel, Target::At { anchor, placement: Placement::Edge(zone) })?);
                }
                if !matches!(layout.place(panel), Some(Place::Floating { .. })) {
                    return Ok(Vec::new());
                }
                let panels = layout.panels();
                let free = || panels.iter().copied().filter(|p| !members.contains(p));
                let docked = free().find(|p| matches!(layout.place(p), Some(Place::Dock { .. })));
                let framed = || free().find(|p| matches!(layout.place(p), Some(Place::Node(_))));
                let anchor = docked.or_else(framed).cloned().ok_or(DockError::MissingTarget)?;
                one(unit.to(layout, panel, Target::At { anchor, placement: Placement::Tab { before: None } })?)
            }
            Self::Activate { panel } => {
                shown(panel)?;
                one(Action::Activate { panel: panel.clone() })
            }
            Self::Close { panel } => one(Action::Close { panel: panel.clone() }),
            Self::CloseGroup { panel } => one(Action::CloseGroup { panel: panel.clone() }),
            Self::CloseOthers { panel } => one(Action::CloseOthers { panel: panel.clone() }),
            Self::Minimize { panel, minimized } => {
                let place = layout.place(panel).ok_or(DockError::MissingPanel)?;
                // An accordion entry is minimized while it is closed.
                let current = match &place {
                    Place::Node(path) => match layout.root.as_ref().and_then(|root| super::dock_model::node_at(root, path)) {
                        Some(super::Node::Stack { entries }) => entries.iter().any(|e| &e.panel == panel && !e.open),
                        _ => false,
                    },
                    Place::Floating { window, .. } => layout.floating.get(*window).is_some_and(|w| w.minimized),
                    place => layout.group(place).is_some_and(|g| g.minimized),
                };
                one(Action::SetMinimized { panel: panel.clone(), minimized: minimized.unwrap_or(!current) })
            }
            Self::Iconic { panel, iconic } => {
                let location = layout.location(panel)?;
                let current = location.group.and_then(|g| g.column).is_some_and(|c| c.iconic);
                one(Action::SetIconic { panel: panel.clone(), iconic: iconic.unwrap_or(!current) })
            }
            Self::Window { panel, native } => {
                shown(panel)?;
                if !native.unwrap_or(true) {
                    return one(Action::SetNative { panel: panel.clone(), rect: None });
                }
                let mut actions = Vec::new();
                let mut scratch = layout.clone();
                if !scratch.floating.iter().any(|w| w.contains(panel)) {
                    let float = Action::MoveGroup { panel: panel.clone(), to: Target::Float { rect: None } };
                    scratch.apply(float.clone())?;
                    actions.push(float);
                }
                let rect = scratch.floating.iter().find(|w| w.contains(panel)).map(|w| w.rect).ok_or(DockError::MissingPanel)?;
                actions.push(Action::SetNative { panel: panel.clone(), rect: Some(rect) });
                Ok(actions)
            }
            Self::UiDock(UiDock::Operation(operation)) => match operation {
                DockOperation::Open { panel } | DockOperation::Activate { panel } => {
                    shown(panel)?;
                    one(Action::Activate { panel: panel.clone() })
                }
                DockOperation::Close { panel } => one(Action::Close { panel: panel.clone() }),
                DockOperation::Float { panel, rect } => one(Action::Float { panel: panel.clone(), rect: *rect }),
                DockOperation::MoveFloating { panel, rect } => one(Action::MoveFloating { panel: panel.clone(), rect: *rect }),
                DockOperation::Move { panel, target, zone, before } => {
                    let placement = zoned.placement(target, *zone, before, ZoneName::Center)?;
                    one(Action::Move { panel: panel.clone(), anchor: target.clone(), placement })
                }
                DockOperation::SetStackOpen { panel, open } => one(Action::SetStackOpen { panel: panel.clone(), open: *open }),
                DockOperation::ResizeStack { panel, height } => one(Action::ResizeStack { panel: panel.clone(), height: *height }),
                DockOperation::ResizeSplit { path, size } => one(Action::ResizeSplit { path: path.clone(), size: *size }),
            },
            Self::Workspace { .. } | Self::NewWorkspace { .. } | Self::DeleteWorkspace { .. } | Self::ResetWorkspace {} | Self::Lock { .. } => {
                Err(DockError::NotAllowed)
            }
        }
    }

    /// Apply the command to `workspace`, all or nothing, and return the panels it closed so
    /// the application can release their state. Showing a hidden panel restores its last
    /// location, and a panel never laid out opens as [`Action::Open`] without an anchor does
    /// ([`Workspace::set_visible`]); Lock sets [`Workspace::locked`]. Application-owned commands
    /// ([`Self::is_app_owned`]) return [`DockError::NotAllowed`] and change nothing.
    /// `space` is where `window.panel.float` floats, as for [`Self::to_actions_with`].
    pub fn apply(
        &self,
        workspace: &mut Workspace<P>,
        right_to_left: bool,
        space: &FloatSpace<P>,
        permissions: impl Fn(&P) -> Permissions,
    ) -> Result<Vec<P>, DockError> {
        let show = match self {
            Self::Activate { panel }
            | Self::Dock { panel, anchor: None, side: None, .. }
            | Self::UiDock(UiDock::Operation(DockOperation::Open { panel } | DockOperation::Activate { panel })) => Some(panel),
            _ => None,
        };
        if let Some(panel) = show.filter(|p| !workspace.layout.contains(p)) {
            if self.edge().is_none() {
                workspace.set_visible(panel.clone(), true, None, &permissions)?;
                return Ok(Vec::new());
            }
            // Shown, then moved to the edge, all or nothing.
            let mut shown = workspace.clone();
            shown.set_visible(panel.clone(), true, None, &permissions)?;
            let closed = shown.apply_all(self.to_actions_with(&shown, right_to_left, space, &permissions)?, &permissions)?;
            *workspace = shown;
            return Ok(closed);
        }
        if let Self::Lock { locked } = self {
            workspace.locked = locked.unwrap_or(!workspace.locked);
            return Ok(Vec::new());
        }
        let actions = self.to_actions_with(workspace, right_to_left, space, &permissions)?;
        workspace.apply_all(actions, permissions)
    }
}

impl<P> DockCommand<P> {
    /// The outer edge `window.panel.dock` goes to: a side zone without an anchor or side.
    fn edge(&self) -> Option<Zone> {
        match self {
            Self::Dock { anchor: None, side: None, zone: Some(zone), .. } => match zone {
                ZoneName::Left => Some(Zone::Left),
                ZoneName::Right => Some(Zone::Right),
                ZoneName::Top => Some(Zone::Top),
                ZoneName::Bottom => Some(Zone::Bottom),
                _ => None,
            },
            _ => None,
        }
    }
}

/// The panels of the last column of the dock on `side`, or `None` without such a dock.
fn last_column<P: Clone + Eq + Hash>(layout: &Layout<P>, side: Side) -> Option<Vec<P>> {
    let path = layout.dock_path(side)?;
    let node = layout.root.as_ref().and_then(|root| super::dock_model::node_at(root, &path))?;
    match node {
        super::Node::Dock { columns, .. } => Some(columns.last()?.groups.iter().flat_map(|g| g.panels.iter().cloned()).collect()),
        _ => None,
    }
}

fn group_len<P: Clone + Eq + Hash>(layout: &Layout<P>, panel: &P) -> Option<usize> {
    layout.members(panel, super::dock_model::Unit::Group).ok().map(|members| members.len())
}

/// The panels that move with `panel`.
fn members<P: Clone + Eq + Hash>(layout: &Layout<P>, panel: &P, unit: Unit) -> Vec<P> {
    match unit {
        Unit::Panel => vec![panel.clone()],
        Unit::Group => layout.members(panel, super::dock_model::Unit::Group).unwrap_or_default(),
        Unit::Column => match layout.place(panel) {
            Some(Place::Dock { path, column, .. }) => match layout.root.as_ref().and_then(|root| super::dock_model::node_at(root, &path)) {
                Some(super::Node::Dock { columns, .. }) => {
                    columns.get(column).map(|c| c.groups.iter().flat_map(|g| g.panels.iter().cloned()).collect()).unwrap_or_default()
                }
                _ => Vec::new(),
            },
            Some(Place::Floating { .. }) => {
                layout.floating.iter().find(|w| w.contains(panel)).map(|w| w.panels().cloned().collect()).unwrap_or_default()
            }
            _ => vec![panel.clone()],
        },
    }
}
