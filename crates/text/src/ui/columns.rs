//! Text columns: PowerPoint's Columns dialog (Number and Spacing for a text box), Word's More Columns dialog (presets
//! One, Two, Three, Left, Right; number; width and spacing per column; Equal column width; Line between) as one
//! [`TextColumnsDialog`], and Word's Layout > Columns menu ([`columns_menu`]).

use super::length::{DocUnit, length_field, shown};
use craft_behavior::{EditEvent, EditEvents, Metrics, Mixed, NumberCodec};
use craft_theme::variant::Tone;
use craft_ui::buttons::Button;
use craft_ui::choice::Checkbox;
use craft_ui::contract::{Bind, Common, Component, Cx, Entry, MeasureCx, PaintUi, Props as _, Role, Show as _, ShowContainer as _, clicked};
use craft_ui::dialog::{Dialog, DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use craft_ui::form::{Field, FieldGroup};
use craft_ui::layout_components::Row;
use craft_ui::notices::{Notice, NoticeLayout};
use craft_ui::pickers::Select;
use craft_ui::primitives::{PaintShape, StrokeWidth};
use craft_ui::runtime::env::{Env, Label, Units};
use craft_ui::runtime::words::CommonWords;
use craft_ui::sections::{DialogRole, FooterButton, GroupBox};
use craft_ui::style::{ComponentStyle, StyleColor, StyleCx, SurfaceKind, TypeRole};
use craft_ui::text::NumberField;
use egui::{Pos2, Rect, Ui, Vec2, pos2, vec2};
use std::cell::Cell;

#[cfg(feature = "screen-testing")]
mod stories;

/// The most columns PowerPoint takes; the
/// dialog's limit without [`word`](TextColumnsDialog::word) and the default for a host that
/// sets none.
pub const MAX_COLUMNS: u32 = 16;

/// The most columns Word takes; the
/// dialog's limit with [`word`](TextColumnsDialog::word).
pub const MAX_COLUMNS_WORD: u32 = 12;

/// The narrowest column of Word's dialog, points: the engine refuses narrower ones.
pub const MIN_COLUMN_WIDTH: f32 = 18.0;

/// The largest spacing, points (22 in, PowerPoint).
pub const MAX_SPACING: f32 = 1584.0;

/// The spacing between columns Word gives a preset and a new section's columns, points: Word's
/// 0.5 in.
const DEFAULT_SPACING: f32 = 0.5 * 72.0;
/// The Left and Right presets' narrow column is this share of the text width less one spacing,
/// the wide one the rest.
const NARROW_SHARE: f32 = 1.0 / 3.0;

craft_ui::words! {
    /// The dialog's and menu's words (PowerPoint's and Word's strings).
    pub struct TextColumnsWords {
        title: "columns.title" = "Columns";
        number: "columns.number" = "Number:";
        spacing: "columns.spacing" = "Spacing:";
        number_range: template "columns.numberRange" = "Number must be a whole number from {min} to {max}";
        spacing_range: template "columns.spacingRange" = "Spacing must be from {min} to {max}";
        presets: "columns.presets" = "Presets";
        one: "columns.one" = "One";
        two: "columns.two" = "Two";
        three: "columns.three" = "Three";
        left: "columns.left" = "Left";
        right: "columns.right" = "Right";
        width_and_spacing: "columns.widthAndSpacing" = "Width and spacing";
        column: template "columns.columnN" = "Col {n}:";
        width: "columns.width" = "Width:";
        equal_width: "columns.equalWidth" = "Equal column width";
        line_between: "columns.lineBetween" = "Line between";
        more_columns: "columns.more" = "More Columns…";
        preview: "columns.preview" = "Preview";
        apply_to: "columns.applyTo" = "Apply to:";
        this_section: "columns.thisSection" = "This section";
        whole_document: "columns.wholeDocument" = "Whole document";
        this_point_forward: "columns.thisPointForward" = "This point forward";
        selected_text: "columns.selectedText" = "Selected text";
    }
}

/// Column settings; `None` where the selected text boxes or sections differ (a field left as
/// shown is left out of the result, so each keeps its own value).
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct TextColumns {
    /// Number of columns.
    pub count: Option<u32>,
    /// The space between columns, points (all of them, with equal width).
    pub spacing: Option<f32>,
    /// Word: each column's width and the spacing after it, points; empty with equal width.
    pub widths: Vec<(f32, f32)>,
    /// Whether all columns have the same width.
    pub equal_width: Option<bool>,
    /// Whether a line is drawn between columns.
    pub line_between: Option<bool>,
    /// Word: where the columns apply. `Some` only for a dialog with [`apply_to`](TextColumnsDialog::apply_to),
    /// which always returns it with the changes (WordCraft sends it with every `layout.columns`).
    pub apply: Option<ColumnsApply>,
}

/// Word's Apply to: the part of the document the columns apply to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ColumnsApply {
    /// The section the caret is in (offered when the document has more than one).
    Section,
    /// The whole document.
    Document,
    /// From the caret to the end: a new section starts here.
    Forward,
    /// The selected text, as a section of its own.
    Selection,
}

