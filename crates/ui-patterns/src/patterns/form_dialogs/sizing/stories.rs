//! Stories of `ImageSize` and `CanvasSize`.

use super::{CanvasSize, CanvasSizeValues, HostedCanvasSize, HostedImageSize, ImageSize, ImageSizeValues};
use crate::dialog::stories::hosted;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use craft_behavior::PixelSize;
use egui::Ui;

const PICTURE: PixelSize = PixelSize::new(1600, 900);

#[story(ImageSize<'static>, covers = [], drive = Drive::Never)]
#[expect(clippy::redundant_closure, reason = "`ImageSize::new` as a function item is not general over the borrow the host passes")]
fn image_size(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let resample = *data.value("resample", true);
    hosted(ui, data, resample, || HostedImageSize::new(ImageSizeValues::new(PICTURE).resampling(resample), |values| ImageSize::new(values)))
}

#[story(CanvasSize<'static>, covers = [], drive = Drive::Never)]
#[expect(clippy::redundant_closure, reason = "`CanvasSize::new` as a function item is not general over the borrow the host passes")]
fn canvas_size(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    hosted(ui, data, (), || HostedCanvasSize::new(CanvasSizeValues::new(PICTURE), |values| CanvasSize::new(values)))
}
