//! Stories of `TextStylesPanel` and of the style picker built on `PresetPicker`.

use super::{NamedStyleRow, StylePreview, TextStyleKind, TextStyleRow, TextStylesPanel, style_rows};
use craft_behavior::{IconId, Mixed, Selection};
use craft_ui::contract::{Bind, Show as _};
use craft_ui::patterns::presets::{PresetKey, PresetPicker, PresetState};
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

fn paragraph_rows() -> Vec<TextStyleRow> {
    vec![TextStyleRow::new(0, "Basic Paragraph"), TextStyleRow::new(1, "Heading"), TextStyleRow::new(2, "Body")]
}

fn character_rows() -> Vec<TextStyleRow> {
    vec![TextStyleRow::new(0, "None"), TextStyleRow::new(1, "Emphasis"), TextStyleRow::new(2, "Strong"), TextStyleRow::new(3, "Code")]
}

#[story(TextStylesPanel<'static>, covers = [], drive = Drive::Never)]
fn paragraph_styles_with_an_overridden_style(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let rows = paragraph_rows();
    let selection = data.value("paragraph", Selection::<u64>::default());
    TextStylesPanel::new("Paragraph Styles", TextStyleKind::Paragraph, &rows, Bind::bound(selection))
        .current(Some(1), true)
        .target(true)
        .show(ui)
        .into()
}

#[story(TextStylesPanel<'static>, covers = [], drive = Drive::Never)]
fn character_styles_with_highlighted_rows(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let rows = character_rows();
    let selection = data.value("character", Selection::new(vec![1_u64, 2], Some(2), Some(1)));
    TextStylesPanel::new("Character Styles", TextStyleKind::Character, &rows, Bind::bound(selection))
        .current(Some(0), false)
        .target(true)
        .highlighting(true)
        .show(ui)
        .into()
}

fn picker_styles() -> Vec<NamedStyleRow> {
    vec![
        NamedStyleRow::new(1, "Primary", StylePreview::Icon(IconId::new("palette"))).meta("#1473E6"),
        NamedStyleRow::new(2, "Brand/Blue/500", StylePreview::Icon(IconId::new("palette"))).meta("#1473E6"),
        NamedStyleRow::new(3, "Brand/Blue/700", StylePreview::Icon(IconId::new("palette"))).meta("#0D66D0"),
    ]
}

#[story(TextStylesPanel<'static>, covers = [], drive = Drive::Never)]
fn style_picker_over_style_rows(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let rows = style_rows(&picker_styles());
    let (applied, state) = data.value("picker", (Mixed::Same(Some(PresetKey::Item(2_u64))), PresetState::<PresetKey<u64>>::new()));
    PresetPicker::new("Style", Bind::bound_mixed(applied), &rows, Bind::bound(state)).show(ui).into()
}
