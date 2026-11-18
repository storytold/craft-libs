//! The typed OBJECTS records and dimension style overrides.

use craft_dxf::model::objects::{DimAssoc, Group, ImageDef, MLineStyle, PlotSettings, Scale, SortEntsTable, SpatialFilter, dim_overrides};
use craft_dxf::model::{Color, Document, Entity, EntityKind};
use craft_dxf::{Tag, parse};

fn tags(text: &str) -> Vec<Tag> {
    let lines: Vec<&str> = text.split(['\n', '|']).map(str::trim).filter(|l| !l.is_empty()).map(|l| if l == "~" { "" } else { l }).collect();
    parse((lines.join("\n") + "\n").as_bytes()).expect("the test drawing parses")
}

const OBJECTS: &str = "
0|SECTION|2|OBJECTS
0|DICTIONARY|5|C|3|ACAD_GROUP|350|D|3|ACAD_MLINESTYLE|350|17
0|DICTIONARY|5|D|3|Doors|350|40
0|DICTIONARY|5|17|3|Wall|350|41
0|GROUP|5|40|330|D|100|AcDbGroup|300|All doors|70|0|71|0|340|A1|340|A2
0|MLINESTYLE|5|41|100|AcDbMlineStyle|2|WALL|70|273|3|A wall|62|5|51|45.0|52|90.0|71|2|49|0.5|62|1|6|DASHED|49|-0.5|62|256|6|BYLAYER
0|IMAGEDEF|5|42|100|AcDbRasterImageDef|90|0|1|plans/site.png|10|640.0|20|480.0|11|0.01|21|0.01|280|1|281|5
0|SORTENTSTABLE|5|43|330|44|100|AcDbSortentsTable|330|1F|331|A1|5|B|331|A2|5|A|331|zz|5|9|5|8
0|SPATIAL_FILTER|5|45|100|AcDbFilter|100|AcDbSpatialFilter|70|2|10|0.0|20|0.0|10|4.0|20|3.0|210|0.0|220|0.0|230|1.0|11|1.0|21|2.0|31|3.0|71|1|72|1|40|7.5|73|0
40|1.0|40|0.0|40|0.0|40|-5.0|40|0.0|40|1.0|40|0.0|40|-6.0|40|0.0|40|0.0|40|1.0|40|0.0
40|2.0|40|0.0|40|0.0|40|0.0|40|0.0|40|2.0|40|0.0|40|0.0|40|0.0|40|0.0|40|2.0|40|0.0
0|DIMASSOC|5|46|330|77|100|AcDbDimAssoc|330|D1|90|3|70|1|71|0
1|AcDbOsnapPointRef|72|1|331|A1|73|2|91|4|40|0.0|10|1.0|20|2.0|30|0.0
1|AcDbOsnapPointRef|72|6|331|A2|73|0|91|0|40|0.5|10|3.0|20|4.0|30|0.0|332|A3
0|SCALE|5|47|100|AcDbScale|70|0|300|1:50|140|1.0|141|50.0|290|0
0|SCALE|5|48|100|AcDbScale|300|broken|140|1.0
0|LAYOUT|5|49|100|AcDbPlotSettings|1|Final|2|printer.pc3|4|ISO_A3_(420.00_x_297.00_MM)|6|~|40|7.5|41|20.0|42|7.5|43|20.0|44|420.0|45|297.0|46|1.0|47|2.0|48|0.0|49|0.0|140|100.0|141|50.0|142|1.0|143|50.0|70|688|72|1|73|1|74|4|7|monochrome.ctb|75|0|147|0.02|148|3.0|149|4.0|100|AcDbLayout|1|Sheet|70|1|71|1|10|9.0
0|ENDSEC|0|EOF
";

