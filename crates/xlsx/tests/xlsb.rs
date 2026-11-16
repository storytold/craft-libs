//! Binary workbooks: BIFF12 records and packages assembled here from [MS-XLSB].
//! Adapted from GridCraft's `crates/xlsx/src/tests/xlsb_tests.rs`.

mod common;

use common::{read, read_with};
use craft_xlsx::testing::zip;
use craft_xlsx::{
    CellError, CellRef, Color, ColorSource, Error, Fill, FormulaKind, Limits, Package, Pattern, PatternFill, Protection, Range, Row, SheetKind,
    SheetPart, Styles, Value, Visibility, is_xlsb,
};

const ROOT: &str = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="book" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="records/book.bin"/></Relationships>"#;
const RELS: &str = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="one" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="../data/one.bin"/><Relationship Id="two" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="../data/two.bin"/><Relationship Id="strings" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings" Target="../text/strings.bin"/></Relationships>"#;
const TYPES: &str = r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/records/book.bin" ContentType="application/vnd.ms-excel.sheet.binary.macroEnabled.main"/><Override PartName="/data/one.bin" ContentType="application/vnd.ms-excel.worksheet"/><Override PartName="/data/two.bin" ContentType="application/vnd.ms-excel.worksheet"/><Override PartName="/text/strings.bin" ContentType="application/vnd.ms-excel.sharedStrings"/></Types>"#;

fn record(id: u16, bytes: &[u8]) -> Vec<u8> {
    let mut out = vec![];
    if id < 128 {
        out.push(id as u8);
    } else {
        out.extend([(id as u8 & 0x7f) | 0x80, (id >> 7) as u8]);
    }
    let mut len = bytes.len();
    loop {
        let byte = (len & 0x7f) as u8;
        len >>= 7;
        out.push(if len == 0 { byte } else { byte | 0x80 });
        if len == 0 {
            break;
        }
    }
    out.extend(bytes);
    out
}
fn wide(text: &str) -> Vec<u8> {
    let units: Vec<_> = text.encode_utf16().collect();
    let mut out = (units.len() as u32).to_le_bytes().to_vec();
    for unit in units {
        out.extend(unit.to_le_bytes());
    }
    out
}
fn rich(text: &str, extras: bool) -> Vec<u8> {
    let mut out = vec![if extras { 3 } else { 0 }];
    out.extend(wide(text));
    if extras {
        out.extend(1u32.to_le_bytes());
        out.extend([0, 0, 0, 0]); // One formatting run.
        out.extend(wide("sound"));
        out.extend(1u32.to_le_bytes());
        out.extend([0, 0, 0, 0, 1, 0, 0, 0, 0, 0]); // One ten-byte phonetic run.
    }
    out
}
fn row(r: u32) -> Vec<u8> {
    let mut out = r.to_le_bytes().to_vec();
    out.extend([0; 13]);
    record(0, &out)
}
fn cell(id: u16, col: u32, value: &[u8]) -> Vec<u8> {
    let mut data = col.to_le_bytes().to_vec();
    data.extend([0; 4]);
    data.extend(value);
    if (8..=11).contains(&id) {
        data.extend([0; 2]); // Formula flags.
        data.extend(3u32.to_le_bytes());
        data.extend([0x1e, 1, 0]); // PtgInt(1): not decoded, the saved result is read.
        data.extend(0u32.to_le_bytes());
    }
    record(id, &data)
}
fn worksheet(data: &[u8], tail: &[u8]) -> Vec<u8> {
    let mut out = record(129, &[]);
    out.extend(record(148, &[0xff; 16])); // A bogus dimension must never allocate a dense grid.
    out.extend(record(145, &[]));
    out.extend(data);
    out.extend(record(146, &[]));
    out.extend(tail);
    out.extend(record(130, &[]));
    out
}
fn book(date1904: bool) -> Vec<u8> {
    let mut out = record(131, &[]);
    let mut props = u32::from(date1904).to_le_bytes().to_vec();
    props.extend(0u32.to_le_bytes());
    props.extend(wide(""));
    out.extend(record(153, &props));
    out.extend(record(143, &[]));
    for (id, rel, name, state) in [(1u32, "one", "Data", 0u32), (2, "two", "Hidden", 1)] {
        let mut data = state.to_le_bytes().to_vec();
        data.extend(id.to_le_bytes());
        data.extend(wide(rel));
        data.extend(wide(name));
        out.extend(record(156, &data));
    }
    out.extend(record(144, &[]));
    out.extend(record(132, &[]));
    out
}
fn sst() -> Vec<u8> {
    let mut out = record(159, &[1, 0, 0, 0, 1, 0, 0, 0]);
    out.extend(record(19, &rich("shared 🦀 café", true)));
    out.extend(record(160, &[]));
    out
}
fn package(book: &[u8], sheet: &[u8], strings: &[u8], roots: &str, rels: &str, types: &str) -> Vec<u8> {
    zip(&[
        ("[Content_Types].xml", types.as_bytes()),
        ("_rels/.rels", roots.as_bytes()),
        ("records/book.bin", book),
        ("records/_rels/book.bin.rels", rels.as_bytes()),
        ("data/one.bin", sheet),
        ("data/two.bin", &worksheet(&[], &[])),
        ("text/strings.bin", strings),
    ])
}
fn file(sheet: &[u8]) -> Vec<u8> {
    package(&book(false), sheet, &sst(), ROOT, RELS, TYPES)
}
fn text(s: &str) -> Value {
    Value::Text(s.into())
}

#[test]
fn scalar_caches_rk_unicode_visibility_and_epochs() {
    let mut data = row(2);
    let real_rk = (3.5f64.to_bits() >> 32) as u32;
    for c in [
        cell(1, 0, &[]),
        cell(2, 1, &((-123i32 << 2) | 2).to_le_bytes()),
        cell(2, 2, &((123i32 << 2) | 3).to_le_bytes()),
        cell(2, 3, &real_rk.to_le_bytes()),
        cell(2, 4, &(real_rk | 1).to_le_bytes()),
        cell(3, 5, &[0x17]),
        cell(4, 6, &[1]),
        cell(5, 7, &123.5f64.to_le_bytes()),
        cell(6, 8, &wide("inline λ")),
        cell(7, 9, &0u32.to_le_bytes()),
        cell(8, 10, &wide("cached text")),
        cell(9, 11, &2.0f64.to_le_bytes()),
        cell(10, 12, &[0]),
        cell(11, 13, &[7]),
        cell(62, 14, &rich("rich text", true)),
    ] {
        data.extend(c);
    }
    let mut merge = vec![];
    for n in [5u32, 6, 0, 1] {
        merge.extend(n.to_le_bytes());
    }
    let sheet = worksheet(&data, &record(176, &merge));
    let expected = [
        Value::Empty,
        Value::Number(-123.0),
        Value::Number(1.23),
        Value::Number(3.5),
        Value::Number(0.035),
        Value::Error(CellError::Ref),
        Value::Bool(true),
        Value::Number(123.5),
        text("inline λ"),
        text("shared 🦀 café"),
        text("cached text"),
        Value::Number(2.0),
        Value::Bool(false),
        Value::Error(CellError::Div0),
        text("rich text"),
    ];
    for epoch in [false, true] {
        let bytes = package(&book(epoch), &sheet, &sst(), ROOT, RELS, TYPES);
        assert!(is_xlsb(&bytes));
        let wb = read(&bytes).unwrap();
        assert_eq!(wb.date_1904, epoch);
        assert_eq!(wb.sheets.len(), 2);
        assert_eq!((wb.sheets[0].name.as_str(), wb.sheets[1].visibility), ("Data", Visibility::Hidden));
        assert_eq!(wb.sheets[0].cells.len(), 14);
        assert_eq!(wb.sheets[0].merges, vec![Range::new(CellRef::new(5, 0), CellRef::new(6, 1))]);
        for (col, expected) in expected.iter().enumerate() {
            assert_eq!(&wb.sheets[0].value(CellRef::new(2, col as u32)), expected);
        }
    }
}

#[test]
fn cells_carry_their_format_index_and_blank_cells_their_format() {
    let mut data = row(0);
    let mut blank = 0u32.to_le_bytes().to_vec();
    blank.extend((7u32 | 0x0100_0000).to_le_bytes());
    data.extend(record(1, &blank));
    let bytes = file(&worksheet(&data, &[]));
    let mut p = craft_xlsx::Package::open(&bytes, &craft_xlsx::Limits::default()).unwrap();
    assert!(p.styles().unwrap().is_none());
    let wb = p.workbook().unwrap();
    let mut cells = vec![];
    p.sheet(&wb.sheets[0], |r| {
        cells.extend(r.cells);
        Ok(())
    })
    .unwrap();
    assert_eq!(cells.len(), 1);
    assert_eq!((cells[0].style, cells[0].phonetic, &cells[0].value), (7, true, &craft_xlsx::CellValue::Empty));
}

#[test]
fn extension_blocks_are_skipped_with_strict_boundaries_and_record_framing() {
    let mut data = row(0);
    data.extend(cell(5, 0, &1.0f64.to_le_bytes()));
    data.extend(record(35, &[0; 4]));
    data.extend(record(37, &[1, 0, 0, 0, 0, 0]));
    data.extend(cell(5, 1, &99.0f64.to_le_bytes()));
    data.extend(record(130, &[])); // Looks like the sheet's end, but is inside a skipped block.
    data.extend(record(38, &[]));
    data.extend(record(36, &[]));
    data.extend([0xe8, 0x87, 0x80, 0x80, 0x80, 0x80]); // Record 1000: high bits of the second id and fourth length byte ignored.
    data.extend(cell(5, 1, &2.0f64.to_le_bytes()));
    let wb = read(&file(&worksheet(&data, &[]))).unwrap();
    assert_eq!(wb.sheets[0].cells.len(), 2);
    assert_eq!(wb.sheets[0].value(CellRef::new(0, 1)), Value::Number(2.0));
    assert!(wb.warnings.iter().any(|w| w.contains("blocks were skipped")));
    for bad in [
        vec![0x81],
        vec![0x81, 0],
        vec![1, 0x80],
        vec![1, 10, 0],
        record(36, &[]),
        [record(35, &[0; 4]), record(38, &[])].concat(),
        record(35, &[0; 4]),
        record(37, &[0, 0]),
    ] {
        assert!(read(&file(&worksheet(&bad, &[]))).is_err(), "{bad:?}");
    }
    let nested = record(35, &[0; 4]).repeat(65);
    assert!(read(&file(&worksheet(&nested, &[]))).err().unwrap().to_string().contains("nested too deeply"));
    let mut missing_end = worksheet(&[], &[]);
    missing_end.truncate(missing_end.len() - record(130, &[]).len());
    assert!(read(&file(&missing_end)).is_err());
}

#[test]
fn bad_values_strings_and_coordinates_fail_without_silent_data_loss() {
    for bad in [
        cell(4, 0, &[2]),
        cell(3, 0, &[0xff]),
        cell(5, 0, &f64::INFINITY.to_le_bytes()),
        cell(2, 0, &0x7ff00000u32.to_le_bytes()),
        cell(7, 0, &1u32.to_le_bytes()),
        cell(6, 0, &[1, 0, 0, 0, 0, 0xd8]),
        cell(6, 0, &u32::MAX.to_le_bytes()),
        cell(5, 16384, &1.0f64.to_le_bytes()),
        cell(5, 0, &[0; 3]),
        cell(62, 0, &[1, 0, 0, 0, 0, 0xff, 0xff, 0xff, 0xff]),
    ] {
        assert!(read(&file(&worksheet(&[row(0), bad].concat(), &[]))).is_err());
    }
    assert!(read(&file(&worksheet(&cell(5, 0, &1.0f64.to_le_bytes()), &[]))).is_err());
    assert!(read(&file(&worksheet(&row(1048576), &[]))).is_err());
    let repeated = [row(0), cell(5, 0, &1.0f64.to_le_bytes()), cell(5, 0, &2.0f64.to_le_bytes())].concat();
    assert!(read(&file(&worksheet(&repeated, &[]))).is_err());
    let mut formula = [0u8; 18].to_vec(); // Cell header, saved number and flags, but no formula.
    assert!(read(&file(&worksheet(&[row(0), record(9, &formula)].concat(), &[]))).is_err());
    formula.extend(0u32.to_le_bytes()); // A formula of no tokens.
    assert!(read(&file(&worksheet(&[row(0), record(9, &formula)].concat(), &[]))).is_err());
    let bad_sst = [record(159, &[1, 0, 0, 0, 2, 0, 0, 0]), record(160, &[])].concat();
    assert!(read(&package(&book(false), &worksheet(&[], &[]), &bad_sst, ROOT, RELS, TYPES)).is_err());
}

#[test]
fn relationships_must_be_internal_and_typed() {
    let sheet = worksheet(&[], &[]);
    for (roots, rels, types) in [
        (ROOT.replace("officeDocument\"", "wrong\""), RELS.to_owned(), TYPES.to_owned()),
        (ROOT.replace("Target=", "TargetMode=\"External\" Target="), RELS.to_owned(), TYPES.to_owned()),
        (ROOT.to_owned(), RELS.replace("Target=\"../data/one.bin\"", "Target=\"missing.bin\""), TYPES.to_owned()),
        (
            ROOT.to_owned(),
            RELS.replace("Target=\"../data/one.bin\"", "TargetMode=\"External\" Target=\"https://example.com/data.bin\""),
            TYPES.to_owned(),
        ),
        (
            ROOT.to_owned(),
            RELS.replace("Target=\"../text/strings.bin\"", "TargetMode=\"External\" Target=\"https://example.com/strings.bin\""),
            TYPES.to_owned(),
        ),
        (ROOT.to_owned(), RELS.to_owned(), TYPES.replace("application/vnd.ms-excel.worksheet", "application/xml")),
        (ROOT.to_owned(), RELS.replace("Id=\"two\"", "Id=\"three\""), TYPES.to_owned()),
    ] {
        assert!(read(&package(&book(false), &sheet, &sst(), &roots, &rels, &types)).is_err(), "{roots} {rels} {types}");
    }
    let chartsheet = RELS.replace(
        "Id=\"two\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet",
        "Id=\"two\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/chartsheet",
    );
    let wb = read(&package(&book(false), &sheet, &sst(), ROOT, &chartsheet, TYPES)).unwrap();
    assert_eq!(wb.sheets.len(), 1);
    assert_eq!(wb.skipped, [("Hidden".to_string(), SheetKind::Chartsheet)]);
    // A binary part elsewhere does not make an XML workbook package binary.
    let elsewhere = TYPES.replace("/records/book.bin\"", "/elsewhere.bin\"");
    assert!(!is_xlsb(&package(&book(false), &sheet, &sst(), ROOT, RELS, &elsewhere)));
    let spec_type = TYPES.replace("application/vnd.ms-excel.sheet.binary.macroEnabled.main", "application/vnd.ms-excel.main");
    assert!(is_xlsb(&package(&book(false), &sheet, &sst(), ROOT, RELS, &spec_type)));
    assert!(read(&package(&book(false), &sheet, &sst(), ROOT, RELS, &spec_type)).is_ok());
}

