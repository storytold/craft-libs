//! The pen's Auto Add/Delete and close cue: what a press on the open draft does instead of adding an anchor, and
//! whether it would close the path.

use super::bezier::{BezierHit, BezierOverlay, BezierStyle, EditPath, EditSubpath, HandlesShown, HitMark, PathSelection, paint_mark};
use super::{PenMode, PenTool};
use craft_behavior::CanvasView;
use craft_keymap::Mods;
use craft_ui::primitives::Draw;
use craft_ui::style::StyleCx;
use egui::Pos2;

/// What Auto Add/Delete does to the draft for a press.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum AutoEdit {
    /// Insert an anchor into the segment leaving knot `segment` at `t` ([`EditSubpath::insert_at`]).
    Insert { segment: usize, t: f64 },
    /// Remove knot `knot` ([`EditSubpath::remove_knot`]).
    Delete { knot: usize },
}

/// The draft as the one path of a hit test.
fn as_path(draft: &EditSubpath) -> EditPath {
    EditPath { key: 0, subpaths: vec![draft.clone()], color: None }
}

impl PenTool {
    /// Auto Add/Delete for a press at the screen point `at` with `mods` on `draft`: `None` unless
    /// [`auto_add_delete`](PenTool::auto_add_delete) is on, the mode is the Pen and no ⇧, ⌥ or ⌘ is held. Then, by
    /// [`BezierOverlay::hit_test`] on the draft within `reach` (no handles shown): a segment gives
    /// [`AutoEdit::Insert`] at the hit's `t`; an anchor that is not the first or the last gives
    /// [`AutoEdit::Delete`]; the first and the last are left to the Pen (the first closes, the last retracts). The
    /// press applies the edit as one `Update` of the draft and takes no other action. The hover shows the same
    /// answer's mark and no rubber band.
    pub(super) fn auto_edit(&self, draft: &EditSubpath, at: Pos2, mods: Mods, view: &CanvasView, reach: f32) -> Option<AutoEdit> {
        if !self.auto_add_delete || self.mode != PenMode::Pen || mods.shift || mods.alt || mods.cmd || draft.knots.is_empty() {
            return None;
        }
        let found = BezierOverlay::hit_test(&[as_path(draft)], &PathSelection::default(), HandlesShown::Selected, view, at, reach)?;
        match found {
            BezierHit::Segment { segment, t, .. } => Some(AutoEdit::Insert { segment: segment.knot, t }),
            BezierHit::Anchor { knot, .. } => {
                let end = !draft.closed && (knot.knot == 0 || knot.knot + 1 == draft.knots.len());
                (!end).then_some(AutoEdit::Delete { knot: knot.knot })
            }
            BezierHit::Handle { .. } => None,
        }
    }

    /// Whether a press at the screen point `pointer` would close `draft`: the draft open, within `reach` of the first
    /// anchor, with 2 or more anchors in Pen mode and 3 or more in Curvature mode.
    pub(super) fn closes_at(&self, draft: &EditSubpath, pointer: Pos2, view: &CanvasView, reach: f32) -> bool {
        let needed = match self.mode {
            PenMode::Pen => 2,
            PenMode::Curvature => 3,
        };
        draft.knots.len() >= needed && !draft.closed && view.to_screen(draft.knots[0].anchor).distance(pointer) <= reach
    }

    /// The mark of what a press would do, over the draft.
    pub(super) fn paint_auto_mark(&self, draw: &Draw<'_>, style: &StyleCx<'_>, view: &CanvasView, edit: AutoEdit) {
        let found = match edit {
            AutoEdit::Insert { segment, t } => BezierHit::Segment { path: 0, segment: super::KnotRef::new(0, segment), t },
            AutoEdit::Delete { knot } => BezierHit::Anchor { path: 0, knot: super::KnotRef::new(0, knot) },
        };
        let path = as_path(&self.shown);
        paint_mark(draw, view, HitMark::new(&path, found), &BezierStyle::of(style));
    }
}
