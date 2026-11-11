//! [`LassoTool`]: the freehand Lasso with Alt-latched straight segments, and the pieces the Polygonal Lasso shares.

use craft_behavior::geometry::{ConstrainStyle, DraftEvent, DraftKind, DraftPoint, DraftReach, OutlineDraft, SelectionOperation};
use craft_behavior::{CancelReason, Cursor, PaintedCursor};
use craft_keymap::Mods;
use craft_ui::canvas::{CanvasTool, ToolAccess, ToolAction, ToolClaims, ToolContext, ToolKey, ToolPath, ToolPointer};
use craft_ui::primitives::{Draw, PaintShape, StrokeWidth, ToolIcon};
use egui::{Pos2, Vec2};
use kurbo::Rect;

use super::drag::{InsideDrag, PressRule, cursor_over, drag_kind};
use super::{DragKind, Place, Report, SelectEvent, SelectPhase, SelectToolPass, SelectWords, SelectionProbe, SelectionShape};

/// Where a lasso cursor's icon touches the document: the icon's own origin.
const ICON_HOTSPOT: Vec2 = Vec2::ZERO;

/// The reach of an outline's closing presses in this pass: a press within the slop of the first vertex or of the last
/// closes.
pub(super) fn reach(cx: &ToolContext<'_>) -> DraftReach {
    DraftReach::new(f64::from(cx.slop), f64::from(cx.slop))
}

/// Turns a draft's answer into the tool's events: `Started` latches the operation from the press's intent, `Closed`
/// is the commit. Shared by the Lasso and the Polygonal Lasso.
pub(super) fn apply(
    event: Option<DraftEvent>,
    bar: SelectionOperation,
    has_selection: bool,
    operation: &mut SelectionOperation,
    report: &mut Report,
) {
    match event {
        Some(DraftEvent::Started { intent, .. }) => *operation = SelectionOperation::resolve(bar, intent, has_selection),
        Some(DraftEvent::Closed { points, .. }) => report.push(SelectEvent::Commit { shape: SelectionShape::Polygon(points), operation: *operation }),
        _ => {}
    }
}

/// What a lasso is doing: a selection drag, an outline with its point count, or nothing.
pub(super) fn phase(draft: &OutlineDraft, inside: Option<&InsideDrag>) -> SelectPhase {
    if let Some(inside) = inside {
        return SelectPhase::Dragging(inside.kind());
    }
    if draft.is_open() { SelectPhase::Outline { points: draft.points().len() } } else { SelectPhase::Idle }
}

/// What a lasso cursor shows: the tool's icon, and the state that picks its badge.
pub(super) struct LassoCursor<'a> {
    pub(super) icon: ToolIcon,
    pub(super) draft: &'a OutlineDraft,
    pub(super) operation: SelectionOperation,
    pub(super) bar: SelectionOperation,
    pub(super) inside: Option<&'a InsideDrag>,
    pub(super) rule: PressRule,
}

/// The lasso icon with the operation's badge: the latched one while an outline is open, else the live modifiers';
/// Cmd is the Move tool's, so it shows none. Over the selection, the cursors of [`cursor_over`].
pub(super) fn cursor(shown: &LassoCursor<'_>, probe: &dyn SelectionProbe, cx: &ToolContext<'_>) -> Cursor {
    let LassoCursor { icon, draft, operation, bar, inside, rule } = shown;
    if !draft.is_open()
        && let (Some(pointer), Some(place)) = (cx.pointer, Place::of(cx))
        && let Some(cursor) = cursor_over(probe, place.doc(pointer.pos), cx.mods, *bar, *rule, inside.map(InsideDrag::kind))
    {
        return cursor;
    }
    let badge = if draft.is_open() {
        operation.badge()
    } else if cx.mods.cmd {
        None
    } else {
        SelectionOperation::resolve(*bar, cx.mods, probe.has_selection()).badge()
    };
    PaintedCursor::tool(icon.icon(), ICON_HOTSPOT, badge).into()
}

