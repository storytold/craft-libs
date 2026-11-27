//! Binary workbooks ([MS-XLSB]): the workbook part with its defined names, shared strings and
//! worksheets (rows, cells, columns, default sizes, merged cells), from their records. Cell
//! formulas are decoded from their tokens ([`formula`]) into the text an `.xlsx` file holds;
//! one that can't be keeps only its saved result, with a warning saying why. Table parts are
//! read in `table`; the styles part is read into the same [`crate::styles::Styles`]
//! ([`styles`]). A record that does not parse fails the read.

mod expr;
mod external;
mod formula;
mod ftab;
mod styles;
mod table;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use craft_opc::{ContentTypes, Rels};
use craft_sheet::{CellError, CellRef, MAX_COLS, MAX_ROWS, Range, Visibility};

use crate::features::SheetFormat;
use crate::sheet::{Cell, CellFormula, CellValue, Column, FormulaKind, Row, SheetPart};
use crate::strings::StringItem;
use crate::workbook::{DefinedName, SheetEntry, SheetKind, Workbook};
use crate::{Error, Limits, Warnings};
pub(crate) use external::{EXTERNAL_LINK, external_link};
use formula::{Book, Mode, Name, Xti};
pub(crate) use formula::{ExternalInfo, Links};
pub(crate) use styles::styles;
pub(crate) use table::{TABLE, index_table, table};

pub(crate) const MAIN: &str = "application/vnd.ms-excel.main";
const BINARY_MAIN: &str = "application/vnd.ms-excel.sheet.binary.macroEnabled.main";
pub(crate) const WORKSHEET: &str = "application/vnd.ms-excel.worksheet";
pub(crate) const STRINGS: &str = "application/vnd.ms-excel.sharedStrings";
pub(crate) const STYLES: &str = "application/vnd.ms-excel.styles";
/// Most sheets in the workbook.
const MAX_SHEETS: usize = 1024;
/// Supporting links (this workbook, add-ins, other workbooks) read at most; past it, formulas
/// keep their saved results.
const MAX_BOOKS: usize = 65_536;
/// Most merged ranges in one worksheet.
const MAX_MERGES: usize = 100_000;
/// External link references read at most.
const MAX_EXTERNAL_LINKS: usize = 65_536;
/// Table parts one worksheet lists at most.
const MAX_TABLE_PARTS: usize = 65_536;
/// Most column records in one worksheet: one per column.
const MAX_COLUMNS: usize = MAX_COLS as usize;
/// The tallest row, in twips (409.6 points).
const MAX_ROW_TWIPS: u16 = 8192;
/// Defined names (Excel's hidden ones for newer functions included) read at most; past it, the
/// workbook's formulas keep their saved results.
const MAX_NAMES: usize = 65_536;

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
    formula_text: std::cell::Cell<usize>,
}

impl Budget {
    /// Charges a decoded formula's text if it fits [`Limits::binary_formula_text`]; a formula
    /// that doesn't keeps its saved result.
    /// Whether formula text of `len` bytes would still fit, without charging it.
    fn formula_text_fits(&self, len: usize, limits: &Limits) -> bool {
        self.formula_text.get().saturating_add(len) <= limits.binary_formula_text
    }

