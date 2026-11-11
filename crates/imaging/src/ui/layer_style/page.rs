//! The effect page: one field per entry of the effect's [`EffectSpec`], drawn by kind.

use craft_behavior::{EditEvent, EditEvents, Mixed, NumberCodec};
use craft_theme::variant::Tone;
use craft_theme::{Srgba, TypeRole};
use craft_ui::buttons::Button;
use craft_ui::choice::{Checkbox, Swatch};
use craft_ui::contract::{Bind, EditResponse, Entry, Props as _, Show as _, ShowContainer as _, clicked};
use craft_ui::display::Text;
use craft_ui::form::{Field, FieldGroup, SliderField};
use craft_ui::layout::RowAlign;
use craft_ui::layout_components::Row;
use craft_ui::patterns::presets::{PresetPicker, PresetRow, PresetState};
use craft_ui::pickers::Select;
use craft_ui::runtime::env::{Label, UnitKind, Units};
use craft_ui::runtime::requests::ColorRequest;
use craft_ui::text::NumberField;
use craft_ui::value_drag::AngleDial;
use egui::Ui;

use super::blending::BlendingOp;
use super::contour::{ContourAction, ContourField, ContourKey, ContourRequest, ContourWords, SharedSampler, contour_rows};
use super::labels::LayerStyleWords;
use crate::layer_style::{
    Contour, EffectField, EffectId, EffectInstance, EffectKind, EffectSpec, FieldKind, FieldUnit, LayerStyle, LayerStyleDraft, StyleParams,
    StyleValue,
};

/// The angle field shows whole degrees.
const WHOLE: usize = 0;
/// The angle dial's range, in degrees.
const ANGLE_RANGE: std::ops::RangeInclusive<f64> = -180.0..=180.0;

/// What a part of the dialog did to the style: the operation an event replays on the style it started from.
#[derive(Clone, Debug)]
pub(super) enum StyleOp {
    /// [`LayerStyleDraft::toggle`].
    Toggle(EffectId),
    /// [`LayerStyleDraft::add_instance`] from `defaults`.
    Add { kind: EffectKind, defaults: StyleParams },
    /// [`LayerStyleDraft::remove_instance`].
    Remove(EffectId),
    /// A page field: [`LayerStyleDraft::set_param`] of `key` of instance `id`.
    Param { id: EffectId, key: String, value: StyleValue },
    /// A Blending Options control.
    Blending(BlendingOp),
}

/// `op` applied to `style`; `None` when it changes nothing.
pub(super) fn apply_op(op: &StyleOp, style: &LayerStyle) -> Option<LayerStyle> {
    let mut draft = LayerStyleDraft::new(style.clone());
    match op {
        StyleOp::Toggle(id) => draft.toggle(*id),
        StyleOp::Add { kind, defaults } => {
            draft.add_instance(*kind, defaults);
        }
        StyleOp::Remove(id) => draft.remove_instance(*id),
        StyleOp::Param { id, key, value } => draft.set_param(*id, key, value.clone()),
        StyleOp::Blending(op) => op.apply(&mut draft.style.blending),
    }
    (draft.style != *style).then_some(draft.style)
}

/// The style edits of a child's edits: its events with every value replaced by `style` after `op` of that value, so a
/// slider drag stays one gesture of the whole style.
pub(super) fn style_edit<T: Clone>(edit: &EditResponse<T>, style: &LayerStyle, op: impl Fn(&T) -> StyleOp) -> EditEvents<LayerStyle> {
    edit.events.clone().map(|value| apply_op(&op(&value), style).unwrap_or_else(|| style.clone()))
}

/// A one-step edit of the style by `op`, labelled `label`.
pub(super) fn set_edit(style: &LayerStyle, op: &StyleOp, label: &str) -> Option<EditEvent<LayerStyle>> {
    let after = apply_op(op, style)?;
    Some(EditEvent::set(Mixed::Same(style.clone()), after, label.into(), None))
}

/// Adds `next` to the pass's edits, in order.
pub(super) fn push_edit(edit: &mut EditEvents<LayerStyle>, next: impl Into<EditEvents<LayerStyle>>) {
    edit.extend(next.into());
}

/// What an effect page reads besides the instance.
pub(super) struct PageInput<'a> {
    pub spec: &'a EffectSpec,
    pub instance: &'a EffectInstance,
    pub style: &'a LayerStyle,
    pub blend_modes: &'a [Entry<String>],
    pub patterns: &'a [PresetRow<String>],
    pub user_contours: &'a [Contour],
    pub sampler: Option<&'a SharedSampler>,
    pub words: &'a LayerStyleWords,
    pub contour_words: &'a ContourWords,
    pub units: &'a Units,
}

