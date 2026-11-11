//! Stories of `SwatchesPanel`.

use super::{SwatchLibrary, SwatchesPanel, SwatchesState};
use craft_ui::contract::Bind;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

#[story(SwatchesPanel<'static>, covers = [], drive = Drive::Never)]
fn swatches_panel(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (library, state) = data.value("panel", (SwatchLibrary::default(), SwatchesState::default()));
    SwatchesPanel::new(Bind::bound(library), Bind::bound(state), craft_theme::Srgba::rgb(0.2, 0.4, 0.6)).show(ui).into()
}

#[story(SwatchesPanel<'static>, covers = [], drive = Drive::Never)]
fn swatches_panel_empty(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (library, state) = data.value("empty", (SwatchLibrary { groups: Vec::new() }, SwatchesState::default()));
    SwatchesPanel::new(Bind::bound(library), Bind::bound(state), craft_theme::Srgba::rgb(0.2, 0.4, 0.6)).show(ui).into()
}
