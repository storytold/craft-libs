//! Row helpers the Character and Paragraph panels share.

use std::ops::RangeInclusive;
use std::sync::Arc;

use crate::ui::length::{DocUnit, length_field, shown};
use craft_behavior::{EditEvent, EditEvents, IconId, Mixed, NumberCodec, Preview};
use craft_ui::buttons::ToggleButton;
use craft_ui::contract::{Bind, Entry, FieldLayout, GlyphLabel, LabelKind, Props, Show as _, ShowContainer as _, SideLabel, Sizing, TooltipSpec};
use craft_ui::form::Field;
use craft_ui::layout_components::Row;
use craft_ui::pickers::Select;
use craft_ui::runtime::env::{Label, UnitKind, Units};
use craft_ui::text::{NamedOr, NumberField, NumberPick};
use egui::Ui;

#[cfg(feature = "font-picker")]
use super::FontPickerSource;
use super::{
    AUTO_LEADING, AUTO_ROW, CharacterChange, CharacterValues, CharacterWords, KerningName, LEADING_RANGE, Leading, PRESET_DECIMALS, SIZE_PRESETS,
    TRACKING_RANGE,
};
#[cfg(feature = "font-picker")]
use craft_font_manager::picker::FontPicker;

/// The unit the length fields of a pattern show their numbers in.
pub(crate) struct Lengths<'u> {
    pub(crate) units: &'u Units,
    pub(crate) unit: DocUnit,
}

/// A length field of a row: its label, side label, value in points and range in points.
pub(crate) struct LengthRow<'p> {
    pub(crate) label: Label,
    pub(crate) side: SideLabel,
    pub(crate) value: Option<f32>,
    pub(crate) range: RangeInclusive<f64>,
    pub(crate) presets: &'p [Entry<NumberPick>],
    /// Why the field is disabled; `None` while it is enabled.
    pub(crate) disabled: Option<Label>,
}

impl LengthRow<'_> {
    /// The field is disabled for `reason`, which its tooltip shows.
    pub(crate) fn disabled(mut self, reason: &Label) -> Self {
        self.disabled = Some(reason.clone());
        self
    }
}

/// A plain number field of a row: a count, a percentage, or a value in thousandths.
pub(crate) struct PlainRow<'p> {
    pub(crate) label: Label,
    pub(crate) side: SideLabel,
    pub(crate) value: Mixed<Option<f64>>,
    pub(crate) range: RangeInclusive<f64>,
    pub(crate) percent: bool,
    pub(crate) presets: &'p [Entry<NumberPick>],
    /// Why the field is disabled; `None` while it is enabled.
    pub(crate) disabled: Option<Label>,
}

impl PlainRow<'_> {
    /// The field is disabled for `reason`, which its tooltip shows.
    pub(crate) fn disabled(mut self, reason: &Label) -> Self {
        self.disabled = Some(reason.clone());
        self
    }
}

/// `component`, named `name`, disabled for `reason` when one is given: the tooltip shows the reason while it is
/// disabled. Every row helper and pattern control that can be unavailable goes through it.
pub(crate) fn disabled_for<P: Props>(component: P, name: &Label, reason: Option<&Label>) -> P {
    match reason {
        Some(reason) => component.enabled(false).tooltip(TooltipSpec::new(name.clone()).disabled_text(reason.clone())),
        None => component,
    }
}