    fn try_formula_text(&self, len: usize, limits: &Limits) -> bool {
        let total = self.formula_text.get().saturating_add(len);
        let fits = total <= limits.binary_formula_text;
        if fits {
            self.formula_text.set(total);
        }
        fits
    }

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
    fn i32(&mut self) -> Result<i32, Error> {
        Ok(i32::from_le_bytes(self.take(4)?.try_into().map_err(|_| invalid("truncated i32"))?))
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
    /// Each open block's end record, and whether its records are read.
    blocks: Vec<(u16, bool)>,
    /// Read the alternate content meant for Excel 2007 (version 12), as an `.xlsx` reader sees
    /// the main attribute where Excel keeps a newer spelling in an extension.
    excel_2007: bool,
    /// Future-record and alternate-content blocks skipped so far.
    skipped: usize,
}

impl<'a, 'l> Records<'a, 'l> {
    fn new(bytes: &'a [u8], limits: &'l Limits, budget: &'l Budget) -> Self {
        Self { bytes: Bytes::new(bytes), limits, budget, blocks: vec![], excel_2007: false, skipped: 0 }
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
                    let mut read = false;
                    if id == 35 {
                        b.take(4)?;
                    } else {
                        let count = b.u16()?;
                        if count == 0 {
                            return Err(invalid("empty alternate-content version list"));
                        }
                        // ACProductVersion: fileVersion, then fileProduct (15 bits) and
                        // fileExtension (the version and later).
                        for _ in 0..count {
                            let (version, product) = (b.u16()?, b.u16()?);
                            let later = product & 0x8000 != 0;
                            read |= self.excel_2007 && product & 0x7FFF == 0 && (version == 12 || later && version <= 12);
                        }
                    }
                    b.finish()?;
                    if self.blocks.len() >= 64 {
                        return Err(invalid("extension blocks nested too deeply"));
                    }
                    // Inside a skipped block, everything is skipped.
                    let read = read && self.blocks.iter().all(|(_, r)| *r);
                    self.blocks.push((id + 1, read));
                    if !read {
                        self.skipped += 1;
                        warnings.warn("unsupported XLSB future-record and alternate-content blocks were skipped");
                    }
                }
                // BrtFRTEnd, BrtACEnd.
                36 | 38 => {
                    if self.blocks.pop().map(|(end, _)| end) != Some(id) || !payload.is_empty() {
                        return Err(invalid("mismatched extension block end"));
                    }
                }
                _ if self.blocks.iter().any(|(_, read)| !read) => {}
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
/// The workbook part, and the tables its formulas refer to ([`Links`]): `None` when those can't
/// be read, so that the workbook still opens, with every formula kept as its saved result. Its
/// defined names come back undecoded ([`Names`]): [`read_names`] decodes them once the workbook's
/// tables are indexed, so a name's formula can use a structured reference.
pub(crate) fn workbook(
    bytes: &[u8],
    rels: &Rels,
    limits: &Limits,
    budget: &Budget,
    warnings: &mut Warnings,
) -> Result<(Workbook, Option<Links>, Names), Error> {
    let mut records = Records::new(bytes, limits, budget);
    records.first(131, warnings)?;
    let mut wb = Workbook::default();
    let mut links = Some(Links::default());
    let mut pending = vec![];
    // Each BrtBundleSh's place in `wb.sheets`, `None` for a skipped one.
    let mut bundle_sheets: Vec<Option<u32>> = vec![];
    let mut bundle = false;
    let mut bundle_seen = false;
    // Every BrtBundleSh counts toward the cap, a skipped module sheet too: each keeps a place
    // in the sheet links.
    let mut bundle_count = 0usize;
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
                return Ok((wb, links, Names { pending, sheets: bundle_sheets }));
            }
            39 | 355 | 357 | 358 | 362 | 667 => {
                // BrtSupBookSrc: an external link part, by its relationship id, in the order a
                // formula's `[1]` counts.
                // One that can't be read keeps its place, empty, so later links keep their numbers.
                if id == 355 && wb.external_references.len() < MAX_EXTERNAL_LINKS {
                    let mut b = Bytes::new(payload);
                    match b.text(255).and_then(|rel| b.finish().map(|()| rel)) {
                        Ok(rel) => wb.external_references.push(rel),
                        Err(_) => {
                            warnings.warn("XLSB: an external link reference could not be read");
                            wb.external_references.push(String::new());
                        }
                    }
                }
                if let Some(l) = links.as_mut()
                    && link_record(id, payload, l, &mut pending).is_err()
                {
                    links = None;
                }
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
                if bundle_count >= MAX_SHEETS {
                    return Err(Error::TooLarge(format!("XLSB exceeds {MAX_SHEETS} sheets")));
                }
                bundle_count += 1;
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
                    // Formulas name sheets by their place in this list, skipped sheets included.
                    if let Some(l) = links.as_mut() {
                        l.sheets.push(None);
                    }
                    bundle_sheets.push(None);
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
                // Only worksheets and macro sheets have cells a formula can refer to (a macro's
                // name refers to its macro sheet: `Macro1!$A$1`).
                if let Some(l) = links.as_mut() {
                    l.sheets.push(matches!(kind, SheetKind::Worksheet | SheetKind::Macrosheet).then(|| name.clone()));
                }
                bundle_sheets.push(u32::try_from(wb.sheets.len()).ok());
                wb.sheets.push(SheetEntry { name, sheet_id, visibility, rel_id, kind, part: Some(rel.target.clone()) });
            }
            _ => {}
        }
    }
    Err(invalid("missing workbook end record"))
}

/// A `BrtName`'s parts that only its [`DefinedName`] needs, until the workbook part is read.
struct PendingName {
    /// Its place in [`Links`]'s names.
    index: usize,
    flags: u32,
    /// `chKey`: a macro's shortcut key.
    key: u8,
    rgce: Vec<u8>,
    rgcb: Vec<u8>,
    comment: Option<String>,
    description: Option<String>,
    help: Option<String>,
}

/// The workbook part's defined names, not yet decoded, and each BrtBundleSh's place in
/// [`Workbook::sheets`] (`None` for a skipped one).
pub(crate) struct Names {
    pending: Vec<PendingName>,
    sheets: Vec<Option<u32>>,
}