/// What an effect page reports in one pass.
#[derive(Default)]
pub(super) struct PageOutput {
    /// The pass's style edits, in order.
    pub edit: EditEvents<LayerStyle>,
    /// A colour swatch was clicked: key `"{instance id}/{parameter key}"` (`"fx2/color"`), titled
    /// [`LayerStyleWords::color_picker_title`].
    pub pick_color: Option<ColorRequest>,
    /// A library request from a contour field.
    pub contour_action: Option<ContourAction>,
    /// A click on a contour field's thumbnail: the key `"{instance id}/{parameter key}"` and the contour.
    pub edit_contour: Option<ContourRequest>,
    /// Make Default: the page's shown values (the defaults under the instance's).
    pub make_default: Option<StyleParams>,
    /// Reset to Default was pressed.
    pub reset_to_default: bool,
    /// A value the page shows from a default the instance lacks (a Pattern page with an empty value shows the first
    /// pattern): the parameter's key and the value. It is not an edit.
    pub fill_default: Option<(String, StyleValue)>,
}

/// Draws the page body: each field of `input.spec` in order, showing the spec's defaults under the instance's values
/// ([`StyleParams::over`]), so only real edits reach the instance. By [`FieldKind`]:
///
/// - `Slider`: a `SliderField` over the range with the unit, whole numbers; a drag is a gesture, a typed value or an
///   arrow a `Set`.
/// - `Angle`: an `AngleDial` beside a number field, −180..=180; with the instance's `useGlobalLight` on its edits go to
///   the shared light angle.
/// - `Color`: a `Swatch`; a click or Space reports `pick_color`.
/// - `Blend`: a `Select` over `blend_modes`; a value missing from the list shows as its key.
/// - `Choice`: a `Select` over the options; a missing value shows the first.
/// - `GradientMethod`: Smooth, Perceptual, Linear, Classic, with Custom first while the value names none of them.
/// - `Pattern`: a `PresetPicker` over `patterns`; an empty value shows the first pattern and reports it as
///   `fill_default`, not as an edit.
/// - `Contour`: a [`ContourField`] with the Anti-aliased check of the field's `anti_alias` key.
/// - `Check`: a `Checkbox`.
///
/// Under the fields, Make Default and Reset to Default at the trailing end. A value of the wrong type for its field
/// (only reachable when the host builds a bad draft) shows one line, [`LayerStyleWords::invalid_value`] naming the
/// first such field, instead of the fields, and the value stays as it is.
pub(super) fn effect_page(ui: &mut Ui, input: &PageInput<'_>) -> PageOutput {
    let mut out = PageOutput::default();
    let title = input.words.effect(input.instance.kind).clone();
    FieldGroup::new(title).id_salt(("effectPage", input.instance.id.0)).show(ui, |ui, _| {
        if let Some(field) = first_invalid(input.spec, &input.instance.params) {
            let name = field_label(input, field);
            let name = if name.as_str().is_empty() { Label::from(field.key.to_string()) } else { name };
            let line = Label::from(input.words.invalid_value.fill(&[("field", name.as_str())]));
            Text::new(line).role(TypeRole::Body).tone(Tone::Bad).show(ui);
            return;
        }
        let shown = input.instance.params.over(&input.spec.defaults);
        for field in &input.spec.fields {
            field_row(ui, input, field, &shown, &mut out);
        }
        Row::new().fill_width().align(RowAlign::End).wrap(true).show(ui, |ui, _| {
            if clicked(&Button::new(input.words.reset_to_default.clone()).id_salt("reset").show(ui)) {
                out.reset_to_default = true;
            }
            if clicked(&Button::new(input.words.make_default.clone()).id_salt("make").show(ui)) {
                out.make_default = Some(shown.clone());
            }
        });
    });
    out
}

/// The label of `field`: the installed word for a built-in key, else the spec's own.
fn field_label(input: &PageInput<'_>, field: &EffectField) -> Label {
    input.words.field(&field.key).cloned().unwrap_or_else(|| Label::from(field.label.to_string()))
}

fn unit_kind(unit: FieldUnit) -> UnitKind {
    match unit {
        FieldUnit::Percent => UnitKind::Percent,
        FieldUnit::Pixels => UnitKind::Pixels,
        FieldUnit::Degrees => UnitKind::Degrees,
    }
}

fn text_of(value: Option<&StyleValue>) -> Option<&str> {
    match value {
        Some(StyleValue::Text(text)) => Some(text),
        _ => None,
    }
}

