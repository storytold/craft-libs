//! A worksheet part (`CT_Worksheet`, ECMA-376 Part 1, 18.3): rows and cells as a stream, the
//! rest of the part as a tree.

use std::sync::Arc;

use craft_opc::Rels;
use craft_sheet::{CellError, CellRef, MAX_COLS, MAX_ROWS, Range, Value};
use craft_xml::{El, Writer};

use crate::features::Features;
use crate::strings::{StringItem, decode_xstring, encode_xstring};
use crate::write::{assemble, list, num, write_streamed};

/// The children of a worksheet in schema order (`CT_Worksheet`).
const WORKSHEET: &[&str] = &[
    "sheetPr",
    "dimension",
    "sheetViews",
    "sheetFormatPr",
    "cols",
    "sheetData",
    "sheetCalcPr",
    "sheetProtection",
    "protectedRanges",
    "scenarios",
    "autoFilter",
    "sortState",
    "dataConsolidate",
    "customSheetViews",
    "mergeCells",
    "phoneticPr",
    "conditionalFormatting",
    "dataValidations",
    "hyperlinks",
    "printOptions",
    "pageMargins",
    "pageSetup",
    "headerFooter",
    "rowBreaks",
    "colBreaks",
    "customProperties",
    "cellWatches",
    "ignoredErrors",
    "smartTags",
    "drawing",
    "legacyDrawing",
    "legacyDrawingHF",
    "drawingHF",
    "picture",
    "oleObjects",
    "controls",
    "webPublishItems",
    "tableParts",
    "extLst",
];

/// Row attributes read into [`Row`]'s fields; the others go to [`Row::attrs`]. `spans` is
/// dropped: it is a hint that goes stale when cells change.
const ROW_ATTRS: &[&str] = &["r", "spans", "ht", "customHeight", "hidden", "outlineLevel", "collapsed", "s", "customFormat"];

/// What a cell's `<v>` holds, by its type (`t`, 18.18.11).
#[derive(Clone, Debug, Default, PartialEq)]
pub enum CellValue {
    #[default]
    Empty,
    Number(f64),
    /// `t="s"`: an index into the shared strings.
    Shared(u32),
    /// `t="str"` (a formula's text result), `_xHHHH_` escapes decoded.
    Text(Arc<str>),
    /// `t="inlineStr"`: the `<is>` item, or the `<v>` text when there is no `<is>`.
    Inline(StringItem),
    Bool(bool),
    /// `t="e"`. An error text no [`CellError`] has reads as `#VALUE!`.
    Error(CellError),
    /// `t="d"`: an ISO 8601 date, time or both, as written. See [`iso_date_serial`].
    Date(String),
    /// A `<v>` that does not read as its type says (a number that is not one, a shared-string
    /// index that is not a number). Kept for the caller to report.
    Invalid(String),
}

impl CellValue {
    /// The value a cell shows: shared strings looked up, dates as serial numbers in the
    /// workbook's date system. A missing shared string, an unreadable date or an invalid value
    /// is empty.
    pub fn resolve(&self, strings: &[StringItem], date_1904: bool) -> Value {
        match self {
            CellValue::Empty | CellValue::Invalid(_) => Value::Empty,
            CellValue::Number(n) => Value::number(*n),
            CellValue::Shared(i) => strings.get(*i as usize).map_or(Value::Empty, |s| Value::Text(s.text.clone())),
            CellValue::Text(t) => Value::Text(t.clone()),
            CellValue::Inline(s) => Value::Text(s.text.clone()),
            CellValue::Bool(b) => Value::Bool(*b),
            CellValue::Error(e) => Value::Error(*e),
            CellValue::Date(d) => iso_date_serial(d, date_1904).map_or(Value::Empty, Value::number),
        }
    }
}

/// `<f>`'s `t` (18.18.6).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FormulaKind {
    #[default]
    Normal,
    Shared,
    Array,
    /// A what-if data table: `r1`, `r2` and its flags are in [`CellFormula::attrs`].
    DataTable,
}

/// A cell's `<f>`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CellFormula {
    pub kind: FormulaKind,
    /// The formula as the file spells it (`_xlfn.` prefixes and all), trimmed. Empty for a
    /// cell that shares the formula of its group's first cell.
    pub text: String,
    /// `si`: the shared formula group.
    pub shared_index: Option<u32>,
    /// `ref`: the range of an array formula or of a shared formula group.
    pub range: Option<Range>,
    /// Every attribute as written.
    pub attrs: Vec<(String, String)>,
}

