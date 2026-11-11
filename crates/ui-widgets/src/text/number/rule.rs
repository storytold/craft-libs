//! The range, step and rounding a number field holds values to, and the ruled codec.

use super::value::NumberValue;
use super::{DEFAULT_STEP, ON_GRID};
use craft_behavior::nav::{LARGE_STEPS, snap_to_step};
use craft_behavior::{Codec, MoveMods, NotAValue, NumberCodec, decimals_for_step};
use std::borrow::Cow;
use std::marker::PhantomData;
use std::ops::RangeInclusive;

/// The value one step from a value in a direction: the up arrow, the wheel and the Page keys of a field whose values
/// lie on a ladder instead of a grid (`NumberField::stepper`). The direction is `1` up and `-1` down.
pub struct NumberStepper<'a>(Box<dyn Fn(f64, i32) -> f64 + 'a>);

impl<'a> NumberStepper<'a> {
    /// The stepper that runs `next`.
    pub fn new(next: impl Fn(f64, i32) -> f64 + 'a) -> Self {
        NumberStepper(Box::new(next))
    }

    pub(super) fn next(&self, from: f64, direction: i32) -> f64 {
        (self.0)(from, direction)
    }
}

/// The range and step values are held to.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Rule {
    pub(super) low: f64,
    pub(super) high: f64,
    pub(super) step: f64,
}

impl Rule {
    pub(super) fn new(range: Option<&RangeInclusive<f64>>, step: Option<f64>) -> Self {
        let (a, b) = range.map_or((f64::NEG_INFINITY, f64::INFINITY), |range| (*range.start(), *range.end()));
        let step = step.filter(|step| step.is_finite() && *step > 0.0).unwrap_or(DEFAULT_STEP);
        Rule { low: a.min(b), high: a.max(b), step }
    }

    /// `value` kept in the range.
    pub(super) fn settle(&self, value: f64) -> f64 {
        value.clamp(self.low, self.high)
    }

    /// Where a blank field steps from: 0, kept in the range.
    pub(super) fn blank(&self) -> f64 {
        self.settle(0.0)
    }

    /// The finite ends of the range.
    pub(super) fn bounds(&self) -> RangeInclusive<f64> {
        self.settle(f64::MIN)..=self.settle(f64::MAX)
    }

    /// The value `notches` steps of `scale` times the step from `from`: a value off the grid of that size first goes
    /// to the next multiple in the direction of the move (by 1, 5.1 goes up to 6 and down to 5).
    pub(super) fn stepped(&self, from: f64, notches: i32, scale: f64) -> f64 {
        let size = self.step * scale;
        let at = from / size;
        let nearest = at.round();
        let n = f64::from(notches);
        let multiple = if (at - nearest).abs() < ON_GRID {
            nearest + n
        } else if notches > 0 {
            at.ceil() + n - 1.0
        } else {
            at.floor() + n + 1.0
        };
        self.settle(round_to(multiple * size, decimals_for_step(size)))
    }

    /// `value` on the nearest multiple of the step.
    fn snap(&self, value: f64) -> f64 {
        round_to(snap_to_step(value, self.step, 0.0, f64::NEG_INFINITY..=f64::INFINITY), decimals_for_step(self.step))
    }
}

/// `value` rounded to `decimals` decimal places.
pub(super) fn round_to(value: f64, decimals: usize) -> f64 {
    let scale = 10f64.powi(i32::try_from(decimals).unwrap_or(0));
    (value * scale).round() / scale
}

/// The scale of a step by an arrow key: Shift moves `LARGE_STEPS` steps, Alt a share of one, both together one.
pub(super) fn key_scale(mods: MoveMods) -> f64 {
    match (mods.extend, mods.fine) {
        (true, false) => LARGE_STEPS,
        (false, true) => 1.0 / LARGE_STEPS,
        _ => 1.0,
    }
}

/// Whether `text` is a plain number rather than an expression or a relative input, which apply on commit only.
fn is_plain(text: &str) -> bool {
    let text = text.trim();
    !text.starts_with(['+', '*', '/']) && text.chars().skip(1).all(|c| !matches!(c, '+' | '-' | '*' | '/' | '(' | ')'))
}

