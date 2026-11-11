//! The stop rules of a gradient bar, as pure functions the bar and an on-canvas gradient line
//! share. Every edit returns the new stops and the index of the stop it touched, so a selection
//! follows a stop across reorders; `None` when the rules refuse the edit.
//!
//! Stops keep their own midpoints and tags when they move. Locations are clamped to 0..=1; the
//! bar rounds pointer locations with [`round_location`] before it gives them to the rules, so a
//! typed location keeps its decimals.

use std::ops::RangeInclusive;

use craft_theme::Srgba;

use super::value::{ColorStop, GradientStops, OpacityStop, StopRow, StopRows};

/// Gives the colour, with alpha, of a gradient at a location 0..=1. Draws the ramp and colours
/// new stops; the default is [`default_sampler`].
pub type Sampler = dyn Fn(&GradientStops, f32) -> Srgba;

/// What a host allows of the stops of one row.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct StopRules {
    /// The row layout.
    pub rows: StopRows,
    /// Fewest stops a row keeps (PhotoCraft 2, EffectCraft 1).
    pub min: usize,
    /// Most stops a row holds (PhotoCraft 1024, its Gradient Map 64).
    pub max: usize,
    /// The range a midpoint may take (PhotoCraft 0.05..=0.95; InDesign and Illustrator 0.13..=0.87).
    pub midpoint: RangeInclusive<f32>,
}

impl StopRules {
    /// The default rules for `rows`.
    pub fn new(rows: StopRows) -> Self {
        Self { rows, ..Self::default() }
    }

    /// The fewest and most stops a row keeps; `max` is at least `min`.
    #[must_use]
    pub fn stops(mut self, min: usize, max: usize) -> Self {
        self.min = min;
        self.max = max.max(min);
        self
    }

    /// The range a midpoint may take.
    #[must_use]
    pub fn midpoint_range(mut self, range: RangeInclusive<f32>) -> Self {
        self.midpoint = range;
        self
    }
}

impl Default for StopRules {
    fn default() -> Self {
        Self { rows: StopRows::Split, min: 2, max: 1024, midpoint: 0.05..=0.95 }
    }
}

/// One Left or Right press moves a stop or midpoint by this share of the bar or line.
pub const STOP_STEP: f32 = 0.01;
/// One Shift press.
pub const STOP_STEP_FAR: f32 = 0.10;

/// `location` clamped to 0..=1 and rounded to 0.001, as pointer locations are (PhotoCraft's
/// Gradient Map).
pub fn round_location(location: f32) -> f32 {
    (clamp_location(location) * 1000.0).round() / 1000.0
}

/// `location` clamped to 0..=1; 0 when it isn't a number.
fn clamp_location(location: f32) -> f32 {
    if location.is_finite() { location.clamp(0.0, 1.0) } else { 0.0 }
}

/// What the two stop types have in common for the rules.
pub(crate) trait Stop: Clone + PartialEq {
    fn location(&self) -> f32;
    fn set_location(&mut self, location: f32);
    fn midpoint(&self) -> f32;
    fn set_midpoint(&mut self, midpoint: f32);
}

impl Stop for ColorStop {
    fn location(&self) -> f32 {
        self.location
    }
    fn set_location(&mut self, location: f32) {
        self.location = location;
    }
    fn midpoint(&self) -> f32 {
        self.midpoint
    }
    fn set_midpoint(&mut self, midpoint: f32) {
        self.midpoint = midpoint;
    }
}

impl Stop for OpacityStop {
    fn location(&self) -> f32 {
        self.location
    }
    fn set_location(&mut self, location: f32) {
        self.location = location;
    }
    fn midpoint(&self) -> f32 {
        self.midpoint
    }
    fn set_midpoint(&mut self, midpoint: f32) {
        self.midpoint = midpoint;
    }
}

/// The location of stop `i` of `row`.
pub(crate) fn location(stops: &GradientStops, row: StopRow, i: usize) -> f32 {
    match row {
        StopRow::Color => stops.color[i].location,
        StopRow::Opacity => stops.opacity_stops()[i].location,
    }
}

/// The midpoint after stop `i` of `row`.
pub(crate) fn midpoint(stops: &GradientStops, row: StopRow, i: usize) -> f32 {
    match row {
        StopRow::Color => stops.color[i].midpoint,
        StopRow::Opacity => stops.opacity_stops()[i].midpoint,
    }
}

/// The locations of `row`'s stops as the bar shows them.
pub(crate) fn locations(stops: &GradientStops, row: StopRow) -> Vec<f32> {
    match row {
        StopRow::Color => stops.color.iter().map(|s| s.location).collect(),
        StopRow::Opacity => stops.opacity_stops().iter().map(|s| s.location).collect(),
    }
}

