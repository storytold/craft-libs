//! One row of a page: the library's control for its kind, in a `Field` or on a card's line, reporting the value it
//! edited.

use super::model::{PrefValue, PreferencesModel, RowKind, RowSpec};
use super::page::PreferencesLayout;
use crate::buttons::Button;
use crate::choice::{Checkbox, Segmented, Swatch};
use crate::contract::{Bind, Entry, FieldLayout, Props as _, Show as _, ShowContainer as _, clicked};
use crate::display::{Shade, Text};
use crate::form::{Field, Help};
use crate::layout_components::{Row, Stack};
use crate::pickers::Select;
use crate::runtime::env::{Label, LengthUnits, UnitKind, Units};
use crate::runtime::requests::ColorRequest;
use crate::style::TypeRole;
use crate::text::{HexField, NumberField, TextField};
use craft_behavior::NumberCodec;
use craft_theme::{Rgba, Srgba};
use egui::Ui;
use std::ops::RangeInclusive;

/// A row's strings and value, copied from the host's borrowed model so that the host can be borrowed again to draw
/// a custom row.
#[derive(Clone, Debug)]
pub(super) struct ShownRow {
    pub key: String,
    pub label: Label,
    pub help: Option<Label>,
    pub enabled: bool,
    pub pills: Option<bool>,
    pub kind: Kind,
}

#[derive(Clone, Debug)]
pub(super) enum Kind {
    Bool(bool),
    Choice { value: String, options: Vec<Entry<String>>, unit: Option<UnitKind> },
    Color(Rgba),
    Int { value: i64, range: RangeInclusive<i64>, unit: Option<UnitKind>, unit_text: Option<Label> },
    Float { value: f64, range: RangeInclusive<f64>, decimals: u8, step: Option<f64>, unit: Option<UnitKind>, unit_text: Option<Label> },
    Length { value: f64, range: RangeInclusive<f64>, lengths: LengthUnits },
    Text(String),
    Custom,
    CustomRows,
}

impl ShownRow {
    pub fn of(row: &RowSpec<'_>) -> Self {
        let kind = match &row.kind {
            RowKind::Bool(value) => Kind::Bool(*value),
            RowKind::Choice { value, options, unit } => Kind::Choice {
                unit: *unit,
                value: (*value).to_owned(),
                options: options
                    .iter()
                    .map(|option| match option.value() {
                        Some(id) => Entry::new((*id).to_owned(), option.label().clone()).enabled(option.is_enabled()),
                        None => Entry::separator(),
                    })
                    .collect(),
            },
            RowKind::Color(color) => Kind::Color(*color),
            RowKind::Int { value, range, unit, unit_text } => {
                Kind::Int { value: *value, range: range.clone(), unit: *unit, unit_text: unit_text.map(|text| Label::from(text.to_owned())) }
            }
            RowKind::Float { value, range, decimals, unit, unit_text } => Kind::Float {
                value: *value,
                range: range.clone(),
                decimals: *decimals,
                step: row.step,
                unit: *unit,
                unit_text: unit_text.map(|text| Label::from(text.to_owned())),
            },
            RowKind::Length { value, range, lengths } => Kind::Length { value: *value, range: range.clone(), lengths: *lengths },
            RowKind::Text(text) => Kind::Text((*text).to_owned()),
            RowKind::Custom => Kind::Custom,
            RowKind::CustomRows => Kind::CustomRows,
        };
        Self {
            key: row.key.to_owned(),
            label: Label::from(row.label.to_owned()),
            help: row.help.map(|help| Label::from(help.to_owned())),
            enabled: row.enabled,
            pills: row.pills,
            kind,
        }
    }
}

/// What a pass of rows asks of the host: the values the rows edited, in order, and the colour row whose swatch was
/// pressed.
#[derive(Default)]
pub(super) struct RowsOut {
    pub edits: Vec<(String, PrefValue)>,
    /// A custom row's host reported that its draft changed.
    pub custom_changed: bool,
    /// The request of the swatch pressed this pass, keyed by the row.
    pub pick_color: Option<ColorRequest>,
}

