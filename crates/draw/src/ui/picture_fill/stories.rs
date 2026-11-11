//! Stories of the picture fill editor: Stretch and Tile.

use super::{PictureFill, PictureFillEditor, PictureMode, TileAlignment, TileMirror};
use craft_ui::contract::Bind;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

#[story(PictureFillEditor<'static>, covers = [], drive = Drive::Never)]
fn picture_fill_stretch(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let fill = data.value("stretch", PictureFill::new("sunrise.png"));
    PictureFillEditor::new("Picture", Bind::bound(fill)).show(ui).into()
}

#[story(PictureFillEditor<'static>, covers = [], drive = Drive::Never)]
fn picture_fill_tile(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mut start = PictureFill::new("sunrise.png");
    start.mode = PictureMode::Tile { offset: [12.0, 0.0], scale: [50.0, 50.0], alignment: TileAlignment::Center, mirror: TileMirror::Horizontal };
    let fill = data.value("tile", start);
    PictureFillEditor::new("Picture", Bind::bound(fill)).show(ui).into()
}