/// The outline as marching ants and, between straight segments, the rubber band.
pub(super) fn paint_outline(draft: &OutlineDraft, draw: &Draw<'_>, cx: &ToolContext<'_>) {
    let Some(place) = Place::of(cx) else { return };
    let points: Vec<Pos2> = draft.points().iter().map(|&at| place.screen(at)).collect();
    if points.len() >= 2 {
        cx.marching_ants(&points, false);
    }
    if let Some([a, b]) = draft.rubber_band()
        && let Some(style) = cx.style()
    {
        let overlay = &style.tokens.color.overlay;
        let band = PaintShape::Path(vec![place.screen(a), place.screen(b)]);
        draw.stroke(&band, StrokeWidth::Medium, style.color(overlay.halo));
        draw.stroke(&band, StrokeWidth::Thin, style.color(overlay.marquee));
    }
}

/// The open outline as one object: its name and point count over the points' bounds; `None` with no outline.
pub(super) fn outline_access(points: &[kurbo::Point], cx: &ToolContext<'_>) -> Option<ToolAccess> {
    let first = *points.first()?;
    let bounds = points.iter().fold(Rect::from_points(first, first), |rect, &p| rect.union_pt(p));
    let words = cx.words::<SelectWords>();
    let count = points.len() as u64;
    Some(ToolAccess::new(words.outline.as_str()).value(words.outline_points.text(count, &count.to_string())).bounds(bounds))
}

/// Starts the selection drag for `kind` at the press.
pub(super) fn begin_inside(
    kind: DragKind,
    pointer: &ToolPointer,
    place: &Place,
    cx: &ToolContext<'_>,
    inside: &mut Option<InsideDrag>,
    report: &mut Report,
) {
    let id = report.gesture();
    let label = cx.words::<SelectWords>().move_selection.as_str().into();
    let style = ConstrainStyle::new(f64::from(cx.slop), f64::from(cx.slop));
    let (drag, event) = InsideDrag::begin(&style, kind, place.doc(pointer.pos), pointer.mods, id, label);
    *inside = Some(drag);
    report.push(event);
}

/// The draft sample for `pointer`.
pub(super) fn sample(pointer: &ToolPointer, place: &Place) -> DraftPoint {
    DraftPoint::new(place.doc(pointer.pos), pointer.mods, place.zoom())
}

/// What a key does to an open outline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DraftKey {
    /// Enter.
    Close,
    /// Escape.
    Cancel,
    /// Backspace or Delete.
    Retract,
}

/// The outline command a key is, with the modifiers it carries. The host sends the editing keys as `Submit`, `Cancel`
/// and `Delete` while the tool has not kept the keyboard, and as the chord itself while it has (`ToolClaims::keyboard`). Chords
/// with Cmd or Ctrl are the app's.
pub(super) fn draft_command(key: &ToolKey, cx: &ToolContext<'_>) -> Option<(DraftKey, Mods)> {
    let (command, mods) = match key {
        ToolKey::Submit => (DraftKey::Close, cx.mods),
        ToolKey::Cancel => (DraftKey::Cancel, cx.mods),
        ToolKey::Delete { .. } => (DraftKey::Retract, cx.mods),
        _ => return None,
    };
    (!mods.cmd && !mods.ctrl).then_some((command, mods))
}

/// Enter closes, Escape cancels, Backspace and Delete retract, while the draft takes them. `true` when the key was
/// taken. Alt may be held: the Lasso retracts only between Alt clicks.
pub(super) fn draft_key(
    draft: &mut OutlineDraft,
    key: &ToolKey,
    cx: &ToolContext<'_>,
    bar: SelectionOperation,
    has_selection: bool,
    operation: &mut SelectionOperation,
    report: &mut Report,
) -> bool {
    if !draft.takes_retract() {
        return false;
    }
    let Some((command, _)) = draft_command(key, cx) else { return false };
    match command {
        DraftKey::Close => {
            let event = draft.close();
            apply(event, bar, has_selection, operation, report);
        }
        DraftKey::Cancel => {
            draft.cancel();
        }
        DraftKey::Retract => {
            draft.retract();
        }
    }
    true
}

