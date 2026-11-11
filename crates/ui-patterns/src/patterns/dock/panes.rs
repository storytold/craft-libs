//! `PaneArea`: task panes around a document. Docked panes are shell panes on the area's edges, floating panes are
//! floating frames, and the app keeps which panes are open and where they are.
//!
//! Drops. A pane area has no dock `Layout`, so it does not register `DockTargets` spots: a spot resolves to a layout
//! `Action`, and the area's drops resolve to [`PaneRequest`]s on the app's own pane list. The area's edge bands are
//! the one `craft_behavior::dock::zones` function over the area at the docking `edge_band` size (the dock's own
//! `frame::edge_bands` additionally follows the layout's groups), and the drop marks and the drop-over-a-pane halves
//! follow the pane order the app keeps.

use crate::contract::SizeLimits;
use crate::contract::overlay::{PlacementChange, PlacementKind, Popup};
use crate::contract::{
    BodyCx, Common, Component, Container, ContainerProps, Cx, DropMark, DropOperation, DropTarget, HeaderRegion, Mount, NoValue, Outcome, Pad,
    PaneEdge, Pointer as _, Props as _, Remember as _, Role, Scroll, ShellPane, Show as _, ShowContainer as _, Sizing, WordSetCx as _,
};
use crate::display::Divider;
use crate::docking::{DockRoot, DockWords, DragUnit, FloatingGroup, GroupDrag, PanelKey};
use crate::menu::MenuItem;
use crate::runtime::commands::{CommandSet as _, CommandState, Commands as _};
use crate::runtime::env::Label;
use crate::runtime::focus::FocusPolicy;
use crate::runtime::windows::ResizeEdge;
use crate::sections::{HeaderAction, HeaderStrength, MenuRows, TitleRow};
use crate::style::{ComponentStyle, StyleCx, SurfaceKind};
use craft_behavior::dock::{ZoneSize, zones};
use craft_behavior::{DockSide, GesturePhase, Metrics, Sense};
use egui::{Id, Pos2, Rect, Response, Ui, Vec2};
use std::cell::RefCell;
use std::fmt::Debug;
use std::hash::Hash;

#[cfg(feature = "testing")]
mod stories;

/// Where a pane is shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PanePlace {
    /// Docked to an edge: `Start`, `End`, `Top` or `Bottom`. `Center` is not an edge of the area; such a pane is not
    /// shown.
    Docked(PaneEdge),
    /// In a floating frame.
    Floating,
}

/// The layout of one pane. The app keeps it where it keeps its preferences and gives it back through
/// [`PaneContent::state`].
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct PaneState {
    /// Docked to an edge or floating.
    pub place: PanePlace,
    /// The width of a start or end pane, or the height of a top or bottom pane. `None`: the pane keeps the size it
    /// first took.
    pub size: Option<f32>,
    /// The floating frame's rect, also kept while the pane is docked. `None`: the frame opens at the pane's float
    /// size.
    pub float_rect: Option<Rect>,
    /// A floating pane shows its title row only. Kept while the pane is docked, where it has no effect.
    pub minimized: bool,
}

impl PaneState {
    /// A pane docked to `edge`, `size` along the edge, with no remembered floating rect.
    pub const fn docked(edge: PaneEdge, size: f32) -> Self {
        PaneState { place: PanePlace::Docked(edge), size: Some(size), float_rect: None, minimized: false }
    }

    /// A pane floating in `rect`.
    pub const fn floating(rect: Rect) -> Self {
        PaneState { place: PanePlace::Floating, size: None, float_rect: Some(rect), minimized: false }
    }

    /// The state after `request`, for an app that keeps states by applying requests.
    pub fn apply<P>(&mut self, request: &PaneRequest<P>) {
        match request {
            PaneRequest::Place(_, place) => self.place = *place,
            PaneRequest::FloatRect(_, rect) => self.float_rect = Some(*rect),
            PaneRequest::Minimize(_, on) => self.minimized = *on,
            PaneRequest::Resize(_, size) => self.size = Some(*size),
            PaneRequest::Close(_) | PaneRequest::Focus(_) | PaneRequest::Before(..) => {}
        }
    }
}

