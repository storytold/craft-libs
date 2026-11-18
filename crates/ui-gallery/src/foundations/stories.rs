//! Stories of the Foundations cards.

use super::{Colours, EditPolicy, PointerLooks, Surfaces};
use craft_behavior::EditPolicy as Policy;
use craft_ui::runtime::env::Env;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

// The colour roles of the story's own theme: the card shows the theme, mode and contrast it is drawn in.
#[story(Colours<'static>, covers = [], drive = Drive::Never, title = "Semantic")]
fn semantic(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let tokens = std::sync::Arc::clone(&Env::of(ui).tokens);
    Colours::new(&tokens).show(ui);
    StoryOutput::default()
}

// Each surface kind in the story's own theme, with the ink that reads on it.
#[story(Surfaces<'static>, covers = [], drive = Drive::Never, title = "Surface kinds")]
fn surface_kinds(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Surfaces::new().show(ui);
    StoryOutput::default()
}

// The painted pointers at three scales, as the window takes them (canvas-w3's card, cv.K29).
#[story(PointerLooks, covers = [], drive = Drive::Never, title = "Pointer looks")]
fn pointer_looks(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    PointerLooks::new().show(ui);
    StoryOutput::default()
}

#[story(EditPolicy, covers = [], drive = Drive::Never, group = "Event log", title = "Write-through")]
fn write_through(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    EditPolicy::new(Policy::WriteThrough).show(ui);
    StoryOutput::default()
}

#[story(EditPolicy, covers = [], drive = Drive::Never, group = "Event log", title = "On commit")]
fn on_commit(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    EditPolicy::new(Policy::OnCommit).show(ui);
    StoryOutput::default()
}
