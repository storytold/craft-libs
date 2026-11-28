//! What a spreadsheet reads into: the file's own cells, nothing interpreted beyond the format.

use std::collections::BTreeMap;

use craft_sheet::{CellRef, Range, Value};
use craft_xml::El;

/// `office:value-type` (ODF 1.3 Part 3, 19.389), and `calcext:value-type="error"`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueType {
    Float,
    Percentage,
    Currency,
    Date,
    Time,
    Boolean,
    String,
    Void,
    /// LibreOffice's error result of a formula; the error text is the cell's text.
    Error,
}

impl ValueType {
    pub(crate) fn parse(s: &str) -> Option<ValueType> {
        Some(match s {
            "float" => ValueType::Float,
            "percentage" => ValueType::Percentage,
            "currency" => ValueType::Currency,
            "date" => ValueType::Date,
            "time" => ValueType::Time,
            "boolean" => ValueType::Boolean,
            "string" => ValueType::String,
            "void" => ValueType::Void,
            "error" => ValueType::Error,
            _ => return None,
        })
    }
}

/// One `table:table-cell` or `table:covered-table-cell`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Cell {
    /// `None` when the cell has no value type (or one ODF does not define).
    pub value_type: Option<ValueType>,
    /// The attribute that holds the value for the type, as written: `office:value`,
    /// `office:date-value`, `office:time-value`, `office:boolean-value` or `office:string-value`.
    pub raw: Option<String>,
    /// `office:currency`.
    pub currency: Option<String>,
    /// The displayed text: the cell's paragraphs joined with line feeds, white space collapsed
    /// as ODF says, `text:s`, `text:tab` and `text:line-break` expanded. Annotations are left out.
    pub text: String,
    /// Paragraphs (`text:p`, `text:h`) the cell has.
    pub paragraphs: usize,
    /// `table:formula` as written (`of:=…`).
    pub formula: Option<String>,
    /// `table:style-name`.
    pub style: Option<String>,
    /// A `table:covered-table-cell`: hidden under a merged cell.
    pub covered: bool,
    /// `table:content-validation-name`.
    pub validation: Option<String>,
    /// The columns and rows of the matrix formula this cell holds
    /// (`table:number-matrix-columns-spanned`, `table:number-matrix-rows-spanned`).
    pub matrix: Option<(u32, u32)>,
    /// The first link (`text:a`) in the cell's paragraphs.
    pub link: Option<Link>,
    /// The stretches of [`Cell::text`] inside a `text:span` with a style, in order; where spans
    /// nest, the innermost one with a style. At most 4,096 per cell.
    pub runs: Vec<Run>,
    /// `office:annotation`, the cell's note, as written.
    pub annotation: Option<El>,
    /// Drawings anchored to the cell (`draw:frame`, `draw:custom-shape`, `draw:g` and the
    /// other `draw:` elements among its children), as written.
    pub drawings: Vec<El>,
}

/// A `text:a` in a cell.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Link {
    /// `xlink:href`.
    pub href: String,
    /// `office:title`.
    pub title: Option<String>,
    /// `office:target-frame-name`.
    pub target_frame: Option<String>,
}

/// A stretch of a cell's text in a character style: byte offsets into [`Cell::text`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Run {
    pub start: usize,
    pub end: usize,
    /// `text:style-name` of the span.
    pub style: String,
}

impl Cell {
    /// Nothing to keep but a style or a validation: no value, no text, no formula, no note,
    /// no drawing.
    pub fn is_blank(&self) -> bool {
        self.formula.is_none()
            && self.text.is_empty()
            && matches!(self.value_type, None | Some(ValueType::Void))
            && self.raw.is_none()
            && self.annotation.is_none()
            && self.drawings.is_empty()
    }

