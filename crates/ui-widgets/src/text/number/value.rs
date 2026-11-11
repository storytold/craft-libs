//! The values a number field holds and reports: NumberValue, NamedOr, NumberPick and NumberOutput.

use crate::contract::{EditResponse, Outcome};
use crate::runtime::env::UnitKind;
use craft_behavior::Value;
use egui::Response;
use std::ops::Deref;

/// What a presets row picks: a number the field takes, or a choice that is not a number (Auto, Metrics) and
/// leaves the value alone.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum NumberPick {
    /// A pick sets the field to this number. The row is checked while the field holds it.
    Number(f64),
    /// A pick leaves the value alone and `NumberOutput::preset` names it. The row is checked by its entry's
    /// `checked`.
    Choice(u32),
}

/// A value a number field holds: a number, or a number that may also be a name from the presets list.
/// `Option<f64>` is the plain value (empty text is `None`); [`NamedOr`] adds named entries.
pub trait NumberValue: Value {
    /// The number the value holds; `None` for a blank value or a name.
    fn number(&self) -> Option<f64>;
    /// The value that holds `number`.
    fn of_number(number: f64) -> Self;
    /// The value empty text stands for; `None` when a blank is not a value of this type.
    fn blank() -> Option<Self>;
    /// The id of the `NumberPick::Choice` row the value is, `None` for a number.
    fn named(&self) -> Option<u32>;
    /// The value that is the name behind a `NumberPick::Choice` row; `None` when this type holds no names.
    fn from_name(id: u32) -> Option<Self>;
}

impl NumberValue for Option<f64> {
    fn number(&self) -> Option<f64> {
        *self
    }

    fn of_number(number: f64) -> Self {
        Some(number)
    }

    fn blank() -> Option<Self> {
        Some(None)
    }

    fn named(&self) -> Option<u32> {
        None
    }

    fn from_name(_: u32) -> Option<Self> {
        None
    }
}

/// A name a [`NamedOr`] value holds, tied to a `NumberPick::Choice` id of the field's presets.
pub trait NumberName: Value + Copy {
    /// The id of the `NumberPick::Choice` row that names it.
    fn id(self) -> u32;
    /// The name behind a row id, `None` for an id that names nothing.
    fn from_id(id: u32) -> Option<Self>;
}

/// A number or a name from the presets list (kerning: Metrics, Optical or a number). Empty text is not a value.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum NamedOr<K> {
    /// A name; the presets row `NumberPick::Choice(name.id())` shows its label.
    Named(K),
    /// A number.
    Number(f64),
}

impl<K: NumberName> NumberValue for NamedOr<K> {
    fn number(&self) -> Option<f64> {
        match self {
            NamedOr::Number(number) => Some(*number),
            NamedOr::Named(_) => None,
        }
    }

    fn of_number(number: f64) -> Self {
        NamedOr::Number(number)
    }

    fn blank() -> Option<Self> {
        None
    }

    fn named(&self) -> Option<u32> {
        match self {
            NamedOr::Named(name) => Some(name.id()),
            NamedOr::Number(_) => None,
        }
    }

    fn from_name(id: u32) -> Option<Self> {
        K::from_id(id).map(NamedOr::Named)
    }
}

/// What a number field reports: the edit and the preset row picked this pass.
#[non_exhaustive]
pub struct NumberOutput<V: NumberValue = Option<f64>> {
    /// The edit response of the value.
    pub edit: EditResponse<V>,
    /// The presets row the user picked this pass.
    pub preset: Option<NumberPick>,
    /// The unit the committed text named ("50 mm"), in the pass the text commits; `None` for a bare number, for
    /// text that named no unit, and in every other pass. The value still holds the number in the field's stored
    /// unit.
    pub typed_unit: Option<UnitKind>,
}

impl<V: NumberValue> Deref for NumberOutput<V> {
    type Target = EditResponse<V>;

    fn deref(&self) -> &Self::Target {
        &self.edit
    }
}

impl<V: NumberValue> Outcome for NumberOutput<V> {
    type Value = V;

    fn response(&self) -> &Response {
        &self.edit.response
    }

    #[cfg(feature = "testing")]
    fn story_facts(&self) -> crate::testing::StoryOutput {
        self.edit.story_facts()
    }
}