/// A `Select` over `entries` inside a field named `label`; its edit.
fn pick(ui: &mut Ui, salt: &str, label: &Label, current: &str, entries: &[Entry<String>]) -> Option<EditResponse<String>> {
    Field::new(label.clone()).id_salt(salt).show(ui, |ui, _| Select::new(label.clone(), Bind::given(current.to_owned()), entries).show(ui)).inner
}

fn field_row(ui: &mut Ui, input: &PageInput<'_>, field: &EffectField, shown: &StyleParams, out: &mut PageOutput) {
    let (words, key) = (input.words, field.key.as_ref());
    let label = field_label(input, field);
    let id = input.instance.id;
    let param = |value: StyleValue| StyleOp::Param { id, key: key.to_owned(), value };
    match &field.kind {
        FieldKind::Slider { min, max, unit } => {
            let mut bind = Bind::given(shown.number(key).unwrap_or(*min));
            if let Some(default) = input.spec.defaults.number(key) {
                bind = bind.reset(default);
            }
            let slider = SliderField::new(label.clone(), bind.reborrowed(), *min..=*max).unit_kind(unit_kind(*unit)).step(1.0).id_salt(key).show(ui);
            push_edit(&mut out.edit, style_edit(&slider, input.style, |v| param(StyleValue::Number(v.round()))));
        }
        FieldKind::Angle => {
            let value = shown.number(key).unwrap_or(0.0);
            let edits = Field::new(label.clone())
                .id_salt(key)
                .show(ui, |ui, _| {
                    Row::new()
                        .show(ui, |ui, _| {
                            let dial = AngleDial::new(Bind::given(value)).label(label.clone()).id_salt("dial").show(ui);
                            let number = NumberField::new(label.clone(), Bind::given(Some(value)))
                                .codec(NumberCodec::new(input.units.reader(Some(UnitKind::Degrees)), WHOLE))
                                .range(ANGLE_RANGE)
                                .id_salt("field")
                                .show(ui)
                                .edit;
                            (dial, number)
                        })
                        .inner
                })
                .inner
                .flatten();
            if let Some((dial, number)) = edits {
                push_edit(&mut out.edit, style_edit(&dial, input.style, |v| param(StyleValue::Number(*v))));
                push_edit(&mut out.edit, style_edit(&number.map(|v| v.unwrap_or(value)), input.style, |v| param(StyleValue::Number(*v))));
            }
        }
        FieldKind::Color => {
            let color = match shown.get(key) {
                Some(StyleValue::Color(color)) => *color,
                _ => Srgba::BLACK,
            };
            let request = format!("{}/{key}", id.key());
            let swatch =
                Field::new(label.clone()).id_salt(key).show(ui, |ui, _| Swatch::new(label.clone(), color).picker(request.clone()).show(ui)).inner;
            if let Some(request) = swatch.and_then(|swatch| swatch.pick_color) {
                out.pick_color = Some(request.title(words.color_picker_title.clone()));
            }
        }
        FieldKind::Blend => {
            let current = text_of(shown.get(key)).unwrap_or("Normal");
            let mut entries = input.blend_modes.to_vec();
            if !entries.iter().any(|entry| entry.value().is_some_and(|value| value == current)) {
                entries.push(Entry::new(current.to_owned(), current.to_owned()));
            }
            if let Some(response) = pick(ui, key, &label, current, &entries) {
                push_edit(&mut out.edit, style_edit(&response, input.style, |v| param(StyleValue::Text(v.clone()))));
            }
        }
        FieldKind::Choice(choices) => {
            let Some(first) = choices.first() else { return };
            let current = text_of(shown.get(key)).unwrap_or(&first.value);
            let entries: Vec<Entry<String>> = choices
                .iter()
                .map(|c| Entry::new(c.value.to_string(), words.choice(&c.value).cloned().unwrap_or_else(|| Label::from(c.label.to_string()))))
                .collect();
            if let Some(response) = pick(ui, key, &label, current, &entries) {
                push_edit(&mut out.edit, style_edit(&response, input.style, |v| param(StyleValue::Text(v.clone()))));
            }
        }
        FieldKind::GradientMethod => {
            const METHODS: [&str; 4] = ["smooth", "perceptual", "linear", "classic"];
            let stored = text_of(shown.get(key)).unwrap_or("custom");
            let current = if METHODS.contains(&stored) { stored } else { "custom" };
            let mut entries = Vec::new();
            if current == "custom" {
                entries.push(Entry::new("custom".to_owned(), words.custom.clone()));
            }
            for (value, text) in METHODS.into_iter().zip([&words.smooth, &words.perceptual, &words.linear, &words.classic]) {
                entries.push(Entry::new(value.to_owned(), text.clone()));
            }
            if let Some(response) = pick(ui, key, &label, current, &entries) {
                push_edit(&mut out.edit, style_edit(&response, input.style, |v| param(StyleValue::Text(v.clone()))));
            }
        }
        FieldKind::Pattern => {
            let stored = text_of(shown.get(key)).filter(|t| !t.is_empty());
            let first = input.patterns.iter().find(|row| !row.is_group()).map(|row| row.key.clone());
            let current = stored.map(str::to_owned).or(first.clone());
            let picker = Field::new(label.clone())
                .id_salt(key)
                .show(ui, |ui, _| PresetPicker::new(label.clone(), Bind::given(current.clone()), input.patterns, Bind::kept()).show(ui))
                .inner;
            if let Some(picker) = picker {
                if picker.edit.events.is_empty() && stored.is_none() && first.is_some() {
                    out.fill_default = current.map(|text| (key.to_owned(), StyleValue::Text(text)));
                }
                let edit = picker.edit.map(Option::unwrap_or_default);
                push_edit(&mut out.edit, style_edit(&edit, input.style, |v| param(StyleValue::Text(v.clone()))));
            }
        }
        FieldKind::Contour { anti_alias } => {
            let contour = match shown.get(key) {
                Some(StyleValue::Contour(contour)) => contour.clone(),
                _ => Contour::linear(),
            };
            let before = anti_alias.as_deref().map(|k| shown.bool(k).unwrap_or(false));
            let mut checked = before;
            let presets = Contour::presets();
            let rows = contour_rows(&presets, input.user_contours, input.contour_words, input.sampler.cloned());
            // A field with no visible label (Emboss's contour under its check box) is still named for assistive tech.
            let name = if label.as_str().is_empty() {
                words.field("contour").cloned().unwrap_or_else(|| Label::from(field.key.to_string()))
            } else {
                label.clone()
            };
            let shown_field = Field::new(label.clone())
                .id_salt(key)
                .show(ui, |ui, _| {
                    let mut field = ContourField::new(name.clone(), Bind::given(contour.clone()), &rows, Bind::<PresetState<ContourKey>>::kept())
                        .user(input.user_contours);
                    if anti_alias.is_some() {
                        field = field.anti_alias(Bind::bound(&mut checked));
                    }
                    field.show(ui)
                })
                .inner;
            if let Some(field) = shown_field {
                push_edit(&mut out.edit, style_edit(&field.edit, input.style, |v| param(StyleValue::Contour(v.clone()))));
                if let (Some(aa_key), Some(on)) = (anti_alias.as_deref(), checked.filter(|_| checked != before)) {
                    let op = StyleOp::Param { id, key: aa_key.to_owned(), value: StyleValue::Bool(on) };
                    push_edit(&mut out.edit, set_edit(input.style, &op, label.as_str()));
                }
                out.edit_contour = field.edit_contour.map(|mut asked| {
                    asked.key = format!("{}/{key}", id.key());
                    asked
                });
                out.contour_action = field.action;
            }
        }
        FieldKind::Check => {
            let check = Checkbox::new(Bind::given(shown.bool(key).unwrap_or(false))).label(label.clone()).id_salt(key).show(ui);
            push_edit(&mut out.edit, style_edit(&check, input.style, |v| param(StyleValue::Bool(*v))));
        }
    }
}