/// The Lasso: a drag draws a freehand outline; the release closes it into a selection.
///
/// The drawing rules are [`OutlineDraft`]'s with [`DraftKind::Mixed`]: Alt pressed after the press switches to
/// straight segments (each click adds a vertex, a drag draws freehand again), and releasing Alt with the button up
/// closes; Alt held at the press subtracts (the operation) until it is released and pressed again, except with nothing
/// selected, where it starts straight segments. Between clicks the outline closes on a click near the first point (3
/// or more points), on a click at the last, on Enter or on a double-click. Backspace or Delete retract the last
/// vertex (the last one cancels); the tool claims those keys from Edit > Clear (`CanvasTool::claims`) while
/// [`OutlineDraft::takes_retract`]. Escape cancels. The outline is dropped when the document changes
/// ([`SelectionProbe::document`]) and on [`reset`](Self::reset).
///
/// A press inside the selection drags it (module docs) instead of drawing; a Cmd press outside it is
/// `ToolAction::Ignored`. The closed outline is [`SelectEvent::Commit`] with a `SelectionShape::Polygon` and the
/// operation latched at the first press.
#[derive(Clone, Debug)]
pub struct LassoTool {
    draft: OutlineDraft,
    bar: SelectionOperation,
    /// The operation latched at the outline's first press.
    operation: SelectionOperation,
    /// The modifiers of the last pass, to report changes to the draft.
    mods: craft_keymap::Mods,
    inside: Option<InsideDrag>,
    report: Report,
}

impl Default for LassoTool {
    fn default() -> Self {
        Self::new()
    }
}