/// Decodes the workbook part's defined names `names` into `wb`, after the tables in `links` are
/// indexed. Without `links` none can be read, and formulas keep their saved results.
pub(crate) fn read_names(wb: &mut Workbook, names: Names, links: Option<&mut Links>, limits: &Limits, budget: &Budget, warnings: &mut Warnings) {
    match links {
        Some(l) => wb.defined_names = defined_names(names.pending, &names.sheets, l, limits, budget, warnings),
        None => warnings.warn("XLSB: the workbook's sheet links or names could not be read; formulas keep their saved results"),
    }
}

/// The workbook's defined names, as `.xlsx` has them in `definedNames`: every `BrtName` but the
/// hidden ones for newer functions and LET parameters, which Excel doesn't write there. `sheets`
/// maps a name's `itab` (its sheet's place in `BrtBundleSh` order) to [`Workbook::sheets`].
/// A name whose formula can't be read, or that belongs to a sheet the reader skips, is left out
/// with a warning, and a cell formula using it keeps its saved result.
fn defined_names(
    pending: Vec<PendingName>,
    sheets: &[Option<u32>],
    links: &mut Links,
    limits: &Limits,
    budget: &Budget,
    warnings: &mut Warnings,
) -> Vec<DefinedName> {
    // Each name's formula, and the names it uses. A name whose formula uses a name that is left
    // out is left out too, whatever their order, so a cell using it keeps its saved result.
    let mut omitted = vec![false; links.names.len()];
    let mut kept = vec![];
    // PtgName identifies a record, but a bare workbook name in a sheet-scoped definition
    // would instead bind to a same-spelled local name. Without a workbook qualifier, keep
    // the cached results rather than silently changing that identity. Index once so hostile
    // files with many names and references cannot turn this check into a quadratic scan.
    // Ordinary shadows still count if their own formulas are omitted: do not infer a new
    // binding from a partially emitted name environment. Synthetic names are not definitions.
    let local_names: HashSet<_> =
        links.names.iter().filter(|n| !n.future && !formula::is_param(&n.text)).filter_map(|n| n.scope.map(|s| (s, n.text.to_lowercase()))).collect();
    for p in pending {
        let Some(name) = links.names.get(p.index) else { continue };
        if name.future || formula::is_param(&name.text) {
            continue;
        }
        let local_sheet_id = match name.scope {
            None => None,
            Some(itab) => match usize::try_from(itab).ok().and_then(|i| sheets.get(i)) {
                Some(Some(i)) => Some(*i),
                _ => {
                    warnings.warn("XLSB: some defined names were left out (names of sheets the reader skips)");
                    omit(&mut omitted, p.index);
                    continue;
                }
            },
        };
        // Excel omits the formula for a workbook-level Database built-in and a VBA function
        // (fFunc, fOB, fProc). They remain usable by formulas without a definedNames entry.
        // Other empty names are malformed: omitting them must also omit their dependants.
        if p.rgce.is_empty() {
            let database = p.flags & 0x20 != 0 && name.scope.is_none() && name.text == "_xlnm.Database";
            let vba_function = p.flags & 0x0E == 0x0E;
            if database || vba_function {
                if let Some(n) = links.names.get_mut(p.index) {
                    n.read = true;
                }
            } else {
                warnings.warn("XLSB: some defined names were left out (empty formula)");
                omit(&mut omitted, p.index);
            }
            continue;
        }
        let mut used = vec![];
        let formula = match formula::decode_using(&p.rgce, &p.rgcb, links, Mode::Name, &mut used) {
            Ok(e) => expr::print(&e),
            Err(formula::Unsupported(why)) => {
                warnings.warn(format!("XLSB: some defined names were left out ({why})"));
                omit(&mut omitted, p.index);
                continue;
            }
        };
        if name.scope.is_some_and(|scope| {
            used.iter().filter_map(|&i| links.names.get(i)).any(|target| {
                target.scope.is_none()
                    && !target.future
                    && !formula::is_param(&target.text)
                    && local_names.contains(&(scope, target.text.to_lowercase()))
            })
        }) {
            warnings.warn("XLSB: some defined names were left out (workbook names shadowed by local names)");
            omit(&mut omitted, p.index);
            continue;
        }
        if !budget.try_formula_text(formula.len(), limits) {
            warnings.warn("XLSB: some defined names were left out (more formula text than the limits allow)");
            omit(&mut omitted, p.index);
            continue;
        }
        kept.push((p, local_sheet_id, formula, used));
    }
    // Who uses each name, then every name reached from a left-out one: once each, so a chain
    // of any length costs what its uses do.
    let mut users: HashMap<usize, Vec<usize>> = HashMap::new();
    for (k, (.., used)) in kept.iter().enumerate() {
        for &u in used {
            users.entry(u).or_default().push(k);
        }
    }
    let mut queue: Vec<usize> = (0..omitted.len()).filter(|&i| omitted[i]).collect();
    while let Some(i) = queue.pop() {
        for &k in users.get(&i).map_or(&[][..], Vec::as_slice) {
            let Some((p, ..)) = kept.get(k) else { continue };
            if omit(&mut omitted, p.index) {
                warnings.warn("XLSB: some defined names were left out (names using a name that was left out)");
                queue.push(p.index);
            }
        }
    }
    let mut out = vec![];
    for (p, local_sheet_id, formula, _) in kept {
        if omitted.get(p.index).copied().unwrap_or(true) {
            continue;
        }
        let Some(name) = links.names.get(p.index) else { continue };
        let mut attrs = vec![];
        let mut put = |k: &str, v: String| attrs.push((k.to_string(), v));
        // In `CT_DefinedName`'s order.
        if let Some(d) = p.description {
            put("description", d);
        }
        if let Some(h) = p.help {
            put("help", h);
        }
        let (func, vba, proc) = (p.flags & 0x02 != 0, p.flags & 0x04 != 0, p.flags & 0x08 != 0);
        if func {
            put("function", "1".into());
        }
        if vba {
            put("vbProcedure", "1".into());
        } else if proc {
            put("xlm", "1".into());
        }
        let group = (p.flags >> 6) & 0x1FF;
        if proc && group != 0 {
            put("functionGroupId", group.to_string());
        }
        if p.key != 0 {
            put("shortcutKey", char::from(p.key).to_string());
        }
        if p.flags & (1 << 15) != 0 {
            put("publishToServer", "1".into());
        }
        if p.flags & (1 << 16) != 0 {
            put("workbookParameter", "1".into());
        }
        out.push(DefinedName {
            name: name.text.clone(),
            local_sheet_id,
            formula,
            comment: p.comment.unwrap_or_default(),
            hidden: p.flags & 1 != 0,
            attrs,
        });
        if let Some(n) = links.names.get_mut(p.index) {
            n.read = true;
        }
    }
    out
}

