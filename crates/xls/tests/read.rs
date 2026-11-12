//! Reading BIFF8 workbooks: values, formulas, layout, names, limits and hostile input.

mod common;

use common::{
    at, bof, cat, cell, compound, eof, exp, fmt, formula, ftext, num, ptg_int, ptg_ref, put32, read_limited, read_xls, rec, special, text, u16s, v,
    xlstr,
};
use craft_xls::{CalcMode, CellError, CellRef, Column, Error, Limits, Range, Row, Value, View, Workbook, is_xls};

fn range(a1: &str) -> Range {
    let (a, b) = a1.split_once(':').unwrap_or((a1, a1));
    Range::new(at(a), at(b))
}

fn text_limit(max_text: usize) -> Limits {
    Limits { max_text, ..Limits::default() }
}

/// A workbook with two worksheets and a chart sheet covering the stage-1 features.
fn sample(date1904: bool) -> Vec<u8> {
    // SST: "Hello", then "Wörld" as UTF-16 split across a Continue record.
    let mut sst = cat(&[&2u32.to_le_bytes(), &2u32.to_le_bytes(), &xlstr("Hello")]);
    sst.extend_from_slice(&[5, 0, 0, b'W']);
    let sst_rec = cat(&[&rec(0x00FC, &sst), &rec(0x003C, &cat(&[&[1], &u16s(&[0xF6, 'r' as u16, 'l' as u16, 'd' as u16])]))]);
    let xf = |ifmt: u16| rec(0x00E0, &cat(&[&u16s(&[0, ifmt, 0]), &[0; 14]]));
    // Lbl "Rate" = Data!$A$1 (PtgRef3d, absolute).
    let rate = cat(&[&[0x3A], &1u16.to_le_bytes(), &u16s(&[0, 0])]);
    let lbl = rec(0x0018, &cat(&[&u16s(&[0]), &[0, 4], &u16s(&[rate.len() as u16, 0, 0]), &[0; 4], &[0], b"Rate", &rate]));
    // Print_Area on Sheet1 (built-in name 6, itab 1) = Sheet1!$A$1:$B$2.
    let pa = cat(&[&[0x3B], &0u16.to_le_bytes(), &u16s(&[0, 1, 0, 1])]);
    let print_area = rec(0x0018, &cat(&[&u16s(&[0x0020]), &[0, 1], &u16s(&[pa.len() as u16, 0, 1]), &[0; 4], &[0, 6], &pa]));
    let boundsheet = |pos: u32, kind: u8, name: &str| rec(0x0085, &cat(&[&pos.to_le_bytes(), &[0, kind, name.len() as u8, 0], name.as_bytes()]));
    let globals = |positions: [u32; 3]| {
        cat(&[
            &bof(0x0005),
            &rec(0x0022, &u16s(&[u16::from(date1904)])),
            &rec(0x041E, &cat(&[&164u16.to_le_bytes(), &xlstr("yyyy-mm-dd")])),
            &xf(0),
            &xf(164),
            &xf(10), // built-in 0.00%
            &boundsheet(positions[0], 0, "Sheet1"),
            &boundsheet(positions[1], 0, "Data"),
            &boundsheet(positions[2], 2, "Chart1"),
            &rec(0x01AE, &u16s(&[3, 0x0401])),
            &rec(0x0017, &cat(&[&u16s(&[2]), &u16s(&[0, 0, 0]), &u16s(&[0, 1, 1])])),
            &lbl,
            &print_area,
            &sst_rec,
            &eof(),
        ])
    };
    let mulrk = cat(&[&u16s(&[1, 2]), &u16s(&[0]), &((7u32 << 2) | 2).to_le_bytes(), &u16s(&[2]), &((150u32 << 2) | 3).to_le_bytes(), &u16s(&[3])]);
    let shared = cat(&[&[0x4C], &u16s(&[0, 0xFFFF]), &ptg_int(10), &[0x05]]);
    let array = cat(&[&[0x65], &u16s(&[4, 5, 0xC000, 0xC000]), &ptg_int(2), &[0x05]]);
    let sum = cat(&[&[0x25], &u16s(&[0, 0, 1 | 0xC000, 2 | 0xC000]), &[0x22, 1], &4u16.to_le_bytes()]);
    let concat = cat(&[&ptg_ref(0, 0), &[0x17, 1, 0, b'!'], &[0x08]]);
    let round = cat(&[&ptg_ref(0, 1), &ptg_int(0), &[0x41], &27u16.to_le_bytes()]);
    let name_ref = cat(&[&[0x43], &1u32.to_le_bytes(), &ptg_int(2), &[0x05]]);
    let ref3d = cat(&[&[0x5A], &1u16.to_le_bytes(), &u16s(&[0, 0xC000])]);
    let sheet1 = cat(&[
        &bof(0x0010),
        &rec(0x0225, &u16s(&[0, 255])),
        &rec(0x007D, &u16s(&[0, 0, 20 * 256, 0, 0x0002, 0])),
        &rec(0x0208, &cat(&[&u16s(&[0, 0, 4, 600, 0, 0]), &(0x0140u32).to_le_bytes()])),
        &rec(0x00FD, &cat(&[&cell(0, 0, 0), &0u32.to_le_bytes()])),
        &rec(0x0203, &cat(&[&cell(0, 1, 0), &1.5f64.to_le_bytes()])),
        &rec(0x027E, &cat(&[&cell(0, 2, 0), &((42u32 << 2) | 2).to_le_bytes()])),
        &rec(0x00FD, &cat(&[&cell(0, 3, 0), &1u32.to_le_bytes()])),
        &rec(0x0205, &cat(&[&cell(1, 0, 0), &[1, 0]])),
        &rec(0x0205, &cat(&[&cell(1, 1, 0), &[0x07, 1]])),
        &rec(0x00BD, &mulrk),
        &formula(2, 0, num(3.0), &cat(&[&ptg_ref(0, 1), &ptg_int(2), &[0x05]])),
        &formula(2, 1, num(43.5), &sum),
        &formula(2, 2, special(0, 0), &concat),
        &rec(0x0207, &xlstr("Hello!")),
        &formula(2, 3, special(1, 1), &cat(&[&ptg_ref(0, 1), &ptg_int(1), &[0x0D]])),
        &formula(2, 4, num(14.0), &name_ref),
        &formula(2, 5, num(2.0), &round),
        &formula(2, 6, num(7.0), &ref3d),
        &formula(2, 7, special(2, 0x2A), &[0x1C, 0x2A]),
        &rec(0x0203, &cat(&[&cell(3, 0, 1), &0.0f64.to_le_bytes()])),
        &rec(0x0203, &cat(&[&cell(3, 1, 2), &0.25f64.to_le_bytes()])),
        &rec(0x027E, &cat(&[&cell(4, 0, 0), &((1u32 << 2) | 2).to_le_bytes()])),
        &rec(0x027E, &cat(&[&cell(5, 0, 0), &((2u32 << 2) | 2).to_le_bytes()])),
        &formula(4, 1, num(10.0), &exp(4, 1)),
        &rec(0x04BC, &cat(&[&u16s(&[4, 5]), &[1, 1, 0, 2], &(shared.len() as u16).to_le_bytes(), &shared])),
        &formula(5, 1, num(20.0), &exp(4, 1)),
        &formula(4, 2, num(2.0), &exp(4, 2)),
        &rec(0x0221, &cat(&[&u16s(&[4, 5]), &[2, 2], &[0; 6], &(array.len() as u16).to_le_bytes(), &array])),
        &formula(5, 2, num(4.0), &exp(4, 2)),
        &rec(0x00E5, &cat(&[&u16s(&[1]), &u16s(&[6, 6, 0, 1])])),
        // An embedded chart substream: its records are not cells.
        &bof(0x0020),
        &rec(0x0203, &cat(&[&cell(9, 9, 0), &9.0f64.to_le_bytes()])),
        &eof(),
        &eof(),
    ]);
    let data = cat(&[&bof(0x0010), &rec(0x0203, &cat(&[&cell(0, 0, 0), &7.0f64.to_le_bytes()])), &eof()]);
    let chart = cat(&[&bof(0x0020), &eof()]);
    let len = globals([0; 3]).len() as u32;
    let p1 = len;
    let p2 = p1 + sheet1.len() as u32;
    let p3 = p2 + data.len() as u32;
    cat(&[&globals([p1, p2, p3]), &sheet1, &data, &chart])
}

