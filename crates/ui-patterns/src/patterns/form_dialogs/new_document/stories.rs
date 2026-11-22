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
    dialog.opened(data.bind("open", true)).show(ui).into()
}

#[story(NewDocumentDialog<'static>, covers = [], drive = Drive::Never)]
fn new_document_dialog_with_app_presets(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let presets = (
        PresetLibrary::<DocumentOptions>::new(),
        vec![DocumentOptions { name: "Poster".to_owned(), ..DocumentOptions::default() }],
        "recent".to_owned(),
    );
    let (((library, recent, tab), presets_at), (open, open_at)) = data.pair_at(("presets", presets), ("open", true));
    NewDocumentDialog::new(Bind::kept())
        .saved(presets_at.bind(library))
        .recent(presets_at.bind(recent))
        .tab(presets_at.bind(tab))
        .opened(open_at.bind(open))
        .show(ui)
        .into()
}
