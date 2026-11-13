//! The symbol table records, typed.

use super::document::Record;
use super::{Color, Lineweight, Point, Transparency, xdata, xdata_list};
use crate::Tag;

/// A LAYER record.
#[derive(Clone, Debug, PartialEq)]
pub struct Layer {
    pub handle: Option<u64>,
    pub name: String,
    /// The layer's colour: an index (1–255) or a true colour.
    pub color: Color,
    /// Switched on (a negative colour index switches a layer off).
    pub on: bool,
    /// Flags (70) bit 1.
    pub frozen: bool,
    /// Flags bit 2: frozen in new viewports.
    pub vp_freeze_new: bool,
    /// Flags bit 4.
    pub locked: bool,
    /// Group 290 (plotted unless it is 0). CAD apps never plot the layer `Defpoints` either,
    /// which is the caller's to apply.
    pub plot: bool,
    /// Group 6 as written (`Continuous` when absent).
    pub linetype: String,
    pub lineweight: Lineweight,
    /// Group 390: the plot style name object.
    pub plot_style: Option<u64>,
    /// The `AcCmTransparency` extended data.
    pub transparency: Transparency,
    /// The description in the `AcAecLayerStandard` extended data (its second string).
    pub description: Option<String>,
}

impl Layer {
    /// `None` for a record without a name.
    pub fn read(r: &Record<'_>) -> Option<Layer> {
        let g = r.groups();
        let name = g.name(2)?;
        let aci = g.i_or(62, 7);
        let flags = g.i_or(70, 0);
        let index = u8::try_from(aci.unsigned_abs().clamp(1, 255)).unwrap_or(7);
        let alpha = xdata(r.tags, "AcCmTransparency").iter().find(|t| t.code == 1071).map(Tag::i64);
        Some(Layer {
            handle: r.handle(),
            name,
            color: g.i(420).map_or(Color::Index(index), Color::from_true),
            on: aci >= 0,
            frozen: flags & 1 != 0,
            vp_freeze_new: flags & 2 != 0,
            locked: flags & 4 != 0,
            plot: g.i_or(290, 1) != 0,
            linetype: g.name(6).unwrap_or_else(|| "Continuous".into()),
            lineweight: g.i(370).map_or(Lineweight::Default, Lineweight::from_dxf),
            plot_style: g.handle(390),
            transparency: alpha.map_or(Transparency::ByLayer, Transparency::from_dxf),
            description: xdata(r.tags, "AcAecLayerStandard").iter().filter(|t| t.code == 1000).nth(1).map(Tag::str),
        })
    }
}

/// One element of a linetype's pattern: a dash, gap or dot, with the text or shape drawn at it.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct DashElement {
    /// Group 49: a dash (positive), a gap (negative) or a dot (0).
    pub length: f64,
    /// Group 74 bit 1: the rotation is absolute, not relative to the line.
    pub absolute: bool,
    /// Group 9, when group 74 bit 2 says the element draws text.
    pub text: Option<String>,
    /// Group 75, when group 74 bit 4 says the element draws a shape.
    pub shape: Option<u16>,
    /// Group 340: the STYLE of the text or shape.
    pub style: Option<u64>,
    /// Group 46 (1 when absent).
    pub scale: f64,
    /// Group 50, as stored.
    pub rotation: f64,
    /// Groups 44 and 45.
    pub offset: [f64; 2],
}

/// An LTYPE record.
#[derive(Clone, Debug, PartialEq)]
pub struct Linetype {
    pub handle: Option<u64>,
    pub name: String,
    /// Group 3.
    pub description: String,
    /// Group 40: the pattern's length as the file states it.
    pub length: f64,
    pub pattern: Vec<DashElement>,
}

impl Linetype {
    pub fn read(r: &Record<'_>) -> Option<Linetype> {
        let g = r.groups();
        let mut pattern: Vec<DashElement> = Vec::new();
        for t in r.tags.iter().take_while(|t| t.code < 1000) {
            if t.code == 49 {
                pattern.push(DashElement { length: t.f64(), scale: 1.0, ..DashElement::default() });
                continue;
            }
            let Some(el) = pattern.last_mut() else { continue };
            let complex = el.text.is_some() || el.shape.is_some();
            match t.code {
                74 => {
                    let flags = t.i64();
                    el.absolute = flags & 1 != 0;
                    if flags & 2 != 0 {
                        el.text = Some(String::new());
                    } else if flags & 4 != 0 {
                        el.shape = Some(0);
                    }
                }
                75 if el.shape.is_some() => el.shape = Some(u16::try_from(t.i64()).unwrap_or(0)),
                340 if complex => el.style = super::parse_handle(&t.str()).filter(|h| *h != 0),
                9 if el.text.is_some() => el.text = Some(t.str()),
                46 if complex => el.scale = t.f64(),
                50 if complex => el.rotation = t.f64(),
                44 if complex => el.offset[0] = t.f64(),
                45 if complex => el.offset[1] = t.f64(),
                _ => {}
            }
        }
        Some(Linetype { handle: r.handle(), name: g.name(2)?, description: g.s(3).unwrap_or_default(), length: g.f_or(40, 0.0), pattern })
    }
}