/// One `<c>`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Cell {
    pub at: CellRef,
    /// `s`: an index into [`crate::Styles::cell_xfs`], 0 when absent.
    pub style: u32,
    pub value: CellValue,
    pub formula: Option<CellFormula>,
    /// `vm`: value metadata (a rich value such as a picture in the cell), 1-based.
    pub value_metadata: Option<u32>,
    /// `cm`: cell metadata (a dynamic array formula), 1-based.
    pub cell_metadata: Option<u32>,
    /// `ph`: show the phonetic reading.
    pub phonetic: bool,
}

/// One `<row>` and its cells.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Row {
    /// Zero-based.
    pub index: u32,
    /// `ht`, points.
    pub height: Option<f64>,
    pub custom_height: bool,
    pub hidden: bool,
    pub outline_level: u8,
    pub collapsed: bool,
    /// `s` when `customFormat` is set: the format of the row's empty cells.
    pub style: Option<u32>,
    pub cells: Vec<Cell>,
    /// The other attributes as written (`thickBot`, `x14ac:dyDescent`).
    pub attrs: Vec<(String, String)>,
}

/// One `<col>`: columns `first..=last` (zero-based).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Column {
    pub first: u32,
    pub last: u32,
    /// Characters of the default font's maximum digit width, padding included.
    pub width: Option<f64>,
    pub custom_width: bool,
    pub best_fit: bool,
    pub hidden: bool,
    pub outline_level: u8,
    pub collapsed: bool,
    pub style: Option<u32>,
}

/// A worksheet after its rows were handed out: the rest of the part and what was lifted out of it.
#[derive(Clone, Debug, Default)]
pub struct SheetPart {
    pub part: String,
    /// The worksheet element without its rows: properties, views, columns, merged cells,
    /// conditional formats, validation, hyperlinks, page setup, extensions. Empty for a binary
    /// worksheet.
    pub root: El,
    pub rels: Rels,
    pub columns: Vec<Column>,
    pub merges: Vec<Range>,
    /// Rows read.
    pub rows: u64,
    /// Rows and cells dropped because their position was outside the grid or unreadable.
    pub dropped: u64,
    /// Why reading stopped early, when the part is damaged; the rows before the damage were
    /// handed out.
    pub damaged: Option<String>,
    /// Views, print setup, data validation, conditional formats, hyperlinks: written from here,
    /// not from [`SheetPart::root`].
    pub features: Features,
}

impl SheetPart {
    pub(crate) fn lift(&mut self) {
        if let Some(cols) = self.root.child("cols") {
            for c in cols.children("col") {
                let (Some(min), Some(max)) = (c.attr_u32("min"), c.attr_u32("max")) else { continue };
                if min == 0 || min > MAX_COLS || max < min {
                    continue;
                }
                self.columns.push(Column {
                    first: min - 1,
                    last: max.min(MAX_COLS) - 1,
                    width: c.attr_f64("width"),
                    custom_width: c.flag("customWidth", false),
                    best_fit: c.flag("bestFit", false),
                    hidden: c.flag("hidden", false),
                    outline_level: c.attr_u32("outlineLevel").unwrap_or(0).min(7) as u8,
                    collapsed: c.flag("collapsed", false),
                    style: c.attr_u32("style"),
                });
            }
        }
        self.features = Features::read(&self.root, &self.rels);
        if let Some(m) = self.root.child("mergeCells") {
            self.merges.extend(m.children("mergeCell").filter_map(|m| m.attr("ref").and_then(Range::parse)).filter(|r| !r.is_single()));
        }
    }
}

impl SheetPart {
    /// The worksheet part with `rows` as its `sheetData`. The dimension, the columns, the
    /// merged cells and [`SheetPart::features`] come from the fields; everything else in [`SheetPart::root`] is written as
    /// kept, in place.
    pub fn to_xml(&self, rows: &[Row]) -> Vec<u8> {
        let root = if self.root.name.is_empty() { El::new("worksheet") } else { self.root.clone() };
        let mut cols = El::new("cols");
        for c in &self.columns {
            cols.push(column_el(c));
        }
        let merges = self.merges.iter().map(|m| El::new("mergeCell").with_attr("ref", m.a1())).collect();
        let mut owned = vec![
            ("dimension", vec![El::new("dimension").with_attr("ref", used(rows).map_or_else(|| "A1".to_string(), |r| r.a1()))]),
            ("cols", Vec::from_iter((!self.columns.is_empty()).then_some(cols))),
            ("sheetData", vec![El::new("sheetData")]),
            ("mergeCells", Vec::from_iter(list(&root, "mergeCells", merges))),
        ];
        owned.extend(self.features.owned());
        let mut root = assemble(&root, WORKSHEET, owned);
        let prefixes: Vec<&str> = rows.iter().flat_map(|r| &r.attrs).filter_map(|(k, _)| k.split_once(':').map(|(p, _)| p)).collect();
        crate::ns::declare_with(&mut root, &prefixes);
        write_streamed(&root, "sheetData", |w| rows.iter().for_each(|r| write_row(w, r)))
    }
}