/// Word's Layout › Columns presets.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ColumnsPreset {
    /// One.
    One,
    /// Two.
    Two,
    /// Three.
    Three,
    /// Two columns, the left one narrower.
    Left,
    /// Two columns, the right one narrower.
    Right,
}

/// The Columns dialog: only what changed comes back.
///
/// - [`word`](Self::word) off (PowerPoint): Number (1 to [`MAX_COLUMNS`]) and Spacing (0 to [`MAX_SPACING`] points);
///   values out of range keep the dialog open with the range messages under the fields.
/// - On (Word): the presets row, Number, the width and spacing grid (one row per column, Col 1 to n; width fields
///   disabled after the first with Equal column width), Equal column width, Line between, and the page's text width for
///   the grid's sums.
#[derive(craft_ui::contract::Pattern)]
pub struct TextColumnsDialog {
    shown: TextColumns,
    edited: TextColumns,
    word: Option<f32>,
    max_columns: Option<u32>,
    apply_choices: Vec<ColumnsApply>,
    unit: DocUnit,
    /// The edited columns from before the running gesture: Escape restores them.
    draft: Option<TextColumns>,
}

impl TextColumnsDialog {
    /// A dialog editing `columns`, with lengths in points.
    pub fn new(columns: TextColumns) -> Self {
        TextColumnsDialog {
            shown: columns.clone(),
            edited: columns,
            word: None,
            max_columns: None,
            apply_choices: Vec::new(),
            unit: DocUnit::default(),
            draft: None,
        }
    }

    /// Word's dialog, over a text width of `width` points.
    #[must_use]
    pub fn word(mut self, width: f32) -> Self {
        self.word = Some(width);
        self
    }

    /// The most columns Number takes, in place of the family's: [`MAX_COLUMNS_WORD`] with [`word`](Self::word),
    /// [`MAX_COLUMNS`] without. At least 1.
    #[must_use]
    pub fn max_columns(mut self, max: u32) -> Self {
        self.max_columns = Some(max.max(1));
        self
    }

    /// Word's Apply to: the parts of the document the columns can apply to, in the order the list shows them, and the
    /// one chosen when the dialog opens (added to the list when it is missing). The answer carries the choice in
    /// [`TextColumns::apply`], changed or not.
    #[must_use]
    pub fn apply_to(mut self, choices: &[ColumnsApply], current: ColumnsApply) -> Self {
        self.apply_choices = choices.to_vec();
        if !self.apply_choices.contains(&current) {
            self.apply_choices.push(current);
        }
        self.shown.apply = Some(current);
        self.edited.apply = Some(current);
        self
    }

    /// Sets the unit the length fields show.
    #[must_use]
    pub fn unit(mut self, unit: DocUnit) -> Self {
        self.unit = unit;
        self
    }