#[test]
fn reads_values_formulas_formats_and_layout() {
    for big in [false, true] {
        let mut stream = sample(false);
        if big {
            // Past the mini stream cutoff: the stream lives in regular sectors.
            stream.extend(std::iter::repeat_n(0u8, 5000));
        }
        let bytes = compound("Workbook", &stream);
        assert!(is_xls(&bytes));
        let (wb, report) = read_xls(&bytes).unwrap();
        assert!(report.warnings.iter().any(|w| w.contains("chart sheets")));
        assert_eq!(wb.sheets.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["Sheet1", "Data"]);
        assert_values_roundtrip(&wb);
        assert_formula_roundtrip(&wb);
        assert_layout_roundtrip(&wb);
    }
    let (wb, _) = read_xls(&compound("Workbook", &sample(true))).unwrap();
    assert!(wb.date_1904);
}

fn assert_values_roundtrip(wb: &Workbook) {
    assert_eq!(v(wb, 0, "A1"), text("Hello"));
    assert_eq!(v(wb, 0, "B1"), Value::Number(1.5));
    assert_eq!(v(wb, 0, "C1"), Value::Number(42.0));
    assert_eq!(v(wb, 0, "D1"), text("Wörld"));
    assert_eq!(v(wb, 0, "A2"), Value::Bool(true));
    assert_eq!(v(wb, 0, "B2"), Value::Error(CellError::Div0));
    assert_eq!(v(wb, 0, "C2"), Value::Number(7.0));
    assert_eq!(v(wb, 0, "D2"), Value::Number(1.5));
}

fn assert_formula_roundtrip(wb: &Workbook) {
    // Formulas: text plus the cached result.
    assert_eq!(ftext(wb, "A3").as_deref(), Some("B1*2"));
    assert_eq!(v(wb, 0, "A3"), Value::Number(3.0));
    assert_eq!(ftext(wb, "B3").as_deref(), Some("SUM(B1:C1)"));
    assert_eq!(ftext(wb, "C3").as_deref(), Some("A1&\"!\""));
    assert_eq!(v(wb, 0, "C3"), text("Hello!"));
    assert_eq!(ftext(wb, "D3").as_deref(), Some("B1>1"));
    assert_eq!(v(wb, 0, "D3"), Value::Bool(true));
    assert_eq!(ftext(wb, "E3").as_deref(), Some("Rate*2"));
    assert_eq!(ftext(wb, "F3").as_deref(), Some("ROUND(B1,0)"));
    assert_eq!(ftext(wb, "G3").as_deref(), Some("Data!A1"));
    assert_eq!(v(wb, 0, "H3"), Value::Error(CellError::NA));
    // Shared formula: relative offsets resolve per cell.
    assert_eq!(ftext(wb, "B5").as_deref(), Some("A5*10"));
    assert_eq!(ftext(wb, "B6").as_deref(), Some("A6*10"));
    // Array formula: the anchor holds it; the other cell's cached result is dropped so that
    // the formula fills the range on recalculation .
    let c5 = wb.sheets[0].cells[&at("C5")].formula.clone().unwrap();
    assert_eq!((c5.text.as_str(), c5.array), ("A5:A6*2", Some(range("C5:C6"))));
    // The follower keeps its (unlocked, so not plain) format on a blank cell.
    assert_eq!(wb.sheets[0].cells[&at("C6")], craft_xls::Cell { value: Value::Empty, format: Some(0), formula: None });
    assert_eq!(ftext(wb, "C6"), None);
    assert_eq!(v(wb, 0, "C6"), Value::Empty);
}

fn assert_layout_roundtrip(wb: &Workbook) {
    // Number formats, merges, sizes, names, print area; embedded chart records ignored.
    let sh = &wb.sheets[0];
    assert_eq!(fmt(wb, "A4"), "yyyy-mm-dd");
    assert_eq!(fmt(wb, "B4"), "0.00%");
    assert_eq!(fmt(wb, "A1"), "General");
    assert_eq!(sh.merges, vec![range("A7:B7")]);
    // Column A: 20 characters wide, set by the user, in XF 0 (unlocked here, so not plain). Row 1: 600 twips (30 pt), set by the user.
    assert_eq!(sh.columns, [(0, Column { width: 20 * 256, custom_width: true, format: Some(0), ..Column::default() })].into());
    assert_eq!(sh.rows, [(0, Row { height: 600, custom_height: true, ..Row::default() })].into());
    assert_eq!(sh.default_row_height, Some(255));
    assert_eq!(sh.view, None);
    assert_eq!(sh.print_area.as_deref(), Some("Sheet1!$A$1:$B$2"));
    assert_eq!(sh.print_titles, None);
    assert_eq!(v(wb, 0, "J10"), Value::Empty);
    assert_eq!(wb.names.len(), 1);
    assert_eq!((wb.names[0].name.as_str(), wb.names[0].formula.as_str()), ("Rate", "Data!$A$1"));
    assert_eq!(v(wb, 1, "A1"), Value::Number(7.0));
    assert!(!wb.date_1904);
}

