//! The drawing model: sections, header variables, tables, blocks, layouts.

use craft_dxf::model::{Color, Dictionary, Document, Groups, Lineweight, Transparency, aci, dim_var_code, dim_var_name, ocs_to_wcs, parse_handle};
use craft_dxf::{Tag, Version, parse};

/// The tags of `text`, whose lines alternate group code and value; `|` also ends a line, so a
/// record fits on one, and `~` is an empty value.
fn tags(text: &str) -> Vec<Tag> {
    let lines: Vec<&str> = text.split(['\n', '|']).map(str::trim).filter(|l| !l.is_empty()).map(|l| if l == "~" { "" } else { l }).collect();
    parse((lines.join("\n") + "\n").as_bytes()).expect("the test drawing parses")
}

const DRAWING: &str = "
0|SECTION|2|HEADER
9|$ACADVER|1|AC1027
9|$INSUNITS|70|4
9|$LTSCALE|40|0.0
9|$EXTMIN|10|1.5|20|2.5|30|0.0
9|$HANDSEED|5|FFFF
0|ENDSEC
0|SECTION|2|TABLES
0|TABLE|2|LAYER|70|2
0|LAYER|5|10|2|Walls|70|5|62|-3|6|DASHED|370|35|290|0|390|F
1001|AcAecLayerStandard|1000|~|1000|outer walls
1001|AcCmTransparency|1071|33554559
0|LAYER|5|11|2|True|62|7|420|16711935
0|LAYER|5|12|70|0
0|ENDTAB
0|TABLE|2|LTYPE
0|LTYPE|5|14|2|GAS|3|Gas line|73|3|40|1.0|49|0.5|74|0|49|-0.2|74|2|75|0|340|20|46|0.1|50|15.0|44|-0.1|45|-0.05|9|GAS|49|-0.25|74|0
0|ENDTAB
0|TABLE|2|STYLE
0|STYLE|5|20|2|Notes|70|4|40|2.5|41|0.8|50|15.0|71|6|3|arial.ttf|4|~
0|STYLE|5|21|2|Face|3|~|1001|ACAD|1000|Noto Sans|1071|0
0|STYLE|5|22|2|~|70|1|3|ltypeshp.shx
0|STYLE|5|23|2|Cad|3|txt.shx
0|ENDTAB
0|TABLE|2|DIMSTYLE
0|DIMSTYLE|105|30|2|ISO-25|70|0|41|2.5|278|44|340|20|69|2|70|5
1001|ACAD|1000|ignored|1040|9.0
0|ENDTAB
0|TABLE|2|BLOCK_RECORD
0|BLOCK_RECORD|5|40|2|*Model_Space|340|50
0|BLOCK_RECORD|5|41|2|*Paper_Space|340|51
0|BLOCK_RECORD|5|42|2|DOOR|70|4|280|0|281|1
0|ENDTAB
0|TABLE|2|VIEW
0|VIEW|5|60|2|Front|70|1|40|10.0|41|20.0|10|1.0|20|2.0|12|5.0|22|6.0|32|7.0|50|30.0
0|ENDTAB
0|TABLE|2|UCS
0|UCS|5|61|2|Site|10|1.0|20|2.0|30|3.0|11|0.0|21|1.0|31|0.0|12|-1.0|22|0.0|32|0.0
0|ENDTAB
0|TABLE|2|VPORT
0|VPORT|5|62|2|*ACTIVE|10|0.0|20|0.0|11|1.0|21|1.0|12|4.0|22|5.0|14|0.5|24|0.5|40|9.0|41|1.5|51|10.0
0|ENDTAB
0|TABLE|2|APPID
0|APPID|5|63|2|ACAD
0|ENDTAB
0|ENDSEC
0|SECTION|2|BLOCKS
0|LINE|8|stray
0|BLOCK|5|70|330|42|2|DOOR|70|2|10|1.0|20|2.0|30|3.0|4|A door
0|POLYLINE|5|71|66|1|70|1
0|VERTEX|10|0.0|20|0.0
0|VERTEX|10|1.0|20|0.0
0|SEQEND|5|72
0|VERTEX|10|9.0
0|ENDBLK|5|73
0|BLOCK|5|74|330|99|3|NOTABLE|70|4|1|other.dwg
0|ENDBLK
0|BLOCK|2|OPEN
0|CIRCLE|40|1.0
0|ENDSEC
0|LINE|8|between
0|SECTION|2|ENTITIES
0|INSERT|5|80|2|DOOR|66|1
0|ATTRIB|1|one
0|ATTRIB|1|two
0|SEQEND
0|INSERT|5|81|2|DOOR
0|SEQEND
0|ATTRIB|1|stray
0|ENDSEC
0|SECTION|2|OBJECTS
0|DICTIONARY|5|C|3|ACAD_GROUP|350|D|3|ACAD_LAYOUT|350|1A
0|LAYOUT|5|51|330|1A|100|AcDbPlotSettings|1|page setup|10|9.0|100|AcDbLayout|1|Sheet 1|70|1|71|1|10|0.0|20|0.0|11|420.0|21|297.0|330|41
0|LAYOUT|5|50|100|AcDbLayout|1|Model|71|0|330|40
0|ENDSEC
0|SECTION|2|ENTITIES
0|LINE|5|82|67|1
0|ENDSEC
0|SECTION|2|ACDSDATA
0|ACDSRECORD|90|1
0|ENDSEC
0|EOF
";

