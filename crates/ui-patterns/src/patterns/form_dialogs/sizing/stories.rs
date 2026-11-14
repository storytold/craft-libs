//! Stories of `ImageSize` and `CanvasSize`.

use super::{CanvasSize, CanvasSizeValues, HostedCanvasSize, HostedImageSize, ImageSize, ImageSizeValues};
use crate::dialog::stories::hosted;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use craft_behavior::PixelSize;
use egui::Ui;

const PICTURE: PixelSize = PixelSize::new(1600, 900);

#[story(ImageSize<'static>, covers = [], drive = Drive::Never)]
fn image_size(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let resample = *data.value("resample", true);
    hosted(ui, data, resample, || HostedImageSize::new(ImageSizeValues::new(PICTURE).resampling(resample), ImageSize::new))
}

#[story(CanvasSize<'static>, covers = [], drive = Drive::Never)]
fn canvas_size(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    hosted(ui, data, (), || HostedCanvasSize::new(CanvasSizeValues::new(PICTURE), CanvasSize::new))
}