#[test]
fn declared_counts_and_decoded_text_are_bounded() {
    let bad_sst = record(159, &[0xff; 8]);
    assert!(read(&package(&book(false), &worksheet(&[], &[]), &bad_sst, ROOT, RELS, TYPES)).is_err());
    let mut count_mismatch = record(159, &[1, 0, 0, 0, 1, 0, 0, 0]);
    count_mismatch.extend(record(160, &[]));
    assert!(read(&package(&book(false), &worksheet(&[], &[]), &count_mismatch, ROOT, RELS, TYPES)).is_err());
    let mut data = row(0);
    let text = wide(&"x".repeat(32767));
    for col in 0..2050 {
        data.extend(cell(6, col, &text));
    }
    // Just over 64 MiB of text, the allowance before storytold/gridcraft#2126.
    assert_eq!(read(&file(&worksheet(&data, &[]))).unwrap().sheets[0].cells.len(), 2050);
}

/// A row as Excel writes it: the row header, then `BrtRwDescent` (1024) in an alternate-content
/// block. [MS-XLSB] 2.4.791: one per non-empty row whenever the workbook lists its fonts, which
/// Excel always does. Four records per row besides the cells.
fn excel_row(r: u32) -> Vec<u8> {
    [row(r), record(37, &[1, 0, 0, 0, 0, 0]), record(1024, &[0, 0]), record(38, &[])].concat()
}
/// `n` TRUE cells, `width` to a row from A1, in rows as Excel writes them.
fn bool_cells(n: u32, width: u32) -> Vec<u8> {
    let mut data = vec![];
    for i in 0..n {
        if i % width == 0 {
            data.extend(excel_row(i / width));
        }
        data.extend(cell(4, i % width, &[1]));
    }
    data
}
/// A workbook whose two worksheets are the given parts.
fn two_sheets(one: &[u8], two: &[u8]) -> Vec<u8> {
    zip(&[
        ("[Content_Types].xml", TYPES.as_bytes()),
        ("_rels/.rels", ROOT.as_bytes()),
        ("records/book.bin", &book(false)),
        ("records/_rels/book.bin.rels", RELS.as_bytes()),
        ("data/one.bin", one),
        ("data/two.bin", two),
        ("text/strings.bin", &sst()),
    ])
}

#[test]
fn more_than_a_million_cells_open() {
    // storytold/gridcraft#2126: a million cells over the workbook was a quarter of what `.xls` allows.
    let wb = read(&file(&worksheet(&bool_cells(1_000_001, 16), &[]))).unwrap();
    assert_eq!(wb.sheets[0].cells.len(), 1_000_001);
    assert_eq!(wb.sheets[0].value(CellRef::new(62_500, 0)), Value::Bool(true));
}

#[test]
fn a_full_height_column_opens() {
    // 1,048,576 cells in Excel's rows are 5.2 million records in one part.
    let wb = read(&file(&worksheet(&bool_cells(1_048_576, 1), &[]))).unwrap();
    assert_eq!(wb.sheets[0].cells.len(), 1_048_576);
    assert_eq!(wb.sheets[0].value(CellRef::new(1_048_575, 0)), Value::Bool(true));
}

#[test]
fn formatted_blank_cells_cost_records_not_cells() {
    // Filling A1:Z200000 and typing 30 values: Excel writes a BrtCellBlank for every formatted
    // empty cell, 6 million records in all, which an allowance of 5 million per part refused.
    let mut data = vec![];
    for r in 0..200_000u32 {
        data.extend(excel_row(r));
        for c in 0..26 {
            data.extend(if r < 10 && c < 3 { cell(4, c, &[1]) } else { cell(1, c, &[]) });
        }
    }
    let wb = read(&file(&worksheet(&data, &[]))).unwrap();
    assert_eq!(wb.sheets[0].cells.len(), 30);
}

#[test]
fn cells_are_limited_to_four_million_over_the_workbook() {
    // The `.xls` limit, counted across sheets: 2M + 2M opens, 2M + 2M + 1 does not.
    let half = bool_cells(2_000_000, 40);
    let mut over = half.clone();
    over.extend(excel_row(50_000));
    over.extend(cell(4, 0, &[1]));
    let wb = read(&two_sheets(&worksheet(&half, &[]), &worksheet(&half, &[]))).unwrap();
    assert_eq!(wb.sheets.iter().map(|s| s.cells.len()).sum::<usize>(), 4_000_000);
    let err = read(&two_sheets(&worksheet(&half, &[]), &worksheet(&over, &[]))).err().unwrap();
    assert!(matches!(err, Error::TooLarge(_)), "{err}");
    assert!(err.to_string().contains("4000000"), "{err}");
}

#[test]
fn records_share_one_allowance_over_the_workbook() {
    // About 1,400 records a sheet: over a 2,000 shared allowance, though each sheet alone fits.
    let sheet = worksheet(&bool_cells(1000, 10), &[]);
    let bytes = two_sheets(&sheet, &sheet);
    let records = |n| Limits { binary_records: n, ..Limits::default() };
    assert!(matches!(read_with(&bytes, &records(2_000)), Err(Error::TooLarge(_))));
    assert!(read_with(&bytes, &records(4_000)).is_ok());
    // The default allowance: exactly 40 million records, then one more. The other parts hold 20
    // (7 in the workbook, 3 shared-string records, 5 + 5 sheet framing records).
    let filler = record(30, &[]); // Not a cell record: skipped inside the sheet data.
    let mut data = filler.repeat(40_000_000 - 20);
    assert!(read(&file(&worksheet(&data, &[]))).is_ok());
    data.extend(filler.repeat(1));
    let err = read(&file(&worksheet(&data, &[]))).err().unwrap();
    assert!(matches!(err, Error::TooLarge(_)) && err.to_string().contains("too many records (more than 40000000)"), "{err}");
}

#[test]
fn text_and_shared_strings_are_limited() {
    let bytes = file(&worksheet(&[row(0), cell(6, 0, &wide("eleven char"))].concat(), &[]));
    // The shared string ("shared 🦀 café", 17 bytes) and the cell's 11 are charged.
    let text = |n| Limits { binary_text: n, ..Limits::default() };
    assert!(read_with(&bytes, &text(28)).is_ok());
    assert!(matches!(read_with(&bytes, &text(27)), Err(Error::TooLarge(_))));
    // A declared count over 4 million shared strings is refused before any item is read.
    let sst_header = |unique: u32| [record(159, &[unique.to_le_bytes(), unique.to_le_bytes()].concat()), record(160, &[])].concat();
    let over = read(&package(&book(false), &worksheet(&[], &[]), &sst_header(4_000_001), ROOT, RELS, TYPES)).err().unwrap();
    assert!(matches!(over, Error::TooLarge(_)), "{over}");
    // At the limit the count passes, and the missing items are what is wrong.
    let at = read(&package(&book(false), &worksheet(&[], &[]), &sst_header(4_000_000), ROOT, RELS, TYPES)).err().unwrap();
    assert!(matches!(at, Error::Invalid(_)), "{at}");
}

#[test]
fn phonetic_runs_use_documented_size_and_tolerate_padding() {
    // [MS-XLSB] 2.5.103: a PhRun is 10 bytes. Padding after the runs is ignored; a run shorter
    // than documented is refused.
    let phonetic = |runs: u32, bytes: usize| {
        let mut out = vec![2];
        out.extend(wide("ruby"));
        out.extend(wide("ルビ"));
        out.extend(runs.to_le_bytes());
        out.extend(vec![0; bytes]);
        out
    };
    for padded in [phonetic(1, 10), phonetic(1, 12), phonetic(2, 24)] {
        let wb = read(&file(&worksheet(&[row(0), cell(62, 0, &padded)].concat(), &[]))).unwrap();
        assert_eq!(wb.sheets[0].value(CellRef::new(0, 0)), text("ruby"));
    }
    assert!(read(&file(&worksheet(&[row(0), cell(62, 0, &phonetic(2, 12))].concat(), &[]))).is_err());
}

/// A formula cell (`BrtFmlaNum`, cached 0) with its own tokens and extra data.
fn formula_cell(col: u32, rgce: &[u8], rgcb: &[u8]) -> Vec<u8> {
    let mut data = col.to_le_bytes().to_vec();
    data.extend([0; 4]);
    data.extend(0f64.to_le_bytes());
    data.extend([0; 2]);
    data.extend((rgce.len() as u32).to_le_bytes());
    data.extend(rgce);
    data.extend((rgcb.len() as u32).to_le_bytes());
    data.extend(rgcb);
    record(9, &data)
}

fn name(flags: u32, text: &str) -> Vec<u8> {
    let mut data = flags.to_le_bytes().to_vec();
    data.push(0);
    data.extend(u32::MAX.to_le_bytes()); // Workbook scope.
    data.extend(wide(text));
    data.extend([2, 0, 0, 0, 0x1c, 0x1d, 0, 0, 0, 0]); // Formula `#NAME?`, no extra data.
    data.extend(u32::MAX.to_le_bytes()); // No comment.
    if flags & 0x08 != 0 {
        data.extend(u32::MAX.to_le_bytes().repeat(4)); // A macro's four strings, all null.
    }
    record(39, &data)
}

/// `book(false)` with a supporting-link block and defined names, as Excel writes them: this
/// workbook, then another; sheet links Data, Hidden, Data:Hidden, a deleted sheet and the other
/// workbook; hidden names for CONCAT, LET and a LET parameter, then the user name Rate.
fn book_with_links(extern_sheet: &[u8]) -> Vec<u8> {
    let plain = book(false);
    let end = record(132, &[]);
    let mut out = plain[..plain.len() - end.len()].to_vec();
    out.extend(record(353, &[]));
    out.extend(record(357, &[]));
    out.extend(record(355, &wide("rId9")));
    out.extend(record(362, extern_sheet));
    out.extend(record(354, &[]));
    const FUTURE: u32 = 0x0002_000b;
    out.extend(name(FUTURE, "_xlfn.CONCAT"));
    out.extend(name(FUTURE, "_xlfn.LET"));
    out.extend(name(0x0002_0019, "_xlpm.x"));
    out.extend(name(0, "Rate"));
    out.extend(end);
    out
}

fn xti() -> Vec<u8> {
    let mut out = 5u32.to_le_bytes().to_vec();
    for (book, first, last) in [(0u32, 0i32, 0i32), (0, 1, 1), (0, 0, 1), (0, -1, -1), (1, 0, 0)] {
        out.extend(book.to_le_bytes());
        out.extend(first.to_le_bytes());
        out.extend(last.to_le_bytes());
    }
    out
}

fn loc(row: u32, col: u16) -> Vec<u8> {
    [&row.to_le_bytes()[..], &col.to_le_bytes()].concat()
}

/// Relative row and column flags of a `ColRelShort`.
const REL: u16 = 0xC000;

fn warned(wb: &common::Book, why: &str) -> bool {
    wb.warnings.iter().any(|w| w.contains("kept as their saved results") && w.contains(why))
}

#[test]
fn cell_formulas_are_read_in_their_xlsx_spelling() {
    let ref3d = |ixti: u16, row: u32, col: u16| [&[0x5Au8][..], &ixti.to_le_bytes(), &loc(row, col)].concat();
    let cells: Vec<(u32, Vec<u8>, Vec<u8>)> = vec![
        (1, [&[0x44u8][..], &loc(0, REL), &[0x1E, 2, 0, 0x05]].concat(), vec![]),
        (2, [&[0x25u8][..], &0u32.to_le_bytes(), &1u32.to_le_bytes(), &0u16.to_le_bytes(), &REL.to_le_bytes(), &[0x19, 0x10, 0, 0]].concat(), vec![]),
        (3, ref3d(1, 1, 1 | REL), vec![]),
        (
            4,
            [
                &[0x3Bu8][..],
                &2u16.to_le_bytes(),
                &0u32.to_le_bytes(),
                &0xFFFFFu32.to_le_bytes(),
                &REL.to_le_bytes(),
                &REL.to_le_bytes(),
                &[0x19, 0x10, 0, 0],
            ]
            .concat(),
            vec![],
        ),
        (5, [&[0x23u8, 1, 0, 0, 0][..], &[0x17, 1, 0, b'a', 0], &[0x17, 1, 0, b'b', 0], &[0x42, 3, 0xFF, 0]].concat(), vec![]),
        (6, vec![0x1E, 1, 0, 0x1E, 0, 0, 0x06, 0x1E, 2, 0, 0x41, 0xE0, 0x01], vec![]),
        (
            7,
            [&[0x60u8][..], &[0xAB; 14]].concat(),
            [&2u32.to_le_bytes()[..], &2u32.to_le_bytes(), &[0], &1f64.to_le_bytes(), &[1, 1, 0, b'x', 0], &[2, 1], &[4, 0x2A, 0, 0, 0]].concat(),
        ),
        (
            8,
            [&[0x23u8, 2, 0, 0, 0][..], &[0x23, 3, 0, 0, 0, 0x1E, 1, 0], &[0x23, 3, 0, 0, 0, 0x1E, 1, 0, 0x03], &[0x42, 4, 0xFF, 0]].concat(),
            vec![],
        ),
        (9, ref3d(3, 0, 0), vec![]),
        (10, [&[0x25u8][..], &0u32.to_le_bytes(), &1u32.to_le_bytes(), &REL.to_le_bytes(), &REL.to_le_bytes(), &[0x41, 20, 0]].concat(), vec![]),
        (11, vec![0x43, 4, 0, 0, 0, 0x1E, 2, 0, 0x05], vec![]),
        // Kept as their saved results:
        (12, ref3d(4, 0, REL), vec![]),
        (13, vec![0x01, 0, 0, 0, 0], vec![0, 0, 0, 0]), // Part of a shared formula at A1 that isn't one.
    ];
    let mut data = row(0);
    data.extend(cell(5, 0, &3.0f64.to_le_bytes()));
    for (col, rgce, rgcb) in &cells {
        data.extend(formula_cell(*col, rgce, rgcb));
    }
    data.extend(record(49, &1u32.to_le_bytes())); // BrtCellMeta: the next formula is a dynamic array.
    data.extend(formula_cell(14, &[0x1E, 1, 0], &[]));
    data.extend(formula_cell(15, &[0x18, 0x19, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0], &[])); // A table reference.
    let wb = read(&package(&book_with_links(&xti()), &worksheet(&data, &[]), &sst(), ROOT, RELS, TYPES)).unwrap();
    let sheet = &wb.sheets[0];
    let got: Vec<_> = (1..=15).map(|c| sheet.formula(CellRef::new(0, c))).collect();
    // As an `.xlsx` file holds them: names in their file spelling (`_xlfn.`, `_xlpm.`), and no
    // implicit `@`, which is how a calculator reads a legacy formula, not how the file spells it.
    let want = [
        Some("A1*2"),
        Some("SUM($A$1:A2)"),
        Some("Hidden!B2"),
        Some("SUM(Data:Hidden!A:A)"),
        Some("_xlfn.CONCAT(\"a\",\"b\")"),
        Some("IFERROR(1/0,2)"),
        Some("{1,\"x\";TRUE,#N/A}"),
        Some("_xlfn.LET(_xlpm.x,1,_xlpm.x+1)"),
        Some("#REF!"),
        Some("SQRT(A1:A2)"),
        Some("Rate*2"),
        None,
        None,
        // A BrtCellMeta before a cell gives it cell metadata, as `cm` does in .xlsx.
        Some("1"),
        None,
    ];
    assert_eq!(got, want);
    assert!(sheet.formulas.values().all(|f| f.kind == FormulaKind::Normal));
    assert_eq!(sheet.cell_metadata.iter().collect::<Vec<_>>(), [(&CellRef::new(0, 14), &1)]);
    // Every formula cell keeps its saved result as its value.
    assert!((1..=15).all(|c| sheet.value(CellRef::new(0, c)) == Value::Number(0.0)));
    for why in ["other workbooks", "missing from the file", "structured table references"] {
        assert!(warned(&wb, why), "{why}: {:?}", wb.warnings);
    }
}