#[test]
fn the_sections_of_a_drawing() {
    let t = tags(DRAWING);
    let d = Document::read(&t);
    assert_eq!(d.sections, 7);
    assert_eq!(d.header.vars.len(), 5);
    assert_eq!(d.header.acadver().as_deref(), Some("AC1027"));
    assert_eq!(d.header.version(), Some(Version::R2013));
    assert_eq!(d.header.insunits(), 4);
    assert_eq!(d.header.measurement(), None);
    assert_eq!(d.header.ltscale(), 1.0, "a scale of 0 is none");
    assert_eq!(d.header.lwdefault(), 25);
    assert_eq!(d.header.point("extmin"), Some([1.5, 2.5, 0.0]));
    assert_eq!(d.header.handseed(), Some(0xFFFF));
    assert_eq!(d.header.f("$NOSUCH"), None);
    // Entities of both ENTITIES sections, in order; the second INSERT takes no SEQEND.
    let kinds: Vec<&str> = d.entities.iter().map(|e| e.kind).collect();
    assert_eq!(kinds, ["INSERT", "INSERT", "LINE"]);
    assert_eq!(d.entities[0].followers.len(), 2);
    assert!(d.entities[0].seqend.is_some());
    assert!(d.entities[1].followers.is_empty() && d.entities[1].seqend.is_none());
    assert!(d.entities[2].paper_space() && !d.entities[0].paper_space());
    assert_eq!(d.other_sections.len(), 1);
    assert_eq!(d.other_sections[0].name, "ACDSDATA");
    // What was left out, by section: the LINE between sections, the LINE outside a block and the
    // VERTEX after a SEQEND, the ATTRIB after an INSERT without attributes.
    assert_eq!(d.skipped, [("(no section)", 1), ("BLOCKS", 2), ("ENTITIES", 1)]);
}

#[test]
fn blocks_hold_their_entities_up_to_endblk() {
    let t = tags(DRAWING);
    let d = Document::read(&t);
    let names: Vec<&str> = d.blocks.iter().map(|b| b.name).collect();
    assert_eq!(names, ["DOOR", "NOTABLE", "OPEN"]);
    let door = d.block("door").expect("found in any case");
    assert_eq!(door.base(), [1.0, 2.0, 3.0]);
    assert_eq!(door.description().as_deref(), Some("A door"));
    assert!(!door.is_xref());
    assert_eq!(door.block_record(), Some(0x42));
    assert_eq!(door.entities.len(), 1);
    assert_eq!(door.entities[0].followers.len(), 2);
    assert_eq!(door.entities[0].seqend.map(|s| s.handle()), Some(Some(0x72)));
    assert_eq!(door.end.map(|e| e.handle()), Some(Some(0x73)));
    let xref = &d.blocks[1];
    assert!(xref.is_xref());
    assert_eq!(xref.xref_path().as_deref(), Some("other.dwg"));
    // A block the section ends in keeps what it has.
    assert_eq!(d.blocks[2].entities.len(), 1);
    assert!(d.blocks[2].end.is_none());
    // Block record handle to name: the table's, and a BLOCK's own 330 where the table has none.
    let by_record = d.block_names();
    assert_eq!(by_record.get(&0x42).copied(), Some("DOOR"));
    assert_eq!(by_record.get(&0x99).copied(), Some("NOTABLE"));
    assert_eq!(d.table_names("STYLE").get(&0x20).copied(), Some("Notes"));
}

