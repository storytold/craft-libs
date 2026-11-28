//! [`LassoTool`]: the freehand Lasso with Alt-latched straight segments, and the pieces the Polygonal Lasso shares.

use craft_behavior::geometry::{ConstrainStyle, DraftEvent, DraftKind, DraftPoint, DraftReach, OutlineDraft, SelectionOperation};
use craft_behavior::{CancelReason, Cursor, CursorIcon, PaintedCursor, PointerButton, PointerGlyph};
use craft_keymap::{Key, Mods};
use craft_ui::canvas::{CanvasTool, ToolAccess, ToolAction, ToolClaims, ToolContext, ToolKey, ToolPath, ToolPointer};
use craft_ui::primitives::{Draw, PaintShape, StrokeWidth, ToolIcon};
use egui::Pos2;
use kurbo::Rect;

use super::drag::{InsideDrag, PressRule, cursor_over, drag_kind, moved_selection_snap};
use super::{DragKind, Place, Report, SelectEvent, SelectPhase, SelectToolPass, SelectWords, SelectionProbe, SelectionShape};

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
        return SelectPhase::Dragging(inside.selection_drag());
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

/// The arrow with its tip on the point the outline starts from, the lasso icon below and right of it and the
/// operation's badge beside the icon: the latched one while an outline is open, else the live modifiers';
/// Cmd is the Move tool's, so it shows none. Under the canvas's precise cursor, the crosshair with the badge. Over the
/// selection, the cursors of [`cursor_over`].
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
    // Under the canvas's precise cursor (Other Cursors > Precise, swapped by Caps Lock) the pointer is the crosshair,
    // its centre the point the outline takes, with the operation's badge.
    if cx.precise() {
        return badge.map_or(CursorIcon::Crosshair.into(), |badge| PaintedCursor::badged(CursorIcon::Crosshair, badge).into());
    }
    PaintedCursor::pointer_tool_badged(PointerGlyph::Arrow, icon.icon(), badge).into()
}

/// The outline as marching ants and, between straight segments, the rubber band: the one painter of an
/// [`OutlineDraft`], for the lassos and the app's Freeform Pen.
pub fn paint_outline(draft: &OutlineDraft, draw: &Draw<'_>, cx: &ToolContext<'_>) {
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

/// Starts the selection drag for `kind` at the press, snapping by the selection's `bounds`.
pub(super) fn begin_inside(
    kind: DragKind,
    pointer: &ToolPointer,
    place: &Place,
    cx: &ToolContext<'_>,
    bounds: Option<Rect>,
    inside: &mut Option<InsideDrag>,
    report: &mut Report,
) {
    let id = report.gesture();
    let label = cx.words::<SelectWords>().move_selection.as_str().into();
    let style = ConstrainStyle::new(f64::from(cx.slop), f64::from(cx.slop));
    let (drag, event) = InsideDrag::begin(&style, kind, place.doc(pointer.pos), pointer.mods, id, label, bounds);
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
/// or more points), on a click at the last, on Enter or on a double-click. Backspace, Delete or a secondary click
/// retract the last vertex (the last one cancels); the tool claims those keys from Edit > Clear
/// (`CanvasTool::claims`) and takes the secondary button while [`OutlineDraft::takes_retract`], so the app shows no
/// context menu then. Escape cancels. The outline is dropped when the document changes
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

    /// An outline with the operation New is open: the app hides the selection's ants.
    pub fn replaces_selection(&self) -> bool {
        self.outline_operation() == Some(SelectionOperation::New)
    }

    /// The operation the open outline latched at its first press, whatever is held now (the cursor's badge and the
    /// commit agree); `None` while no outline is open.
    pub fn outline_operation(&self) -> Option<SelectionOperation> {
        self.draft.is_open().then_some(self.operation)
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

    /// The primary button always, the secondary between straight-segment clicks (it retracts the last vertex).
    fn takes(&self, button: PointerButton, _mods: Mods) -> bool {
        button == PointerButton::Primary || (button == PointerButton::Secondary && self.tool.draft.takes_retract())
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

    /// A secondary press retracts the last vertex (`ToolAction::Done`: no drag follows; the last one cancels the
    /// outline with no event). With no outline open: a selection drag inside the selection, `Ignored` for Cmd
    /// outside, else the outline begins (`OutlineDraft::press`; the operation resolves from the `Started` intent).
    /// With one open: the press goes to the draft (a vertex, or a close). A second click of a run closes
    /// (`OutlineDraft::close`).
    fn press(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) -> ToolAction {
        let (tool, probe) = (&mut *self.tool, self.probe);
        if pointer.button == PointerButton::Secondary {
            tool.draft.retract();
            return ToolAction::Done;
        }
        let Some(place) = Place::of(cx) else { return ToolAction::Ignored };
        let open = tool.draft.is_open();
        if !open {
            if let Some(kind) = drag_kind(probe, place.doc(pointer.pos), pointer.mods, tool.bar, PressRule::Drags) {
                begin_inside(kind, pointer, &place, cx, probe.bounds(), &mut tool.inside, &mut tool.report);
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
            if let Some(event) = inside.drag(place.doc(pointer.pos), pointer.mods, place.zoom(), &mut |rect| moved_selection_snap(cx, rect)) {
                tool.report.push(event);
            }
            return;
        }
        // Space held moves the outline being drawn by the pointer's step instead of adding points to it.
        let event = if cx.held(Key::Space) { tool.draft.move_to(sample(pointer, &place)) } else { tool.draft.drag(sample(pointer, &place)) };
        apply(event, tool.bar, probe.has_selection(), &mut tool.operation, &mut tool.report);
    }

    /// The release closes a freehand outline (`Commit`) unless straight segments are on; a freehand outline of fewer
    /// than 3 points (a click) is [`SelectEvent::Deselect`] when its operation is New, and nothing for Add, Subtract
    /// and Intersect, which keep the selection. Ends a selection drag.
    fn release(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        let (tool, probe) = (&mut *self.tool, self.probe);
        let Some(place) = Place::of(cx) else { return };
        if let Some(inside) = tool.inside.take() {
            tool.report.push(inside.end(place.doc(pointer.pos), pointer.mods, place.zoom(), &mut |rect| moved_selection_snap(cx, rect)));
            return;
        }
        if cx.held(Key::Space) {
            tool.draft.move_to(sample(pointer, &place));
        }
        let event = tool.draft.release(sample(pointer, &place));
        if event == Some(DraftEvent::Cancelled) {
            if tool.operation == SelectionOperation::New {
                tool.report.push(SelectEvent::Deselect);
            }
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

    /// A selection drag and an outline with the button down take every move from the press: a drag of a pixel or two
    /// moves the selection, and the outline follows the pointer from where it went down, so a lasso drawn inside the
    /// click distance still selects. [`MoveGesture::left_press`](craft_behavior::geometry::MoveGesture::left_press)
    /// decides a selection drag's click, and the draft an outline's (fewer than 3 points, [`OutlineDraft::close`]).
    fn takes_every_move(&self) -> bool {
        self.tool.inside.is_some() || self.tool.draft.is_down()
    }

    fn command_form(&self) -> ToolPath {
        ToolPath::Freehand("the Select Area... command takes the same outline as points")
    }
}
