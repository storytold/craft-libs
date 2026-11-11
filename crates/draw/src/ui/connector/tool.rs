//! [`ConnectorTool`]: the Connector tool on the tool host.

use std::sync::Arc;

use craft_behavior::geometry::draft::turn_to_45;
use craft_behavior::{CancelReason, CanvasView, Cursor, CursorIcon, EditEvent, GestureId, Mixed};
use craft_ui::canvas::{CanvasTool, ToolAccess, ToolAction, ToolContext, ToolPath, ToolPointer};
use craft_ui::primitives::Draw;
use egui::Pos2;
use kurbo::{Point, Rect};

use super::overlay::{ConnectorLook, paint_route, paint_sites};
use super::place::{Placed, anchor, end_at, glued};
use super::words::ConnectorWords;
use super::{ConnectionTarget, ConnectorDraft, ConnectorEnd, ConnectorEvent};
use crate::geometry::connector::{RouteEnd, Routing, route};

/// The Connector tool (FigJam's, as FigCraft has it; DeckCraft's glue to numbered sites). The app
/// owns the value, sets the targets near the pointer and the routing each pass, hands it to
/// `ToolHost::run` and drains [`take_events`](Self::take_events). The tool never changes the
/// document.
///
/// - Hover: the target under the pointer shows its sites (attach points); the one an end would glue
///   to is the larger see-through ring.
/// - Draw: a press on a target or on empty canvas starts; the drag previews the route (an accent
///   polyline through [`route`], unless [`custom_preview`](Self::custom_preview)); the release makes
///   it. A drag under the press slop makes nothing; one that ends on its start object is cancelled
///   as `Dismissed`. ⇧ turns a free end to 45° steps from the other end
///   ([`snap_free_end`](Self::snap_free_end), on by default). ⌘ or Ctrl on a target with no sites
///   takes the point itself.
/// - Esc cancels the drag; a tool switch cancels it too; Delete is the app's.
///
/// The selected connector's ends and bends are [`ConnectorHandles`](super::ConnectorHandles), a canvas
/// control: a press on one of its parts never reaches this tool. Its keyboard path is the handles'
/// arrow keys, which move a picked end to the next object.
#[derive(Debug)]
pub struct ConnectorTool {
    targets: Vec<ConnectionTarget>,
    routing: Routing,
    snap_free_end: bool,
    custom_preview: bool,
    events: Vec<ConnectorEvent>,
    drag: Option<Drag>,
    /// The target and site under the pointer (hover and drag), for the attach points.
    hover: Option<(u64, Option<u32>)>,
    /// The id the next gesture takes.
    next: GestureId,
}

impl Default for ConnectorTool {
    fn default() -> Self {
        ConnectorTool {
            targets: Vec::new(),
            routing: Routing::default(),
            snap_free_end: true,
            custom_preview: false,
            events: Vec::new(),
            drag: None,
            hover: None,
            next: GestureId::default(),
        }
    }
}

/// A gesture between passes.
#[derive(Clone, Debug)]
struct Drag {
    id: GestureId,
    label: Arc<str>,
    routing: Routing,
    /// The end the press placed.
    from: ConnectorEnd,
    from_route: RouteEnd,
    /// The object the press landed on.
    start_target: Option<u64>,
    /// The press on screen, for the press slop.
    press: Pos2,
    /// The drag has left the slop and reported `Begin`.
    began: bool,
    /// The latest draft, `None` when the pointer is where no route exists.
    value: Option<ConnectorDraft>,
    /// The draft differs from nothing: it does not end on its start object.
    changed: bool,
    /// The last draft reported, so a still pointer sends no update.
    sent: Option<ConnectorDraft>,
    /// The route to draw, document units.
    preview: Vec<Point>,
}

#[derive(Clone, Copy)]
enum Phase {
    Begin,
    Update,
    Commit,
    Cancel(CancelReason),
}

