//! BIFF12 parsed formulas ([MS-XLSB] 2.2.2, 2.5.98): reverse-Polish `Ptg` tokens back to an
//! expression tree, for cell formulas. The tokens are BIFF8's, but nearly every operand differs
//! (32-bit rows, 14-bit columns, UTF-16 strings, its own array-constant codes, newer functions
//! called through hidden names), so this is its own decoder, with its own function table
//! ([`super::ftab`]) and its own copies of craft-xls's depth guards and limits. Anything outside the
//! token set it knows is reported as unsupported, and the caller keeps the cell's cached value.
//! So is a formula that needs something the reader doesn't read: a defined name whose own
//! formula could not be read, or Python code stored outside the cell. Defined names' formulas
//! are decoded here too ([`Mode::Name`]).
//!
//! Names keep their file spelling (`_xlfn.CONCAT`, `_xlpm.x`), as `.xlsx` formula text has them.

use std::sync::Arc;

use craft_sheet::{CellError, CellRef, MAX_COLS, MAX_ROWS};

use super::expr::{Anchor, BinOp, Expr, RefKind, Reference, SheetSel, UnOp};
use super::ftab::function;

/// Formula token bytes decoded at most (a cell formula holds at most 16,384).
pub(super) const MAX_RGCE: usize = 16 * 1024;
/// Values in one formula's array constants at most, all of them together: each array is charged
/// before its values are held, so a formula of many arrays can't hold more than this.
const MAX_ARRAY: usize = 65_536;

/// Why a formula could not be turned back into text.
#[derive(Debug, PartialEq)]
pub(super) struct Unsupported(pub &'static str);

/// Expression nesting at most: deeper trees would overflow the stack when printed or dropped (a
/// hostile file can chain thousands of PtgParen tokens).
const MAX_DEPTH: usize = 256;

/// Operand stack entries: an expression and its nesting depth.
type Stack = Vec<(Expr, usize)>;

/// Pops an operand, recording its depth in `kids` (the deepest operand this token consumed).
fn pop(stack: &mut Stack, kids: &mut usize) -> Result<Expr, Unsupported> {
    let (e, d) = stack.pop().ok_or(Unsupported("malformed formula (operand missing)"))?;
    *kids = (*kids).max(d);
    Ok(e)
}

fn pop_n(stack: &mut Stack, n: usize, kids: &mut usize) -> Result<Vec<Expr>, Unsupported> {
    let at = stack.len().checked_sub(n).ok_or(Unsupported("malformed formula (argument missing)"))?;
    let popped = stack.split_off(at);
    *kids = popped.iter().map(|(_, d)| *d).fold(*kids, usize::max);
    Ok(popped.into_iter().map(|(e, _)| e).collect())
}

/// Pushes the result of a token one level above the operands it consumed (two for one with the
/// spacing typed around it, which prints as a level of its own).
fn push(stack: &mut Stack, kids: usize, e: Expr) -> Result<(), Unsupported> {
    let depth = kids.saturating_add(1).saturating_add(usize::from(matches!(e, Expr::Spaced(..))));
    if depth > MAX_DEPTH {
        return Err(Unsupported("formula nested too deeply"));
    }
    stack.push((e, depth));
    Ok(())
}

/// Spaces and line breaks typed in the formula (`PtgAttrSpace`, [MS-XLSB] 2.5.98.32), waiting
/// for the token they belong to. Excel writes the attribute just before that token: spaces
/// before an operand, an operator's symbol or a function call go in `before`; those before a
/// parenthesis's `(`, or before the `)` of a parenthesis or a call, in `open` and `close`.
#[derive(Default)]
struct Spacing {
    before: String,
    open: String,
    close: String,
}

/// `e` with `ws` typed before it.
fn spaced_before(e: Expr, ws: String) -> Expr {
    match e {
        _ if ws.is_empty() => e,
        Expr::Spaced(b, x, a) => Expr::Spaced(ws + &b, x, a),
        e => Expr::Spaced(ws, Box::new(e), String::new()),
    }
}

/// `e` with `ws` typed after it.
fn spaced_after(e: Expr, ws: String) -> Expr {
    match e {
        _ if ws.is_empty() => e,
        Expr::Spaced(b, x, a) => Expr::Spaced(b, x, a + &ws),
        e => Expr::Spaced(String::new(), Box::new(e), ws),
    }
}

/// A call's arguments with the spacing typed before its `)`: after the last argument, or alone
/// between the parentheses (`PI( )`).
fn close_args(mut args: Vec<Expr>, ws: String) -> Vec<Expr> {
    if ws.is_empty() {
        return args;
    }
    match args.pop() {
        Some(last) => args.push(spaced_after(last, ws)),
        None => args.push(Expr::Spaced(String::new(), Box::new(Expr::Missing), ws)),
    }
    args
}

/// An error constant's code ([MS-XLSB] 2.5.98.21, `BErr`).
fn error_code(code: u8) -> Option<CellError> {
    Some(match code {
        0x00 => CellError::Null,
        0x07 => CellError::Div0,
        0x0F => CellError::Value,
        0x17 => CellError::Ref,
        0x1D => CellError::Name,
        0x24 => CellError::Num,
        0x2A => CellError::NA,
        0x2B => CellError::GettingData,
        _ => return None,
    })
}

/// A supporting link record ([MS-XLSB] 2.2.7.3), in the order of the workbook's externals block.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Book {
    /// `BrtSupSelf`: this workbook.
    This,
    /// `BrtSupSame`: the caller's sheet (macro sheets and names only).
    Same,
    /// `BrtSupAddin`: add-in functions.
    Addin,
    /// `BrtSupBookSrc`: another workbook, DDE or OLE. Never followed.
    External,
}