/// True when a record carries `AcadAnnotative` extended data with the annotative flag set.
fn annotative(tags: &[Tag]) -> bool {
    xdata_list(xdata(tags, "AcadAnnotative"), "AnnotativeData").iter().filter(|t| t.code == 1070).nth(1).is_some_and(|t| t.i64() != 0)
}

/// A STYLE record: a text style, or a shape file (which has no name).
#[derive(Clone, Debug, PartialEq)]
pub struct TextStyle {
    pub handle: Option<u64>,
    /// Empty for a shape file entry.
    pub name: String,
    /// Group 3: the font file (`txt.shx`, `arial.ttf`), or the shape file.
    pub font_file: String,
    /// Group 4.
    pub big_font: String,
    /// The typeface 2000-and-later files name in the style's `ACAD` extended data (how a
    /// TrueType style is stored); `None` for a CAD font.
    pub typeface: Option<String>,
    /// Group 40: the fixed height (0: none).
    pub height: f64,
    /// Group 41 (1 when absent).
    pub width_factor: f64,
    /// Group 50, in degrees.
    pub oblique: f64,
    /// Flags (70) bit 1: a shape file entry.
    pub shape_file: bool,
    /// Flags bit 4.
    pub vertical: bool,
    /// Group 71 bit 2.
    pub backwards: bool,
    /// Group 71 bit 4.
    pub upside_down: bool,
    pub annotative: bool,
}

impl TextStyle {
    /// Every STYLE record, the unnamed shape file entries included.
    pub fn read(r: &Record<'_>) -> Option<TextStyle> {
        let g = r.groups();
        let (flags, generation) = (g.i_or(70, 0), g.i_or(71, 0));
        let typeface = xdata(r.tags, "ACAD").iter().find(|t| t.code == 1000).map(|t| t.str().trim().to_string()).filter(|f| !f.is_empty());
        Some(TextStyle {
            handle: r.handle(),
            name: g.name(2).unwrap_or_default(),
            font_file: g.s(3).unwrap_or_default(),
            big_font: g.s(4).unwrap_or_default(),
            typeface,
            height: g.f_or(40, 0.0),
            width_factor: g.f_or(41, 1.0),
            oblique: g.f_or(50, 0.0),
            shape_file: flags & 1 != 0,
            vertical: flags & 4 != 0,
            backwards: generation & 2 != 0,
            upside_down: generation & 4 != 0,
            annotative: annotative(r.tags),
        })
    }
    /// The family of a TrueType style: [`TextStyle::typeface`], else the stem of a `.ttf`, `.otf`
    /// or `.ttc` font file. `None` for a CAD font (`.shx`).
    pub fn family(&self) -> Option<String> {
        if let Some(f) = &self.typeface {
            return Some(f.clone());
        }
        let file = self.font_file.trim().rsplit(['/', '\\']).next().unwrap_or_default();
        let (stem, ext) = file.rsplit_once('.')?;
        (matches!(ext.to_ascii_lowercase().as_str(), "ttf" | "otf" | "ttc") && !stem.is_empty()).then(|| stem.to_string())
    }
}