#[test]
fn unreadable_links_keep_every_formula_as_its_value() {
    // A sheet-link count past the spec's limit, and a truncated list: the file opens as it did
    // before formulas were read, with values only.
    let mut truncated = xti();
    truncated.truncate(20);
    for extern_sheet in [70_000u32.to_le_bytes().to_vec(), truncated] {
        let data = [row(0), formula_cell(0, &[0x1E, 1, 0], &[])].concat();
        let wb = read(&package(&book_with_links(&extern_sheet), &worksheet(&data, &[]), &sst(), ROOT, RELS, TYPES)).unwrap();
        assert_eq!(wb.sheets[0].formula(CellRef::new(0, 0)), None);
        assert_eq!(wb.sheets[0].value(CellRef::new(0, 0)), Value::Number(0.0));
        assert!(wb.warnings.iter().any(|w| w.contains("could not be read")), "{:?}", wb.warnings);
    }
}

#[test]
fn formula_grouping_follows_token_order() {
    // Reverse-Polish tokens group by their order alone. Excel writes PtgParen wherever its text
    // has parentheses, but another writer need not: the text gets the ones it needs to mean the
    // same, and no more.
    let a = |row: u32| [&[0x24u8][..], &loc(row, REL)].concat();
    let int = |n: u8| vec![0x1E, n, 0];
    let cases: Vec<(Vec<u8>, &str)> = vec![
        ([int(1), int(2), vec![0x03], int(3), vec![0x05]].concat(), "(1+2)*3"),
        ([int(1), int(2), int(3), vec![0x04, 0x04]].concat(), "1-(2-3)"),
        ([int(1), int(2), vec![0x04], int(3), vec![0x04]].concat(), "1-2-3"),
        ([int(2), vec![0x13], int(2), vec![0x07]].concat(), "-2^2"),
        ([int(2), int(2), vec![0x07, 0x13]].concat(), "-(2^2)"),
        // Excel's tokens for its own `-1%`: the parser groups it the other way, to the same value.
        ([int(1), vec![0x13, 0x14]].concat(), "-1%"),
        ([int(1), int(2), vec![0x03, 0x14]].concat(), "(1+2)%"),
        ([a(0), a(1), vec![0x10], vec![0x19, 0x10, 0, 0]].concat(), "SUM((A1,A2))"),
        ([int(1), int(2), vec![0x03, 0x15], int(3), vec![0x05]].concat(), "(1+2)*3"),
    ];
    let data: Vec<u8> =
        [row(0)].into_iter().chain(cases.iter().enumerate().map(|(i, (rgce, _))| formula_cell(i as u32, rgce, &[]))).flatten().collect();
    let wb = read(&package(&book_with_links(&xti()), &worksheet(&data, &[]), &sst(), ROOT, RELS, TYPES)).unwrap();
    for (i, (_, want)) in cases.iter().enumerate() {
        assert_eq!(wb.sheets[0].formula(CellRef::new(0, i as u32)), Some(*want));
    }
}

#[test]
fn functions_past_0x17b_are_called_by_number() {
    // [MS-XLSB] 2.5.98.10's numbers past the .xls table: IFERROR (PtgFunc) and SUMIFS (PtgFuncVar).
    let sumifs = [&[0x1Eu8, 1, 0][..], &[0x1E, 2, 0], &[0x17, 2, 0, b'>', 0, b'0', 0], &[0x42, 3, 0xE2, 0x01]].concat();
    let data = [row(0), formula_cell(0, &[0x1E, 1, 0, 0x1E, 2, 0, 0x41, 0xE0, 0x01], &[]), formula_cell(1, &sumifs, &[])].concat();
    let wb = read(&package(&book_with_links(&xti()), &worksheet(&data, &[]), &sst(), ROOT, RELS, TYPES)).unwrap();
    assert_eq!(wb.sheets[0].formula(CellRef::new(0, 0)), Some("IFERROR(1,2)"));
    assert_eq!(wb.sheets[0].formula(CellRef::new(0, 1)), Some("SUMIFS(1,2,\">0\")"));
}

#[test]
fn hostile_formula_tokens_keep_the_saved_result() {
    let mut deep = vec![0x1E, 1, 0];
    deep.resize(16384, 0x15); // Thousands of parentheses: refused by the depth guard.
    let empty_array = [&[0x60u8][..], &[0; 14]].concat();
    let bad: Vec<(Vec<u8>, Vec<u8>)> = vec![
        (vec![0x1E, 1], vec![]),                                             // Truncated PtgInt.
        (vec![0x17, 1, 0, 0x00, 0xD8], vec![]),                              // Unpaired surrogate.
        (vec![0x17, 0, 1], vec![]),                                          // Text constant over 255.
        ([&[0x24u8][..], &loc(1_048_576, 0)].concat(), vec![]),              // Row past the grid.
        (empty_array.clone(), vec![0; 8]),                                   // Empty array constant.
        (empty_array, vec![0xFF; 8]),                                        // Huge one.
        (vec![0x1E, 1, 0, 0x1E, 2, 0], vec![]),                              // Unused operand.
        (vec![0x21, 0xD4, 0x01], vec![]),                                    // Unassigned function number.
        (vec![0x2C, 0, 0, 0, 0, 0, 0], vec![]),                              // PtgRefN outside a shared formula.
        (vec![0x23, 0xFF, 0xFF, 0xFF, 0xFF], vec![]),                        // Name index past the list.
        ([&[0x5Au8][..], &9u16.to_le_bytes(), &loc(0, 0)].concat(), vec![]), // Missing sheet link.
        (deep, vec![]),
    ];
    for (rgce, rgcb) in bad {
        let data = [row(0), formula_cell(0, &rgce, &rgcb)].concat();
        let wb = read(&package(&book_with_links(&xti()), &worksheet(&data, &[]), &sst(), ROOT, RELS, TYPES)).unwrap();
        assert_eq!(wb.sheets[0].formula(CellRef::new(0, 0)), None, "{rgce:?}");
        assert_eq!(wb.sheets[0].value(CellRef::new(0, 0)), Value::Number(0.0));
    }
}

#[test]
fn python_formulas_need_their_code_in_the_cell() {
    // `PY("code", 0)` carries its code; `PY(0, 0)` names a script elsewhere in the package, which
    // the reader doesn't read, and keeps its saved result.
    let mut book = book_with_links(&xti());
    let end = record(132, &[]);
    book.truncate(book.len() - end.len());
    book.extend(name(0x0002_000b, "_xlfn._xlws.PY"));
    book.extend(end);
    let py = |arg: &[u8]| [&[0x23u8, 5, 0, 0, 0][..], arg, &[0x1E, 0, 0, 0x42, 3, 0xFF, 0]].concat();
    // Code longer than one string constant is written as constants joined with `&`.
    let joined = [&[0x17u8, 1, 0, b'1', 0][..], &[0x17, 2, 0, b'+', 0, b'1', 0], &[0x08]].concat();
    // The same strings nested to the right, `"1"&("+"&"1")` by token order: still one flat chain.
    let nested = [&[0x17u8, 1, 0, b'1', 0][..], &[0x17, 1, 0, b'+', 0], &[0x17, 1, 0, b'1', 0], &[0x08, 0x08]].concat();
    let data = [
        row(0),
        formula_cell(0, &py(&[0x17, 1, 0, b'1', 0]), &[]),
        formula_cell(1, &py(&[0x1E, 0, 0]), &[]),
        formula_cell(2, &py(&joined), &[]),
        formula_cell(3, &py(&nested), &[]),
    ]
    .concat();
    let wb = read(&package(&book, &worksheet(&data, &[]), &sst(), ROOT, RELS, TYPES)).unwrap();
    let formula = |c| wb.sheets[0].formula(CellRef::new(0, c));
    assert_eq!(formula(0), Some("_xlfn._xlws.PY(\"1\",0)"));
    assert_eq!(formula(1), None);
    assert_eq!(formula(2), Some("_xlfn._xlws.PY(\"1\"&\"+1\",0)"));
    assert_eq!(formula(3), Some("_xlfn._xlws.PY(\"1\"&\"+\"&\"1\",0)"));
    assert!(warned(&wb, "Python code stored outside"), "{:?}", wb.warnings);
}

#[test]
fn formula_text_past_the_allowance_keeps_the_saved_result() {
    // `1` takes the last byte of a one-byte allowance; the next formula doesn't fit.
    let data = [row(0), formula_cell(0, &[0x1E, 1, 0], &[]), formula_cell(1, &[0x1E, 1, 0], &[])].concat();
    let limits = Limits { binary_formula_text: 1, ..Limits::default() };
    let wb = read_with(&package(&book_with_links(&xti()), &worksheet(&data, &[]), &sst(), ROOT, RELS, TYPES), &limits).unwrap();
    assert_eq!(wb.sheets[0].formula(CellRef::new(0, 0)), Some("1"));
    assert_eq!(wb.sheets[0].formula(CellRef::new(0, 1)), None);
    assert!(warned(&wb, "more formula text than the limits allow"), "{:?}", wb.warnings);
}

fn range_formula(id: u16, (r0, c0, r1, c1): (u32, u32, u32, u32), rgce: &[u8]) -> Vec<u8> {
    let mut data = vec![];
    for n in [r0, r1, c0, c1] {
        data.extend(n.to_le_bytes());
    }
    if id == 426 {
        data.push(0x08); // The flags Excel writes for an ordinary array.
    }
    data.extend((rgce.len() as u32).to_le_bytes());
    data.extend(rgce);
    data.extend(0u32.to_le_bytes());
    record(id, &data)
}

/// A cell of the shared or array formula whose first cell is `row`, `col` (PtgExp + PtgExtraCol).
fn part(col: u32, (row, first_col): (u32, u32)) -> Vec<u8> {
    formula_cell(col, &[&[0x01u8][..], &row.to_le_bytes()].concat(), &first_col.to_le_bytes())
}

fn ref_n(row: i32, col: u16) -> Vec<u8> {
    [&[0x4Cu8][..], &row.to_le_bytes(), &col.to_le_bytes()].concat()
}

fn at(a1: &str) -> CellRef {
    CellRef::parse(a1).unwrap()
}

#[test]
fn shared_array_and_dynamic_formulas_read_as_from_xlsx() {
    // As Excel writes them: C1:C3 share `A1*10` (PtgRefN, column offset -2 as 16382); D1:E2
    // share `$A1+A$1`; F1:F2 hold the Ctrl+Shift+Enter array {=A1:A2*2}; G1 spills `=A1:A2`.
    let area = [&[0x45u8][..], &0u32.to_le_bytes(), &1u32.to_le_bytes(), &REL.to_le_bytes(), &REL.to_le_bytes()].concat();
    let mut data = row(0);
    data.extend(cell(5, 0, &1.0f64.to_le_bytes()));
    data.extend(part(2, (0, 2)));
    data.extend(range_formula(427, (0, 2, 2, 2), &[ref_n(0, 16382 | REL), vec![0x1E, 10, 0, 0x05]].concat()));
    data.extend(part(3, (0, 3)));
    data.extend(range_formula(427, (0, 3, 1, 4), &[ref_n(0, 0x8000), ref_n(0, 16381 | 0x4000), vec![0x03]].concat()));
    data.extend(part(4, (0, 3)));
    data.extend(part(5, (0, 5)));
    data.extend(range_formula(426, (0, 5, 1, 5), &[area.clone(), vec![0x1E, 2, 0, 0x05]].concat()));
    data.extend(record(49, &1u32.to_le_bytes()));
    data.extend(record(50, &1u32.to_le_bytes())); // BrtValueMeta in between, as Excel writes for a spill error.
    data.extend(part(6, (0, 6)));
    data.extend(range_formula(426, (0, 6, 1, 6), &area));
    data.extend(row(1));
    data.extend(cell(5, 0, &2.0f64.to_le_bytes()));
    for (col, first) in [(2, (0, 2)), (3, (0, 3)), (4, (0, 3)), (5, (0, 5)), (6, (0, 6))] {
        data.extend(part(col, first));
    }
    data.extend(row(2));
    data.extend(part(2, (0, 2)));
    let wb = read(&file(&worksheet(&data, &[]))).unwrap();
    let sheet = &wb.sheets[0];
    let formula = |a: &str| sheet.formulas.get(&at(a));
    // Each cell of a shared formula has its own text, decoded at the cell: the first with the
    // group's range, all with its index.
    for (a, want, si) in [
        ("C1", "A1*10", 0),
        ("C2", "A2*10", 0),
        ("C3", "A3*10", 0),
        ("D1", "$A1+A$1", 1),
        ("E1", "$A1+B$1", 1),
        ("D2", "$A2+A$1", 1),
        ("E2", "$A2+B$1", 1),
    ] {
        let f = formula(a).unwrap_or_else(|| panic!("{a}"));
        assert_eq!((f.kind, f.text.as_str(), f.shared_index), (FormulaKind::Shared, want, Some(si)), "{a}");
    }
    assert_eq!(formula("C1").and_then(|f| f.range), Range::parse("C1:C3"));
    assert_eq!(formula("D1").and_then(|f| f.range), Range::parse("D1:E2"));
    assert_eq!(formula("C2").and_then(|f| f.range), None);
    // An array formula's first cell has it, with its range; a dynamic array's has its cell
    // metadata too, as `cm` in .xlsx, though Excel wrote a BrtValueMeta in between.
    let f1 = formula("F1").unwrap();
    assert_eq!((f1.kind, f1.text.as_str(), f1.range), (FormulaKind::Array, "A1:A2*2", Range::parse("F1:F2")));
    let g1 = formula("G1").unwrap();
    assert_eq!((g1.kind, g1.text.as_str(), g1.range), (FormulaKind::Array, "A1:A2", Range::parse("G1:G2")));
    assert_eq!(sheet.cell_metadata.get(&at("G1")), Some(&1));
    assert_eq!(sheet.cell_metadata.get(&at("F1")), None);
    // Their other cells have no formula and keep their saved results, as in .xlsx.
    for a in ["F2", "G2"] {
        assert!(formula(a).is_none(), "{a}");
        assert_eq!(sheet.value(at(a)), Value::Number(0.0), "{a}");
    }
    assert!(!wb.warnings.iter().any(|w| w.contains("kept as their saved results")), "{:?}", wb.warnings);
}

