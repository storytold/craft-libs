//! [`ParagraphDialog`]: the Paragraph dialog of PowerPoint (Home › Paragraph launcher) and Word: alignment,
//! indentation (before text, and a first-line or hanging indent by an amount), spacing before and after, line
//! spacing, with a live sample of the result and a Tabs… button. OK applies everything as one change.
//!
//! The Paragraph controls ([`ParagraphControls`](super::ParagraphControls)) are Photoshop's panel; this is the productivity
//! apps' dialog over the same paragraph.
//!
//! ```ignore
//! dialogs.open(ParagraphDialog::new(format_at_caret).unit(unit));
//! if let Some(DialogResult::Ok(answer)) = dialogs.take::<ParagraphDialog>() {
//!     if answer.open_tabs { dialogs.open(TabStopsDialog::new(..)) } else { doc.apply_paragraph(answer.format) }
//! }
//! ```

use std::borrow::Cow;
use std::cell::Cell;

use crate::ui::length::{DocUnit, length_field, shown};
use craft_behavior::{Axis, EditEvent, Metrics, Mixed, NumberCodec, SelectionChange, keyed};
use craft_theme::variant::Tone;
use craft_ui::choice::{Checkbox, Radio, RadioGroup};
use craft_ui::contract::{
    Bind, CollectionEvent, Common, Component, Cx, Entry, FieldLayout, MeasureCx, PaintUi, Props as _, Role, Show as _, ShowCollection as _,
    ShowContainer as _, SideLabel,
};
use craft_ui::dialog::{
    ButtonPlace, Dialog, DialogId, DialogResult, DialogStack, HeldAnswer, HeldOk, StackDialog, dialog_id, serde_fields, set_serde_field,
};
use craft_ui::display::Text;
use craft_ui::form::{Field, FieldGroup, Help};
use craft_ui::layout_components::Stack;
use craft_ui::pickers::Select;
use craft_ui::primitives::TextRun;
use craft_ui::runtime::env::{Env, Label, UnitKind, Units};
use craft_ui::runtime::words::CommonWords;
use craft_ui::sections::{DialogRole, FooterButton, GroupBox};
use craft_ui::style::{ComponentStyle, Corners, StyleColor, StyleCx, SurfaceKind, TypeRole};
use craft_ui::tabs::{Tab, TabKind, TabStrip};
use craft_ui::text::NumberField;
use egui::{Rect, Response, Ui, Vec2, pos2, vec2};

craft_ui::words! {
    /// The dialog's words (DeckCraft's strings).
    #[non_exhaustive]
    pub struct ParagraphDialogWords {
        title: "paragraphDialog.title" = "Paragraph";
        general: "paragraphDialog.general" = "General";
        alignment: "paragraphDialog.alignment" = "Alignment";
        left: "paragraphDialog.left" = "Left";
        center: "paragraphDialog.center" = "Center";
        right: "paragraphDialog.right" = "Right";
        justify: "paragraphDialog.justify" = "Justify";
        justify_low: "paragraphDialog.justifyLow" = "Justify low";
        distributed: "paragraphDialog.distributed" = "Distributed";
        indentation: "paragraphDialog.indentation" = "Indentation";
        before_text: "paragraphDialog.beforeText" = "Before text";
        after_text: "paragraphDialog.afterText" = "After text";
        special: "paragraphDialog.special" = "Special";
        special_none: "paragraphDialog.specialNone" = "None";
        first_line: "paragraphDialog.firstLine" = "First line";
        hanging: "paragraphDialog.hanging" = "Hanging";
        by: "paragraphDialog.by" = "By";
        spacing: "paragraphDialog.spacing" = "Spacing";
        spacing_before: "paragraphDialog.spaceBefore" = "Spacing before";
        spacing_after: "paragraphDialog.spaceAfter" = "Spacing after";
        line_spacing: "paragraphDialog.lineSpacing" = "Line spacing";
        single: "paragraphDialog.single" = "Single";
        one_and_half: "paragraphDialog.oneAndHalf" = "1.5 lines";
        double: "paragraphDialog.double" = "Double";
        multiple: "paragraphDialog.multiple" = "Multiple";
        exactly: "paragraphDialog.exactly" = "Exactly";
        at: "paragraphDialog.at" = "At";
        preview: "paragraphDialog.preview" = "Preview";
        tabs: "paragraphDialog.tabs" = "Tabs…";
        at_least: "paragraphDialog.atLeast" = "At least";
        auto: "paragraphDialog.auto" = "Auto";
        sample: "paragraphDialog.sample" = "Sample Text";
        out_of_range: template "paragraphDialog.outOfRange" = "{field} must be from {min} to {max}";
        indents_and_spacing: "paragraphDialog.tabIndents" = "Indents and Spacing";
        line_and_page_breaks: "paragraphDialog.tabBreaks" = "Line and Page Breaks";
        asian_typography: "paragraphDialog.tabAsian" = "Asian Typography";
        direction: "paragraphDialog.direction" = "Direction:";
        left_to_right: "paragraphDialog.leftToRight" = "Left-to-right";
        right_to_left: "paragraphDialog.rightToLeft" = "Right-to-left";
        mirror_indents: "paragraphDialog.mirrorIndents" = "Mirror indents";
        contextual_spacing: "paragraphDialog.contextualSpacing" = "Don't add space between paragraphs of the same style";
        inside: "paragraphDialog.inside" = "Inside";
        outside: "paragraphDialog.outside" = "Outside";
        exceptions: "paragraphDialog.exceptions" = "Formatting exceptions";
        widow_control: "paragraphDialog.widowControl" = "Widow/Orphan control";
        keep_with_next: "paragraphDialog.keepWithNext" = "Keep with next";
        keep_lines_together: "paragraphDialog.keepLinesTogether" = "Keep lines together";
        page_break_before: "paragraphDialog.pageBreakBefore" = "Page break before";
        suppress_line_numbers: "paragraphDialog.suppressLineNumbers" = "Suppress line numbers";
        dont_hyphenate: "paragraphDialog.dontHyphenate" = "Don't hyphenate";
        line_breaking: "paragraphDialog.lineBreaking" = "Line breaking";
        character_spacing: "paragraphDialog.characterSpacing" = "Character Spacing";
        kinsoku: "paragraphDialog.kinsoku" = "Apply Asian line-breaking rules (kinsoku)";
        word_wrap: "paragraphDialog.wordWrap" = "Allow Latin text to wrap in the middle of a word";
        overflow_punct: "paragraphDialog.overflowPunct" = "Allow hanging punctuation";
        top_line_punct: "paragraphDialog.topLinePunct" = "Compress punctuation at the start of a line";
        auto_space_de: "paragraphDialog.autoSpaceDe" = "Add space between Asian and Latin text";
        auto_space_dn: "paragraphDialog.autoSpaceDn" = "Add space between Asian text and numbers";
    }
}

