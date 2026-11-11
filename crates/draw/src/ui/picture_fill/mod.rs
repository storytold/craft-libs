//! [`PictureFillEditor`]: PowerPoint's "Picture or texture fill" pane (DeckCraft): the picture's
//! source, picked through the host's files, then Stretch offsets or Tile offsets, scale,
//! alignment and mirror, Rotate with shape and Transparency.
//!
//! ```ignore
//! let out = PictureFillEditor::new("Picture", Bind::bound(&mut fill)).show(ui);
//! if let Some(request) = out.requests.take_path() { /* open the file dialog, store the picture, set `fill.source` */ }
//! ```

mod editor;
#[cfg(feature = "screen-testing")]
mod stories;
mod words;

pub use editor::{PICTURE_FILL_KEY, PictureFillEditor, PictureFillOutput};
pub use words::PictureFillWords;

/// Where a tiled picture's first tile sits in the shape.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TileAlignment {
    /// Top left.
    #[default]
    TopLeft,
    /// Top.
    Top,
    /// Top right.
    TopRight,
    /// Left.
    Left,
    /// Center.
    Center,
    /// Right.
    Right,
    /// Bottom left.
    BottomLeft,
    /// Bottom.
    Bottom,
    /// Bottom right.
    BottomRight,
}

/// How tiles flip from one to the next.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TileMirror {
    /// No mirroring.
    #[default]
    None,
    /// Every second tile in a row is flipped left to right.
    Horizontal,
    /// Every second tile in a column is flipped top to bottom.
    Vertical,
    /// Both.
    Both,
}

/// How the picture fills the shape.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum PictureMode {
    /// Stretched to the shape, inset by `offsets` (left, top, right, bottom) in percent of the
    /// shape (negative grows past the edge).
    #[non_exhaustive]
    Stretch {
        /// Inset from the left, top, right and bottom edges, in percent of the shape.
        offsets: [f64; 4],
    },
    /// Tiled: `offset` (x, y) in points, `scale` (x, y) in percent, the first tile's alignment and
    /// the mirror between tiles.
    #[non_exhaustive]
    Tile {
        /// Offset of the tiling, x and y in points.
        offset: [f64; 2],
        /// Scale of a tile, x and y in percent.
        scale: [f64; 2],
        /// Where the first tile sits.
        alignment: TileAlignment,
        /// Mirroring between tiles.
        mirror: TileMirror,
    },
}

impl PictureMode {
    /// Stretched to the shape, inset by `offsets` (left, top, right, bottom) in percent.
    pub fn stretch(offsets: [f64; 4]) -> Self {
        PictureMode::Stretch { offsets }
    }

    /// Tiled with `offset`, `scale`, first-tile `alignment` and `mirror`.
    pub fn tile(offset: [f64; 2], scale: [f64; 2], alignment: TileAlignment, mirror: TileMirror) -> Self {
        PictureMode::Tile { offset, scale, alignment, mirror }
    }

    /// The smallest tile scale in percent: DeckCraft's smallest step (its picture-fill fields show
    /// one decimal), so a scale is never zero or negative. There is no upper limit.
    pub const SCALE: std::ops::RangeInclusive<f64> = 0.1..=f64::INFINITY;
}

impl Default for PictureMode {
    /// Stretched with no offsets.
    fn default() -> Self {
        PictureMode::Stretch { offsets: [0.0; 4] }
    }
}

/// A picture fill's values; the picture itself is the app's.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct PictureFill {
    /// The picture's name as the source row shows it; `None` before one is picked.
    pub source: Option<String>,
    /// How the picture fills the shape.
    pub mode: PictureMode,
    /// The picture turns with the shape.
    pub rotate_with_shape: bool,
    /// 0 (opaque) to 1.
    pub transparency: f64,
}

impl PictureFill {
    /// A stretched fill of the picture `source`, turning with the shape, opaque.
    pub fn new(source: impl Into<String>) -> Self {
        PictureFill { source: Some(source.into()), mode: PictureMode::default(), rotate_with_shape: true, transparency: 0.0 }
    }
}