    /// The fields that differ from what the dialog showed; `None` when none does.
    pub fn changes(&self) -> Option<TextColumns> {
        let (shown, edited) = (&self.shown, &self.edited);
        fn kept<T: Clone + PartialEq>(a: &Option<T>, b: &Option<T>) -> Option<T> {
            if a != b { b.clone() } else { None }
        }
        let changes = TextColumns {
            count: kept(&shown.count, &edited.count),
            spacing: kept(&shown.spacing, &edited.spacing),
            widths: if shown.widths != edited.widths { edited.widths.clone() } else { Vec::new() },
            equal_width: kept(&shown.equal_width, &edited.equal_width),
            line_between: kept(&shown.line_between, &edited.line_between),
            apply: edited.apply.filter(|_| !self.apply_choices.is_empty()),
        };
        (changes != TextColumns::default()).then_some(changes)
    }

    /// The most columns Number takes.
    pub fn most_columns(&self) -> u32 {
        self.max_columns.unwrap_or(if self.word.is_some() { MAX_COLUMNS_WORD } else { MAX_COLUMNS })
    }

    fn count_ok(&self, count: u32) -> bool {
        (1..=self.most_columns()).contains(&count)
    }

    /// Whether the typed count is within one and the most the text width fits.
    pub fn count_in_range(&self) -> bool {
        self.edited.count.is_none_or(|n| self.count_ok(n))
    }

    /// Whether the typed spacing is within the allowed range.
    pub fn spacing_in_range(&self) -> bool {
        self.edited.spacing.is_none_or(|s| (0.0..=MAX_SPACING).contains(&s))
    }

    /// Whether the columns are of equal width.
    pub fn equal(&self) -> bool {
        self.edited.equal_width == Some(true)
    }

    /// The spacing the columns have for the arithmetic: the edited one, else the first column's.
    fn spacing_now(&self) -> f32 {
        self.edited.spacing.or(self.edited.widths.first().map(|w| w.1)).unwrap_or(DEFAULT_SPACING)
    }

    /// `count` equal columns of `spacing` filling `width`, each at least [`MIN_COLUMN_WIDTH`]: the
    /// spacing gives way when the columns would be narrower (WordCraft `even_out`).
    pub fn even_rows(width: f32, spacing: f32, count: u32) -> Vec<(f32, f32)> {
        let room = (width - MIN_COLUMN_WIDTH * count as f32).max(0.0);
        let spacing = if count > 1 { spacing.clamp(0.0, room / (count - 1) as f32) } else { spacing };
        let each = ((width - spacing * (count - 1) as f32) / count as f32).max(0.0);
        (0..count).map(|i| (each, if i + 1 < count { spacing } else { 0.0 })).collect()
    }

    /// The width and the spacing after each column, as the grid shows them.
    /// Each column's width and the spacing after it, in points.
    pub fn rows(&self) -> Vec<(f32, f32)> {
        let (Some(width), Some(count)) = (self.word, self.edited.count.filter(|n| self.count_ok(*n))) else {
            return Vec::new();
        };
        if !self.equal() && self.edited.widths.len() == count as usize {
            return self.edited.widths.clone();
        }
        Self::even_rows(width, self.spacing_now(), count)
    }

    /// Sets the number of columns.
    pub fn set_count(&mut self, count: u32) {
        self.edited.count = Some(count);
        if self.word.is_some() && self.count_ok(count) && !self.equal() {
            self.edited.widths = self.rows_for(count);
        }
    }

    fn rows_for(&self, count: u32) -> Vec<(f32, f32)> {
        Self::even_rows(self.word.unwrap_or(0.0), self.spacing_now(), count)
    }

