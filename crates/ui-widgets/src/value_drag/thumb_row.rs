//! `ThumbRow`: triangle thumbs on one row, each setting a value of its own, as the input and output rows of a levels
//! editor do. One leaf with a focusable thumb part per value. A press on a thumb grabs it and a drag moves it by the
//! pointer's travel; a press elsewhere on the row jumps the nearest thumb there; arrows, the wheel and AccessKit step
//! the focused thumb. Each value is mixed or not on its own. The row's rules (order, wrap, split halves) are
//! `RowRules`; what a press off the thumbs does is the row's [`OffThumbs`].
//!
//! Alt keeps the meaning it has in Photoshop's Blend If: on a split thumb it moves one half alone, at full rate on a
//! drag (from the moment it is held) and by the plain step on a key or the wheel; without it the pair moves together.
//! On a thumb that is not split, Alt scales a move finer as on every value control.

use super::track::{self, Pick, Press, Scale, SliderScale, ThumbRing, TrackFace, Travel, travel};
use super::words::ValueDragWords;
use crate::contract::{
    Access, Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, Focus as _, Input, MeasureCx, Numeric, Outcome, PaintUi, PartsCx,
    Pointer as _, Remember as _, Role, WordSetCx as _,
};
use crate::primitives::{PaintShape, StrokeWidth};
use crate::runtime::access::Name;
use crate::runtime::env::Label;
use crate::runtime::focus::{ActivateKeys, FocusShape, KeepKeys, KeyUse};
use crate::style::{ComponentStyle, DataColor, FocusLook, StyleColor, StyleCx, TrackLook};
use craft_behavior::nav::{LARGE_STEPS, StepResult, ValueAxes, ValueSteps, step_value};
use craft_behavior::part::PartRef;
use craft_behavior::{
    HitLevel, HitPriority, Metrics, Mixed, MoveMods, NavMove, PartKey, PressPhase, PressState, Sense, Wheel, wheel_step, wrap_value,
};
use craft_keymap::Mods;
use craft_theme::{Rgba, Stateful, States};
use egui::{CornerRadius, Pos2, Rangef, Rect, Vec2, pos2};
use std::ops::RangeInclusive;

#[cfg(feature = "testing")]
mod stories;

/// The row-wide part: a press off the thumbs lands here.
pub(crate) const ROW: PartKey = PartKey::new("row");

/// The style of a thumb row.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct ThumbRowStyle {
    #[style(metrics)]
    metrics: Metrics,
    #[style(focus)]
    focus: FocusLook,
    /// A thumb's fill when its `Thumb` gives no face.
    face: Stateful<StyleColor>,
    /// A thumb's edge.
    edge: Stateful<StyleColor>,
    /// The ring round a thumb while a press at the pointer would move it.
    ring: ThumbRing,
    /// How far from a thumb's centre a press grabs it.
    hit: f32,
    /// The width and height of a thumb's triangle.
    thumb: Vec2,
    /// The track a track face paints.
    track: TrackLook,
    /// The length a row asks for along its axis.
    length: f32,
    /// The ink of a [`SpanInk::Faint`] span.
    span_faint: StyleColor,
    /// The ink of a [`SpanInk::Muted`] span.
    span_muted: StyleColor,
    /// The thickness of a span.
    span: f32,
}

/// How a thumb's value is kept among the row's other values.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum Order {
    /// Anywhere in its range: it passes the other thumbs.
    Free,
    /// At least `gap` above the value of thumb `below` and below that of thumb `above`. A move stops there, or pushes
    /// the other thumb along, as the row's [`Neighbours`] say.
    #[non_exhaustive]
    Between {
        /// The thumb whose value this one stays above, if any.
        below: Option<usize>,
        /// The thumb whose value this one stays below, if any.
        above: Option<usize>,
        /// The least distance kept from each of them, in the row's values.
        gap: f64,
    },
}

impl Order {
    /// Between thumbs `below` and `above`, `gap` apart.
    pub const fn between(below: Option<usize>, above: Option<usize>, gap: f64) -> Self {
        Order::Between { below, above, gap }
    }
}

/// What a thumb does when a move takes it into a thumb it is ordered with ([`Order::Between`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum Neighbours {
    /// It stops there.
    #[default]
    Stop,
    /// It pushes the other thumb along, and that one the next, up to the end of the range: a Blend If bar's handles.
    Push,
}

/// What a press off the thumbs does: one beyond the reach of every thumb.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum OffThumbs {
    /// The nearest thumb jumps to the pointer, on a tap and on a drag: a levels row.
    #[default]
    Jump,
    /// A drag takes the nearest thumb to the pointer; a tap only focuses it: a Blend If bar.
    Nearest,
    /// Nothing: only a press on a thumb moves one. The range thumbs of a hue editor.
    Ignore,
}

/// The ink of a [`ThumbSpan`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SpanInk {
    /// The theme's faint text colour.
    Faint,
    /// The theme's muted text colour.
    Muted,
}

/// A band along the row from one thumb to another, under the thumbs' tips: the extent of a hue range and of its
/// full-strength part. In a wrapping row it runs forward from `from` round to `to`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ThumbSpan {
    /// The thumb the span starts at.
    pub from: usize,
    /// The thumb the span ends at.
    pub to: usize,
    /// Its ink.
    pub ink: SpanInk,
}

impl ThumbSpan {
    /// A span from thumb `from` to thumb `to` in `ink`.
    pub const fn new(from: usize, to: usize, ink: SpanInk) -> Self {
        ThumbSpan { from, to, ink }
    }
}

/// Where a thumb sits that a function of the row's values places (a levels gamma thumb, between the black and the
/// white one). Both functions get the row's values as drawn.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct Follow {
    /// The row value the thumb sits at, for its own value.
    pub at: fn(&[f64], f64) -> f64,
    /// The thumb's own value that puts it at row value `x`, unsnapped.
    pub value_at: fn(&[f64], f64) -> f64,
}

impl Follow {
    /// A thumb placed by `at` and set from a row position by `value_at`.
    pub const fn new(at: fn(&[f64], f64) -> f64, value_at: fn(&[f64], f64) -> f64) -> Self {
        Follow { at, value_at }
    }
}

/// One thumb of a [`ThumbRow`].
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct Thumb {
    label: Label,
    face: Option<DataColor>,
    scale: Option<Scale>,
    follow: Option<Follow>,
    order: Order,
    unset: Option<f64>,
    reset: Option<f64>,
    falling: bool,
    split: bool,
    alt_reports: bool,
}

impl Thumb {
    /// The part key of a row's thumbs: thumb `i` is `Thumb::PART.at(i)`.
    pub const PART: PartKey = PartKey::new("thumb");

