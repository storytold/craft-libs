//! An audio level meter: a bar on a decibel scale with zone colors, an optional RMS mark and an
//! optional peak-hold mark.
//!
//! The meter does not measure audio. The caller passes levels in dB relative to full scale (0 dB
//! is full scale), for example from its own peak and RMS detectors, and keeps a [`PeakHold`] per
//! channel when it wants a held peak. [`amplitude_to_db`] converts a linear peak. One call paints
//! one bar; a stereo meter is two calls.

use egui::{Color32, Rect, Response, Sense, Stroke, Ui, Vec2, accesskit, pos2};

/// The decibel range a meter shows, mapped linearly onto the bar.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeterScale {
    floor_db: f32,
    ceiling_db: f32,
}

impl MeterScale {
    /// The widest range [`MeterScale::new`] accepts, in dB.
    pub const MAX_SPAN_DB: f32 = 200.0;

    /// A scale from `floor_db` (the empty bar) to `ceiling_db` (the full bar).
    ///
    /// Returns `None` when either value is not finite, when `floor_db` is not below `ceiling_db`,
    /// or when the range is wider than [`MeterScale::MAX_SPAN_DB`].
    pub fn new(floor_db: f32, ceiling_db: f32) -> Option<Self> {
        let valid = floor_db.is_finite() && ceiling_db.is_finite() && floor_db < ceiling_db && ceiling_db - floor_db <= Self::MAX_SPAN_DB;
        valid.then_some(Self { floor_db, ceiling_db })
    }

    pub fn floor_db(&self) -> f32 {
        self.floor_db
    }

    pub fn ceiling_db(&self) -> f32 {
        self.ceiling_db
    }

    /// Where `db` falls on the bar, from 0 (at or below the floor) to 1 (at or above the ceiling).
    /// NaN and negative infinity give 0.
    pub fn position(&self, db: f32) -> f32 {
        if db.is_nan() {
            return 0.0;
        }
        ((db - self.floor_db) / (self.ceiling_db - self.floor_db)).clamp(0.0, 1.0)
    }

    /// `db` limited to the scale's range; NaN gives the floor.
    pub fn clamp(&self, db: f32) -> f32 {
        if db.is_nan() { self.floor_db } else { db.clamp(self.floor_db, self.ceiling_db) }
    }
}

impl Default for MeterScale {
    /// -60 dB to 0 dB.
    fn default() -> Self {
        Self { floor_db: -60.0, ceiling_db: 0.0 }
    }
}

/// A linear amplitude (1.0 is full scale) in dB: `20 * log10(|amplitude|)`.
///
/// Zero gives negative infinity and NaN gives NaN, which [`MeterScale::position`] draws as an
/// empty bar.
pub fn amplitude_to_db(amplitude: f32) -> f32 {
    20.0 * amplitude.abs().log10()
}

/// A level in dB as a linear amplitude: `10^(db / 20)`. Negative infinity gives 0.
pub fn db_to_amplitude(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

/// The lit and unlit colors of one zone of the bar.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeterZone {
    /// The part of the zone below the current level.
    pub lit: Color32,
    /// The part of the zone above the current level.
    pub unlit: Color32,
}

/// Colors and zone boundaries for a [`LevelMeter`], taken from the application's theme.
///
/// The bar has three zones: `low` from the scale's floor to `mid_from_db`, `mid` up to
/// `high_from_db`, and `high` up to the ceiling. Boundaries outside the scale are limited to it,
/// and a `mid_from_db` above `high_from_db` is lowered to it. A muted channel is usually drawn
/// with a second style whose zones are gray.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LevelMeterStyle {
    pub scale: MeterScale,
    pub low: MeterZone,
    pub mid: MeterZone,
    pub high: MeterZone,
    pub mid_from_db: f32,
    pub high_from_db: f32,
    /// The line across the bar at the RMS level; `Stroke::NONE` hides it.
    pub rms: Stroke,
    /// The line across the bar at the held peak; `Stroke::NONE` hides it.
    pub hold: Stroke,
}

/// Which way a [`LevelMeter`] fills.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MeterOrientation {
    /// Fills from the left edge to the right.
    #[default]
    Horizontal,
    /// Fills from the bottom edge up.
    Vertical,
}

/// One bar of an audio level meter.
///
/// The surrounding UI scope supplies the ID, as for `Ui::allocate_exact_size`. The meter takes no
/// clicks or keyboard focus. Screen readers get a meter whose name is `label`, whose value is the
/// level in dB limited to the scale, and whose minimum and maximum are the scale's floor and
/// ceiling. `value_text` adds a spoken value such as a translated "-12.0 dB".
pub struct LevelMeter<'a> {
    label: &'a str,
    size: Vec2,
    style: &'a LevelMeterStyle,
    level_db: f32,
    rms_db: Option<f32>,
    hold_db: Option<f32>,
    orientation: MeterOrientation,
    value_text: Option<&'a str>,
}

