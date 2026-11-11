//! The Shape tools and the Create Shape dialog. The view maps one document unit to one screen point.

use craft_behavior::CanvasView;
use craft_behavior::kurbo::Point;
use craft_draw::ui::shape_drag::{ShapeDragOptions, ShapeDragTool, ShapeKind};
use craft_ui::testing::{ToolFixture, trial};

/// The view of a canvas filling `room`, with its middle at the document's middle.
fn room_view(room: egui::Rect) -> CanvasView {
    CanvasView::new(room, Point::new(f64::from(room.width()) / 2.0, f64::from(room.height()) / 2.0), 1.0)
}

fn tool(kind: ShapeKind) -> ShapeDragTool {
    let mut tool = ShapeDragTool::new();
    tool.options(ShapeDragOptions::new(kind));
    tool
}

#[trial]
fn the_shape_tool_owes_the_canvas_what_every_tool_owes() {
    ToolFixture::with_view("Shape", tool(ShapeKind::Rectangle), room_view, |_, _| {}).check();
}
