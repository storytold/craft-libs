//! `content.xml`: the document body's tables, read as a stream of events.

use craft_sheet::{CellRef, MAX_COLS, MAX_ROWS, Range};
use craft_xml::{Canonical, Doctype, Event, Options as XmlOptions};

use crate::model::{Cell, Sheet, ValueType, Workbook};
use crate::{Error, Options};

const NAMESPACES: &[(&str, &str)] = &[
    ("office", "urn:oasis:names:tc:opendocument:xmlns:office:1.0"),
    ("table", "urn:oasis:names:tc:opendocument:xmlns:table:1.0"),
    ("text", "urn:oasis:names:tc:opendocument:xmlns:text:1.0"),
    ("calcext", "urn:org:documentfoundation:names:experimental:calc:xmlns:calcext:1.0"),
];
/// Most tables in one document.
const MAX_SHEETS: usize = 1024;

fn xml_error(e: impl std::fmt::Display) -> Error {
    Error::Xml(format!("ODS: {e}"))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tag {
    Document,
    Body,
    Spreadsheet,
    Table,
    Rows,
    Row,
    Cell,
    Covered,
    Paragraph,
    Inline,
    Space,
    Tab,
    Break,
    Other,
}

fn tag(name: &str) -> Tag {
    match name {
        "office:document-content" => Tag::Document,
        "office:body" => Tag::Body,
        "office:spreadsheet" => Tag::Spreadsheet,
        "table:table" => Tag::Table,
        "table:table-row-group" | "table:table-header-rows" | "table:table-rows" => Tag::Rows,
        "table:table-row" => Tag::Row,
        "table:table-cell" => Tag::Cell,
        "table:covered-table-cell" => Tag::Covered,
        "text:p" | "text:h" => Tag::Paragraph,
        "text:span" | "text:a" | "text:meta" | "text:meta-field" | "text:ruby" | "text:ruby-base" => Tag::Inline,
        "text:s" => Tag::Space,
        "text:tab" => Tag::Tab,
        "text:line-break" => Tag::Break,
        _ => Tag::Other,
    }
}

struct SheetDraft {
    sheet: Sheet,
    depth: usize,
    row: u32,
    keep: bool,
}

struct RowDraft {
    depth: usize,
    repeat: u32,
    col: u32,
    cells: Vec<RepeatedCell>,
}

struct RepeatedCell {
    col: u32,
    repeat: u32,
    width: u32,
    height: u32,
    cell: Cell,
}

struct CellDraft {
    depth: usize,
    repeat: u32,
    width: u32,
    height: u32,
    cell: Cell,
    paragraph: Option<usize>,
    paragraph_started: bool,
    pending_space: bool,
}

struct Import<'o> {
    options: &'o Options,
    wb: Workbook,
    stack: Vec<Tag>,
    sheet: Option<SheetDraft>,
    row: Option<RowDraft>,
    cell: Option<CellDraft>,
    cells: u64,
    merges: u64,
    text: u64,
    spreadsheet_seen: bool,
}

/// Reads the tables of a `content.xml` (the XML alone, for a caller with its own package).
pub fn read_content(bytes: &[u8], options: &Options) -> Result<Workbook, Error> {
    let canonical = Canonical::new(NAMESPACES);
    let xml = XmlOptions { doctype: Doctype::Reject, ..XmlOptions::canonical(&canonical).strict() };
    let text = craft_xml::to_utf8(bytes);
    let mut events = xml.events(&text);
    let mut import = Import {
        options,
        wb: Workbook::default(),
        stack: vec![],
        sheet: None,
        row: None,
        cell: None,
        cells: 0,
        merges: 0,
        text: 0,
        spreadsheet_seen: false,
    };
    let mut root_seen = false;
    while let Some(event) = events.next().map_err(xml_error)? {
        match event {
            Event::Start { name, attrs } => {
                if import.stack.is_empty() {
                    if root_seen || tag(&name) != Tag::Document {
                        return Err(xml_error("expected one office:document-content root"));
                    }
                    root_seen = true;
                }
                import.start(&name, &attrs)?;
            }
            Event::End { .. } => import.end()?,
            Event::Text(t) | Event::CData(t) => import.append(&t)?,
            Event::Pi { .. } | Event::Comment(_) => {}
        }
    }
    if !root_seen || !import.stack.is_empty() || !import.spreadsheet_seen {
        return Err(xml_error("incomplete ODS document"));
    }
    if import.wb.sheets.is_empty() {
        return Err(Error::Invalid("the spreadsheet has no tables".into()));
    }
    Ok(import.wb)
}