/// How docked panes share an edge.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum SidePolicy {
    /// Every pane on an edge shows, side by side in `panes` order, the first outermost. A drop over the outer half of
    /// a docked pane docks directly outside it, over its inner half directly inside it, and in the area's edge band
    /// outermost.
    #[default]
    Columns,
    /// One pane per edge: the last closable pane of the edge in `panes`. Panes that are not closable show beside it.
    /// A closable pane dropped on an edge takes the slot.
    OneSlot,
}

/// A change the user asked for. The area applies none of them to the app's state.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum PaneRequest<P> {
    /// The user closed the pane.
    Close(P),
    /// The user moved the pane to a new place.
    Place(P, PanePlace),
    /// The user moved or resized the floating frame to this rect. It arrives every pass a gesture changes the rect.
    FloatRect(P, Rect),
    /// The user minimized the floating pane (`true`) or restored it (`false`).
    Minimize(P, bool),
    /// The user dragged a docked pane's inner edge to this extent along its edge, within the pane's limits. It arrives
    /// once, when the drag ends; the pane follows the pointer while it runs.
    Resize(P, f32),
    /// The pane took focus.
    Focus(P),
    /// Move the first pane in `panes` to directly before the second (`None`: after every pane), which puts it
    /// directly outside the second on their edge. Sent after the `Place` of a drop that picks a position on an edge,
    /// and only when the order changes.
    Before(P, Option<P>),
}

impl<P> PaneRequest<P> {
    /// The pane the request is about.
    pub fn pane(&self) -> &P {
        match self {
            Self::Close(p)
            | Self::Place(p, _)
            | Self::FloatRect(p, _)
            | Self::Minimize(p, _)
            | Self::Resize(p, _)
            | Self::Focus(p)
            | Self::Before(p, _) => p,
        }
    }
}

/// The commands that act on the pane that holds focus. Every item of the pane menu is one of them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, craft_ui_macros::CommandSet)]
#[command(prefix = "pane", scope = "pane")]
#[non_exhaustive]
pub enum PaneCommands {
    /// Dock the pane outermost at the start edge.
    #[command(label = "Move to Start")]
    MoveToStart,
    /// Dock the pane outermost at the end edge.
    #[command(label = "Move to End")]
    MoveToEnd,
    /// Dock the pane outermost at the top edge.
    #[command(label = "Move to Top")]
    MoveToTop,
    /// Dock the pane outermost at the bottom edge.
    #[command(label = "Move to Bottom")]
    MoveToBottom,
    /// Float the pane.
    #[command(label = "Float")]
    Float,
    /// Close the pane.
    #[command(label = "Close")]
    Close,
}

/// The commands that move a pane, in menu order.
const MOVES: [PaneCommands; 5] =
    [PaneCommands::MoveToStart, PaneCommands::MoveToEnd, PaneCommands::MoveToTop, PaneCommands::MoveToBottom, PaneCommands::Float];

/// What the app says about a pane.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct PaneSpec {
    /// The title; it may follow the selection.
    pub title: Label,
    /// The state used when the app keeps none.
    pub default_state: PaneState,
    /// The limits of the pane's body: along the edge for a docked pane, both ways for a floating one.
    pub limits: SizeLimits,
    /// The size of the floating frame the first time the pane floats.
    pub float_size: Vec2,
    /// `false`: no Move or Float items and no title drag.
    pub movable: bool,
    /// `false`: no close action and no Close item (a context pane the app shows from its own state).
    pub closable: bool,
    /// `false`: a docked pane has no title row, for a body that draws its own header. A floating pane always has
    /// one, since it moves the frame.
    pub title_bar: bool,
    /// `true`: the body is one collection (a layers list, a bin) that owns the scroll. `false`: the pane scrolls its
    /// body.
    pub collection_body: bool,
}

impl PaneSpec {
    /// A movable, closable pane titled `title` with a title row and no size limits.
    pub fn new(title: impl Into<Label>, default_state: PaneState, float_size: Vec2) -> Self {
        PaneSpec {
            title: title.into(),
            default_state,
            limits: SizeLimits::default(),
            float_size,
            movable: true,
            closable: true,
            title_bar: true,
            collection_body: false,
        }
    }
}

/// The app's side of a pane: its data and its contents.
pub trait PaneContent<P> {
    /// The pane's title, limits and defaults.
    fn spec(&self, pane: &P) -> PaneSpec;