#[test]
fn text_result_of_a_formula_kept_as_its_value() {
    // A what-if data table cell ({=TABLE(B1,)}) can't be imported as a formula, so its saved result
    // is kept. With the default XF the cell is blank until the String record brings the text.
    let table = cat(&[&[0x02], &u16s(&[0, 0])]);
    let sheet = cat(&[&bof(0x0010), &formula(0, 0, special(0, 0), &table), &rec(0x0207, &xlstr("103a")), &eof()]);
    let boundsheet = |pos: u32| rec(0x0085, &cat(&[&pos.to_le_bytes(), &[0, 0, 6, 0], b"Sheet1"]));
    let len = cat(&[&bof(0x0005), &boundsheet(0), &eof()]).len() as u32;
    let stream = cat(&[&bof(0x0005), &boundsheet(len), &eof(), &sheet]);
    let (wb, report) = read_xls(&compound("Workbook", &stream)).unwrap();
    assert!(report.warnings.iter().any(|w| w.contains("what-if data tables")), "{:?}", report.warnings);
    assert_eq!(ftext(&wb, "A1"), None);
    assert_eq!(v(&wb, 0, "A1"), text("103a"));
}

#[test]
fn refuses_encrypted_old_and_foreign_files() {
    // FILEPASS in the globals: an encrypted workbook (encryption.rs reads them); this one is cut
    // short, so it is a damaged file.
    let mut stream = bof(0x0005);
    stream.extend(rec(0x002F, &[1, 0, 1, 0]));
    stream.extend(eof());
    assert!(matches!(read_xls(&compound("Workbook", &stream)), Err(Error::Invalid(_))));
    // A BIFF5 (Excel 5.0/95) workbook keeps its data in a `Book` stream.
    let book = compound("Book", &stream);
    assert!(is_xls(&book));
    assert_eq!(read_xls(&book).err(), Some(Error::Biff5));
    // So does one whose Workbook stream starts with a BIFF5 BOF.
    let mut old = rec(0x0809, &u16s(&[0x0500, 0x0005, 0, 0]));
    old.extend(eof());
    assert_eq!(read_xls(&compound("Workbook", &old)).err(), Some(Error::Biff5));
    // Another compound file (a word processor document): not a workbook.
    let doc = compound("WordDocument", &[0u8; 100]);
    assert!(!is_xls(&doc));
    assert!(matches!(read_xls(&doc), Err(Error::Invalid(_))));
    assert!(!is_xls(b"PK\x03\x04") && !is_xls(&[]));
}

#[test]
fn hostile_xls_input_never_panics_or_hangs() {
    let stream = sample(false);
    let good = compound("Workbook", &stream);
    // Truncation anywhere.
    for cut in (0..good.len()).step_by(37) {
        let _ = read_xls(&good[..cut]);
        let _ = is_xls(&good[..cut]);
    }
    // Byte mutations in the container and the stream.
    for at in (0..good.len()).step_by(7) {
        for x in [0x00u8, 0x01, 0x7F, 0xFF] {
            let mut b = good.clone();
            b[at] = x;
            let _ = read_xls(&b);
        }
    }
    for at in 0..stream.len() {
        let mut s = stream.clone();
        s[at] ^= 0xA5;
        let _ = read_xls(&compound("Workbook", &s));
    }
    // A FAT chain that loops on itself (big stream in regular sectors).
    let mut big = stream.clone();
    big.extend(std::iter::repeat_n(0u8, 5000));
    let mut b = compound("Workbook", &big);
    let fat_at = 512;
    let first_data = 2usize; // FAT, directory, then data
    put32(&mut b, fat_at + (first_data + 3) * 4, first_data as u32);
    assert!(read_xls(&b).is_err());
    // Huge declared counts: shared strings, merges, external sheets.
    let mut huge = bof(0x0005);
    huge.extend(rec(0x00FC, &cat(&[&u32::MAX.to_le_bytes(), &u32::MAX.to_le_bytes()])));
    huge.extend(eof());
    assert!(matches!(read_xls(&compound("Workbook", &huge)), Err(Error::TooLarge(_))));
    let mut huge = bof(0x0005);
    huge.extend(rec(0x00FC, &cat(&[&9u32.to_le_bytes(), &1_000_000u32.to_le_bytes(), &xlstr("x")])));
    huge.extend(rec(0x0017, &u16s(&[0xFFFF])));
    huge.extend(eof());
    assert!(read_xls(&compound("Workbook", &huge)).is_err()); // no worksheets
    // A sheet pointing outside the stream and a stream size larger than the file.
    let mut bad = bof(0x0005);
    bad.extend(rec(0x0085, &cat(&[&u32::MAX.to_le_bytes(), &[0, 0, 1, 0], b"S"])));
    bad.extend(eof());
    assert!(read_xls(&compound("Workbook", &bad)).is_err());
    let mut b = compound("Workbook", &big);
    put32(&mut b, 512 * 2 + 128 + 120, 0x3FFF_FFFF);
    assert!(read_xls(&b).is_err());
    // Random bytes behind a compound-file signature.
    let mut seed = 0x2545_F491_4F6C_DD1Du64;
    for _ in 0..200 {
        let mut b = good[..512].to_vec();
        for _ in 0..2048 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            b.push(seed as u8);
        }
        let _ = read_xls(&b);
    }
}

#[test]
fn dialog_sheets_are_skipped_not_read_as_worksheets() {
    let boundsheet = |pos: u32, name: &str| rec(0x0085, &cat(&[&pos.to_le_bytes(), &[0, 0, name.len() as u8, 0], name.as_bytes()]));
    let globals = |positions: [u32; 2]| cat(&[&bof(0x0005), &boundsheet(positions[0], "Dialog1"), &boundsheet(positions[1], "Sheet1"), &eof()]);
    let dialog = cat(&[&bof(0x0010), &rec(0x0081, &[0x10, 0x00]), &rec(0x0203, &cat(&[&cell(0, 0, 0), &1.0f64.to_le_bytes()])), &eof()]);
    let worksheet = cat(&[&bof(0x0010), &rec(0x0081, &[0x01, 0x04]), &rec(0x0203, &cat(&[&cell(0, 0, 0), &7.0f64.to_le_bytes()])), &eof()]);
    let first = globals([0; 2]).len() as u32;
    let stream = cat(&[&globals([first, first + dialog.len() as u32]), &dialog, &worksheet]);
    let (wb, report) = read_xls(&compound("Workbook", &stream)).unwrap();
    assert_eq!(wb.sheets.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["Sheet1"]);
    assert_eq!(v(&wb, 0, "A1"), Value::Number(7.0));
    assert!(report.warnings.iter().any(|w| w.contains("dialog sheets")), "{:?}", report.warnings);
}

