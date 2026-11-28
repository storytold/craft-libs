//! Stories of the Format Text panel: added text, a paragraph being edited, a mixed selection and a panel whose host
//! cannot apply most rows.

use super::{FormatControl, FormatText};
use crate::ui::character::{CharacterValues, LineSpacing, ParagraphValues, TextAlign};
use craft_theme::Rgba;
use craft_ui::runtime::env::Label;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

/// The standard families every PDF reader has.
fn standard_families() -> Vec<String> {
    ["Helvetica", "Times", "Courier"].map(str::to_owned).to_vec()
}

/// The fonts of a page and the standard families.
fn page_families() -> Vec<String> {
    ["Source Serif", "Source Sans", "Helvetica", "Times", "Courier"].map(str::to_owned).to_vec()
}

/// A single paragraph's values.
fn single() -> (CharacterValues, ParagraphValues) {
    let character = CharacterValues {
        family: Some("Source Serif".to_owned()),
        size: Some(11.0),
        color: Some(Rgba([30, 30, 30, 255])),
        faux_bold: Some(false),
        faux_italic: Some(true),
        underline: Some(false),
        horizontal_scale: Some(100.0),
        char_spacing: Some(0.5),
        ..CharacterValues::default()
    };
    let paragraph = ParagraphValues { align: Some(TextAlign::Left), space_after: Some(6.0), ..ParagraphValues::default() };
    (character, paragraph)
}

/// Text the user added: the standard families, and no line spacing or scale to set.
#[story(FormatText<'static>, covers = [], drive = Drive::Never)]
fn added_text(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let character = CharacterValues {
        family: Some("Helvetica".to_owned()),
        size: Some(12.0),
        color: Some(Rgba([200, 60, 40, 255])),
        faux_bold: Some(true),
        faux_italic: Some(false),
        char_spacing: Some(0.0),
        ..CharacterValues::default()
    };
    let paragraph = ParagraphValues { align: Some(TextAlign::Center), ..ParagraphValues::default() };
    let families = standard_families();
    let unsupported = [
        (FormatControl::LineSpacing, Label::from("Added text keeps its own line spacing")),
        (FormatControl::HorizontalScale, Label::from("Added text cannot be scaled")),
    ];
    FormatText::new("Format text", &character, &paragraph, &families).unsupported(&unsupported).show(ui).into()
}

/// A paragraph being edited: the page's fonts, and every spacing row.
#[story(FormatText<'static>, covers = [], drive = Drive::Never)]
fn paragraph(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let (character, paragraph) = single();
    let families = page_families();
    FormatText::new("Format text", &character, &paragraph, &families)
        .line_spacing(LineSpacing::Multiple(1.15))
        .scale_presets(&[50.0, 75.0, 100.0, 125.0, 150.0, 200.0])
        .char_spacing_presets(&[-1.0, -0.5, 0.0, 0.5, 1.0, 2.0])
        .show(ui)
        .into()
}

/// A selection that mixes every value: the fields show empty, the toggles unpressed.
#[story(FormatText<'static>, covers = [], drive = Drive::Never)]
fn mixed(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let (character, paragraph) = (CharacterValues::default(), ParagraphValues::default());
    let families = page_families();
    FormatText::new("Format text", &character, &paragraph, &families).show(ui).into()
}

/// A host that applies only the font, size and colour: every other control is disabled with its reason.
#[story(FormatText<'static>, covers = [], drive = Drive::Never)]
fn unsupported_rows(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let (character, paragraph) = single();
    let families = page_families();
    let planned = Label::from("Not available yet");
    let unsupported = [
        FormatControl::BulletedList,
        FormatControl::NumberedList,
        FormatControl::Align,
        FormatControl::Bold,
        FormatControl::Italic,
        FormatControl::Underline,
        FormatControl::Superscript,
        FormatControl::Subscript,
        FormatControl::LineSpacing,
        FormatControl::ParagraphSpacing,
        FormatControl::HorizontalScale,
        FormatControl::CharSpacing,
    ]
    .map(|control| (control, planned.clone()));
    FormatText::new("Format text", &character, &paragraph, &families).line_spacing(LineSpacing::Auto).unsupported(&unsupported).show(ui).into()
}
