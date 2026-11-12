//! The Workbook stream ([MS-XLS] 2.1.7): the globals substream, then one substream per sheet.
//!
//! Every formula in the file is a legacy (pre-dynamic-array) one. A caller whose formula
//! language has dynamic arrays reads them with Excel's implicit intersection; nothing here
//! adds an `@`.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::formula::{Links, Mode, SupBook, Unsupported, Xti, cell_error, decode};
use crate::model::{
    CalcMode, Cell, CellRef, Column, ConditionRule, ConditionalFormat, DefinedName, Formula, Note, Picture, Range, Row, Sheet, Value, View,
    Visibility, Workbook,
};
use crate::records::{Cursor, Record, Records};
use crate::{Error, Limits, Read, Warning, cond, drawing, notes, styles};

const MAX_STRINGS: usize = 4_000_000;
const MAX_SHEETS: usize = 4096;
const MAX_NAMES: usize = 65_536;
const MAX_XFS: usize = 65_536;
const MAX_MERGES: usize = 100_000;
/// Bytes of drawing records kept for the workbook's drawing group, and for one sheet.
const MAX_DRAWING: usize = 256 << 20;
/// Pictures placed on one sheet at most.
const MAX_PICTURES: usize = 65_536;
/// Comments kept on one sheet at most.
const MAX_NOTES: usize = 65_536;
/// The Workbook stream is read whole; larger ones are refused.
const MAX_STREAM: u64 = 1 << 30;
/// Distinct warnings kept.
const MAX_WARNINGS: usize = 1000;
/// The BIFF8 grid.
const GRID_ROWS: u32 = 65_536;
const GRID_COLS: u32 = 256;

const BOF: u16 = 0x0809;
const EOF: u16 = 0x000A;
const BIFF8: u16 = 0x0600;
/// WsBool (MS-XLS 2.4.351): its first byte's `fDialog` bit marks a dialog sheet, whose BOF says worksheet.
const WSBOOL: u16 = 0x0081;
const WSBOOL_DIALOG: u8 = 0x10;

fn invalid(msg: impl Into<String>) -> Error {
    Error::Invalid(msg.into())
}

#[derive(Default)]
struct Report {
    warnings: Vec<Warning>,
}

impl Report {
    fn warn(&mut self, w: Warning) {
        if self.warnings.len() < MAX_WARNINGS && !self.warnings.contains(&w) {
            self.warnings.push(w);
        }
    }
}

/// The root-level stream called `name` (compared as compound files compare names).
fn root_stream(cf: &craft_cfb::CompoundFile<'_>, name: &str) -> Option<usize> {
    cf.child(0, name).filter(|&id| cf.entry(id).is_some_and(|e| e.kind == craft_cfb::EntryKind::Stream))
}

/// The Workbook stream of a compound file, or why there is none.
fn workbook_stream(bytes: &[u8]) -> Result<Vec<u8>, Error> {
    let cf = craft_cfb::CompoundFile::open(bytes).map_err(|e| invalid(e.to_string()))?;
    let Some(id) = root_stream(&cf, "Workbook") else {
        return Err(if root_stream(&cf, "Book").is_some() {
            Error::Biff5
        } else {
            invalid("the file has no Workbook stream, so it is not an Excel workbook")
        });
    };
    cf.read_capped(id, MAX_STREAM).map_err(|e| match e {
        craft_cfb::Error::LimitExceeded(_) => Error::TooLarge("the Workbook stream is larger than 1 GiB".into()),
        e => invalid(e.to_string()),
    })
}

/// Whether the bytes are a compound file holding an Excel workbook stream (BIFF8 `Workbook`
/// or BIFF5 `Book`). The stream itself is not read.
pub fn is_xls(bytes: &[u8]) -> bool {
    craft_cfb::CompoundFile::open(bytes).is_ok_and(|cf| root_stream(&cf, "Workbook").or_else(|| root_stream(&cf, "Book")).is_some())
}

/// Most shared formulas one sheet expands, and the longest template text one attach may
/// produce: a template shared by its followers was costing every one of them a full decode and
/// print however often that could never succeed, so each anchor is tried once and failed ones
/// are skipped for the rest of the sheet. Beyond the cap the remaining references keep their
/// cached values.
const MAX_SHARED_ATTACHES: usize = 262_144;
const MAX_SHARED_TEMPLATE: usize = 8192;

struct Budget {
    limits: Limits,
    cells: usize,
    text: usize,
    records: usize,
    /// A shared formula's text was refused: no later shared formula, in any sheet, is expanded.
    shared_text_spent: bool,
}

impl Budget {
    fn new(limits: Limits) -> Self {
        Budget { limits, cells: 0, text: 0, records: 0, shared_text_spent: false }
    }
    fn record(&mut self) -> Result<(), Error> {
        self.records = self.records.saturating_add(1);
        if self.records > self.limits.max_records {
            return Err(Error::TooLarge("too many records".into()));
        }
        Ok(())
    }
    fn cell(&mut self) -> Result<(), Error> {
        self.cells = self.cells.saturating_add(1);
        if self.cells > self.limits.max_cells {
            return Err(Error::TooLarge(format!("more than {} cells", self.limits.max_cells)));
        }
        Ok(())
    }
    /// Charges `len` bytes of text if they fit the allowance, leaving it untouched otherwise:
    /// for text the read can do without (expanded shared formulas keep their cached values).
    fn try_text(&mut self, len: usize) -> bool {
        let total = self.text.saturating_add(len);
        let fits = total <= self.limits.max_text;
        if fits {
            self.text = total;
        }
        fits
    }
    /// [`try_text`](Self::try_text) for a shared formula's text. The first refusal ends shared
    /// expansion for the whole workbook, so a shorter formula on a later sheet doesn't fit in the
    /// room the refused one left, and refused formulas aren't decoded again on each sheet.
    fn try_shared_text(&mut self, len: usize) -> bool {
        self.shared_text_spent = self.shared_text_spent || !self.try_text(len);
        !self.shared_text_spent
    }
    fn text(&mut self, len: usize) -> Result<(), Error> {
        self.text = self.text.saturating_add(len);
        if self.text > self.limits.max_text {
            return Err(Error::TooLarge(format!("more than {} bytes of text", self.limits.max_text)));
        }
        Ok(())
    }
}