#[test]
fn unreadable_shared_formulas_keep_their_values() {
    // A shared formula with an unsupported token, a range that doesn't start at its first cell,
    // a truncated record, and a first cell with no formula record after it: every cell of each
    // keeps its saved result, and the file opens.
    let unknown = range_formula(427, (0, 0, 1, 0), &[0xFF]);
    let wrong_start = range_formula(427, (0, 1, 1, 1), &[0x1E, 1, 0]);
    // A well-framed record whose payload stops before rgcb's length.
    let short: Vec<u8> = [&[0u8; 4][..], &1u32.to_le_bytes(), &[0; 8], &3u32.to_le_bytes(), &[0x1E, 1, 0]].concat();
    let truncated = record(427, &short);
    for after_first in [unknown, wrong_start, truncated, cell(5, 1, &1.0f64.to_le_bytes())] {
        let data = [row(0), part(0, (0, 0)), after_first, row(1), part(0, (0, 0))].concat();
        let wb = read(&file(&worksheet(&data, &[]))).unwrap();
        for r in [0, 1] {
            assert_eq!(wb.sheets[0].formula(CellRef::new(r, 0)), None);
            assert_eq!(wb.sheets[0].value(CellRef::new(r, 0)), Value::Number(0.0));
        }
        assert!(wb.warnings.iter().any(|w| w.contains("kept as their saved results")), "{:?}", wb.warnings);
    }
}

#[test]
fn shared_formulas_wrap_and_may_start_inside_their_range() {
    // `=` the cell to the left (column offset -1, written 16383), shared over A1:B2 from B1: A1
    // isn't part of it. In column A the offset wraps around the grid, as [MS-XLSB] defines it.
    let left = ref_n(0, 16383 | REL);
    let data = [
        row(0),
        cell(5, 0, &1.0f64.to_le_bytes()),
        part(1, (0, 1)),
        range_formula(427, (0, 0, 1, 1), &left),
        row(1),
        part(0, (0, 1)),
        part(1, (0, 1)),
    ]
    .concat();
    let wb = read(&file(&worksheet(&data, &[]))).unwrap();
    let text = |a: &str| wb.sheets[0].formula(at(a));
    assert_eq!((text("A1"), text("B1"), text("A2"), text("B2")), (None, Some("A1"), Some("XFD2"), Some("A2")));
    assert!(!wb.warnings.iter().any(|w| w.contains("kept as their saved results")), "{:?}", wb.warnings);
}

#[test]
fn a_shared_formula_copy_may_be_longer_than_its_first_cell() {
    // SUM of nine SUMs of 255 references to the cell on the left: about 6,900 characters at B9,
    // over 18,000 at B999999, where each `A9` is `A999999`. Each copy has its own text.
    let mut rgce = vec![];
    for _ in 0..9 {
        for _ in 0..255 {
            rgce.extend(ref_n(0, 16383 | REL));
        }
        rgce.extend([0x22, 255, 4, 0]);
    }
    rgce.extend([0x22, 9, 4, 0]);
    let data = [row(8), part(1, (8, 1)), range_formula(427, (8, 1, 999_998, 1), &rgce), row(999_998), part(1, (8, 1))].concat();
    let wb = read(&file(&worksheet(&data, &[]))).unwrap();
    let (b9, last) = (wb.sheets[0].formula(at("B9")).unwrap(), wb.sheets[0].formula(at("B999999")).unwrap());
    assert!(b9.len() < 8192 && last.len() > 18_000, "{} {}", b9.len(), last.len());
    assert!(last.starts_with("SUM(SUM(A999999,A999999"), "{}", &last[..40]);
}

#[test]
fn shared_formula_copies_are_charged_before_they_are_made() {
    // A1:A3 share `1+1` (3 bytes); with 6 bytes allowed, the first cell and one copy fit, and the
    // last copy keeps its value without being decoded.
    let one_plus_one = [0x1E, 1, 0, 0x1E, 1, 0, 0x03];
    let data = [row(0), part(0, (0, 0)), range_formula(427, (0, 0, 2, 0), &one_plus_one), row(1), part(0, (0, 0)), row(2), part(0, (0, 0))].concat();
    let limits = Limits { binary_formula_text: 6, ..Limits::default() };
    let wb = read_with(&file(&worksheet(&data, &[])), &limits).unwrap();
    let got: Vec<_> = (0..3).map(|r| wb.sheets[0].formula(CellRef::new(r, 0))).collect();
    assert_eq!(got, [Some("1+1"), Some("1+1"), None]);
    assert!(warned(&wb, "more formula text than the limits allow"), "{:?}", wb.warnings);
}

#[test]
fn a_shared_formula_stops_decoding_copies_once_one_is_kept_as_its_value() {
    // A1:Z2 share `=` the cell to the right. A1 is `B1` (2 bytes), Z1's copy is `AA1` (3): it
    // passes the estimate from the first cell but not the allowance, after being decoded. The
    // group's later copies keep their values without being decoded, though B2's (`C2`) would fit.
    let right = ref_n(0, 1 | REL);
    let data = [row(0), part(0, (0, 0)), range_formula(427, (0, 0, 1, 25), &right), part(25, (0, 0)), row(1), part(1, (0, 0))].concat();
    let limits = Limits { binary_formula_text: 4, ..Limits::default() };
    let wb = read_with(&file(&worksheet(&data, &[])), &limits).unwrap();
    let text = |a: &str| wb.sheets[0].formula(at(a));
    assert_eq!((text("A1"), text("Z1"), text("B2")), (Some("B1"), None, None));
    assert!(warned(&wb, "more formula text than the limits allow"), "{:?}", wb.warnings);
}

#[test]
fn convert_is_called_by_its_number_as_excel_writes_it() {
    // Excel's tokens for `=CONVERT(1,"m","ft")` in .xlsb: PtgFunc 0x1D4, which the
    // specification's Ftab table leaves out.
    let rgce = [0x1E, 0x01, 0x00, 0x17, 0x01, 0x00, 0x6D, 0x00, 0x17, 0x02, 0x00, 0x66, 0x00, 0x74, 0x00, 0x41, 0xD4, 0x01];
    let wb = read(&file(&worksheet(&[row(0), formula_cell(0, &rgce, &[])].concat(), &[]))).unwrap();
    assert_eq!(wb.sheets[0].formula(CellRef::new(0, 0)), Some("CONVERT(1,\"m\",\"ft\")"));
}

#[test]
fn a_dynamic_array_kept_as_its_value_has_no_cell_metadata() {
    let meta = record(49, &1u32.to_le_bytes());
    let table_ref = [0x18, 0x19, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0];
    let data = [
        row(0),
        // A1: a dynamic-array formula the reader can't decode (a table reference).
        meta.clone(),
        formula_cell(0, &table_ref, &[]),
        // B1:B2: a dynamic array whose BrtArrFmla holds an unknown token.
        meta.clone(),
        part(1, (0, 1)),
        range_formula(426, (0, 1, 1, 1), &[0xFF]),
        // C1: a value with cell metadata keeps it, as the file has it.
        meta.clone(),
        cell(5, 2, &1.0f64.to_le_bytes()),
        // D1: the first cell of a dynamic array with no BrtArrFmla after it, then a value, E1.
        meta.clone(),
        part(3, (0, 3)),
        cell(5, 4, &1.0f64.to_le_bytes()),
        // F1: the same as the last cell of its row.
        meta,
        part(5, (0, 5)),
        row(1),
        cell(5, 0, &1.0f64.to_le_bytes()),
    ]
    .concat();
    let wb = read(&file(&worksheet(&data, &[]))).unwrap();
    let sheet = &wb.sheets[0];
    for a in ["A1", "B1", "D1", "F1"] {
        assert_eq!(sheet.formula(at(a)), None, "{a}");
    }
    assert_eq!(sheet.cell_metadata.iter().collect::<Vec<_>>(), [(&at("C1"), &1)]);
    assert!(warned(&wb, "missing from the file"), "{:?}", wb.warnings);
}

// Defined names ([MS-XLSB] 2.4.718): `BrtName` records, and the `definedNames` Excel writes for
// the same names when it saves the workbook as `.xlsx`.

/// `BrtName`. A macro's name (`fProc`, 0x08) has four more strings, null here.
fn brt_name(flags: u32, key: u8, itab: u32, text: &str, rgce: &[u8], comment: Option<&str>) -> Vec<u8> {
    let mut data = flags.to_le_bytes().to_vec();
    data.push(key);
    data.extend(itab.to_le_bytes());
    data.extend(wide(text));
    data.extend((rgce.len() as u32).to_le_bytes());
    data.extend(rgce);
    data.extend(0u32.to_le_bytes());
    data.extend(comment.map_or_else(|| u32::MAX.to_le_bytes().to_vec(), wide));
    if flags & 0x08 != 0 {
        data.extend(u32::MAX.to_le_bytes().repeat(4));
    }
    record(39, &data)
}
fn ref3d_name(ixti: u16, row: u32, col: u16) -> Vec<u8> {
    [&[0x3Au8][..], &ixti.to_le_bytes(), &loc(row, col)].concat()
}
const WORKBOOK: u32 = u32::MAX;

/// `book(false)` with the supporting links of `book_with_links` (sheet links Data, Hidden,
/// Data:Hidden, a deleted sheet, the other workbook, and this workbook as a whole, which
/// `PtgNameX` uses), and `names`.
fn book_with_names(names: &[Vec<u8>]) -> Vec<u8> {
    let plain = book(false);
    let end = record(132, &[]);
    let mut xti = 6u32.to_le_bytes().to_vec();
    for (book, first, last) in [(0u32, 0i32, 0i32), (0, 1, 1), (0, 0, 1), (0, -1, -1), (1, 0, 0), (0, -2, -2)] {
        xti.extend([book.to_le_bytes(), first.to_le_bytes(), last.to_le_bytes()].concat());
    }
    let links = [record(353, &[]), record(357, &[]), record(355, &wide("rId9")), record(362, &xti), record(354, &[])];
    [&plain[..plain.len() - end.len()], &links.concat(), &names.concat(), &end].concat()
}

/// The names below, in record order, as `PtgName` numbers them from 1.
fn names() -> Vec<Vec<u8>> {
    vec![
        // 1: a newer function's hidden name: not a defined name in `.xlsx`.
        brt_name(0x0002_000b, 0, WORKBOOK, "_xlfn.CONCAT", &[0x1C, 0x1D], None),
        // 2: built-in and hidden, on Data.
        brt_name(0x21, 0, 0, "_FilterDatabase", &[&[0x3Bu8, 0, 0][..], &0u32.to_le_bytes(), &9u32.to_le_bytes(), &[0, 0, 2, 0]].concat(), None),
        // 3: refers to another workbook: left out.
        brt_name(0, 0, WORKBOOK, "External", &ref3d_name(4, 0, 0), None),
        // 4: a deleted cell.
        brt_name(0, 0, WORKBOOK, "Gone", &[0x3C, 0, 0, 0, 0, 0, 0, 0, 0], None),
        brt_name(1, 0, WORKBOOK, "Hid", &[0x1E, 42, 0], None),
        // 6, 7: the same name on two sheets.
        brt_name(0, 0, 0, "Local", &ref3d_name(0, 1, 1), None),
        brt_name(0, 0, 1, "Local", &ref3d_name(1, 0, 1), None),
        // 8: an XLM macro with a shortcut key, in the "User Defined" group (14).
        brt_name(0x388, b'k', WORKBOOK, "MacroName", &ref3d_name(0, 0, 0), None),
        // 9
        brt_name(0, 0, WORKBOOK, "Rate", &[&[0x1Fu8][..], &0.05f64.to_le_bytes()].concat(), Some("a note")),
        // 10, 11: relative references are offsets from A1, wrapping around the grid.
        brt_name(0, 0, WORKBOOK, "Rel", &ref3d_name(0, 0, 0xFFFF), None),
        brt_name(0, 0, WORKBOOK, "RelN", &[&[0x2Cu8][..], &loc(u32::MAX, REL)].concat(), None),
        // 12: on a sheet the workbook doesn't have: left out.
        brt_name(0, 0, 5, "Skipped", &[0x1E, 1, 0], None),
        // 13: a name of a name.
        brt_name(0, 0, WORKBOOK, "Twice", &[0x23, 9, 0, 0, 0, 0x1E, 2, 0, 0x05], None),
    ]
}

const NAMES_XML: &str = r#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><definedNames><definedName name="_xlnm._FilterDatabase" localSheetId="0" hidden="1">Data!$A$1:$C$10</definedName><definedName name="Gone">Data!#REF!</definedName><definedName name="Hid" hidden="1">42</definedName><definedName name="Local" localSheetId="0">Data!$B$2</definedName><definedName name="Local" localSheetId="1">Hidden!$B$1</definedName><definedName name="MacroName" xlm="1" functionGroupId="14" shortcutKey="k">Data!$A$1</definedName><definedName name="Rate" comment="a note">0.05</definedName><definedName name="Rel">Data!XFD1</definedName><definedName name="RelN">A1048576</definedName><definedName name="Twice">Rate*2</definedName></definedNames></workbook>"#;

