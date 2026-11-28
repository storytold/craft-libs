//! The W × H × Resolution cells both crop bars share, and the rules on [`CropSizeFields`].

use super::options::{CropLength, FIELD_MAX, LENGTH_MIN, ResolutionUnit, trim_number};
use super::{CropBarWords, CropDocument, CropSizeFields};
use craft_behavior::{IconId, NumberCodec};
use craft_ui::buttons::IconButton;
use craft_ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _, clicked};
use craft_ui::form::Field;
use craft_ui::menu::Menu;
use craft_ui::pickers::Select;
use craft_ui::runtime::env::{ColumnSize, Label, LengthUnits, UnitKind, Units};
use craft_ui::text::{NumberField, RangeAlert};
use egui::Ui;
use std::ops::RangeInclusive;

/// Pixels per inch in one pixel per centimetre: 1 px/cm is 2.54 px/in.
const CM_PER_INCH: f64 = 2.54;

/// The units of the W and H menus, in the menu's order; Columns follows when the document has a column size.
const MENU_UNITS: [UnitKind; 6] =
    [UnitKind::Pixels, UnitKind::Inches, UnitKind::Centimetres, UnitKind::Millimetres, UnitKind::Points, UnitKind::Picas];

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
    /// Whether a right click on W or H opens its units menu (and the fields take Columns when the document has a
    /// column size).
    pub unit_menu: bool,
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
    pub(super) const CROP: Self =
        Self { length: (LENGTH_MIN, FIELD_MAX), resolution: (0.0, FIELD_MAX), in_pixels: false, resolution_decimals: 2, unit_menu: true };
    /// The perspective bar's ranges: 1 to 300 000 px and 1 to 29 999.999 px/in.
    pub(super) const PERSPECTIVE: Self =
        Self { length: (1.0, 300_000.0), resolution: (1.0, 29_999.999), in_pixels: true, resolution_decimals: 3, unit_menu: false };

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

/// The numbers a length converts through: the resolution, the point size and the column size, when there is one.
#[derive(Clone, Copy, Debug)]
struct Lengths {
    pixels_per_inch: f64,
    points_per_inch: f64,
    columns: Option<ColumnSize>,
}

impl Lengths {
    /// `value` in `unit` as pixels; Columns through the column size, `None` without one.
    fn to_pixels(self, value: f64, unit: UnitKind) -> Option<f64> {
        match unit {
            UnitKind::Columns => Some(self.columns?.points(value, false) / self.points_per_inch * self.pixels_per_inch.max(1.0)),
            _ => unit.to_pixels(value, 1.0, self.pixels_per_inch, self.points_per_inch),
        }
    }

    /// `pixels` in `unit`; the inverse of [`to_pixels`](Self::to_pixels).
    fn in_unit(self, pixels: f64, unit: UnitKind) -> Option<f64> {
        match unit {
            UnitKind::Columns => Some(self.columns?.columns(pixels / self.pixels_per_inch.max(1.0) * self.points_per_inch, false)),
            _ => unit.from_pixels(pixels, 1.0, self.pixels_per_inch, self.points_per_inch),
        }
    }

