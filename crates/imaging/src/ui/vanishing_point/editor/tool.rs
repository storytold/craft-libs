//! [`VanishingPointTool`]: the presses of the plane editor that fall on no node.

use craft_behavior::{CancelReason, Cursor, CursorIcon, PaintedCursor};
use craft_ui::canvas::{CanvasTool, ToolAction, ToolClaims, ToolContext, ToolKey, ToolPath, ToolPointer};
use craft_ui::primitives::Draw;

use super::work::{Pressed, Work};
use super::{Shared, VanishingTool};
use crate::ui::vanishing_point::words::VanishingPointWords;

/// The editor as the `CanvasTool` of one pass; the planes' nodes are [`VanishingPlanes`](super::VanishingPlanes)'
/// parts, which take their own presses.
///
/// - **Create Plane**: four clicks in any order make a plane (`VanishingScene::normalized`), selected; then Edit Plane
///   takes over (as Photoshop). A rubber band runs from the last corner to the pointer. Nothing past
///   [`MAX_PLANES`](super::super::MAX_PLANES).
/// - **Edit Plane**: a click selects the plane under it (`VanishingScene::plane_at`) or none; a press on the pasted
///   image moves it on the plane.
/// - **Stamp**: Alt-click sets the source; a press, drag, release paints a `StampStroke` with the settings (Aligned
///   keeps the first stroke's offset). A press the stroke limits or a missing source refuse reports its message
///   through [`VanishingPointEditor::take_status`](super::VanishingPointEditor::take_status).
/// - **Keys**: Delete and Backspace remove the selected plane; Esc drops the pending corners.
/// - **Events**: `VanishingPointEvent::Planes` Begin, Update, Commit for a drag and `Set` for a click that made or
///   deleted a plane; `Paste` for the image's drag; `Stamp` for a stroke (each Update carries the stroke so far). Esc
///   during a drag reports Cancel and puts back what the press found.
pub struct VanishingPointTool<'a> {
    pass: &'a dyn Shared,
}

impl<'a> VanishingPointTool<'a> {
    pub(super) fn new(pass: &'a dyn Shared) -> Self {
        Self { pass }
    }

    fn run<R>(&self, f: impl FnOnce(&mut Work<'_>) -> R) -> R {
        let mut f = Some(f);
        let mut out = None;
        self.pass.with(&mut |work| {
            if let Some(f) = f.take() {
                out = Some(f(work));
            }
        });
        out.expect("the pass runs the closure once")
    }

    /// The document point under a screen point, `None` for a canvas with no view.
    fn doc(cx: &ToolContext<'_>, pointer: &ToolPointer) -> Option<[f64; 2]> {
        let at = cx.space()?.view.to_doc(pointer.pos);
        Some([at.x, at.y])
    }
}

impl CanvasTool for VanishingPointTool<'_> {
    fn press(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) -> ToolAction {
        // pc: crates/ui-egui/src/dialogs/vp_ui.rs:658 `press`.
        let (Some(p), words) = (Self::doc(cx, pointer), cx.words::<VanishingPointWords>()) else { return ToolAction::Ignored };
        let gesture = cx.gesture_id();
        let pressed = self.run(|work| {
            work.gesture = gesture;
            work.background_down(p, pointer.mods, &words)
        });
        match pressed {
            Some(Pressed::Claimed) => ToolAction::Claimed,
            Some(Pressed::Done) => ToolAction::Done,
            None => ToolAction::Ignored,
        }
    }

    fn drag(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        // pc: crates/ui-egui/src/dialogs/vp_ui.rs:718 `drag_to`, :501 `stamp_to`.
        if let (Some(p), Some(space)) = (Self::doc(cx, pointer), cx.space()) {
            let dead = space.view.doc_len(cx.slop);
            self.run(|work| work.drag_to(p, dead));
        }
    }

    fn release(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        // pc: crates/ui-egui/src/dialogs/vp_ui.rs:1025 `pointer`.
        if let (Some(p), Some(space)) = (Self::doc(cx, pointer), cx.space()) {
            let dead = space.view.doc_len(cx.slop);
            self.run(|work| work.release_at(p, dead));
        }
    }

    fn hover(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        // new: the rubber band and the stamp ring follow the pointer.
        let p = Self::doc(cx, pointer).filter(|p| p.iter().all(|v| v.is_finite()));
        self.run(|work| work.editor.pointer = p);
    }

    fn cancel(&mut self, reason: CancelReason, _: &mut ToolContext<'_>) {
        self.run(|work| work.editor.cancel(work.session, reason));
    }

    fn key(&mut self, key: &ToolKey, cx: &mut ToolContext<'_>) -> bool {
        // pc: crates/ui-egui/src/dialogs/vp_ui.rs:1045 `keys` (Delete, Backspace), :1068 `escape`.
        let words = cx.words::<VanishingPointWords>();
        match key {
            ToolKey::Delete { .. } => {
                self.run(|work| work.editor.delete_selected(work.session, work.scene, words.edit_planes_step.as_str()).is_some())
            }
            ToolKey::Cancel => self.run(|work| work.editor.drop_pending()),
            _ => false,
        }
    }

    fn claims(&self) -> ToolClaims {
        if self.run(|work| work.editor.selected().is_some()) { ToolClaims::NONE.deletes() } else { ToolClaims::NONE }
    }

    fn paint(&self, _: &Draw<'_>, _: &ToolContext<'_>) {
        // The planes, the rubber band, the stamp ring and the source cross are the control's.
    }

    fn cursor(&self, cx: &ToolContext<'_>) -> Cursor {
        // new: the crosshair for Create Plane, the stamp ring's own cursor on a plane (PhotoCraft draws none of its own).
        let crosshair = || Cursor::Painted(PaintedCursor::Crosshair);
        let pointer = cx.pointer.as_ref();
        let over = pointer.and_then(|pointer| Self::doc(cx, pointer));
        self.run(|work| match work.editor.tool {
            VanishingTool::Stamp if over.is_some_and(|p| work.on_plane(p)) => Cursor::System(CursorIcon::None),
            VanishingTool::EditPlane => Cursor::System(CursorIcon::Default),
            _ => crosshair(),
        })
    }

    fn command_form(&self) -> ToolPath {
        self.run(|work| match work.editor.tool {
            VanishingTool::Stamp => ToolPath::Freehand("none; a stamp stroke follows the pointer"),
            _ => ToolPath::Command("the New Plane… command"),
        })
    }
}