/// Marks name `i` left out; whether it wasn't already.
fn omit(omitted: &mut [bool], i: usize) -> bool {
    omitted.get_mut(i).is_some_and(|o| !std::mem::replace(o, true))
}

/// A supporting link (`BrtSupSelf`, `BrtSupSame`, `BrtSupAddin`, `BrtSupBookSrc`), the sheet links
/// (`BrtExternSheet`) or a defined name (`BrtName`), added to `links`; a name's other parts go to
/// `pending`.
fn link_record(id: u16, payload: &[u8], links: &mut Links, pending: &mut Vec<PendingName>) -> Result<(), Error> {
    let mut b = Bytes::new(payload);
    if matches!(id, 355 | 357 | 358 | 667) && links.books.len() >= MAX_BOOKS {
        return Err(invalid("too many supporting links"));
    }
    match id {
        357 | 358 | 667 => {
            b.finish()?;
            links.books.push(match id {
                357 => Book::This,
                358 => Book::Same,
                _ => Book::Addin,
            });
        }
        355 => {
            links.books.push(Book::External(links.external.len()));
            links.external.push(None);
        }
        362 => {
            // [MS-XLSB] 2.4.672: cXti "MUST be less than 65536" (PtgRef3d's ixti is 16 bits).
            let count = b.u32()?;
            if count >= 65_536 || !links.xti.is_empty() {
                return Err(invalid("invalid sheet link list"));
            }
            for _ in 0..count {
                let book = b.u32()?;
                let (first, last) = (b.i32()?, b.i32()?);
                links.xti.push(Xti { book, first, last });
            }
            b.finish()?;
        }
        _ => {
            // BrtName ([MS-XLSB] 2.4.718). Its formula is decoded once the workbook part is read.
            if links.names.len() >= MAX_NAMES {
                return Err(invalid("too many defined names"));
            }
            let flags = b.u32()?;
            let key = b.u8()?;
            let itab = b.u32()?;
            let text = b.text(255)?;
            let cce = b.u32()? as usize;
            let rgce = b.take(cce)?.to_vec();
            let cb = b.u32()? as usize;
            let rgcb = b.take(cb)?.to_vec();
            let comment = b.string(255, true)?;
            // A macro's name (fProc) has four more strings: one unused, its description, its help
            // topic and another unused. ([MS-XLSB] calls the second the custom menu and the third
            // the description; Excel writes them as `.xlsx`'s `description` and `help`.) Excel
            // leaves them out of a VBA function's name (fOB), which ends after its comment.
            let vba_without_strings = flags & 0x04 != 0 && b.rest.is_empty();
            let (description, help) = if flags & 0x08 != 0 && !vba_without_strings {
                b.string(32767, true)?;
                let description = b.string(32767, true)?;
                let help = b.string(32767, true)?;
                b.string(32767, true)?;
                (description, help)
            } else {
                (None, None)
            };
            b.finish()?;
            let scope = (itab != u32::MAX).then_some(itab);
            // A built-in name (fBuiltin) is stored bare; `.xlsx` spells it `_xlnm.Print_Area`, in
            // `definedNames` and in formulas alike.
            let text = if flags & 0x20 != 0 { format!("_xlnm.{text}") } else { text };
            pending.push(PendingName { index: links.names.len(), flags, key, rgce, rgcb, comment, description, help });
            links.names.push(Name { text, future: flags & (1 << 17) != 0, scope, read: false });
        }
    }
    Ok(())
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

/// The `grbitFlags` and `CellParsedFormula` after a formula cell's value: its tokens (`rgce`)
/// and their extra data (`rgcb`). The flags' only bit, `fAlwaysCalc`, is the calculator's to know.
fn cell_formula<'a>(b: &mut Bytes<'a>) -> Result<(&'a [u8], &'a [u8]), Error> {
    b.take(2)?;
    let cce = b.u32()?;
    if !(1..=16384).contains(&cce) {
        return Err(invalid("invalid formula token length"));
    }
    let rgce = b.take(cce as usize)?;
    let extra = b.u32()?;
    let rgcb = b.take(extra as usize)?;
    Ok((rgce, rgcb))
}

