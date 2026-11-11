//! The Paragraph panel: alignment, direction, values, changes, words and `ParagraphControls`.

use std::ops::RangeInclusive;

use crate::ui::length::DocUnit;
use craft_behavior::{EditEvent, IconId};
use craft_ui::choice::{Checkbox, Segmented};
use craft_ui::contract::{Bind, Entry, GlyphLabel, NoValue, Outcome, Props as _, Show as _, ShowContainer as _};
use craft_ui::layout_components::{Grid, Stack};
use craft_ui::runtime::env::{Env, Label};
use egui::{Response, Ui};

use super::TWO_COLUMNS;
use super::rows::{LengthRow, Lengths, mixed, number_change};

craft_ui::words! {
    /// The Paragraph controls' words, English by default (PhotoCraft's tooltips).
    #[non_exhaustive]
    pub struct ParagraphWords {
        paragraph: "paragraph.title" = "Paragraph";
        alignment: "paragraph.alignment" = "Alignment";
        direction: "paragraph.direction" = "Direction";
        left: "paragraph.left" = "Left align text";
        center: "paragraph.center" = "Center text";
        right: "paragraph.right" = "Right align text";
        justify_left: "paragraph.justifyLeft" = "Justify last left";
        justify_center: "paragraph.justifyCenter" = "Justify last centered";
        justify_right: "paragraph.justifyRight" = "Justify last right";
        justify_all: "paragraph.justifyAll" = "Justify all";
        justify_needs_box: "paragraph.justifyNeedsBox" = "Justification needs a text box; point text has none";
        direction_auto: "paragraph.directionAuto" = "Automatic paragraph direction";
        direction_ltr: "paragraph.directionLtr" = "Left-to-right paragraph direction";
        direction_rtl: "paragraph.directionRtl" = "Right-to-left paragraph direction";
        start_indent: "paragraph.startIndent" = "Indent left margin";
        end_indent: "paragraph.endIndent" = "Indent right margin";
        first_line_indent: "paragraph.firstLineIndent" = "Indent first line";
        space_before: "paragraph.spaceBefore" = "Add space before paragraph";
        space_after: "paragraph.spaceAfter" = "Add space after paragraph";
        hyphenate: "paragraph.hyphenate" = "Hyphenate";
    }
}

/// The range of the indent fields, points.
const INDENT_RANGE: RangeInclusive<f64> = -1296.0..=1296.0;
/// The range of the space fields, points.
const SPACE_RANGE: RangeInclusive<f64> = 0.0..=1296.0;

/// Paragraph alignment, in Photoshop's button order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TextAlign {
    /// Left.
    #[default]
    Left,
    /// Center.
    Center,
    /// Right.
    Right,
    /// Justify left.
    JustifyLeft,
    /// Justify center.
    JustifyCenter,
    /// Justify right.
    JustifyRight,
    /// Justify all.
    JustifyAll,
}

impl TextAlign {
    /// Whether the alignment needs a text box: justification has a line width to fill.
    fn needs_box(self) -> bool {
        matches!(self, TextAlign::JustifyLeft | TextAlign::JustifyCenter | TextAlign::JustifyRight | TextAlign::JustifyAll)
    }
}

/// A paragraph's direction.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ParagraphDirection {
    /// From the paragraph's first strong character.
    #[default]
    Automatic,
    /// Left to right.
    LeftToRight,
    /// Right to left.
    RightToLeft,
}

/// The paragraph attributes at the type target; `None` where the selected paragraphs differ.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct ParagraphValues {
    /// The align.
    pub align: Option<TextAlign>,
    /// The direction.
    pub direction: Option<ParagraphDirection>,
    /// Indent at the start edge in points (the left in left-to-right text).
    pub start_indent: Option<f32>,
    /// Indent at the end edge in points (the right in left-to-right text).
    pub end_indent: Option<f32>,
    /// First-line indent in points.
    pub first_line_indent: Option<f32>,
    /// Space before the paragraph in points.
    pub space_before: Option<f32>,
    /// Space after the paragraph in points.
    pub space_after: Option<f32>,
    /// The hyphenate.
    pub hyphenate: Option<bool>,
}

/// One change from the Paragraph controls.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum ParagraphChange {
    /// Align.
    Align(TextAlign),
    /// Direction.
    Direction(ParagraphDirection),
    /// Indent at the start edge in points.
    StartIndent(f32),
    /// Indent at the end edge in points.
    EndIndent(f32),
    /// First-line indent in points.
    FirstLineIndent(f32),
    /// Space before the paragraph in points.
    SpaceBefore(f32),
    /// Space after the paragraph in points.
    SpaceAfter(f32),
    /// Hyphenate.
    Hyphenate(bool),
}

/// What the Paragraph controls did this pass.
#[derive(Debug)]
#[non_exhaustive]
pub struct ParagraphControlsOutput {
    /// The controls' response.
    pub response: Response,
    /// As [`CharacterControlsOutput::edits`].
    pub edits: Vec<EditEvent<ParagraphChange>>,
}

