//! `content.xml`: the document body's tables, read as a stream of events. What is not a table's
//! rows, columns or cells is kept as written, as element trees.

use craft_sheet::{CellRef, MAX_COLS, MAX_ROWS, Range};
use craft_xml::{Canonical, Doctype, El, Event, Node, Options as XmlOptions};

use crate::model::{Blank, Cell, Group, Line, Link, Run, Sheet, ValueType, Visibility, Workbook};
use crate::ns::NAMESPACES;
use crate::{Error, Options};

/// Most tables in one document.
const MAX_SHEETS: usize = 1024;
/// Most styled stretches kept for one cell's text.
const MAX_RUNS: usize = 4096;
/// What an element, attribute, text, line or range costs against [`Options::max_kept`] besides
/// its strings: about what it takes in memory.
const NODE_COST: u64 = 64;

fn xml_error(e: impl std::fmt::Display) -> Error {
    Error::Xml(format!("ODS: {e}"))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tag {
    Document,
    Body,
    Spreadsheet,
    Table,
    /// `table:table-rows`.
    Rows,
    RowGroup,
    HeaderRows,
    Row,
    /// `table:table-columns`.
    Columns,
    ColumnGroup,
    HeaderColumns,
    Column,
    Cell,
    Covered,
    Paragraph,
    Span,
    Link,
    Inline,
    Space,
    Tab,
    Break,
    Other,
}

impl Tag {
    fn holds_rows(self) -> bool {
        matches!(self, Tag::Rows | Tag::RowGroup | Tag::HeaderRows)
    }
    fn holds_columns(self) -> bool {
        matches!(self, Tag::Columns | Tag::ColumnGroup | Tag::HeaderColumns)
    }
    /// What a table's own reading handles: its other children are kept as trees.
    fn is_table_structure(self) -> bool {
        self.holds_rows() || self.holds_columns() || matches!(self, Tag::Row | Tag::Column)
    }
    fn is_inline(self) -> bool {
        matches!(self, Tag::Paragraph | Tag::Span | Tag::Link | Tag::Inline)
    }
}

fn tag(name: &str) -> Tag {
    match name {
        "office:document-content" => Tag::Document,
        "office:body" => Tag::Body,
        "office:spreadsheet" => Tag::Spreadsheet,
        "table:table" => Tag::Table,
        "table:table-rows" => Tag::Rows,
        "table:table-row-group" => Tag::RowGroup,
        "table:table-header-rows" => Tag::HeaderRows,
        "table:table-row" => Tag::Row,
        "table:table-columns" => Tag::Columns,
        "table:table-column-group" => Tag::ColumnGroup,
        "table:table-header-columns" => Tag::HeaderColumns,
        "table:table-column" => Tag::Column,
        "table:table-cell" => Tag::Cell,
        "table:covered-table-cell" => Tag::Covered,
        "text:p" | "text:h" => Tag::Paragraph,
        "text:span" => Tag::Span,
        "text:a" => Tag::Link,
        "text:meta" | "text:meta-field" | "text:ruby" | "text:ruby-base" => Tag::Inline,
        "text:s" => Tag::Space,
        "text:tab" => Tag::Tab,
        "text:line-break" => Tag::Break,
        _ => Tag::Other,
    }
}

fn visibility(s: Option<&str>) -> Visibility {
    match s {
        Some("collapse") => Visibility::Collapse,
        Some("filter") => Visibility::Filter,
        _ => Visibility::Visible,
    }
}

/// Where a tree being kept goes once it is complete.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    FontFaces,
    AutomaticStyles,
    Body,
    Table,
    Annotation,
    Drawing,
}

/// A tree being kept: its elements still open, outermost first.
struct Capture {
    target: Target,
    open: Vec<El>,
}

/// An open row or column group, or header rows or columns: where it started.
struct OpenLines {
    depth: usize,
    first: u32,
    display: bool,
}

