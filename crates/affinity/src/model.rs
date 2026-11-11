//! The document model the Craft apps import: spreads, artboards, layers, groups, curves, shapes,
//! text and images, with transforms composed into document space. Anything else is reported
//! by [`Document::warnings`] instead of being dropped silently.

use std::collections::{BTreeMap, HashSet};

use crate::paint::{self, Paint, Stroke};
use crate::stream::{ObjId, Stream, Tag};
use crate::{Archive, Error, Limits, stream};

mod bounds;
mod read;

pub(crate) use bounds::rect;
pub use bounds::{kind_bounds, nodes_bounds};
pub(crate) use read::Reader;
pub use read::read;
#[cfg(test)]
pub(crate) use read::read_within;

/// Affine map `x' = a·x + c·y + e`, `y' = b·x + d·y + f` (kurbo's coefficient order).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Affine(pub [f64; 6]);

impl Affine {
    pub const IDENTITY: Self = Self([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);

    /// Affinity's `Xfrm`, stored row by row: (a, c, e, b, d, f).
    pub(crate) fn from_xfrm(m: [f64; 6]) -> Self {
        Self([m[0], m[3], m[1], m[4], m[2], m[5]])
    }

    pub fn then(self, outer: Self) -> Self {
        let [a, b, c, d, e, f] = self.0;
        let [oa, ob, oc, od, oe, of] = outer.0;
        Self([oa * a + oc * b, ob * a + od * b, oa * c + oc * d, ob * c + od * d, oa * e + oc * f + oe, ob * e + od * f + of])
    }

    pub fn apply(&self, p: Point) -> Point {
        let [a, b, c, d, e, f] = self.0;
        Point { x: a * p.x + c * p.y + e, y: b * p.x + d * p.y + f }
    }

    /// Geometric mean of the axis scales: how much a stroke width or font size grows.
    pub fn scale(&self) -> f64 {
        let [a, b, c, d, ..] = self.0;
        (a * d - b * c).abs().sqrt()
    }

    pub fn is_finite(&self) -> bool {
        self.0.iter().all(|v| v.is_finite())
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

/// One cubic Bézier subpath, in document pixels.
#[derive(Debug, Clone, PartialEq)]
pub struct SubPath {
    pub start: Point,
    /// (control 1, control 2, end) per segment.
    pub segments: Vec<[Point; 3]>,
    pub closed: bool,
}

/// An outline in document pixels.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Path {
    pub subpaths: Vec<SubPath>,
}

impl Path {
    pub fn transformed(&self, m: Affine) -> Self {
        Self {
            subpaths: self
                .subpaths
                .iter()
                .map(|s| SubPath {
                    start: m.apply(s.start),
                    segments: s.segments.iter().map(|seg| seg.map(|p| m.apply(p))).collect(),
                    closed: s.closed,
                })
                .collect(),
        }
    }
}

/// Affinity's blend modes, numbered by the `Blnd` enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Blend {
    #[default]
    Normal,
    PassThrough,
    Darken,
    DarkerColor,
    Multiply,
    ColorBurn,
    Lighten,
    LighterColor,
    Screen,
    ColorDodge,
    Add,
    Overlay,
    SoftLight,
    HardLight,
    VividLight,
    PinLight,
    LinearLight,
    HardMix,
    Difference,
    Exclusion,
    Subtract,
    Hue,
    Saturation,
    Luminosity,
    Color,
    Average,
    Negation,
    Reflect,
    Glow,
    Erase,
}

impl Blend {
    /// The enum (id, version) pairs observed in a document whose layers are named after their mode.
    fn from_enum(id: u16, version: u16) -> Option<Self> {
        use Blend::*;
        Some(match (id, version) {
            (0, _) => Normal,
            (1, 0) => Darken,
            (2, 1) => DarkerColor,
            (2, 0) => Multiply,
            (3, 0) => ColorBurn,
            (4, 0) => Lighten,
            (6, 1) => LighterColor,
            (5, 0) => Screen,
            (6, 0) => ColorDodge,
            (7, 0) => Add,
            (8, 0) => Overlay,
            (9, 0) => SoftLight,
            (10, 0) => HardLight,
            (11, 0) => VividLight,
            (12, 0) => PinLight,
            (15, 1) => LinearLight,
            (13, 0) => HardMix,
            (14, 0) => Difference,
            (15, 0) => Exclusion,
            (16, 0) => Subtract,
            (17, 0) => Hue,
            (18, 0) => Saturation,
            (19, 0) => Luminosity,
            (20, 0) => Color,
            (21, 0) => Average,
            (22, 0) => Negation,
            (23, 0) => Reflect,
            (24, 0) => Glow,
            (25, 0) => Erase,
            _ => return None,
        })
    }
}

/// A run of text with one set of character attributes.
#[derive(Debug, Clone, PartialEq)]
pub struct TextRun {
    pub text: String,
    /// PostScript name and family of the chosen font.
    pub postscript: String,
    pub family: String,
    /// CSS weight (400 regular, 700 bold).
    pub weight: i64,
    pub italic: bool,
    /// Boolean OpenType feature overrides, as tags or `-tag` for disabled features.
    pub features: Vec<String>,
    pub all_caps: bool,
    /// Underlined characters (`Ints` slot 0 is 1; identified on underlined words in a document its
    /// owner made, all other characters there and in the public corpus have 0).
    pub underline: bool,
    /// Struck-through characters (`Ints` slot 1 is 1; identified the same way, on prices struck
    /// through in another document its owner made).
    pub strikethrough: bool,
    /// Font size in document pixels, before the node's transform.
    pub size: f64,
    /// Tracking in em.
    pub tracking: f64,
    /// Horizontal scale of the glyphs (1 = as designed).
    pub h_scale: f64,
    /// Fixed line pitch in document pixels, when the run overrides automatic leading.
    pub leading: Option<f64>,
    pub fill: Paint,
    /// Outlined characters: the outline's paint and width (node space).
    pub stroke: Option<crate::paint::Stroke>,
    /// The run's paragraph: alignment, indents and spacing.
    pub paragraph: Paragraph,
}

/// Paragraph alignment, indents, spacing and leading, in node space (document pixels before the
/// node's text scale).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Paragraph {
    pub align: Align,
    /// Indent of every line but the first, from the left.
    pub left: f64,
    pub right: f64,
    /// Indent of the first line, from the left.
    pub first: f64,
    pub space_before: f64,
    pub space_after: f64,
    /// Fixed distance between baselines, when the paragraph sets one.
    pub leading: Option<f64>,
    /// Automatic leading as a multiple of the font's own line height (ascent plus descent).
    pub auto_leading: f64,
}

