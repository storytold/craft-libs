//! The W × H × Resolution cells both crop bars share, and the rules on [`CropSizeFields`].

use super::options::{CropLength, FIELD_MAX, LENGTH_MIN, ResolutionUnit, trim_number};
use super::{CropBarWords, CropDocument, CropSizeFields};
use craft_behavior::{IconId, NumberCodec};
use craft_ui::buttons::IconButton;
use craft_ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _, clicked};
use craft_ui::form::Field;
use craft_ui::pickers::Select;
use craft_ui::runtime::env::{LengthUnits, UnitKind, Units};
use craft_ui::text::{NumberField, RangeAlert};
use egui::Ui;
use std::ops::RangeInclusive;

/// Pixels per inch in one pixel per centimetre: 1 px/cm is 2.54 px/in.
const CM_PER_INCH: f64 = 2.54;

/// The ranges of the cells' fields.
///
/// A range is in the field's own unit (a W typed in inches is held to a range of inches, a resolution in px/cm to a
/// range of px/cm) unless `in_pixels` is set. Then `length` is in pixels and `resolution` in px/in, and the cells
/// convert both into the field's unit at the field's resolution; a resolution range converted to px/cm is rounded
/// inward to `resolution_decimals`, so a stored value never converts past the limit.
#[derive(Clone, Copy, Debug)]
pub(super) struct SizeLimits {
    /// The length range.
    pub length: (f64, f64),
    /// The resolution range.
    pub resolution: (f64, f64),
    /// Whether the ranges are in pixels and px/in rather than the field's own unit.
    pub in_pixels: bool,
    /// The decimals the resolution field keeps.
    pub resolution_decimals: usize,
}

/// What the cells reported this pass.
pub(super) struct SizeCells {
    /// A field changed.
    pub changed: bool,
    /// A typed value was out of range.
    pub out_of_range: Option<RangeAlert>,
}

impl SizeLimits {
    /// The crop bar's ranges, in the field's own unit; its fields never alert.
    pub(super) const CROP: Self = Self { length: (LENGTH_MIN, FIELD_MAX), resolution: (0.0, FIELD_MAX), in_pixels: false, resolution_decimals: 2 };
    /// The perspective bar's ranges: 1 to 300 000 px and 1 to 29 999.999 px/in.
    pub(super) const PERSPECTIVE: Self = Self { length: (1.0, 300_000.0), resolution: (1.0, 29_999.999), in_pixels: true, resolution_decimals: 3 };

    /// The length range in `unit`, at `pixels_per_inch` and `points_per_inch`.
    fn length_range(&self, unit: UnitKind, pixels_per_inch: f64, points_per_inch: f64) -> RangeInclusive<f64> {
        if !self.in_pixels {
            return self.length.0..=self.length.1;
        }
        let at = |pixels: f64| unit.from_pixels(pixels, 1.0, pixels_per_inch, points_per_inch);
        match (at(self.length.0), at(self.length.1)) {
            (Some(low), Some(high)) if low <= high => low..=high,
            _ => LENGTH_MIN..=FIELD_MAX,
        }
    }

    /// The resolution range in `unit`, rounded inward to the decimals the field keeps.
    pub(super) fn resolution_bounds(&self, unit: ResolutionUnit) -> (f64, f64) {
        let per_unit = match unit {
            ResolutionUnit::PixelsPerCentimetre if self.in_pixels => CM_PER_INCH,
            _ => 1.0,
        };
        let scale = 10f64.powi(self.resolution_decimals as i32);
        ((self.resolution.0 / per_unit * scale).ceil() / scale, (self.resolution.1 / per_unit * scale).floor() / scale)
    }

    /// The resolution range Front Image rounds into: given only for ranges in pixels.
    pub(super) fn front_limits(&self, unit: ResolutionUnit) -> Option<(f64, f64)> {
        self.in_pixels.then(|| self.resolution_bounds(unit))
    }
}

/// W, swap, H, Resolution and its unit.
pub(super) fn show(
    ui: &mut Ui,
    size: &mut CropSizeFields,
    limits: SizeLimits,
    document: &CropDocument,
    units: &Units,
    words: &CropBarWords,
) -> SizeCells {
    let mut changed = false;
    let mut out_of_range = None;
    let points_per_inch = units.points_per_inch();
    let pixels_per_inch = size.resolution_ppi().unwrap_or(document.resolution);
    changed |= length_field(ui, "size-width", &words.width, &mut size.width, pixels_per_inch, points_per_inch, &limits, &mut out_of_range);
    if clicked(&IconButton::new(IconId::new("swap_horizontal")).label(words.swap.clone()).show(ui)) {
        size.swap();
        changed = true;
    }
    changed |= length_field(ui, "size-height", &words.height, &mut size.height, pixels_per_inch, points_per_inch, &limits, &mut out_of_range);

    let unit = size.resolution_unit;
    let (low, high) = limits.resolution_bounds(unit);
    let resolution = size.resolution.trim().replace(',', ".").parse::<f64>().ok().filter(|v| v.is_finite() && *v > 0.0);
    let mut typed = None;
    let mut alert = None;
    Field::new(words.resolution.clone()).id_salt("size-resolution").show(ui, |ui, _| {
        let codec = NumberCodec::new(units.reader(None), limits.resolution_decimals);
        let field = NumberField::new(words.resolution.clone(), Bind::given(resolution)).codec(codec).range(low..=high).automatic(true).show(ui);
        typed = field.value().copied();
        alert = field.out_of_range;
    });
    if alert.is_some() {
        out_of_range = alert;
    }
    if let Some(value) = typed {
        let text = value.filter(|v| *v > 0.0).map_or_else(String::new, |v| trim_number(v.clamp(low, high), limits.resolution_decimals));
        if text != size.resolution {
            size.resolution = text;
            changed = true;
        }
    }

    let unit_entries = [
        Entry::new(ResolutionUnit::PixelsPerInch, words.pixels_per_inch.clone()),
        Entry::new(ResolutionUnit::PixelsPerCentimetre, words.pixels_per_centimetre.clone()),
    ];
    let pick = Select::new(words.resolution_unit.clone(), Bind::given(size.resolution_unit), &unit_entries).show(ui);
    if let Some(next) = pick.value().copied()
        && next != size.resolution_unit
    {
        // The same resolution in the other unit, as in Photoshop.
        if let Some(v) = resolution {
            let converted = if next == ResolutionUnit::PixelsPerCentimetre { v / CM_PER_INCH } else { v * CM_PER_INCH };
            let (low, high) = limits.resolution_bounds(next);
            size.resolution = trim_number(converted.clamp(low, high), limits.resolution_decimals);
        }
        size.resolution_unit = next;
        changed = true;
    }
    SizeCells { changed, out_of_range }
}

