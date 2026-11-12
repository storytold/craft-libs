//! MULTILEADER, ACAD_TABLE and their styles through the writer: what the model reads from a
//! file, written as ASCII DXF and read again, is what it was.

use craft_dxf::model::mleader::{LeaderRoot, MLeader, MLeaderStyle};
use craft_dxf::model::table::{Border, CellTypeStyle, Table, TableCell, TableStyle};
use craft_dxf::model::write::{BlockDef, Drawing, Format};
use craft_dxf::model::{Color, Common, Document, Entity, EntityKind, Lineweight, Transparency};
use craft_dxf::{Tag, parse, write_ascii_as};

fn tags(text: &str) -> Vec<Tag> {
    let lines: Vec<&str> = text.split(['\n', '|']).map(str::trim).filter(|l| !l.is_empty()).map(|l| if l == "~" { "" } else { l }).collect();
    parse((lines.join("\n") + "\n").as_bytes()).expect("the test drawing parses")
}

fn common() -> Common {
    Common {
        handle: None,
        owner: None,
        layer: "Notes".into(),
        linetype: None,
        color: Color::ByLayer,
        lineweight: Lineweight::ByLayer,
        ltscale: 1.0,
        visible: true,
        paper_space: false,
        transparency: Transparency::ByLayer,
        thickness: 0.0,
        extrusion: [0.0, 0.0, 1.0],
        plot_style: None,
    }
}

/// `d` written as ASCII text in its format and read back.
fn through_text(d: &Drawing) -> Vec<Tag> {
    let (t, skipped) = d.tags();
    assert_eq!(skipped, 0);
    parse(write_ascii_as(&t, d.format.utf8()).as_bytes()).expect("the written file parses")
}

/// What #986's reader takes from a multileader with every value it has.
fn mleader() -> MLeader {
    MLeader {
        style: Some(0x60),
        leader_type: Some(2),
        dogleg_on: Some(true),
        dogleg: Some(0.36),
        arrow_size: Some(0.18),
        content_type: Some(2),
        text_style: Some(0x61),
        block: Some(0x62),
        block_scale: Some([2.0, 2.0, 1.0]),
        block_rotation: Some(0.5),
        left_attachment: Some(1),
        right_attachment: Some(3),
        attribs: vec![(0xAD, "value one".into()), (0xAE, "value two".into())],
        base: Some([5.0, 5.0, 0.5]),
        text_height: Some(2.5),
        context_arrow_size: Some(0.2),
        landing_gap: Some(0.09),
        has_text: Some(true),
        text: Some("first\nsecond ^ caret".into()),
        context_text_style: Some(0x61),
        text_location: Some([6.0, 5.0, 0.0]),
        text_direction: Some([1.0, 0.0, 0.0]),
        text_rotation: Some(0.25),
        text_width: Some(20.0),
        line_spacing: Some(1.25),
        line_spacing_style: Some(2),
        text_alignment: Some(3),
        context_left_attachment: Some(1),
        context_right_attachment: Some(2),
        has_block: Some(false),
        context_block: Some(0x62),
        block_location: Some([7.0, 8.0, 0.0]),
        context_block_scale: Some([2.0, 2.0, 1.0]),
        context_block_rotation: Some(0.5),
        roots: vec![
            LeaderRoot {
                landing: Some([4.0, 5.0, 0.0]),
                direction: Some([1.0, 0.0, 0.0]),
                dogleg: Some(0.36),
                lines: vec![vec![[0.0, 0.0, 0.0], [2.0, 3.0, 1.0]], vec![[0.0, 9.0, 0.0]]],
            },
            LeaderRoot { landing: Some([8.0, 5.0, 0.0]), direction: Some([-1.0, 0.0, 0.0]), dogleg: Some(0.5), lines: vec![] },
        ],
    }
}

fn mleader_style() -> MLeaderStyle {
    MLeaderStyle {
        handle: Some(0x60),
        description: "Notes".into(),
        content_type: 2,
        leader_type: 2,
        landing_gap: 0.09,
        dogleg: 0.36,
        arrow_size: 0.18,
        text_height: 2.5,
        text_style: Some(0x61),
        block: Some(0x62),
        annotative: true,
    }
}

