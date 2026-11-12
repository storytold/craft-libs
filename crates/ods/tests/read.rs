//! ODS packages assembled here, not files from another application. Adapted from GridCraft's
//! `crates/xlsx/src/tests/ods_tests.rs`.

use craft_ods::testing::{content, package, spreadsheet};
use craft_ods::{CellRef, Error, Options, Range, Sheet, Value, ValueType, Workbook, read};

fn ods(tables: &str) -> Vec<u8> {
    spreadsheet(tables)
}
fn row(cells: &str) -> Vec<u8> {
    ods(&format!(r#"<table:table table:name="Data"><table:table-row>{cells}</table:table-row></table:table>"#))
}
fn text(s: &str) -> Value {
    Value::text(s)
}
fn open(bytes: &[u8]) -> Result<Workbook, Error> {
    read(bytes, &Options::default())
}
/// What an application shows: the value, else the text, else the raw value.
fn shown(s: &Sheet, at: CellRef) -> Value {
    let Some(c) = s.cells.get(&at) else { return Value::Empty };
    c.value().unwrap_or_else(|| if !c.text.is_empty() { text(&c.text) } else { c.raw.as_deref().map_or(Value::Empty, text) })
}
#[test]
fn a_namespace_declaration_bomb_is_refused() {
    let bomb = |n: usize| -> String { (0..n).map(|i| format!(" xmlns:p{i}=\"urn:p{i}\"")).collect() };
    let doc = |decls: &str| {
        content(r#"<table:table table:name="Data"><table:table-row><table:table-cell office:value-type="float" office:value="1"/></table:table-row></table:table>"#)
            .replacen("<office:document-content ", &format!("<office:document-content{decls} "), 1)
    };
    assert!(open(&package(doc(&bomb(8)).as_bytes())).is_ok());
    assert!(open(&package(doc(&bomb(10_000)).as_bytes())).is_err());
}

#[test]
fn typed_values_and_formula_caches() {
    let wb = open(&row(r#"
        <table:table-cell office:value-type="float" office:value="42.5"/>
        <table:table-cell office:value-type="boolean" office:boolean-value="true"/>
        <table:table-cell office:value-type="percentage" office:value="0.25"/>
        <table:table-cell office:value-type="currency" office:currency="USD" office:value="19.99"/>
        <table:table-cell office:value-type="string" office:string-value="literal =1+1"/>
        <table:table-cell table:formula="of:=A1*2" office:value-type="float" office:value="85"><text:p>85</text:p></table:table-cell>
        <table:table-cell table:formula="of:=1/0" office:value-type="string"><text:p>#DIV/0!</text:p></table:table-cell>
        <table:table-cell table:formula="of:=UNKNOWN()"/>
        <table:table-cell office:value-type="float" office:value="NaN"><text:p>unavailable</text:p></table:table-cell>
        <table:table-cell office:value-type="boolean" office:boolean-value="0" table:style-name="ce1"/>
    "#))
    .unwrap();
    let s = &wb.sheets[0];
    let expected = [
        Value::Number(42.5),
        Value::Bool(true),
        Value::Number(0.25),
        Value::Number(19.99),
        text("literal =1+1"),
        Value::Number(85.0),
        text("#DIV/0!"),
        Value::Empty,
        text("unavailable"),
        Value::Bool(false),
    ];
    for (c, value) in expected.iter().enumerate() {
        assert_eq!(&shown(s, CellRef::new(0, c as u32)), value, "{c}");
    }
    let at = |c| &s.cells[&CellRef::new(0, c)];
    assert_eq!((at(3).value_type, at(3).currency.as_deref()), (Some(ValueType::Currency), Some("USD")));
    assert_eq!(at(5).formula.as_deref(), Some("of:=A1*2"));
    assert_eq!((at(7).formula.as_deref(), at(7).value()), (Some("of:=UNKNOWN()"), None));
    assert_eq!(at(8).value(), None);
    assert_eq!(at(9).style.as_deref(), Some("ce1"));
}

#[test]
fn text_keeps_paragraphs_runs_entities_spaces_tabs_and_breaks() {
    let wb = open(&row(
        r#"<table:table-cell office:value-type="string"><text:p>one <text:span>two &amp; three</text:span> four<text:s text:c="3"/><text:a>five</text:a><text:tab/>six<text:line-break/>seven&#33;<![CDATA[<eight>]]></text:p><text:p>nine</text:p><office:annotation><text:p>not cell text</text:p></office:annotation></table:table-cell>"#,
    ))
    .unwrap();
    assert_eq!(shown(&wb.sheets[0], CellRef::new(0, 0)), text("one two & three four   five\tsix\nseven!<eight>\nnine"));
    let wb = open(&row(r#"<table:table-cell office:value-type="string"><text:p>  one
        <text:span> two  </text:span>  three <text:s text:c="0"/>four<text:s text:c="2"/>  </text:p><text:p><text:s/>  five  </text:p></table:table-cell>"#))
    .unwrap();
    assert_eq!(shown(&wb.sheets[0], CellRef::new(0, 0)), text("one two three four  \n  five"));
}

#[test]
fn repeats_merges_names_and_an_empty_tail_stay_sparse() {
    let wb = open(&ods(r#"
      <table:table table:name="First &amp; Second">
        <table:table-header-rows><table:table-row><table:table-cell table:number-columns-spanned="2" table:number-rows-spanned="2" office:value-type="string"><text:p>merged</text:p></table:table-cell><table:covered-table-cell office:value-type="float" office:value="99"/></table:table-row></table:table-header-rows>
        <table:table-row><table:covered-table-cell table:number-columns-repeated="2"/></table:table-row>
        <table:table-row-group><table:table-row table:number-rows-repeated="2"><table:table-cell table:number-columns-repeated="3" office:value-type="float" office:value="7"/></table:table-row></table:table-row-group>
        <table:table-row table:number-rows-repeated="1048572"><table:table-cell table:number-columns-repeated="16384"/></table:table-row>
      </table:table>
      <table:table table:name="'History'"><table:table-row><table:table-cell office:value-type="string" office:string-value="second sheet"/></table:table-row></table:table>
    "#))
    .unwrap();
    assert_eq!(wb.sheets.len(), 2);
    assert_eq!((wb.sheets[0].name.as_str(), wb.sheets[1].name.as_str()), ("First & Second", "'History'"));
    assert_eq!(wb.sheets[0].cells.len(), 8);
    assert_eq!(shown(&wb.sheets[0], CellRef::new(0, 1)), Value::Number(99.0));
    assert!(wb.sheets[0].cells[&CellRef::new(0, 1)].covered);
    assert_eq!(wb.sheets[0].merges, vec![Range::new(CellRef::new(0, 0), CellRef::new(1, 1))]);
    assert_eq!(shown(&wb.sheets[0], CellRef::new(3, 2)), Value::Number(7.0));
    assert_eq!(shown(&wb.sheets[1], CellRef::new(0, 0)), text("second sheet"));
}

#[test]
fn one_sheet_can_be_read_alone() {
    let bytes = ods(
        r#"<table:table table:name="A"><table:table-row><table:table-cell office:value-type="float" office:value="1"/></table:table-row></table:table><table:table table:name="B"><table:table-row><table:table-cell office:value-type="float" office:value="2"/></table:table-row></table:table>"#,
    );
    let wb = read(&bytes, &Options { only_sheet: Some(1), ..Options::default() }).unwrap();
    assert_eq!(wb.sheets.iter().map(|s| (s.name.as_str(), s.cells.len())).collect::<Vec<_>>(), [("A", 0), ("B", 1)]);
}

#[test]
fn dates_and_durations_are_serials_and_unreadable_ones_stay_raw() {
    let wb = open(&row(r#"
        <table:table-cell office:value-type="date" office:date-value="1900-02-28"/>
        <table:table-cell office:value-type="date" office:date-value="2026-10-07T18:30:00.125"/>
        <table:table-cell office:value-type="time" office:time-value="P1DT2H3M4.5S"/>
        <table:table-cell office:value-type="time" office:time-value="-PT2H"/>
        <table:table-cell office:value-type="date" office:date-value="1900-02-29"/>
        <table:table-cell office:value-type="date" office:date-value="2026-10-07T18:00:00Z"/>
        <table:table-cell office:value-type="time" office:time-value="P1M"/>
        <table:table-cell office:value-type="time" office:time-value="PT2M1H"/>
    "#))
    .unwrap();
    let s = &wb.sheets[0];
    assert_eq!(shown(s, CellRef::new(0, 0)), Value::Number(59.0));
    assert_eq!(shown(s, CellRef::new(0, 1)), Value::Number(46302.0 + 66600.125 / 86400.0));
    assert_eq!(shown(s, CellRef::new(0, 2)), Value::Number(1.0 + 7384.5 / 86400.0));
    assert_eq!(shown(s, CellRef::new(0, 3)), Value::Number(-2.0 / 24.0));
    for (c, value) in [(4, "1900-02-29"), (5, "2026-10-07T18:00:00Z"), (6, "P1M"), (7, "PT2M1H")] {
        assert_eq!(shown(s, CellRef::new(0, c)), text(value));
    }
}

#[test]
fn namespaces_are_resolved_not_inferred_from_prefixes() {
    let xml = content(r#"<table:table table:name="Actual"><table:table-row><table:table-cell office:value-type="float" office:value="9"/></table:table-row></table:table>"#)
        .replace("<office:", "<o:")
        .replace("</office:", "</o:")
        .replace(" office:", " o:")
        .replace("xmlns:office=", "xmlns:o=")
        .replace("<table:", "<t:")
        .replace("</table:", "</t:")
        .replace(" table:", " t:")
        .replace("xmlns:table=", "xmlns:t=");
    assert_eq!(shown(&open(&package(xml.as_bytes())).unwrap().sheets[0], CellRef::new(0, 0)), Value::Number(9.0));
    let spoofed = xml.replace("urn:oasis:names:tc:opendocument:xmlns:table:1.0", "wrong-namespace");
    assert!(open(&package(spoofed.as_bytes())).is_err());
    let wb = open(&row(
        r#"<table:table-cell xmlns:fake="wrong-namespace" fake:value-type="float" fake:value="123"><text:p>kept</text:p></table:table-cell>"#,
    ))
    .unwrap();
    assert_eq!(shown(&wb.sheets[0], CellRef::new(0, 0)), text("kept"));
}

#[test]
fn bad_packages_encryption_and_bad_xml_are_refused() {
    assert!(matches!(open(b"not zip"), Err(Error::Invalid(_))));
    let xml = content(r#"<table:table table:name="Data"/>"#);
    let mut w = craft_odf::PackageWriter::new("application/zip");
    w.file("content.xml", "text/xml", xml.as_bytes());
    assert!(matches!(open(&w.finish().unwrap()), Err(Error::Invalid(_))));
    assert!(craft_ods::is_ods(&package(xml.as_bytes())));
    let mut w = craft_odf::PackageWriter::new(craft_odf::mime::SPREADSHEET);
    w.file("content.xml", "text/xml", xml.as_bytes());
    if let Some(e) = w.manifest_mut().entries.iter_mut().find(|e| e.path == "content.xml") {
        e.encryption = Some(craft_odf::Encryption::default());
    }
    assert_eq!(open(&w.finish().unwrap()), Err(Error::Encrypted));
    for bad in [
        xml.trim_end_matches("</office:document-content>").to_owned(),
        xml.replace("</office:spreadsheet>", "</office:wrong>"),
        "<root/>".to_owned(),
        format!("{xml}{xml}"),
        format!("<!DOCTYPE x>{xml}"),
        xml.replace("Data", "&custom;"),
        content(""),
    ] {
        assert!(open(&package(bad.as_bytes())).is_err(), "{bad}");
    }
    let deep = content(&format!("{}{}", "<x>".repeat(300), "</x>".repeat(300)));
    assert!(open(&package(deep.as_bytes())).is_err());
}

#[test]
fn expansion_bounds_stop_small_repeat_bombs() {
    for cells in [
        r#"<table:table-cell table:number-columns-repeated="4294967296"/>"#,
        r#"<table:table-cell table:number-columns-repeated="4294967295" office:value-type="float" office:value="1"/>"#,
        r#"<table:table-cell table:number-columns-repeated="0"/>"#,
        r#"<table:table-cell table:number-columns-repeated="16384"/><table:table-cell office:value-type="float" office:value="1"/>"#,
        r#"<table:table-cell table:number-columns-repeated="16384"/><table:table-cell table:number-columns-spanned="2"/>"#,
        r#"<table:table-cell table:number-columns-repeated="16384" table:number-columns-spanned="2" office:value-type="float" office:value="1"/>"#,
        r#"<table:table-cell office:value-type="string"><text:p><text:s text:c="4294967295"/></text:p></table:table-cell>"#,
        r#"<table:table-cell table:number-columns-repeated="2" table:number-columns-spanned="2"/>"#,
    ] {
        assert!(open(&row(cells)).is_err(), "{cells}");
    }
    let cells = ods(
        r#"<table:table table:name="Data"><table:table-row table:number-rows-repeated="1048576"><table:table-cell office:value-type="float" office:value="1"/></table:table-row></table:table>"#,
    );
    assert!(matches!(open(&cells), Err(Error::TooLarge(_))));
    let text = format!(
        r#"<table:table table:name="Data"><table:table-row table:number-rows-repeated="100000"><table:table-cell office:value-type="string" office:string-value="{}"/></table:table-row></table:table>"#,
        "x".repeat(1024)
    );
    assert!(matches!(open(&ods(&text)), Err(Error::TooLarge(_))));
    // One row of cells each just under the 1 MiB cell limit, past 64 MiB in all: refused while
    // the row is read. The row's end is never reached.
    let long = format!(r#"<table:table-cell office:value-type="string" office:string-value="{}"/>"#, "x".repeat(1024 * 1024 - 1));
    let cells = package(content(&format!(r#"<table:table table:name="Data"><table:table-row>{}"#, long.repeat(65))).as_bytes());
    assert!(matches!(open(&cells), Err(Error::TooLarge(_))));
    // A row repeated once more than its cells' text allows is refused, and one that fits reads.
    let cell = format!(r#"<table:table-cell office:value-type="string" office:string-value="{}"/>"#, "y".repeat(1024));
    let rows = |n: u32| {
        ods(&format!(r#"<table:table table:name="Data"><table:table-row table:number-rows-repeated="{n}">{cell}</table:table-row></table:table>"#))
    };
    assert!(open(&rows(65_536)).is_ok());
    assert!(matches!(open(&rows(65_537)), Err(Error::TooLarge(_))));
    // An attribute over the cell text limit.
    let small = Options { max_cell_text: 10, ..Options::default() };
    assert!(matches!(
        read(&row(r#"<table:table-cell office:value-type="string" office:string-value="eleven char"/>"#), &small),
        Err(Error::TooLarge(_))
    ));
    let few = Options { max_cells: 3, max_merges: 1, ..Options::default() };
    assert!(matches!(
        read(&row(r#"<table:table-cell table:number-columns-repeated="4" office:value-type="float" office:value="1"/>"#), &few),
        Err(Error::TooLarge(_))
    ));
    assert!(matches!(
        read(&row(r#"<table:table-cell table:number-columns-spanned="2"/><table:table-cell table:number-columns-spanned="2"/>"#), &few),
        Err(Error::TooLarge(_))
    ));
}

#[test]
fn an_error_result_never_reads_as_its_placeholder_zero() {
    let wb = open(&row(
        r#"<table:table-cell calcext:value-type="error" office:value-type="float" office:value="0" table:formula="of:=1/0"><text:p>#DIV/0!</text:p></table:table-cell>"#,
    ))
    .unwrap();
    let c = &wb.sheets[0].cells[&CellRef::new(0, 0)];
    assert_eq!((c.value_type, c.value(), c.text.as_str()), (Some(ValueType::Error), None, "#DIV/0!"));
}

#[test]
fn blank_repeats_past_the_sheet_edge_are_clipped() {
    // Writers with larger sheets pad to their own edge; blank padding past ours carries no data.
    let wb = open(&ods(r#"<table:table table:name="Data">
        <table:table-row><table:table-cell office:value-type="float" office:value="1"/><table:table-cell table:number-columns-repeated="4294967295"/><table:table-cell/></table:table-row>
        <table:table-row table:number-rows-repeated="16777215"><table:table-cell table:number-columns-repeated="16777216"/></table:table-row>
        <table:table-row table:number-rows-repeated="5"><table:table-cell/></table:table-row>
      </table:table>"#))
    .unwrap();
    assert_eq!(wb.sheets[0].cells.len(), 1);
    assert_eq!(shown(&wb.sheets[0], CellRef::new(0, 0)), Value::Number(1.0));
    // Data after the clipped edge is an error, never silently dropped.
    let late = ods(
        r#"<table:table table:name="Data"><table:table-row table:number-rows-repeated="1048576"><table:table-cell/></table:table-row><table:table-row><table:table-cell office:value-type="float" office:value="1"/></table:table-row></table:table>"#,
    );
    assert!(matches!(open(&late), Err(Error::TooLarge(_))));
}
