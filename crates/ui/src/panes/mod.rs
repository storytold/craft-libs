//! Task panes for document apps: side and bottom panes around the app's own central document.
//!
//! [`PaneHost`] draws each open pane as an egui side or bottom panel, or as a floating frame, with
//! a title bar (title, ▾ menu, ×) and a resizable inner edge. Call it before the app's
//! `CentralPanel`; the document never becomes a panel. The app owns which panes are open and
//! where they are: the host takes the open list and each pane's [`PaneState`] every frame and
//! returns [`PaneRequest`]s for the app to run through its own commands. The order of `open`
//! is the order of the panes on each edge, the first outermost; a drop that picks a position on
//! an edge asks for a new order with [`PaneRequest::Before`]. Docking chrome
//! ([`crate::docking::chrome`]) draws the floating frames and title bars. [`Rail`] draws an icon
//! strip at one edge that opens one pane at a time.

use std::fmt::Debug;
use std::hash::Hash;
use std::ops::RangeInclusive;

use egui::{
    Context, Frame, Id, Key, LayerId, Margin, Modifiers, Order, Pos2, Rect, Response, Role, Sense, Ui, UiBuilder, Vec2, WidgetInfo, pos2, vec2,
};

use crate::docking::PanelLimits;
use crate::docking::chrome::{self, ChromeStyle, FloatingFrame, MoveStep, TitleBar};
use crate::layout::{ResizeHandle, ResizeHandleStyle, SplitAxis};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

mod rail;

pub use rail::{RAIL_BUTTON, RAIL_WIDTH, Rail, RailItem, RailOutput, RailRequest};

/// A title dragged this far out of a docked pane floats it; a pane dropped this close to the
/// host's left or right edge docks there, outermost.
pub const DOCK_SNAP: f32 = 32.0;
/// Height of a pane's title bar.
pub const TITLE_BAR_HEIGHT: f32 = 24.0;
/// Size of a pane floated for the first time.
pub const FLOAT_DEFAULT: Vec2 = vec2(270.0, 480.0);
const FLOAT_MIN_HEIGHT: f32 = 160.0;
/// Where the title bar is held when a dragged-out pane floats at the pointer.
const FLOAT_GRAB: Vec2 = vec2(40.0, 12.0);

/// A logical edge of the host area. `Start` is left in left-to-right interfaces and right in
/// right-to-left ones.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum Edge {
    Start,
    End,
    Bottom,
}

/// Where a pane is shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum Place {
    Docked(Edge),
    Floating,
}

/// The persisted layout of one pane. The app stores it where it keeps its preferences.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct PaneState {
    pub place: Place,
    /// Width of a side pane or height of a bottom pane, in points. `None`: the pane's default.
    pub size: Option<f32>,
    /// `[x, y, width, height]` of the floating pane, also kept while it is docked.
    pub float_rect: Option<[f32; 4]>,
}

impl PaneState {
    pub fn docked(edge: Edge, size: f32) -> Self {
        Self { place: Place::Docked(edge), size: Some(size), float_rect: None }
    }

    fn apply<P>(&mut self, request: &PaneRequest<P>) {
        match request {
            PaneRequest::Place(_, place) => self.place = *place,
            PaneRequest::Resize(_, size) => self.size = Some(*size),
            PaneRequest::FloatRect(_, rect) => self.float_rect = Some(*rect),
            PaneRequest::Close(_) | PaneRequest::Focus(_) | PaneRequest::Before(..) => {}
        }
    }
}

/// How docked panes share an edge.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SidePolicy {
    /// Every pane on an edge is shown, side by side in `open` order, the first outermost. A drop
    /// over the outer third of a docked side pane docks directly outside it, over its inner third
    /// directly inside it, and at the host's left or right edge outermost; the host asks for that
    /// position with [`PaneRequest::Before`].
    #[default]
    Columns,
    /// One pane per edge: the last of the edge's closable panes in `open`. Panes that are not
    /// closable (context panes the app shows from its own state) are shown beside the slot. A
    /// closable pane dropped on an edge takes its slot: [`PaneRequest::Before`] moves it after the
    /// slot's pane in `open`.
    OneSlot,
}

/// A change the user asked for. The host applies none of them to the app's state.
#[derive(Clone, Debug, PartialEq)]
pub enum PaneRequest<P> {
    Close(P),
    Place(P, Place),
    Resize(P, f32),
    FloatRect(P, [f32; 4]),
    /// The pane took focus from a click, keyboard focus or F6.
    Focus(P),
    /// Move the first pane in `open` to directly before the second (`None`: after every pane),
    /// which puts it directly outside the second on their edge. Sent after the `Place` of a drop
    /// that picks a position on an edge, and only when `open` changes.
    Before(P, Option<P>),
}

impl<P> PaneRequest<P> {
    pub fn pane(&self) -> &P {
        match self {
            Self::Close(p) | Self::Place(p, _) | Self::Resize(p, _) | Self::FloatRect(p, _) | Self::Focus(p) | Self::Before(p, _) => p,
        }
    }
}

