//! Stories of `HyperlinkDialog`.

use super::{CurrentLink, HyperlinkDialog};
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

#[story(HyperlinkDialog, covers = [], drive = Drive::Never)]
fn hyperlink_dialog_new_web_link(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    data.value("web", HyperlinkDialog::new(CurrentLink::None, Some("word".to_owned()))).show(ui).into()
}

#[story(HyperlinkDialog, covers = [], drive = Drive::Never)]
fn hyperlink_dialog_place_in_the_document(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let dialog = HyperlinkDialog::new(CurrentLink::One(super::Hyperlink::new(super::LinkTarget::Place(1))), None)
        .places(vec!["1. Title".to_owned(), "2. Plan".to_owned()])
        .highlight_click(true);
    data.value("place", dialog).show(ui).into()
}
