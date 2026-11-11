//! Stories of `Tour`.

use super::{Tour, TourStep};
use crate::contract::Bind;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use egui::{Rect, Ui, pos2, vec2};
use std::sync::LazyLock;

/// Where the first step's part sits in the story's window, and how big it is.
const FIRST_AT: (f32, f32) = (40.0, 60.0);
const FIRST_SIZE: (f32, f32) = (160.0, 48.0);

static STEPS: LazyLock<Vec<TourStep>> = LazyLock::new(|| {
    vec![
        TourStep::new("Layers", "Every object of the poster is a layer. Reorder them here.")
            .target(Rect::from_min_size(pos2(FIRST_AT.0, FIRST_AT.1), vec2(FIRST_SIZE.0, FIRST_SIZE.1))),
        TourStep::new("Properties", "The selected layer's settings show in this panel."),
        TourStep::new("Export", "When the poster is ready, export it from the File menu."),
    ]
});

#[story(Tour<'static>, covers = [], drive = Drive::Never)]
fn tour(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    Tour::new("Welcome tour", STEPS.as_slice(), Bind::bound(data.value("index", 0_usize))).show(ui);
    StoryOutput::default()
}