    /// A thumb named `label`, free among the others, with the row's range and step. The label is the accessible
    /// name, a whole phrase the app translates; the row's own label comes before it.
    pub fn new(label: impl Into<Label>) -> Self {
        Thumb {
            label: label.into(),
            face: None,
            scale: None,
            follow: None,
            order: Order::Free,
            unset: None,
            reset: None,
            falling: false,
            split: false,
            alt_reports: false,
        }
    }

    /// The triangle's fill, a colour from data; the theme's knob colour by default.
    #[must_use]
    pub fn face(mut self, face: DataColor) -> Self {
        self.face = Some(face);
        self
    }

    /// How the value is kept among the others.
    #[must_use]
    pub fn order(mut self, order: Order) -> Self {
        self.order = order;
        self
    }

    /// A value with its own `range` and `step`, which `follow` places on the row.
    #[must_use]
    pub fn follows(mut self, range: &RangeInclusive<f64>, step: f64, follow: Follow) -> Self {
        self.scale = Some(Scale::new(range, Some(step), SliderScale::Linear));
        self.follow = Some(follow);
        self
    }

    /// Where a mixed value, or one that is not a finite number, is drawn; the start of its range by default.
    #[must_use]
    pub fn unset_at(mut self, value: f64) -> Self {
        self.unset = Some(value);
        self
    }

    /// The value a reset of this thumb restores: Reset while it is focused and a double click on it set this thumb
    /// alone; a thumb without one resets the whole binding.
    #[must_use]
    pub fn reset(mut self, value: f64) -> Self {
        self.reset = Some(value);
        self
    }

    /// The thumb is split into two halves and the next thumb of the row is its high half: this one is the low half,
    /// drawn as the left half of a triangle; the next one only brings its own face, range and order. The pair's
    /// values `[low, high]` sit in consecutive entries with `low <= high`.
    ///
    /// A drag, a key, the wheel and an AccessKit change of either half move the pair together, keeping the spread.
    /// While Alt is held a drag moves only the half it grabs, at full rate, and a key or the wheel moves the focused
    /// half alone by the plain step: Alt is not the fine scale on a split thumb. Where the halves coincide they draw
    /// as one triangle, and a press takes the half on its side of the tip. A row does not combine a split thumb with
    /// wrapping.
    #[must_use]
    pub fn split(mut self) -> Self {
        self.split = true;
        self
    }

    /// Alt held on a drag of this thumb is reported in [`ThumbRowOutput::held`] and is not the fine scale: the thumb
    /// moves at full rate (a levels black or white point, where Alt shows the clipped pixels).
    #[must_use]
    pub fn alt_reports(mut self) -> Self {
        self.alt_reports = true;
        self
    }

    /// The value falls as the thumb moves toward the end (a levels gamma): arrows, Page keys, Home, End and the
    /// wheel still move the thumb the way they point.
    #[must_use]
    pub fn falling(mut self) -> Self {
        self.falling = true;
        self
    }
}

/// The order of a wrapping row's thumbs round the period.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct Anchor {
    /// The thumb the others are measured from; it is free to turn.
    pub first: usize,
    /// The most the last thumb may be past the first, in the row's values.
    pub max_span: f64,
}

impl Anchor {
    /// Thumbs keep their index order going round from thumb `first`, the last at most `max_span` past it. The span
    /// is positive and less than the range.
    pub const fn new(first: usize, max_span: f64) -> Self {
        Anchor { first, max_span }
    }
}

/// What a key, the wheel, a screen-reader request or a tap did to a thumb.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum ThumbOp {
    /// A move in the direction of the value: `Forward` and `Up` increase it.
    Move(NavMove, MoveMods),
    /// Sets the value.
    Set(f64),
    /// A wheel event: one notch is one step, Shift scales it larger and Alt finer. The flag is whether the row reads
    /// right to left.
    Wheel(Wheel, bool),
}

/// An operation on one thumb of a row.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct RowOp {
    /// The thumb's index in the row.
    pub thumb: usize,
    /// What happens to it.
    pub op: ThumbOp,
}

impl RowOp {
    /// `op` on thumb `thumb`.
    pub const fn new(thumb: usize, op: ThumbOp) -> Self {
        RowOp { thumb, op }
    }
}

/// What a row's values may be: each thumb's range and its order among the others. The row applies every key, wheel
/// and pointer change through these rules.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct RowRules<'r, const N: usize> {
    thumbs: &'r [Thumb; N],
    scale: Scale,
    /// The period values wrap at.
    wrap: Option<f64>,
    anchored: Option<Anchor>,
    push: bool,
}