/// Xti ([MS-XLSB] 2.5.173): a supporting link and a range of its sheets.
#[derive(Clone, Copy, Debug)]
pub(super) struct Xti {
    pub book: u32,
    pub first: i32,
    pub last: i32,
}

/// A `BrtName` as far as formulas need it.
#[derive(Clone, Debug)]
pub(super) struct Name {
    /// The name as `.xlsx` spells it (`_xlfn.CONCAT`, `_xlpm.x`, `_xlnm.Print_Area`, `Rate`).
    pub text: String,
    /// `fFutureFunction`: a hidden name standing for a function newer than the file format.
    pub future: bool,
    /// `itab`: the sheet a sheet-scoped name belongs to, by its place in `BrtBundleSh` order.
    pub scope: Option<u32>,
    /// Its own formula was read, so it is one of the workbook's defined names.
    pub read: bool,
}

impl Name {
    /// Whether a formula may use the name: one the formula text itself defines (a newer
    /// function, a LET parameter), or a defined name the reader gives. A name's formula may use
    /// any other name: its text is the same either way.
    fn usable(&self, mode: Mode) -> bool {
        self.future || is_param(&self.text) || self.read || mode == Mode::Name
    }
}

/// Workbook-level tables formulas refer to.
#[derive(Default)]
pub(crate) struct Links {
    /// Every sheet's name in `BrtBundleSh` order, `None` for the sheets that hold no cells (chart,
    /// dialog and module sheets).
    pub(super) sheets: Vec<Option<String>>,
    /// Every `BrtName`, in order: `PtgName` indexes them from 1.
    pub(super) names: Vec<Name>,
    pub(super) books: Vec<Book>,
    pub(super) xti: Vec<Xti>,
}

/// Whether a name stands for a LET parameter (`_xlpm.x`), which only the formula itself defines.
pub(super) fn is_param(name: &str) -> bool {
    name.get(..6).is_some_and(|p| p.eq_ignore_ascii_case("_xlpm."))
}

impl Links {
    /// `PtgName`'s and `PtgNameX`'s name: a 1-based index into every `BrtName`. A cell formula
    /// using a defined name the reader doesn't give would calculate to `#NAME?`, so it keeps its
    /// saved result instead.
    fn name(&self, index: u32, mode: Mode) -> Result<&Name, Unsupported> {
        let i = usize::try_from(index).ok().and_then(|i| i.checked_sub(1));
        let name = i.and_then(|i| self.names.get(i)).ok_or(Unsupported("reference to a missing defined name"))?;
        if !name.usable(mode) {
            return Err(Unsupported("defined names whose formula could not be read"));
        }
        Ok(name)
    }

    /// Sheet link `ixti`, which must be through this workbook's own supporting link.
    fn this_book(&self, ixti: u16) -> Result<&Xti, Unsupported> {
        let x = self.xti.get(usize::from(ixti)).ok_or(Unsupported("reference to a missing sheet link"))?;
        match usize::try_from(x.book).ok().and_then(|b| self.books.get(b)) {
            Some(Book::This) => Ok(x),
            Some(Book::External) => Err(Unsupported("references to other workbooks (external links)")),
            Some(_) => Err(Unsupported("references through add-in or same-sheet links")),
            None => Err(Unsupported("reference to a missing workbook link")),
        }
    }

    /// The sheets a 3-D reference names, or `None` for a deleted sheet (the reference is `#REF!`).
    fn sheet_sel(&self, ixti: u16) -> Result<Option<SheetSel>, Unsupported> {
        let x = self.this_book(ixti)?;
        // -1: "the first sheet in the reference cannot be found" (a deleted sheet).
        if x.first == -1 || x.last == -1 {
            return Ok(None);
        }
        // -2 (the whole workbook) is for names, not cell references.
        let (Ok(first), Ok(last)) = (usize::try_from(x.first), usize::try_from(x.last)) else {
            return Err(Unsupported("reference to a missing sheet"));
        };
        // A span is spelled by its ends. A sheet between them with no cells (a chart sheet) adds
        // nothing to it, and `.xlsx` spells the span the same; an end with no cells can't be named.
        let span = self.sheets.get(first..=last).filter(|s| !s.is_empty()).ok_or(Unsupported("reference to a missing sheet"))?;
        let (Some(Some(a)), Some(Some(b))) = (span.first(), span.last()) else {
            return Err(Unsupported("references to sheets with no cells"));
        };
        Ok(Some(if first == last { SheetSel::Named(a.clone()) } else { SheetSel::Span(a.clone(), b.clone()) }))
    }
}

