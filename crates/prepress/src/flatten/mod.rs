//! The display list the transparency flattener reads and writes.
//!
//! Coordinates are page space in points, origin at the top left and y down, as krilla draws.
//! Opacity, blend modes, soft masks and clips live on [`GroupItem`]s only: a transparent or
//! clipped paint arrives as a group with one child. Strokes arrive outlined as fills.
//! Overprint stays a per-item attribute; the flattener applies it per plate.

use std::fmt;
use std::ops::Range;
use std::sync::Arc;

pub use kurbo;
use kurbo::{Affine, BezPath, Point, Rect};

use crate::ink::{InkColor, InkId, InkSet, Overprint};

pub use compose::{ComplexGroup, ComplexReason, ComplexRegion, FlattenError, FlattenOptions, FlattenReport, Flattened, flatten};
pub use document::{
    FlattenDocumentError, FlattenDocumentOptions, FlattenDocumentReport, FlattenTotals, Unreadable, flatten_document, flatten_document_with_progress,
};
pub use geom::FillRule;
pub use raster::{FlattenPageError, FlattenPageOptions, FlattenPageReport, flatten_page};

mod compose;
mod document;
pub mod font;
pub mod geom;
pub mod pdf;
pub mod raster;
mod shade;
pub mod write;

/// One page's content as the flattener sees it.
#[derive(Clone, Debug, PartialEq)]
pub struct DisplayList {
    /// The page box in page space.
    pub page: Rect,
    /// The inks the items' colors refer to.
    pub inks: InkSet,
    /// Items in painting order.
    pub items: Vec<Item>,
}

/// A painted element.
#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    Fill(FillItem),
    Text(TextItem),
    Image(ImageItem),
    Group(GroupItem),
}

/// A filled path.
#[derive(Clone, Debug, PartialEq)]
pub struct FillItem {
    pub path: BezPath,
    pub rule: FillRule,
    pub paint: Paint,
    pub overprint: Overprint,
}

/// A run of glyphs from one font at one size and transform, kept as live text when
/// written out.
#[derive(Clone, Debug, PartialEq)]
pub struct TextItem {
    /// The font to embed again; `None` when the source font can't be re-embedded, and the
    /// run is then written as `outlines`.
    pub font: Option<Arc<TextFont>>,
    /// The em size in text space units.
    pub size: f32,
    /// Text space to page space. Text space is y up, as in a PDF text matrix.
    pub transform: Affine,
    pub glyphs: Vec<PlacedGlyph>,
    /// The Unicode text of the run, when the source had it; [`PlacedGlyph::text`] indexes it.
    pub text: Option<String>,
    /// The glyphs' filled outlines in page space. Always present: the flattener's geometry for
    /// the run, and what is drawn when `font` is `None`.
    pub outlines: BezPath,
    pub paint: Paint,
    pub overprint: Overprint,
}

/// A glyph of a [`TextItem`].
#[derive(Clone, Debug, PartialEq)]
pub struct PlacedGlyph {
    /// The glyph id in [`TextFont::data`].
    pub id: u32,
    /// The glyph origin in text space.
    pub origin: Point,
    /// The bytes of [`TextItem::text`] this glyph shows; empty when it shows none.
    pub text: Range<usize>,
}

/// Font bytes shared without copying, as hayro-interpret and krilla hold them.
pub type FontBytes = Arc<dyn AsRef<[u8]> + Send + Sync>;

/// An OpenType (TrueType or CFF outlines) font that krilla can embed.
#[derive(Clone)]
pub struct TextFont {
    pub data: FontBytes,
    /// The face index in a font collection; 0 for a single font.
    pub index: u32,
    /// Identifies the source font: runs with equal keys embed one font, so equal keys mean
    /// equal bytes.
    pub key: u128,
}

