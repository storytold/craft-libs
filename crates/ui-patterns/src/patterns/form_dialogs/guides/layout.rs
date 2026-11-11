//! The New Guide Layout calculation and the units the guide dialogs measure in. Pure, so the dialog's live preview
//! and the app's command make the same guides.

use super::GuideDialogWords;
use crate::canvas_data::{Guide, GuideKey};
use crate::runtime::env::{Label, UnitKind, Units};
use craft_behavior::Axis;
use craft_behavior::quantity::Reader;
use craft_theme::Rgba;

/// The most columns or rows of a layout.
pub const MAX_COUNT: u32 = 1000;
/// The most guide edges a layout makes over all targets.
const MAX_EDGES: u64 = 16_000;
/// Guides closer than this are one guide.
const SAME_GUIDE: f64 = 0.01;
/// The tolerance when columns are tested against the room they must fit in.
const FIT_TOLERANCE: f64 = 0.001;

/// A rectangle a layout fills, in document pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct GuideRect {
    /// Left edge.
    pub x0: f64,
    /// Top edge.
    pub y0: f64,
    /// Right edge.
    pub x1: f64,
    /// Bottom edge.
    pub y1: f64,
}

impl GuideRect {
    /// The rectangle from (`x0`, `y0`) to (`x1`, `y1`).
    pub const fn new(x0: f64, y0: f64, x1: f64, y1: f64) -> Self {
        GuideRect { x0, y0, x1, y1 }
    }

    /// The width.
    pub fn width(&self) -> f64 {
        self.x1 - self.x0
    }

    /// The height.
    pub fn height(&self) -> f64 {
        self.y1 - self.y0
    }
}

/// Where New Guide Layout puts its guides.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum GuideTarget {
    /// The whole document.
    #[default]
    Document,
    /// Each selected artboard (listed only when the app passes some with `GuideLayoutDialog::artboards`).
    SelectedArtboards,
}

/// The Preset list.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum GuideLayoutPreset {
    /// No preset: the fields keep the user's values.
    #[default]
    Custom,
    /// 5 columns, 3 rows, no gutter.
    FiveColumnsThreeRows,
    /// 8 columns, gutter 20 px.
    EightColumns,
    /// 12 columns, gutter 24 px.
    TwelveColumns,
}

impl GuideLayoutPreset {
    /// Every preset in list order.
    pub const ALL: [GuideLayoutPreset; 4] =
        [GuideLayoutPreset::Custom, GuideLayoutPreset::FiveColumnsThreeRows, GuideLayoutPreset::EightColumns, GuideLayoutPreset::TwelveColumns];

    /// `layout` with this preset's columns, rows and gutters (in pixels), no margin and Center Columns off; target
    /// and Clear Existing Guides stay. Custom changes nothing.
    pub fn apply(self, layout: &mut GuideLayoutOptions) {
        let (columns, rows, gutter) = match self {
            GuideLayoutPreset::Custom => return,
            GuideLayoutPreset::FiveColumnsThreeRows => (5, 3, 0.0),
            GuideLayoutPreset::EightColumns => (8, 0, 20.0),
            GuideLayoutPreset::TwelveColumns => (12, 0, 24.0),
        };
        layout.preset = self;
        layout.columns = GuideGroup::new(true, columns, None, GuideLength::pixels(gutter));
        layout.rows = GuideGroup::new(rows > 0, rows.max(1), None, GuideLength::pixels(0.0));
        layout.margin_enabled = true;
        layout.margin = [GuideLength::pixels(0.0); 4];
        layout.center_columns = false;
    }

    pub(super) fn label(self, words: &GuideDialogWords) -> Label {
        match self {
            GuideLayoutPreset::Custom => words.preset_custom.clone(),
            GuideLayoutPreset::FiveColumnsThreeRows => words.preset_five.clone(),
            GuideLayoutPreset::EightColumns => words.preset_eight.clone(),
            GuideLayoutPreset::TwelveColumns => words.preset_twelve.clone(),
        }
    }
}

/// A length in a unit; a percentage is of the target's width or height.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct GuideLength {
    /// The number of units.
    pub value: f64,
    /// A length unit, or `UnitKind::Percent`.
    pub unit: UnitKind,
}

