//! Stories of the appearance page and the theme chip.

use super::{AppearancePage, ThemeChip};
use crate::{SelectionChange, SuiteClient};
use craft_theme::{ThemeId, builtins};
use craft_ui::choice::RadioGroup;
use craft_ui::contract::{Props as _, Show as _, ShowContainer as _};
use craft_ui::form::FieldGroup;
use craft_ui::layout_components::Row;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

/// The favourites of the page stories: every third built-in theme after the first.
fn favorites() -> Vec<ThemeId> {
    builtins().iter().skip(1).step_by(3).map(|theme| theme.id.clone()).collect()
}

#[story(AppearancePage<'static>, covers = [Default], drive = Drive::Never)]
fn appearance_page(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let client = SuiteClient::memory().view();
    let favorites = favorites();
    let mut draft = data.bind("draft", SelectionChange::default());
    FieldGroup::new("Appearance").show(ui, |ui, _| AppearancePage::new(&client, draft.reborrowed()).favorites(&favorites).show(ui)).output.into()
}

#[story(AppearancePage<'static>, covers = [], drive = Drive::Never)]
fn appearance_page_without_text_size(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let client = SuiteClient::memory().view();
    let mut draft = data.bind("plain", SelectionChange::default());
    FieldGroup::new("Appearance")
        .show(ui, |ui, _| AppearancePage::new(&client, draft.reborrowed()).text_size(false).system_dark(true).show(ui))
        .output
        .into()
}

/// The page of an app that keeps its own appearance: the mode, the two theme rows without favourite stars, and Motion.
#[story(AppearancePage<'static>, covers = [], drive = Drive::Never)]
fn appearance_page_with_kept_rows_only(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let client = SuiteClient::memory().view();
    let draft = data.bind("kept", SelectionChange::default());
    AppearancePage::new(&client, draft).can_favorite(false).size(false).text_size(false).contrast(false).show(ui).into()
}

/// Chips that offer no favourite star, for an app that keeps no favourites.
#[story(ThemeChip<'static>, covers = [], drive = Drive::Never)]
fn theme_chips_without_favorites(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let themes = builtins();
    RadioGroup::new("Light Theme", data.bind("chosen", Some(themes[1].id.clone())))
        .show(ui, |ui, _| {
            Row::new().show(ui, |ui, _| {
                for theme in themes.iter().take(4) {
                    ThemeChip::new(theme, false).can_favorite(false).id_salt(theme.id.as_str()).show(ui);
                }
            });
        })
        .into()
}

/// One theme's chips in a group: plain, chosen, favourite, and both. A click on a star toggles the chip's favourite.
#[story(ThemeChip<'static>, covers = [Default], drive = Drive::Never)]
fn theme_chips(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let themes = builtins();
    let mut favorites = data.value("favorites", themes.iter().take(4).skip(2).map(|theme| theme.id.clone()).collect::<Vec<_>>()).clone();
    let mut flipped = None;
    let (chosen, chosen_at) = data.value_at("chosen", Some(themes[1].id.clone()));
    let shown = RadioGroup::new("Light Theme", chosen_at.bind(chosen)).show(ui, |ui, _| {
        Row::new().show(ui, |ui, _| {
            for theme in themes.iter().take(4) {
                let chip = ThemeChip::new(theme, false).favorite(favorites.contains(&theme.id)).id_salt(theme.id.as_str()).show(ui);
                if chip.favorite_clicked {
                    flipped = Some(theme.id.clone());
                }
            }
        });
    });
    if let Some(id) = flipped {
        match favorites.iter().position(|held| *held == id) {
            Some(at) => {
                favorites.remove(at);
            }
            None => favorites.push(id),
        }
        *data.value("favorites", Vec::new()) = favorites;
    }
    shown.into()
}
