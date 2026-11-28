//! A press inside the selection: which drag it starts, the cursor over the selection, and the drag itself (the lasso
//! tools use it unchanged). Public for the app's own tools: the Magic Wand in New mode drags the outline like a
//! marquee, so it takes [`PressRule::Drags`].

use craft_behavior::geometry::{ConstrainStyle, Directions, MoveGesture, MoveValue, SelectionOperation, SnapKind, SnapKinds};
use craft_behavior::{CancelReason, Cursor, CursorBadge, CursorIcon, EditEvent, GestureId, Mixed, PaintedCursor};
use craft_keymap::Mods;
use kurbo::{Point, Vec2};
use std::sync::Arc;

use super::{DragKind, SelectEvent, SelectionDrag, SelectionProbe};

/// How a tool takes a press inside the selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PressRule {
    /// A drag tool (the marquees, the Lasso, the app's Magic Wand): a plain press with the operation New drags the
    /// outline, Cmd the pixels.
    Drags,
    /// A click-driven tool (the Polygonal Lasso): only Cmd drags, and never while `outline_open`.
    #[non_exhaustive]
    Clicks {
        /// The tool has an outline open (the Polygonal Lasso between clicks).
        outline_open: bool,
    },
}

impl PressRule {
    /// A click-driven tool, with or without an outline open.
    pub fn clicks(outline_open: bool) -> Self {
        Self::Clicks { outline_open }
    }
}

/// What a press at `at` with `mods` drags, or `None` when it draws (or is not the tool's).
///
/// With a floating piece: on it (`probe.on_floating`), Cmd without Shift drags the piece as `Pixels { copy: Alt }`,
/// and a plain press (no Cmd, Shift or Alt) of a drag tool whose resolved operation is New drags the `Outline` (the
/// app drops the piece before the outline moves); elsewhere the press draws a new selection. Without one, a press
/// outside the selection draws; inside, Cmd without Shift is `Pixels { copy: Alt }` (Cmd+Alt copies), and a drag tool
/// with no Cmd whose resolved operation (`SelectionOperation::resolve(bar, mods, has_selection)`) is New drags the
/// `Outline`. A click-driven tool needs Cmd, and returns `None` for everything while its outline is open.
pub fn drag_kind(probe: &dyn SelectionProbe, at: Point, mods: Mods, bar: SelectionOperation, rule: PressRule) -> Option<DragKind> {
    let clicky = match rule {
        PressRule::Drags => false,
        PressRule::Clicks { outline_open } if outline_open => return None,
        PressRule::Clicks { .. } => true,
    };
    let cut = mods.cmd && !mods.shift;
    let copy = mods.alt && mods.cmd;
    if probe.floating() {
        if !probe.on_floating(at) {
            return None;
        }
        if cut {
            return Some(DragKind::Pixels { copy });
        }
        let plain = !clicky && !mods.cmd && !mods.shift && !mods.alt && SelectionOperation::resolve(bar, mods, true) == SelectionOperation::New;
        return plain.then_some(DragKind::Outline);
    }
    if !probe.inside_selection(at) {
        return None;
    }
    if cut {
        return Some(DragKind::Pixels { copy });
    }
    let plain = !clicky && !mods.cmd && SelectionOperation::resolve(bar, mods, probe.has_selection()) == SelectionOperation::New;
    plain.then_some(DragKind::Outline)
}

/// The cursor over the selection for a press with `mods`, or `None` for the tool's own: `Outline` shows the move
/// cursor, `Pixels` the arrow with the scissors badge (the copy badge for Cmd+Alt), a floating piece the plain arrow
/// (the copy badge for Cmd+Alt). While `dragging`, `Outline` keeps the move cursor and `Pixels` shows the hollow
/// arrowhead.
pub fn cursor_over(
    probe: &dyn SelectionProbe,
    at: Point,
    mods: Mods,
    bar: SelectionOperation,
    rule: PressRule,
    dragging: Option<DragKind>,
) -> Option<Cursor> {
    match dragging {
        Some(DragKind::Outline) => return Some(CursorIcon::Move.into()),
        Some(DragKind::Pixels { .. }) => return Some(PaintedCursor::arrow(true).into()),
        None => {}
    }
    // `drag_kind` answers over a floating piece only on the piece.
    let on_piece = probe.floating();
    Some(match drag_kind(probe, at, mods, bar, rule)? {
        DragKind::Pixels { copy: true } => PaintedCursor::badged(CursorIcon::Default, CursorBadge::Copy).into(),
        _ if on_piece => CursorIcon::Default.into(),
        DragKind::Outline => CursorIcon::Move.into(),
        DragKind::Pixels { copy: false } => PaintedCursor::badged(CursorIcon::Default, CursorBadge::Scissors).into(),
    })
}

/// A selection drag in progress: a [`MoveGesture`] with eight Shift directions (selected pixels and outlines lock to
/// the axes and the diagonals), reported as whole-pixel [`SelectionDrag`] edit events. A tool keeps one from the press
/// [`drag_kind`] answered until the release or Escape.
#[derive(Clone, Debug)]
pub struct InsideDrag {
    kind: DragKind,
    id: GestureId,
    gesture: MoveGesture,
    /// The selection's bounds at the press, which snap as they move; `None` leaves the drag unsnapped.
    bounds: Option<kurbo::Rect>,
    /// The whole-pixel delta of the last event.
    last: [i32; 2],
}

