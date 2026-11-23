//! Stories of the page range field and options.

use super::{PageRangeField, PageRangeOptions, PageRangeOptionsWords};
use crate::range::{PageRange, Subset};
use craft_ui::contract::Entry;
use craft_ui::runtime::env::Env;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

/// The pages of the document the stories show.
const PAGES: usize = 12;

#[story(PageRangeField<'static>, covers = [], drive = Drive::Never)]
fn page_range_field(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let pages = data.value("pages", vec![0usize, 1, 2, 4]).clone();
    let words = Env::of(ui).words::<PageRangeOptionsWords>();
    let menu = [
        Entry::new((0..PAGES).collect::<Vec<usize>>(), words.all_pages.clone()),
        Entry::new((0..PAGES).step_by(2).collect::<Vec<usize>>(), words.odd_only.clone()),
        Entry::new((1..PAGES).step_by(2).collect::<Vec<usize>>(), words.even_only.clone()),
    ];
    let out = PageRangeField::new("Selected pages", &pages, PAGES, 0).menu(&menu).show(ui);
    if let Some(picked) = out.picked.clone() {
        *data.value("pages", vec![0usize, 1, 2, 4]) = picked;
    }
    StoryOutput::from(out)
}

#[story(PageRangeOptions, covers = [], drive = Drive::Never)]
fn page_range_options(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let start = PageRange { all: false, from: 3, to: Some(9), subset: Subset::Odd };
    let range = *data.value("range", start);
    let out = PageRangeOptions::new("Page range", range, PAGES).show(ui);
    if let Some(next) = out.changed {
        *data.value("range", start) = next;
    }
    StoryOutput::from(out)
}