impl fmt::Debug for TextFont {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TextFont")
            .field("bytes", &(*self.data).as_ref().len())
            .field("index", &self.index)
            .field("key", &format_args!("{:032x}", self.key))
            .finish()
    }
}

impl PartialEq for TextFont {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key && self.index == other.index
    }
}

/// A sampled image, 8 bits per component, rows top to bottom.
#[derive(Clone, Debug, PartialEq)]
pub struct ImageItem {
    pub space: ImageSpace,
    /// Width and height in samples.
    pub size: (u32, u32),
    /// Interleaved components, `size.0 * size.1 * components` bytes.
    pub samples: Arc<[u8]>,
    /// The unit square to page space; (0, 0) is the top left sample.
    pub transform: Affine,
    /// Alpha, one byte per sample, at `size`.
    pub smask: Option<Arc<[u8]>>,
    pub overprint: Overprint,
    /// The source's JPEG (DCTDecode) data, when `samples` are its decoding and the writer can
    /// write it as it is: [`ImageSpace::Gray`] or [`ImageSpace::Cmyk`], no `smask`.
    pub jpeg: Option<Arc<[u8]>>,
}

/// The components of an [`ImageItem`]'s samples.
#[derive(Clone, Debug, PartialEq)]
pub enum ImageSpace {
    Gray,
    Rgb,
    Cmyk,
    /// Tints of these inks, 255 = 100% ink, in this order.
    Inks(Vec<InkId>),
}

/// A transparency group, or a plain clip when it is opaque, Normal and unmasked.
#[derive(Clone, Debug, PartialEq)]
pub struct GroupItem {
    pub opacity: f32,
    pub blend: BlendMode,
    pub isolated: bool,
    pub knockout: bool,
    /// Nonzero clip in page space.
    pub clip: Option<BezPath>,
    pub mask: Option<Box<Mask>>,
    /// The Unicode text the children show, for content drawn as items that is text in the
    /// source, such as a colored Type 3 glyph.
    pub actual_text: Option<String>,
    pub children: Vec<Item>,
}

/// A soft mask (ISO 32000-2 §11.6.5).
#[derive(Clone, Debug, PartialEq)]
pub struct Mask {
    pub kind: MaskKind,
    pub group: GroupItem,
    /// The luminosity mask's backdrop (`/BC`), what the group composites over; `None` for an
    /// alpha mask.
    pub backdrop: Option<InkColor>,
    /// The transfer function (`/TR`) sampled at 256 evenly spaced mask values from 0 to 1, as
    /// bytes; `None` for the identity.
    pub transfer: Option<Arc<[u8]>>,
}

/// Which value of the mask group masks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaskKind {
    Alpha,
    Luminosity,
}

/// What an item paints with.
#[derive(Clone, Debug, PartialEq)]
pub enum Paint {
    Solid(InkColor),
    /// Stops are offsets `0.0..=1.0` in increasing order. Function-based and mesh shadings
    /// arrive as images.
    Gradient {
        stops: Vec<(f32, InkColor)>,
        geometry: GradientGeometry,
    },
}

/// Where a gradient runs, in gradient space.
#[derive(Clone, Debug, PartialEq)]
pub enum GradientGeometry {
    Axial { start: Point, end: Point, extend: [bool; 2], transform: Affine },
    Radial { start: Point, start_radius: f64, end: Point, end_radius: f64, extend: [bool; 2], transform: Affine },
}

/// PDF blend modes (ISO 32000-2 §11.3.5).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BlendMode {
    #[default]
    Normal,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    HardLight,
    SoftLight,
    Difference,
    Exclusion,
    Hue,
    Saturation,
    Color,
    Luminosity,
}

impl BlendMode {
    /// Hue, Saturation, Color and Luminosity; on spot inks they act as Normal
    /// (ISO 32000-1 §11.3.5.3).
    pub fn is_non_separable(self) -> bool {
        matches!(self, Self::Hue | Self::Saturation | Self::Color | Self::Luminosity)
    }
}