#[test]
fn sheets_share_one_record_budget() {
    // Many BoundSheet records point into one long substream (at successive nested BOFs), so each
    // sheet would read the whole tail. The record allowance is shared, not per sheet.
    let n = 300u32;
    let tail: Vec<u8> = (0..2000u16).flat_map(|r| rec(0x0203, &cat(&[&cell(r, 0, 0), &1.0f64.to_le_bytes()]))).collect();
    let boundsheet = |pos: u32, name: &str| rec(0x0085, &cat(&[&pos.to_le_bytes(), &[0, 0, name.len() as u8, 0], name.as_bytes()]));
    let globals = |base: u32| {
        let mut g = bof(0x0005);
        for i in 0..n {
            g.extend(boundsheet(base + i * bof(0x0010).len() as u32, &format!("S{i}")));
        }
        g.extend(eof());
        g
    };
    let base = globals(0).len() as u32;
    let mut stream = globals(base);
    for _ in 0..n {
        stream.extend(bof(0x0010));
    }
    stream.extend(tail);
    stream.extend(eof());
    let file = compound("Workbook", &stream);
    // About n × 2,000 records in all: over a 100,000 shared allowance, though each sheet alone fits.
    assert!(matches!(read_limited(&file, Limits { max_records: 100_000, ..Limits::default() }), Err(Error::TooLarge(_))));
    assert!(read_limited(&file, Limits { max_records: 10_000_000, ..Limits::default() }).is_ok());

    // Two sheets at the same offset are refused outright.
    let mut dup = bof(0x0005);
    let at = (dup.len() + 2 * boundsheet(0, "A").len() + eof().len()) as u32;
    dup.extend(boundsheet(at, "A"));
    dup.extend(boundsheet(at, "B"));
    dup.extend(eof());
    dup.extend(bof(0x0010));
    dup.extend(eof());
    let err = read_xls(&compound("Workbook", &dup)).unwrap_err().to_string();
    assert!(err.contains("same substream"), "{err}");
}

#[test]
fn formulas_using_a_skipped_name_keep_their_saved_results() {
    // Lbl "ExtRange" = [Source.xlsx]Sheet1!$A$1:$A$3 (PtgArea3d through an external SupBook),
    // "Local" = Sheet1!$A$1 and "Twice" = ExtRange*2. Only the first is skipped on import; a
    // name that uses it is still read as it was.
    let ext = cat(&[&[0x3B], &1u16.to_le_bytes(), &u16s(&[0, 2, 0, 0])]);
    let local = cat(&[&[0x3A], &0u16.to_le_bytes(), &u16s(&[0, 0])]);
    let twice = cat(&[&[0x23], &1u32.to_le_bytes(), &ptg_int(2), &[0x05]]);
    let lbl = |name: &str, rgce: &[u8]| {
        rec(0x0018, &cat(&[&u16s(&[0]), &[0, name.len() as u8], &u16s(&[rgce.len() as u16, 0, 0]), &[0; 4], &[0], name.as_bytes(), rgce]))
    };
    let boundsheet = |pos: u32| rec(0x0085, &cat(&[&pos.to_le_bytes(), &[0, 0, 6, 0], b"Sheet1"]));
    let globals = |pos: u32| {
        cat(&[
            &bof(0x0005),
            &boundsheet(pos),
            &rec(0x01AE, &u16s(&[1, 0x0401])),
            &rec(0x01AE, &cat(&[&u16s(&[1]), &xlstr("Source.xlsx"), &xlstr("Sheet1")])),
            &rec(0x0017, &cat(&[&u16s(&[2]), &u16s(&[0, 0, 0]), &u16s(&[1, 0, 0])])),
            &lbl("ExtRange", &ext),
            &lbl("Local", &local),
            &lbl("Twice", &twice),
            &eof(),
        ])
    };
    let sum_ext = cat(&[&[0x43], &1u32.to_le_bytes(), &[0x22, 1], &4u16.to_le_bytes()]);
    let local_x2 = cat(&[&[0x43], &2u32.to_le_bytes(), &ptg_int(2), &[0x05]]);
    let sheet = cat(&[
        &bof(0x0010),
        &rec(0x0203, &cat(&[&cell(0, 0, 0), &10.0f64.to_le_bytes()])),
        &formula(5, 1, num(60.0), &sum_ext),
        &formula(6, 1, num(20.0), &local_x2),
        &eof(),
    ]);
    let pos = globals(0).len() as u32;
    let (wb, report) = read_xls(&compound("Workbook", &cat(&[&globals(pos), &sheet]))).unwrap();
    let names: Vec<_> = wb.names.iter().map(|n| (n.name.as_str(), n.formula.as_str())).collect();
    assert_eq!(names, [("Local", "Sheet1!$A$1"), ("Twice", "ExtRange*2")]);
    assert_eq!(ftext(&wb, "B6"), None);
    assert_eq!(v(&wb, 0, "B6"), Value::Number(60.0));
    assert_eq!(ftext(&wb, "B7").as_deref(), Some("Local*2"));
    assert!(report.warnings.iter().any(|w| w.contains("defined names") && w.contains("saved results")), "{:?}", report.warnings);
}

#[test]
fn reads_iterative_calculation_settings() {
    // CalcMode, CalcCount, CalcIter and CalcDelta repeat in every worksheet substream; here only
    // the first sheet carries them, so the sheet after it must not reset them to the defaults.
    // CalcPrecision sits in the globals.
    let calc = cat(&[&rec(0x000D, &u16s(&[0])), &rec(0x000C, &u16s(&[50])), &rec(0x0011, &u16s(&[1])), &rec(0x0010, &0.01f64.to_le_bytes())]);
    let sheet1 = cat(&[&bof(0x0010), &calc, &eof()]);
    let sheet2 = cat(&[&bof(0x0010), &eof()]);
    let boundsheet = |pos: u32, name: &str| rec(0x0085, &cat(&[&pos.to_le_bytes(), &[0, 0, name.len() as u8, 0], name.as_bytes()]));
    let globals = |p1: u32, p2: u32| cat(&[&bof(0x0005), &rec(0x000E, &u16s(&[0])), &boundsheet(p1, "A"), &boundsheet(p2, "B"), &eof()]);
    let p1 = globals(0, 0).len() as u32;
    let p2 = p1 + sheet1.len() as u32;
    let (wb, _) = read_xls(&compound("Workbook", &cat(&[&globals(p1, p2), &sheet1, &sheet2]))).unwrap();
    assert_eq!(wb.calc.iterative, Some(true));
    assert_eq!((wb.calc.max_iterations, wb.calc.max_change), (Some(50), Some(0.01)));
    assert_eq!(wb.calc.mode, Some(CalcMode::Manual));
    assert_eq!((wb.sheets[0].calc, wb.sheets[1].calc), (wb.calc, craft_xls::Calc::default()));
    assert!(wb.precision_as_displayed);
    // Without the records nothing is set: the caller applies Excel's defaults.
    let (wb, _) = read_xls(&compound("Workbook", &sample(false))).unwrap();
    assert_eq!(wb.calc, craft_xls::Calc::default());
    assert!(!wb.precision_as_displayed);
}

