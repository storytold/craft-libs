//! Which path overlay a pen or anchor tool runs, and with what: [`PathMode::resolve`].

use super::BezierTool;
use craft_keymap::Mods;

/// What the pen draft does with presses.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PenMode {
    /// The Pen: clicks add corner anchors, drags pull out handles.
    #[default]
    Pen,
    /// The Curvature Pen: clicks add smooth anchors the curve flows through; handles follow
    /// [`curvature_handles`](crate::geometry::curvature::curvature_handles).
    Curvature,
}

/// The host's path tool, the input of [`PathMode::resolve`]. The Path Selection and Freeform Pen tools are the
/// app's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PathTool {
    /// The Pen.
    Pen,
    /// The Curvature Pen.
    Curvature,
    /// The Add Anchor Point tool.
    AddAnchor,
    /// The Delete Anchor Point tool.
    DeleteAnchor,
    /// The Convert Point tool.
    ConvertPoint,
    /// The Direct Selection tool.
    DirectSelection,
}

/// What a pass of the pen and path tools shows: the [`BezierOverlay`](super::BezierOverlay)'s tool, and the pen
/// draft's mode when it shows (`None`: no draft this pass).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct PathMode {
    /// The path overlay's tool.
    pub bezier: BezierTool,
    /// The pen draft's mode, or `None` when the draft is not shown.
    pub draft: Option<PenMode>,
    /// Whether the draft takes presses ([`PenTool::sensing`](super::PenTool::sensing)): false while ⌘ makes the
    /// pointer Direct Selection and under the anchor tools, so the draft is drawn and keeps its keys while the
    /// overlay edits it.
    pub draft_senses: bool,
}

impl PathMode {
    /// The table of which overlay a tool and the held keys select, for `tool` with `mods` held, `drawing` true while
    /// a pen draft is open, and the Pen's `auto_add_delete` option:
    ///
    /// | Tool | Keys | `bezier` | `draft` | `draft_senses` |
    /// |---|---|---|---|---|
    /// | any pen or anchor tool, Convert Point | ⌘ | `Direct` | the tool's draft mode if `drawing`, else `None` | false |
    /// | Pen | none, drawing | `Show` | `Pen` | true |
    /// | Pen | none, not drawing | `AutoAddDelete` if `auto_add_delete`, else `Show` | `Pen` | true |
    /// | Pen | ⌥, not drawing | `Convert` | `Pen` | true |
    /// | Pen | ⇧ or ⌥, drawing; ⇧, not drawing | `Show` | `Pen` | true |
    /// | Curvature | none or ⇧ or ⌥, not drawing | `Curvature` | `Curvature` | true |
    /// | Curvature | drawing | `Show` | `Curvature` | true |
    /// | Add Anchor, Delete Anchor | no ⌘ | `AddAnchor`, `DeleteAnchor` (the overlay swaps them with ⌥) | `Pen` if `drawing` (shown, not sensing), else `None` | false |
    /// | Convert Point | no ⌘ | `Convert` | as Add Anchor | false |
    /// | Direct Selection | any | `Direct` | as Add Anchor | false |
    ///
    /// The draft's own Auto Add/Delete (while drawing) is [`PenTool::auto_add_delete`](super::PenTool::auto_add_delete),
    /// which the host passes the same option to; it is off while ⇧, ⌥ or ⌘ is held.
    ///
    /// Where the draft shows without sensing (⌘, Add Anchor, Delete Anchor, Convert Point, Direct Selection), the
    /// anchor tools edit it as they edit a path: the host passes the open draft to the
    /// [`BezierOverlay`](super::BezierOverlay) as one more [`EditPath`](super::EditPath), last in the slice under a
    /// key of its own, and applies the overlay's `insert`, `delete` and `edit` for that key to its draft
    /// ([`EditSubpath::insert_at`](super::EditSubpath::insert_at),
    /// [`remove_knot`](super::EditSubpath::remove_knot), [`PathEdit::apply`](super::PathEdit::apply)).
    pub fn resolve(tool: PathTool, mods: Mods, drawing: bool, auto_add_delete: bool) -> PathMode {
        let draft_mode = match tool {
            PathTool::Curvature => PenMode::Curvature,
            _ => PenMode::Pen,
        };
        // The draft shows, without sensing, only while a path is being drawn.
        let shown_only = drawing.then_some(draft_mode);
        let overlay = |bezier| PathMode { bezier, draft: shown_only, draft_senses: false };
        if mods.cmd {
            return overlay(BezierTool::Direct);
        }
        let takes = |bezier, mode| PathMode { bezier, draft: Some(mode), draft_senses: true };
        match tool {
            PathTool::Pen if drawing => takes(BezierTool::Anchors, PenMode::Pen),
            PathTool::Pen if mods.alt => takes(BezierTool::Convert, PenMode::Pen),
            PathTool::Pen if auto_add_delete && !mods.shift => takes(BezierTool::AutoAddDelete, PenMode::Pen),
            PathTool::Pen => takes(BezierTool::Anchors, PenMode::Pen),
            PathTool::Curvature if drawing => takes(BezierTool::Anchors, PenMode::Curvature),
            PathTool::Curvature => takes(BezierTool::Curvature, PenMode::Curvature),
            PathTool::AddAnchor => overlay(BezierTool::AddAnchor),
            PathTool::DeleteAnchor => overlay(BezierTool::DeleteAnchor),
            PathTool::ConvertPoint => overlay(BezierTool::Convert),
            PathTool::DirectSelection => overlay(BezierTool::Direct),
        }
    }
}