    /// Applies a preset to the edited columns.
    pub fn apply_preset(&mut self, preset: ColumnsPreset) {
        let Some(width) = self.word else { return };
        let spacing = DEFAULT_SPACING;
        self.edited.spacing = Some(spacing);
        let (count, narrow_first) = match preset {
            ColumnsPreset::One => (1, None),
            ColumnsPreset::Two => (2, None),
            ColumnsPreset::Three => (3, None),
            ColumnsPreset::Left => (2, Some(true)),
            ColumnsPreset::Right => (2, Some(false)),
        };
        self.edited.count = Some(count);
        match narrow_first {
            None => {
                self.edited.equal_width = Some(true);
                self.edited.widths.clear();
            }
            Some(left) => {
                let room = (width - spacing).max(0.0);
                let narrow = room * NARROW_SHARE;
                let wide = room - narrow;
                let (first, second) = if left { (narrow, wide) } else { (wide, narrow) };
                self.edited.equal_width = Some(false);
                self.edited.widths = vec![(first, spacing), (second, 0.0)];
            }
        }
    }

    /// Column `row`'s width becomes `value`: with equal widths the spacing takes the difference,
    /// else the neighbouring column does.
    pub fn set_width(&mut self, row: usize, value: f32) {
        let (Some(width), Some(count)) = (self.word, self.edited.count) else { return };
        let value = value.clamp(MIN_COLUMN_WIDTH.min(width), width);
        if self.equal() {
            if count > 1 {
                self.edited.spacing = Some(((width - count as f32 * value) / (count - 1) as f32).clamp(0.0, MAX_SPACING));
            }
            return;
        }
        let mut rows = self.rows();
        let Some(old) = rows.get(row).map(|r| r.0) else { return };
        let other = if row + 1 < rows.len() { row + 1 } else { row.saturating_sub(1) };
        rows[row].0 = value;
        if other != row {
            // The neighbour gives the difference up to its minimum; the edited column takes back
            // what the neighbour could not give (WordCraft `rebalance`).
            let wanted = rows[other].0 - (value - old);
            let short = (MIN_COLUMN_WIDTH - wanted).max(0.0);
            rows[other].0 = wanted.max(MIN_COLUMN_WIDTH);
            if short > 0.0 {
                let from_width = short.min((rows[row].0 - MIN_COLUMN_WIDTH).max(0.0));
                rows[row].0 -= from_width;
                rows[row].1 = (rows[row].1 - (short - from_width)).max(0.0);
            }
        }
        self.edited.widths = rows;
    }

    /// The spacing after column `row` becomes `value`: with equal widths it is every column's;
    /// else the next column's width takes the difference.
    pub fn set_spacing_after(&mut self, row: usize, value: f32) {
        let value = value.clamp(0.0, MAX_SPACING);
        if self.equal() {
            self.edited.spacing = Some(value);
            return;
        }
        let mut rows = self.rows();
        let Some(old) = rows.get(row).map(|r| r.1) else { return };
        // The next column keeps its minimum width: the spacing stops growing there.
        let value = rows.get(row + 1).map_or(value, |next| value.min(old + (next.0 - MIN_COLUMN_WIDTH).max(0.0)));
        rows[row].1 = value;
        if let Some(next) = rows.get_mut(row + 1) {
            next.0 = (next.0 - (value - old)).max(MIN_COLUMN_WIDTH);
        }
        self.edited.widths = rows;
    }
}

impl TextColumnsDialog {
    /// Applies a field's event: every `Update` shows in the dialog, and Escape (`Cancel`) restores the columns from
    /// before the gesture. `set` writes the field's value into the columns.
    fn edit<V>(&mut self, events: impl IntoIterator<Item = EditEvent<V>>, set: impl Fn(&mut Self, V)) {
        for event in events {
            match event {
                EditEvent::Begin { .. } => self.draft = Some(self.edited.clone()),
                EditEvent::Update { value, .. } => {
                    self.draft.get_or_insert_with(|| self.edited.clone());
                    set(self, value);
                }
                EditEvent::Commit { after, .. } | EditEvent::Set { after, .. } => {
                    self.draft = None;
                    set(self, after);
                }
                EditEvent::Cancel { .. } => {
                    if let Some(before) = self.draft.take() {
                        self.edited = before;
                    }
                }
                _ => {}
            }
        }
    }