#[test]
fn layers() {
    let t = tags(DRAWING);
    let layers = Document::read(&t).layers();
    assert_eq!(layers.len(), 2, "a layer without a name is no layer");
    let w = &layers[0];
    assert_eq!((w.handle, w.name.as_str(), w.color), (Some(0x10), "Walls", Color::Index(3)));
    assert!(!w.on && w.frozen && w.locked && !w.vp_freeze_new && !w.plot);
    assert_eq!((w.linetype.as_str(), w.lineweight, w.plot_style), ("DASHED", Lineweight::Hundredths(35), Some(0xF)));
    assert_eq!(w.transparency, Transparency::Alpha(127));
    assert_eq!(w.description.as_deref(), Some("outer walls"));
    let tc = &layers[1];
    assert_eq!(tc.color, Color::Rgb([255, 0, 255]));
    assert!(tc.on && tc.plot && !tc.frozen);
    assert_eq!((tc.linetype.as_str(), tc.lineweight, tc.transparency), ("Continuous", Lineweight::Default, Transparency::ByLayer));
}

#[test]
fn linetypes_with_text_elements() {
    let t = tags(DRAWING);
    let lt = Document::read(&t).linetypes();
    let gas = &lt[0];
    assert_eq!((gas.name.as_str(), gas.description.as_str(), gas.length), ("GAS", "Gas line", 1.0));
    let lengths: Vec<f64> = gas.pattern.iter().map(|e| e.length).collect();
    assert_eq!(lengths, [0.5, -0.2, -0.25]);
    let text = &gas.pattern[1];
    assert_eq!(text.text.as_deref(), Some("GAS"));
    assert_eq!((text.style, text.scale, text.rotation, text.offset), (Some(0x20), 0.1, 15.0, [-0.1, -0.05]));
    assert!(gas.pattern[0].text.is_none() && gas.pattern[0].shape.is_none());
}

#[test]
fn text_styles_and_their_fonts() {
    let t = tags(DRAWING);
    let st = Document::read(&t).text_styles();
    assert_eq!(st.len(), 4, "the unnamed shape file entry is kept");
    let notes = &st[0];
    assert_eq!((notes.height, notes.width_factor, notes.oblique), (2.5, 0.8, 15.0));
    assert!(notes.vertical && notes.backwards && notes.upside_down && !notes.shape_file && !notes.annotative);
    assert_eq!(notes.family().as_deref(), Some("arial"));
    assert_eq!(st[1].family().as_deref(), Some("Noto Sans"), "the typeface of the ACAD extended data");
    assert!(st[2].shape_file && st[2].name.is_empty());
    assert_eq!(st[3].family(), None, "a CAD font has no family");
}

#[test]
fn dimension_styles_name_their_variables() {
    let t = tags(DRAWING);
    let ds = Document::read(&t).dim_styles();
    let iso = &ds[0];
    assert_eq!((iso.handle, iso.name.as_str(), iso.flags), (Some(0x30), "ISO-25", 0), "the handle is group 105");
    let names: Vec<&str> = iso.vars.iter().map(|(n, _)| *n).collect();
    assert_eq!(names, ["DIMASZ", "DIMDSEP", "DIMTXSTY", "DIMTFILL", "DIMTFILLCLR"], "the first 70 is the flags; extended data is no variable");
    assert_eq!(iso.var("$dimasz").map(Tag::f64), Some(2.5));
    assert_eq!(iso.var("DIMTFILLCLR").map(Tag::i64), Some(5));
    assert_eq!((dim_var_name(147), dim_var_code("dimgap")), (Some("DIMGAP"), Some(147)));
    assert_eq!((dim_var_name(1), dim_var_code("DIMNOPE")), (None, None));
}

