//! A drawing above the tag level: what a DXF file says, typed.
//!
//! [`Document::read`] indexes a tag list (from [`crate::parse`]) into its header variables,
//! symbol tables, blocks, entities and objects. Nothing is copied: every record borrows its tags,
//! so a caller can still read a group this module gives no name to (its own extended data, a
//! newer group). The typed forms ([`Layer`], [`Linetype`], [`TextStyle`], [`Layout`], and
//! [`Entity`] for an entity record) are made on request.
//!
//! The values are the file's: points in the coordinate system the format stores them in (the
//! object coordinate system for planar entities, see [`ocs_to_wcs`]), angles in the unit the
//! group holds (degrees, except where a field says radians), names as written. Turning that into
//! an application's document (resolving ByLayer, flattening, choosing fonts) is the caller's.
//!
//! Merged from the readers of two applications: CADCraft's (`cadcraft-io`, `dxf_read`) and
//! VectorCraft's (`vectorcraft-cad`, `import`). Written from Autodesk's published DXF Reference.

use crate::Tag;

pub mod aci;
mod document;
mod entity;
mod hatch;
pub mod mleader;
pub mod objects;
pub mod table;
mod tables;
pub mod text;
pub mod write;

pub use document::{Block, Dictionary, Document, EntityRecord, Header, Layout, Record, entity_records, records};
pub use entity::{
    Attrib, Common, Dimension, Entity, EntityKind, Helix, Image, Insert, Leader, LwPolyline, LwVertex, MLine, MLineVertex, Polyline, Spline, Text,
    Vertex, Viewport,
};
pub use hatch::{Gradient, Hatch, HatchEdge, HatchLoop, HatchPath, PatternLine};
pub use tables::{AppId, BlockRecord, DashElement, DimStyle, Layer, Linetype, TextStyle, Ucs, View, Vport, dim_var_code, dim_var_name};
pub use text::{MText, MTextColumns};

/// A point or vector: x, y, z.
pub type Point = [f64; 3];

/// A handle (the hexadecimal of groups 5, 105 and 320–369), or `None` for text that is not one.
/// Handle 0 is "no object" and is returned as `Some(0)`.
pub fn parse_handle(s: &str) -> Option<u64> {
    u64::from_str_radix(s.trim(), 16).ok()
}

/// The groups of one record, read by code.
///
/// A missing number is `None` (or the default given); a value that is not a finite number reads
/// as 0, as [`Tag::f64`] gives it.
#[derive(Clone, Copy, Debug)]
pub struct Groups<'a>(pub &'a [Tag]);

