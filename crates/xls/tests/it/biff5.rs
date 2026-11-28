//! Original synthetic BIFF5 records assembled from the published specification; no user files.
use super::common::{cat, cell, compound, eof, num, rec, special, u16s, v};
use craft_xls::{CellRef, Error, Limits, Pattern, Value, Warning, read, read_with};

fn bof(kind: u16) -> Vec<u8> {
    rec(0x0809, &u16s(&[0x0500, kind, 0, 0]))
}
fn label(row: u16, col: u16, text: &[u8]) -> Vec<u8> {
    rec(0x0204, &cat(&[&cell(row, col, 0), &(text.len() as u16).to_le_bytes(), text]))
}
fn formula(row: u16, value: [u8; 8], tokens: &[u8]) -> Vec<u8> {
    rec(6, &cat(&[&cell(row, 1, 0), &value, &[0; 6], &(tokens.len() as u16).to_le_bytes(), tokens]))
}
fn sample(cp: u16, more: &[u8]) -> Vec<u8> {
    let sheet = cat(&[
        &bof(0x10),
        &label(0, 0, b"Anticipo pi\xf1on"),
        &rec(0x0203, &cat(&[&cell(1, 0, 0), &1234.5f64.to_le_bytes()])),
        &formula(1, num(2469.0), &[0x44, 0x01, 0xC0, 0, 0x1E, 2, 0, 5]),
        &formula(2, special(0, 0), &[0x17, 1, 0xF1]),
        &rec(0x0207, &[1, 0, 0xF1]),
        // A defined-name token has a different layout; keep its numeric cache with a warning.
        &formula(3, num(99.0), &[0x23, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
        &rec(0x007D, &u16s(&[0, 1, 20 * 256, 0, 2, 0])),
        &rec(0x0208, &cat(&[&u16s(&[0, 0, 2, 600, 0, 0]), &0x40u32.to_le_bytes()])),
        more,
        &eof(),
    ]);
    let globals = |pos: u32| {
        cat(&[
            &bof(5),
            // Format and font deliberately precede CODEPAGE.
            &rec(0x041E, &cat(&[&164u16.to_le_bytes(), &[4], b"0.00"])),
            &rec(0x0031, &cat(&[&u16s(&[220, 2, 8, 700, 0]), &[0, 2, 0, 0, 5], b"Pi\xf1on"])),
            &rec(0x00E0, &cat(&[&u16s(&[0, 164, 1]), &[0x2A, 0], &(8u32 | (1 << 16)).to_le_bytes(), &1u32.to_le_bytes()])),
            &rec(0x0085, &cat(&[&pos.to_le_bytes(), &[0, 0, 4], b"A\xf1os"])),
            &rec(0x0042, &cp.to_le_bytes()),
            &eof(),
        ])
    };
    let pos = globals(0).len() as u32;
    cat(&[&globals(pos), &sheet])
}

#[test]
fn imports_biff5_values_formats_and_common_formulas() {
    for stream_name in ["Book", "Workbook"] {
        let imported = read(&compound(stream_name, &sample(1252, &[]))).unwrap();
        let s = &imported.workbook.sheets[0];
        assert_eq!(s.name, "Años");
        assert_eq!(v(&imported.workbook, 0, "A1"), Value::Text("Anticipo piñon".into()));
        assert_eq!(v(&imported.workbook, 0, "A2"), Value::Number(1234.5));
        assert_eq!(s.cells[&CellRef::new(1, 1)].formula.as_ref().unwrap().text, "A2*2");
        assert_eq!(v(&imported.workbook, 0, "B3"), Value::Text("ñ".into()));
        assert_eq!(s.cells[&CellRef::new(2, 1)].formula.as_ref().unwrap().text, "\"ñ\"");
        assert_eq!(v(&imported.workbook, 0, "B4"), Value::Number(99.0));
        assert!(s.cells[&CellRef::new(3, 1)].formula.is_none());
        assert!(imported.warnings.iter().any(|w| matches!(w, Warning::FormulaKept(_))));
        assert_eq!(s.columns[&0].width, 20 * 256);
        assert_eq!(s.rows[&0].height, 600);
        let format = &imported.workbook.formats[0];
        assert_eq!(format.number_format, "0.00");
        assert_eq!(format.font.name.as_deref(), Some("Piñon"));
        assert!(format.font.bold && format.font.italic);
        assert_eq!(format.fill.pattern, Pattern::Solid);
    }
}

#[test]
fn biff5_codepages_grid_and_budgets_are_enforced() {
    let bytes = compound("Book", &sample(1250, &label(0, 2, &[0xE3, 0xFE])));
    let imported = read(&bytes).unwrap();
    assert_eq!(v(&imported.workbook, 0, "C1"), Value::Text("ăţ".into()));
    assert!(matches!(read_with(&bytes, Limits { max_cells: 1, ..Limits::default() }), Err(Error::TooLarge(_))));
    assert!(matches!(read_with(&bytes, Limits { max_text: 1, ..Limits::default() }), Err(Error::TooLarge(_))));
    assert!(matches!(read(&compound("Book", &sample(932, &[]))), Err(Error::Invalid(_))));
    let imported = read(&compound("Book", &sample(1252, &label(16384, 0, b"outside")))).unwrap();
    assert!(imported.warnings.contains(&Warning::CellsOutsideGrid(1)));
    assert!(!imported.workbook.sheets[0].cells.contains_key(&CellRef::new(16384, 0)));
}

#[test]
fn biff5_damaged_records_and_encryption_are_not_misread() {
    let bytes = compound("Book", &sample(1252, &[]));
    for n in (0..bytes.len()).step_by(37) {
        let _ = read(&bytes[..n]);
    }
    let bad = compound("Book", &sample(1252, &rec(0x0204, &[0, 0, 0, 0, 0, 0, 255, 255])));
    assert!(matches!(read(&bad), Err(Error::Invalid(_))));
    let encrypted = compound("Book", &cat(&[&bof(5), &rec(0x002F, &[0; 4]), &eof()]));
    assert!(matches!(read(&encrypted), Err(Error::UnsupportedEncryption(_))));
}
