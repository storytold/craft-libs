//! The stroke style of a path: width, caps, joins, alignment and dashes, without egui.
//!
//! [`StrokeStyle`] is the value an app's engine and the stroke editor (`craft_draw::ui::stroke`)
//! share: PhotoCraft's `ShapeStroke` without its paint, with PhotoCraft's limits
//! ([`StrokeStyle::validate`]) and presets ([`StrokeStyle::with_line_style`]). Colour and paint
//! stay with the app (S7-shapes' paints list).

use std::fmt;
use std::ops::RangeInclusive;

use serde::{Deserialize, Serialize};

/// Where the stroke sits on the path. Only a closed path aligns; an open one is always centred.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StrokeAlign {
    /// The stroke lies inside the path.
    Inside,
    /// The stroke is centred on the path.
    #[default]
    Center,
    /// The stroke lies outside the path.
    Outside,
}

/// The shape at the ends of an open path and of each dash.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LineCap {
    /// Butt / Flat: the stroke ends at the end point.
    #[default]
    Butt,
    /// A semicircle past the end point.
    Round,
    /// The stroke goes on half its width past the end point.
    Square,
}

/// The shape of the stroke where two segments meet at a corner.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LineJoin {
    /// A sharp point, cut off past [`StrokeStyle::miter_limit`].
    #[default]
    Miter,
    /// A circular arc around the corner point.
    Round,
    /// The corner is cut off flat.
    Bevel,
}

/// The name of a dash pattern, as the stroke editor's presets and its opener show it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LineStyle {
    /// No dashes.
    Solid,
    /// Dashes [`StrokeStyle::DASHED`].
    Dashed,
    /// Dashes [`StrokeStyle::DOTTED`] with round caps.
    Dotted,
    /// Any other pattern.
    Custom,
}

/// The unit of the dash lengths and the dash offset. It changes what the numbers mean, never the
/// stored numbers: the stroke editor shows it as the fields' suffix.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DashUnit {
    /// Multiples of the stroke width (PhotoCraft, Photoshop).
    #[default]
    Widths,
    /// Document units (VectorCraft).
    Document,
}

/// Why [`StrokeStyle::validate`] refuses a style. The `Display` text is PhotoCraft's engine
/// message, for logs and the app's own errors; the stroke editor shows its own words.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StrokeError {
    /// The width is not finite or outside [`StrokeStyle::WIDTH_RANGE`].
    Width,
    /// The opacity is not finite or outside 0..=1.
    Opacity,
    /// The miter limit is not finite or outside [`StrokeStyle::MITER_LIMIT_RANGE`].
    MiterLimit,
    /// The dash offset is not finite or its size is over [`StrokeStyle::DASH_OFFSET_MAX`].
    DashOffset,
    /// More than [`StrokeStyle::MAX_DASHES`] lengths, or a length not finite or outside
    /// [`StrokeStyle::DASH_RANGE`].
    DashPattern,
    /// A pattern whose lengths add up to less than [`StrokeStyle::DASH_TOTAL_MIN`].
    DashTotal,
}

impl fmt::Display for StrokeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // pc: crates/doc/src/vector.rs:270-290 `ShapeStroke::validate`, the messages verbatim.
        f.write_str(match self {
            StrokeError::Width => "stroke width must be finite and between 0 and 1000000 px",
            StrokeError::Opacity => "stroke opacity must be between 0 and 100%",
            StrokeError::MiterLimit => "miter limit must be between 1 and 500",
            StrokeError::DashOffset => "dash offset must be finite and within ±1000000 widths",
            StrokeError::DashPattern => "dash pattern must contain at most 32 finite lengths between 0 and 10000 widths",
            StrokeError::DashTotal => "dash pattern total must be at least 0.01 widths",
        })
    }
}

impl std::error::Error for StrokeError {}

/// How a path is stroked, except its paint: width, alignment, caps, corners, dashes and opacity.
/// The default is PhotoCraft's `ShapeStroke::default()` without the paint: width 3, centred, butt
/// caps, miter corners with limit 100, solid, opacity 1, dashes in widths.
///
/// Serialised in camelCase with every field optional (missing fields take the default), so an
/// app keeps it in its settings and documents.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct StrokeStyle {
    /// The width in document units.
    pub width: f32,
    /// Where the stroke lies relative to the path.
    pub align: StrokeAlign,
    /// The shape of open ends.
    pub cap: LineCap,
    /// The shape of corners.
    pub join: LineJoin,
    /// The miter limit as a multiple of the width (Photoshop's 100 mitres nearly every corner).
    pub miter_limit: f32,
    /// Dash and gap lengths, alternating, in [`dash_unit`](Self::dash_unit); empty is solid.
    pub dashes: Vec<f32>,
    /// Where the pattern starts along the path, in [`dash_unit`](Self::dash_unit).
    pub dash_offset: f32,
    /// What a dash or gap length counts in.
    pub dash_unit: DashUnit,
    /// 0..=1.
    pub opacity: f32,
}

