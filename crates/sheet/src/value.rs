//! What a cell stores.

use std::sync::Arc;

/// The error values a spreadsheet file stores. The first eight are the ones BIFF8 has
/// ([MS-XLS] 2.5.10, BErr); the rest are newer and appear in SpreadsheetML as text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum CellError {
    Null,
    Div0,
    Value,
    Ref,
    Name,
    Num,
    NA,
    GettingData,
    Spill,
    Calc,
    Blocked,
    Python,
    Busy,
    Timeout,
}

impl CellError {
    /// Every error value, in the order above.
    pub const ALL: [CellError; 14] = [
        CellError::Null,
        CellError::Div0,
        CellError::Value,
        CellError::Ref,
        CellError::Name,
        CellError::Num,
        CellError::NA,
        CellError::GettingData,
        CellError::Spill,
        CellError::Calc,
        CellError::Blocked,
        CellError::Python,
        CellError::Busy,
        CellError::Timeout,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            CellError::Null => "#NULL!",
            CellError::Div0 => "#DIV/0!",
            CellError::Value => "#VALUE!",
            CellError::Ref => "#REF!",
            CellError::Name => "#NAME?",
            CellError::Num => "#NUM!",
            CellError::NA => "#N/A",
            CellError::GettingData => "#GETTING_DATA",
            CellError::Spill => "#SPILL!",
            CellError::Calc => "#CALC!",
            CellError::Blocked => "#BLOCKED!",
            CellError::Python => "#PYTHON!",
            CellError::Busy => "#BUSY!",
            CellError::Timeout => "#TIMEOUT!",
        }
    }

    /// The error with this text, in any case.
    pub fn parse(s: &str) -> Option<CellError> {
        CellError::ALL.iter().find(|e| e.as_str().eq_ignore_ascii_case(s)).copied()
    }
}

impl std::fmt::Display for CellError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A cell's stored value (for a formula cell, its saved result).
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Value {
    #[default]
    Empty,
    Number(f64),
    Text(Arc<str>),
    Bool(bool),
    Error(CellError),
}

impl Value {
    /// A number, or `#NUM!` when it is NaN or infinite (Excel never stores those). Negative zero
    /// becomes zero.
    pub fn number(v: f64) -> Value {
        if v.is_finite() { Value::Number(if v == 0.0 { 0.0 } else { v }) } else { Value::Error(CellError::Num) }
    }

    pub fn text(s: impl Into<Arc<str>>) -> Value {
        Value::Text(s.into())
    }

    pub fn is_empty(&self) -> bool {
        matches!(self, Value::Empty)
    }
}

/// Whether a sheet shows: `VeryHidden` cannot be unhidden from the user interface.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Visibility {
    #[default]
    Visible,
    Hidden,
    VeryHidden,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_parse_back_from_their_text() {
        for e in CellError::ALL {
            assert_eq!(CellError::parse(e.as_str()), Some(e));
            assert_eq!(CellError::parse(&e.as_str().to_ascii_lowercase()), Some(e));
        }
        assert_eq!(CellError::parse("#NOPE!"), None);
        assert_eq!(CellError::Div0.to_string(), "#DIV/0!");
    }

    #[test]
    fn numbers_are_finite() {
        assert_eq!(Value::number(f64::NAN), Value::Error(CellError::Num));
        assert_eq!(Value::number(f64::INFINITY), Value::Error(CellError::Num));
        assert!(matches!(Value::number(-0.0), Value::Number(n) if n.is_sign_positive()));
        assert!(Value::Empty.is_empty() && !Value::text("").is_empty());
    }
}
