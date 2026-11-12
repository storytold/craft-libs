//! SYLK reading. The files are written here, the way spreadsheets export them.

use craft_csv::sylk::{ColumnWidth, Error, Value, r1c1_formula_to_a1, read};

/// `B3` → (row, column), zero-based.
fn at(a1: &str) -> (u32, u32) {
    let col = u32::from(a1.as_bytes()[0] - b'A');
    (a1[1..].parse::<u32>().unwrap() - 1, col)
}

/// A small SYLK file: a header, number formats, column widths, cells that carry on from the last
/// row or column, formulas in R1C1 and a shared formula.
const SYLK: &str = "ID;PTEST;N;E\r\nP;PGeneral\r\nP;P0.00\r\nP;Pyyyy\\-mm\\-dd\r\nF;W1 1 18\r\nF;W2 3 9\r\nB;Y5;X7\r\n\
C;Y1;X1;K\"Item\"\r\nC;X2;K\"Qty\"\r\nC;X4;K\"Total\"\r\n\
C;Y2;X1;K\"Bolts;; small\"\r\nC;X2;K12\r\nF;P1;F$2G;X3\r\nC;X3;K0.35\r\nC;X4;K4.2;ERC[-2]*RC[-1]\r\n\
C;X5;K52;ER2C2+R[1]C[-3]\r\nC;X6;KTRUE\r\nC;X7;K8.4;ERC[-3]*2\r\n\
C;Y3;X1;K\"Nuts \"\"hex\"\"\"\r\nC;X2;K40\r\nC;X3;K0.12\r\nC;X4;K4.8;ERC[-2]*RC[-1]\r\nC;X5;K\"big\";EIF(R4C4>5,\"big\",\"small\")\r\nC;X7;K9.6;S;R2;C7\r\n\
C;Y4;X1;K\"Sum\"\r\nC;X4;K9;ESUM(R[-2]C:R[-1]C)\r\nC;X5;K#DIV/0!;E1/0\r\nC;X7;K18;ESUM(C4)\r\n\
F;P2;FG0G;Y5;X2\r\nC;Y5;X2;K45000\r\nC;X3;K\"R2C3\";E\"R2C3\"\r\nE\r\n";

#[test]
fn reads_values_formulas_formats_and_widths() {
    let sh = read(SYLK.as_bytes()).unwrap();
    assert_eq!(sh.skipped, 0);
    let value = |a: &str| sh.cells.get(&at(a)).map(|c| c.value.clone()).unwrap_or_default();
    let formula = |a: &str| sh.cells.get(&at(a)).and_then(|c| c.formula.clone());
    let text = |s: &str| Value::Text(s.into());
    assert_eq!(value("A1"), text("Item"));
    assert_eq!(value("A2"), text("Bolts; small"), ";; is a semicolon");
    assert_eq!(value("A3"), text("Nuts \"\"hex\"\""), "text is kept between the outer quotes as written");
    assert_eq!(value("B2"), Value::Number(12.0));
    assert_eq!(value("F2"), Value::Bool(true));
    assert_eq!(value("E4"), Value::Error("#DIV/0!".into()));
    assert_eq!(value("D4"), Value::Number(9.0), "a formula keeps the value the file cached");
    assert_eq!(formula("D2").as_deref(), Some("B2*C2"));
    assert_eq!(formula("E2").as_deref(), Some("$B$2+B3"));
    assert_eq!(formula("E3").as_deref(), Some("IF($D$4>5,\"big\",\"small\")"));
    assert_eq!(formula("D4").as_deref(), Some("SUM(D2:D3)"));
    assert_eq!(formula("G3").as_deref(), Some("D3*2"), "a shared formula is the other cell's, moved");
    assert_eq!(formula("G4").as_deref(), Some("SUM($D:$D)"), "C4 alone is the whole column");
    assert_eq!(formula("C5").as_deref(), Some("\"R2C3\""), "text in quotes isn't a reference");
    assert_eq!(formula("B2"), None);
    // `F;P1;X3` formats the cell on the current row only.
    let format = |a: &str| sh.cells.get(&at(a)).and_then(|c| c.format.clone());
    assert_eq!(format("C2").as_deref(), Some("0.00"));
    assert_eq!(format("C3"), None);
    assert_eq!(format("B5").as_deref(), Some("yyyy\\-mm\\-dd"));
    // Widths are in characters; column D has none.
    assert_eq!(sh.column_widths, [ColumnWidth { first: 0, last: 0, width: 18.0 }, ColumnWidth { first: 1, last: 2, width: 9.0 }]);
}