impl Default for StrokeStyle {
    fn default() -> Self {
        // pc: crates/doc/src/vector.rs:251-264 `ShapeStroke::default`.
        Self {
            width: 3.0,
            align: StrokeAlign::Center,
            cap: LineCap::Butt,
            join: LineJoin::Miter,
            miter_limit: 100.0,
            dashes: Vec::new(),
            dash_offset: 0.0,
            dash_unit: DashUnit::Widths,
            opacity: 1.0,
        }
    }
}

impl StrokeStyle {
    /// The widths [`validate`](Self::validate) accepts.
    pub const WIDTH_RANGE: RangeInclusive<f32> = 0.0..=1_000_000.0;
    /// The miter limits [`validate`](Self::validate) accepts.
    pub const MITER_LIMIT_RANGE: RangeInclusive<f32> = 1.0..=500.0;
    /// The dash and gap lengths [`validate`](Self::validate) accepts.
    pub const DASH_RANGE: RangeInclusive<f32> = 0.0..=10_000.0;
    /// The largest dash offset, either way.
    pub const DASH_OFFSET_MAX: f32 = 1_000_000.0;
    /// The most dash and gap lengths a pattern holds (16 pairs).
    pub const MAX_DASHES: usize = 32;
    /// The smallest total of a dashed pattern.
    pub const DASH_TOTAL_MIN: f32 = 0.01;
    /// The Dashed preset's pattern.
    pub const DASHED: [f32; 2] = [4.0, 2.0];
    /// The Dotted preset's pattern (with round caps).
    pub const DOTTED: [f32; 2] = [0.0, 2.0];

    /// Whether an engine accepts the style. PhotoCraft's rules, checked in this order, the first
    /// failure being the error: width finite and in [`WIDTH_RANGE`](Self::WIDTH_RANGE); opacity
    /// finite and in 0..=1; miter limit finite and in
    /// [`MITER_LIMIT_RANGE`](Self::MITER_LIMIT_RANGE); dash offset finite with `abs() <=`
    /// [`DASH_OFFSET_MAX`](Self::DASH_OFFSET_MAX); at most [`MAX_DASHES`](Self::MAX_DASHES)
    /// lengths, each finite and in [`DASH_RANGE`](Self::DASH_RANGE); a non-empty pattern whose sum
    /// is at least [`DASH_TOTAL_MIN`](Self::DASH_TOTAL_MIN). The limits are the same in either
    /// [`DashUnit`].
    pub fn validate(&self) -> Result<(), StrokeError> {
        // pc: crates/doc/src/vector.rs:270-290 `ShapeStroke::validate`.
        if !self.width.is_finite() || !Self::WIDTH_RANGE.contains(&self.width) {
            return Err(StrokeError::Width);
        }
        if !self.opacity.is_finite() || !(0.0..=1.0).contains(&self.opacity) {
            return Err(StrokeError::Opacity);
        }
        if !self.miter_limit.is_finite() || !Self::MITER_LIMIT_RANGE.contains(&self.miter_limit) {
            return Err(StrokeError::MiterLimit);
        }
        if !self.dash_offset.is_finite() || self.dash_offset.abs() > Self::DASH_OFFSET_MAX {
            return Err(StrokeError::DashOffset);
        }
        if self.dashes.len() > Self::MAX_DASHES || self.dashes.iter().any(|d| !d.is_finite() || !Self::DASH_RANGE.contains(d)) {
            return Err(StrokeError::DashPattern);
        }
        if !self.dashes.is_empty() && self.dashes.iter().sum::<f32>() < Self::DASH_TOTAL_MIN {
            return Err(StrokeError::DashTotal);
        }
        Ok(())
    }

    /// The pattern's name: empty dashes are [`LineStyle::Solid`]; exactly
    /// [`DASHED`](Self::DASHED) is [`LineStyle::Dashed`]; exactly [`DOTTED`](Self::DOTTED) with
    /// [`LineCap::Round`] is [`LineStyle::Dotted`]; anything else is [`LineStyle::Custom`]
    /// (`[0, 2]` with another cap too). The dash offset plays no part.
    pub fn line_style(&self) -> LineStyle {
        // pc: crates/ui-egui/src/tools/shape_stroke_ui.rs:53-63 `StrokeOptions::label`.
        if self.dashes.is_empty() {
            LineStyle::Solid
        } else if self.dashes == Self::DASHED {
            LineStyle::Dashed
        } else if self.dashes == Self::DOTTED && self.cap == LineCap::Round {
            LineStyle::Dotted
        } else {
            LineStyle::Custom
        }
    }