/// The arrow-key step of a number row that names none: whole steps for whole numbers and wide ranges, else one unit
/// of the last decimal place.
pub(super) fn step_for(range: &RangeInclusive<f64>, decimals: u8) -> f64 {
    if decimals == 0 || range.end() - range.start() >= 10.0 { 1.0 } else { 10f64.powi(-i32::from(decimals)) }
}

/// A number field over `value` that reads the spellings of `unit` and shows its symbol after the number, or else
/// `unit_text`. A counted `unit` wins over `unit_text`, which is for quantities with no `UnitKind`.
fn number(units: &Units, label: &Label, value: f64, unit: Option<UnitKind>, unit_text: Option<&Label>, decimals: usize) -> NumberField<'static> {
    let field = NumberField::new(label.clone(), Bind::given(Some(value))).codec(NumberCodec::new(units.reader(unit), decimals));
    match (unit, unit_text) {
        (Some(kind), _) => field.unit_kind(kind),
        (None, Some(text)) => field.unit(text.clone()),
        (None, None) => field,
    }
}

/// `option` labelled as its id counted in `kind`; an option whose id is not a number keeps its label.
fn counted_option(units: &Units, kind: UnitKind, option: &Entry<String>) -> Entry<String> {
    match option.value().map(|id| (id, id.parse::<f64>())) {
        Some((id, Ok(n))) => Entry::new(id.clone(), units.show(kind, n)).enabled(option.is_enabled()),
        _ => option.clone(),
    }
}

/// The most options and the most label characters of a choice a card shows as a segmented control by default; a
/// longer choice is a drop-down.
const SEGMENTS_MAX: usize = 5;
const SEGMENTS_CHARS: usize = 48;

/// Whether a choice of `options` shows as a segmented control: as the row's [`RowSpec::pills`] says, else on a card
/// (`cards`) when it is short, and never with separators or fewer than two options.
fn as_segments(options: &[Entry<String>], unit: Option<UnitKind>, pills: Option<bool>, cards: bool) -> bool {
    let plain = options.iter().all(|option| !option.is_separator());
    let count = options.len();
    let chars: usize = options.iter().map(|option| option.label().as_str().chars().count()).sum();
    match pills {
        Some(want) => want && plain && count >= 2,
        None => cards && plain && unit.is_none() && (2..=SEGMENTS_MAX).contains(&count) && chars <= SEGMENTS_CHARS,
    }
}

/// The help line under a row: a caption in the muted shade.
fn help_line(ui: &mut Ui, help: &Option<Label>) {
    if let Some(help) = help {
        Text::new(help.clone()).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
    }
}

/// The field a row's control stands in: named by the row, with its help.
fn field_of(row: &ShownRow) -> Field {
    let field = Field::new(row.label.clone()).id_salt(row.key.as_str()).enabled(row.enabled);
    match &row.help {
        Some(help) => field.help(Help::Text(help.clone())),
        None => field,
    }
}