/// What [`PaneHost::show`] returns.
#[derive(Clone, Debug, PartialEq)]
pub struct PaneOutput<P> {
    /// In the order they happened; apply them in order. Resize and FloatRect arrive every
    /// frame a gesture changes the size or rect.
    pub requests: Vec<PaneRequest<P>>,
    /// The pane holding keyboard focus or last clicked, if it is still shown.
    pub focused: Option<P>,
    /// F6 moved focus past the last pane (or Shift+F6 past the first): focus the document.
    pub to_document: bool,
}

/// The app's side of a pane: its contents and defaults.
pub trait PaneContent<P> {
    /// May follow the selection (Format Shape, Format Picture).
    fn title(&self, pane: &P) -> String;
    fn body(&mut self, ui: &mut Ui, pane: &P);
    /// Used when the state callback returns `None` and the host has none in memory.
    fn default_state(&self, pane: &P) -> PaneState;
    /// Width (or bottom height) limits of the docked pane.
    fn size_range(&self, _pane: &P) -> RangeInclusive<f32> {
        200.0..=520.0
    }
    /// `false`: no Move or Float items and no title drag.
    fn movable(&self, _pane: &P) -> bool {
        true
    }
    /// `false`: no × and no Close item (context panes such as PivotTable Fields).
    fn closable(&self, _pane: &P) -> bool {
        true
    }
    /// `false`: a docked pane draws no title bar (no title, ▾ menu, × or title drag), for bodies
    /// that draw their own header. The body fills the pane, the resize edge stays and F6 still
    /// reaches the pane. Close then comes only from the app. A floating pane always draws its
    /// title bar, which moves it.
    fn title_bar(&self, _pane: &P) -> bool {
        true
    }
    /// App items, shown above the Move, Float and Close items of the ▾ menu.
    fn menu(&mut self, _ui: &mut Ui, _pane: &P) {}
}

/// Labels of the chrome; translate them in the app.
#[derive(Clone, Debug)]
pub struct PaneLabels {
    pub menu: String,
    pub move_start: String,
    pub move_end: String,
    pub move_bottom: String,
    pub float: String,
    pub close: String,
    /// Accessibility label of the title bar. `{title}` is replaced with the pane's title, so
    /// each title bar has its own name.
    pub move_pane: String,
    /// Accessibility label of the resize edges.
    pub resize: String,
}

impl Default for PaneLabels {
    fn default() -> Self {
        Self {
            menu: "Pane options".into(),
            move_start: "Move to Start".into(),
            move_end: "Move to End".into(),
            move_bottom: "Move to Bottom".into(),
            float: "Float".into(),
            close: "Close".into(),
            move_pane: "Move {title} pane".into(),
            resize: "Resize pane".into(),
        }
    }
}

/// Draws the open panes. Build it every frame.
pub struct PaneHost {
    id: Id,
    rtl: bool,
    policy: SidePolicy,
    max_fraction: f32,
    f6: bool,
    labels: PaneLabels,
    frame: Option<Frame>,
    margin: Margin,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Side {
    Left,
    Right,
    Bottom,
}

#[derive(Clone, Copy, Debug, Default)]
struct Memory {
    active: Option<Id>,
    last_focus: Option<Id>,
    /// F6 pressed or [`focus_next`] called: `true` cycles backwards.
    cycle: Option<bool>,
    drag_out: Option<DragOut>,
}

#[derive(Clone, Copy, Debug)]
struct DragOut {
    pane: Id,
    canceled: bool,
}

#[derive(Clone)]
struct Shown<P> {
    pane: P,
    id: Id,
    state: PaneState,
}

/// Where a dragged pane docks when dropped: at `edge`, directly before the pane `before` in
/// `open` (`None`: after every pane).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Target {
    edge: Edge,
    before: Option<Id>,
}

/// `panes` with `pane` docked at the target's edge and at its place in the order.
fn arrange<P: Clone>(panes: &[Shown<P>], pane: Id, target: Target) -> Vec<Shown<P>> {
    let mut after = panes.to_vec();
    let Some(index) = after.iter().position(|s| s.id == pane) else { return after };
    let mut moved = after.remove(index);
    moved.state.place = Place::Docked(target.edge);
    let at = match target.before {
        Some(before) => after.iter().position(|s| s.id == before).unwrap_or(index),
        None => after.len(),
    };
    after.insert(at.min(after.len()), moved);
    after
}

/// The requests that dock `pane` at `target`: its `Place` when the edge changes, then `Before`
/// when `open` changes. None when it lands where it is.
fn dock_requests<P: Clone>(panes: &[Shown<P>], pane: Id, target: Target) -> Vec<PaneRequest<P>> {
    let mut requests = Vec::new();
    let Some(s) = panes.iter().find(|s| s.id == pane) else { return requests };
    let after = arrange(panes, pane, target);
    let docked = Place::Docked(target.edge);
    let on_edge = |list: &[Shown<P>]| list.iter().filter(|s| s.state.place == docked).map(|s| s.id).collect::<Vec<_>>();
    if s.state.place == docked && on_edge(panes) == on_edge(&after) {
        return requests;
    }
    if s.state.place != docked {
        requests.push(PaneRequest::Place(s.pane.clone(), docked));
    }
    if !panes.iter().map(|s| s.id).eq(after.iter().map(|s| s.id)) {
        let before = target.before.and_then(|id| panes.iter().find(|s| s.id == id)).map(|s| s.pane.clone());
        requests.push(PaneRequest::Before(s.pane.clone(), before));
    }
    requests
}

