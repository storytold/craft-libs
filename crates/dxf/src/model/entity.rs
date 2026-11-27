//! Entities, typed: the properties every entity has ([`Common`]) and the geometry of each type.
//!
//! Values are as the file stores them. Planar entities (CIRCLE, ARC, LWPOLYLINE, 2D POLYLINE,
//! TEXT, INSERT, SOLID, TRACE, HATCH) have their points in the object coordinate system of
//! [`Common::extrusion`] ([`super::ocs_to_wcs`]); LINE, POINT, ELLIPSE, SPLINE, RAY, XLINE,
//! 3DFACE, MTEXT, LEADER and DIMENSION definition points are world coordinates. Angles are in
//! degrees, except an ELLIPSE's parameters, which the format stores in radians.

use std::borrow::Cow;

use super::document::{EntityRecord, Record};
use super::hatch::Hatch;
use super::text::{MText, decode_line, split_embedded};
use super::{Color, Groups, Lineweight, Point, Transparency, outside_app, parse_handle};
use crate::Tag;

/// The properties every entity has.
#[derive(Clone, Debug, PartialEq)]
pub struct Common {
    pub handle: Option<u64>,
    /// Group 330: the block record (or other object) that owns the entity.
    pub owner: Option<u64>,
    /// Group 8 (`0` when absent).
    pub layer: String,
    /// Group 6; `None` is ByLayer.
    pub linetype: Option<String>,
    pub color: Color,
    pub lineweight: Lineweight,
    /// Group 48 (1 when absent or not positive).
    pub ltscale: f64,
    /// Group 60 is not 1.
    pub visible: bool,
    /// Group 67 is 1.
    pub paper_space: bool,
    pub transparency: Transparency,
    /// Group 39.
    pub thickness: f64,
    /// Groups 210/220/230 (the world Z axis when absent).
    pub extrusion: Point,
    /// Group 390: the plot style name object.
    pub plot_style: Option<u64>,
}

/// The AcDbEntity group codes a writer can put after the entity's own subclass marker instead
/// (DXF Reference, Common Group Codes for Entities): paper space, layout, layer, linetype,
/// material, colour, lineweight, linetype scale, visibility, true colour, colour name,
/// transparency, plot style and shadow mode.
const ENTITY_CODES: [i32; 14] = [67, 410, 8, 6, 347, 62, 370, 48, 60, 420, 430, 440, 390, 284];

/// The groups of the AcDbEntity subclass: the record up to the entity's own subclass marker (all
/// of it in files without markers, and in records with no AcDbEntity marker, whose layer and
/// colour may follow the entity's marker), or up to an embedded object (101). Later groups reuse
/// the codes: an MTEXT's column width is 48. Markers inside an application-defined group do not
/// count.
///
/// Some writers leave the AcDbEntity subclass without a layer and put its groups right after the
/// entity's own marker. Every entity has a layer, so its absence marks that layout, and the run
/// of [`ENTITY_CODES`] that starts after the marker is read too. The run ends at the first other
/// code, so the entity's own groups that reuse a code are not taken for it. Only such a record
/// pays for the copy.
fn entity_groups(tags: &[Tag]) -> Cow<'_, [Tag]> {
    let is_entity = |t: &Tag| t.str_ref().is_some_and(|s| s == "AcDbEntity");
    let marked = outside_app(tags).any(|(_, t)| t.code == 100 && is_entity(t));
    let end = outside_app(tags).find(|(_, t)| (marked && t.code == 100 && !is_entity(t)) || t.code == 101).map_or(tags.len(), |(i, _)| i);
    let groups = tags.get(..end).unwrap_or(tags);
    let after_own_marker = marked && tags.get(end).is_some_and(|t| t.code == 100);
    if !after_own_marker || outside_app(groups).any(|(_, t)| t.code == 8) {
        return Cow::Borrowed(groups);
    }
    let run = tags.get(end + 1..).unwrap_or_default().iter().take_while(|t| ENTITY_CODES.contains(&t.code));
    Cow::Owned(groups.iter().chain(run).cloned().collect())
}