impl Lengths<'_> {
    /// A field over a length in points, shown in the unit: its event in points and the preset row picked. The field
    /// fills its grid cell.
    pub(crate) fn row(&self, ui: &mut Ui, spec: LengthRow<'_>) -> (EditEvents<Option<f64>>, Option<NumberPick>) {
        let LengthRow { label, side, value, range, presets, disabled } = spec;
        let points = value.map_or(Mixed::Mixed, |value| Mixed::Same(Some(f64::from(value))));
        let field = Field::new(label.clone()).side(side).show(ui, |ui, _| {
            let field = length_field(self.unit, label.clone(), points, range.clone())
                .presets(presets)
                .scrub_by_label()
                .mixed_word(true)
                .sizing(Sizing::FillWidth);
            let out = disabled_for(field, &label, disabled.as_ref()).show(ui);
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
        hint: Label,
        value: Option<NamedOr<KerningName>>,
        presets: &[Entry<NumberPick>],
    ) -> EditEvents<NamedOr<KerningName>> {
        let units = self.units;
        let shown = Field::new(label.clone()).side(mark(glyph, Some(hint), FieldLayout::Beside)).show(ui, |ui, _| {
            let field = NumberField::named(label.clone(), Bind::given(mixed(value))).range(TRACKING_RANGE).presets(presets).scrub_by_label();
            field.codec(NumberCodec::new(units.reader(None), 0)).show(ui).edit.events
        });
        shown.inner.unwrap_or_default()
    }

    /// A field over a number that is not a length: its event and the preset row picked.
    pub(crate) fn plain(&self, ui: &mut Ui, spec: PlainRow<'_>) -> (EditEvents<Option<f64>>, Option<NumberPick>) {
        let PlainRow { label, side, value, range, percent, presets, disabled } = spec;
        let units = self.units;
        let shown = Field::new(label.clone()).side(side).show(ui, |ui, _| {
            let field = NumberField::new(label.clone(), Bind::given(value)).range(range.clone()).presets(presets).scrub_by_label();
            let field = if percent {
                field.unit_kind(UnitKind::Percent).step(UnitKind::Percent.step())
            } else {
                field.codec(NumberCodec::new(units.reader(None), 0))
            };
            let out = disabled_for(field, &label, disabled.as_ref()).show(ui);
            (out.edit.events, out.preset)
        });
        shown.inner.unwrap_or_default()
    }
}

/// A label shown as a mark at `layout`, whose drag scrubs the field; `hint`, when given, is its tooltip in place of the
/// label.
pub(crate) fn mark(glyph: GlyphLabel, hint: Option<Label>, layout: FieldLayout) -> SideLabel {
    let side = SideLabel::new().kind(LabelKind::Glyph(glyph)).layout(layout).scrub(true);
    match hint {
        Some(hint) => side.tooltip(hint),
        None => side,
    }
}

/// The side label of a row that shows none (a select).
pub(crate) fn hidden_side() -> SideLabel {
    SideLabel::new().kind(LabelKind::Hidden)
}

/// `value` as a field's value: a missing one is mixed.
pub(crate) fn mixed<T>(value: Option<T>) -> Mixed<T> {
    value.map_or(Mixed::Mixed, Mixed::Same)
}

/// `value` as a plain number field's value: a missing one is mixed.
pub(crate) fn plain(value: Option<f32>) -> Mixed<Option<f64>> {
    value.map_or(Mixed::Mixed, |value| Mixed::Same(Some(f64::from(value))))
}

/// A number field's event as a change; a cleared field is not one.
pub(crate) fn number_change<C>(events: EditEvents<Option<f64>>, change: impl Fn(f32) -> C) -> EditEvents<C> {
    events
        .into_iter()
        .filter(|event| !matches!(event.value(), Some(None)))
        .map(|event| event.map(|value| change(value.unwrap_or_default() as f32)))
        .collect()
}

/// A menu row that is not a number, as the one-step change it makes.
pub(crate) fn picked_event<C>(change: C, name: &str) -> EditEvent<C> {
    EditEvent::set(Mixed::Mixed, change, Arc::from(name), None)
}

/// The rows of a presets menu over groups of numbers, a separator between the groups.
pub(crate) fn number_presets<'g>(units: &Units, groups: impl Iterator<Item = &'g [f32]>) -> Vec<Entry<NumberPick>> {
    let mut rows = Vec::new();
    for (at, group) in groups.enumerate() {
        if at > 0 {
            rows.push(Entry::separator());
        }
        rows.extend(group.iter().map(|&p| Entry::new(NumberPick::Number(f64::from(p)), units.number(f64::from(p), 0..=PRESET_DECIMALS))));
    }
    rows
}