    /// The pane's layout as the app keeps it; `None` shows the pane at its default state.
    fn state(&self, _pane: &P) -> Option<PaneState> {
        None
    }

    /// Draws the pane's contents.
    fn body(&mut self, pane: &P, ui: &mut Ui, cx: &mut BodyCx<'_>);

    /// App rows of the pane menu, above the pane commands.
    fn menu(&mut self, _pane: &P, _ui: &mut Ui, _cx: &mut BodyCx<'_>) {}
}

/// What a [`PaneArea`] reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct PaneAreaOutput<P> {
    /// The area's response.
    pub response: Response,
    /// In the order they happened; apply them in order.
    pub requests: Vec<PaneRequest<P>>,
    /// The pane that holds keyboard focus, if it is shown.
    pub focused: Option<P>,
}

impl<P> Outcome for PaneAreaOutput<P> {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// A pane with its data for this pass.
#[derive(Clone)]
struct Shown<P> {
    pane: P,
    key: PanelKey,
    spec: PaneSpec,
    state: PaneState,
}

impl<P> Shown<P> {
    fn docked_at(&self) -> Option<PaneEdge> {
        match self.state.place {
            PanePlace::Docked(edge) if edge != PaneEdge::Center => Some(edge),
            _ => None,
        }
    }
}

/// The requests that dock `dragged` at `edge`, directly before the pane `before` in the order (`None`: after every
/// pane). None when it lands where it is.
fn dock_requests<P: Clone>(panes: &[Shown<P>], dragged: PanelKey, edge: PaneEdge, before: Option<PanelKey>) -> Vec<PaneRequest<P>> {
    let Some(moved) = panes.iter().find(|s| s.key == dragged) else { return Vec::new() };
    let order: Vec<PanelKey> = panes.iter().map(|s| s.key).collect();
    let mut after: Vec<PanelKey> = order.iter().copied().filter(|key| *key != dragged).collect();
    let at = before.and_then(|before| after.iter().position(|key| *key == before)).unwrap_or(after.len());
    after.insert(at, dragged);
    let docked = PanePlace::Docked(edge);
    let on_edge = |list: &[PanelKey]| -> Vec<PanelKey> {
        list.iter().copied().filter(|key| *key == dragged || panes.iter().any(|s| s.key == *key && s.state.place == docked)).collect()
    };
    if moved.state.place == docked && on_edge(&order) == on_edge(&after) {
        return Vec::new();
    }
    let mut requests = Vec::new();
    if moved.state.place != docked {
        requests.push(PaneRequest::Place(moved.pane.clone(), docked));
    }
    if order != after {
        let before = before.and_then(|key| panes.iter().find(|s| s.key == key)).map(|s| s.pane.clone());
        requests.push(PaneRequest::Before(moved.pane.clone(), before));
    }
    requests
}

/// The pane directly outside which a pane docks at `edge`: the first pane docked there other than `dragged`. A
/// one-slot edge has no position.
fn outermost<P>(panes: &[Shown<P>], policy: SidePolicy, edge: PaneEdge, dragged: PanelKey) -> Option<PanelKey> {
    match policy {
        SidePolicy::OneSlot => None,
        _ => panes.iter().find(|s| s.docked_at() == Some(edge) && s.key != dragged).map(|s| s.key),
    }
}

/// The requests of a pane command acting on `pane`.
fn command_requests<P: Clone>(panes: &[Shown<P>], policy: SidePolicy, pane: PanelKey, command: PaneCommands) -> Vec<PaneRequest<P>> {
    let Some(shown) = panes.iter().find(|s| s.key == pane) else { return Vec::new() };
    let dock = |edge| dock_requests(panes, pane, edge, outermost(panes, policy, edge, pane));
    match command {
        PaneCommands::MoveToStart if shown.spec.movable => dock(PaneEdge::Start),
        PaneCommands::MoveToEnd if shown.spec.movable => dock(PaneEdge::End),
        PaneCommands::MoveToTop if shown.spec.movable => dock(PaneEdge::Top),
        PaneCommands::MoveToBottom if shown.spec.movable => dock(PaneEdge::Bottom),
        PaneCommands::Float if shown.spec.movable && shown.state.place != PanePlace::Floating => {
            vec![PaneRequest::Place(shown.pane.clone(), PanePlace::Floating)]
        }
        PaneCommands::Close if shown.spec.closable => vec![PaneRequest::Close(shown.pane.clone())],
        _ => Vec::new(),
    }
}

/// A side of the area as drawn, after the reading direction is applied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Side {
    Left,
    Right,
    Top,
    Bottom,
}

impl Side {
    fn of(edge: PaneEdge, rtl: bool) -> Option<Side> {
        match (edge, rtl) {
            (PaneEdge::Start, false) | (PaneEdge::End, true) => Some(Side::Left),
            (PaneEdge::Start, true) | (PaneEdge::End, false) => Some(Side::Right),
            (PaneEdge::Top, _) => Some(Side::Top),
            (PaneEdge::Bottom, _) => Some(Side::Bottom),
            _ => None,
        }
    }