impl Common {
    pub fn read(tags: &[Tag]) -> Common {
        let own = entity_groups(tags);
        let (all, e) = (Groups(tags), Groups(&own));
        Common {
            handle: e.handle(5),
            owner: outside_app(tags).find(|(_, t)| t.code == 330).and_then(|(_, t)| parse_handle(&t.str())),
            layer: e.name(8).unwrap_or_else(|| "0".into()),
            linetype: e.name(6).filter(|l| !l.eq_ignore_ascii_case("ByLayer")),
            color: Color::of(&e),
            lineweight: e.i(370).map_or(Lineweight::ByLayer, Lineweight::from_dxf),
            ltscale: e.f(48).filter(|s| *s > 0.0).unwrap_or(1.0),
            visible: e.i(60) != Some(1),
            paper_space: e.i(67) == Some(1),
            transparency: e.i(440).map_or(Transparency::ByLayer, Transparency::from_dxf),
            thickness: all.f_or(39, 0.0),
            extrusion: [all.f_or(210, 0.0), all.f_or(220, 0.0), all.f_or(230, 1.0)],
            plot_style: e.handle(390).filter(|h| *h != 0),
        }
    }
}

/// A vertex of an LWPOLYLINE.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct LwVertex {
    pub x: f64,
    pub y: f64,
    /// Group 42: the tangent of a quarter of the arc to the next vertex (0: straight).
    pub bulge: f64,
    pub start_width: f64,
    pub end_width: f64,
}

/// An LWPOLYLINE.
#[derive(Clone, Debug, PartialEq)]
pub struct LwPolyline {
    pub vertices: Vec<LwVertex>,
    /// Flags (70) bit 1.
    pub closed: bool,
    /// Flags bit 128: the linetype runs around the vertices.
    pub plinegen: bool,
    /// Group 43.
    pub const_width: f64,
    /// Group 38.
    pub elevation: f64,
}

/// A VERTEX of a POLYLINE.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vertex {
    pub location: Point,
    pub bulge: f64,
    pub start_width: f64,
    pub end_width: f64,
    /// Group 70: 1 added by curve fitting, 8 added by spline fitting, 16 a spline frame control
    /// point, 32 a 3D polyline vertex, 64 a mesh vertex, 128 a polyface vertex (128 without 64:
    /// a face record).
    pub flags: i64,
    /// Groups 71–74: the vertex numbers of a polyface face record (1-based; negative: the edge
    /// from it is invisible; 0: none).
    pub face: [i64; 4],
}

impl Vertex {
    /// A face record of a polyface mesh, which holds vertex numbers and no point.
    pub fn is_face(&self) -> bool {
        self.flags & 128 != 0 && self.flags & 64 == 0
    }
    /// A control point of the frame a spline-fit polyline was made from: not on the curve.
    pub fn is_spline_frame(&self) -> bool {
        self.flags & 16 != 0
    }
}

/// A POLYLINE with its VERTEX records: a 2D or 3D polyline, a polygon mesh or a polyface mesh.
#[derive(Clone, Debug, PartialEq)]
pub struct Polyline {
    /// Group 70: 1 closed (a mesh: closed in M), 2 curve-fit, 4 spline-fit, 8 a 3D polyline, 16 a
    /// polygon mesh, 32 a mesh closed in N, 64 a polyface mesh, 128 the linetype runs around the
    /// vertices.
    pub flags: i64,
    /// The z of the POLYLINE's own point: a 2D polyline's elevation.
    pub elevation: f64,
    /// Groups 40 and 41: the default widths.
    pub start_width: f64,
    pub end_width: f64,
    /// Groups 71 and 72: a mesh's M and N vertex counts (a polyface mesh: vertices and faces).
    pub m: i64,
    pub n: i64,
    /// Group 75: the smooth surface type (5 quadratic, 6 cubic, 8 Bezier).
    pub smooth: i64,
    pub vertices: Vec<Vertex>,
}

impl Polyline {
    pub fn closed(&self) -> bool {
        self.flags & 1 != 0
    }
    pub fn is_3d(&self) -> bool {
        self.flags & 8 != 0
    }
    pub fn is_mesh(&self) -> bool {
        self.flags & 16 != 0
    }
    pub fn is_polyface(&self) -> bool {
        self.flags & 64 != 0
    }
}

