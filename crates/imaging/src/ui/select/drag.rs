//! A press inside the selection: which drag it starts, the cursor over the selection, and the drag itself (the lasso
//! tools use it unchanged). Public for the app's own click tools: the Magic Wand takes the same rule as the Polygonal
//! Lasso ([`PressRule::Clicks`]).

use craft_behavior::geometry::{ConstrainStyle, Directions, MoveGesture, SelectionOperation};
use craft_behavior::{CancelReason, Cursor, CursorBadge, CursorIcon, EditEvent, GestureId, Mixed, PaintedCursor};
use craft_keymap::Mods;
use kurbo::{Point, Vec2};
use std::sync::Arc;

use super::{DragKind, SelectEvent, SelectionDrag, SelectionProbe};

/// How a tool takes a press inside the selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PressRule {
    /// A drag tool (the marquees, the Lasso): a plain press with the operation New drags the outline, Cmd the pixels.
    Drags,
    /// A click-driven tool (the Polygonal Lasso, the app's Magic Wand): only Cmd drags, and never while
    /// `outline_open`.
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
/// With a floating piece: a press on it (`probe.on_floating`) drags it as `Pixels` unless Shift is held, or Alt
/// without Cmd; a click-driven tool needs Cmd; elsewhere the press draws a new selection. Without one, a press outside
/// the selection draws; inside, Cmd without Shift is `Pixels { copy: Alt }` (Cmd+Alt copies), and a drag tool with no
/// Cmd whose resolved operation (`SelectionOperation::resolve(bar, mods, has_selection)`) is New drags the `Outline`.
/// A click-driven tool returns `None` for everything while its outline is open.
pub fn drag_kind(probe: &dyn SelectionProbe, at: Point, mods: Mods, bar: SelectionOperation, rule: PressRule) -> Option<DragKind> {
    let clicky = match rule {
        PressRule::Drags => false,
        PressRule::Clicks { outline_open } if outline_open => return None,
        PressRule::Clicks { .. } => true,
    };
    let cut = mods.cmd && !mods.shift;
    let copy = mods.alt && mods.cmd;
    if probe.floating() {
        let on = probe.on_floating(at) && !mods.shift && (!mods.alt || mods.cmd) && (cut || !clicky);
        return on.then_some(DragKind::Pixels { copy });
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
/// cursor, `Pixels` the arrow (with the copy badge for Cmd+Alt), a floating piece the plain arrow. While `dragging`,
/// `Outline` keeps the move cursor and `Pixels` shows the plain arrow.
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
        Some(DragKind::Pixels { .. }) => return Some(CursorIcon::Default.into()),
        None => {}
    }
    Some(match drag_kind(probe, at, mods, bar, rule)? {
        DragKind::Outline => CursorIcon::Move.into(),
        DragKind::Pixels { copy: true } => PaintedCursor::badged(CursorIcon::Default, CursorBadge::Copy).into(),
        DragKind::Pixels { copy: false } => CursorIcon::Default.into(),
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
    /// The whole-pixel delta of the last event.
    last: [i32; 2],
}

impl InsideDrag {
    /// Starts a `kind` drag named `id` at `at` (document units): the gesture and the `Begin` event, which names the
    /// undo step `label`.
    pub fn begin(style: &ConstrainStyle, kind: DragKind, at: Point, mods: Mods, id: GestureId, label: Arc<str>) -> (InsideDrag, SelectEvent) {
        let gesture = MoveGesture::press(style, at, mods, Directions::Eight);
        let event = SelectEvent::DragSelection(EditEvent::begin(id, Mixed::Same(SelectionDrag { kind, delta: [0, 0] }), label, None));
        (InsideDrag { kind, id, gesture, last: [0, 0] }, event)
    }

    /// The kind of drag.
    pub fn kind(&self) -> DragKind {
        self.kind
    }

    /// The pointer moved to `at` at `zoom` screen points per document unit: an `Update` when the whole-pixel delta
    /// changed. The delta is the gesture's offset rounded per axis and clamped to 10^7; the gesture's own Alt copy is
    /// ignored (the copy is latched in `kind`).
    pub fn drag(&mut self, at: Point, mods: Mods, zoom: f64) -> Option<SelectEvent> {
        let delta = whole_pixels(self.gesture.follow(at, mods, zoom)?.delta);
        if delta == self.last {
            return None;
        }
        self.last = delta;
        Some(SelectEvent::DragSelection(EditEvent::update(self.id, SelectionDrag { kind: self.kind, delta })))
    }

    /// The release: the `Commit`. A delta of 0 means nothing moved.
    pub fn end(self, at: Point, mods: Mods, zoom: f64) -> SelectEvent {
        let after = SelectionDrag { kind: self.kind, delta: whole_pixels(self.gesture.release(at, mods, zoom).delta) };
        let before = SelectionDrag { kind: self.kind, delta: [0, 0] };
        SelectEvent::DragSelection(EditEvent::commit(self.id, Mixed::Same(before), after))
    }

    /// Escape or a lost pointer: the `Cancel`, with the host's `reason`.
    pub fn cancel(self, reason: CancelReason) -> SelectEvent {
        let before = SelectionDrag { kind: self.kind, delta: [0, 0] };
        SelectEvent::DragSelection(EditEvent::cancel(self.id, reason, Some(Mixed::Same(before))))
    }
}

/// The offset rounded per axis and clamped to 10^7.
fn whole_pixels(delta: Vec2) -> [i32; 2] {
    let axis = |v: f64| v.round().clamp(-1e7, 1e7) as i32;
    [axis(delta.x), axis(delta.y)]
}
