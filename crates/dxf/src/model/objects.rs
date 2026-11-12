//! The records of the OBJECTS section, typed: page setups, groups, image definitions, draw
//! order, clip filters, dimension associativity, annotation scales and multiline styles. And
//! the dimension style overrides an entity carries in its extended data.
//!
//! Each `read` takes a [`Record`] of [`Document::objects`](super::Document). An object's name is
//! not in its record but in the dictionary that lists it
//! ([`Document::dictionary_names`](super::Document::dictionary_names)).

use super::document::Record;
use super::tables::dim_var_name;
use super::{Color, Groups, Point, parse_handle, xdata, xdata_list};
use crate::Tag;

/// The groups of a record's subclass `marker` and everything after it up to the next marker;
/// the whole record when it has no such marker.
fn own<'a>(r: &Record<'a>, marker: &str) -> &'a [Tag] {
    let Some(at) = r.tags.iter().position(|t| t.code == 100 && t.str_ref().is_some_and(|s| s.trim() == marker)) else { return r.tags };
    let rest = r.tags.get(at + 1..).unwrap_or(&[]);
    rest.get(..rest.iter().position(|t| t.code == 100).unwrap_or(rest.len())).unwrap_or(rest)
}

/// The plot settings of a LAYOUT (its page setup), or a PLOTSETTINGS object: the
/// `AcDbPlotSettings` groups.
#[derive(Clone, Debug, PartialEq)]
pub struct PlotSettings {
    /// Group 1: the page setup's name.
    pub name: String,
    /// Group 2: the printer or plotter configuration (`none_device` for none).
    pub device: String,
    /// Group 4: the paper size's name.
    pub paper: String,
    /// Group 6: the plot view's name.
    pub view: String,
    /// Groups 40 to 43: the unprintable margins, left, bottom, right, top, in millimetres.
    pub margins: [f64; 4],
    /// Groups 44 and 45: the paper's width and height in millimetres.
    pub paper_size: [f64; 2],
    /// Groups 46 and 47: the plot origin's offset in millimetres.
    pub origin: [f64; 2],
    /// Groups 48, 49, 140 and 141: the plot window, lower-left x and y, upper-right x and y.
    pub window: [f64; 4],
    /// Groups 142 and 143: the custom scale, paper units over drawing units.
    pub scale: [f64; 2],
    /// Group 70: 1 viewport borders, 2 show plot styles, 4 centred, 8 hidden, 16 standard scale,
    /// 32 plot with plot styles, 64 scale lineweights, 128 print lineweights, 512 paper space
    /// last, 1024 model type, 2048 update paper, 4096 zoom to paper on update.
    pub flags: i64,
    /// Group 72: 0 inches, 1 millimetres, 2 pixels.
    pub paper_units: i64,
    /// Group 73: 0 none, 1 90° counter-clockwise, 2 upside down, 3 90° clockwise.
    pub rotation: i64,
    /// Group 74: 0 the display, 1 extents, 2 limits, 3 the view of group 6, 4 the window, 5 the
    /// layout.
    pub plot_type: i64,
    /// Group 7: the plot style table (`monochrome.ctb`).
    pub style_sheet: String,
    /// Group 75: the standard scale (0 scaled to fit).
    pub standard_scale: i64,
    /// Group 147: the scale as one factor.
    pub scale_factor: f64,
    /// Groups 148 and 149: the paper image origin.
    pub image_origin: [f64; 2],
}

impl PlotSettings {
    /// The plot settings of a LAYOUT or PLOTSETTINGS record: the groups of its
    /// `AcDbPlotSettings` subclass (a record without the marker is read up to `AcDbLayout`).
    pub fn read(r: &Record<'_>) -> PlotSettings {
        let g = Groups(own(r, "AcDbPlotSettings"));
        let pair = |a: i32, b: i32| [g.f_or(a, 0.0), g.f_or(b, 0.0)];
        PlotSettings {
            name: g.s(1).unwrap_or_default(),
            device: g.s(2).unwrap_or_default(),
            paper: g.s(4).unwrap_or_default(),
            view: g.s(6).unwrap_or_default(),
            margins: [g.f_or(40, 0.0), g.f_or(41, 0.0), g.f_or(42, 0.0), g.f_or(43, 0.0)],
            paper_size: pair(44, 45),
            origin: pair(46, 47),
            window: [g.f_or(48, 0.0), g.f_or(49, 0.0), g.f_or(140, 0.0), g.f_or(141, 0.0)],
            scale: [g.f_or(142, 1.0), g.f_or(143, 1.0)],
            flags: g.i_or(70, 0),
            paper_units: g.i_or(72, 0),
            rotation: g.i_or(73, 0),
            plot_type: g.i_or(74, 5),
            style_sheet: g.s(7).unwrap_or_default(),
            standard_scale: g.i_or(75, 0),
            scale_factor: g.f_or(147, 1.0),
            image_origin: pair(148, 149),
        }
    }
}