/// A formula's text, or `None` (with a warning saying why) for one that keeps its saved result:
/// the workbook's links couldn't be read, the tokens can't be decoded, or the formula text
/// allowance is spent.
fn decode_formula(
    rgce: &[u8],
    rgcb: &[u8],
    mode: Mode,
    links: Option<&Links>,
    limits: &Limits,
    budget: &Budget,
    warnings: &mut Warnings,
) -> Option<String> {
    let links = links?;
    // Once the allowance is spent no formula fits it, so none is decoded only to be dropped.
    if !budget.formula_text_fits(1, limits) {
        kept(warnings, "more formula text than the limits allow");
        return None;
    }
    let text = match formula::decode(rgce, rgcb, links, mode) {
        Ok(e) => expr::print(&e),
        Err(formula::Unsupported(why)) => {
            kept(warnings, why);
            return None;
        }
    };
    if !budget.try_formula_text(text.len(), limits) {
        kept(warnings, "more formula text than the limits allow");
        return None;
    }
    Some(text)
}

/// Warns, once per reason, that formulas were kept as their saved results.
fn kept(warnings: &mut Warnings, why: &str) {
    warnings.warn(format!("XLSB: some formulas were kept as their saved results ({why})"));
}

/// A `BrtShrFmla`: the cells it may cover, its tokens, its first cell's text length (what a copy
/// is expected to cost) and the group's index (`si`). The tokens are borrowed from the worksheet
/// part, which outlives the sheet's read: a record can be up to 256 MiB, and copying every
/// shared formula's would cost as much again as the part.
#[derive(Clone, Copy)]
struct SharedFormula<'a> {
    range: Range,
    rgce: &'a [u8],
    rgcb: &'a [u8],
    len: usize,
    si: u32,
}

/// A formula cell whose tokens are only `PtgExp`: part of a shared or array formula.
enum Part {
    /// The first cell: its formula is the `BrtShrFmla` or `BrtArrFmla` record that follows.
    First,
    /// Another cell of the formula whose first cell is this.
    Of(CellRef),
}

/// `PtgExp` ([MS-XLSB] 2.5.98.40) with its `PtgExtraCol`: the first cell of the shared or array
/// formula this cell belongs to. BIFF12 keeps only the row in the token and the column in rgcb.
fn part_of(rgce: &[u8], rgcb: &[u8], at: CellRef) -> Option<Result<Part, &'static str>> {
    let (&ptg, row) = rgce.split_first()?;
    if ptg != 0x01 {
        return None;
    }
    let (Ok(row), Ok(col)) = (<[u8; 4]>::try_from(row), <[u8; 4]>::try_from(rgcb)) else {
        return Some(Err("malformed shared or array formula references"));
    };
    let first = CellRef::new(u32::from_le_bytes(row), u32::from_le_bytes(col));
    Some(Ok(if first == at { Part::First } else { Part::Of(first) }))
}

/// One sheet's shared and array formulas, by first cell, as the reader meets them. `None` when
/// the formula keeps its values (so its other cells keep theirs).
#[derive(Default)]
struct RangeFormulas<'a> {
    shared: HashMap<CellRef, Option<SharedFormula<'a>>>,
    arrays: HashMap<CellRef, Option<Range>>,
}

