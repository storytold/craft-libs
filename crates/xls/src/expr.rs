//! The expression tree a parsed formula decodes to, and its A1 formula text (without `=`).

use std::fmt::Write;
use std::sync::Arc;

use craft_sheet::{push_col, quote_sheet, sheet_needs_quotes};

use crate::CellError;

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

/// A number as a formula spells it: up to 15 significant digits, scientific for very large or
/// very small magnitudes.
pub(crate) fn number_to_text(n: f64) -> String {
    if n == 0.0 || !n.is_finite() {
        return "0".into();
    }
    let a = n.abs();
    if !(1e-9..1e15).contains(&a) && a.round() != a || a >= 1e15 {
        return sci_text(n, 15);
    }
    // A value just below 1e15 can round up to 1e15 at 15 significant digits.
    if format!("{a:.14e}").parse::<f64>().is_ok_and(|r| r >= 1e15) {
        return sci_text(n, 15);
    }
    let mag = a.log10().floor() as i32;
    let decimals = (14 - mag).clamp(0, 30) as usize;
    trim_decimal(&format!("{n:.decimals$}"))
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

/// Prints an expression in A1 notation.
pub(crate) fn print(e: &Expr) -> String {
    let mut s = String::new();
    write_expr(&mut s, e);
    s
}

fn write_args(s: &mut String, args: &[Expr], sep: char) {
    for (i, a) in args.iter().enumerate() {
        if i > 0 {
            s.push(sep);
        }
        write_expr(s, a);
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
            s.push_str(name);
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
        assert_eq!(number_to_text(1e15), "1E+15");
        assert_eq!(number_to_text(1234567890123456.0), "1.23456789012346E+15");
        assert_eq!(number_to_text(0.0001), "0.0001");
        assert_eq!(number_to_text(1e-10), "1E-10");
        assert!(number(f64::MAX).parse::<f64>().is_ok_and(f64::is_finite));
    }
}