/// Where a pane was drawn, for focus tracking.
struct Region {
    pane: Id,
    rect: Rect,
    floating: Option<LayerId>,
}

fn memory_id(id: Id) -> Id {
    id.with("panes-memory")
}

/// Move focus to the next pane (the previous one with `reverse`) at the host's next frame. For
/// apps that route F6 through their own commands; see [`PaneHost::f6`].
/// Pass the same explicit [`Id`] used for the host; strings require [`Id::unique`].
pub fn focus_next(ctx: &Context, id: impl Into<Id>, reverse: bool) {
    let key = memory_id(id.into());
    ctx.data_mut(|data| data.get_temp_mut_or_default::<Memory>(key).cycle = Some(reverse));
    ctx.request_repaint();
}

impl PaneHost {
    /// Creates a host with a stable global ID. Use [`Id::unique`] to turn a string or tuple
    /// into an ID, and reuse that ID for [`focus_next`].
    pub fn new(id: impl Into<Id>) -> Self {
        Self {
            id: id.into(),
            rtl: false,
            policy: SidePolicy::Columns,
            max_fraction: 1.0,
            f6: true,
            labels: PaneLabels::default(),
            frame: None,
            margin: Margin::same(8),
        }
    }

    /// Right-to-left interface: `Start` is the right edge.
    pub fn rtl(mut self, rtl: bool) -> Self {
        self.rtl = rtl;
        self
    }

    pub fn policy(mut self, policy: SidePolicy) -> Self {
        self.policy = policy;
        self
    }

    /// Largest share of the host's width (height for bottom panes) one docked pane may take.
    pub fn max_fraction(mut self, fraction: f32) -> Self {
        self.max_fraction = if fraction.is_finite() { fraction.clamp(0.0, 1.0) } else { 1.0 };
        self
    }

    /// Whether the host takes F6 and Shift+F6 to cycle focus through the panes and back to the
    /// document. Default on. Apps that bind F6 themselves call [`focus_next`].
    pub fn f6(mut self, f6: bool) -> Self {
        self.f6 = f6;
        self
    }

    pub fn labels(mut self, labels: PaneLabels) -> Self {
        self.labels = labels;
        self
    }

    /// Frame of the docked panels. Default: the panel fill and a hairline border, no margin.
    pub fn frame(mut self, frame: Frame) -> Self {
        self.frame = Some(frame);
        self
    }

    /// Margin around each pane's body. Default 8.
    pub fn body_margin(mut self, margin: Margin) -> Self {
        self.margin = margin;
        self
    }

    fn pane_id<P: Hash + Debug>(&self, pane: &P) -> Id {
        self.id.with(("pane", pane))
    }

    fn side(&self, edge: Edge) -> Side {
        match (edge, self.rtl) {
            (Edge::Bottom, _) => Side::Bottom,
            (Edge::Start, false) | (Edge::End, true) => Side::Left,
            (Edge::Start, true) | (Edge::End, false) => Side::Right,
        }
    }

    /// The edge a pane dropped at `pointer` docks against, if it is within [`DOCK_SNAP`] of the
    /// host's left or right edge.
    fn snap(&self, pointer: Pos2, bounds: Rect) -> Option<Edge> {
        let left = if self.rtl { Edge::End } else { Edge::Start };
        let right = if self.rtl { Edge::Start } else { Edge::End };
        if pointer.x <= bounds.left() + DOCK_SNAP {
            Some(left)
        } else if pointer.x >= bounds.right() - DOCK_SNAP {
            Some(right)
        } else {
            None
        }
    }