/// A GROUP: entities selected together.
#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    pub handle: Option<u64>,
    /// Group 300.
    pub description: String,
    /// Group 70 is 1: an unnamed group (`*A1`).
    pub unnamed: bool,
    /// Group 71 is not 0.
    pub selectable: bool,
    /// Groups 340.
    pub members: Vec<u64>,
}

impl Group {
    pub fn read(r: &Record<'_>) -> Group {
        let g = Groups(own(r, "AcDbGroup"));
        Group {
            handle: r.handle(),
            description: g.s(300).unwrap_or_default(),
            unnamed: g.i_or(70, 0) == 1,
            selectable: g.i_or(71, 1) != 0,
            members: g.0.iter().filter(|t| t.code == 340).filter_map(|t| parse_handle(&t.str())).collect(),
        }
    }
}

/// An IMAGEDEF: the file an IMAGE shows.
#[derive(Clone, Debug, PartialEq)]
pub struct ImageDef {
    pub handle: Option<u64>,
    /// Group 1: the file's path.
    pub file: String,
    /// Groups 10/20: the image's size in pixels.
    pub size: [f64; 2],
    /// Groups 11/21: one pixel's size in drawing units.
    pub pixel_size: [f64; 2],
    /// Group 280 is not 0.
    pub loaded: bool,
    /// Group 281: 0 none, 2 centimetres, 5 inches.
    pub units: i64,
}

impl ImageDef {
    pub fn read(r: &Record<'_>) -> ImageDef {
        let g = Groups(own(r, "AcDbRasterImageDef"));
        ImageDef {
            handle: r.handle(),
            file: g.s(1).unwrap_or_default(),
            size: [g.f_or(10, 0.0), g.f_or(20, 0.0)],
            pixel_size: [g.f_or(11, 1.0), g.f_or(21, 1.0)],
            loaded: g.i_or(280, 1) != 0,
            units: g.i_or(281, 0),
        }
    }
}

/// A SORTENTSTABLE: the draw order of a block's entities.
#[derive(Clone, Debug, PartialEq)]
pub struct SortEntsTable {
    /// Group 330 after the marker: the BLOCK_RECORD whose entities are ordered.
    pub block_record: Option<u64>,
    /// Each entity (group 331) with the sort handle (group 5) it is drawn by: entities draw in
    /// the order of their sort handles, one not listed by its own handle.
    pub entries: Vec<(u64, u64)>,
}

impl SortEntsTable {
    /// `None` for a record without the `AcDbSortentsTable` marker.
    pub fn read(r: &Record<'_>) -> Option<SortEntsTable> {
        let start = r.tags.iter().position(|t| t.code == 100 && t.str_ref().is_some_and(|s| s.trim() == "AcDbSortentsTable"))?;
        let tail = r.tags.get(start + 1..)?;
        let mut entries = Vec::new();
        let mut entity = None;
        for t in tail {
            match t.code {
                331 => entity = parse_handle(&t.str()),
                5 => entries.extend(entity.take().zip(parse_handle(&t.str()))),
                _ => {}
            }
        }
        Some(SortEntsTable { block_record: Groups(tail).handle(330), entries })
    }
}

/// A SPATIAL_FILTER: the clip boundary of a block reference (XCLIP).
#[derive(Clone, Debug, PartialEq)]
pub struct SpatialFilter {
    pub handle: Option<u64>,
    /// Groups 10/20: the boundary in the filter's own coordinate system; two points are the
    /// corners of a rectangle.
    pub boundary: Vec<[f64; 2]>,
    /// Groups 210/220/230: the boundary plane's normal.
    pub normal: Point,
    /// Groups 11/21/31: the boundary plane's origin.
    pub origin: Point,
    /// Group 71 is not 0.
    pub enabled: bool,
    /// The front and back clip distances, when groups 72 and 73 switch them on.
    pub front: Option<f64>,
    pub back: Option<f64>,
    /// The first twelve 40s after them: the inverse of the block reference's transform, a 4x3
    /// matrix by rows. Then twelve more: from block coordinates to the boundary's.
    pub inverse_insert: Option<[f64; 12]>,
    pub to_boundary: Option<[f64; 12]>,
    /// Group 290 is not 0: the clip keeps the outside.
    pub inverted: bool,
}