struct BoundSheet {
    pos: usize,
    name: String,
    visibility: Visibility,
    kind: u8,
}

struct Lbl {
    name: String,
    builtin: Option<u8>,
    hidden: bool,
    macro_name: bool,
    itab: u16,
    rgce: Vec<u8>,
    rgcb: Vec<u8>,
}

#[derive(Default)]
struct Globals {
    sheets: Vec<BoundSheet>,
    date1904: bool,
    formats: HashMap<u16, String>,
    styles: styles::StyleRecords,
    /// The workbook's pictures (the drawing group's BStore), `pib` 1 first.
    blips: Vec<drawing::Blip>,
    sst: Vec<Arc<str>>,
    links: Links,
    names: Vec<Lbl>,
    precision_as_displayed: bool,
    /// The sheet the workbook was saved on (`itabCur` of the first Window1), counted over every
    /// BoundSheet8 record.
    active_tab: Option<u16>,
}

/// Reads an Excel 97-2003 workbook with the default [`Limits`].
pub fn read(bytes: &[u8]) -> Result<Read, Error> {
    read_with(bytes, Limits::default())
}

/// [`read`] with the caller's [`Limits`].
pub fn read_with(bytes: &[u8], limits: Limits) -> Result<Read, Error> {
    read_stream(workbook_stream(bytes)?, limits, None)
}

/// [`read_with`] for a workbook encrypted with a password (RC4 or RC4 CryptoAPI). A workbook
/// without one reads as with [`read_with`], whatever the password.
pub fn read_with_password(bytes: &[u8], limits: Limits, password: &str) -> Result<Read, Error> {
    read_stream(workbook_stream(bytes)?, limits, Some(password))
}

/// Whether the bytes are a workbook encrypted with a password (its globals hold a FilePass
/// record).
pub fn is_encrypted(bytes: &[u8]) -> bool {
    workbook_stream(bytes).is_ok_and(|s| crate::crypt::filepass(&s).is_some())
}

/// Decrypts an encrypted stream: with `password`, or without one with Excel's default password,
/// as Excel opens a workbook protected only against changes.
fn decrypt(stream: &mut [u8], password: Option<&str>) -> Result<(), Error> {
    use crate::crypt::DecryptError;
    match crate::crypt::decrypt(stream, password.unwrap_or(crate::crypt::DEFAULT_PASSWORD)) {
        Ok(()) => Ok(()),
        Err(DecryptError::WrongPassword) if password.is_none() => Err(Error::Encrypted),
        Err(DecryptError::WrongPassword) => Err(Error::WrongPassword),
        Err(DecryptError::Unsupported(s)) => Err(Error::UnsupportedEncryption(s)),
        Err(DecryptError::Malformed(s)) => Err(invalid(format!("the encryption record is damaged: {s}"))),
    }
}

fn read_stream(mut stream: Vec<u8>, limits: Limits, password: Option<&str>) -> Result<Read, Error> {
    if crate::crypt::filepass(&stream).is_some() {
        decrypt(&mut stream, password)?;
    }
    let mut budget = Budget::new(limits);
    let mut report = Report::default();
    let g = globals(&stream, &mut budget, &mut report)?;

    let mut wb = Workbook { date_1904: g.date1904, precision_as_displayed: g.precision_as_displayed, ..Workbook::default() };
    // Per XF: whether its format is plain.
    let plain = styles::build(&g.styles, &g.formats, &mut wb);
    if g.styles.short {
        report.warn(Warning::ShortFormatting);
    }

    // BoundSheet8 index → index in `wb.sheets`.
    let mut model_index: Vec<Option<usize>> = Vec::with_capacity(g.sheets.len());
    let mut parsed = Vec::new();
    for bs in &g.sheets {
        if bs.kind != 0 {
            report.warn(match bs.kind {
                2 => Warning::ChartSheet,
                1 => Warning::MacroSheet,
                _ => Warning::OtherSheet,
            });
            model_index.push(None);
            continue;
        }
        let Some(st) = worksheet(&stream, bs, &g, &plain, &mut budget, &mut report)? else {
            report.warn(Warning::DialogSheet);
            model_index.push(None);
            continue;
        };
        model_index.push(Some(parsed.len()));
        parsed.push((bs.visibility, st));
    }
    // Every sheet's mandatory text is charged by now; shared formulas take what is left.
    let mut filtered = Vec::with_capacity(parsed.len());
    for (visibility, st) in parsed {
        filtered.push(st.filtered);
        let mut sheet = finish_sheet(st, &mut budget, &mut report);
        sheet.visibility = visibility;
        // Every worksheet repeats the workbook's calculation settings; Excel applies the first.
        if wb.sheets.is_empty() {
            wb.calc = sheet.calc;
        }
        wb.sheets.push(sheet);
    }
    if wb.sheets.is_empty() {
        return Err(invalid("the workbook has no worksheets"));
    }
    wb.active_sheet = g.active_tab.and_then(|t| model_index.get(usize::from(t)).copied().flatten());
    defined_names(&g, &model_index, &filtered, &mut wb, &mut report);
    Ok(Read { workbook: wb, warnings: report.warnings })
}