    /// Draw the panes in `open` with the state `states` returns for each (`None`: the state the
    /// host kept from earlier requests, else the pane's default). Call between the app's top
    /// and bottom bars and its `CentralPanel`.
    pub fn show<P: Clone + Hash + Debug, C: PaneContent<P>>(
        self,
        ui: &mut Ui,
        open: &[P],
        states: impl Fn(&P) -> Option<PaneState>,
        content: &mut C,
    ) -> PaneOutput<P> {
        let ctx = ui.ctx().clone();
        let bounds = ui.available_rect_before_wrap();
        let style = ChromeStyle { right_to_left: self.rtl, ..ChromeStyle::from_ui(ui) };
        let key = memory_id(self.id);
        let mut memory = ctx.data(|data| data.get_temp::<Memory>(key)).unwrap_or_default();
        let mut requests = Vec::new();
        let mut to_document = false;

        if self.f6 {
            ui.input_mut(|input| {
                if input.consume_key(Modifiers::SHIFT, Key::F6) {
                    memory.cycle = Some(true);
                } else if input.consume_key(Modifiers::NONE, Key::F6) {
                    memory.cycle = Some(false);
                }
            });
        }

        let mut shown: Vec<Shown<P>> = Vec::new();
        for pane in open {
            let id = self.pane_id(pane);
            if shown.iter().any(|s| s.id == id) {
                continue;
            }
            let state =
                states(pane).or_else(|| ctx.data(|data| data.get_temp::<PaneState>(id.with("state")))).unwrap_or_else(|| content.default_state(pane));
            shown.push(Shown { pane: pane.clone(), id, state });
        }
        // Before the one-slot filter: a drop can reveal a pane this frame hides.
        let open_panes = shown.clone();
        self.one_slot(&mut shown, content);

        let previous = memory.active.filter(|active| shown.iter().any(|s| s.id == *active));
        memory.active = previous;
        let mut focus_target = None;
        if let Some(reverse) = memory.cycle.take() {
            let current = memory.active.and_then(|active| shown.iter().position(|s| s.id == active));
            let next = match (current, reverse) {
                (None, false) => shown.first(),
                (None, true) => shown.last(),
                (Some(i), false) => shown.get(i + 1),
                (Some(i), true) => i.checked_sub(1).and_then(|i| shown.get(i)),
            };
            focus_target = next.map(|s| s.id);
            memory.active = focus_target;
            if focus_target.is_none() {
                to_document = true;
                ctx.memory_mut(|m| {
                    if let Some(focused) = m.focused() {
                        m.surrender_focus(focused);
                    }
                });
            }
        }

        let preview = ctx.layer_painter(LayerId::new(Order::Tooltip, self.id.with("drop-preview")));
        let mut regions = Vec::new();
        for edge in [Edge::Start, Edge::End, Edge::Bottom] {
            for s in shown.iter().filter(|s| s.state.place == Place::Docked(edge)) {
                let rect =
                    self.docked(ui, &open_panes, s, edge, content, &style, focus_target == Some(s.id), bounds, &mut memory, &preview, &mut requests);
                regions.push(Region { pane: s.id, rect, floating: None });
            }
        }
        for s in shown.iter().filter(|s| s.state.place == Place::Floating) {
            let (rect, layer) = self.floating(&ctx, &open_panes, s, content, &style, focus_target == Some(s.id), bounds, &preview, &mut requests);
            regions.push(Region { pane: s.id, rect, floating: Some(layer) });
        }

        if focus_target.is_none() && !to_document {
            self.track_focus(&ctx, ui.layer_id(), &regions, &mut memory);
        }
        memory.last_focus = ctx.memory(|m| m.focused());
        if memory.active != previous
            && let Some(s) = memory.active.and_then(|active| shown.iter().find(|s| s.id == active))
        {
            requests.push(PaneRequest::Focus(s.pane.clone()));
        }

        for s in &shown {
            let mut state = s.state;
            let mut changed = false;
            for request in requests.iter().filter(|r| self.pane_id(r.pane()) == s.id) {
                state.apply(request);
                changed = true;
            }
            if changed {
                ctx.data_mut(|data| data.insert_temp(s.id.with("state"), state));
            }
        }
        let focused = memory.active.and_then(|active| shown.iter().find(|s| s.id == active)).map(|s| s.pane.clone());
        ctx.data_mut(|data| data.insert_temp(key, memory));
        PaneOutput { requests, focused, to_document }
    }

    /// The pane under a primary press, or the pane holding a newly focused widget, becomes active.
    /// Presses and focus outside every pane (the document) clear it; presses on other layers
    /// (menus, the app's own windows) leave it.
    fn track_focus(&self, ctx: &Context, layer: LayerId, regions: &[Region], memory: &mut Memory) {
        let find = |at_layer: Option<LayerId>, pos: Pos2| -> Option<Option<Id>> {
            if let Some(region) = regions.iter().find(|r| r.floating.is_some() && r.floating == at_layer) {
                return Some(Some(region.pane));
            }
            if at_layer.is_none() || at_layer == Some(layer) {
                return Some(regions.iter().find(|r| r.floating.is_none() && r.rect.contains(pos)).map(|r| r.pane));
            }
            None
        };
        if ctx.input(|input| input.pointer.primary_pressed())
            && let Some(pos) = ctx.input(|input| input.pointer.press_origin())
            && let Some(active) = find(ctx.layer_id_at(pos), pos)
        {
            memory.active = active;
        }
        let focused = ctx.memory(|m| m.focused());
        if focused != memory.last_focus
            && let Some(response) = focused.and_then(|id| ctx.read_response(id))
            && let Some(active) = find(Some(response.layer_id), response.rect.center())
        {
            memory.active = active;
        }
    }

    fn limits<P, C: PaneContent<P>>(&self, content: &C, pane: &P, extent: f32) -> (f32, f32) {
        let range = content.size_range(pane);
        let min = if range.start().is_finite() { range.start().max(0.0) } else { 0.0 };
        let max = if range.end().is_finite() { range.end().max(min) } else { min.max(extent) };
        (min, max.min(self.max_fraction * extent).max(min))
    }

