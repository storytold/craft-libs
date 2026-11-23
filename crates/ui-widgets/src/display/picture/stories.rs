//! Stories of `Picture`.

use std::sync::Arc;

use craft_host::InlineJobs;

use super::{Picture, PictureSize};
use crate::contract::Show as _;
use crate::list::{ThumbAspect, ThumbSize};
use crate::primitives::textures::{FakeThumbSource, ThumbnailOptions, Thumbnails};
use crate::testing::{Drive, StoryData, StoryOutput, story};
use egui::{Color32, ColorImage, Ui};

/// A diagonal gradient, so a fitted or stretched picture shows.
fn gradient() -> ColorImage {
    const SIDE: usize = 32;
    let pixels = (0..SIDE * SIDE).map(|at| Color32::from_rgb((at % SIDE * 255 / SIDE) as u8, (at / SIDE * 255 / SIDE) as u8, 160)).collect();
    ColorImage::new([SIDE, SIDE], pixels)
}

#[story(Picture<'static>, covers = [Default], drive = Drive::Never)]
fn picture(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Picture::new("Gradient", 1, &gradient).show(ui).into()
}

#[story(Picture<'static>, covers = [], drive = Drive::Never)]
fn picture_thumbnail(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    ui.horizontal(|ui| {
        for (key, step) in [(2, ThumbSize::Small), (3, ThumbSize::Medium), (4, ThumbSize::Large)] {
            Picture::new("Gradient", key, &gradient).extent(PictureSize::Thumb(step)).show(ui);
        }
        Picture::new("Gradient", 5, &gradient).extent(PictureSize::Icon).show(ui)
    })
    .inner
    .into()
}

/// A picture made off the frame: its source renders as a `Later` job, so the picture shows the placeholder until the
/// job lands on the loader's lanes, then the picture. The job runs inline, so the picture lands in the next frame and
/// not whenever a thread ends: the suite's settled frames are the same on every run.
#[story(Picture<'static>, covers = [], drive = Drive::Never)]
fn picture_rendered(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let options = ThumbnailOptions::new().jobs(Arc::new(InlineJobs));
    let thumbnails = Thumbnails::<u64>::in_context(ui.ctx(), "craft_ui::stories::picture_rendered", options);
    let source = FakeThumbSource::<u64>::new().later(true).opaque(true);
    Picture::rendered("Rendered", 1, &thumbnails, &source).extent(PictureSize::Thumb(ThumbSize::Large)).show(ui).into()
}

#[story(Picture<'static>, covers = [], drive = Drive::Never)]
fn picture_painter(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Picture::painter("Swatch", ThumbAspect::new(4, 1), |cx, rect| cx.checker(rect)).extent(PictureSize::Thumb(ThumbSize::Large)).show(ui).into()
}

/// A painter preview that fills the room its parent gives, 4 wide to 1 high.
#[story(Picture<'static>, covers = [], drive = Drive::Never)]
fn picture_fill(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Picture::painter("Preview", ThumbAspect::new(4, 1), |cx, rect| cx.checker(rect)).extent(PictureSize::Fill).show(ui).into()
}
