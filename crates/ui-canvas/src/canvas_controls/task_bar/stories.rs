//! Stories of `TaskBar`.

use super::{TaskBar, TaskBarPlace};
use crate::buttons::{Button, IconButton};
use crate::canvas::testing::canvas_story::{StoryDocument, on_canvas};
use crate::contract::{Bind, Props as _, Show as _};
use crate::testing::{Drive, StoryData, StoryOutput, story};
use craft_behavior::IconId;
use craft_behavior::kurbo::{Point, Vec2};
use egui::Ui;

/// The corners of a crop frame turned by a few degrees about the document's middle: the Crop tool hangs its bar from
/// them, so the bar sits under the frame's lowest corner.
fn turned_frame() -> [Point; 4] {
    let centre = StoryDocument::STANDARD.centre();
    let (sin, cos) = 0.2_f64.sin_cos();
    [(-220.0, -130.0), (220.0, -130.0), (220.0, 130.0), (-220.0, 130.0)].map(|(x, y)| centre + Vec2::new(x * cos - y * sin, x * sin + y * cos))
}

/// The Crop tool's bar: three buttons, the More menu, then Cancel and Done after it.
#[story(TaskBar<'static>, covers = [], drive = Drive::Never)]
fn crop_bar(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let corners = turned_frame();
    on_canvas(ui, |ui, area, _| {
        let place = data.value("place", TaskBarPlace::default());
        TaskBar::new("Crop", &corners, Bind::bound(place))
            .show(ui, area, |ui, cx| {
                for (icon, name) in [("rotate_ccw", "Rotate left"), ("rotate_cw", "Rotate right"), ("grid", "Overlay")] {
                    let _ = IconButton::new(IconId::new(icon)).label(name).show(ui);
                }
                cx.more_menu(ui);
                let _ = Button::new("Cancel").show(ui);
                let _ = Button::new("Done").show(ui);
            })
            .output
    })
}

/// A bar the user moved and pinned: it keeps its place near the canvas's top left while the frame stays where it is.
#[story(TaskBar<'static>, covers = [], drive = Drive::Never)]
fn placed_and_pinned(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let corners = turned_frame();
    on_canvas(ui, |ui, area, _| {
        let place = data.value("place", TaskBarPlace { pinned: true, at: Some([24.0, 24.0]) });
        TaskBar::new("Crop", &corners, Bind::bound(place))
            .show(ui, area, |ui, _| {
                let _ = Button::new("Done").show(ui);
            })
            .output
    })
}
