//! The Add Anchor Point, Delete Anchor Point, Auto Add/Delete and Curvature tools of [`BezierOverlay`](super::BezierOverlay):
//! which parts each tool takes a press on, what a click reports and what the hover mark shows.

use craft_keymap::Mods;

use super::{AnchorDelete, BezierHit, BezierTool, Report};

impl BezierTool {
    /// The tool a press acts as: [`BezierTool::AddAnchor`] and [`BezierTool::DeleteAnchor`] swap while ⌥ is held;
    /// every other tool is itself. (⌘ making the pointer Direct Selection is the host's, through
    /// [`PathMode`](crate::ui::pen::PathMode).)
    pub(super) fn acting(self, mods: Mods) -> BezierTool {
        // pc: crates/ui-egui/src/tools/anchor_tools.rs:27-35 `effective`.
        match (self, mods.alt) {
            (BezierTool::AddAnchor, true) => BezierTool::DeleteAnchor,
            (BezierTool::DeleteAnchor, true) => BezierTool::AddAnchor,
            (tool, _) => tool,
        }
    }

    /// Whether the tool takes a press on `hit`; `interior` is whether an anchor is neither end of an open subpath and
    /// `straight` whether a segment has both handles retracted.
    ///
    /// - Direct: every part. Convert: anchors and handles, and a straight segment (a drag bends it, or moves a fully
    ///   selected path); never a curved one.
    /// - Add Anchor: a segment, or an anchor (the press is taken so it starts no path; it reports nothing). Delete
    ///   Anchor: an anchor, or a segment (taken, nothing reported). Handles are never taken.
    /// - Auto Add/Delete: a segment, or an anchor that is not the first or last of an open subpath (those are the
    ///   pen draft's: the first closes, the last retracts). Nothing else, so the pen draft starts a path on empty
    ///   canvas.
    /// - Curvature: an anchor or a segment, never a handle or empty canvas.
    /// - Anchors: none.
    pub(super) fn takes(self, hit: &BezierHit, interior: bool, straight: bool) -> bool {
        // pc: crates/ui-egui/src/tools/anchor_tools.rs:37-48 `target`, :57-84 `pen_auto` (ends excluded :74-78);
        // crates/ui-egui/src/tools/direct_select.rs:368-376 `curvature_target`.
        match self {
            BezierTool::Direct => true,
            BezierTool::Convert => straight || !matches!(hit, BezierHit::Segment { .. }),
            BezierTool::Anchors => false,
            // The swap with ⌥ changes what a press reports, not what it takes.
            BezierTool::AddAnchor | BezierTool::DeleteAnchor | BezierTool::Curvature => !matches!(hit, BezierHit::Handle { .. }),
            BezierTool::AutoAddDelete => match hit {
                BezierHit::Handle { .. } => false,
                BezierHit::Segment { .. } => true,
                BezierHit::Anchor { .. } => interior,
            },
        }
    }

    /// Whether a press is one click that reports on the press: no gesture, no drag, no selection change.
    pub(super) fn clicks(self) -> bool {
        matches!(self, BezierTool::AddAnchor | BezierTool::DeleteAnchor | BezierTool::AutoAddDelete)
    }

    /// Whether the tool has the arrow, ⌘A and Esc keys: Direct, Convert and Curvature. Add and Delete have Delete
    /// only, Auto Add/Delete none.
    pub(super) fn keyed(self) -> bool {
        matches!(self, BezierTool::Direct | BezierTool::Convert | BezierTool::Curvature)
    }

    /// Whether Delete and Backspace report [`AnchorDelete`](super::AnchorDelete) for the selection. Under Anchors and
    /// Auto Add/Delete the key goes on to the pen draft.
    pub(super) fn deletes(self) -> bool {
        matches!(self, BezierTool::Direct | BezierTool::Convert | BezierTool::Curvature | BezierTool::AddAnchor | BezierTool::DeleteAnchor)
    }

    /// Whether a click with ⌥ on an anchor picks its whole subpath: Delete Anchor with ⌥, where the swapped Add
    /// Anchor has nothing to do on an anchor.
    pub(super) fn picks_subpath(self, mods: Mods) -> bool {
        self == BezierTool::DeleteAnchor && mods.alt
    }

    /// Whether a press on a segment adds an anchor: Add Anchor, or Auto Add/Delete, unless ⌥ swaps Add for Delete.
    pub(super) fn adds(self, mods: Mods) -> bool {
        matches!(self.acting(mods), BezierTool::AddAnchor | BezierTool::AutoAddDelete)
    }

    /// Whether the tool shows the hover mark of what a press would take.
    pub(super) fn marks(self) -> bool {
        matches!(self, BezierTool::AddAnchor | BezierTool::DeleteAnchor | BezierTool::AutoAddDelete | BezierTool::Curvature)
    }
}

/// A click of Delete Anchor or Auto Add/Delete on an anchor, reported on press as PhotoCraft does: `delete` of that one
/// anchor, one undo step. A segment press is the overlay's own ([`BezierTool::adds`]: an anchor that drags into
/// place). The pen draft, when the host passes it as one more path
/// ([`PathMode::resolve`](crate::ui::pen::PathMode::resolve)), is hit and reported like any path, under the key the
/// host gave it.
pub(super) fn anchor_click(tool: BezierTool, hit: BezierHit, mods: Mods, report: &mut Report) {
    // pc: crates/ui-egui/src/tools/anchor_tools.rs:88-114 `pointer`, `click`; :130-150 `apply`.
    if let (BezierHit::Anchor { path, knot }, BezierTool::DeleteAnchor | BezierTool::AutoAddDelete) = (hit, tool.acting(mods)) {
        report.delete = Some(AnchorDelete::new(path, vec![knot], false));
    }
}

/// The part the hover mark shows for `hit` with `mods` held: what a press would edit. Under Add an anchor and under
/// Delete a segment are taken and do nothing (PhotoCraft draws no mark there). Curvature marks every part it takes.
pub(super) fn hover_mark(tool: BezierTool, hit: BezierHit, mods: Mods) -> Option<BezierHit> {
    // pc: crates/ui-egui/src/tools/anchor_tools.rs:152-169 `draw_hover`, `draw_pen_hover`;
    // crates/ui-egui/src/tools/direct_select.rs:378-389 `curvature_hover`.
    if !tool.marks() || mods.cmd {
        return None;
    }
    if tool == BezierTool::Curvature {
        return Some(match hit {
            BezierHit::Segment { path, segment, t } => BezierHit::Segment { path, segment, t: curvature_t(t) },
            other => other,
        });
    }
    let marked = match hit {
        BezierHit::Segment { .. } => tool.adds(mods),
        _ => {
            let mut would = Report::default();
            anchor_click(tool, hit, mods, &mut would);
            would.delete.is_some()
        }
    };
    marked.then_some(hit)
}

/// `t` clamped to the range the Curvature tool inserts at.
pub(super) fn curvature_t(t: f64) -> f64 {
    use crate::geometry::curvature::CURVATURE_T_RANGE;
    t.clamp(*CURVATURE_T_RANGE.start(), *CURVATURE_T_RANGE.end())
}