/// PowerPoint's alignments (Word has the first four).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum ParagraphAlign {
    /// Left.
    #[default]
    Left,
    /// Center.
    Center,
    /// Right.
    Right,
    /// Justify.
    Justify,
    /// Justify low.
    JustifyLow,
    /// Distributed.
    Distributed,
}

/// The first line against the others.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum SpecialIndent {
    /// None.
    #[default]
    None,
    /// The first line starts `by` further in.
    FirstLine,
    /// The other lines start `by` further in (the first line hangs).
    Hanging,
}

/// What a paragraph's spacing before or after counts, as DeckCraft keeps it (`spaceBeforeKind`
/// and `spaceAfterKind`, `pts` or `pct`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum SpacingKind {
    /// A distance in points.
    #[default]
    Points,
    /// A multiple of the line (DeckCraft's `pct`: 1 is a single line).
    Lines,
}

/// Line spacing.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum LineSpacing {
    /// The font's own line height; it replaces the `Multiple(0.0)` sentinel.
    Auto,
    /// A multiple of single spacing: Single is 1, 1.5 lines 1.5, Double 2, Multiple any other.
    Multiple(f32),
    /// A fixed distance, points.
    Exactly(f32),
    /// At least this distance, points (Word's At least).
    AtLeast(f32),
}

impl Default for LineSpacing {
    fn default() -> Self {
        LineSpacing::Multiple(1.0)
    }
}

/// Serde form of a row that is hidden (outer `None`), shown empty (`Some(None)`) or shown with a value: a hidden row has
/// no key and an empty one is `null`, so a round trip through JSON keeps the two apart.
mod shown_or_hidden {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub(super) fn serialize<T: Serialize, S: Serializer>(value: &Option<Option<T>>, serializer: S) -> Result<S::Ok, S::Error> {
        match value {
            Some(shown) => shown.serialize(serializer),
            None => serializer.serialize_none(),
        }
    }

    pub(super) fn deserialize<'de, T: Deserialize<'de>, D: Deserializer<'de>>(deserializer: D) -> Result<Option<Option<T>>, D::Error> {
        Option::<T>::deserialize(deserializer).map(Some)
    }
}

/// What the dialog edits; `None` where the selected paragraphs differ (the field starts empty
/// and a field left empty is left as it was).
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct ParagraphFormat {
    /// The align.
    pub align: Option<ParagraphAlign>,
    /// Points.
    pub before_text: Option<f32>,
    /// Word's right indent ("After text"). The outer `None` hides the field (PowerPoint has
    /// none); `Some(None)` shows it empty, for paragraphs that differ.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "shown_or_hidden")]
    pub after_text: Option<Option<f32>>,
    /// The special first-line indent: none, first line or hanging.
    pub special: Option<SpecialIndent>,
    /// Size of the special indent, in points.
    pub by: Option<f32>,
    /// Points, or lines when [`space_before_kind`](Self::space_before_kind) says so.
    pub space_before: Option<f32>,
    /// Points, or lines when [`space_after_kind`](Self::space_after_kind) says so.
    pub space_after: Option<f32>,
    /// What `space_before` counts. The dialog keeps the kind while the value is the one it
    /// opened with and answers [`Points`](SpacingKind::Points) once the value changes (DeckCraft
    /// `dialogs/paragraph.rs:33`: the kind is kept while the text equals its seed).
    pub space_before_kind: SpacingKind,
    /// What `space_after` counts, kept as `space_before_kind` is.
    pub space_after_kind: SpacingKind,
    /// The line spacing rule and its value.
    pub line_spacing: Option<LineSpacing>,
    /// The paragraph's direction. The outer `None` hides the row; `Some(None)` shows it empty, for paragraphs that
    /// differ.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "shown_or_hidden")]
    pub direction: Option<Option<super::ParagraphDirection>>,
    /// Mirror indents: Left and Right read Inside and Outside when on. The outer `None` hides it; `Some(None)` is mixed.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "shown_or_hidden")]
    pub mirror_indents: Option<Option<bool>>,
    /// Don't add space between paragraphs of the same style (Word's contextual spacing): the spacing before and after
    /// is left out between two paragraphs of one style. The outer `None` hides it; `Some(None)` is mixed.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "shown_or_hidden")]
    pub contextual_spacing: Option<Option<bool>>,
    /// The Line and Page Breaks check boxes. `None` hides the page.
    pub breaks: Option<LineBreakOptions>,
    /// The Asian Typography check boxes. `None` hides the page.
    pub asian: Option<AsianTypography>,
}

/// The Line and Page Breaks check boxes of Word's Paragraph dialog; `None` is a box whose paragraphs differ, shown
/// mixed, and a click makes it checked.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LineBreakOptions {
    /// Widow/orphan control.
    pub widow_control: Option<bool>,
    /// Keep with next.
    pub keep_with_next: Option<bool>,
    /// Keep lines together.
    pub keep_lines_together: Option<bool>,
    /// Page break before.
    pub page_break_before: Option<bool>,
    /// Suppress line numbers.
    pub suppress_line_numbers: Option<bool>,
    /// Don't hyphenate.
    pub dont_hyphenate: Option<bool>,
}

