//! Timeline markers: named flags, with a duration for range markers, a comment and a colour
//! label. Sequence markers sit in the ruler, clip and layer markers on their bars.

use crate::time::{Tick, TimeRange};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
/// Identifies a marker within its owner (sequence, clip or layer).
pub struct MarkerId(pub u64);

/// A colour label by index into the app's label palette: Premiere's sixteen labels (Violet, Iris,
/// Caribbean, Lavender, Cerulean, Forest, Rose, Mango, Purple, Blue, Teal, Magenta, Tan, Green,
/// Brown, Yellow), which the user can rename and recolour (FilmCraft `crates/project/src/label.rs:5`).
/// Tracks and clips use the same labels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct MarkerColor(pub u8);

/// A marker.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Marker {
    /// The marker's identity.
    pub id: MarkerId,
    /// Where the marker starts.
    pub start: Tick,
    /// Zero for a point marker.
    pub duration: Tick,
    /// The marker's name.
    pub name: String,
    /// A free-text comment.
    pub comment: String,
    /// The marker's colour label.
    pub color: MarkerColor,
}

impl Marker {
    /// The marker's time range; empty for a point marker.
    pub fn range(&self) -> TimeRange {
        TimeRange::new(self.start, self.duration)
    }

    /// A range marker (it has a duration).
    pub fn is_range(&self) -> bool {
        self.duration > Tick::ZERO
    }
}