impl InsideDrag {
    /// Starts a `kind` drag named `id` at `at` (document units, unsnapped): the gesture and the `Begin` event, which
    /// names the undo step `label`. `bounds` are the selection's at the press ([`SelectionProbe::bounds`]).
    pub fn begin(
        style: &ConstrainStyle,
        kind: DragKind,
        at: Point,
        mods: Mods,
        id: GestureId,
        label: Arc<str>,
        bounds: Option<kurbo::Rect>,
    ) -> (InsideDrag, SelectEvent) {
        let gesture = MoveGesture::press(style, at, mods, Directions::Eight);
        let event = SelectEvent::DragSelection(EditEvent::begin(id, Mixed::Same(SelectionDrag { kind, delta: [0, 0] }), label, None));
        (InsideDrag { kind, id, gesture, bounds, last: [0, 0] }, event)
    }

    /// The kind of drag.
    pub fn kind(&self) -> DragKind {
        self.kind
    }

    /// What moves and the whole-pixel offset of the last event, for [`SelectPhase::Dragging`](super::SelectPhase).
    pub fn selection_drag(&self) -> SelectionDrag {
        SelectionDrag { kind: self.kind, delta: self.last }
    }

    /// The pointer moved to `at` (document units, unsnapped) at `zoom` screen points per document unit: an `Update`
    /// when the whole-pixel delta changed. The gesture's offset moves the press bounds, which `snap` (the tool's
    /// `ToolContext::snap_rect`) moves onto a snap target; the sum is rounded per axis and clamped to 10^7. Nothing
    /// moves or snaps while the pointer has not left the press ([`MoveGesture::left_press`]): the press is still a
    /// click. The gesture's own Alt copy is ignored (the copy is latched in `kind`).
    pub fn drag(&mut self, at: Point, mods: Mods, zoom: f64, snap: &mut dyn FnMut(kurbo::Rect) -> Vec2) -> Option<SelectEvent> {
        let value = self.gesture.follow(at, mods, zoom);
        if !self.gesture.left_press() {
            return None;
        }
        let value = value?;
        let delta = whole_pixels(snapped(self.bounds, &value, snap));
        if delta == self.last {
            return None;
        }
        self.last = delta;
        Some(SelectEvent::DragSelection(EditEvent::update(self.id, SelectionDrag { kind: self.kind, delta })))
    }

    /// The release at `at` (document units, unsnapped), snapped as [`drag`](Self::drag) is. A press whose pointer
    /// never left it ([`MoveGesture::left_press`], the drag's only click rule) is a click: the `Commit` of a zero
    /// delta. A drag that left and ends at a zero delta (it came back, or snapping pulled it back) changed nothing:
    /// the `Cancel` ([`CancelReason::Dismissed`]). Any other drag is the `Commit` of its delta.
    pub fn end(mut self, at: Point, mods: Mods, zoom: f64, snap: &mut dyn FnMut(kurbo::Rect) -> Vec2) -> SelectEvent {
        // Followed first, so a release that leaves the press with no move before it counts as leaving.
        self.gesture.follow(at, mods, zoom);
        let (left, value) = (self.gesture.left_press(), self.gesture.value());
        let before = SelectionDrag { kind: self.kind, delta: [0, 0] };
        if !left {
            return SelectEvent::DragSelection(EditEvent::commit(self.id, Mixed::Same(before), before));
        }
        let after = SelectionDrag { kind: self.kind, delta: whole_pixels(snapped(self.bounds, &value, snap)) };
        if after.delta == [0, 0] {
            return self.cancel(CancelReason::Dismissed);
        }
        SelectEvent::DragSelection(EditEvent::commit(self.id, Mixed::Same(before), after))
    }

    /// Escape or a lost pointer: the `Cancel`, with the host's `reason`.
    pub fn cancel(self, reason: CancelReason) -> SelectEvent {
        let before = SelectionDrag { kind: self.kind, delta: [0, 0] };
        SelectEvent::DragSelection(EditEvent::cancel(self.id, reason, Some(Mixed::Same(before))))
    }
}

/// The offset that snaps the moved selection's bounds `rect` to the canvas's targets: every kind but the selection's own
/// edges, which move with it.
pub(super) fn moved_selection_snap(cx: &mut craft_ui::canvas::ToolContext<'_>, rect: kurbo::Rect) -> Vec2 {
    cx.snap_rect_of(rect, SnapKinds::ALL.without(SnapKind::Selection))
}

/// The value's offset plus the snap of the press `bounds` moved by it; the offset itself without bounds. A value that
/// changed nothing (a click) is `Vec2::ZERO` and calls no `snap`.
fn snapped(bounds: Option<kurbo::Rect>, value: &MoveValue, snap: &mut dyn FnMut(kurbo::Rect) -> Vec2) -> Vec2 {
    if !value.changed() {
        return Vec2::ZERO;
    }
    match bounds {
        Some(bounds) => value.delta + snap(bounds + value.delta),
        None => value.delta,
    }
}

/// The offset rounded per axis and clamped to 10^7.
fn whole_pixels(delta: Vec2) -> [i32; 2] {
    let axis = |v: f64| v.round().clamp(-1e7, 1e7) as i32;
    [axis(delta.x), axis(delta.y)]
}