struct Bytes<'a> {
    b: &'a [u8],
    pos: usize,
}

const TRUNC: Unsupported = Unsupported("truncated formula");

impl Bytes<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], Unsupported> {
        let s = self.b.get(self.pos..self.pos.checked_add(n).ok_or(TRUNC)?).ok_or(TRUNC)?;
        self.pos += n;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, Unsupported> {
        self.take(1)?.first().copied().ok_or(TRUNC)
    }
    fn u16(&mut self) -> Result<u16, Unsupported> {
        let s: [u8; 2] = self.take(2)?.try_into().map_err(|_| TRUNC)?;
        Ok(u16::from_le_bytes(s))
    }
    fn u32(&mut self) -> Result<u32, Unsupported> {
        let s: [u8; 4] = self.take(4)?.try_into().map_err(|_| TRUNC)?;
        Ok(u32::from_le_bytes(s))
    }
    fn f64(&mut self) -> Result<f64, Unsupported> {
        let s: [u8; 8] = self.take(8)?.try_into().map_err(|_| TRUNC)?;
        let n = f64::from_le_bytes(s);
        if !n.is_finite() {
            return Err(Unsupported("invalid number constant"));
        }
        Ok(n)
    }
    /// `cch` UTF-16 code units; unpaired surrogates are refused, not replaced.
    fn chars(&mut self, cch: usize) -> Result<String, Unsupported> {
        let units = self.take(cch.checked_mul(2).ok_or(TRUNC)?)?.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c));
        char::decode_utf16(units).collect::<Result<String, _>>().map_err(|_| Unsupported("invalid text constant"))
    }
}

/// The last row and column of the grid: a range from 0 to these is a whole column or row.
const LAST_ROW: u32 = MAX_ROWS - 1;
const LAST_COL: u32 = MAX_COLS - 1;

/// Where a formula lives: a cell formula's references hold coordinates; a shared formula's
/// relative tokens (`PtgRefN`, `PtgAreaN`) hold offsets from the shared formula's first cell.
///
/// A defined name's relative references are offsets from A1 that wrap around the grid
/// (`RgceLocRel`), in its 3-D references too: `.xlsx` spells them as the cell they reach from A1
/// (one column left is `Data!XFD1`).
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Mode {
    Cell,
    Shared(CellRef),
    Name,
}

/// RgceLoc ([MS-XLSB] 2.5.98.91): a row and a `ColRelShort` (14-bit column, then the
/// column-relative and row-relative flags). In a cell formula the coordinates are absolute
/// whatever the flags say; the flags only decide where `$` goes.
fn anchor(row: u32, col: u16) -> Result<Anchor, Unsupported> {
    let c = u32::from(col & 0x3FFF);
    if row > LAST_ROW || c > LAST_COL {
        return Err(Unsupported("reference outside the grid"));
    }
    Ok(Anchor { row, col: c, row_abs: col & 0x8000 == 0, col_abs: col & 0x4000 == 0 })
}

/// RgceLocRel ([MS-XLSB] 2.5.98.92), in a shared formula: a relative row or column is an offset
/// from `base`, wrapping around the grid (1,048,576 rows, 16,384 columns), as Excel adds them.
/// An absolute one is a coordinate.
fn relative_anchor(row: u32, col: u16, base: CellRef) -> Result<Anchor, Unsupported> {
    let (row_rel, col_rel) = (col & 0x8000 != 0, col & 0x4000 != 0);
    let r = if row_rel { base.row.wrapping_add(row) % MAX_ROWS } else { row };
    let c = if col_rel { (base.col + u32::from(col & 0x3FFF)) % MAX_COLS } else { u32::from(col & 0x3FFF) };
    anchor(r, (c as u16) | (col & 0xC000))
}

fn loc(row: u32, col: u16, mode: Mode) -> Result<Anchor, Unsupported> {
    match mode {
        Mode::Cell => anchor(row, col),
        Mode::Shared(base) => relative_anchor(row, col, base),
        Mode::Name => relative_anchor(row, col, CellRef::new(0, 0)),
    }
}

/// A 3-D reference's cell: a coordinate, except in a defined name.
fn loc3d(row: u32, col: u16, mode: Mode) -> Result<Anchor, Unsupported> {
    if mode == Mode::Name { loc(row, col, mode) } else { anchor(row, col) }
}

/// RgceArea: a range spanning every row (or column) is a whole-column (row) reference.
fn area(r0: u32, r1: u32, c0: u16, c1: u16) -> Result<RefKind, Unsupported> {
    area_of(anchor(r0, c0)?, anchor(r1, c1)?)
}

fn area_of(a: Anchor, b: Anchor) -> Result<RefKind, Unsupported> {
    Ok(if a.row == 0 && b.row == LAST_ROW {
        RefKind::Cols(a.col, a.col_abs, b.col, b.col_abs)
    } else if a.col == 0 && b.col == LAST_COL {
        RefKind::Rows(a.row, a.row_abs, b.row, b.row_abs)
    } else {
        RefKind::Range(a, b)
    })
}