/// Where the midpoint after stop `i` of `row` is, 0..=1 of the bar.
pub(crate) fn midpoint_at(stops: &GradientStops, row: StopRow, i: usize) -> f32 {
    let at = locations(stops, row);
    at[i] + (at[i + 1] - at[i]) * midpoint(stops, row, i)
}

/// Inserts `stop` after the stops at its location. Returns its index.
fn place<T: Stop>(list: &mut Vec<T>, stop: T) -> usize {
    let at = list.partition_point(|s| s.location() <= stop.location());
    list.insert(at, stop);
    at
}

/// Inserts `stop` and resets the midpoint of the segment it splits.
fn insert<T: Stop>(list: &mut Vec<T>, stop: T) -> usize {
    let at = place(list, stop);
    if let Some(before) = at.checked_sub(1) {
        list[before].set_midpoint(0.5);
    }
    at
}

/// The stops to edit in `row`: the opacity list is written out when it was empty and an opacity
/// stop is edited.
fn explicit(stops: &GradientStops, row: StopRow) -> GradientStops {
    match row {
        StopRow::Color => stops.clone(),
        StopRow::Opacity => GradientStops { color: stops.color.clone(), opacity: stops.opacity_stops().into_owned() },
    }
}

fn has_row(rules: &StopRules, row: StopRow) -> bool {
    row == StopRow::Color || rules.rows == StopRows::Split
}

/// A new stop at `location` in `row`, taking the gradient's colour or opacity there from
/// `sampler`, selected by its index. The segment it splits gets its midpoint reset to 0.5. Under
/// [`StopRows::Split`] and [`StopRows::ColorOnly`] a colour stop is opaque.
pub fn add(stops: &GradientStops, row: StopRow, location: f32, sampler: &Sampler, rules: &StopRules) -> Option<(GradientStops, usize)> {
    if !has_row(rules, row) || stops.len(row) >= rules.max {
        return None;
    }
    let location = clamp_location(location);
    let sampled = sampler(stops, location);
    let mut out = explicit(stops, row);
    let at = match row {
        StopRow::Color => {
            let color = if rules.rows == StopRows::Combined { sampled } else { Srgba { a: 1.0, ..sampled } };
            insert(&mut out.color, ColorStop::new(location, color))
        }
        StopRow::Opacity => insert(&mut out.opacity, OpacityStop::new(location, sampled.a)),
    };
    Some((out, at))
}

/// Moves stop `i` of `row` to `location`. It may pass its neighbours; it keeps its place among
/// stops at its new location, on the side it came from, so a stop that doesn't move or only
/// reaches a neighbour's location never swaps with it. Returns its new index.
pub fn move_to(stops: &GradientStops, row: StopRow, i: usize, location: f32, rules: &StopRules) -> Option<(GradientStops, usize)> {
    if !has_row(rules, row) || i >= stops.len(row) {
        return None;
    }
    let location = clamp_location(location);
    let mut out = explicit(stops, row);
    let at = match row {
        StopRow::Color => relocate(&mut out.color, i, location),
        StopRow::Opacity => relocate(&mut out.opacity, i, location),
    };
    Some((out, at))
}

fn relocate<T: Stop>(list: &mut Vec<T>, i: usize, location: f32) -> usize {
    let mut stop = list.remove(i);
    let from = stop.location();
    stop.set_location(location);
    // Moving up it stops before the stops at its new location, moving down after them.
    let at = match location > from {
        true => list.partition_point(|s| s.location() < location),
        false if location == from => i,
        false => list.partition_point(|s| s.location() <= location),
    };
    list.insert(at, stop);
    at
}

/// Removes stop `i` of `row`, never below `rules.min`. Returns the index of the stop that takes
/// its place: the next, or the last when it was the last.
pub fn remove(stops: &GradientStops, row: StopRow, i: usize, rules: &StopRules) -> Option<(GradientStops, usize)> {
    let len = stops.len(row);
    if !has_row(rules, row) || i >= len || len <= rules.min {
        return None;
    }
    let mut out = explicit(stops, row);
    match row {
        StopRow::Color => drop(out.color.remove(i)),
        StopRow::Opacity => drop(out.opacity.remove(i)),
    }
    Some((out, i.min(len - 2)))
}

