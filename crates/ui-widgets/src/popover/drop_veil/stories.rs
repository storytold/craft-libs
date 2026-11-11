//! Stories of `DropVeil`.

use super::DropVeil;
use crate::choice::Checkbox;
use crate::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use crate::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

#[story(DropVeil, covers = [Default], drive = Drive::Never)]
fn drop_veil(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    DropVeil::new("Import 3 photos")
        .show(ui, |ui, _| {
            Checkbox::new(Bind::given(true)).label("Keep originals").show(ui);
        })
        .output
        .response
        .into()
}
