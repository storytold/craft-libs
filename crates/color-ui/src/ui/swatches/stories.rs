//! Stories of `SwatchesPanel`.

use super::{SwatchLibrary, SwatchesPanel, SwatchesState};
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

/// The panel over a library and state kept in the story's data; the reported edits run on the library, so the story stays
/// interactive.
fn interactive(ui: &mut Ui, data: &mut StoryData, name: &'static str, start: SwatchLibrary) -> StoryOutput {
    let ((library, state), library_state_at) = data.value_at(name, (start, SwatchesState::default()));
    let out = SwatchesPanel::new(&library.groups, library_state_at.bind(state), craft_theme::Srgba::rgb(0.2, 0.4, 0.6)).show(ui);
    for edit in &out.edits {
        library.apply(edit);
    }
    out.into()
}

#[story(SwatchesPanel<'static>, covers = [], drive = Drive::Never)]
fn swatches_panel(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    interactive(ui, data, "panel", SwatchLibrary::default())
}

#[story(SwatchesPanel<'static>, covers = [], drive = Drive::Never)]
fn swatches_panel_empty(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    interactive(ui, data, "empty", SwatchLibrary { groups: Vec::new() })
}