#[test]
fn a_format_shorter_than_its_count_keeps_its_characters() {
    // A FORMAT whose cch is one more than the characters in the record (seen in real files):
    // Excel opens them, the record's characters are the format code.
    let short = rec(0x041E, &cat(&[&165u16.to_le_bytes(), &6u16.to_le_bytes(), &[0], b"0.000"]));
    let xf = |ifmt: u16| rec(0x00E0, &cat(&[&u16s(&[0, ifmt, 0]), &[0; 14]]));
    let globals = |pos: u32| {
        cat(&[
            &bof(0x0005),
            &short,
            &rec(0x041E, &cat(&[&164u16.to_le_bytes(), &xlstr("yyyy-mm-dd")])),
            &xf(0),
            &xf(164),
            &xf(165),
            &rec(0x0085, &cat(&[&pos.to_le_bytes(), &[0, 0, 6, 0], b"Sheet1"])),
            &eof(),
        ])
    };
    let sheet = cat(&[
        &bof(0x0010),
        &rec(0x0203, &cat(&[&cell(0, 0, 0), &7.0f64.to_le_bytes()])),
        &rec(0x0203, &cat(&[&cell(1, 0, 1), &0.0f64.to_le_bytes()])),
        &rec(0x0203, &cat(&[&cell(2, 0, 2), &1.25f64.to_le_bytes()])),
        &eof(),
    ]);
    let stream = cat(&[&globals(globals(0).len() as u32), &sheet]);
    let (wb, report) = read_xls(&compound("Workbook", &stream)).unwrap();
    let fmt = |a1: &str| fmt(&wb, a1);
    assert_eq!(v(&wb, 0, "A1"), Value::Number(7.0));
    assert_eq!(v(&wb, 0, "A3"), Value::Number(1.25));
    assert_eq!(fmt("A2"), "yyyy-mm-dd");
    assert_eq!(fmt("A3"), "0.000");
    assert!(report.warnings.iter().any(|w| w.contains("declared length")), "{:?}", report.warnings);
}

/// A workbook whose "Report" sheet holds legacy formulas over ranges of "Data": Data!A1 = 11,
/// B1:G1 = 1, A2 = 5, A3 = 7, and Report!B1:G1 filled, so a formula in Report!A1 that spilled
/// would be blocked.
fn legacy_formulas_book() -> Vec<u8> {
    let area3d = |r0: u16, r1: u16, c0: u16, c1: u16| cat(&[&[0x3B], &0u16.to_le_bytes(), &u16s(&[r0, r1, c0, c1])]);
    let a2a3 = area3d(1, 2, 0, 0);
    let sum = |inner: &[u8]| cat(&[inner, &[0x22, 1], &4u16.to_le_bytes()]);
    let boundsheet = |pos: u32, name: &str| rec(0x0085, &cat(&[&pos.to_le_bytes(), &[0, 0, name.len() as u8, 0], name.as_bytes()]));
    let globals = |p1: u32, p2: u32| {
        cat(&[
            &bof(0x0005),
            &boundsheet(p1, "Data"),
            &boundsheet(p2, "Report"),
            &rec(0x01AE, &u16s(&[2, 0x0401])),
            &rec(0x0017, &cat(&[&u16s(&[1]), &u16s(&[0, 0, 0])])),
            &eof(),
        ])
    };
    let number = |r: u16, c: u16, n: f64| rec(0x0203, &cat(&[&cell(r, c, 0), &n.to_le_bytes()]));
    let mut data = bof(0x0010);
    data.extend(number(0, 0, 11.0));
    for c in 1..7 {
        data.extend(number(0, c, 1.0));
    }
    data.extend(number(1, 0, 5.0));
    data.extend(number(2, 0, 7.0));
    data.extend(eof());
    let shared = cat(&[&a2a3, &ptg_int(10), &[0x05]]);
    let array = cat(&[&a2a3, &ptg_int(2), &[0x05]]);
    let single_array = sum(&cat(&[&a2a3, &ptg_int(2), &[0x05]]));
    let mut report = bof(0x0010);
    for c in 1..7 {
        report.extend(number(0, c, 0.0));
    }
    report.extend(cat(&[
        // A1 = Data!A1:G1, A2 = +Data!A1:G1 (a title row repeated from another sheet).
        &formula(0, 0, num(11.0), &area3d(0, 0, 0xC000, 6 | 0xC000)),
        &formula(1, 0, num(11.0), &cat(&[&area3d(0, 0, 0xC000, 6 | 0xC000), &[0x12]])),
        // C2:C3 share Data!$A$2:$A$3*10.
        &formula(1, 2, num(50.0), &exp(1, 2)),
        &rec(0x04BC, &cat(&[&u16s(&[1, 2]), &[2, 2, 0, 2], &(shared.len() as u16).to_le_bytes(), &shared])),
        &formula(2, 2, num(70.0), &exp(1, 2)),
        // D2:D3 = {Data!$A$2:$A$3*2}, entered with Ctrl+Shift+Enter.
        &formula(1, 3, num(10.0), &exp(1, 3)),
        &rec(0x0221, &cat(&[&u16s(&[1, 2]), &[3, 3], &[0; 6], &(array.len() as u16).to_le_bytes(), &array])),
        &formula(2, 3, num(14.0), &exp(1, 3)),
        // H1 = {SUM(Data!$A$2:$A$3*2)}, a one-cell Ctrl+Shift+Enter array.
        &formula(0, 7, num(24.0), &exp(0, 7)),
        &rec(0x0221, &cat(&[&u16s(&[0, 0]), &[7, 7], &[0; 6], &(single_array.len() as u16).to_le_bytes(), &single_array])),
        // H2 = MAX(SUM(Data!$A$2:$A$3/12),0), H3 = SUM(Data!$A$2:$A$3), H4 = SQRT(Data!$A$2:$A$3).
        &formula(1, 7, num(5.0 / 12.0), &cat(&[&sum(&cat(&[&a2a3, &ptg_int(12), &[0x06]])), &ptg_int(0), &[0x22, 2], &7u16.to_le_bytes()])),
        &formula(2, 7, num(12.0), &sum(&a2a3)),
        &formula(3, 7, special(2, 0x0F), &cat(&[&a2a3, &[0x21], &20u16.to_le_bytes()])),
        &eof(),
    ]));
    let p1 = globals(0, 0).len() as u32;
    let p2 = p1 + data.len() as u32;
    cat(&[&globals(p1, p2), &data, &report])
}

/// Every formula in an .xls file is a legacy one. They are read as stored: adding Excel's
/// implicit intersection (`@`) is the job of a caller whose formula language has dynamic arrays.
#[test]
fn legacy_formulas_are_read_as_stored() {
    let bytes = compound("Workbook", &legacy_formulas_book());
    let formula = |wb: &Workbook, a1: &str| {
        let f = wb.sheets[1].cells[&at(a1)].formula.clone().unwrap();
        (f.text, f.array)
    };
    let (wb, _) = read_xls(&bytes).unwrap();
    assert_eq!(formula(&wb, "A1"), ("Data!A1:G1".into(), None));
    assert_eq!(formula(&wb, "A2"), ("+Data!A1:G1".into(), None));
    // Shared formulas, each cell on its own.
    assert_eq!(formula(&wb, "C2"), ("Data!$A$2:$A$3*10".into(), None));
    assert_eq!(formula(&wb, "C3"), ("Data!$A$2:$A$3*10".into(), None));
    // Ctrl+Shift+Enter arrays, one cell or several, keep their range on the anchor.
    assert_eq!(formula(&wb, "D2"), ("Data!$A$2:$A$3*2".into(), Some(range("D2:D3"))));
    assert_eq!(formula(&wb, "H1"), ("SUM(Data!$A$2:$A$3*2)".into(), Some(range("H1"))));
    assert_eq!(formula(&wb, "H2"), ("MAX(SUM(Data!$A$2:$A$3/12),0)".into(), None));
    assert_eq!(formula(&wb, "H3"), ("SUM(Data!$A$2:$A$3)".into(), None));
    assert_eq!(formula(&wb, "H4"), ("SQRT(Data!$A$2:$A$3)".into(), None));
    assert_eq!(v(&wb, 1, "H4"), Value::Error(CellError::Value));
}

