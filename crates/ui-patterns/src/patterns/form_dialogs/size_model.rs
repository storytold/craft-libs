//! The pure model of the size dialogs: a pixel size, the linked width, height and resolution fields of Image Size and
//! Canvas Size, and the two size rules of the export dialogs. It reads no egui and draws nothing; the dialogs show
//! `LinkedSize` through `UnitSelect` rows, one per field.

use crate::runtime::env::UnitKind;
use craft_behavior::PixelSize;
use std::ops::RangeInclusive;

/// The engine's greatest document side, in pixels.
pub const MAX_DOCUMENT_SIDE: f64 = 300_000.0;
/// The least and greatest resolution, in pixels per inch.
const PPI_RANGE: RangeInclusive<f64> = 1.0..=30_000.0;
/// The resolution of a document whose dialog names none, in pixels per inch.
const DEFAULT_RESOLUTION: f64 = 72.0;
/// The greatest scale of an export, in percent.
const MAX_SCALE: f64 = 1000.0;
/// The least scale of an export, in percent.
const MIN_SCALE: f64 = 1.0;
const PERCENT: f64 = 100.0;

/// The pixels of `size` along `field`.
fn pixels_along(size: PixelSize, field: Side) -> f64 {
    f64::from(match field {
        Side::Width => size.width,
        Side::Height => size.height,
    })
}

/// Why a size was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SizeError {
    /// The scale is not within 1 to 1000 percent.
    Scale,
    /// A side is not within 1 to 300,000 pixels, or is not a number.
    Pixels,
}

/// The output size at `scale` percent (1 to 1000) of a `source` size: each side scaled and rounded, at least 1 pixel
/// (a source side under 1 counts as 1). A scale outside the range is [`SizeError::Scale`]; a side over 300,000 pixels
/// is [`SizeError::Pixels`].
pub fn resize_from_scale(source: PixelSize, scale: f64) -> Result<PixelSize, SizeError> {
    if !scale.is_finite() || !(MIN_SCALE..=MAX_SCALE).contains(&scale) {
        return Err(SizeError::Scale);
    }
    let side = |source: u32| (f64::from(source.max(1)) * scale / PERCENT).round().max(1.0);
    let (width, height) = (side(source.width), side(source.height));
    if width > MAX_DOCUMENT_SIDE || height > MAX_DOCUMENT_SIDE {
        return Err(SizeError::Pixels);
    }
    Ok(PixelSize::new(width as u32, height as u32))
}

/// The output size after the user typed `width` (`edited_width`) or `height`: the typed side is rounded and must be 1
/// to 300,000 pixels. With `lock` the other side follows the source's aspect (at least 1 pixel, same range);
/// otherwise it keeps its value (same range). Returns the size and the scale it amounts to, in percent of the edited
/// side.
pub fn resize_from_side(source: PixelSize, width: f64, height: f64, lock: bool, edited_width: bool) -> Result<(PixelSize, f64), SizeError> {
    let (source_width, source_height) = (f64::from(source.width.max(1)), f64::from(source.height.max(1)));
    let (width, height) = if edited_width {
        let width = finite_pixels(width)?;
        let height = if lock { finite_pixels((source_height * width / source_width).max(1.0))? } else { finite_pixels(height)? };
        (width, height)
    } else {
        let height = finite_pixels(height)?;
        let width = if lock { finite_pixels((source_width * height / source_height).max(1.0))? } else { finite_pixels(width)? };
        (width, height)
    };
    let scale = if edited_width { width / source_width * PERCENT } else { height / source_height * PERCENT };
    Ok((PixelSize::new(width as u32, height as u32), scale))
}

fn finite_pixels(value: f64) -> Result<f64, SizeError> {
    if value.is_finite() && (1.0..=MAX_DOCUMENT_SIDE).contains(&value) { Ok(value.round()) } else { Err(SizeError::Pixels) }
}

/// Whole pixels from `pixels`, at least one and at most the engine's limit; `fallback` for a value that is not a
/// number.
fn clamp_pixels(pixels: f64, fallback: u32) -> u32 {
    if pixels.is_finite() { pixels.round().clamp(1.0, MAX_DOCUMENT_SIDE) as u32 } else { fallback }
}

