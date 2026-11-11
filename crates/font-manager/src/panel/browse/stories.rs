//! Stories of `FamilyCard` in its four states, and of the card list.

use craft_ui::contract::ShowCollection as _;
use craft_ui::testing::{Drive, StoryData, StoryOutput, show_as_only_item, story};
use egui::Ui;

use super::{CardData, CardKeys, CardState, FamilyCard, FamilyCards};
use crate::panel::jobs::Kind;

const SAMPLE: &str = "The quick brown fox jumps over the lazy dog";

fn data(name: &str, state: CardState) -> CardData {
    CardData {
        name: name.to_owned(),
        meta: "4 styles · Sans serif · Google Fonts".to_owned(),
        sample: SAMPLE.to_owned(),
        points: 36.0,
        curated: true,
        state,
        face: None,
    }
}

#[story(FamilyCard, covers = [Default], drive = Drive::Never)]
fn family_card_not_installed(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    show_as_only_item(ui, FamilyCard::new(data("Hack", CardState::Offer { preview: true }))).into()
}

#[story(FamilyCard, covers = [], drive = Drive::Never)]
fn family_card_installing(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    show_as_only_item(ui, FamilyCard::new(data("Hack", CardState::Running(Kind::Install, Some(0.4))))).into()
}

#[story(FamilyCard, covers = [], drive = Drive::Never)]
fn family_card_failed(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    show_as_only_item(ui, FamilyCard::new(data("Hack", CardState::Failed(Kind::Install, "network unreachable".to_owned(), true)))).into()
}

#[story(FamilyCard, covers = [], drive = Drive::Never)]
fn family_card_installed(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    show_as_only_item(ui, FamilyCard::new(data("Hack", CardState::Installed))).into()
}

#[story(FamilyCard, covers = [], drive = Drive::Never)]
fn family_card_not_curated(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let mut card = data("Hack", CardState::Offer { preview: false });
    card.curated = false;
    show_as_only_item(ui, FamilyCard::new(card)).into()
}

fn cards(ui: &mut Ui, keys: &[usize]) -> StoryOutput {
    FamilyCards::new("Families")
        .show_items(ui, &CardKeys::new(keys, |_| None), &mut |key, _| {
            let state = if key % 2 == 0 { CardState::Installed } else { CardState::Offer { preview: true } };
            FamilyCard::new(data(&format!("Family {key}"), state))
        })
        .into()
}

#[story(FamilyCards, covers = [Empty], drive = Drive::Never)]
fn family_cards_empty(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    cards(ui, &[])
}

#[story(FamilyCards, covers = [One], drive = Drive::Never)]
fn family_cards_one(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    cards(ui, &[1])
}

#[story(FamilyCards, covers = [Default, Many], drive = Drive::Never)]
fn family_cards_many(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    cards(ui, &[0, 1, 2, 3, 4, 5])
}