fn reference(sheet: Option<SheetSel>, kind: RefKind) -> Expr {
    match sheet {
        Some(sheet) => Expr::Ref(Reference { sheet, kind }),
        None => Expr::Error(CellError::Ref),
    }
}

/// Decodes a formula's `rgce` (tokens) with its `rgcb` (extra data for array constants and
/// cached areas). Returns the expression, with names in their file spelling, or why it can't be
/// represented. A shared formula's tokens are decoded once for each of its cells
/// (`Mode::Shared` with that cell), whose relative references are offsets from it.
pub(super) fn decode(rgce: &[u8], rgcb: &[u8], links: &Links, mode: Mode) -> Result<Expr, Unsupported> {
    decode_using(rgce, rgcb, links, mode, &mut Vec::new())
}

/// [`decode`], adding to `used` the 0-based index of every defined name the formula uses.
pub(super) fn decode_using(rgce: &[u8], rgcb: &[u8], links: &Links, mode: Mode, used: &mut Vec<usize>) -> Result<Expr, Unsupported> {
    if rgce.len() > MAX_RGCE {
        return Err(Unsupported("formula too long"));
    }
    let mut t = Bytes { b: rgce, pos: 0 };
    let mut extra = Bytes { b: rgcb, pos: 0 };
    let mut stack: Stack = Vec::new();
    let mut array_values = MAX_ARRAY;
    let mut ws = Spacing::default();
    while t.pos < rgce.len() {
        let ptg = t.u8()?;
        let mut kids = 0usize;
        // Operand classes (reference / value / array) share one token number.
        let id = if ptg & 0x60 != 0 { 0x20 | (ptg & 0x1F) } else { ptg };
        // An operand (a constant, reference or name) takes the spacing typed before it; operators,
        // calls and parentheses place theirs below, and attribute and cache tokens take none.
        let operand = !matches!(id, 0x03..=0x15 | 0x19 | 0x21 | 0x22 | 0x26..=0x29);
        let height = stack.len();
        match id {
            0x03..=0x11 => {
                let op = match id {
                    0x03 => BinOp::Add,
                    0x04 => BinOp::Sub,
                    0x05 => BinOp::Mul,
                    0x06 => BinOp::Div,
                    0x07 => BinOp::Pow,
                    0x08 => BinOp::Concat,
                    0x09 => BinOp::Lt,
                    0x0A => BinOp::Le,
                    0x0B => BinOp::Eq,
                    0x0C => BinOp::Ge,
                    0x0D => BinOp::Gt,
                    0x0E => BinOp::Ne,
                    0x0F => BinOp::Intersect,
                    0x10 => BinOp::Union,
                    _ => BinOp::Range,
                };
                let b = pop(&mut stack, &mut kids)?;
                let a = pop(&mut stack, &mut kids)?;
                // Spacing before the operator's symbol follows the left operand.
                let before = std::mem::take(&mut ws.before);
                kids = kids.saturating_add(usize::from(!before.is_empty()));
                let a = spaced_after(a, before);
                push(&mut stack, kids, Expr::Binary(op, Box::new(a), Box::new(b)))?;
            }
            0x12..=0x14 => {
                let op = match id {
                    0x12 => UnOp::Plus,
                    0x13 => UnOp::Neg,
                    _ => UnOp::Percent,
                };
                let x = pop(&mut stack, &mut kids)?;
                // A sign's spacing comes before it; `%`'s before its symbol, after the operand.
                let before = std::mem::take(&mut ws.before);
                let e = if op == UnOp::Percent {
                    kids = kids.saturating_add(usize::from(!before.is_empty()));
                    Expr::Unary(op, Box::new(spaced_after(x, before)))
                } else {
                    spaced_before(Expr::Unary(op, Box::new(x)), before)
                };
                push(&mut stack, kids, e)?;
            }
            0x15 => {
                let x = pop(&mut stack, &mut kids)?;
                let close = std::mem::take(&mut ws.close);
                kids = kids.saturating_add(usize::from(!close.is_empty()));
                let x = spaced_after(x, close);
                let before = std::mem::take(&mut ws.open) + &std::mem::take(&mut ws.before);
                push(&mut stack, kids, spaced_before(Expr::Paren(Box::new(x)), before))?;
            }
            0x16 => push(&mut stack, kids, Expr::Missing)?,
            // PtgStr: cch (u16, at most 255) and always UTF-16, unlike BIFF8.
            0x17 => {
                let cch = usize::from(t.u16()?);
                if cch > 255 {
                    return Err(Unsupported("text constant too long"));
                }
                push(&mut stack, kids, Expr::Text(Arc::from(t.chars(cch)?)))?;
            }
            0x19 => {
                let kind = t.u8()?;
                if kind & 0x04 != 0 {
                    // PtgAttrChoose: jump table of cOffset + 1 entries.
                    let n = usize::from(t.u16()?) + 1;
                    t.take(n * 2)?;
                } else {
                    let data = t.take(2)?;
                    if kind & 0x40 != 0 {
                        // PtgAttrSpace / PtgAttrSpaceSemi: PtgAttrSpaceType's `type` and `cch`.
                        let (kind, n) = (data.first().copied().unwrap_or(0), data.get(1).copied().unwrap_or(0));
                        let text = if matches!(kind, 1 | 3 | 5) { "\n" } else { " " }.repeat(usize::from(n));
                        match kind {
                            0 | 1 | 6 => ws.before.push_str(&text),
                            2 | 3 => ws.open.push_str(&text),
                            4 | 5 => ws.close.push_str(&text),
                            _ => return Err(Unsupported("unknown spacing in a formula")),
                        }
                    }
                }
                if kind & 0x10 != 0 {
                    // PtgAttrSum: SUM of the single operand.
                    let x = pop(&mut stack, &mut kids)?;
                    let args = close_args(vec![x], std::mem::take(&mut ws.close));
                    let call = spaced_before(Expr::Call("SUM".into(), args), std::mem::take(&mut ws.before));
                    push(&mut stack, kids.saturating_add(1), call)?;
                }
                // Semi (volatile), If, Goto, Baxcel: no effect on the expression.
            }
            0x1C => {
                let e = t.u8()?;
                push(&mut stack, kids, Expr::Error(error_code(e).ok_or(Unsupported("unknown error constant"))?))?;
            }
            0x1D => push(&mut stack, kids, Expr::Bool(t.u8()? != 0))?,
            0x1E => push(&mut stack, kids, Expr::Number(f64::from(t.u16()?)))?,
            0x1F => {
                let n = t.f64()?;
                push(&mut stack, kids, Expr::Number(n))?;
            }
            0x20 => {
                // PtgArray: 14 unused bytes (Excel leaves junk in them); the values are in rgcb.
                t.take(14)?;
                push(&mut stack, kids, array_constant(&mut extra, &mut array_values)?)?;
            }
            0x21 => {
                let iftab = t.u16()?;
                let (name, args) = function(iftab).ok_or(Unsupported("unknown function"))?;
                let n = usize::try_from(args).map_err(|_| Unsupported("unknown function argument count"))?;
                let args = close_args(pop_n(&mut stack, n, &mut kids)?, std::mem::take(&mut ws.close));
                let call = spaced_before(Expr::Call(name.to_string(), args), std::mem::take(&mut ws.before));
                push(&mut stack, kids.saturating_add(1), call)?;
            }
            0x22 => {
                let n = usize::from(t.u8()?);
                let tab = t.u16()?;
                if tab & 0x8000 != 0 {
                    return Err(Unsupported("macro sheet commands"));
                }
                let mut args = pop_n(&mut stack, n, &mut kids)?;
                if tab & 0x7FFF == 0x00FF {
                    // A function called by name (a newer function, add-in or LAMBDA): the name
                    // comes first, as a PtgName to a hidden `_xlfn.` name for a newer function.
                    if args.is_empty() {
                        return Err(Unsupported("malformed function call"));
                    }
                    // Spacing typed after the `(` comes before the name's token: it belongs to the
                    // first argument.
                    let (lead, first) = match args.remove(0) {
                        Expr::Spaced(before, x, _) => (before, *x),
                        e => (String::new(), e),
                    };
                    let name = match first {
                        Expr::Name(n) => n,
                        _ => return Err(Unsupported("function call through an expression")),
                    };
                    if let Some(a) = args.first_mut() {
                        *a = spaced_before(std::mem::replace(a, Expr::Missing), lead);
                    }
                    // `PY(index, …)` runs a script stored elsewhere in the package, which this
                    // import doesn't read. `PY("code", …)` carries its code and becomes a Python cell.
                    let py = name.rsplit('.').next().is_some_and(|n| n.eq_ignore_ascii_case("PY"));
                    if py && !args.first().is_some_and(literal_text) {
                        return Err(Unsupported("Python code stored outside the cell"));
                    }
                    let args = close_args(args, std::mem::take(&mut ws.close));
                    let call = spaced_before(Expr::Call(bare_function(name), args), std::mem::take(&mut ws.before));
                    push(&mut stack, kids.saturating_add(1), call)?;
                } else {
                    let (name, _) = function(tab & 0x7FFF).ok_or(Unsupported("unknown function"))?;
                    let args = close_args(args, std::mem::take(&mut ws.close));
                    let call = spaced_before(Expr::Call(name.to_string(), args), std::mem::take(&mut ws.before));
                    push(&mut stack, kids.saturating_add(1), call)?;
                }
            }
            0x23 => {
                // PtgName: a 1-based index into every BrtName, hidden ones included.
                let index = t.u32()?;
                let name = links.name(index, mode)?;
                used.extend(usize::try_from(index).ok().and_then(|i| i.checked_sub(1)));
                push(&mut stack, kids, Expr::Name(name.text.clone()))?;
            }
            0x24 => {
                let (r, c) = (t.u32()?, t.u16()?);
                push(&mut stack, kids, Expr::Ref(Reference { sheet: SheetSel::Current, kind: RefKind::Cell(anchor(r, c)?) }))?;
            }
            0x25 => {
                let (r0, r1, c0, c1) = (t.u32()?, t.u32()?, t.u16()?, t.u16()?);
                push(&mut stack, kids, Expr::Ref(Reference { sheet: SheetSel::Current, kind: area(r0, r1, c0, c1)? }))?;
            }
            0x26 => {
                // PtgMemArea: a cache of the sub-expression that follows. Its rgcb entry
                // (PtgExtraMem) is a count and 16 bytes per cached area: skip both.
                t.take(6)?;
                let n = usize::try_from(extra.u32()?).map_err(|_| TRUNC)?;
                extra.take(n.checked_mul(16).ok_or(TRUNC)?)?;
            }
            // PtgMemErr, PtgMemNoMem: 6 bytes; PtgMemFunc: 2. The sub-expression follows them.
            0x27 | 0x28 => {
                t.take(6)?;
            }
            0x29 => {
                t.take(2)?;
            }
            0x2A => {
                t.take(6)?;
                push(&mut stack, kids, Expr::Error(CellError::Ref))?;
            }
            0x2B => {
                t.take(12)?;
                push(&mut stack, kids, Expr::Error(CellError::Ref))?;
            }
            0x39 => {
                // PtgNameX: a name through a supporting link. Through this workbook's own link
                // it is a sheet-scoped name used from another sheet, which `.xlsx` spells
                // `'Other Sheet'!Local`: Excel's files show it, [MS-XLSB] doesn't say.
                let ixti = t.u16()?;
                let index = t.u32()?;
                links.this_book(ixti)?;
                let name = links.name(index, mode)?;
                used.extend(usize::try_from(index).ok().and_then(|i| i.checked_sub(1)));
                let sheet = name.scope.and_then(|s| links.sheets.get(usize::try_from(s).ok()?).cloned().flatten());
                let sheet = sheet.ok_or(Unsupported("names in other workbooks or on other sheets"))?;
                push(&mut stack, kids, Expr::Name(format!("{sheet}!{}", name.text)))?;
            }
            0x3A => {
                let ixti = t.u16()?;
                let (r, c) = (t.u32()?, t.u16()?);
                let kind = RefKind::Cell(loc3d(r, c, mode)?);
                push(&mut stack, kids, reference(links.sheet_sel(ixti)?, kind))?;
            }
            0x3B => {
                let ixti = t.u16()?;
                let (r0, r1, c0, c1) = (t.u32()?, t.u32()?, t.u16()?, t.u16()?);
                let kind = area_of(loc3d(r0, c0, mode)?, loc3d(r1, c1, mode)?)?;
                push(&mut stack, kids, reference(links.sheet_sel(ixti)?, kind))?;
            }
            0x3C | 0x3D => {
                // PtgRefErr3d, PtgAreaErr3d: a deleted cell or range, which `.xlsx` spells
                // `Sheet2!#REF!` (`#REF!` when the sheet is gone too). The sheet link must still
                // be one the reader follows: an external #REF! stays cached.
                let ixti = t.u16()?;
                t.take(if id == 0x3C { 6 } else { 12 })?;
                push(&mut stack, kids, reference(links.sheet_sel(ixti)?, RefKind::Deleted))?;
            }
            0x01 => return Err(Unsupported("the shared or array formula form")),
            0x02 => return Err(Unsupported("what-if data tables")),
            0x18 => return Err(Unsupported("structured table references")),
            0x2C => {
                if mode == Mode::Cell {
                    return Err(Unsupported("relative references in a cell formula"));
                }
                let (r, c) = (t.u32()?, t.u16()?);
                push(&mut stack, kids, Expr::Ref(Reference { sheet: SheetSel::Current, kind: RefKind::Cell(loc(r, c, mode)?) }))?;
            }
            0x2D => {
                if mode == Mode::Cell {
                    return Err(Unsupported("relative references in a cell formula"));
                }
                let (r0, r1, c0, c1) = (t.u32()?, t.u32()?, t.u16()?, t.u16()?);
                let kind = area_of(loc(r0, c0, mode)?, loc(r1, c1, mode)?)?;
                push(&mut stack, kids, Expr::Ref(Reference { sheet: SheetSel::Current, kind }))?;
            }
            _ => return Err(Unsupported("unknown formula token")),
        }
        if operand && !ws.before.is_empty() && stack.len() == height + 1 {
            let (e, depth) = stack.pop().ok_or(Unsupported("malformed formula (operand missing)"))?;
            push(&mut stack, depth, spaced_before(e, std::mem::take(&mut ws.before)))?;
        }
        if stack.len() > MAX_RGCE {
            return Err(Unsupported("formula too complex"));
        }
    }
    let (e, _) = stack.pop().ok_or(Unsupported("malformed formula (operand missing)"))?;
    if !stack.is_empty() {
        return Err(Unsupported("malformed formula (unused operands)"));
    }
    Ok(grouped(e, false))
}