    /// The message under Number when it is out of range, its bounds in the locale's digits.
    pub fn number_message(words: &TextColumnsWords, units: &Units, max: u32) -> String {
        let bound = |n: u32| units.number(f64::from(n), 0..=0);
        words.number_range.fill(&[("min", &bound(1)), ("max", &bound(max))])
    }

    /// The Number field's event.
    pub fn edit_count(&mut self, events: EditEvents<Option<f64>>) {
        self.edit(events, |me, v| {
            if let Some(v) = v {
                me.set_count(v.max(0.0) as u32);
            }
        });
    }

    /// The columns as edited so far.
    pub fn edited(&self) -> &TextColumns {
        &self.edited
    }

    /// Whether the number and spacing are in range, which OK waits for.
    pub fn is_valid(&self) -> bool {
        self.count_in_range() && self.spacing_in_range()
    }

    fn spacing_message(&self, words: &TextColumnsWords, units: &Units) -> String {
        let show = |points: f32| units.show(self.unit.kind, shown(units, self.unit, f64::from(points)));
        words.spacing_range.fill(&[("min", &show(0.0)), ("max", &show(MAX_SPACING))])
    }

    /// One width or spacing field of the grid, named `caption`; its event in points.
    fn length_row(
        &self,
        ui: &mut Ui,
        caption: &str,
        points: Option<f32>,
        enabled: bool,
        range: std::ops::RangeInclusive<f32>,
    ) -> EditEvents<Option<f32>> {
        let points = points.map_or(Mixed::Mixed, |p| Mixed::Same(Some(f64::from(p))));
        let field = length_field(self.unit, caption.to_owned(), points, f64::from(*range.start())..=f64::from(*range.end())).enabled(enabled);
        field.show(ui).edit.events.map(|v| v.map(|p| p as f32))
    }

    fn body(&mut self, ui: &mut Ui, words: &TextColumnsWords, units: &Units, valid: &Cell<bool>) {
        if self.word.is_some() {
            GroupBox::new(words.presets.clone(), None).show(ui, |ui, _| self.presets(ui, words));
        }
        FieldGroup::new(words.title.clone()).show(ui, |ui, _| {
            let bad = !self.count_in_range();
            let shown = self.edited.count.map_or(Mixed::Mixed, |n| Mixed::Same(Some(f64::from(n))));
            let codec = NumberCodec::new(units.reader(None), 0);
            let edit = Field::new(words.number.clone())
                .show(ui, |ui, _| NumberField::new(words.number.clone(), Bind::given(shown)).codec(codec.clone()).show(ui))
                .inner;
            self.edit_count(edit.map(|out| out.edit.events).unwrap_or_default());
            if bad {
                Notice::new(Tone::Bad, Self::number_message(words, units, self.most_columns())).layout(NoticeLayout::Inline).show(ui);
            }
            if self.word.is_none() {
                let edit = Field::new(words.spacing.clone())
                    .show(ui, |ui, _| self.length_row(ui, words.spacing.as_str(), self.edited.spacing, true, 0.0..=MAX_SPACING))
                    .inner
                    .unwrap_or_default();
                self.edit(edit, |me, v: Option<f32>| {
                    if let Some(v) = v {
                        me.edited.spacing = Some(v);
                    }
                });
                if !self.spacing_in_range() {
                    Notice::new(Tone::Bad, self.spacing_message(words, units)).layout(NoticeLayout::Inline).show(ui);
                }
            }
        });
        if self.word.is_some() {
            Row::new().show(ui, |ui, _| {
                GroupBox::new(words.width_and_spacing.clone(), None).show(ui, |ui, _| self.grid(ui, words, units));
                let text_width = self.word.unwrap_or(0.0);
                let preview = ColumnsPreview::new(words.preview.clone(), self.rows(), text_width, self.edited.line_between == Some(true));
                preview.show(ui);
            });
            let equal = self.edited.equal_width.map_or(Mixed::Mixed, Mixed::Same);
            if let Some(on) = Checkbox::new(Bind::given(equal)).label(words.equal_width.clone()).show(ui).value().copied() {
                self.edited.equal_width = Some(on);
                if on {
                    self.edited.spacing = Some(self.spacing_now());
                    self.edited.widths.clear();
                } else if let Some(count) = self.edited.count.filter(|n| self.count_ok(*n)) {
                    self.edited.widths = self.rows_for(count);
                }
            }
        }
        let line = self.edited.line_between.map_or(Mixed::Mixed, Mixed::Same);
        if let Some(on) = Checkbox::new(Bind::given(line)).label(words.line_between.clone()).show(ui).value().copied() {
            self.edited.line_between = Some(on);
        }
        if !self.apply_choices.is_empty() {
            self.apply_select(ui, words);
        }
        valid.set(self.is_valid());
    }