struct SheetDraft {
    sheet: Sheet,
    depth: usize,
    row: u32,
    col: u32,
    keep: bool,
    row_groups: Vec<OpenLines>,
    column_groups: Vec<OpenLines>,
    header_rows: Option<OpenLines>,
    header_columns: Option<OpenLines>,
}

struct RowDraft {
    depth: usize,
    repeat: u32,
    col: u32,
    style: Option<String>,
    default_cell_style: Option<String>,
    visibility: Visibility,
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
    /// White space waiting to be written as one space, with the style it was written in: not
    /// that of a span opened after it.
    pending_space: Option<Option<String>>,
    /// The spans open in the paragraph: their depth and style.
    spans: Vec<(usize, Option<String>)>,
}

impl CellDraft {
    /// The style of the innermost styled span open.
    fn style(&self) -> Option<String> {
        self.spans.iter().rev().find_map(|(_, s)| s.clone())
    }

    /// Appends `text` to the cell's text, in the style of the innermost styled span open.
    fn push(&mut self, text: &str, max: usize) -> Result<(), Error> {
        if let Some(style) = self.pending_space.take() {
            self.push_in(" ", style, max)?;
        }
        let style = self.style();
        self.push_in(text, style, max)
    }

    fn push_in(&mut self, text: &str, style: Option<String>, max: usize) -> Result<(), Error> {
        let start = self.cell.text.len();
        push_text(&mut self.cell.text, text, max)?;
        let Some(style) = style else { return Ok(()) };
        let end = self.cell.text.len();
        if let Some(last) = self.cell.runs.last_mut()
            && last.end == start
            && last.style == style
        {
            last.end = end;
        } else if self.cell.runs.len() < MAX_RUNS {
            self.cell.runs.push(Run { start, end, style });
        }
        Ok(())
    }
}

struct Import<'o> {
    options: &'o Options,
    wb: Workbook,
    stack: Vec<Tag>,
    sheet: Option<SheetDraft>,
    row: Option<RowDraft>,
    cell: Option<CellDraft>,
    capture: Option<Capture>,
    cells: u64,
    merges: u64,
    text: u64,
    kept: u64,
    spreadsheet_seen: bool,
}

/// Reads the tables of a `content.xml` (the XML alone, for a caller with its own package).
pub fn read_content(bytes: &[u8], options: &Options) -> Result<Workbook, Error> {
    read_content_kept(bytes, options).map(|(wb, _)| wb)
}

/// [`read_content`], and what it charged against [`Options::max_kept`].
pub(crate) fn read_content_kept(bytes: &[u8], options: &Options) -> Result<(Workbook, u64), Error> {
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
        capture: None,
        cells: 0,
        merges: 0,
        text: 0,
        kept: 0,
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
                import.start(name, attrs)?;
            }
            Event::End { .. } => import.end()?,
            Event::Text(t) | Event::CData(t) => import.append(t)?,
            Event::Pi { .. } | Event::Comment(_) => {}
        }
    }
    if !root_seen || !import.stack.is_empty() || !import.spreadsheet_seen {
        return Err(xml_error("incomplete ODS document"));
    }
    if import.wb.sheets.is_empty() {
        return Err(Error::Invalid("the spreadsheet has no tables".into()));
    }
    Ok((import.wb, import.kept))
}

/// What a tree costs against [`Options::max_kept`].
pub(crate) fn tree_cost(e: &El) -> u64 {
    let own = NODE_COST + e.name.len() as u64 + e.attrs.iter().map(|(k, v)| NODE_COST + (k.len() + v.len()) as u64).sum::<u64>();
    e.children.iter().fold(own, |sum, n| {
        sum.saturating_add(match n {
            Node::El(c) => tree_cost(c),
            Node::Text(t) | Node::CData(t) | Node::Comment(t) => NODE_COST + t.len() as u64,
            Node::Pi { target, data } => NODE_COST + (target.len() + data.len()) as u64,
        })
    })
}

/// What a cell's trees cost, for each further copy of it.
fn cell_trees_cost(cell: &Cell) -> u64 {
    cell.annotation.iter().chain(&cell.drawings).map(tree_cost).fold(0, u64::saturating_add)
}

