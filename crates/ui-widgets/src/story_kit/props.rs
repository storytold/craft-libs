//! The Properties form of a story: one row per value the story keeps, with an editor for the kinds the framework
//! knows.
//!
//! Booleans get a check box, numbers a number field (inside the range the story gave with [`StoryData::ranged`]),
//! text a text field and values the story listed with [`StoryData::choice`] a select. Values of other types are
//! listed and cannot be edited. The edits write straight into the story's values, so the story reads them in the
//! next pass; [`StoryData::changed_since`] and [`StoryData::restore`] give "changed" and "reset".

use crate::choice::Checkbox;
use crate::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _};
use crate::display::{Shade, Text};
use crate::form::{Field, FieldGroup};
use crate::pickers::Select;
use crate::testing::story::{Edit, Slot, StoryData};
use crate::text::{NumberField, TextField};
use egui::{Response, Ui};
use std::any::Any;

/// `value` as a number when it has a plain number type.
fn number_of(value: &dyn Any) -> Option<f64> {
    macro_rules! try_each {
        ($($t:ty),*) => {$(
            if let Some(n) = value.downcast_ref::<$t>() {
                return Some(*n as f64);
            }
        )*};
    }
    try_each!(f64, f32, i8, i16, i32, i64, isize, u8, u16, u32, u64, usize);
    None
}

/// Writes `number` into `value` when it has a plain number type; an integer takes the nearest whole number.
fn set_number(value: &mut dyn Any, number: f64) {
    macro_rules! try_each {
        ($($t:ty),*) => {$(
            if let Some(slot) = value.downcast_mut::<$t>() {
                *slot = number.round() as $t;
                return;
            }
        )*};
    }
    if let Some(slot) = value.downcast_mut::<f64>() {
        *slot = number;
    } else if let Some(slot) = value.downcast_mut::<f32>() {
        *slot = number as f32;
    }
    try_each!(i8, i16, i32, i64, isize, u8, u16, u32, u64, usize);
}

fn is_whole(value: &dyn Any) -> bool {
    !(value.is::<f64>() || value.is::<f32>())
}

/// The name of a story value as a row label: `start_day` is "Start day".
fn label_of(name: &str) -> String {
    let spaced = name.replace(['_', '-'], " ");
    let mut chars = spaced.chars();
    chars.next().map(|first| first.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

/// The editable form of `data`'s values, one row each, sorted by name; returns the form's response. A value an editor
/// changes is one the story's view binds too (`StoryData::bind`): a story view that read it earlier in the pass runs
/// again.
pub fn story_props(ui: &mut Ui, data: &mut StoryData) -> Response {
    let mut keys: Vec<_> = data.values.keys().copied().collect();
    keys.sort_by_key(|(name, _)| *name);
    let ctx = ui.ctx().clone();
    FieldGroup::new("Properties")
        .show(ui, |ui, _| {
            for key in keys.iter().copied() {
                let label = label_of(key.0);
                let Some(mut slot) = data.__slot(key) else { continue };
                let mut wrote = false;
                Field::new(label.clone()).show(ui, |ui, _| row(ui, &label, &mut slot, &mut wrote));
                if wrote {
                    data.__wrote(&ctx, key);
                }
            }
        })
        .output
}

/// The editor of one value; `wrote` is set when it assigned the value outside its binding.
fn row(ui: &mut Ui, label: &str, slot: &mut Slot<'_>, wrote: &mut bool) -> Response {
    let how = slot.how;
    if let Some(Edit::Choice(options)) = how {
        let entries: Vec<_> = options.iter().enumerate().map(|(at, (name, _))| Entry::new(at, *name)).collect();
        let mut at = options.iter().position(|(_, option)| option.same(&*slot.value)).unwrap_or(usize::MAX);
        let shown = Select::new(label.to_owned(), Bind::bound(&mut at), &entries).show(ui);
        if let Some((_, picked)) = options.get(at)
            && !picked.same(&*slot.value)
        {
            slot.value.assign(&**picked);
            *wrote = true;
        }
        return shown.response;
    }
    if let Some(on) = slot.bind::<bool>() {
        return Checkbox::new(on).label(label.to_owned()).show(ui).response;
    }
    if let Some(text) = slot.bind::<String>() {
        return TextField::new(label.to_owned(), text).show(ui).response;
    }
    if let Some(number) = slot.bind::<Option<f64>>() {
        let mut field = NumberField::new(label.to_owned(), number);
        if let Some(Edit::Range(range)) = how {
            field = field.range(range.clone());
        }
        return field.show(ui).response.clone();
    }
    let any = slot.value.any_mut();
    if let Some(current) = number_of(&*any) {
        let mut number = Some(current);
        let mut field = NumberField::new(label.to_owned(), Bind::bound(&mut number));
        if is_whole(&*any) {
            field = field.step(1.0);
        }
        if let Some(Edit::Range(range)) = how {
            field = field.range(range.clone());
        }
        let shown = field.show(ui).response.clone();
        if let Some(edited) = number.filter(|edited| *edited != current) {
            set_number(any, edited);
            *wrote = true;
        }
        return shown;
    }
    Text::new("Not editable").shade(Shade::Muted).show(ui)
}
