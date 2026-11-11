//! The W and H fields' units: [`SizeUnit`], the [`Conversion`] between box and field values, and the field limits.

use super::{TransformField, TransformRow};
use craft_behavior::geometry::quad::{BoxState, SKEW_LIMIT};
use craft_behavior::kurbo::{Rect, Vec2};
use craft_behavior::{NumberCodec, RulerUnits};
use craft_ui::canvas::RulerText;
use craft_ui::runtime::env::{UnitKind, Units};

/// The largest position of the box's reference point, in document units.
const POSITION_LIMIT: f64 = 300_000.0;
/// The largest scale a W or H field takes, as a factor (1 000 000 %).
const SCALE_LIMIT: f64 = 10_000.0;
/// The largest angle.
const ANGLE_LIMIT: f64 = 180.0;
/// The decimals the angle and the skews show and commit.
const ANGLE_DECIMALS: usize = 2;
/// The decimals W and H show in percent.
const PERCENT_DECIMALS: usize = 2;
/// The side of the box, in document units, a typed percentage reads against when the box has none.
const NO_SIDE: f64 = 1.0;

/// The unit of the W and H fields.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum SizeUnit {
    /// The scale in percent of the box's size before the transform, shown and committed with up to `decimals`
    /// decimals.
    #[non_exhaustive]
    Percent {
        /// Most decimals shown and committed.
        decimals: usize,
    },
    /// The box's length in the document's units along each axis: the unit the W and H menu picked, which PhotoCraft
    /// keeps apart from the rulers'.
    #[non_exhaustive]
    Length {
        /// The unit along x.
        x: RulerText,
        /// The unit along y.
        y: RulerText,
    },
}

impl SizeUnit {
    /// Percent of the box before the transform, with up to `decimals` decimals.
    pub fn percent(decimals: usize) -> Self {
        Self::Percent { decimals }
    }

    /// A length in `x` and `y`.
    pub fn length(x: RulerText, y: RulerText) -> Self {
        Self::Length { x, y }
    }
}

impl Default for SizeUnit {
    fn default() -> Self {
        Self::Percent { decimals: PERCENT_DECIMALS }
    }
}

/// How the fields read and write a box: the box before the transform (`rect`), the units, and whether W and H are
/// linked.
#[derive(Clone, Debug)]
pub(super) struct Conversion {
    pub(super) rect: Rect,
    pub(super) x: RulerText,
    pub(super) y: RulerText,
    pub(super) size: SizeUnit,
    pub(super) linked: bool,
}

impl Conversion {
    /// The length of the box's side before the transform along `axis` (0: W, 1: H).
    fn side(&self, axis: usize) -> f64 {
        (if axis == 0 { self.rect.width() } else { self.rect.height() }).abs()
    }

    /// Field units of the size fields per unit of scale factor.
    fn per_factor(&self, axis: usize) -> f64 {
        match &self.size {
            SizeUnit::Percent { .. } => 100.0,
            SizeUnit::Length { x, y } => self.side(axis).max(1e-9) * if axis == 0 { x.units.per_doc } else { y.units.per_doc },
        }
    }

    /// The largest scale factor along `axis`: as far as the box stays within the position limit.
    fn limit(&self, axis: usize) -> f64 {
        let side = self.side(axis);
        if side.is_finite() && side >= 1e-6 { (POSITION_LIMIT / side).clamp(1.0, SCALE_LIMIT) } else { SCALE_LIMIT }
    }

    /// The largest value a field shows or takes.
    pub(super) fn bound(&self, field: TransformField) -> f64 {
        match field {
            TransformField::X => POSITION_LIMIT * self.x.units.per_doc,
            TransformField::Y => POSITION_LIMIT * self.y.units.per_doc,
            TransformField::W => self.limit(0) * self.per_factor(0),
            TransformField::H => self.limit(1) * self.per_factor(1),
            TransformField::Angle => ANGLE_LIMIT,
            TransformField::SkewH | TransformField::SkewV => SKEW_LIMIT,
        }
    }

    /// The unit a field counts in and the most decimals it shows.
    pub(super) fn format(&self, field: TransformField) -> (UnitKind, usize) {
        let end = |kind: UnitKind| *kind.decimals().end();
        match (field, &self.size) {
            (TransformField::X, _) => (self.x.kind, end(self.x.kind)),
            (TransformField::Y, _) => (self.y.kind, end(self.y.kind)),
            (TransformField::W, SizeUnit::Length { x, .. }) => (x.kind, end(x.kind)),
            (TransformField::H, SizeUnit::Length { y, .. }) => (y.kind, end(y.kind)),
            (TransformField::W | TransformField::H, SizeUnit::Percent { decimals }) => (UnitKind::Percent, *decimals),
            _ => (UnitKind::Degrees, ANGLE_DECIMALS),
        }
    }