    /// With [`SidePolicy::OneSlot`], drop every closable docked pane but the last on its edge.
    fn one_slot<P, C: PaneContent<P>>(&self, shown: &mut Vec<Shown<P>>, content: &C) {
        if self.policy != SidePolicy::OneSlot {
            return;
        }
        let mut taken = Vec::new();
        for index in (0..shown.len()).rev() {
            if let Place::Docked(edge) = shown[index].state.place
                && content.closable(&shown[index].pane)
            {
                if taken.contains(&edge) {
                    shown.remove(index);
                } else {
                    taken.push(edge);
                }
            }
        }
    }

    /// The width (height at the bottom) of a pane docked at `edge`: its size, else its default,
    /// within its limits.
    fn docked_size<P, C: PaneContent<P>>(&self, content: &C, s: &Shown<P>, edge: Edge, bounds: Rect) -> f32 {
        let extent = if self.side(edge) == Side::Bottom { bounds.height() } else { bounds.width() };
        let (min, max) = self.limits(content, &s.pane, extent);
        let default = content.default_state(&s.pane).size;
        s.state.size.filter(|v| v.is_finite()).or(default.filter(|v| v.is_finite())).unwrap_or(min).clamp(min, max)
    }

    /// The rects of the side panes in `panes` as [`Self::show`] draws them: `Start` panes, then
    /// `End` panes, each in order from the outside in and no wider than the space left.
    fn side_rects<P, C: PaneContent<P>>(&self, panes: &[Shown<P>], content: &C, bounds: Rect) -> Vec<(Id, Edge, Rect)> {
        let mut rects = Vec::new();
        let mut free = bounds;
        for edge in [Edge::Start, Edge::End] {
            for s in panes.iter().filter(|s| s.state.place == Place::Docked(edge)) {
                let size = self.docked_size(content, s, edge, bounds).min(free.width()).max(0.0);
                let rect = if self.side(edge) == Side::Left {
                    let rect = Rect::from_x_y_ranges(free.left()..=free.left() + size, free.y_range());
                    free.min.x = rect.right();
                    rect
                } else {
                    let rect = Rect::from_x_y_ranges(free.right() - size..=free.right(), free.y_range());
                    free.max.x = rect.left();
                    rect
                };
                rects.push((s.id, edge, rect));
            }
        }
        rects
    }

    /// Where `pane`, dragged with the pointer at `pointer`, docks if dropped: at the host's left or
    /// right edge, outermost there, or over the outer or inner third of another docked side pane,
    /// directly outside or inside it. `panes` is the open list before the one-slot filter.
    fn target<P: Clone, C: PaneContent<P>>(&self, pointer: Pos2, panes: &[Shown<P>], pane: Id, content: &C, bounds: Rect) -> Option<Target> {
        if !pointer.is_finite() {
            return None;
        }
        let others = || panes.iter().filter(|s| s.id != pane);
        // The pane after `id` in `open`, skipping the dragged pane: directly after `id` is
        // directly before that one.
        let after = |id: Id| others().skip_while(|s| s.id != id).nth(1).map(|s| s.id);
        // Directly before the pane that follows it, the dragged pane keeps its place in `open`.
        let keep = panes.iter().skip_while(|s| s.id != pane).nth(1).map(|s| s.id);
        let mut visible = panes.to_vec();
        self.one_slot(&mut visible, content);
        // One slot per edge leaves no position to pick: a closable pane takes the edge's slot by
        // going after the slot's pane.
        let slot = |edge: Edge, columns: Option<Id>| match self.policy {
            SidePolicy::Columns => columns,
            SidePolicy::OneSlot => {
                let closable = panes.iter().any(|s| s.id == pane && content.closable(&s.pane));
                match visible.iter().find(|s| closable && s.id != pane && s.state.place == Place::Docked(edge) && content.closable(&s.pane)) {
                    Some(taken) => after(taken.id),
                    None => keep,
                }
            }
        };
        if let Some(edge) = self.snap(pointer, bounds) {
            let first = others().find(|s| s.state.place == Place::Docked(edge)).map(|s| s.id);
            return Some(Target { edge, before: slot(edge, first.or(keep)) });
        }
        let (id, edge, rect) = self.side_rects(&visible, content, bounds).into_iter().find(|(id, _, rect)| *id != pane && rect.contains(pointer))?;
        let third = rect.width() / 3.0;
        let from_outside = if self.side(edge) == Side::Left { pointer.x - rect.left() } else { rect.right() - pointer.x };
        if from_outside < third {
            Some(Target { edge, before: slot(edge, Some(id)) })
        } else if from_outside > rect.width() - third {
            Some(Target { edge, before: slot(edge, after(id)) })
        } else {
            None
        }
    }

    /// The rect `pane` takes on the next frame once it is docked at `target`. `None` when
    /// [`SidePolicy::OneSlot`] would hide it there.
    fn landing<P: Clone, C: PaneContent<P>>(&self, panes: &[Shown<P>], pane: Id, target: Target, content: &C, bounds: Rect) -> Option<Rect> {
        let mut after = arrange(panes, pane, target);
        self.one_slot(&mut after, content);
        self.side_rects(&after, content, bounds).into_iter().find(|(id, _, _)| *id == pane).map(|(_, _, rect)| rect)
    }

