//! Stories of `NewDocumentDialog`.

use super::NewDocumentDialog;
use crate::contract::Bind;
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