#[test]
fn objects_typed() {
    let t = tags(OBJECTS);
    let d = Document::read(&t);
    let names = d.dictionary_names();
    assert_eq!((names.get(&0x40).copied(), names.get(&0x41).copied(), names.get(&0xD).copied()), (Some("Doors"), Some("Wall"), Some("ACAD_GROUP")));
    let one = |kind: &'static str| *d.objects_of(kind).next().expect("the record");

    let g = Group::read(&one("GROUP"));
    assert_eq!(
        (g.handle, g.description.as_str(), g.unnamed, g.selectable, g.members.as_slice()),
        (Some(0x40), "All doors", false, false, [0xA1, 0xA2].as_slice())
    );

    let m = MLineStyle::read(&one("MLINESTYLE")).expect("a style");
    assert_eq!(
        (m.name.as_str(), m.description.as_str(), m.flags, m.fill, m.start_angle, m.end_angle),
        ("WALL", "A wall", 273, Color::Index(5), 45.0, 90.0)
    );
    assert_eq!(m.elements.len(), 2);
    assert_eq!((m.elements[0].offset, m.elements[0].color, m.elements[0].linetype.as_str()), (0.5, Color::Index(1), "DASHED"));
    assert_eq!((m.elements[1].offset, m.elements[1].color), (-0.5, Color::ByLayer));

    let i = ImageDef::read(&one("IMAGEDEF"));
    assert_eq!(
        (i.handle, i.file.as_str(), i.size, i.pixel_size, i.loaded, i.units),
        (Some(0x42), "plans/site.png", [640.0, 480.0], [0.01, 0.01], true, 5)
    );

    let s = SortEntsTable::read(&one("SORTENTSTABLE")).expect("a table");
    // The block is the 330 after the marker, not the owning dictionary; a pair needs both handles.
    assert_eq!((s.block_record, s.entries.as_slice()), (Some(0x1F), [(0xA1, 0xB), (0xA2, 0xA)].as_slice()));

    let f = SpatialFilter::read(&one("SPATIAL_FILTER"));
    assert_eq!(
        (f.boundary.as_slice(), f.normal, f.origin, f.enabled, f.inverted),
        ([[0.0, 0.0], [4.0, 3.0]].as_slice(), [0.0, 0.0, 1.0], [1.0, 2.0, 3.0], true, false)
    );
    // The front clip distance is the 40 after its switch, not the first matrix value.
    assert_eq!((f.front, f.back), (Some(7.5), None));
    assert_eq!(f.inverse_insert.map(|m| (m[0], m[3], m[7])), Some((1.0, -5.0, -6.0)));
    assert_eq!(f.to_boundary.map(|m| (m[0], m[5], m[10])), Some((2.0, 2.0, 2.0)));

    let a = DimAssoc::read(&one("DIMASSOC"));
    assert_eq!((a.handle, a.dimension, a.flags, a.trans_space, a.refs.len()), (Some(0x46), Some(0xD1), 3, true, 2));
    assert_eq!((a.refs[0].snap, a.refs[0].entity, a.refs[0].sub_type, a.refs[0].sub_index, a.refs[0].point), (1, Some(0xA1), 2, 4, [1.0, 2.0, 0.0]));
    assert_eq!((a.refs[1].snap, a.refs[1].other, a.refs[1].near), (6, Some(0xA3), 0.5));

    let scales: Vec<Scale> = d.objects_of("SCALE").filter_map(Scale::read).collect();
    assert_eq!(scales.len(), 1, "a scale without both numbers is none");
    assert_eq!((scales[0].name.as_str(), scales[0].paper, scales[0].drawing, scales[0].unit), ("1:50", 1.0, 50.0, false));

    let p = PlotSettings::read(&one("LAYOUT"));
    // The plot settings' own 1, 70 and 10, not the layout's after them.
    assert_eq!((p.name.as_str(), p.device.as_str(), p.paper.as_str(), p.view.as_str()), ("Final", "printer.pc3", "ISO_A3_(420.00_x_297.00_MM)", ""));
    assert_eq!((p.margins, p.paper_size, p.origin, p.window), ([7.5, 20.0, 7.5, 20.0], [420.0, 297.0], [1.0, 2.0], [0.0, 0.0, 100.0, 50.0]));
    assert_eq!((p.scale, p.flags, p.paper_units, p.rotation, p.plot_type), ([1.0, 50.0], 688, 1, 1, 4));
    assert_eq!((p.style_sheet.as_str(), p.standard_scale, p.scale_factor, p.image_origin), ("monochrome.ctb", 0, 0.02, [3.0, 4.0]));
}

#[test]
fn damaged_objects_read_as_what_they_hold() {
    let t = tags(
        "0|SECTION|2|OBJECTS|0|GROUP|0|MLINESTYLE|0|IMAGEDEF|0|SORTENTSTABLE|0|SPATIAL_FILTER|72|1|0|DIMASSOC|1|x|0|SCALE|0|LAYOUT|0|DICTIONARY|3|a|350|zz|0|ENDSEC|0|EOF",
    );
    let d = Document::read(&t);
    let one = |kind: &'static str| *d.objects_of(kind).next().expect("the record");
    assert!(Group::read(&one("GROUP")).members.is_empty());
    assert!(MLineStyle::read(&one("MLINESTYLE")).is_none());
    assert_eq!(ImageDef::read(&one("IMAGEDEF")).file, "");
    assert!(SortEntsTable::read(&one("SORTENTSTABLE")).is_none());
    let f = SpatialFilter::read(&one("SPATIAL_FILTER"));
    assert_eq!((f.front, f.inverse_insert, f.to_boundary, f.boundary.len()), (None, None, None, 0));
    assert_eq!(DimAssoc::read(&one("DIMASSOC")).refs.len(), 1);
    assert!(Scale::read(&one("SCALE")).is_none());
    assert_eq!(PlotSettings::read(&one("LAYOUT")).plot_type, 5);
    assert!(d.dictionary_names().is_empty());
}

