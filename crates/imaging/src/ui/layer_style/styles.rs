//! The Styles page: the host's style presets in a [`PresetBrowser`] grid.

use craft_ui::buttons::Button;
use craft_ui::contract::{Bind, Props as _, Show as _, clicked};
use craft_ui::patterns::presets::{PresetBrowser, PresetEvent, PresetRow, PresetState, ThumbInfo};
use craft_ui::primitives::ContentPainterCx;
use egui::{Rect, Ui};

use super::dialog::title_row;
use super::labels::LayerStyleWords;

/// The host's style presets for the Styles page: the rows (groups and presets keyed by the preset's name) and the
/// painter of one preset's thumbnail. The page shows them in the library [`PresetBrowser`] grid and keeps the
/// browser's view state itself.
pub struct StylePresets<'a> {
    pub(super) rows: &'a [PresetRow<String>],
    pub(super) thumb: Thumb<'a>,
}

type Thumb<'a> = Box<dyn Fn(&ContentPainterCx, Rect, ThumbInfo<'_, String>) + 'a>;

impl<'a> StylePresets<'a> {
    /// The presets `rows`, each thumbnail painted by `thumb`.
    pub fn new(rows: &'a [PresetRow<String>], thumb: impl Fn(&ContentPainterCx, Rect, ThumbInfo<'_, String>) + 'a) -> Self {
        StylePresets { rows, thumb: Box::new(thumb) }
    }
}

/// What the Styles page asked for.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct StylesOutput {
    /// A click, Space or Enter on a preset: apply it to the draft (the host answers with
    /// [`LayerStyleDraft::apply_preset`](crate::layer_style::LayerStyleDraft::apply_preset); the document is untouched
    /// until OK).
    pub apply_style: Option<String>,
    /// New Style… was pressed.
    pub new_style: bool,
}

/// Draws the page: the title with the "New Style…" button at its end, then the browser. Without presets the page
/// shows only the button.
pub(super) fn styles_page(ui: &mut Ui, presets: Option<&StylePresets<'_>>, words: &LayerStyleWords) -> StylesOutput {
    let mut new_style = false;
    title_row(ui, words.styles.clone(), |ui| {
        new_style = clicked(&Button::new(words.new_style.clone()).id_salt("new-style").show(ui));
    });
    let mut out = StylesOutput { apply_style: None, new_style };
    if let Some(StylePresets { rows, thumb }) = presets {
        let browser = PresetBrowser::new(words.styles.clone(), Bind::<PresetState<String>>::kept(), rows)
            .thumb(move |cx, rect, info| thumb(cx, rect, info))
            .row_menu(false);
        for event in browser.show(ui).events {
            if let PresetEvent::Select { key, .. } | PresetEvent::Activate { key, .. } = event {
                out.apply_style = Some(key);
                break;
            }
        }
    }
    out
}