/// A SPLINE (NURBS), in world coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct Spline {
    /// Group 71 (3 when absent).
    pub degree: i64,
    /// Group 70: 1 closed, 2 periodic, 4 rational, 8 planar, 16 linear.
    pub flags: i64,
    pub knots: Vec<f64>,
    pub control: Vec<Point>,
    /// Group 41: one per control point when rational.
    pub weights: Vec<f64>,
    pub fit: Vec<Point>,
    /// Groups 12/22/32 and 13/23/33.
    pub start_tangent: Option<Point>,
    pub end_tangent: Option<Point>,
    /// Group 44.
    pub fit_tolerance: f64,
}

/// A HELIX: a spline (the curve as drawn) and the parameters it was made from.
#[derive(Clone, Debug, PartialEq)]
pub struct Helix {
    /// The `AcDbSpline` groups.
    pub spline: Spline,
    /// Group 10: the axis base point.
    pub axis_base: Point,
    /// Group 11: the start point.
    pub start: Point,
    /// Group 12: the axis direction.
    pub axis: Point,
    /// Group 40: the base radius.
    pub radius: f64,
    /// Group 41.
    pub turns: f64,
    /// Group 42: the height of one turn.
    pub turn_height: f64,
    /// Group 290 is not 0: counter-clockwise (the default).
    pub ccw: bool,
    /// Group 280: what stays fixed when the helix is edited (0 turn height, 1 turns, 2 height).
    pub constrain: i64,
}

/// A vertex of an MLINE.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct MLineVertex {
    /// Groups 11/21/31.
    pub location: Point,
    /// Groups 12/22/32: the direction of the segment that starts here.
    pub direction: Point,
    /// Groups 13/23/33: the miter direction.
    pub miter: Point,
    /// Per element of the style: its 74 count of 41 parameters (the distance from the vertex
    /// along the miter, then the pairs that start and end its breaks).
    pub element_params: Vec<Vec<f64>>,
    /// Per element: its 75 count of 42 area fill parameters.
    pub fill_params: Vec<Vec<f64>>,
}

/// An MLINE: parallel lines drawn by an MLINESTYLE.
#[derive(Clone, Debug, PartialEq)]
pub struct MLine {
    /// Group 2: the style's name.
    pub style: String,
    /// Group 340: the MLINESTYLE.
    pub style_handle: Option<u64>,
    /// Group 40.
    pub scale: f64,
    /// Group 70: 0 top, 1 zero (centred), 2 bottom.
    pub justification: i64,
    /// Group 71: 1 has vertices, 2 closed, 4 start caps suppressed, 8 end caps suppressed.
    pub flags: i64,
    /// Group 73: the number of elements of the style, as the entity states it.
    pub elements: i64,
    /// Groups 10/20/30.
    pub start: Point,
    pub vertices: Vec<MLineVertex>,
}

/// TEXT, and the text of an ATTRIB or ATTDEF.
#[derive(Clone, Debug, PartialEq)]
pub struct Text {
    /// Group 10: the first alignment point.
    pub insert: Point,
    /// Group 11: the second alignment point, which places text that is not left-baseline.
    pub align: Option<Point>,
    pub height: f64,
    /// The string, decoded ([`decode_line`]); `%%` codes are left as written.
    pub value: String,
    /// Group 50, in degrees.
    pub rotation: f64,
    /// Group 41 (1 when absent).
    pub width_factor: f64,
    /// Group 51, in degrees.
    pub oblique: f64,
    /// Group 7 (`Standard` when absent).
    pub style: String,
    /// Group 71: 2 backwards, 4 upside down.
    pub generation: i64,
    /// Group 72: 0 left, 1 centre, 2 right, 3 aligned, 4 middle, 5 fit.
    pub halign: i64,
    /// Group 73 (74 in an ATTRIB or ATTDEF, whose 73 is the field length): 0 baseline, 1 bottom,
    /// 2 middle, 3 top.
    pub valign: i64,
}

impl Text {
    /// The typed form of TEXT groups.
    pub fn read(g: &Groups<'_>) -> Text {
        Self::read_as(g, false)
    }

    /// A TEXT, or (with `attribute`) the text of an ATTRIB or ATTDEF. The vertical alignment is
    /// group 73 for TEXT and group 74 for attributes, whose group 73 is the field length: a
    /// writer that emits the field length would otherwise turn every attribute into baseline
    /// text.
    pub fn read_as(g: &Groups<'_>, attribute: bool) -> Text {
        Text {
            insert: g.point(10),
            align: g.point_opt(11),
            height: g.f_or(40, 0.0),
            value: decode_line(&g.s(1).unwrap_or_default()),
            rotation: g.f_or(50, 0.0),
            width_factor: g.f_or(41, 1.0),
            oblique: g.f_or(51, 0.0),
            style: g.name(7).unwrap_or_else(|| "Standard".into()),
            generation: g.i_or(71, 0),
            halign: g.i_or(72, 0),
            valign: g.i_or(if attribute { 74 } else { 73 }, 0),
        }
    }
}

