//! Stories of `HyperlinkDialog`.

use super::{CurrentLink, HyperlinkDialog, HyperlinkParts, LinkPlace, LinkPlaceGroup};
use craft_ui::dialog::stories::hosted;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

#[story(HyperlinkDialog, covers = [], drive = Drive::Never)]
fn hyperlink_dialog_new_web_link(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    data.value("web", HyperlinkDialog::new(CurrentLink::None, Some("word".to_owned()))).show(ui).into()
}

#[story(HyperlinkDialog, covers = [], drive = Drive::Never)]
fn hyperlink_dialog_place_in_the_document(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let dialog = HyperlinkDialog::new(CurrentLink::One(super::Hyperlink::new(super::LinkTarget::Place(1))), None)
        .place_groups(vec![
            LinkPlaceGroup::new("Headings", vec![LinkPlace::new("1. Title", 0), LinkPlace::new("2. Plan", 0), LinkPlace::new("2.1 Scope", 1)]),
            LinkPlaceGroup::new("Bookmarks", vec![LinkPlace::new("fig1", 0)]),
        ])
        .highlight_click(true);
    data.value("place", dialog).show(ui).into()
}

#[story(HyperlinkDialog, covers = [], drive = Drive::Never)]
fn hyperlink_dialog_no_screen_tip(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    hosted(ui, data, (), || HyperlinkDialog::new(CurrentLink::None, Some("word".to_owned())).parts(HyperlinkParts::NoScreenTip))
}
