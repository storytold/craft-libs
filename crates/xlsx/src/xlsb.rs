//! Binary workbooks ([MS-XLSB]): the workbook part, shared strings and worksheet cells, from
//! their records. Saved values only: formulas are token streams that are not decoded, and
//! styles are not read. A record that does not parse fails the read.

use std::sync::Arc;

use craft_opc::{ContentTypes, Rels};
use craft_sheet::{CellError, CellRef, MAX_COLS, MAX_ROWS, Range, Visibility};

use crate::sheet::{Cell, CellValue, Row, SheetPart};
use crate::strings::StringItem;
use crate::workbook::{SheetEntry, SheetKind, Workbook};
use crate::{Error, Limits, Warnings};

pub(crate) const MAIN: &str = "application/vnd.ms-excel.main";
const BINARY_MAIN: &str = "application/vnd.ms-excel.sheet.binary.macroEnabled.main";
pub(crate) const WORKSHEET: &str = "application/vnd.ms-excel.worksheet";
pub(crate) const STRINGS: &str = "application/vnd.ms-excel.sharedStrings";
/// Most sheets in the workbook.
const MAX_SHEETS: usize = 1024;
/// Most merged ranges in one worksheet.
const MAX_MERGES: usize = 100_000;

pub(crate) fn invalid(msg: impl Into<String>) -> Error {
    Error::Invalid(format!("XLSB: {}", msg.into()))
}

/// Is `part`'s content type `expected`? The workbook part may also have the macro-enabled type.
pub(crate) fn check_type(types: &ContentTypes, part: &str, expected: &str) -> Result<(), Error> {
    let actual = types.of(part);
    if actual != Some(expected) && !(expected == MAIN && actual == Some(BINARY_MAIN)) {
        return Err(invalid(format!("wrong or missing content type for {part}")));
    }
    Ok(())
}

/// What the workbook has used of [`Limits`], over all its parts. Every part's [`Records`]
/// charges the same record allowance while cells and text are charged, hence the `Cell`s.
#[derive(Default)]
pub(crate) struct Budget {
    cells: std::cell::Cell<usize>,
    text: std::cell::Cell<usize>,
    records: std::cell::Cell<usize>,
}

impl Budget {
    fn text(&self, len: usize, limits: &Limits) -> Result<(), Error> {
        self.text.set(self.text.get().saturating_add(len));
        if self.text.get() > limits.binary_text {
            return Err(Error::TooLarge(format!("XLSB decoded text exceeds {} MiB", limits.binary_text >> 20)));
        }
        Ok(())
    }

    fn cell(&self, limits: &Limits) -> Result<(), Error> {
        self.cells.set(self.cells.get().saturating_add(1));
        if self.cells.get() > limits.binary_cells {
            return Err(Error::TooLarge(format!("XLSB exceeds {} imported cells", limits.binary_cells)));
        }
        Ok(())
    }

    fn record(&self, limits: &Limits) -> Result<(), Error> {
        self.records.set(self.records.get().saturating_add(1));
        if self.records.get() > limits.binary_records {
            return Err(Error::TooLarge(format!("XLSB workbook contains too many records (more than {})", limits.binary_records)));
        }
        Ok(())
    }
}

struct Bytes<'a> {
    rest: &'a [u8],
}

