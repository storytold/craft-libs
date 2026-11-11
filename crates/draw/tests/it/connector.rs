//! The Connector tool, its handles and its bar. The view maps one document unit to one screen point.

use craft_behavior::CanvasView;
use craft_behavior::kurbo::{Point, Rect};
use craft_draw::ui::connector::{ConnectionTarget, ConnectorTool};
use craft_ui::testing::{ToolFixture, trial};

const A: Rect = Rect::new(20.0, 100.0, 100.0, 200.0);

fn target(id: u64, rect: Rect) -> ConnectionTarget {
    ConnectionTarget::new(id, rect)
}

/// Three boxes: A (1) on the left, B (2) on the right at the same height, C (3) above A's right.
fn boxes() -> Vec<ConnectionTarget> {
    vec![target(1, A), target(2, Rect::new(250.0, 100.0, 330.0, 200.0)), target(3, Rect::new(150.0, 20.0, 200.0, 60.0))]
}

/// The view of a canvas filling `room`, with its middle at the document's middle.
fn room_view(room: egui::Rect) -> CanvasView {
    CanvasView::new(room, Point::new(f64::from(room.width()) / 2.0, f64::from(room.height()) / 2.0), 1.0)
}

fn tool(targets: Vec<ConnectionTarget>) -> ConnectorTool {
    let mut tool = ConnectorTool::new();
    tool.targets(&targets);
    tool
}

#[trial]
fn the_connector_tool_owes_the_canvas_what_every_tool_owes() {
    ToolFixture::with_view("Connector", tool(boxes()), room_view, |_, _| {}).check();
}
