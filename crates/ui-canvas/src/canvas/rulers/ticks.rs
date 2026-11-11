//! The tick rule of a ruler: which values carry a tick and which of them are labelled.

use crate::primitives::TickRank;
use craft_behavior::RulerUnits;
use std::ops::RangeInclusive;

/// Word's inch ruler: sixteenths, down to halves.
pub const INCH_SIXTEENTHS: [u32; 4] = [16, 8, 4, 2];
/// DesignCraft's inch ruler: eighths, down to halves.
pub const INCH_EIGHTHS: [u32; 3] = [8, 4, 2];

/// The least distance between labelled ticks, in points.
pub(crate) const LABEL_STEP_MIN: f32 = 60.0;

/// The metric rule's subdivisions of a labelled step.
const METRIC_SUBDIVISIONS: f32 = 10.0;
/// Labelled inch steps in inches: 1, 2, 5, 10 times the base.
const INCH_STEPS: [f64; 9] = [1.0, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0, 200.0, 500.0];
/// The coarsest inch step, when even 500 inches are closer than the least label step.
const INCH_STEP_LAST: f64 = 1000.0;
/// Ticks beyond this many in one ruler are not produced: a custom table that asks for them has no useful picture to
/// give.
const MAX_TICKS: i64 = 20_000;

/// The step table of a ruler that counts a time unit (frames): the labelled step is the first of
/// `[1, 2, 5, 10, base/2, base, 2·base, 5·base, 10·base, 15·base, 30·base, 60·base, 120·base, 300·base, 600·base,
/// 1800·base, 3600·base]` units whose width is at least `label_gap` points, so labels never touch, and the ticks
/// subdivide it by the largest count of nine or fewer that keeps them `tick_gap` points apart, so every label sits on
/// a tick. `base` is the units in the timecode's second.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct StepLadder {
    /// The units in a timecode second (the frame rate, rounded).
    pub base: i64,
    /// The least distance between labelled ticks, in points.
    pub label_gap: f32,
    /// The least distance between ticks, in points.
    pub tick_gap: f32,
}

impl StepLadder {
    /// The ladder for `base` units to the second with the given gaps.
    pub const fn new(base: i64, label_gap: f32, tick_gap: f32) -> Self {
        StepLadder { base, label_gap, tick_gap }
    }

    /// The tick stride and the label stride in units, `(tick, label)`, when one unit is `points_per_unit` points
    /// wide. Strides past `base` saturate, since rates are file data.
    pub fn steps(&self, points_per_unit: f64) -> (i64, i64) {
        let base = self.base;
        let big = |k: i64| base.saturating_mul(k);
        let ladder =
            [1, 2, 5, 10, base / 2, base, big(2), big(5), big(10), big(15), big(30), big(60), big(120), big(300), big(600), big(1800), big(3600)];
        let (label_gap, tick_gap) = (f64::from(self.label_gap), f64::from(self.tick_gap));
        let label = ladder.iter().copied().find(|step| *step as f64 * points_per_unit >= label_gap).unwrap_or(big(3600)).max(1);
        let tick = (1..=9)
            .rev()
            .filter(|k| label.rem_euclid(*k) == 0)
            .map(|k| label / k)
            .find(|step| *step as f64 * points_per_unit >= tick_gap)
            .unwrap_or(label)
            .max(1);
        (tick, label)
    }
}

/// Which tick table [`ticks`] follows.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum TickRule<'a> {
    /// The units' own table ([`RulerUnits::step`]): decimal steps and ten subdivisions, or the host's custom table.
    Metric,
    /// Inches: a label every inch (every 2, 5, 10 and so on when smaller), subdivided by the first of these counts,
    /// finest first, that keeps the ticks a tenth of the least label step apart ([`INCH_SIXTEENTHS`],
    /// [`INCH_EIGHTHS`]).
    Inches(&'a [u32]),
    /// A time unit's own table: the units are frames (or ticks of the unit the ladder counts), and the labelled step
    /// and subdivisions come from the [`StepLadder`].
    Steps(StepLadder),
}

/// One tick of [`ticks`].
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct RulerTick {
    /// Ruler units from the zero point.
    pub value: f64,
    /// [`TickRank::Major`] for a labelled tick (a multiple of the labelled step), [`TickRank::Mid`] for the tick halfway
    /// between two labelled ticks in a subdivision into an even count, [`TickRank::Minor`] otherwise.
    pub rank: TickRank,
    /// The labelled step in ruler units, which the label of a major tick is written for.
    pub step: f64,
}

/// The labelled inch step and its subdivisions at `points_per_inch` screen points per inch.
fn inch_step(points_per_inch: f64, finest: &[u32]) -> (f64, u32) {
    let step_min = f64::from(LABEL_STEP_MIN);
    let step = INCH_STEPS.into_iter().find(|step| step * points_per_inch >= step_min).unwrap_or(INCH_STEP_LAST);
    if step > 1.0 {
        return (step, 2);
    }
    let gap = f64::from(LABEL_STEP_MIN / METRIC_SUBDIVISIONS);
    let subdivisions = finest.iter().copied().find(|n| points_per_inch / f64::from(*n) >= gap).unwrap_or(1);
    (step, subdivisions)
}

/// The ticks of one ruler over `values` (ruler units from the zero point): the one tick rule of every ruler, so a
/// text ruler or a page view's ruler shows the same ticks as the canvas's. One labelled step beyond each end is
/// included, since a label starts right of its tick. `zoom` is screen points per document unit along the axis, as
/// [`RulerUnits::step`] takes it. Empty when the units are unusable or the span would need more than 20,000 ticks.
pub fn ticks(units: &RulerUnits, rule: TickRule<'_>, zoom: f64, values: RangeInclusive<f64>) -> Vec<RulerTick> {
    let (step, subdivisions) = match rule {
        TickRule::Metric => units.step(zoom, LABEL_STEP_MIN),
        TickRule::Inches(finest) => inch_step(zoom / units.per_doc, finest),
        TickRule::Steps(ladder) => {
            let (tick, label) = ladder.steps(zoom / units.per_doc);
            (label as f64, u32::try_from(label / tick).unwrap_or(1))
        }
    };
    let minor = step / f64::from(subdivisions);
    if !(minor.is_finite() && minor > 0.0 && units.per_doc.is_finite() && units.per_doc > 0.0) {
        return Vec::new();
    }
    let (lo, hi) = (*values.start(), *values.end());
    let subdivisions = i64::from(subdivisions);
    let (first, last) = ((lo / minor).floor() as i64 - subdivisions, (hi / minor).ceil() as i64 + subdivisions);
    if last - first > MAX_TICKS {
        return Vec::new();
    }
    (first..=last)
        .map(|m| {
            let rem = m.rem_euclid(subdivisions);
            let rank = if rem == 0 {
                TickRank::Major
            } else if subdivisions % 2 == 0 && rem == subdivisions / 2 {
                TickRank::Mid
            } else {
                TickRank::Minor
            };
            RulerTick { value: m as f64 * minor, rank, step }
        })
        .collect()
}