    /// The value the cell shows: a number for the numeric types, a serial number (1900 date
    /// system) for a date, days for a duration, the string value or else the text for a string.
    /// `None` when the type's value is missing or unreadable, or the cell is a formula without a
    /// cached result: the caller decides what to show (the text, [`Cell::raw`]).
    pub fn value(&self) -> Option<Value> {
        let raw = self.raw.as_deref();
        match self.value_type {
            Some(ValueType::Float | ValueType::Percentage | ValueType::Currency) => {
                raw.and_then(|s| s.trim().parse::<f64>().ok()).filter(|n| n.is_finite()).map(Value::Number)
            }
            Some(ValueType::Boolean) => match raw {
                Some("true" | "1") => Some(Value::Bool(true)),
                Some("false" | "0") => Some(Value::Bool(false)),
                _ => None,
            },
            Some(ValueType::String) if raw.is_some() || self.paragraphs > 0 || self.formula.is_none() => Some(Value::text(raw.unwrap_or(&self.text))),
            Some(ValueType::Void) if self.formula.is_none() => Some(Value::Empty),
            Some(ValueType::Date) => raw.and_then(date_serial).map(|(n, _)| Value::Number(n)),
            Some(ValueType::Time) => raw.and_then(duration_days).map(Value::Number),
            None if self.formula.is_none() => Some(if self.text.is_empty() { Value::Empty } else { Value::text(self.text.as_str()) }),
            _ => None,
        }
    }
}

/// `table:visibility` of a row or column.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Visibility {
    #[default]
    Visible,
    /// Hidden (`collapse`).
    Collapse,
    /// Hidden by a filter (`filter`).
    Filter,
}

/// A run of `table:table-row` or `table:table-column` elements' lines (one element and its
/// repeats) that states a style, a default cell style or a visibility. Lines that state none of
/// these are not listed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Line {
    /// The first row or column, from 0.
    pub first: u32,
    /// Lines in the run, held to the grid.
    pub count: u32,
    /// `table:style-name`: a row or column style (its height or width, its page break).
    pub style: Option<String>,
    /// `table:default-cell-style-name`.
    pub default_cell_style: Option<String>,
    pub visibility: Visibility,
}

/// A `table:table-row-group` or `table:table-column-group`: the lines it holds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Group {
    pub first: u32,
    /// Lines in the group; 0 for an empty group.
    pub count: u32,
    /// 1 for a group directly in the table, 2 for one inside that, and so on.
    pub level: u32,
    /// `table:display`: `false` when the group is collapsed.
    pub display: bool,
}

/// Blank cells ([`Cell::is_blank`]) that state a style or a validation, over a range: a cell
/// element and its repeats. Not expanded into [`Sheet::cells`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Blank {
    pub range: Range,
    pub style: Option<String>,
    pub validation: Option<String>,
}

/// One `table:table`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sheet {
    /// `table:name` as written.
    pub name: String,
    /// The `table:table` element's attributes as written, the name included: `table:style-name`,
    /// `table:print`, `table:print-ranges`, `table:protected`, `table:protection-key` and so on.
    pub attrs: Vec<(String, String)>,
    /// Cells that are not blank ([`Cell::is_blank`]), repeats expanded.
    pub cells: BTreeMap<CellRef, Cell>,
    /// Merged ranges, from their top left cell.
    pub merges: Vec<Range>,
    /// Blank cells with a style or a validation, as ranges.
    pub blanks: Vec<Blank>,
    /// Columns with a style, a default cell style or a visibility, in order.
    pub columns: Vec<Line>,
    /// Rows with a style, a default cell style or a visibility, in order.
    pub rows: Vec<Line>,
    pub column_groups: Vec<Group>,
    pub row_groups: Vec<Group>,
    /// `table:table-header-columns`: (first column, count).
    pub header_columns: Option<(u32, u32)>,
    /// `table:table-header-rows`: (first row, count).
    pub header_rows: Option<(u32, u32)>,
    /// The table's other child elements as written: `table:shapes`, `calcext:conditional-formats`,
    /// `table:named-expressions`, `table:table-source`, `table:scenario`, `office:forms` and so on.
    pub elements: Vec<El>,
}

