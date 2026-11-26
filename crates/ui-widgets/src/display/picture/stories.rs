//! Stories of `Picture`.

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
/// job lands on the loader's lanes, then the picture.
#[story(Picture<'static>, covers = [], drive = Drive::Never)]
fn picture_rendered(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let options = ThumbnailOptions::new();
    let thumbnails = Thumbnails::<u64>::in_context(ui.ctx(), "craft_ui::stories::picture_rendered", options);
    let source = FakeThumbSource::<u64>::new().later(true).opaque(true);
    Picture::rendered("Rendered", 1, &thumbnails, &source).extent(PictureSize::Thumb(ThumbSize::Large)).show(ui).into()
}

/// An original mark: a teal disc with an amber diamond, so the SVG's own colours and its fit show.
const MARK: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><circle cx="12" cy="12" r="11" fill="#1f8a8a"/><path d="M12 4 L19 12 L12 20 L5 12 Z" fill="#f2b233"/></svg>"##;

/// An original 4:1 drawing: a wordmark-shaped bar with two dots.
const WIDE: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 80 20"><rect x="1" y="1" width="78" height="18" rx="9" fill="#3b5fc0"/><circle cx="12" cy="10" r="5" fill="#ffffff"/><circle cx="68" cy="10" r="5" fill="#f2b233"/></svg>"##;

/// Bytes that are not an SVG document.
const NOT_SVG: &[u8] = b"<svg xmlns=\"http://www.w3.org/2000/svg\"><circle";

#[story(Picture<'static>, covers = [], drive = Drive::Never)]
fn picture_svg(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    ui.horizontal(|ui| {
        Picture::svg("Mark", MARK).extent(PictureSize::Icon).show(ui);
        Picture::svg("Mark", MARK).extent(PictureSize::Thumb(ThumbSize::Small)).show(ui);
        Picture::svg("Mark", MARK).extent(PictureSize::Thumb(ThumbSize::Large)).show(ui)
    })
    .inner
    .into()
}

/// A 4:1 SVG with no aspect set is shown four times wider than tall.
#[story(Picture<'static>, covers = [], drive = Drive::Never)]
fn picture_wide_svg(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Picture::svg("Wide mark", WIDE).extent(PictureSize::Thumb(ThumbSize::Small)).show(ui).into()
}

#[story(Picture<'static>, covers = [], drive = Drive::Never)]
fn picture_svg_invalid(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Picture::svg("Broken", NOT_SVG).extent(PictureSize::Thumb(ThumbSize::Small)).show(ui).into()
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

/// A rendered picture without the transparency grid, for a mask that sits on the surface.
#[story(Picture<'static>, covers = [], drive = Drive::Never)]
fn picture_rendered_without_checker(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let options = ThumbnailOptions::new();
    let thumbnails = Thumbnails::<u64>::in_context(ui.ctx(), "craft_ui::stories::picture_rendered_without_checker", options);
    let source = FakeThumbSource::<u64>::new().later(true);
    Picture::rendered("Mask", 1, &thumbnails, &source).extent(PictureSize::Thumb(ThumbSize::Large)).checker(false).show(ui).into()
}
