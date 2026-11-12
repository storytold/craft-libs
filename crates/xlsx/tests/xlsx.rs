//! SpreadsheetML workbooks through the part layer.

mod common;

use common::{read, read_with};
use craft_xlsx::testing::{workbook, zip};
use craft_xlsx::{CellRef, CellValue, Error, FormulaKind, Limits, Package, Range, SheetKind, Value, Visibility};

/// A workbook in the strict namespaces with the `x:` prefix, as some writers save it.
fn strict() -> Vec<u8> {
    const NS: &str = r#"xmlns:x="http://purl.oclc.org/ooxml/spreadsheetml/main" xmlns:r="http://purl.oclc.org/ooxml/officeDocument/relationships""#;
    const RT: &str = "http://purl.oclc.org/ooxml/officeDocument/relationships";
    let book = format!(
        r#"<x:workbook {NS}><x:workbookPr date1904="1"/><x:bookViews><x:workbookView visibility="hidden" activeTab="0"/><x:workbookView activeTab="2"/></x:bookViews>
        <x:sheets><x:sheet name="One" sheetId="4" r:id="a"/><x:sheet name="Chart" sheetId="5" r:id="b"/><x:sheet name="Two" sheetId="6" state="veryHidden" r:id="c"/><x:sheet name="Lost" sheetId="7" r:id="zz"/></x:sheets>
        <x:definedNames><x:definedName name="_xlnm.Print_Area" localSheetId="0"> One!$A$1:$B$2 </x:definedName><x:definedName name="Rate" comment="c" hidden="1">0.2</x:definedName><x:definedName>orphan</x:definedName></x:definedNames></x:workbook>"#
    );
    let rels = format!(
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="a" Type="{RT}/worksheet" Target="sheets/one.xml"/><Relationship Id="b" Type="{RT}/chartsheet" Target="charts/c.xml"/><Relationship Id="c" Type="{RT}/worksheet" Target="/xl/sheets/two.xml"/><Relationship Id="s" Type="{RT}/sharedStrings" Target="strings.xml"/><Relationship Id="t" Type="{RT}/styles" Target="styles.xml"/></Relationships>"#
    );
    let one = format!(
        r#"<x:worksheet {NS}><x:cols><x:col min="2" max="3" width="20.5" customWidth="1"/><x:col min="0" max="1"/></x:cols><x:sheetData>
        <x:row r="1"><x:c r="A1" t="s"><x:v>1</x:v></x:c><x:c r="B1" s="1"><x:v>1</x:v></x:c><x:c r="C1"><x:f>B1*2</x:f><x:v>2</x:v></x:c></x:row>
        <x:row r="2"><x:c r="A2" t="d"><x:v>1904-01-03</x:v></x:c><x:c r="B2" t="s"><x:v>9</x:v></x:c></x:row></x:sheetData>
        <x:mergeCells><x:mergeCell ref="A3:B4"/><x:mergeCell ref="C3"/></x:mergeCells><x:pageSetup orientation="landscape"/><x:drawing r:id="d1"/>
        <x:extLst><x:ext uri="{{05C60535-1F16-4fd2-B633-F4F36F0B64E0}}" xmlns:e="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main"><e:sparklineGroups xmlns:m="http://schemas.microsoft.com/office/excel/2006/main"><e:sparklineGroup><e:sparklines><e:sparkline><m:f>A1:E1</m:f><m:sqref>F1</m:sqref></e:sparkline></e:sparklines></e:sparklineGroup></e:sparklineGroups></x:ext></x:extLst></x:worksheet>"#
    );
    zip(&[
        ("[Content_Types].xml", b"<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"/>"),
        (
            "_rels/.rels",
            format!(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="w" Type="{RT}/officeDocument" Target="xl/book.xml"/></Relationships>"#).as_bytes(),
        ),
        ("xl/book.xml", book.as_bytes()),
        ("xl/_rels/book.xml.rels", rels.as_bytes()),
        ("xl/sheets/one.xml", one.as_bytes()),
        ("xl/sheets/two.xml", format!("<x:worksheet {NS}><x:sheetData/></x:worksheet>").as_bytes()),
        ("xl/strings.xml", format!(r#"<x:sst {NS}><x:si><x:t>zero</x:t></x:si><x:si><x:r><x:t>o</x:t></x:r><x:r><x:t>ne</x:t></x:r></x:si></x:sst>"#).as_bytes()),
        (
            "xl/styles.xml",
            format!(r#"<x:styleSheet {NS}><x:numFmts><x:numFmt numFmtId="170" formatCode="0.000"/></x:numFmts><x:cellXfs><x:xf/><x:xf numFmtId="170"/></x:cellXfs></x:styleSheet>"#)
                .as_bytes(),
        ),
    ])
}

#[test]
fn a_strict_workbook_with_prefixes() {
    let bytes = strict();
    let mut p = Package::open(&bytes, &Limits::default()).unwrap();
    assert!(!p.is_binary());
    assert_eq!(p.workbook_part(), "xl/book.xml");
    let wb = p.workbook().unwrap();
    assert!(wb.date_1904);
    assert_eq!(wb.active_tab, Some(2));
    let kinds: Vec<_> = wb.sheets.iter().map(|s| (s.name.as_str(), s.sheet_id, s.kind.clone(), s.part.as_deref(), s.visibility)).collect();
    assert_eq!(
        kinds,
        [
            ("One", 4, SheetKind::Worksheet, Some("xl/sheets/one.xml"), Visibility::Visible),
            ("Chart", 5, SheetKind::Chartsheet, Some("xl/charts/c.xml"), Visibility::Visible),
            ("Two", 6, SheetKind::Worksheet, Some("xl/sheets/two.xml"), Visibility::VeryHidden),
            ("Lost", 7, SheetKind::Other(String::new()), None, Visibility::Visible),
        ]
    );
    let names: Vec<_> = wb.defined_names.iter().map(|d| (d.name.as_str(), d.local_sheet_id, d.formula.as_str(), d.hidden)).collect();
    assert_eq!(names, [("_xlnm.Print_Area", Some(0), "One!$A$1:$B$2", false), ("Rate", None, "0.2", true)]);
    assert!(wb.root.child("bookViews").is_some());

    let styles = p.styles().unwrap().unwrap();
    assert_eq!(styles.cell_number_format(1), Some("0.000"));
    let strings = p.shared_strings().unwrap();
    assert_eq!(strings.iter().map(|s| &*s.text).collect::<Vec<_>>(), ["zero", "one"]);
    let mut cells = vec![];
    let part = p
        .sheet(&wb.sheets[0], |row| {
            cells.extend(row.cells);
            Ok(())
        })
        .unwrap();
    assert_eq!(part.merges, [Range::parse("A3:B4").unwrap()]);
    assert_eq!(part.columns.len(), 1);
    assert_eq!((part.columns[0].first, part.columns[0].last, part.columns[0].width), (1, 2, Some(20.5)));
    assert_eq!(part.root.child("pageSetup").and_then(|e| e.attr("orientation")), Some("landscape"));
    assert!(part.root.child("sheetData").is_some_and(|d| d.elements().next().is_none()));
    // Other namespaces keep fixed prefixes, so the rest of the part can be written back.
    assert_eq!(part.root.child("drawing").and_then(|d| d.attr("r:id")), Some("d1"));
    assert_eq!(part.root.find("xm:sqref").map(|e| e.text().into_owned()).as_deref(), Some("F1"));
    let mut root = part.root.clone();
    craft_xlsx::ns::declare(&mut root);
    let again = craft_xml::Options::canonical(&craft_xlsx::ns::CANONICAL).parse(root.to_xml().as_bytes()).unwrap();
    assert_eq!(again, part.root);
    assert_eq!((part.rows, part.dropped, part.damaged.as_deref()), (2, 0, None));
    let values: Vec<_> = cells.iter().map(|c| c.value.resolve(&strings, true)).collect();
    assert_eq!(values, [Value::text("one"), Value::Number(1.0), Value::Number(2.0), Value::Number(2.0), Value::Empty]);
    assert_eq!(cells[1].style, 1);
    assert_eq!(cells[2].formula.as_ref().map(|f| (f.kind, f.text.as_str())), Some((FormulaKind::Normal, "B1*2")));
    assert_eq!(cells[4].value, CellValue::Shared(9));
    assert!(p.sheet(&wb.sheets[3], |_| Ok(())).is_err());
}

#[test]
fn a_damaged_sheet_keeps_the_rows_before_the_damage() {
    let bytes = workbook(&[("S", r#"<row r="1"><c r="A1"><v>1</v></c></row><row r="2"><c r="A2"><v>2</v></c></row><!-- never closed"#)], &[]);
    let wb = read(&bytes).unwrap();
    assert_eq!(wb.sheets[0].cells.len(), 2);
    assert!(wb.warnings.iter().any(|w| w.contains("is damaged") && w.contains("kept 2 rows")), "{:?}", wb.warnings);
}

#[test]
fn the_caller_can_stop_a_sheet() {
    let bytes = workbook(&[("S", r#"<row r="1"/><row r="2"/><row r="3"/>"#)], &[]);
    let mut p = Package::open(&bytes, &Limits::default()).unwrap();
    let wb = p.workbook().unwrap();
    let mut seen = 0;
    let stop = p.sheet(&wb.sheets[0], |_| {
        seen += 1;
        if seen == 2 { Err(Error::TooLarge("enough".into())) } else { Ok(()) }
    });
    assert_eq!(stop.err(), Some(Error::TooLarge("enough".into())));
    assert_eq!(seen, 2);
}

#[test]
fn cells_outside_the_grid_are_dropped_and_counted() {
    let bytes = workbook(
        &[("S", r#"<row r="1048577"><c><v>1</v></c></row><row r="2"><c r="XFE2"><v>1</v></c><c r="A0"><v>1</v></c><c r="XFD2"><v>3</v></c></row>"#)],
        &[],
    );
    let wb = read(&bytes).unwrap();
    assert_eq!(wb.sheets[0].cells.keys().copied().collect::<Vec<_>>(), [CellRef::new(1, 16383)]);
    assert!(wb.warnings.iter().any(|w| w.contains("3 cells or rows outside")), "{:?}", wb.warnings);
}

#[test]
fn the_workbook_part_is_found_or_named() {
    // No package relationship: the usual name.
    let plain = zip(&[("xl/workbook.xml", br#"<workbook><sheets><sheet name="A" r:id="x"/></sheets></workbook>"#)]);
    assert_eq!(Package::open(&plain, &Limits::default()).unwrap().workbook().unwrap().sheets.len(), 1);
    // A relationship to a part that is not there, with the usual part present.
    let stray = zip(&[
        ("_rels/.rels", br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="w" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="nowhere.xml"/></Relationships>"#),
        ("xl/workbook.xml", b"<workbook/>"),
    ]);
    let mut p = Package::open(&stray, &Limits::default()).unwrap();
    assert!(p.workbook().is_ok());
    assert_eq!(p.workbook_part(), "xl/workbook.xml");
    let odf = zip(&[("mimetype", b"application/vnd.oasis.opendocument.spreadsheet"), ("content.xml", b"<x/>")]);
    assert!(Package::open(&odf, &Limits::default()).unwrap().workbook().unwrap_err().to_string().contains("OpenDocument"));
    let other = zip(&[("xl/workbook.xml", b"<document/>")]);
    assert!(matches!(Package::open(&other, &Limits::default()).unwrap().workbook(), Err(Error::Invalid(_))));
    assert!(matches!(Package::open(b"not a zip", &Limits::default()), Err(Error::Zip(_))));
}

#[test]
fn a_missing_shared_strings_part_is_a_warning() {
    let bytes = workbook(&[("S", r#"<row r="1"><c r="A1" t="s"><v>0</v></c></row>"#)], &["x"]);
    let mut entries = vec![];
    let mut archive = Package::open(&bytes, &Limits::default()).unwrap();
    for n in archive.opc().names().iter().map(|s| s.to_string()).collect::<Vec<_>>() {
        if n != "xl/sharedstrings.xml" {
            entries.push((n.clone(), archive.opc().read(&n).unwrap().unwrap()));
        }
    }
    let entries: Vec<(&str, &[u8])> = entries.iter().map(|(n, b)| (n.as_str(), b.as_slice())).collect();
    let wb = read(&zip(&entries)).unwrap();
    assert!(wb.sheets[0].cells.is_empty());
    assert!(wb.warnings.iter().any(|w| w.contains("missing part xl/sharedStrings.xml")), "{:?}", wb.warnings);
}

#[test]
fn limits_are_errors() {
    let rows: String = (1..=2000).map(|r| format!(r#"<row r="{r}"><c r="A{r}"><v>{r}</v></c></row>"#)).collect();
    let bytes = workbook(&[("S", &rows)], &[]);
    let small = Limits { package: craft_opc::Limits::new(100, 4096, 1 << 20), ..Limits::default() };
    assert!(matches!(read_with(&bytes, &small), Err(Error::TooLarge(_))));
    let few = Limits { package: craft_opc::Limits::new(3, 1 << 20, 1 << 20), ..Limits::default() };
    assert!(matches!(Package::open(&bytes, &few), Err(Error::TooLarge(_))));
    // A row is held to the XML node limit on its own; a sheet of many rows is not.
    let narrow = Limits { xml: craft_xml::Limits { max_nodes: 50, ..craft_xml::Limits::DEFAULT }, ..Limits::default() };
    assert_eq!(read_with(&bytes, &narrow).unwrap().sheets[0].cells.len(), 2000);
    let wide: String = (0..40).map(|c| format!("<c><v>{c}</v></c>")).collect();
    let damaged = read_with(&workbook(&[("S", &format!("<row>{wide}</row>"))], &[]), &narrow).unwrap();
    assert!(damaged.sheets[0].cells.is_empty() && damaged.warnings.iter().any(|w| w.contains("is damaged")));
}
