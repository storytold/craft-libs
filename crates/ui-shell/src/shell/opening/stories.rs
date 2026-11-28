//! Stories of `OpeningCard`.

use super::OpeningCard;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

#[story(OpeningCard, covers = [], drive = Drive::Never)]
fn opening_card(ui: &mut Ui, _data: &mut StoryData) -> StoryOutput {
    OpeningCard::new("Poster.psd").message("Reading layers…").fraction(Some(0.42)).cancel("Cancel").show(ui).into()
}

#[story(OpeningCard, covers = [], drive = Drive::Never)]
fn opening_card_with_a_long_name(ui: &mut Ui, _data: &mut StoryData) -> StoryOutput {
    OpeningCard::new("a7rv-test_2024_02_17_shot_30_3705_with_an_even_longer_suffix_that_never_ends.psb")
        .message("Reading layers…")
        .fraction(Some(0.75))
        .cancel("Cancel")
        .show(ui)
        .into()
}

#[story(OpeningCard, covers = [], drive = Drive::Never)]
fn opening_card_without_a_fraction(ui: &mut Ui, _data: &mut StoryData) -> StoryOutput {
    OpeningCard::new("Poster.psd").message("Reading…").cancel("Cancel").show(ui).into()
}

#[story(OpeningCard, covers = [], drive = Drive::Never)]
fn opening_card_without_cancel(ui: &mut Ui, _data: &mut StoryData) -> StoryOutput {
    OpeningCard::new("Poster.psd").fraction(Some(0.9)).show(ui).into()
}