impl<const N: usize> RowRules<'_, N> {
    fn scale_of(&self, i: usize) -> Scale {
        self.thumbs[i].scale.unwrap_or(self.scale)
    }

    /// The other half of the split thumb that thumb `i` is a half of.
    fn partner(&self, i: usize) -> Option<usize> {
        if self.thumbs[i].split { Some(i + 1).filter(|j| *j < N) } else { i.checked_sub(1).filter(|j| self.thumbs[*j].split) }
    }

    /// `value` in a wrapping row: from the start of `scale`, less than a period on.
    fn wrapped(&self, scale: &Scale, value: f64) -> f64 {
        self.wrap.map_or(value, |period| wrap_value(value, scale.min, period))
    }

    /// `delta` as the short way round a wrapping row, in `-period / 2..=period / 2`.
    fn short(&self, delta: f64) -> f64 {
        self.wrap.map_or(delta, |period| delta - period * (delta / period).round())
    }

    /// The values as drawn: each in its range, and one that is mixed or not finite where its thumb says.
    pub fn placed(&self, value: &[Mixed<f64>; N]) -> [f64; N] {
        std::array::from_fn(|i| {
            let scale = self.scale_of(i);
            let place = |v: f64| if self.wrap.is_some() { self.wrapped(&scale, v) } else { scale.place(v) };
            match value[i].same() {
                Some(&v) if v.is_finite() => place(v),
                _ => self.thumbs[i].unset.map_or(scale.min, place),
            }
        })
    }

    /// Where thumb `i` may go among the thumbs it is ordered with, which are at `values`. In a wrapping row the
    /// bounds are measured the short way round from the thumb's own value, and open where it has no neighbour.
    fn order_bounds(&self, values: &[f64; N], i: usize) -> (f64, f64) {
        self.order_bounds_besides(values, i, None)
    }

    /// [`order_bounds`](Self::order_bounds), with thumb `except` not counted: it moves with thumb `i`.
    fn order_bounds_besides(&self, values: &[f64; N], i: usize, except: Option<usize>) -> (f64, f64) {
        if let Some(anchor) = self.anchored {
            return self.anchored_bounds(values, i, anchor);
        }
        let scale = self.scale_of(i);
        let open = if self.wrap.is_some() { (f64::NEG_INFINITY, f64::INFINITY) } else { (scale.min, scale.max) };
        if self.push && self.wrap.is_none() {
            let (lowest, highest) = self.push_room();
            return (lowest[i], highest[i]);
        }
        let Order::Between { below, above, gap } = self.thumbs[i].order else { return open };
        let side = |j: Option<usize>, gap: f64, end: f64| {
            j.filter(|j| Some(*j) != except).and_then(|j| values.get(j)).map_or(end, |v| {
                if self.wrap.is_some() { values[i] + self.short(v - values[i]) + gap } else { (v + gap).clamp(scale.min, scale.max) }
            })
        };
        let (lo, hi) = (side(below, gap, open.0), side(above, -gap, open.1));
        (lo.min(hi), hi)
    }

    /// The order the row keeps as pairs `(a, b, gap)`, thumb `a` at least `gap` below thumb `b`: each thumb's
    /// `Order::Between`, and each split thumb's low half below its high half.
    fn orders(&self) -> Vec<(usize, usize, f64)> {
        let mut orders = Vec::new();
        for (i, thumb) in self.thumbs.iter().enumerate() {
            if let Order::Between { below, above, gap } = thumb.order {
                orders.extend(below.filter(|k| *k < N).map(|k| (k, i, gap)));
                orders.extend(above.filter(|k| *k < N).map(|k| (i, k, gap)));
            }
            if thumb.split && i + 1 < N {
                orders.push((i, i + 1, 0.0));
            }
        }
        orders
    }

    /// The lowest and the highest value each thumb can take in a row that pushes: the range's ends, less the gaps of
    /// the thumbs it would push ahead of it.
    fn push_room(&self) -> ([f64; N], [f64; N]) {
        let mut lowest: [f64; N] = std::array::from_fn(|i| self.scale_of(i).min);
        let mut highest: [f64; N] = std::array::from_fn(|i| self.scale_of(i).max);
        let orders = self.orders();
        for _ in 0..N {
            for &(a, b, gap) in &orders {
                lowest[b] = lowest[b].max(lowest[a] + gap);
                highest[a] = highest[a].min(highest[b] - gap);
            }
        }
        (lowest, highest)
    }

    /// `to` with the thumbs a move of the thumbs in `moved` ran into pushed along, in the direction of the move, so
    /// every order holds again; `from` is where the moved thumbs were. A row that stops changes nothing.
    fn pushed(&self, from: &[f64; N], to: &mut [f64; N], moved: &[usize]) {
        if !self.push || self.wrap.is_some() {
            return;
        }
        let rising = moved.first().is_some_and(|&m| to[m] > from[m]);
        let orders = self.orders();
        for _ in 0..N * N {
            let mut changed = false;
            for &(a, b, gap) in &orders {
                if to[a] + gap <= to[b] {
                    continue;
                }
                let (fixed_a, fixed_b) = (moved.contains(&a), moved.contains(&b));
                if fixed_a && fixed_b {
                    continue;
                }
                if fixed_a || (!fixed_b && rising) {
                    to[b] = self.scale_of(b).snap(to[a] + gap);
                } else {
                    to[a] = self.scale_of(a).snap(to[b] - gap);
                }
                changed = true;
            }
            if !changed {
                break;
            }
        }
    }

    /// `row` with the thumbs in `moved` set to `to`'s values and the thumbs they ran into pushed along; the others
    /// keep `row`'s values, mixed or not.
    fn with_moved(&self, row: &[Mixed<f64>; N], to: [f64; N], moved: &[usize]) -> [Mixed<f64>; N] {
        let from = self.placed(row);
        let mut to = to;
        self.pushed(&from, &mut to, moved);
        std::array::from_fn(|k| if moved.contains(&k) || to[k] != from[k] { Mixed::Same(to[k]) } else { row[k] })
    }

    /// How far past the first thumb each thumb is, going round from it in index order: the distance from the thumb
    /// before it, the short way on, and at most the span.
    fn offsets(&self, values: &[f64; N], anchor: Anchor, period: f64) -> [f64; N] {
        let mut offset = [0.0; N];
        let mut before = anchor.first;
        for k in 1..N {
            let j = (anchor.first + k) % N;
            offset[j] = (offset[before] + (values[j] - values[before]).rem_euclid(period)).min(anchor.max_span);
            before = j;
        }
        offset
    }

    /// Where thumb `i` may go in an anchored row, in values counted from the first thumb's: a thumb between its
    /// neighbours' places and, for the last, the span; the first thumb at most half a period before the next one
    /// and so that the last stays within the span.
    fn anchored_bounds(&self, values: &[f64; N], i: usize, anchor: Anchor) -> (f64, f64) {
        let Some(period) = self.wrap else { return (f64::NEG_INFINITY, f64::INFINITY) };
        let offset = self.offsets(values, anchor, period);
        let base = values[anchor.first];
        let at = |k: usize| offset[(anchor.first + k) % N];
        match (i + N - anchor.first) % N {
            0 if N == 1 => (f64::NEG_INFINITY, f64::INFINITY),
            0 => (base + (at(1) - period / 2.0).max(at(N - 1) - anchor.max_span), base + at(1)),
            k if k + 1 == N => (base + at(k - 1), base + anchor.max_span),
            k => (base + at(k - 1), base + at(k + 1)),
        }
    }

    /// `value` as thumb `i` is aimed at it: in an anchored row the first thumb goes the short way round, the others
    /// to where the value is, counted from the first thumb forward.
    fn aim(&self, values: &[f64; N], i: usize, value: f64) -> f64 {
        match (self.anchored, self.wrap) {
            (Some(anchor), Some(period)) if i != anchor.first => wrap_value(value, values[anchor.first], period),
            (Some(_), _) => values[i] + self.short(value - values[i]),
            _ => value,
        }
    }

    /// Where thumb `i` may go, with the others at `values`: among the thumbs it is ordered with, and a half of a
    /// split thumb between its partner and the end.
    fn bounds(&self, values: &[f64; N], i: usize) -> (f64, f64) {
        let (mut lo, mut hi) = self.order_bounds(values, i);
        // In a row that pushes, a half pushes its partner instead of stopping at it.
        match self.partner(i).filter(|_| !self.push) {
            Some(j) if j > i => {
                hi = hi.min(values[j]);
                lo = lo.min(hi);
            }
            Some(j) => {
                lo = lo.max(values[j]);
                hi = hi.max(lo);
            }
            None => {}
        }
        (lo, hi)
    }

    /// Where thumb `i` may go when its split partner `j` moves with it, keeping the spread: within its own bounds
    /// among the other thumbs while the partner stays within its own.
    fn pair_bounds(&self, values: &[f64; N], i: usize, j: usize) -> (f64, f64) {
        let (lo_i, hi_i) = self.order_bounds_besides(values, i, Some(j));
        let (lo_j, hi_j) = self.order_bounds_besides(values, j, Some(i));
        let hi = (hi_i - values[i]).min(hi_j - values[j]);
        let lo = (lo_i - values[i]).max(lo_j - values[j]).min(hi);
        (values[i] + lo, values[i] + hi)
    }

    /// The split partner a move of thumb `i` takes along: none when the thumb is not split or moves `alone`.
    fn taken_along(&self, i: usize, alone: bool) -> Option<usize> {
        self.partner(i).filter(|_| !alone)
    }

    /// Where a move of thumb `i` may take it: with its partner, if it takes one, or else alone among the others.
    fn move_bounds(&self, values: &[f64; N], i: usize, alone: bool) -> (f64, f64) {
        match self.taken_along(i, alone) {
            Some(j) => self.pair_bounds(values, i, j),
            None => self.bounds(values, i),
        }
    }

    /// `row`, whose values are drawn at `values`, with thumb `i` moved to `to_i`, a value settled within its
    /// [`move_bounds`](Self::move_bounds): a split half takes its partner along by the same amount unless it moves
    /// `alone`, and the thumbs they run into are pushed along.
    fn moved_to(&self, row: &[Mixed<f64>; N], values: &[f64; N], i: usize, to_i: f64, alone: bool) -> [Mixed<f64>; N] {
        let mut to = *values;
        to[i] = to_i;
        match self.taken_along(i, alone) {
            Some(j) => {
                to[j] = values[j] + (to[i] - values[i]);
                self.with_moved(row, to, &[i, j])
            }
            None => self.with_moved(row, to, &[i]),
        }
    }

    /// The bounds AccessKit reports for thumb `i`: the range of a wrapping row, else where a change of its value may
    /// take it (a split half with its partner).
    fn reported_bounds(&self, values: &[f64; N], i: usize) -> (f64, f64) {
        let scale = self.scale_of(i);
        if self.wrap.is_some() { (scale.min, scale.max) } else { self.move_bounds(values, i, false) }
    }

    /// `value` for thumb `i`: within `bounds` and on its stops; in a wrapping row, then wrapped.
    fn settle(&self, i: usize, value: f64, (lo, hi): (f64, f64)) -> f64 {
        let scale = self.scale_of(i);
        if self.wrap.is_none() {
            return scale.snap(value).clamp(lo, hi);
        }
        let wrap = |v: f64| self.wrapped(&scale, v);
        wrap(scale.snap(wrap(value.clamp(lo, hi))))
    }

    /// `row` with `op` applied to its thumb: from where the thumb is drawn (an unset or mixed thumb steps from where
    /// it sits), on its stops and within its bounds among the others, so the order and the gap hold. The thumb takes
    /// a number; the other thumbs keep `row`'s values, mixed or not. On a half of a split thumb the op moves the pair,
    /// keeping the spread; a move with the fine flag (Alt) or a wheel event with Alt moves the half alone, by the
    /// plain step.
    pub fn apply(&self, op: &RowOp, row: &[Mixed<f64>; N]) -> [Mixed<f64>; N] {
        let values = self.placed(row);
        let i = op.thumb;
        let (op, alone) = match (self.partner(i), op.op) {
            (None, op) => (op, false),
            (Some(_), ThumbOp::Move(mv, mods)) => (ThumbOp::Move(mv, mods.fine(false)), mods.fine),
            (Some(_), ThumbOp::Wheel(wheel, rtl)) => {
                let mut plain = wheel;
                plain.mods.alt = false;
                (ThumbOp::Wheel(plain, rtl), wheel.mods.alt)
            }
            (Some(_), op @ ThumbOp::Set(_)) => (op, false),
        };
        let bounds = self.move_bounds(&values, i, alone);
        let to = if self.wrap.is_some() { self.apply_wrapped(&op, &values, i, bounds) } else { self.apply_plain(&op, &values, i, bounds) };
        self.moved_to(row, &values, i, to, alone)
    }

    /// What `op` does to the step maths: a move or a wheel event gives the new value from `steps`.
    fn stepped(op: &ThumbOp, steps: &ValueSteps) -> StepResult {
        match op {
            ThumbOp::Move(mv, mods) => step_value(steps, ValueAxes::Both, *mv, *mods),
            ThumbOp::Wheel(wheel, rtl) => wheel_step(steps, ValueAxes::Both, wheel, *rtl),
            ThumbOp::Set(value) => StepResult::Set(*value),
        }
    }

    /// The value `op` gives thumb `i`, which may go within `bounds`.
    fn apply_plain(&self, op: &ThumbOp, values: &[f64; N], i: usize, bounds: (f64, f64)) -> f64 {
        let scale = self.scale_of(i);
        let small = scale.small();
        let steps = ValueSteps::new(values[i], bounds.0, bounds.1, small, small * LARGE_STEPS);
        match Self::stepped(op, &steps) {
            StepResult::Set(next) => scale.snap(next).clamp(bounds.0, bounds.1),
            _ => values[i],
        }
    }

    /// [`apply_plain`](Self::apply_plain) for a wrapping row: a step goes on past the range's end and comes back at
    /// its start, and Home and End go to the neighbours, or the range's start and its end less a step where there
    /// are none.
    fn apply_wrapped(&self, op: &ThumbOp, values: &[f64; N], i: usize, bounds: (f64, f64)) -> f64 {
        let scale = self.scale_of(i);
        let from = if self.anchored.is_some() { self.aim(values, i, values[i]) } else { values[i] };
        let target = match op {
            ThumbOp::Set(value) => self.aim(values, i, *value),
            ThumbOp::Move(NavMove::Start, _) if bounds.0.is_finite() => bounds.0,
            ThumbOp::Move(NavMove::Start, _) => scale.min,
            ThumbOp::Move(NavMove::End, _) if bounds.1.is_finite() => bounds.1,
            ThumbOp::Move(NavMove::End, _) => scale.max - scale.small(),
            ThumbOp::Move(..) | ThumbOp::Wheel(..) => {
                let small = scale.small();
                let steps = ValueSteps::new(0.0, f64::MIN, f64::MAX, small, small * LARGE_STEPS);
                let StepResult::Set(delta) = Self::stepped(op, &steps) else { return values[i] };
                from.clamp(bounds.0, bounds.1) + delta
            }
        };
        self.settle(i, target, bounds)
    }
}