impl SpatialFilter {
    pub fn read(r: &Record<'_>) -> SpatialFilter {
        let tags = own(r, "AcDbSpatialFilter");
        let g = Groups(tags);
        // 72 and 73 switch the front and back clips on; each distance follows its switch.
        let after = |code: i32| tags.iter().position(|t| t.code == code).map(|i| (tags.get(i).is_some_and(|t| t.i64() != 0), i + 1));
        let front = after(72).and_then(|(on, i)| tags.get(i).filter(|t| on && t.code == 40).map(Tag::f64));
        let back = after(73).and_then(|(on, i)| tags.get(i).filter(|t| on && t.code == 41).map(Tag::f64));
        let from = after(73).or(after(72)).map_or(0, |(_, i)| i);
        let m: Vec<f64> = tags.get(from..).unwrap_or(&[]).iter().filter(|t| t.code == 40).take(24).map(Tag::f64).collect();
        let matrix = |at: usize| m.get(at..at + 12).and_then(|s| <[f64; 12]>::try_from(s).ok());
        SpatialFilter {
            handle: r.handle(),
            boundary: g.points(10).into_iter().map(|p| [p[0], p[1]]).collect(),
            normal: [g.f_or(210, 0.0), g.f_or(220, 0.0), g.f_or(230, 1.0)],
            origin: g.point(11),
            enabled: g.i_or(71, 1) != 0,
            front,
            back,
            inverse_insert: matrix(0),
            to_boundary: matrix(12),
            inverted: g.i_or(290, 0) != 0,
        }
    }
}

/// One point reference of a DIMASSOC: the geometry a definition point follows.
#[derive(Clone, Debug, PartialEq)]
pub struct OsnapRef {
    /// Group 72: the object snap (1 endpoint, 2 midpoint, 3 centre, 4 node, 5 quadrant, 6
    /// intersection, 7 insertion, 8 perpendicular, 9 tangent, 10 nearest, 11 apparent
    /// intersection, 12 parallel, 13 start point).
    pub snap: i64,
    /// Group 331: the entity.
    pub entity: Option<u64>,
    /// Group 73: the kind of its part (1 the whole, 2 an edge, 3 a vertex), and group 91, the
    /// part's index.
    pub sub_type: i64,
    pub sub_index: i64,
    /// Group 332: the second entity of an intersection.
    pub other: Option<u64>,
    /// Group 40: the parameter along the geometry, for a nearest snap.
    pub near: f64,
    /// Groups 10/20/30: the point.
    pub point: Point,
}

/// A DIMASSOC: what the definition points of a dimension are attached to.
#[derive(Clone, Debug, PartialEq)]
pub struct DimAssoc {
    pub handle: Option<u64>,
    /// Group 330 after the marker: the dimension.
    pub dimension: Option<u64>,
    /// Group 90: bit n is set when the nth definition point has a reference (in the order the
    /// references follow).
    pub flags: i64,
    /// Group 70 is not 0: the dimension is in paper space and its geometry in model space.
    pub trans_space: bool,
    /// Group 71: the rotated dimension type (parallel or perpendicular).
    pub rotated_type: i64,
    /// The references, each started by `1 AcDbOsnapPointRef`.
    pub refs: Vec<OsnapRef>,
}

impl DimAssoc {
    pub fn read(r: &Record<'_>) -> DimAssoc {
        let start = r.tags.iter().position(|t| t.code == 100 && t.str_ref().is_some_and(|s| s.trim() == "AcDbDimAssoc")).unwrap_or(0);
        let tail = r.tags.get(start..).unwrap_or(&[]);
        let starts: Vec<usize> = tail.iter().enumerate().filter(|(_, t)| t.code == 1).map(|(i, _)| i).collect();
        let head = Groups(tail.get(..starts.first().copied().unwrap_or(tail.len())).unwrap_or(tail));
        let refs = starts
            .iter()
            .enumerate()
            .map(|(k, s)| {
                let g = Groups(tail.get(s + 1..starts.get(k + 1).copied().unwrap_or(tail.len())).unwrap_or(&[]));
                OsnapRef {
                    snap: g.i_or(72, 0),
                    entity: g.handle(331),
                    sub_type: g.i_or(73, 0),
                    sub_index: g.i_or(91, 0),
                    other: g.handle(332),
                    near: g.f_or(40, 0.0),
                    point: g.point(10),
                }
            })
            .collect();
        DimAssoc {
            handle: r.handle(),
            dimension: head.handle(330),
            flags: head.i_or(90, 0),
            trans_space: head.i_or(70, 0) != 0,
            rotated_type: head.i_or(71, 0),
            refs,
        }
    }
}