impl<'a> Bytes<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { rest: bytes }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], Error> {
        let out = self.rest.get(..n).ok_or_else(|| invalid("truncated record or string"))?;
        self.rest = self.rest.get(n..).ok_or_else(|| invalid("truncated record"))?;
        Ok(out)
    }

    fn u8(&mut self) -> Result<u8, Error> {
        self.take(1)?.first().copied().ok_or_else(|| invalid("truncated byte"))
    }

    fn u16(&mut self) -> Result<u16, Error> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().map_err(|_| invalid("truncated u16"))?))
    }

    fn u32(&mut self) -> Result<u32, Error> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().map_err(|_| invalid("truncated u32"))?))
    }

    fn f64(&mut self) -> Result<f64, Error> {
        let n = f64::from_le_bytes(self.take(8)?.try_into().map_err(|_| invalid("truncated number"))?);
        if !n.is_finite() {
            return Err(invalid("nonfinite numeric value"));
        }
        Ok(n)
    }

    /// An `XLWideString` of at most `max` UTF-16 units; `None` for the null string when
    /// `nullable`.
    fn string(&mut self, max: u32, nullable: bool) -> Result<Option<String>, Error> {
        let count = self.u32()?;
        if nullable && count == u32::MAX {
            return Ok(None);
        }
        if count > max {
            return Err(Error::TooLarge("XLSB string length exceeds its limit".into()));
        }
        let size = (count as usize).checked_mul(2).ok_or_else(|| invalid("string length overflow"))?;
        let bytes = self.take(size)?;
        let mut text = String::new();
        for ch in char::decode_utf16(bytes.as_chunks::<2>().0.iter().copied().map(u16::from_le_bytes)) {
            text.push(ch.map_err(|_| invalid("invalid UTF-16 string"))?);
        }
        Ok(Some(text))
    }

    fn text(&mut self, max: u32) -> Result<String, Error> {
        self.string(max, false)?.ok_or_else(|| invalid("null string"))
    }

    fn finish(self) -> Result<(), Error> {
        if !self.rest.is_empty() {
            return Err(invalid("unexpected bytes after record payload"));
        }
        Ok(())
    }
}

/// The records of one part, with future-record and alternate-content blocks skipped.
struct Records<'a, 'l> {
    bytes: Bytes<'a>,
    limits: &'l Limits,
    budget: &'l Budget,
    blocks: Vec<u16>,
}

impl<'a, 'l> Records<'a, 'l> {
    fn new(bytes: &'a [u8], limits: &'l Limits, budget: &'l Budget) -> Self {
        Self { bytes: Bytes::new(bytes), limits, budget, blocks: vec![] }
    }

    fn next(&mut self, warnings: &mut Warnings) -> Result<Option<(u16, &'a [u8])>, Error> {
        while !self.bytes.rest.is_empty() {
            self.budget.record(self.limits)?;
            let first = self.bytes.u8()?;
            let id = if first & 0x80 == 0 {
                u16::from(first)
            } else {
                let id = u16::from(first & 0x7f) | (u16::from(self.bytes.u8()? & 0x7f) << 7);
                if id < 128 {
                    return Err(invalid("invalid two-byte record type"));
                }
                id
            };
            let mut size = 0usize;
            for shift in [0, 7, 14, 21] {
                let b = self.bytes.u8()?;
                size |= usize::from(b & 0x7f) << shift;
                if b & 0x80 == 0 || shift == 21 {
                    break;
                }
            }
            let payload = self.bytes.take(size)?;
            match id {
                // BrtFRTBegin, BrtACBegin.
                35 | 37 => {
                    let mut b = Bytes::new(payload);
                    if id == 35 {
                        b.take(4)?;
                    } else {
                        let count = b.u16()?;
                        if count == 0 {
                            return Err(invalid("empty alternate-content version list"));
                        }
                        b.take(usize::from(count) * 4)?;
                    }
                    b.finish()?;
                    if self.blocks.len() >= 64 {
                        return Err(invalid("extension blocks nested too deeply"));
                    }
                    self.blocks.push(id + 1);
                    warnings.warn("unsupported XLSB future-record and alternate-content blocks were skipped");
                }
                // BrtFRTEnd, BrtACEnd.
                36 | 38 => {
                    if self.blocks.pop() != Some(id) || !payload.is_empty() {
                        return Err(invalid("mismatched extension block end"));
                    }
                }
                _ if !self.blocks.is_empty() => {}
                _ => return Ok(Some((id, payload))),
            }
        }
        if !self.blocks.is_empty() {
            return Err(invalid("unterminated extension block"));
        }
        Ok(None)
    }