/// The name a function called through a hidden name is written with in `.xlsx`: that name
/// (`_xlfn.CONCAT`), except for the four ISO/IEC 29500 functions, which `.xlsx` names without the
/// prefix though `.xlsb` calls them as it calls newer functions.
fn bare_function(name: String) -> String {
    const ISO: [&str; 4] = ["ISO.CEILING", "ECMA.CEILING", "NETWORKDAYS.INTL", "WORKDAY.INTL"];
    match name.get(..6).filter(|p| p.eq_ignore_ascii_case("_xlfn.")).and(name.get(6..)) {
        Some(bare) if ISO.iter().any(|f| f.eq_ignore_ascii_case(bare)) => bare.to_string(),
        _ => name,
    }
}

/// Text written out in the formula: a string, or strings joined with `&` (how Python code
/// longer than a string constant is written).
fn literal_text(e: &Expr) -> bool {
    match e {
        Expr::Text(_) => true,
        Expr::Spaced(_, x, _) => literal_text(x),
        Expr::Binary(BinOp::Concat, a, b) => literal_text(a) && literal_text(b),
        _ => false,
    }
}

/// How many strings a [`literal_text`] chain joins, or `None` for anything else. It accepts
/// what [`literal_text`] does, typed spacing included, so a chain the Python check accepts is
/// one that is written flat.
fn literal_texts(e: &Expr) -> Option<usize> {
    match e {
        Expr::Text(_) => Some(1),
        Expr::Spaced(_, x, _) => literal_texts(x),
        Expr::Binary(BinOp::Concat, a, b) => literal_texts(a)?.checked_add(literal_texts(b)?),
        _ => None,
    }
}

