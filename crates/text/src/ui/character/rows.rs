//! Row helpers the Character and Paragraph panels share.

use std::ops::RangeInclusive;
use std::sync::Arc;

use crate::ui::length::{DocUnit, length_field};
use craft_behavior::{EditEvent, EditEvents, IconId, Mixed, NumberCodec};
use craft_ui::buttons::ToggleButton;
use craft_ui::contract::{Bind, Entry, GlyphLabel, LabelKind, Show as _, ShowContainer as _, SideLabel};
use craft_ui::form::Field;
use craft_ui::layout_components::Row;
use craft_ui::pickers::Select;
use craft_ui::runtime::env::{Label, UnitKind, Units};
use craft_ui::text::{NamedOr, NumberField, NumberPick};
use egui::Ui;

use super::{KerningName, PRESET_DECIMALS, TRACKING_RANGE};

/// The unit the length fields of a pattern show their numbers in.
pub(super) struct Lengths<'u> {
    pub(super) units: &'u Units,
    pub(super) unit: DocUnit,
}

/// A length field of a row: its label, mark, value in points and range in points.
pub(super) struct LengthRow<'p> {
    pub(super) label: Label,
    pub(super) glyph: GlyphLabel,
    pub(super) value: Option<f32>,
    pub(super) range: RangeInclusive<f64>,
    pub(super) presets: &'p [Entry<NumberPick>],
}

/// A plain number field of a row: a count, a percentage, or a value in thousandths.
pub(super) struct PlainRow<'p> {
    pub(super) label: Label,
    pub(super) glyph: GlyphLabel,
    pub(super) value: Mixed<Option<f64>>,
    pub(super) range: RangeInclusive<f64>,
    pub(super) percent: bool,
    pub(super) presets: &'p [Entry<NumberPick>],
}

impl Lengths<'_> {
    /// A field over a length in points, shown in the unit: its event in points and the preset row picked.
    pub(super) fn row(&self, ui: &mut Ui, spec: LengthRow<'_>) -> (EditEvents<Option<f64>>, Option<NumberPick>) {
        let LengthRow { label, glyph, value, range, presets } = spec;
        let points = value.map_or(Mixed::Mixed, |value| Mixed::Same(Some(f64::from(value))));
        let field = Field::new(label.clone()).side(mark(glyph)).show(ui, |ui, _| {
            let out = length_field(self.unit, label.clone(), points, range.clone()).presets(presets).scrub_by_label().show(ui);
            (out.edit.events, out.preset)
        });
        field.inner.unwrap_or_default()
    }

    /// The kerning field: a number or the name of a presets row (Metrics, Optical).
    pub(super) fn kerning(
        &self,
        ui: &mut Ui,
        label: Label,
        glyph: GlyphLabel,
        value: Option<NamedOr<KerningName>>,
        presets: &[Entry<NumberPick>],
    ) -> EditEvents<NamedOr<KerningName>> {
        let units = self.units;
        let shown = Field::new(label.clone()).side(mark(glyph)).show(ui, |ui, _| {
            let field = NumberField::named(label.clone(), Bind::given(mixed(value))).range(TRACKING_RANGE).presets(presets).scrub_by_label();
            field.codec(NumberCodec::new(units.reader(None), 0)).show(ui).edit.events
        });
        shown.inner.unwrap_or_default()
    }

    /// A field over a number that is not a length: its event and the preset row picked.
    pub(super) fn plain(&self, ui: &mut Ui, spec: PlainRow<'_>) -> (EditEvents<Option<f64>>, Option<NumberPick>) {
        let PlainRow { label, glyph, value, range, percent, presets } = spec;
        let units = self.units;
        let shown = Field::new(label.clone()).side(mark(glyph)).show(ui, |ui, _| {
            let field = NumberField::new(label.clone(), Bind::given(value)).range(range.clone()).presets(presets).scrub_by_label();
            let field = if percent {
                field.unit_kind(UnitKind::Percent).unit(units.symbol(UnitKind::Percent)).step(UnitKind::Percent.step())
            } else {
                field.codec(NumberCodec::new(units.reader(None), 0))
            };
            let out = field.show(ui);
            (out.edit.events, out.preset)
        });
        shown.inner.unwrap_or_default()
    }
}