    /// Word's Apply to list.
    fn apply_select(&mut self, ui: &mut Ui, words: &TextColumnsWords) {
        let entries: Vec<Entry<ColumnsApply>> = self
            .apply_choices
            .iter()
            .map(|choice| {
                let text = match choice {
                    ColumnsApply::Section => &words.this_section,
                    ColumnsApply::Document => &words.whole_document,
                    ColumnsApply::Forward => &words.this_point_forward,
                    ColumnsApply::Selection => &words.selected_text,
                };
                Entry::new(*choice, text.clone())
            })
            .collect();
        let current = self.edited.apply.map_or(Mixed::Mixed, Mixed::Same);
        let picked = Field::new(words.apply_to.clone())
            .show(ui, |ui, _| Select::new(words.apply_to.clone(), Bind::given(current), &entries).show(ui).value().copied())
            .inner
            .flatten();
        if let Some(choice) = picked {
            self.edited.apply = Some(choice);
        }
    }

    fn presets(&mut self, ui: &mut Ui, words: &TextColumnsWords) {
        let every = [
            (ColumnsPreset::One, &words.one),
            (ColumnsPreset::Two, &words.two),
            (ColumnsPreset::Three, &words.three),
            (ColumnsPreset::Left, &words.left),
            (ColumnsPreset::Right, &words.right),
        ];
        let mut chosen = None;
        Row::new().show(ui, |ui, _| {
            for (preset, label) in every {
                if clicked(&Button::new(label.clone()).show(ui)) {
                    chosen = Some(preset);
                }
            }
        });
        if let Some(preset) = chosen {
            self.apply_preset(preset);
        }
    }

    /// One row per column: its number, then its width and the spacing after it.
    fn grid(&mut self, ui: &mut Ui, words: &TextColumnsWords, units: &Units) {
        let rows = self.rows();
        let equal = self.equal();
        let mut width_edits: Vec<(usize, EditEvent<Option<f32>>)> = Vec::new();
        let mut spacing_edits: Vec<(usize, EditEvent<Option<f32>>)> = Vec::new();
        let (width_name, spacing_name) = (bare(words.width.as_str()), bare(words.spacing.as_str()));
        for (i, (width, spacing)) in rows.iter().enumerate() {
            let number = units.number(i as f64 + 1.0, 0..=0);
            let label = words.column.fill(&[("n", &number)]);
            let last = i + 1 == rows.len();
            Field::new(label).show(ui, |ui, _| {
                let w = self.length_row(ui, width_name, Some(*width), !equal || i == 0, 0.0..=MAX_SPACING);
                let s = self.length_row(ui, spacing_name, (!last).then_some(*spacing), !last, 0.0..=MAX_SPACING);
                // A run of a row without events leaves the events an earlier run of it reported alone.
                if !w.is_empty() {
                    width_edits.retain(|(row, _)| *row != i);
                    width_edits.extend(w.into_iter().map(|e| (i, e)));
                }
                if !s.is_empty() {
                    spacing_edits.retain(|(row, _)| *row != i);
                    spacing_edits.extend(s.into_iter().map(|e| (i, e)));
                }
            });
        }
        for (i, event) in width_edits {
            self.edit([event], |me, v: Option<f32>| {
                if let Some(v) = v {
                    me.set_width(i, v);
                }
            });
        }
        for (i, event) in spacing_edits {
            self.edit([event], |me, v: Option<f32>| {
                if let Some(v) = v {
                    me.set_spacing_after(i, v);
                }
            });
        }
    }
}

