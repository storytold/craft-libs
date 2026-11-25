//! [`RulerUnits`]: how rulers, guide readouts and guide values measure the document along one axis. The tick maths
//! lives here; the text of a distance or a tick is written by craft-ui over `Env::units`, so no second number
//! formatter exists.

use std::ops::RangeInclusive;

/// How a ruler divides its unit at a zoom.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub enum Ticks {
    /// PhotoCraft's decimal steps: the labelled step is the smallest of 1, 2, 2.5 and 5 × 10ⁿ units that leaves the
    /// ruler's minimum label step points between labels, in 10 subdivisions. `whole` keeps the step a whole number
    /// of units without 2.5 (pixels).
    #[non_exhaustive]
    Decimal {
        /// Keep the step a whole number of units, without 2.5.
        whole: bool,
    },
    /// The host's table: screen points per unit to (labelled step in units, subdivisions), for DesignCraft's
    /// eighths of an inch and picas in sixths.
    Custom(fn(f64) -> (f64, u32)),
}

/// How a ruler writes the number at a labelled tick.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum TickLabels {
    /// PhotoCraft's: the distance from the zero point with no minus sign, a step under 1 with at least two decimals
    /// (`2.50`, `0.30`, `1.0000` on a sixteenth-inch ruler).
    #[default]
    Distance,
    /// VectorCraft's: the signed position, with only the decimals the tick needs (`-2.5`, `0.3`, `1`).
    Signed,
}

/// The ruler unit of one document axis: its size, its ticks and how it labels them. Rulers label their ticks with it;
/// guides use it for the readout of a drag, their AccessKit values and Shift's snap to the ticks.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct RulerUnits {
    /// Ruler units per document unit along this axis (percent is per axis).
    pub per_doc: f64,
    /// How the ticks are spaced.
    pub ticks: Ticks,
    /// How the labelled ticks are written.
    pub labels: TickLabels,
}

impl Ticks {
    /// PhotoCraft's decimal steps; `whole` keeps the step a whole number of units without 2.5.
    pub const fn decimal(whole: bool) -> Self {
        Ticks::Decimal { whole }
    }
}

impl RulerUnits {
    /// Ruler units of `per_doc` per document unit, spaced by `ticks`, labelled as [`TickLabels::Distance`].
    pub const fn new(per_doc: f64, ticks: Ticks) -> Self {
        Self { per_doc, ticks, labels: TickLabels::Distance }
    }

    /// The units with their ticks written as `labels`.
    #[must_use]
    pub const fn with_labels(mut self, labels: TickLabels) -> Self {
        self.labels = labels;
        self
    }

    /// Whole document pixels.
    pub const fn pixels() -> Self {
        Self::new(1.0, Ticks::Decimal { whole: true })
    }

    /// The labelled step in ruler units and its subdivisions when one document unit is `zoom` screen points along
    /// this axis, with labels at least `step_min` points apart.
    pub fn step(&self, zoom: f64, step_min: f32) -> (f64, u32) {
        let points_per_unit = zoom / self.per_doc;
        match self.ticks {
            Ticks::Decimal { whole } => (decimal_step(f64::from(step_min) / points_per_unit, whole), 10),
            Ticks::Custom(table) => {
                let (step, subdivisions) = table(points_per_unit);
                (step, subdivisions.max(1))
            }
        }
    }

    /// `position` (document units) moved to the nearest minor tick of the ruler counting from `zero`, at `zoom`
    /// screen points per document unit (PhotoCraft's Shift snap, #3770).
    pub fn snap_to_tick(&self, position: f64, zero: f64, zoom: f64, step_min: f32) -> f64 {
        let (step, subdivisions) = self.step(zoom, step_min);
        let minor = step / f64::from(subdivisions);
        if !(minor.is_finite() && minor > 0.0 && self.per_doc > 0.0) {
            return position;
        }
        let units = (position - zero) * self.per_doc;
        zero + (units / minor).round() * minor / self.per_doc
    }

    /// The distance of `position` (document units) from `zero`, in ruler units.
    pub fn distance(&self, position: f64, zero: f64) -> f64 {
        (position - zero) * self.per_doc
    }

    /// The least and most decimals a ruler writes a tick of labelled step `step` with. Whole ticks are whole numbers;
    /// a step of at least 1 takes the decimals it has (none for 5, one for 2.5). Under [`TickLabels::Distance`] a
    /// smaller step takes at least two (PhotoCraft's `0.50`), more when it has them (0.025), and every tick takes them
    /// all; under [`TickLabels::Signed`] a tick drops the trailing zeros (`1` on a sixteenth-inch ruler).
    pub fn label_decimals(&self, step: f64) -> RangeInclusive<usize> {
        let most = match self.ticks {
            Ticks::Decimal { whole: true } => 0,
            _ if step.abs() < 1.0 && self.labels == TickLabels::Distance => step_decimals(step).max(2),
            _ => step_decimals(step),
        };
        match self.labels {
            TickLabels::Distance => most..=most,
            TickLabels::Signed => 0..=most,
        }
    }

    /// The number a ruler prints at the tick `value` (ruler units from the zero point) for a labelled step `step`:
    /// under [`TickLabels::Distance`] the distance without a minus sign left of or above the zero point (PhotoCraft's
    /// `ruler_label`), under [`TickLabels::Signed`] the value itself; rounded when the ticks are whole.
    pub fn label_value(&self, value: f64, step: f64) -> f64 {
        let value = match self.labels {
            TickLabels::Distance => value.abs(),
            TickLabels::Signed => value,
        };
        if *self.label_decimals(step).end() == 0 { value.round() } else { value }
    }
}

/// The decimals that write `step` exactly, up to 6.
fn step_decimals(step: f64) -> usize {
    let step = step.abs();
    (0..6)
        .find(|&d| {
            let scaled = step * 10f64.powi(d as i32);
            (scaled - scaled.round()).abs() <= 1e-6 * scaled.max(1.0)
        })
        .unwrap_or(6)
}

/// The smallest of 1, 2, 2.5 and 5 × 10ⁿ at least `min`; whole steps (no 2.5, at least 1) with `whole`
/// (PhotoCraft's `nice_step`).
fn decimal_step(min: f64, whole: bool) -> f64 {
    if !min.is_finite() || min <= 0.0 {
        return 1.0;
    }
    let mut step = if whole { 1.0 } else { 10f64.powi(min.log10().floor() as i32 - 1) };
    loop {
        for m in [1.0, 2.0, 2.5, 5.0] {
            if (m != 2.5 || !whole) && step * m >= min {
                return step * m;
            }
        }
        step *= 10.0;
    }
}