fn table() -> Table {
    let cell = |text: &str, span: (u32, u32), merged: bool| TableCell {
        cell_type: 1,
        text: text.into(),
        span: Some(span),
        merged,
        text_height: Some(2.5),
        alignment: Some(5),
        text_style: Some("Bold".into()),
        block: None,
    };
    Table {
        block: Some("*T1".into()),
        style: Some(0x70),
        insert: [1.0, 2.0, 0.0],
        direction: [1.0, 0.0, 0.0],
        rows: 2,
        columns: 2,
        row_heights: vec![0.5, 0.7],
        column_widths: vec![3.0, 4.0],
        cells: vec![
            cell("Title", (1, 2), false),
            cell("", (0, 0), true),
            // Longer than one string group: it goes out in pieces and comes back whole.
            cell(&"long ".repeat(80), (1, 1), false),
            TableCell { cell_type: 2, block: Some(0x62), ..cell("", (1, 1), false) },
        ],
    }
}

fn table_style() -> TableStyle {
    let border = |k: i64| Border {
        lineweight: Some(Lineweight::Hundredths(u16::try_from(k * 5).unwrap_or(0))),
        visible: Some(k % 2 == 0),
        color: Some(Color::Index(u8::try_from(k + 1).unwrap_or(1))),
    };
    let cell_type = |style: &str, fill: Option<Color>| CellTypeStyle {
        text_style: style.into(),
        text_height: Some(0.18),
        alignment: Some(5),
        text_color: Some(Color::ByBlock),
        fill,
        borders: [0, 1, 2, 3, 4, 5].map(border),
    };
    TableStyle {
        handle: Some(0x70),
        description: "Plain".into(),
        flow: 1,
        margin_h: Some(0.06),
        margin_v: Some(0.07),
        title_suppressed: false,
        header_suppressed: true,
        cell_types: vec![cell_type("Standard", Some(Color::Index(7))), cell_type("Bold", None), cell_type("Title", Some(Color::Index(3)))],
    }
}

fn drawing(format: Format) -> Drawing<'static> {
    let e = |kind| Entity { common: common(), kind };
    Drawing {
        format,
        blocks: vec![BlockDef { name: "*T1".into(), ..BlockDef::default() }],
        entities: vec![e(EntityKind::MLeader(mleader())), e(EntityKind::Table(table()))],
        mleader_styles: vec![("Notes".into(), mleader_style())],
        table_styles: vec![("Plain".into(), table_style())],
        ..Drawing::default()
    }
}

#[test]
fn a_multileader_and_a_table_read_back_as_they_were_written() {
    for format in [Format::R2007, Format::R2018] {
        let t = through_text(&drawing(format));
        let d = Document::read(&t);
        let back: Vec<EntityKind> = d.entities.iter().map(|e| Entity::read(e).kind).collect();
        assert_eq!(back, [EntityKind::MLeader(mleader()), EntityKind::Table(table())], "{format:?}");
        // The table names its block's record, as AutoCAD reads it.
        let names = d.block_names();
        let record = d.entities[1].groups().handle(343);
        assert_eq!(record.and_then(|h| names.get(&h).copied()), Some("*T1"));
    }
}

#[test]
fn the_styles_read_back_in_their_dictionaries() {
    let t = through_text(&drawing(Format::R2018));
    let d = Document::read(&t);
    let root = d.root_dictionary().expect("the root dictionary");
    let names = d.dictionary_names();
    let mleader_dict = d.dictionary(root.get("ACAD_MLEADERSTYLE").expect("the multileader styles")).expect("a dictionary");
    let table_dict = d.dictionary(root.get("ACAD_TABLESTYLE").expect("the table styles")).expect("a dictionary");
    assert_eq!((mleader_dict.get("Notes"), table_dict.get("Plain")), (Some(0x60), Some(0x70)), "given handles are kept");
    let ms = MLeaderStyle::read(d.objects_of("MLEADERSTYLE").next().expect("the style"));
    let ts = TableStyle::read(d.objects_of("TABLESTYLE").next().expect("the style"));
    assert_eq!((ms, ts), (mleader_style(), table_style()));
    assert_eq!((names.get(&0x60).copied(), names.get(&0x70).copied()), (Some("Notes"), Some("Plain")));
    let classes: Vec<String> = d.classes.iter().filter_map(|c| c.groups().s(1)).collect();
    for c in ["MULTILEADER", "MLEADERSTYLE", "ACAD_TABLE", "TABLESTYLE"] {
        assert!(classes.iter().any(|x| x == c), "{c} is declared");
    }
}

