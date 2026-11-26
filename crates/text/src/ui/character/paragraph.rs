//! The Paragraph panel: alignment, direction, values, changes, words and `ParagraphControls`.

use std::ops::RangeInclusive;

use crate::ui::length::DocUnit;
use craft_behavior::EditEvent;
use craft_ui::choice::Checkbox;
use craft_ui::contract::{Bind, FieldLayout, GlyphLabel, NoValue, Outcome, Props as _, Show as _, ShowContainer as _};
use craft_ui::layout_components::{Grid, Stack};
use craft_ui::runtime::env::{Env, Label};
use egui::{Response, Ui};

use super::TWO_COLUMNS;
use super::rows::{LengthRow, Lengths, Toggle, agree, mark, mixed, number_change, toggle_row};

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
        top: "paragraph.top" = "Align text to the top";
        middle: "paragraph.middle" = "Center text vertically";
        bottom: "paragraph.bottom" = "Align text to the bottom";
    }
}

/// Which direction toggles the Paragraph controls show.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum DirectionRow {
    /// Automatic, Left to Right and Right to Left.
    #[default]
    Automatic,
    /// Left to Right and Right to Left only.
    Explicit,
    /// No direction row.
    Hidden,
}

/// The range of the indent fields, points.
pub(super) const INDENT_RANGE: RangeInclusive<f64> = -1296.0..=1296.0;
/// The range of the space fields, points.
pub(super) const SPACE_RANGE: RangeInclusive<f64> = 0.0..=1296.0;

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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
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

impl ParagraphValues {
    /// The values of two selections together: each field stays where both are equal and becomes `None` where they
    /// differ.
    pub fn mixed_with(&self, other: &Self) -> Self {
        ParagraphValues {
            align: agree(&self.align, &other.align),
            direction: agree(&self.direction, &other.direction),
            start_indent: agree(&self.start_indent, &other.start_indent),
            end_indent: agree(&self.end_indent, &other.end_indent),
            first_line_indent: agree(&self.first_line_indent, &other.first_line_indent),
            space_before: agree(&self.space_before, &other.space_before),
            space_after: agree(&self.space_after, &other.space_after),
            hyphenate: agree(&self.hyphenate, &other.hyphenate),
        }
    }
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
    /// As `CharacterControlsOutput::edits`.
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
    /// Vertical text: Left, Center and Right show the top, middle and bottom glyphs and words.
    #[prop]
    vertical: bool,
    /// Shows the Hyphenate check box. Default true.
    #[prop]
    hyphenate: bool,
    /// Which direction toggles the direction row shows.
    #[prop]
    direction_row: DirectionRow,
}

impl<'a> ParagraphControls<'a> {
    /// Controls named `label` for the paragraph attributes `values`.
    pub fn new(label: impl Into<Label>, values: &'a ParagraphValues) -> Self {
        ParagraphControls {
            label: label.into(),
            values,
            unit: DocUnit::points(),
            point_text: false,
            vertical: false,
            hyphenate: true,
            direction_row: DirectionRow::default(),
        }
    }

    /// Shows the controls and returns the changes the user made.
    pub fn show(self, ui: &mut Ui) -> ParagraphControlsOutput {
        self.show_with(ui, |_| {})
    }