impl Outcome for ParagraphControlsOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The Paragraph panel's controls: the seven alignment buttons; the three direction buttons; left and right indent;
/// first-line indent; space before and after; Hyphenate.
///
/// - The four justify buttons are disabled on point text ([`point_text`](Self::point_text)): justification needs a
///   paragraph box or area type (Photoshop greys them).
/// - Ranges in points: indents −1296–1296, space before and after 0–1296.
#[must_use = "show the controls with `show`"]
#[derive(craft_ui::contract::Pattern, craft_ui::contract::Props)]
pub struct ParagraphControls<'a> {
    label: Label,
    values: &'a ParagraphValues,
    /// The unit the length fields show. Default points.
    #[prop]
    unit: DocUnit,
    /// Any of the formatted text is point text (no box): the justify buttons are disabled.
    #[prop]
    point_text: bool,
}

impl<'a> ParagraphControls<'a> {
    /// Controls named `label` for the paragraph attributes `values`.
    pub fn new(label: impl Into<Label>, values: &'a ParagraphValues) -> Self {
        ParagraphControls { label: label.into(), values, unit: DocUnit::points(), point_text: false }
    }

    /// Shows the controls and returns the changes the user made.
    pub fn show(self, ui: &mut Ui) -> ParagraphControlsOutput {
        let values = self.values;
        let point_text = self.point_text;
        let env = Env::of(ui);
        let words = env.words::<ParagraphWords>();
        let units = env.units();
        let lengths = Lengths { units: &units, unit: self.unit };

        let aligns: Vec<Entry<TextAlign>> = [
            (TextAlign::Left, "align_left", &words.left),
            (TextAlign::Center, "align_center", &words.center),
            (TextAlign::Right, "align_right", &words.right),
            (TextAlign::JustifyLeft, "align_justify", &words.justify_left),
            (TextAlign::JustifyCenter, "align_justify", &words.justify_center),
            (TextAlign::JustifyRight, "align_justify", &words.justify_right),
            (TextAlign::JustifyAll, "align_justify", &words.justify_all),
        ]
        .into_iter()
        .map(|(align, icon, name)| {
            let entry = Entry::new(align, name.clone()).icon(IconId::new(icon));
            if point_text && align.needs_box() { entry.disabled_reason(words.justify_needs_box.clone()) } else { entry }
        })
        .collect();
        let directions: Vec<Entry<ParagraphDirection>> = [
            (ParagraphDirection::Automatic, "pilcrow", &words.direction_auto),
            (ParagraphDirection::LeftToRight, "pilcrow_right", &words.direction_ltr),
            (ParagraphDirection::RightToLeft, "pilcrow_left", &words.direction_rtl),
        ]
        .into_iter()
        .map(|(direction, icon, name)| Entry::new(direction, name.clone()).icon(IconId::new(icon)))
        .collect();

        let mut edits: Vec<EditEvent<ParagraphChange>> = Vec::new();
        let shown = Stack::new().fill_width().label(self.label).show(ui, |ui, _| {
            let out = Segmented::new(words.alignment.clone(), Bind::given(mixed(values.align)), &aligns).show(ui);
            edits.extend(out.events.map(ParagraphChange::Align));
            let out = Segmented::new(words.direction.clone(), Bind::given(mixed(values.direction)), &directions).show(ui);
            edits.extend(out.events.map(ParagraphChange::Direction));

            Grid::new(&TWO_COLUMNS).fill_width().show(ui, |ui, _| {
                let start = LengthRow {
                    label: words.start_indent.clone(),
                    glyph: GlyphLabel::text("→|"),
                    value: values.start_indent,
                    range: INDENT_RANGE,
                    presets: &[],
                };
                let (event, _) = lengths.row(ui, start);
                edits.extend(number_change(event, ParagraphChange::StartIndent));
                let end = LengthRow {
                    label: words.end_indent.clone(),
                    glyph: GlyphLabel::text("|←"),
                    value: values.end_indent,
                    range: INDENT_RANGE,
                    presets: &[],
                };
                let (event, _) = lengths.row(ui, end);
                edits.extend(number_change(event, ParagraphChange::EndIndent));
                let first = LengthRow {
                    label: words.first_line_indent.clone(),
                    glyph: GlyphLabel::text("¶→"),
                    value: values.first_line_indent,
                    range: INDENT_RANGE,
                    presets: &[],
                };
                let (event, _) = lengths.row(ui, first);
                edits.extend(number_change(event, ParagraphChange::FirstLineIndent));
            });
            Grid::new(&TWO_COLUMNS).fill_width().show(ui, |ui, _| {
                let before = LengthRow {
                    label: words.space_before.clone(),
                    glyph: GlyphLabel::text("↑¶"),
                    value: values.space_before,
                    range: SPACE_RANGE,
                    presets: &[],
                };
                let (event, _) = lengths.row(ui, before);
                edits.extend(number_change(event, ParagraphChange::SpaceBefore));
                let after = LengthRow {
                    label: words.space_after.clone(),
                    glyph: GlyphLabel::text("¶↓"),
                    value: values.space_after,
                    range: SPACE_RANGE,
                    presets: &[],
                };
                let (event, _) = lengths.row(ui, after);
                edits.extend(number_change(event, ParagraphChange::SpaceAfter));
            });
            let out = Checkbox::new(Bind::given(mixed(values.hyphenate))).label(words.hyphenate.clone()).show(ui);
            edits.extend(out.events.map(ParagraphChange::Hyphenate));
        });
        ParagraphControlsOutput { response: shown.output, edits }
    }
}
