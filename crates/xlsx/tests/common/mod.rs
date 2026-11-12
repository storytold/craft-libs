//! A whole workbook read through the part layer, for assertions.
#![allow(dead_code)]

use std::collections::BTreeMap;

use craft_xlsx::{CellRef, Error, Limits, Package, Range, SheetKind, Value, Visibility};

pub struct Sheet {
    pub name: String,
    pub visibility: Visibility,
    pub cells: BTreeMap<CellRef, Value>,
    pub merges: Vec<Range>,
}

impl Sheet {
    pub fn value(&self, at: CellRef) -> Value {
        self.cells.get(&at).cloned().unwrap_or_default()
    }
}

pub struct Book {
    pub date_1904: bool,
    pub sheets: Vec<Sheet>,
    pub skipped: Vec<(String, SheetKind)>,
    pub warnings: Vec<String>,
}

/// Reads every worksheet; cells with no value are left out.
pub fn read_with(bytes: &[u8], limits: &Limits) -> Result<Book, Error> {
    let mut p = Package::open(bytes, limits)?;
    let wb = p.workbook()?;
    let strings = p.shared_strings()?;
    let mut book = Book { date_1904: wb.date_1904, sheets: vec![], skipped: vec![], warnings: vec![] };
    for e in &wb.sheets {
        if e.kind != SheetKind::Worksheet {
            book.skipped.push((e.name.clone(), e.kind.clone()));
            continue;
        }
        let mut cells = BTreeMap::new();
        let part = p.sheet(e, |row| {
            for c in row.cells {
                let v = c.value.resolve(&strings, wb.date_1904);
                if !v.is_empty() {
                    cells.insert(c.at, v);
                }
            }
            Ok(())
        })?;
        book.sheets.push(Sheet { name: e.name.clone(), visibility: e.visibility, cells, merges: part.merges });
    }
    book.warnings = p.warnings().to_vec();
    Ok(book)
}

pub fn read(bytes: &[u8]) -> Result<Book, Error> {
    read_with(bytes, &Limits::default())
}