fn bof(rec: &Record<'_>) -> Result<(u16, u16), Error> {
    if rec.id != BOF {
        return Err(invalid("a substream does not start with a BOF record"));
    }
    let mut c = Cursor::new(&rec.parts);
    Ok((c.u16()?, c.u16()?))
}

#[allow(clippy::cognitive_complexity, reason = "one arm per globals record of [MS-XLS] 2.1.7.20.3")]
fn globals(stream: &[u8], budget: &mut Budget, report: &mut Report) -> Result<Globals, Error> {
    let mut recs = Records::new(stream, 0);
    let first = recs.next_record()?.ok_or_else(|| invalid("empty Workbook stream"))?;
    let (vers, dt) = bof(&first)?;
    if vers != BIFF8 {
        return Err(if vers == 0x0500 { Error::Biff5 } else { invalid("unsupported BIFF version (only Excel 97-2003 BIFF8 files are read)") });
    }
    if dt != 0x0005 {
        return Err(invalid("the Workbook stream does not start with the workbook globals"));
    }
    let mut g = Globals::default();
    let mut offsets = HashSet::new();
    let mut dgg: Vec<u8> = Vec::new();
    while let Some(rec) = recs.next_record()? {
        budget.record()?;
        let mut c = Cursor::new(&rec.parts);
        match rec.id {
            EOF => break,
            // FilePass: only reached once the stream was decrypted.
            0x002F => {}
            0x0085 => {
                if g.sheets.len() >= MAX_SHEETS {
                    return Err(Error::TooLarge("too many sheets".into()));
                }
                let pos = c.u32()? as usize;
                if !offsets.insert(pos) {
                    return Err(invalid("two sheets point at the same substream"));
                }
                let state = c.u8()?;
                let kind = c.u8()?;
                let name = c.short_string()?;
                let visibility = match state & 3 {
                    0 => Visibility::Visible,
                    1 => Visibility::Hidden,
                    _ => Visibility::VeryHidden,
                };
                g.links.sheets.push(name.clone());
                g.sheets.push(BoundSheet { pos, name, visibility, kind });
            }
            0x0022 => g.date1904 = c.u16()? == 1,
            0x000E => g.precision_as_displayed = c.u16()? == 0,
            0x041E => {
                let ifmt = c.u16()?;
                let cch = usize::from(c.u16()?);
                let high = c.u8()? & 1 != 0;
                // Some writers count one character more than the record holds; Excel opens
                // those files, so keep the characters that are there.
                let fit = c.remaining() / if high { 2 } else { 1 };
                if cch > fit {
                    report.warn(Warning::ShortNumberFormat);
                }
                let code = c.chars(cch.min(fit), high)?;
                budget.text(code.len())?;
                g.formats.insert(ifmt, code);
            }
            // Window1: position, size and flags (five u16), then the active sheet. Only the first
            // window counts; a record too short for it is ignored.
            0x003D if g.active_tab.is_none() && c.remaining() >= 12 => {
                c.skip(10)?;
                g.active_tab = Some(c.u16()?);
            }
            0x00E0 => g.styles.xf(&mut c, MAX_XFS),
            0x0031 => g.styles.font(&mut c),
            0x0092 => g.styles.palette(&mut c),
            0x0293 => g.styles.style(&mut c),
            // MsoDrawingGroup: one buffer of drawing records, cut at MAX_DRAWING.
            0x00EB if dgg.len() < MAX_DRAWING => dgg.extend(c.rest()),
            0x00FC => g.sst = shared_strings(&mut c, budget)?,
            0x01AE => {
                let _ctab = c.u16()?;
                let cch = c.u16()?;
                g.links.books.push(SupBook { internal: cch == 0x0401, addin: cch == 0x3A01, names: vec![] });
            }
            0x0023 => {
                // ExternName: flags, then an AddinUdf or ExternDocName body; both start with
                // four bytes before the name.
                c.u16()?;
                c.skip(4)?;
                let name = c.short_string()?;
                if let Some(book) = g.links.books.last_mut()
                    && book.names.len() < MAX_NAMES
                {
                    book.names.push(name);
                }
            }
            0x0017 => {
                let n = usize::from(c.u16()?);
                for _ in 0..n.min(c.remaining() / 6) {
                    let book = c.u16()?;
                    let first = c.u16()? as i16;
                    let last = c.u16()? as i16;
                    g.links.xti.push(Xti { book, first, last });
                }
            }
            0x0018 => {
                if g.names.len() >= MAX_NAMES {
                    continue;
                }
                let lbl = label(&mut c)?;
                g.links.names.push(lbl.name.clone());
                g.names.push(lbl);
            }
            _ => {}
        }
    }
    // The same test `defined_names` applies, before any cell formula is decoded.
    let skipped = g.names.iter().map(|l| !l.rgce.is_empty() && decode(&l.rgce, &l.rgcb, CellRef::new(0, 0), Mode::Name, &g.links).is_err()).collect();
    g.links.skipped = skipped;
    g.blips = drawing::blip_store(&dgg);
    Ok(g)
}

/// Built-in defined names ([MS-XLS] 2.4.150, Lbl.Name).
const BUILTIN_NAMES: [&str; 14] = [
    "Consolidate_Area",
    "Auto_Open",
    "Auto_Close",
    "Extract",
    "Database",
    "Criteria",
    "Print_Area",
    "Print_Titles",
    "Recorder",
    "Data_Form",
    "Auto_Activate",
    "Auto_Deactivate",
    "Sheet_Title",
    "_FilterDatabase",
];