impl Sheet {
    /// An attribute of the `table:table` element.
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
}

/// The tables of a spreadsheet document, in order, and what `content.xml` holds besides them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Workbook {
    pub sheets: Vec<Sheet>,
    /// [`Options::truncate_text`](crate::Options::truncate_text) cut the text short: the cells
    /// after the point where it passed `max_text` were left out.
    pub text_truncated: bool,
    /// The `office:spreadsheet` element's attributes (`table:structure-protected`,
    /// `table:protection-key`, …).
    pub attrs: Vec<(String, String)>,
    /// The spreadsheet's child elements other than tables, as written: `table:named-expressions`,
    /// `table:database-ranges`, `table:content-validations`, `table:data-pilot-tables`,
    /// `table:calculation-settings` and so on.
    pub body: Vec<El>,
    /// `content.xml`'s `office:font-face-decls`.
    pub font_faces: Option<El>,
    /// `content.xml`'s `office:automatic-styles`: the styles its cells, rows, columns and tables
    /// name.
    pub automatic_styles: Option<El>,
}

impl Workbook {
    /// An attribute of the `office:spreadsheet` element.
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
}

fn digits(s: &str) -> Option<&str> {
    (!s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())).then_some(s)
}

fn decimal(s: &str) -> Option<f64> {
    match s.split_once('.') {
        Some((a, b)) => {
            digits(a)?;
            digits(b)?;
        }
        None => {
            digits(s)?;
        }
    }
    s.parse::<f64>().ok().filter(|n| n.is_finite())
}

/// Days from 1970-01-01 to a proleptic Gregorian date.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * ((m + 9) % 12) + 2) / 5 + d - 1;
    era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468
}

