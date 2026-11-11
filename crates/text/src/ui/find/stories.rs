//! Stories of the find bar and the Find and Replace dialog.

use craft_ui::dialog::{DialogStack, StackDialog as _};
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

use super::{FindBar, FindReplaceDialog, FindTab};
use crate::find::{FindMatches, FindOptions, FindPattern, MAX_COUNT};

fn matches(text: &str, query: &str) -> FindMatches {
    let mut found = FindMatches::new(0);
    if let Ok(pattern) = FindPattern::compile(query, &FindOptions::default()) {
        found.set_page(0, pattern.find_all(text, MAX_COUNT as usize));
    }
    found
}

#[story(FindBar<'static>, covers = [], drive = Drive::Never)]
fn with_matches(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mut query = data.value("query", "craft".to_owned()).clone();
    let mut options = FindOptions::default();
    let mut found = matches("craft makes craft apps with craft", &query);
    FindBar::new(&mut query, &mut options, &mut found).show(ui).response.into()
}

#[story(FindBar<'static>, covers = [], drive = Drive::Never)]
fn searching(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mut query = data.value("query", "craft".to_owned()).clone();
    let mut options = FindOptions::default();
    let mut found = FindMatches::new(0);
    FindBar::new(&mut query, &mut options, &mut found).progress(3, 12).show(ui).response.into()
}

#[story(FindBar<'static>, covers = [], drive = Drive::Never)]
fn refused_pattern(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mut query = data.value("query", "(craft".to_owned()).clone();
    let mut options = FindOptions { regex: true, ..FindOptions::default() };
    let mut found = FindMatches::new(0);
    FindBar::new(&mut query, &mut options, &mut found).show(ui).response.into()
}

fn dialog(ui: &mut Ui, data: &mut StoryData, tab: FindTab) -> StoryOutput {
    let dialog = data.value("dialog", ());
    let _ = dialog;
    let mut shown = FindReplaceDialog::new("craft", tab).word(true).go_to_kinds(vec!["Page".into(), "Line".into()]);
    let _ = shown.show(ui, &DialogStack::default());
    StoryOutput::default()
}

#[story(FindReplaceDialog, covers = [], drive = Drive::Never)]
fn find_tab(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    dialog(ui, data, FindTab::Find)
}

#[story(FindReplaceDialog, covers = [], drive = Drive::Never)]
fn replace_tab(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    dialog(ui, data, FindTab::Replace)
}
