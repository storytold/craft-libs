//! The Brush Preset picker's parts and persisted view ([`BrushPickerSettings`]).

use std::ops::RangeInclusive;

use serde::{Deserialize, Serialize};

use super::picker::BrushPickerState;

/// One part of a brush preset as the picker shows it, as the gear menu names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum BrushCardPart {
    /// The preset's name ("Brush Name").
    Name,
    /// The stroke preview ("Brush Stroke").
    Stroke,
    /// The tip thumbnail with its size ("Brush Tip").
    Tip,
}

/// Which parts the picker shows. At least one is always on: [`set`](Self::set) refuses to clear the last one and
/// [`normalised`](Self::normalised) turns all three on when none is. All on by default.
///
/// - The tip alone is the browser's grid of thumbnails with the size as each caption.
/// - The name or the stroke is the browser's list: the stroke as the row's picture while it is on, else the tip.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
#[non_exhaustive]
pub struct BrushCardParts {
    /// The name ("Brush Name").
    pub name: bool,
    /// The stroke preview ("Brush Stroke").
    pub stroke: bool,
    /// The tip thumbnail ("Brush Tip").
    pub tip: bool,
}

impl Default for BrushCardParts {
    fn default() -> Self {
        Self::ALL
    }
}

impl BrushCardParts {
    /// Every part on.
    pub const ALL: BrushCardParts = BrushCardParts { name: true, stroke: true, tip: true };

    /// Whether `part` shows.
    pub fn get(self, part: BrushCardPart) -> bool {
        match part {
            BrushCardPart::Name => self.name,
            BrushCardPart::Stroke => self.stroke,
            BrushCardPart::Tip => self.tip,
        }
    }

    /// `part` turned `on`, unless that would clear the last part (the picker would show no brush): then unchanged.
    pub fn set(self, part: BrushCardPart, on: bool) -> Self {
        let mut next = self;
        match part {
            BrushCardPart::Name => next.name = on,
            BrushCardPart::Stroke => next.stroke = on,
            BrushCardPart::Tip => next.tip = on,
        }
        if next.name || next.stroke || next.tip { next } else { self }
    }

    /// These parts, or all three when none is on (a loaded setting, a host's own value).
    pub fn normalised(self) -> Self {
        if self.name || self.stroke || self.tip { self } else { Self::ALL }
    }

    /// The tip alone: the grid of thumbnails.
    pub fn tip_only(self) -> bool {
        !self.name && !self.stroke && self.tip
    }
}

/// What the picker remembers across restarts: which parts show and the thumbnail scale. The app saves it in its
/// settings (serde) when [`BrushPresetPickerOutput::settings`](super::BrushPresetPickerOutput::settings) is set and
/// restores it with [`BrushPickerState::restore`]; the search, open groups and an open rename are not saved.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
#[non_exhaustive]
pub struct BrushPickerSettings {
    /// Which parts show.
    pub parts: BrushCardParts,
    /// The thumbnail scale.
    pub scale: f32,
}

impl Default for BrushPickerSettings {
    fn default() -> Self {
        Self { parts: BrushCardParts::ALL, scale: 1.0 }
    }
}

impl BrushPickerSettings {
    /// The footer slider's range: under 0.15 the thumbnails and rows stop making sense.
    pub const SCALE_RANGE: RangeInclusive<f32> = 0.15..=2.0;

    /// The settings with the given parts and scale.
    pub const fn new(parts: BrushCardParts, scale: f32) -> Self {
        Self { parts, scale }
    }

    /// The settings made safe to show: the scale clamped to [`SCALE_RANGE`](Self::SCALE_RANGE) (one that isn't a
    /// finite number gives 1), and every part on when none is.
    pub fn restored(self) -> Self {
        let scale = if self.scale.is_finite() { self.scale.clamp(*Self::SCALE_RANGE.start(), *Self::SCALE_RANGE.end()) } else { 1.0 };
        Self { parts: self.parts.normalised(), scale }
    }
}

impl<K> BrushPickerState<K> {
    /// The part of this state that persists.
    pub fn settings(&self) -> BrushPickerSettings {
        BrushPickerSettings { parts: self.parts, scale: self.browser.zoom as f32 }
    }

    /// Sets the parts and scale from `settings`, [`restored`](BrushPickerSettings::restored) first.
    pub fn restore(&mut self, settings: &BrushPickerSettings) {
        let BrushPickerSettings { parts, scale } = settings.restored();
        self.parts = parts;
        self.browser.zoom = f64::from(scale);
    }
}