/// The Asian Typography check boxes of Word's Paragraph dialog, mixed as [`LineBreakOptions`] is.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AsianTypography {
    /// Apply East Asian line-break rules (kinsoku).
    pub kinsoku: Option<bool>,
    /// Allow Latin text to wrap in the middle of a word.
    pub word_wrap: Option<bool>,
    /// Allow punctuation to hang past the margin.
    pub overflow_punct: Option<bool>,
    /// Allow punctuation at the start of a line.
    pub top_line_punct: Option<bool>,
    /// Automatically space East Asian and Latin text.
    pub auto_space_de: Option<bool>,
    /// Automatically space East Asian text and numbers.
    pub auto_space_dn: Option<bool>,
}

/// The page of Word's Paragraph dialog the dialog opens on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum ParagraphTab {
    /// Indents and Spacing.
    #[default]
    IndentsAndSpacing,
    /// Line and Page Breaks.
    LineAndPageBreaks,
    /// Asian Typography.
    AsianTypography,
}

/// OK's answer: the format, and whether Tabs… was pressed (the app applies the format, then opens
/// the Tabs dialog, and after it the Paragraph dialog shows what was applied).
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct ParagraphAnswer {
    /// The format as the user left it.
    pub format: ParagraphFormat,
    /// Whether Tabs… was pressed.
    pub open_tabs: bool,
}

/// The Paragraph dialog.
///
/// - Alignment, Special (None, First line, Hanging) and Line spacing (Single, 1.5 lines, Double, Multiple, Exactly;
///   Word adds At least) are `Select`s; Before text, After text and By are number fields in the dialog's unit,
///   Spacing before and after number fields in points (PowerPoint and Word both show points there); At is a number
///   field (lines or points, by the line spacing). Choosing Exactly with an At of 9.99 or less sets 18 pt; choosing a
///   multiple with an At above 9.99 sets 1 (DeckCraft).
/// - Lengths in the dialog's unit (inches by default, as PowerPoint). Typed numbers are clamped by their field; text
///   that is not a number is not taken (the field marks it, and OK waits until it is fixed or left). Esc in a field's
///   gesture restores the value the gesture began from. A format the app hands over with a value out of range shows
///   the range message beside its field and OK waits until it is fixed.
/// - Preview (Word's; DeckCraft has none): three sample paragraphs, the outer two grey and the middle one in the text
///   colour, laid out with the values as they are now (Word's Preview box), redrawn as fields change. A typographic
///   point is a point of the screen there.
/// - Keys: Enter is OK, Esc Cancel.
#[derive(craft_ui::contract::Pattern)]
pub struct ParagraphDialog {
    format: ParagraphFormat,
    unit: DocUnit,
    word: bool,
    /// The format the dialog opened with: a spacing that returns to its value takes its kind back.
    opened: ParagraphFormat,
    tab: ParagraphTab,
    /// Whether OK files its answer for the app to judge instead of closing.
    hold: bool,
    held: HeldAnswer<ParagraphAnswer>,
}

impl ParagraphDialog {
    /// A dialog editing `format`, with lengths in inches.
    pub fn new(format: ParagraphFormat) -> Self {
        ParagraphDialog {
            opened: format.clone(),
            format,
            unit: DocUnit::new(UnitKind::Inches, UnitKind::POINTS_PER_INCH),
            word: false,
            tab: ParagraphTab::default(),
            hold: false,
            held: HeldAnswer::new(),
        }
    }

    /// Opens the dialog on `tab`; Word's form with line-break and Asian typography options shows the three pages.
    #[must_use]
    pub fn tab(mut self, tab: ParagraphTab) -> Self {
        self.tab = tab;
        self
    }

    /// Sets the unit the length fields show.
    #[must_use]
    pub fn unit(mut self, unit: DocUnit) -> Self {
        self.unit = unit;
        self
    }

    /// Word's form: four alignments, and At least among the line spacings.
    #[must_use]
    pub fn word(mut self, word: bool) -> Self {
        self.word = word;
        self
    }

    /// The format as edited so far.
    pub fn format(&self) -> &ParagraphFormat {
        &self.format
    }
}

/// The largest length Word takes, points.
const MAX_POINTS: f32 = 1584.0;
/// PowerPoint's switch between a multiple and a distance in At.
const MULTIPLE_LIMIT: f32 = 9.99;
/// The distance Exactly and At least start at, points.
const DEFAULT_DISTANCE: f32 = 18.0;
/// The indent a first-line or hanging indent starts at, points (Word's half inch).
const DEFAULT_BY: f32 = 36.0;
/// The most a multiple of single spacing can be (Word).
const MAX_MULTIPLE: f32 = 132.0;
/// The least a multiple of single spacing can be (Word).
const MIN_MULTIPLE: f32 = 0.06;
/// The least an exact distance can be, points (Word).
const MIN_EXACT: f32 = 0.7;
/// The most spacing before or after of kind lines (DeckCraft `write_spacing` clamps `pct` to 10).
const MAX_SPACING_LINES: f32 = 10.0;
/// Decimals the At field shows.
const AT_DECIMALS: usize = 2;
/// Lines in each sample paragraph of the preview.
const SAMPLE_LINES: usize = 3;
/// How many samples wide the preview is when its room is not known.
const PREVIEW_SAMPLES: f32 = 6.0;
/// The index of the OK button among the dialog's buttons.
const OK: usize = 0;
/// The index of the Tabs… button among the dialog's buttons.
const TABS: usize = 2;
/// The index of the Cancel button among the dialog's buttons.
const CANCEL: usize = 1;

/// The entries of Line spacing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LinePreset {
    Auto,
    Single,
    OneAndHalf,
    Double,
    Multiple,
    AtLeast,
    Exactly,
}

impl LinePreset {
    fn of(spacing: LineSpacing) -> Self {
        match spacing {
            LineSpacing::Auto => LinePreset::Auto,
            LineSpacing::Multiple(1.0) => LinePreset::Single,
            LineSpacing::Multiple(1.5) => LinePreset::OneAndHalf,
            LineSpacing::Multiple(2.0) => LinePreset::Double,
            LineSpacing::Multiple(_) => LinePreset::Multiple,
            LineSpacing::AtLeast(_) => LinePreset::AtLeast,
            LineSpacing::Exactly(_) => LinePreset::Exactly,
        }
    }

