//! The view tools: what a gesture asks the app's view to become, and the checks every tool owes the canvas.

use craft_ui::canvas::{HandTool, NavigationOptions, RotateViewTool, ZoomTool};
use craft_ui::testing::{ToolFixture, trial};

#[trial]
fn each_view_tool_owes_the_canvas_its_checks() {
    ToolFixture::new("Hand", HandTool::new(NavigationOptions::default())).check();
    ToolFixture::new("Zoom", ZoomTool::new(NavigationOptions::default())).check();
    ToolFixture::new("Rotate View", RotateViewTool::default()).check();
}