    /// The line across `rect` at its outer (`outer`) or inner boundary.
    fn boundary(self, rect: Rect, outer: bool) -> DropMark {
        let (from, to) = match (self, outer) {
            (Side::Left, true) | (Side::Right, false) => (rect.left_top(), rect.left_bottom()),
            (Side::Left, false) | (Side::Right, true) => (rect.right_top(), rect.right_bottom()),
            (Side::Top, true) | (Side::Bottom, false) => (rect.left_top(), rect.right_top()),
            (Side::Top, false) | (Side::Bottom, true) => (rect.left_bottom(), rect.right_bottom()),
        };
        DropMark::line(from, to)
    }
}

/// The edge of the area a dock side names; the centre is no edge.
fn pane_edge(side: DockSide) -> Option<PaneEdge> {
    match side {
        DockSide::Start => Some(PaneEdge::Start),
        DockSide::End => Some(PaneEdge::End),
        DockSide::Top => Some(PaneEdge::Top),
        DockSide::Bottom => Some(PaneEdge::Bottom),
        _ => None,
    }
}

/// What a drop on a pane area asks for.
struct PaneDocked<P> {
    requests: Vec<PaneRequest<P>>,
    mark: DropMark,
}

/// Where in the area a drop target applies.
#[derive(Clone, Copy)]
enum Zone {
    /// The area's edge bands: a drop docks the pane outermost at the edge.
    Edges { bounds: Rect, band: f32 },
    /// A docked pane: a drop docks the pane directly outside or inside it.
    Over { pane: PanelKey, edge: PaneEdge, rect: Rect },
}

/// The drop target of a pane area for the move of a pane's floating frame.
struct PaneDrop<'a, P> {
    panes: &'a [Shown<P>],
    policy: SidePolicy,
    rtl: bool,
    zone: Zone,
}

impl<P: Clone + Eq + Hash + Send + Sync + 'static> PaneDrop<'_, P> {
    fn resolve(&self, dragged: PanelKey, at: Pos2) -> Option<PaneDocked<P>> {
        let one_slot = self.policy == SidePolicy::OneSlot;
        let (edge, before, mark) = match self.zone {
            Zone::Edges { bounds, band } => {
                let (edge, zone) = zones(bounds, ZoneSize::Depth(band), self.rtl)
                    .into_iter()
                    .find_map(|(side, zone)| zone.contains(at).then(|| pane_edge(side).map(|edge| (edge, zone))).flatten())?;
                (edge, outermost(self.panes, self.policy, edge, dragged), DropMark::Into(zone))
            }
            Zone::Over { pane, edge, rect } => {
                let side = Side::of(edge, self.rtl)?;
                if one_slot {
                    (edge, None, DropMark::Into(rect))
                } else {
                    let (along, mid, low_is_outer) = match side {
                        Side::Left => (at.x, rect.center().x, true),
                        Side::Right => (at.x, rect.center().x, false),
                        Side::Top => (at.y, rect.center().y, true),
                        Side::Bottom => (at.y, rect.center().y, false),
                    };
                    let outer = (along < mid) == low_is_outer;
                    let before = if outer {
                        Some(pane)
                    } else {
                        let docked = self.panes.iter().filter(|s| s.docked_at() == Some(edge) && s.key != dragged).map(|s| s.key);
                        docked.skip_while(|key| *key != pane).nth(1)
                    };
                    (edge, before, side.boundary(rect, outer))
                }
            }
        };
        let requests = dock_requests(self.panes, dragged, edge, before);
        (!requests.is_empty()).then_some(PaneDocked { requests, mark })
    }
}