    fn first(&mut self, id: u16, warnings: &mut Warnings) -> Result<(), Error> {
        if self.next(warnings)? != Some((id, &[])) {
            return Err(invalid("missing part begin record"));
        }
        Ok(())
    }

    fn end(&mut self, payload: &[u8], warnings: &mut Warnings) -> Result<(), Error> {
        if !payload.is_empty() || self.next(warnings)?.is_some() {
            return Err(invalid("records after part end"));
        }
        Ok(())
    }
}

/// The workbook part: BrtBeginBook, BrtWbProp, the BrtBundleSh list.
pub(crate) fn workbook(bytes: &[u8], rels: &Rels, limits: &Limits, budget: &Budget, warnings: &mut Warnings) -> Result<Workbook, Error> {
    let mut records = Records::new(bytes, limits, budget);
    records.first(131, warnings)?;
    let mut wb = Workbook::default();
    let mut bundle = false;
    let mut bundle_seen = false;
    while let Some((id, payload)) = records.next(warnings)? {
        let mut b = Bytes::new(payload);
        match id {
            132 => {
                if bundle || !bundle_seen {
                    return Err(invalid("incomplete worksheet list"));
                }
                records.end(payload, warnings)?;
                if wb.sheets.is_empty() {
                    return Err(invalid("no supported worksheets"));
                }
                return Ok(wb);
            }
            143 => {
                if bundle_seen {
                    return Err(invalid("duplicate worksheet list"));
                }
                bundle_seen = true;
                bundle = true;
                b.finish()?;
            }
            144 => {
                if !bundle {
                    return Err(invalid("unexpected worksheet list end"));
                }
                bundle = false;
                b.finish()?;
            }
            153 => {
                wb.date_1904 = b.u32()? & 1 != 0;
                b.u32()?;
                b.text(31)?;
                b.finish()?;
            }
            156 => {
                if !bundle {
                    return Err(invalid("worksheet outside the worksheet list"));
                }
                if wb.sheets.len() >= MAX_SHEETS {
                    return Err(Error::TooLarge(format!("XLSB exceeds {MAX_SHEETS} sheets")));
                }
                let visibility = match b.u32()? {
                    0 => Visibility::Visible,
                    1 => Visibility::Hidden,
                    2 => Visibility::VeryHidden,
                    _ => return Err(invalid("invalid sheet visibility")),
                };
                let sheet_id = b.u32()?;
                if !(1..=65535).contains(&sheet_id) {
                    return Err(invalid("invalid worksheet ID"));
                }
                let rel_id = b.string(255, true)?;
                let name = b.text(31)?;
                b.finish()?;
                let Some(rel_id) = rel_id else {
                    if visibility != Visibility::VeryHidden {
                        return Err(invalid("worksheet relationship is null"));
                    }
                    warnings.warn("XLSB module sheets were skipped");
                    continue;
                };
                let rel = rels.by_id(&rel_id).ok_or_else(|| invalid("worksheet relationship is missing"))?;
                if rel.external {
                    return Err(invalid("external sheet relationship"));
                }
                let kind = SheetKind::of(rel.kind());
                if let SheetKind::Other(_) = kind {
                    return Err(invalid("unknown sheet relationship type"));
                }
                wb.sheets.push(SheetEntry { name, sheet_id, visibility, rel_id, kind, part: Some(rel.target.clone()) });
            }
            _ => {}
        }
    }
    Err(invalid("missing workbook end record"))
}

/// A `RichStr`: the text; formatting and phonetic runs are skipped.
fn rich_string(b: &mut Bytes<'_>) -> Result<String, Error> {
    let flags = b.u8()?;
    let text = b.text(32767)?;
    if flags & 1 != 0 {
        let count = b.u32()?;
        if count > 32767 {
            return Err(invalid("too many rich-text runs"));
        }
        b.take(count as usize * 4)?;
    }
    if flags & 2 != 0 {
        b.text(32767)?;
        let count = b.u32()?;
        if count > 32767 {
            return Err(invalid("too many phonetic runs"));
        }
        b.take(count as usize * 10)?;
        // Writers pad the phonetic data; nothing after it is read.
        b.rest = &[];
    }
    Ok(text)
}

