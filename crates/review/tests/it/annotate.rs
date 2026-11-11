//! The Eraser on a canvas.

use craft_behavior::CanvasView;
use craft_review::annotate::{Annotation, AnnotationEdit, AnnotationId, AnnotationKind, AnnotationTool};
use craft_review::ui::AnnotateTool;
use craft_ui::testing::{ToolFixture, trial};
use egui::{Pos2, Rect as ScreenRect, vec2};
use kurbo::{Point, Rect};

fn view(room: ScreenRect) -> CanvasView {
    CanvasView::new(room, Point::new(f64::from(room.width()) / 2.0, f64::from(room.height()) / 2.0), 1.0)
}

/// `tool` on a canvas whose page (0, 0) is the canvas's top left corner at zoom 1.
fn rig(tool: AnnotateTool) -> ToolFixture<AnnotateTool> {
    ToolFixture::with_view("Document", tool, view, |_, _| {})
}

/// The screen position of page point `at`.
fn on_screen(rig: &ToolFixture<AnnotateTool>, at: Point) -> Pos2 {
    rig.canvas().rect.min + vec2(at.x as f32, at.y as f32)
}

fn annotation(id: u64, kind: AnnotationKind, rect: Rect) -> Annotation {
    Annotation {
        id: AnnotationId(id),
        page: 0,
        kind,
        rect,
        points: Vec::new(),
        strokes: Vec::new(),
        color: [255, 0, 0],
        opacity: 1.0,
        stroke_width: 1.0,
        text: String::new(),
        locked: false,
    }
}

fn ink(id: u64, xs: &[f64]) -> Annotation {
    let mut ink = annotation(id, AnnotationKind::Ink, Rect::new(0.0, 0.0, 300.0, 200.0));
    ink.strokes = xs.iter().map(|&x| vec![Point::new(x, 0.0), Point::new(x, 100.0)]).collect();
    ink
}

/// The edits' values, in order.
fn values(rig: &ToolFixture<AnnotateTool>) -> Vec<AnnotationEdit> {
    rig.tool_mut().take_edits().iter().filter_map(|edit| edit.value().cloned()).collect()
}

/// Drags the tool from page point `from` to page point `to`.
fn drag(rig: &mut ToolFixture<AnnotateTool>, from: Point, to: Point) {
    let (start, end) = (on_screen(rig, from), on_screen(rig, to));
    rig.drag_from(start, end - start);
}

/// The Eraser reports each annotation once, even when its strokes are crossed with another annotation's in
/// between: an app removing strokes by index would otherwise delete the wrong ones.
#[trial]
fn eraser_reports_one_erase_per_annotation_crossing_a_b_a() {
    let mut tool = AnnotateTool::new(AnnotationTool::Eraser, 0);
    tool.annotations = vec![ink(1, &[50.0, 250.0]), ink(2, &[150.0])];
    let mut rig = rig(tool);
    drag(&mut rig, Point::new(10.0, 50.0), Point::new(280.0, 50.0));
    assert_eq!(
        values(&rig),
        vec![AnnotationEdit::Erase { id: AnnotationId(1), strokes: vec![0, 1] }, AnnotationEdit::Erase { id: AnnotationId(2), strokes: vec![0] }]
    );
}