/// Shows `row` as `layout` lays rows out and records what it edited in `out`.
pub(super) fn show_row<H: PreferencesModel + ?Sized>(
    ui: &mut Ui,
    host: &mut H,
    row: &ShownRow,
    units: &Units,
    layout: PreferencesLayout,
    out: &mut RowsOut,
) {
    let key = row.key.as_str();
    let label = &row.label;
    let cards = layout == PreferencesLayout::Cards;
    let field = || field_of(row);
    let mut record = |value: Option<PrefValue>| {
        if let Some(value) = value {
            out.edits.push((key.to_owned(), value));
        }
    };
    match &row.kind {
        Kind::Bool(value) => {
            let mut next = None;
            field().layout(FieldLayout::After).show(ui, |ui, _| {
                next = Checkbox::new(Bind::given(*value)).show(ui).events.landed().cloned();
            });
            record(next.map(PrefValue::Bool));
        }
        Kind::Choice { value, options, unit } => {
            let mut next = None;
            let counted: Vec<Entry<String>>;
            let options = match unit {
                Some(kind) => {
                    counted = options.iter().map(|option| counted_option(units, *kind, option)).collect();
                    &counted
                }
                None => options,
            };
            let segments = as_segments(options, *unit, row.pills, cards);
            control(ui, row, cards, |ui| {
                next = if segments {
                    Segmented::new(label.clone(), Bind::given(value.clone()), options).show(ui).value().cloned()
                } else {
                    Select::new(label.clone(), Bind::given(value.clone()), options).show(ui).events.landed().cloned()
                };
            });
            record(next.map(PrefValue::Choice));
        }
        Kind::Int { value, range, unit, unit_text } => {
            let mut next = None;
            control(ui, row, cards, |ui| {
                let shown =
                    number(units, label, *value as f64, *unit, unit_text.as_ref(), 0).range(*range.start() as f64..=*range.end() as f64).step(1.0);
                next = shown.show(ui).edit.events.landed().cloned().flatten();
            });
            record(next.map(|v| PrefValue::Int(v.round() as i64)));
        }
        Kind::Float { value, range, decimals, step, unit, unit_text } => {
            let mut next = None;
            control(ui, row, cards, |ui| {
                let shown = number(units, label, *value, *unit, unit_text.as_ref(), usize::from(*decimals))
                    .range(range.clone())
                    .step(step.unwrap_or_else(|| step_for(range, *decimals)));
                next = shown.show(ui).edit.events.landed().cloned().flatten();
            });
            record(next.map(PrefValue::Float));
        }
        Kind::Length { value, range, lengths } => {
            let mut next = None;
            field().show(ui, |ui, _| {
                let shown = NumberField::length(label.clone(), Bind::given(Some(*value)), *lengths).range(range.clone());
                next = shown.show(ui).edit.events.landed().cloned().flatten();
            });
            record(next.map(PrefValue::Float));
        }
        Kind::Text(value) => {
            let mut next = None;
            control(ui, row, cards, |ui| {
                next = TextField::new(label.clone(), Bind::given(value.clone())).show(ui).events.landed().cloned();
            });
            record(next.map(PrefValue::Text));
        }
        Kind::Color(color) => {
            let mut next = None;
            let mut ask = None;
            let mut pair = |ui: &mut Ui| {
                let swatch = Swatch::new(label.clone(), Srgba::from(*color)).picker(key).id_salt("choose").show(ui);
                ask = swatch.pick_color;
                next = HexField::hex(label.clone(), Bind::given(*color)).id_salt("hex").show(ui).events.landed().cloned();
            };
            // A card's line holds the swatch and the hex field after the label; a field holds them in a row.
            control(ui, row, cards, |ui| {
                if cards {
                    pair(ui);
                } else {
                    Row::new().show(ui, |ui, _| pair(ui));
                }
            });
            record(next.map(PrefValue::Color));
            if ask.is_some() {
                out.pick_color = ask;
            }
        }
        Kind::CustomRows => {
            // The host draws the rows and their labels; the rule's enabled state and help apply to them as to a field.
            Stack::new().id_salt(key).enabled(row.enabled).show(ui, |ui, _| {
                out.custom_changed |= host.custom_row(ui, key);
                if let Some(help) = &row.help {
                    Help::Text(help.clone()).text().show(ui);
                }
            });
        }
        Kind::Custom => {
            field().show(ui, |ui, _| {
                Stack::new().show(ui, |ui, _| out.custom_changed |= host.custom_row(ui, key));
            });
        }
    }
}

/// Shows a row's control, drawn by `body`: on one line after the row's label on a card (`cards`), else in the row's field.
fn control(ui: &mut Ui, row: &ShownRow, cards: bool, body: impl FnOnce(&mut Ui)) {
    if cards {
        inline(ui, row, body);
        return;
    }
    let mut body = Some(body);
    field_of(row).show(ui, |ui, _| {
        if let Some(body) = body.take() {
            body(ui);
        }
    });
}

/// One row on a line, as a card draws it: the label, then the control, then the help under both.
fn inline(ui: &mut Ui, row: &ShownRow, control: impl FnOnce(&mut Ui)) {
    let mut control = Some(control);
    Stack::new().id_salt(row.key.as_str()).enabled(row.enabled).show(ui, |ui, _| {
        Row::new().show(ui, |ui, _| {
            Text::new(row.label.clone()).role(TypeRole::Body).show(ui);
            if let Some(control) = control.take() {
                control(ui);
            }
        });
        help_line(ui, &row.help);
    });
}

/// The Reset Section button of a page. Returns whether it was pressed.
pub(super) fn reset_button(ui: &mut Ui, label: Label) -> bool {
    clicked(&Button::new(label).id_salt("reset").show(ui))
}
