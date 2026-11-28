//! ODS packages assembled here, not files from another application. Adapted from GridCraft's
//! `crates/xlsx/src/tests/ods_tests.rs`.

use craft_ods::testing::{NAMESPACES, content, files, package, spreadsheet};
use craft_ods::{CellRef, Document, Error, Group, Options, Range, Sheet, Value, ValueType, Visibility, Workbook, package_path, read};

fn ods(tables: &str) -> Vec<u8> {
    spreadsheet(tables).unwrap()
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
    assert!(open(&package(doc(&bomb(8)).as_bytes()).unwrap()).is_ok());
    assert!(open(&package(doc(&bomb(10_000)).as_bytes()).unwrap()).is_err());
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
    assert_eq!(shown(&open(&package(xml.as_bytes()).unwrap()).unwrap().sheets[0], CellRef::new(0, 0)), Value::Number(9.0));
    let spoofed = xml.replace("urn:oasis:names:tc:opendocument:xmlns:table:1.0", "wrong-namespace");
    assert!(open(&package(spoofed.as_bytes()).unwrap()).is_err());
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
    assert!(craft_ods::is_ods(&package(xml.as_bytes()).unwrap()));
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
        assert!(open(&package(bad.as_bytes()).unwrap()).is_err(), "{bad}");
    }
    let deep = content(&format!("{}{}", "<x>".repeat(300), "</x>".repeat(300)));
    assert!(open(&package(deep.as_bytes()).unwrap()).is_err());
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
    // Text limits are set small so the files stay small: 1 MiB of text over all sheets, 64 KiB a cell.
    let tight = Options { max_text: 1 << 20, max_cell_text: 64 << 10, ..Options::default() };
    let text = format!(
        r#"<table:table table:name="Data"><table:table-row table:number-rows-repeated="2000"><table:table-cell office:value-type="string" office:string-value="{}"/></table:table-row></table:table>"#,
        "x".repeat(1024)
    );
    assert!(matches!(read(&ods(&text), &tight), Err(Error::TooLarge(_))));
    // One row of cells each just under the cell limit, past the text limit in all: refused while
    // the row is read. The row's end is never reached.
    let long = format!(r#"<table:table-cell office:value-type="string" office:string-value="{}"/>"#, "x".repeat(64 * 1024 - 1));
    let cells = package(content(&format!(r#"<table:table table:name="Data"><table:table-row>{}"#, long.repeat(17))).as_bytes()).unwrap();
    assert!(matches!(read(&cells, &tight), Err(Error::TooLarge(_))));
    // A row repeated once more than its cells' text allows is refused, and one that fits reads.
    let cell = format!(r#"<table:table-cell office:value-type="string" office:string-value="{}"/>"#, "y".repeat(1024));
    let rows = |n: u32| {
        ods(&format!(r#"<table:table table:name="Data"><table:table-row table:number-rows-repeated="{n}">{cell}</table:table-row></table:table>"#))
    };
    assert!(read(&rows(1024), &tight).is_ok());
    assert!(matches!(read(&rows(1025), &tight), Err(Error::TooLarge(_))));
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
fn text_past_the_limit_can_be_cut_short_instead_of_refused() {
    let cut = Options { truncate_text: true, ..Options::default() };
    // A repeated row past 64 MiB keeps the copies that fit.
    let cell = format!(r#"<table:table-cell office:value-type="string" office:string-value="{}"/>"#, "y".repeat(1024));
    let rows =
        ods(&format!(r#"<table:table table:name="Data"><table:table-row table:number-rows-repeated="65537">{cell}</table:table-row></table:table>"#));
    assert!(matches!(open(&rows), Err(Error::TooLarge(_))));
    let wb = read(&rows, &cut).unwrap();
    assert!(wb.text_truncated);
    assert_eq!(wb.sheets[0].cells.len(), 65_536);
    assert!(wb.sheets[0].cells.contains_key(&CellRef::new(65_535, 0)));
    assert!(
        !open(&ods(&format!(r#"<table:table table:name="Data"><table:table-row>{cell}</table:table-row></table:table>"#))).unwrap().text_truncated
    );
    // Repeated columns keep the copies that fit, and nothing after them is kept: not a later
    // cell that would fit, not the row's further copies, not later rows or sheets.
    let small = Options { max_text: 10, ..cut };
    let bytes = ods(r#"
      <table:table table:name="A">
        <table:table-row table:number-rows-repeated="2"><table:table-cell table:number-columns-repeated="3" office:value-type="string" office:string-value="abcd"/><table:table-cell office:value-type="string" office:string-value="z"/><table:table-cell table:number-columns-spanned="2"/></table:table-row>
        <table:table-row><table:table-cell office:value-type="float" office:value="1"/></table:table-row>
      </table:table>
      <table:table table:name="B"><table:table-row><table:table-cell table:formula="of:=1"/></table:table-row></table:table>"#);
    assert!(matches!(read(&bytes, &Options { max_text: 10, ..Options::default() }), Err(Error::TooLarge(_))));
    let wb = read(&bytes, &small).unwrap();
    assert!(wb.text_truncated);
    assert_eq!(wb.sheets.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["A", "B"]);
    assert_eq!(wb.sheets[0].cells.keys().copied().collect::<Vec<_>>(), [CellRef::new(0, 0), CellRef::new(0, 1)]);
    assert_eq!(shown(&wb.sheets[0], CellRef::new(0, 1)), text("abcd"));
    assert!(wb.sheets[0].merges.is_empty() && wb.sheets[1].cells.is_empty());
    // A repeated row keeps the cells of its next copy that fit.
    let bytes = ods(
        r#"<table:table table:name="A"><table:table-row table:number-rows-repeated="3"><table:table-cell office:value-type="string" office:string-value="abcd"/><table:table-cell office:value-type="string" office:string-value="efgh"/></table:table-row></table:table>"#,
    );
    let wb = read(&bytes, &Options { max_text: 12, ..cut }).unwrap();
    assert!(wb.text_truncated);
    assert_eq!(wb.sheets[0].cells.keys().copied().collect::<Vec<_>>(), [CellRef::new(0, 0), CellRef::new(0, 1), CellRef::new(1, 0)]);
    // Cells after the cut are not read, so their own size limit does not refuse the document.
    let small = Options { max_text: 4, max_cell_text: 10, ..cut };
    let bytes = ods(r#"<table:table table:name="A">
        <table:table-row><table:table-cell office:value-type="string" office:string-value="abcd"/><table:table-cell office:value-type="string" office:string-value="x"/><table:table-cell office:value-type="string" office:string-value="eleven char"/></table:table-row>
        <table:table-row><table:table-cell><text:p>more than ten bytes</text:p></table:table-cell></table:table-row>
      </table:table>"#);
    assert!(matches!(read(&bytes, &Options { truncate_text: false, ..small }), Err(Error::TooLarge(_))));
    assert_eq!(read(&bytes, &small).unwrap().sheets[0].cells.keys().copied().collect::<Vec<_>>(), [CellRef::new(0, 0)]);
    // The cell that passes the limit is read whole first, so its own size limit still applies.
    let over = row(r#"<table:table-cell office:value-type="string" office:string-value="eleven char"/>"#);
    assert!(matches!(read(&over, &small), Err(Error::TooLarge(_))));
    // The other limits refuse the cells kept, and do not count the cells left out.
    let few = Options { max_cells: 1, ..cut };
    let after = row(
        r#"<table:table-cell office:value-type="string" office:string-value="ab"/><table:table-cell table:number-columns-repeated="2" office:value-type="float" office:value="1"/>"#,
    );
    assert!(read(&after, &Options { max_text: 1, ..few }).unwrap().sheets[0].cells.is_empty());
    assert!(matches!(
        read(&row(r#"<table:table-cell table:number-columns-repeated="2" office:value-type="float" office:value="1"/>"#), &few),
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

fn document(bytes: &[u8]) -> Result<Document<'_>, Error> {
    Document::open(bytes, None, &Options::default())
}

#[test]
fn styled_blank_cells_rows_and_columns_stay_runs_to_the_grid_edge() {
    // Office suites pad a formatted sheet with styled blank cells to their own edge.
    let wb = open(&ods(r#"<table:table table:name="Data">
        <table:table-column table:style-name="co1" table:number-columns-repeated="2" table:default-cell-style-name="Money"/>
        <table:table-column table:number-columns-repeated="3"/>
        <table:table-column table:visibility="collapse"/>
        <table:table-column table:style-name="co2" table:number-columns-repeated="99999"/>
        <table:table-row table:style-name="ro1"><table:table-cell office:value-type="float" office:value="1" table:style-name="ce1"/><table:table-cell table:style-name="ce2" table:number-columns-repeated="4294967295"/></table:table-row>
        <table:table-row table:style-name="ro2" table:number-rows-repeated="16777215"><table:table-cell table:style-name="ce3" table:number-columns-repeated="16384"/></table:table-row>
      </table:table>"#))
    .unwrap();
    let s = &wb.sheets[0];
    assert_eq!(s.cells.len(), 1, "styled blanks are not expanded into cells");
    assert_eq!(s.cells[&CellRef::new(0, 0)].style.as_deref(), Some("ce1"));
    let edge = CellRef::new(craft_ods::MAX_ROWS - 1, craft_ods::MAX_COLS - 1);
    let tail = s.blanks.iter().find(|b| b.style.as_deref() == Some("ce3")).unwrap();
    assert!(tail.range.contains(edge) && tail.range.contains(CellRef::new(1, 0)));
    let first_row = s.blanks.iter().find(|b| b.style.as_deref() == Some("ce2")).unwrap();
    assert!(first_row.range.contains(CellRef::new(0, 1)) && first_row.range.contains(CellRef::new(0, craft_ods::MAX_COLS - 1)));
    assert!(!first_row.range.contains(CellRef::new(0, 0)) && !first_row.range.contains(CellRef::new(1, 1)));
    // Columns that state nothing are left out; a run past the grid's edge is clipped.
    let cols: Vec<_> = s.columns.iter().map(|l| (l.first, l.style.as_deref(), l.visibility)).collect();
    assert_eq!(cols, [(0, Some("co1"), Visibility::Visible), (5, None, Visibility::Collapse), (6, Some("co2"), Visibility::Visible)]);
    assert_eq!(s.columns[0].default_cell_style.as_deref(), Some("Money"));
    assert!(s.columns[2].first + s.columns[2].count <= craft_ods::MAX_COLS);
    assert_eq!(s.rows[1].first + s.rows[1].count, craft_ods::MAX_ROWS);
    // Data after styled padding is still refused, never silently dropped.
    let late = ods(
        r#"<table:table table:name="Data"><table:table-row table:number-rows-repeated="1048576"><table:table-cell table:style-name="ce1"/></table:table-row><table:table-row><table:table-cell office:value-type="float" office:value="1"/></table:table-row></table:table>"#,
    );
    assert!(matches!(open(&late), Err(Error::TooLarge(_))));
}

#[test]
fn groups_and_header_lines_keep_their_positions_and_levels() {
    let wb = open(&ods(r#"<table:table table:name="Data">
        <table:table-header-columns><table:table-column table:number-columns-repeated="2"/></table:table-header-columns>
        <table:table-column-group table:display="false"><table:table-column table:number-columns-repeated="3" table:visibility="collapse"/></table:table-column-group>
        <table:table-header-rows><table:table-row><table:table-cell/></table:table-row></table:table-header-rows>
        <table:table-row-group>
          <table:table-row table:number-rows-repeated="2"><table:table-cell/></table:table-row>
          <table:table-row-group table:display="false"><table:table-row table:number-rows-repeated="4" table:visibility="collapse"><table:table-cell/></table:table-row></table:table-row-group>
        </table:table-row-group>
        <table:table-row><table:table-cell office:value-type="float" office:value="7"/></table:table-row>
      </table:table>"#))
    .unwrap();
    let s = &wb.sheets[0];
    assert_eq!((s.header_columns, s.header_rows), (Some((0, 2)), Some((0, 1))));
    assert_eq!(s.column_groups, [Group { first: 2, count: 3, level: 1, display: false }]);
    assert!(s.row_groups.contains(&Group { first: 3, count: 4, level: 2, display: false }));
    assert!(s.row_groups.contains(&Group { first: 1, count: 6, level: 1, display: true }));
    assert_eq!(shown(s, CellRef::new(7, 0)), Value::Number(7.0));
    assert!(s.rows.iter().any(|l| l.first == 3 && l.count == 4 && l.visibility == Visibility::Collapse));
}

#[test]
fn a_cell_keeps_its_note_link_styled_text_validation_matrix_and_drawings() {
    let wb = open(&ods(r#"<table:table table:name="Data"><table:table-row>
        <table:table-cell office:value-type="string" table:content-validation-name="val1">
          <office:annotation><dc:creator>Ann</dc:creator><text:p>a note</text:p></office:annotation>
          <text:p>Go <text:span text:style-name="T1">to <text:span>the</text:span></text:span> <text:a xlink:href="https://example.org/" office:title="Example">site</text:a></text:p>
          <draw:frame draw:name="Picture 1" svg:width="2cm"><draw:image xlink:href="Pictures/a.png"/></draw:frame>
        </table:table-cell>
        <table:table-cell table:formula="of:=TRANSPOSE([.A1:.B1])" table:number-matrix-columns-spanned="1" table:number-matrix-rows-spanned="2" office:value-type="float" office:value="1"/>
        <table:table-cell table:content-validation-name="val2" table:number-columns-repeated="3"/>
        <table:table-cell table:number-columns-repeated="2"><office:annotation><text:p>on a blank cell</text:p></office:annotation></table:table-cell>
      </table:table-row></table:table>"#))
    .unwrap();
    let s = &wb.sheets[0];
    let c = &s.cells[&CellRef::new(0, 0)];
    assert_eq!(c.text, "Go to the site", "the note's text is not the cell's");
    let note = c.annotation.as_ref().unwrap();
    assert_eq!(note.child("dc:creator").map(|e| e.text().into_owned()).as_deref(), Some("Ann"));
    let link = c.link.as_ref().unwrap();
    assert_eq!((link.href.as_str(), link.title.as_deref()), ("https://example.org/", Some("Example")));
    let styled: Vec<_> = c.runs.iter().map(|r| (&c.text[r.start..r.end], r.style.as_str())).collect();
    assert_eq!(styled, [("to the", "T1")]);
    assert_eq!(c.validation.as_deref(), Some("val1"));
    assert_eq!(c.drawings.len(), 1);
    assert_eq!(c.drawings[0].child("draw:image").and_then(|i| i.attr("xlink:href")), Some("Pictures/a.png"));
    assert_eq!(s.cells[&CellRef::new(0, 1)].matrix, Some((1, 2)));
    // A blank cell's validation is a range; its note makes it a cell at every repeat.
    let v = s.blanks.iter().find(|b| b.validation.as_deref() == Some("val2")).unwrap();
    assert_eq!(v.range, Range::new(CellRef::new(0, 2), CellRef::new(0, 4)));
    assert!(s.cells[&CellRef::new(0, 5)].annotation.is_some() && s.cells[&CellRef::new(0, 6)].annotation.is_some());
}

#[test]
fn what_is_not_a_table_row_or_cell_is_kept_as_written_with_fixed_prefixes() {
    // A writer may choose any prefixes; the trees name everything with this crate's.
    let xml = r#"<o:document-content xmlns:o="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:t="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:s="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:c="urn:org:documentfoundation:names:experimental:calc:xmlns:calcext:1.0">
          <o:automatic-styles><s:style s:name="ta1" s:family="table"/></o:automatic-styles>
          <o:body><o:spreadsheet t:structure-protected="true">
            <t:calculation-settings t:null-year="1930"/>
            <t:table t:name="Data" t:style-name="ta1" t:print="false">
              <t:table-row><t:table-cell/></t:table-row>
              <c:conditional-formats><c:conditional-format c:target-range-address="Data.A1:A5"/></c:conditional-formats>
            </t:table>
            <t:named-expressions><t:named-range t:name="Rates" t:cell-range-address="$Data.$A$1"/></t:named-expressions>
          </o:spreadsheet></o:body></o:document-content>"#;
    let wb = open(&package(xml.as_bytes()).unwrap()).unwrap();
    assert_eq!(wb.attr("table:structure-protected"), Some("true"));
    let s = &wb.sheets[0];
    assert_eq!((s.attr("table:style-name"), s.attr("table:print")), (Some("ta1"), Some("false")));
    assert_eq!(s.elements.len(), 1);
    assert_eq!(s.elements[0].find("calcext:conditional-format").and_then(|e| e.attr("calcext:target-range-address")), Some("Data.A1:A5"));
    let names: Vec<_> = wb.body.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["table:calculation-settings", "table:named-expressions"]);
    assert_eq!(wb.automatic_styles.as_ref().and_then(|a| a.child("style:style")).and_then(|s| s.attr("style:name")), Some("ta1"));
}

#[test]
fn a_document_keeps_its_other_parts_and_skips_a_damaged_one() {
    let body = content(
        r#"<table:table table:name="Data"><table:table-row><table:table-cell office:value-type="float" office:value="1"/></table:table-row></table:table>"#,
    );
    let styles = format!(
        r#"<office:document-styles {NAMESPACES}><office:styles><style:default-style style:family="table-cell"/></office:styles></office:document-styles>"#
    );
    let settings = format!(r#"<office:document-settings {NAMESPACES}><office:settings/></office:document-settings>"#);
    let meta = format!(r#"<office:document-meta {NAMESPACES}><office:meta><dc:title>Budget</dc:title></office:meta></office:document-meta>"#);
    let bytes = files(&[
        ("content.xml", body.as_bytes()),
        ("styles.xml", styles.as_bytes()),
        ("settings.xml", settings.as_bytes()),
        ("meta.xml", meta.as_bytes()),
        ("Pictures/a.png", b"\x89PNG"),
        ("Object 1/content.xml", body.as_bytes()),
    ])
    .unwrap();
    let doc = document(&bytes).unwrap();
    assert!(doc.warnings.is_empty(), "{:?}", doc.warnings);
    assert!(doc.styles.as_ref().and_then(|s| s.find("style:default-style")).is_some());
    assert!(doc.settings.is_some());
    assert_eq!(doc.meta.as_ref().and_then(|m| m.find("dc:title")).map(|t| t.text().into_owned()).as_deref(), Some("Budget"));
    assert_eq!(doc.file("Pictures/a.png").unwrap().as_deref(), Some(&b"\x89PNG"[..]));
    let object = package_path("./Object 1").unwrap();
    assert!(doc.xml(&format!("{object}/content.xml")).unwrap().is_some_and(|e| e.name == "office:document-content"));
    assert_eq!(doc.file("Pictures/none.png").unwrap(), None);
    // A damaged optional part is left out with a warning; the tables still open.
    let broken = files(&[("content.xml", body.as_bytes()), ("styles.xml", b"<office:document-styles><unclosed>")]).unwrap();
    let doc = document(&broken).unwrap();
    assert!(doc.styles.is_none() && doc.warnings.iter().any(|w| w.contains("styles.xml")));
    assert_eq!(shown(&doc.workbook.sheets[0], CellRef::new(0, 0)), Value::Number(1.0));
}

#[test]
fn flat_and_password_protected_documents_open() {
    let flat = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><office:document {NAMESPACES} office:mimetype="application/vnd.oasis.opendocument.spreadsheet"><office:meta><dc:title>Flat</dc:title></office:meta><office:body><office:spreadsheet><table:table table:name="Data"><table:table-row><table:table-cell office:value-type="float" office:value="3"><draw:frame><draw:image><office:binary-data>iVBORw==</office:binary-data></draw:image></draw:frame></table:table-cell></table:table-row></table:table></office:spreadsheet></office:body></office:document>"#
    );
    let doc = document(flat.as_bytes()).unwrap();
    let cell = &doc.workbook.sheets[0].cells[&CellRef::new(0, 0)];
    assert_eq!(cell.value(), Some(Value::Number(3.0)));
    let href = cell.drawings[0].child("draw:image").and_then(|i| i.attr("xlink:href")).unwrap();
    assert_eq!(doc.file(&package_path(href).unwrap()).unwrap().as_deref(), Some(&b"\x89PNG"[..]));
    assert!(doc.meta.as_ref().and_then(|m| m.find("dc:title")).is_some());

    let plain = ods(
        r#"<table:table table:name="Secret"><table:table-row><table:table-cell office:value-type="float" office:value="42"/></table:table-row></table:table>"#,
    );
    let locked = craft_odf::crypt::encrypt(&plain, "right").unwrap();
    assert!(matches!(document(&locked), Err(Error::Encrypted)));
    assert!(matches!(Document::open(&locked, Some("wrong"), &Options::default()), Err(Error::Password)));
    let doc = Document::open(&locked, Some("right"), &Options::default()).unwrap();
    assert_eq!(shown(&doc.workbook.sheets[0], CellRef::new(0, 0)), Value::Number(42.0));
}

#[test]
fn links_outside_the_package_have_no_package_path() {
    assert_eq!(package_path("./Object 1/").as_deref(), Some("Object 1"));
    for outside in ["../other.ods", "file:///tmp/a.png", "https://example.org/a.png", "/etc/passwd", "#Sheet1.A1", "Pictures//a.png", ""] {
        assert_eq!(package_path(outside), None, "{outside}");
    }
}

#[test]
fn kept_trees_count_every_copy_against_their_bound() {
    let note = format!(r#"<office:annotation><text:p>{}</text:p></office:annotation>"#, "n".repeat(1000));
    let wide = ods(&format!(
        r#"<table:table table:name="Data"><table:table-row table:number-rows-repeated="100"><table:table-cell table:number-columns-repeated="100">{note}</table:table-cell></table:table-row></table:table>"#
    ));
    let small = Options { max_kept: 1 << 20, ..Options::default() };
    assert!(matches!(read(&wide, &small), Err(Error::TooLarge(_))), "10,000 copies of a 1 KB note pass 1 MiB");
    let one = ods(&format!(
        r#"<table:table table:name="Data"><table:table-row><table:table-cell>{note}</table:table-cell></table:table-row></table:table>"#
    ));
    assert!(read(&one, &small).is_ok());
    // A body element alone past the bound is refused too.
    let names: String = (0..20_000).map(|i| format!(r#"<table:named-range table:name="n{i}" table:cell-range-address="$Data.$A$1"/>"#)).collect();
    let many = ods(&format!(r#"<table:table table:name="Data"/><table:named-expressions>{names}</table:named-expressions>"#));
    assert!(matches!(read(&many, &small), Err(Error::TooLarge(_))));
}
