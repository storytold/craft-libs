//! ACAD_TABLE and TABLESTYLE.

use craft_dxf::model::table::{MAX_SIDE, Table, TableStyle};
use craft_dxf::model::write::{Drawing, Format};
use craft_dxf::model::{Color, Document, Entity, EntityKind, Lineweight, entity_records, records};
use craft_dxf::{Tag, parse};

fn tags(text: &str) -> Vec<Tag> {
    let lines: Vec<&str> = text.split(['\n', '|']).map(str::trim).filter(|l| !l.is_empty()).map(|l| if l == "~" { "" } else { l }).collect();
    parse((lines.join("\n") + "\n").as_bytes()).expect("the test drawing parses")
}

const TABLE: &str = "0|ACAD_TABLE|5|90|100|AcDbEntity|8|0|100|AcDbBlockReference|2|*T1|10|1.0|20|2.0|30|0.0|100|AcDbTable|280|0|342|5A|343|*T1|11|1.0|21|0.0|31|0.0
90|22|91|2|92|2|93|0|94|0|95|0|96|0|141|0.5|141|0.7|142|3.0|142|4.0
171|1|172|0|173|0|174|0|175|2|176|1|91|0|178|0|145|0.0|92|0|301|CELL_VALUE|93|6|90|4|1|Title|94|0|140|99.0|300|~|302|Title|304|ACVALUE_END|140|2.5|170|5|7|Bold
171|1|172|0|173|1|174|0|175|0|176|0|91|0|178|0
171|1|172|0|173|0|174|0|175|1|176|1|2|first-|3|part-|1|last
171|2|172|0|173|0|174|0|175|1|176|1|340|B1
1001|CADCRAFT|1000|TABLE";

fn read(text: &str) -> Table {
    let t = tags(text);
    let (e, _) = entity_records(&records(&t));
    let EntityKind::Table(table) = Entity::read(&e[0]).kind else { panic!("a table") };
    table
}

#[test]
fn a_table_and_its_cells() {
    let t = read(TABLE);
    assert_eq!((t.block.as_deref(), t.style, t.insert, t.direction), (Some("*T1"), Some(0x5A), [1.0, 2.0, 0.0], [1.0, 0.0, 0.0]));
    assert_eq!((t.rows, t.columns, t.row_heights.as_slice(), t.column_widths.as_slice()), (2, 2, [0.5, 0.7].as_slice(), [3.0, 4.0].as_slice()));
    assert_eq!(t.cells.len(), 4);
    let title = t.cell(0, 0).expect("a cell");
    // The 140 inside the value block is a number, not the height; the extended data ends the last cell.
    assert_eq!(
        (title.text.as_str(), title.span, title.merged, title.text_height, title.alignment, title.text_style.as_deref()),
        ("Title", Some((1, 2)), false, Some(2.5), Some(5), Some("Bold"))
    );
    assert_eq!((t.cell(0, 1).map(|c| c.merged), t.cell(0, 1).map(|c| c.span)), (Some(true), Some(Some((0, 0)))));
    assert_eq!(t.cell(1, 0).map(|c| c.text.as_str()), Some("first-part-last"));
    let block = t.cell(1, 1).expect("a cell");
    assert_eq!((block.cell_type, block.block, block.text.as_str()), (2, Some(0xB1), ""));
    assert_eq!((t.cell(2, 0), t.cell(usize::MAX, 2)), (None, None));
}

#[test]
fn a_size_a_table_cannot_have_keeps_nothing() {
    for (r, c) in [(0, 5), (5, 0), (MAX_SIDE + 1, 1), (2000, 2000)] {
        let t = read(&format!("0|ACAD_TABLE|100|AcDbTable|91|{r}|92|{c}|141|1.0|142|1.0|171|1|1|x"));
        assert_eq!((t.rows, t.columns, t.cells.len(), t.row_heights.len(), t.column_widths.len()), (0, 0, 0, 0, 0), "{r} x {c}");
    }
    // A large size the file backs with no cells costs nothing.
    let t = read("0|ACAD_TABLE|100|AcDbTable|91|1000|92|1000|171|1|1|only");
    assert_eq!((t.rows, t.columns, t.cells.len()), (1000, 1000, 1));
    // More cells than the size are left out.
    let t = read("0|ACAD_TABLE|100|AcDbTable|91|1|92|1|171|1|1|a|171|1|1|b");
    assert_eq!(t.cells.len(), 1);
    assert_eq!(read("0|ACAD_TABLE|91|-3|92|2").rows, 0);
}

#[test]
fn a_table_style() {
    let t = tags(
        "0|SECTION|2|OBJECTS|0|TABLESTYLE|5|5A|100|AcDbTableStyle|3|Plain|70|0|71|0|40|0.06|41|0.07|280|0|281|1
7|Standard|140|0.18|170|5|62|1|63|7|283|1|90|512|91|1|274|-2|275|50|284|1|285|0|64|2|65|0
7|Bold|140|0.25|170|2|62|0|283|0
7|Title|140|0.3
1001|CADCRAFT|1000|CELLSTYLES|0|ENDSEC|0|EOF",
    );
    let d = Document::read(&t);
    let s = TableStyle::read(d.objects_of("TABLESTYLE").next().expect("the style"));
    assert_eq!(
        (s.handle, s.description.as_str(), s.flow, s.margin_h, s.margin_v, s.title_suppressed, s.header_suppressed),
        (Some(0x5A), "Plain", 0, Some(0.06), Some(0.07), false, true)
    );
    assert_eq!(s.cell_types.len(), 3);
    let data = &s.cell_types[0];
    assert_eq!(
        (data.text_style.as_str(), data.text_height, data.alignment, data.text_color, data.fill),
        ("Standard", Some(0.18), Some(5), Some(Color::Index(1)), Some(Color::Index(7)))
    );
    assert_eq!(
        (data.borders[0].lineweight, data.borders[1].lineweight, data.borders[0].visible, data.borders[1].visible),
        (Some(Lineweight::ByBlock), Some(Lineweight::Hundredths(50)), Some(true), Some(false))
    );
    assert_eq!((data.borders[0].color, data.borders[1].color, data.borders[2].color), (Some(Color::Index(2)), Some(Color::ByBlock), None));
    let header = &s.cell_types[1];
    assert_eq!((header.text_style.as_str(), header.fill, header.text_color), ("Bold", None, Some(Color::ByBlock)));
    assert_eq!(s.cell_types[2].text_height, Some(0.3));
    let bare = tags("0|SECTION|2|OBJECTS|0|TABLESTYLE|0|ENDSEC|0|EOF");
    let empty = Document::read(&bare);
    assert!(TableStyle::read(empty.objects_of("TABLESTYLE").next().expect("the style")).cell_types.is_empty());
}

#[test]
fn the_writer_leaves_a_table_out_before_2004() {
    let t = tags(TABLE);
    let e = Entity::read(&entity_records(&records(&t)).0[0]);
    for (format, left_out) in [(Format::R2000, 1), (Format::R2004, 0)] {
        let (_, skipped) = Drawing { format, entities: vec![e.clone()], ..Default::default() }.tags();
        assert_eq!(skipped, left_out, "{format:?}");
    }
}
