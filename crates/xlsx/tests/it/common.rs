//! A whole workbook read through the part layer, for assertions.
#![allow(dead_code)]

use std::collections::BTreeMap;

use craft_xlsx::{CellFormula, CellRef, Error, Limits, Package, Range, SheetKind, Value, Visibility};

pub struct Sheet {
    pub name: String,
    pub visibility: Visibility,
    pub cells: BTreeMap<CellRef, Value>,
    /// Every cell's formula, and its cell metadata index (`cm`), where it has one.
    pub formulas: BTreeMap<CellRef, CellFormula>,
    pub cell_metadata: BTreeMap<CellRef, u32>,
    pub merges: Vec<Range>,
}

impl Sheet {
    pub fn value(&self, at: CellRef) -> Value {
        self.cells.get(&at).cloned().unwrap_or_default()
    }

    /// The formula text at `at`, if the cell has a formula.
    pub fn formula(&self, at: CellRef) -> Option<&str> {
        self.formulas.get(&at).map(|f| f.text.as_str())
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
        let (mut formulas, mut cell_metadata) = (BTreeMap::new(), BTreeMap::new());
        let part = p.sheet(e, |row| {
            for c in row.cells {
                if let Some(f) = c.formula {
                    formulas.insert(c.at, f);
                }
                if let Some(m) = c.cell_metadata {
                    cell_metadata.insert(c.at, m);
                }
                let v = c.value.resolve(&strings, wb.date_1904);
                if !v.is_empty() {
                    cells.insert(c.at, v);
                }
            }
            Ok(())
        })?;
        book.sheets.push(Sheet { name: e.name.clone(), visibility: e.visibility, cells, formulas, cell_metadata, merges: part.merges });
    }
    book.warnings = p.warnings().to_vec();
    Ok(book)
}

pub fn read(bytes: &[u8]) -> Result<Book, Error> {
    read_with(bytes, &Limits::default())
}