fn one_sheet_with(sheet_records: &[u8]) -> Vec<u8> {
    let globals = |pos: u32| cat(&[&bof(0x0005), &rec(0x0085, &cat(&[&pos.to_le_bytes(), &[0, 0, 6, 0], b"Sheet1"])), &eof()]);
    let sheet = cat(&[&bof(0x0010), sheet_records, &rec(0x0203, &cat(&[&cell(0, 0, 0), &7.0f64.to_le_bytes()])), &eof()]);
    compound("Workbook", &cat(&[&globals(globals(0).len() as u32), &sheet]))
}

#[test]
fn xls_window2_right_to_left_sheet_opens_right_to_left() {
    let window2 = |flags: u16| rec(0x023E, &u16s(&[flags, 0, 0, 64, 0, 0, 0, 0, 0]));
    let view = |file: &[u8]| {
        let (wb, _) = read_xls(file).unwrap();
        let view = wb.sheets[0].view.unwrap();
        (view.right_to_left, view.show_gridlines, view.show_headings, view.show_zeros, view.show_formulas)
    };
    assert_eq!(view(&one_sheet_with(&window2(0x0040 | 0x0004))), (true, false, true, false, false));
    assert_eq!(view(&one_sheet_with(&window2(0x0001 | 0x0002 | 0x0010))), (false, true, false, true, true));
}

#[test]
fn xls_short_window2_is_ignored() {
    for payload in [&[][..], &[0x40][..], &[0x40, 0][..], &[0x40, 0, 0, 0, 0, 0, 0, 0, 0][..]] {
        let (wb, _) = read_xls(&one_sheet_with(&rec(0x023E, payload))).unwrap();
        assert_eq!(wb.sheets[0].view, None::<View>);
        assert_eq!(v(&wb, 0, "A1"), Value::Number(7.0));
    }
}

/// Shared-formula followers each keep their own text, charged to the workbook's text allowance:
/// with no allowance left they keep their cached values, and the import still succeeds.
#[test]
fn shared_formula_text_counts_against_the_text_allowance() {
    let file = compound("Workbook", &sample(false));
    // The smallest allowance that reads every string: nothing is left for expanded formulas.
    let left = (0..1000).find(|n| read_limited(&file, text_limit(*n)).is_ok()).unwrap();
    let (wb, report) = read_limited(&file, text_limit(left)).unwrap();
    assert_eq!(ftext(&wb, "B6"), None);
    assert_eq!(v(&wb, 0, "B6"), Value::Number(20.0));
    assert!(report.warnings.iter().any(|w| w.contains("shared-formula text")), "{:?}", report.warnings);
    // an array anchor after the cut-off still gets its formula
    assert!(ftext(&wb, "C5").is_some());
    // With room, the followers keep their formulas.
    let (wb, _) = read_xls(&file).unwrap();
    assert!(ftext(&wb, "B6").is_some());
}

/// Shared-formula text is optional: an early sheet's expanded formulas must not use the allowance
/// a later sheet's inline strings need, or the whole import fails for text that fits.
#[test]
fn shared_formulas_do_not_starve_a_later_sheets_text() {
    let shared = cat(&[&[0x4C], &u16s(&[0, 0xFFFF]), &ptg_int(10), &[0x05]]);
    let boundsheet = |pos: u32, name: &str| rec(0x0085, &cat(&[&pos.to_le_bytes(), &[0, 0, name.len() as u8, 0], name.as_bytes()]));
    let globals = |p: [u32; 2]| cat(&[&bof(0x0005), &boundsheet(p[0], "One"), &boundsheet(p[1], "Two"), &eof()]);
    let one = cat(&[
        &bof(0x0010),
        &formula(4, 1, num(10.0), &exp(4, 1)),
        &rec(0x04BC, &cat(&[&u16s(&[4, 5]), &[1, 1, 0, 2], &(shared.len() as u16).to_le_bytes(), &shared])),
        &formula(5, 1, num(20.0), &exp(4, 1)),
        &eof(),
    ]);
    let two = cat(&[&bof(0x0010), &rec(0x0204, &cat(&[&cell(0, 0, 0), &xlstr("abcdefgh")])), &eof()]);
    let p1 = globals([0; 2]).len() as u32;
    let file = compound("Workbook", &cat(&[&globals([p1, p1 + one.len() as u32]), &one, &two]));
    // The smallest allowance that reads every string leaves nothing for expanded formulas.
    let left = (0..1000).find(|n| read_limited(&file, text_limit(*n)).is_ok()).unwrap();
    let (wb, _) = read_limited(&file, text_limit(left)).unwrap();
    assert_eq!(v(&wb, 1, "A1"), text("abcdefgh"));
    assert_eq!(ftext(&wb, "B6"), None);
    assert_eq!(v(&wb, 0, "B6"), Value::Number(20.0));
    let (wb, _) = read_xls(&file).unwrap();
    assert!(ftext(&wb, "B6").is_some());
}

/// Sheet names are returned as stored: two worksheets may share a name (a caller that needs
/// unique names checks them).
#[test]
fn duplicate_sheet_names_are_returned_as_stored() {
    let sheet = cat(&[&bof(0x0010), &eof()]);
    let boundsheet = |pos: u32, name: &str| rec(0x0085, &cat(&[&pos.to_le_bytes(), &[0, 0, name.len() as u8, 0], name.as_bytes()]));
    let globals = |p: u32| cat(&[&bof(0x0005), &boundsheet(p, "One"), &boundsheet(p + sheet.len() as u32, "one"), &eof()]);
    let file = compound("Workbook", &cat(&[&globals(globals(0).len() as u32), &sheet, &sheet]));
    let (wb, _) = read_xls(&file).unwrap();
    assert_eq!(wb.sheets.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["One", "one"]);
}

/// RString ([MS-XLS] 2.4.229): a label with formatting runs. The text is read, the runs skipped.
#[test]
fn rstring_cells_are_read() {
    let rstring = rec(0x00D6, &cat(&[&cell(1, 0, 0), &xlstr("rich"), &u16s(&[1, 0, 5])]));
    let (wb, _) = read_xls(&one_sheet_with(&rstring)).unwrap();
    assert_eq!(v(&wb, 0, "A2"), text("rich"));
}