    /// `value` in `from` as the same length in `to`.
    fn convert(self, value: f64, from: UnitKind, to: UnitKind) -> Option<f64> {
        self.in_unit(self.to_pixels(value, from)?, to).filter(|v| v.is_finite())
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
    let columns = document.columns.filter(|_| limits.unit_menu);
    let lengths = Lengths { pixels_per_inch, points_per_inch, columns };
    let menu: Option<Vec<Entry<UnitKind>>> = limits.unit_menu.then(|| {
        let kinds = MENU_UNITS.into_iter().chain(columns.map(|_| UnitKind::Columns));
        kinds.map(|kind| Entry::new(kind, units.name(kind))).collect()
    });
    let menu = menu.as_deref();
    let width = LengthCell { salt: "size-width", caption: &words.width, menu };
    changed |= width.show(ui, &mut size.width, &mut size.width_unit, lengths, &limits, units, &mut out_of_range);
    if clicked(&IconButton::new(IconId::new("swap_horizontal")).label(words.swap.clone()).show(ui)) {
        size.swap();
        changed = true;
    }
    let height = LengthCell { salt: "size-height", caption: &words.height, menu };
    changed |= height.show(ui, &mut size.height, &mut size.height_unit, lengths, &limits, units, &mut out_of_range);

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

/// One of W and H: a length field over a length kept in its own unit.
struct LengthCell<'a> {
    salt: &'static str,
    caption: &'a Label,
    /// The units menu a right click opens, when the bar has one.
    menu: Option<&'a [Entry<UnitKind>]>,
}

impl LengthCell<'_> {
    /// A typed number is in the field's unit (`empty_unit` while the field is empty, pixels while that is unset), a
    /// typed unit is converted into it and becomes the field's unit, and a cleared field is unset. A typed count of
    /// columns (`2 col`) is kept in Columns. A unit picked from the menu converts the length into that unit, or becomes
    /// `empty_unit` of an empty field. Returns whether `value` or `empty_unit` changed; a typed value outside the
    /// range of `limits` is clamped and reported in `out_of_range`.
    #[expect(clippy::too_many_arguments, reason = "the cell's inputs")]
    fn show(
        &self,
        ui: &mut Ui,
        value: &mut Option<CropLength>,
        empty_unit: &mut Option<UnitKind>,
        lengths: Lengths,
        limits: &SizeLimits,
        units: &Units,
        out_of_range: &mut Option<RangeAlert>,
    ) -> bool {
        let unit = value.map_or(empty_unit.unwrap_or(UnitKind::Pixels), |length| length.unit);
        // The unit a typed length is read in: the field's, or inches for a Columns field, whose bare numbers count
        // columns.
        let base = if unit == UnitKind::Columns { UnitKind::Inches } else { unit };
        let range = limits.length_range(unit, lengths.pixels_per_inch, lengths.points_per_inch);
        let mut typed = None;
        let mut typed_unit = None;
        let mut alert = None;
        let mut picked = None;
        Field::new(self.caption.clone()).id_salt(self.salt).show(ui, |ui, _| {
            let shown = LengthUnits::new(unit, unit, lengths.pixels_per_inch);
            let mut field = NumberField::length(self.caption.clone(), Bind::given(value.map(|length| length.value)), shown).range(range.clone());
            if self.menu.is_some() {
                // Columns are no factor of a length: a typed `2 col` is read as a count, beside the lengths in `base`.
                // The right click is the units menu, so the field opens no edit menu of its own.
                let reader = units.length_reader(LengthUnits::new(base, base, lengths.pixels_per_inch));
                let reader = if unit == UnitKind::Columns || lengths.columns.is_some() { units.with_columns(reader) } else { reader };
                field = field.codec(NumberCodec::new(reader, *unit.decimals().end())).own_context_menu();
            }
            let field = field.show(ui);
            typed = field.value().copied();
            typed_unit = field.typed_unit;
            alert = field.out_of_range;
            if let Some(entries) = self.menu {
                let entries: Vec<Entry<UnitKind>> = entries.iter().map(|entry| entry.clone().checked(entry.value() == Some(&unit))).collect();
                picked = Menu::context(self.caption.clone(), &field.edit.response).values().pick(ui, &entries).picked;
            }
        });
        if alert.is_some() {
            *out_of_range = alert;
        }
        let mut changed = false;
        if let Some(to) = picked.filter(|to| *to != unit) {
            match *value {
                // The same length in the picked unit, unrounded and unclamped, so the target size stays; a length the
                // picked unit cannot hold (past `FIELD_MAX`) keeps its unit.
                Some(length) => {
                    if let Some(converted) = lengths.convert(length.value, unit, to).filter(|v| v.is_finite() && *v > 0.0 && *v <= FIELD_MAX) {
                        *value = Some(CropLength::new(converted, to));
                        changed = true;
                    }
                }
                None => {
                    *empty_unit = Some(to);
                    changed = true;
                }
            }
        }
        let Some(next) = typed else { return changed };
        let next = next.map(|length| match typed_unit {
            // A count of columns as typed.
            Some(UnitKind::Columns) => CropLength::new(length, UnitKind::Columns),
            // The value is in the field's unit; a typed unit becomes the field's.
            Some(to) if to != unit => lengths.convert(length, base, to).map_or(CropLength::new(length, unit), |value| CropLength::new(value, to)),
            _ => CropLength::new(length, unit),
        });
        changed |= next != *value;
        *value = next;
        changed
    }
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
        std::mem::swap(&mut self.width_unit, &mut self.height_unit);
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

    /// Empties W, H and the resolution and resets the units of empty W and H to pixels and the resolution unit to
    /// pixels per inch.
    pub fn clear_size(&mut self) {
        self.width = None;
        self.height = None;
        self.width_unit = None;
        self.height_unit = None;
        self.resolution.clear();
        self.resolution_unit = ResolutionUnit::PixelsPerInch;
    }

    /// The size the fields name: each length in pixels (physical lengths at the typed resolution, else the
    /// document's; points and picas at `units`' points per inch; columns through the document's column size; a bare
    /// number in pixels), each side `None` while it is empty or not a positive length of at most [`FIELD_MAX`].
    pub fn target(&self, document: &CropDocument, units: &Units) -> CropSizeTarget {
        let resolution = self.resolution_ppi();
        let pixels_per_inch = resolution.unwrap_or(document.resolution);
        let lengths = Lengths { pixels_per_inch, points_per_inch: units.points_per_inch(), columns: document.columns };
        let pixels = |length: Option<CropLength>| {
            let length = CropLength::usable(length)?;
            let px = lengths.to_pixels(length.value, length.unit)?;
            (px.is_finite() && px > 0.0).then_some(px)
        };
        CropSizeTarget { width: pixels(self.width), height: pixels(self.height), resolution }
    }
}
