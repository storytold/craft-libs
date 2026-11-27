//! The row of sliders that edits the axes of a face (Photoshop's Properties › Character axis sliders; FigCraft's axis panel).

#[cfg(feature = "suite")]
pub(crate) mod stories;

use std::sync::Arc;

use super::{FontAxis, NamedInstance, matching_instance};

use craft_behavior::{EditEvent, EditEvents, Mixed};
use craft_ui::contract::{Bind, Entry, NoValue, Outcome, Pattern, Props, Show as _, ShowContainer as _};
use craft_ui::form::{Field, FieldGroup, SliderField};
use craft_ui::pickers::Select;
use craft_ui::runtime::env::{Env, Label};
use egui::{Response, Ui};

use crate::picker::{read_bind, write_event};
use skrifa::raw::types::Tag;

craft_ui::words! {
    /// The words of the axis sliders, English by default.
    #[non_exhaustive]
    pub struct AxisSlidersWords {
        weight: "axisSliders.weight" = "Weight";
        width: "axisSliders.width" = "Width";
        slant: "axisSliders.slant" = "Slant";
        italic: "axisSliders.italic" = "Italic";
        optical_size: "axisSliders.opticalSize" = "Optical size";
        style: "axisSliders.style" = "Style";
        custom: "axisSliders.custom" = "Custom";
    }
}

/// The label of an axis: its name when the font gives one, else the registered tag's
/// (`Weight`, `Width`, `Slant`, `Italic`, `Optical size`), else the tag itself.
pub fn axis_label(axis: &FontAxis) -> String {
    label_in(axis, &AxisSlidersWords::default())
}

fn label_in(axis: &FontAxis, labels: &AxisSlidersWords) -> String {
    if !axis.name.is_empty() {
        return axis.name.clone();
    }
    match &axis.tag {
        b"wght" => labels.weight.as_str().to_owned(),
        b"wdth" => labels.width.as_str().to_owned(),
        b"slnt" => labels.slant.as_str().to_owned(),
        b"ital" => labels.italic.as_str().to_owned(),
        b"opsz" => labels.optical_size.as_str().to_owned(),
        tag => Tag::new(tag).to_string(),
    }
}

/// The value an axis shows: the stored one, else what the style draws, else the axis default,
/// within the axis range.
fn value_of(values: &[([u8; 4], f32)], axis: &FontAxis, drawn: Drawn) -> f32 {
    let default = if axis.default.is_finite() { axis.default } else { axis.min };
    let stored = values.iter().find(|(t, _)| *t == axis.tag).map(|(_, v)| *v).filter(|v| v.is_finite());
    let styled = match &axis.tag {
        b"wght" => drawn.weight.filter(|w| w.is_finite()),
        b"ital" if drawn.weight.is_some() && drawn.italic => Some(axis.max).filter(|m| m.is_finite()),
        _ => None,
    };
    let (lo, hi) = if axis.min <= axis.max { (axis.min, axis.max) } else { (axis.max, axis.min) };
    stored.or(styled).unwrap_or(default).clamp(lo, hi)
}

fn set_value(values: &mut AxisValues, tag: [u8; 4], value: f32) {
    match values.iter_mut().find(|(t, _)| *t == tag) {
        Some(slot) => slot.1 = value,
        None => values.push((tag, value)),
    }
}

/// Where the optical size axis (`opsz`) takes its value from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum OpticalSize {
    /// The text layout sets it from the font size: it has no slider.
    #[default]
    FontSize,
    /// Its own slider, as any axis: for text drawn only to look at the face.
    Slider,
}

/// The axes the sliders show: not hidden, `opsz` only where it has a slider, with a finite, non-empty range.
pub(crate) fn shown_axes(axes: &[FontAxis], optical_size: OpticalSize) -> impl Iterator<Item = &FontAxis> {
    let slides = move |a: &FontAxis| a.tag != *b"opsz" || optical_size == OpticalSize::Slider;
    axes.iter().filter(move |a| !a.hidden && slides(a) && a.min.is_finite() && a.max.is_finite() && a.min < a.max)
}

/// What changed in the sliders this pass.
#[derive(Debug)]
#[non_exhaustive]
pub struct AxisSlidersOutput {
    /// The sliders' response.
    pub response: Response,
    /// The edits of this pass on the values, in order: a slider drag is one gesture (begin, updates, commit); a key
    /// step, a typed value and an instance pick are one set. The binding has already taken the values the events
    /// carry.
    pub edit: EditEvents<AxisValues>,
    /// A named instance was picked from the list.
    pub instance: Option<NamedInstance>,
}

impl Outcome for AxisSlidersOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The values of the axes: (tag, value) for every axis that has one.
pub type AxisValues = Vec<([u8; 4], f32)>;

/// The style the sliders' unset axes show.
#[derive(Clone, Copy, Default)]
struct Drawn {
    weight: Option<f32>,
    italic: bool,
}

