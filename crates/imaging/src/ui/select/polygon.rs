//! [`PolygonLassoTool`]: the Polygonal Lasso, click by click.

use craft_behavior::geometry::{DraftEvent, DraftKind, DraftPoint, DraftReach, OutlineDraft, SelectionOperation};
use craft_behavior::{CancelReason, Cursor, PointerButton};
use craft_keymap::Mods;
use craft_ui::canvas::{CanvasTool, ToolAccess, ToolAction, ToolClaims, ToolContext, ToolKey, ToolPath, ToolPointer};
use craft_ui::primitives::{Draw, ToolIcon};
use kurbo::Point;

use super::drag::{InsideDrag, PressRule, drag_kind, moved_selection_snap};
use super::lasso::{DraftKey, LassoCursor, apply, begin_inside, cursor, draft_command, outline_access, paint_outline, phase, reach, sample};
use super::{Place, Report, SelectEvent, SelectPhase, SelectToolPass, SelectionProbe};

/// The Polygonal Lasso: each click adds a vertex; the outline closes into a selection.
///
/// The drawing rules are [`OutlineDraft`]'s with [`DraftKind::Polygonal`]: Shift (without Alt) puts the new vertex in
/// the nearest of eight directions from the last vertex (not the first vertex), an Alt drag draws freehand. The first
/// click latches the operation (Alt with nothing selected is New). The outline closes on a click near the first
/// vertex (3 or more), on a click at the last, on Enter or a double-click. Backspace, Delete or a secondary click
/// retract the last vertex (the last one cancels): the tool takes the secondary button while an outline is open, so
/// the app calls nothing and shows no context menu then. Escape cancels, between clicks and during a press alike; a
/// document change or [`reset`](Self::reset) drops it. While an outline is open, hovering near the canvas edge
/// scrolls the view toward it (`CanvasTool::autoscrolls_on_hover`).
///
/// Click-driven: a plain press inside the selection places a vertex. Only Cmd drags the selection (pixels; module
/// docs), and not while an outline is open. A Cmd press outside the selection is `ToolAction::Ignored`.
///
/// While a New outline is open the selection it will replace is hidden: [`replaces_selection`](Self::replaces_selection)
/// is for the selection overlay's `visible`.
///
/// Vertex history, for an app's Edit › Undo and Redo while the tool owns them: [`undo_vertex`](Self::undo_vertex)
/// retracts as Backspace does and [`redo_vertex`](Self::redo_vertex) puts the retracted vertices back in order (every
/// retract counts; a new vertex forgets them). The last outline closed is remembered as it was just before the close
/// (points, straight segments, operation): after undoing its selection the app calls [`reopen`](Self::reopen) to
/// carry on drawing it, and [`reclose`](Self::reclose) drops it again for a redo of that selection. The memory lasts
/// until the next outline begins, another document is active, [`reset`](Self::reset) or
/// [`forget_closed`](Self::forget_closed); the app decides whether its history still allows a reopen.
#[derive(Clone, Debug)]
pub struct PolygonLassoTool {
    draft: OutlineDraft,
    bar: SelectionOperation,
    operation: SelectionOperation,
    mods: Mods,
    inside: Option<InsideDrag>,
    report: Report,
    /// Retracted vertices, the latest last, each with whether its incoming segment was straight.
    undone: Vec<(Point, bool)>,
    /// The last outline closed, as it was just before the close.
    closed: Option<ClosedOutline>,
    /// The outline [`reopen`](PolygonLassoTool::reopen) gave back, for [`reclose`](PolygonLassoTool::reclose).
    reopened: Option<ClosedOutline>,
}

/// An outline as it was just before it closed, and the document it was drawn on.
#[derive(Clone, Debug, PartialEq)]
struct ClosedOutline {
    points: Vec<Point>,
    straight: Vec<usize>,
    operation: SelectionOperation,
    document: u64,
}

impl Default for PolygonLassoTool {
    fn default() -> Self {
        Self::new()
    }
}

impl PolygonLassoTool {
    /// A Polygonal Lasso with the operation New and no outline.
    pub fn new() -> Self {
        Self {
            // The reach comes from the pass's slop in every pass (`each_pass`, `OutlineDraft::set_reach`).
            draft: OutlineDraft::new(DraftKind::Polygonal, DraftReach::new(0.0, 0.0)),
            bar: SelectionOperation::New,
            operation: SelectionOperation::New,
            mods: Mods::NONE,
            inside: None,
            report: Report::default(),
            undone: Vec::new(),
            closed: None,
            reopened: None,
        }
    }

