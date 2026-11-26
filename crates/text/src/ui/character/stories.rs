//! Stories of the Character and Paragraph controls, the Paragraph dialog and its preview.

use super::{
    AsianTypography, BaselinePosition, Caps, CharacterControls, CharacterRows, CharacterValues, DirectionRow, FontHeightReference, Kerning, Leading,
    LineBreakOptions, LineSpacing, ParagraphAlign, ParagraphControls, ParagraphDialog, ParagraphFormat, ParagraphPreview, ParagraphTab,
    ParagraphValues, SpacingKind, SpecialIndent, TextAlign,
};
#[cfg(feature = "font-picker")]
use super::{AxesLayout, FaceAxes};
use crate::ui::type_options::TextAntiAlias;
use craft_theme::Rgba;
use craft_ui::contract::Show as _;
use craft_ui::dialog::stories::hosted;
use craft_ui::dialog::{DialogStack, StackDialog as _};
use craft_ui::display::Text;
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

/// The values every row story starts from.
fn single() -> CharacterValues {
    CharacterValues {
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
        ..CharacterValues::default()
    }
}

#[story(CharacterControls<'static>, covers = [], drive = Drive::Never)]
fn single_values(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let values = single();
    let (families, styles, languages) = (families(), styles(), languages());
    CharacterControls::new("Character", &values, &families, &styles)
        .rows(CharacterRows::default() | CharacterRows::SUPERSCRIPT)
        .languages(&languages)
        .show(ui)
        .into()
}

#[story(CharacterControls<'static>, covers = [], drive = Drive::Never)]
fn rows_illustrator(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let values = CharacterValues {
        rotation: Some(0.0),
        anti_alias: Some(TextAntiAlias::Sharp),
        font_height: Some(FontHeightReference::CapHeight),
        ..single()
    };
    let rows = CharacterRows::default()
        | CharacterRows::SCALES
        | CharacterRows::ROTATION
        | CharacterRows::ANTI_ALIAS
        | CharacterRows::FONT_HEIGHT
        | CharacterRows::SUPERSCRIPT;
    let (families, styles) = (families(), styles());
    CharacterControls::new("Character", &values, &families, &styles).rows(rows).show(ui).into()
}

#[story(CharacterControls<'static>, covers = [], drive = Drive::Never)]
fn rows_east_asian_and_middle_eastern(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let values = CharacterValues {
        aki_left: Some(0.0),
        aki_right: Some(250.0),
        tsume: Some(50.0),
        diacritic_x: Some(0.0),
        diacritic_y: Some(0.0),
        ..single()
    };
    let rows = CharacterRows::default() | CharacterRows::EAST_ASIAN | CharacterRows::MIDDLE_EASTERN;
    let (families, styles) = (families(), styles());
    CharacterControls::new("Character", &values, &families, &styles).rows(rows).show(ui).into()
}