fn attrs_cost(attrs: &[(String, String)]) -> u64 {
    attrs.iter().map(|(k, v)| NODE_COST + (k.len() + v.len()) as u64).fold(0, u64::saturating_add)
}

/// What a non-blank cell's copy keeps besides its text and trees: its styled stretches and link.
fn cell_extras_cost(cell: &Cell) -> u64 {
    let runs = cell.runs.iter().map(|r| NODE_COST + r.style.len() as u64).fold(0, u64::saturating_add);
    let link = cell
        .link
        .as_ref()
        .map_or(0, |l| NODE_COST + (l.href.len() + l.title.as_ref().map_or(0, String::len) + l.target_frame.as_ref().map_or(0, String::len)) as u64);
    runs.saturating_add(link)
}

fn line_cost(a: Option<&String>, b: Option<&String>) -> u64 {
    NODE_COST + a.map_or(0, String::len) as u64 + b.map_or(0, String::len) as u64
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

    /// Refuses an attribute longer than [`Options::max_cell_text`] among `attrs`.
    fn check_attrs(&self, attrs: &[(String, String)]) -> Result<(), Error> {
        if attrs.iter().any(|(_, v)| v.len() > self.options.max_cell_text) {
            return Err(Error::TooLarge(format!("an attribute is longer than {} bytes", self.options.max_cell_text)));
        }
        Ok(())
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

    fn charge_kept(&mut self, bytes: u64) -> Result<(), Error> {
        self.kept = self.kept.saturating_add(bytes);
        if self.kept > self.options.max_kept {
            return Err(Error::TooLarge(format!("more than {} MiB of styles, notes, drawings and other parts", self.options.max_kept >> 20)));
        }
        Ok(())
    }

    /// The sheet being read, when its contents are kept.
    fn kept_sheet(&self) -> Option<&SheetDraft> {
        self.sheet.as_ref().filter(|s| s.keep)
    }

    /// Inside a kept sheet, with only `allowed` tags between the sheet's element and here.
    fn inside_sheet(&self, allowed: impl Fn(Tag) -> bool) -> bool {
        self.kept_sheet().is_some_and(|s| self.stack.get(s.depth + 1..).is_some_and(|p| p.iter().all(|t| allowed(*t))))
    }

    /// The tree an element starts, if it is one that is kept as written.
    fn capture_target(&self, t: Tag, name: &str, depth: usize) -> Option<Target> {
        match self.stack.as_slice() {
            [Tag::Document] => {
                return match name {
                    "office:font-face-decls" if self.wb.font_faces.is_none() => Some(Target::FontFaces),
                    "office:automatic-styles" if self.wb.automatic_styles.is_none() => Some(Target::AutomaticStyles),
                    _ => None,
                };
            }
            [Tag::Document, Tag::Body, Tag::Spreadsheet] if t != Tag::Table => return Some(Target::Body),
            _ => {}
        }
        if let Some(cell) = &self.cell
            && depth == cell.depth + 1
        {
            return if name == "office:annotation" {
                cell.cell.annotation.is_none().then_some(Target::Annotation)
            } else if name.starts_with("draw:") || name.starts_with("dr3d:") {
                Some(Target::Drawing)
            } else {
                None
            };
        }
        // A table's own child that is not its rows or columns.
        (self.kept_sheet().is_some_and(|s| depth == s.depth + 1) && !t.is_table_structure()).then_some(Target::Table)
    }

    fn start(&mut self, name: String, attrs: Vec<(String, String)>) -> Result<(), Error> {
        let t = tag(&name);
        let depth = self.stack.len();
        let target = if self.capture.is_some() { None } else { self.capture_target(t, &name, depth) };
        if self.capture.is_some() || target.is_some() {
            self.check_attrs(&attrs)?;
            let el = El { name, attrs, children: vec![] };
            self.charge_kept(tree_cost(&el))?;
            match (self.capture.as_mut(), target) {
                (Some(c), _) => c.open.push(el),
                (None, Some(target)) => self.capture = Some(Capture { target, open: vec![el] }),
                (None, None) => {}
            }
            self.stack.push(Tag::Other);
            return Ok(());
        }
        match t {
            Tag::Spreadsheet if self.stack == [Tag::Document, Tag::Body] => {
                if self.spreadsheet_seen {
                    return Err(xml_error("multiple spreadsheets in one document"));
                }
                self.check_attrs(&attrs)?;
                self.charge_kept(attrs_cost(&attrs))?;
                self.spreadsheet_seen = true;
                self.wb.attrs = attrs;
            }
            Tag::Table if self.stack == [Tag::Document, Tag::Body, Tag::Spreadsheet] => {
                if self.wb.sheets.len() >= MAX_SHEETS {
                    return Err(Error::TooLarge(format!("more than {MAX_SHEETS} sheets")));
                }
                self.check_attrs(&attrs)?;
                self.charge_kept(attrs_cost(&attrs))?;
                let keep = self.options.only_sheet.is_none_or(|i| i == self.wb.sheets.len());
                let name = self.attr(&attrs, "table:name")?.unwrap_or_default();
                self.sheet = Some(SheetDraft {
                    sheet: Sheet { name, attrs, ..Sheet::default() },
                    depth,
                    row: 0,
                    col: 0,
                    keep,
                    row_groups: vec![],
                    column_groups: vec![],
                    header_rows: None,
                    header_columns: None,
                });
            }
            Tag::RowGroup | Tag::HeaderRows if self.inside_sheet(Tag::holds_rows) => {
                let display = self.attr(&attrs, "table:display")?.as_deref() != Some("false");
                if let Some(s) = self.sheet.as_mut() {
                    let open = OpenLines { depth, first: s.row, display };
                    if t == Tag::RowGroup {
                        s.row_groups.push(open);
                    } else if s.header_rows.is_none() && s.sheet.header_rows.is_none() {
                        s.header_rows = Some(open);
                    }
                }
            }
            Tag::ColumnGroup | Tag::HeaderColumns if self.inside_sheet(Tag::holds_columns) => {
                let display = self.attr(&attrs, "table:display")?.as_deref() != Some("false");
                if let Some(s) = self.sheet.as_mut() {
                    let open = OpenLines { depth, first: s.col, display };
                    if t == Tag::ColumnGroup {
                        s.column_groups.push(open);
                    } else if s.header_columns.is_none() && s.sheet.header_columns.is_none() {
                        s.header_columns = Some(open);
                    }
                }
            }
            Tag::Column if self.inside_sheet(Tag::holds_columns) => self.column(&attrs)?,
            // Once the text is cut short, nothing after it is kept: its rows and cells are not read.
            Tag::Row if !self.wb.text_truncated && self.inside_sheet(Tag::holds_rows) => {
                self.row = Some(RowDraft {
                    depth,
                    repeat: self.count(&attrs, "table:number-rows-repeated", u32::MAX)?,
                    col: 0,
                    style: self.attr(&attrs, "table:style-name")?,
                    default_cell_style: self.attr(&attrs, "table:default-cell-style-name")?,
                    visibility: visibility(self.attr(&attrs, "table:visibility")?.as_deref()),
                    cells: vec![],
                });
            }
            Tag::Cell | Tag::Covered if !self.wb.text_truncated && self.row.as_ref().is_some_and(|r| depth == r.depth + 1) => {
                self.start_cell(t, depth, &attrs)?;
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
                    cell.pending_space = None;
                    cell.spans.clear();
                }
            }
            Tag::Span if self.captures_text() => {
                let style = self.attr(&attrs, "text:style-name")?;
                if let Some(cell) = self.cell.as_mut() {
                    cell.spans.push((depth, style));
                }
            }
            Tag::Link if self.captures_text() && self.cell.as_ref().is_some_and(|c| c.cell.link.is_none()) => {
                let link = Link {
                    href: self.attr(&attrs, "xlink:href")?.unwrap_or_default(),
                    title: self.attr(&attrs, "office:title")?,
                    target_frame: self.attr(&attrs, "office:target-frame-name")?,
                };
                if let Some(cell) = self.cell.as_mut() {
                    cell.cell.link = Some(link);
                }
            }
            Tag::Space | Tag::Tab | Tag::Break if self.captures_text() => {
                let max = self.options.max_cell_text;
                let count = match t {
                    Tag::Space => self.attr(&attrs, "text:c")?.map(|s| s.parse::<usize>()).transpose().map_err(xml_error)?.unwrap_or(1),
                    _ => 1,
                };
                let unit = match t {
                    Tag::Space => " ",
                    Tag::Tab => "\t",
                    _ => "\n",
                };
                if let Some(cell) = self.cell.as_mut() {
                    cell.paragraph_started = true;
                    if cell.cell.text.len().saturating_add(count) > max {
                        return Err(Error::TooLarge(format!("a cell's text is longer than {max} bytes")));
                    }
                    cell.push(&unit.repeat(count), max)?;
                }
            }
            _ => {}
        }
        self.stack.push(t);
        Ok(())
    }

    fn start_cell(&mut self, t: Tag, depth: usize, attrs: &[(String, String)]) -> Result<(), Error> {
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
        let has = |name: &str| attrs.iter().any(|(k, _)| k == name);
        let matrix = if has("table:number-matrix-columns-spanned") || has("table:number-matrix-rows-spanned") {
            Some((
                self.count(attrs, "table:number-matrix-columns-spanned", MAX_COLS)?,
                self.count(attrs, "table:number-matrix-rows-spanned", MAX_ROWS)?,
            ))
        } else {
            None
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
                validation: self.attr(attrs, "table:content-validation-name")?,
                matrix,
                ..Cell::default()
            },
            paragraph: None,
            paragraph_started: false,
            pending_space: None,
            spans: vec![],
        });
        Ok(())
    }

    fn column(&mut self, attrs: &[(String, String)]) -> Result<(), Error> {
        let repeat = self.count(attrs, "table:number-columns-repeated", u32::MAX)?;
        let style = self.attr(attrs, "table:style-name")?;
        let default_cell_style = self.attr(attrs, "table:default-cell-style-name")?;
        let visibility = visibility(self.attr(attrs, "table:visibility")?.as_deref());
        let Some(s) = self.sheet.as_mut() else { return Ok(()) };
        let first = s.col;
        // Columns past the grid's edge are clipped.
        let count = repeat.min(MAX_COLS.saturating_sub(first));
        s.col = first.saturating_add(count);
        if count == 0 || (style.is_none() && default_cell_style.is_none() && visibility == Visibility::Visible) {
            return Ok(());
        }
        self.charge_kept(line_cost(style.as_ref(), default_cell_style.as_ref()))?;
        if let Some(s) = self.sheet.as_mut() {
            s.sheet.columns.push(Line { first, count, style, default_cell_style, visibility });
        }
        Ok(())
    }

    /// Inside a cell's paragraph, through inline elements only.
    fn captures_text(&self) -> bool {
        self.cell.as_ref().and_then(|c| c.paragraph).is_some_and(|depth| self.stack.get(depth..).is_some_and(|p| p.iter().all(|t| t.is_inline())))
    }

    fn append(&mut self, text: String) -> Result<(), Error> {
        if let Some(capture) = self.capture.as_mut() {
            let cost = NODE_COST + text.len() as u64;
            if let Some(el) = capture.open.last_mut() {
                match el.children.last_mut() {
                    Some(Node::Text(t)) => t.push_str(&text),
                    _ => el.children.push(Node::Text(text)),
                }
            }
            return self.charge_kept(cost);
        }
        self.append_cell_text(&text)
    }

    /// Text, with ODF's white-space rule: runs collapse to one space, none at a paragraph's start.
    fn append_cell_text(&mut self, text: &str) -> Result<(), Error> {
        if !self.captures_text() {
            return Ok(());
        }
        let max = self.options.max_cell_text;
        let Some(cell) = self.cell.as_mut() else { return Ok(()) };
        for ch in text.chars() {
            if matches!(ch, ' ' | '\t' | '\n' | '\r') {
                if cell.paragraph_started && cell.pending_space.is_none() {
                    cell.pending_space = Some(cell.style());
                }
            } else {
                cell.push(ch.encode_utf8(&mut [0; 4]), max)?;
                cell.paragraph_started = true;
            }
        }
        Ok(())
    }

    /// A kept tree is complete: it goes where it was kept for.
    fn deliver(&mut self, target: Target, el: El) {
        match target {
            Target::FontFaces => self.wb.font_faces = Some(el),
            Target::AutomaticStyles => self.wb.automatic_styles = Some(el),
            Target::Body => self.wb.body.push(el),
            Target::Table => {
                if let Some(s) = self.sheet.as_mut() {
                    s.sheet.elements.push(el);
                }
            }
            Target::Annotation => {
                if let Some(c) = self.cell.as_mut() {
                    c.cell.annotation = Some(el);
                }
            }
            Target::Drawing => {
                if let Some(c) = self.cell.as_mut() {
                    c.cell.drawings.push(el);
                }
            }
        }
    }

    fn end(&mut self) -> Result<(), Error> {
        self.stack.pop().ok_or_else(|| xml_error("unexpected closing element"))?;
        let depth = self.stack.len();
        if let Some(capture) = self.capture.as_mut() {
            let el = capture.open.pop().ok_or_else(|| xml_error("unexpected closing element"))?;
            match capture.open.last_mut() {
                Some(parent) => parent.children.push(Node::El(el)),
                None => {
                    let target = capture.target;
                    self.capture = None;
                    self.deliver(target, el);
                }
            }
            return Ok(());
        }
        if let Some(cell) = self.cell.as_mut() {
            if cell.paragraph == Some(depth) {
                cell.paragraph = None;
            }
            while cell.spans.last().is_some_and(|(d, _)| *d >= depth) {
                cell.spans.pop();
            }
        }
        if self.cell.as_ref().is_some_and(|c| c.depth == depth) {
            self.finish_cell()?;
        }
        if self.row.as_ref().is_some_and(|r| r.depth == depth) {
            self.finish_row()?;
        }
        self.close_lines(depth)?;
        if self.sheet.as_ref().is_some_and(|s| s.depth == depth)
            && let Some(sheet) = self.sheet.take()
        {
            self.wb.sheets.push(sheet.sheet);
        }
        Ok(())
    }

    /// Closes the row or column group, or the header rows or columns, that ends at `depth`.
    fn close_lines(&mut self, depth: usize) -> Result<(), Error> {
        let Some(s) = self.sheet.as_mut() else { return Ok(()) };
        let mut closed = None;
        if s.row_groups.last().is_some_and(|g| g.depth == depth)
            && let Some(g) = s.row_groups.pop()
        {
            closed = Some((Group { first: g.first, count: s.row - g.first, level: s.row_groups.len() as u32 + 1, display: g.display }, true));
        }
        if s.column_groups.last().is_some_and(|g| g.depth == depth)
            && let Some(g) = s.column_groups.pop()
        {
            closed = Some((Group { first: g.first, count: s.col - g.first, level: s.column_groups.len() as u32 + 1, display: g.display }, false));
        }
        if s.header_rows.as_ref().is_some_and(|h| h.depth == depth)
            && let Some(h) = s.header_rows.take()
        {
            s.sheet.header_rows = Some((h.first, s.row - h.first)).filter(|(_, n)| *n > 0);
        }
        if s.header_columns.as_ref().is_some_and(|h| h.depth == depth)
            && let Some(h) = s.header_columns.take()
        {
            s.sheet.header_columns = Some((h.first, s.col - h.first)).filter(|(_, n)| *n > 0);
        }
        if let Some((group, rows)) = closed {
            self.charge_kept(NODE_COST)?;
            if let Some(s) = self.sheet.as_mut() {
                if rows {
                    s.sheet.row_groups.push(group);
                } else {
                    s.sheet.column_groups.push(group);
                }
            }
        }
        Ok(())
    }

    /// With [`Options::truncate_text`], how many of `copies` copies of `each` bytes of text fit
    /// in what is left of `max_text`; fewer than `copies` marks the workbook truncated, and none
    /// fit once it is.
    fn fit(&mut self, copies: u32, each: u64) -> u32 {
        if self.wb.text_truncated {
            return 0;
        }
        if each == 0 {
            return copies;
        }
        let room = self.options.max_text.saturating_sub(self.text) / each;
        if room < u64::from(copies) {
            self.wb.text_truncated = true;
            return u32::try_from(room).unwrap_or(0);
        }
        copies
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
        let styled = draft.cell.style.is_some() || draft.cell.validation.is_some();
        let end = match row.col.checked_add(draft.repeat).filter(|n| *n <= MAX_COLS) {
            Some(end) => end,
            // Blank padding past the grid's edge is clipped.
            None if !kept => MAX_COLS,
            None => return Err(Error::TooLarge("a row is wider than the grid".into())),
        };
        let start = row.col;
        row.col = end;
        if kept {
            let len = draft.cell.text.len() + draft.cell.raw.as_ref().map_or(0, String::len);
            let repeat = if self.options.truncate_text { self.fit(draft.repeat, len as u64) } else { draft.repeat };
            if repeat == 0 {
                return Ok(());
            }
            self.charge_text(u64::from(repeat).saturating_mul(len as u64))?;
            if let Some(row) = self.row.as_mut() {
                row.cells.push(RepeatedCell { col: start, repeat, width, height, cell: draft.cell });
            }
        } else if styled && end > start {
            row.cells.push(RepeatedCell { col: start, repeat: end - start, width: 1, height: 1, cell: draft.cell });
        }
        Ok(())
    }

    fn finish_row(&mut self) -> Result<(), Error> {
        let Some(row) = self.row.take() else { return Ok(()) };
        let Some(sheet) = self.sheet.as_mut() else { return Ok(()) };
        // Rows of blank, unmerged cells past the grid's edge are padding, and are clipped.
        let padding = row.cells.iter().all(|e| e.cell.is_blank() && e.width == 1 && e.height == 1);
        let end = match sheet.row.checked_add(row.repeat).filter(|n| *n <= MAX_ROWS) {
            Some(end) => end,
            None if padding => MAX_ROWS,
            None => return Err(Error::TooLarge("the sheet is taller than the grid".into())),
        };
        let first_row = sheet.row;
        sheet.row = end;
        if end > first_row && (row.style.is_some() || row.default_cell_style.is_some() || row.visibility != Visibility::Visible) {
            self.charge_kept(line_cost(row.style.as_ref(), row.default_cell_style.as_ref()))?;
            if let Some(sheet) = self.sheet.as_mut() {
                let (style, default_cell_style) = (row.style.clone(), row.default_cell_style.clone());
                sheet.sheet.rows.push(Line { first: first_row, count: end - first_row, style, default_cell_style, visibility: row.visibility });
            }
        }
        // The first copy of the row was charged cell by cell. With truncation, keep the further
        // copies whose text fits, then the cells of the next copy that fit.
        let row_repeat = row.repeat;
        let mut partial = vec![];
        let repeat = if self.options.truncate_text && row.repeat > 1 && !self.wb.text_truncated {
            let each = row.cells.iter().map(|e| u64::from(e.repeat).saturating_mul(charged(&e.cell))).fold(0u64, u64::saturating_add);
            let full = 1 + self.fit(row.repeat - 1, each);
            if full < row.repeat {
                let room = self.options.max_text.saturating_sub(self.text).saturating_sub(each.saturating_mul(u64::from(full - 1)));
                partial = cells_that_fit(&row.cells, room);
            }
            full
        } else if self.options.truncate_text && self.wb.text_truncated {
            1
        } else {
            row.repeat
        };
        let kept_end = first_row.saturating_add(repeat).min(end);
        // The first copy's text was charged in `finish_cell`; the partial copy's was not.
        for (cells, rows, first_charged) in [(row.cells, first_row..kept_end, true), (partial, kept_end..kept_end + 1, false)] {
            for entry in cells {
                self.place(&entry, rows.clone(), first_charged, end, row_repeat)?;
            }
        }
        Ok(())
    }

    /// Counts, checks and stores one cell entry over `rows`.
    fn place(&mut self, entry: &RepeatedCell, rows: std::ops::Range<u32>, first_charged: bool, end: u32, row_repeat: u32) -> Result<(), Error> {
        if rows.is_empty() {
            return Ok(());
        }
        let count = u64::from(entry.repeat) * u64::from(rows.end - rows.start);
        let blank = entry.cell.is_blank();
        if !blank {
            self.cells = self.cells.saturating_add(count);
            // The first row's copies were charged with the cell.
            let copies = if first_charged { count.saturating_sub(u64::from(entry.repeat)) } else { count };
            self.text = self.text.saturating_add(copies.saturating_mul(charged(&entry.cell)));
            // The trees of the first row's first copy were charged as they were read; its runs
            // and link, and every other copy, are charged here.
            let further = if first_charged { count.saturating_sub(1) } else { count };
            self.charge_kept(
                further.saturating_mul(cell_trees_cost(&entry.cell)).saturating_add(count.saturating_mul(cell_extras_cost(&entry.cell))),
            )?;
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
        if merged && ((entry.repeat > 1 && entry.width > 1) || (row_repeat > 1 && entry.height > 1)) {
            return Err(Error::Invalid("repeated cells have overlapping merges".into()));
        }
        if blank && (entry.cell.style.is_some() || entry.cell.validation.is_some()) {
            self.charge_kept(line_cost(entry.cell.style.as_ref(), entry.cell.validation.as_ref()))?;
            let range = Range::new(CellRef::new(rows.start, entry.col), CellRef::new(rows.end - 1, entry.col + entry.repeat - 1));
            if let Some(sheet) = self.sheet.as_mut() {
                sheet.sheet.blanks.push(Blank { range, style: entry.cell.style.clone(), validation: entry.cell.validation.clone() });
            }
        }
        if blank && !merged {
            return Ok(());
        }
        let Some(sheet) = self.sheet.as_mut() else { return Ok(()) };
        for r in rows {
            for c in entry.col..entry.col + entry.repeat {
                let at = CellRef::new(r, c);
                if !blank {
                    sheet.sheet.cells.insert(at, entry.cell.clone());
                }
                if merged {
                    sheet.sheet.merges.push(Range::new(at, CellRef::new(r + entry.height - 1, c + entry.width - 1)));
                }
            }
        }
        Ok(())
    }
}

/// Bytes of text a kept cell is charged: its displayed text and its value attribute.
fn charged(cell: &Cell) -> u64 {
    if cell.is_blank() { 0 } else { (cell.text.len() + cell.raw.as_ref().map_or(0, String::len)) as u64 }
}

/// The leading entries of a row copy whose text fits in `room` bytes, the last one with only
/// the repeats that fit.
fn cells_that_fit(cells: &[RepeatedCell], mut room: u64) -> Vec<RepeatedCell> {
    let mut out = vec![];
    for e in cells {
        let len = charged(&e.cell);
        let n = room.checked_div(len).map_or(e.repeat, |fit| u32::try_from(fit).unwrap_or(u32::MAX).min(e.repeat));
        if n == 0 {
            break;
        }
        room -= u64::from(n) * len;
        out.push(RepeatedCell { col: e.col, repeat: n, width: e.width, height: e.height, cell: e.cell.clone() });
        if n < e.repeat {
            break;
        }
    }
    out
}

fn push_text(out: &mut String, text: &str, max: usize) -> Result<(), Error> {
    if out.len().saturating_add(text.len()) > max {
        return Err(Error::TooLarge(format!("a cell's text is longer than {max} bytes")));
    }
    out.push_str(text);
    Ok(())
}