/// An ATTRIB (an attribute of a block reference) or ATTDEF (its definition in the block).
#[derive(Clone, Debug, PartialEq)]
pub struct Attrib {
    /// Group 2.
    pub tag: String,
    pub text: Text,
    /// Group 3 of an ATTDEF, decoded; empty in an ATTRIB.
    pub prompt: String,
    /// Group 70: 1 invisible, 2 constant, 4 verify, 8 preset.
    pub flags: i64,
    /// The 280 after the tag in the attribute's subclass (one before the tag is the subclass
    /// version).
    pub lock_position: bool,
    /// The embedded MTEXT of a multiline attribute.
    pub multiline: Option<MText>,
}

impl Attrib {
    pub fn read(kind: &str, tags: &[Tag]) -> Attrib {
        // A multiline attribute has its text as embedded MTEXT after its own groups.
        let (own, embedded) = split_embedded(tags);
        let g = Groups(own);
        let sub = own.iter().position(|t| t.code == 100 && t.str_ref().is_some_and(|s| s.trim().starts_with("AcDbAttribute"))).unwrap_or(0);
        let rest = own.get(sub..).unwrap_or_default();
        let lock_position = rest.iter().position(|t| t.code == 2).is_some_and(|at| rest.iter().skip(at + 1).any(|t| t.code == 280 && t.i64() != 0));
        // The text is described by the groups up to the attribute's own subclass, and from there
        // by all but the field length (73) and a multiline attribute's header (71, 72 and the
        // point 11): there 72 isn't the justification nor 11 the alignment point, and a
        // left-justified attribute writes no 72 of its own, so the multiline one was read as
        // centred.
        let at = own.iter().position(|t| t.code == 100 && t.str_ref().is_some_and(|s| s.trim().starts_with("AcDbAttribute"))).unwrap_or(own.len());
        let (head, tail) = own.split_at(at);
        let text_groups: Vec<Tag> = head.iter().chain(tail.iter().filter(|t| !matches!(t.code, 71 | 72 | 73 | 11 | 21 | 31))).cloned().collect();
        Attrib {
            tag: g.s(2).unwrap_or_default(),
            text: Text::read_as(&Groups(&text_groups), true),
            prompt: if kind == "ATTDEF" { decode_line(&g.s(3).unwrap_or_default()) } else { String::new() },
            flags: g.i_or(70, 0),
            lock_position,
            multiline: embedded.filter(|e| !e.is_empty()).map(MText::read),
        }
    }
}

/// An INSERT: a block reference, with its attributes.
#[derive(Clone, Debug, PartialEq)]
pub struct Insert {
    /// Group 2.
    pub block: String,
    pub insert: Point,
    /// Groups 41, 42 and 43 (1 when absent).
    pub scale: Point,
    /// Group 50, in degrees.
    pub rotation: f64,
    /// Groups 70 and 71 (1 when absent or less).
    pub columns: i64,
    pub rows: i64,
    /// Groups 44 and 45.
    pub column_spacing: f64,
    pub row_spacing: f64,
    /// The ATTRIB records that follow, each with its own properties.
    pub attribs: Vec<(Common, Attrib)>,
}

