//! Cell positions and their A1 text.

use std::fmt::Write;

/// Rows in the grid of Excel 2007 and later.
pub const MAX_ROWS: u32 = 1_048_576;
/// Columns in the grid of Excel 2007 and later (`A` to `XFD`).
pub const MAX_COLS: u32 = 16_384;

/// A cell position, zero-based. Ordered by row, then column.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CellRef {
    pub row: u32,
    pub col: u32,
}

impl CellRef {
    pub const fn new(row: u32, col: u32) -> Self {
        CellRef { row, col }
    }

    /// The A1 text (`B3`).
    pub fn a1(self) -> String {
        let mut s = String::new();
        self.push_a1(&mut s);
        s
    }

    /// Appends the A1 text.
    pub fn push_a1(self, s: &mut String) {
        push_col(s, self.col);
        let _ = write!(s, "{}", u64::from(self.row) + 1);
    }

    /// Reads `B3`, `$B$3`, `b3`: a cell inside the grid, with nothing after it.
    pub fn parse(s: &str) -> Option<CellRef> {
        match CellRef::parse_prefix(s)? {
            (cell, "") => Some(cell),
            _ => None,
        }
    }

    /// Reads an A1 reference at the start of `s` and returns it with the rest of `s`.
    pub fn parse_prefix(s: &str) -> Option<(CellRef, &str)> {
        let s = s.strip_prefix('$').unwrap_or(s);
        let letters = s.bytes().take_while(u8::is_ascii_alphabetic).count();
        let (cols, rest) = s.split_at_checked(letters)?;
        let col = col_from_letters(cols)?;
        let rest = rest.strip_prefix('$').unwrap_or(rest);
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 || digits > 7 {
            return None;
        }
        let (rows, rest) = rest.split_at_checked(digits)?;
        let row: u32 = rows.parse().ok()?;
        if row == 0 || row > MAX_ROWS {
            return None;
        }
        Some((CellRef::new(row - 1, col), rest))
    }
}

impl std::fmt::Display for CellRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.a1())
    }
}

/// A rectangle of cells, both corners included; `first` is the top left one. The default is
/// `A1`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Range {
    pub first: CellRef,
    pub last: CellRef,
}

impl Range {
    /// The rectangle spanned by two corners, in any order.
    pub fn new(a: CellRef, b: CellRef) -> Self {
        Range { first: CellRef::new(a.row.min(b.row), a.col.min(b.col)), last: CellRef::new(a.row.max(b.row), a.col.max(b.col)) }
    }

    pub fn contains(&self, c: CellRef) -> bool {
        (self.first.row..=self.last.row).contains(&c.row) && (self.first.col..=self.last.col).contains(&c.col)
    }

    pub fn is_single(&self) -> bool {
        self.first == self.last
    }

    /// How many cells the rectangle holds.
    pub fn count(&self) -> u64 {
        (u64::from(self.last.row - self.first.row) + 1) * (u64::from(self.last.col - self.first.col) + 1)
    }

    /// The A1 text: `B3` for one cell, `A1:C2` otherwise.
    pub fn a1(&self) -> String {
        let mut s = self.first.a1();
        if !self.is_single() {
            s.push(':');
            self.last.push_a1(&mut s);
        }
        s
    }

    /// Reads `A1:C2`, `$A$1:$C$2`, one cell (`B3`), whole columns (`A:C`) and whole rows
    /// (`1:3`), with surrounding space ignored. Whole columns and rows span the grid.
    pub fn parse(s: &str) -> Option<Range> {
        let s = s.trim();
        let Some((a, b)) = s.split_once(':') else {
            return CellRef::parse(s).map(|c| Range { first: c, last: c });
        };
        if let (Some(x), Some(y)) = (CellRef::parse(a), CellRef::parse(b)) {
            return Some(Range::new(x, y));
        }
        let (a, b) = (a.strip_prefix('$').unwrap_or(a), b.strip_prefix('$').unwrap_or(b));
        if let (Some(x), Some(y)) = (col_from_letters(a), col_from_letters(b)) {
            return Some(Range::new(CellRef::new(0, x), CellRef::new(MAX_ROWS - 1, y)));
        }
        let row = |t: &str| -> Option<u32> {
            if t.is_empty() || t.len() > 7 || !t.bytes().all(|c| c.is_ascii_digit()) {
                return None;
            }
            t.parse::<u32>().ok().filter(|r| (1..=MAX_ROWS).contains(r)).map(|r| r - 1)
        };
        Some(Range::new(CellRef::new(row(a)?, 0), CellRef::new(row(b)?, MAX_COLS - 1)))
    }
}

impl std::fmt::Display for Range {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.a1())
    }
}

/// Appends a column's letters: 0 is `A`, 25 is `Z`, 26 is `AA`.
pub fn push_col(s: &mut String, col: u32) {
    let mut letters = [0u8; 8];
    let mut i = letters.len();
    let mut n = u64::from(col) + 1;
    while n > 0 && i > 0 {
        i -= 1;
        if let Some(l) = letters.get_mut(i) {
            *l = b'A' + ((n - 1) % 26) as u8;
        }
        n = (n - 1) / 26;
    }
    for &l in letters.get(i..).unwrap_or_default() {
        s.push(char::from(l));
    }
}

/// A column's letters: 0 is `A`, 16383 is `XFD`.
pub fn col_letters(col: u32) -> String {
    let mut s = String::new();
    push_col(&mut s, col);
    s
}