fn label(c: &mut Cursor<'_, '_>) -> Result<Lbl, Error> {
    let flags = c.u16()?;
    let _key = c.u8()?;
    let cch = usize::from(c.u8()?);
    let cce = usize::from(c.u16()?);
    c.skip(2)?;
    let itab = c.u16()?;
    c.skip(4)?;
    let high = c.u8()? & 1 != 0;
    let raw = c.chars(cch, high)?;
    let builtin = flags & 0x0020 != 0;
    let (name, builtin) = if builtin {
        let code = raw.chars().next().map_or(0, |ch| ch as u32);
        let code = u8::try_from(code).unwrap_or(u8::MAX);
        (BUILTIN_NAMES.get(usize::from(code)).map_or_else(|| raw.clone(), |n| (*n).to_string()), Some(code))
    } else {
        (raw, None)
    };
    let rest = c.rest();
    let rgce = rest.get(..cce).unwrap_or(&rest).to_vec();
    let rgcb = rest.get(cce..).unwrap_or(&[]).to_vec();
    Ok(Lbl { name, builtin, hidden: flags & 1 != 0, macro_name: flags & 0x000A != 0, itab, rgce, rgcb })
}

fn shared_strings(c: &mut Cursor<'_, '_>, budget: &mut Budget) -> Result<Vec<Arc<str>>, Error> {
    let _total = c.u32()?;
    let unique = c.u32()? as usize;
    if unique > MAX_STRINGS {
        return Err(Error::TooLarge("too many shared strings".into()));
    }
    // Each string takes at least three bytes: the declared count can't exceed what's there.
    let mut out = Vec::with_capacity(unique.min(c.remaining() / 3));
    for _ in 0..unique {
        if c.remaining() == 0 {
            break;
        }
        let s = c.rich_string()?;
        budget.text(s.len())?;
        out.push(Arc::from(s));
    }
    Ok(out)
}

/// A formula cell waiting for its shared or array formula (which follows it in the file).
struct Pending {
    pos: CellRef,
    anchor: CellRef,
}

/// A parsed formula: tokens and extra data.
struct Parsed {
    rgce: Vec<u8>,
    rgcb: Vec<u8>,
}

fn parsed(c: &mut Cursor<'_, '_>) -> Result<Parsed, Error> {
    let cce = usize::from(c.u16()?);
    let rest = c.rest();
    if cce > rest.len() {
        return Err(invalid("formula runs past the end of its record"));
    }
    Ok(Parsed { rgce: rest.get(..cce).unwrap_or(&[]).to_vec(), rgcb: rest.get(cce..).unwrap_or(&[]).to_vec() })
}

struct SheetState<'g> {
    g: &'g Globals,
    plain: &'g [bool],
    sheet: Sheet,
    pending: Vec<Pending>,
    shared: HashMap<CellRef, Parsed>,
    arrays: HashMap<CellRef, (Range, Parsed)>,
    last_formula: Option<CellRef>,
    /// The cell waiting for its String record, with its format: a blank cell isn't stored, so a
    /// formula kept as its value may have no cell to receive the text.
    pending_string: Option<(CellRef, Option<u16>)>,
    dropped: usize,
    unsupported: Vec<&'static str>,
    /// Dv records still to read: the last DVal's count, less those read since. Excel reads
    /// exactly that many after a DVal, and none without one.
    dv_left: u32,
    /// The comment object the last Obj record described, waiting for its TxO.
    note_object: Option<u16>,
    /// Comment texts by object id.
    note_texts: HashMap<u16, String>,
    /// Note records: the cell, its comment object, the author and whether it shows.
    notes: Vec<(CellRef, u16, String, bool)>,
    /// The CondFmt whose CF rules follow: its bounding range's top left cell and its ranges.
    cf: Option<(CellRef, Vec<Range>)>,
    /// The workbook's colour table, for conditional formats.
    palette: [u32; 64],
    /// The sheet's MsoDrawing records, joined (shapes may run across them).
    drawing: Vec<u8>,
    /// The sheet has an AutoFilterInfo record: its filter is on.
    filtered: bool,
}

impl SheetState<'_> {
    /// Records why a formula was kept as its value (each reason once).
    fn unsupported(&mut self, why: &'static str) {
        if !self.unsupported.contains(&why) {
            self.unsupported.push(why);
        }
    }

    /// The format index a cell, row or column keeps: none for a plain or missing XF.
    fn format(&self, ixfe: u16) -> Option<u16> {
        (self.plain.get(usize::from(ixfe)) == Some(&false)).then_some(ixfe)
    }

    fn pos(&mut self, row: u16, col: u16) -> Option<CellRef> {
        let (r, c) = (u32::from(row), u32::from(col));
        if r >= GRID_ROWS || c >= GRID_COLS {
            self.dropped += 1;
            return None;
        }
        Some(CellRef::new(r, c))
    }

    fn put(&mut self, budget: &mut Budget, row: u16, col: u16, ixfe: u16, value: Value) -> Result<Option<CellRef>, Error> {
        let Some(pos) = self.pos(row, col) else { return Ok(None) };
        let format = self.format(ixfe);
        if value.is_empty() && format.is_none() {
            return Ok(Some(pos));
        }
        budget.cell()?;
        self.sheet.cells.insert(pos, Cell { value, format, formula: None });
        Ok(Some(pos))
    }

    /// Decodes a formula and attaches it to the cell at `pos`, or records why it can't be.
    fn attach(&mut self, pos: CellRef, p: &Parsed, mode: Mode, array: Option<Range>) {
        match decode(&p.rgce, &p.rgcb, pos, mode, &self.g.links) {
            // A one-cell Ctrl+Shift+Enter array keeps its range: it evaluates as an array.
            Ok(e) => self.sheet.cells.entry(pos).or_default().formula = Some(Formula { text: crate::expr::print(&e), array }),
            Err(Unsupported(why)) => self.unsupported(why),
        }
    }

    fn formula_at(&self, pos: CellRef) -> Option<&Formula> {
        self.sheet.cells.get(&pos).and_then(|c| c.formula.as_ref())
    }
}