/// The event of `phase` for gesture `d`; none when it has no draft to report.
fn event(d: &Drag, phase: Phase) -> Option<ConnectorEvent> {
    Some(ConnectorEvent::Draw(match phase {
        Phase::Begin => EditEvent::begin(d.id, Mixed::Mixed, Arc::clone(&d.label), None),
        Phase::Update => EditEvent::update(d.id, Box::new(d.value.clone()?)),
        Phase::Commit => EditEvent::commit(d.id, Mixed::Mixed, Box::new(d.value.clone()?)),
        Phase::Cancel(reason) => EditEvent::cancel(d.id, reason, None),
    }))
}

impl ConnectorTool {
    /// A tool drawing straight connectors.
    pub fn new() -> Self {
        Self::default()
    }

    /// The objects an end may glue to this pass, topmost first: the app passes those under or near
    /// the pointer (or every visible one). A target with no sites offers its four side middles.
    pub fn targets(&mut self, targets: &[ConnectionTarget]) -> &mut Self {
        self.targets.clear();
        self.targets.extend_from_slice(targets);
        self
    }

    /// The routing new connectors get.
    pub fn routing(&mut self, routing: Routing) -> &mut Self {
        self.routing = routing;
        self
    }

    /// Whether ⇧ turns a free end to 45° steps from the other end; on by default (Illustrator and
    /// Figma; FigCraft turns it off).
    pub fn snap_free_end(&mut self, on: bool) -> &mut Self {
        self.snap_free_end = on;
        self
    }

    /// The app previews the connector in the document from `Update` events; the tool draws no
    /// route. Off by default.
    pub fn custom_preview(&mut self, on: bool) -> &mut Self {
        self.custom_preview = on;
        self
    }

    /// The events since the last call, in order.
    pub fn take_events(&mut self) -> Vec<ConnectorEvent> {
        std::mem::take(&mut self.events)
    }

    /// The end under the pointer for a gesture: a free end snaps with the canvas, and ⇧ turns it to
    /// 45° steps from `origin`.
    fn place(&self, pointer: &ToolPointer, view: &CanvasView, cx: &ToolContext<'_>, origin: Option<Point>) -> Placed {
        let placed = end_at(&self.targets, view.to_doc(pointer.pos), view, cx.reach(), pointer.mods);
        let ConnectorEnd::Free(raw) = placed.end else { return placed };
        let mut at = cx.snap(raw);
        if let (Some(origin), true) = (origin, pointer.mods.shift && self.snap_free_end) {
            at = turn_to_45(origin, at);
        }
        Placed { end: ConnectorEnd::Free(at), route: RouteEnd::Point(at), ..placed }
    }

    /// Follows the pointer: the draft, the preview and the hover, and the `Begin` and `Update` events
    /// once the pointer is past the press slop.
    fn advance(&mut self, d: &mut Drag, pointer: &ToolPointer, cx: &ToolContext<'_>, update: bool) {
        let Some(space) = cx.space() else { return };
        self.hover = None;
        let placed = self.place(pointer, &space.view, cx, Some(anchor(d.from_route)));
        let draft =
            route(d.from_route, placed.route, d.routing).map(|points| ConnectorDraft { from: d.from, to: placed.end, routing: d.routing, points });
        // Ending on the object it started from makes nothing.
        let same = d.start_target.is_some() && glued(&placed.end) == d.start_target;
        d.changed = !same;
        d.preview = if same { Vec::new() } else { draft.as_ref().map(|x| x.points.clone()).unwrap_or_default() };
        d.value = draft;
        self.hover = placed.target.map(|t| (t, placed.active));
        if !d.began && pointer.pos.distance(d.press) >= cx.slop {
            d.began = true;
            self.events.extend(event(d, Phase::Begin));
        }
        if update && d.began && d.value.is_some() && d.value != d.sent {
            self.events.extend(event(d, Phase::Update));
            d.sent.clone_from(&d.value);
        }
    }
}

