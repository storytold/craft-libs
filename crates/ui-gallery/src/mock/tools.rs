//! The canvas tools of the mock: what the window keeps across passes in its root container, the Move tool and the
//! rectangular Marquee.

use super::model::{Document, MarqueeDrag};
use craft_behavior::kurbo::{Point, Rect as DocRect};
use craft_behavior::{CancelReason, Cursor, CursorIcon};
use craft_ui::canvas::{CanvasTool, HandTool, Momentary, NavigationOptions, ToolAction, ToolContext, ToolKey, ToolPath, ToolPointer, ZoomTool};
use craft_ui::primitives::Draw;
use craft_ui::runtime::env::Label;
use egui::Pos2;

/// The tools the canvas runs for the rail's Hand and Zoom, and the momentary pair of both.
#[derive(Clone, Debug)]
pub(super) struct Tools {
    pub(super) hand: HandTool,
    pub(super) zoom: ZoomTool,
    pub(super) momentary: Momentary,
}

impl Default for Tools {
    fn default() -> Self {
        let options = NavigationOptions::default();
        Tools { hand: HandTool::new(options), zoom: ZoomTool::new(options), momentary: Momentary::new(options) }
    }
}

/// The Move tool: it takes no press of its own (the guides over the canvas take theirs) and its command form is
/// moving the current layer.
#[derive(Debug, craft_ui::contract::Pattern)]
#[pattern(tool)]
pub(super) struct MoveTool;

impl CanvasTool for MoveTool {
    fn press(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) -> ToolAction {
        ToolAction::Ignored
    }

    fn drag(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {}

    fn release(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {}

    fn cancel(&mut self, _: CancelReason, _: &mut ToolContext<'_>) {}

    fn paint(&self, _: &Draw<'_>, _: &ToolContext<'_>) {}

    fn command_form(&self) -> ToolPath {
        ToolPath::Command("the Move Layer command")
    }
}

/// The rectangular Marquee over the document's selection: a drag makes the selection, a click away clears it, and
/// Escape clears it too.
#[derive(Debug)]
pub(super) struct MarqueeTool<'a> {
    document: &'a mut Document,
}

impl<'a> MarqueeTool<'a> {
    pub(super) fn new(document: &'a mut Document) -> Self {
        MarqueeTool { document }
    }

    fn drag_to(&mut self, at: Pos2, cx: &mut ToolContext<'_>) {
        let (Some(space), Some(drag)) = (cx.space(), self.document.dragging.as_mut()) else { return };
        drag.now = space.view.to_doc(at);
        let rect = drag.rect();
        cx.readout(vec![(Label::from("W"), format!("{:.0}", rect.width())), (Label::from("H"), format!("{:.0}", rect.height()))]);
    }
}

impl CanvasTool for MarqueeTool<'_> {
    fn press(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) -> ToolAction {
        let Some(space) = cx.space() else { return ToolAction::Ignored };
        let start = space.view.to_doc(pointer.pos);
        self.document.dragging = Some(MarqueeDrag { start, now: start });
        ToolAction::Claimed
    }

    fn drag(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        self.drag_to(pointer.pos, cx);
    }

    fn release(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        self.drag_to(pointer.pos, cx);
        let rect = self.document.dragging.take().map(|drag| drag.rect());
        // A press that never grew is a click away from the selection: it clears it.
        self.document.marquee = rect.filter(|rect| rect.area() > 0.0);
    }

    fn cancel(&mut self, _: CancelReason, _: &mut ToolContext<'_>) {
        self.document.dragging = None;
    }

    fn key(&mut self, key: &ToolKey, _: &mut ToolContext<'_>) -> bool {
        let clears = matches!(key, ToolKey::Cancel) && self.document.marquee.is_some();
        if clears {
            self.document.marquee = None;
        }
        clears
    }

    fn paint(&self, _: &Draw<'_>, cx: &ToolContext<'_>) {
        let (Some(space), Some(drag)) = (cx.space(), self.document.dragging) else { return };
        let DocRect { x0, y0, x1, y1 } = drag.rect();
        let corners = [(x0, y0), (x1, y0), (x1, y1), (x0, y1)].map(|(x, y)| space.view.to_screen(Point::new(x, y)));
        cx.marching_ants(&corners, true);
    }

    fn cursor(&self, _: &ToolContext<'_>) -> Cursor {
        Cursor::System(CursorIcon::Crosshair)
    }

    fn command_form(&self) -> ToolPath {
        ToolPath::Command("the Select Rectangle command")
    }
}