    /// The spacing choosing this entry gives when `current` was set.
    fn choose(self, current: Option<LineSpacing>) -> LineSpacing {
        // An Auto current has no amount, so the fallbacks apply.
        let at = current.and_then(at_value);
        let distance = at.filter(|x| *x > MULTIPLE_LIMIT).unwrap_or(DEFAULT_DISTANCE);
        match self {
            LinePreset::Auto => LineSpacing::Auto,
            LinePreset::Single => LineSpacing::Multiple(1.0),
            LinePreset::OneAndHalf => LineSpacing::Multiple(1.5),
            LinePreset::Double => LineSpacing::Multiple(2.0),
            LinePreset::Multiple => LineSpacing::Multiple(at.filter(|x| *x <= MULTIPLE_LIMIT).unwrap_or(1.0)),
            LinePreset::AtLeast => LineSpacing::AtLeast(distance),
            LinePreset::Exactly => LineSpacing::Exactly(distance),
        }
    }
}

/// The values a spacing before or after of `kind` can take: points up to Word's largest length, lines up to the most
/// DeckCraft keeps.
fn spacing_bounds(kind: SpacingKind) -> (f32, f32) {
    match kind {
        SpacingKind::Points => (0.0, MAX_POINTS),
        SpacingKind::Lines => (0.0, MAX_SPACING_LINES),
    }
}

/// The bounds of At for a line spacing, in lines or points.
fn at_bounds(spacing: LineSpacing) -> (f32, f32) {
    match spacing {
        // Auto has no At: the bounds of no spacing.
        LineSpacing::Auto => (0.0, MAX_POINTS),
        LineSpacing::Multiple(_) => (MIN_MULTIPLE, MAX_MULTIPLE),
        LineSpacing::Exactly(_) => (MIN_EXACT, MAX_POINTS),
        LineSpacing::AtLeast(_) => (0.0, MAX_POINTS),
    }
}

/// The amount At shows for a line spacing; Auto has none.
fn at_value(spacing: LineSpacing) -> Option<f32> {
    match spacing {
        LineSpacing::Auto => None,
        LineSpacing::Multiple(x) | LineSpacing::Exactly(x) | LineSpacing::AtLeast(x) => Some(x),
    }
}

fn with_at(spacing: LineSpacing, at: f32) -> LineSpacing {
    match spacing {
        LineSpacing::Auto => LineSpacing::Auto,
        LineSpacing::Multiple(_) => LineSpacing::Multiple(at),
        LineSpacing::Exactly(_) => LineSpacing::Exactly(at),
        LineSpacing::AtLeast(_) => LineSpacing::AtLeast(at),
    }
}

/// The fields that can be out of range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Row {
    BeforeText,
    AfterText,
    By,
    SpaceBefore,
    SpaceAfter,
    At,
}

/// What a number field counts: a length shown in a unit, or a plain number of lines.
#[derive(Clone, Copy)]
enum Count {
    Length(DocUnit),
    Lines,
}

/// What a pass of the dialog reads from its scope.
struct Scope<'s> {
    words: &'s ParagraphDialogWords,
    units: &'s Units,
}

impl ParagraphDialog {
    /// How `row` counts its number.
    fn count(&self, row: Row) -> Count {
        let f = &self.format;
        match row {
            Row::BeforeText | Row::AfterText | Row::By => Count::Length(self.unit),
            Row::SpaceBefore => Self::spacing_count(f.space_before_kind),
            Row::SpaceAfter => Self::spacing_count(f.space_after_kind),
            Row::At if matches!(f.line_spacing, Some(LineSpacing::Multiple(_))) => Count::Lines,
            Row::At => Count::Length(DocUnit::points()),
        }
    }

    fn spacing_count(kind: SpacingKind) -> Count {
        match kind {
            SpacingKind::Points => Count::Length(DocUnit::points()),
            SpacingKind::Lines => Count::Lines,
        }
    }

    /// The values `row` can take, in points or lines.
    fn bounds(&self, row: Row) -> (f32, f32) {
        let f = &self.format;
        match row {
            Row::BeforeText | Row::AfterText => (-MAX_POINTS, MAX_POINTS),
            Row::By => (0.0, MAX_POINTS),
            Row::SpaceBefore => spacing_bounds(f.space_before_kind),
            Row::SpaceAfter => spacing_bounds(f.space_after_kind),
            Row::At => f.line_spacing.map_or((0.0, MAX_POINTS), at_bounds),
        }
    }

    /// Whether the row's field can be edited.
    fn enabled(&self, row: Row) -> bool {
        match row {
            Row::By => matches!(self.format.special, Some(SpecialIndent::FirstLine | SpecialIndent::Hanging)),
            Row::At => matches!(self.format.line_spacing, Some(l) if at_value(l).is_some()),
            _ => true,
        }
    }

    /// The rows whose value is outside its bounds.
    fn out_of_range(&self) -> Vec<Row> {
        [Row::BeforeText, Row::AfterText, Row::By, Row::SpaceBefore, Row::SpaceAfter, Row::At]
            .into_iter()
            .filter(|row| {
                let (low, high) = self.bounds(*row);
                self.value(*row).is_some_and(|value| !(low..=high).contains(&value))
            })
            .collect()
    }

    fn value(&self, row: Row) -> Option<f32> {
        let f = &self.format;
        match row {
            Row::BeforeText => f.before_text,
            Row::AfterText => f.after_text.flatten(),
            Row::By => f.by,
            Row::SpaceBefore => f.space_before,
            Row::SpaceAfter => f.space_after,
            Row::At => f.line_spacing.and_then(at_value),
        }
    }

    fn set_value(&mut self, row: Row, value: Option<f32>) {
        let f = &mut self.format;
        match row {
            Row::BeforeText => f.before_text = value,
            Row::AfterText => f.after_text = Some(value),
            Row::By => f.by = value,
            Row::SpaceBefore => {
                f.space_before = value;
                f.space_before_kind = if value == self.opened.space_before { self.opened.space_before_kind } else { SpacingKind::Points };
            }
            Row::SpaceAfter => {
                f.space_after = value;
                f.space_after_kind = if value == self.opened.space_after { self.opened.space_after_kind } else { SpacingKind::Points };
            }
            // An emptied At keeps the spacing's number.
            Row::At => {
                f.line_spacing = f.line_spacing.map(|l| match value.or_else(|| at_value(l)) {
                    Some(at) => with_at(l, at),
                    None => l,
                })
            }
        }
    }