fn item(text: String) -> StringItem {
    StringItem { text: Arc::from(text), ..StringItem::default() }
}

/// The shared strings part: BrtBeginSst, BrtSSTItem, BrtEndSst.
pub(crate) fn shared_strings(bytes: &[u8], limits: &Limits, budget: &Budget, warnings: &mut Warnings) -> Result<Vec<StringItem>, Error> {
    let mut records = Records::new(bytes, limits, budget);
    let Some((159, payload)) = records.next(warnings)? else {
        return Err(invalid("missing shared-string begin record"));
    };
    let mut header = Bytes::new(payload);
    let total = header.u32()?;
    let unique = header.u32()?;
    header.finish()?;
    // Each item is kept with its own allocation: no more than there are cells to show them.
    if unique > total || unique as usize > limits.binary_cells {
        return Err(Error::TooLarge("invalid or oversized shared-string count".into()));
    }
    let mut strings = vec![];
    while let Some((id, payload)) = records.next(warnings)? {
        match id {
            160 => {
                records.end(payload, warnings)?;
                if strings.len() != unique as usize {
                    return Err(invalid("shared-string count does not match records"));
                }
                return Ok(strings);
            }
            19 => {
                if strings.len() >= unique as usize {
                    return Err(invalid("too many shared-string items"));
                }
                let mut b = Bytes::new(payload);
                let text = rich_string(&mut b)?;
                b.finish()?;
                budget.text(text.len(), limits)?;
                strings.push(item(text));
            }
            _ => return Err(invalid("unexpected shared-string record")),
        }
    }
    Err(invalid("missing shared-string end record"))
}

fn error(code: u8) -> Result<CellError, Error> {
    Ok(match code {
        0x00 => CellError::Null,
        0x07 => CellError::Div0,
        0x0f => CellError::Value,
        0x17 => CellError::Ref,
        0x1d => CellError::Name,
        0x24 => CellError::Num,
        0x2a => CellError::NA,
        0x2b => CellError::GettingData,
        _ => return Err(invalid("unknown cell error code")),
    })
}

fn boolean(b: &mut Bytes<'_>) -> Result<bool, Error> {
    match b.u8()? {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(invalid("invalid boolean value")),
    }
}

/// The formula after a cell's saved result: checked for framing, not decoded.
fn skip_formula(b: &mut Bytes<'_>) -> Result<(), Error> {
    b.take(2)?;
    let cce = b.u32()?;
    if !(1..=16384).contains(&cce) {
        return Err(invalid("invalid formula token length"));
    }
    b.take(cce as usize)?;
    let extra = b.u32()?;
    b.take(extra as usize)?;
    Ok(())
}

fn value(id: u16, b: &mut Bytes<'_>, strings: usize, limits: &Limits, budget: &Budget) -> Result<CellValue, Error> {
    let value = match id {
        1 => CellValue::Empty,
        2 => {
            let rk = b.u32()?;
            let n = if rk & 2 != 0 { f64::from((rk as i32) >> 2) } else { f64::from_bits(u64::from(rk & !3) << 32) };
            let n = if rk & 1 != 0 { n / 100.0 } else { n };
            if !n.is_finite() {
                return Err(invalid("nonfinite RK number"));
            }
            CellValue::Number(n)
        }
        3 | 11 => CellValue::Error(error(b.u8()?)?),
        4 | 10 => CellValue::Bool(boolean(b)?),
        5 | 9 => CellValue::Number(b.f64()?),
        6 | 8 => {
            let s = b.text(32767)?;
            budget.text(s.len(), limits)?;
            CellValue::Text(Arc::from(s))
        }
        62 => {
            let s = rich_string(b)?;
            budget.text(s.len(), limits)?;
            CellValue::Inline(item(s))
        }
        7 => {
            let i = b.u32()?;
            if i as usize >= strings {
                return Err(invalid("shared-string index is out of bounds"));
            }
            CellValue::Shared(i)
        }
        _ => return Err(invalid("unsupported cell record")),
    };
    if (8..=11).contains(&id) {
        skip_formula(b)?;
    }
    Ok(value)
}