/// A SCALE: an annotation scale of the drawing's scale list.
#[derive(Clone, Debug, PartialEq)]
pub struct Scale {
    pub handle: Option<u64>,
    /// Group 300 (`1:50`).
    pub name: String,
    /// Groups 140 and 141: paper units and drawing units.
    pub paper: f64,
    pub drawing: f64,
    /// Group 290 is not 0: the 1:1 scale.
    pub unit: bool,
}

impl Scale {
    /// `None` for a record without a name or one of its numbers.
    pub fn read(r: &Record<'_>) -> Option<Scale> {
        let g = Groups(own(r, "AcDbScale"));
        Some(Scale { handle: r.handle(), name: g.name(300)?, paper: g.f(140)?, drawing: g.f(141)?, unit: g.i_or(290, 0) != 0 })
    }
}

/// One line of a multiline style.
#[derive(Clone, Debug, PartialEq)]
pub struct MLineElement {
    /// Group 49: the offset from the multiline's path.
    pub offset: f64,
    /// Group 62 (ByLayer when absent).
    pub color: Color,
    /// Group 6 (`BYLAYER` when absent), as written.
    pub linetype: String,
}

/// An MLINESTYLE.
#[derive(Clone, Debug, PartialEq)]
pub struct MLineStyle {
    pub handle: Option<u64>,
    /// Group 2.
    pub name: String,
    /// Group 3.
    pub description: String,
    /// Group 70: 1 filled, 2 joints shown, 16 start line cap, 32 start inner arcs, 64 start
    /// outer arc, 256 end line cap, 512 end inner arcs, 1024 end outer arc.
    pub flags: i64,
    /// The 62 before the first element: the fill colour.
    pub fill: Color,
    /// Groups 51 and 52: the start and end cap angles, in degrees.
    pub start_angle: f64,
    pub end_angle: f64,
    pub elements: Vec<MLineElement>,
}

impl MLineStyle {
    /// `None` for a style without a name.
    pub fn read(r: &Record<'_>) -> Option<MLineStyle> {
        let body = own(r, "AcDbMlineStyle");
        let g = Groups(body);
        let mut st = MLineStyle {
            handle: r.handle(),
            name: g.name(2)?,
            description: g.s(3).unwrap_or_default(),
            flags: g.i_or(70, 0),
            fill: Color::ByLayer,
            start_angle: g.f_or(51, 90.0),
            end_angle: g.f_or(52, 90.0),
            elements: Vec::new(),
        };
        for t in body {
            match (t.code, st.elements.last_mut()) {
                (49, _) => st.elements.push(MLineElement { offset: t.f64(), color: Color::ByLayer, linetype: "BYLAYER".into() }),
                (62, Some(e)) => e.color = Color::from_index(t.i64()),
                (62, None) => st.fill = Color::from_index(t.i64()),
                (6, Some(e)) => e.linetype = t.str(),
                _ => {}
            }
        }
        Some(st)
    }
}

/// The dimension variables an entity overrides: the `DSTYLE` list of its `ACAD` extended data,
/// pairs of a `1070` group code (the variable's DIMSTYLE code) and the value. Each is the
/// variable's name and the group holding the value, as [`super::DimStyle::vars`] has them
/// (a handle value is a `1005`).
pub fn dim_overrides(tags: &[Tag]) -> Vec<(&'static str, &Tag)> {
    let list = xdata_list(xdata(tags, "ACAD"), "DSTYLE");
    let mut out = Vec::new();
    let mut i = 0;
    while let (Some(code), Some(value)) = (list.get(i), list.get(i + 1)) {
        // A value is never a 1070 naming a variable twice in a row: realign on a stray group.
        if code.code != 1070 {
            i += 1;
            continue;
        }
        if let Some(name) = i32::try_from(code.i64()).ok().and_then(dim_var_name) {
            out.push((name, value));
        }
        i += 2;
    }
    out
}
