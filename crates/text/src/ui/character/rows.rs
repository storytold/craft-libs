//! Row helpers the Character and Paragraph panels share.

use std::ops::RangeInclusive;
use std::sync::Arc;

use crate::ui::length::{DocUnit, length_field, shown};
use craft_behavior::{DocKey, EditEvent, EditEvents, IconId, Mixed, NumberCodec, Preview, Value};
use craft_ui::buttons::ToggleButton;
use craft_ui::contract::{Bind, Entry, FieldLayout, GlyphLabel, LabelKind, Props, Show as _, ShowContainer as _, SideLabel, Sizing, TooltipSpec};
use craft_ui::form::Field;
use craft_ui::layout_components::Row;
use craft_ui::pickers::Select;
use craft_ui::primitives::{Glyph, IconSpec};
use craft_ui::runtime::env::{Label, UnitKind, Units};
use craft_ui::style::UiTokens;
use craft_ui::text::{NamedOr, NumberField, NumberPick};
use egui::{Color32, Painter, Rect, Shape, Stroke, Ui, vec2};

#[cfg(feature = "font-picker")]
use super::FontPickerSource;
use super::{
    AUTO_LEADING, AUTO_ROW, CharacterChange, CharacterValues, CharacterWords, FontHeightReference, KerningName, LEADING_RANGE, Leading,
    PRESET_DECIMALS, SIZE_PRESETS, TRACKING_RANGE,
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
        self.row_keyed(ui, spec, None)
    }

    /// As [`row`](Self::row), the field carrying the document `key` a host focuses it by.
    pub(crate) fn row_keyed(&self, ui: &mut Ui, spec: LengthRow<'_>, key: Option<DocKey>) -> (EditEvents<Option<f64>>, Option<NumberPick>) {
        let LengthRow { label, side, value, range, presets, disabled } = spec;
        let points = value.map_or(Mixed::Mixed, |value| Mixed::Same(Some(f64::from(value))));
        let field = Field::new(label.clone()).side(side).show(ui, |ui, _| {
            let field = length_field(self.unit, label.clone(), points, range.clone())
                .presets(presets)
                .scrub_by_label()
                .mixed_word(true)
                .sizing(Sizing::FillWidth);
            let field = match key {
                Some(key) => field.doc_key(key),
                None => field,
            };
            let out = disabled_for(field, &label, disabled.as_ref()).show(ui);
            (out.edit.events, out.preset)
        });
        field.inner.unwrap_or_default()
    }

    /// The kerning field: a number or the name of a presets row (Metrics, Optical). The field fills its grid cell.
    pub(super) fn kerning(
        &self,
        ui: &mut Ui,
        label: Label,
        side: SideLabel,
        value: Option<NamedOr<KerningName>>,
        presets: &[Entry<NumberPick>],
        key: Option<DocKey>,
    ) -> EditEvents<NamedOr<KerningName>> {
        let units = self.units;
        let shown = Field::new(label.clone()).side(side).show(ui, |ui, _| {
            let field = NumberField::named(label.clone(), Bind::given(mixed(value)))
                .range(TRACKING_RANGE)
                .presets(presets)
                .scrub_by_label()
                .sizing(Sizing::FillWidth);
            let field = field.codec(NumberCodec::new(units.reader(None), 0));
            match key {
                Some(key) => field.doc_key(key),
                None => field,
            }
            .show(ui)
            .edit
            .events
        });
        shown.inner.unwrap_or_default()
    }

    /// A field over a number that is not a length: its event and the preset row picked. The field fills its grid cell.
    pub(crate) fn plain(&self, ui: &mut Ui, spec: PlainRow<'_>) -> (EditEvents<Option<f64>>, Option<NumberPick>) {
        self.plain_keyed(ui, spec, None)
    }

    /// As [`plain`](Self::plain), the field carrying the document `key` a host focuses it by.
    pub(crate) fn plain_keyed(&self, ui: &mut Ui, spec: PlainRow<'_>, key: Option<DocKey>) -> (EditEvents<Option<f64>>, Option<NumberPick>) {
        let PlainRow { label, side, value, range, percent, presets, disabled } = spec;
        let units = self.units;
        let shown = Field::new(label.clone()).side(side).show(ui, |ui, _| {
            let field =
                NumberField::new(label.clone(), Bind::given(value)).range(range.clone()).presets(presets).scrub_by_label().sizing(Sizing::FillWidth);
            let field = if percent {
                field.unit_kind(UnitKind::Percent).step(UnitKind::Percent.step())
            } else {
                field.codec(NumberCodec::new(units.reader(None), 0))
            };
            let field = match key {
                Some(key) => field.doc_key(key),
                None => field,
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
    leading_row_keyed(lengths, ui, words, side, leading, size, presets, None)
}

/// As [`leading_row`], the field carrying the document `key` a host focuses it by.
#[allow(clippy::too_many_arguments)]
pub(crate) fn leading_row_keyed(
    lengths: &Lengths<'_>,
    ui: &mut Ui,
    words: &CharacterWords,
    side: SideLabel,
    leading: Option<Leading>,
    size: Option<f32>,
    presets: &[Entry<NumberPick>],
    key: Option<DocKey>,
) -> Vec<EditEvent<CharacterChange>> {
    let value = match leading {
        Some(Leading::Fixed(points)) => Some(points),
        Some(Leading::Auto) => size.map(|size| size * AUTO_LEADING),
        None => None,
    };
    let row = LengthRow { label: words.leading.clone(), side, value, range: LEADING_RANGE, presets, disabled: None };
    let (event, picked) = lengths.row_keyed(ui, row, key);
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

/// A row with a select over `entries`, sized by `sizing` (`None` keeps the select's own width); its pick as a change.
pub(crate) fn select_row<V: Value + 'static, C>(
    ui: &mut Ui,
    label: &Label,
    side: SideLabel,
    value: Mixed<V>,
    entries: &[Entry<V>],
    sizing: Option<Sizing>,
    change: impl Fn(V) -> C,
) -> EditEvents<C> {
    let shown = Field::new(label.clone())
        .side(side)
        .show(ui, |ui, _| sized(Select::new(label.clone(), Bind::given(value.clone()), entries).placeholder("—"), sizing).show(ui).events);
    shown.inner.unwrap_or_default().map(&change)
}

/// `component` sized by `sizing`; `None` keeps its own sizing.
fn sized<P: Props>(component: P, sizing: Option<Sizing>) -> P {
    match sizing {
        Some(sizing) => component.sizing(sizing),
        None => component,
    }
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
    /// The picker's id this pass; `None` without a picker.
    pub picker: Option<egui::Id>,
    /// The key of the host row run in the open list; a host pick is never a family edit.
    pub host_picked: Option<String>,
}

/// The family field of the character controls, the style options and the type bar: the font picker in a
/// hidden-label field when a `picker` source is given, else a select over `families`, sized by `sizing` (`None` keeps
/// the control's own width). A `disabled` reason disables the field and shows in its tooltip.
pub(crate) fn family_field(
    ui: &mut Ui,
    words: &CharacterWords,
    values: &CharacterValues,
    families: &[String],
    disabled: Option<&Label>,
    sizing: Option<Sizing>,
    #[cfg(feature = "font-picker")] picker: Option<&mut FontPickerSource<'_>>,
) -> FamilyField {
    #[cfg(feature = "font-picker")]
    {
        family_field_keyed(ui, words, values, families, None, disabled, sizing, picker)
    }
    #[cfg(not(feature = "font-picker"))]
    {
        family_field_keyed(ui, words, values, families, None, disabled, sizing)
    }
}

/// As [`family_field`], the picker carrying the document `key` a host focuses it by (the plain select has none).
#[allow(clippy::too_many_arguments)]
pub(crate) fn family_field_keyed(
    ui: &mut Ui,
    words: &CharacterWords,
    values: &CharacterValues,
    families: &[String],
    #[cfg(feature = "font-picker")] key: Option<DocKey>,
    #[cfg(not(feature = "font-picker"))] _key: Option<DocKey>,
    disabled: Option<&Label>,
    sizing: Option<Sizing>,
    #[cfg(feature = "font-picker")] picker: Option<&mut FontPickerSource<'_>>,
) -> FamilyField {
    let value = mixed(values.family.clone());
    #[cfg(feature = "font-picker")]
    if let Some(source) = picker {
        let shown = Field::new(words.family.clone()).side(hidden_side()).show(ui, |ui, _| {
            let mut field = FontPicker::new(words.family.clone(), Bind::given(value.clone()), source.catalog)
                .served(source.served)
                .sets(source.sets)
                .scripts(source.scripts)
                .filter_layout(source.filter_layout)
                .host(source.host);
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
            if let Some(key) = key {
                field = field.doc_key(key);
            }
            disabled_for(sized(field, sizing), &words.family, disabled).show(ui)
        });
        let Some(out) = shown.inner else { return FamilyField::default() };
        let id = craft_ui::contract::Outcome::response(&out).id;
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
        return FamilyField { edits, preview: out.preview, rested: out.rested, picker: Some(id), host_picked: out.host_picked };
    }
    let entries = string_entries(families.iter().map(|name| (name.clone(), name.clone())));
    let shown = Field::new(words.family.clone()).side(hidden_side()).show(ui, |ui, _| {
        let select = Select::new(words.family.clone(), Bind::given(value.clone()), &entries).placeholder("—");
        disabled_for(sized(select, sizing), &words.family, disabled).show(ui).events
    });
    let edits = shown.inner.unwrap_or_default().map(CharacterChange::Family).into_iter().collect();
    FamilyField { edits, ..FamilyField::default() }
}

/// Where two selections' optional values agree: that value, else `None` (mixed).
pub(crate) fn agree<T: Clone + PartialEq>(a: &Option<T>, b: &Option<T>) -> Option<T> {
    if a == b { a.clone() } else { None }
}

/// The share of a glyph's side its stroke may take.
const STROKE_SHARE: f32 = 0.2;

/// The mark of a Font Height Reference value: the box or the guide lines the value measures from.
#[derive(craft_ui::contract::MemoProps)]
struct FontHeightGlyph(FontHeightReference);

const EM_BOX: FontHeightGlyph = FontHeightGlyph(FontHeightReference::EmBox);
const CAP_HEIGHT: FontHeightGlyph = FontHeightGlyph(FontHeightReference::CapHeight);
const X_HEIGHT: FontHeightGlyph = FontHeightGlyph(FontHeightReference::XHeight);
const ICF_BOX: FontHeightGlyph = FontHeightGlyph(FontHeightReference::IcfBox);

impl Glyph for FontHeightGlyph {
    fn paint(&self, painter: &Painter, tokens: &UiTokens, rect: Rect, color: Color32, _mirrored: bool) {
        let side = rect.width().min(rect.height());
        if !rect.is_finite() || side <= 0.0 {
            return;
        }
        let thin = tokens.stroke.thin;
        let stroke = Stroke::new(if thin.is_finite() { thin.clamp(0.0, side * STROKE_SHARE) } else { 0.0 }, color);
        let at = |(x, y): (f32, f32)| rect.center() + vec2(x - 0.5, y - 0.5) * side;
        let boxed = |left: f32, top: f32, right: f32, bottom: f32| {
            Shape::closed_line([(left, top), (right, top), (right, bottom), (left, bottom)].map(at).to_vec(), stroke)
        };
        let segment = |from: (f32, f32), to: (f32, f32)| Shape::line_segment([at(from), at(to)], stroke);
        // A letter between a top guide and the baseline: the guide's height is what the value measures.
        let guided = |top: f32| vec![segment((0.1, top), (0.9, top)), segment((0.1, 0.8), (0.9, 0.8)), segment((0.5, top), (0.5, 0.8))];
        let shapes = match self.0 {
            FontHeightReference::EmBox => vec![boxed(0.1, 0.1, 0.9, 0.9), segment((0.1, 0.75), (0.9, 0.75))],
            FontHeightReference::CapHeight => guided(0.2),
            FontHeightReference::XHeight => guided(0.45),
            FontHeightReference::IcfBox => vec![boxed(0.2, 0.2, 0.8, 0.8)],
        };
        painter.extend(shapes);
    }
}

/// The Set Font Height Reference select's entries, each with the mark of its value.
pub(crate) fn font_height_entries(words: &CharacterWords) -> Vec<Entry<FontHeightReference>> {
    [
        (FontHeightReference::EmBox, &words.em_box, &EM_BOX),
        (FontHeightReference::CapHeight, &words.cap_height, &CAP_HEIGHT),
        (FontHeightReference::XHeight, &words.x_height, &X_HEIGHT),
        (FontHeightReference::IcfBox, &words.icf_box, &ICF_BOX),
    ]
    .into_iter()
    .map(|(value, name, glyph)| Entry::new(value, name.clone()).icon(IconSpec::glyph(glyph)))
    .collect()
}