impl<'a> LevelMeter<'a> {
    /// A meter showing silence until [`LevelMeter::level_db`] sets a level.
    pub fn new(label: &'a str, size: Vec2, style: &'a LevelMeterStyle) -> Self {
        Self {
            label,
            size,
            style,
            level_db: f32::NEG_INFINITY,
            rms_db: None,
            hold_db: None,
            orientation: MeterOrientation::Horizontal,
            value_text: None,
        }
    }

    /// The current level in dB; the bar is lit up to it.
    pub fn level_db(mut self, db: f32) -> Self {
        self.level_db = db;
        self
    }

    /// The RMS level in dB, drawn as a line with the style's `rms` stroke.
    pub fn rms_db(mut self, db: f32) -> Self {
        self.rms_db = Some(db);
        self
    }

    /// The held peak in dB, drawn as a line with the style's `hold` stroke.
    pub fn hold_db(mut self, db: f32) -> Self {
        self.hold_db = Some(db);
        self
    }

    pub fn orientation(mut self, orientation: MeterOrientation) -> Self {
        self.orientation = orientation;
        self
    }

    /// The value a screen reader speaks, for example "-12.0 dB" in the user's language.
    pub fn value_text(mut self, text: &'a str) -> Self {
        self.value_text = Some(text);
        self
    }

    /// Paint the meter and return its hover response.
    ///
    /// Non-finite, non-positive or excessively large dimensions (over 4096 points) produce an
    /// inert zero-size response without painting.
    pub fn show(self, ui: &mut Ui) -> Response {
        let valid = self.size.is_finite() && self.size.x > 0.0 && self.size.y > 0.0 && self.size.x <= 4096.0 && self.size.y <= 4096.0;
        let (rect, response) = ui.allocate_exact_size(if valid { self.size } else { Vec2::ZERO }, Sense::hover());
        let scale = self.style.scale;
        response.widget_info(|| {
            let mut info = egui::WidgetInfo::labeled(accesskit::Role::Meter, ui.is_enabled(), self.label);
            info.value = Some(accessible_value(&scale, self.level_db));
            info
        });
        ui.ctx().accesskit_node_builder(response.id, |node| {
            node.set_min_numeric_value(f64::from(scale.floor_db()));
            node.set_max_numeric_value(f64::from(scale.ceiling_db()));
            node.set_orientation(match self.orientation {
                MeterOrientation::Horizontal => accesskit::Orientation::Horizontal,
                MeterOrientation::Vertical => accesskit::Orientation::Vertical,
            });
            if let Some(text) = self.value_text.filter(|text| !text.is_empty()) {
                node.set_value(text);
            }
        });
        if !valid || !ui.is_rect_visible(rect) {
            return response;
        }

        let painter = ui.painter_at(rect);
        for (part, color) in segments(self.style, rect, self.orientation, self.level_db) {
            if color != Color32::TRANSPARENT {
                painter.rect_filled(part, 0.0, color);
            }
        }
        for (db, stroke) in [(self.rms_db, self.style.rms), (self.hold_db, self.style.hold)] {
            if let Some(db) = db
                && let Some(line) = mark(&scale, rect, self.orientation, db)
                && stroke.width.is_finite()
                && stroke.width > 0.0
            {
                painter.line_segment(line, stroke);
            }
        }
        response
    }
}

/// A level rounded to 0.1 dB, so a screen reader is not sent a new value for every small change,
/// and kept within the scale when the floor or ceiling is not a whole tenth.
fn accessible_value(scale: &MeterScale, db: f32) -> f64 {
    let rounded = (f64::from(scale.clamp(db)) * 10.0).round() / 10.0;
    rounded.clamp(f64::from(scale.floor_db()), f64::from(scale.ceiling_db()))
}

/// The part of `rect` between bar positions `from` and `to` (0 to 1).
fn span(rect: Rect, orientation: MeterOrientation, from: f32, to: f32) -> Rect {
    match orientation {
        MeterOrientation::Horizontal => {
            Rect::from_min_max(pos2(rect.left() + rect.width() * from, rect.top()), pos2(rect.left() + rect.width() * to, rect.bottom()))
        }
        MeterOrientation::Vertical => {
            Rect::from_min_max(pos2(rect.left(), rect.bottom() - rect.height() * to), pos2(rect.right(), rect.bottom() - rect.height() * from))
        }
    }
}