    /// Paints where `pane` lands when it is dropped at `target`.
    #[allow(clippy::too_many_arguments)]
    fn preview_dock<P: Clone, C: PaneContent<P>>(
        &self,
        preview: &egui::Painter,
        style: &ChromeStyle,
        panes: &[Shown<P>],
        pane: Id,
        target: Target,
        content: &C,
        bounds: Rect,
    ) {
        if let Some(rect) = self.landing(panes, pane, target, content, bounds) {
            chrome::drop_preview(preview, rect.shrink(1.0), style.accent, true);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn docked<P: Clone + Hash + Debug, C: PaneContent<P>>(
        &self,
        ui: &mut Ui,
        shown: &[Shown<P>],
        s: &Shown<P>,
        edge: Edge,
        content: &mut C,
        style: &ChromeStyle,
        focus: bool,
        bounds: Rect,
        memory: &mut Memory,
        preview: &egui::Painter,
        requests: &mut Vec<PaneRequest<P>>,
    ) -> Rect {
        let side = self.side(edge);
        let extent = if side == Side::Bottom { bounds.height() } else { bounds.width() };
        let (min, max) = self.limits(content, &s.pane, extent);
        // A side pane takes no more than the width the panes outside it leave: egui would place
        // a wider one partly beyond the host's edge.
        let room = if side == Side::Bottom { f32::INFINITY } else { ui.available_rect_before_wrap().width().max(0.0) };
        let size = self.docked_size(content, s, edge, bounds).min(room);
        let panel_id = s.id.with("panel");
        let panel = match side {
            Side::Left => egui::Panel::left(panel_id),
            Side::Right => egui::Panel::right(panel_id),
            Side::Bottom => egui::Panel::bottom(panel_id),
        };
        let frame = self.frame.unwrap_or_else(|| Frame::NONE.fill(ui.visuals().panel_fill).stroke(ui.visuals().widgets.noninteractive.bg_stroke));
        let title_bar = content.title_bar(&s.pane);
        let drawn = panel.exact_size(size).resizable(false).frame(frame).show(ui, |ui| {
            let top = ui.max_rect();
            let (bar, body_top) = if title_bar {
                let title = Rect::from_min_size(top.min, vec2(top.width(), TITLE_BAR_HEIGHT));
                ui.allocate_rect(title, Sense::hover());
                (Some(self.chrome(ui, title, s, content, style, focus, bounds, requests)), title.bottom())
            } else {
                (None, top.top())
            };
            // The pane reserves exactly its size and clips its body. egui cuts a panel wider
            // than its exact size at its outer edge, so a wide body would shift a right or
            // bottom pane's reserved rect inwards and the document would draw over the pane.
            let body = Rect::from_min_max(pos2(top.left(), body_top), top.max);
            // Without a title bar, F6 focuses the body itself; Tab goes on into its widgets,
            // which register after it.
            let stop = (!title_bar).then(|| {
                let stop = ui.interact(body, s.id.with("body"), Sense::focusable_noninteractive());
                let title = content.title(&s.pane);
                stop.widget_info(|| WidgetInfo::labeled(Role::Pane, true, &title));
                if focus {
                    stop.request_focus();
                }
                stop
            });
            let mut child = ui.new_child(UiBuilder::new().max_rect(body).layout(*ui.layout()));
            child.set_clip_rect(body.intersect(ui.clip_rect()));
            Frame::NONE.inner_margin(self.margin).show(&mut child, |ui| {
                ui.set_min_width(ui.available_width());
                content.body(ui, &s.pane);
            });
            if stop.as_ref().is_some_and(Response::has_focus) {
                ui.painter().rect_stroke(body.shrink(1.0), 0.0, egui::Stroke::new(1.0, style.accent), egui::StrokeKind::Inside);
            }
            ui.allocate_rect(body, Sense::hover());
            bar
        });
        let rect = drawn.response.rect;
        let bar = drawn.inner;

        let (axis, strip) = match side {
            Side::Left => (SplitAxis::Horizontal, Rect::from_x_y_ranges(rect.right() - 2.0..=rect.right() + 2.0, rect.y_range())),
            Side::Right => (SplitAxis::Horizontal, Rect::from_x_y_ranges(rect.left() - 2.0..=rect.left() + 2.0, rect.y_range())),
            Side::Bottom => (SplitAxis::Vertical, Rect::from_x_y_ranges(rect.x_range(), rect.top() - 2.0..=rect.top() + 2.0)),
        };
        let handle_style = ResizeHandleStyle { hovered: style.accent.gamma_multiply(0.5), dragged: style.accent, focused: style.accent };
        // The edge's hit strip reaches over the document, whose widgets register after the host
        // and would take presses there on the host's layer. A sublayer sits directly above it:
        // over the document, under floating windows.
        let edge_layer = LayerId::new(ui.layer_id().order, s.id.with("resize-layer"));
        ui.ctx().set_sublayer(ui.layer_id(), edge_layer);
        let mut edge_ui = ui.new_child(UiBuilder::new().layer_id(edge_layer));
        let resize = ResizeHandle::new(s.id.with("resize"), axis, strip, &self.labels.resize, handle_style).show(&mut edge_ui);
        let next = if resize.response.dragged() {
            resize.response.interact_pointer_pos().map(|p| match side {
                Side::Left => p.x - rect.left(),
                Side::Right => rect.right() - p.x,
                Side::Bottom => rect.bottom() - p.y,
            })
        } else if resize.delta != 0.0 {
            // Arrow keys move the edge: right grows a left pane, left grows a right pane.
            Some(if side == Side::Left { size + resize.delta } else { size - resize.delta })
        } else {
            None
        };
        if let Some(next) = next.filter(|v| v.is_finite()).map(|v| v.clamp(min, max).round())
            && next != size
        {
            requests.push(PaneRequest::Resize(s.pane.clone(), next));
        }

        if content.movable(&s.pane)
            && let Some(bar) = &bar
        {
            self.drag_out(ui.ctx(), bar, shown, s, edge, rect, content, bounds, memory, preview, style, requests);
        }
        rect
    }

    /// A docked pane's title dragged beyond [`DOCK_SNAP`] of the pane: dropped on a dock target
    /// ([`Self::target`]) it docks there, elsewhere it floats under the pointer.
    #[allow(clippy::too_many_arguments)]
    fn drag_out<P: Clone, C: PaneContent<P>>(
        &self,
        ctx: &Context,
        bar: &Response,
        shown: &[Shown<P>],
        s: &Shown<P>,
        edge: Edge,
        rect: Rect,
        content: &C,
        bounds: Rect,
        memory: &mut Memory,
        preview: &egui::Painter,
        style: &ChromeStyle,
        requests: &mut Vec<PaneRequest<P>>,
    ) {
        if bar.drag_started() {
            memory.drag_out = Some(DragOut { pane: s.id, canceled: false });
        }
        let Some(mut drag) = memory.drag_out.filter(|d| d.pane == s.id) else { return };
        if ctx.input(|input| input.key_pressed(Key::Escape)) || crate::focus::lost(ctx) {
            drag.canceled = true;
        }
        let near = rect.expand(DOCK_SNAP);
        let Some(pointer) = ctx.pointer_latest_pos() else { return };
        let outside = if edge == Edge::Bottom { !near.y_range().contains(pointer.y) } else { !near.x_range().contains(pointer.x) };
        let size = s.state.float_rect.map_or(FLOAT_DEFAULT, |r| vec2(r[2], r[3]));
        let float = float_rect(Some(rect_array(Rect::from_min_size(pointer - FLOAT_GRAB, size))), bounds, vec2(0.0, FLOAT_MIN_HEIGHT));
        let target = self.target(pointer, shown, s.id, content, bounds);
        if bar.drag_stopped() {
            memory.drag_out = None;
            if drag.canceled || !outside {
                return;
            }
            match target {
                Some(target) => requests.extend(dock_requests(shown, s.id, target)),
                None => {
                    requests.push(PaneRequest::Place(s.pane.clone(), Place::Floating));
                    requests.push(PaneRequest::FloatRect(s.pane.clone(), rect_array(float)));
                }
            }
            return;
        }
        if !bar.dragged() {
            memory.drag_out = None;
            return;
        }
        memory.drag_out = Some(drag);
        if drag.canceled || !outside {
            return;
        }
        match target {
            Some(target) => self.preview_dock(preview, style, shown, s.id, target, content, bounds),
            None => chrome::drop_preview(preview, float.shrink(1.0), style.accent, true),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn floating<P: Clone + Hash + Debug, C: PaneContent<P>>(
        &self,
        ctx: &Context,
        panes: &[Shown<P>],
        s: &Shown<P>,
        content: &mut C,
        style: &ChromeStyle,
        focus: bool,
        bounds: Rect,
        preview: &egui::Painter,
        requests: &mut Vec<PaneRequest<P>>,
    ) -> (Rect, LayerId) {
        let (min_width, _) = self.limits(content, &s.pane, bounds.width());
        let min = vec2(min_width, FLOAT_MIN_HEIGHT);
        let rect = float_rect(s.state.float_rect, bounds, min);
        let limits = PanelLimits { min, max: bounds.size().max(min) };
        let shown = FloatingFrame::new(s.id.with("float"), rect).bounds(bounds).limits(limits).resize_label(&self.labels.resize).show(
            ctx,
            style,
            |ui, rect| {
                let title = Rect::from_min_size(rect.min, vec2(rect.width(), TITLE_BAR_HEIGHT));
                let bar = self.chrome(ui, title, s, content, style, focus, bounds, requests);
                let body = Rect::from_min_max(pos2(rect.left(), title.bottom()), rect.max) - self.margin;
                if body.is_positive() {
                    ui.scope_builder(UiBuilder::new().max_rect(body), |ui| {
                        ui.set_clip_rect(body.intersect(ui.clip_rect()));
                        content.body(ui, &s.pane);
                    });
                }
                bar
            },
        );
        if let Some(resized) = shown.resized {
            requests.push(PaneRequest::FloatRect(s.pane.clone(), rect_array(resized)));
        }
        if content.movable(&s.pane) {
            let step = chrome::live_move(ctx, &shown.inner, shown.rect, bounds);
            if let Some(MoveStep::Moving(_)) = step
                && let Some(target) = ctx.pointer_latest_pos().and_then(|pointer| self.target(pointer, panes, s.id, content, bounds))
            {
                self.preview_dock(preview, style, panes, s.id, target, content, bounds);
            }
            match step {
                Some(MoveStep::Moving(next) | MoveStep::Canceled(next)) if next != shown.rect => {
                    requests.push(PaneRequest::FloatRect(s.pane.clone(), rect_array(next)));
                }
                Some(MoveStep::Dropped { rect, pointer }) => {
                    if let Some(target) = self.target(pointer, panes, s.id, content, bounds) {
                        requests.extend(dock_requests(panes, s.id, target));
                    } else if rect != shown.rect {
                        requests.push(PaneRequest::FloatRect(s.pane.clone(), rect_array(rect)));
                    }
                }
                _ => {}
            }
        }
        (shown.rect, shown.layer)
    }

    /// Title, ▾ menu and ×. Returns the bar, which senses the title drag.
    #[allow(clippy::too_many_arguments)]
    fn chrome<P: Clone, C: PaneContent<P>>(
        &self,
        ui: &mut Ui,
        rect: Rect,
        s: &Shown<P>,
        content: &mut C,
        style: &ChromeStyle,
        focus: bool,
        bounds: Rect,
        requests: &mut Vec<PaneRequest<P>>,
    ) -> Response {
        let movable = content.movable(&s.pane);
        let closable = content.closable(&s.pane);
        let title = content.title(&s.pane);
        let move_label = self.labels.move_pane.replace("{title}", &title);
        let mut bar = TitleBar::new(s.id.with("title"), &move_label).title(title).menu(&self.labels.menu).movable(movable);
        if closable {
            bar = bar.close(&self.labels.close);
        }
        let title = bar.show(ui, rect, style);
        if focus {
            title.bar.request_focus();
        }
        if title.close.as_ref().is_some_and(Response::clicked) {
            requests.push(PaneRequest::Close(s.pane.clone()));
        }
        if let Some(menu) = &title.menu {
            egui::Popup::menu(menu).show(|ui| {
                let top = ui.cursor().top();
                content.menu(ui, &s.pane);
                if ui.cursor().top() > top && (movable || closable) {
                    ui.separator();
                }
                if movable {
                    let mut places = vec![(Place::Docked(Edge::Start), &self.labels.move_start), (Place::Docked(Edge::End), &self.labels.move_end)];
                    if content.default_state(&s.pane).place == Place::Docked(Edge::Bottom) {
                        places.push((Place::Docked(Edge::Bottom), &self.labels.move_bottom));
                    }
                    places.push((Place::Floating, &self.labels.float));
                    for (place, label) in places {
                        if ui.selectable_label(s.state.place == place, label.as_str()).clicked() && s.state.place != place {
                            requests.push(PaneRequest::Place(s.pane.clone(), place));
                            if place == Place::Floating {
                                let rect = float_rect(s.state.float_rect, bounds, vec2(0.0, FLOAT_MIN_HEIGHT));
                                requests.push(PaneRequest::FloatRect(s.pane.clone(), rect_array(rect)));
                            }
                        }
                    }
                }
                if closable && ui.button(self.labels.close.as_str()).clicked() {
                    requests.push(PaneRequest::Close(s.pane.clone()));
                }
            });
        }
        title.bar
    }
}

fn rect_array(rect: Rect) -> [f32; 4] {
    [rect.min.x, rect.min.y, rect.width(), rect.height()]
}

/// The floating rect for `saved` (`[x, y, width, height]`), at least `min` and at most the
/// bounds in size, inside `bounds`. Without a saved rect: [`FLOAT_DEFAULT`] right of the middle.
/// Non-finite or negative values fall back to the defaults, so the result is always finite.
pub fn float_rect(saved: Option<[f32; 4]>, bounds: Rect, min: Vec2) -> Rect {
    let bounds = if bounds.is_finite() && bounds.is_positive() { bounds } else { Rect::from_min_size(Pos2::ZERO, FLOAT_DEFAULT) };
    let finite = |v: f32, fallback: f32| if v.is_finite() { v } else { fallback };
    let size = saved.map_or(FLOAT_DEFAULT, |r| vec2(finite(r[2], FLOAT_DEFAULT.x), finite(r[3], FLOAT_DEFAULT.y)));
    let size = size.max(min.max(Vec2::splat(1.0))).min(bounds.size());
    let free = (bounds.size() - size).max(Vec2::ZERO);
    let default = (bounds.min + vec2(free.x * 0.75, free.y * 0.3)).round();
    let pos = saved.map_or(default, |r| pos2(finite(r[0], default.x), finite(r[1], default.y)));
    let pos = pos.max(bounds.min).min((bounds.max - size).max(bounds.min));
    Rect::from_min_size(pos, size)
}

#[cfg(test)]
mod rail_tests;
#[cfg(test)]
mod tests;