/// The thumbs on the row's track, in the row's pixels.
struct Layout<'r, const N: usize> {
    rules: RowRules<'r, N>,
    left: f32,
    right: f32,
    rtl: bool,
    /// Which thumb a press at a point moves.
    pick: Pick,
    /// The press moves only the half of a split thumb it grabs.
    alone: bool,
    off: OffThumbs,
}

impl<'r, const N: usize> Layout<'r, N> {
    fn new(rules: RowRules<'r, N>, rect: Rect, thumb: Vec2, rtl: bool, reach: f32, alone: bool, off: OffThumbs) -> Self {
        let travel = travel(rect, thumb.x);
        let (left, right) = (travel.min, travel.max);
        // A wrapping row's period in points; 0 when the row does not wrap.
        let period = match rules.wrap {
            Some(period) if rules.scale.span() > 0.0 => ((right - left) as f64 * period / rules.scale.span()) as f32,
            _ => 0.0,
        };
        Layout { rules, left, right, rtl, pick: Pick { reach, overlap: thumb.x, period, span: false }, alone, off }
    }

    fn x_of(&self, value: f64) -> f32 {
        let fraction = self.rules.scale.position(value) as f32;
        self.left + (self.right - self.left) * if self.rtl { 1.0 - fraction } else { fraction }
    }

