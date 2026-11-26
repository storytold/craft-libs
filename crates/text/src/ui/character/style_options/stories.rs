//! Stories of `TextStyleOptions`: a character style with some rows included, a paragraph style and the Basic Paragraph.

use super::{StyleAttribute, StyleOptionValues, TextStyleOptions};
use crate::ui::character::{CharacterValues, Kerning, Leading, ParagraphValues, TextAlign};
use craft_theme::Rgba;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

fn families() -> Vec<String> {
    ["Hack", "Ubuntu", "Noto Serif"].map(str::to_owned).to_vec()
}

fn styles() -> Vec<String> {
    ["Regular", "Bold", "Italic"].map(str::to_owned).to_vec()
}

fn character_values() -> CharacterValues {
    CharacterValues {
        family: Some("Ubuntu".to_owned()),
        style: Some("Bold".to_owned()),
        size: Some(24.0),
        leading: Some(Leading::Auto),
        kerning: Some(Kerning::Metrics),
        tracking: Some(25.0),
        baseline_shift: Some(0.0),
        color: Some(Rgba([200, 60, 40, 255])),
        faux_bold: Some(false),
        faux_italic: Some(true),
        underline: Some(false),
        strikethrough: Some(false),
        ..CharacterValues::default()
    }
}

fn paragraph_values() -> ParagraphValues {
    ParagraphValues {
        align: Some(TextAlign::JustifyLeft),
        first_line_indent: Some(12.0),
        start_indent: Some(0.0),
        end_indent: Some(0.0),
        space_before: Some(0.0),
        space_after: Some(6.0),
        hyphenate: Some(true),
        ..ParagraphValues::default()
    }
}

#[story(TextStyleOptions<'static>, covers = [], drive = Drive::Never)]
fn character_style_with_some_rows_included(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let mut values = StyleOptionValues::new("Emphasis", character_values());
    values.included = vec![StyleAttribute::Font, StyleAttribute::Size, StyleAttribute::Color];
    let (families, styles) = (families(), styles());
    TextStyleOptions::new("Character Style Options", &values, &StyleAttribute::CHARACTER, &families, &styles).show(ui).into()
}

#[story(TextStyleOptions<'static>, covers = [], drive = Drive::Never)]
fn paragraph_style(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let mut values = StyleOptionValues::new("Heading", character_values());
    values.paragraph = Some(paragraph_values());
    values.included = vec![StyleAttribute::Align, StyleAttribute::SpaceAfter, StyleAttribute::Hyphenate];
    let rows: Vec<StyleAttribute> = StyleAttribute::PARAGRAPH.to_vec();
    let (families, styles) = (families(), styles());
    TextStyleOptions::new("Paragraph Style Options", &values, &rows, &families, &styles).show(ui).into()
}

#[story(TextStyleOptions<'static>, covers = [], drive = Drive::Never)]
fn basic_paragraph_not_renamable(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let mut values = StyleOptionValues::new("Basic Paragraph", character_values());
    values.paragraph = Some(paragraph_values());
    values.included = StyleAttribute::PARAGRAPH.to_vec();
    let rows: Vec<StyleAttribute> = StyleAttribute::PARAGRAPH.to_vec();
    let (families, styles) = (families(), styles());
    TextStyleOptions::new("Paragraph Style Options", &values, &rows, &families, &styles).renamable(false).show(ui).into()
}