/// RK number ([MS-XLS] 2.5.217): a 30-bit integer or the top of an IEEE double, maybe ×100.
fn rk(v: u32) -> Value {
    let n = if v & 2 != 0 { f64::from((v as i32) >> 2) } else { f64::from_bits(u64::from(v & !3) << 32) };
    Value::number(if v & 1 != 0 { n / 100.0 } else { n })
}

/// Attaches every pending formula to its anchor's template. An anchor whose template can't be
/// read is refused once: its followers re-decode the same template for nothing, so they are
/// skipped instead (they keep their cached values).
fn resolve_pending(
    st: &mut SheetState<'_>,
    budget: &mut Budget,
    arrays: &HashMap<CellRef, (Range, Parsed)>,
    shared: &HashMap<CellRef, Parsed>,
    pending: &[Pending],
) {
    const TOO_MUCH: &str = "more shared-formula text than the read allows";
    let mut failed: HashSet<CellRef> = HashSet::new();
    let mut shared_attaches = 0usize;
    for p in pending {
        if let Some((range, f)) = arrays.get(&p.anchor) {
            if p.pos == p.anchor {
                st.attach(p.pos, f, Mode::Cell, Some(*range));
            }
        } else if let Some(f) = shared.get(&p.anchor) {
            if budget.shared_text_spent {
                st.unsupported(TOO_MUCH);
                continue;
            }
            if failed.contains(&p.anchor) || shared_attaches >= MAX_SHARED_ATTACHES {
                continue;
            }
            shared_attaches += 1;
            st.attach(p.pos, f, Mode::Shared, None);
            // Every follower keeps its own text: charge it to the workbook's text allowance, and
            // past it drop the formula again so the cell keeps its cached value.
            if let Some(len) = st.formula_at(p.pos).map(|f| f.text.len())
                && !budget.try_shared_text(len)
            {
                if let Some(cell) = st.sheet.cells.get_mut(&p.pos) {
                    cell.formula = None;
                }
                st.unsupported(TOO_MUCH);
                continue;
            }
            if !st.formula_at(p.pos).is_some_and(|f| f.text.chars().count() <= MAX_SHARED_TEMPLATE) {
                failed.insert(p.anchor);
            }
        } else {
            st.unsupported("a shared formula that is missing from the file");
        }
    }
    if shared_attaches >= MAX_SHARED_ATTACHES {
        st.unsupported("more shared formulas than the expansion cap; some formulas kept their values");
    }
}

/// The other cells of a legacy array formula hold only cached results: blanked, so that the
/// formula fills its range on recalculation. If the formula itself couldn't be read, they keep
/// their values.
fn blank_array_followers(st: &mut SheetState<'_>, pending: &[Pending]) {
    for p in pending {
        let filled = st.formula_at(p.anchor).and_then(|f| f.array).is_some_and(|r| r.contains(p.pos));
        if p.pos == p.anchor || !filled {
            continue;
        }
        match st.sheet.cells.get_mut(&p.pos) {
            Some(cell) if cell.format.is_some() => *cell = Cell { value: Value::Empty, format: cell.format, formula: None },
            _ => {
                st.sheet.cells.remove(&p.pos);
            }
        }
    }
}

fn worksheet<'g>(
    stream: &[u8],
    bs: &BoundSheet,
    g: &'g Globals,
    plain: &'g [bool],
    budget: &mut Budget,
    report: &mut Report,
) -> Result<Option<SheetState<'g>>, Error> {
    if bs.pos >= stream.len() {
        return Err(invalid(format!("sheet '{}' points outside the Workbook stream", bs.name)));
    }
    let mut recs = Records::new(stream, bs.pos);
    let first = recs.next_record()?.ok_or_else(|| invalid("missing sheet substream"))?;
    let (vers, dt) = bof(&first)?;
    if vers != BIFF8 || dt != 0x0010 {
        return Ok(None);
    }
    let mut st = SheetState {
        g,
        plain,
        sheet: Sheet { name: bs.name.clone(), ..Sheet::default() },
        pending: vec![],
        shared: HashMap::new(),
        arrays: HashMap::new(),
        last_formula: None,
        pending_string: None,
        dropped: 0,
        unsupported: vec![],
        dv_left: 0,
        note_object: None,
        note_texts: HashMap::new(),
        notes: vec![],
        cf: None,
        palette: g.styles.color_table(),
        drawing: vec![],
        filtered: false,
    };
    let mut depth = 1usize;
    while let Some(rec) = recs.next_record()? {
        budget.record()?;
        match rec.id {
            BOF => {
                // An embedded chart's substream.
                depth += 1;
                continue;
            }
            EOF => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
                continue;
            }
            _ if depth > 1 => continue,
            WSBOOL if rec.parts.first().and_then(|p| p.first()).is_some_and(|b| b & WSBOOL_DIALOG != 0) => return Ok(None),
            _ => {}
        }
        sheet_record(&rec, &mut st, budget, report)?;
    }
    Ok(Some(st))
}

/// Attaches a sheet's shared and array formulas (their definitions follow the first cell that
/// uses them). Run for every sheet only after all sheets' mandatory text is charged, so the
/// optional shared-formula text can't use up the allowance a later sheet's strings need.
fn finish_sheet(mut st: SheetState<'_>, budget: &mut Budget, report: &mut Report) -> Sheet {
    let (arrays, shared) = (std::mem::take(&mut st.arrays), std::mem::take(&mut st.shared));
    let pending = std::mem::take(&mut st.pending);
    resolve_pending(&mut st, budget, &arrays, &shared, &pending);
    blank_array_followers(&mut st, &pending);
    let mut skipped_pictures = false;
    for shape in drawing::picture_shapes(&std::mem::take(&mut st.drawing)) {
        let blip = usize::try_from(shape.pib).ok().and_then(|i| i.checked_sub(1)).and_then(|i| st.g.blips.get(i)).cloned().flatten();
        match blip {
            Some((data, mime)) if st.sheet.pictures.len() < MAX_PICTURES => st.sheet.pictures.push(Picture { anchor: shape.anchor, data, mime }),
            _ => skipped_pictures = true,
        }
    }
    if skipped_pictures {
        report.warn(Warning::PicturesSkipped);
    }
    // A Note names its text's object, whose TxO came earlier in the substream.
    for (cell, object, author, visible) in std::mem::take(&mut st.notes) {
        let text = st.note_texts.get(&object).cloned().unwrap_or_default();
        st.sheet.notes.push(Note { cell, author, text, visible });
    }
    if st.dropped > 0 {
        report.warn(Warning::CellsOutsideGrid(st.dropped));
    }
    for why in &st.unsupported {
        report.warn(Warning::FormulaKept(why));
    }
    st.sheet
}