    /// This style with the preset `style`: the dash offset goes to 0 and the dashes become empty
    /// (Solid), [`DASHED`](Self::DASHED), or [`DOTTED`](Self::DOTTED) with the cap set to
    /// [`LineCap::Round`]. Every other field is kept. [`LineStyle::Custom`] returns the style
    /// unchanged.
    #[must_use]
    pub fn with_line_style(&self, style: LineStyle) -> StrokeStyle {
        // pc: crates/ui-egui/src/tools/shape_stroke_ui.rs:65-75 `StrokeOptions::preset`.
        let mut out = self.clone();
        match style {
            LineStyle::Custom => return out,
            LineStyle::Solid => out.dashes = Vec::new(),
            LineStyle::Dashed => out.dashes = Self::DASHED.to_vec(),
            LineStyle::Dotted => {
                out.dashes = Self::DOTTED.to_vec();
                out.cap = LineCap::Round;
            }
        }
        out.dash_offset = 0.0;
        out
    }

    /// The dashes as (dash, gap) pairs in order. An odd-length pattern's last pair has no gap
    /// (`None`), as PhotoCraft's editor shows it (one Dash field in the last row).
    pub fn dash_pairs(&self) -> Vec<(f32, Option<f32>)> {
        // pc: crates/ui-egui/src/tools/shape_stroke_ui/editor.rs:201-211 (`dashes.chunks_mut(2)`).
        self.dashes.chunks(2).map(|c| (c[0], c.get(1).copied())).collect()
    }

    /// This style with its first `pairs.len()` pairs replaced by `pairs` and every later entry of
    /// the pattern kept, so a host that shows three pairs (VectorCraft, Photoshop) never loses
    /// pairs four and on. A `None` gap drops the gap when it replaces the pattern's last pair and
    /// keeps the stored gap anywhere else. Pairs past the stored ones are appended. The result is
    /// not validated.
    #[must_use]
    pub fn with_dash_pairs(&self, pairs: &[(f32, Option<f32>)]) -> StrokeStyle {
        // new: entries a field row does not show stay unchanged (step7-pen "Stroke editor", Dashes).
        let mut out = self.clone();
        for (i, &(dash, gap)) in pairs.iter().enumerate() {
            let at = i * 2;
            if at >= out.dashes.len() {
                out.dashes.push(dash);
                out.dashes.extend(gap);
                continue;
            }
            out.dashes[at] = dash;
            let is_last = at + 1 >= out.dashes.len();
            match (gap, is_last) {
                (Some(g), false) => out.dashes[at + 1] = g,
                (Some(g), true) => out.dashes.push(g),
                (None, true) => {}
                (None, false) => {
                    // The stored gap stays unless this pair is the pattern's last.
                    if at + 2 >= out.dashes.len() {
                        out.dashes.truncate(at + 1);
                    }
                }
            }
        }
        out
    }

    /// The style as a kurbo stroke, for an engine and the editor's line samples: width, join,
    /// miter limit and the cap at both ends as they are; dash lengths and offset multiplied by the
    /// width under [`DashUnit::Widths`], taken as they are under [`DashUnit::Document`]. Alignment
    /// and opacity are not part of a kurbo stroke: an engine strokes an aligned closed path twice
    /// as wide and clips to its inside ([`StrokeAlign::Inside`]) or outside
    /// ([`StrokeAlign::Outside`]), and applies the opacity itself.
    pub fn to_kurbo(&self) -> kurbo::Stroke {
        // new: kurbo `Stroke::new(width).with_join(..).with_miter_limit(..).with_caps(..)
        // .with_dashes(offset, pattern)`. PhotoCraft strokes with its own engine
        // (crates/ui-egui/src/tools/shape_stroke_ui/editor.rs:329-343 renders through it).
        let scale = match self.dash_unit {
            DashUnit::Widths => f64::from(self.width),
            DashUnit::Document => 1.0,
        };
        let cap = match self.cap {
            LineCap::Butt => kurbo::Cap::Butt,
            LineCap::Round => kurbo::Cap::Round,
            LineCap::Square => kurbo::Cap::Square,
        };
        let join = match self.join {
            LineJoin::Miter => kurbo::Join::Miter,
            LineJoin::Round => kurbo::Join::Round,
            LineJoin::Bevel => kurbo::Join::Bevel,
        };
        kurbo::Stroke::new(f64::from(self.width))
            .with_join(join)
            .with_miter_limit(f64::from(self.miter_limit))
            .with_caps(cap)
            .with_dashes(f64::from(self.dash_offset) * scale, self.dashes.iter().map(|&d| f64::from(d) * scale))
    }
}
