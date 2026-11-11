//! Stories of `ImageSize` and `CanvasSize`.

use super::{CanvasSize, CanvasSizeValues, ImageSize, ImageSizeValues};
use crate::contract::Bind;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use craft_behavior::PixelSize;
use egui::Ui;

const PICTURE: PixelSize = PixelSize::new(1600, 900);

#[story(ImageSize<'static>, covers = [], drive = Drive::Never)]
fn image_size(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let resample = *data.value("resample", true);
    let (values, applied) = data.value("values", (ImageSizeValues::new(PICTURE), true));
    if *applied != resample {
        *values = values.clone().resampling(resample);
        *applied = resample;
    }
    ImageSize::new(Bind::bound(values)).opened(Bind::given(true)).show(ui).into()
}

#[story(CanvasSize<'static>, covers = [], drive = Drive::Never)]
fn canvas_size(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let values = data.value("values", CanvasSizeValues::new(PICTURE));
    CanvasSize::new(Bind::bound(values)).opened(Bind::given(true)).show(ui).into()
}
