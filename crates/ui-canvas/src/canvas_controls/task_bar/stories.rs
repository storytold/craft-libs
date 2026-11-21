//! Stories of `TaskBar`.

use super::{TaskBar, TaskBarDismiss, TaskBarPlace};
use crate::buttons::{Button, IconButton};
use crate::canvas::testing::canvas_story::{StoryDocument, on_canvas};
use crate::contract::{Props as _, Show as _};
use crate::testing::{Drive, StoryData, StoryOutput, story};
use craft_behavior::IconId;
use craft_behavior::kurbo::{Point, Vec2};
use craft_behavior::overlay::Side;
use egui::Ui;

/// The corners of a crop frame turned by a few degrees about the document's middle: the Crop tool hangs its bar from
/// them, so the bar sits under the frame's lowest corner.
fn turned_frame() -> [Point; 4] {
    let centre = StoryDocument::STANDARD.centre();
    let (sin, cos) = 0.2_f64.sin_cos();
    [(-220.0, -130.0), (220.0, -130.0), (220.0, 130.0), (-220.0, 130.0)].map(|(x, y)| centre + Vec2::new(x * cos - y * sin, x * sin + y * cos))
}

/// A strip of document at its top edge, where a bar above it has no room.
fn top_strip() -> [Point; 4] {
    [(120.0, 0.0), (520.0, 0.0), (520.0, 30.0), (120.0, 30.0)].map(|(x, y)| Point::new(x, y))
}

/// A selection near the document's start edge, with room after it for a bar on its end side.
fn start_box() -> [Point; 4] {
    [(40.0, 120.0), (200.0, 120.0), (200.0, 240.0), (40.0, 240.0)].map(|(x, y)| Point::new(x, y))
}

/// The members of a selection bar.
fn formatting(ui: &mut Ui) {
    for (icon, name) in [("rotate_ccw", "Rotate left"), ("rotate_cw", "Rotate right")] {
        let _ = IconButton::new(IconId::new(icon)).label(name).show(ui);
    }
    let _ = Button::new("Done").show(ui);
}

/// A selection bar over a selection: above it, centred.
#[story(TaskBar<'static>, covers = [], drive = Drive::Never)]
fn selection_bar_over_a_selection(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let corners = turned_frame();
    on_canvas(ui, |ui, area, _| TaskBar::selection("Selection", &corners).show(ui, area, |ui, _| formatting(ui)).output)
}

/// A selection bar flips below its selection when there is no room above it.
#[story(TaskBar<'static>, covers = [], drive = Drive::Never)]
fn selection_bar_flipped_near_the_top_edge(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let strip = top_strip();
    on_canvas(ui, |ui, area, _| TaskBar::selection("Selection", &strip).show(ui, area, |ui, _| formatting(ui)).output)
}

/// A selection bar on the other side of its selection: after it in reading order.
#[story(TaskBar<'static>, covers = [], drive = Drive::Never)]
fn selection_bar_on_the_other_side(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let corners = start_box();
    on_canvas(ui, |ui, area, _| TaskBar::selection("Selection", &corners).side(Side::End).show(ui, area, |ui, _| formatting(ui)).output)
}

/// A selection bar that survives presses outside it while its graphic is selected.
#[story(TaskBar<'static>, covers = [], drive = Drive::Never)]
fn selection_bar_kept_while_selected(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let corners = turned_frame();
    on_canvas(ui, |ui, area, _| TaskBar::selection("Selection", &corners).dismiss(TaskBarDismiss::Keep).show(ui, area, |ui, _| formatting(ui)).output)
}

/// The Crop tool's bar: three buttons, the More menu, then Cancel and Done after it.
#[story(TaskBar<'static>, covers = [], drive = Drive::Never)]
fn crop_bar(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let corners = turned_frame();
    on_canvas(ui, |ui, area, _| {
        let (place, place_at) = data.value_at("place", TaskBarPlace::default());
        TaskBar::new("Crop", &corners, place_at.bind(place))
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
        let (place, place_at) = data.value_at("place", TaskBarPlace { pinned: true, at: Some([24.0, 24.0]) });
        TaskBar::new("Crop", &corners, place_at.bind(place))
            .show(ui, area, |ui, _| {
                let _ = Button::new("Done").show(ui);
            })
            .output
    })
}