fn xml_names(xml: &str) -> Vec<craft_xlsx::DefinedName> {
    let types = r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/></Types>"#;
    let root = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="r" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#;
    let bytes = zip(&[("[Content_Types].xml", types.as_bytes()), ("_rels/.rels", root.as_bytes()), ("xl/workbook.xml", xml.as_bytes())]);
    Package::open(&bytes, &Limits::default()).unwrap().workbook().unwrap().defined_names
}

#[test]
fn defined_names_are_read_as_their_xlsx_spelling() {
    let cells: Vec<Vec<u8>> = vec![
        vec![0x23, 9, 0, 0, 0, 0x1E, 100, 0, 0x05],
        vec![0x23, 6, 0, 0, 0],
        // PtgNameX through this workbook's own link: a sheet-scoped name used from another sheet.
        vec![0x39, 5, 0, 7, 0, 0, 0],
        vec![0x23, 2, 0, 0, 0],
        vec![0x3C, 1, 0, 0, 0, 0, 0, 0, 0],
        // Kept as their saved results: a name left out, a name in another workbook, and a
        // workbook name through PtgNameX.
        vec![0x23, 3, 0, 0, 0],
        vec![0x39, 4, 0, 9, 0, 0, 0],
        vec![0x39, 5, 0, 9, 0, 0, 0],
    ];
    let mut data = row(0);
    for (i, rgce) in cells.iter().enumerate() {
        data.extend(formula_cell(i as u32 + 1, rgce, &[]));
    }
    let bytes = package(&book_with_names(&names()), &worksheet(&data, &[]), &sst(), ROOT, RELS, TYPES);
    let wb = Package::open(&bytes, &Limits::default()).unwrap().workbook().unwrap();
    assert_eq!(wb.defined_names, xml_names(NAMES_XML));
    let book = read(&bytes).unwrap();
    let got: Vec<_> = (1..=8).map(|c| book.sheets[0].formula(CellRef::new(0, c))).collect();
    let want = [Some("Rate*100"), Some("Local"), Some("Hidden!Local"), Some("_xlnm._FilterDatabase"), Some("Hidden!#REF!"), None, None, None];
    assert_eq!(got, want);
    for why in ["defined names whose formula could not be read", "other workbooks", "names in other workbooks or on other sheets"] {
        assert!(warned(&book, why), "{why}: {:?}", book.warnings);
    }
    for why in ["(references to other workbooks (external links))", "(names of sheets the reader skips)"] {
        assert!(book.warnings.iter().any(|w| w.contains("defined names were left out") && w.contains(why)), "{why}: {:?}", book.warnings);
    }
}

#[test]
fn a_damaged_name_leaves_names_and_formulas_as_values() {
    // A truncated BrtName: the workbook opens with no names, and every formula keeps its value.
    let mut list = names();
    let full = brt_name(0, 0, WORKBOOK, "Rate", &[0x1E, 1, 0], None);
    list[8] = record(39, &full[2..full.len() - 2]); // The payload without its record header and last two bytes.
    let data = [row(0), formula_cell(1, &[0x1E, 1, 0], &[])].concat();
    let bytes = package(&book_with_names(&list), &worksheet(&data, &[]), &sst(), ROOT, RELS, TYPES);
    let wb = Package::open(&bytes, &Limits::default()).unwrap().workbook().unwrap();
    assert!(wb.defined_names.is_empty());
    let book = read(&bytes).unwrap();
    assert_eq!(book.sheets[0].formula(CellRef::new(0, 1)), None);
    assert!(book.warnings.iter().any(|w| w.contains("names could not be read")), "{:?}", book.warnings);
}

#[test]
fn names_on_macro_sheets_spans_over_chart_sheets_and_empty_built_ins() {
    // Sheets Data, a chart sheet, an XLM macro sheet, Hidden; sheet links Macro1, Data:Hidden
    // (over both) and Data.
    let mut book = record(131, &[]);
    book.extend(record(153, &[[0u8; 8].as_slice(), &wide("")].concat()));
    book.extend(record(143, &[]));
    for (id, rel, name) in [(1u32, "one", "Data"), (3, "c", "Chart1"), (4, "m", "Macro1"), (2, "two", "Hidden")] {
        book.extend(record(156, &[&0u32.to_le_bytes()[..], &id.to_le_bytes(), &wide(rel), &wide(name)].concat()));
    }
    book.extend(record(144, &[]));
    let mut xti = 3u32.to_le_bytes().to_vec();
    for (first, last) in [(2i32, 2i32), (0, 3), (0, 0)] {
        xti.extend([0u32.to_le_bytes(), first.to_le_bytes(), last.to_le_bytes()].concat());
    }
    book.extend([record(353, &[]), record(357, &[]), record(362, &xti), record(354, &[])].concat());
    let area = [&[0x3Bu8][..], &2u16.to_le_bytes(), &0u32.to_le_bytes(), &1u32.to_le_bytes(), &0u16.to_le_bytes(), &1u16.to_le_bytes()].concat();
    // A sheet's built-in Database, then the workbook-level one Excel writes with no formula.
    book.extend(brt_name(0x20, 0, 0, "Database", &area, None));
    book.extend(brt_name(0x20, 0, WORKBOOK, "Database", &[], None));
    book.extend(brt_name(0x388, b'k', WORKBOOK, "Cmd", &ref3d_name(0, 0, 0), None));
    book.extend(brt_name(0, 0, WORKBOOK, "Span", &ref3d_name(1, 0, 0), None));
    book.extend(record(132, &[]));
    let rels = RELS.replace(
        "</Relationships>",
        r#"<Relationship Id="c" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/chartsheet" Target="../data/c.bin"/><Relationship Id="m" Type="http://schemas.microsoft.com/office/2006/relationships/xlMacrosheet" Target="../data/m.bin"/></Relationships>"#,
    );
    let ref3d_cell = |ixti: u16| [&[0x5Au8][..], &ixti.to_le_bytes(), &loc(0, 0)].concat();
    let data = [
        row(0),
        formula_cell(1, &[0x23, 2, 0, 0, 0], &[]),
        formula_cell(2, &[0x23, 4, 0, 0, 0], &[]),
        formula_cell(3, &ref3d_cell(0), &[]),
        formula_cell(4, &ref3d_cell(1), &[]),
    ]
    .concat();
    let bytes = package(&book, &worksheet(&data, &[]), &sst(), ROOT, &rels, TYPES);
    let wb = Package::open(&bytes, &Limits::default()).unwrap().workbook().unwrap();
    let xml = r#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><definedNames><definedName name="_xlnm.Database" localSheetId="0">Data!$A$1:$B$2</definedName><definedName name="Cmd" xlm="1" functionGroupId="14" shortcutKey="k">Macro1!$A$1</definedName><definedName name="Span">Data:Hidden!$A$1</definedName></definedNames></workbook>"#;
    assert_eq!(wb.defined_names, xml_names(xml));
    let read = read(&bytes).unwrap();
    let got: Vec<_> = (1..=4).map(|c| read.sheets[0].formula(CellRef::new(0, c))).collect();
    assert_eq!(got, [Some("_xlnm.Database"), Some("Span"), Some("Macro1!$A$1"), Some("Data:Hidden!$A$1")]);
    assert!(read.warnings.iter().all(|w| !w.contains("left out") && !w.contains("saved results")), "{:?}", read.warnings);
}

/// `BrtRowHdr` with its style, height and three flag bytes, and no column spans.
fn row_hdr(r: u32, ixfe: u32, twips: u16, flags: [u8; 3]) -> Vec<u8> {
    record(0, &[&r.to_le_bytes()[..], &ixfe.to_le_bytes(), &twips.to_le_bytes(), &flags, &0u32.to_le_bytes()].concat())
}
/// `BrtColInfo`.
fn col_info(first: u32, last: u32, coldx: u32, ixfe: u32, flags: u16) -> Vec<u8> {
    record(60, &[&first.to_le_bytes()[..], &last.to_le_bytes(), &coldx.to_le_bytes(), &ixfe.to_le_bytes(), &flags.to_le_bytes()].concat())
}
/// `BrtWsFmtInfo`.
fn ws_fmt_info(dx: u32, base: u16, twips: u16, flags: u16, levels: [u8; 2]) -> Vec<u8> {
    record(485, &[&dx.to_le_bytes()[..], &base.to_le_bytes(), &twips.to_le_bytes(), &flags.to_le_bytes(), &levels].concat())
}
/// A worksheet with `head` before its data (the sheet format and columns) and `rows` in it.
fn sized_sheet(head: &[u8], rows: &[u8]) -> Vec<u8> {
    [&record(129, &[])[..], head, &record(145, &[]), rows, &record(146, &[]), &record(130, &[])].concat()
}
fn sizes_of(bytes: &[u8]) -> Result<(Vec<Row>, SheetPart, Vec<String>), Error> {
    let mut p = Package::open(bytes, &Limits::default())?;
    let wb = p.workbook()?;
    let mut rows = vec![];
    let part = p.sheet(&wb.sheets[0], |r| {
        rows.push(r);
        Ok(())
    })?;
    Ok((rows, part, p.warnings().to_vec()))
}
fn xml_sheet(xml: &str) -> (Vec<Row>, SheetPart) {
    let types = r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/sheet.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>"#;
    let root = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="r" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#;
    let rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="s" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="sheet.xml"/></Relationships>"#;
    let book = r#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Sizes" sheetId="1" r:id="s"/></sheets></workbook>"#;
    let bytes = zip(&[
        ("[Content_Types].xml", types.as_bytes()),
        ("_rels/.rels", root.as_bytes()),
        ("xl/workbook.xml", book.as_bytes()),
        ("xl/_rels/workbook.xml.rels", rels.as_bytes()),
        ("xl/sheet.xml", xml.as_bytes()),
    ]);
    let (rows, part, _) = sizes_of(&bytes).unwrap();
    (rows, part)
}

/// The binary sheet below as Excel spells it in `.xlsx`. Excel writes `ht` for a row whose
/// height differs from the default even when it was not set by hand.
const SIZES_XML: &str = r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetFormatPr baseColWidth="10" defaultColWidth="12.59765625" defaultRowHeight="14.25" zeroHeight="1" thickBottom="1" outlineLevelRow="2" outlineLevelCol="1"/><cols><col min="3" max="3" width="20.59765625" customWidth="1"/><col min="4" max="6" width="0" hidden="1" customWidth="1" outlineLevel="1"/><col min="8" max="8" width="9.06640625" style="7"/><col min="9" max="9" width="23.73046875" bestFit="1" customWidth="1"/><col min="10" max="10" width="9.06640625" collapsed="1"/><col min="16384" max="16384" width="30.59765625" customWidth="1"/></cols><sheetData><row r="1"><c r="A1"><v>1</v></c></row><row r="2" ht="30" customHeight="1"/><row r="3" ht="26.65"/><row r="4" hidden="1"/><row r="5" ht="15" thickTop="1" thickBot="1"/><row r="6" outlineLevel="2"/><row r="7" hidden="1" outlineLevel="1"/><row r="8" collapsed="1"/><row r="9" s="3" customFormat="1"/><row r="10"/><row r="11" ph="1"/><row r="12"/></sheetData></worksheet>"#;

fn sizes_bin() -> Vec<u8> {
    let head = [
        ws_fmt_info(3225, 10, 285, 0x0A, [2, 1]),
        record(390, &[]),
        col_info(2, 2, 5273, 0, 0x0002),
        col_info(3, 5, 0, 0, 0x0103),
        col_info(7, 7, 2321, 7, 0),
        col_info(8, 8, 6075, 0, 0x0006),
        col_info(9, 9, 2321, 0, 0x1000),
        col_info(16_383, 16_383, 7833, 0, 0x0002),
        record(391, &[]),
    ]
    .concat();
    let rows = [
        row_hdr(0, 0, 285, [0, 0, 0]),
        cell(2, 0, &((1u32 << 2) | 2).to_le_bytes()),
        row_hdr(1, 0, 600, [0, 0x20, 0]),
        row_hdr(2, 0, 533, [0, 0, 0]),
        row_hdr(3, 0, 285, [0, 0x10, 0]),
        row_hdr(4, 0, 300, [3, 0, 0]),
        row_hdr(5, 0, 285, [0, 2, 0]),
        row_hdr(6, 0, 285, [0, 0x11, 0]),
        row_hdr(7, 0, 285, [0, 0x08, 0]),
        row_hdr(8, 3, 285, [0, 0x40, 0]),
        // A style without fGhostDirty is not applied.
        row_hdr(9, 5, 285, [0, 0, 0]),
        row_hdr(10, 0, 285, [0, 0, 1]),
        // Past 8192 twips the height is ignored, set by hand or not.
        row_hdr(11, 0, 9000, [0, 0x20, 0]),
    ]
    .concat();
    sized_sheet(&head, &rows)
}