/// The range from the first to the last cell of `rows`.
fn used(rows: &[Row]) -> Option<Range> {
    let mut cells = rows.iter().flat_map(|r| &r.cells).map(|c| c.at);
    let first = cells.next()?;
    let (lo, hi) = cells
        .fold((first, first), |(lo, hi), c| (CellRef::new(lo.row.min(c.row), lo.col.min(c.col)), CellRef::new(hi.row.max(c.row), hi.col.max(c.col))));
    Some(Range::new(lo, hi))
}

fn column_el(c: &Column) -> El {
    let mut e = El::new("col").with_attr("min", c.first + 1).with_attr("max", c.last + 1);
    let mut put = |k: &str, v: Option<String>| {
        if let Some(v) = v {
            e.set_attr(k, v);
        }
    };
    put("width", c.width.map(num));
    put("style", c.style.map(|s| s.to_string()));
    put("hidden", c.hidden.then(|| "1".into()));
    put("bestFit", c.best_fit.then(|| "1".into()));
    put("customWidth", c.custom_width.then(|| "1".into()));
    put("outlineLevel", (c.outline_level > 0).then(|| c.outline_level.to_string()));
    put("collapsed", c.collapsed.then(|| "1".into()));
    e
}

fn write_row(w: &mut Writer, row: &Row) {
    let mut t = w.tag("row").attr("r", row.index + 1);
    if let Some(s) = row.style {
        t = t.attr("s", s).attr("customFormat", 1);
    }
    t = t.attr_opt("ht", row.height.map(num));
    if row.custom_height {
        t = t.attr("customHeight", 1);
    }
    if row.hidden {
        t = t.attr("hidden", 1);
    }
    if row.outline_level > 0 {
        t = t.attr("outlineLevel", row.outline_level);
    }
    if row.collapsed {
        t = t.attr("collapsed", 1);
    }
    for (k, v) in row.attrs.iter().filter(|(k, _)| craft_xml::is_name(k)) {
        t = t.attr(k, v);
    }
    if row.cells.is_empty() {
        t.empty();
        return;
    }
    t.open();
    for c in &row.cells {
        write_cell(w, c);
    }
    w.close();
}

fn write_cell(w: &mut Writer, c: &Cell) {
    let (kind, v): (Option<&str>, Option<String>) = match &c.value {
        CellValue::Empty => (None, None),
        CellValue::Number(n) => (None, Some(num(*n))),
        CellValue::Shared(i) => (Some("s"), Some(i.to_string())),
        // An unreadable value is kept as the text it was.
        CellValue::Text(t) => (Some("str"), Some(encode_xstring(t))),
        CellValue::Invalid(t) => (Some("str"), Some(encode_xstring(t))),
        CellValue::Inline(_) => (Some("inlineStr"), None),
        CellValue::Bool(b) => (Some("b"), Some(u8::from(*b).to_string())),
        CellValue::Error(e) => (Some("e"), Some(e.as_str().to_string())),
        CellValue::Date(d) => (Some("d"), Some(d.clone())),
    };
    let mut t = w.tag("c").attr("r", c.at.a1());
    if c.style != 0 {
        t = t.attr("s", c.style);
    }
    let t = t.attr_opt("t", kind).attr_opt("cm", c.cell_metadata).attr_opt("vm", c.value_metadata);
    let t = if c.phonetic { t.attr("ph", 1) } else { t };
    if c.formula.is_none() && v.is_none() && !matches!(c.value, CellValue::Inline(_)) {
        t.empty();
        return;
    }
    t.open();
    if let Some(f) = &c.formula {
        let kind = match f.kind {
            FormulaKind::Normal => None,
            FormulaKind::Shared => Some("shared"),
            FormulaKind::Array => Some("array"),
            FormulaKind::DataTable => Some("dataTable"),
        };
        let mut t = w.tag("f").attr_opt("t", kind).attr_opt("ref", f.range.map(|r| r.a1())).attr_opt("si", f.shared_index);
        for (k, v) in f.attrs.iter().filter(|(k, _)| !matches!(k.as_str(), "t" | "ref" | "si") && craft_xml::is_name(k)) {
            t = t.attr(k, v);
        }
        if f.text.is_empty() { t.empty() } else { t.text(&f.text) }
    }
    if let CellValue::Inline(s) = &c.value {
        w.tree(&s.to_el("is"));
    }
    if let Some(v) = v {
        w.tag("v").text(&v);
    }
    w.close();
}

