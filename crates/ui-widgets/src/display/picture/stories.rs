//! Stories of `Picture`.

use super::{Picture, PictureSize};
use crate::contract::Show as _;
use crate::list::ThumbSize;
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