    /// The row value thumb `i` sits at.
    fn at(&self, values: &[f64; N], i: usize) -> f64 {
        self.rules.thumbs[i].follow.map_or(values[i], |f| (f.at)(values, values[i]))
    }

    fn xs(&self, values: &[f64; N]) -> [f32; N] {
        std::array::from_fn(|i| self.x_of(self.at(values, i)))
    }

    /// What a press at `x` moves, with the thumbs at `values`: none off the thumbs in a row that ignores such a
    /// press.
    fn press(&self, values: &[f64; N], x: f32) -> Option<Press> {
        let xs = self.xs(values);
        let press = self.side_of_tip(self.pick.press(&xs, x)?, &xs, x);
        match (press, self.off) {
            (Press::Jump(_), OffThumbs::Ignore) => None,
            _ => Some(press),
        }
    }

    /// `press` on a split thumb whose halves coincide, drawn as one triangle at `xs`, as a press on the half on the
    /// side of the tip where `x` is: the low half toward the lower values. Other presses are as they are.
    fn side_of_tip(&self, press: Press, xs: &[f32; N], x: f32) -> Press {
        let pair = match press {
            Press::Either { first, last } => Some((first, last)),
            Press::Grab(i) | Press::Jump(i) => self.rules.partner(i).map(|j| (i.min(j), i.max(j))),
            Press::Span => None,
        };
        let Some((low, high)) = pair.filter(|&(low, high)| self.rules.partner(low) == Some(high) && xs[low] == xs[high]) else {
            return press;
        };
        let toward_low = if self.rtl { x > xs[low] } else { x <= xs[low] };
        let half = if toward_low { low } else { high };
        if let Press::Jump(_) = press { Press::Jump(half) } else { Press::Grab(half) }
    }

    /// The pieces of `span` along the row, as x ranges: between its thumbs, and in a wrapping row forward from the
    /// first round to the second, in two pieces when that passes the end of the range.
    fn span_xs(&self, values: &[f64; N], span: &ThumbSpan) -> Vec<(f32, f32)> {
        if span.from >= N || span.to >= N {
            return Vec::new();
        }
        let (a, b) = (self.at(values, span.from), self.at(values, span.to));
        let scale = &self.rules.scale;
        let pieces = if self.rules.wrap.is_some() && b < a { vec![(a, scale.max), (scale.min, b)] } else { vec![(a.min(b), a.max(b))] };
        pieces
            .into_iter()
            .map(|(lo, hi)| {
                let (x, y) = (self.x_of(lo), self.x_of(hi));
                (x.min(y), x.max(y))
            })
            .collect()
    }

    /// The values with `press` dragged to the position `state` holds: the moved thumb from where it was at `anchor`
    /// (the values the gesture began from), the others as they were, mixed or not. A grabbed thumb moves by the
    /// pointer's travel and stays while the pointer is still; a jumping one goes to the pointer. The moved value
    /// snaps to its step and stays in its bounds among the others. A half of a split thumb takes its partner along,
    /// keeping the spread, unless the press moves it alone. `click` is a release that was not a drag: Alt does not
    /// scale it.
    fn moved(&self, anchor: &[Mixed<f64>; N], press: Press, state: &PressState, click: bool) -> [Mixed<f64>; N] {
        let out = *anchor;
        let Some((origin, pos)) = track::pointer(state) else { return out };
        let Some(i) = press.thumb(origin.x, pos.x) else { return out };
        if press.grabs() && pos.x == origin.x {
            return out;
        }
        let rules = &self.rules;
        let from = rules.placed(anchor);
        let placed_at = if rules.anchored.is_some() { rules.aim(&from, i, self.at(&from, i)) } else { self.at(&from, i) };
        let travel = Travel { track: self.left..self.right, units: rules.scale.min..=rules.scale.max, rtl: self.rtl };
        // Alt moves one half of a split thumb alone: that drag is not scaled finer.
        let fine = !click && !rules.thumbs[i].alt_reports && !(self.alone && rules.partner(i).is_some());
        let Some(at) = travel.value(press, placed_at, state, fine) else { return out };
        let value = rules.thumbs[i].follow.map_or(at, |f| (f.value_at)(&from, at));
        // A jump in a wrapping row goes the short way round, in an anchored row forward from the first thumb.
        let value = if press.grabs() || rules.wrap.is_none() {
            value
        } else if rules.anchored.is_some() {
            rules.aim(&from, i, value)
        } else {
            from[i] + rules.short(value - from[i])
        };
        let to = rules.settle(i, value, rules.move_bounds(&from, i, self.alone));
        rules.moved_to(anchor, &from, i, to, self.alone)
    }
}

/// A half of a split thumb, drawn as the half of a triangle on its side of the tip's vertical.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Half {
    Low,
    High,
}

/// The horizontal extent of a thumb centred on `x` with `width`, or of one `half` of it. The low half lies toward the
/// lower values: on the left in a left-to-right row and on the right in a right-to-left one.
fn span(x: f32, width: f32, half: Option<Half>, rtl: bool) -> (f32, f32) {
    let (left, right) = (x - width / 2.0, x + width / 2.0);
    match (half, rtl) {
        (None, _) => (left, right),
        (Some(Half::Low), false) | (Some(Half::High), true) => (left, x),
        (Some(Half::High), false) | (Some(Half::Low), true) => (x, right),
    }
}

