//! Stories of `PreferencesWindow`.

use super::{PreferencesState, PreferencesWindow};
use crate::contract::{Bind, Entry};
use crate::patterns::preferences::{PageSpec, PrefValue, PreferencesModel, RowKind, RowSpec};
use crate::runtime::env::UnitKind;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use craft_theme::Rgba;
use egui::{Context, Ui};

/// The settings the stories edit.
#[derive(Clone, Debug, PartialEq)]
struct Values {
    snap: bool,
    language: String,
    name: String,
    size: i64,
    opacity: f64,
    tint: Rgba,
}

impl Default for Values {
    fn default() -> Self {
        Values { snap: true, language: "en".to_owned(), name: "Poster".to_owned(), size: 24, opacity: 0.5, tint: Rgba([0xff, 0x88, 0x00, 0xff]) }
    }
}

/// A host model over a saved copy and a draft of the settings.
#[derive(Clone, Debug, Default, PartialEq)]
struct Demo {
    saved: Values,
    draft: Values,
    refuse: bool,
}

impl PreferencesModel for Demo {
    fn pages(&self) -> Vec<PageSpec<'_>> {
        let draft = &self.draft;
        let languages = vec![Entry::new("en", "English"), Entry::new("fr", "French")];
        vec![
            PageSpec::new(
                "general",
                "General",
                vec![
                    RowSpec::new("general.snap", "Snap to grid", RowKind::Bool(draft.snap)),
                    RowSpec::new("general.language", "Language", RowKind::choice(&draft.language, languages)).help("Applies at next launch."),
                    RowSpec::new("general.name", "Name", RowKind::Text(&draft.name)),
                ],
            ),
            PageSpec::new(
                "canvas",
                "Canvas",
                vec![
                    RowSpec::new("canvas.size", "Brush size", RowKind::int(draft.size, 1..=64).counting(UnitKind::Pixels)),
                    RowSpec::new("canvas.opacity", "Opacity", RowKind::float(draft.opacity, 0.0..=1.0, 2)),
                    RowSpec::new("canvas.tint", "Tint", RowKind::Color(draft.tint)),
                ],
            )
            .footnote("Canvas settings apply to new documents."),
            PageSpec::new("plugins", "Plug-ins", Vec::new()),
        ]
    }

    fn set(&mut self, key: &str, value: PrefValue) {
        match (key, value) {
            ("general.snap", PrefValue::Bool(on)) => self.draft.snap = on,
            ("general.language", PrefValue::Choice(id)) => self.draft.language = id,
            ("general.name", PrefValue::Text(text)) => self.draft.name = text,
            ("canvas.size", PrefValue::Int(size)) => self.draft.size = size,
            ("canvas.opacity", PrefValue::Float(opacity)) => self.draft.opacity = opacity,
            ("canvas.tint", PrefValue::Color(tint)) => self.draft.tint = tint,
            _ => {}
        }
    }

    fn custom_row(&mut self, _: &mut Ui, _: &str) {}

    fn custom_page(&mut self, _: &mut Ui, _: &str) {}

    fn differs(&self) -> bool {
        self.draft != self.saved
    }

    fn apply(&mut self) -> Result<(), String> {
        if self.refuse {
            return Err("The brush size can't be applied.".to_owned());
        }
        self.saved = self.draft.clone();
        Ok(())
    }

    fn cancel(&mut self) {
        self.draft = self.saved.clone();
    }

    fn reset_page(&mut self, page: &str) {
        let defaults = Values::default();
        match page {
            "general" => (self.draft.snap, self.draft.language, self.draft.name) = (defaults.snap, defaults.language, defaults.name),
            "canvas" => (self.draft.size, self.draft.opacity, self.draft.tint) = (defaults.size, defaults.opacity, defaults.tint),
            _ => {}
        }
    }

    fn nested(&mut self, _: &Context) {}
}

fn window(ui: &mut Ui, data: &mut StoryData, state: PreferencesState, demo: Demo) -> StoryOutput {
    let mut demo = data.value("demo", demo).clone();
    let state = data.value("state", state);
    let shown = PreferencesWindow::new(Bind::bound(state)).embedded().show(ui, &mut demo).into();
    *data.value("demo", Demo::default()) = demo;
    shown
}

#[story(PreferencesWindow<'static>, covers = [], drive = Drive::Never)]
fn preferences_window(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    window(ui, data, PreferencesState::default(), Demo::default())
}

#[story(PreferencesWindow<'static>, covers = [], drive = Drive::Never)]
fn preferences_window_without_matches(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let state = PreferencesState { search: "zzz".to_owned(), ..PreferencesState::default() };
    window(ui, data, state, Demo::default())
}

#[story(PreferencesWindow<'static>, covers = [], drive = Drive::Never)]
fn preferences_window_with_error(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let state =
        PreferencesState { selected: "canvas".to_owned(), error: Some("The brush size can't be applied.".to_owned()), ..PreferencesState::default() };
    window(ui, data, state, Demo { refuse: true, ..Demo::default() })
}
