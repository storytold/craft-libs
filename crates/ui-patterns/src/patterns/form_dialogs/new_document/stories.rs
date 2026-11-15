//! Stories of `NewDocumentDialog`.

use super::{DocumentOptions, NewDocumentDialog};
use crate::contract::Bind;
use crate::patterns::presets::PresetLibrary;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use craft_behavior::PixelSize;
use egui::Ui;

#[story(NewDocumentDialog<'static>, covers = [], drive = Drive::Never)]
fn new_document_dialog(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let clipboard = *data.value("clipboard_image", false);
    let mut dialog = NewDocumentDialog::new(Bind::kept());
    if clipboard {
        dialog = dialog.clipboard(PixelSize::new(1920, 1080));
    }
    dialog.opened(Bind::bound(data.value("open", true))).show(ui).into()
}

#[story(NewDocumentDialog<'static>, covers = [], drive = Drive::Never)]
fn new_document_dialog_with_app_presets(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mut library = data.value("library", PresetLibrary::<DocumentOptions>::new()).clone();
    let mut recent = data.value("recent", vec![DocumentOptions { name: "Poster".to_owned(), ..DocumentOptions::default() }]).clone();
    let mut tab = data.value("tab", "recent".to_owned()).clone();
    let output = NewDocumentDialog::new(Bind::kept())
        .saved(Bind::bound(&mut library))
        .recent(Bind::bound(&mut recent))
        .tab(Bind::bound(&mut tab))
        .opened(Bind::bound(data.value("open", true)))
        .show(ui);
    *data.value("library", PresetLibrary::<DocumentOptions>::new()) = library;
    *data.value("tab", "recent".to_owned()) = tab;
    output.into()
}
