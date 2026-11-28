//! Stories of `EyedropperBar`.

use super::{EyedropperBar, EyedropperOptions, SampleSize, SampleSizeSelect};
use craft_ui::bars::{Bar, BarKind};
use craft_ui::contract::ShowContainer as _;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

#[story(SampleSizeSelect<'static>, covers = [], drive = Drive::Never)]
fn sample_size_select(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (size, size_at) = data.value_at("size", SampleSize::Point);
    StoryOutput::from(SampleSizeSelect::new(size_at.bind(size)).show(ui))
}

#[story(EyedropperBar<'static>, covers = [], drive = Drive::Never)]
fn eyedropper_bar(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let options = data.value("options", EyedropperOptions::default());
    let shown = Bar::new("Eyedropper options", BarKind::Options).show(ui, |ui, _| EyedropperBar::new(&mut *options).show(ui));
    match shown.inner {
        Some(output) => StoryOutput::from(output),
        None => StoryOutput::from(shown.output),
    }
}