    /// Applies a field's event, in points or lines, to the row's value: every `Update` shows in the dialog, and
    /// Escape (`Cancel`) restores the value from before the gesture.
    fn apply(&mut self, row: Row, event: &EditEvent<Option<f32>>) {
        let mut value = self.value(row).map_or(Mixed::Mixed, |value| Mixed::Same(Some(value)));
        event.apply_mixed(&mut value);
        self.set_value(row, value.same().copied().flatten());
    }

    /// The message beside a field whose value is out of range, its bounds in the locale's digits.
    fn message(&self, scope: &Scope<'_>, row: Row, caption: &str) -> String {
        let (low, high) = self.bounds(row);
        let show = |bound: f32| match self.count(row) {
            Count::Length(unit) => scope.units.show(unit.kind, shown(scope.units, unit, f64::from(bound))),
            Count::Lines => scope.units.number(f64::from(bound), 0..=AT_DECIMALS),
        };
        scope.words.out_of_range.fill(&[("field", caption), ("min", &show(low)), ("max", &show(high))])
    }

    /// One number field of the form, under `caption`: edits go to the row's value; a rejected text or an out of range
    /// value makes `valid` false.
    fn field(&mut self, ui: &mut Ui, scope: &Scope<'_>, row: Row, caption: &Label, valid: &Cell<bool>) {
        let count = self.count(row);
        let (low, high) = self.bounds(row);
        let enabled = self.enabled(row);
        let shown = self.value(row).map_or(Mixed::Mixed, |value| Mixed::Same(Some(f64::from(value))));
        let mut field = Field::new(caption.clone());
        if self.out_of_range().contains(&row) {
            field = field.help(Help::Status(Tone::Bad, self.message(scope, row, caption.as_str()).into()));
        }
        let units = scope.units;
        let shown_field = field.show(ui, |ui, _| {
            let out = match count {
                Count::Length(unit) => length_field(unit, caption.clone(), shown, f64::from(low)..=f64::from(high)).enabled(enabled).show(ui),
                Count::Lines => {
                    // No range: a number typed here replaces the lines with points, whatever it is.
                    let codec = NumberCodec::new(units.reader(None), AT_DECIMALS);
                    NumberField::new(caption.clone(), Bind::given(shown)).codec(codec).enabled(enabled).show(ui)
                }
            };
            (out.edit.events, out.edit.unparsed)
        });
        let Some((events, unparsed)) = shown_field.inner else { return };
        if unparsed {
            valid.set(false);
        }
        for event in events {
            let event = match count {
                Count::Length(_) | Count::Lines => event,
            };
            self.apply(row, &event.map(|value| value.map(|value| value as f32)));
        }
    }

    /// Whether the dialog shows Word's three pages.
    fn paged(&self) -> bool {
        self.word && self.format.breaks.is_some() && self.format.asian.is_some()
    }

    fn body(&mut self, ui: &mut Ui, scope: &Scope<'_>, valid: &Cell<bool>) {
        let words = scope.words;
        valid.set(true);
        let paged = self.paged();
        if paged {
            self.tab_strip(ui, words);
        }
        match self.tab {
            ParagraphTab::LineAndPageBreaks if paged => self.breaks_page(ui, words),
            ParagraphTab::AsianTypography if paged => self.asian_page(ui, words),
            _ => self.indents_page(ui, scope, valid),
        }
        if !self.out_of_range().is_empty() {
            valid.set(false);
        }
    }

    fn indents_page(&mut self, ui: &mut Ui, scope: &Scope<'_>, valid: &Cell<bool>) {
        let words = scope.words;
        FieldGroup::new(words.title.clone()).show(ui, |ui, _| {
            GroupBox::new(words.general.clone(), None).show(ui, |ui, _| {
                self.direction(ui, words);
                self.alignment(ui, words);
            });
            GroupBox::new(words.indentation.clone(), None).show(ui, |ui, _| {
                self.indentation(ui, scope, valid);
                self.mirror(ui, words);
            });
            GroupBox::new(words.spacing.clone(), None).show(ui, |ui, _| self.spacing(ui, scope, valid));
            GroupBox::new(words.preview.clone(), None).show(ui, |ui, _| {
                ParagraphPreview::new(words.preview.clone(), &self.format, words.sample.as_str()).show(ui);
            });
        });
    }

    fn tab_strip(&mut self, ui: &mut Ui, words: &ParagraphDialogWords) {
        let tabs = [
            (ParagraphTab::IndentsAndSpacing, words.indents_and_spacing.as_str().to_owned()),
            (ParagraphTab::LineAndPageBreaks, words.line_and_page_breaks.as_str().to_owned()),
            (ParagraphTab::AsianTypography, words.asian_typography.as_str().to_owned()),
        ];
        let source = keyed(&tabs[..], |tab: &(ParagraphTab, String)| tab.0).with_text(|tab| Cow::Borrowed(tab.1.as_str()));
        let strip = TabStrip::new(words.title.clone()).kind(TabKind::Text).shown(self.tab).natural_height(true);
        let out = strip
            .show_items(ui, &source, &mut |key, _| Tab::new(tabs.iter().find(|tab| tab.0 == *key).map_or_else(String::new, |tab| tab.1.clone())));
        for event in out.response.events {
            if let CollectionEvent::Selection { change: SelectionChange::Set(keys), .. } = event
                && let Some(chosen) = keys.last()
            {
                self.tab = *chosen;
            }
        }
    }