impl<'a> RangeFormulas<'a> {
    /// The formula of another cell `at` of the shared or array formula starting at `first`. A
    /// shared formula's copy is decoded from the shared tokens at its own cell, as [MS-XLSB]
    /// defines it (relative offsets wrap around the grid), and checked like any formula: its text
    /// may be longer than the first cell's. That length estimates its cost, and a copy that can't
    /// fit isn't decoded at all, so a long formula shared by a million cells stops costing once
    /// the allowance is spent. An array formula's other cells have none, as in `.xlsx`.
    fn follower(
        &mut self,
        at: CellRef,
        first: CellRef,
        links: Option<&Links>,
        limits: &Limits,
        budget: &Budget,
        warnings: &mut Warnings,
    ) -> Option<CellFormula> {
        if let Some(shared) = self.shared.get(&first) {
            let sh = (*shared)?;
            if !sh.range.contains(at) {
                kept(warnings, "malformed shared or array formula references");
                return None;
            }
            if !budget.formula_text_fits(sh.len, limits) {
                kept(warnings, "more formula text than the limits allow");
                return None;
            }
            let Some(text) = decode_formula(sh.rgce, sh.rgcb, Mode::Shared(at), links, limits, budget, warnings) else {
                // A copy that was decoded but kept as its value charged nothing: one past the
                // allowance (it may be longer than the first cell's estimate) or one that can't be
                // decoded. The group's later copies would fail the same way, so they keep their
                // values without being decoded: a group costs at most one wasted decode.
                self.shared.insert(first, None);
                return None;
            };
            return Some(CellFormula { kind: FormulaKind::Shared, text, shared_index: Some(sh.si), ..CellFormula::default() });
        }
        match self.arrays.get(&first) {
            Some(Some(range)) if range.contains(at) => {}
            Some(Some(_)) => kept(warnings, "malformed shared or array formula references"),
            Some(None) => {}
            None => kept(warnings, "a shared or array formula that is missing from the file"),
        }
        None
    }

    /// `BrtShrFmla` (427) or `BrtArrFmla` (426) right after the first cell `first` of its range:
    /// the formula that cell keeps, recorded for the range's other cells. A record that can't be
    /// read leaves the whole range as values; it never fails the file, as these records were
    /// skipped before formulas were read.
    #[allow(clippy::too_many_arguments)]
    fn range_formula(
        &mut self,
        id: u16,
        payload: &'a [u8],
        first: CellRef,
        links: Option<&Links>,
        limits: &Limits,
        budget: &Budget,
        warnings: &mut Warnings,
    ) -> Option<CellFormula> {
        let read = |b: &mut Bytes<'a>| -> Result<(Range, &'a [u8], &'a [u8]), Error> {
            let (r0, r1, c0, c1) = (b.u32()?, b.u32()?, b.u32()?, b.u32()?);
            if id == 426 {
                b.u8()?; // Only fAlwaysCalc, the calculator's to know.
            }
            let cce = b.u32()?;
            if !(1..=16384).contains(&cce) {
                return Err(invalid("invalid formula token length"));
            }
            let rgce = b.take(cce as usize)?;
            let cb = b.u32()?;
            let rgcb = b.take(cb as usize)?;
            if r0 > r1 || c0 > c1 || r1 >= MAX_ROWS || c1 >= MAX_COLS {
                return Err(invalid("invalid formula range"));
            }
            let range = Range::new(CellRef::new(r0, c0), CellRef::new(r1, c1));
            // An array formula's first cell is its range's top-left one. A shared formula's is the
            // first cell that uses it, which need not be: the top-left cell may not share it.
            let start_ok = if id == 427 { range.contains(first) } else { range.first == first };
            if !start_ok {
                return Err(invalid("invalid formula range"));
            }
            Ok((range, rgce, rgcb))
        };
        let mut b = Bytes::new(payload);
        let Ok((range, rgce, rgcb)) = read(&mut b).and_then(|r| b.finish().map(|()| r)) else {
            kept(warnings, "malformed shared or array formulas");
            self.shared.insert(first, None);
            self.arrays.insert(first, None);
            return None;
        };
        if id == 427 {
            let text = decode_formula(rgce, rgcb, Mode::Shared(first), links, limits, budget, warnings);
            let si = u32::try_from(self.shared.len()).unwrap_or(u32::MAX);
            let shared = text.as_ref().map(|t| SharedFormula { range, rgce, rgcb, len: t.len(), si });
            self.shared.insert(first, shared);
            return text.map(|text| CellFormula {
                kind: FormulaKind::Shared,
                text,
                shared_index: Some(si),
                range: Some(range),
                ..CellFormula::default()
            });
        }
        let text = decode_formula(rgce, rgcb, Mode::Cell, links, limits, budget, warnings);
        self.arrays.insert(first, text.as_ref().map(|_| range));
        text.map(|text| CellFormula { kind: FormulaKind::Array, text, range: Some(range), ..CellFormula::default() })
    }
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
    Ok(value)
}