impl<P: Clone + Eq + Hash + Send + Sync + 'static> DropTarget<GroupDrag<P>> for PaneDrop<'_, P> {
    type Drop = PaneDocked<P>;

    fn accept(&self, payload: &GroupDrag<P>, at: Pos2, op: DropOperation) -> Option<PaneDocked<P>> {
        if !matches!(op, DropOperation::Move) || !matches!(payload.unit, DragUnit::Window(_)) {
            return None;
        }
        let dragged = PanelKey::of(payload.unit.lead());
        if !self.panes.iter().any(|s| s.key == dragged && s.spec.movable) {
            return None;
        }
        self.resolve(dragged, at)
    }

    fn mark(&self, drop: &PaneDocked<P>) -> DropMark {
        drop.mark
    }
}

/// The style of a pane: the surface and padding come from the container.
#[derive(Clone, Debug, Default, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct PaneFrameStyle {
    #[style(metrics)]
    metrics: Metrics,
}

/// What a docked pane reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct PaneFrameOutput {
    /// The pane's response.
    pub response: Response,
    /// The header action part taken this pass.
    pub action: Option<HeaderAction>,
    /// Whether the pane holds keyboard focus.
    pub focused: bool,
    /// The extent the user gave the pane by dragging its inner edge, in the pass the drag ends.
    pub resized: Option<f32>,
}

impl Outcome for PaneFrameOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// Takes the drop released on a container.
type TakeDrop<'a, C> = Box<dyn for<'b, 'c> FnMut(&'b mut Cx<'c, C>) + 'a>;

/// A docked pane: a shell pane on one edge of the area, with a title row and a scrolling body.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(container)]
pub struct PaneFrame<'a> {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    actions: Vec<HeaderAction>,
    menu: Option<MenuRows<'a>>,
    take_drop: Option<TakeDrop<'a, PaneFrame<'a>>>,
    taken: Option<HeaderAction>,
    menu_opener: Option<Id>,
    focused: bool,
    resized: Option<f32>,
}

impl<'a> PaneFrame<'a> {
    /// A pane for `spec` on `edge`, `extent` along it (`None`: the size it first took). The body scrolls unless the
    /// spec says it is one collection, which owns the scroll.
    fn new(spec: &PaneSpec, edge: PaneEdge, extent: Option<f32>) -> Self {
        let mut pane = ShellPane::new("pane").edge(edge);
        if let Some(extent) = extent {
            pane = pane.extent(extent);
        }
        let mut props = ContainerProps::new()
            .mount(Mount::Pane(pane))
            .surface(SurfaceKind::Panel)
            .pad(Pad::Regular)
            .sizing(Sizing::Fill)
            .focus(FocusPolicy::PANE)
            .command_scope(PaneCommands::SCOPE);
        props = if spec.collection_body { props.collection_body() } else { props.scroll(Scroll::vertical()) };
        if spec.title_bar {
            props = props.header(HeaderRegion::new().rule(true));
        }
        PaneFrame {
            common: Common::new(),
            props,
            actions: Vec::new(),
            menu: None,
            take_drop: None,
            taken: None,
            menu_opener: None,
            focused: false,
            resized: None,
        }
        .label(spec.title.clone())
    }
}

impl Component for PaneFrame<'_> {
    const ROLE: Role = Role::GROUP;
    const PARTS: bool = false;
    const STOPS: bool = false;
    type Style = PaneFrameStyle;
    type Output = PaneFrameOutput;

    fn style(_: &StyleCx<'_>, _: PaneFrameStyleKey) -> PaneFrameStyle {
        PaneFrameStyle::default()
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        self.focused = cx.core.focus.focused;
        if let Some(mut take) = self.take_drop.take() {
            take(cx);
            self.take_drop = Some(take);
        }
    }

    fn finish(self, mut cx: Cx<'_, Self>) -> PaneFrameOutput {
        if let Some(opener) = self.menu_opener {
            Popup::toggle(opener, &mut cx);
        }
        PaneFrameOutput { response: cx.response.clone(), action: self.taken, focused: self.focused, resized: self.resized }
    }
}

