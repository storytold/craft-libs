//! Stories of the page preview.

use super::{PagePicture, PagePreview};
use crate::ui::fixtures::{DemoPages, PAPER};
use craft_ui::contract::{Props as _, Show as _};
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

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