/// A length field over a length kept in its own unit: a typed number is in that unit (pixels while unset), a typed
/// unit is converted into it, and a cleared field is unset. Returns whether `value` changed; a typed value outside
/// the range of `limits` is clamped and reported in `out_of_range`.
#[expect(clippy::too_many_arguments, reason = "the cells' inputs")]
fn length_field(
    ui: &mut Ui,
    salt: &'static str,
    caption: &craft_ui::runtime::env::Label,
    value: &mut Option<CropLength>,
    pixels_per_inch: f64,
    points_per_inch: f64,
    limits: &SizeLimits,
    out_of_range: &mut Option<RangeAlert>,
) -> bool {
    let unit = value.map_or(UnitKind::Pixels, |length| length.unit);
    let mut typed = None;
    let mut typed_unit = None;
    let mut alert = None;
    Field::new(caption.clone()).id_salt(salt).show(ui, |ui, _| {
        let lengths = LengthUnits::new(unit, unit, pixels_per_inch);
        let range = limits.length_range(unit, pixels_per_inch, points_per_inch);
        let field = NumberField::length(caption.clone(), Bind::given(value.map(|length| length.value)), lengths).range(range).show(ui);
        typed = field.value().copied();
        typed_unit = field.typed_unit;
        alert = field.out_of_range;
    });
    if alert.is_some() {
        *out_of_range = alert;
    }
    let Some(next) = typed else { return false };
    let next = next.map(|length| match typed_unit {
        // The value is in the stored unit; a typed unit becomes the stored one, as in Photoshop.
        Some(to) if to != unit => {
            let pixels = unit.to_pixels(length, 1.0, pixels_per_inch, points_per_inch);
            let converted = pixels.and_then(|pixels| to.from_pixels(pixels, 1.0, pixels_per_inch, points_per_inch));
            converted.map_or(CropLength::new(length, unit), |value| CropLength::new(value, to))
        }
        _ => CropLength::new(length, unit),
    });
    let changed = next != *value;
    *value = next;
    changed
}

/// The size a crop commit resamples to.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct CropSizeTarget {
    /// The width in pixels.
    pub width: Option<f64>,
    /// The height in pixels.
    pub height: Option<f64>,
    /// The resolution in px/in.
    pub resolution: Option<f64>,
}

impl CropSizeFields {
    /// Swaps W and H.
    pub fn swap(&mut self) {
        std::mem::swap(&mut self.width, &mut self.height);
    }

    /// Fills W and H with the document's size in pixels and the resolution with the document's, in the field's unit.
    /// With `limits` (the resolution range in the field's unit) the resolution keeps 3 decimals and is clamped into
    /// them, so a stored value never converts past a limit; without, it keeps 2.
    pub fn front_image(&mut self, document: &CropDocument, limits: Option<(f64, f64)>) {
        self.width = Some(CropLength::new(document.size.width, UnitKind::Pixels));
        self.height = Some(CropLength::new(document.size.height, UnitKind::Pixels));
        let resolution = match self.resolution_unit {
            ResolutionUnit::PixelsPerInch => document.resolution,
            ResolutionUnit::PixelsPerCentimetre => document.resolution / CM_PER_INCH,
        };
        self.resolution = match limits {
            Some((low, high)) => trim_number(((resolution * 1000.0).round() / 1000.0).clamp(low, high), 3),
            None => trim_number(resolution, 2),
        };
    }

    /// Empties W, H and the resolution and resets the resolution unit to pixels per inch.
    pub fn clear_size(&mut self) {
        self.width = None;
        self.height = None;
        self.resolution.clear();
        self.resolution_unit = ResolutionUnit::PixelsPerInch;
    }

    /// The size the fields name: each length in pixels (physical lengths at the typed resolution, else the
    /// document's; points and picas at `units`' points per inch; a bare number in pixels), each side `None` while it
    /// is empty or not a positive length of at most [`FIELD_MAX`].
    pub fn target(&self, document: &CropDocument, units: &Units) -> CropSizeTarget {
        let resolution = self.resolution_ppi();
        let dpi = resolution.unwrap_or(document.resolution);
        let pixels = |length: Option<CropLength>| {
            let length = CropLength::usable(length)?;
            let px = length.unit.to_pixels(length.value, 0.0, dpi, units.points_per_inch())?;
            (px.is_finite() && px > 0.0).then_some(px)
        };
        CropSizeTarget { width: pixels(self.width), height: pixels(self.height), resolution }
    }
}