#[test]
fn the_fixture_writer_reads_back() {
    let long = "é".repeat(3000);
    let rows =
        vec![vec!["Name".to_string(), "Żółw".into(), String::new(), "2.5".into()], vec!["date:45000".into(), long.clone(), long.clone() + "x"]];
    let bytes = craft_xls::testing::workbook(&[("Erste", rows), ("Second", vec![vec!["Name".into()]])]);
    assert!(is_xls(&bytes));
    let read = craft_xls::read(&bytes).unwrap();
    assert_eq!(read.warnings, []);
    let wb = read.workbook;
    assert_eq!(wb.sheets.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["Erste", "Second"]);
    assert_eq!((v(&wb, 0, "A1"), v(&wb, 0, "B1"), v(&wb, 0, "C1"), v(&wb, 0, "D1")), (text("Name"), text("Żółw"), Value::Empty, Value::Number(2.5)));
    assert_eq!((v(&wb, 0, "A2"), fmt(&wb, "A2")), (Value::Number(45000.0), "m/d/yyyy".to_string()));
    // Two 3,000-character strings do not fit one record: the table continues in a Continue record.
    assert_eq!((v(&wb, 0, "B2"), v(&wb, 0, "C2")), (text(&long), text(&(long.clone() + "x"))));
    assert_eq!(v(&wb, 1, "A1"), text("Name"));
}

#[test]
fn cell_sheet_and_merge_caps_are_enforced() {
    let number = |r: u16| rec(0x0203, &cat(&[&cell(r, 0, 0), &1.0f64.to_le_bytes()]));
    // `one_sheet_with` adds one cell of its own.
    let file = one_sheet_with(&cat(&[&number(1), &number(2)]));
    assert!(read_limited(&file, Limits { max_cells: 3, ..Limits::default() }).is_ok());
    assert!(matches!(read_limited(&file, Limits { max_cells: 2, ..Limits::default() }), Err(Error::TooLarge(_))));

    // 4,097 BoundSheet records.
    let mut globals = bof(0x0005);
    for i in 0..4097u32 {
        globals.extend(rec(0x0085, &cat(&[&i.to_le_bytes(), &[0, 0, 1, 0], b"S"])));
    }
    globals.extend(eof());
    assert!(matches!(read_xls(&compound("Workbook", &globals)), Err(Error::TooLarge(_))));

    // 100,001 merged ranges, 1,000 to a record.
    let merges = |n: u16| {
        (0..n).fold(u16s(&[n]), |mut p, i| {
            p.extend(u16s(&[i, i, 0, 1]));
            p
        })
    };
    let mut records: Vec<u8> = (0..100).flat_map(|_| rec(0x00E5, &merges(1000))).collect();
    assert_eq!(read_xls(&one_sheet_with(&records)).unwrap().0.sheets[0].merges.len(), 100_000);
    records.extend(rec(0x00E5, &merges(1)));
    assert!(matches!(read_xls(&one_sheet_with(&records)), Err(Error::TooLarge(_))));
}

/// The sheet a workbook was saved on (Window1's `itabCur`) is counted over all the file's
/// sheets, skipped ones included.
#[test]
fn reports_the_sheet_it_was_saved_on() {
    // BoundSheet: position, visibility (0 visible, 1 hidden), kind, name.
    let boundsheet = |pos: u32, state: u8, name: &str| rec(0x0085, &cat(&[&pos.to_le_bytes(), &[state, 0, name.len() as u8, 0], name.as_bytes()]));
    let window1 = |tab: u16| rec(0x003D, &u16s(&[0, 0, 0x4000, 0x2000, 0x0038, tab, 0, 1, 600]));
    let dialog = cat(&[&bof(0x0010), &rec(0x0081, &[0x10, 0x00]), &eof()]);
    let worksheet = |n: f64| cat(&[&bof(0x0010), &rec(0x0081, &[0x01, 0x04]), &rec(0x0203, &cat(&[&cell(0, 0, 0), &n.to_le_bytes()])), &eof()]);
    // Dialog1 (skipped), Sheet1, Sheet2, Hidden (hidden).
    let book = |window: &[u8]| {
        let globals = |p: [u32; 4]| {
            cat(&[
                &bof(0x0005),
                window,
                &boundsheet(p[0], 0, "Dialog1"),
                &boundsheet(p[1], 0, "Sheet1"),
                &boundsheet(p[2], 0, "Sheet2"),
                &boundsheet(p[3], 1, "Hidden"),
                &eof(),
            ])
        };
        let (d, s1, s2, h) = (dialog.clone(), worksheet(1.0), worksheet(2.0), worksheet(3.0));
        let base = globals([0; 4]).len() as u32;
        let p = [base, base + d.len() as u32, base + (d.len() + s1.len()) as u32, base + (d.len() + s1.len() + s2.len()) as u32];
        let stream = cat(&[&globals(p), &d, &s1, &s2, &h]);
        let (wb, _) = read_xls(&compound("Workbook", &stream)).unwrap();
        wb.active_sheet.map(|i| wb.sheets[i].name.clone())
    };
    assert_eq!(book(&window1(2)).as_deref(), Some("Sheet2"), "itabCur 2 is Sheet2, counting the skipped dialog sheet");
    assert_eq!(book(&window1(1)).as_deref(), Some("Sheet1"));
    assert_eq!(book(&window1(3)).as_deref(), Some("Hidden"), "a hidden sheet is reported; the caller falls back");
    assert_eq!(book(&window1(0)), None, "a skipped sheet");
    assert_eq!(book(&window1(99)), None, "an index past the sheets");
    assert_eq!(book(&[]), None, "no window record");
    assert_eq!(book(&rec(0x003D, &u16s(&[0, 0, 0, 0, 0]))), None, "a window record too short for the index");
    // Only the first window counts.
    assert_eq!(book(&cat(&[&window1(1), &window1(2)])).as_deref(), Some("Sheet1"));
}

// Encrypted workbooks. Ported from storytold/gridcraft#847 (RC4) and #3713 (RC4 CryptoAPI).

fn read_pw(file: &[u8], password: &str) -> Result<craft_xls::Read, Error> {
    craft_xls::read_with_password(file, Limits::default(), password)
}

/// A sheet over several 1024-byte blocks, with records straddling the boundaries.
fn long_sheet() -> Vec<u8> {
    let boundsheet = |pos: u32| rec(0x0085, &cat(&[&pos.to_le_bytes(), &[0, 0, 4, 0], b"Long"]));
    let globals = |pos: u32| cat(&[&bof(0x0005), &boundsheet(pos), &eof()]);
    let mut stream = globals(globals(0).len() as u32);
    stream.extend(bof(0x0010));
    for r in 0..500u16 {
        stream.extend(rec(0x0203, &cat(&[&cell(r, 0, 0), &f64::from(r).to_le_bytes()])));
    }
    stream.extend(eof());
    stream
}

fn with_filepass(payload: &[u8]) -> Vec<u8> {
    compound("Workbook", &cat(&[&bof(0x0005), &rec(0x002F, payload), &eof()]))
}

