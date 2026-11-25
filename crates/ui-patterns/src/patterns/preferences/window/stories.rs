//! Stories of `PreferencesWindow`.

use super::{PreferencesLayout, PreferencesMode, PreferencesState, PreferencesWindow};
use crate::buttons::Button;
use crate::contract::{Bind, Entry, Show as _, clicked};
use crate::patterns::preferences::{GroupSpec, PageSpec, PrefValue, PreferencesDraft, PreferencesModel, RowKind, RowSpec};
use crate::runtime::env::{LengthUnits, UnitKind};
use crate::testing::{Drive, StoryData, StoryOutput, story};
use craft_theme::Rgba;
use egui::Ui;

/// The settings the stories edit.
#[derive(Clone, Debug, PartialEq)]
struct Values {
    snap: bool,
    language: String,
    name: String,
    size: i64,
    opacity: f64,
    tint: Rgba,
    /// In points, shown in millimetres.
    margin: f64,
    /// In seconds.
    delay: f64,
    load_plugins: bool,
    plugin_folder: String,
}

impl Default for Values {
    fn default() -> Self {
        Values {
            snap: true,
            language: "en".to_owned(),
            name: "Poster".to_owned(),
            size: 24,
            opacity: 0.5,
            tint: Rgba([0xff, 0x88, 0x00, 0xff]),
            margin: 36.0,
            delay: 0.5,
            load_plugins: true,
            plugin_folder: "Plug-ins".to_owned(),
        }
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
                    RowSpec::new(
                        "canvas.margin",
                        "Margin",
                        RowKind::length(draft.margin, 0.0..=720.0, LengthUnits::new(UnitKind::Points, UnitKind::Millimetres, 72.0)),
                    ),
                    RowSpec::new("canvas.delay", "Preview delay", RowKind::float(draft.delay, 0.0..=10.0, 1).unit_text("s")),
                ],
            )
            .footnote("Canvas settings apply to new documents."),
            // The host draws the page's own part (nothing here); the group under it is drawn and searched as rows.
            PageSpec::custom("plugins", "Plug-ins").group(GroupSpec::new(
                Some("Loading"),
                vec![
                    RowSpec::new("plugins.load", "Load plug-ins at launch", RowKind::Bool(draft.load_plugins)),
                    RowSpec::new("plugins.folder", "Plug-in folder", RowKind::Text(&draft.plugin_folder)),
                ],
            )),
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
            ("canvas.margin", PrefValue::Float(margin)) => self.draft.margin = margin,
            ("canvas.delay", PrefValue::Float(delay)) => self.draft.delay = delay,
            ("plugins.load", PrefValue::Bool(on)) => self.draft.load_plugins = on,
            ("plugins.folder", PrefValue::Text(text)) => self.draft.plugin_folder = text,
            _ => {}
        }
    }

    fn custom_row(&mut self, _: &mut Ui, _: &str) -> bool {
        false
    }

    fn custom_page(&mut self, _: &mut Ui, _: &str) -> bool {
        false
    }

    fn reset_page(&mut self, page: &str) {
        let defaults = Values::default();
        match page {
            "general" => (self.draft.snap, self.draft.language, self.draft.name) = (defaults.snap, defaults.language, defaults.name),
            "canvas" => {
                (self.draft.size, self.draft.opacity, self.draft.tint) = (defaults.size, defaults.opacity, defaults.tint);
                (self.draft.margin, self.draft.delay) = (defaults.margin, defaults.delay);
            }
            "plugins" => (self.draft.load_plugins, self.draft.plugin_folder) = (defaults.load_plugins, defaults.plugin_folder),
            _ => {}
        }
    }

    fn nested(&mut self, _: &mut Ui) {}
}

impl PreferencesDraft for Demo {
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
}

fn window(ui: &mut Ui, data: &mut StoryData, state: PreferencesState, demo: Demo) -> StoryOutput {
    laid_out(ui, data, state, demo, PreferencesLayout::Paged)
}

/// The window laid out as `layout` says.
fn laid_out(ui: &mut Ui, data: &mut StoryData, state: PreferencesState, demo: Demo, layout: PreferencesLayout) -> StoryOutput {
    let mut demo = data.value("demo", demo).clone();
    let (state, state_at) = data.value_at("state", state);
    let shown = PreferencesWindow::new(state_at.bind(state)).layout(layout).embedded().show(ui, &mut demo).into();
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

/// The custom Plug-ins page: the host's part, then the page's own titled group.
#[story(PreferencesWindow<'static>, covers = [], drive = Drive::Never)]
fn preferences_window_custom_page(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let state = PreferencesState { selected: "plugins".to_owned(), ..PreferencesState::default() };
    window(ui, data, state, Demo::default())
}

#[story(PreferencesWindow<'static>, covers = [], drive = Drive::Never)]
fn preferences_window_with_error(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let state =
        PreferencesState { selected: "canvas".to_owned(), error: Some("The brush size can't be applied.".to_owned()), ..PreferencesState::default() };
    window(ui, data, state, Demo { refuse: true, ..Demo::default() })
}

/// The cards layout: one scrolling column of cards, short choices as segments.
#[story(PreferencesWindow<'static>, covers = [], drive = Drive::Never)]
fn preferences_window_cards(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    laid_out(ui, data, PreferencesState::default(), Demo::default(), PreferencesLayout::Cards)
}

/// The cards layout under a search: the field shows how many settings match.
#[story(PreferencesWindow<'static>, covers = [], drive = Drive::Never)]
fn preferences_window_cards_searching(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let state = PreferencesState { search: "size".to_owned(), ..PreferencesState::default() };
    laid_out(ui, data, state, Demo::default(), PreferencesLayout::Cards)
}

/// Preferences that apply each edit immediately.
#[story(PreferencesWindow<'static>, covers = [], drive = Drive::Never)]
fn preferences_window_live(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mut open = *data.value("live_open", false);
    if clicked(&Button::new("Open preferences").show(ui)) {
        open = true;
    }
    let mut demo = data.value("live_demo", Demo::default()).clone();
    let mut result = StoryOutput::default();
    if open {
        let state = data.value("live_state", PreferencesState::default());
        let shown =
            PreferencesWindow::new(Bind::bound(state)).mode(PreferencesMode::Live).layout(PreferencesLayout::Cards).embedded().show(ui, &mut demo);
        open = shown.closed.is_none();
        result = shown.into();
    }
    *data.value("live_open", false) = open;
    *data.value("live_demo", Demo::default()) = demo;
    result
}