fn days_in_month(y: i64, m: i64) -> i64 {
    match m {
        2 if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// An `office:date-value` (`2024-02-29` or `2024-02-29T13:45:30.5`) as a serial number in the
/// 1900 date system, and whether it had a time. Years 1900 to 9999; dates before 1 March 1900
/// are one lower than the day count, for the 29 February 1900 that spreadsheets count.
pub fn date_serial(s: &str) -> Option<(f64, bool)> {
    let (date, time) = s.split_once('T').map_or((s, None), |(d, t)| (d, Some(t)));
    if date.len() != 10 || date.get(4..5)? != "-" || date.get(7..8)? != "-" {
        return None;
    }
    let year: i64 = digits(date.get(..4)?)?.parse().ok()?;
    let month: i64 = digits(date.get(5..7)?)?.parse().ok()?;
    let day: i64 = digits(date.get(8..10)?)?.parse().ok()?;
    if !(1900..=9999).contains(&year) || !(1..=12).contains(&month) || day == 0 || day > days_in_month(year, month) {
        return None;
    }
    let serial = (days_from_civil(year, month, day) - days_from_civil(1899, 12, 30) - i64::from(year == 1900 && month <= 2)) as f64;
    let Some(time) = time else { return Some((serial, false)) };
    let mut parts = time.split(':');
    let h = parts.next()?;
    let m = parts.next()?;
    let sec = parts.next()?;
    if parts.next().is_some() || h.len() != 2 || m.len() != 2 || sec.split('.').next()?.len() != 2 {
        return None;
    }
    let h: u32 = digits(h)?.parse().ok()?;
    let m: u32 = digits(m)?.parse().ok()?;
    let sec = decimal(sec)?;
    if h >= 24 || m >= 60 || sec >= 60.0 {
        return None;
    }
    Some((serial + (f64::from(h) * 3600.0 + f64::from(m) * 60.0 + sec) / 86400.0, true))
}

/// An `office:time-value`, an ISO 8601 duration (`PT36H05M10.5S`, `P2DT1H`, `-PT1H`), in days.
/// At most 2,958,465 days (the last day spreadsheets count).
pub fn duration_days(s: &str) -> Option<f64> {
    let (negative, s) = s.strip_prefix('-').map_or((false, s), |s| (true, s));
    let mut rest = s.strip_prefix('P')?;
    let mut total = 0.0;
    let mut any = false;
    if let Some((days, tail)) = rest.split_once('D') {
        total += digits(days)?.parse::<f64>().ok()? * 86400.0;
        rest = tail;
        any = true;
    }
    if let Some(mut time) = rest.strip_prefix('T') {
        let mut time_any = false;
        for (unit, factor) in [('H', 3600.0), ('M', 60.0), ('S', 1.0)] {
            if let Some((n, tail)) = time.split_once(unit) {
                let value = if unit == 'S' { decimal(n)? } else { digits(n)?.parse::<f64>().ok()? };
                total += value * factor;
                time = tail;
                time_any = true;
            }
        }
        if !time.is_empty() || !time_any {
            return None;
        }
        any = true;
    } else if !rest.is_empty() {
        return None;
    }
    let days = total / 86400.0;
    (any && days.is_finite() && days <= 2_958_465.0).then_some(if negative { -days } else { days })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_and_durations() {
        assert_eq!(date_serial("1900-01-01"), Some((1.0, false)));
        assert_eq!(date_serial("1900-03-01"), Some((61.0, false)));
        assert_eq!(date_serial("2024-02-29T12:00:00.5"), Some((45351.0 + 43200.5 / 86400.0, true)));
        for bad in ["2023-02-29", "1899-12-31", "2024-1-01", "2024-01-01T1:00:00", "2024-01-01T24:00:00", "+024-01-01", "2024-01-01T12:00"] {
            assert_eq!(date_serial(bad), None, "{bad}");
        }
        assert_eq!(duration_days("PT36H"), Some(1.5));
        assert_eq!(duration_days("-P1DT12H"), Some(-1.5));
        assert_eq!(duration_days("PT1.5S"), Some(1.5 / 86400.0));
        for bad in ["P", "PT", "1H", "PT1X", "P1", "PT-1H", "P9999999D"] {
            assert_eq!(duration_days(bad), None, "{bad}");
        }
    }

    #[test]
    fn values() {
        let cell = |t: Option<ValueType>, raw: Option<&str>, text: &str, formula: bool| Cell {
            value_type: t,
            raw: raw.map(str::to_string),
            text: text.into(),
            paragraphs: usize::from(!text.is_empty()),
            formula: formula.then(|| "of:=1".to_string()),
            ..Cell::default()
        };
        assert_eq!(cell(Some(ValueType::Percentage), Some(" 0.25"), "25%", false).value(), Some(Value::Number(0.25)));
        assert_eq!(cell(Some(ValueType::Float), Some("inf"), "", false).value(), None);
        assert_eq!(cell(Some(ValueType::Boolean), Some("1"), "", false).value(), Some(Value::Bool(true)));
        assert_eq!(cell(Some(ValueType::String), None, "shown", true).value(), Some(Value::text("shown")));
        assert_eq!(cell(Some(ValueType::String), None, "", true).value(), None);
        assert_eq!(cell(Some(ValueType::String), Some("raw"), "shown", false).value(), Some(Value::text("raw")));
        assert_eq!(cell(Some(ValueType::Void), None, "", true).value(), None);
        assert_eq!(cell(Some(ValueType::Error), None, "#DIV/0!", true).value(), None);
        assert_eq!(cell(Some(ValueType::Date), Some("1900-01-02"), "", false).value(), Some(Value::Number(2.0)));
        assert_eq!(cell(None, None, "plain", false).value(), Some(Value::text("plain")));
        assert!(cell(Some(ValueType::Void), None, "", false).is_blank());
        assert!(!cell(None, None, "", true).is_blank());
    }
}
