//! Stories of the appearance page and the theme chip.

use super::{AppearancePage, ThemeChip};
use crate::{SelectionChange, SuiteClient};
use craft_theme::{ThemeId, builtins};
use craft_ui::choice::RadioGroup;
use craft_ui::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use craft_ui::layout_components::Row;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

/// The favourites of the page stories: every third built-in theme after the first.
fn favorites() -> Vec<ThemeId> {
    builtins().iter().skip(1).step_by(3).map(|theme| theme.id.clone()).collect()
}

#[story(AppearancePage<'static>, covers = [Default], drive = Drive::Never)]
fn appearance_page(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let client = SuiteClient::memory();
    let favorites = favorites();
    AppearancePage::new(&client, Bind::bound(data.value("draft", SelectionChange::default()))).favorites(&favorites).show(ui).into()
}

#[story(AppearancePage<'static>, covers = [], drive = Drive::Never)]
fn appearance_page_without_text_size(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let client = SuiteClient::memory();
    AppearancePage::new(&client, Bind::bound(data.value("plain", SelectionChange::default()))).text_size(false).system_dark(true).show(ui).into()
}

/// One theme's chips in a group: plain, chosen, favourite, and both. A click on a star toggles the chip's favourite.
#[story(ThemeChip<'static>, covers = [Default], drive = Drive::Never)]
fn theme_chips(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let themes = builtins();
    let mut favorites = data.value("favorites", themes.iter().take(4).skip(2).map(|theme| theme.id.clone()).collect::<Vec<_>>()).clone();
    let mut flipped = None;
    let chosen = data.value("chosen", Some(themes[1].id.clone()));
    let shown = RadioGroup::new("Light Theme", Bind::bound(chosen)).show(ui, |ui, _| {
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
