//! The view tools: what a gesture asks the app's view to become, and the checks every tool owes the canvas.

use craft_behavior::CanvasView;
use craft_behavior::canvas::zoom;
use craft_behavior::kurbo::{Point, Rect, Size};
use craft_ui::canvas::{HandTool, NavigationOptions, RotateViewTool, ViewCommands, ViewTarget, ZoomTool};
use craft_ui::testing::{ToolFixture, trial};

#[trial]
fn each_view_tool_owes_the_canvas_its_checks() {
    ToolFixture::new("Hand", HandTool::new(NavigationOptions::default())).check();
    ToolFixture::new("Zoom", ZoomTool::new(NavigationOptions::default())).check();
    ToolFixture::new("Rotate View", RotateViewTool::default()).check();
}

/// The view commands that read a fact of the document beyond its size: each runs with its fact, and without it is
/// disabled and leaves the view as it is.
#[trial]
fn view_commands_run_with_their_facts_and_are_disabled_without() {
    let ppp = 2.0;
    let size = Size::new(4000.0, 3000.0);
    let board = Rect::new(2500.0, 400.0, 3500.0, 1200.0);
    let layers = Rect::new(100.0, 2000.0, 300.0, 2900.0);
    let full = ViewTarget::new(size).artboard(board).layers(layers).ppi(300.0);
    let bare = ViewTarget::from(size);
    let start = CanvasView::new(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 700.0)), Point::new(2000.0, 1500.0), 0.1);
    let device = |view: &CanvasView| zoom::device(view, ppp);
    let shows = |view: &CanvasView, rect: Rect| {
        [rect.origin(), Point::new(rect.x1, rect.y1)].iter().all(|corner| view.rect.expand(0.5).contains(view.to_screen(*corner)))
    };
    // What each command owes the view it changed.
    let rows: [(ViewCommands, &dyn Fn(&CanvasView) -> bool); 4] = [
        (ViewCommands::TwoHundredPercent, &|view| (device(view) - 2.0).abs() < 1e-9 && view.center == start.center),
        // Print Size: the screen's 72 ppi over the document's 300.
        (ViewCommands::PrintSize, &|view| (device(view) - 72.0 / 300.0).abs() < 1e-9 && view.center == start.center),
        (ViewCommands::FitArtboard, &|view| view.center == board.center() && shows(view, board) && device(view) > device(&start)),
        (ViewCommands::FitLayers, &|view| view.center == layers.center() && shows(view, layers) && device(view) > device(&start)),
    ];
    for (command, owed) in rows {
        let mut view = start;
        assert!(command.available(&start, &full, ppp), "{command:?} runs with its fact");
        assert!(command.apply(&mut view, &full, ppp) && owed(&view), "{command:?} gives {view:?}");
        if command == ViewCommands::TwoHundredPercent {
            assert!(command.available(&start, &bare, ppp), "200% needs no fact");
            continue;
        }
        let mut view = start;
        assert!(!command.available(&start, &bare, ppp), "{command:?} is disabled without its fact");
        assert!(!command.apply(&mut view, &bare, ppp) && view == start, "{command:?} leaves the view without its fact");
    }
}