#[story(CharacterControls<'static>, covers = [], drive = Drive::Never)]
fn rows_properties(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let values = single();
    let (families, styles) = (families(), styles());
    CharacterControls::new("Character", &values, &families, &styles).rows(CharacterRows::PROPERTIES).show(ui).into()
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
fn direction_explicit(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let values =
        ParagraphValues { align: Some(TextAlign::Left), direction: Some(super::ParagraphDirection::RightToLeft), ..ParagraphValues::default() };
    ParagraphControls::new("Paragraph", &values).direction_row(DirectionRow::Explicit).show(ui).into()
}

#[story(ParagraphControls<'static>, covers = [], drive = Drive::Never)]
fn direction_hidden(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let values = ParagraphValues { align: Some(TextAlign::Left), ..ParagraphValues::default() };
    ParagraphControls::new("Paragraph", &values).direction_row(DirectionRow::Hidden).show(ui).into()
}

#[story(ParagraphControls<'static>, covers = [], drive = Drive::Never)]
fn between_rows(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let values = ParagraphValues { align: Some(TextAlign::Left), space_after: Some(6.0), ..ParagraphValues::default() };
    ParagraphControls::new("Paragraph", &values)
        .show_with(ui, |ui| {
            Text::new("Host rows").show(ui);
        })
        .into()
}

// No variable face ships with the stories, so the sliders have no axis to list; the state shows the slot is taken.
#[cfg(feature = "font-picker")]
#[story(CharacterControls<'static>, covers = [], drive = Drive::Never)]
fn with_axes(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let values = CharacterValues {
        family: Some("Ubuntu".to_owned()),
        style: Some("Bold".to_owned()),
        axes: Some(vec![(*b"wght", 700.0)]),
        ..CharacterValues::default()
    };
    let (families, styles) = (families(), styles());
    CharacterControls::new("Character", &values, &families, &styles).axes(FaceAxes::new(&[], &[])).show(ui).into()
}

#[cfg(feature = "font-picker")]
#[story(CharacterControls<'static>, covers = [], drive = Drive::Never)]
fn axes_button(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let values = CharacterValues {
        family: Some("Ubuntu".to_owned()),
        style: Some("Bold".to_owned()),
        axes: Some(vec![(*b"wght", 700.0)]),
        ..CharacterValues::default()
    };
    let axes = [craft_font_manager::axes::FontAxis::new(*b"wght", "Weight", 100.0, 400.0, 900.0)];
    let (families, styles) = (families(), styles());
    CharacterControls::new("Character", &values, &families, &styles).axes(FaceAxes::new(&axes, &[])).axes_layout(AxesLayout::Button).show(ui).into()
}

#[story(ParagraphControls<'static>, covers = [], drive = Drive::Never)]
fn vertical(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let values = ParagraphValues { align: Some(TextAlign::Center), space_after: Some(6.0), ..ParagraphValues::default() };
    ParagraphControls::new("Paragraph", &values).vertical(true).show(ui).into()
}

#[story(ParagraphControls<'static>, covers = [], drive = Drive::Never)]
fn no_hyphenate(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let values = ParagraphValues { align: Some(TextAlign::Left), hyphenate: Some(true), ..ParagraphValues::default() };
    ParagraphControls::new("Paragraph", &values).hyphenate(false).show(ui).into()
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
        direction: Some(Some(super::ParagraphDirection::LeftToRight)),
        mirror_indents: Some(Some(false)),
        breaks: Some(LineBreakOptions {
            widow_control: Some(true),
            keep_with_next: None,
            keep_lines_together: Some(false),
            page_break_before: Some(false),
            suppress_line_numbers: Some(false),
            dont_hyphenate: Some(true),
        }),
        asian: Some(AsianTypography {
            kinsoku: Some(true),
            word_wrap: Some(false),
            overflow_punct: None,
            top_line_punct: Some(true),
            auto_space_de: Some(true),
            auto_space_dn: Some(false),
        }),
        ..ParagraphFormat::default()
    }
}

#[story(ParagraphDialog, covers = [], drive = Drive::Never)]
fn indents_and_spacing(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let mut dialog = ParagraphDialog::new(indented()).word(true);
    let _ = dialog.show(ui, &DialogStack::default());
    StoryOutput::default()
}

/// Paragraphs that differ in "Don't add space between paragraphs of the same style": the box under the spacing rows is
/// mixed.
#[story(ParagraphDialog, covers = [], drive = Drive::Never)]
fn contextual_spacing_mixed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    hosted(ui, data, (), || ParagraphDialog::new(ParagraphFormat { contextual_spacing: Some(None), ..indented() }).word(true))
}

#[story(ParagraphDialog, covers = [], drive = Drive::Never)]
fn line_and_page_breaks(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    hosted(ui, data, (), || ParagraphDialog::new(indented()).word(true).tab(ParagraphTab::LineAndPageBreaks))
}

#[story(ParagraphDialog, covers = [], drive = Drive::Never)]
fn asian_typography(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    hosted(ui, data, (), || ParagraphDialog::new(indented()).word(true).tab(ParagraphTab::AsianTypography))
}

#[story(ParagraphPreview, covers = [Default], drive = Drive::Never)]
fn preview(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    ParagraphPreview::new("Preview", &indented(), "Sample Text").show(ui).into()
}
