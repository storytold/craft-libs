//! Stories of the page thumbnails.

use super::{PageThumbnails, ThumbnailArrangement, ThumbnailMarks, ThumbnailSection};
use crate::ui::fixtures::{DemoPages, PAPER};
use craft_behavior::{IconId, Selection};
use craft_ui::runtime::commands::CommandRef;
use craft_ui::runtime::env::Label;
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

/// Acrobat's Pages panel: the picked pages beside the current one, which a plain click empties.
#[story(PageThumbnails<'static>, covers = [], drive = Drive::Never)]
fn strip_with_picks_beside_the_current_page(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (picked, zoom) = data.value("state", (Selection::new(vec![1_usize, 5], Some(5), Some(1)), 1.0_f64));
    let pages = DemoPages::new(PAGES);
    PageThumbnails::new("Pages", &pages, picked, 3, PAPER).size(zoom).picks_beside_current(true).show(ui).into()
}

/// The actions in each page's band in the Organize stories.
fn organize_actions() -> [(IconId, CommandRef); 2] {
    [(IconId::new("rotate_cw"), CommandRef::new("pages.rotate")), (IconId::new("trash"), CommandRef::new("pages.delete"))]
}

/// The name of the insert gap before page `n` in the Organize stories.
fn insert_before(n: usize) -> Label {
    Label::from(format!("Insert before page {}", n + 1))
}

#[story(PageThumbnails<'static>, covers = [], drive = Drive::Never)]
fn organize_editable(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (picked, zoom) = data.value("state", (Selection::new(vec![1_usize, 2], Some(2), Some(1)), 1.0_f64));
    let pages = DemoPages::new(PAGES);
    let actions = organize_actions();
    PageThumbnails::new("Pages", &pages, picked, 2, PAPER)
        .arrangement(ThumbnailArrangement::Organize)
        .size(zoom)
        .actions(&actions)
        .insert_label(&insert_before)
        .show(ui)
        .into()
}

#[story(PageThumbnails<'static>, covers = [], drive = Drive::Never)]
fn organize_read_only(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (picked, zoom) = data.value("state", (Selection::new(vec![3_usize], Some(3), Some(3)), 1.0_f64));
    let pages = DemoPages::new(PAGES);
    let actions = organize_actions();
    PageThumbnails::new("Pages", &pages, picked, 3, PAPER)
        .arrangement(ThumbnailArrangement::Organize)
        .editable(false)
        .size(zoom)
        .actions(&actions)
        .insert_label(&insert_before)
        .show(ui)
        .into()
}

#[story(PageThumbnails<'static>, covers = [], drive = Drive::Never)]
fn organize_one_page(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (picked, zoom) = data.value("state", (Selection::<usize>::default(), 1.0_f64));
    let pages = DemoPages::new(1);
    let actions = organize_actions();
    PageThumbnails::new("Pages", &pages, picked, 0, PAPER)
        .arrangement(ThumbnailArrangement::Organize)
        .size(zoom)
        .actions(&actions)
        .insert_label(&insert_before)
        .show(ui)
        .into()
}