/// A DIMENSION. The geometry it is drawn with is in its block ([`Dimension::block`]).
#[derive(Clone, Debug, PartialEq)]
pub struct Dimension {
    /// Group 70: the type in the low bits (0 rotated, 1 aligned, 2 angular, 3 diameter, 4 radius,
    /// 5 angular 3-point, 6 ordinate), 32 the block is this dimension's alone, 64 an ordinate's
    /// X type, 128 the text is at a user position.
    pub flags: i64,
    /// Group 2: the block that holds the dimension's picture.
    pub block: Option<String>,
    /// Group 3 (`Standard` when absent).
    pub style: String,
    /// Group 10: the definition point.
    pub defpt: Point,
    /// Group 11: the middle of the text, in the object coordinate system.
    pub text_mid: Point,
    /// Group 12: the insertion point for clones (baseline and continue).
    pub insertion: Point,
    /// Groups 13 to 16: the definition points of the type.
    pub p13: Point,
    pub p14: Point,
    pub p15: Point,
    pub p16: Point,
    /// Group 1, decoded: empty or `<>` for the measurement, a space for none.
    pub text: String,
    /// Group 42: the measurement, when the file states it.
    pub measurement: Option<f64>,
    /// Group 50: the angle of a rotated dimension, in degrees.
    pub rotation: f64,
    /// Group 52: the oblique angle of the extension lines, in degrees.
    pub oblique: f64,
    /// Group 53: the text's rotation, in degrees.
    pub text_rotation: f64,
    /// Group 51: the horizontal direction, in degrees.
    pub horizontal: f64,
    /// Group 40: the leader length of a radius or diameter dimension.
    pub leader_length: f64,
}

impl Dimension {
    /// The type: the low four bits of the flags.
    pub fn kind(&self) -> i64 {
        self.flags & 0x0f
    }
}

/// A LEADER.
#[derive(Clone, Debug, PartialEq)]
pub struct Leader {
    pub vertices: Vec<Point>,
    /// Group 71 is not 0.
    pub arrow: bool,
    /// Group 72 is not 0: a spline path.
    pub spline: bool,
    /// Group 3 (`Standard` when absent).
    pub style: String,
    /// Group 73: what the leader was made with (0 text, 1 a tolerance, 2 a block, 3 nothing).
    pub annotation_type: i64,
    /// Group 340: the annotation.
    pub annotation: Option<u64>,
}

/// A paper space VIEWPORT.
#[derive(Clone, Debug, PartialEq)]
pub struct Viewport {
    /// Group 10: the centre on the sheet.
    pub center: Point,
    /// Groups 40 and 41: the size on the sheet.
    pub width: f64,
    pub height: f64,
    /// Group 69: the id (1 is the sheet's own view). Other programs write -1 for unknown.
    pub id: i64,
    /// Group 90: the status flags (16384 locked, 65536 clipped by the entity of group 340,
    /// 131072 off).
    pub status: i64,
    /// Groups 12/22: the view centre in display coordinates, whose origin is the target.
    pub view_center: [f64; 2],
    /// Groups 16/26/36 and 17/27/37.
    pub view_direction: Point,
    pub view_target: Point,
    /// Group 45.
    pub view_height: f64,
    /// Group 51: the twist, in degrees.
    pub twist: f64,
    /// Group 340: the entity that clips the viewport.
    pub clip: Option<u64>,
    /// Groups 331 and 341: the layers frozen in the viewport.
    pub frozen_layers: Vec<u64>,
}

/// An IMAGE or WIPEOUT: a raster's place and clip. The file is named by the IMAGEDEF object.
#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    /// Group 10: the lower-left corner.
    pub insert: Point,
    /// Groups 11 and 12: the vectors of one pixel along the image's width and height.
    pub u: Point,
    pub v: Point,
    /// Groups 13/23: the size in pixels.
    pub size: [f64; 2],
    /// Group 340: the IMAGEDEF.
    pub image_def: Option<u64>,
    /// Group 70: 1 show, 2 show when not aligned with the screen, 4 use the clip, 8 transparency.
    pub display: i64,
    /// Group 280 is not 0.
    pub clipping: bool,
    /// Groups 281, 282 and 283 (50, 50 and 0 when absent).
    pub brightness: i64,
    pub contrast: i64,
    pub fade: i64,
    /// Group 71: 1 a rectangle (two corners), 2 a polygon.
    pub clip_type: i64,
    /// Groups 14/24: the clip boundary in pixels, with the origin at the image's top-left corner
    /// and y down: a point `q` is at `insert + u * (q.x + 0.5) + v * (0.5 - q.y)`.
    pub clip: Vec<[f64; 2]>,
    /// Group 290 is not 0: the clip keeps the outside.
    pub clip_inverted: bool,
}