/// The shortest WINDOW2 a file holds (the chart-sheet form); a shorter one is truncated.
const WINDOW2_MIN_LEN: usize = 10;

#[allow(clippy::cognitive_complexity, reason = "one arm per worksheet record of [MS-XLS] 2.1.7.20.5")]
fn sheet_record(rec: &Record<'_>, st: &mut SheetState<'_>, budget: &mut Budget, report: &mut Report) -> Result<(), Error> {
    let mut c = Cursor::new(&rec.parts);
    // A formula's string result is the String record right after it (shared/array records may sit between).
    if !matches!(rec.id, 0x0207 | 0x04BC | 0x0221 | 0x0236) {
        st.pending_string = None;
    }
    match rec.id {
        0x0203 => {
            let (r, col, x) = (c.u16()?, c.u16()?, c.u16()?);
            let v = Value::number(c.f64()?);
            st.put(budget, r, col, x, v)?;
        }
        0x027E => {
            let (r, col, x) = (c.u16()?, c.u16()?, c.u16()?);
            let v = rk(c.u32()?);
            st.put(budget, r, col, x, v)?;
        }
        0x00BD => {
            let r = c.u16()?;
            let first = c.u16()?;
            let n = c.remaining().saturating_sub(2) / 6;
            for i in 0..n {
                let x = c.u16()?;
                let v = rk(c.u32()?);
                let col = u16::try_from(usize::from(first) + i).unwrap_or(u16::MAX);
                st.put(budget, r, col, x, v)?;
            }
        }
        0x00FD => {
            let (r, col, x) = (c.u16()?, c.u16()?, c.u16()?);
            let i = c.u32()? as usize;
            match st.g.sst.get(i) {
                Some(s) => {
                    st.put(budget, r, col, x, Value::Text(s.clone()))?;
                }
                None => report.warn(Warning::MissingSharedString),
            }
        }
        // Label, and RString (a label followed by formatting runs, which are skipped).
        0x0204 | 0x00D6 => {
            let (r, col, x) = (c.u16()?, c.u16()?, c.u16()?);
            let s = c.string()?;
            budget.text(s.len())?;
            st.put(budget, r, col, x, Value::Text(Arc::from(s)))?;
        }
        0x0205 => {
            let (r, col, x) = (c.u16()?, c.u16()?, c.u16()?);
            let (v, is_err) = (c.u8()?, c.u8()?);
            let v = if is_err != 0 { Value::Error(cell_error(v)) } else { Value::Bool(v != 0) };
            st.put(budget, r, col, x, v)?;
        }
        0x0201 => {
            let (r, col, x) = (c.u16()?, c.u16()?, c.u16()?);
            st.put(budget, r, col, x, Value::Empty)?;
        }
        0x00BE => {
            let r = c.u16()?;
            let first = c.u16()?;
            let n = c.remaining().saturating_sub(2) / 2;
            for i in 0..n {
                let x = c.u16()?;
                let col = u16::try_from(usize::from(first) + i).unwrap_or(u16::MAX);
                st.put(budget, r, col, x, Value::Empty)?;
            }
        }
        0x0006 => formula_cell(&mut c, st, budget)?,
        0x0207 => {
            if let Some((pos, format)) = st.pending_string.take() {
                let s = c.string()?;
                budget.text(s.len())?;
                let cell = st.sheet.cells.entry(pos).or_insert_with(|| Cell { value: Value::Empty, format, formula: None });
                cell.value = Value::Text(Arc::from(s));
            }
        }
        0x04BC => {
            // RefU (the cells that may use it), reserved, cUse.
            c.skip(8)?;
            let f = parsed(&mut c)?;
            if let Some(at) = st.last_formula {
                st.shared.insert(at, f);
            }
        }
        0x0221 => {
            let (r0, r1) = (c.u16()?, c.u16()?);
            let (c0, c1) = (c.u8()?, c.u8()?);
            c.skip(6)?;
            let f = parsed(&mut c)?;
            let range = Range::new(CellRef::new(u32::from(r0), u32::from(c0)), CellRef::new(u32::from(r1.max(r0)), u32::from(c1.max(c0))));
            if let Some(at) = st.last_formula {
                st.arrays.insert(at, (range, f));
            }
        }
        0x000D => {
            st.sheet.calc.mode = Some(match c.u16()? as i16 {
                0 => CalcMode::Manual,
                -1 => CalcMode::AutomaticExceptTables,
                _ => CalcMode::Automatic,
            });
        }
        0x000C => st.sheet.calc.max_iterations = Some(c.u16()?),
        0x0011 => st.sheet.calc.iterative = Some(c.u16()? == 1),
        0x0010 => st.sheet.calc.max_change = Some(c.f64()?),
        0x0236 => report.warn(Warning::DataTable),
        0x00E5 => merges(&mut c, st)?,
        0x007D => {
            let (first, last) = (c.u16()?, c.u16()?);
            let width = c.u16()?;
            let x = c.u16()?;
            let flags = c.u16()?;
            let column = Column {
                width,
                custom_width: flags & 0x0002 != 0,
                hidden: flags & 0x0001 != 0,
                outline: ((flags >> 8) & 7) as u8,
                collapsed: flags & 0x1000 != 0,
                format: st.format(x),
            };
            // An empty range when `first` is past `last`.
            for col in first..=last.min(255) {
                st.sheet.columns.insert(col, column);
            }
        }
        0x0208 => {
            let r = c.u16()?;
            c.skip(4)?;
            let height = c.u16()?;
            c.skip(4)?;
            let flags = c.u32()?;
            let format = if flags & 0x80 != 0 { st.format(((flags >> 16) & 0x0FFF) as u16) } else { None };
            let row = Row {
                height: height & 0x7FFF,
                custom_height: flags & 0x40 != 0,
                hidden: flags & 0x20 != 0,
                outline: (flags & 7) as u8,
                collapsed: flags & 0x10 != 0,
                format,
            };
            st.sheet.rows.insert(u32::from(r), row);
        }
        0x0225 => {
            let flags = c.u16()?;
            let height = c.u16()?;
            if flags & 0x0002 == 0 && height > 0 {
                st.sheet.default_row_height = Some(height);
            }
        }
        0x023E => {
            let flags = if c.remaining() >= WINDOW2_MIN_LEN { c.u16().ok() } else { None };
            let Some(flags) = flags else { return Ok(()) };
            let on = |bit: u16| flags & bit != 0;
            st.sheet.view = Some(View {
                show_formulas: on(0x0001),
                show_gridlines: on(0x0002),
                show_headings: on(0x0004),
                show_zeros: on(0x0010),
                right_to_left: on(0x0040),
            });
        }
        0x005D | 0x01B6 | 0x001C | 0x00EC => match object_record(rec, &mut c, st, budget, report) {
            Err(Error::Invalid(_)) => {
                if rec.id == 0x005D {
                    st.note_object = None;
                }
                report.warn(Warning::DamagedObjects);
            }
            r => r?,
        },
        0x009D => st.filtered = true,
        0x01B0 => match cond::condfmt(&mut c) {
            Ok(cf) => st.cf = Some(cf),
            Err(_) => {
                st.cf = None;
                report.warn(Warning::ConditionalFormattingSkipped);
            }
        },
        0x01B1 => {
            if conditional_format(&mut c, st).is_none() {
                report.warn(Warning::ConditionalFormattingSkipped);
            }
        }
        0x0879 => report.warn(Warning::ConditionalFormatting12),
        // A short DVal starts no rules.
        0x01B2 => st.dv_left = crate::dv::dval(&mut c).unwrap_or(0),
        0x01BE => {
            if st.dv_left == 0 {
                report.warn(Warning::DataValidationIgnored);
                return Ok(());
            }
            st.dv_left -= 1;
            let rule = match crate::dv::dv(&mut c) {
                Ok(raw) => {
                    budget.text(raw.text_len())?;
                    crate::dv::validation(raw, &st.g.links)
                }
                Err(_) => None,
            };
            match rule {
                Some(v) if st.sheet.validations.len() < crate::dv::MAX_RULES => st.sheet.validations.push(v),
                _ => report.warn(Warning::DataValidationSkipped),
            }
        }
        0x01B8 => report.warn(Warning::Hyperlinks),
        0x00B0 => report.warn(Warning::PivotTables),
        _ => {}
    }
    Ok(())
}