impl Container for PaneFrame<'_> {
    fn header(&mut self, ui: &mut Ui, cx: &mut BodyCx<'_>) {
        let shown = TitleRow::new(cx.label()).strength(HeaderStrength::Strong).actions(self.actions.iter().copied()).show(ui);
        self.taken = shown.action.or(self.taken);
        if let Some(rows) = self.menu.as_mut() {
            self.menu_opener = rows.hang(ui, &shown, cx.words::<DockWords>().pane_menu.clone());
        }
    }

    /// The grip on the pane's inner edge ended a drag: the pane's extent along its edge.
    fn placement_changed(&mut self, change: &PlacementChange) {
        if let (GesturePhase::Commit, PlacementKind::Size(edge)) = (change.phase, change.kind) {
            self.resized = Some(match edge {
                ResizeEdge::Left | ResizeEdge::Right => change.rect.width(),
                _ => change.rect.height(),
            });
        }
    }
}

/// The pane that held focus in the last pass, for the `Focus` request.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct LastFocus(Option<PanelKey>);

/// The app's side panes, bottom panes and floating panes around its document.
///
/// - **Docked panes** are shell panes on the area's edges, in `panes` order with the first outermost. Under
///   [`SidePolicy::OneSlot`] an edge shows its last closable pane and the panes that are not closable.
/// - **Floating panes** are floating frames inside the shell's float bounds. A move by the title docks the pane where
///   it is released over an edge band or a docked pane.
/// - **State.** The app owns which panes are open, where they are and how big. The area returns the user's changes as
///   [`PaneRequest`]s for the app to apply, and reads the states again through [`PaneContent::state`].
/// - **Menu.** Each pane's menu holds the app's rows, then the [`PaneCommands`] rows. Every row is also a command that
///   acts on the pane holding focus.
/// - **Centre.** The closure given to `show` fills what the docked panes leave.
#[must_use = "show the pane area with `show`"]
#[derive(crate::contract::Pattern, crate::contract::Props)]
#[pattern(probe(PaneArea<'static, u32>))]
pub struct PaneArea<'a, P> {
    label: Label,
    panes: &'a [P],
    content: &'a mut dyn PaneContent<P>,
    /// How docked panes share an edge.
    #[prop]
    policy: SidePolicy,
}

/// What every pane of one pass shares.
struct Pass<'a, P> {
    panes: &'a [Shown<P>],
    policy: SidePolicy,
    rtl: bool,
    content: RefCell<&'a mut dyn PaneContent<P>>,
    sink: RefCell<Vec<PaneRequest<P>>>,
    focused: RefCell<Option<P>>,
}

/// A drop taker with the signature a container's `update` gives it.
fn taker<'a, C: Component>(take: impl for<'b, 'c> FnMut(&'b mut Cx<'c, C>) + 'a) -> TakeDrop<'a, C> {
    Box::new(take)
}