    /// What `field` shows for `state`, whose V skew is `skew_v`.
    pub(super) fn shown(&self, state: &BoxState, skew_v: f64, field: TransformField) -> f64 {
        let parts = state.parts(self.rect, skew_v);
        match field {
            TransformField::X => state.pivot.x * self.x.units.per_doc,
            TransformField::Y => state.pivot.y * self.y.units.per_doc,
            TransformField::W => parts.w * self.per_factor(0),
            TransformField::H => parts.h * self.per_factor(1),
            TransformField::Angle => parts.angle,
            TransformField::SkewH => parts.skew_h,
            TransformField::SkewV => skew_v,
        }
    }

    /// The box and V skew after `field` is set to `value`: X and Y move the box with its reference point; the others
    /// scale, turn and skew it about the reference point. Linked, the other side of W or H follows by the same
    /// factor, each kept within its limit.
    fn edit(&self, state: &BoxState, skew_v: f64, field: TransformField, value: f64) -> (BoxState, f64) {
        let now = state.parts(self.rect, skew_v);
        let mut to = now;
        match field {
            TransformField::X => return (state.moved(Vec2::new(value / self.x.units.per_doc - state.pivot.x, 0.0)), skew_v),
            TransformField::Y => return (state.moved(Vec2::new(0.0, value / self.y.units.per_doc - state.pivot.y)), skew_v),
            TransformField::W | TransformField::H => {
                let axis = usize::from(field == TransformField::H);
                let factor = value / self.per_factor(axis);
                if self.linked {
                    let (side, other) = if axis == 0 { (now.w, now.h) } else { (now.h, now.w) };
                    let k = factor / nonzero(side);
                    let cap = (self.limit(axis) / nonzero(side.abs())).min(self.limit(1 - axis) / nonzero(other.abs()));
                    let capped = side * k.signum() * k.abs().min(cap);
                    (to.w, to.h) = BoxState::linked_scale((now.w, now.h), axis, capped);
                } else if axis == 0 {
                    to.w = factor;
                } else {
                    to.h = factor;
                }
            }
            TransformField::Angle => to.angle = value,
            TransformField::SkewH => to.skew_h = value,
            TransformField::SkewV => to.skew_v = value,
        }
        (state.with_parts(self.rect, skew_v, to), to.skew_v)
    }

    /// The row after `field` is set to `value` in `row`.
    pub(super) fn row(&self, row: &TransformRow, field: TransformField, value: f64) -> TransformRow {
        let (state, skew_v) = self.edit(&row.state, row.skew_v, field, value);
        TransformRow { state, skew_v }
    }

    /// The reader of typed text for `field`: the field's own unit spellings, and for lengths and percentages every
    /// other pixel and length unit, converted through the document's resolution. A percentage is read against the
    /// box's side before the transform.
    pub(super) fn codec(&self, units: &Units, field: TransformField) -> NumberCodec {
        let (kind, decimals) = self.format(field);
        let reader = match field {
            TransformField::Angle | TransformField::SkewH | TransformField::SkewV => units.reader(Some(kind)),
            _ => {
                let points = units.points_per_inch();
                let side = self.side(usize::from(matches!(field, TransformField::H | TransformField::Y))).max(NO_SIDE);
                units.document_reader(kind, side, pixels_per_inch(self.x, points), points)
            }
        };
        NumberCodec::new(reader, decimals)
    }
}

/// The pixels in an inch of a document measured in `ruler`; points to the inch in `points` when the ruler counts no
/// length.
pub(crate) fn pixels_per_inch(ruler: RulerText, points: f64) -> f64 {
    match ruler.kind.per_inch(points) {
        Some(per_inch) if ruler.units.per_doc > 0.0 => per_inch / ruler.units.per_doc,
        _ => UnitKind::POINTS_PER_INCH,
    }
}

/// How W and H show in `unit`: percent of the original side, or a length of the document's pixels at the resolution
/// of `ruler`.
pub(crate) fn size_unit_of(unit: UnitKind, ruler: RulerText, points: f64) -> SizeUnit {
    if unit == UnitKind::Percent {
        return SizeUnit::default();
    }
    let per_doc = unit.from_pixels(1.0, NO_SIDE, pixels_per_inch(ruler, points), points).unwrap_or(1.0);
    let axis = RulerText::new(RulerUnits::new(per_doc, craft_behavior::Ticks::decimal(unit == UnitKind::Pixels)), unit);
    SizeUnit::length(axis, axis)
}

fn nonzero(v: f64) -> f64 {
    if v.abs() < 1e-9 { 1e-9_f64.copysign(if v == 0.0 { 1.0 } else { v }) } else { v }
}
