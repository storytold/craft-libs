//! The tile grid family: the `Tile` item of a tile grid and the `ThumbnailSlider` that sizes the tiles.
//!
//! The data types of a tile (badges, status, marks, the caption's place) live here; `TileGrid` shows tiles in
//! rows and columns and reads the tile size from the value a `ThumbnailSlider` edits.

mod grid;
mod headers;
mod layout;
mod slider;
#[cfg(feature = "testing")]
mod stories;
mod tile;
mod words;

use crate::runtime::env::Label;
use craft_behavior::IconId;
use craft_theme::variant::Tone;

pub use grid::{GridEvent, RatingKeys, TileGrid, TileGridOutput, TileGridStyle};
pub use layout::{Around, ArrangeSpacing, GridLayout, TailRow, TileArrangement, TileGroup, TileRow, Tiling, arrange, aspects_of};
pub use slider::{ThumbnailSlider, ThumbnailSliderStyle, TileZoom, ZoomStep};
pub use tile::{Tile, TileOutput, TilePartEvent, TileStyle};
pub use words::TileWords;

/// Where a tile's caption goes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum CaptionPlace {
    /// Under the picture.
    #[default]
    Below,
    /// In a column at the picture's reading start, top aligned: the name, crossed by a slash on a hidden tile, and
    /// the tile's marks under it, one per line. The tile is that much wider than its picture.
    Start,
    /// No caption: the picture takes the whole tile.
    None,
}

/// A pick or reject flag.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TileFlag {
    /// Picked.
    Picked,
    /// Rejected.
    Rejected,
}

/// What a corner badge shows.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum BadgeKind {
    /// A glyph on a neutral pill.
    Glyph(IconId),
    /// A short text in the badge's tone ("Duplicate", "1/3").
    Text(Label),
}

/// A corner of the picture, logical: Start and End swap in a right-to-left interface.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum BadgeSlot {
    /// The reading start's top corner.
    StartTop,
    /// The reading end's top corner.
    EndTop,
    /// The reading start's bottom corner, above the badge bar when it shows.
    StartBottom,
    /// The reading end's bottom corner.
    EndBottom,
}

/// A corner badge. Badges of one slot stack toward the middle of the picture and never lie over the check box.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct TileBadge {
    /// The corner.
    pub slot: BadgeSlot,
    /// What the badge shows.
    pub kind: BadgeKind,
    /// The tone of a text badge.
    pub tone: Tone,
    /// The tooltip, also a part of the tile's accessible name.
    pub tip: Option<Label>,
}

impl TileBadge {
    /// A badge of `kind` in `slot`, in `tone`, with the tooltip `tip`.
    pub fn new(slot: BadgeSlot, kind: BadgeKind, tone: Tone, tip: Option<Label>) -> Self {
        TileBadge { slot, kind, tone, tip }
    }
}

/// A glyph under a caption at the reading start (an animation star on a slide).
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct TileMark {
    /// The glyph.
    pub icon: IconId,
    /// Its tooltip, also a part of the tile's accessible name.
    pub tip: Label,
}

impl TileMark {
    /// A mark showing `icon` with the tooltip `tip`.
    pub fn new(icon: IconId, tip: impl Into<Label>) -> Self {
        TileMark { icon, tip: tip.into() }
    }
}

/// A tile's file status.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub enum TileStatus {
    /// Nothing to say.
    #[default]
    Normal,
    /// The media is offline: a band along the picture's bottom with the text, or the word "Offline" for `None`, and
    /// the name in the failure colour.
    Offline(Option<Label>),
    /// The file can't be read: a failure badge.
    Unreadable,
    /// The picture failed: a failure badge; the thumbnail painter shows the failure.
    Failed,
}

/// When the badge bar (rating, flag, colour dot) shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum BadgeVisibility {
    /// While the pointer is over the tile, or when a rating, flag or label is set.
    #[default]
    Auto,
    /// Always.
    Always,
    /// Never. Corner badges still show.
    Never,
}
