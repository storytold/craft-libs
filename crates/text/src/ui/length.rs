//! One owner for a length kept in points and shown in the document's unit: the unit type the text controls take,
//! and the number field over it.

use craft_behavior::Mixed;
use craft_ui::contract::Bind;
use craft_ui::runtime::env::{Label, LengthUnits, UnitKind, Units};
use craft_ui::text::NumberField;
use std::ops::RangeInclusive;

/// The unit a text control shows its lengths in. Lengths are stored in points.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct DocUnit {
    /// What the shown number counts: a length kind or `Pixels`.
    pub kind: UnitKind,
    /// The document's resolution, which sizes `Pixels`.
    pub pixels_per_inch: f64,
}

impl DocUnit {
    /// `kind` (a length kind or `Pixels`), with the document's resolution for pixels.
    pub const fn new(kind: UnitKind, pixels_per_inch: f64) -> Self {
        DocUnit { kind, pixels_per_inch }
    }

    /// Points at 72 pixels per inch: the default of every control.
    pub const fn points() -> Self {
        DocUnit { kind: UnitKind::Points, pixels_per_inch: UnitKind::POINTS_PER_INCH }
    }

    /// The lengths a field over points shows in this unit.
    pub(crate) const fn lengths(self) -> LengthUnits {
        LengthUnits::new(UnitKind::Points, self.kind, self.pixels_per_inch)
    }
}

impl Default for DocUnit {
    fn default() -> Self {
        DocUnit::points()
    }
}

/// A `NumberField` over a length kept in points: it shows `points` in `unit`, reads typed units ("2 cm", "12 pt",
/// expressions) in any length unit, and keeps `range` (points) as the shown range. Its events are in points.
pub(crate) fn length_field<'a>(unit: DocUnit, label: impl Into<Label>, points: Mixed<Option<f64>>, range: RangeInclusive<f64>) -> NumberField<'a> {
    NumberField::length(label, Bind::given(points), unit.lengths()).range(range)
}

/// `points` as the number `unit` shows for it, for the text around a field (a range message, a ruler's scale).
pub(crate) fn shown(units: &Units, unit: DocUnit, points: f64) -> f64 {
    points * units.length_scale(unit.lengths()).unwrap_or(1.0)
}