/// `BrtRowHdr` after `rw`, up to its column spans: the row's height, flags and style, as the
/// `.xlsx` row Excel writes. `miyRw` is the height of every row: it is `ht` when set by hand
/// (`fUnsynced`) or when it differs from the sheet's default, and past 8192 twips it is ignored.
fn row_header(index: u32, b: &mut Bytes<'_>, default_twips: Option<u16>) -> Result<Row, Error> {
    let ixfe = b.u32()?;
    let twips = b.u16()?;
    let [borders, flags, phonetic] = [b.u8()?, b.u8()?, b.u8()?];
    let custom = flags & 0x20 != 0;
    let height = (twips <= MAX_ROW_TWIPS && (custom || default_twips.is_some_and(|d| d != twips))).then(|| f64::from(twips) / 20.0);
    let mut attrs = vec![];
    for (bit, byte, name) in [(1, borders, "thickTop"), (2, borders, "thickBot"), (1, phonetic, "ph")] {
        if byte & bit != 0 {
            attrs.push((name.to_string(), "1".to_string()));
        }
    }
    Ok(Row {
        index,
        height,
        custom_height: custom && height.is_some(),
        hidden: flags & 0x10 != 0,
        outline_level: flags & 7,
        collapsed: flags & 0x08 != 0,
        style: (flags & 0x40 != 0).then_some(ixfe),
        cells: vec![],
        attrs,
    })
}

/// `BrtWsFmtInfo`: `sheetFormatPr` as Excel writes it, and the default row height in twips.
/// `dxGCol` is the default width in 256ths of a character (`0xFFFFFFFF`: none set);
/// `cchDefColWidth` is the base width, left out at its default of 8.
fn sheet_format(b: &mut Bytes<'_>) -> Result<(SheetFormat, u16), Error> {
    let width = b.u32()?;
    let base = b.u16()?;
    let twips = b.u16()?;
    let flags = b.u16()?;
    let rows = b.u8()?;
    let cols = b.u8()?;
    let set = |bit: u16| (flags & bit != 0).then_some(true);
    let level = |n: u8| (n > 0).then_some(u32::from(n.min(7)));
    let format = SheetFormat {
        base_col_width: (base != 8).then_some(u32::from(base)),
        default_col_width: (width != u32::MAX).then(|| f64::from(width) / 256.0),
        default_row_height: Some(f64::from(twips) / 20.0),
        custom_height: set(1),
        zero_height: set(2),
        thick_top: set(4),
        thick_bottom: set(8),
        outline_level_row: level(rows),
        outline_level_col: level(cols),
        attrs: vec![],
    };
    Ok((format, twips))
}

/// `BrtColInfo`: `None` for a range outside the grid or backwards. `coldx` is the width in 256ths
/// of a character, which is the `.xlsx` width exactly.
fn column(b: &mut Bytes<'_>) -> Result<Option<Column>, Error> {
    let first = b.u32()?;
    let last = b.u32()?;
    let width = b.u32()?;
    let style = b.u32()?;
    let flags = b.u16()?;
    if first > last || last >= MAX_COLS {
        return Ok(None);
    }
    Ok(Some(Column {
        first,
        last,
        width: Some(f64::from(width) / 256.0),
        custom_width: flags & 2 != 0,
        best_fit: flags & 4 != 0,
        hidden: flags & 1 != 0,
        outline_level: ((flags >> 8) & 7) as u8,
        collapsed: flags & 0x1000 != 0,
        style: (style != 0).then_some(style),
    }))
}