/// The geometry of an entity.
#[derive(Clone, Debug, PartialEq)]
pub enum EntityKind {
    Line {
        start: Point,
        end: Point,
    },
    /// `angle` is group 50: the X axis of the point's symbol, in degrees.
    Point {
        location: Point,
        angle: f64,
    },
    Circle {
        center: Point,
        radius: f64,
    },
    Arc {
        center: Point,
        radius: f64,
        start_angle: f64,
        end_angle: f64,
    },
    /// `major` is the end of the major axis from the centre; `start` and `end` are parameters in
    /// radians (0 and 2π for a full ellipse).
    Ellipse {
        center: Point,
        major: Point,
        ratio: f64,
        start: f64,
        end: f64,
    },
    LwPolyline(LwPolyline),
    Polyline(Polyline),
    Spline(Spline),
    Helix(Helix),
    MLine(MLine),
    /// Written from the 2007 format on.
    MLeader(super::mleader::MLeader),
    /// Written from the 2004 format on.
    Table(super::table::Table),
    Ray {
        base: Point,
        direction: Point,
    },
    XLine {
        base: Point,
        direction: Point,
    },
    Text(Text),
    MText(MText),
    AttDef(Attrib),
    Insert(Insert),
    Dimension(Dimension),
    Leader(Leader),
    /// The fourth corner repeats the third when the file has three.
    Solid {
        corners: [Point; 4],
    },
    Trace {
        corners: [Point; 4],
    },
    /// `hidden_edges` is group 70: bit n hides edge n.
    Face3d {
        corners: [Point; 4],
        hidden_edges: i64,
    },
    Hatch(Hatch),
    Viewport(Viewport),
    Image(Image),
    Wipeout(Image),
    /// `text` is the feature control frame's string, as written.
    Tolerance {
        insert: Point,
        direction: Point,
        style: String,
        text: String,
    },
    /// A type this module does not model (or a HATCH with no boundary data): read it from the
    /// record's tags.
    Other,
}

/// An entity: its properties and geometry.
#[derive(Clone, Debug, PartialEq)]
pub struct Entity {
    pub common: Common,
    pub kind: EntityKind,
}

fn lwpolyline(tags: &[Tag], g: &Groups<'_>) -> LwPolyline {
    let mut vertices: Vec<LwVertex> = Vec::new();
    for t in tags {
        if t.code == 10 {
            vertices.push(LwVertex { x: t.f64(), ..LwVertex::default() });
        } else if let Some(v) = vertices.last_mut() {
            match t.code {
                20 => v.y = t.f64(),
                40 => v.start_width = t.f64(),
                41 => v.end_width = t.f64(),
                42 => v.bulge = t.f64(),
                _ => {}
            }
        }
    }
    let flags = g.i_or(70, 0);
    LwPolyline { vertices, closed: flags & 1 != 0, plinegen: flags & 128 != 0, const_width: g.f_or(43, 0.0), elevation: g.f_or(38, 0.0) }
}

fn polyline(g: &Groups<'_>, followers: &[Record<'_>]) -> Polyline {
    let vertices = followers
        .iter()
        .map(|r| {
            let v = r.groups();
            Vertex {
                location: v.point(10),
                bulge: v.f_or(42, 0.0),
                start_width: v.f_or(40, 0.0),
                end_width: v.f_or(41, 0.0),
                flags: v.i_or(70, 0),
                face: [71, 72, 73, 74].map(|c| v.i_or(c, 0)),
            }
        })
        .collect();
    Polyline {
        flags: g.i_or(70, 0),
        elevation: g.f_or(30, 0.0),
        start_width: g.f_or(40, 0.0),
        end_width: g.f_or(41, 0.0),
        m: g.i_or(71, 0),
        n: g.i_or(72, 0),
        smooth: g.i_or(75, 0),
        vertices,
    }
}

fn mline(tags: &[Tag], g: &Groups<'_>) -> MLine {
    let mut vertices: Vec<MLineVertex> = Vec::new();
    // Which list the 41s or 42s go to: the last one a 74 or 75 started.
    let mut fill = false;
    for t in tags {
        if t.code == 11 {
            vertices.push(MLineVertex { location: [t.f64(), 0.0, 0.0], ..MLineVertex::default() });
            continue;
        }
        let Some(v) = vertices.last_mut() else { continue };
        let axis = match t.code / 10 {
            1 => 0,
            2 => 1,
            _ => 2,
        };
        match t.code {
            21 | 31 => v.location[axis] = t.f64(),
            12 | 22 | 32 => v.direction[axis] = t.f64(),
            13 | 23 | 33 => v.miter[axis] = t.f64(),
            74 => {
                v.element_params.push(Vec::new());
                fill = false;
            }
            75 => {
                v.fill_params.push(Vec::new());
                fill = true;
            }
            41 if !fill => v.element_params.last_mut().into_iter().for_each(|l| l.push(t.f64())),
            42 if fill => v.fill_params.last_mut().into_iter().for_each(|l| l.push(t.f64())),
            _ => {}
        }
    }
    MLine {
        style: g.s(2).unwrap_or_default(),
        style_handle: g.handle(340).filter(|h| *h != 0),
        scale: g.f_or(40, 1.0),
        justification: g.i_or(70, 0),
        flags: g.i_or(71, 1),
        elements: g.i_or(73, 0),
        start: g.point(10),
        vertices,
    }
}

