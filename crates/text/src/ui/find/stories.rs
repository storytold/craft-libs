//! Stories of the find bar and the Find and Replace dialog.

use craft_ui::dialog::{DialogStack, StackDialog as _};
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

use super::{FindBar, FindCheck, FindReplaceDialog, FindTab, ReplaceButtons};
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

/// The Word layout of the dialog on `tab`, with "craft" in Find what.
fn word(tab: FindTab) -> FindReplaceDialog {
    FindReplaceDialog::new("craft", tab).word(true).go_to_kinds(vec!["Page".into(), "Line".into()])
}

/// The dialog `fresh` builds, while `open`. The dialog keeps what is typed in it until it closes; the card's Show
/// dialog opens a fresh one.
fn dialog(ui: &mut Ui, data: &mut StoryData, fresh: impl Fn() -> FindReplaceDialog) -> StoryOutput {
    if !*data.value("open", true) {
        return StoryOutput::default();
    }
    let dialog = data.note("dialog", fresh());
    let closed = dialog.show(ui, &DialogStack::default()).is_some();
    let request = dialog.take_request();
    if closed {
        *dialog = fresh();
        *data.value("open", true) = false;
    }
    let last = data.note("last", None::<String>);
    if let Some(request) = request {
        *last = Some(format!("{request:?}"));
    }
    let readout = last.as_ref().map_or_else(|| "No event yet".to_owned(), |event| format!("Last event: {event}"));
    StoryOutput::default().readout(readout)
}

#[story(FindReplaceDialog, covers = [], drive = Drive::Never, title = "Dialog")]
fn find_tab(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    dialog(ui, data, || word(FindTab::Find))
}

#[story(FindReplaceDialog, covers = [], drive = Drive::Never)]
fn replace_tab(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    dialog(ui, data, || word(FindTab::Replace))
}

#[story(FindReplaceDialog, covers = [], drive = Drive::Never)]
fn find_tab_with_checks(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let checks = || vec![FindCheck::new("backwards", "Search Backwards", false), FindCheck::new("hidden", "Check Hidden Layers", true)];
    dialog(ui, data, move || word(FindTab::Find).checks(checks()))
}

#[story(FindReplaceDialog, covers = [], drive = Drive::Never)]
fn replace_tab_separate_buttons(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    dialog(ui, data, || word(FindTab::Replace).replace_buttons(ReplaceButtons::Separate))
}

#[story(FindReplaceDialog, covers = [], drive = Drive::Never)]
fn powerpoint_layout(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    dialog(ui, data, || FindReplaceDialog::new("craft", FindTab::Find).word(false))
}