impl Import<'_> {
    fn attr(&self, attrs: &[(String, String)], name: &str) -> Result<Option<String>, Error> {
        match attrs.iter().find(|(k, _)| k == name) {
            Some((_, v)) if v.len() > self.options.max_cell_text => {
                Err(Error::TooLarge(format!("an attribute is longer than {} bytes", self.options.max_cell_text)))
            }
            Some((_, v)) => Ok(Some(v.clone())),
            None => Ok(None),
        }
    }

    /// A repeat or span count: 1 when absent, refused when 0, unreadable or over `limit`.
    fn count(&self, attrs: &[(String, String)], name: &str, limit: u32) -> Result<u32, Error> {
        let n = match self.attr(attrs, name)? {
            Some(s) => s.parse::<u32>().map_err(|_| Error::Invalid("invalid ODS repeat or span count".into()))?,
            None => 1,
        };
        if n == 0 || n > limit {
            return Err(Error::TooLarge("ODS repeat or span exceeds worksheet bounds".into()));
        }
        Ok(n)
    }

    fn start(&mut self, name: &str, attrs: &[(String, String)]) -> Result<(), Error> {
        let t = tag(name);
        let depth = self.stack.len();
        match t {
            Tag::Spreadsheet if self.stack == [Tag::Document, Tag::Body] => {
                if self.spreadsheet_seen {
                    return Err(xml_error("multiple spreadsheets in one document"));
                }
                self.spreadsheet_seen = true;
            }
            Tag::Table if self.stack == [Tag::Document, Tag::Body, Tag::Spreadsheet] => {
                if self.wb.sheets.len() >= MAX_SHEETS {
                    return Err(Error::TooLarge(format!("more than {MAX_SHEETS} sheets")));
                }
                let keep = self.options.only_sheet.is_none_or(|i| i == self.wb.sheets.len());
                let name = self.attr(attrs, "table:name")?.unwrap_or_default();
                self.sheet = Some(SheetDraft { sheet: Sheet { name, ..Sheet::default() }, depth, row: 0, keep });
            }
            Tag::Row
                if self
                    .sheet
                    .as_ref()
                    .is_some_and(|s| s.keep && self.stack.get(s.depth + 1..).is_some_and(|p| p.iter().all(|t| *t == Tag::Rows))) =>
            {
                self.row = Some(RowDraft { depth, repeat: self.count(attrs, "table:number-rows-repeated", u32::MAX)?, col: 0, cells: vec![] });
            }
            Tag::Cell | Tag::Covered if self.row.as_ref().is_some_and(|r| depth == r.depth + 1) => {
                let value_type = if self.attr(attrs, "calcext:value-type")?.as_deref() == Some("error") {
                    Some(ValueType::Error)
                } else {
                    self.attr(attrs, "office:value-type")?.as_deref().and_then(ValueType::parse)
                };
                let key = match value_type {
                    Some(ValueType::Boolean) => "office:boolean-value",
                    Some(ValueType::Date) => "office:date-value",
                    Some(ValueType::Time) => "office:time-value",
                    Some(ValueType::String) => "office:string-value",
                    _ => "office:value",
                };
                self.cell = Some(CellDraft {
                    depth,
                    repeat: self.count(attrs, "table:number-columns-repeated", u32::MAX)?,
                    width: self.count(attrs, "table:number-columns-spanned", MAX_COLS)?,
                    height: self.count(attrs, "table:number-rows-spanned", MAX_ROWS)?,
                    cell: Cell {
                        value_type,
                        raw: self.attr(attrs, key)?,
                        currency: self.attr(attrs, "office:currency")?,
                        formula: self.attr(attrs, "table:formula")?,
                        style: self.attr(attrs, "table:style-name")?,
                        covered: t == Tag::Covered,
                        ..Cell::default()
                    },
                    paragraph: None,
                    paragraph_started: false,
                    pending_space: false,
                });
            }
            Tag::Paragraph if self.cell.as_ref().is_some_and(|c| depth == c.depth + 1) => {
                let max = self.options.max_cell_text;
                if let Some(cell) = self.cell.as_mut() {
                    if cell.cell.paragraphs > 0 {
                        push_text(&mut cell.cell.text, "\n", max)?;
                    }
                    cell.paragraph = Some(depth);
                    cell.cell.paragraphs += 1;
                    cell.paragraph_started = false;
                    cell.pending_space = false;
                }
            }
            Tag::Space | Tag::Tab | Tag::Break if self.captures_text() => {
                let max = self.options.max_cell_text;
                let count = match t {
                    Tag::Space => self.attr(attrs, "text:c")?.map(|s| s.parse::<usize>()).transpose().map_err(xml_error)?.unwrap_or(1),
                    _ => 1,
                };
                let unit = match t {
                    Tag::Space => " ",
                    Tag::Tab => "\t",
                    _ => "\n",
                };
                if let Some(cell) = self.cell.as_mut() {
                    if cell.pending_space {
                        push_text(&mut cell.cell.text, " ", max)?;
                        cell.pending_space = false;
                    }
                    cell.paragraph_started = true;
                    if cell.cell.text.len().saturating_add(count) > max {
                        return Err(Error::TooLarge(format!("a cell's text is longer than {max} bytes")));
                    }
                    cell.cell.text.push_str(&unit.repeat(count));
                }
            }
            _ => {}
        }
        self.stack.push(t);
        Ok(())
    }

    /// Inside a cell's paragraph, through inline elements only.
    fn captures_text(&self) -> bool {
        self.cell
            .as_ref()
            .and_then(|c| c.paragraph)
            .is_some_and(|depth| self.stack.get(depth..).is_some_and(|p| p.iter().all(|t| matches!(t, Tag::Paragraph | Tag::Inline))))
    }

    /// Text, with ODF's white-space rule: runs collapse to one space, none at a paragraph's start.
    fn append(&mut self, text: &str) -> Result<(), Error> {
        if !self.captures_text() {
            return Ok(());
        }
        let max = self.options.max_cell_text;
        let Some(cell) = self.cell.as_mut() else { return Ok(()) };
        for ch in text.chars() {
            if matches!(ch, ' ' | '\t' | '\n' | '\r') {
                cell.pending_space = cell.paragraph_started;
            } else {
                if cell.pending_space {
                    push_text(&mut cell.cell.text, " ", max)?;
                    cell.pending_space = false;
                }
                push_text(&mut cell.cell.text, ch.encode_utf8(&mut [0; 4]), max)?;
                cell.paragraph_started = true;
            }
        }
        Ok(())
    }

    fn end(&mut self) -> Result<(), Error> {
        self.stack.pop().ok_or_else(|| xml_error("unexpected closing element"))?;
        let depth = self.stack.len();
        if let Some(cell) = self.cell.as_mut()
            && cell.paragraph == Some(depth)
        {
            cell.paragraph = None;
        }
        if self.cell.as_ref().is_some_and(|c| c.depth == depth) {
            self.finish_cell()?;
        }
        if self.row.as_ref().is_some_and(|r| r.depth == depth) {
            self.finish_row()?;
        }
        if self.sheet.as_ref().is_some_and(|s| s.depth == depth)
            && let Some(sheet) = self.sheet.take()
        {
            self.wb.sheets.push(sheet.sheet);
        }
        Ok(())
    }

    fn charge_text(&mut self, bytes: u64) -> Result<(), Error> {
        self.text = self.text.saturating_add(bytes);
        if self.text > self.options.max_text {
            return Err(Error::TooLarge(format!("more than {} MiB of text", self.options.max_text >> 20)));
        }
        Ok(())
    }

    fn finish_cell(&mut self) -> Result<(), Error> {
        let Some(draft) = self.cell.take() else { return Ok(()) };
        let (width, height) = if draft.cell.covered { (1, 1) } else { (draft.width, draft.height) };
        let Some(row) = self.row.as_mut() else { return Ok(()) };
        let kept = !draft.cell.is_blank() || width > 1 || height > 1;
        let end = match row.col.checked_add(draft.repeat).filter(|n| *n <= MAX_COLS) {
            Some(end) => end,
            None if !kept => MAX_COLS,
            None => return Err(Error::TooLarge("a row is wider than the grid".into())),
        };
        let start = row.col;
        row.col = end;
        if kept {
            let len = draft.cell.text.len() + draft.cell.raw.as_ref().map_or(0, String::len);
            self.charge_text(u64::from(draft.repeat).saturating_mul(len as u64))?;
            if let Some(row) = self.row.as_mut() {
                row.cells.push(RepeatedCell { col: start, repeat: draft.repeat, width, height, cell: draft.cell });
            }
        }
        Ok(())
    }

    fn finish_row(&mut self) -> Result<(), Error> {
        let Some(row) = self.row.take() else { return Ok(()) };
        let Some(sheet) = self.sheet.as_mut() else { return Ok(()) };
        let end = match sheet.row.checked_add(row.repeat).filter(|n| *n <= MAX_ROWS) {
            Some(end) => end,
            None if row.cells.is_empty() => MAX_ROWS,
            None => return Err(Error::TooLarge("the sheet is taller than the grid".into())),
        };
        let first_row = sheet.row;
        sheet.row = end;
        for entry in row.cells {
            let count = u64::from(entry.repeat) * u64::from(row.repeat);
            if !entry.cell.is_blank() {
                self.cells = self.cells.saturating_add(count);
                let len = entry.cell.text.len() + entry.cell.raw.as_ref().map_or(0, String::len);
                // The first row's copies were charged with the cell.
                self.text = self.text.saturating_add(count.saturating_sub(u64::from(entry.repeat)).saturating_mul(len as u64));
            }
            let merged = entry.width > 1 || entry.height > 1;
            if merged {
                self.merges = self.merges.saturating_add(count);
            }
            if self.cells > self.options.max_cells || self.merges > self.options.max_merges || self.text > self.options.max_text {
                return Err(Error::TooLarge(format!(
                    "more than {} cells, {} merged ranges or {} MiB of text",
                    self.options.max_cells,
                    self.options.max_merges,
                    self.options.max_text >> 20
                )));
            }
            let last_col = entry.col.checked_add(entry.repeat - 1).and_then(|c| c.checked_add(entry.width - 1));
            let last_row = end.checked_add(entry.height - 1);
            if last_col.is_none_or(|c| c >= MAX_COLS) || last_row.is_none_or(|r| r > MAX_ROWS) {
                return Err(Error::TooLarge("a merged cell reaches outside the grid".into()));
            }
            if merged && ((entry.repeat > 1 && entry.width > 1) || (row.repeat > 1 && entry.height > 1)) {
                return Err(Error::Invalid("repeated cells have overlapping merges".into()));
            }
            let Some(sheet) = self.sheet.as_mut() else { return Ok(()) };
            for r in first_row..end {
                for c in entry.col..entry.col + entry.repeat {
                    let at = CellRef::new(r, c);
                    if !entry.cell.is_blank() {
                        sheet.sheet.cells.insert(at, entry.cell.clone());
                    }
                    if merged {
                        sheet.sheet.merges.push(Range::new(at, CellRef::new(r + entry.height - 1, c + entry.width - 1)));
                    }
                }
            }
        }
        Ok(())
    }
}

fn push_text(out: &mut String, text: &str, max: usize) -> Result<(), Error> {
    if out.len().saturating_add(text.len()) > max {
        return Err(Error::TooLarge(format!("a cell's text is longer than {max} bytes")));
    }
    out.push_str(text);
    Ok(())
}