#[test]
fn r1c1_formulas_become_a1() {
    for (r1c1, a1) in [
        ("RC", "B2"),
        ("R[1]C[-1]", "A3"),
        ("R1C1:R2C2", "$A$1:$B$2"),
        ("R3", "$3:$3"),
        ("R1:R[2]", "$1:4"),
        ("C[1]", "C:C"),
        ("SUM(C3:C4)", "SUM($C:$D)"),
        ("ROUND(RC[1],2)+RATE(1,2,3)", "ROUND(C2,2)+RATE(1,2,3)"),
        ("'R1C1 sheet'!R1C1", "'R1C1 sheet'!$A$1"),
        ("Sheet2!RC", "Sheet2!B2"),
        ("R2D2+rc", "R2D2+B2"),
        ("R[-5]C", "#REF!"),
        ("\"RC\"&R1C1", "\"RC\"&$A$1"),
        ("RC[26]+RC16384", "AB2+$XFD2"),
        ("RC16385", "#REF!"),
    ] {
        assert_eq!(r1c1_formula_to_a1(r1c1, at("B2")), a1, "{r1c1}");
    }
}

#[test]
fn rejects_other_files_and_survives_damage() {
    assert_eq!(read(b"A,B\n1,2\n"), Err(Error::NotSylk), "a SYLK file starts with ID");
    assert_eq!(read(b""), Err(Error::NotSylk));
    // A byte order mark and a single-byte code page are accepted.
    assert_eq!(read("\u{feff}ID;P\nC;Y1;X1;K\"café\"\n".as_bytes()).unwrap().cells[&(0, 0)].value, Value::Text("café".into()));
    assert_eq!(read(b"ID;P\nC;Y1;X1;K\"caf\xE9\"\n").unwrap().cells[&(0, 0)].value, Value::Text("café".into()));
    // Positions off the sheet, broken numbers, a missing end, a format past the list and a
    // formula that is no formula (kept as text for the caller to judge).
    let sh = read(b"ID;P\nC;Y0;X1;K1\nC;Y2;X99999;K2\nC;Y3;X1;K1e999\nF;P7;Y3;X1\nF;W5 2 9\nF;W1 2 256\nC;Y4;X1;K5;E(((\n").unwrap();
    assert_eq!(sh.cells.keys().copied().collect::<Vec<_>>(), [(2, 0), (3, 0)]);
    assert_eq!(sh.cells[&(2, 0)].value, Value::Text("1e999".into()));
    assert_eq!((sh.cells[&(3, 0)].value.clone(), sh.cells[&(3, 0)].formula.as_deref()), (Value::Number(5.0), Some("(((")));
    assert_eq!(sh.skipped, 2, "the two cells off the sheet");
    assert!(sh.column_widths.is_empty());
}

#[test]
fn survives_huge_relative_offsets_and_width_floods() {
    // Relative offsets at the edge of i64 fall off the sheet instead of overflowing.
    let text = b"ID;P\nC;Y1;X1;K0;ER[9223372036854775807]C\nC;Y2;X1;K0;ER[-9223372036854775808]C[-9223372036854775808]\nE\n";
    let sh = read(text).unwrap();
    assert_eq!(sh.cells[&(0, 0)].formula.as_deref(), Some("#REF!"));
    // Width records over every column, many times over: a bounded number is kept.
    let mut flood = b"ID;P\n".to_vec();
    for _ in 0..5000 {
        flood.extend_from_slice(b"F;W1 16384 12\n");
    }
    flood.extend_from_slice(b"C;Y1;X1;K1\nE\n");
    let sh = read(&flood).unwrap();
    assert_eq!(sh.column_widths.len(), 1024);
    assert_eq!(sh.cells[&(0, 0)].value, Value::Number(1.0));
    // Truncation and byte damage anywhere never panic.
    let good = SYLK.as_bytes().to_vec();
    for cut in 0..good.len() {
        let _ = read(&good[..cut]);
        let mut b = good.clone();
        b[cut] = b'[';
        let _ = read(&b);
        b[cut] = 0xFF;
        let _ = read(&b);
    }
}
