//! [`AnchorTool`]: the canvas tool of the anchor tools and Direct Selection, whose presses are the
//! [`BezierOverlay`](super::BezierOverlay)'s.

use super::tool::ICON_BOX;
use super::{PathTool, PenTool};
use craft_behavior::{CancelReason, Cursor, CursorBadge, CursorIcon, PaintedCursor, PointerButton};
use craft_keymap::Mods;
use craft_ui::canvas::{CanvasTool, ToolAccess, ToolAction, ToolClaims, ToolContext, ToolKey, ToolPath, ToolPointer};
use craft_ui::primitives::{Draw, ToolIcon};
use egui::Vec2;

/// The apex of the Convert Point icon's caret, in the units of the icon's box.
const CONVERT_TIP: Vec2 = Vec2::new(ICON_BOX / 2.0, 5.5);

/// The canvas tool of Add Anchor Point, Delete Anchor Point, Convert Point and Direct Selection. Every press of these
/// tools is the path overlay's ([`PathMode::resolve`](super::PathMode::resolve) gives its tool); a press off the
/// overlay does nothing, so it never starts a path.
///
/// - The open pen draft ([`draft`](Self::draft)), when the host passes it, is drawn and keeps its keys (Enter,
///   Escape, and Delete unless the draft is [`PenDraft::Foreign`](super::PenDraft::Foreign)) as [`PathMode`](super::PathMode)
///   describes; it takes no press.
/// - Cursor off the overlay's parts: the crosshair under the canvas's precise cursor
///   ([`CanvasSpace::precise`](craft_ui::canvas::CanvasSpace::precise)); with ⌘ Direct Selection's arrow; else Add
///   Anchor the arrow (a click there adds nothing; with ⌥ the arrow with "+"), Delete Anchor the pen
///   with "-" (with ⌥, Add Anchor, the pen with "+"), Convert Point its icon, Direct Selection the arrow. Over a part
///   the overlay sets its own cursor.
#[derive(Debug)]
pub struct AnchorTool<'a> {
    tool: PathTool,
    draft: Option<&'a mut PenTool>,
}

impl<'a> AnchorTool<'a> {
    /// The canvas tool of `tool`, with no pen draft.
    pub fn new(tool: PathTool) -> Self {
        Self { tool, draft: None }
    }

    /// The open pen draft, drawn and keyed while the tool runs: `pen` stops sensing.
    #[must_use]
    pub fn draft(mut self, pen: &'a mut PenTool) -> Self {
        pen.sensing(false);
        self.draft = Some(pen);
        self
    }
}

/// The cursor of `tool` off the overlay's parts with `mods` held; `precise` is the canvas's precise cursor.
pub(super) fn cursor_of(tool: PathTool, mods: Mods, precise: bool) -> PaintedCursor {
    // The pen's tip is the bottom left corner of the icon's box.
    let pen = |badge| PaintedCursor::tool(ToolIcon::Pen.icon(), Vec2::Y * ICON_BOX, Some(badge));
    match tool {
        _ if precise => PaintedCursor::Crosshair,
        _ if mods.cmd => PaintedCursor::arrow(false),
        PathTool::AddAnchor if mods.alt => PaintedCursor::badged(CursorIcon::Default, CursorBadge::Add),
        PathTool::DeleteAnchor if mods.alt => pen(CursorBadge::Add),
        PathTool::DeleteAnchor => pen(CursorBadge::Subtract),
        PathTool::ConvertPoint => PaintedCursor::tool(ToolIcon::ConvertPoint.icon(), CONVERT_TIP, None),
        _ => PaintedCursor::arrow(false),
    }
}

impl CanvasTool for AnchorTool<'_> {
    fn each_pass(&mut self, cx: &mut ToolContext<'_>) {
        if let Some(pen) = self.draft.as_deref_mut() {
            pen.each_pass(cx);
        }
    }

    fn press(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) -> ToolAction {
        ToolAction::Ignored
    }

    fn drag(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {}

    fn release(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {}

    /// The tool never holds a gesture, so a cancel has nothing to end.
    fn cancel(&mut self, _: CancelReason, _: &mut ToolContext<'_>) {}

    fn key(&mut self, key: &ToolKey, cx: &mut ToolContext<'_>) -> bool {
        self.draft.as_deref_mut().is_some_and(|pen| pen.key(key, cx))
    }

    fn claims(&self) -> ToolClaims {
        self.draft.as_deref().map_or(ToolClaims::NONE, |pen| pen.claims())
    }

    fn takes(&self, _: PointerButton, _: Mods) -> bool {
        false
    }

    fn paint(&self, draw: &Draw<'_>, cx: &ToolContext<'_>) {
        if let Some(pen) = self.draft.as_deref() {
            pen.paint(draw, cx);
        }
    }

    fn cursor(&self, cx: &ToolContext<'_>) -> Cursor {
        Cursor::Painted(cursor_of(self.tool, cx.mods, cx.precise()))
    }

    fn access(&self, cx: &ToolContext<'_>) -> Option<ToolAccess> {
        self.draft.as_deref()?.access(cx)
    }

    fn command_form(&self) -> ToolPath {
        ToolPath::Command("the Direct Selection tool's anchor keys")
    }
}