/// Reads rows one at a time, remembering where the last one ended.
#[derive(Default)]
pub(crate) struct RowReader {
    next_row: u32,
    pub(crate) rows: u64,
    pub(crate) dropped: u64,
}

impl RowReader {
    /// `None` for a row outside the grid (counted in `dropped`).
    pub(crate) fn row(&mut self, e: &El) -> Option<Row> {
        self.rows += 1;
        let index = match e.attr_u32("r") {
            Some(r) if (1..=MAX_ROWS).contains(&r) => r - 1,
            Some(_) => {
                self.dropped += 1;
                return None;
            }
            None if self.next_row < MAX_ROWS => self.next_row,
            None => {
                self.dropped += 1;
                return None;
            }
        };
        self.next_row = index + 1;
        let mut row = Row {
            index,
            height: e.attr_f64("ht"),
            custom_height: e.flag("customHeight", false),
            hidden: e.flag("hidden", false),
            outline_level: e.attr_u32("outlineLevel").unwrap_or(0).min(7) as u8,
            collapsed: e.flag("collapsed", false),
            style: if e.flag("customFormat", false) { e.attr_u32("s") } else { None },
            cells: Vec::new(),
            attrs: e.attrs.iter().filter(|(k, _)| !ROW_ATTRS.contains(&k.as_str())).cloned().collect(),
        };
        let mut next_col = 0u32;
        for c in e.children("c") {
            let at = match c.attr("r") {
                Some(a) => match CellRef::parse(a.trim()) {
                    Some(p) => p,
                    None => {
                        self.dropped += 1;
                        continue;
                    }
                },
                None if next_col < MAX_COLS => CellRef::new(index, next_col),
                None => {
                    self.dropped += 1;
                    continue;
                }
            };
            next_col = at.col + 1;
            row.cells.push(cell(c, at));
        }
        Some(row)
    }
}

fn cell(c: &El, at: CellRef) -> Cell {
    let v = c.child("v").map(|v| v.text());
    let v = v.as_deref();
    let value = match c.attr("t").unwrap_or("n") {
        "s" => match v.map(str::trim) {
            Some(t) => t.parse().map_or_else(|_| CellValue::Invalid(t.to_string()), CellValue::Shared),
            None => CellValue::Empty,
        },
        "str" => v.map_or(CellValue::Empty, |v| CellValue::Text(decode_xstring(v).into())),
        "inlineStr" => match c.child("is") {
            Some(is) => CellValue::Inline(StringItem::read(is)),
            None => v.map_or(CellValue::Empty, |v| CellValue::Text(decode_xstring(v).into())),
        },
        "b" => v.map_or(CellValue::Empty, |v| CellValue::Bool(matches!(v.trim(), "1" | "true" | "TRUE"))),
        "e" => v.map_or(CellValue::Empty, |v| CellValue::Error(CellError::parse(v.trim()).unwrap_or(CellError::Value))),
        "d" => match v.map(str::trim).filter(|v| !v.is_empty()) {
            Some(d) => CellValue::Date(d.to_string()),
            None => CellValue::Empty,
        },
        _ => match v.map(str::trim).filter(|v| !v.is_empty()) {
            Some(t) => match t.parse::<f64>() {
                Ok(n) if n.is_finite() => CellValue::Number(if n == 0.0 { 0.0 } else { n }),
                _ => CellValue::Invalid(t.to_string()),
            },
            None => CellValue::Empty,
        },
    };
    let formula = c.child("f").map(|f| CellFormula {
        kind: match f.attr("t") {
            Some("shared") => FormulaKind::Shared,
            Some("array") => FormulaKind::Array,
            Some("dataTable") => FormulaKind::DataTable,
            _ => FormulaKind::Normal,
        },
        text: f.text().trim().to_string(),
        shared_index: f.attr_u32("si"),
        range: f.attr("ref").and_then(Range::parse),
        attrs: f.attrs.clone(),
    });
    Cell {
        at,
        style: c.attr_u32("s").unwrap_or(0),
        value,
        formula,
        value_metadata: c.attr_u32("vm"),
        cell_metadata: c.attr_u32("cm"),
        phonetic: c.flag("ph", false),
    }
}

