//! The expression tree an XLSB formula decodes to, and its A1 formula text (without `=`), in the
//! file's spelling (`_xlfn.` and `_xlpm.` names as written). A copy of craft-xls's: the two
//! readers follow separate specifications, and can share one tree when both settle.

use std::fmt::Write;
use std::sync::Arc;

use craft_sheet::{CellError, push_col, quote_sheet, sheet_needs_quotes};

/// One end of a reference, with `$` anchors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Anchor {
    pub row: u32,
    pub col: u32,
    pub row_abs: bool,
    pub col_abs: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RefKind {
    Cell(Anchor),
    Range(Anchor, Anchor),
    /// Whole rows `3:5` as (row, absolute) pairs.
    Rows(u32, bool, u32, bool),
    /// Whole columns `A:C`.
    Cols(u32, bool, u32, bool),
    /// A deleted cell or range on a named sheet: `Sheet2!#REF!`.
    Deleted,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SheetSel {
    /// Unqualified: the formula's own sheet.
    Current,
    Named(String),
    /// 3-D span `Sheet1:Sheet3`.
    Span(String, String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Reference {
    pub sheet: SheetSel,
    pub kind: RefKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UnOp {
    Neg,
    Plus,
    Percent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Pow,
    Concat,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    /// `:` between two references.
    Range,
    /// `,` inside parentheses.
    Union,
    /// A space between two references.
    Intersect,
}

impl BinOp {
    /// How tightly the operator binds, as formula text is read: `:` 90, space 85, `,` 80, `^` 50,
    /// `*` `/` 40, `+` `-` 30, `&` 20, comparisons 10.
    pub(crate) fn precedence(self) -> u8 {
        match self {
            BinOp::Range => 90,
            BinOp::Intersect => 85,
            BinOp::Union => 80,
            BinOp::Pow => 50,
            BinOp::Mul | BinOp::Div => 40,
            BinOp::Add | BinOp::Sub => 30,
            BinOp::Concat => 20,
            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge => 10,
        }
    }

    fn symbol(self) -> &'static str {
        match self {
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::Pow => "^",
            BinOp::Concat => "&",
            BinOp::Eq => "=",
            BinOp::Ne => "<>",
            BinOp::Lt => "<",
            BinOp::Gt => ">",
            BinOp::Le => "<=",
            BinOp::Ge => ">=",
            BinOp::Range => ":",
            BinOp::Union => ",",
            BinOp::Intersect => " ",
        }
    }
}

/// A formula's syntax tree. The decoder bounds its depth, so printing and dropping it recurse
/// safely.
#[derive(Debug, PartialEq)]
pub(crate) enum Expr {
    Number(f64),
    Text(Arc<str>),
    Bool(bool),
    Error(CellError),
    /// Array constant `{1,2;3,4}` (rows of literals).
    Array(Vec<Vec<Expr>>),
    Ref(Reference),
    /// A defined name.
    Name(String),
    /// Spaces and line breaks typed before and after an expression (`PtgAttrSpace`), which
    /// `.xlsx` formula text keeps.
    Spaced(String, Box<Expr>, String),
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    Call(String, Vec<Expr>),
    /// An omitted argument.
    Missing,
    Paren(Box<Expr>),
}

fn push_anchor(s: &mut String, a: &Anchor) {
    if a.col_abs {
        s.push('$');
    }
    push_col(s, a.col);
    if a.row_abs {
        s.push('$');
    }
    let _ = write!(s, "{}", u64::from(a.row) + 1);
}

fn push_reference(s: &mut String, r: &Reference) {
    match &r.sheet {
        SheetSel::Current => {}
        SheetSel::Named(n) => {
            s.push_str(&quote_sheet(n));
            s.push('!');
        }
        SheetSel::Span(a, b) => {
            let joined = format!("{a}:{b}");
            if sheet_needs_quotes(a) || sheet_needs_quotes(b) {
                let _ = write!(s, "'{}'!", joined.replace('\'', "''"));
            } else {
                let _ = write!(s, "{joined}!");
            }
        }
    }
    let dollar = |abs: bool| if abs { "$" } else { "" };
    match &r.kind {
        RefKind::Cell(a) => push_anchor(s, a),
        RefKind::Range(a, b) => {
            push_anchor(s, a);
            s.push(':');
            push_anchor(s, b);
        }
        RefKind::Rows(r0, a0, r1, a1) => {
            let _ = write!(s, "{}{}:{}{}", dollar(*a0), u64::from(*r0) + 1, dollar(*a1), u64::from(*r1) + 1);
        }
        RefKind::Cols(c0, a0, c1, a1) => {
            s.push_str(dollar(*a0));
            push_col(s, *c0);
            s.push(':');
            s.push_str(dollar(*a1));
            push_col(s, *c1);
        }
        RefKind::Deleted => s.push_str(CellError::Ref.as_str()),
    }
}

/// Removes trailing zeros (and a trailing point) from a decimal string.
fn trim_decimal(s: &str) -> String {
    if s.contains('.') {
        let t = s.trim_end_matches('0').trim_end_matches('.');
        if t == "-0" { "0".into() } else { t.to_string() }
    } else {
        s.to_string()
    }
}

/// Scientific text like `1.23456789012346E+15` with up to `sig` significant digits.
fn sci_text(n: f64, sig: usize) -> String {
    let s = format!("{:.*e}", sig.saturating_sub(1), n);
    let Some((mant, exp)) = s.split_once('e') else { return s };
    let mant = trim_decimal(mant);
    let e: i32 = exp.parse().unwrap_or(0);
    format!("{}E{}{:02}", mant, if e < 0 { '-' } else { '+' }, e.abs())
}

/// A number as Excel spells it in a formula: rounded to 15 significant digits, written out in
/// full (`100000000000000000000`, `0.0000000000000000001`) when that takes at most 21 characters
/// without the sign, and scientific (`1E+21`, `1.5E-19`, `1.23456789012345E-06`) when it would
/// take more.
pub(crate) fn number_to_text(n: f64) -> String {
    if n == 0.0 || !n.is_finite() {
        return "0".into();
    }
    let s = format!("{:.14e}", n.abs());
    let Some((mant, exp)) = s.split_once('e') else { return s };
    let Ok(e) = exp.parse::<i32>() else { return s };
    let digits: String = mant.chars().filter(char::is_ascii_digit).collect();
    let digits = digits.trim_end_matches('0');
    let digits = if digits.is_empty() { "0" } else { digits };
    let k = digits.len() as i32;
    let plain_len = if e >= 0 { (e + 1).max(k + i32::from(k > e + 1)) } else { 1 - e + k };
    if plain_len > 21 {
        return sci_text(n, 15);
    }
    let sign = if n < 0.0 { "-" } else { "" };
    let plain = if e >= 0 {
        let int = (e + 1) as usize;
        if digits.len() <= int { format!("{digits}{}", "0".repeat(int - digits.len())) } else { format!("{}.{}", &digits[..int], &digits[int..]) }
    } else {
        format!("0.{}{digits}", "0".repeat((-e - 1) as usize))
    };
    format!("{sign}{plain}")
}

fn number(n: f64) -> String {
    let text = number_to_text(n);
    // 15 significant digits can round past f64::MAX, which no longer parses.
    if text.parse::<f64>().is_ok_and(f64::is_finite) {
        return text;
    }
    let exact = format!("{n:E}");
    match exact.split_once('E') {
        Some((m, e)) if !e.starts_with('-') => format!("{m}E+{e}"),
        _ => exact,
    }
}

/// Splits a sheet-qualified name at its last `!` outside brackets.
fn sheet_split(n: &str) -> Option<(&str, &str)> {
    let mut depth = 0usize;
    let mut escaped = false;
    let mut at = None;
    for (i, c) in n.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '\'' if depth > 0 => escaped = true,
            '[' => depth = depth.saturating_add(1),
            ']' => depth = depth.saturating_sub(1),
            '!' if depth == 0 => at = Some(i),
            _ => {}
        }
    }
    at.and_then(|i| Some((n.get(..i)?, n.get(i + 1..)?)))
}

/// Prints an expression in A1 notation, trimmed as the `.xlsx` reader trims formula text.
pub(crate) fn print(e: &Expr) -> String {
    let mut s = String::new();
    write_expr(&mut s, e);
    match s.trim() {
        t if t.len() == s.len() => s,
        t => t.to_string(),
    }
}

fn write_args(s: &mut String, args: &[Expr], sep: char) {
    for (i, a) in args.iter().enumerate() {
        if i > 0 {
            s.push(sep);
        }
        write_expr(s, a);
    }
}

/// A function's name; a sheet-scoped one (`My Sheet!Fn`) is quoted as a sheet-qualified name is.
/// Kept out of [`write_expr`], whose frame every nesting level of a formula repeats.
#[inline(never)]
fn push_function_name(s: &mut String, name: &str) {
    match sheet_split(name) {
        Some((sheet, name)) => {
            s.push_str(&quote_sheet(sheet));
            s.push('!');
            s.push_str(name);
        }
        None => s.push_str(name),
    }
}

fn write_expr(s: &mut String, e: &Expr) {
    match e {
        Expr::Number(n) => s.push_str(&number(*n)),
        Expr::Text(t) => {
            s.push('"');
            s.push_str(&t.replace('"', "\"\""));
            s.push('"');
        }
        Expr::Bool(b) => s.push_str(if *b { "TRUE" } else { "FALSE" }),
        Expr::Error(err) => s.push_str(err.as_str()),
        Expr::Array(rows) => {
            s.push('{');
            for (ri, row) in rows.iter().enumerate() {
                if ri > 0 {
                    s.push(';');
                }
                write_args(s, row, ',');
            }
            s.push('}');
        }
        Expr::Ref(r) => push_reference(s, r),
        Expr::Name(n) => match sheet_split(n) {
            // A sheet-qualified name holds the bare sheet name (`My Sheet!Total`): quote it so it parses back.
            Some((sheet, name)) => {
                s.push_str(&quote_sheet(sheet));
                s.push('!');
                s.push_str(name);
            }
            None => s.push_str(n),
        },
        Expr::Unary(UnOp::Neg, x) => {
            s.push('-');
            write_expr(s, x);
        }
        Expr::Unary(UnOp::Plus, x) => {
            s.push('+');
            write_expr(s, x);
        }
        Expr::Unary(UnOp::Percent, x) => {
            write_expr(s, x);
            s.push('%');
        }
        Expr::Binary(op, a, b) => {
            write_expr(s, a);
            s.push_str(op.symbol());
            write_expr(s, b);
        }
        Expr::Call(name, args) => {
            push_function_name(s, name);
            s.push('(');
            write_args(s, args, ',');
            s.push(')');
        }
        Expr::Missing => {}
        Expr::Paren(x) => {
            s.push('(');
            write_expr(s, x);
            s.push(')');
        }
        Expr::Spaced(before, x, after) => {
            s.push_str(before);
            write_expr(s, x);
            s.push_str(after);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_print_with_fifteen_significant_digits() {
        assert_eq!(number_to_text(1.0), "1");
        assert_eq!(number_to_text(-2.5), "-2.5");
        assert_eq!(number_to_text(0.1 + 0.2), "0.3");
        assert_eq!(number_to_text(123456789012345.0), "123456789012345");
        // Written out in full up to 21 characters, as Excel writes them in .xlsx.
        assert_eq!(number_to_text(1e15), "1000000000000000");
        assert_eq!(number_to_text(1234567890123456.0), "1234567890123460");
        assert_eq!(number_to_text(-1e20), "-100000000000000000000");
        assert_eq!(number_to_text(1e21), "1E+21");
        assert_eq!(number_to_text(1.5e21), "1.5E+21");
        assert_eq!(number_to_text(0.0001), "0.0001");
        assert_eq!(number_to_text(1e-10), "0.0000000001");
        assert_eq!(number_to_text(1e-19), "0.0000000000000000001");
        assert_eq!(number_to_text(1e-20), "1E-20");
        assert_eq!(number_to_text(1.5e-19), "1.5E-19");
        assert_eq!(number_to_text(1.23456789012345e-5), "0.0000123456789012345");
        assert_eq!(number_to_text(1.23456789012345e-6), "1.23456789012345E-06");
        assert_eq!(number_to_text(1234567890.12345), "1234567890.12345");
        assert_eq!(number_to_text(1e100), "1E+100");
        assert!(number(f64::MAX).parse::<f64>().is_ok_and(f64::is_finite));
    }

    #[test]
    fn a_sheet_scoped_function_name_is_quoted() {
        let call = |name: &str| print(&Expr::Call(name.into(), vec![Expr::Number(1.0)]));
        assert_eq!(call("My Sheet!Fn"), "'My Sheet'!Fn(1)");
        assert_eq!(call("Data!Fn"), "Data!Fn(1)");
        assert_eq!(call("SUM"), "SUM(1)");
    }
}