/// A worksheet part: rows (BrtRowHdr) and cells inside BrtBeginSheetData, merged cells after.
pub(crate) fn worksheet(
    bytes: &[u8],
    strings: usize,
    limits: &Limits,
    budget: &Budget,
    warnings: &mut Warnings,
    each: &mut dyn FnMut(Row) -> Result<(), Error>,
) -> Result<SheetPart, Error> {
    let mut records = Records::new(bytes, limits, budget);
    records.first(129, warnings)?;
    let mut sheet = SheetPart::default();
    let mut data = false;
    let mut data_seen = false;
    let mut row: Option<Row> = None;
    let mut col = None;
    while let Some((id, payload)) = records.next(warnings)? {
        let mut b = Bytes::new(payload);
        match id {
            130 => {
                if data || !data_seen {
                    return Err(invalid("incomplete worksheet data"));
                }
                records.end(payload, warnings)?;
                return Ok(sheet);
            }
            145 => {
                if data_seen {
                    return Err(invalid("duplicate worksheet data"));
                }
                data_seen = true;
                data = true;
                b.finish()?;
            }
            146 => {
                if !data {
                    return Err(invalid("unexpected worksheet data end"));
                }
                data = false;
                b.finish()?;
                if let Some(r) = row.take() {
                    each(r)?;
                }
            }
            0 => {
                if !data {
                    return Err(invalid("row outside worksheet data"));
                }
                let r = b.u32()?;
                if r >= MAX_ROWS || row.as_ref().is_some_and(|previous| r <= previous.index) {
                    return Err(invalid("row is out of bounds or unordered"));
                }
                // ixfe, miyRw and the flags are not read.
                b.take(9)?;
                let spans = b.u32()?;
                if spans > 16 {
                    return Err(invalid("too many row spans"));
                }
                b.take(spans as usize * 8)?;
                b.finish()?;
                if let Some(previous) = row.replace(Row { index: r, ..Row::default() }) {
                    each(previous)?;
                }
                sheet.rows += 1;
                col = None;
            }
            1..=11 | 62 => {
                if !data {
                    return Err(invalid("cell outside worksheet data"));
                }
                let current = row.as_mut().ok_or_else(|| invalid("cell before row header"))?;
                let c = b.u32()?;
                if c >= MAX_COLS || col.is_some_and(|previous| c <= previous) {
                    return Err(invalid("column is out of bounds or unordered"));
                }
                let style = b.u32()?;
                let value = value(id, &mut b, strings, limits, budget)?;
                b.finish()?;
                if value != CellValue::Empty {
                    budget.cell(limits)?;
                }
                current.cells.push(Cell {
                    at: CellRef::new(current.index, c),
                    style: style & 0x00FF_FFFF,
                    phonetic: style & 0x0100_0000 != 0,
                    value,
                    ..Cell::default()
                });
                col = Some(c);
            }
            176 => {
                if data {
                    return Err(invalid("merge inside worksheet data"));
                }
                let r0 = b.u32()?;
                let r1 = b.u32()?;
                let c0 = b.u32()?;
                let c1 = b.u32()?;
                b.finish()?;
                if r0 > r1 || c0 > c1 || r1 >= MAX_ROWS || c1 >= MAX_COLS {
                    warnings.warn("invalid XLSB merged ranges were skipped");
                    continue;
                }
                if sheet.merges.len() >= MAX_MERGES {
                    return Err(Error::TooLarge(format!("XLSB exceeds {MAX_MERGES} merges")));
                }
                sheet.merges.push(Range::new(CellRef::new(r0, c0), CellRef::new(r1, c1)));
            }
            _ => {}
        }
    }
    Err(invalid("missing worksheet end record"))
}
