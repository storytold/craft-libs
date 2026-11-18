//! [`FormatText`]: the Format Text panel, a character and paragraph editor in five rows.

use std::ops::RangeInclusive;

use craft_behavior::EditEvent;
use craft_ui::contract::{NoValue, Outcome};
use craft_ui::patterns::form_dialogs::HostRequests;
use craft_ui::runtime::env::Label;
use egui::{Response, Ui};

#[cfg(feature = "font-picker")]
use super::FontPickerSource;
use super::{CharacterChange, CharacterValues, LineSpacing, ParagraphChange, ParagraphValues};

craft_ui::words! {
    /// The Format Text panel's words, English by default.
    #[non_exhaustive]
    pub struct FormatTextWords {
        bulleted_list: "formatText.bulletedList" = "Bulleted list";
        numbered_list: "formatText.numberedList" = "Numbered list";
        line_spacing: "formatText.lineSpacing" = "Line spacing";
        paragraph_spacing: "formatText.paragraphSpacing" = "Paragraph spacing";
        char_spacing: "formatText.charSpacing" = "Character spacing";
    }
}

/// A control of the Format Text panel, named to disable it with a reason through
/// [`FormatText::unsupported`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum FormatControl {
    /// The font family.
    Font,
    /// The font size.
    Size,
    /// The text colour.
    Color,
    /// The bulleted list button.
    BulletedList,
    /// The numbered list button.
    NumberedList,
    /// The alignment buttons.
    Align,
    /// Bold.
    Bold,
    /// Italic.
    Italic,
    /// Underline.
    Underline,
    /// Superscript.
    Superscript,
    /// Subscript.
    Subscript,
    /// The line spacing.
    LineSpacing,
    /// The space before and after a paragraph.
    ParagraphSpacing,
    /// The horizontal scale.
    HorizontalScale,
    /// The character spacing.
    CharSpacing,
}

/// A kind of list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListKind {
    /// A bulleted list.
    Bulleted,
    /// A numbered list.
    Numbered,
}

/// One change from the Format Text panel.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum FormatTextChange {
    /// A character change; a style row pick is `Character(Family(..))` then `Character(Style(..))`.
    Character(CharacterChange),
    /// The key of a host row picked in the font picker.
    HostFont(String),
    /// A paragraph change.
    Paragraph(ParagraphChange),
    /// A line spacing.
    LineSpacing(LineSpacing),
    /// A list button.
    List(ListKind),
}

/// What the Format Text panel did this pass.
#[non_exhaustive]
pub struct FormatTextOutput {
    /// The panel's response.
    pub response: Response,
    /// The edit events of this pass, in the order of the controls; a scrub or a typing run is one gesture.
    pub edits: Vec<EditEvent<FormatTextChange>>,
    /// The colour swatch was clicked: the app opens its colour picker for this request.
    pub requests: HostRequests,
}

impl Outcome for FormatTextOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The Format Text panel pattern: five rows (font; size, colour, list buttons, alignment; bold, italic, underline,
/// superscript, subscript; line and paragraph spacing; scale and character spacing). A control in `unsupported` is
/// disabled with its reason. It keeps no state between passes.
#[must_use = "show the controls with `show`"]
#[derive(craft_ui::contract::Pattern, craft_ui::contract::Props)]
#[allow(dead_code)] // P3(pdf-edit-w1): ED-F1
pub struct FormatText<'a> {
    label: Label,
    character: &'a CharacterValues,
    paragraph: &'a ParagraphValues,
    families: &'a [String],
    /// The family field is craft-font-manager's font picker over the source, with the host's rows.
    #[cfg(feature = "font-picker")]
    #[prop(name = "font_picker")]
    picker: Option<FontPickerSource<'a>>,
    /// The paragraphs' line spacing; `None` while they mix it.
    #[prop]
    line_spacing: Option<LineSpacing>,
    /// The line spacings the line spacing menu offers.
    #[prop]
    line_presets: &'a [LineSpacing],
    /// The font sizes the size menu offers, in points.
    #[prop]
    size_presets: &'a [f32],
    /// The range of the size field, in points.
    #[prop]
    size_range: RangeInclusive<f32>,
    /// The horizontal scales the scale menu offers, in percent.
    #[prop]
    scale_presets: &'a [f32],
    /// The range of the horizontal scale field, in percent.
    #[prop]
    scale_range: RangeInclusive<f32>,
    /// The character spacings the character spacing menu offers, in points.
    #[prop]
    char_spacing_presets: &'a [f32],
    /// The range of the character spacing field, in points.
    #[prop]
    char_spacing_range: RangeInclusive<f32>,
    /// Controls the host cannot apply, each disabled with its reason.
    #[prop]
    unsupported: &'a [(FormatControl, Label)],
}

impl<'a> FormatText<'a> {
    /// A Format Text panel named `label` for the `character` and `paragraph` values, with the installed `families`.
    pub fn new(label: impl Into<Label>, character: &'a CharacterValues, paragraph: &'a ParagraphValues, families: &'a [String]) -> Self {
        FormatText {
            label: label.into(),
            character,
            paragraph,
            families,
            #[cfg(feature = "font-picker")]
            picker: None,
            line_spacing: None,
            line_presets: &[],
            size_presets: &[],
            size_range: 0.1..=1296.0,
            scale_presets: &[],
            scale_range: 0.0..=1000.0,
            char_spacing_presets: &[],
            char_spacing_range: -1000.0..=10000.0,
            unsupported: &[],
        }
    }

    /// Shows the panel and returns the changes the user made.
    pub fn show(self, ui: &mut Ui) -> FormatTextOutput {
        let _ = (self, ui);
        todo!("ED-F1")
    }
}