/// Days from 1970-01-01 to a proleptic Gregorian date (the civil-from-days inverse).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The serial number of a `t="d"` value: `2024-02-29`, `2024-02-29T13:45:30`, `13:45` (a time
/// alone), a trailing `Z` or `+hh:mm` ignored. Years 1900 (1904 in the 1904 system) to 9999;
/// a day past the month's end counts on into the next month, as Excel's `DATE` does. In the
/// 1900 system, dates before 1 March 1900 are one lower than the day count, for the 29 February
/// 1900 that Excel counts.
pub fn iso_date_serial(s: &str, date_1904: bool) -> Option<f64> {
    let s = s.trim().trim_end_matches('Z');
    let (date, time) = match s.split_once('T') {
        Some((d, t)) => (d, Some(t)),
        None if s.contains(':') => ("", Some(s)),
        None => (s, None),
    };
    let mut serial = 0.0;
    if !date.is_empty() {
        let mut it = date.split('-');
        let y: i64 = it.next()?.parse().ok()?;
        let m: i64 = it.next()?.parse().ok()?;
        let d: i64 = it.next()?.parse().ok()?;
        if it.next().is_some() || !(1..=12).contains(&m) || !(1..=31).contains(&d) || y > 9999 || y < if date_1904 { 1904 } else { 1900 } {
            return None;
        }
        let first = days_from_civil(y, m, 1);
        let first =
            if date_1904 { first - days_from_civil(1904, 1, 1) } else { first - days_from_civil(1899, 12, 30) - i64::from(y == 1900 && m <= 2) };
        let n = first + d - 1;
        let max = days_from_civil(9999, 12, 31) - if date_1904 { days_from_civil(1904, 1, 1) } else { days_from_civil(1899, 12, 30) };
        if !(0..=max).contains(&n) {
            return None;
        }
        serial = n as f64;
    }
    if let Some(t) = time {
        let t = t.split('+').next().unwrap_or(t);
        let mut it = t.split(':');
        let h: f64 = it.next()?.parse().ok()?;
        let mi: f64 = it.next().unwrap_or("0").parse().ok()?;
        let se: f64 = it.next().unwrap_or("0").parse().ok()?;
        if !(0.0..24.0).contains(&h) || !(0.0..60.0).contains(&mi) || !(0.0..61.0).contains(&se) {
            return None;
        }
        serial += (h * 3600.0 + mi * 60.0 + se) / 86400.0;
    }
    Some(serial)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_dates() {
        assert_eq!(iso_date_serial("1900-01-01", false), Some(1.0));
        assert_eq!(iso_date_serial("1900-02-28", false), Some(59.0));
        assert_eq!(iso_date_serial("1900-03-01", false), Some(61.0));
        assert_eq!(iso_date_serial("2024-02-29T12:00:00Z", false), Some(45351.5));
        assert_eq!(iso_date_serial("1904-01-02", true), Some(1.0));
        assert_eq!(iso_date_serial("06:00", false), Some(0.25));
        assert_eq!(iso_date_serial("2023-02-31", false), iso_date_serial("2023-03-03", false));
        assert_eq!(iso_date_serial("9999-12-31", false), Some(2_958_465.0));
        for bad in ["1899-12-31", "10000-01-01", "2024-13-01", "2024-01-00", "x", "2024-01-01T25:00", "2024-01-01-01"] {
            assert_eq!(iso_date_serial(bad, false), None, "{bad}");
        }
        assert_eq!(iso_date_serial("1903-12-31", true), None);
    }

    #[test]
    fn rows_and_cells() {
        let xml = r#"<row r="3" ht="20" customHeight="1" s="4" customFormat="1" outlineLevel="9"><c r="B3" s="2" t="s"><v>7</v></c><c><f t="shared" ref="C3:C9" si="0">A1*2</f><v>4</v></c>
            <c r="XFE3"><v>1</v></c><c r="E3" t="e"><v>#SPILL!</v></c><c r="F3" t="e"><v>#WHAT</v></c><c t="b"><v>1</v></c><c t="inlineStr"><is><t>in_x000A_line</t></is></c><c t="n"><v>abc</v></c><c t="str"><v>a_x0009_</v></c><c t="d"><v>2024-01-01</v></c><c vm="1" cm="2" ph="1"/></row>"#;
        let mut r = RowReader::default();
        let row = r.row(&craft_xml::Options::local().parse(xml.as_bytes()).unwrap_or_default()).unwrap_or_default();
        assert_eq!((row.index, row.height, row.custom_height, row.style, row.outline_level), (2, Some(20.0), true, Some(4), 7));
        assert_eq!(r.dropped, 1);
        let at: Vec<String> = row.cells.iter().map(|c| c.at.a1()).collect();
        assert_eq!(at, ["B3", "C3", "E3", "F3", "G3", "H3", "I3", "J3", "K3", "L3"]);
        let values: Vec<&CellValue> = row.cells.iter().map(|c| &c.value).collect();
        assert_eq!(values[0], &CellValue::Shared(7));
        assert_eq!(values[2], &CellValue::Error(CellError::Spill));
        assert_eq!(values[3], &CellValue::Error(CellError::Value));
        assert_eq!(values[4], &CellValue::Bool(true));
        assert!(matches!(values[5], CellValue::Inline(s) if &*s.text == "in\nline"));
        assert_eq!(values[6], &CellValue::Invalid("abc".into()));
        assert_eq!(values[7], &CellValue::Text("a\t".into()));
        assert_eq!(values[8].resolve(&[], false), Value::Number(45292.0));
        let f = row.cells[1].formula.clone().unwrap_or_default();
        assert_eq!((f.kind, f.text.as_str(), f.shared_index, f.range.map(|r| r.a1())), (FormulaKind::Shared, "A1*2", Some(0), Some("C3:C9".into())));
        let last = &row.cells[9];
        assert_eq!((last.value_metadata, last.cell_metadata, last.phonetic, last.style), (Some(1), Some(2), true, 0));
        assert_eq!(row.cells[0].value.resolve(&[], false), Value::Empty);
        // Without `r`, the next row follows the last one.
        let next = r.row(&craft_xml::El::new("row")).map(|r| r.index);
        assert_eq!(next, Some(3));
        assert!(r.row(&craft_xml::El::new("row").with_attr("r", "1048577")).is_none());
        assert!(r.row(&craft_xml::El::new("row").with_attr("r", "0")).is_none());
        assert_eq!((r.rows, r.dropped), (4, 3));
    }

    #[test]
    fn rows_write_back() {
        let xml = r#"<row r="3" spans="1:9" ht="20" customHeight="1" s="4" customFormat="1" outlineLevel="2" collapsed="1" hidden="1" thickBot="1"><c r="B3" s="2" t="s"><v>7</v></c><c r="C3"><f t="shared" ref="C3:C9" si="0" aca="1">A1*2</f><v>4</v></c><c r="D3"><f t="shared" si="0"/><v>0.1</v></c><c r="E3" t="e"><v>#SPILL!</v></c><c r="G3" t="b"><v>0</v></c><c r="H3" t="inlineStr"><is><t xml:space="preserve">in_x000A_line </t></is></c><c r="J3" t="str"><f>"a"&amp;"b"</f><v>a_x0009_b</v></c><c r="K3" t="d"><v>2024-01-01</v></c><c r="L3" s="1" cm="1" vm="2" ph="1"/></row>"#;
        let read = |xml: &str| RowReader::default().row(&craft_xml::Options::local().parse(xml.as_bytes()).unwrap_or_default()).unwrap_or_default();
        let row = read(xml);
        let mut w = Writer::new();
        write_row(&mut w, &row);
        let out = w.finish();
        assert!(out.starts_with(r#"<row r="3" s="4" customFormat="1" ht="20" customHeight="1" hidden="1" outlineLevel="2" collapsed="1" thickBot="1"><c r="B3" s="2" t="s"><v>7</v></c><c r="C3"><f t="shared" ref="C3:C9" si="0" aca="1">A1*2</f>"#), "{out}");
        assert_eq!(read(&out), row);
        let empty = Row { index: 0, ..Row::default() };
        let mut w = Writer::new();
        write_row(&mut w, &empty);
        assert_eq!(w.finish(), r#"<row r="1"/>"#);
    }
}