/// The drawing-layer records of a sheet: comment objects (Obj, TxO, Note) and the drawing
/// stream (MsoDrawing) that places pictures.
fn object_record(rec: &Record<'_>, c: &mut Cursor<'_, '_>, st: &mut SheetState<'_>, budget: &mut Budget, report: &mut Report) -> Result<(), Error> {
    match rec.id {
        // Obj: FtCmo first (ft 0x15, cb, object type, id).
        0x005D => {
            let ft = c.u16()?;
            c.u16()?;
            let ot = c.u16()?;
            let id = c.u16()?;
            st.note_object = (ft == 0x15 && ot == notes::OT_NOTE).then_some(id);
            if ft == 0x15 && ot == 5 {
                report.warn(Warning::Charts);
            }
        }
        // TxO: the text of the comment object just described.
        0x01B6 => {
            if let Some(id) = st.note_object.take() {
                c.skip(10)?;
                let cch = usize::from(c.u16()?);
                let text = notes::txo_text(&rec.parts, cch);
                budget.text(text.len())?;
                st.note_texts.insert(id, text);
            }
        }
        // Note: the cell, flags (0x0002: shown), the object id and the author.
        0x001C => {
            let (row, col, flags, object) = (c.u16()?, c.u16()?, c.u16()?, c.u16()?);
            let author = c.string().unwrap_or_default();
            budget.text(author.len())?;
            if let Some(cell) = st.pos(row, col)
                && st.notes.len() < MAX_NOTES
            {
                st.notes.push((cell, object, author, flags & 0x0002 != 0));
            }
        }
        0x00EC if st.drawing.len() < MAX_DRAWING => st.drawing.extend(c.rest()),
        _ => {}
    }
    Ok(())
}