/// A caption without its trailing colon: the name of a field whose row label carries the colon.
fn bare(caption: &str) -> &str {
    caption.trim_end_matches([':', '\u{ff1a}'])
}

impl StackDialog for TextColumnsDialog {
    /// The fields that changed; `None` when none did (OK applies nothing).
    type Result = Option<TextColumns>;

    fn id(&self) -> DialogId {
        dialog_id("text-columns")
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<Option<TextColumns>>> {
        let env = Env::of(ui);
        let words = env.words::<TextColumnsWords>();
        let common = env.words::<CommonWords>();
        let units = env.units();
        let valid = Cell::new(self.is_valid());
        let buttons = [FooterButton::new(common.ok.clone(), DialogRole::Accept), FooterButton::new(common.cancel.clone(), DialogRole::Cancel)];
        let shown = Dialog::new(words.title.clone())
            .buttons(buttons)
            .settle_buttons(|_, buttons| {
                if let Some(ok) = buttons.first_mut() {
                    ok.set_enabled(valid.get());
                }
            })
            .show(ui, |ui, _| self.body(ui, &words, &units, &valid));
        match shown.output.result(|| self.changes()) {
            Some(DialogResult::Ok(changes)) if valid.get() => Some(DialogResult::Ok(changes)),
            Some(DialogResult::Cancel) => Some(DialogResult::Cancel),
            _ => None,
        }
    }
}

/// The preview's style: a sunken well holding a raised page with the columns' text lines.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct ColumnsPreviewStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The page.
    page: StyleColor,
    /// The page's edge.
    edge: StyleColor,
    /// A text line.
    line: StyleColor,
    /// The line between columns.
    separator: StyleColor,
    /// The side of the pane.
    pane: f32,
    /// The distance between text lines.
    pitch: f32,
}

/// The page preview: the columns and the line between them on a page, as Word's dialog draws it.
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
pub struct ColumnsPreview {
    #[common]
    common: Common,
    rows: Vec<(f32, f32)>,
    text_width: f32,
    line_between: bool,
}

impl ColumnsPreview {
    /// A preview named `label` of `rows` (each column's width and the spacing after it, points) over a text width of
    /// `text_width` points.
    pub fn new(label: impl Into<Label>, rows: Vec<(f32, f32)>, text_width: f32, line_between: bool) -> Self {
        ColumnsPreview { common: Common::new(), rows, text_width, line_between }.label(label)
    }
}

impl Component for ColumnsPreview {
    const ROLE: Role = Role::IMAGE;
    const OVERLAYS: bool = false;
    type Style = ColumnsPreviewStyle;
    type Output = egui::Response;