/// How tightly an expression binds when printed, as the formula parser reads it: binary
/// operators by [`BinOp::precedence`], prefix `+`/`-` at 60 and `%` at 70 (`-2^2` is `(-2)^2`);
/// anything else is one operand.
fn binding(e: &Expr) -> u8 {
    match e {
        Expr::Binary(op, ..) => op.precedence(),
        Expr::Unary(UnOp::Neg | UnOp::Plus, _) => 60,
        Expr::Unary(UnOp::Percent, _) => 70,
        Expr::Spaced(_, x, _) => binding(x),
        _ => u8::MAX,
    }
}

/// The strings of a [`literal_text`] chain, left to right. Spacing typed around a part of the
/// chain stays with that part's first and last strings, where it is written.
fn flatten_texts(e: Expr, out: &mut Vec<Expr>) {
    match e {
        Expr::Binary(BinOp::Concat, a, b) => {
            flatten_texts(*a, out);
            flatten_texts(*b, out);
        }
        Expr::Spaced(before, x, after) if matches!(*x, Expr::Binary(BinOp::Concat, ..)) => {
            let start = out.len();
            flatten_texts(*x, out);
            if let Some(first) = out.get_mut(start) {
                *first = spaced_before(std::mem::replace(first, Expr::Missing), before);
            }
            if let Some(last) = out.last_mut() {
                *last = spaced_after(std::mem::replace(last, Expr::Missing), after);
            }
        }
        text => out.push(text),
    }
}