impl<'a, P: Clone + Eq + Hash + Debug + Send + Sync + 'static> PaneArea<'a, P> {
    /// An area named `label` for the open `panes` (the order of a shared edge), filled by `content`.
    pub fn new(label: impl Into<Label>, panes: &'a [P], content: &'a mut dyn PaneContent<P>) -> Self {
        PaneArea { label: label.into(), panes, content, policy: SidePolicy::default() }
    }

    /// Shows the area across the room offered and fills what the docked panes leave with `centre`.
    pub fn show(self, ui: &mut Ui, mut centre: impl FnMut(&mut Ui, &mut BodyCx<'_>)) -> PaneAreaOutput<P> {
        let PaneArea { label, panes, content, policy } = self;
        let shown: Vec<Shown<P>> = panes
            .iter()
            .map(|pane| {
                let spec = content.spec(pane);
                let state = content.state(pane).unwrap_or(spec.default_state);
                Shown { pane: pane.clone(), key: PanelKey::of(pane), spec, state }
            })
            .collect();
        let rtl = crate::runtime::env::Env::of(ui).direction.is_rtl();
        let pass =
            Pass { panes: &shown, policy, rtl, content: RefCell::new(&mut *content), sink: RefCell::new(Vec::new()), focused: RefCell::new(None) };
        let root = DockRoot::new(|cx| {
            let band = cx.style.sizes().edge_band;
            let target = PaneDrop { panes: pass.panes, policy, rtl, zone: Zone::Edges { bounds: cx.rect, band } };
            cx.drop_target::<GroupDrag<P>, _>(&target).map(|dropped| dropped.requests)
        })
        .label(label);

        let shown_root = root.show(ui, |ui, cx| {
            let Some(sizes) = cx.style::<DockRoot<'_, Vec<PaneRequest<P>>>>().map(|style| *style.sizes()) else { return };
            let (docks, floats) = cx.shell().as_ref().map_or((true, true), |shell| (shell.dock_visible(), shell.floating_visible()));
            let bounds = cx.shell().map_or(cx.rect, |shell| shell.float_bounds());
            let mut order = [PaneEdge::Bottom, PaneEdge::Top, PaneEdge::Start, PaneEdge::End];
            if rtl {
                order.swap(2, 3);
            }
            if docks {
                for edge in order {
                    let on_edge: Vec<&Shown<P>> = shown.iter().filter(|s| s.docked_at() == Some(edge)).collect();
                    let slot = on_edge.iter().rposition(|s| s.spec.closable);
                    for (at, pane) in on_edge.iter().enumerate() {
                        if policy == SidePolicy::OneSlot && pane.spec.closable && slot != Some(at) {
                            continue;
                        }
                        if show_docked(ui, &pass, pane, edge) {
                            *pass.focused.borrow_mut() = Some(pane.pane.clone());
                        }
                    }
                }
            }
            centre(ui, cx);
            if floats {
                let inset = sizes.cascade_inset;
                let mut cascade = 0.0;
                for pane in shown.iter().filter(|s| s.state.place == PanePlace::Floating) {
                    let rect = pane.state.float_rect.unwrap_or_else(|| Rect::from_min_size(bounds.min + Vec2::splat(cascade), pane.spec.float_size));
                    cascade += inset;
                    show_floating(ui, &pass, pane, rect, bounds);
                }
            }
            let held = pass.focused.borrow().as_ref().map(PanelKey::of);
            let last = cx.keep::<LastFocus>();
            if let Some(pane) = pass.focused.borrow().clone()
                && last.0 != held
            {
                pass.sink.borrow_mut().push(PaneRequest::Focus(pane));
            }
            last.0 = held;
        });
        // A drop is taken before the body runs, so its requests come first.
        let mut requests = shown_root.output.dropped.unwrap_or_default();
        requests.extend(pass.sink.into_inner());
        PaneAreaOutput { response: shown_root.output.response, requests, focused: pass.focused.into_inner() }
    }
}

/// The extent of a docked pane along its edge: its state's size within the limits.
fn extent_of<P>(pane: &Shown<P>, edge: PaneEdge) -> Option<f32> {
    pane.state.size.map(|size| within_limits(pane, edge, size))
}

/// `size` within the limits of `pane` along `edge`.
fn within_limits<P>(pane: &Shown<P>, edge: PaneEdge, size: f32) -> f32 {
    let limits = pane.spec.limits;
    let (least, most) = match edge {
        PaneEdge::Start | PaneEdge::End => (limits.min.x, limits.max.map_or(f32::INFINITY, |max| max.x)),
        _ => (limits.min.y, limits.max.map_or(f32::INFINITY, |max| max.y)),
    };
    size.clamp(least, most.max(least))
}

/// Shows one docked pane and queues what the user did with it. Returns whether the pane holds focus.
fn show_docked<P: Clone + Eq + Hash + Debug + Send + Sync + 'static>(ui: &mut Ui, pass: &Pass<'_, P>, pane: &Shown<P>, edge: PaneEdge) -> bool {
    let mut frame = PaneFrame::new(&pane.spec, edge, extent_of(pane, edge));
    if pane.spec.movable || pane.spec.closable {
        frame.actions.push(HeaderAction::Menu);
    }
    if pane.spec.closable {
        frame.actions.push(HeaderAction::Close);
    }
    frame.menu = Some(MenuRows::new(|ui, menu| pane_menu(ui, menu, pane, &pass.content)));
    let key = pane.key;
    frame.take_drop = Some(taker(|cx: &mut Cx<'_, PaneFrame<'_>>| {
        let zone = Zone::Over { pane: key, edge, rect: cx.rect };
        let target = PaneDrop { panes: pass.panes, policy: pass.policy, rtl: pass.rtl, zone };
        if let Some(dropped) = cx.drop_target::<GroupDrag<P>, _>(&target) {
            pass.sink.borrow_mut().extend(dropped.requests);
        }
    }));
    let shown = frame
        .id_salt(("pane", key.0))
        .show(ui, |ui, cx| {
            take_commands(cx, pass, pane);
            pass.content.borrow_mut().body(&pane.pane, ui, cx);
        })
        .output;
    if let Some(size) = shown.resized {
        pass.sink.borrow_mut().push(PaneRequest::Resize(pane.pane.clone(), within_limits(pane, edge, size)));
    }
    queue_action(shown.action, pass, pane);
    shown.focused
}