impl GuideLength {
    /// `value` of `unit`.
    pub const fn new(value: f64, unit: UnitKind) -> Self {
        GuideLength { value, unit }
    }

    /// `value` pixels.
    pub const fn pixels(value: f64) -> Self {
        GuideLength { value, unit: UnitKind::Pixels }
    }

    /// `value` percent.
    pub const fn percent(value: f64) -> Self {
        GuideLength { value, unit: UnitKind::Percent }
    }
}

/// The document a guide dialog measures in: its size in pixels (for percent), resolution and ruler unit.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct GuideUnits {
    /// The ruler's length unit.
    pub ruler: UnitKind,
    /// Document width in pixels.
    pub width_px: f64,
    /// Document height in pixels.
    pub height_px: f64,
    /// Document resolution, for converting inches, centimetres and points to pixels.
    pub pixels_per_inch: f64,
    /// 72 (PostScript) or 72.27 (traditional): Preferences > Units & Rulers > Point/Pica Size.
    pub points_per_inch: f64,
}

impl GuideUnits {
    /// A `width_px` by `height_px` document at `pixels_per_inch`, measured in `ruler`, with PostScript points.
    pub fn new(ruler: UnitKind, width_px: f64, height_px: f64, pixels_per_inch: f64) -> Self {
        GuideUnits { ruler, width_px, height_px, pixels_per_inch, points_per_inch: UnitKind::POINTS_PER_INCH }
    }

    /// The same units with points of `points_per_inch` to the inch.
    #[must_use]
    pub fn points_per_inch(mut self, points_per_inch: f64) -> Self {
        self.points_per_inch = points_per_inch;
        self
    }

    /// `length` in pixels across `extent` pixels.
    pub fn to_pixels(&self, length: GuideLength, extent: f64) -> Option<f64> {
        length.unit.to_pixels(length.value, extent, self.pixels_per_inch, self.points_per_inch)
    }

    /// `pixels` in `unit`, across `extent` pixels.
    pub fn from_pixels(&self, pixels: f64, unit: UnitKind, extent: f64) -> Option<f64> {
        unit.from_pixels(pixels, extent, self.pixels_per_inch, self.points_per_inch)
    }

    /// The reader of the fields that show the ruler unit: it takes "50%", "2.5 cm", "1in", "12 pt", "3 pica" and
    /// "100 px", each converted to the ruler unit, percent of `extent`; a bare number is in the ruler unit.
    pub fn reader(&self, units: &Units, extent: f64) -> Reader {
        units.document_reader(self.ruler, extent, self.pixels_per_inch, self.points_per_inch)
    }
}

/// Columns or rows of a layout.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct GuideGroup {
    /// The group's check box.
    pub enabled: bool,
    /// 1 to [`MAX_COUNT`].
    pub number: u32,
    /// Width of a column or height of a row; `None` shares the room out evenly.
    pub size: Option<GuideLength>,
    /// Space between columns or rows.
    pub gutter: GuideLength,
}

impl GuideGroup {
    /// A group with these settings.
    pub const fn new(enabled: bool, number: u32, size: Option<GuideLength>, gutter: GuideLength) -> Self {
        GuideGroup { enabled, number, size, gutter }
    }
}

/// New Guide Layout's values: what it opens with, what it answers, and the calculation ([`lines`](Self::lines)).
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct GuideLayoutOptions {
    /// The Preset list's choice.
    pub preset: GuideLayoutPreset,
    /// Where the guides go.
    pub target: GuideTarget,
    /// The column group.
    pub columns: GuideGroup,
    /// The row group.
    pub rows: GuideGroup,
    /// The margin group's check box.
    pub margin_enabled: bool,
    /// Top, left, bottom, right.
    pub margin: [GuideLength; 4],
    /// With a fixed column width, centre the columns (and with a fixed row height the rows) between the margins
    /// instead of packing them from the start.
    pub center_columns: bool,
    /// Replaces the existing guides instead of adding to them.
    pub clear_existing: bool,
}

