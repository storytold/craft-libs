//! One row of a page: the library's control for its kind in a `Field`, reporting the value it edited.

use super::model::{PrefValue, PreferencesModel, RowKind, RowSpec};
use crate::buttons::Button;
use crate::choice::Checkbox;
use crate::contract::{Bind, Entry, FieldLayout, Props as _, Show as _, ShowContainer as _, clicked};
use crate::form::{Field, Help};
use crate::layout_components::Stack;
use crate::pickers::Select;
use crate::runtime::env::{Label, UnitKind, Units};
use crate::text::{HexField, NumberField, TextField};
use craft_behavior::NumberCodec;
use craft_theme::Rgba;
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
    pub kind: Kind,
}

#[derive(Clone, Debug)]
pub(super) enum Kind {
    Bool(bool),
    Choice { value: String, options: Vec<Entry<String>>, unit: Option<UnitKind> },
    Color(Rgba),
    Int { value: i64, range: RangeInclusive<i64>, unit: Option<UnitKind> },
    Float { value: f64, range: RangeInclusive<f64>, decimals: u8, step: Option<f64>, unit: Option<UnitKind> },
    Text(String),
    Custom,
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
            RowKind::Int { value, range, unit, .. } => Kind::Int { value: *value, range: range.clone(), unit: *unit },
            RowKind::Float { value, range, decimals, unit, .. } => {
                Kind::Float { value: *value, range: range.clone(), decimals: *decimals, step: row.step, unit: *unit }
            }
            RowKind::Text(text) => Kind::Text((*text).to_owned()),
            RowKind::Custom => Kind::Custom,
        };
        Self {
            key: row.key.to_owned(),
            label: Label::from(row.label.to_owned()),
            help: row.help.map(|help| Label::from(help.to_owned())),
            enabled: row.enabled,
            kind,
        }
    }
}

/// What a pass of rows asks of the host: the values the rows edited, in order.
#[derive(Default)]
pub(super) struct RowsOut {
    pub edits: Vec<(String, PrefValue)>,
}

/// The arrow-key step of a number row that names none: whole steps for whole numbers and wide ranges, else one unit
/// of the last decimal place.
pub(super) fn step_for(range: &RangeInclusive<f64>, decimals: u8) -> f64 {
    if decimals == 0 || range.end() - range.start() >= 10.0 { 1.0 } else { 10f64.powi(-i32::from(decimals)) }
}

fn number(units: &Units, label: &Label, value: f64, unit: Option<UnitKind>, decimals: usize) -> NumberField<'static> {
    NumberField::new(label.clone(), Bind::given(Some(value))).codec(NumberCodec::new(units.reader(unit), decimals))
}

/// `option` labelled as its id counted in `kind`; an option whose id is not a number keeps its label.
fn counted_option(units: &Units, kind: UnitKind, option: &Entry<String>) -> Entry<String> {
    match option.value().map(|id| (id, id.parse::<f64>())) {
        Some((id, Ok(n))) => Entry::new(id.clone(), units.show(kind, n)).enabled(option.is_enabled()),
        _ => option.clone(),
    }
}

/// Shows `row` and records what it edited in `out`.
pub(super) fn show_row<H: PreferencesModel + ?Sized>(ui: &mut Ui, host: &mut H, row: &ShownRow, units: &Units, out: &mut RowsOut) {
    let key = row.key.as_str();
    let label = &row.label;
    let field = || {
        let field = Field::new(label.clone()).id_salt(key).enabled(row.enabled);
        match &row.help {
            Some(help) => field.help(Help::Text(help.clone())),
            None => field,
        }
    };
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
            field().show(ui, |ui, _| {
                next = Select::new(label.clone(), Bind::given(value.clone()), options).show(ui).events.landed().cloned();
            });
            record(next.map(PrefValue::Choice));
        }
        Kind::Int { value, range, unit } => {
            let mut next = None;
            field().show(ui, |ui, _| {
                let shown = number(units, label, *value as f64, *unit, 0).range(*range.start() as f64..=*range.end() as f64).step(1.0);
                next = shown.show(ui).edit.events.landed().cloned().flatten();
            });
            record(next.map(|v| PrefValue::Int(v.round() as i64)));
        }
        Kind::Float { value, range, decimals, step, unit } => {
            let mut next = None;
            field().show(ui, |ui, _| {
                let shown = number(units, label, *value, *unit, usize::from(*decimals))
                    .range(range.clone())
                    .step(step.unwrap_or_else(|| step_for(range, *decimals)));
                next = shown.show(ui).edit.events.landed().cloned().flatten();
            });
            record(next.map(PrefValue::Float));
        }
        Kind::Text(value) => {
            let mut next = None;
            field().show(ui, |ui, _| {
                next = TextField::new(label.clone(), Bind::given(value.clone())).show(ui).events.landed().cloned();
            });
            record(next.map(PrefValue::Text));
        }
        Kind::Color(color) => {
            let mut next = None;
            field().show(ui, |ui, _| {
                next = HexField::hex(label.clone(), Bind::given(*color)).show(ui).events.landed().cloned();
            });
            record(next.map(PrefValue::Color));
        }
        Kind::Custom if label.as_str().is_empty() => {
            Stack::new().id_salt(key).show(ui, |ui, _| host.custom_row(ui, key));
        }
        Kind::Custom => {
            field().show(ui, |ui, _| {
                Stack::new().show(ui, |ui, _| host.custom_row(ui, key));
            });
        }
    }
}

/// The Reset Section button of a page. Returns whether it was pressed.
pub(super) fn reset_button(ui: &mut Ui, label: Label) -> bool {
    clicked(&Button::new(label).id_salt("reset").show(ui))
}