    /// The options bar's operation for outlines begun from now on.
    pub fn operation(&mut self, bar: SelectionOperation) -> &mut Self {
        self.bar = bar;
        self
    }

    /// The tool as the `CanvasTool` for one pass, asking `probe` about the document.
    pub fn pass<'a>(&'a mut self, probe: &'a dyn SelectionProbe) -> SelectToolPass<'a, Self> {
        SelectToolPass::new(self, probe)
    }

    /// The events since the last call, in order.
    pub fn take_events(&mut self) -> Vec<SelectEvent> {
        self.report.take()
    }

    /// Drop the outline, any selection drag, the vertex history and the closed outline without an event (a tool
    /// switch).
    pub fn reset(&mut self) {
        self.draft.cancel();
        self.inside = None;
        self.undone.clear();
        self.forget_closed();
    }

    /// The open outline's points; empty with none.
    pub fn points(&self) -> &[Point] {
        self.draft.points()
    }

    /// The operation the open outline latched at its first press (or had when it closed, once reopened); `None` while
    /// no outline is open.
    pub fn outline_operation(&self) -> Option<SelectionOperation> {
        self.draft.is_open().then_some(self.operation)
    }

    /// Whether [`undo_vertex`](Self::undo_vertex) acts: an outline is open between clicks.
    pub fn can_undo_vertex(&self) -> bool {
        self.draft.takes_retract()
    }

    /// Retracts the last vertex as Backspace does, keeping it for [`redo_vertex`](Self::redo_vertex); the only vertex
    /// cancels the outline (and nothing is kept). False when no outline is open between clicks.
    pub fn undo_vertex(&mut self) -> bool {
        self.retract()
    }

    /// Whether [`redo_vertex`](Self::redo_vertex) acts: a retracted vertex waits and the outline is between clicks.
    pub fn can_redo_vertex(&self) -> bool {
        self.draft.takes_retract() && !self.undone.is_empty()
    }

    /// Puts back the vertex retracted last, with its straight segment. False when there is none.
    pub fn redo_vertex(&mut self) -> bool {
        if !self.can_redo_vertex() {
            return false;
        }
        let Some((point, straight)) = self.undone.pop() else { return false };
        let (mut points, mut indices) = (self.draft.points().to_vec(), self.draft.straight().to_vec());
        if straight {
            indices.push(points.len());
        }
        points.push(point);
        self.draft.cancel();
        self.draft.reopen(points, indices);
        true
    }

    /// Whether [`reopen`](Self::reopen) acts: an outline closed since the last one began, and none is open.
    pub fn can_reopen(&self) -> bool {
        !self.draft.is_open() && self.closed.is_some()
    }

    /// Opens the last closed outline again as it was just before the close, with its operation, waiting for the next
    /// click. Call it once the selection it made has been undone. False, changing nothing, without one.
    pub fn reopen(&mut self) -> bool {
        if !self.can_reopen() {
            return false;
        }
        let Some(closed) = self.closed.take() else { return false };
        if self.draft.reopen(closed.points.clone(), closed.straight.clone()).is_none() {
            return false;
        }
        self.operation = closed.operation;
        self.undone.clear();
        self.reopened = Some(closed);
        true
    }

    /// Whether [`reclose`](Self::reclose) acts: the open outline is the one [`reopen`](Self::reopen) gave back,
    /// unchanged, with no retracted vertex waiting.
    pub fn can_reclose(&self) -> bool {
        self.draft.takes_retract()
            && self.undone.is_empty()
            && self.reopened.as_ref().is_some_and(|r| r.points == self.draft.points() && r.straight == self.draft.straight())
    }

    /// Drops the reopened outline without an event and remembers it as closed again: the app redid the selection it
    /// had made. False, changing nothing, unless [`can_reclose`](Self::can_reclose).
    pub fn reclose(&mut self) -> bool {
        if !self.can_reclose() {
            return false;
        }
        self.draft.cancel();
        self.closed = self.reopened.take();
        true
    }

    /// Forgets the closed outline: the app's history has moved on from the selection it made.
    pub fn forget_closed(&mut self) {
        self.closed = None;
        self.reopened = None;
    }