/// A label shown as a mark, whose drag scrubs the field.
pub(super) fn mark(glyph: GlyphLabel) -> SideLabel {
    SideLabel::new().kind(LabelKind::Glyph(glyph)).scrub(true)
}

/// `value` as a field's value: a missing one is mixed.
pub(super) fn mixed<T>(value: Option<T>) -> Mixed<T> {
    value.map_or(Mixed::Mixed, Mixed::Same)
}

/// `value` as a plain number field's value: a missing one is mixed.
pub(super) fn plain(value: Option<f32>) -> Mixed<Option<f64>> {
    value.map_or(Mixed::Mixed, |value| Mixed::Same(Some(f64::from(value))))
}

/// A number field's event as a change; a cleared field is not one.
pub(super) fn number_change<C>(events: EditEvents<Option<f64>>, change: impl Fn(f32) -> C) -> EditEvents<C> {
    events
        .into_iter()
        .filter(|event| !matches!(event.value(), Some(None)))
        .map(|event| event.map(|value| change(value.unwrap_or_default() as f32)))
        .collect()
}

/// A menu row that is not a number, as the one-step change it makes.
pub(super) fn picked_event<C>(change: C, name: &str) -> EditEvent<C> {
    EditEvent::set(Mixed::Mixed, change, Arc::from(name), None)
}

/// The rows of a presets menu over groups of numbers, a separator between the groups.
pub(super) fn number_presets<'g>(units: &Units, groups: impl Iterator<Item = &'g [f32]>) -> Vec<Entry<NumberPick>> {
    let mut rows = Vec::new();
    for (at, group) in groups.enumerate() {
        if at > 0 {
            rows.push(Entry::separator());
        }
        rows.extend(group.iter().map(|&p| Entry::new(NumberPick::Number(f64::from(p)), units.number(f64::from(p), 0..=PRESET_DECIMALS))));
    }
    rows
}

/// Select entries over `(value, text)` pairs.
pub(super) fn string_entries(pairs: impl Iterator<Item = (String, String)>) -> Vec<Entry<String>> {
    pairs.map(|(value, text)| Entry::new(value, text)).collect()
}

/// A row with a select over `entries`; its pick as a change.
pub(super) fn select_row<C>(
    ui: &mut Ui,
    label: &Label,
    value: Mixed<String>,
    entries: &[Entry<String>],
    change: impl Fn(String) -> C,
) -> EditEvents<C> {
    let side = SideLabel::new().kind(LabelKind::Hidden);
    let shown = Field::new(label.clone())
        .side(side)
        .show(ui, |ui, _| Select::new(label.clone(), Bind::given(value.clone()), entries).placeholder("—").show(ui).events);
    shown.inner.unwrap_or_default().map(&change)
}

/// One toggle button of a row: its name, icon, state (`None` while the selection mixes) and the change it makes.
pub(super) struct Toggle<C> {
    pub(super) name: Label,
    pub(super) icon: &'static str,
    pub(super) on: Option<bool>,
    pub(super) change: Box<dyn Fn(bool) -> C>,
}

impl<C> Toggle<C> {
    pub(super) fn new(name: &Label, icon: &'static str, on: Option<bool>, change: Box<dyn Fn(bool) -> C>) -> Self {
        Toggle { name: name.clone(), icon, on, change }
    }
}

/// A row of icon toggle buttons; the pressed ones report their change when clicked.
pub(super) fn toggle_row<C>(ui: &mut Ui, toggles: Vec<Toggle<C>>) -> Vec<EditEvent<C>> {
    let mut edits = Vec::new();
    Row::new().wrap(true).show(ui, |ui, _| {
        for toggle in &toggles {
            let bind = Bind::given(mixed(toggle.on));
            let out = ToggleButton::new(toggle.name.clone(), bind).icon(IconId::new(toggle.icon)).icon_only().show(ui);
            edits.extend(out.events.map(&toggle.change));
        }
    });
    edits
}