impl<'a> Groups<'a> {
    fn first(&self, code: i32) -> Option<&'a Tag> {
        self.0.iter().find(|t| t.code == code)
    }
    /// The first `code` as a number.
    pub fn f(&self, code: i32) -> Option<f64> {
        self.first(code).map(Tag::f64)
    }
    pub fn f_or(&self, code: i32, default: f64) -> f64 {
        self.f(code).unwrap_or(default)
    }
    /// The first `code` as a whole number.
    pub fn i(&self, code: i32) -> Option<i64> {
        self.first(code).map(Tag::i64)
    }
    pub fn i_or(&self, code: i32, default: i64) -> i64 {
        self.i(code).unwrap_or(default)
    }
    /// The first `code` as text, as written.
    pub fn s(&self, code: i32) -> Option<String> {
        self.first(code).map(Tag::str)
    }
    /// The first `code` as a name: trimmed, and `None` when empty.
    pub fn name(&self, code: i32) -> Option<String> {
        self.first(code).map(|t| t.str().trim().to_string()).filter(|s| !s.is_empty())
    }
    /// The first `code` as a handle.
    pub fn handle(&self, code: i32) -> Option<u64> {
        self.first(code).and_then(|t| parse_handle(&t.str()))
    }
    /// The point at `code` (x), `code + 10` (y) and `code + 20` (z); 0 for what is missing.
    pub fn point(&self, code: i32) -> Point {
        [self.f_or(code, 0.0), self.f_or(code + 10, 0.0), self.f_or(code + 20, 0.0)]
    }
    /// [`Groups::point`] when its x is given.
    pub fn point_opt(&self, code: i32) -> Option<Point> {
        self.f(code).map(|_| self.point(code))
    }
    /// Every `code` as a number, in order.
    pub fn all_f(&self, code: i32) -> Vec<f64> {
        self.0.iter().filter(|t| t.code == code).map(Tag::f64).collect()
    }
    /// The points a record repeats: each `code` (x) starts one, and the `code + 10` (y) and
    /// `code + 20` (z) before the next x belong to it. A point that leaves its y or z out has 0
    /// there, and does not take the next point's.
    pub fn points(&self, code: i32) -> Vec<Point> {
        let mut out: Vec<Point> = Vec::new();
        for t in self.0 {
            if t.code == code {
                out.push([t.f64(), 0.0, 0.0]);
            } else if let Some(p) = out.last_mut() {
                if t.code == code + 10 {
                    p[1] = t.f64();
                } else if t.code == code + 20 {
                    p[2] = t.f64();
                }
            }
        }
        out
    }
    /// The groups after the subclass marker `100 <marker>`, up to the next marker; empty when the
    /// record has no such marker (R12 files have none: read the whole record there).
    pub fn subclass(&self, marker: &str) -> Groups<'a> {
        let Some(at) = self.0.iter().position(|t| t.code == 100 && t.str_ref().is_some_and(|s| s.trim() == marker)) else {
            return Groups(&[]);
        };
        let rest = self.0.get(at + 1..).unwrap_or(&[]);
        let end = rest.iter().position(|t| t.code == 100).unwrap_or(rest.len());
        Groups(rest.get(..end).unwrap_or(&[]))
    }
}

/// The extended data of one application: the groups after its `1001` up to the next `1001`.
pub fn xdata<'a>(tags: &'a [Tag], app: &str) -> &'a [Tag] {
    let Some(start) = tags.iter().position(|t| t.code == 1001 && t.str_ref().is_some_and(|s| s.trim().eq_ignore_ascii_case(app))) else {
        return &[];
    };
    let rest = tags.get(start + 1..).unwrap_or(&[]);
    let end = rest.iter().position(|t| t.code == 1001).unwrap_or(rest.len());
    rest.get(..end).unwrap_or(&[])
}

/// The groups inside `1000 <name>`, `1002 {` ... `1002 }` of an xdata run (nested braces kept).
pub fn xdata_list<'a>(x: &'a [Tag], name: &str) -> &'a [Tag] {
    let Some(at) = x.iter().position(|t| t.code == 1000 && t.str_ref().is_some_and(|s| s.trim().eq_ignore_ascii_case(name))) else {
        return &[];
    };
    if !x.get(at + 1).is_some_and(|t| t.code == 1002 && t.str_ref().is_some_and(|s| s.trim() == "{")) {
        return &[];
    }
    let body = x.get(at + 2..).unwrap_or(&[]);
    let mut depth = 0usize;
    for (i, t) in body.iter().enumerate() {
        if t.code == 1002 {
            match t.str_ref().map(str::trim) {
                Some("{") => depth += 1,
                Some("}") if depth == 0 => return body.get(..i).unwrap_or(&[]),
                Some("}") => depth -= 1,
                _ => {}
            }
        }
    }
    body
}

/// The tags outside application-defined groups (`102 {` to `102 }`, whose codes are the
/// application's), with their positions in `tags`.
pub fn outside_app(tags: &[Tag]) -> impl Iterator<Item = (usize, &Tag)> {
    let mut in_app = false;
    tags.iter().enumerate().filter(move |(_, t)| {
        if t.code == 102 {
            in_app = t.str_ref().is_some_and(|s| s.starts_with('{'));
        }
        !in_app && t.code != 102
    })
}

/// A colour as a record gives it: group 62 (index), overridden by 420 (true colour).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Color {
    #[default]
    ByLayer,
    ByBlock,
    /// A colour index, 1–255 ([`aci::rgb`]).
    Index(u8),
    Rgb([u8; 3]),
}