/// The field's codec with what a committed value is held to: the range, the step grid when typed values snap, and
/// the decimals a committed value is rounded to. A plain number applies as it is typed unless typed values snap,
/// which apply on commit only. While the field shows no value (a mixed binding) typed numbers apply on commit only,
/// and empty text is not a value: it would clear every target, so the draft stays open until Escape or a focus
/// loss restores the mixed display. Text that spells a name of the presets (case ignored) is that name.
pub(super) struct Ruled<V> {
    pub(super) inner: NumberCodec,
    pub(super) rule: Rule,
    pub(super) snap: bool,
    pub(super) decimals: Option<usize>,
    pub(super) mixed: bool,
    pub(super) names: Vec<(u32, String)>,
    /// How many shown units make one stored unit (1 for a field that shows what it holds).
    pub(super) scale: f64,
    pub(super) value: PhantomData<fn() -> V>,
}

impl<V> Clone for Ruled<V> {
    fn clone(&self) -> Self {
        Ruled {
            inner: self.inner.clone(),
            rule: self.rule.clone(),
            snap: self.snap,
            decimals: self.decimals,
            mixed: self.mixed,
            names: self.names.clone(),
            scale: self.scale,
            value: PhantomData,
        }
    }
}

impl<V: NumberValue> Ruled<V> {
    fn held(&self, number: f64) -> f64 {
        let snapped = if self.snap { self.rule.snap(number) } else { number };
        self.rule.settle(self.decimals.map_or(snapped, |decimals| round_to(snapped, decimals)))
    }

    /// The number the value holds, as the field shows it; a non-finite number is shown as no number.
    fn shown(&self, v: &V) -> Option<f64> {
        v.number().filter(|number| number.is_finite()).map(|number| number * self.scale)
    }

    /// The name `text` spells, as a value of the field's type.
    fn name_of(&self, text: &str) -> Option<V> {
        let text = text.trim();
        self.names.iter().find(|(_, name)| name.eq_ignore_ascii_case(text)).and_then(|(id, _)| V::from_name(*id))
    }
}

impl<V: NumberValue> Codec for Ruled<V> {
    type Value = V;

    fn format(&self, v: &V) -> String {
        // A mixed field of a type with no blank value is handed a placeholder number; it shows empty.
        if self.mixed && V::blank().is_none() {
            return String::new();
        }
        match v.named().and_then(|id| self.names.iter().find(|(name, _)| *name == id)) {
            Some((_, label)) => label.clone(),
            None => self.inner.format(&self.shown(v)),
        }
    }

    fn parse(&self, text: &str, start: &V) -> Result<V, NotAValue> {
        self.parse_tagged(text, start).map(|(value, _)| value)
    }

    fn parse_tagged(&self, text: &str, start: &V) -> Result<(V, Option<u32>), NotAValue> {
        if let Some(name) = self.name_of(text) {
            return Ok((name, None));
        }
        if text.trim().is_empty() && (self.mixed || V::blank().is_none()) {
            return Err(NotAValue);
        }
        let (read, tag) = self.inner.parse_tagged(text, &self.shown(start))?;
        match read {
            Some(number) => Ok((V::of_number(self.held(number) / self.scale), tag)),
            None => V::blank().map(|blank| (blank, None)).ok_or(NotAValue),
        }
    }

    fn applies_as_typed(&self, text: &str) -> bool {
        !self.snap && !self.mixed && is_plain(text) && matches!(self.inner.parse(text, &None), Ok(Some(_)))
    }

    fn filter(&self, c: char) -> bool {
        self.inner.filter(c) || (!self.names.is_empty() && c.is_alphabetic())
    }

    fn normalise(&self, text: &str, v: &V) -> String {
        match v.named() {
            Some(_) => self.format(v),
            None => self.inner.normalise(text, &self.shown(v)),
        }
    }

    fn caret_on_focus(&self, text: &str) -> craft_behavior::Caret {
        self.inner.caret_on_focus(text)
    }

    fn sample(&self, current: &V) -> Cow<'_, str> {
        match current.named() {
            Some(_) => Cow::Owned(self.format(current)),
            None if self.rule.low.is_finite() && self.rule.high.is_finite() => Cow::Owned(self.inner.widest(&(self.rule.low..=self.rule.high))),
            None => self.inner.sample(&self.shown(current)),
        }
    }
}