/// `A` is 0, `xfd` is 16383. `None` for empty text, anything but one to three letters, or a
/// column beyond `XFD`.
pub fn col_from_letters(s: &str) -> Option<u32> {
    if s.is_empty() || s.len() > 3 || !s.bytes().all(|b| b.is_ascii_alphabetic()) {
        return None;
    }
    let n = s.bytes().fold(0u32, |n, b| n * 26 + u32::from(b.to_ascii_uppercase() - b'A') + 1);
    (n <= MAX_COLS).then(|| n - 1)
}

/// Whether `name` reads as an R1C1 reference: `R`, `C`, `RC`, `R1`, `C1`, `R1C`, `RC1`, `R1C1`
/// in any case.
fn is_r1c1_token(name: &str) -> bool {
    let mut chars = name.chars().peekable();
    let mut letters = 0;
    for letter in ['r', 'c'] {
        if chars.peek().is_some_and(|c| c.to_ascii_lowercase() == letter) {
            chars.next();
            letters += 1;
            while chars.peek().is_some_and(char::is_ascii_digit) {
                chars.next();
            }
        }
    }
    letters > 0 && chars.next().is_none()
}

/// Whether Excel quotes a sheet name in a reference (`'My Sheet'!A1`): it starts with a digit or
/// `.`, holds anything but letters (any script), digits, `_` and `.`, reads as an A1 cell or as
/// an R1C1 reference, or is a boolean.
pub fn sheet_needs_quotes(name: &str) -> bool {
    let Some(first) = name.chars().next() else {
        return true;
    };
    if first.is_ascii_digit() || first == '.' || name.chars().any(|c| !(c.is_alphanumeric() || c == '_' || c == '.')) {
        return true;
    }
    // No `$` can be here (it is not alphanumeric), so this is exactly "an A1 cell in the grid".
    CellRef::parse(name).is_some() || is_r1c1_token(name) || name.eq_ignore_ascii_case("TRUE") || name.eq_ignore_ascii_case("FALSE")
}

/// A sheet name as it is written in a reference: quoted, with `'` doubled, when Excel quotes it.
pub fn quote_sheet(name: &str) -> String {
    if sheet_needs_quotes(name) { format!("'{}'", name.replace('\'', "''")) } else { name.to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a1_text_both_ways() {
        assert_eq!(CellRef::new(2, 27).a1(), "AB3");
        assert_eq!(CellRef::parse("AB3"), Some(CellRef::new(2, 27)));
        assert_eq!(CellRef::parse("$ab$3"), Some(CellRef::new(2, 27)));
        assert_eq!(CellRef::parse("XFD1048576"), Some(CellRef::new(MAX_ROWS - 1, MAX_COLS - 1)));
        assert_eq!(CellRef::new(MAX_ROWS - 1, MAX_COLS - 1).a1(), "XFD1048576");
        for bad in ["", "A", "1", "A0", "XFE1", "A1048577", "A1x", "A+1", "A-1", "AAAA1", "A12345678", "É1"] {
            assert_eq!(CellRef::parse(bad), None, "{bad}");
        }
        assert_eq!(CellRef::parse_prefix("B2:C3"), Some((CellRef::new(1, 1), ":C3")));
        assert_eq!(col_letters(16383), "XFD");
        assert_eq!((col_letters(26), col_letters(u32::MAX).len()), ("AA".to_string(), 7));
        assert_eq!(col_from_letters("xfd"), Some(16383));
        assert_eq!(col_from_letters("XFE"), None);
    }

    #[test]
    fn ranges() {
        let r = Range::parse(" $C$2:A1 ");
        assert_eq!(r, Some(Range { first: CellRef::new(0, 0), last: CellRef::new(1, 2) }));
        assert_eq!(r.map(|r| (r.a1(), r.count())), Some(("A1:C2".to_string(), 6)));
        assert_eq!(Range::parse("B3").map(|r| (r.is_single(), r.a1())), Some((true, "B3".to_string())));
        assert_eq!(Range::parse("B:$A"), Some(Range::new(CellRef::new(0, 0), CellRef::new(MAX_ROWS - 1, 1))));
        assert_eq!(Range::parse("3:2"), Some(Range::new(CellRef::new(1, 0), CellRef::new(2, MAX_COLS - 1))));
        assert_eq!(Range::parse("1:1048577"), None);
        assert_eq!(Range::parse("0:1"), None);
        assert_eq!(Range::parse("A1:"), None);
        assert_eq!(Range::parse("A:1"), None);
        assert!(Range::new(CellRef::new(0, 0), CellRef::new(MAX_ROWS - 1, MAX_COLS - 1)).count() == 17_179_869_184);
    }

    #[test]
    fn sheet_names_are_quoted_when_excel_quotes_them() {
        for plain in ["Sheet1", "Data_2", "Übersicht", "a.b", "XFE1", "A1048577", "RC1x"] {
            assert!(!sheet_needs_quotes(plain), "{plain}");
        }
        for quoted in ["", "My Sheet", "1st", ".x", "A1", "xfd1048576", "R1C1", "rc", "C", "TRUE", "false", "a-b", "It's"] {
            assert!(sheet_needs_quotes(quoted), "{quoted}");
        }
        assert_eq!(quote_sheet("It's"), "'It''s'");
        assert_eq!(quote_sheet("Sheet1"), "Sheet1");
    }
}
