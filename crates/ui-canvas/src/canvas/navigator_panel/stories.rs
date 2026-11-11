//! Stories of the Navigator panel.

use super::NavigatorPanel;
use crate::contract::Bind;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use craft_behavior::CanvasView;
use craft_behavior::kurbo::{Point, Size};
use egui::{Pos2, Rect, Ui};

/// The document the panel shows, in document units.
const DOCUMENT: Size = Size::new(2000.0, 1500.0);

/// The canvas rect of the story's view, in points.
const CANVAS: Rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(800.0, 600.0));

#[story(NavigatorPanel<'static>, covers = [], drive = Drive::Never)]
fn navigator_panel(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let view = data.value("view", CanvasView::new(CANVAS, Point::new(1000.0, 750.0), 1.0));
    NavigatorPanel::new("Navigator", Bind::bound(view), DOCUMENT).show(ui).into()
}

#[story(NavigatorPanel<'static>, covers = [], drive = Drive::Never)]
fn navigator_panel_disabled(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let view = data.value("view", CanvasView::new(CANVAS, Point::new(1000.0, 750.0), 1.0));
    NavigatorPanel::new("Navigator", Bind::bound(view), DOCUMENT).enabled(false).show(ui).into()
}