impl Default for GuideLayoutOptions {
    /// The factory layout: 8 columns with a 20 px gutter, rows off, no margin.
    fn default() -> Self {
        GuideLayoutOptions {
            preset: GuideLayoutPreset::Custom,
            target: GuideTarget::Document,
            columns: GuideGroup::new(true, 8, None, GuideLength::pixels(20.0)),
            rows: GuideGroup::new(false, 1, None, GuideLength::pixels(0.0)),
            margin_enabled: true,
            margin: [GuideLength::pixels(0.0); 4],
            center_columns: false,
            clear_existing: false,
        }
    }
}

/// Why a layout can't be made; the dialog shows it under the fields with OK disabled.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum GuideLayoutError {
    /// Columns or rows with their size and gutters are wider than the room inside the margins.
    DoesNotFit,
    /// The margins leave no room inside a target.
    MarginsTooLarge,
    /// Columns, Rows and Margin are all off (or empty).
    Nothing,
    /// More guide edges than a layout may make over all targets.
    TooMany,
    /// `GuideTarget::SelectedArtboards` with none selected.
    NoArtboards,
}

impl GuideLayoutError {
    /// The translated message the dialog shows for this error.
    pub(super) fn message(self, words: &GuideDialogWords) -> Label {
        match self {
            GuideLayoutError::DoesNotFit => words.does_not_fit.clone(),
            GuideLayoutError::MarginsTooLarge => words.margins_too_large.clone(),
            GuideLayoutError::Nothing => words.nothing.clone(),
            GuideLayoutError::TooMany => words.too_many.clone(),
            GuideLayoutError::NoArtboards => words.no_artboards.clone(),
        }
    }
}

/// Adds `value` to `lines` unless a line within [`SAME_GUIDE`] is there. Fails for a value that is not finite.
fn add(lines: &mut Vec<f64>, value: f64) -> Result<(), GuideLayoutError> {
    if !value.is_finite() {
        return Err(GuideLayoutError::DoesNotFit);
    }
    if !lines.iter().any(|x| (x - value).abs() < SAME_GUIDE) {
        lines.push(value);
    }
    Ok(())
}

/// `length` in pixels across `extent`, when it is a finite length of at least zero.
fn pixels_of(length: GuideLength, units: &GuideUnits, extent: f64) -> Option<f64> {
    units.to_pixels(length, extent).filter(|v| v.is_finite() && *v >= 0.0)
}

/// The edges of `count` columns (or rows) across `start..end`: both edges of every one. A fixed `size` packs them
/// from `start`, or centres them; otherwise they share the room left by the gutters.
fn axis_edges(start: f64, end: f64, count: u64, fixed: f64, gap: f64, center: bool, out: &mut Vec<f64>) -> Result<(), GuideLayoutError> {
    if count == 0 {
        return Ok(());
    }
    let available = end - start;
    let n = count as f64;
    let size = if fixed > 0.0 { fixed } else { (available - (n - 1.0) * gap) / n };
    let total = n * size + (n - 1.0) * gap;
    if size <= 0.0 || !total.is_finite() || total > available + FIT_TOLERANCE {
        return Err(GuideLayoutError::DoesNotFit);
    }
    let origin = if center && fixed > 0.0 { start + (available - total) / 2.0 } else { start };
    for i in 0..count {
        let a = origin + i as f64 * (size + gap);
        add(out, a)?;
        add(out, a + size)?;
    }
    Ok(())
}