    /// Left to right and Right to left; a paragraph with another or differing direction leaves both unchecked.
    fn direction(&mut self, ui: &mut Ui, words: &ParagraphDialogWords) {
        let Some(current) = self.format.direction else { return };
        let before = current.filter(|direction| matches!(direction, super::ParagraphDirection::LeftToRight | super::ParagraphDirection::RightToLeft));
        let mut picked = before;
        RadioGroup::new(words.direction.clone(), Bind::bound(&mut picked)).show(ui, |ui, _| {
            Stack::new().show(ui, |ui, cx| {
                Text::new(words.direction.clone()).show(ui);
                let options =
                    [(super::ParagraphDirection::LeftToRight, &words.left_to_right), (super::ParagraphDirection::RightToLeft, &words.right_to_left)];
                for (at, (direction, label)) in options.into_iter().enumerate() {
                    cx.labelled(ui, at, label.clone(), SideLabel::new().layout(FieldLayout::After), Radio::new(Some(direction)));
                }
            });
        });
        if picked != before
            && let Some(direction) = picked
        {
            self.format.direction = Some(Some(direction));
        }
    }

    fn mirror(&mut self, ui: &mut Ui, words: &ParagraphDialogWords) {
        let Some(current) = self.format.mirror_indents else { return };
        if let Some(on) = check(ui, &words.mirror_indents, current) {
            self.format.mirror_indents = Some(Some(on));
        }
    }

    fn breaks_page(&mut self, ui: &mut Ui, words: &ParagraphDialogWords) {
        let Some(breaks) = self.format.breaks.as_mut() else { return };
        GroupBox::new(words.line_and_page_breaks.clone(), None).show(ui, |ui, _| {
            toggle(ui, &words.widow_control, &mut breaks.widow_control);
            toggle(ui, &words.keep_with_next, &mut breaks.keep_with_next);
            toggle(ui, &words.keep_lines_together, &mut breaks.keep_lines_together);
            toggle(ui, &words.page_break_before, &mut breaks.page_break_before);
        });
        GroupBox::new(words.exceptions.clone(), None).show(ui, |ui, _| {
            toggle(ui, &words.suppress_line_numbers, &mut breaks.suppress_line_numbers);
            toggle(ui, &words.dont_hyphenate, &mut breaks.dont_hyphenate);
        });
    }

    fn asian_page(&mut self, ui: &mut Ui, words: &ParagraphDialogWords) {
        let Some(asian) = self.format.asian.as_mut() else { return };
        GroupBox::new(words.line_breaking.clone(), None).show(ui, |ui, _| {
            toggle(ui, &words.kinsoku, &mut asian.kinsoku);
            toggle(ui, &words.word_wrap, &mut asian.word_wrap);
            toggle(ui, &words.overflow_punct, &mut asian.overflow_punct);
        });
        GroupBox::new(words.character_spacing.clone(), None).show(ui, |ui, _| {
            toggle(ui, &words.top_line_punct, &mut asian.top_line_punct);
            toggle(ui, &words.auto_space_de, &mut asian.auto_space_de);
            toggle(ui, &words.auto_space_dn, &mut asian.auto_space_dn);
        });
    }

    fn alignment(&mut self, ui: &mut Ui, words: &ParagraphDialogWords) {
        let every = [
            (ParagraphAlign::Left, &words.left),
            (ParagraphAlign::Center, &words.center),
            (ParagraphAlign::Right, &words.right),
            (ParagraphAlign::Justify, &words.justify),
            (ParagraphAlign::JustifyLow, &words.justify_low),
            (ParagraphAlign::Distributed, &words.distributed),
        ];
        let entries: Vec<Entry<ParagraphAlign>> = every
            .into_iter()
            .filter(|(value, _)| {
                let word_only = matches!(value, ParagraphAlign::JustifyLow | ParagraphAlign::Distributed);
                !self.word || !word_only || self.format.align == Some(*value)
            })
            .map(|(value, text)| Entry::new(value, text.clone()))
            .collect();
        let current = self.format.align.map_or(Mixed::Mixed, Mixed::Same);
        let picked = Field::new(words.alignment.clone())
            .show(ui, |ui, _| Select::new(words.alignment.clone(), Bind::given(current), &entries).show(ui).value().copied())
            .inner
            .flatten();
        if let Some(align) = picked {
            self.format.align = Some(align);
        }
    }

    fn indentation(&mut self, ui: &mut Ui, scope: &Scope<'_>, valid: &Cell<bool>) {
        let words = scope.words;
        // Mirrored indents are measured from the inside and outside edges.
        let mirrored = self.format.mirror_indents == Some(Some(true));
        let before = if mirrored { &words.inside } else { &words.before_text };
        self.field(ui, scope, Row::BeforeText, before, valid);
        if self.format.after_text.is_some() {
            let after = if mirrored { &words.outside } else { &words.after_text };
            self.field(ui, scope, Row::AfterText, after, valid);
        }
        let entries = [
            Entry::new(SpecialIndent::None, words.special_none.clone()),
            Entry::new(SpecialIndent::FirstLine, words.first_line.clone()),
            Entry::new(SpecialIndent::Hanging, words.hanging.clone()),
        ];
        let current = self.format.special.map_or(Mixed::Mixed, Mixed::Same);
        let picked = Field::new(words.special.clone())
            .show(ui, |ui, _| Select::new(words.special.clone(), Bind::given(current), &entries).show(ui).value().copied())
            .inner
            .flatten();
        if let Some(special) = picked.filter(|special| Some(*special) != self.format.special) {
            self.format.special = Some(special);
            if matches!(special, SpecialIndent::FirstLine | SpecialIndent::Hanging) && self.format.by.is_none_or(|by| by == 0.0) {
                self.format.by = Some(DEFAULT_BY);
            }
        }
        self.field(ui, scope, Row::By, &words.by, valid);
    }