fn paren(e: Expr) -> Expr {
    Expr::Paren(Box::new(e))
}

/// The tree with the parentheses its text needs to mean the same. Tokens are reverse Polish, so
/// grouping lives in their order: Excel adds `PtgParen` wherever its text has parentheses, but
/// `1 2 + 3 *` (another writer's, or a crafted file's) would print as `1+2*3`. Binary operators
/// are left-associative; a union in a function's arguments needs parentheses of its own.
/// Recursion follows the decoded tree, which `push` bounds at `MAX_DEPTH` levels.
fn grouped(e: Expr, in_args: bool) -> Expr {
    match e {
        // Strings joined with `&` are written as one flat chain, however their tokens nest: `&`
        // is associative, and `PY("…"&"…")` is read as Python code only in that form. The chain
        // nests one level per string, so it is made only up to `MAX_DEPTH` strings, the bound
        // `push` keeps on the decoded tree (Excel's 8,192 characters hold at most about 33
        // full-length constants). A longer one prints as its tokens nest, with the parentheses
        // that needs: a balanced tree of thousands of strings is only a few levels deep, and
        // printing or dropping a chain of thousands would recurse past the stack.
        e @ Expr::Binary(BinOp::Concat, ..) if literal_texts(&e).is_some_and(|n| n <= MAX_DEPTH) => {
            let mut texts = vec![];
            flatten_texts(e, &mut texts);
            let mut texts = texts.into_iter();
            let first = texts.next().unwrap_or(Expr::Missing);
            texts.fold(first, |a, b| Expr::Binary(BinOp::Concat, Box::new(a), Box::new(b)))
        }
        Expr::Binary(op, a, b) => {
            let (a, b) = (grouped(*a, in_args), grouped(*b, in_args));
            let a = if binding(&a) < op.precedence() { paren(a) } else { a };
            let b = if binding(&b) <= op.precedence() { paren(b) } else { b };
            let e = Expr::Binary(op, Box::new(a), Box::new(b));
            if in_args && op == BinOp::Union { paren(e) } else { e }
        }
        Expr::Unary(op, x) => {
            let x = grouped(*x, in_args);
            // Each of `+`, `-` and `%` takes an operand that binds at least as tightly as a sign. The
            // parser reads `-1%` as `-(1%)`, Excel's tokens as `(-1)%`: the same value, so a sign
            // under `%` keeps Excel's spelling.
            let needs = binding(&x) < 60;
            Expr::Unary(op, Box::new(if needs { paren(x) } else { x }))
        }
        Expr::Paren(x) => paren(grouped(*x, false)),
        Expr::Spaced(before, x, after) => Expr::Spaced(before, Box::new(grouped(*x, in_args)), after),
        Expr::Call(name, args) => Expr::Call(name, args.into_iter().map(|a| grouped(a, true)).collect()),
        other => other,
    }
}