/// A copy of stop `i` of `row` at `location` (an Alt-drag), with its colour or opacity, midpoint
/// and tag. The segment it splits gets its midpoint reset to 0.5. Returns the copy's index.
pub fn copy(stops: &GradientStops, row: StopRow, i: usize, location: f32, rules: &StopRules) -> Option<(GradientStops, usize)> {
    if !has_row(rules, row) || i >= stops.len(row) || stops.len(row) >= rules.max {
        return None;
    }
    let location = clamp_location(location);
    let mut out = explicit(stops, row);
    let at = match row {
        StopRow::Color => insert(&mut out.color, ColorStop { location, ..stops.color[i] }),
        StopRow::Opacity => {
            let source = out.opacity[i];
            insert(&mut out.opacity, OpacityStop { location, ..source })
        }
    };
    Some((out, at))
}

/// Sets the midpoint after stop `i` of `row`, clamped to `rules.midpoint`. The last stop has none.
pub fn set_midpoint(stops: &GradientStops, row: StopRow, i: usize, midpoint: f32, rules: &StopRules) -> Option<(GradientStops, usize)> {
    if !has_row(rules, row) || i + 1 >= stops.len(row) {
        return None;
    }
    let m = if midpoint.is_finite() { midpoint.clamp(*rules.midpoint.start(), *rules.midpoint.end()) } else { 0.5 };
    let mut out = explicit(stops, row);
    match row {
        StopRow::Color => out.color[i].midpoint = m,
        StopRow::Opacity => out.opacity[i].midpoint = m,
    }
    Some((out, i))
}

/// The location halfway between the selected stop of `row` and the next, or the one before for
/// the last stop; with nothing selected, across the widest gap between stops. A row with one
/// stop is halved towards its farther end of the bar, and an empty row at 0.5.
pub fn halfway(stops: &GradientStops, row: StopRow, selected: Option<usize>) -> f32 {
    let at: Vec<f32> = match row {
        StopRow::Color => stops.color.iter().map(|s| s.location).collect(),
        StopRow::Opacity => stops.opacity_stops().iter().map(|s| s.location).collect(),
    };
    let middle = |a: f32, b: f32| round_location((a + b) / 2.0);
    match (at.as_slice(), selected) {
        ([], _) => 0.5,
        ([only], _) => middle(*only, if *only < 0.5 { 1.0 } else { 0.0 }),
        (at, Some(i)) if i < at.len() => {
            if i + 1 < at.len() {
                middle(at[i], at[i + 1])
            } else {
                middle(at[i - 1], at[i])
            }
        }
        (at, _) => {
            let widest = (0..at.len() - 1).fold(0, |best, i| if at[i + 1] - at[i] > at[best + 1] - at[best] { i } else { best });
            middle(at[widest], at[widest + 1])
        }
    }
}

/// The mix factor at `location` between the stops of `list` it lies between, with the segment's
/// midpoint bent to half: `(from, to, mix)`.
fn segment<T: Stop>(list: &[T], location: f32) -> (usize, usize, f32) {
    let last = list.len() - 1;
    if location <= list[0].location() {
        return (0, 0, 0.0);
    }
    for (i, pair) in list.windows(2).enumerate() {
        let (a, b) = (&pair[0], &pair[1]);
        if location <= b.location() {
            let u = (location - a.location()) / (b.location() - a.location()).max(1e-6);
            let m = a.midpoint().clamp(0.01, 0.99);
            let v = if u < m { 0.5 * u / m } else { 0.5 + 0.5 * (u - m) / (1.0 - m) };
            return (i, i + 1, v);
        }
    }
    (last, last, 0.0)
}

fn mix(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// The gradient at `location` with straight lines in encoded sRGB between stops, each segment
/// bent so its midpoint gives the half mix. Under [`StopRows::Split`] the alpha comes from the
/// opacity stops (opaque when there are none) and the colours' own alpha is ignored; under
/// [`StopRows::Combined`] it is the colours' alpha; under [`StopRows::ColorOnly`] it is 1. No colour stops give opaque black.
pub fn sample(stops: &GradientStops, rows: StopRows, location: f32) -> Srgba {
    if stops.color.is_empty() {
        return Srgba::BLACK;
    }
    let (i, j, t) = segment(&stops.color, location);
    let (a, b) = (stops.color[i].color, stops.color[j].color);
    let mut out = Srgba::new(mix(a.r, b.r, t), mix(a.g, b.g, t), mix(a.b, b.b, t), mix(a.a, b.a, t));
    match rows {
        StopRows::Split if !stops.opacity.is_empty() => {
            let (i, j, t) = segment(&stops.opacity, location);
            out.a = mix(stops.opacity[i].opacity, stops.opacity[j].opacity, t);
        }
        StopRows::Split | StopRows::ColorOnly => out.a = 1.0,
        StopRows::Combined => {}
    }
    out
}

/// The default [`Sampler`] for `rows`: [`sample`].
pub fn default_sampler(rows: StopRows) -> impl Fn(&GradientStops, f32) -> Srgba {
    move |stops, location| sample(stops, rows, location)
}
