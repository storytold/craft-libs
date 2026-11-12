//! What a spreadsheet reads into: the file's own cells, nothing interpreted beyond the format.

use std::collections::BTreeMap;

use craft_sheet::{CellRef, Range, Value};

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
#[derive(Clone, Debug, Default, PartialEq, Eq)]
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
}

impl Cell {
    /// Nothing to keep: no value, no text, no formula.
    pub fn is_blank(&self) -> bool {
        self.formula.is_none() && self.text.is_empty() && matches!(self.value_type, None | Some(ValueType::Void)) && self.raw.is_none()
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

/// One `table:table`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Sheet {
    /// `table:name` as written.
    pub name: String,
    /// Cells that are not blank ([`Cell::is_blank`]), repeats expanded.
    pub cells: BTreeMap<CellRef, Cell>,
    /// Merged ranges, from their top left cell.
    pub merges: Vec<Range>,
}

/// The tables of a spreadsheet document, in order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Workbook {
    pub sheets: Vec<Sheet>,
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