/// DIMSTYLE group code and the dimension variable it holds (DXF Reference, DIMSTYLE). The same
/// codes name variables in the `DSTYLE` override extended data of a dimension. Code 70 is
/// DIMTFILLCLR only after the record's flags, which use the code first.
const DIM_VARS: &[(i32, &str)] = &[
    (3, "DIMPOST"),
    (4, "DIMAPOST"),
    (40, "DIMSCALE"),
    (41, "DIMASZ"),
    (42, "DIMEXO"),
    (43, "DIMDLI"),
    (44, "DIMEXE"),
    (45, "DIMRND"),
    (46, "DIMDLE"),
    (47, "DIMTP"),
    (48, "DIMTM"),
    (49, "DIMFXL"),
    (50, "DIMJOGANG"),
    (69, "DIMTFILL"),
    (70, "DIMTFILLCLR"),
    (71, "DIMTOL"),
    (72, "DIMLIM"),
    (73, "DIMTIH"),
    (74, "DIMTOH"),
    (75, "DIMSE1"),
    (76, "DIMSE2"),
    (77, "DIMTAD"),
    (78, "DIMZIN"),
    (79, "DIMAZIN"),
    (90, "DIMARCSYM"),
    (140, "DIMTXT"),
    (141, "DIMCEN"),
    (142, "DIMTSZ"),
    (143, "DIMALTF"),
    (144, "DIMLFAC"),
    (145, "DIMTVP"),
    (146, "DIMTFAC"),
    (147, "DIMGAP"),
    (148, "DIMALTRND"),
    (170, "DIMALT"),
    (171, "DIMALTD"),
    (172, "DIMTOFL"),
    (173, "DIMSAH"),
    (174, "DIMTIX"),
    (175, "DIMSOXD"),
    (176, "DIMCLRD"),
    (177, "DIMCLRE"),
    (178, "DIMCLRT"),
    (179, "DIMADEC"),
    (271, "DIMDEC"),
    (272, "DIMTDEC"),
    (273, "DIMALTU"),
    (274, "DIMALTTD"),
    (275, "DIMAUNIT"),
    (276, "DIMFRAC"),
    (277, "DIMLUNIT"),
    (278, "DIMDSEP"),
    (279, "DIMTMOVE"),
    (280, "DIMJUST"),
    (281, "DIMSD1"),
    (282, "DIMSD2"),
    (283, "DIMTOLJ"),
    (284, "DIMTZIN"),
    (285, "DIMALTZ"),
    (286, "DIMALTTZ"),
    (288, "DIMUPT"),
    (289, "DIMATFIT"),
    (290, "DIMFXLON"),
    (340, "DIMTXSTY"),
    (341, "DIMLDRBLK"),
    (342, "DIMBLK"),
    (343, "DIMBLK1"),
    (344, "DIMBLK2"),
    (345, "DIMLTYPE"),
    (346, "DIMLTEX1"),
    (347, "DIMLTEX2"),
    (371, "DIMLWD"),
    (372, "DIMLWE"),
];

/// The dimension variable a DIMSTYLE group code holds.
pub fn dim_var_name(code: i32) -> Option<&'static str> {
    DIM_VARS.iter().find(|(c, _)| *c == code).map(|(_, n)| *n)
}

/// The DIMSTYLE group code of a dimension variable (any case, with or without `$`).
pub fn dim_var_code(name: &str) -> Option<i32> {
    let name = name.trim_start_matches('$');
    DIM_VARS.iter().find(|(_, n)| n.eq_ignore_ascii_case(name)).map(|(c, _)| *c)
}

/// A DIMSTYLE record: its name and the variables it sets.
#[derive(Clone, Debug)]
pub struct DimStyle<'a> {
    /// Group 105.
    pub handle: Option<u64>,
    pub name: String,
    /// The record's flags (the first group 70).
    pub flags: i64,
    pub annotative: bool,
    /// The variables in file order: name and the group that holds the value. Angles, handles
    /// and character codes are as the file stores them (DIMJOGANG in radians, DIMTXSTY and the
    /// arrow blocks as handles of STYLE and BLOCK_RECORD records, DIMDSEP as a character code).
    pub vars: Vec<(&'static str, &'a Tag)>,
}

impl<'a> DimStyle<'a> {
    pub fn read(r: &Record<'a>) -> Option<DimStyle<'a>> {
        let g = r.groups();
        // The first 70 is the record's flags; a later one is DIMTFILLCLR.
        let flags_at = r.tags.iter().position(|t| t.code == 70);
        let vars = r
            .tags
            .iter()
            .enumerate()
            .take_while(|(_, t)| t.code < 1000)
            .filter(|(i, _)| Some(*i) != flags_at)
            .filter_map(|(_, t)| Some((dim_var_name(t.code)?, t)))
            .collect();
        Some(DimStyle { handle: r.handle(), name: g.name(2)?, flags: g.i_or(70, 0), annotative: annotative(r.tags), vars })
    }
    /// The group holding variable `name`; the first.
    pub fn var(&self, name: &str) -> Option<&'a Tag> {
        let name = name.trim_start_matches('$');
        self.vars.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, t)| *t)
    }
}

/// A BLOCK_RECORD.
#[derive(Clone, Debug, PartialEq)]
pub struct BlockRecord {
    pub handle: Option<u64>,
    pub name: String,
    /// Group 340: the LAYOUT of a model or paper space block.
    pub layout: Option<u64>,
    /// Group 70: the insertion units (`$INSUNITS` codes).
    pub units: i64,
    /// Group 280 (explodable unless it is 0).
    pub explodable: bool,
    /// Group 281: scaled uniformly only.
    pub uniform_scale: bool,
}