    fn spacing(&mut self, ui: &mut Ui, scope: &Scope<'_>, valid: &Cell<bool>) {
        let words = scope.words;
        self.field(ui, scope, Row::SpaceBefore, &words.spacing_before, valid);
        self.field(ui, scope, Row::SpaceAfter, &words.spacing_after, valid);

        let current = self.format.line_spacing.map(LinePreset::of);
        let mut entries = Vec::new();
        // Auto is the text's own spacing: offered only to a format that opened with it, so it can be chosen back.
        if self.opened.line_spacing == Some(LineSpacing::Auto) {
            entries.push(Entry::new(LinePreset::Auto, words.auto.clone()));
        }
        entries.extend([
            Entry::new(LinePreset::Single, words.single.clone()),
            Entry::new(LinePreset::OneAndHalf, words.one_and_half.clone()),
            Entry::new(LinePreset::Double, words.double.clone()),
            Entry::new(LinePreset::Multiple, words.multiple.clone()),
        ]);
        if self.word || current == Some(LinePreset::AtLeast) {
            entries.push(Entry::new(LinePreset::AtLeast, words.at_least.clone()));
        }
        entries.push(Entry::new(LinePreset::Exactly, words.exactly.clone()));
        let picked = Field::new(words.line_spacing.clone())
            .show(ui, |ui, _| {
                Select::new(words.line_spacing.clone(), Bind::given(current.map_or(Mixed::Mixed, Mixed::Same)), &entries).show(ui).value().copied()
            })
            .inner
            .flatten();
        if let Some(chosen) = picked.filter(|chosen| Some(*chosen) != current) {
            self.format.line_spacing = Some(chosen.choose(self.format.line_spacing));
        }

        self.field(ui, scope, Row::At, &words.at, valid);
        if let Some(current) = self.format.contextual_spacing
            && let Some(on) = check(ui, &words.contextual_spacing, current)
        {
            self.format.contextual_spacing = Some(Some(on));
        }
    }

    fn answer(&self, open_tabs: bool) -> ParagraphAnswer {
        ParagraphAnswer { format: self.format.clone(), open_tabs }
    }
}

/// A check box over a value that may differ between paragraphs: it shows mixed while `None`, and a click makes it
/// `Some`. Returns the click's value.
fn check(ui: &mut Ui, label: &Label, value: Option<bool>) -> Option<bool> {
    let shown = value.map_or(Mixed::Mixed, Mixed::Same);
    Checkbox::new(Bind::given(shown)).label(label.clone()).show(ui).value().copied()
}

/// [`check`] writing a click into `slot`.
fn toggle(ui: &mut Ui, label: &Label, slot: &mut Option<bool>) {
    if let Some(on) = check(ui, label, *slot) {
        *slot = Some(on);
    }
}

/// The dialog's values as automation sees them: the format's fields and the page shown.
#[derive(serde::Serialize, serde::Deserialize)]
struct JsonFields {
    #[serde(flatten)]
    format: ParagraphFormat,
    tab: ParagraphTab,
}

impl ParagraphDialog {
    /// OK files its answer for the app to accept or refuse before the dialog closes.
    #[must_use]
    pub fn held_ok(mut self) -> Self {
        self.hold = true;
        self
    }

    /// The answer OK filed, once, for the app to judge.
    pub fn take_ok(&mut self) -> Option<ParagraphAnswer> {
        self.held.take()
    }

    /// The app's answer to the filed answer: `Ok` closes the dialog with it, `Err` shows the reason and enables OK.
    pub fn answer_ok(&mut self, answer: Result<(), Label>) {
        self.held.answer(answer);
    }

    /// Files `answer` and keeps the dialog open when OK is held; a plain dialog closes with it.
    fn settle(&mut self, answer: ParagraphAnswer) -> Option<ParagraphAnswer> {
        if !self.hold {
            return Some(answer);
        }
        if !self.held.is_waiting() {
            self.held.file(answer);
        }
        None
    }
}

impl StackDialog for ParagraphDialog {
    type Result = ParagraphAnswer;

    fn id(&self) -> DialogId {
        dialog_id("paragraph")
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<ParagraphAnswer>> {
        if let Some(answer) = self.held.finish() {
            return Some(DialogResult::Ok(answer));
        }
        let env = Env::of(ui);
        let words = env.words::<ParagraphDialogWords>();
        let common = env.words::<CommonWords>();
        let units = env.units();
        let scope = Scope { words: &words, units: &units };
        let valid = Cell::new(self.out_of_range().is_empty());
        let waiting = self.held.is_waiting();
        let refused = self.held.refusal().map(|(reason, answer)| (reason.clone(), answer.format.clone()));
        let unchanged = Cell::new(true);
        let buttons = [
            FooterButton::new(common.ok.clone(), DialogRole::Accept).stays_open(self.hold),
            FooterButton::new(common.cancel.clone(), DialogRole::Cancel),
            FooterButton::new(words.tabs.clone(), DialogRole::Other).stays_open(self.hold),
        ];
        let shown = Dialog::new(words.title.clone())
            .button_place(ButtonPlace::Side)
            .buttons(buttons)
            .settle_buttons(|_, buttons| {
                for (at, button) in buttons.iter_mut().enumerate() {
                    if at != CANCEL {
                        button.set_enabled(valid.get() && !waiting);
                    }
                }
            })
            .refusal(|| refused.as_ref().filter(|_| unchanged.get()).map(|(reason, _)| reason.clone()))
            .show(ui, |ui, _| {
                self.body(ui, &scope, &valid);
                if let Some((_, format)) = &refused {
                    unchanged.set(*format == self.format);
                }
            });
        if refused.is_some() && !unchanged.get() {
            self.held.clear_refusal();
        }
        let tabs = shown.output.picked == Some(TABS);
        // A held OK (or Tabs…) stays open: its pick files the answer, and only Cancel closes.
        if self.hold && matches!(shown.output.picked, Some(OK | TABS)) {
            if !valid.get() {
                return None;
            }
            let answer = self.answer(tabs);
            return self.settle(answer).map(DialogResult::Ok);
        }
        match shown.output.result(|| tabs) {
            Some(DialogResult::Ok(open_tabs)) if valid.get() => {
                let answer = self.answer(open_tabs);
                self.settle(answer).map(DialogResult::Ok)
            }
            Some(DialogResult::Cancel) => Some(DialogResult::Cancel),
            _ => None,
        }
    }

    fn confirm(&mut self, _: &egui::Context) -> Option<ParagraphAnswer> {
        if let Some(answer) = self.held.finish() {
            return Some(answer);
        }
        if !self.out_of_range().is_empty() {
            return None;
        }
        let answer = self.answer(false);
        self.settle(answer)
    }

    fn held_state(&self) -> Option<HeldOk> {
        self.held.state()
    }

    fn json_fields(&self) -> Option<serde_json::Value> {
        serde_fields(&JsonFields { format: self.format.clone(), tab: self.tab })
    }

    fn set_json_field(&mut self, _: &egui::Context, field: &str, value: serde_json::Value) -> Result<(), String> {
        let mut fields = JsonFields { format: self.format.clone(), tab: self.tab };
        set_serde_field(&mut fields, field, value)?;
        self.format = fields.format;
        self.tab = fields.tab;
        self.held.clear_refusal();
        Ok(())
    }
}

/// The preview's style: a sunken well holding the sample lines, the paragraph being edited in the text colour and the
/// ones around it muted.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct ParagraphPreviewStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The edited paragraph's lines.
    subject: StyleColor,
    /// The lines around it.
    context: StyleColor,
    /// Between the well's edge and the lines.
    pad: f32,
}