impl Default for Paragraph {
    fn default() -> Self {
        Self { align: Align::Left, left: 0.0, right: 0.0, first: 0.0, space_before: 0.0, space_after: 0.0, leading: None, auto_leading: 1.0 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
    Justify,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Text {
    pub runs: Vec<TextRun>,
    pub align: Align,
    /// Artistic text: the first baseline's anchor point in node space (start, centre or end of the
    /// line according to `align`). Frame text: the frame box in node space.
    pub anchor: Point,
    pub frame: Option<Rect>,
    /// Node space to document pixels.
    pub transform: Affine,
    /// Node space to document pixels for the type (its linear part matters): artistic text scales
    /// with its node, frame text only with its frame's text scale.
    pub text_transform: Affine,
}

/// Vertical alignment of a table cell's text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VAlign {
    Top,
    Center,
    Bottom,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TableCell {
    pub runs: Vec<TextRun>,
    /// Space between the cell edges and its text, in node space: left, top, right, bottom.
    pub inset: [f64; 4],
    pub valign: VAlign,
    /// Part of the cell to its left (merged): it has no text of its own.
    pub merged_left: bool,
}

/// A table: a grid of cells with text, and a line on every cell edge.
#[derive(Debug, Clone, PartialEq)]
pub struct Table {
    /// Column boundaries (one more than the columns), increasing, in node space.
    pub columns: Vec<f64>,
    /// Row boundaries (one more than the rows), increasing, in node space.
    pub rows: Vec<f64>,
    /// Row by row.
    pub cells: Vec<TableCell>,
    /// Vertical edges row by row, one more per row than the columns; `None` draws no line, and
    /// the list is empty when the file's didn't match the grid.
    pub vertical: Vec<Option<Stroke>>,
    /// Horizontal edges boundary by boundary (one more than the rows), one per column (empty
    /// as `vertical` can be).
    pub horizontal: Vec<Option<Stroke>>,
    /// Node space to document pixels.
    pub transform: Affine,
    /// Node space to document pixels for the cells' type: resizing a table moves its grid, not
    /// its type.
    pub text_transform: Affine,
}

impl Table {
    pub fn ncols(&self) -> usize {
        self.columns.len().saturating_sub(1)
    }
    pub fn nrows(&self) -> usize {
        self.rows.len().saturating_sub(1)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Pixels {
    /// Straight-alpha RGBA, 8 bits per channel, row by row.
    Rgba8(Vec<u8>),
    /// A CMYK pixel layer: cyan, magenta, yellow and black ink amounts (0 none, 255 full) and
    /// straight alpha, 8 bits each, row by row. Left in CMYK for the importer to show through a
    /// colour profile: the profile-free formula turns the extra cyan of a separation's neutrals
    /// into a blue cast.
    Cmyka8(Vec<u8>),
    /// The original file a placed image was made from (PNG, JPEG…), as embedded.
    Encoded(Vec<u8>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub pixels: Pixels,
    /// Pixel space (0..w, 0..h) to document pixels.
    pub transform: Affine,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    /// Layer container (`Scop`).
    Layer,
    Group,
    /// Artboard: an export rectangle and vector outline that clips its children.
    /// `background` paints behind them, and `strokes` over them.
    Artboard {
        rect: Rect,
        /// Actual clipping outline in document pixels. The artboard's export rectangle is its
        /// bounding box, but a rotated or non-rectangular board still clips to this outline.
        path: Path,
        even_odd: bool,
        background: Vec<Paint>,
        strokes: Vec<Stroke>,
    },
    /// Curve, parametric shape or compound shape: an outline with its fills and strokes.
    /// Children of a shape are clipped by its outline (painted over its fills, under its strokes).
    Shape {
        path: Path,
        fills: Vec<Paint>,
        strokes: Vec<Stroke>,
        even_odd: bool,
    },
    Text(Text),
    Table(Table),
    Image(Image),
    /// A master page shown on this spread's pages: pages `page..page + count` show the master
    /// spread [`Document::masters`]`[master]`'s pages from `master_page` on.
    MasterInstance {
        master: usize,
        page: usize,
        master_page: usize,
        count: usize,
    },
    /// Something this reader does not import; its children are still imported.
    Unsupported,
}

/// A layer effect. Lengths are document pixels; `angle` is the direction the shadow is offset in
/// (radians, clockwise from +x as y points down: π/2 puts it below).
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    DropShadow {
        color: crate::paint::Color,
        opacity: f64,
        radius: f64,
        distance: f64,
        angle: f64,
    },
    InnerShadow {
        color: crate::paint::Color,
        opacity: f64,
        radius: f64,
        distance: f64,
        angle: f64,
    },
    OuterGlow {
        color: crate::paint::Color,
        opacity: f64,
        radius: f64,
    },
    /// An outline of `width` around the node's shape: `align` 0 outside, 1 inside, 2 centred.
    Outline {
        color: crate::paint::Color,
        opacity: f64,
        width: f64,
        align: u16,
    },
    /// Bevel and emboss: lit and shaded edges `size` wide. `angle` is the light's azimuth
    /// (radians, counter-clockwise from +x with y up), `depth` Affinity's depth (5 by default);
    /// `inner` is false for the outer, emboss and pillow kinds.
    Bevel {
        size: f64,
        depth: f64,
        angle: f64,
        highlight: crate::paint::Color,
        highlight_opacity: f64,
        shadow: crate::paint::Color,
        shadow_opacity: f64,
        inner: bool,
    },
    /// Colour overlay: everything the node paints takes `color`, laid over it in `blend` mode at
    /// `opacity` (the node's own alpha is kept).
    ColorOverlay {
        color: crate::paint::Color,
        opacity: f64,
        blend: Blend,
    },
    /// Gradient overlay: a colour overlay whose colour comes from `gradient` (document pixels).
    GradientOverlay {
        gradient: crate::paint::Gradient,
        opacity: f64,
        blend: Blend,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub class: Tag,
    pub name: String,
    pub visible: bool,
    pub locked: bool,
    pub opacity: f64,
    pub blend: Blend,
    pub kind: Kind,
    /// Vector mask in document pixels: the node shows only inside it (nonzero fill).
    pub mask: Option<Path>,
    /// Pixel mask: grey levels (white shows the node, black hides it); outside it the node is hidden.
    pub pixel_mask: Option<Image>,
    /// Layer effects the importer can show or bake in (others are reported).
    pub effects: Vec<Effect>,
    /// Bottom to top.
    pub children: Vec<Node>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Spread {
    /// Spread bounds in document pixels (the canvas, or the union of the artboards).
    pub bounds: Rect,
    /// Pages of a Publisher-style spread (one, or two facing pages), in document pixels.
    pub pages: Vec<Rect>,
    /// Transparent background; otherwise the page is white.
    pub transparent: bool,
    /// Bottom to top.
    pub nodes: Vec<Node>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Document {
    /// Pixels per inch: all geometry is in document pixels at this resolution.
    pub dpi: f64,
    pub spreads: Vec<Spread>,
    /// Master spreads, which [`Kind::MasterInstance`] nodes show on pages.
    pub masters: Vec<Spread>,
    /// Application that last saved the file, as stored (`Affinity 3.0.2`), when present.
    pub saved_by: Option<String>,
    /// Content that was not imported or was approximated, one line per kind, deduplicated.
    pub warnings: Vec<String>,
    /// The document's CMYK colour profile (an ICC file), when it embeds one: the root's `CMYP`,
    /// which each CMYK pixel layer's colour settings (`Prof`) also name, with relative
    /// colorimetric intent and black-point compensation in every document checked.
    pub cmyk_profile: Option<Vec<u8>>,
}