#[test]
fn the_other_tables() {
    let t = tags(DRAWING);
    let d = Document::read(&t);
    let br = d.block_records();
    assert_eq!(br.len(), 3);
    assert_eq!((br[0].layout, br[0].explodable), (Some(0x50), true));
    assert_eq!((br[2].units, br[2].explodable, br[2].uniform_scale), (4, false, true));
    let v = &d.views()[0];
    assert!(v.paper_space);
    assert_eq!((v.height, v.width, v.center, v.target, v.direction, v.twist), (10.0, 20.0, [1.0, 2.0], [5.0, 6.0, 7.0], [0.0, 0.0, 1.0], 30.0));
    let u = &d.ucss()[0];
    assert_eq!((u.origin, u.x_axis, u.y_axis), ([1.0, 2.0, 3.0], [0.0, 1.0, 0.0], [-1.0, 0.0, 0.0]));
    let vp = &d.vports()[0];
    assert_eq!(
        (vp.name.as_str(), vp.max, vp.center, vp.snap_spacing, vp.grid_spacing),
        ("*ACTIVE", [1.0, 1.0], Some([4.0, 5.0]), Some([0.5, 0.5]), None)
    );
    assert_eq!((vp.height, vp.aspect, vp.twist, vp.snap_angle), (9.0, Some(1.5), 10.0, None));
    assert_eq!(d.app_ids()[0].name, "ACAD");
}

#[test]
fn layouts_and_dictionaries() {
    let t = tags(DRAWING);
    let d = Document::read(&t);
    let layouts = d.layouts();
    assert_eq!(layouts.len(), 2);
    let sheet = &layouts[0];
    // The name and limits are the layout's own, not the plot settings' 1 and 10 before them; the
    // block record is the 330 among its own groups, not its dictionary's.
    assert_eq!((sheet.name.as_str(), sheet.tab_order, sheet.block_record), ("Sheet 1", 1, Some(0x41)));
    assert_eq!(sheet.limits, Some(([0.0, 0.0], [420.0, 297.0])));
    assert_eq!(d.block_names().get(&0x41).copied(), Some("*Paper_Space"));
    assert_eq!((layouts[1].name.as_str(), layouts[1].limits), ("Model", None));
    let root = d.root_dictionary().expect("the first dictionary");
    assert_eq!(root.get("acad_layout"), Some(0x1A));
    assert_eq!(root.entries.len(), 2);
    assert_eq!(d.dictionary(0xC), Some(root));
    assert_eq!(d.dictionary(0xD), None);
    assert_eq!(Dictionary::default().get("x"), None);
}

#[test]
fn repeated_points_keep_their_own_coordinates() {
    // The second point has no z: it must not take the third's.
    let t = tags("0|LEADER|10|1.0|20|2.0|30|3.0|10|4.0|20|5.0|10|6.0|20|7.0|30|8.0");
    let g = Groups(&t);
    assert_eq!(g.points(10), [[1.0, 2.0, 3.0], [4.0, 5.0, 0.0], [6.0, 7.0, 8.0]]);
    assert_eq!(g.point(10), [1.0, 2.0, 3.0]);
    assert_eq!(g.point_opt(11), None);
    assert_eq!(g.all_f(20), [2.0, 5.0, 7.0]);
}

#[test]
fn subclass_groups_and_application_groups() {
    let t = tags("0|X|5|1F|102|{ACAD_REACTORS|330|AA|102|}|330|BB|100|AcDbEntity|8|L|100|AcDbLine|10|1.0|100|AcDbOther|10|2.0");
    let g = Groups(&t[1..]);
    assert_eq!(g.subclass("AcDbLine").f(10), Some(1.0));
    assert_eq!(g.subclass("AcDbOther").f(10), Some(2.0));
    assert!(g.subclass("AcDbNone").0.is_empty());
    // The owner is the 330 outside the reactors group.
    let d = craft_dxf::model::records(&t);
    assert_eq!((d[0].handle(), d[0].owner()), (Some(0x1F), Some(0xBB)));
}