/// Which side of the size a field edits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Side {
    Width,
    Height,
}

/// Whether `unit` measures pixels (pixels or percent of the opening size) rather than a physical length.
fn is_pixel_unit(unit: UnitKind) -> bool {
    matches!(unit, UnitKind::Pixels | UnitKind::Percent)
}

/// Pixels per inch in one of a resolution unit; `None` for a kind that is not a resolution.
fn resolution_per_inch(unit: UnitKind) -> Option<f64> {
    match unit {
        UnitKind::PixelsPerInch => Some(1.0),
        UnitKind::PixelsPerCentimetre => UnitKind::Centimetres.per_inch(UnitKind::POINTS_PER_INCH),
        _ => None,
    }
}

/// A value typed in a width or height field, with the state it produced.
#[derive(Clone, Debug, PartialEq)]
struct Typed {
    side: Side,
    value: f64,
    unit: UnitKind,
    relative: bool,
    size: PixelSize,
    ppi: f64,
}

/// The width, height and resolution of Image Size, Canvas Size, export and Save for Web, linked so that a change in
/// one field changes the others the way Photoshop's dialog does.
///
/// A dialog shows each field as a `UnitSelect` over the value `width()`, `height()` or `resolution()` returns and
/// the unit `unit()` or `resolution_unit()` returns. A value the user commits goes to `set_width`, `set_height` or
/// `set_resolution`, and a unit the user picks goes to `set_unit` or `set_resolution_unit`; the model converts, so
/// the field always shows the pixel size in its unit. Percent is relative to the opening size. With the lock on, a
/// side scales the other by the opening ratio. A value typed in a physical unit shows back exactly while the pixel
/// size it made stands (72.27 pt at 72 ppi is 72 pixels and still reads 72.27 pt).
#[derive(Clone, Debug, PartialEq)]
pub struct LinkedSize {
    original: PixelSize,
    original_ppi: f64,
    current: PixelSize,
    ppi: f64,
    unit: UnitKind,
    resolution_unit: UnitKind,
    lock: bool,
    resampling: bool,
    relative: bool,
    chain: bool,
    points_per_inch: f64,
    typed: Option<Typed>,
}

impl LinkedSize {
    /// Fields for a document of size `original` at 72 ppi, in pixels and pixels per inch, locked, resampling and
    /// absolute, with the chain drawn.
    pub fn new(original: PixelSize) -> Self {
        LinkedSize {
            original,
            original_ppi: DEFAULT_RESOLUTION,
            current: original,
            ppi: DEFAULT_RESOLUTION,
            unit: UnitKind::Pixels,
            resolution_unit: UnitKind::PixelsPerInch,
            lock: true,
            resampling: true,
            relative: false,
            chain: true,
            points_per_inch: UnitKind::POINTS_PER_INCH,
            typed: None,
        }
    }

    /// The resolution the document opens with, in pixels per inch, limited to `PPI_RANGE`. A value that is not a
    /// number keeps the default.
    pub fn opening_resolution(mut self, pixels_per_inch: f64) -> Self {
        if pixels_per_inch.is_finite() {
            let ppi = pixels_per_inch.clamp(*PPI_RANGE.start(), *PPI_RANGE.end());
            self.original_ppi = ppi;
            self.ppi = ppi;
        }
        self
    }

    /// The points in an inch of the Points and Picas units (the Point/Pica Size preference: 72, or 72.27 for
    /// traditional points). A value that is not positive keeps 72.
    pub fn points_per_inch(mut self, points_per_inch: f64) -> Self {
        if points_per_inch.is_finite() && points_per_inch > 0.0 {
            self.points_per_inch = points_per_inch;
        }
        self
    }

    /// Whether the dialog draws the chain that links width and height. Off, the sides never scale together
    /// (Canvas Size).
    pub fn chain(mut self, chain: bool) -> Self {
        self.chain = chain;
        self
    }

    /// Whether the dialog draws the chain.
    pub fn has_chain(&self) -> bool {
        self.chain
    }