/// The paragraph preview: three sample paragraphs laid out with the format's indents, spacing and line spacing, as
/// Word's dialog draws them. The width is the room offered; the height follows from the lines.
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
pub struct ParagraphPreview {
    #[common]
    common: Common,
    format: ParagraphFormat,
    sample: String,
}

impl ParagraphPreview {
    /// A preview named `label` of `format`, its lines made of `sample`.
    pub fn new(label: impl Into<Label>, format: &ParagraphFormat, sample: impl Into<String>) -> Self {
        ParagraphPreview { common: Common::new(), format: format.clone(), sample: sample.into() }.label(label)
    }
}

impl Component for ParagraphPreview {
    const ROLE: Role = Role::IMAGE;
    const OVERLAYS: bool = false;
    type Style = ParagraphPreviewStyle;
    type Output = Response;

    fn style(cx: &StyleCx<'_>, _: ParagraphPreviewStyleKey) -> ParagraphPreviewStyle {
        let colors = &cx.tokens.color;
        ParagraphPreviewStyle {
            metrics: Metrics::new(),
            subject: cx.color(colors.text.normal),
            context: cx.color(colors.text.muted),
            pad: cx.tokens.space.gap.row,
        }
    }

    fn fills(&self) -> Option<Axis> {
        Some(Axis::X)
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let one = cx.layout(&TextRun::new(self.sample.as_str()).role(TypeRole::Body)).size();
        let pad = cx.style.pad;
        let width = cx.room.filter(|room| room.is_finite()).unwrap_or(one.x * PREVIEW_SAMPLES);
        let plan = sample_plan(&self.format, (width - 2.0 * pad).max(0.0), one.y);
        Vec2::new(width, plan.height + 2.0 * pad)
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let style = cx.style;
        let draw = ui.draw();
        draw.surface(cx.rect, SurfaceKind::Sunken, Corners::All);
        let inner = cx.rect.shrink(style.pad);
        let run = |text: String, color: StyleColor| TextRun::new(text).role(TypeRole::Body).color(color);
        let one = draw.layout(&run(self.sample.clone(), style.subject), f32::INFINITY).size();
        let plan = sample_plan(&self.format, inner.width(), one.y);
        for row in &plan.rows {
            let slot = row.rect.translate(inner.min.to_vec2());
            let color = if row.subject { style.subject } else { style.context };
            let text =
                if row.full { format!("{} ", self.sample).repeat((slot.width() / one.x.max(1.0)).ceil() as usize + 1) } else { self.sample.clone() };
            let x = match row.last_align {
                ParagraphAlign::Center if !row.full => slot.center().x - one.x / 2.0,
                ParagraphAlign::Right if !row.full => slot.right() - one.x,
                _ => slot.left(),
            };
            let at = Rect::from_min_size(pos2(x, slot.top()), vec2((slot.right() - x).max(one.x), slot.height()));
            draw.clipped(slot.intersect(inner), |draw| {
                draw.text(at, &run(text, color));
            });
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

/// One line of the preview.
#[derive(Clone, Debug, PartialEq)]
struct SampleRow {
    /// The line's room, relative to the preview's corner.
    rect: Rect,
    /// Whether the line belongs to the paragraph being edited.
    subject: bool,
    /// Whether the line is full (not the last of its paragraph, or a distributed last line).
    full: bool,
    /// Where a short line sits.
    last_align: ParagraphAlign,
}

#[derive(Clone, Debug, PartialEq)]
struct SamplePlan {
    rows: Vec<SampleRow>,
    height: f32,
}

/// The preview's lines for `format` in a box `width` wide, `row_height` being one line of text: a paragraph before
/// and after in the default format, and the edited one between. Fields that differ among the paragraphs (`None`) take
/// the default.
fn sample_plan(format: &ParagraphFormat, width: f32, row_height: f32) -> SamplePlan {
    let width = width.max(0.0);
    let plain = ParagraphFormat::default();
    let mut rows = Vec::new();
    let mut y = 0.0_f32;
    for (index, f) in [&plain, format, &plain].into_iter().enumerate() {
        let align = f.align.unwrap_or_default();
        let left = f.before_text.unwrap_or(0.0);
        let right = f.after_text.flatten().unwrap_or(0.0);
        let by = f.by.unwrap_or(0.0);
        let (first, rest) = match f.special.unwrap_or_default() {
            SpecialIndent::None => (0.0, 0.0),
            SpecialIndent::FirstLine => (by, 0.0),
            SpecialIndent::Hanging => (0.0, by),
        };
        let pitch = match f.line_spacing.unwrap_or_default() {
            LineSpacing::Auto => row_height,
            LineSpacing::Multiple(m) => row_height * m,
            LineSpacing::Exactly(e) => e,
            LineSpacing::AtLeast(a) => row_height.max(a),
        };
        y += f.space_before.unwrap_or(0.0);
        for line in 0..SAMPLE_LINES {
            let start = (left + if line == 0 { first } else { rest }).clamp(0.0, width);
            let end = (width - right).clamp(start, width);
            let last = line + 1 == SAMPLE_LINES;
            rows.push(SampleRow {
                rect: Rect::from_min_max(pos2(start, y), pos2(end, y + pitch)),
                subject: index == 1,
                full: !last || align == ParagraphAlign::Distributed,
                last_align: align,
            });
            y += pitch;
        }
        y += f.space_after.unwrap_or(0.0);
    }
    SamplePlan { rows, height: y }
}