#[test]
fn dimension_style_overrides() {
    let t = tags(
        "0|DIMENSION|70|1|1001|OTHER|1000|DSTYLE|1002|{|1070|41|1040|9.0|1002|}|1001|ACAD|1000|DSTYLE|1002|{|1070|41|1040|2.5|1070|9999|1070|1|1040|7.0|1070|340|1005|2F|1070|147|1002|}|1000|after",
    );
    let o = dim_overrides(&t[1..]);
    // The ACAD list only; a code that names no variable is passed over with its value, and a
    // stray group realigns on the next 1070.
    assert_eq!(o.iter().map(|(n, v)| (*n, v.str())).collect::<Vec<_>>(), [("DIMASZ", "2.5".to_string()), ("DIMTXSTY", "2F".to_string())]);
    assert!(dim_overrides(&tags("0|DIMENSION|70|1")).is_empty());
}

#[test]
fn helix() {
    let t = tags(
        "0|SECTION|2|ENTITIES|0|HELIX|100|AcDbEntity|8|0|100|AcDbSpline|70|0|71|3|40|0.0|40|1.0|10|1.0|20|0.0|30|0.0|10|0.0|20|1.0|30|4.0|100|AcDbHelix|90|29|91|63|10|0.5|20|0.5|30|0.0|11|1.5|21|0.5|31|0.0|12|0.0|22|0.0|32|1.0|40|1.0|41|3.0|42|2.0|290|0|280|1|0|ENDSEC|0|EOF",
    );
    let d = Document::read(&t);
    let EntityKind::Helix(h) = Entity::read(&d.entities[0]).kind else { panic!("a helix") };
    // The spline's 10s and 40s and the helix's own are kept apart by their subclasses.
    assert_eq!((h.spline.control.len(), h.spline.knots.len(), h.spline.degree), (2, 2, 3));
    assert_eq!((h.axis_base, h.start, h.axis), ([0.5, 0.5, 0.0], [1.5, 0.5, 0.0], [0.0, 0.0, 1.0]));
    assert_eq!((h.radius, h.turns, h.turn_height, h.ccw, h.constrain), (1.0, 3.0, 2.0, false, 1));
}

#[test]
fn mline() {
    let t = tags("0|SECTION|2|ENTITIES|0|MLINE|100|AcDbEntity|8|0|100|AcDbMline|2|WALL|340|41|40|2.0|70|1|71|3|72|2|73|2|10|0.0|20|0.0|30|0.0|210|0.0|220|0.0|230|1.0
        11|0.0|21|0.0|31|0.0|12|1.0|22|0.0|32|0.0|13|0.0|23|1.0|33|0.0|74|3|41|0.5|41|1.0|41|2.0|75|0|74|1|41|-0.5|75|1|42|7.0
        11|4.0|21|0.0|31|0.0|12|1.0|22|0.0|32|0.0|13|0.0|23|1.0|33|0.0|74|1|41|0.5|75|0|74|1|41|-0.5|75|0|0|ENDSEC|0|EOF");
    let d = Document::read(&t);
    let EntityKind::MLine(m) = Entity::read(&d.entities[0]).kind else { panic!("a multiline") };
    assert_eq!(
        (m.style.as_str(), m.style_handle, m.scale, m.justification, m.flags, m.elements, m.start),
        ("WALL", Some(0x41), 2.0, 1, 3, 2, [0.0; 3])
    );
    assert_eq!(m.vertices.len(), 2);
    let v = &m.vertices[0];
    assert_eq!((v.location, v.direction, v.miter), ([0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]));
    assert_eq!(v.element_params, [vec![0.5, 1.0, 2.0], vec![-0.5]]);
    assert_eq!(v.fill_params, [vec![], vec![7.0]], "42s go to the list their 75 started");
    assert_eq!(m.vertices[1].location, [4.0, 0.0, 0.0]);
}
