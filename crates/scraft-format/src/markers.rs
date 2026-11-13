//! Memory locations (markers and stored selections) and mix/edit groups.

use crate::soundcraft_time::Samples;
use crate::{GroupId, MarkerId, TrackId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum MarkerKind {
    /// A point marker on a ruler.
    #[default]
    Marker,
    /// A stored selection range.
    Selection,
    /// A location that only recalls view settings.
    None,
}

/// A memory location (Window › Memory Locations).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MemoryLocation {
    pub id: MarkerId,
    /// User-visible number (1-based, unique).
    pub number: u32,
    pub name: String,
    pub kind: MarkerKind,
    pub start: Samples,
    pub end: Samples,
    #[serde(default)]
    pub comments: String,
    #[serde(default)]
    pub color: Option<[u8; 3]>,
    /// Marker ruler lane 1..5.
    #[serde(default = "one")]
    pub ruler: u8,
    /// Track-lane marker on this track, if any.
    #[serde(default)]
    pub track: Option<TrackId>,
}

fn one() -> u8 {
    1
}

/// A track group (Track › Group…).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Group {
    pub id: GroupId,
    pub name: String,
    /// Group letter id shown in the Groups list ("a", "b", …).
    pub letter: char,
    pub members: Vec<TrackId>,
    pub edit: bool,
    pub mix: bool,
    pub active: bool,
    pub color: [u8; 3],
    /// Shared attributes linked by the group (volume, mute, solo, rec, etc.).
    #[serde(default)]
    pub attributes: Vec<String>,
}