    /// Retracts the last vertex (Backspace, Delete, a secondary press, Undo) and keeps it for a redo. Whether it acted.
    fn retract(&mut self) -> bool {
        let Some(last) = self.draft.points().len().checked_sub(1).filter(|_| self.draft.takes_retract()) else { return false };
        let vertex = (self.draft.points()[last], self.draft.straight().contains(&last));
        match self.draft.retract() {
            Some(DraftEvent::Changed) => self.undone.push(vertex),
            _ => self.undone.clear(),
        }
        true
    }

    /// Turns a draft's answer into events (`apply`), remembering an outline as it closes and forgetting the last closed
    /// one when a new outline begins. `before` is the point count before the call: a vertex added forgets the retracted
    /// ones, as does the outline ending.
    fn take(&mut self, event: Option<DraftEvent>, has_selection: bool, document: u64, before: usize) {
        match &event {
            Some(DraftEvent::Started { .. }) => self.forget_closed(),
            Some(DraftEvent::Closed { points, straight, .. }) => {
                let closed = ClosedOutline { points: points.clone(), straight: straight.clone(), operation: self.operation, document };
                self.closed = Some(closed);
                self.reopened = None;
            }
            _ => {}
        }
        if !self.draft.is_open() || self.draft.points().len() > before {
            self.undone.clear();
        }
        apply(event, self.bar, has_selection, &mut self.operation, &mut self.report);
    }

    /// What the tool is doing.
    pub fn phase(&self) -> SelectPhase {
        phase(&self.draft, self.inside.as_ref())
    }

    /// An outline with the operation New is open: the app hides the selection's ants.
    pub fn replaces_selection(&self) -> bool {
        self.draft.is_open() && self.operation == SelectionOperation::New
    }
}

