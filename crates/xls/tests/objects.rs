//! Comments, conditional formats, pictures and the AutoFilter range, each record assembled here
//! from [MS-XLS] and [MS-ODRAW]; no Excel-produced file is used. Ported from
//! storytold/gridcraft#3457, with tests for the fixes its review asked for.

mod common;

use common::*;
use craft_xls::{ClientAnchor, Color, ConditionRule, DxfFill, Operator, Pattern, Range, Workbook};

fn book(globals_extra: &[u8], sheet_records: &[u8]) -> (Workbook, Report) {
    let xf = rec(0x00E0, &cat(&[&u16s(&[0, 0, 0]), &[0; 14]]));
    let globals = |pos: u32| cat(&[&bof(0x0005), &xf, globals_extra, &rec(0x0085, &cat(&[&pos.to_le_bytes(), &[0, 0, 6, 0], b"Sheet1"])), &eof()]);
    let sheet = cat(&[&bof(0x0010), sheet_records, &eof()]);
    let stream = cat(&[&globals(globals(0).len() as u32), &sheet]);
    read_xls(&compound("Workbook", &stream)).unwrap()
}

fn wide(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

/// Obj: FtCmo (ft 0x15, cb 0x12, ot 0x19 comment, id, flags, reserved), then FtEnd.
fn obj(id: u16) -> Vec<u8> {
    rec(0x005D, &cat(&[&u16s(&[0x15, 0x12, 0x19, id, 0x4011]), &[0; 12], &u16s(&[0, 0])]))
}

/// TxO with its text and formatting runs in Continue records.
fn txo(text: &str) -> Vec<u8> {
    let n = text.encode_utf16().count() as u16;
    cat(&[
        &rec(0x01B6, &cat(&[&u16s(&[0x0212, 0]), &[0; 6], &u16s(&[n, 16, 0, 0])])),
        &rec(0x003C, &cat(&[&[1], &wide(text)])),
        &rec(0x003C, &[0; 16]),
    ])
}

/// Note: row, column, flags (0x0002: shown), object id, author (UTF-16).
fn note(r: u16, c: u16, flags: u16, id: u16, author: &str) -> Vec<u8> {
    rec(0x001C, &cat(&[&u16s(&[r, c, flags, id]), &(author.encode_utf16().count() as u16).to_le_bytes(), &[1], &wide(author), &[0]]))
}

#[test]
fn reads_cell_comments() {
    let (wb, report) = book(
        &[],
        &cat(&[
            &rec(0x0203, &cat(&[&cell(1, 2, 0), &1.0f64.to_le_bytes()])),
            &obj(1),
            &txo("Đỗ Anh Hải Sơn:\nKiểm tra lại số tài khoản"),
            &obj(2),
            &txo("Ghi chú"),
            &note(1, 2, 0x0002, 1, "Đỗ Anh Hải Sơn"),
            &note(4, 0, 0, 2, "Kế toán"),
            // A Note for an object with no text, and one outside the grid, break nothing.
            &note(5, 0, 0, 9, "x"),
            &note(0xFFFF, 300, 0, 1, "y"),
        ]),
    );
    let notes = &wb.sheets[0].notes;
    assert_eq!(notes.len(), 3, "{:?}", report.warnings);
    let got: Vec<_> = notes.iter().map(|n| (n.cell.a1(), n.author.as_str(), n.text.as_str(), n.visible)).collect();
    assert_eq!(
        got,
        [
            ("C2".to_string(), "Đỗ Anh Hải Sơn", "Đỗ Anh Hải Sơn:\nKiểm tra lại số tài khoản", true),
            ("A5".to_string(), "Kế toán", "Ghi chú", false),
            ("A6".to_string(), "x", "", false),
        ]
    );
}

#[test]
fn damaged_object_records_are_skipped_not_fatal() {
    // A short Obj, a short Note and a TxO cut before its count: each is skipped with a warning,
    // and the comment after them still reads.
    let (wb, report) = book(
        &[],
        &cat(&[&rec(0x005D, &[0x15, 0]), &rec(0x001C, &[1, 0]), &obj(3), &rec(0x01B6, &[0; 4]), &obj(1), &txo("kept"), &note(0, 0, 0, 1, "a")]),
    );
    assert_eq!(wb.sheets[0].notes.iter().map(|n| n.text.as_str()).collect::<Vec<_>>(), ["kept"]);
    assert!(report.warnings.iter().any(|w| w.contains("comment or object records were damaged")), "{:?}", report.warnings);
}

/// A CF record: kind, operator, formula sizes, DXFN, the formulas.
fn cf(kind: u8, op: u8, rgce: &[u8], dxfn: &[u8]) -> Vec<u8> {
    rec(0x01B1, &cat(&[&[kind, op], &u16s(&[rgce.len() as u16, 0]), dxfn, rgce]))
}

/// CondFmt: rule count, flags, bounding range (rows, columns), then the range list.
fn condfmt(bound: [u16; 4], ranges: &[[u16; 4]]) -> Vec<u8> {
    let list: Vec<u8> = ranges.iter().flat_map(|r| u16s(r)).collect();
    rec(0x01B0, &cat(&[&u16s(&[2, 0]), &u16s(&bound), &(ranges.len() as u16).to_le_bytes(), &list]))
}

/// `PtgRefN` to the anchor cell (both offsets 0), then `> 5`.
fn relative() -> Vec<u8> {
    cat(&[&[0x2C], &u16s(&[0, 0xC000]), &[0x1E], &5u16.to_le_bytes(), &[0x0D]])
}

#[test]
fn reads_conditional_formats() {
    // DXFN: font block (26) and pattern block (29), the foreground colour unset (17).
    let mut font = vec![0u8; 118];
    font[64..68].copy_from_slice(&u32::MAX.to_le_bytes());
    font[72..74].copy_from_slice(&700u16.to_le_bytes());
    font[80..84].copy_from_slice(&10u32.to_le_bytes());
    font[88..92].copy_from_slice(&0x82u32.to_le_bytes());
    font[92..96].copy_from_slice(&1u32.to_le_bytes());
    font[96..100].copy_from_slice(&1u32.to_le_bytes());
    let dxfn = cat(&[&((1u32 << 26) | (1 << 29) | (1 << 17)).to_le_bytes(), &0u16.to_le_bytes(), &font, &u16s(&[1 << 10, 64 | (13 << 7)])]);
    let greater = cat(&[&[0x1E], &100u16.to_le_bytes()]);
    let (wb, report) = book(
        &[],
        &cat(&[
            &rec(0x0203, &cat(&[&cell(1, 1, 0), &150.0f64.to_le_bytes()])),
            &condfmt([1, 3, 1, 2], &[[1, 3, 1, 2]]),
            &cf(1, 5, &greater, &dxfn),
            &cf(2, 0, &relative(), &dxfn),
        ]),
    );
    let cfs = &wb.sheets[0].conditional_formats;
    assert_eq!(cfs.len(), 2, "{:?}", report.warnings);
    assert_eq!(cfs[0].ranges, vec![Range::parse("B2:C4").unwrap()]);
    assert_eq!(cfs[0].rule, ConditionRule::CellIs { operator: Operator::Greater, formula1: "100".into(), formula2: None });
    let f = &cfs[0].format;
    assert_eq!((f.bold, f.italic, f.strike, f.underline), (Some(true), None, None, None));
    assert_eq!(f.font_color, Some(Color::Rgb(0xFF0000)));
    assert_eq!(f.fill, Some(DxfFill { pattern: Some(Pattern::Solid), fg: None, bg: Some(Color::Rgb(0xFFFF00)) }));
    assert_eq!(cfs[1].rule, ConditionRule::Expression { formula: "B2>5".into() });
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
}

#[test]
fn formulas_are_relative_to_the_bounding_range() {
    // Ranges D5:D6 then B2:B3: the bounding range starts at B2, and Excel writes the formula
    // from there, whatever order the ranges are in.
    let (wb, report) =
        book(&[], &cat(&[&condfmt([1, 5, 1, 3], &[[4, 5, 3, 3], [1, 2, 1, 1]]), &cf(2, 0, &relative(), &cat(&[&0u32.to_le_bytes(), &[0, 0]]))]));
    let cfs = &wb.sheets[0].conditional_formats;
    assert_eq!(cfs.len(), 1, "{:?}", report.warnings);
    assert_eq!(cfs[0].rule, ConditionRule::Expression { formula: "B2>5".into() });
}

#[test]
fn unreadable_conditional_formats_are_skipped() {
    let plain = cat(&[&0u32.to_le_bytes(), &[0, 0]]);
    let (wb, report) = book(
        &[],
        &cat(&[
            // A CF with no CondFmt before it, a damaged CondFmt, an unknown rule type, a short CF.
            &cf(2, 0, &relative(), &plain),
            &rec(0x01B0, &[1]),
            &cf(2, 0, &relative(), &plain),
            &condfmt([0, 0, 0, 0], &[[0, 0, 0, 0]]),
            &cf(3, 0, &relative(), &plain),
            &rec(0x01B1, &[1]),
            &cf(2, 0, &relative(), &plain),
            &rec(0x0879, &[0; 20]),
        ]),
    );
    assert_eq!(wb.sheets[0].conditional_formats.len(), 1);
    assert!(report.warnings.iter().any(|w| w.contains("conditional formatting rules could not be read")), "{:?}", report.warnings);
    assert!(report.warnings.iter().any(|w| w.contains("data bars")), "{:?}", report.warnings);
}

fn od(vi: u16, ty: u16, body: &[u8]) -> Vec<u8> {
    cat(&[&vi.to_le_bytes(), &ty.to_le_bytes(), &(body.len() as u32).to_le_bytes(), body])
}

fn fbse(blip: &[u8]) -> Vec<u8> {
    od(0x0062, 0xF007, &cat(&[&[0u8; 36], blip]))
}

fn shape(pib: u32, anchor: [u16; 8]) -> Vec<u8> {
    let opt = od(0x0013, 0xF00B, &cat(&[&0x4104u16.to_le_bytes(), &pib.to_le_bytes()]));
    let a: Vec<u8> = std::iter::once(0u16).chain(anchor).flat_map(u16::to_le_bytes).collect();
    od(0x000F, 0xF004, &cat(&[&opt, &od(0, 0xF010, &a)]))
}

/// The drawing group (split over a Continue record, as real files do) and one sheet's drawing.
fn pictures(blips: &[Vec<u8>], shapes: &[u8]) -> (Workbook, Report) {
    let dgg = od(0x000F, 0xF000, &od(0x002F, 0xF001, &blips.iter().map(|b| fbse(b)).collect::<Vec<_>>().concat()));
    let (first, second) = dgg.split_at(30);
    let dg = od(0x000F, 0xF002, &od(0x000F, 0xF003, shapes));
    // MsoDrawing records are at most 8224 bytes; longer drawings continue in more of them.
    let drawing: Vec<u8> = dg.chunks(8000).flat_map(|c| rec(0x00EC, c)).collect();
    book(&cat(&[&rec(0x00EB, first), &rec(0x003C, second)]), &drawing)
}

#[test]
fn reads_pictures() {
    let png = [0x89, b'P', b'N', b'G', 1, 2, 3];
    let blip = od(0x6E00, 0xF01E, &cat(&[&[0u8; 17], &png]));
    let emf_bytes = b" EMF the picture's own records".to_vec();
    let packed = miniz_oxide::deflate::compress_to_vec_zlib(&emf_bytes, 6);
    let meta_header = cat(&[&(emf_bytes.len() as u32).to_le_bytes(), &[0u8; 24], &(packed.len() as u32).to_le_bytes(), &[0x00, 0xFE]]);
    let emf = od(0x3D40, 0xF01A, &cat(&[&[0u8; 16], &meta_header, &packed]));
    let damaged = od(0x3D40, 0xF01A, &[0u8; 60]);
    let shapes = cat(&[
        &shape(1, [1, 0, 1, 0, 3, 0, 3, 0]),
        &shape(2, [5, 0, 5, 0, 6, 0, 6, 0]),
        &shape(3, [7, 0, 7, 0, 8, 0, 8, 0]),
        &shape(1, [9, 512, 9, 128, 10, 0, 10, 0]),
    ]);
    let (wb, report) = pictures(&[blip, emf, damaged], &shapes);
    let p = &wb.sheets[0].pictures;
    assert_eq!(p.len(), 3, "{:?}", report.warnings);
    assert_eq!((p[0].data.as_slice(), p[0].mime), (&png[..], "image/png"));
    assert_eq!(p[0].anchor, ClientAnchor { col1: 1, dx1: 0, row1: 1, dy1: 0, col2: 3, dx2: 0, row2: 3, dy2: 0 });
    assert_eq!((p[1].data.as_slice(), p[1].mime), (&emf_bytes[..], "image/x-emf"));
    // Two shapes that show one picture share its bytes.
    assert!(std::sync::Arc::ptr_eq(&p[0].data, &p[2].data));
    assert_eq!((p[2].anchor.dx1, p[2].anchor.dy1), (512, 128));
    assert!(report.warnings.iter().any(|w| w.contains("pictures were damaged")), "{:?}", report.warnings);
}

#[test]
fn pictures_past_the_cap_are_skipped() {
    let png = od(0x6E00, 0xF01E, &cat(&[&[0u8; 17], b"png"]));
    let shapes: Vec<u8> = (0..65_537).flat_map(|_| shape(1, [0; 8])).collect();
    let (wb, report) = pictures(&[png], &shapes);
    assert_eq!(wb.sheets[0].pictures.len(), 65_536);
    assert!(report.warnings.iter().any(|w| w.contains("past the limits")), "{:?}", report.warnings);
}

#[test]
fn reads_the_autofilter_range() {
    let read = |filter_on: bool| {
        // _FilterDatabase on sheet 1 (hidden, built-in 13) = Sheet1!$A$8:$DH$19 (PtgArea3d).
        let area = cat(&[&[0x3B], &0u16.to_le_bytes(), &u16s(&[7, 18, 0, 111])]);
        let lbl = rec(0x0018, &cat(&[&u16s(&[0x0021]), &[0, 1], &u16s(&[area.len() as u16, 0, 1]), &[0; 4], &[0, 13], &area]));
        let names = cat(&[&rec(0x01AE, &u16s(&[1, 0x0401])), &rec(0x0017, &cat(&[&u16s(&[1]), &u16s(&[0, 0, 0])])), &lbl]);
        let info = if filter_on { rec(0x009D, &u16s(&[112])) } else { vec![] };
        book(&names, &cat(&[&rec(0x0203, &cat(&[&cell(7, 0, 0), &1.0f64.to_le_bytes()])), &info])).0
    };
    let wb = read(true);
    assert_eq!(wb.sheets[0].autofilter, Some(Range::parse("A8:DH19").unwrap()));
    assert!(!wb.names.iter().any(|n| n.name.contains("FilterDatabase")), "a built-in name, not a user one");
    assert_eq!(read(false).sheets[0].autofilter, None, "the name alone turns no filter on");
}