/// The size menu's rows: the point presets in the type unit.
pub(crate) fn size_menu(units: &Units, unit: DocUnit) -> Vec<Entry<NumberPick>> {
    length_menu(units, unit, &SIZE_PRESETS)
}

/// The rows of a length field's presets menu: `presets` in points, shown in `unit`.
pub(crate) fn length_menu(units: &Units, unit: DocUnit, presets: &[f32]) -> Vec<Entry<NumberPick>> {
    presets
        .iter()
        .map(|&p| {
            let points = f64::from(p);
            Entry::new(NumberPick::Number(points), units.text(unit.kind, shown(units, unit, points), 0..=PRESET_DECIMALS))
        })
        .collect()
}

/// The leading menu's rows: the Auto row (checked while `leading` is Auto), a separator, then the size rows.
pub(crate) fn leading_menu(words: &CharacterWords, leading: Option<Leading>, sizes: &[Entry<NumberPick>]) -> Vec<Entry<NumberPick>> {
    let auto = Entry::new(NumberPick::Choice(AUTO_ROW), words.auto.clone()).checked(leading == Some(Leading::Auto));
    [auto, Entry::separator()].into_iter().chain(sizes.iter().cloned()).collect()
}

/// The leading field: a number in points, or Auto, which shows the font size times the auto-leading.
pub(crate) fn leading_row(
    lengths: &Lengths<'_>,
    ui: &mut Ui,
    words: &CharacterWords,
    side: SideLabel,
    leading: Option<Leading>,
    size: Option<f32>,
    presets: &[Entry<NumberPick>],
) -> Vec<EditEvent<CharacterChange>> {
    let value = match leading {
        Some(Leading::Fixed(points)) => Some(points),
        Some(Leading::Auto) => size.map(|size| size * AUTO_LEADING),
        None => None,
    };
    let row = LengthRow { label: words.leading.clone(), side, value, range: LEADING_RANGE, presets, disabled: None };
    let (event, picked) = lengths.row(ui, row);
    let mut edits: Vec<_> = number_change(event, |points| CharacterChange::Leading(Leading::Fixed(points))).into_iter().collect();
    if picked == Some(NumberPick::Choice(AUTO_ROW)) {
        edits.push(picked_event(CharacterChange::Leading(Leading::Auto), words.leading.as_str()));
    }
    edits
}

/// Select entries over `(value, text)` pairs.
pub(crate) fn string_entries(pairs: impl Iterator<Item = (String, String)>) -> Vec<Entry<String>> {
    pairs.map(|(value, text)| Entry::new(value, text)).collect()
}

/// A row with a select over `entries`; its pick as a change.
pub(crate) fn select_row<C>(
    ui: &mut Ui,
    label: &Label,
    side: SideLabel,
    value: Mixed<String>,
    entries: &[Entry<String>],
    change: impl Fn(String) -> C,
) -> EditEvents<C> {
    let shown = Field::new(label.clone())
        .side(side)
        .show(ui, |ui, _| Select::new(label.clone(), Bind::given(value.clone()), entries).placeholder("—").show(ui).events);
    shown.inner.unwrap_or_default().map(&change)
}

/// One toggle button of a row: its name, icon, state (`None` while the selection mixes) and the change it makes.
pub(crate) struct Toggle<C> {
    pub(crate) name: Label,
    pub(crate) icon: &'static str,
    pub(crate) on: Option<bool>,
    pub(crate) change: Box<dyn Fn(bool) -> C>,
    /// Why the button is disabled; `None` while it is enabled.
    pub(crate) disabled: Option<Label>,
}

impl<C> Toggle<C> {
    pub(crate) fn new(name: &Label, icon: &'static str, on: Option<bool>, change: Box<dyn Fn(bool) -> C>) -> Self {
        Toggle { name: name.clone(), icon, on, change, disabled: None }
    }

    /// The button is disabled for `reason`, which its tooltip shows.
    pub(crate) fn disabled(mut self, reason: &Label) -> Self {
        self.disabled = Some(reason.clone());
        self
    }
}

