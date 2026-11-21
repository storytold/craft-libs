//! The Styles page: the host's style presets in a [`PresetBrowser`] grid.

use craft_theme::TypeRole;
use craft_ui::contract::{Bind, Show as _};
use craft_ui::display::Text;
use craft_ui::patterns::presets::{PresetBrowser, PresetEvent, PresetRowMenu, PresetSource, PresetState};
use egui::Ui;

use super::labels::LayerStyleWords;

/// The rows of styles the Styles page shows before the grid scrolls: the user's style library can grow long, and the
/// dialog page around it scrolls too.
const STYLE_ROWS: u16 = 5;

/// What the Styles page asked for.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct StylesOutput {
    /// A click, Space or Enter on a preset: apply it to the draft (the host answers with
    /// [`LayerStyleDraft::apply_preset`](crate::layer_style::LayerStyleDraft::apply_preset); the document is untouched
    /// until OK).
    pub apply_style: Option<String>,
}

/// Draws the page: the title, then the host's style presets (groups and presets keyed by the preset's name) in the
/// library browser grid, whose view state the page keeps. Without presets the page shows only the title.
pub(super) fn styles_page(ui: &mut Ui, presets: Option<&PresetSource<'_, String>>, words: &LayerStyleWords) -> StylesOutput {
    Text::new(words.styles.clone()).role(TypeRole::Title).show(ui);
    let mut out = StylesOutput { apply_style: None };
    if let Some(presets) = presets {
        let browser =
            PresetBrowser::new(words.styles.clone(), Bind::<PresetState<String>>::kept(), presets).row_menu(PresetRowMenu::Off).max_rows(STYLE_ROWS);
        for event in browser.show(ui).events {
            if let PresetEvent::Select { key, .. } | PresetEvent::Activate { key, .. } = event {
                out.apply_style = Some(key);
                break;
            }
        }
    }
    out
}
