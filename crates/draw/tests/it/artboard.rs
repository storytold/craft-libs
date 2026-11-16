//! The Artboard tool's placing rule, driven through the canvas fixture's pointer input.

use craft_behavior::kurbo::{Point, Size};
use craft_behavior::{CancelReason, CanvasView, EditEvent};
use craft_draw::ui::{ArtboardEvent, ArtboardTool};
use craft_ui::canvas::{CanvasTool, ToolAction, ToolContext, ToolKey, ToolPath, ToolPointer};
use craft_ui::primitives::Draw;
use craft_ui::testing::{ToolFixture, trial};
use egui::Vec2;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// The tool for the fixture: it sets the shared placing size before every call and keeps the events the tool reported,
/// as the app would.
struct Driven {
    tool: ArtboardTool,
    placing: Rc<Cell<Option<Size>>>,
    log: Rc<RefCell<Vec<ArtboardEvent>>>,
}

impl Driven {
    fn with_tool<R>(&mut self, call: impl FnOnce(&mut ArtboardTool) -> R) -> R {
        self.tool.placing(self.placing.get());
        let result = call(&mut self.tool);
        self.log.borrow_mut().extend(self.tool.take_events());
        result
    }
}

impl CanvasTool for Driven {
    fn press(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) -> ToolAction {
        self.with_tool(|tool| tool.press(pointer, cx))
    }

    fn drag(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        self.with_tool(|tool| tool.drag(pointer, cx));
    }

    fn release(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        self.with_tool(|tool| tool.release(pointer, cx));
    }

    fn cancel(&mut self, reason: CancelReason, cx: &mut ToolContext<'_>) {
        self.with_tool(|tool| tool.cancel(reason, cx));
    }

    fn key(&mut self, key: &ToolKey, cx: &mut ToolContext<'_>) -> bool {
        self.with_tool(|tool| tool.key(key, cx))
    }

    fn paint(&self, _: &Draw<'_>, _: &ToolContext<'_>) {}

    fn command_form(&self) -> ToolPath {
        ToolPath::Command("the Artboard tool")
    }
}

/// The fixture, the size being placed and the events the tool raised.
type Shown = (ToolFixture<Driven>, Rc<Cell<Option<Size>>>, Rc<RefCell<Vec<ArtboardEvent>>>);

fn shown(placing: Option<Size>) -> Shown {
    let placing = Rc::new(Cell::new(placing));
    let log = Rc::new(RefCell::new(Vec::new()));
    let driven = Driven { tool: ArtboardTool::new(), placing: Rc::clone(&placing), log: Rc::clone(&log) };
    let fixture = ToolFixture::with_view(
        "Artboard",
        driven,
        |room| CanvasView::new(room, Point::new(f64::from(room.width()) / 2.0, f64::from(room.height()) / 2.0), 1.0),
        |_, _| {},
    );
    (fixture, placing, log)
}

/// While placing, a click on the pasteboard reports a board of the placing size at the click in whole pixels, and a drag
/// still draws; with no placing size a click reports nothing under Photoshop's rules.
#[trial]
fn placing_puts_a_board_at_a_click_and_a_drag_still_draws() {
    let size = Size::new(180.0, 240.0);
    let (mut fixture, placing, log) = shown(Some(size));
    let room = fixture.canvas();
    let at = room.center();
    fixture.press_at(at).release_at(at);
    let events: Vec<ArtboardEvent> = log.borrow_mut().drain(..).collect();
    match events.as_slice() {
        [ArtboardEvent::Place { rect, .. }] => {
            assert_eq!(Size::new(rect.width(), rect.height()), size);
            // The view puts document point (room/2, room/2) at the canvas centre, where the press lands.
            let centre = Point::new((f64::from(room.rect.width()) / 2.0).round(), (f64::from(room.rect.height()) / 2.0).round());
            assert_eq!(rect.origin(), centre);
        }
        other => panic!("a click while placing must report one Place, got {other:?}"),
    }

    fixture.drag(Vec2::new(60.0, 40.0));
    let events: Vec<ArtboardEvent> = log.borrow_mut().drain(..).collect();
    assert!(events.iter().all(|event| matches!(event, ArtboardEvent::Draw(_))), "a drag only draws: {events:?}");
    assert!(matches!(events.first(), Some(ArtboardEvent::Draw(EditEvent::Begin { .. }))), "{events:?}");
    assert!(matches!(events.last(), Some(ArtboardEvent::Draw(EditEvent::Commit { .. }))), "{events:?}");

    placing.set(None);
    fixture.press_at(at).release_at(at);
    assert!(log.borrow().is_empty(), "a click with no placing size reports nothing: {:?}", log.borrow());
}