#[test]
fn rc4_encrypted_workbooks_read_with_their_password() {
    use craft_xls::testing::encrypt_rc4;
    let plain = compound("Workbook", &sample(false));
    let file = compound("Workbook", &encrypt_rc4(&sample(false), "a password").unwrap());
    assert!(is_xls(&file) && craft_xls::is_encrypted(&file) && !craft_xls::is_encrypted(&plain));
    assert_eq!(read_xls(&file).err(), Some(Error::Encrypted));
    assert_eq!(read_pw(&file, "A password").err(), Some(Error::WrongPassword));
    let wb = read_pw(&file, "a password").unwrap().workbook;
    let (expected, _) = read_xls(&plain).unwrap();
    assert_eq!(wb.sheets.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["Sheet1", "Data"]);
    assert_eq!(v(&wb, 0, "D1"), text("Wörld"));
    assert_eq!(ftext(&wb, "B3").as_deref(), Some("SUM(B1:C1)"));
    assert_eq!(wb.sheets[0].cells, expected.sheets[0].cells);
    // A workbook without a password ignores the one given.
    assert!(read_pw(&plain, "x").is_ok());
    let file = compound("Workbook", &encrypt_rc4(&long_sheet(), "a longer pass").unwrap());
    let wb = read_pw(&file, "a longer pass").unwrap().workbook;
    for r in [0u32, 56, 57, 499] {
        assert_eq!(wb.sheets[0].cells[&CellRef::new(r, 0)].value, Value::Number(f64::from(r)), "row {r}");
    }
    // Schemes no password opens are named.
    assert_eq!(read_pw(&with_filepass(&[0, 0, 0x34, 0x12, 0x78, 0x56]), "x").err(), Some(Error::UnsupportedEncryption("XOR obfuscation".into())));
    assert_eq!(read_pw(&with_filepass(&[1, 0, 3, 0, 3, 0]), "x").err(), Some(Error::UnsupportedEncryption("version 3.3".into())));
    assert!(matches!(read_pw(&with_filepass(&[1, 0, 1, 0, 1, 0, 9]), "x"), Err(Error::Invalid(_))));
}

#[test]
fn excels_default_password_opens_without_asking() {
    // A workbook protected only against changes is encrypted with "VelvetSweatshop".
    let file = compound("Workbook", &craft_xls::testing::encrypt_rc4(&sample(false), "VelvetSweatshop").unwrap());
    assert_eq!(read_xls(&file).unwrap().0.sheets.len(), 2);
    let file = compound("Workbook", &craft_xls::testing::encrypt_cryptoapi(&sample(false), "VelvetSweatshop", 128).unwrap());
    assert_eq!(read_xls(&file).unwrap().0.sheets.len(), 2);
}

#[test]
fn rc4_cryptoapi_encrypted_workbooks_read_with_their_password() {
    use craft_xls::testing::encrypt_cryptoapi;
    let (expected, _) = read_xls(&compound("Workbook", &sample(false))).unwrap();
    // Current Excel's 128-bit key, the oldest 40-bit one (padded to 128 bits) and one between.
    for bits in [128, 40, 56] {
        let file = compound("Workbook", &encrypt_cryptoapi(&sample(false), "Synthetic pw 9", bits).unwrap());
        assert!(craft_xls::is_encrypted(&file), "{bits}");
        assert_eq!(read_xls(&file).err(), Some(Error::Encrypted), "{bits}");
        assert_eq!(read_pw(&file, "synthetic pw 9").err(), Some(Error::WrongPassword), "{bits}");
        let wb = read_pw(&file, "Synthetic pw 9").unwrap().workbook;
        assert_eq!(v(&wb, 0, "D1"), text("Wörld"));
        assert_eq!(ftext(&wb, "B3").as_deref(), Some("SUM(B1:C1)"));
        assert_eq!(wb.sheets[0].cells, expected.sheets[0].cells, "{bits}");
        assert_eq!(wb.sheets[1].cells, expected.sheets[1].cells, "{bits}");
    }
    let file = compound("Workbook", &encrypt_cryptoapi(&long_sheet(), "pw", 128).unwrap());
    let wb = read_pw(&file, "pw").unwrap().workbook;
    for r in [0u32, 56, 57, 120, 499] {
        assert_eq!(wb.sheets[0].cells[&CellRef::new(r, 0)].value, Value::Number(f64::from(r)), "row {r}");
    }
}

/// The RC4 CryptoAPI FilePass fields are the file's word: other ciphers, hashes and key sizes are
/// named, and sizes that don't fit are a damaged file, never a panic.
#[test]
fn rc4_cryptoapi_filepass_fields_are_checked() {
    let good = craft_xls::testing::cryptoapi_filepass("pw", 128);
    // The EncryptionHeader starts at byte 14.
    let edited = |at: usize, word: u32| {
        let mut f = good.clone();
        f[at..at + 4].copy_from_slice(&word.to_le_bytes());
        read_pw(&with_filepass(&f), "pw")
    };
    let unsupported = |r: Result<craft_xls::Read, Error>| match r {
        Err(Error::UnsupportedEncryption(s)) => s,
        other => panic!("{other:?}"),
    };
    let damaged = |r: Result<craft_xls::Read, Error>| matches!(&r, Err(Error::Invalid(m)) if m.contains("encryption record"));
    // Decrypted, the record alone is a workbook with no sheets: refused for that, not for its key.
    let decrypts = |r: Result<craft_xls::Read, Error>| matches!(&r, Err(Error::Invalid(m)) if m.contains("no worksheets"));
    assert!(decrypts(read_pw(&with_filepass(&good), "pw")));
    assert_eq!(unsupported(edited(14 + 8, 0x660E)), "CryptoAPI with cipher 0x660E");
    assert_eq!(unsupported(edited(6, 0x24)), "CryptoAPI AES");
    assert_eq!(unsupported(edited(6, 0x14)), "CryptoAPI with an external provider");
    assert_eq!(unsupported(edited(14 + 12, 0x800C)), "RC4 CryptoAPI with hash 0x800C");
    for bits in [32, 136, 256, 44, u32::MAX] {
        assert_eq!(unsupported(edited(14 + 16, bits)), format!("RC4 CryptoAPI with a {bits}-bit key"));
    }
    // A key size of 0 means 40 bits: a valid header, so only the verifier tells.
    assert_eq!(edited(14 + 16, 0).err(), Some(Error::WrongPassword));
    // An algorithm of 0 means RC4 and SHA-1.
    assert!(decrypts(edited(14 + 8, 0)) && decrypts(edited(14 + 12, 0)));
    let header_size = u32::from_le_bytes(good[10..14].try_into().unwrap()) as usize;
    for size in [0, 31, u32::MAX, header_size as u32 + 1000] {
        assert!(damaged(edited(10, size)), "header size {size}");
    }
    let verifier_at = 14 + header_size;
    assert!(damaged(edited(verifier_at, 20)), "salt size");
    assert!(damaged(edited(verifier_at, u32::MAX)), "salt size");
    assert!(damaged(edited(verifier_at + 36, 16)), "verifier hash size");
    // Cut anywhere, the record is a damaged file, never a panic.
    for cut in 6..good.len() {
        assert!(damaged(read_pw(&with_filepass(&good[..cut]), "pw")), "cut at {cut}");
    }
}