    /// Keeps the aspect ratio when one side changes (the chain toggle).
    pub fn lock(&mut self, on: bool) {
        self.lock = on;
    }

    /// Whether the chain shows as linked: the model has a chain and its lock is on, or resampling is off (the pixel
    /// count is fixed, so the sides scale together).
    pub fn is_linked(&self) -> bool {
        !self.resampling || self.sides_scale_together()
    }

    /// Whether setting one side scales the other: only a model with a chain links its sides.
    fn sides_scale_together(&self) -> bool {
        self.chain && self.lock
    }

    /// Whether the pixel count may change: Image Size with Resample on, and always in Canvas Size. Turning it off
    /// puts back the opening pixel size, since the pixel count is fixed from then on.
    pub fn resampling(&mut self, on: bool) {
        self.resampling = on;
        if !on {
            self.current = self.original;
            self.typed = None;
        }
    }

    /// Whether the pixel count may change.
    pub fn is_resampling(&self) -> bool {
        self.resampling
    }

    /// A relative size (Canvas Size's Relative box): the fields show the change in size, not the size.
    pub fn relative(&mut self, on: bool) {
        self.relative = on;
    }

    /// Whether the fields show the change in size.
    pub fn is_relative(&self) -> bool {
        self.relative
    }

    /// The unit the width and height fields show.
    pub fn unit(&self) -> UnitKind {
        self.unit
    }

    /// Shows the width and height in `unit`. Returns `false` and changes nothing for a kind that is not a length.
    pub fn set_unit(&mut self, unit: UnitKind) -> bool {
        let length = unit.from_pixels(0.0, 1.0, self.ppi, self.points_per_inch).is_some();
        if length {
            self.unit = unit;
        }
        length
    }

    /// The unit the resolution field shows.
    pub fn resolution_unit(&self) -> UnitKind {
        self.resolution_unit
    }

    /// Shows the resolution in `unit`. Returns `false` and changes nothing for a kind that is not a resolution.
    pub fn set_resolution_unit(&mut self, unit: UnitKind) -> bool {
        let resolution = resolution_per_inch(unit).is_some();
        if resolution {
            self.resolution_unit = unit;
        }
        resolution
    }

    /// The pixel size being edited.
    pub fn pixels(&self) -> PixelSize {
        self.current
    }

    /// The pixel size the dialog opened with.
    pub fn original(&self) -> PixelSize {
        self.original
    }

    /// The resolution being edited, in pixels per inch.
    pub fn pixels_per_inch(&self) -> f64 {
        self.ppi
    }

    /// Puts back the opening size and resolution.
    pub fn reset(&mut self) {
        self.current = self.original;
        self.ppi = self.original_ppi;
        self.typed = None;
    }

    /// Whether the width and height fields take input: in pixels and percent they need Resample on, since only
    /// resampling changes the pixel count.
    pub fn is_editable(&self) -> bool {
        self.resampling || !is_pixel_unit(self.unit)
    }

    /// The width in the current unit (the change in width, in a relative size), unrounded.
    pub fn width(&self) -> f64 {
        self.shown(Side::Width)
    }

    /// The height in the current unit (the change in height, in a relative size), unrounded.
    pub fn height(&self) -> f64 {
        self.shown(Side::Height)
    }

    /// The resolution in the current resolution unit, unrounded.
    pub fn resolution(&self) -> f64 {
        self.ppi / self.resolution_factor()
    }

    /// The values the width field takes, in the current unit: from one pixel (the whole change, in a relative size)
    /// to the engine's limit.
    pub fn width_range(&self) -> RangeInclusive<f64> {
        self.range(Side::Width)
    }

    /// The values the height field takes, in the current unit.
    pub fn height_range(&self) -> RangeInclusive<f64> {
        self.range(Side::Height)
    }

    /// The values the resolution field takes, in the current resolution unit.
    pub fn resolution_range(&self) -> RangeInclusive<f64> {
        let factor = self.resolution_factor();
        PPI_RANGE.start() / factor..=PPI_RANGE.end() / factor
    }

