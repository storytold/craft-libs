//! Stories of `PreferencesPage`.

use super::PreferencesLayout;
use crate::patterns::preferences::{PageSpec, PrefValue, PreferencesModel, PreferencesPage, PreferencesState, RowKind, RowSpec};
use crate::testing::{Drive, StoryData, StoryOutput, story};
use craft_theme::Rgba;
use egui::Ui;

/// Settings that change as soon as a row does: a model with no draft, as an options page keeps.
#[derive(Clone, Debug, PartialEq)]
struct Live {
    snap: bool,
    name: String,
    size: i64,
    tint: Rgba,
}

impl Default for Live {
    fn default() -> Self {
        Live { snap: true, name: "Poster".to_owned(), size: 24, tint: Rgba([0xff, 0x88, 0x00, 0xff]) }
    }
}

impl PreferencesModel for Live {
    fn pages(&self) -> Vec<PageSpec<'_>> {
        vec![
            PageSpec::new(
                "general",
                "General",
                vec![
                    RowSpec::new("general.snap", "Snap to grid", RowKind::Bool(self.snap)),
                    RowSpec::new("general.name", "Name", RowKind::Text(&self.name)),
                ],
            ),
            PageSpec::new(
                "canvas",
                "Canvas",
                vec![
                    RowSpec::new("canvas.size", "Brush size", RowKind::int(self.size, 1..=64)),
                    RowSpec::new("canvas.tint", "Tint", RowKind::Color(self.tint)),
                ],
            ),
        ]
    }

    fn set(&mut self, key: &str, value: PrefValue) {
        match (key, value) {
            ("general.snap", PrefValue::Bool(on)) => self.snap = on,
            ("general.name", PrefValue::Text(text)) => self.name = text,
            ("canvas.size", PrefValue::Int(size)) => self.size = size,
            ("canvas.tint", PrefValue::Color(tint)) => self.tint = tint,
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
        let defaults = Live::default();
        match page {
            "general" => (self.snap, self.name) = (defaults.snap, defaults.name),
            "canvas" => (self.size, self.tint) = (defaults.size, defaults.tint),
            _ => {}
        }
    }
}

/// The page over the story's settings.
fn page(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    laid_out(ui, data, PreferencesLayout::Paged)
}

/// The page over the story's settings, laid out as `layout` says.
fn laid_out(ui: &mut Ui, data: &mut StoryData, layout: PreferencesLayout) -> StoryOutput {
    let mut live = data.value("live", Live::default()).clone();
    let shown = PreferencesPage::new(data.bind("state", PreferencesState::default())).layout(layout).show(ui, &mut live);
    *data.value("live", Live::default()) = live;
    shown.into()
}

#[story(PreferencesPage<'static>, covers = [], drive = Drive::Never)]
fn preferences_page(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    page(ui, data)
}

/// The search field takes the focus first, so the typed text lands in it.
#[story(PreferencesPage<'static>, covers = [], drive = Drive::Type("size"))]
fn preferences_page_searching(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    page(ui, data)
}

/// The page as one scrolling column of cards.
#[story(PreferencesPage<'static>, covers = [], drive = Drive::Never)]
fn preferences_page_cards(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    laid_out(ui, data, PreferencesLayout::Cards)
}