/// One slider field per visible axis (label, slider, number field with the axis's range), plus a `Select` of named
/// instances above them when the face has any. Hidden axes, `opsz` ([`OpticalSize::FontSize`]) and axes with an empty or
/// non-finite range are left out. Values are the current coordinates; an axis
/// without one shows what the layout draws (its style's weight and italic; else its default). Axes whose range is 10 wide
/// or more step by whole units, the others by hundredths. A drag is one edit; an instance sets every axis in one
/// edit.
#[must_use = "show the sliders with `show`"]
#[derive(Pattern, Props)]
pub struct AxisSliders<'a> {
    label: Label,
    axes: &'a [FontAxis],
    values: Bind<'a, AxisValues>,
    /// The named instances of the face, listed above the sliders.
    #[prop]
    instances: &'a [NamedInstance],
    /// The style's weight, which an unset `wght` axis shows (an unset axis shows its default without it).
    #[prop(quantity = "font weight, the `wght` coordinate")]
    weight: Option<f32>,
    /// Whether the style is italic: an unset `ital` axis then shows its maximum, once a weight is given.
    #[prop]
    italic: bool,
    /// Whether the `opsz` axis has a slider.
    #[prop]
    optical_size: OpticalSize,
    /// The axes (by tag) the selection mixes: each shows `Mixed` and is left out of the edit's held values.
    #[prop]
    #[allow(dead_code)] // P3(type-w3): ty.X21a
    mixed: &'a [[u8; 4]],
}

impl<'a> AxisSliders<'a> {
    /// Sliders named `label` over the `axes` of a face, editing the coordinates `values`.
    pub fn new(label: impl Into<Label>, axes: &'a [FontAxis], values: Bind<'a, AxisValues>) -> Self {
        AxisSliders {
            label: label.into(),
            axes,
            values,
            instances: &[],
            weight: None,
            italic: false,
            optical_size: OpticalSize::default(),
            mixed: &[],
        }
    }

    /// Draws the sliders (and the instance list, if any) and reports the edit made this pass.
    pub fn show(self, ui: &mut Ui) -> AxisSlidersOutput {
        let AxisSliders { label, axes, mut values, instances, weight, italic, optical_size, mixed: _ } = self;
        let drawn = Drawn { weight, italic };
        let env = Env::of(ui);
        let words = env.words::<AxisSlidersWords>();
        let mut edit = EditEvents::<AxisValues>::new();
        let mut instance: Option<NamedInstance> = None;
        let shown = FieldGroup::new(label).show(ui, |ui, cx| {
            if !instances.is_empty() {
                let held = read_bind(&mut values, cx).unwrap_or_default();
                let current: AxisValues = axes.iter().map(|a| (a.tag, value_of(&held, a, drawn))).collect();
                let before = matching_instance(instances, &current).and_then(|m| instances.iter().position(|i| std::ptr::eq(i, m)));
                let mut picked = before;
                let entries: Vec<Entry<Option<usize>>> =
                    instances.iter().enumerate().map(|(at, instance)| Entry::new(Some(at), instance.name.clone())).collect();
                Field::new(words.style.clone()).show(ui, |ui, _| {
                    Select::new(words.style.clone(), Bind::bound(&mut picked), &entries).placeholder(words.custom.clone()).show(ui);
                });
                if picked != before
                    && let Some(chosen) = picked.and_then(|at| instances.get(at))
                {
                    let mut after = held.clone();
                    for &(tag, value) in &chosen.coordinates {
                        set_value(&mut after, tag, value);
                    }
                    let event = EditEvent::set(Mixed::Same(held), after, Arc::from("Font instance"), None);
                    write_event(&mut values, cx, &event);
                    edit.push(event);
                    instance = Some(chosen.clone());
                }
            }
            for axis in shown_axes(axes, optical_size) {
                let held = read_bind(&mut values, cx).unwrap_or_default();
                let wide = axis.max - axis.min >= 10.0;
                let mut value = f64::from(value_of(&held, axis, drawn));
                let name = Label::from(label_in(axis, &words));
                let default = f64::from(axis.default.clamp(axis.min, axis.max));
                let slider = SliderField::new(name.clone(), Bind::bound(&mut value).reset(default), f64::from(axis.min)..=f64::from(axis.max))
                    .step(if wide { 1.0 } else { 0.01 })
                    .show(ui);
                for event in slider.edit.events {
                    let clamp = |v: f64| (v as f32).clamp(axis.min, axis.max);
                    let event = event.map(|v| {
                        let mut all = held.clone();
                        set_value(&mut all, axis.tag, clamp(v));
                        all
                    });
                    write_event(&mut values, cx, &event);
                    edit.push(event);
                }
            }
        });
        AxisSlidersOutput { response: shown.output, edit, instance }
    }
}