fn spline(g: &Groups<'_>) -> Spline {
    Spline {
        degree: g.i_or(71, 3),
        flags: g.i_or(70, 0),
        knots: g.all_f(40),
        control: g.points(10),
        weights: g.all_f(41),
        fit: g.points(11),
        start_tangent: g.point_opt(12),
        end_tangent: g.point_opt(13),
        fit_tolerance: g.f_or(44, 0.0),
    }
}

fn insert(g: &Groups<'_>, followers: &[Record<'_>]) -> Insert {
    Insert {
        block: g.s(2).unwrap_or_default(),
        insert: g.point(10),
        scale: [g.f_or(41, 1.0), g.f_or(42, 1.0), g.f_or(43, 1.0)],
        rotation: g.f_or(50, 0.0),
        columns: g.i_or(70, 1).max(1),
        rows: g.i_or(71, 1).max(1),
        column_spacing: g.f_or(44, 0.0),
        row_spacing: g.f_or(45, 0.0),
        attribs: followers.iter().map(|r| (Common::read(r.tags), Attrib::read("ATTRIB", r.tags))).collect(),
    }
}

fn dimension(g: &Groups<'_>) -> Dimension {
    Dimension {
        flags: g.i_or(70, 0),
        block: g.name(2),
        style: g.name(3).unwrap_or_else(|| "Standard".into()),
        defpt: g.point(10),
        text_mid: g.point(11),
        insertion: g.point(12),
        p13: g.point(13),
        p14: g.point(14),
        p15: g.point(15),
        p16: g.point(16),
        text: decode_line(&g.s(1).unwrap_or_default()),
        measurement: g.f(42),
        rotation: g.f_or(50, 0.0),
        oblique: g.f_or(52, 0.0),
        text_rotation: g.f_or(53, 0.0),
        horizontal: g.f_or(51, 0.0),
        leader_length: g.f_or(40, 0.0),
    }
}

fn viewport(tags: &[Tag], g: &Groups<'_>) -> Viewport {
    Viewport {
        center: g.point(10),
        width: g.f_or(40, 0.0),
        height: g.f_or(41, 0.0),
        id: g.i_or(69, 0),
        status: g.i_or(90, 0),
        view_center: [g.f_or(12, 0.0), g.f_or(22, 0.0)],
        view_direction: g.point_opt(16).unwrap_or([0.0, 0.0, 1.0]),
        view_target: g.point(17),
        view_height: g.f_or(45, 0.0),
        twist: g.f_or(51, 0.0),
        clip: g.handle(340).filter(|h| *h != 0),
        frozen_layers: tags.iter().filter(|t| matches!(t.code, 331 | 341)).filter_map(|t| parse_handle(&t.str())).collect(),
    }
}

fn image(g: &Groups<'_>) -> Image {
    Image {
        insert: g.point(10),
        u: g.point(11),
        v: g.point(12),
        size: [g.f_or(13, 0.0), g.f_or(23, 0.0)],
        image_def: g.handle(340).filter(|h| *h != 0),
        display: g.i_or(70, 0),
        clipping: g.i_or(280, 0) != 0,
        brightness: g.i_or(281, 50),
        contrast: g.i_or(282, 50),
        fade: g.i_or(283, 0),
        clip_type: g.i_or(71, 1),
        clip: g.points(14).into_iter().map(|p| [p[0], p[1]]).collect(),
        clip_inverted: g.i_or(290, 0) != 0,
    }
}

fn corners(g: &Groups<'_>) -> [Point; 4] {
    [g.point(10), g.point(11), g.point(12), g.point_opt(13).unwrap_or_else(|| g.point(12))]
}

