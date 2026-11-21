//! The Add Anchor Point, Delete Anchor Point, Auto Add/Delete and Curvature tools of [`BezierOverlay`](super::BezierOverlay):
//! which parts each tool takes a press on, what a click reports and what the hover mark shows.

use craft_behavior::{Cursor, CursorBadge, CursorIcon, PaintedCursor};
use craft_keymap::Mods;
use craft_ui::primitives::ToolIcon;
use egui::Vec2;

use super::{AnchorDelete, BezierHit, BezierTool, PathSelection, Report};
use crate::ui::pen::tool::ICON_BOX;

impl BezierTool {
    /// The tool a press acts as: [`BezierTool::DeleteAnchor`] acts as [`BezierTool::AddAnchor`] while ⌥ is held;
    /// every other tool is itself. Add Anchor keeps itself under ⌥, where a segment press takes the whole subpath
    /// ([`BezierTool::takes_subpath`]). (⌘ making the pointer Direct Selection is the host's, through
    /// [`PathMode`](crate::ui::pen::PathMode).)
    pub(super) fn acting(self, mods: Mods) -> BezierTool {
        // pc: crates/ui-egui/src/tools/anchor_tools.rs:27-35 `effective`; Add Anchor with ⌥ (PhotoCraft #12709).
        match (self, mods.alt) {
            (BezierTool::DeleteAnchor, true) => BezierTool::AddAnchor,
            (tool, _) => tool,
        }
    }

    /// Whether the tool takes a press on `hit`; `interior` is whether an anchor is neither end of an open subpath and
    /// `straight` whether a segment has both handles retracted.
    ///
    /// - Direct: every part. Convert: anchors and handles, and a straight segment (a drag bends it, or moves a fully
    ///   selected path); never a curved one.
    /// - Add Anchor: every part: a segment adds an anchor, an anchor or a shown handle is selected and dragged as
    ///   under Direct. Delete Anchor: an anchor, or a segment (a drag bends it). Delete Anchor never takes a handle.
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
            BezierTool::AddAnchor => true,
            BezierTool::DeleteAnchor | BezierTool::Curvature => !matches!(hit, BezierHit::Handle { .. }),
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

    /// Whether the tool has the arrow and ⌘A keys: Direct, Convert and Curvature. Add and Delete have Delete
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

    /// Whether a press on a segment adds an anchor: Add Anchor without ⌥, Delete Anchor with ⌥, or Auto Add/Delete.
    pub(super) fn adds(self, mods: Mods) -> bool {
        matches!(self.acting(mods), BezierTool::AddAnchor | BezierTool::AutoAddDelete) && !self.takes_subpath(mods)
    }

    /// Whether a press on a segment takes its whole subpath instead, as Direct's ⌥ does: Add Anchor with ⌥, where a
    /// click selects the subpath and a drag moves a copy of it, as in the reference application.
    pub(super) fn takes_subpath(self, mods: Mods) -> bool {
        self == BezierTool::AddAnchor && mods.alt
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
///
/// Delete Anchor on an anchor whose subpath `selection` does not show deletes nothing: it selects the anchor and
/// shows only its subpath (Photoshop, #12769). The host selects the anchor after a delete with
/// [`PathSelection::after_delete`].
pub(super) fn anchor_click(tool: BezierTool, hit: BezierHit, mods: Mods, selection: &PathSelection, report: &mut Report) {
    // pc: crates/ui-egui/src/tools/anchor_tools.rs:88-114 `pointer`, `click`; :130-150 `apply`; `click` (#12769).
    let BezierHit::Anchor { path, knot } = hit else { return };
    match tool.acting(mods) {
        BezierTool::DeleteAnchor if !selection.shows(path, knot.subpath) => {
            report.select = Some(PathSelection::new(Some(path), vec![knot], None).shown(Some(vec![knot.subpath])));
        }
        BezierTool::DeleteAnchor | BezierTool::AutoAddDelete => report.delete = Some(AnchorDelete::new(path, vec![knot], false)),
        _ => {}
    }
}

/// The part the hover mark shows for `hit` with `mods` held: what a press would add or delete. Under Add an anchor or
/// a handle (which a press selects and drags as Direct does) and under Delete a segment get no mark (PhotoCraft draws
/// none there), nor does a hidden anchor under Delete, which a click only shows. Curvature marks every part it takes.
pub(super) fn hover_mark(tool: BezierTool, hit: BezierHit, mods: Mods, selection: &PathSelection) -> Option<BezierHit> {
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
            anchor_click(tool, hit, mods, selection, &mut would);
            would.delete.is_some()
        }
    };
    marked.then_some(hit)
}

/// The cursor over `hit`, a part the tool takes, with `mods` held; `precise` is the canvas's precise cursor
/// ([`CanvasSpace::precise`](craft_ui::canvas::CanvasSpace::precise)). Add Anchor shows the pen with "+" over a
/// segment a click splits and the arrow over an anchor, the crosshair when precise; with ⌥ the arrow with "+" over
/// every part. Delete Anchor with ⌥ shows Add Anchor's cursors. Under ⌘ the cursor stays the host tool's, as it does
/// off the parts.
pub(super) fn hover_cursor(tool: BezierTool, hit: BezierHit, mods: Mods, precise: bool) -> Option<Cursor> {
    // pc: crates/ui-egui/src/canvas/tool_cursor.rs, crates/ui-egui/src/tools/anchor_tools.rs (#12729, #12709).
    if tool.acting(mods) != BezierTool::AddAnchor || mods.cmd {
        return None;
    }
    let cursor = match hit {
        _ if precise => PaintedCursor::Crosshair,
        _ if tool.takes_subpath(mods) => PaintedCursor::badged(CursorIcon::Default, CursorBadge::Add),
        // The pen's tip is the bottom left corner of the icon's box.
        BezierHit::Segment { .. } => PaintedCursor::tool(ToolIcon::Pen.icon(), Vec2::Y * ICON_BOX, Some(CursorBadge::Add)),
        BezierHit::Anchor { .. } | BezierHit::Handle { .. } => PaintedCursor::arrow(false),
    };
    Some(Cursor::Painted(cursor))
}

/// `t` clamped to the range the Curvature tool inserts at.
pub(super) fn curvature_t(t: f64) -> f64 {
    use crate::geometry::curvature::CURVATURE_T_RANGE;
    t.clamp(*CURVATURE_T_RANGE.start(), *CURVATURE_T_RANGE.end())
}
