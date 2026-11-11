//! Stories of the page list.

use super::{PageList, PageListRow};
use craft_behavior::Selection;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

#[story(PageList<'static>, covers = [], drive = Drive::Never)]
fn artboards(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let rows = [
        PageListRow { name: "Cover".to_owned(), locked: false, hidden: false },
        PageListRow { name: "Spread".to_owned(), locked: true, hidden: false },
        PageListRow { name: "Back".to_owned(), locked: false, hidden: true },
    ];
    let picked = data.value("picked", Selection::single(0_usize));
    PageList::new("Artboards", &rows, picked, 0).show(ui).into()
}
