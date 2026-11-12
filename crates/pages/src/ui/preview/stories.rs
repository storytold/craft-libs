//! Stories of the page preview.

use super::{PagePicture, PagePreview, PagePreviewWords, PreviewLayer};
use crate::ui::fixtures::{DemoPages, PAPER};
use craft_ui::contract::{Bind, Props as _, Show as _, ShowContainer};
use craft_ui::runtime::env::{Env, Label};
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;
use std::cell::Cell;

#[story(PagePreview<'static>, covers = [Open], drive = Drive::Never)]
fn open(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let at = data.value("at", 1_usize);
    let pages = DemoPages::new(5);
    let shown = [0, 1, 2, 3, 4];
    PagePreview::new(&pages, &shown, at, PAPER).show(ui).into()
}

#[story(PagePicture<'static>, covers = [Default], drive = Drive::Never)]
fn page_picture(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let pages = DemoPages::new(5);
    PagePicture::new(&pages, 2, "Page unavailable", PAPER).label("Page 3").show(ui).into()
}

#[story(PreviewLayer<'static>, covers = [Default, Open], drive = Drive::Never)]
fn preview_layer(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let words = Env::of(ui).words::<PagePreviewWords>();
    let step = Cell::new(0);
    let pages = DemoPages::new(5);
    let layer = PreviewLayer::new(&words, Label::from("Page 2 of 5"), 1, 4, &step, Bind::bound(data.value("open", true)));
    ShowContainer::show(layer, ui, |ui, cx| {
        PagePicture::new(&pages, 1, "Page unavailable", PAPER).label("Page 2").show_at(ui, cx.rect);
    })
    .into()
}