/// PtgExtraArray ([MS-XLSB] 2.5.98.41): row and column counts, then `SerAr` values in row-major
/// order. The type codes are not BIFF8's: 0x00 is a number here, not a missing value.
fn array_constant(b: &mut Bytes<'_>, left: &mut usize) -> Result<Expr, Unsupported> {
    let rows = usize::try_from(b.u32()?).map_err(|_| TRUNC)?;
    let cols = usize::try_from(b.u32()?).map_err(|_| TRUNC)?;
    let values = rows.saturating_mul(cols);
    if values == 0 {
        return Err(Unsupported("empty array constant"));
    }
    // Charged before the rows are allocated, against what the formula's arrays may hold in all.
    *left = left.checked_sub(values).ok_or(Unsupported("array constants too large"))?;
    let mut out = Vec::with_capacity(rows);
    for _ in 0..rows {
        let mut row = Vec::with_capacity(cols);
        for _ in 0..cols {
            row.push(match b.u8()? {
                0x00 => Expr::Number(b.f64()?),
                0x01 => {
                    let cch = usize::from(b.u16()?);
                    if cch > 255 {
                        return Err(Unsupported("text constant too long"));
                    }
                    Expr::Text(Arc::from(b.chars(cch)?))
                }
                0x02 => Expr::Bool(b.u8()? != 0),
                0x04 => {
                    let e = b.u8()?;
                    b.take(3)?;
                    Expr::Error(error_code(e).ok_or(Unsupported("unknown error constant"))?)
                }
                _ => return Err(Unsupported("unknown array constant value")),
            });
        }
        out.push(row);
    }
    Ok(Expr::Array(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Postfix tokens for `n` empty strings joined by `&` as a balanced tree.
    fn balanced_strings(n: usize, out: &mut Vec<u8>) {
        if n == 1 {
            out.extend([0x17, 0, 0]);
        } else {
            balanced_strings(n / 2, out);
            balanced_strings(n - n / 2, out);
            out.push(0x08);
        }
    }

    #[test]
    fn a_spaced_part_of_a_string_chain_is_written_flat() {
        // `"x"`, then `"="` typed with a space before it joined to `"1"`, then joined: the tokens
        // nest to the right, and the space is on the inner chain. Written flat, as Python code
        // must be, with the space where it was typed.
        let s = |c: u8| [0x17, 1, 0, c, 0];
        let rgce = [&s(b'x')[..], &[0x19, 0x40, 0, 1], &s(b'=')[..], &s(b'1')[..], &[0x08, 0x08]].concat();
        let e = decode(&rgce, &[], &Links::default(), Mode::Cell).unwrap();
        assert_eq!(super::super::expr::print(&e), "\"x\"& \"=\"&\"1\"");
        assert!(literal_text(&e) && literal_texts(&e) == Some(3));
    }

    #[test]
    fn a_long_chain_of_strings_stays_within_the_depth_bound() {
        // 4,096 strings and 4,095 `&` fill 16,383 token bytes, 13 levels deep as written. Made
        // into one flat chain they would be 4,096 levels deep, past what printing and dropping
        // the tree may recurse; on a small stack that overflows. They print as written instead.
        let mut rgce = vec![];
        balanced_strings(4096, &mut rgce);
        assert!(rgce.len() <= MAX_RGCE);
        let text = std::thread::Builder::new()
            .stack_size(256 * 1024)
            .spawn(move || {
                let e = decode(&rgce, &[], &Links::default(), Mode::Cell).unwrap();
                let text = super::super::expr::print(&e);
                drop(e);
                text
            })
            .unwrap()
            .join()
            .unwrap();
        assert_eq!(text.matches("\"\"").count(), 4096);
        // A short chain, such as Python code split into constants, is still one flat chain.
        let mut short = vec![];
        balanced_strings(4, &mut short);
        let e = decode(&short, &[], &Links::default(), Mode::Cell).unwrap();
        assert_eq!(super::super::expr::print(&e), "\"\"&\"\"&\"\"&\"\"");
    }
}