#[test]
fn colours_lineweights_and_transparency() {
    assert_eq!(Color::from_index(0), Color::ByBlock);
    assert_eq!(Color::from_index(256), Color::ByLayer);
    assert_eq!(Color::from_index(-5), Color::ByLayer);
    assert_eq!(Color::from_index(255), Color::Index(255));
    assert_eq!(Color::from_true(0x7F00_FF80), Color::Rgb([0, 255, 128]), "the top byte is not a channel");
    let t = tags("0|LINE|62|1|420|255");
    assert_eq!(Color::of(&Groups(&t)), Color::Rgb([0, 0, 255]), "a true colour wins over its index");
    for v in [-1, -2, -3, 0, 25, 211] {
        assert_eq!(Lineweight::from_dxf(v).to_dxf(), v);
    }
    assert_eq!(Lineweight::from_dxf(-9), Lineweight::ByLayer);
    assert_eq!(Lineweight::from_dxf(1 << 40), Lineweight::ByLayer);
    assert_eq!(Transparency::from_dxf(0x0100_0000), Transparency::ByBlock);
    assert_eq!(Transparency::from_dxf(0x0200_0080), Transparency::Alpha(128));
    assert_eq!(Transparency::from_dxf(0), Transparency::ByLayer);
    assert_eq!(Transparency::Alpha(128).to_dxf(), 0x0200_0080);
    assert_eq!(aci::rgb(5), [0, 0, 255]);
    assert_eq!((parse_handle(" 1f "), parse_handle("xyz"), parse_handle("")), (Some(31), None, None));
}

#[test]
fn object_coordinates() {
    let close = |a: [f64; 3], b: [f64; 3]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-12);
    assert!(close(ocs_to_wcs([0.0, 0.0, 1.0], [1.0, 2.0, 3.0]), [1.0, 2.0, 3.0]));
    // The Z axis turned over: x runs the other way and the elevation is below.
    assert!(close(ocs_to_wcs([0.0, 0.0, -1.0], [1.0, 2.0, 3.0]), [-1.0, 2.0, -3.0]));
    // A plane facing +X: its X axis is world Y, its Y axis world Z.
    assert!(close(ocs_to_wcs([1.0, 0.0, 0.0], [1.0, 2.0, 3.0]), [3.0, 1.0, 2.0]));
    // No direction at all is the world system, not a division by zero.
    assert!(close(ocs_to_wcs([0.0, 0.0, 0.0], [1.0, 2.0, 3.0]), [1.0, 2.0, 3.0]));
    assert!(close(ocs_to_wcs([f64::NAN, 0.0, 1.0], [1.0, 2.0, 3.0]), [1.0, 2.0, 3.0]));
}

#[test]
fn damaged_input_reads_as_what_it_holds() {
    for text in [
        "0|SECTION",
        "0|SECTION|2|BLOCKS|0|BLOCK",
        "0|SECTION|2|TABLES|0|LAYER|0|LTYPE|0|STYLE|0|DIMSTYLE|0|VIEW|0|UCS|0|VPORT|0|APPID|0|BLOCK_RECORD",
        "9|$X",
        "0|SECTION|2|OBJECTS|0|LAYOUT|0|DICTIONARY|3|a|3|b|350|zz|350|1",
    ] {
        let t = tags(text);
        let d = Document::read(&t);
        let _ = (d.layers(), d.linetypes(), d.text_styles(), d.dim_styles(), d.block_records(), d.views(), d.ucss(), d.vports(), d.app_ids());
        let _ = (d.layouts(), d.root_dictionary(), d.block_names(), d.header.version());
        assert!(d.layers().is_empty() && d.layouts().is_empty());
    }
    let none: Vec<Tag> = Vec::new();
    assert_eq!(Document::read(&none).sections, 0);
}