    /// Sets the width as typing `value` in its field does, in the current unit. A value that is not a number is
    /// ignored.
    ///
    /// With Resample off the pixel count is fixed: a width in pixels or percent cannot change, and a physical width
    /// changes the resolution instead.
    pub fn set_width(&mut self, value: f64) {
        self.edit(Side::Width, value);
    }

    /// Sets the height as typing `value` in its field does; see [`set_width`](LinkedSize::set_width).
    pub fn set_height(&mut self, value: f64) {
        self.edit(Side::Height, value);
    }

    /// Sets the resolution as typing `value` in its field does, in the current resolution unit. With Resample on and
    /// a physical unit the print size stays and the pixels follow the resolution; otherwise the pixel count stays.
    pub fn set_resolution(&mut self, value: f64) {
        if !value.is_finite() {
            return;
        }
        let ppi = (value * self.resolution_factor()).clamp(*PPI_RANGE.start(), *PPI_RANGE.end());
        let old = self.ppi;
        if self.resampling && !is_pixel_unit(self.unit) && old > 0.0 {
            self.current.width = clamp_pixels(f64::from(self.current.width) * ppi / old, self.current.width);
            self.current.height = clamp_pixels(f64::from(self.current.height) * ppi / old, self.current.height);
        }
        self.ppi = ppi;
    }

    fn resolution_factor(&self) -> f64 {
        resolution_per_inch(self.resolution_unit).unwrap_or(1.0)
    }

    fn range(&self, side: Side) -> RangeInclusive<f64> {
        let original = pixels_along(self.original, side);
        let limit = self.unit_value(MAX_DOCUMENT_SIDE, original);
        if self.relative { -limit..=limit } else { self.unit_value(1.0, original).min(self.shown(side))..=limit }
    }

    fn unit_value(&self, pixels: f64, original: f64) -> f64 {
        self.unit.from_pixels(pixels, original, self.ppi, self.points_per_inch).unwrap_or(pixels)
    }

    /// The field's value in the current unit: the value typed last while the pixel size it made stands, else the
    /// pixel size converted.
    fn shown(&self, side: Side) -> f64 {
        if let Some(typed) = &self.typed
            && typed.side == side
            && typed.unit == self.unit
            && typed.relative == self.relative
            && typed.size == self.current
            && typed.ppi == self.ppi
        {
            return typed.value;
        }
        let (current, original) = (pixels_along(self.current, side), pixels_along(self.original, side));
        self.unit_value(if self.relative { current - original } else { current }, original)
    }

    fn edit(&mut self, side: Side, value: f64) {
        if !value.is_finite() {
            return;
        }
        let (current, original) = (pixels_along(self.current, side), pixels_along(self.original, side));
        let Some(pixels) = self.unit.to_pixels(value, original, self.ppi, self.points_per_inch) else { return };
        self.typed = None;
        if !self.resampling {
            if !is_pixel_unit(self.unit) && pixels > 0.0 {
                self.ppi = (self.ppi * current / pixels).clamp(*PPI_RANGE.start(), *PPI_RANGE.end());
                self.current = self.original;
                self.remember(side, value);
            }
            return;
        }
        let pixels = if self.relative { original + pixels } else { pixels };
        self.scale(side, pixels);
        self.remember(side, value);
    }

    fn remember(&mut self, side: Side, value: f64) {
        self.typed = Some(Typed { side, value, unit: self.unit, relative: self.relative, size: self.current, ppi: self.ppi });
    }

    /// Sets one side in pixels, scaling the other by the opening ratio when locked.
    fn scale(&mut self, side: Side, pixels: f64) {
        let (original_width, original_height) = (f64::from(self.original.width.max(1)), f64::from(self.original.height.max(1)));
        match side {
            Side::Width => {
                self.current.width = clamp_pixels(pixels, self.current.width);
                if self.sides_scale_together() {
                    let scaled = f64::from(self.current.width) * original_height / original_width;
                    self.current.height = clamp_pixels(scaled, self.current.height);
                }
            }
            Side::Height => {
                self.current.height = clamp_pixels(pixels, self.current.height);
                if self.sides_scale_together() {
                    let scaled = f64::from(self.current.height) * original_width / original_height;
                    self.current.width = clamp_pixels(scaled, self.current.width);
                }
            }
        }
    }
}