impl GuideLayoutOptions {
    /// The guides OK commits for `targets` (the document, or each selected artboard) over the `existing` ones: the
    /// existing guides (none with Clear Existing Guides; with artboards only those inside a selected artboard go)
    /// and the layout's own, keyed from `first_key` up and drawn in `color`.
    ///
    /// Per target the layout has the margin edges when any margin is not zero, then both edges of every column and
    /// row. Without a size, columns share the room after the gutters, `(room - (n - 1) x gutter) / n`; with one they
    /// pack from the left margin, or centre with Center Columns.
    pub fn lines(
        &self,
        targets: &[GuideRect],
        existing: &[Guide],
        units: &GuideUnits,
        first_key: u64,
        color: Option<Rgba>,
    ) -> Result<Vec<Guide>, GuideLayoutError> {
        let count = |group: &GuideGroup| if group.enabled { u64::from(group.number) } else { 0 };
        let (columns, rows) = (count(&self.columns), count(&self.rows));
        if columns.max(rows) > u64::from(MAX_COUNT) {
            return Err(GuideLayoutError::TooMany);
        }
        if targets.is_empty() {
            return Err(GuideLayoutError::NoArtboards);
        }
        if (columns + rows + 4).saturating_mul(2).saturating_mul(targets.len() as u64) > MAX_EDGES {
            return Err(GuideLayoutError::TooMany);
        }
        let zero = GuideLength::pixels(0.0);
        let margin = if self.margin_enabled { self.margin } else { [zero; 4] };
        let (mut across_x, mut across_y): (Vec<f64>, Vec<f64>) = (Vec::new(), Vec::new());
        for rect in targets {
            let (across, down) = (rect.width(), rect.height());
            let sides = [(margin[0], down), (margin[1], across), (margin[2], down), (margin[3], across)];
            let [top, left, bottom, right] = sides.map(|(length, extent)| pixels_of(length, units, extent).ok_or(GuideLayoutError::MarginsTooLarge));
            let (top, left, bottom, right) = (top?, left?, bottom?, right?);
            let (x0, y0, x1, y1) = (rect.x0 + left, rect.y0 + top, rect.x1 - right, rect.y1 - bottom);
            if x0 >= x1 || y0 >= y1 {
                return Err(GuideLayoutError::MarginsTooLarge);
            }
            if [top, left, bottom, right].iter().any(|m| *m != 0.0) {
                add(&mut across_x, x0)?;
                add(&mut across_x, x1)?;
                add(&mut across_y, y0)?;
                add(&mut across_y, y1)?;
            }
            // A disabled group's size and gutter are not read, so stale values can't fail it.
            for (group, number, extent, from, to, out) in
                [(&self.columns, columns, across, x0, x1, &mut across_x), (&self.rows, rows, down, y0, y1, &mut across_y)]
            {
                if number == 0 {
                    continue;
                }
                let size = match group.size {
                    Some(size) => pixels_of(size, units, extent).ok_or(GuideLayoutError::DoesNotFit)?,
                    None => 0.0,
                };
                let gap = pixels_of(group.gutter, units, extent).ok_or(GuideLayoutError::DoesNotFit)?;
                axis_edges(from, to, number, size, gap, self.center_columns, out)?;
            }
        }
        if across_x.is_empty() && across_y.is_empty() {
            return Err(GuideLayoutError::Nothing);
        }
        let mut all: Vec<Guide> = existing.to_vec();
        if self.clear_existing {
            if self.target == GuideTarget::SelectedArtboards {
                // Guides are document-wide lines: only those inside a selected artboard go.
                all.retain(|guide| {
                    !targets.iter().any(|r| match guide.along {
                        Axis::Y => guide.position >= r.x0 && guide.position <= r.x1,
                        _ => guide.position >= r.y0 && guide.position <= r.y1,
                    })
                });
            } else {
                all.clear();
            }
        }
        across_x.sort_by(f64::total_cmp);
        across_y.sort_by(f64::total_cmp);
        let mut key = first_key;
        for (along, positions) in [(Axis::Y, across_x), (Axis::X, across_y)] {
            for position in positions {
                let same = |guide: &Guide| guide.along == along && (guide.position - position).abs() < SAME_GUIDE;
                if all.iter().any(same) {
                    continue;
                }
                let mut guide = Guide::new(GuideKey::new(key), along, position);
                guide.color = color;
                all.push(guide);
                key += 1;
            }
        }
        Ok(all)
    }

    /// Whether `other` has the same layout fields: everything but the preset, the target and Clear Existing Guides
    /// (a typed edit to one of those fields turns the preset to Custom).
    pub(super) fn same_fields(&self, other: &GuideLayoutOptions) -> bool {
        (self.columns, self.rows, self.margin_enabled, self.margin, self.center_columns)
            == (other.columns, other.rows, other.margin_enabled, other.margin, other.center_columns)
    }
}