/// Shows one floating pane and queues what the user did with it.
fn show_floating<P: Clone + Eq + Hash + Debug + Send + Sync + 'static>(ui: &mut Ui, pass: &Pass<'_, P>, pane: &Shown<P>, rect: Rect, bounds: Rect) {
    let mut group = FloatingGroup::new(pane.spec.title.clone(), rect).bounds(bounds).limits(pane.spec.limits).minimized(pane.state.minimized);
    let mut actions = Vec::new();
    if pane.spec.collection_body {
        group = group.collection_body();
    }
    if pane.spec.movable {
        group = group.window_of(pane.pane.clone());
        actions.push(HeaderAction::Float);
    }
    if pane.spec.closable {
        actions.push(HeaderAction::Close);
    }
    let shown = group
        .actions(actions)
        .id_salt(("float", pane.key.0))
        .show_group(ui, |ui, bcx| {
            take_commands(bcx, pass, pane);
            pass.content.borrow_mut().body(&pane.pane, ui, bcx);
        })
        .output;
    if let Some(change) = shown.placed
        && change.phase != GesturePhase::Cancel
    {
        pass.sink.borrow_mut().push(PaneRequest::FloatRect(pane.pane.clone(), change.rect));
    }
    if shown.minimized_toggled {
        pass.sink.borrow_mut().push(PaneRequest::Minimize(pane.pane.clone(), !pane.state.minimized));
    }
    if shown.response.has_focus() {
        *pass.focused.borrow_mut() = Some(pane.pane.clone());
    }
    queue_action(shown.action, pass, pane);
}

/// The request of a title row action. A floating pane's Float action docks it at the edge of its default state.
fn queue_action<P: Clone>(action: Option<HeaderAction>, pass: &Pass<'_, P>, pane: &Shown<P>) {
    let command = match (action, pane.state.place) {
        (Some(HeaderAction::Close), _) => PaneCommands::Close,
        (Some(HeaderAction::Float), PanePlace::Floating) => match pane.spec.default_state.place {
            PanePlace::Docked(PaneEdge::End) => PaneCommands::MoveToEnd,
            PanePlace::Docked(PaneEdge::Top) => PaneCommands::MoveToTop,
            PanePlace::Docked(PaneEdge::Bottom) => PaneCommands::MoveToBottom,
            _ => PaneCommands::MoveToStart,
        },
        (Some(HeaderAction::Float), _) => PaneCommands::Float,
        _ => return,
    };
    pass.sink.borrow_mut().extend(command_requests(pass.panes, pass.policy, pane.key, command));
}

/// Publishes the state of the pane commands and queues the ones that reached the pane this pass.
fn take_commands<P: Clone>(cx: &mut BodyCx<'_>, pass: &Pass<'_, P>, pane: &Shown<P>) {
    for command in MOVES {
        cx.publish(command, CommandState::new().enabled(pane.spec.movable));
    }
    cx.publish(PaneCommands::Close, CommandState::new().enabled(pane.spec.closable));
    while let Some((command, _)) = cx.take_command::<PaneCommands>() {
        pass.sink.borrow_mut().extend(command_requests(pass.panes, pass.policy, pane.key, command));
    }
}

/// The rows of a pane's menu: the app's, then the pane commands.
fn pane_menu<P: Clone>(ui: &mut Ui, cx: &mut BodyCx<'_>, pane: &Shown<P>, content: &RefCell<&mut dyn PaneContent<P>>) {
    content.borrow_mut().menu(&pane.pane, ui, cx);
    Divider::horizontal().show(ui);
    let mut commands = Vec::new();
    if pane.spec.movable {
        commands.extend(MOVES);
    }
    if pane.spec.closable {
        commands.push(PaneCommands::Close);
    }
    for command in commands {
        MenuItem::command(command).show(ui);
    }
}