/// The first field of `spec` whose value in `params` has the wrong type: a slider or angle not a number, a check not a
/// bool, a colour not a colour, a contour not a contour, a choice, blend or pattern not text. Missing keys are fine
/// (the defaults show). A contour's Anti-aliased key is held to a bool as well.
fn first_invalid<'s>(spec: &'s EffectSpec, params: &StyleParams) -> Option<&'s EffectField> {
    let wrong = |key: &str, ok: fn(&StyleValue) -> bool| params.get(key).is_some_and(|v| !ok(v));
    spec.fields.iter().find(|field| match &field.kind {
        FieldKind::Slider { .. } | FieldKind::Angle => wrong(&field.key, |v| matches!(v, StyleValue::Number(_))),
        FieldKind::Check => wrong(&field.key, |v| matches!(v, StyleValue::Bool(_))),
        FieldKind::Color => wrong(&field.key, |v| matches!(v, StyleValue::Color(_))),
        FieldKind::Blend | FieldKind::Choice(_) | FieldKind::GradientMethod | FieldKind::Pattern => {
            wrong(&field.key, |v| matches!(v, StyleValue::Text(_)))
        }
        FieldKind::Contour { anti_alias } => {
            wrong(&field.key, |v| matches!(v, StyleValue::Contour(_)))
                || anti_alias.as_deref().is_some_and(|k| wrong(k, |v| matches!(v, StyleValue::Bool(_))))
        }
    })
}
