//! Stories of the page thumbnails.

use super::{PageThumbnails, ThumbnailMarks, ThumbnailSection};
use crate::ui::fixtures::{DemoPages, PAPER};
use craft_behavior::Selection;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

/// Pages in the strip.
const PAGES: usize = 8;

#[story(PageThumbnails<'static>, covers = [], drive = Drive::Never)]
fn strip_with_sections_and_a_picked_range(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (picked, zoom) = data.value("state", (Selection::new(vec![2_usize, 3, 4], Some(4), Some(2)), 1.0_f64));
    let pages = DemoPages::new(PAGES);
    let sections = [
        ThumbnailSection { name: "Intro".to_owned(), first: 0, count: 3, open: true },
        ThumbnailSection { name: "Body".to_owned(), first: 3, count: 5, open: true },
    ];
    let marks = |page: usize| ThumbnailMarks { hidden: page == 5, animated: page == 1 };
    PageThumbnails::new("Pages", &pages, picked, 4, PAPER).size(zoom).sections(&sections).marks(&marks).new_page_row(true).show(ui).into()
}