impl CanvasTool for SelectToolPass<'_, PolygonLassoTool> {
    /// The reach, the document check, modifier changes to the draft, and the kept keys while an outline is open.
    fn each_pass(&mut self, cx: &mut ToolContext<'_>) {
        let (tool, probe) = (&mut *self.tool, self.probe);
        tool.draft.set_reach(reach(cx));
        if tool.draft.for_document(probe.document()).is_some() {
            tool.inside = None;
            tool.undone.clear();
        }
        if tool.closed.as_ref().or(tool.reopened.as_ref()).is_some_and(|c| c.document != probe.document()) {
            tool.forget_closed();
        }
        if cx.mods != tool.mods {
            tool.mods = cx.mods;
            let before = tool.draft.points().len();
            let event = tool.draft.mods_changed(cx.mods);
            tool.take(event, probe.has_selection(), probe.document(), before);
        }
    }

    fn claims(&self) -> ToolClaims {
        if self.tool.draft.takes_retract() { ToolClaims::NONE.keyboard().deletes() } else { ToolClaims::NONE }
    }

    /// The rubber band of an open outline reaches past the view: hovering near the canvas edge scrolls toward it.
    fn autoscrolls_on_hover(&self) -> bool {
        self.tool.draft.is_open()
    }

    /// The primary button always, the secondary while an outline is open (it retracts the last vertex).
    fn takes(&self, button: PointerButton, _mods: Mods) -> bool {
        button == PointerButton::Primary || (button == PointerButton::Secondary && self.tool.draft.is_open())
    }

    fn cursor(&self, cx: &ToolContext<'_>) -> Cursor {
        let tool = &*self.tool;
        let rule = PressRule::Clicks { outline_open: tool.draft.is_open() };
        let shown = LassoCursor {
            icon: ToolIcon::PolygonalLasso,
            draft: &tool.draft,
            operation: tool.operation,
            bar: tool.bar,
            inside: tool.inside.as_ref(),
            rule,
        };
        cursor(&shown, self.probe, cx)
    }

    /// A secondary press retracts the last vertex (`ToolAction::Done`: no drag follows). Cmd inside the selection with
    /// no outline open: a pixel drag. Otherwise the click goes to the draft (the first latches the operation); a
    /// double-click closes.
    fn press(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) -> ToolAction {
        let (tool, probe) = (&mut *self.tool, self.probe);
        let Some(place) = Place::of(cx) else { return ToolAction::Ignored };
        if pointer.button == PointerButton::Secondary {
            tool.retract();
            return ToolAction::Done;
        }
        let open = tool.draft.is_open();
        if let Some(kind) = drag_kind(probe, place.doc(pointer.pos), pointer.mods, tool.bar, PressRule::Clicks { outline_open: open }) {
            begin_inside(kind, pointer, &place, cx, probe.bounds(), &mut tool.inside, &mut tool.report);
            return ToolAction::Claimed;
        }
        if pointer.mods.cmd && !open {
            return ToolAction::Ignored;
        }
        let before = tool.draft.points().len();
        let event = if open && pointer.clicks >= 2 { tool.draft.close() } else { tool.draft.press(sample(pointer, &place), probe.has_selection()) };
        tool.take(event, probe.has_selection(), probe.document(), before);
        ToolAction::Claimed
    }

    /// An Alt drag adds freehand points; a selection drag moves.
    fn drag(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        let (tool, probe) = (&mut *self.tool, self.probe);
        let Some(place) = Place::of(cx) else { return };
        if let Some(inside) = tool.inside.as_mut() {
            if let Some(event) = inside.drag(place.doc(pointer.pos), pointer.mods, place.zoom(), &mut |rect| moved_selection_snap(cx, rect)) {
                tool.report.push(event);
            }
            return;
        }
        let before = tool.draft.points().len();
        let event = tool.draft.drag(sample(pointer, &place));
        tool.take(event, probe.has_selection(), probe.document(), before);
    }

    /// Ends an Alt-drag segment or a selection drag; the outline stays open.
    fn release(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        let (tool, probe) = (&mut *self.tool, self.probe);
        let Some(place) = Place::of(cx) else { return };
        if let Some(inside) = tool.inside.take() {
            tool.report.push(inside.end(place.doc(pointer.pos), pointer.mods, place.zoom(), &mut |rect| moved_selection_snap(cx, rect)));
            return;
        }
        let before = tool.draft.points().len();
        let event = tool.draft.release(sample(pointer, &place));
        tool.take(event, probe.has_selection(), probe.document(), before);
    }

    /// The rubber band follows the pointer; it ends where a click would place the vertex.
    fn hover(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        let tool = &mut *self.tool;
        let Some(place) = Place::of(cx) else { return };
        let at = sample(pointer, &place);
        if tool.draft.hover(at).is_some() {
            cx.request_repaint();
        }
    }

    /// Escape during a press drops the whole outline, as Escape between clicks does. Any other reason (a lost
    /// pointer, a tool switch) ends an Alt-drag segment where it is and keeps the outline (the app calls
    /// [`reset`](PolygonLassoTool::reset) to drop it). A selection drag reports `Cancel`.
    fn cancel(&mut self, reason: CancelReason, _cx: &mut ToolContext<'_>) {
        let tool = &mut *self.tool;
        if let Some(inside) = tool.inside.take() {
            tool.report.push(inside.cancel(reason));
        }
        if reason == CancelReason::Escape {
            tool.draft.cancel();
            tool.undone.clear();
            return;
        }
        if tool.draft.is_down()
            && let Some(&last) = tool.draft.points().last()
        {
            tool.draft.release(DraftPoint::new(last, Mods::NONE, 1.0));
        }
    }

    /// Between clicks: Enter closes, Escape cancels, Backspace and Delete retract (kept for a redo).
    fn key(&mut self, key: &ToolKey, cx: &mut ToolContext<'_>) -> bool {
        let (tool, probe) = (&mut *self.tool, self.probe);
        if !tool.draft.takes_retract() {
            return false;
        }
        let Some((command, _)) = draft_command(key, cx) else { return false };
        match command {
            DraftKey::Close => {
                let before = tool.draft.points().len();
                let event = tool.draft.close();
                tool.take(event, probe.has_selection(), probe.document(), before);
            }
            DraftKey::Cancel => {
                tool.draft.cancel();
                tool.undone.clear();
            }
            DraftKey::Retract => {
                tool.retract();
            }
        }
        true
    }

    fn paint(&self, draw: &Draw<'_>, cx: &ToolContext<'_>) {
        paint_outline(&self.tool.draft, draw, cx);
    }

    fn access(&self, cx: &ToolContext<'_>) -> Option<ToolAccess> {
        outline_access(self.tool.draft.points(), cx)
    }

    /// A selection drag takes every move from the press, so a drag of a pixel or two moves the selection;
    /// [`MoveGesture::left_press`](craft_behavior::geometry::MoveGesture::left_press) alone decides its click.
    fn takes_every_move(&self) -> bool {
        self.tool.inside.is_some()
    }

    fn command_form(&self) -> ToolPath {
        ToolPath::Command("the Select Area... command")
    }
}
