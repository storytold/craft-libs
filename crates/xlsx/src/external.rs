//! External links (`xl/externalLinks/externalLinkN.xml`, ECMA-376 Part 1, 18.14): the other
//! workbooks a workbook's formulas refer to as `[1]Sheet1!A1`, numbered in the order of the
//! workbook's `externalReferences` ([`crate::Workbook::external_references`]). A binary workbook's
//! `externalLinkN.bin` reads into the same types.

use craft_opc::Rels;
use craft_sheet::{CellError, CellRef, Value};
use craft_xml::El;

/// One external link part.
#[derive(Clone, Debug, PartialEq)]
pub enum ExternalLink {
    /// `externalBook`: another workbook.
    Book(ExternalBook),
    /// `ddeLink` or `oleLink`: a DDE or OLE source, not read.
    Other,
}

/// `externalBook`: another workbook, what its formulas use of it, and the values last read from it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ExternalBook {
    /// The other workbook as its relationship's target is written (`Src%20One.xlsx`,
    /// `sub/three.xlsx`, a URL); `None` without one.
    pub target: Option<String>,
    /// `sheetNames`: the other workbook's sheets, which `sheet_id`s index.
    pub sheet_names: Vec<String>,
    pub defined_names: Vec<ExternalName>,
    /// `sheetDataSet`: the cached cells, one entry per cached sheet.
    pub sheet_data: Vec<ExternalSheetData>,
}

/// `definedName` in an external book: a name in the other workbook that formulas here use.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExternalName {
    pub name: String,
    /// `refersTo` as written (`='Data'!$A$1:$A$3`, `#REF!`); `None` when the other workbook didn't
    /// define it.
    pub refers_to: Option<String>,
    /// `sheetId`: the sheet it is scoped to, an index into [`ExternalBook::sheet_names`].
    pub sheet_id: Option<u32>,
}

/// `sheetData` in an external book: the cached cells of one sheet.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ExternalSheetData {
    /// An index into [`ExternalBook::sheet_names`].
    pub sheet_id: u32,
    /// `refreshError`: the last update of this sheet's cells failed.
    pub refresh_error: bool,
    /// The cells, in file order: `Value::Empty` for a cell with no value.
    pub cells: Vec<(CellRef, Value)>,
}

impl ExternalLink {
    /// An external link part read with [`crate::ns::CANONICAL`], with its relationships (for the
    /// target).
    pub fn read(root: &El, rels: &Rels) -> ExternalLink {
        let Some(book) = root.child("externalBook") else { return ExternalLink::Other };
        let target = book.attr_or_plain("r:id").and_then(|id| rels.by_id(id)).map(|r| r.target_raw.clone());
        let sheet_names = book
            .child("sheetNames")
            .map(|s| s.children("sheetName").map(|n| n.attr("val").unwrap_or_default().to_string()).collect())
            .unwrap_or_default();
        let defined_names = book
            .child("definedNames")
            .map(|d| {
                d.children("definedName")
                    .map(|n| ExternalName {
                        name: n.attr("name").unwrap_or_default().to_string(),
                        refers_to: n.attr("refersTo").map(str::to_string),
                        sheet_id: n.attr_u32("sheetId"),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let sheet_data = book
            .child("sheetDataSet")
            .map(|set| {
                set.children("sheetData")
                    .map(|d| ExternalSheetData {
                        sheet_id: d.attr_u32("sheetId").unwrap_or(0),
                        refresh_error: d.flag("refreshError", false),
                        cells: d.children("row").flat_map(|r| r.children("cell")).filter_map(cell).collect(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        ExternalLink::Book(ExternalBook { target, sheet_names, defined_names, sheet_data })
    }
}

/// A cached `cell`: its value by `t` (a number by default; `str`, `b`, `e`). One without a
/// readable reference is skipped.
fn cell(c: &El) -> Option<(CellRef, Value)> {
    let at = CellRef::parse(c.attr("r")?)?;
    let Some(v) = c.child("v").map(|v| v.text().into_owned()) else { return Some((at, Value::Empty)) };
    let value = match c.attr("t") {
        Some("str" | "s" | "inlineStr") => Value::Text(v.into()),
        Some("b") => Value::Bool(v.trim() == "1" || v.trim().eq_ignore_ascii_case("true")),
        Some("e") => CellError::parse(v.trim()).map_or(Value::Text(v.into()), Value::Error),
        _ => v.trim().parse::<f64>().map_or(Value::Text(v.into()), Value::number),
    };
    Some((at, value))
}