/// The triangle, tip up at `tip`, or one `half` of it.
fn triangle(tip: Pos2, size: Vec2, half: Option<Half>, rtl: bool) -> Vec<Pos2> {
    let (left, right) = span(tip.x, size.x, half, rtl);
    vec![tip, pos2(right, tip.y + size.y), pos2(left, tip.y + size.y)]
}

/// The values of the running drag when it began and the press it resolved to.
#[derive(Clone, Default)]
struct Held {
    anchor: Vec<Mixed<f64>>,
    press: Option<Press>,
    /// The thumb the pointer moves now, and the modifiers held in the latest pass of the drag.
    thumb: Option<usize>,
    mods: Mods,
}

/// The thumb a running drag moves and the modifiers held on it.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct ThumbHold {
    /// The thumb's index in the row.
    pub thumb: usize,
    /// The modifiers held in this pass.
    pub mods: Mods,
}

/// What a [`ThumbRow`] reports: the edit, and the thumb being dragged.
#[derive(Debug)]
#[non_exhaustive]
pub struct ThumbRowOutput<const N: usize> {
    /// The row's edit.
    pub edit: EditResponse<[Mixed<f64>; N]>,
    /// The drag in progress: its thumb and the modifiers held, so an editor can show what Alt means on a thumb that
    /// asked for [`Thumb::alt_reports`]. `None` when no drag runs.
    pub held: Option<ThumbHold>,
    /// The horizontal span the thumbs' centres travel along: the row's rect inset by half a thumb on each side. A
    /// thumb sits at one end of it at the range minimum and at the other at the maximum (the right end under a
    /// right-to-left flow). An editor places a plot or bar over it so the thumbs line up with what they edit.
    pub travel: Rangef,
}

impl<const N: usize> Outcome for ThumbRowOutput<N> {
    type Value = [Mixed<f64>; N];

    fn response(&self) -> &egui::Response {
        &self.edit.response
    }

    #[cfg(feature = "testing")]
    fn story_facts(&self) -> crate::testing::StoryOutput {
        self.edit.story_facts()
    }
}

/// The direction of the thumb when the value falls toward the end of the row.
fn against(mv: NavMove) -> NavMove {
    match mv {
        NavMove::Forward => NavMove::Back,
        NavMove::Back => NavMove::Forward,
        NavMove::Up => NavMove::Down,
        NavMove::Down => NavMove::Up,
        NavMove::PageUp => NavMove::PageDown,
        NavMove::PageDown => NavMove::PageUp,
        NavMove::Start => NavMove::End,
        NavMove::End => NavMove::Start,
        other => other,
    }
}

/// Thumbs on one row, tip up, each a value of its own. The row spans the width its flow offers, so it lines up with
/// the bars it marks positions on.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(probe(ThumbRow<'static, 4>))]
pub struct ThumbRow<'a, const N: usize> {
    #[common]
    common: Common,
    value: Bind<'a, [Mixed<f64>; N]>,
    range: RangeInclusive<f64>,
    /// The thumbs, one per value; unnamed free thumbs by default.
    #[prop]
    thumbs: [Thumb; N],
    /// The change of one arrow press; the default is a hundredth of the range.
    #[prop(quantity = "step")]
    step: Option<f64>,
    /// Values wrap at the length of the range (a hue bar): a thumb dragged or stepped past the end appears at the
    /// start. Distances between thumbs are measured the short way round.
    #[prop(flag)]
    wrap: bool,
    /// Thumbs keep their index order going round the period from one anchor thumb; implies `wrap`. An `Order` on a
    /// thumb of an anchored row has no meaning and stays `Free`.
    #[prop]
    anchor: Option<Anchor>,
    /// What a thumb does when a move takes it into a thumb it is ordered with: stop (the default) or push it along.
    #[prop]
    neighbours: Neighbours,
    /// What a press off the thumbs does: the nearest thumb jumps there (the default), only takes a drag, or nothing.
    #[prop]
    off_thumbs: OffThumbs,
    /// Bands along the row between thumbs, under their tips; none by default.
    #[prop]
    spans: &'a [ThumbSpan],
    /// Paints a groove above the thumbs with `face`, a component shown as the look of the track (a colour gradient);
    /// the thumbs point up at it.
    #[prop]
    track_face: Option<TrackFace<'a>>,
}

/// The values of a row whose binding is mixed as a whole: every part is mixed.
fn all_mixed<const N: usize>() -> [Mixed<f64>; N] {
    std::array::from_fn(|_| Mixed::Mixed)
}

/// The part of `rect` the thumbs use: all of it, less the groove of a track face above them.
fn band(rect: Rect, groove: Option<f32>) -> Rect {
    Rect::from_min_max(pos2(rect.left(), rect.top() + groove.unwrap_or(0.0)), rect.max)
}

/// The accessible name of a thumb: the row's label, then the thumb's own.
fn named(row: &str, thumb: String) -> Name {
    match (row.is_empty(), thumb.is_empty()) {
        (true, _) => Name::Text(thumb),
        (false, true) => Name::Text(row.to_owned()),
        (false, false) => Name::Joined(vec![row.to_owned(), thumb]),
    }
}

impl<'a, const N: usize> ThumbRow<'a, N> {
    /// A row editing `value` over `range`.
    pub fn new(value: Bind<'a, [Mixed<f64>; N]>, range: RangeInclusive<f64>) -> Self {
        ThumbRow {
            common: Common::new(),
            value,
            range,
            thumbs: std::array::from_fn(|_| Thumb::new("")),
            step: None,
            wrap: false,
            anchor: None,
            neighbours: Neighbours::Stop,
            off_thumbs: OffThumbs::Jump,
            spans: &[],
            track_face: None,
        }
    }

    /// What the values of this row may be.
    pub fn rules(&self) -> RowRules<'_, N> {
        let scale = Scale::new(&self.range, self.step, SliderScale::Linear);
        let wrap = (self.wrap || self.anchor.is_some()).then_some(scale.span()).filter(|period| *period > 0.0);
        let anchored = self.anchor.filter(|a| wrap.is_some() && a.first < N);
        RowRules { thumbs: &self.thumbs, scale, wrap, anchored, push: self.neighbours == Neighbours::Push }
    }

    fn halves(&self, rules: &RowRules<'_, N>, shown: &[f64; N]) -> [Option<Half>; N] {
        std::array::from_fn(|i| match rules.partner(i) {
            Some(j) if shown[i] != shown[j] => Some(if j > i { Half::Low } else { Half::High }),
            _ => None,
        })
    }
}