/// A row of icon toggle buttons, each its own Tab stop; a clicked one reports its change. `label` names the row as a
/// group (Alignment).
pub(crate) fn toggle_row<C>(ui: &mut Ui, label: Option<&Label>, toggles: &[Toggle<C>]) -> Vec<EditEvent<C>> {
    let mut edits = Vec::new();
    let row = match label {
        Some(label) => Row::new().label(label.clone()),
        None => Row::new(),
    };
    row.wrap(true).show(ui, |ui, _| {
        for toggle in toggles {
            let bind = Bind::given(mixed(toggle.on));
            let button = ToggleButton::new(toggle.name.clone(), bind).icon(IconId::new(toggle.icon)).icon_only();
            let out = disabled_for(button, &toggle.name, toggle.disabled.as_ref()).show(ui);
            edits.extend(out.events.map(&toggle.change));
        }
    });
    edits
}

/// What [`family_field`] reports: the family edits, and the family the picker previews and rests on.
#[derive(Default)]
pub(crate) struct FamilyField {
    /// The family edit, and the style edit of a style picked in an expanded family's rows; a hovered row is not one.
    pub edits: Vec<EditEvent<CharacterChange>>,
    /// What this pass did to the live preview: the family under the pointer or keyboard row in the open list.
    pub preview: Preview<String>,
    /// The family the pointer or keyboard rested on in the open list.
    pub rested: Option<String>,
    /// The key of the host row run in the open list; a host pick is never a family edit.
    pub host_picked: Option<String>,
}

/// The family field of the character controls, the style options and the type bar: the font picker in a
/// hidden-label field when a `picker` source is given, else a select over `families`. A `disabled` reason disables the
/// field and shows in its tooltip.
pub(crate) fn family_field(
    ui: &mut Ui,
    words: &CharacterWords,
    values: &CharacterValues,
    families: &[String],
    disabled: Option<&Label>,
    #[cfg(feature = "font-picker")] picker: Option<&mut FontPickerSource<'_>>,
) -> FamilyField {
    let value = mixed(values.family.clone());
    #[cfg(feature = "font-picker")]
    if let Some(source) = picker {
        let shown = Field::new(words.family.clone()).side(hidden_side()).show(ui, |ui, _| {
            let mut field = FontPicker::new(words.family.clone(), Bind::given(value.clone()), source.catalog).served(source.served).host(source.host);
            if let Some(key) = source.current_host.clone() {
                field = field.current_host(key);
            }
            if let Some(settings) = source.settings.as_mut() {
                field = field.settings(settings.reborrowed());
            }
            if let Some(filters) = source.filters.as_mut() {
                field = field.filters(filters.reborrowed());
            }
            if let Some(text) = source.preview_text {
                field = field.preview_text(text);
            }
            disabled_for(field, &words.family, disabled).show(ui)
        });
        let Some(out) = shown.inner else { return FamilyField::default() };
        let mut edits: Vec<EditEvent<CharacterChange>> = out
            .edit
            .events
            .into_iter()
            .filter(|event| matches!(event, EditEvent::Commit { .. } | EditEvent::Set { .. }))
            .map(|event| event.map(CharacterChange::Family))
            .collect();
        if let Some(style) = out.picked_style {
            edits.push(picked_event(CharacterChange::Style(style), words.style.as_str()));
        }
        return FamilyField { edits, preview: out.preview, rested: out.rested, host_picked: out.host_picked };
    }
    let entries = string_entries(families.iter().map(|name| (name.clone(), name.clone())));
    let shown = Field::new(words.family.clone()).side(hidden_side()).show(ui, |ui, _| {
        let select = Select::new(words.family.clone(), Bind::given(value.clone()), &entries).placeholder("—");
        disabled_for(select, &words.family, disabled).show(ui).events
    });
    let edits = shown.inner.unwrap_or_default().map(CharacterChange::Family).into_iter().collect();
    FamilyField { edits, ..FamilyField::default() }
}

/// Where two selections' optional values agree: that value, else `None` (mixed).
pub(crate) fn agree<T: Clone + PartialEq>(a: &Option<T>, b: &Option<T>) -> Option<T> {
    if a == b { a.clone() } else { None }
}
