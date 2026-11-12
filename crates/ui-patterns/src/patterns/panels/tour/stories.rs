//! Stories of `Tour`.

use super::{Tour, TourStep};
use crate::buttons::Button;
use crate::contract::{Bind, Show as _};
use crate::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

#[story(Tour<'static>, covers = [], drive = Drive::Never)]
fn tour(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let layers = Button::new("Layers").show(ui);
    let steps = [
        TourStep::new("Layers", "Every object of the poster is a layer. Reorder them here.").target(layers.rect),
        TourStep::new("Properties", "The selected layer's settings show in this panel."),
        TourStep::new("Export", "When the poster is ready, export it from the File menu."),
    ];
    Tour::new("Welcome tour", &steps, Bind::bound(data.value("index", 0_usize))).show(ui);
    StoryOutput::default()
}
