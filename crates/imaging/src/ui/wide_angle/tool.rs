//! `ConstraintTool`: Adaptive Wide Angle's Constraint tool.

use craft_behavior::kurbo::Point;
use craft_behavior::{CancelReason, Cursor, PaintedCursor};
use craft_ui::canvas::{CanvasTool, ToolAction, ToolContext, ToolPath, ToolPointer};
use craft_ui::primitives::Draw;

use super::session::{WideAngleConstraint, WideAngleConstraintEdit};

/// A drag shorter than this many screen points adds no constraint (PhotoCraft's dead zone).
const DRAG_MINIMUM: f32 = 6.0;

/// The Constraint tool: a drag on the picture draws a constraint from the press to the pointer, added on release
/// when longer than the drag minimum, with Shift at the release making it Horizontal or Vertical.
///
/// The state lives between passes in the tool; [`picture`](Self::picture) says each pass what the canvas shows. The
/// constraint being drawn is painted by `ConstraintLines` (the tool has no colours of its own): hand it
/// [`draft`](Self::draft).
#[derive(Clone, Debug, Default)]
pub struct ConstraintTool {
    /// The drag's two ends in canvas units.
    drag: Option<[Point; 2]>,
    /// The canvas picture's size in canvas units; a press outside it is not the tool's.
    canvas: Option<[f64; 2]>,
    /// Document pixels per canvas unit, per axis.
    scale: [f64; 2],
    edits: Vec<WideAngleConstraintEdit>,
}

impl ConstraintTool {
    /// An idle tool.
    pub fn new() -> Self {
        ConstraintTool { scale: [1.0, 1.0], ..ConstraintTool::default() }
    }

    /// The canvas picture is `canvas` pixels and the document `document` pixels: the pointer converts between them.
    pub fn picture(&mut self, canvas: [u32; 2], document: [u32; 2]) {
        let ratio = |document: u32, canvas: u32| if canvas == 0 { 1.0 } else { f64::from(document) / f64::from(canvas) };
        self.canvas = Some([f64::from(canvas[0]), f64::from(canvas[1])]);
        self.scale = [ratio(document[0], canvas[0]), ratio(document[1], canvas[1])];
    }

    /// Takes the edits collected since the last call.
    pub fn take_edits(&mut self) -> Vec<WideAngleConstraintEdit> {
        std::mem::take(&mut self.edits)
    }

    /// The constraint being drawn, in canvas units.
    pub fn draft(&self) -> Option<[Point; 2]> {
        self.drag
    }

    /// A drag is drawing a constraint: the canvas shows the source, not the corrected preview, and the history waits.
    pub fn dragging(&self) -> bool {
        self.drag.is_some()
    }

    fn to_document(&self, p: Point) -> Point {
        Point::new(p.x * self.scale[0], p.y * self.scale[1])
    }

    fn doc_point(cx: &ToolContext<'_>, pointer: &ToolPointer) -> Option<Point> {
        cx.space().map(|space| space.view.to_doc(pointer.pos))
    }
}

impl CanvasTool for ConstraintTool {
    fn press(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) -> ToolAction {
        let Some(at) = Self::doc_point(cx, pointer) else { return ToolAction::Ignored };
        let inside = self.canvas.is_none_or(|[w, h]| (0.0..=w).contains(&at.x) && (0.0..=h).contains(&at.y));
        if !inside {
            return ToolAction::Ignored;
        }
        self.drag = Some([at, at]);
        ToolAction::Claimed
    }

    fn drag(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        if let (Some(drag), Some(at)) = (self.drag.as_mut(), Self::doc_point(cx, pointer)) {
            drag[1] = at;
        }
    }

    fn release(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        let Some([a, _]) = self.drag.take() else { return };
        let Some(b) = Self::doc_point(cx, pointer) else { return };
        let Some(space) = cx.space() else { return };
        if space.view.to_screen(a).distance(space.view.to_screen(b)) > DRAG_MINIMUM {
            let constraint = WideAngleConstraint::from_drag(self.to_document(a), self.to_document(b), pointer.mods.shift);
            self.edits.push(WideAngleConstraintEdit::Add(constraint));
        }
    }

    fn cancel(&mut self, _: CancelReason, _: &mut ToolContext<'_>) {
        self.drag = None;
    }

    fn paint(&self, _: &Draw<'_>, _: &ToolContext<'_>) {}

    fn cursor(&self, _: &ToolContext<'_>) -> Cursor {
        PaintedCursor::Crosshair.into()
    }

    fn command_form(&self) -> ToolPath {
        ToolPath::Command("the Add Constraint… command")
    }
}
