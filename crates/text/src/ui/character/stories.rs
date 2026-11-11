//! Stories of the Character and Paragraph controls, the Paragraph dialog and its preview.

use super::{
    BaselinePosition, Caps, CharacterControls, CharacterValues, Kerning, Leading, LineSpacing, ParagraphAlign, ParagraphControls, ParagraphDialog,
    ParagraphFormat, ParagraphPreview, ParagraphValues, SpacingKind, SpecialIndent, TextAlign,
};
use craft_theme::Rgba;
use craft_ui::contract::Show as _;
use craft_ui::dialog::{DialogStack, StackDialog as _};
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

fn families() -> Vec<String> {
    ["Hack", "Ubuntu", "Noto Serif"].map(str::to_owned).to_vec()
}

fn styles() -> Vec<String> {
    ["Regular", "Bold", "Italic"].map(str::to_owned).to_vec()
}

fn languages() -> Vec<(String, String)> {
    [("en-US", "English: USA"), ("fr-FR", "French"), ("de-DE", "German")].map(|(tag, name)| (tag.to_owned(), name.to_owned())).to_vec()
}

#[story(CharacterControls<'static>, covers = [], drive = Drive::Never)]
fn single_values(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let values = CharacterValues {
        family: Some("Ubuntu".to_owned()),
        style: Some("Bold".to_owned()),
        size: Some(24.0),
        leading: Some(Leading::Auto),
        kerning: Some(Kerning::Optical),
        tracking: Some(25.0),
        vertical_scale: Some(100.0),
        horizontal_scale: Some(100.0),
        baseline_shift: Some(0.0),
        color: Some(Rgba([200, 60, 40, 255])),
        faux_bold: Some(false),
        faux_italic: Some(false),
        caps: Some(Caps::SmallCaps),
        position: Some(BaselinePosition::Normal),
        underline: Some(true),
        strikethrough: Some(false),
        opentype: vec!["liga", "frac"],
        language: Some("en-US".to_owned()),
    };
    let (families, styles, languages) = (families(), styles(), languages());
    CharacterControls::new("Character", &values, &families, &styles).superscript(true).languages(&languages).show(ui).into()
}

#[story(CharacterControls<'static>, covers = [], drive = Drive::Never)]
fn mixed_values(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let values = CharacterValues { size: Some(12.0), kerning: Some(Kerning::Manual(-20.0)), ..CharacterValues::default() };
    let (families, styles) = (families(), styles());
    CharacterControls::new("Character", &values, &families, &styles).show(ui).into()
}

#[story(ParagraphControls<'static>, covers = [], drive = Drive::Never)]
fn paragraph_text(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let values = ParagraphValues {
        align: Some(TextAlign::JustifyLeft),
        direction: Some(super::ParagraphDirection::Automatic),
        start_indent: Some(0.0),
        end_indent: Some(0.0),
        first_line_indent: Some(12.0),
        space_before: Some(0.0),
        space_after: Some(6.0),
        hyphenate: Some(true),
    };
    ParagraphControls::new("Paragraph", &values).show(ui).into()
}

#[story(ParagraphControls<'static>, covers = [], drive = Drive::Never)]
fn point_text(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let values = ParagraphValues { align: Some(TextAlign::Center), space_after: Some(6.0), ..ParagraphValues::default() };
    ParagraphControls::new("Paragraph", &values).point_text(true).show(ui).into()
}

fn indented() -> ParagraphFormat {
    ParagraphFormat {
        align: Some(ParagraphAlign::Justify),
        before_text: Some(36.0),
        after_text: Some(Some(0.0)),
        special: Some(SpecialIndent::Hanging),
        by: Some(18.0),
        space_before: Some(6.0),
        space_after: Some(1.0),
        space_after_kind: SpacingKind::Lines,
        line_spacing: Some(LineSpacing::Exactly(20.0)),
        ..ParagraphFormat::default()
    }
}

#[story(ParagraphDialog, covers = [], drive = Drive::Never)]
fn indents_and_spacing(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let mut dialog = ParagraphDialog::new(indented()).word(true);
    let _ = dialog.show(ui, &DialogStack::default());
    StoryOutput::default()
}

#[story(ParagraphPreview, covers = [Default], drive = Drive::Never)]
fn preview(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    ParagraphPreview::new("Preview", &indented(), "Sample Text").show(ui).into()
}