    /// As [`show`](Self::show), with the host's rows drawn by `between` after the alignment and direction rows and
    /// before the indents.
    pub fn show_with(self, ui: &mut Ui, between: impl FnOnce(&mut Ui)) -> ParagraphControlsOutput {
        let mut between = Some(between);
        let direction_row = self.direction_row;
        let values = self.values;
        let point_text = self.point_text;
        let vertical = self.vertical;
        let show_hyphenate = self.hyphenate;
        let env = Env::of(ui);
        let words = env.words::<ParagraphWords>();
        let units = env.units();
        let lengths = Lengths { units: &units, unit: self.unit };

        let aligns: Vec<Toggle<ParagraphChange>> =
            align_toggles(values, &words, vertical).into_iter().chain(justify_toggles(values, &words, point_text)).collect();
        let directions = match direction_row {
            DirectionRow::Automatic => direction_toggles(values, &words),
            DirectionRow::Explicit => explicit_toggles(values, &words),
            DirectionRow::Hidden => Vec::new(),
        };

        let mut edits: Vec<EditEvent<ParagraphChange>> = Vec::new();
        let shown = Stack::new().label(self.label).show(ui, |ui, _| {
            edits.extend(toggle_row(ui, Some(&words.alignment), &aligns));
            if direction_row != DirectionRow::Hidden {
                edits.extend(toggle_row(ui, Some(&words.direction), &directions));
            }
            if let Some(between) = between.take() {
                between(ui);
            }

            Grid::new(&TWO_COLUMNS).fill_width().show(ui, |ui, _| {
                let start = LengthRow {
                    label: words.start_indent.clone(),
                    side: mark(GlyphLabel::text("→|"), None, FieldLayout::Inside),
                    value: values.start_indent,
                    range: INDENT_RANGE,
                    presets: &[],
                    disabled: None,
                };
                let (event, _) = lengths.row(ui, start);
                edits.extend(number_change(event, ParagraphChange::StartIndent));
                let end = LengthRow {
                    label: words.end_indent.clone(),
                    side: mark(GlyphLabel::text("|←"), None, FieldLayout::Inside),
                    value: values.end_indent,
                    range: INDENT_RANGE,
                    presets: &[],
                    disabled: None,
                };
                let (event, _) = lengths.row(ui, end);
                edits.extend(number_change(event, ParagraphChange::EndIndent));
                let first = LengthRow {
                    label: words.first_line_indent.clone(),
                    side: mark(GlyphLabel::text("¶→"), None, FieldLayout::Inside),
                    value: values.first_line_indent,
                    range: INDENT_RANGE,
                    presets: &[],
                    disabled: None,
                };
                let (event, _) = lengths.row(ui, first);
                edits.extend(number_change(event, ParagraphChange::FirstLineIndent));
            });
            Grid::new(&TWO_COLUMNS).fill_width().show(ui, |ui, _| {
                let before = LengthRow {
                    label: words.space_before.clone(),
                    side: mark(GlyphLabel::text("↑¶"), None, FieldLayout::Inside),
                    value: values.space_before,
                    range: SPACE_RANGE,
                    presets: &[],
                    disabled: None,
                };
                let (event, _) = lengths.row(ui, before);
                edits.extend(number_change(event, ParagraphChange::SpaceBefore));
                let after = LengthRow {
                    label: words.space_after.clone(),
                    side: mark(GlyphLabel::text("¶↓"), None, FieldLayout::Inside),
                    value: values.space_after,
                    range: SPACE_RANGE,
                    presets: &[],
                    disabled: None,
                };
                let (event, _) = lengths.row(ui, after);
                edits.extend(number_change(event, ParagraphChange::SpaceAfter));
            });
            if show_hyphenate {
                let out = Checkbox::new(Bind::given(mixed(values.hyphenate))).label(words.hyphenate.clone()).show(ui);
                edits.extend(out.events.map(ParagraphChange::Hyphenate));
            }
        });
        ParagraphControlsOutput { response: shown.output, edits }
    }
}

/// The Left, Center and Right toggles; with `vertical` they are the Top, Middle and Bottom ones, which set the same
/// [`TextAlign`] values.
pub(crate) fn align_toggles(values: &ParagraphValues, words: &ParagraphWords, vertical: bool) -> Vec<Toggle<ParagraphChange>> {
    let marks: [(TextAlign, &'static str, &Label); 3] = if vertical {
        [
            (TextAlign::Left, "text_align_top", &words.top),
            (TextAlign::Center, "text_align_middle", &words.middle),
            (TextAlign::Right, "text_align_bottom", &words.bottom),
        ]
    } else {
        [
            (TextAlign::Left, "align_left", &words.left),
            (TextAlign::Center, "align_center", &words.center),
            (TextAlign::Right, "align_right", &words.right),
        ]
    };
    marks
        .into_iter()
        .map(|(align, icon, name)| Toggle::new(name, icon, Some(values.align == Some(align)), Box::new(move |_| ParagraphChange::Align(align))))
        .collect()
}

/// The four justify toggles; in point text, which has no box to fill, they are disabled.
pub(crate) fn justify_toggles(values: &ParagraphValues, words: &ParagraphWords, point_text: bool) -> Vec<Toggle<ParagraphChange>> {
    [
        (TextAlign::JustifyLeft, &words.justify_left),
        (TextAlign::JustifyCenter, &words.justify_center),
        (TextAlign::JustifyRight, &words.justify_right),
        (TextAlign::JustifyAll, &words.justify_all),
    ]
    .into_iter()
    .map(|(align, name)| {
        let toggle = Toggle::new(name, "align_justify", Some(values.align == Some(align)), Box::new(move |_| ParagraphChange::Align(align)));
        if point_text && align.needs_box() { toggle.disabled(&words.justify_needs_box) } else { toggle }
    })
    .collect()
}

/// The three direction toggles.
pub(crate) fn direction_toggles(values: &ParagraphValues, words: &ParagraphWords) -> Vec<Toggle<ParagraphChange>> {
    std::iter::once(direction_toggle(values, ParagraphDirection::Automatic, "pilcrow", &words.direction_auto))
        .chain(explicit_toggles(values, words))
        .collect()
}

/// The Left to Right and Right to Left toggles: the row of [`DirectionRow::Explicit`].
fn explicit_toggles(values: &ParagraphValues, words: &ParagraphWords) -> Vec<Toggle<ParagraphChange>> {
    vec![
        direction_toggle(values, ParagraphDirection::LeftToRight, "pilcrow_right", &words.direction_ltr),
        direction_toggle(values, ParagraphDirection::RightToLeft, "pilcrow_left", &words.direction_rtl),
    ]
}

fn direction_toggle(values: &ParagraphValues, direction: ParagraphDirection, icon: &'static str, name: &Label) -> Toggle<ParagraphChange> {
    Toggle::new(name, icon, Some(values.direction == Some(direction)), Box::new(move |_| ParagraphChange::Direction(direction)))
}