impl Entity {
    /// The typed form of an entity record.
    pub fn read(e: &EntityRecord<'_>) -> Entity {
        let g = e.groups();
        let line = |g: &Groups<'_>| (g.point(10), g.point(11));
        let kind = match e.kind {
            "LINE" => EntityKind::Line { start: line(&g).0, end: line(&g).1 },
            "POINT" => EntityKind::Point { location: g.point(10), angle: g.f_or(50, 0.0) },
            "CIRCLE" => EntityKind::Circle { center: g.point(10), radius: g.f_or(40, 0.0) },
            "ARC" => EntityKind::Arc { center: g.point(10), radius: g.f_or(40, 0.0), start_angle: g.f_or(50, 0.0), end_angle: g.f_or(51, 360.0) },
            "ELLIPSE" => EntityKind::Ellipse {
                center: g.point(10),
                major: g.point(11),
                ratio: g.f_or(40, 1.0),
                start: g.f_or(41, 0.0),
                end: g.f_or(42, std::f64::consts::TAU),
            },
            "LWPOLYLINE" => EntityKind::LwPolyline(lwpolyline(e.tags, &g)),
            "POLYLINE" => EntityKind::Polyline(polyline(&g, &e.followers)),
            "SPLINE" => EntityKind::Spline(spline(&g)),
            "HELIX" => {
                let h = g.subclass("AcDbHelix");
                EntityKind::Helix(Helix {
                    spline: spline(&g.subclass("AcDbSpline")),
                    axis_base: h.point(10),
                    start: h.point(11),
                    axis: h.point_opt(12).unwrap_or([0.0, 0.0, 1.0]),
                    radius: h.f_or(40, 0.0),
                    turns: h.f_or(41, 1.0),
                    turn_height: h.f_or(42, 0.0),
                    ccw: h.i_or(290, 1) != 0,
                    constrain: h.i_or(280, 2),
                })
            }
            "MLINE" => {
                let own = g.subclass("AcDbMline");
                let own = if own.0.is_empty() { g } else { own };
                EntityKind::MLine(mline(own.0, &own))
            }
            "MULTILEADER" | "MLEADER" => EntityKind::MLeader(super::mleader::MLeader::read(e.tags)),
            "ACAD_TABLE" => EntityKind::Table(super::table::Table::read(e.tags)),
            "RAY" => EntityKind::Ray { base: line(&g).0, direction: line(&g).1 },
            "XLINE" => EntityKind::XLine { base: line(&g).0, direction: line(&g).1 },
            "TEXT" => EntityKind::Text(Text::read(&g)),
            "MTEXT" => EntityKind::MText(MText::read(e.tags)),
            "ATTDEF" => EntityKind::AttDef(Attrib::read("ATTDEF", e.tags)),
            "INSERT" => EntityKind::Insert(insert(&g, &e.followers)),
            "DIMENSION" => EntityKind::Dimension(dimension(&g)),
            "LEADER" => EntityKind::Leader(Leader {
                vertices: g.points(10),
                arrow: g.i_or(71, 1) != 0,
                spline: g.i_or(72, 0) != 0,
                style: g.name(3).unwrap_or_else(|| "Standard".into()),
                annotation_type: g.i_or(73, 3),
                annotation: g.handle(340).filter(|h| *h != 0),
            }),
            "SOLID" => EntityKind::Solid { corners: corners(&g) },
            "TRACE" => EntityKind::Trace { corners: corners(&g) },
            "3DFACE" => EntityKind::Face3d { corners: corners(&g), hidden_edges: g.i_or(70, 0) },
            "HATCH" => Hatch::read(e.tags).map_or(EntityKind::Other, EntityKind::Hatch),
            "VIEWPORT" => EntityKind::Viewport(viewport(e.tags, &g)),
            "IMAGE" => EntityKind::Image(image(&g)),
            "WIPEOUT" => EntityKind::Wipeout(image(&g)),
            "TOLERANCE" => EntityKind::Tolerance {
                insert: g.point(10),
                direction: g.point_opt(11).unwrap_or([1.0, 0.0, 0.0]),
                style: g.name(3).unwrap_or_else(|| "Standard".into()),
                text: g.s(1).unwrap_or_default(),
            },
            _ => EntityKind::Other,
        };
        Entity { common: Common::read(e.tags), kind }
    }
}