#[test]
fn row_heights_and_column_widths_are_read_as_their_xlsx_spelling() {
    let (rows, part, _) = sizes_of(&file(&sizes_bin())).unwrap();
    let (xml_rows, xml_part) = xml_sheet(SIZES_XML);
    assert_eq!(rows, xml_rows);
    assert_eq!(part.columns, xml_part.columns);
    assert_eq!(part.features.format, xml_part.features.format);
    // A few of the conversions, spelled out.
    assert_eq!((rows[1].height, rows[1].custom_height), (Some(30.0), true));
    assert_eq!((rows[2].height, rows[2].custom_height), (Some(26.65), false));
    assert_eq!(rows[8].style, Some(3));
    assert_eq!(part.columns[0].width, Some(5273.0 / 256.0));
    let format = part.features.format.clone().unwrap();
    assert_eq!((format.default_col_width, format.default_row_height), (Some(12.59765625), Some(14.25)));
    // Written as SpreadsheetML, the binary sheet has its default sizes and columns.
    let written = String::from_utf8(part.to_xml(&rows)).unwrap();
    assert!(written.contains(r#"<sheetFormatPr baseColWidth="10" defaultColWidth="12.59765625" defaultRowHeight="14.25""#), "{written}");
    assert!(written.contains(r#"<cols><col min="3" max="3" width="20.59765625" customWidth="1"/>"#), "{written}");
    assert!(written.contains(r#"<row r="2" ht="30" customHeight="1"/>"#), "{written}");
}

#[test]
fn damaged_sizes_are_skipped_or_refused() {
    // A column range outside the grid, or backwards, is skipped with a warning.
    let head = [record(390, &[]), col_info(0, 16_384, 2321, 0, 0), col_info(5, 4, 2321, 0, 0), col_info(1, 1, 512, 0, 2), record(391, &[])].concat();
    let (_, part, warnings) = sizes_of(&file(&sized_sheet(&head, &row(0)))).unwrap();
    assert_eq!(part.columns.len(), 1);
    assert_eq!((part.columns[0].first, part.columns[0].width), (1, Some(2.0)));
    assert!(warnings.iter().any(|w| w.contains("column ranges")), "{warnings:?}");
    // Without a sheet format, only a height set by hand is a row's `ht`.
    let (rows, _, _) = sizes_of(&file(&sized_sheet(&[], &[row_hdr(0, 0, 300, [0, 0, 0]), row_hdr(1, 0, 400, [0, 0x20, 0])].concat()))).unwrap();
    assert_eq!((rows[0].height, rows[1].height), (None, Some(20.0)));
    // More column records than columns are refused.
    let many = [&record(390, &[])[..], &col_info(0, 0, 2321, 0, 0).repeat(16_385), &record(391, &[])].concat();
    assert!(matches!(sizes_of(&file(&sized_sheet(&many, &[]))), Err(Error::TooLarge(_))));
    // Skipped (reversed) column records count toward that cap too.
    let reversed = [&record(390, &[])[..], &col_info(5, 4, 2321, 0, 0).repeat(16_385), &record(391, &[])].concat();
    assert!(matches!(sizes_of(&file(&sized_sheet(&reversed, &[]))), Err(Error::TooLarge(_))));
    // A sheet format or column after the data, or a record of the wrong size, fails the read.
    for (why, sheet) in [
        ("format after data", worksheet(&[], &ws_fmt_info(u32::MAX, 8, 285, 0, [0, 0]))),
        ("column after data", worksheet(&[], &col_info(0, 0, 2321, 0, 0))),
        ("short column", sized_sheet(&record(60, &[0; 16]), &[])),
        ("short format", sized_sheet(&record(485, &[0; 11]), &[])),
        ("long row header", sized_sheet(&[], &record(0, &[0; 18]))),
    ] {
        assert!(matches!(sizes_of(&file(&sheet)), Err(Error::Invalid(_))), "{why}");
    }
}

// Styles ([MS-XLSB] 2.1.7.50): a styles part assembled from records, and the `styles.xml` Excel
// writes for the same formats when it saves the workbook as `.xlsx`.

const STYLES_RELS: &str = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="one" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="../data/one.bin"/><Relationship Id="two" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="../data/two.bin"/><Relationship Id="styles" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="../look/styles.bin"/></Relationships>"#;
const STYLES_TYPES: &str = r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/records/book.bin" ContentType="application/vnd.ms-excel.sheet.binary.macroEnabled.main"/><Override PartName="/data/one.bin" ContentType="application/vnd.ms-excel.worksheet"/><Override PartName="/data/two.bin" ContentType="application/vnd.ms-excel.worksheet"/><Override PartName="/look/styles.bin" ContentType="application/vnd.ms-excel.styles"/></Types>"#;

fn styled_file(styles: &[u8], types: &str) -> Vec<u8> {
    let sheet = worksheet(&[row(0), cell(5, 0, &1.0f64.to_le_bytes())].concat(), &[]);
    zip(&[
        ("[Content_Types].xml", types.as_bytes()),
        ("_rels/.rels", ROOT.as_bytes()),
        ("records/book.bin", &book(false)),
        ("records/_rels/book.bin.rels", STYLES_RELS.as_bytes()),
        ("data/one.bin", &sheet),
        ("data/two.bin", &worksheet(&[], &[])),
        ("look/styles.bin", styles),
    ])
}

/// The styles part of a package, and the warnings reading it gave.
fn styles_of(bytes: &[u8]) -> (Option<Styles>, Vec<String>) {
    let mut p = Package::open(bytes, &Limits::default()).unwrap();
    let styles = p.styles().unwrap();
    (styles, p.warnings().to_vec())
}

/// `BrtColor`: auto 0, indexed 1, RGB 2, theme 3.
fn color(kind: u8, index: u8, tint: i16, rgba: [u8; 4]) -> Vec<u8> {
    [&[kind << 1 | 1, index][..], &tint.to_le_bytes(), &rgba].concat()
}
fn indexed(i: u8) -> Vec<u8> {
    color(1, i, 0, [0, 0, 0, 255])
}
fn theme(i: u8, tint: i16) -> Vec<u8> {
    color(3, i, tint, [0, 0, 0, 255])
}
fn rgb(r: u8, g: u8, b: u8) -> Vec<u8> {
    color(2, 255, 0, [r, g, b, 255])
}
#[allow(clippy::too_many_arguments)]
fn font(twips: u16, flags: u16, weight: u16, script: u16, underline: u8, family: u8, charset: u8, c: &[u8], scheme: u8, name: &str) -> Vec<u8> {
    let head =
        [&twips.to_le_bytes()[..], &flags.to_le_bytes(), &weight.to_le_bytes(), &script.to_le_bytes(), &[underline, family, charset, 0]].concat();
    record(43, &[&head[..], c, &[scheme], &wide(name)].concat())
}
fn pattern_fill(fls: u32, fg: &[u8], bg: &[u8]) -> Vec<u8> {
    record(45, &[&fls.to_le_bytes()[..], fg, bg, &[0; 48]].concat())
}
fn edge(dg: u8, c: &[u8]) -> Vec<u8> {
    [&[dg, 0][..], c].concat()
}
#[allow(clippy::too_many_arguments)]
fn brt_xf(parent: u16, fmt: u16, font: u16, fill: u16, border: u16, rotation: u8, indent: u8, flags: u16, apply: u8) -> Vec<u8> {
    let ids = [parent, fmt, font, fill, border].map(u16::to_le_bytes).concat();
    record(47, &[&ids[..], &[rotation, indent], &flags.to_le_bytes(), &[apply, 0]].concat())
}
fn style(xf: u32, flags: u16, builtin: u8, level: u8, name: &str) -> Vec<u8> {
    record(48, &[&xf.to_le_bytes()[..], &flags.to_le_bytes(), &[builtin, level], &wide(name)].concat())
}
fn prop(kind: u16, data: &[u8]) -> Vec<u8> {
    [&kind.to_le_bytes()[..], &(data.len() as u16 + 4).to_le_bytes(), data].concat()
}
fn short(text: &str) -> Vec<u8> {
    let units: Vec<u16> = text.encode_utf16().collect();
    [(units.len() as u16).to_le_bytes().to_vec(), units.iter().flat_map(|u| u.to_le_bytes()).collect()].concat()
}
fn brt_dxf(props: &[Vec<u8>]) -> Vec<u8> {
    record(507, &[&[0, 0x80, 0, 0][..], &(props.len() as u16).to_le_bytes(), &props.concat()].concat())
}
fn begin(id: u16, count: u32) -> Vec<u8> {
    record(id, &count.to_le_bytes())
}

/// Every kind of record the styles part has, as Excel writes them.
fn styles_bin() -> Vec<u8> {
    let normal = font(220, 0, 400, 0, 0, 2, 0, &theme(1, 0), 2, "Aptos Narrow");
    // Bold, italic, struck out, superscript, double accounting underline; family and
    // character set 0 and scheme "none" are not written in `.xlsx`.
    let fancy = font(290, 0x0A, 700, 1, 0x22, 0, 2, &rgb(0, 128, 255), 0, "Wingdings");
    let heading = font(220, 0, 400, 0, 0, 2, 0, &theme(4, 13106), 1, "Aptos Display");
    // An automatic colour: no `<color>` in `.xlsx`.
    let auto = font(220, 0x02, 400, 0, 0, 2, 0, &color(0, 64, 0, [0, 0, 0, 255]), 2, "Aptos Narrow");
    let gradient = [
        &0x28u32.to_le_bytes()[..],
        &indexed(64),
        &indexed(65),
        &1u32.to_le_bytes(),
        &[0.0f64, 0.5, 0.5, 0.5, 0.5].map(f64::to_le_bytes).concat(),
        &2u32.to_le_bytes(),
        &theme(0, 0),
        &0.0f64.to_le_bytes(),
        &theme(4, 0),
        &1.0f64.to_le_bytes(),
    ]
    .concat();
    let none = edge(0, &color(0, 0, 0, [0, 0, 0, 0]));
    let parts: Vec<Vec<u8>> = vec![
        record(278, &[]),
        begin(615, 2),
        record(44, &[&164u16.to_le_bytes()[..], &wide(r"yyyy\-mm\-dd")].concat()),
        // A format in two alternate-content blocks: the newer spelling for version 13 and later
        // (skipped), and the one for versions 12 and 13, which `.xlsx` has as `formatCode`.
        record(37, &[1, 0, 13, 0, 0, 0x80]),
        record(44, &[&165u16.to_le_bytes()[..], &wide("[$-ja-JP-x-gannen]ge")].concat()),
        record(38, &[]),
        record(37, &[2, 0, 12, 0, 0, 0, 13, 0, 0, 0]),
        record(44, &[&165u16.to_le_bytes()[..], &wide("[$]ge")].concat()),
        record(38, &[]),
        record(616, &[]),
        begin(611, 4),
        normal,
        fancy,
        heading,
        auto,
        record(612, &[]),
        begin(603, 6),
        pattern_fill(0, &indexed(64), &indexed(65)),
        pattern_fill(17, &indexed(64), &indexed(65)),
        pattern_fill(1, &rgb(255, 255, 0), &indexed(64)),
        pattern_fill(9, &theme(5, -8191), &indexed(65)),
        record(45, &gradient),
        // A tinted pattern colour: `.xlsx` states the system background, as for darkGrid above.
        pattern_fill(1, &theme(4, 26213), &indexed(65)),
        record(604, &[]),
        begin(613, 2),
        record(46, &[&[0][..], &none, &none, &none, &none, &none].concat()),
        record(
            46,
            &[&[3][..], &edge(2, &rgb(200, 0, 0)), &none, &edge(7, &indexed(64)), &edge(13, &color(0, 0, 0, [0, 0, 0, 0])), &edge(1, &theme(4, 0))]
                .concat(),
        ),
        record(614, &[]),
        begin(626, 3),
        brt_xf(0xFFFF, 0, 0, 0, 0, 0, 0, 0x1010, 0),
        brt_xf(0xFFFF, 9, 0, 0, 0, 0, 0, 0x1010, 0x3E),
        brt_xf(0xFFFF, 0, 2, 0, 0, 0, 0, 0x1010, 0),
        record(627, &[]),
        begin(617, 4),
        brt_xf(0, 0, 0, 0, 0, 0, 0, 0x1010, 0),
        brt_xf(0, 164, 0, 0, 0, 0, 0, 0x1010, 0x01),
        // Left, top, wrapped, shrunk, right to left, unlocked, hidden, quote prefix.
        brt_xf(0, 0, 0, 0, 0, 180, 2, 0x01 | 0x40 | 0x100 | 0x800 | 0x2000 | 0x8000, 0x24),
        brt_xf(1, 0, 1, 4, 1, 0, 0, 0x1010, 0x1A),
        record(618, &[]),
        begin(619, 3),
        style(0, 1, 0, 0, "Normal"),
        // Excel wraps each style in alternate content, which is skipped.
        record(37, &[1, 0, 2, 0x0e, 0, 0x80]),
        record(3072, &[0; 16]),
        record(38, &[]),
        style(1, 1 | 4, 1, 1, "RowLevel_2"),
        style(2, 2, 0, 0, "Probe Style"),
        record(620, &[]),
        begin(505, 2),
        brt_dxf(&[
            prop(0x05, &rgb(0x9C, 0, 6)),
            prop(0x19, &700u16.to_le_bytes()),
            prop(0x1C, &[0]),
            prop(0x1A, &0x21u16.to_le_bytes()),
            prop(0x24, &240u32.to_le_bytes()),
            prop(0x18, &short("Arial")),
            prop(0x29, &170u16.to_le_bytes()),
            prop(0x26, &short("0.0")),
            prop(0x02, &rgb(0xFF, 0xC7, 0xCE)),
            prop(0x0F, &[2]),
            prop(0x14, &[1]),
            prop(0x2B, &[0]),
            prop(0x07, &[&color(0, 0, 0, [0, 0, 0, 0])[..], &1u16.to_le_bytes()].concat()),
        ]),
        brt_dxf(&[
            prop(0x03, &[&0u32.to_le_bytes()[..], &[90.0f64, 0.0, 0.0, 0.0, 0.0].map(f64::to_le_bytes).concat()].concat()),
            prop(0x04, &[&[0, 0][..], &0.0f64.to_le_bytes(), &rgb(255, 255, 255)].concat()),
            prop(0x04, &[&[0, 0][..], &1.0f64.to_le_bytes(), &theme(4, 0)].concat()),
        ]),
        record(506, &[]),
        record(508, &[&0u32.to_le_bytes()[..], &wide("TableStyleMedium2"), &wide("PivotStyleLight16")].concat()),
        record(509, &[]),
        record(473, &[]),
        record(565, &[]),
        record(475, &[0x11, 0x22, 0x33, 0]),
        record(475, &[0xFF, 0xFF, 0xFF, 0]),
        record(566, &[]),
        record(474, &[]),
        record(279, &[]),
    ];
    parts.concat()
}

const STYLES_XML: &str = r#"<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><numFmts count="2"><numFmt numFmtId="164" formatCode="yyyy\-mm\-dd"/><numFmt numFmtId="165" formatCode="[$]ge"/></numFmts><fonts count="3"><font><sz val="11"/><color theme="1"/><name val="Aptos Narrow"/><family val="2"/><scheme val="minor"/></font><font><b/><i/><strike/><u val="doubleAccounting"/><vertAlign val="superscript"/><sz val="14.5"/><color rgb="FF0080FF"/><name val="Wingdings"/><charset val="2"/></font><font><sz val="11"/><color theme="4" tint="0.39997558519241921"/><name val="Aptos Display"/><family val="2"/><scheme val="major"/></font><font><i/><sz val="11"/><name val="Aptos Narrow"/><family val="2"/><scheme val="minor"/></font></fonts><fills count="5"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill><fill><patternFill patternType="solid"><fgColor rgb="FFFFFF00"/><bgColor indexed="64"/></patternFill></fill><fill><patternFill patternType="darkGrid"><fgColor theme="5" tint="-0.249977111117893"/><bgColor indexed="65"/></patternFill></fill><fill><gradientFill type="path" left="0.5" right="0.5" top="0.5" bottom="0.5"><stop position="0"><color theme="0"/></stop><stop position="1"><color theme="4"/></stop></gradientFill></fill><fill><patternFill patternType="solid"><fgColor theme="4" tint="0.79998168889431442"/><bgColor indexed="65"/></patternFill></fill></fills><borders count="2"><border><left/><right/><top/><bottom/><diagonal/></border><border diagonalUp="1" diagonalDown="1"><left style="hair"><color indexed="64"/></left><right style="slantDashDot"><color auto="1"/></right><top style="medium"><color rgb="FFC80000"/></top><bottom/><diagonal style="thin"><color theme="4"/></diagonal></border></borders><cellStyleXfs count="3"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/><xf numFmtId="9" fontId="0" fillId="0" borderId="0" applyFont="0" applyFill="0" applyBorder="0" applyAlignment="0" applyProtection="0"/><xf numFmtId="0" fontId="2" fillId="0" borderId="0"/></cellStyleXfs><cellXfs count="4"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/><xf numFmtId="164" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0" quotePrefix="1" applyAlignment="1" applyProtection="1"><alignment horizontal="left" vertical="top" textRotation="180" wrapText="1" indent="2" shrinkToFit="1" readingOrder="2"/><protection locked="0" hidden="1"/></xf><xf numFmtId="0" fontId="1" fillId="4" borderId="1" xfId="1" applyFont="1" applyFill="1" applyBorder="1"/></cellXfs><cellStyles count="3"><cellStyle name="Normal" xfId="0" builtinId="0"/><cellStyle name="RowLevel_2" xfId="1" builtinId="1" iLevel="1" customBuiltin="1"/><cellStyle name="Probe Style" xfId="2" hidden="1"/></cellStyles><dxfs count="2"><dxf><font><b/><i val="0"/><u val="singleAccounting"/><sz val="12"/><color rgb="FF9C0006"/><name val="Arial"/></font><numFmt numFmtId="170" formatCode="0.0"/><fill><patternFill><bgColor rgb="FFFFC7CE"/></patternFill></fill><alignment horizontal="center" wrapText="1"/><protection locked="0"/><border><bottom style="thin"><color auto="1"/></bottom></border></dxf><dxf><fill><gradientFill degree="90"><stop position="0"><color rgb="FFFFFFFF"/></stop><stop position="1"><color theme="4"/></stop></gradientFill></fill></dxf></dxfs><colors><indexedColors><rgbColor rgb="00112233"/><rgbColor rgb="00FFFFFF"/></indexedColors></colors></styleSheet>"#;

/// The same styles from a SpreadsheetML package.
fn xml_styles(xml: &str) -> Styles {
    let types = r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/></Types>"#;
    let root = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="r" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#;
    let rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="s" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>"#;
    let bytes = zip(&[
        ("[Content_Types].xml", types.as_bytes()),
        ("_rels/.rels", root.as_bytes()),
        ("xl/workbook.xml", br#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"/>"#),
        ("xl/_rels/workbook.xml.rels", rels.as_bytes()),
        ("xl/styles.xml", xml.as_bytes()),
    ]);
    styles_of(&bytes).0.unwrap()
}

/// What both readers lift out: an `<xf>`'s element keeps only its `apply*` flags, and the XML
/// part's own element is dropped.
fn lifted(mut s: Styles) -> Styles {
    for xf in s.cell_style_xfs.iter_mut().chain(&mut s.cell_xfs) {
        let attrs = xf.el.attrs.iter().filter(|(k, _)| k.starts_with("apply")).cloned().collect();
        xf.el = craft_xml::El::new("xf");
        xf.el.attrs = attrs;
    }
    s.root = craft_xml::El::default();
    s
}

#[test]
fn styles_are_read_as_their_xlsx_spelling() {
    let (binary, warnings) = styles_of(&styled_file(&styles_bin(), STYLES_TYPES));
    let binary = binary.expect("styles are read");
    assert_eq!(lifted(binary.clone()), lifted(xml_styles(STYLES_XML)));
    // A few of the conversions, spelled out.
    assert_eq!(binary.fonts[1].size, Some(14.5));
    assert_eq!(binary.fonts[1].family, None);
    let solid = Fill::Pattern(PatternFill {
        pattern: Some(Pattern::Solid),
        fg: Some(Color { source: ColorSource::Rgb(0xFFFF_FF00), tint: 0.0 }),
        bg: Some(Color { source: ColorSource::Indexed(64), tint: 0.0 }),
    });
    assert_eq!(binary.fills[2], solid);
    assert_eq!(binary.cell_xfs[2].protection, Some(Protection { locked: Some(false), hidden: Some(true) }));
    assert_eq!(binary.palette()[0], 0x11_2233);
    // Written back as SpreadsheetML, a cell style format says what it leaves out, as Excel's does.
    let written = String::from_utf8(binary.to_xml()).unwrap();
    assert!(written.contains(r#"applyFont="0" applyFill="0" applyBorder="0" applyAlignment="0" applyProtection="0""#), "{written}");
    assert!(warnings.iter().any(|w| w.contains("alternate-content")), "{warnings:?}");
    assert!(!warnings.iter().any(|w| w.contains("table styles")), "{warnings:?}");
}

#[test]
fn damaged_styles_are_a_warning_and_the_workbook_opens() {
    let good = styles_bin();
    let end = record(279, &[]);
    let body = &good[..good.len() - end.len()];
    let with = |extra: &[u8]| [body, extra, &end].concat();
    let cases: Vec<(&str, Vec<u8>, &str)> = vec![
        ("truncated", good[..good.len() - 1].to_vec(), STYLES_TYPES),
        ("no end record", body.to_vec(), STYLES_TYPES),
        ("unknown fill pattern", with(&pattern_fill(99, &indexed(64), &indexed(65))), STYLES_TYPES),
        ("unknown border style", with(&record(46, &[&[0][..], &edge(14, &indexed(64)), &[0; 40]].concat())), STYLES_TYPES),
        ("cell format outside its list", with(&brt_xf(0, 0, 0, 0, 0, 0, 0, 0x1010, 0)), STYLES_TYPES),
        ("short property", with(&brt_dxf(&[vec![0x19, 0, 2, 0]])), STYLES_TYPES),
        ("wrong content type", good.clone(), TYPES),
    ];
    for (why, part, types) in cases {
        let bytes = styled_file(&part, types);
        let (styles, warnings) = styles_of(&bytes);
        assert!(styles.is_none(), "{why}");
        assert!(warnings.iter().any(|w| w.contains("look/styles.bin")), "{why}: {warnings:?}");
        assert_eq!(read(&bytes).unwrap().sheets[0].value(CellRef::new(0, 0)), Value::Number(1.0), "{why}");
    }
    // A table past 65,536 entries is past a fixed cap: refused before it is held, as too large.
    let xfs = brt_xf(0, 0, 0, 0, 0, 0, 0, 0x1010, 0).repeat(65_537);
    let bytes = styled_file(&with(&[&begin(617, 65_537)[..], &xfs, &record(618, &[])].concat()), STYLES_TYPES);
    let mut p = Package::open(&bytes, &Limits::default()).unwrap();
    assert!(matches!(p.styles(), Err(Error::TooLarge(m)) if m.contains("65536 entries")));
    // Custom table styles are not read, and say so.
    let table_styles = [body, &record(508, &[&1u32.to_le_bytes()[..], &wide("Probe Table"), &wide("")].concat()), &end].concat();
    let (styles, warnings) = styles_of(&styled_file(&table_styles, STYLES_TYPES));
    assert!(styles.is_some());
    assert!(warnings.iter().any(|w| w.contains("table styles")), "{warnings:?}");
}

/// `PtgAttrSpace` ([MS-XLSB] 2.5.98.31): its type (0 spaces before the next token, 1 line breaks
/// before it, 2 spaces before a `(`, 4 spaces before a `)`) and count.
fn space(kind: u8, n: u8) -> Vec<u8> {
    vec![0x19, 0x40, kind, n]
}

#[test]
fn typed_spacing_numbers_and_iso_functions_read_as_in_xlsx() {
    let a1 = || [&[0x24u8][..], &loc(0, REL)].concat();
    let b1 = || [&[0x24u8][..], &loc(0, 1 | REL)].concat();
    let num = |n: f64| [&[0x1Fu8][..], &n.to_le_bytes()].concat();
    let area = |c: u16| [&[0x25u8][..], &0u32.to_le_bytes(), &1u32.to_le_bytes(), &(c | REL).to_le_bytes(), &(c | REL).to_le_bytes()].concat();
    let goto = [0x19u8, 0x08, 0, 0];
    let cells: Vec<Vec<u8>> = vec![
        // Spacing goes before the next token: an operand, or an operator's symbol.
        [&[0x1Eu8, 1, 0][..], &space(0, 2), &[0x1E, 2, 0], &space(0, 1), &[0x03]].concat(),
        // ... or a call's `)`, and inside a call's parentheses.
        [space(0, 1), a1(), space(0, 1), b1(), space(4, 1), vec![0x42, 2, 4, 0]].concat(),
        // Line breaks.
        [
            a1(),
            vec![0x1E, 0, 0, 0x0D, 0x19, 0x02, 0, 0],
            space(1, 1),
            space(0, 2),
            vec![0x1E, 1, 0],
            goto.to_vec(),
            space(1, 1),
            space(0, 2),
            vec![0x1E, 2, 0],
            goto.to_vec(),
            vec![0x42, 3, 1, 0],
        ]
        .concat(),
        // Inside and before parentheses.
        [space(0, 1), vec![0x1E, 1, 0], space(0, 1), vec![0x1E, 2, 0], space(0, 1), vec![0x03], space(4, 1), vec![0x15]].concat(),
        [vec![0x1E, 1, 0, 0x1E, 2, 0], space(2, 1), vec![0x15, 0x03]].concat(),
        // A call with no arguments, and one through PtgAttrSum.
        [space(4, 1), vec![0x41, 19, 0]].concat(),
        [area(0), area(1), space(0, 1), vec![0x0F, 0x19, 0x10, 0, 0]].concat(),
        // A sign, `%`, and leading spacing (trimmed, as `.xlsx` text is read).
        [space(0, 1), vec![0x1E, 1, 0, 0x13], space(0, 1), vec![0x14]].concat(),
        [space(0, 3), a1()].concat(),
        // A newer function: spacing before its name's token is the first argument's.
        [vec![0x1E, 1, 0], space(0, 1), vec![0x23, 5, 0, 0, 0], num(1.5), space(0, 1), vec![0x42, 2, 0xFF, 0, 0x03]].concat(),
        vec![0x23, 1, 0, 0, 0, 0x17, 1, 0, b'a', 0, 0x42, 2, 0xFF, 0],
        // Numbers written out up to 21 characters, scientific past them.
        [num(1e20), num(1e21), vec![0x03], num(1e-19), vec![0x03], num(1.5e-19), vec![0x03], num(1234567890123456.0), vec![0x03]].concat(),
    ];
    let mut data = row(0);
    for (i, rgce) in cells.iter().enumerate() {
        data.extend(formula_cell(i as u32 + 1, rgce, &[]));
    }
    // Name 5: the hidden name ISO.CEILING is called through, as Excel writes it.
    let mut book = book_with_links(&xti());
    let end = record(132, &[]);
    book.truncate(book.len() - end.len());
    book.extend(name(0x0002_000b, "_xlfn.ISO.CEILING"));
    book.extend(end);
    let wb = read(&package(&book, &worksheet(&data, &[]), &sst(), ROOT, RELS, TYPES)).unwrap();
    let got: Vec<_> = (1..=cells.len() as u32).map(|c| wb.sheets[0].formula(CellRef::new(0, c))).collect();
    let want = [
        "1 +  2",
        "SUM( A1, B1 )",
        "IF(A1>0,\n  1,\n  2)",
        "( 1 + 2 )",
        "1+ (2)",
        "PI( )",
        "SUM(A1:A2  B1:B2)",
        "- 1 %",
        "A1",
        "1+ ISO.CEILING( 1.5)",
        "_xlfn.CONCAT(\"a\")",
        "100000000000000000000+1E+21+0.0000000000000000001+1.5E-19+1234567890123460",
    ];
    assert_eq!(got, want.map(Some));
}

#[test]
fn references_to_macro_sheets_and_spans_over_chart_sheets_are_named() {
    // Sheets Data, a chart sheet, an XLM macro sheet, Hidden; sheet links Macro1, Data:Hidden
    // (over the chart sheet) and Chart1 (no cells).
    let mut book = record(131, &[]);
    book.extend(record(153, &[[0u8; 8].as_slice(), &wide("")].concat()));
    book.extend(record(143, &[]));
    for (id, rel, name) in [(1u32, "one", "Data"), (3, "c", "Chart1"), (4, "m", "Macro1"), (2, "two", "Hidden")] {
        book.extend(record(156, &[&0u32.to_le_bytes()[..], &id.to_le_bytes(), &wide(rel), &wide(name)].concat()));
    }
    book.extend(record(144, &[]));
    let mut xti = 3u32.to_le_bytes().to_vec();
    for (first, last) in [(2i32, 2i32), (0, 3), (1, 1)] {
        xti.extend([0u32.to_le_bytes(), first.to_le_bytes(), last.to_le_bytes()].concat());
    }
    book.extend([record(353, &[]), record(357, &[]), record(362, &xti), record(354, &[]), record(132, &[])].concat());
    let rels = RELS.replace(
        "</Relationships>",
        r#"<Relationship Id="c" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/chartsheet" Target="../data/c.bin"/><Relationship Id="m" Type="http://schemas.microsoft.com/office/2006/relationships/xlMacrosheet" Target="../data/m.bin"/></Relationships>"#,
    );
    let ref3d = |ixti: u16| [&[0x5Au8][..], &ixti.to_le_bytes(), &loc(0, 0)].concat();
    let data = [row(0), formula_cell(1, &ref3d(0), &[]), formula_cell(2, &ref3d(1), &[]), formula_cell(3, &ref3d(2), &[])].concat();
    let wb = read(&package(&book, &worksheet(&data, &[]), &sst(), ROOT, &rels, TYPES)).unwrap();
    let got: Vec<_> = (1..=3).map(|c| wb.sheets[0].formula(CellRef::new(0, c))).collect();
    assert_eq!(got, [Some("Macro1!$A$1"), Some("Data:Hidden!$A$1"), None]);
    assert!(warned(&wb, "references to sheets with no cells"), "{:?}", wb.warnings);
}

#[test]
fn vba_function_names_without_macro_strings_are_read() {
    // Excel writes a VBA function a cell calls as a BrtName with fFunc, fOB and
    // fProc, no formula, and none of the four strings fProc names otherwise carry. `.xlsm` lists
    // no such name; a cell calling the function spells it.
    let vba = |text: &str| {
        let data = [
            &0x0Eu32.to_le_bytes()[..],
            &[0],
            &u32::MAX.to_le_bytes(),
            &wide(text),
            &0u32.to_le_bytes(),
            &0u32.to_le_bytes(),
            &u32::MAX.to_le_bytes(),
        ]
        .concat();
        record(39, &data)
    };
    let mut list = names();
    list.push(vba("Twice"));
    let twice = [&[0x23u8, 14, 0, 0, 0][..], &[0x1E, 4, 0], &[0x42, 2, 0xFF, 0]].concat();
    let data = [row(0), formula_cell(1, &twice, &[]), formula_cell(2, &[0x23, 9, 0, 0, 0], &[])].concat();
    let bytes = package(&book_with_names(&list), &worksheet(&data, &[]), &sst(), ROOT, RELS, TYPES);
    let wb = Package::open(&bytes, &Limits::default()).unwrap().workbook().unwrap();
    assert_eq!(wb.defined_names, xml_names(NAMES_XML));
    let book = read(&bytes).unwrap();
    assert_eq!([1, 2].map(|c| book.sheets[0].formula(CellRef::new(0, c))), [Some("Twice(4)"), Some("Rate")]);
}

#[test]
fn a_macro_names_description_and_help_topic_are_read_as_excel_writes_them() {
    // An XLM function in category 3 with the strings Excel writes after its comment: unused, the
    // description, the help topic, unused. [MS-XLSB] labels the middle two the custom menu and the
    // description; Excel's `.xlsm` has them as `description` and `help`.
    let flags: u32 = 0x02 | 0x08 | (3 << 6);
    let mut data = [&flags.to_le_bytes()[..], &[0], &u32::MAX.to_le_bytes(), &wide("FnA")].concat();
    let rgce = ref3d_name(0, 0, 0);
    data.extend([&(rgce.len() as u32).to_le_bytes()[..], &rgce, &0u32.to_le_bytes(), &u32::MAX.to_le_bytes()].concat());
    data.extend([u32::MAX.to_le_bytes().to_vec(), wide("Function A"), wide("!0"), u32::MAX.to_le_bytes().to_vec()].concat());
    let bytes = package(&book_with_names(&[record(39, &data)]), &worksheet(&row(0), &[]), &sst(), ROOT, RELS, TYPES);
    let wb = Package::open(&bytes, &Limits::default()).unwrap().workbook().unwrap();
    let xml = r#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><definedNames><definedName name="FnA" description="Function A" help="!0" function="1" xlm="1" functionGroupId="3">Data!$A$1</definedName></definedNames></workbook>"#;
    assert_eq!(wb.defined_names, xml_names(xml));
}

#[test]
fn a_formulas_array_constants_share_one_allowance() {
    // Two arrays of 40,000 Boolean values, each within what one array may hold, added together:
    // charged before their values are held, together they pass what a formula's arrays may hold
    // (65,536 values), so the cell keeps its saved result. One of them alone decodes.
    let array = |n: u32| [&n.to_le_bytes()[..], &1u32.to_le_bytes(), &[2u8, 1].repeat(n as usize)].concat();
    let ptg = [&[0x60u8][..], &[0; 14]].concat();
    let two = [&ptg[..], &ptg, &[0x03]].concat();
    let data = [row(0), formula_cell(0, &two, &[array(40_000), array(40_000)].concat()), formula_cell(1, &ptg, &array(40_000))].concat();
    let wb = read(&package(&book_with_links(&xti()), &worksheet(&data, &[]), &sst(), ROOT, RELS, TYPES)).unwrap();
    assert_eq!(wb.sheets[0].formula(CellRef::new(0, 0)), None);
    assert_eq!(wb.sheets[0].value(CellRef::new(0, 0)), Value::Number(0.0));
    assert!(wb.sheets[0].formula(CellRef::new(0, 1)).is_some_and(|f| f.starts_with("{TRUE;TRUE")));
    assert!(warned(&wb, "array constants too large"), "{:?}", wb.warnings);
}

#[test]
fn a_spent_formula_allowance_skips_decoding() {
    // With no allowance, a formula is not decoded at all: an unknown token is never reached.
    let data = [row(0), formula_cell(0, &[0xFF], &[])].concat();
    let limits = Limits { binary_formula_text: 0, ..Limits::default() };
    let wb = read_with(&package(&book_with_links(&xti()), &worksheet(&data, &[]), &sst(), ROOT, RELS, TYPES), &limits).unwrap();
    assert_eq!(wb.sheets[0].formula(CellRef::new(0, 0)), None);
    assert!(warned(&wb, "more formula text than the limits allow"), "{:?}", wb.warnings);
    assert!(!warned(&wb, "unknown formula token"), "{:?}", wb.warnings);
}

#[test]
fn a_name_using_a_left_out_name_is_left_out_too() {
    // 14 Early = Later (a later name), 15 Alias = External (left out: another workbook),
    // 16 Later = Alias, 17 Fine = Rate. Early, Alias and Later are left out whatever their order,
    // and cells using them keep their saved results; Fine is read.
    let mut list = names();
    for (text, index) in [("Early", 16u32), ("Alias", 3), ("Later", 15), ("Fine", 9)] {
        list.push(brt_name(0, 0, WORKBOOK, text, &[&[0x23u8][..], &index.to_le_bytes()].concat(), None));
    }
    let cell = |i: u32| [&[0x23u8][..], &i.to_le_bytes()].concat();
    let data =
        [row(0), formula_cell(1, &cell(14), &[]), formula_cell(2, &cell(15), &[]), formula_cell(3, &cell(16), &[]), formula_cell(4, &cell(17), &[])]
            .concat();
    let bytes = package(&book_with_names(&list), &worksheet(&data, &[]), &sst(), ROOT, RELS, TYPES);
    let wb = Package::open(&bytes, &Limits::default()).unwrap().workbook().unwrap();
    let xml = NAMES_XML.replace("</definedNames>", r#"<definedName name="Fine">Rate</definedName></definedNames>"#);
    assert_eq!(wb.defined_names, xml_names(&xml));
    let book = read(&bytes).unwrap();
    assert_eq!((1..=4).map(|c| book.sheets[0].formula(CellRef::new(0, c))).collect::<Vec<_>>(), [None, None, None, Some("Fine")]);
    assert!(book.warnings.iter().any(|w| w.contains("names using a name that was left out")), "{:?}", book.warnings);
}

#[test]
fn differential_format_strings_are_charged_to_the_text_allowance() {
    // A DXF's font name and number format are text the styles part keeps, charged like a font's.
    let dxf = |name: &str| [record(278, &[]), begin(505, 1), brt_dxf(&[prop(0x18, &short(name))]), record(506, &[]), record(279, &[])].concat();
    let limits = Limits { binary_text: 100, ..Limits::default() };
    let open = |part: &[u8]| Package::open(&styled_file(part, STYLES_TYPES), &limits).unwrap().styles();
    assert!(open(&dxf("Arial")).is_ok_and(|s| s.is_some_and(|s| s.dxfs.len() == 1)));
    assert!(matches!(open(&dxf(&"x".repeat(200))), Err(Error::TooLarge(_))));
}

#[test]
fn module_sheets_and_supporting_links_are_bounded() {
    // 1,025 very hidden module sheets (no relationship) before the worksheets: each keeps a place
    // in the sheet links, so together they are past the 1,024-sheet cap.
    let plain = book(false);
    let at = plain.windows(3).position(|w| w == record(143, &[]).as_slice()).unwrap() + 3;
    let module = record(156, &[&2u32.to_le_bytes()[..], &9u32.to_le_bytes(), &u32::MAX.to_le_bytes(), &wide("Module")].concat());
    let modules = [&plain[..at], &module.repeat(1025), &plain[at..]].concat();
    assert!(matches!(read(&package(&modules, &worksheet(&[], &[]), &sst(), ROOT, RELS, TYPES)), Err(Error::TooLarge(_))));
    // 65,537 supporting links: past the cap the links aren't read, and formulas keep their values.
    let end = record(132, &[]);
    let selves = [&plain[..plain.len() - end.len()], &record(353, &[]), &record(357, &[]).repeat(65_537), &record(354, &[]), &end].concat();
    let data = [row(0), formula_cell(0, &[0x1E, 1, 0], &[])].concat();
    let wb = read(&package(&selves, &worksheet(&data, &[]), &sst(), ROOT, RELS, TYPES)).unwrap();
    assert_eq!(wb.sheets[0].formula(CellRef::new(0, 0)), None);
    assert!(wb.warnings.iter().any(|w| w.contains("sheet links or names could not be read")), "{:?}", wb.warnings);
}

#[test]
fn empty_ordinary_names_and_their_dependants_keep_cached_values() {
    // Forward and transitive references to an empty ordinary name must not become dangling
    // formulas. An empty sheet-scoped Database or another built-in is not the observed
    // workbook-level Database exception either.
    let mut list = names();
    let cell = |i: u32| [&[0x23u8][..], &i.to_le_bytes()].concat();
    list.push(brt_name(0, 0, WORKBOOK, "Early", &cell(16), None)); // 14
    list.push(brt_name(0, 0, WORKBOOK, "Empty", &[], None)); // 15
    list.push(brt_name(0, 0, WORKBOOK, "Alias", &cell(15), None)); // 16
    list.push(brt_name(0x20, 0, 0, "Database", &[], None)); // 17
    list.push(brt_name(0x20, 0, WORKBOOK, "Print_Area", &[], None)); // 18
    list.push(brt_name(0, 0, WORKBOOK, "Fine", &cell(9), None)); // 19
    let mut data = row(0);
    for i in 14..=19 {
        data.extend(formula_cell(i - 13, &cell(i), &[]));
    }
    let bytes = package(&book_with_names(&list), &worksheet(&data, &[]), &sst(), ROOT, RELS, TYPES);
    let wb = Package::open(&bytes, &Limits::default()).unwrap().workbook().unwrap();
    let xml = NAMES_XML.replace("</definedNames>", r#"<definedName name="Fine">Rate</definedName></definedNames>"#);
    assert_eq!(wb.defined_names, xml_names(&xml));
    let book = read(&bytes).unwrap();
    for col in 1..=5 {
        assert_eq!(book.sheets[0].formula(CellRef::new(0, col)), None);
        assert_eq!(book.sheets[0].value(CellRef::new(0, col)), Value::Number(0.0));
    }
    assert_eq!(book.sheets[0].formula(CellRef::new(0, 6)), Some("Fine"));
    assert!(book.warnings.iter().any(|w| w.contains("empty formula")), "{:?}", book.warnings);
    assert!(book.warnings.iter().any(|w| w.contains("names using a name that was left out")), "{:?}", book.warnings);
}

#[test]
fn scoped_names_do_not_rebind_shadowed_workbook_names() {
    let token = |i: u32| [&[0x23u8][..], &i.to_le_bytes()].concat();
    let list = vec![
        brt_name(0, 0, WORKBOOK, "Rate", &[0x1E, 1, 0], None),         // 1
        brt_name(0, 0, 1, "rAtE", &[0x1E, 2, 0], None),                // 2: Hidden shadows Rate case-insensitively.
        brt_name(0, 0, 1, "Alias", &token(1), None),                   // 3: must not become Hidden!rAtE.
        brt_name(0, 0, 1, "Forward", &token(5), None),                 // 4
        brt_name(0, 0, 1, "Dependent", &token(3), None),               // 5
        brt_name(0, 0, 0, "Fine", &token(1), None),                    // 6: no shadow on Data.
        brt_name(0, 0, 1, "LocalAlias", &token(2), None),              // 7: intentionally uses Hidden!rAtE.
        brt_name(0, 0, WORKBOOK, "Other", &[0x1E, 3, 0], None),        // 8
        brt_name(0, 0, 1, "Other", &[], None),                         // 9: malformed, but still an ordinary source shadow.
        brt_name(0, 0, 1, "Conservative", &token(8), None),            // 10
        brt_name(0, 0, WORKBOOK, "Synthetic", &[0x1E, 4, 0], None),    // 11
        brt_name(0x0002_0000, 0, 1, "Synthetic", &[0x1C, 0x1D], None), // 12: future-function metadata.
        brt_name(0, 0, 1, "SyntheticControl", &token(11), None),       // 13: synthetic name is not a local definition.
    ];
    let mut data = row(0);
    for (col, index) in [3u32, 4, 5, 6, 10, 13].into_iter().enumerate() {
        // PtgNameX qualifies these names on their own sheets.
        let formula = [&[0x39u8, 0, 0][..], &index.to_le_bytes()].concat();
        data.extend(formula_cell(col as u32 + 1, &formula, &[]));
    }
    let bytes = package(&book_with_names(&list), &worksheet(&data, &[]), &sst(), ROOT, RELS, TYPES);
    let wb = Package::open(&bytes, &Limits::default()).unwrap().workbook().unwrap();
    let xml = r#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><definedNames><definedName name="Rate">1</definedName><definedName name="rAtE" localSheetId="1">2</definedName><definedName name="Fine" localSheetId="0">Rate</definedName><definedName name="LocalAlias" localSheetId="1">rAtE</definedName><definedName name="Other">3</definedName><definedName name="Synthetic">4</definedName><definedName name="SyntheticControl" localSheetId="1">Synthetic</definedName></definedNames></workbook>"#;
    assert_eq!(wb.defined_names, xml_names(xml));
    let book = read(&bytes).unwrap();
    for col in 1..=3 {
        assert_eq!(book.sheets[0].formula(CellRef::new(0, col)), None);
        assert_eq!(book.sheets[0].value(CellRef::new(0, col)), Value::Number(0.0));
    }
    assert_eq!(book.sheets[0].formula(CellRef::new(0, 4)), Some("Data!Fine"));
    assert_eq!(book.sheets[0].formula(CellRef::new(0, 5)), None);
    assert_eq!(book.sheets[0].value(CellRef::new(0, 5)), Value::Number(0.0));
    assert_eq!(book.sheets[0].formula(CellRef::new(0, 6)), Some("Hidden!SyntheticControl"));
    assert!(book.warnings.iter().any(|w| w.contains("workbook names shadowed by local names")), "{:?}", book.warnings);
    assert!(book.warnings.iter().any(|w| w.contains("names using a name that was left out")), "{:?}", book.warnings);
}