impl<const N: usize> Component for ThumbRow<'_, N> {
    const ROLE: Role = Role::GROUP;
    const OVERLAYS: bool = false;
    const KEYS: KeyUse = KeyUse::of(Input::Pointer).with_keep(KeepKeys::BlockArrows).with_activate(ActivateKeys::Neither);
    type Style = ThumbRowStyle;
    type Output = ThumbRowOutput<N>;

    fn style(cx: &StyleCx<'_>, _: ThumbRowStyleKey<N>) -> ThumbRowStyle {
        let control = cx.control();
        let colors = &cx.tokens.color;
        let length = cx.tokens.size.length.track;
        let fixed = |rest: Rgba, disabled: Rgba| {
            Stateful::new(cx.color(rest)).off(States::new(cx.color(rest), cx.color(rest), cx.color(rest), cx.color(disabled)))
        };
        ThumbRowStyle {
            metrics: Metrics::new().min_size(Vec2::new(length, control.height)),
            focus: cx.focus_look(),
            face: fixed(colors.surface.knob, colors.state.disabled_fill),
            edge: fixed(colors.text.muted, colors.text.disabled),
            ring: ThumbRing::new(cx),
            hit: cx.tokens.size.hit.target,
            thumb: Vec2::new(cx.tokens.size.marker.width, cx.tokens.size.marker.height),
            track: TrackLook { thickness: cx.tokens.size.ramp.bar, ..cx.track() },
            length,
            span_faint: cx.color(colors.text.faint),
            span_muted: cx.color(colors.text.muted),
            span: cx.tokens.size.ramp.groove,
        }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let groove = if self.track_face.is_some() { cx.style.track.thickness } else { 0.0 };
        Vec2::new(cx.style.length, cx.icon() + groove)
    }

    fn fills(&self) -> Option<craft_behavior::Axis> {
        Some(craft_behavior::Axis::X)
    }

    /// No wheel bits: as a value control the row takes the vertical wheel only while focused or armed, and no scope
    /// frees it, since a wheel over the track has no one thumb to step.
    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let rules = self.rules();
        debug_assert!(
            rules.anchored.is_none() || rules.thumbs.iter().all(|t| t.order == Order::Free),
            "an anchored row orders its thumbs itself; their `Order` stays `Free`"
        );
        debug_assert!(
            rules.thumbs.iter().enumerate().all(|(i, t)| !t.split || (i + 1 < N && !rules.thumbs[i + 1].split && rules.wrap.is_none())),
            "a split thumb needs a high half after it, which is not split itself, in a row that does not wrap"
        );
        let row = cx.value().cloned().unwrap_or_else(all_mixed);
        let shown = rules.placed(&row);
        let size = cx.style.thumb;
        let layout = Layout::new(rules, rect, size, cx.flow.is_rtl(), 0.0, false, self.off_thumbs);
        let xs = layout.xs(&shown);
        let groove = self.track_face.is_some().then_some(cx.style.track.thickness);
        let thumbs = band(rect, groove);
        let top = thumbs.top();
        let halves = self.halves(&rules, &shown);
        if let Some(thickness) = groove {
            // The groove spans the thumbs' travel, so the end thumbs' tips sit on its ends.
            let span = travel(rect, cx.style.thumb.x);
            cx.part(track::GROOVE, Rect::from_x_y_ranges(span, rect.min.y..=rect.min.y + thickness));
        }
        cx.part(ROW, rect).sense(Sense::CLICK | Sense::DRAG).value_part().pointer_only("the arrow keys step the focused thumb");
        for i in 0..N {
            let (left, right) = span(xs[i], size.x, halves[i], cx.flow.is_rtl());
            let thumb = Rect::from_min_size(pos2(left, top), Vec2::new(right - left, size.y));
            let part = cx
                .part(Thumb::PART.at(i as u64), thumb)
                .sense(Sense::DRAG)
                .priority(HitPriority::new(HitLevel::Handle, 0))
                .focusable()
                .focus_shape(FocusShape::Rect)
                .value_part();
            if rules.thumbs[i].reset.is_some() {
                part.own_reset();
            }
        }
    }

    fn bind(&mut self) -> Option<BindMut<'_, [Mixed<f64>; N]>> {
        Some(self.value.reborrow())
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let rules = self.rules();
        let (rect, rtl, reach, off) = (cx.rect, cx.flow.is_rtl(), cx.style.hit, self.off_thumbs);
        let size = cx.style.thumb;
        let row = cx.value().cloned().unwrap_or_else(all_mixed);
        let parts: Vec<PartRef> = std::iter::once(PartRef::from(ROW)).chain((0..N).map(|i| Thumb::PART.at(i as u64))).collect();
        for part in parts {
            let press = cx.part(part).press;
            match press.phase {
                PressPhase::DragStart { start, pos, .. } => {
                    // Alt moves a half of a split thumb alone while it is held, from the values the drag began at.
                    let layout = Layout::new(rules, rect, size, rtl, reach, press.mods.alt, off);
                    if let Some(picked) = layout.press(&rules.placed(&row), start.origin.x) {
                        let thumb = picked.thumb(start.origin.x, pos.x);
                        *cx.memory::<Held>() = Held { anchor: row.to_vec(), press: Some(picked), thumb, mods: press.mods };
                        if let Some(thumb) = thumb {
                            cx.focus_part(Thumb::PART.at(thumb as u64));
                        }
                        cx.begin();
                        cx.propose(layout.moved(&row, picked, &press, false));
                    }
                }
                PressPhase::Drag { start, pos, .. } => {
                    let held = cx.memory::<Held>().clone();
                    if let Some(picked) = held.press {
                        let now = cx.memory::<Held>();
                        now.thumb = picked.thumb(start.origin.x, pos.x);
                        now.mods = press.mods;
                    }
                    if let (Some(picked), Ok(anchor)) = (held.press, <[Mixed<f64>; N]>::try_from(held.anchor)) {
                        let layout = Layout::new(rules, rect, size, rtl, reach, press.mods.alt, off);
                        cx.propose(layout.moved(&anchor, picked, &press, false));
                    }
                }
                PressPhase::Release { start, click, .. } => {
                    if cx.gesture().is_some() {
                        let held = cx.memory::<Held>().clone();
                        if let (Some(picked), Ok(anchor)) = (held.press, <[Mixed<f64>; N]>::try_from(held.anchor)) {
                            let layout = Layout::new(rules, rect, size, rtl, reach, press.mods.alt, off);
                            cx.propose(layout.moved(&anchor, picked, &press, false));
                        }
                        cx.commit();
                    } else if click && start.clicks < 2 {
                        // A tap off the thumbs focuses the nearest one and, in a row whose thumbs jump, moves it there;
                        // a tap on a thumb moves none. A second click is the reset, which the pipeline applies.
                        let layout = Layout::new(rules, rect, size, rtl, reach, false, off);
                        if let Some(picked @ Press::Jump(thumb)) = layout.press(&rules.placed(&row), start.origin.x) {
                            if off == OffThumbs::Jump
                                && let Some(&target) = layout.moved(&row, picked, &press, true)[thumb].same()
                            {
                                cx.set(rules.apply(&RowOp::new(thumb, ThumbOp::Set(target)), &row));
                            }
                            cx.focus_part(Thumb::PART.at(thumb as u64));
                        }
                    }
                }
                _ => {}
            }
        }
        let current = |cx: &Cx<'_, Self>| cx.value().cloned().unwrap_or_else(all_mixed);
        if let Some(part) = cx.reset_part().filter(|part| part.key == Thumb::PART)
            && let Some(thumb) = part.index.map(|i| i as usize).filter(|i| *i < N)
            && let Some(value) = rules.thumbs[thumb].reset
        {
            cx.set(rules.apply(&RowOp::new(thumb, ThumbOp::Set(value)), &current(&*cx)));
        }
        // A running drag is one edit: keys, the wheel and SetValue wait for it.
        let running = cx.gesture().is_some();
        for thumb in 0..N {
            if let Some(next) = cx.intents().take_set_value_at(Thumb::PART.at(thumb as u64)).filter(|_| !running) {
                cx.set(rules.apply(&RowOp::new(thumb, ThumbOp::Set(next)), &current(&*cx)));
            }
        }
        let focused = cx.active_part().filter(|part| part.key == Thumb::PART).and_then(|part| part.index).map(|i| i as usize).filter(|i| *i < N);
        let Some(thumb) = focused else { return };
        let falling = rules.thumbs[thumb].falling;
        let direction = |mv: NavMove| if falling { against(mv) } else { mv };
        while let Some((mv, mods)) = cx.intents().take_move() {
            if !running {
                cx.set(rules.apply(&RowOp::new(thumb, ThumbOp::Move(direction(mv), mods)), &current(&*cx)));
            }
        }
        if let Some(wheel) = cx.take_wheel().filter(|_| !running) {
            // A falling thumb moves the way the wheel points, so its value goes the other way.
            let wheel = if falling { Wheel::new(-wheel.scroll, wheel.mods, wheel.unit, wheel.phase) } else { wheel };
            cx.set(rules.apply(&RowOp::new(thumb, ThumbOp::Wheel(wheel, rtl)), &current(&*cx)));
        }
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let rules = self.rules();
        let words = cx.words::<ValueDragWords>();
        let row = cx.value().cloned().unwrap_or_else(all_mixed);
        let shown = rules.placed(&row);
        let text = |thumb: &Thumb| cx.env.resolve(&thumb.label).as_str().to_owned();
        for i in 0..N {
            let (lo, hi) = rules.reported_bounds(&shown, i);
            let own = text(&rules.thumbs[i]);
            let name = match rules.partner(i) {
                Some(j) if j > i => words.thumb_low.fill(&[("name", own.as_str())]),
                Some(j) => words.thumb_high.fill(&[("name", text(&rules.thumbs[j]).as_str())]),
                None => own,
            };
            let step = rules.scale_of(i).small();
            let numeric = match row[i].same() {
                Some(value) if value.is_finite() => Numeric::new(shown[i], lo..=hi),
                _ => Numeric::empty(lo..=hi),
            };
            let node = access.node(Thumb::PART.at(i as u64), Role::SLIDER);
            node.name = named(cx.resolved.label, name);
            node.numeric = Some(numeric.step(step).wraps(rules.wrap.is_some()));
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let rules = self.rules();
        let style = cx.style;
        if let Some(face) = &self.track_face {
            face.paint(ui);
        }
        let draw = ui.draw();
        let row = cx.value().cloned().unwrap_or_else(all_mixed);
        let shown = rules.placed(&row);
        let size = style.thumb;
        let layout = Layout::new(rules, cx.rect, size, cx.flow.is_rtl(), style.hit, false, self.off_thumbs);
        let xs = layout.xs(&shown);
        let running = cx.gesture().and_then(|_| cx.recall::<Held>().press);
        let hovered = std::iter::once(PartRef::from(ROW))
            .chain((0..N).map(|i| Thumb::PART.at(i as u64)))
            .find_map(|part| cx.hover_pos(part))
            .and_then(|at| layout.press(&shown, at.x));
        let thumbs = band(cx.rect, self.track_face.is_some().then_some(style.track.thickness));
        let top = thumbs.top();
        let halves = self.halves(&rules, &shown);
        for span in self.spans {
            let ink = match span.ink {
                SpanInk::Faint => style.span_faint,
                SpanInk::Muted => style.span_muted,
            };
            for (left, right) in layout.span_xs(&shown, span) {
                draw.fill(&PaintShape::Rect(Rect::from_x_y_ranges(left..=right, top..=top + style.span), CornerRadius::ZERO), ink);
            }
        }
        for i in 0..N {
            // Coinciding halves are one triangle, which the low half draws.
            if halves[i].is_none() && rules.partner(i).is_some_and(|j| j < i) {
                continue;
            }
            let points = triangle(pos2(xs[i], top), size, halves[i], cx.flow.is_rtl());
            let part = Thumb::PART.at(i as u64);
            let state = cx.core.part_state_lit(part, track::lit(running, hovered, i));
            style.ring.paint_pointer(&draw, &points, state.resolve(&style.ring.color));
            let shape = PaintShape::Polygon(points);
            match rules.thumbs[i].face {
                Some(face) => draw.data_fill(&shape, face),
                None => draw.fill(&shape, state.resolve(&style.face)),
            }
            draw.stroke(&shape, StrokeWidth::Thin, state.resolve(&style.edge));
        }
    }

    fn finish(self, mut cx: Cx<'_, Self>) -> ThumbRowOutput<N> {
        let running = cx.gesture().is_some();
        let travel = travel(cx.rect, cx.style.thumb.x);
        let held = running.then(|| cx.memory::<Held>().clone()).and_then(|held| held.thumb.map(|thumb| ThumbHold { thumb, mods: held.mods }));
        ThumbRowOutput { edit: cx.edit_response(), held, travel }
    }
}
