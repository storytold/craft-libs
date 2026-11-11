//! Stories of `CheckSpellingDialog`.

use super::{CheckSpellingDialog, SpellingItem};
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

fn items() -> Vec<SpellingItem> {
    vec![
        SpellingItem::new(1, 0..3, "teh", vec!["the".to_owned(), "tea".to_owned()]),
        SpellingItem::new(1, 4..11, "recieve", vec!["receive".to_owned()]),
    ]
}

#[story(CheckSpellingDialog, covers = [], drive = Drive::Never)]
fn check_spelling_with_suggestions(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    data.value("suggestions", CheckSpellingDialog::new(items())).show(ui).into()
}

#[story(CheckSpellingDialog, covers = [], drive = Drive::Never)]
fn check_spelling_complete(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    data.value("complete", CheckSpellingDialog::new(Vec::new())).show(ui).into()
}