/// A worksheet part: the sheet's default sizes (BrtWsFmtInfo) and columns (BrtColInfo) before
/// BrtBeginSheetData, rows (BrtRowHdr) and cells inside it, merged cells after.
/// Formulas refer to the workbook's `links`; with none, each keeps its saved result.
#[allow(clippy::too_many_arguments)]
pub(crate) fn worksheet(
    bytes: &[u8],
    strings: usize,
    links: Option<&Links>,
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
    // Every BrtColInfo counts toward the cap, a skipped one too.
    let mut column_records = 0usize;
    let mut row: Option<Row> = None;
    let mut col = None;
    // The default row height in twips: a row of another height has `ht` even when it was not set
    // by hand (a large font, a thick border), as Excel writes it in `.xlsx`.
    let mut default_twips: Option<u16> = None;
    // BrtCellMeta applies to the cell record that follows it (a dynamic-array formula), as `cm`
    // does in `.xlsx`: a one-based index into the metadata part's cell metadata. Excel may write
    // the cell's BrtValueMeta (a rich value, such as a spill error) in between.
    let mut meta: Option<u32> = None;
    let mut ranges = RangeFormulas::default();
    // The first cell of a shared or array formula, waiting for the record holding the formula.
    let mut first: Option<CellRef> = None;
    while let Some((id, payload)) = records.next(warnings)? {
        let mut b = Bytes::new(payload);
        let cell_metadata = if id == 50 { meta } else { meta.take() };
        if let Some(at) = first.take() {
            if matches!(id, 426 | 427) {
                let f = ranges.range_formula(id, payload, at, links, limits, budget, warnings);
                if let Some(cell) = row.as_mut().and_then(|r| r.cells.last_mut()).filter(|c| c.at == at) {
                    // A dynamic array's cell metadata describes its formula: with none kept, the
                    // cell is a value, and `cm` on it would mark a formula that isn't there.
                    if f.is_none() {
                        cell.cell_metadata = None;
                    }
                    cell.formula = f;
                }
                continue;
            }
            // No formula record: the cell stays a value, so its formula's cell metadata goes too.
            if let Some(cell) = row.as_mut().and_then(|r| r.cells.last_mut()).filter(|c| c.at == at) {
                cell.cell_metadata = None;
            }
            kept(warnings, "a shared or array formula that is missing from the file");
        }
        match id {
            49 => {
                let icmb = b.i32()?;
                b.finish()?;
                meta = u32::try_from(icmb).ok().filter(|&i| i >= 1);
            }
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
                let next = row_header(r, &mut b, default_twips)?;
                let spans = b.u32()?;
                if spans > 16 {
                    return Err(invalid("too many row spans"));
                }
                b.take(spans as usize * 8)?;
                b.finish()?;
                if let Some(previous) = row.replace(next) {
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
                let at = CellRef::new(current.index, c);
                let formula = if (8..=11).contains(&id) {
                    let (rgce, rgcb) = cell_formula(&mut b)?;
                    b.finish()?;
                    match part_of(rgce, rgcb, at) {
                        None => decode_formula(rgce, rgcb, Mode::Cell, links, limits, budget, warnings)
                            .map(|text| CellFormula { text, ..CellFormula::default() }),
                        Some(Ok(Part::First)) => {
                            first = links.map(|_| at);
                            None
                        }
                        Some(Ok(Part::Of(start))) if links.is_some() => ranges.follower(at, start, links, limits, budget, warnings),
                        Some(Ok(Part::Of(_))) => None,
                        Some(Err(why)) => {
                            kept(warnings, why);
                            None
                        }
                    }
                } else {
                    b.finish()?;
                    None
                };
                if value != CellValue::Empty {
                    budget.cell(limits)?;
                }
                // A formula record's cell metadata describes its formula (a dynamic array), so a
                // formula kept as its value drops it; the first cell of a shared or array formula
                // keeps it until its formula record is read. Other cells keep theirs as the file
                // has it.
                let formula_kept = !(8..=11).contains(&id) || formula.is_some() || first == Some(at);
                let cell_metadata = cell_metadata.filter(|_| formula_kept);
                current.cells.push(Cell {
                    at,
                    style: style & 0x00FF_FFFF,
                    phonetic: style & 0x0100_0000 != 0,
                    value,
                    formula,
                    cell_metadata,
                    ..Cell::default()
                });
                col = Some(c);
            }
            // BrtListPart: one of the sheet's tables, by relationship id.
            661 => {
                if data {
                    return Err(invalid("table part inside worksheet data"));
                }
                let rel = b.text(255)?;
                b.finish()?;
                if sheet.features.table_parts.len() >= MAX_TABLE_PARTS {
                    return Err(Error::TooLarge(format!("XLSB exceeds {MAX_TABLE_PARTS} tables on a sheet")));
                }
                sheet.features.table_parts.push(rel);
            }
            485 => {
                if data_seen {
                    return Err(invalid("sheet format after worksheet data"));
                }
                let (format, twips) = sheet_format(&mut b)?;
                b.finish()?;
                default_twips = Some(twips);
                sheet.features.format = Some(format);
            }
            60 => {
                if data_seen {
                    return Err(invalid("column after worksheet data"));
                }
                if column_records >= MAX_COLUMNS {
                    return Err(Error::TooLarge(format!("XLSB exceeds {MAX_COLUMNS} column records")));
                }
                column_records += 1;
                let column = column(&mut b)?;
                b.finish()?;
                let Some(column) = column else {
                    warnings.warn("invalid XLSB column ranges were skipped");
                    continue;
                };
                sheet.columns.push(column);
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

#[cfg(test)]
mod records_merge_tests {
    use super::*;

    #[test]
    fn table_skip_count_preserves_excel_2007_style_blocks() {
        // BrtACBegin for Excel 2007, an empty record, BrtACEnd.
        let bytes = [37, 6, 1, 0, 12, 0, 0, 0, 1, 0, 38, 0];
        let limits = Limits::default();
        let budget = Budget::default();
        let mut warnings = Warnings::default();
        let mut styles = Records::new(&bytes, &limits, &budget);
        styles.excel_2007 = true;
        assert_eq!(styles.next(&mut warnings).unwrap(), Some((1, &[][..])));
        assert_eq!(styles.skipped, 0, "supported style content was read, not skipped");
        assert!(styles.next(&mut warnings).unwrap().is_none());
        let mut tables = Records::new(&bytes, &limits, &budget);
        assert!(tables.next(&mut warnings).unwrap().is_none());
        assert_eq!(tables.skipped, 1, "table filters still detect skipped alternate content");
    }
}
