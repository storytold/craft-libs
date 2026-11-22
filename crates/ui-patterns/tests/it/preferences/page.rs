//! Preferences is one scrolling page of cards: every page is shown at once, short choices are segmented and the
//! search counts its matches.

use craft_ui::contract::{Bind, Entry};
use craft_ui::patterns::preferences::{PageSpec, PrefValue, PreferencesModel, PreferencesState, PreferencesWindow, RowKind, RowSpec};
use craft_ui::testing::{Fixture, trial};
use egui::Ui;

struct Model {
    side: String,
}

impl PreferencesModel for Model {
    fn pages(&self) -> Vec<PageSpec<'_>> {
        let sides = vec![Entry::new("left", "Left"), Entry::new("right", "Right")];
        vec![
            PageSpec::new("general", "General", vec![RowSpec::new("side", "Tools side", RowKind::choice(&self.side, sides))]),
            PageSpec::new("canvas", "Canvas", vec![RowSpec::new("snap", "Snap", RowKind::Bool(true))]),
        ]
    }
    fn set(&mut self, _: &str, value: PrefValue) {
        if let PrefValue::Choice(side) = value {
            self.side = side;
        }
    }
    fn custom_row(&mut self, _: &mut Ui, _: &str) -> bool {
        false
    }
    fn custom_page(&mut self, _: &mut Ui, _: &str) -> bool {
        false
    }
    fn differs(&self) -> bool {
        false
    }
    fn apply(&mut self) -> Result<(), String> {
        Ok(())
    }
    fn cancel(&mut self) {}
    fn reset_page(&mut self, _: &str) {}
}

fn fixture() -> Fixture {
    let mut model = Model { side: "left".into() };
    let mut state = PreferencesState::default();
    Fixture::new(move |ui| {
        let _ = PreferencesWindow::new(Bind::bound(&mut state)).embedded().show(ui, &mut model);
    })
}

#[trial]
fn every_page_is_a_card_on_one_scrolling_page() {
    let mut fixture = fixture();
    fixture.room(egui::vec2(700.0, 900.0)).settle();
    assert!(fixture.by_label("General").is_some() && fixture.by_label("Canvas").is_some(), "both cards show at once");
}

#[trial]
fn a_short_choice_is_a_row_of_segments() {
    let mut fixture = fixture();
    fixture.room(egui::vec2(700.0, 900.0)).settle();
    let right = fixture.by_label("Right").expect("a segment for each option");
    fixture.click(&right).settle();
    assert!(fixture.texts().iter().any(|t| t.text == "Left"), "the other option stays shown beside it");
}

#[trial]
fn the_search_shows_how_many_settings_match() {
    let mut fixture = fixture();
    fixture.room(egui::vec2(700.0, 900.0)).settle();
    assert!(!fixture.texts().iter().any(|t| t.text.contains("match")), "no count without a search");
    let search = fixture.by_label("Search settings").expect("the search field");
    fixture.click(&search).type_text("snap").settle();
    assert!(fixture.texts().iter().any(|t| t.text == "1 match"), "{:?}", fixture.texts().iter().map(|t| &t.text).collect::<Vec<_>>());
}