impl CanvasTool for ConnectorTool {
    fn press(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) -> ToolAction {
        // inv-figcraft: crates/engine/src/boards/board/tools.rs:102-130 (press on an object or the
        // canvas).
        self.drag = None;
        let Some(space) = cx.space() else { return ToolAction::Ignored };
        let placed = self.place(pointer, &space.view, cx, None);
        self.next = self.next.next();
        self.drag = Some(Drag {
            id: self.next,
            label: Arc::from(cx.words::<ConnectorWords>().draw.as_str()),
            routing: self.routing,
            from: placed.end,
            from_route: placed.route,
            start_target: glued(&placed.end),
            press: pointer.pos,
            began: false,
            value: None,
            changed: false,
            sent: None,
            preview: Vec::new(),
        });
        ToolAction::Claimed
    }

    fn drag(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        // inv-figcraft: crates/engine/src/boards/board/tools.rs:102-245 (the preview route).
        let Some(mut d) = self.drag.take() else { return };
        self.advance(&mut d, pointer, cx, true);
        self.drag = Some(d);
    }

    fn release(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        // inv-figcraft: crates/engine/src/boards/board/tools.rs:129 and :218 (a drag under the slop
        // makes nothing; ending on the start object makes nothing).
        let Some(mut d) = self.drag.take() else { return };
        self.advance(&mut d, pointer, cx, false);
        self.hover = None;
        if !d.began {
            return;
        }
        // A route that can't be placed there makes nothing.
        let phase = if d.value.is_none() {
            Phase::Cancel(CancelReason::Refused)
        } else if d.changed {
            Phase::Commit
        } else {
            Phase::Cancel(CancelReason::Dismissed)
        };
        self.events.extend(event(&d, phase));
    }

    fn hover(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        // inv-figcraft: crates/engine/src/boards/board/tools.rs:171-174 `connector_hover`.
        let Some(space) = cx.space() else { return };
        let placed = end_at(&self.targets, space.view.to_doc(pointer.pos), &space.view, cx.reach(), pointer.mods);
        self.hover = placed.target.map(|t| (t, placed.active));
    }

    fn cancel(&mut self, reason: CancelReason, _cx: &mut ToolContext<'_>) {
        self.hover = None;
        if let Some(d) = self.drag.take()
            && d.began
        {
            self.events.extend(event(&d, Phase::Cancel(reason)));
        }
    }

    fn paint(&self, draw: &Draw<'_>, cx: &ToolContext<'_>) {
        // inv-figcraft: crates/ui-egui/src/canvas/overlays.rs:407-418 `attach_points`, :73-79 (the
        // drop target outlined).
        let (Some(space), Some(style)) = (cx.space(), cx.style()) else { return };
        let look = ConnectorLook::of(style);
        if let Some((id, active)) = self.hover
            && let Some(t) = self.targets.iter().find(|t| t.id == id)
        {
            paint_sites(draw, &look, &space.view, t, active);
        }
        if let Some(d) = self.drag.as_ref().filter(|d| d.began && !self.custom_preview) {
            paint_route(draw, &look, &d.preview, d.routing, &space.view);
        }
    }

    fn cursor(&self, _cx: &ToolContext<'_>) -> Cursor {
        // A site the end would glue to is a pointing hand; everywhere else, and while drawing, the
        // crosshair. Over a handle the handles' parts set the move cursor.
        let over_site = self.drag.is_none() && self.hover.is_some_and(|(_, active)| active.is_some());
        Cursor::System(if over_site { CursorIcon::PointingHand } else { CursorIcon::Crosshair })
    }

    fn access(&self, cx: &ToolContext<'_>) -> Option<ToolAccess> {
        // The connector being drawn, as one graphics object over the route's bounds.
        let d = self.drag.as_ref().filter(|d| d.began && d.preview.len() >= 2)?;
        let bounds = d.preview.iter().fold(Rect::from_points(d.preview[0], d.preview[1]), |r, &p| r.union_pt(p));
        Some(ToolAccess::new(cx.words::<ConnectorWords>().group.as_str()).bounds(bounds))
    }

    fn command_form(&self) -> ToolPath {
        ToolPath::Command("the connector handles' arrow keys")
    }
}
