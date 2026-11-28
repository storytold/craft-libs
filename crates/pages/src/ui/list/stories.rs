//! Stories of the page list.

use super::{PageList, PageListRow};
use craft_behavior::Selection;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

#[story(PageList<'static>, covers = [], drive = Drive::Never)]
fn artboards(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let rows = [PageListRow::new("Cover"), PageListRow::new("Spread").locked(true), PageListRow::new("Back")];
    let picked = data.value("picked", Selection::single(0_usize));
    PageList::new("Artboards", &rows, picked, 0).show(ui).into()
}