impl BlockRecord {
    pub fn read(r: &Record<'_>) -> Option<BlockRecord> {
        let g = r.groups();
        Some(BlockRecord {
            handle: r.handle(),
            name: g.name(2)?,
            layout: g.handle(340).filter(|h| *h != 0),
            units: g.i_or(70, 0),
            explodable: g.i_or(280, 1) != 0,
            uniform_scale: g.i_or(281, 0) != 0,
        })
    }
}

/// A VIEW record: a named view.
#[derive(Clone, Debug, PartialEq)]
pub struct View {
    pub handle: Option<u64>,
    pub name: String,
    /// Flags (70) bit 1: a paper space view.
    pub paper_space: bool,
    /// Group 40.
    pub height: f64,
    /// Group 41.
    pub width: f64,
    /// Groups 10/20: the centre in display coordinates, whose origin is the target.
    pub center: [f64; 2],
    /// Groups 11/21/31: the direction from the target.
    pub direction: Point,
    /// Groups 12/22/32.
    pub target: Point,
    /// Group 50: the twist, in degrees.
    pub twist: f64,
}

impl View {
    pub fn read(r: &Record<'_>) -> Option<View> {
        let g = r.groups();
        Some(View {
            handle: r.handle(),
            name: g.name(2)?,
            paper_space: g.i_or(70, 0) & 1 != 0,
            height: g.f_or(40, 0.0),
            width: g.f_or(41, 0.0),
            center: [g.f_or(10, 0.0), g.f_or(20, 0.0)],
            direction: g.point_opt(11).unwrap_or([0.0, 0.0, 1.0]),
            target: g.point(12),
            twist: g.f_or(50, 0.0),
        })
    }
}

/// A UCS record: a named coordinate system.
#[derive(Clone, Debug, PartialEq)]
pub struct Ucs {
    pub handle: Option<u64>,
    pub name: String,
    pub origin: Point,
    pub x_axis: Point,
    pub y_axis: Point,
}

impl Ucs {
    pub fn read(r: &Record<'_>) -> Option<Ucs> {
        let g = r.groups();
        Some(Ucs { handle: r.handle(), name: g.name(2)?, origin: g.point(10), x_axis: g.point(11), y_axis: g.point(12) })
    }
}

/// A VPORT record: one tile of a viewport configuration (`*ACTIVE` is the one shown).
#[derive(Clone, Debug, PartialEq)]
pub struct Vport {
    pub handle: Option<u64>,
    pub name: String,
    /// Groups 10/20 and 11/21: the tile's corners as fractions of the drawing area.
    pub min: [f64; 2],
    pub max: [f64; 2],
    /// Groups 12/22: the view centre in display coordinates, when given.
    pub center: Option<[f64; 2]>,
    /// Groups 13/23, 14/24 and 15/25.
    pub snap_base: Option<[f64; 2]>,
    pub snap_spacing: Option<[f64; 2]>,
    pub grid_spacing: Option<[f64; 2]>,
    /// Groups 16/26/36: the direction from the target.
    pub direction: Point,
    /// Groups 17/27/37.
    pub target: Point,
    /// Group 40: the view height.
    pub height: f64,
    /// Group 41: the tile's aspect ratio.
    pub aspect: Option<f64>,
    /// Group 50: the snap rotation, in degrees.
    pub snap_angle: Option<f64>,
    /// Group 51: the twist, in degrees.
    pub twist: f64,
}

impl Vport {
    pub fn read(r: &Record<'_>) -> Option<Vport> {
        let g = r.groups();
        let pair = |x: i32| Some([g.f(x)?, g.f(x + 10)?]);
        Some(Vport {
            handle: r.handle(),
            name: g.name(2)?,
            min: [g.f_or(10, 0.0), g.f_or(20, 0.0)],
            max: [g.f_or(11, 0.0), g.f_or(21, 0.0)],
            center: pair(12),
            snap_base: pair(13),
            snap_spacing: pair(14),
            grid_spacing: pair(15),
            direction: g.point_opt(16).unwrap_or([0.0, 0.0, 1.0]),
            target: g.point(17),
            height: g.f_or(40, 0.0),
            aspect: g.f(41),
            snap_angle: g.f(50),
            twist: g.f_or(51, 0.0),
        })
    }
}

/// An APPID record: an application registered for extended data.
#[derive(Clone, Debug, PartialEq)]
pub struct AppId {
    pub handle: Option<u64>,
    pub name: String,
}

impl AppId {
    pub fn read(r: &Record<'_>) -> Option<AppId> {
        Some(AppId { handle: r.handle(), name: r.groups().name(2)? })
    }
}