impl Color {
    /// The colour of a group 62 value: 0 ByBlock, 1–255 an index, anything else (256, or a
    /// negative number) ByLayer.
    pub fn from_index(v: i64) -> Color {
        match u8::try_from(v) {
            Ok(0) => Color::ByBlock,
            Ok(i) => Color::Index(i),
            Err(_) => Color::ByLayer,
        }
    }
    /// The colour of a group 420 value (`0x00RRGGBB`).
    pub fn from_true(v: i64) -> Color {
        let [_, r, g, b] = u32::try_from(v & 0xff_ffff).unwrap_or(0).to_be_bytes();
        Color::Rgb([r, g, b])
    }
    /// The colour of a record: its 420 when it has one, else its 62.
    pub fn of(g: &Groups<'_>) -> Color {
        match (g.i(420), g.i(62)) {
            (Some(tc), _) => Color::from_true(tc),
            (None, Some(v)) => Color::from_index(v),
            (None, None) => Color::ByLayer,
        }
    }
}

/// A lineweight (group 370): hundredths of a millimetre, or one of the three special values.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Lineweight {
    #[default]
    ByLayer,
    ByBlock,
    /// The drawing's default (`$LWDEFAULT`).
    Default,
    /// Hundredths of a millimetre (0–211).
    Hundredths(u16),
}

impl Lineweight {
    /// -1 ByLayer, -2 ByBlock, -3 the default, else hundredths of a millimetre. Any other
    /// negative number is ByLayer.
    pub fn from_dxf(v: i64) -> Lineweight {
        match v {
            -2 => Lineweight::ByBlock,
            -3 => Lineweight::Default,
            v => u16::try_from(v).map_or(Lineweight::ByLayer, Lineweight::Hundredths),
        }
    }
    pub fn to_dxf(self) -> i64 {
        match self {
            Lineweight::ByLayer => -1,
            Lineweight::ByBlock => -2,
            Lineweight::Default => -3,
            Lineweight::Hundredths(v) => i64::from(v),
        }
    }
}

/// A transparency (group 440, and the `AcCmTransparency` extended data of a layer).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Transparency {
    #[default]
    ByLayer,
    ByBlock,
    /// Opacity: 255 opaque, 0 clear.
    Alpha(u8),
}

impl Transparency {
    /// The top byte says which: `0x01` ByBlock, `0x02` an alpha in the low byte, anything else
    /// ByLayer.
    pub fn from_dxf(v: i64) -> Transparency {
        match (v >> 24) & 0xff {
            1 => Transparency::ByBlock,
            2 => Transparency::Alpha(u8::try_from(v & 0xff).unwrap_or(255)),
            _ => Transparency::ByLayer,
        }
    }
    pub fn to_dxf(self) -> i64 {
        match self {
            Transparency::ByLayer => 0,
            Transparency::ByBlock => 0x0100_0000,
            Transparency::Alpha(a) => 0x0200_0000 | i64::from(a),
        }
    }
}

fn cross(a: Point, b: Point) -> Point {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn unit(v: Point) -> Option<Point> {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    (len.is_finite() && len > 1e-12).then(|| [v[0] / len, v[1] / len, v[2] / len])
}

/// The X, Y and Z axes of the object coordinate system whose extrusion direction (groups
/// 210/220/230) is `normal`: the DXF Reference's arbitrary axis algorithm. A normal that is no
/// direction (zero, or not finite) gives the world axes.
pub fn ocs_axes(normal: Point) -> [Point; 3] {
    const WORLD: [Point; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    let Some(z) = unit(normal) else { return WORLD };
    // Near the world Z axis the X axis comes from world Y, elsewhere from world Z.
    let near_z = z[0].abs() < 1.0 / 64.0 && z[1].abs() < 1.0 / 64.0;
    let Some(x) = unit(cross(if near_z { WORLD[1] } else { WORLD[2] }, z)) else { return WORLD };
    [x, cross(z, x), z]
}

/// A point of the object coordinate system of `normal` in world coordinates.
pub fn ocs_to_wcs(normal: Point, p: Point) -> Point {
    let [x, y, z] = ocs_axes(normal);
    [x[0] * p[0] + y[0] * p[1] + z[0] * p[2], x[1] * p[0] + y[1] * p[1] + z[1] * p[2], x[2] * p[0] + y[2] * p[1] + z[2] * p[2]]
}
