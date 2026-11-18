//! Stories of the page range field and options.

use super::{PageRangeChoices, PageRangeField, PageRangeOptions, PageRangeOptionsWords};
use crate::range::{PageRange, PageScope, Subset};
use craft_ui::contract::{Bind, Entry};
use craft_ui::patterns::form_dialogs::{DialogFields, ParamRows};
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
    let start = PageRange { scope: PageScope::Range, from: 3, to: Some(9), subset: Subset::Odd };
    options(ui, data, start, PageRangeChoices::Acrobat)
}

#[story(PageRangeOptions, covers = [], drive = Drive::Never)]
fn page_range_options_current(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let start = PageRange { scope: PageScope::Current, ..PageRange::default() };
    options(ui, data, start, PageRangeChoices::Current)
}

#[story(PageRangeOptions, covers = [], drive = Drive::Never)]
fn page_range_options_selection(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let start = PageRange { scope: PageScope::Selection, ..PageRange::default() };
    options(ui, data, start, PageRangeChoices::Selection)
}

#[story(PageRangeOptions, covers = [], drive = Drive::Never)]
fn page_range_options_from_to(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let start = PageRange { scope: PageScope::Range, from: 2, to: Some(5), subset: Subset::All };
    options(ui, data, start, PageRangeChoices::FromTo)
}

#[story(PageRangeOptions, covers = [], drive = Drive::Never)]
fn page_range_options_current_from_to(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    options(ui, data, PageRange::default(), PageRangeChoices::CurrentFromTo)
}

/// The page-range row as a `ParamDialog` shows it, among the dialog's rows, keeping its range in the dialog's fields.
#[story(PageRangeOptions, covers = [], drive = Drive::Never)]
fn page_range_row(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let params = [PageRangeOptions::param("pages", "Page range", PAGES, CURRENT, PageRangeChoices::Current)];
    let fields = data.value("fields", DialogFields::new());
    StoryOutput::from(ParamRows::new("Options", &params, Bind::bound(fields)).show(ui))
}

/// The page the stories' Current page names (0-based).
const CURRENT: usize = 3;

/// The options over the story's range, opening on `start`, in the arrangement `choices`.
fn options(ui: &mut Ui, data: &mut StoryData, start: PageRange, choices: PageRangeChoices) -> StoryOutput {
    let range = *data.value("range", start);
    let out = PageRangeOptions::new("Page range", range, PAGES).choices(choices).current(CURRENT).show(ui);
    if let Some(next) = out.changed {
        *data.value("range", start) = next;
    }
    StoryOutput::from(out)
}