/// The rectangles a meter at `level_db` fills, with their colors, in painting order.
fn segments(style: &LevelMeterStyle, rect: Rect, orientation: MeterOrientation, level_db: f32) -> Vec<(Rect, Color32)> {
    let scale = style.scale;
    let high = scale.position(style.high_from_db);
    let mid = scale.position(style.mid_from_db).min(high);
    let lit = scale.position(level_db);
    let mut out = Vec::with_capacity(6);
    for (lo, hi, zone) in [(0.0, mid, style.low), (mid, high, style.mid), (high, 1.0, style.high)] {
        if hi <= lo {
            continue;
        }
        let top = lit.clamp(lo, hi);
        if top > lo {
            out.push((span(rect, orientation, lo, top), zone.lit));
        }
        if hi > top {
            out.push((span(rect, orientation, top, hi), zone.unlit));
        }
    }
    out
}

/// The line across the bar at `db`, or `None` at or below the floor.
fn mark(scale: &MeterScale, rect: Rect, orientation: MeterOrientation, db: f32) -> Option<[egui::Pos2; 2]> {
    let at = scale.position(db);
    if at <= 0.0 {
        return None;
    }
    let line = span(rect, orientation, at, at);
    Some(match orientation {
        MeterOrientation::Horizontal => [line.center_top(), line.center_bottom()],
        MeterOrientation::Vertical => [line.left_center(), line.right_center()],
    })
}

/// A held peak for one channel: it follows a rising level at once, stays for a hold time, then
/// falls at a fixed rate until it meets the level again.
///
/// Time comes from the caller in seconds (for example `ui.input(|i| i.time)`), so the hold does
/// not depend on the frame rate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PeakHold {
    hold_secs: f64,
    fall_db_per_sec: f64,
    /// The loudest level since the hold last restarted, and when it arrived.
    caught_db: f32,
    caught_at: f64,
    /// The time of the last update.
    last_at: f64,
    /// The value the last update returned.
    held_db: f32,
}

impl PeakHold {
    /// The loudest level [`PeakHold::update`] holds, in dB. A louder level, positive infinity
    /// included, is held as this value, so the peak can still fall. It is just below
    /// `amplitude_to_db(f32::MAX)`.
    pub const MAX_LEVEL_DB: f32 = 770.0;

    /// A hold of `hold_secs` seconds followed by a fall of `fall_db_per_sec` dB per second.
    ///
    /// A hold of 0 lets the peak start falling at once, and a fall of 0 keeps the peak until
    /// [`PeakHold::reset`]. Returns `None` when either value is negative or not finite.
    pub fn new(hold_secs: f32, fall_db_per_sec: f32) -> Option<Self> {
        let valid = |v: f32| v.is_finite() && v >= 0.0;
        (valid(hold_secs) && valid(fall_db_per_sec)).then_some(Self {
            hold_secs: f64::from(hold_secs),
            fall_db_per_sec: f64::from(fall_db_per_sec),
            caught_db: f32::NEG_INFINITY,
            caught_at: 0.0,
            last_at: f64::NEG_INFINITY,
            held_db: f32::NEG_INFINITY,
        })
    }

    /// Take the level at time `now` (seconds) and return the held peak in dB.
    ///
    /// A NaN level counts as silence, and a level above [`PeakHold::MAX_LEVEL_DB`] counts as that
    /// value. A time that is not finite leaves the hold unchanged. A time earlier than the last
    /// update's keeps the peak from falling and restarts the hold time from `now`; a louder level
    /// is still taken at once, so the peak is never below the level.
    pub fn update(&mut self, level_db: f32, now: f64) -> f32 {
        if !now.is_finite() {
            return self.held_db;
        }
        let level = if level_db.is_nan() { f32::NEG_INFINITY } else { level_db.min(Self::MAX_LEVEL_DB) };
        if now < self.last_at {
            self.caught_db = self.held_db;
            self.caught_at = now;
        }
        self.last_at = now;
        let held = self.held_at(now);
        if level >= held {
            self.caught_db = level;
            self.caught_at = now;
            self.held_db = level;
        } else {
            self.held_db = held;
        }
        self.held_db
    }

    /// The held peak returned by the last [`PeakHold::update`]; negative infinity before the
    /// first one and after [`PeakHold::reset`].
    pub fn peak_db(&self) -> f32 {
        self.held_db
    }

    /// Forget the held peak, as after a click on the meter or a change of source.
    pub fn reset(&mut self) {
        self.caught_db = f32::NEG_INFINITY;
        self.held_db = f32::NEG_INFINITY;
    }

    fn held_at(&self, now: f64) -> f32 {
        let falling = now - self.caught_at - self.hold_secs;
        if falling <= 0.0 || self.caught_db == f32::NEG_INFINITY {
            self.caught_db
        } else {
            (f64::from(self.caught_db) - falling * self.fall_db_per_sec) as f32
        }
    }
}

#[cfg(test)]
mod tests;