    fn style(cx: &StyleCx<'_>, _: ColumnsPreviewStyleKey) -> ColumnsPreviewStyle {
        let colors = &cx.tokens.color;
        ColumnsPreviewStyle {
            metrics: Metrics::new(),
            page: cx.color(colors.surface.raised),
            edge: cx.color(colors.line.border),
            line: cx.color(colors.text.faint),
            separator: cx.color(colors.text.normal),
            pane: cx.tokens.size.thumb.md,
            pitch: cx.tokens.typography.get(TypeRole::Body).line_height * LINE_PITCH,
        }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::splat(cx.style.pane)
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let style = cx.style;
        let pane = cx.rect;
        let height = pane.height().min(pane.width() / PAGE_ASPECT);
        let page = Rect::from_center_size(pane.center(), vec2(height * PAGE_ASPECT, height));
        let draw = ui.draw();
        draw.surface(pane, SurfaceKind::Sunken, craft_ui::style::Corners::None);
        draw.fill(&PaintShape::Rect(page, egui::CornerRadius::ZERO), style.page);
        draw.stroke(&PaintShape::Rect(page, egui::CornerRadius::ZERO), StrokeWidth::Thin, style.edge);
        let plan = preview_plan(&self.rows, self.text_width, self.line_between, page);
        let line = |a: Pos2, b: Pos2, color: StyleColor| draw.stroke(&PaintShape::Path(vec![a, b]), StrokeWidth::Thin, color);
        for &(left, right) in &plan.columns {
            let mut y = plan.text.top();
            while y + style.pitch / 2.0 <= plan.text.bottom() {
                line(pos2(left, y), pos2(right, y), style.line);
                y += style.pitch;
            }
        }
        for &x in &plan.separators {
            line(pos2(x, plan.text.top()), pos2(x, plan.text.bottom()), style.separator);
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> egui::Response {
        cx.response.clone()
    }
}

/// A choice from the Columns menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ColumnsChoice {
    /// One of the column presets.
    Preset(ColumnsPreset),
    /// More Columns...: open the dialog.
    More,
}

/// Layout > Columns as menu entries: One, Two, Three, Left, Right with the matching one checked, a separator and
/// More Columns.... The app shows them with `Menu::pick` from its ribbon button or menu bar and applies the chosen
/// value.
pub fn columns_menu(current: Option<ColumnsPreset>, words: &TextColumnsWords) -> Vec<Entry<ColumnsChoice>> {
    let presets = [
        (ColumnsPreset::One, &words.one),
        (ColumnsPreset::Two, &words.two),
        (ColumnsPreset::Three, &words.three),
        (ColumnsPreset::Left, &words.left),
        (ColumnsPreset::Right, &words.right),
    ];
    let mut entries: Vec<Entry<ColumnsChoice>> = presets
        .into_iter()
        .map(|(preset, label)| Entry::new(ColumnsChoice::Preset(preset), label.clone()).checked(current == Some(preset)))
        .collect();
    entries.push(Entry::separator());
    entries.push(Entry::new(ColumnsChoice::More, words.more_columns.clone()));
    entries
}

/// A page's width over its height.
const PAGE_ASPECT: f32 = 84.0 / 110.0;
/// The margin around the preview's text, as a share of the page's width.
const PAGE_MARGIN: f32 = 9.0 / 84.0;
/// The distance between the preview's text lines, as a share of a text row's height.
const LINE_PITCH: f32 = 0.5;

/// Where the preview draws: the text area, each column's left and right edge and the x of the
/// line between each two columns (when [`line_between`](TextColumns::line_between) is on).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct PreviewPlan {
    /// The text area on the page.
    pub text: Rect,
    /// The left and right edge of each column.
    pub columns: Vec<(f32, f32)>,
    /// The x of the line between each two columns.
    pub separators: Vec<f32>,
}

/// The preview of `rows` (each column's width and the spacing after it, points) over a text width
/// of `text_width` points on `page`.
pub fn preview_plan(rows: &[(f32, f32)], text_width: f32, line_between: bool, page: Rect) -> PreviewPlan {
    let text = page.shrink(page.width() * PAGE_MARGIN);
    let scale = text.width() / text_width.max(f32::EPSILON);
    let mut x = text.left();
    let mut columns = Vec::new();
    let mut separators = Vec::new();
    for (i, (width, spacing)) in rows.iter().enumerate() {
        let right = (x + width * scale).min(text.right());
        columns.push((x, right));
        x = right;
        if i + 1 < rows.len() {
            let gap = spacing * scale;
            if line_between {
                separators.push(x + gap / 2.0);
            }
            x += gap;
        }
    }
    PreviewPlan { text, columns, separators }
}