#[test]
fn versions_without_them_leave_them_out() {
    // 2004: tables and table styles, no multileaders or their styles.
    let (t, skipped) = drawing(Format::R2004).tags();
    assert_eq!(skipped, 1);
    let d = Document::read(&t);
    assert_eq!(d.entities.iter().map(|e| e.kind).collect::<Vec<_>>(), ["ACAD_TABLE"]);
    assert!(d.objects_of("MLEADERSTYLE").next().is_none() && d.objects_of("TABLESTYLE").next().is_some());
    assert!(d.classes.iter().all(|c| c.groups().s(1).as_deref() != Some("MULTILEADER")));
    // 2000: neither.
    let (t, skipped) = drawing(Format::R2000).tags();
    assert_eq!(skipped, 2);
    let d = Document::read(&t);
    assert!(
        d.entities.is_empty()
            && d.objects_of("TABLESTYLE").next().is_none()
            && d.root_dictionary().is_some_and(|r| r.get("ACAD_TABLESTYLE").is_none())
    );
}

/// A multileader and a table as #986's and #994's tests read them from files written elsewhere.
const FROM_FILES: &str = "0|SECTION|2|ENTITIES
0|MULTILEADER|5|80|100|AcDbEntity|8|0|100|AcDbMLeader|270|2
300|CONTEXT_DATA{|40|1.0|10|5.0|20|5.0|30|0.0|41|2.5|140|0.18|145|0.09|174|1|175|1|290|1|304|first^Jsecond|11|0.0|21|0.0|31|1.0|340|11
12|6.0|22|5.0|32|0.0|13|1.0|23|0.0|33|0.0|42|0.0|43|20.0|45|1.25|170|2|171|1|296|0|110|0.0|120|0.0|130|0.0
302|LEADER{|290|1|291|1|10|4.0|20|5.0|30|0.0|11|1.0|21|0.0|31|0.0|90|0|40|0.36
304|LEADER_LINE{|10|0.0|20|0.0|10|2.0|20|3.0|30|1.0|91|0|305|}
304|LEADER_LINE{|10|0.0|20|9.0|305|}
303|}
302|LEADER{|10|8.0|20|5.0|40|0.5|303|}
301|}
340|1A|90|0|170|1|91|0|171|0|290|1|291|1|41|0.36|42|0.18|172|2|343|11|173|1|95|1|174|0|175|0|176|0|178|0|179|1|45|1.0|271|0|272|9|273|9
330|AD|177|1|44|0.0|302|value one|330|AE|177|2|44|0.0|302|value two
0|ACAD_TABLE|5|90|100|AcDbEntity|8|0|100|AcDbBlockReference|2|*T1|10|1.0|20|2.0|30|0.0|100|AcDbTable|280|0|342|5A|343|*T1|11|1.0|21|0.0|31|0.0
90|22|91|2|92|2|93|0|94|0|95|0|96|0|141|0.5|141|0.7|142|3.0|142|4.0
171|1|172|0|173|0|174|0|175|2|176|1|91|0|178|0|145|0.0|92|0|301|CELL_VALUE|93|6|90|4|1|Title|94|0|140|99.0|300|~|302|Title|304|ACVALUE_END|140|2.5|170|5|7|Bold
171|1|172|0|173|1|174|0|175|0|176|0|91|0|178|0
171|1|172|0|173|0|174|0|175|1|176|1|2|first-|3|part-|1|last
171|2|172|0|173|0|174|0|175|1|176|1|340|B1
0|ENDSEC|0|EOF";

#[test]
fn what_was_read_from_another_file_writes_back_the_same() {
    let t = tags(FROM_FILES);
    let read: Vec<Entity> = Document::read(&t).entities.iter().map(Entity::read).collect();
    assert!(matches!(read[0].kind, EntityKind::MLeader(_)) && matches!(read[1].kind, EntityKind::Table(_)));
    let d = Drawing { entities: read.clone(), blocks: vec![BlockDef { name: "*T1".into(), ..BlockDef::default() }], ..Drawing::default() };
    let again: Vec<EntityKind> = Document::read(&through_text(&d)).entities.iter().map(|e| Entity::read(e).kind).collect();
    let mut want: Vec<EntityKind> = read.into_iter().map(|e| e.kind).collect();
    // The third cell's text was in a 3 group, which the writer puts in a 2: it reads the same.
    if let Some(EntityKind::Table(tb)) = want.get_mut(1) {
        assert_eq!(tb.cells[2].text, "first-part-last");
    }
    assert_eq!(again, want);
}