/// One CF record of the current CondFmt; `None` when it is not kept.
fn conditional_format(c: &mut Cursor<'_, '_>, st: &mut SheetState<'_>) -> Option<()> {
    let g = st.g;
    let rule = cond::cf(c, &st.palette, |ifmt| g.formats.get(&ifmt).cloned()).ok()?;
    let (base, ranges) = st.cf.as_ref()?;
    if ranges.is_empty() || st.sheet.conditional_formats.len() >= cond::MAX_RULES {
        return None;
    }
    // The formulas are offsets from the top left cell of the CondFmt's bounding range.
    let formula =
        |rgce: &[u8]| (!rgce.is_empty()).then(|| decode(rgce, &[], *base, Mode::Cell, &g.links).ok().map(|e| crate::expr::print(&e))).flatten();
    let rule_kind = match (rule.kind, rule.operator) {
        (1, Some(operator)) => ConditionRule::CellIs { operator, formula1: formula(&rule.rgce1)?, formula2: formula(&rule.rgce2) },
        (2, _) => ConditionRule::Expression { formula: formula(&rule.rgce1)? },
        _ => return None,
    };
    st.sheet.conditional_formats.push(ConditionalFormat { ranges: ranges.clone(), rule: rule_kind, format: rule.format });
    Some(())
}

/// MergeCells ([MS-XLS] 2.4.168).
fn merges(c: &mut Cursor<'_, '_>, st: &mut SheetState<'_>) -> Result<(), Error> {
    let n = usize::from(c.u16()?);
    for _ in 0..n.min(c.remaining() / 8) {
        let (r0, r1, c0, c1) = (c.u16()?, c.u16()?, c.u16()?, c.u16()?);
        if r1 < r0 || c1 < c0 || u32::from(c1) >= GRID_COLS {
            continue;
        }
        if st.sheet.merges.len() >= MAX_MERGES {
            return Err(Error::TooLarge("too many merged ranges".into()));
        }
        let range = Range::new(CellRef::new(u32::from(r0), u32::from(c0)), CellRef::new(u32::from(r1), u32::from(c1)));
        if !range.is_single() {
            st.sheet.merges.push(range);
        }
    }
    Ok(())
}

fn formula_cell(c: &mut Cursor<'_, '_>, st: &mut SheetState<'_>, budget: &mut Budget) -> Result<(), Error> {
    let (r, col, x) = (c.u16()?, c.u16()?, c.u16()?);
    let mut val = [0u8; 8];
    for b in val.iter_mut() {
        *b = c.u8()?;
    }
    let _flags = c.u16()?;
    c.skip(4)?;
    let f = parsed(c)?;
    let [b0, _, b2, _, _, _, e0, e1] = val;
    let mut string_result = false;
    let value = if [e0, e1] == [0xFF, 0xFF] {
        match b0 {
            0x00 => {
                string_result = true;
                Value::Empty
            }
            0x01 => Value::Bool(b2 != 0),
            0x02 => Value::Error(cell_error(b2)),
            0x03 => Value::Text(Arc::from("")),
            _ => Value::Empty,
        }
    } else {
        Value::number(f64::from_le_bytes(val))
    };
    let Some(pos) = st.pos(r, col) else { return Ok(()) };
    budget.cell()?;
    let format = st.format(x);
    st.sheet.cells.insert(pos, Cell { value, format, formula: None });
    st.last_formula = Some(pos);
    if string_result {
        st.pending_string = Some((pos, format));
    }
    // PtgExp: part of a shared or array formula anchored at another cell.
    if f.rgce.len() == 5 && f.rgce.first() == Some(&0x01) {
        let get = |i: usize| f.rgce.get(i).copied().unwrap_or(0);
        let anchor = CellRef::new(u32::from(u16::from_le_bytes([get(1), get(2)])), u32::from(u16::from_le_bytes([get(3), get(4)])));
        st.pending.push(Pending { pos, anchor });
        return Ok(());
    }
    st.attach(pos, &f, Mode::Cell, None);
    Ok(())
}

fn defined_names(g: &Globals, model_index: &[Option<usize>], filtered: &[bool], wb: &mut Workbook, report: &mut Report) {
    let scope_of = |itab: u16| -> Option<Option<usize>> {
        if itab == 0 {
            return Some(None);
        }
        model_index.get(usize::from(itab) - 1).copied().flatten().map(Some)
    };
    for lbl in &g.names {
        let Some(scope) = scope_of(lbl.itab) else { continue };
        let text = match decode(&lbl.rgce, &lbl.rgcb, CellRef::new(0, 0), Mode::Name, &g.links) {
            Ok(e) => crate::expr::print(&e),
            Err(_) if lbl.rgce.is_empty() => continue,
            Err(Unsupported(why)) => {
                report.warn(Warning::NameSkipped(why));
                continue;
            }
        };
        match lbl.builtin {
            Some(6 | 7) => {
                let Some(sheet) = scope.and_then(|si| wb.sheets.get_mut(si)) else { continue };
                if lbl.builtin == Some(6) {
                    sheet.print_area = Some(text);
                } else {
                    sheet.print_titles = Some(text);
                }
            }
            // _FilterDatabase: the AutoFilter's range, on a sheet whose filter is on.
            Some(13) => {
                let Some(si) = scope else { continue };
                if !filtered.get(si).copied().unwrap_or(false) {
                    continue;
                }
                let area = text.rsplit('!').next().unwrap_or("").replace('$', "");
                if let (Some(range), Some(sheet)) = (Range::parse(&area), wb.sheets.get_mut(si))
                    && sheet.autofilter.is_none()
                {
                    sheet.autofilter = Some(range);
                }
            }
            Some(_) => {}
            None if lbl.macro_name => report.warn(Warning::MacroNames),
            None => wb.names.push(DefinedName { name: lbl.name.clone(), scope, formula: text, hidden: lbl.hidden }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refused_shared_formula_ends_shared_expansion_for_later_sheets() {
        let mut budget = Budget::new(Limits { max_text: 10, ..Limits::default() });
        assert!(budget.try_shared_text(8));
        assert!(!budget.try_shared_text(8), "over the allowance");
        // a shorter formula would fit the 2 bytes left, but the cut-off holds
        assert!(!budget.try_shared_text(1));
        assert!(budget.try_text(1), "text the read needs is still charged normally");
    }
}