impl LassoTool {
    /// A Lasso with the operation New and no outline.
    pub fn new() -> Self {
        Self {
            // The reach comes from the pass's slop in every pass (`each_pass`, `OutlineDraft::set_reach`).
            draft: OutlineDraft::new(DraftKind::Mixed, DraftReach::new(0.0, 0.0)),
            bar: SelectionOperation::New,
            operation: SelectionOperation::New,
            mods: craft_keymap::Mods::NONE,
            inside: None,
            report: Report::default(),
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

    /// Drop the outline and any selection drag without an event: the app switched tools (an outline open between
    /// clicks has no host gesture for `ToolHost::cancel` to end).
    pub fn reset(&mut self) {
        self.draft.cancel();
        self.inside = None;
    }

    /// What the tool is doing: an outline with its point count, or a selection drag.
    pub fn phase(&self) -> SelectPhase {
        phase(&self.draft, self.inside.as_ref())
    }
}

impl CanvasTool for SelectToolPass<'_, LassoTool> {
    /// Takes the reach, checks the document (`OutlineDraft::for_document`), and passes a change of the held
    /// modifiers to `OutlineDraft::mods_changed` (Alt released with the button up closes: `Commit`). Claims the keys
    /// while the draft takes retract.
    fn each_pass(&mut self, cx: &mut ToolContext<'_>) {
        let (tool, probe) = (&mut *self.tool, self.probe);
        tool.draft.set_reach(reach(cx));
        if tool.draft.for_document(probe.document()).is_some() {
            tool.inside = None;
        }
        if cx.mods != tool.mods {
            tool.mods = cx.mods;
            let event = tool.draft.mods_changed(cx.mods);
            apply(event, tool.bar, probe.has_selection(), &mut tool.operation, &mut tool.report);
        }
    }

    fn claims(&self) -> ToolClaims {
        if self.tool.draft.takes_retract() { ToolClaims::NONE.keyboard().deletes() } else { ToolClaims::NONE }
    }

    fn cursor(&self, cx: &ToolContext<'_>) -> Cursor {
        let tool = &*self.tool;
        let shown = LassoCursor {
            icon: ToolIcon::Lasso,
            draft: &tool.draft,
            operation: tool.operation,
            bar: tool.bar,
            inside: tool.inside.as_ref(),
            rule: PressRule::Drags,
        };
        cursor(&shown, self.probe, cx)
    }

    /// With no outline open: a selection drag inside the selection, `Ignored` for Cmd outside, else the outline begins
    /// (`OutlineDraft::press`; the operation resolves from the `Started` intent). With one open: the press goes to
    /// the draft (a vertex, or a close). A second click of a run closes (`OutlineDraft::close`).
    fn press(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) -> ToolAction {
        let (tool, probe) = (&mut *self.tool, self.probe);
        let Some(place) = Place::of(cx) else { return ToolAction::Ignored };
        let open = tool.draft.is_open();
        if !open {
            if let Some(kind) = drag_kind(probe, place.doc(pointer.pos), pointer.mods, tool.bar, PressRule::Drags) {
                begin_inside(kind, pointer, &place, cx, &mut tool.inside, &mut tool.report);
                return ToolAction::Claimed;
            }
            if pointer.mods.cmd {
                return ToolAction::Ignored;
            }
        }
        let event = if open && pointer.clicks >= 2 { tool.draft.close() } else { tool.draft.press(sample(pointer, &place), probe.has_selection()) };
        apply(event, tool.bar, probe.has_selection(), &mut tool.operation, &mut tool.report);
        ToolAction::Claimed
    }

    /// Freehand points, or the selection drag.
    fn drag(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        let (tool, probe) = (&mut *self.tool, self.probe);
        let Some(place) = Place::of(cx) else { return };
        if let Some(inside) = tool.inside.as_mut() {
            if let Some(event) = inside.drag(place.doc(pointer.pos), pointer.mods, place.zoom()) {
                tool.report.push(event);
            }
            return;
        }
        let event = tool.draft.drag(sample(pointer, &place));
        apply(event, tool.bar, probe.has_selection(), &mut tool.operation, &mut tool.report);
    }

    /// The release closes a freehand outline (`Commit`) unless straight segments are on; a freehand outline of fewer
    /// than 3 points (a click) is [`SelectEvent::Deselect`]. Ends a selection drag.
    fn release(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        let (tool, probe) = (&mut *self.tool, self.probe);
        let Some(place) = Place::of(cx) else { return };
        if let Some(inside) = tool.inside.take() {
            tool.report.push(inside.end(place.doc(pointer.pos), pointer.mods, place.zoom()));
            return;
        }
        let event = tool.draft.release(sample(pointer, &place));
        if event == Some(DraftEvent::Cancelled) {
            tool.report.push(SelectEvent::Deselect);
            return;
        }
        apply(event, tool.bar, probe.has_selection(), &mut tool.operation, &mut tool.report);
    }

    /// Between clicks the rubber band follows the pointer (`OutlineDraft::hover`); never adds a vertex.
    fn hover(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        let Some(place) = Place::of(cx) else { return };
        if self.tool.draft.hover(sample(pointer, &place)).is_some() {
            cx.request_repaint();
        }
    }

    /// Escape during a drag, a lost pointer or `ToolHost::cancel`, whatever the reason: drops the outline (no event);
    /// a selection drag reports `Cancel`.
    fn cancel(&mut self, reason: CancelReason, _cx: &mut ToolContext<'_>) {
        let tool = &mut *self.tool;
        if let Some(inside) = tool.inside.take() {
            tool.report.push(inside.cancel(reason));
        }
        tool.draft.cancel();
    }

    /// Between clicks: Enter closes, Escape cancels, Backspace and Delete retract.
    fn key(&mut self, key: &ToolKey, cx: &mut ToolContext<'_>) -> bool {
        let (tool, probe) = (&mut *self.tool, self.probe);
        draft_key(&mut tool.draft, key, cx, tool.bar, probe.has_selection(), &mut tool.operation, &mut tool.report)
    }

    fn paint(&self, draw: &Draw<'_>, cx: &ToolContext<'_>) {
        paint_outline(&self.tool.draft, draw, cx);
    }

    fn access(&self, cx: &ToolContext<'_>) -> Option<ToolAccess> {
        outline_access(self.tool.draft.points(), cx)
    }

    fn command_form(&self) -> ToolPath {
        ToolPath::Freehand("the Select Area... command takes the same outline as points")
    }
}
