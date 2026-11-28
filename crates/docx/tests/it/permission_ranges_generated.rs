//! Permission ranges in generated documents: as read and through two saves, no range covers
//! anything its pair of markers doesn't cover in the file, and a valid range without columns
//! covers the same text as in the file.
//!
//! Each case is a small document: a body, a header and some glossary entries, with paragraphs and
//! tables (nested ones too, cells spanning columns or merged into the cell before them
//! (`w:hMerge`), rows starting after `w:gridBefore` columns and ending before `w:gridAfter` ones,
//! rows in content controls, rows and tables with no cells, now and then past the 63 columns the
//! model keeps), and permission markers at random places in them: in paragraphs, between blocks,
//! between rows and between cells. Some of their `w:id`s come from a small pool, so a case has
//! starts and ends that close nothing and ids used twice; the others make nested pairs of ids of
//! their own. A start may have column attributes, in order or not, in the table or past it, or
//! ones that aren't column numbers. Every word and every
//! editor is unique, and every cell holds a paragraph, so a word stands for its cell.
//!
//! The oracle reads the generated markup on its own, with no code of the reader's. In each story,
//! an end closes the latest open start of its `w:id` (as `Document::perm_ranges` pairs a model's).
//! A pair covers the words between its markers. One with column attributes (ECMA-376-1:2016
//! §17.13.7.2) also covers, in the table its start is in, the rows from its start's to its end's (a
//! marker between rows starts the row after it, or ends the row before it), and in those the cells
//! wholly inside its columns, as the file grids them: `w:gridBefore`, then each cell's `w:gridSpan`,
//! a merged cell joining the one before it. One attribute alone names one column; with its end
//! outside that table, it covers no cell. A model range covers the words between its markers, and
//! with columns, the cells the model's reading gives (`Document::perm_ranges`): in the table its
//! start is in, from the row of its start to the row of its end, the cells wholly inside them.
//!
//! The fixed seed and the shared case count (`PROPTEST_CASES`, 64 in CI's gate) keep a run short;
//! `PROPTEST_RNG_SEED` picks other cases.
//!
//! Synthetic documents built for these tests from the schema.

use std::collections::{BTreeMap, BTreeSet};

use craft_wordmodel::para::OBJ;
use craft_wordmodel::walk::each_para;
use craft_wordmodel::{Document, PartKind, PermRange, Pos, StoryRef};
use proptest::prelude::*;
use proptest::test_runner::RngSeed;

use crate::fixtures::zip;

// ---- the documents ----

/// A permission marker, by its `w:id`: a start with its column attributes, or an end. Ids below
/// [`SHARED`] are the file's own, so a case has starts and ends that close nothing and ids used
/// twice; the others stand for fresh pairs (see [`Xml::file_id`]).
#[derive(Clone, Debug)]
enum Mark {
    Start(u8, Cols),
    End(u8),
}

/// A start's column attributes.
#[derive(Clone, Debug)]
enum Cols {
    None,
    Pair(u32, u32),
    First(u32),
    Last(u32),
    /// `w:colFirst` that isn't a column number, with a good `w:colLast`.
    Bad(&'static str),
}

#[derive(Clone, Debug)]
enum Blk {
    /// A paragraph: its words (`None`) and markers, in order (with no word, it gets one last).
    Para(Vec<Option<Mark>>),
    Table(Tbl),
    /// A marker between blocks.
    Mark(Mark),
}

/// A table: its `w:gridCol` count and its rows, with markers between them.
#[derive(Clone, Debug)]
struct Tbl {
    grid: u32,
    rows: Vec<RowItem>,
}

#[derive(Clone, Debug)]
enum RowItem {
    Row(Row),
    Mark(Mark),
    /// Rows and markers in a row-level content control.
    Control(Vec<RowItem>),
}

/// A row: `w:gridBefore`, `w:gridAfter` and its cells, with markers between them.
#[derive(Clone, Debug)]
struct Row {
    before: u32,
    after: u32,
    cells: Vec<CellItem>,
}

#[derive(Clone, Debug)]
enum CellItem {
    Cell(CellSpec),
    Mark(Mark),
}

/// A cell: its `w:gridSpan`, whether it is merged into the cell before it (`w:hMerge`), and its
/// blocks (with no paragraph among them, it gets one last).
#[derive(Clone, Debug)]
struct CellSpec {
    span: u32,
    merged: bool,
    blocks: Vec<Blk>,
}

/// A generated document: its body, a header, and glossary entries.
#[derive(Clone, Debug)]
struct Gen {
    body: Vec<Blk>,
    header: Vec<Blk>,
    glossary: Vec<Vec<Blk>>,
}

/// A column number: mostly one of a small table's, sometimes about the 63 columns the model keeps.
fn col() -> impl Strategy<Value = u32> {
    prop_oneof![6 => 0u32..5, 1 => 61u32..66]
}

fn cols() -> impl Strategy<Value = Cols> {
    prop_oneof![
        5 => Just(Cols::None),
        4 => (col(), col()).prop_map(|(a, b)| Cols::Pair(a, b)),
        1 => col().prop_map(Cols::First),
        1 => col().prop_map(Cols::Last),
        1 => prop_oneof![Just("x"), Just("-1"), Just("4294967296")].prop_map(Cols::Bad),
    ]
}

/// Ids below this are written as they are; the others give fresh pairs.
const SHARED: u8 = 4;

fn mark() -> impl Strategy<Value = Mark> {
    prop_oneof![(0u8..2 * SHARED, cols()).prop_map(|(id, c)| Mark::Start(id, c)), (0u8..2 * SHARED).prop_map(Mark::End)]
}

/// A grid count (`w:gridSpan`, `w:gridBefore`, `w:gridAfter`): mostly `usual`, sometimes past the
/// 63 columns the model keeps.
fn count(usual: impl Strategy<Value = u32>) -> impl Strategy<Value = u32> {
    prop_oneof![12 => usual, 1 => 62u32..66]
}

fn para() -> impl Strategy<Value = Blk> {
    prop::collection::vec(prop_oneof![3 => Just(None), 2 => mark().prop_map(Some)], 1..5).prop_map(Blk::Para)
}

/// Blocks with tables nested up to `depth` deep.
fn blocks(depth: u32) -> BoxedStrategy<Vec<Blk>> {
    let leaf = prop_oneof![4 => para(), 1 => mark().prop_map(Blk::Mark)];
    if depth == 0 {
        return prop::collection::vec(leaf, 1..4).boxed();
    }
    prop::collection::vec(prop_oneof![5 => leaf, 2 => table(depth - 1).prop_map(Blk::Table)], 1..5).boxed()
}

fn cell(depth: u32) -> impl Strategy<Value = CellItem> {
    let spec = (count(1u32..3), prop::bool::weighted(0.1), blocks(depth)).prop_map(|(span, merged, blocks)| CellSpec { span, merged, blocks });
    prop_oneof![5 => spec.prop_map(CellItem::Cell), 2 => mark().prop_map(CellItem::Mark)]
}

fn row(depth: u32) -> impl Strategy<Value = Row> {
    let cells = prop_oneof![
        20 => prop::collection::vec(cell(depth), 0..4),
        // A row of more cells than the model keeps, some with markers in them.
        1 => prop::collection::vec(
            prop_oneof![
                8 => Just(CellItem::Cell(CellSpec { span: 1, merged: false, blocks: vec![Blk::Para(vec![None])] })),
                1 => para().prop_map(|p| CellItem::Cell(CellSpec { span: 1, merged: false, blocks: vec![p] })),
                1 => mark().prop_map(CellItem::Mark),
            ],
            64..68
        ),
    ];
    let side = || count(prop_oneof![5 => Just(0u32), 1 => 1u32..3]);
    (side(), side(), cells).prop_map(|(before, after, cells)| Row { before, after, cells })
}

fn table(depth: u32) -> impl Strategy<Value = Tbl> {
    let item = prop_oneof![5 => row(depth).prop_map(RowItem::Row), 2 => mark().prop_map(RowItem::Mark)];
    let items = prop_oneof![
        6 => item.clone(),
        1 => prop::collection::vec(item, 1..3).prop_map(RowItem::Control),
    ];
    (prop_oneof![5 => 1u32..5, 1 => 62u32..66], prop::collection::vec(items, 1..5)).prop_map(|(grid, rows)| Tbl { grid, rows })
}

/// A story starts with a paragraph of its own, so that it is never read as empty.
fn story(depth: u32) -> impl Strategy<Value = Vec<Blk>> {
    blocks(depth).prop_map(|b| std::iter::once(Blk::Para(vec![None])).chain(b).collect())
}

fn document() -> impl Strategy<Value = Gen> {
    (story(2), story(1), prop::collection::vec(story(1), 0..3)).prop_map(|(body, header, glossary)| Gen { body, header, glossary })
}

/// Writes the markup, giving each word and each start's editor a name of its own.
#[derive(Default)]
struct Xml {
    words: usize,
    editors: usize,
    /// The fresh pairs' ids so far, and those of the story's still open, latest last.
    fresh: u32,
    open: Vec<u32>,
}

impl Xml {
    fn word(&mut self) -> String {
        self.words += 1;
        format!(r#"<w:r><w:t xml:space="preserve">w{} </w:t></w:r>"#, self.words)
    }

    fn mark(&mut self, m: &Mark) -> String {
        match m {
            Mark::Start(id, cols) => {
                self.editors += 1;
                let cols = match cols {
                    Cols::None => String::new(),
                    Cols::Pair(a, b) => format!(r#" w:colFirst="{a}" w:colLast="{b}""#),
                    Cols::First(a) => format!(r#" w:colFirst="{a}""#),
                    Cols::Last(b) => format!(r#" w:colLast="{b}""#),
                    Cols::Bad(v) => format!(r#" w:colFirst="{v}" w:colLast="1""#),
                };
                format!(r#"<w:permStart w:id="{}" w:ed="e{}"{cols}/>"#, self.file_id(*id, true), self.editors)
            }
            Mark::End(id) => format!(r#"<w:permEnd w:id="{}"/>"#, self.file_id(*id, false)),
        }
    }

    /// A marker's `w:id`: below [`SHARED`], the id itself; otherwise, for a start, an id no other
    /// marker has, and for an end, the id of the story's latest fresh start still open (or, with
    /// none, one no other marker has).
    fn file_id(&mut self, id: u8, start: bool) -> u32 {
        if id < SHARED {
            return u32::from(id);
        }
        if !start && let Some(open) = self.open.pop() {
            return open;
        }
        self.fresh += 1;
        let id = 100 + self.fresh;
        if start {
            self.open.push(id);
        }
        id
    }

    /// A story's blocks: fresh pairs don't run from one story into the next.
    fn story(&mut self, blocks: &[Blk]) -> String {
        self.open.clear();
        self.blocks(blocks)
    }

    fn blocks(&mut self, blocks: &[Blk]) -> String {
        let mut s = String::new();
        for b in blocks {
            match b {
                Blk::Para(items) => {
                    s += "<w:p>";
                    for item in items {
                        s += &match item {
                            None => self.word(),
                            Some(m) => self.mark(m),
                        };
                    }
                    if items.iter().all(Option::is_some) {
                        s += &self.word();
                    }
                    s += "</w:p>";
                }
                Blk::Table(t) => s += &self.table(t),
                Blk::Mark(m) => s += &self.mark(m),
            }
        }
        s
    }

    fn table(&mut self, t: &Tbl) -> String {
        let mut s = format!("<w:tbl><w:tblPr/><w:tblGrid>{}</w:tblGrid>", r#"<w:gridCol w:w="400"/>"#.repeat(t.grid as usize));
        s += &self.rows(&t.rows);
        s + "</w:tbl>"
    }

    fn rows(&mut self, items: &[RowItem]) -> String {
        let mut s = String::new();
        for item in items {
            match item {
                RowItem::Mark(m) => s += &self.mark(m),
                RowItem::Control(items) => s += &format!("<w:sdt><w:sdtPr/><w:sdtContent>{}</w:sdtContent></w:sdt>", self.rows(items)),
                RowItem::Row(r) => {
                    s += &format!(r#"<w:tr><w:trPr><w:gridBefore w:val="{}"/><w:gridAfter w:val="{}"/></w:trPr>"#, r.before, r.after);
                    for c in &r.cells {
                        match c {
                            CellItem::Mark(m) => s += &self.mark(m),
                            CellItem::Cell(c) => {
                                let merge = if c.merged { r#"<w:hMerge/>"# } else { "" };
                                s += &format!(r#"<w:tc><w:tcPr><w:gridSpan w:val="{}"/>{merge}</w:tcPr>{}"#, c.span, self.blocks(&c.blocks));
                                if !c.blocks.iter().any(|b| matches!(b, Blk::Para(_))) {
                                    s += &format!("<w:p>{}</w:p>", self.word());
                                }
                                s += "</w:tc>";
                            }
                        }
                    }
                    s += "</w:tr>";
                }
            }
        }
        s
    }
}

const NS: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#;
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

/// The markup of each story: the body's, the header's and each glossary entry's blocks.
struct Markup {
    body: String,
    header: String,
    glossary: Vec<String>,
}

impl Markup {
    fn of(g: &Gen) -> Markup {
        let mut x = Xml::default();
        Markup { body: x.story(&g.body), header: x.story(&g.header), glossary: g.glossary.iter().map(|b| x.story(b)).collect() }
    }

    fn stories(&self) -> Vec<&str> {
        [self.body.as_str(), self.header.as_str()].into_iter().chain(self.glossary.iter().map(String::as_str)).collect()
    }

    fn package(&self) -> Vec<u8> {
        let doc = format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document {NS}><w:body>{}<w:sectPr><w:headerReference w:type="default" r:id="rIdH"/></w:sectPr></w:body></w:document>"#,
            self.body
        );
        let header = format!(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:hdr {NS}>{}</w:hdr>"#, self.header);
        let entries: String = self
            .glossary
            .iter()
            .enumerate()
            .map(|(i, b)| {
                format!(
                    r#"<w:docPart><w:docPartPr><w:name w:val="g{i}"/><w:category><w:name w:val="General"/><w:gallery w:val="docParts"/></w:category></w:docPartPr><w:docPartBody>{b}</w:docPartBody></w:docPart>"#
                )
            })
            .collect();
        let glossary = format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:glossaryDocument {NS}><w:docParts>{entries}</w:docParts></w:glossaryDocument>"#
        );
        let types = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/header1.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml"/><Override PartName="/word/glossary/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.glossary+xml"/></Types>"#;
        let root = format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{REL}/officeDocument" Target="word/document.xml"/></Relationships>"#
        );
        let rels = format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rIdH" Type="{REL}/header" Target="header1.xml"/><Relationship Id="rIdG" Type="{REL}/glossaryDocument" Target="glossary/document.xml"/></Relationships>"#
        );
        zip(&[
            ("[Content_Types].xml", types.as_bytes()),
            ("_rels/.rels", root.as_bytes()),
            ("word/document.xml", doc.as_bytes()),
            ("word/_rels/document.xml.rels", rels.as_bytes()),
            ("word/header1.xml", header.as_bytes()),
            ("word/glossary/document.xml", glossary.as_bytes()),
        ])
    }
}

// ---- the oracle: what the file's pairs cover ----

/// The most cells of a row a model keeps (`craft_wordmodel::table::MAX_COLS`): later ones, and the
/// markers in them, are dropped, as is a cell merged into the one before it.
const KEPT_CELLS: usize = 63;

/// Where a marker is in a table: in row `k`, or between rows, before row `k`.
#[derive(Clone, Copy, Debug)]
enum Place {
    In(usize),
    Before(usize),
}

/// A permission marker as the file has it.
#[derive(Debug)]
struct SrcMark {
    start: bool,
    id: String,
    editor: String,
    /// The column attributes: none, the first and last column, or `Err` when one isn't a column
    /// number.
    cols: Option<Result<(u64, u64), ()>>,
    /// The number of words before it in its story.
    at: usize,
    /// The tables it is in, outermost first, and where in each.
    tables: Vec<(usize, Place)>,
    /// In content a model doesn't keep.
    lost: bool,
}

/// A cell as the file grids it, with the cells merged into it: its first grid column, how many it
/// spans, and every word in it.
#[derive(Debug)]
struct SrcCell {
    start: u64,
    span: u64,
    words: BTreeSet<String>,
}

#[derive(Default, Debug)]
struct SrcRow {
    cells: Vec<SrcCell>,
    /// The grid column the next cell starts at, the number of cells so far, and the last cell kept.
    next: u64,
    count: usize,
    kept: Option<usize>,
}

/// A cell being read: its table, row and cell (`None`: one a model doesn't keep, its words lost
/// with it), and, until its properties end, its span and whether it is merged.
struct OpenCell {
    at: Option<(usize, usize, usize)>,
    pending: Option<(u64, bool)>,
}

/// A story as its markup gives it.
#[derive(Default, Debug)]
struct Src {
    words: Vec<String>,
    marks: Vec<SrcMark>,
    tables: Vec<Vec<SrcRow>>,
    /// Each start's end, by their indexes in `marks`.
    pairs: BTreeMap<usize, usize>,
}

/// The value of attribute `name` in a tag's attributes.
fn attr<'a>(attrs: &'a str, name: &str) -> Option<&'a str> {
    let key = format!("{name}=\"");
    let i = attrs.find(&key)?;
    attrs[i + key.len()..].split('"').next()
}

/// A column attribute: `Err` when it isn't a column number (`ST_DecimalNumber`, zero-based).
fn column(v: &str) -> Result<u64, ()> {
    v.parse::<i64>().ok().and_then(|n| u32::try_from(n).ok()).map(u64::from).ok_or(())
}

impl Src {
    /// Reads a story's markup: its words, markers and tables, and pairs the markers.
    fn read(xml: &str) -> Src {
        let mut s = Src::default();
        // Open tables: (index, inside a row).
        let mut tables: Vec<(usize, bool)> = Vec::new();
        let mut cells: Vec<OpenCell> = Vec::new();
        let mut in_text = false;
        for piece in xml.split('<').skip(1) {
            let (tag, text) = piece.split_once('>').unwrap_or((piece, ""));
            let close = tag.starts_with('/');
            let tag = tag.trim_start_matches('/').trim_end_matches('/');
            let (name, attrs) = tag.split_once(' ').unwrap_or((tag, ""));
            let val = || attr(attrs, "w:val").and_then(|v| v.parse::<u64>().ok());
            match (name, close) {
                ("w:tbl", false) => {
                    s.tables.push(Vec::new());
                    tables.push((s.tables.len() - 1, false));
                }
                ("w:tbl", true) => {
                    tables.pop();
                }
                ("w:tr", _) => {
                    if let Some((t, in_row)) = tables.last_mut() {
                        *in_row = !close;
                        if !close {
                            s.tables[*t].push(SrcRow::default());
                        }
                    }
                }
                ("w:gridBefore", false) => {
                    if let Some(row) = tables.last().and_then(|(t, _)| s.tables[*t].last_mut()) {
                        row.next = val().unwrap_or(0);
                    }
                }
                ("w:tc", false) => cells.push(OpenCell { at: None, pending: Some((1, false)) }),
                ("w:gridSpan", false) => {
                    if let Some(Some(p)) = cells.last_mut().map(|c| &mut c.pending) {
                        p.0 = val().unwrap_or(1).max(1);
                    }
                }
                ("w:hMerge", false) => {
                    if let Some(Some(p)) = cells.last_mut().map(|c| &mut c.pending) {
                        p.1 = true;
                    }
                }
                ("w:tcPr", true) => s.place_cell(&tables, &mut cells),
                ("w:tc", true) => {
                    cells.pop();
                }
                ("w:t", _) => in_text = !close,
                ("w:permStart" | "w:permEnd", false) => {
                    let start = name == "w:permStart";
                    let col = |a| attr(attrs, a).map(column);
                    let cols = match (col("w:colFirst"), col("w:colLast")) {
                        (None, None) => None,
                        (Some(Ok(a)), Some(Ok(b))) => Some(Ok((a, b))),
                        (Some(Ok(a)), None) | (None, Some(Ok(a))) => Some(Ok((a, a))),
                        _ => Some(Err(())),
                    };
                    let place = |(t, in_row): &(usize, bool)| {
                        let rows = s.tables[*t].len();
                        (*t, if *in_row { Place::In(rows - 1) } else { Place::Before(rows) })
                    };
                    s.marks.push(SrcMark {
                        start,
                        id: attr(attrs, "w:id").unwrap_or("").to_string(),
                        editor: attr(attrs, "w:ed").unwrap_or("").to_string(),
                        cols: if start { cols } else { None },
                        at: s.words.len(),
                        tables: tables.iter().map(place).collect(),
                        lost: cells.iter().any(|c| c.at.is_none()),
                    });
                }
                _ => {}
            }
            if in_text && !close {
                for w in text.split_whitespace() {
                    s.words.push(w.to_string());
                    for &(t, r, c) in cells.iter().filter_map(|c| c.at.as_ref()) {
                        s.tables[t][r].cells[c].words.insert(w.to_string());
                    }
                }
            }
        }
        // An end closes the latest open start of its id.
        let mut open: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
        for (i, m) in s.marks.iter().enumerate() {
            if m.start {
                open.entry(&m.id).or_default().push(i);
            } else if let Some(st) = open.get_mut(m.id.as_str()).and_then(Vec::pop) {
                s.pairs.insert(st, i);
            }
        }
        s
    }

    /// The cell being read has its properties: it takes its place in its row's grid, unless it is
    /// past the cells a model keeps, or merged into the cell before it (with its words, which a
    /// model drops).
    fn place_cell(&mut self, tables: &[(usize, bool)], cells: &mut [OpenCell]) {
        let (Some(&(t, _)), Some(open)) = (tables.last(), cells.last_mut()) else { return };
        let Some((span, merged)) = open.pending.take() else { return };
        let r = self.tables[t].len() - 1;
        let row = &mut self.tables[t][r];
        let (count, start) = (row.count, row.next);
        row.count += 1;
        row.next += span;
        match row.kept {
            _ if count >= KEPT_CELLS => {}
            Some(k) if merged => row.cells[k].span += span,
            _ => {
                row.cells.push(SrcCell { start, span, words: BTreeSet::new() });
                row.kept = Some(row.cells.len() - 1);
                open.at = Some((t, r, row.cells.len() - 1));
            }
        }
    }

    /// The words between a pair's markers.
    fn text(&self, start: usize, end: usize) -> BTreeSet<String> {
        self.words[self.marks[start].at..self.marks[end].at].iter().cloned().collect()
    }

    /// The words of the cells a pair with column attributes covers.
    fn cells(&self, start: usize, end: usize) -> BTreeSet<String> {
        let (s, e) = (&self.marks[start], &self.marks[end]);
        let mut out = BTreeSet::new();
        let (Some(Ok((first, last))), Some(&(t, from))) = (s.cols, s.tables.last()) else { return out };
        let Some(&(_, to)) = e.tables.iter().find(|(x, _)| *x == t) else { return out };
        let from = match from {
            Place::In(k) | Place::Before(k) => k,
        };
        let Some(to) = (match to {
            Place::In(k) => Some(k),
            Place::Before(k) => k.checked_sub(1),
        }) else {
            return out;
        };
        for row in self.tables[t].iter().take(to + 1).skip(from) {
            for c in row.cells.iter().filter(|c| first <= c.start && c.start + c.span <= last + 1) {
                out.extend(c.words.iter().cloned());
            }
        }
        out
    }

    /// A pair without column attributes whose `w:id` no other marker in the story has, and whose
    /// markers are in content a model keeps: one the reader must keep over the same text.
    fn valid(&self, start: usize, end: usize) -> bool {
        let (s, e) = (&self.marks[start], &self.marks[end]);
        s.cols.is_none() && !s.lost && !e.lost && self.marks.iter().filter(|m| m.id == s.id).count() == 2
    }
}

// ---- the model ----

fn words_of(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|c: char| c == OBJ || c.is_whitespace()).filter(|w| !w.is_empty()).map(str::to_string)
}

/// The words of story `story` from `from` to `to`.
fn words_between(d: &Document, story: StoryRef, from: &Pos, to: &Pos) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for path in d.para_paths(story) {
        if path < from.path || path > to.path {
            continue;
        }
        let Some(p) = d.para(story, &path) else { continue };
        let a = if path == from.path { from.off } else { 0 };
        let b = if path == to.path { to.off } else { p.text.len() };
        if let Some(t) = p.text.get(a..b) {
            out.extend(words_of(t));
        }
    }
    out
}

/// Every word of a story.
fn story_words(d: &Document, story: StoryRef) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for path in d.para_paths(story) {
        if let Some(p) = d.para(story, &path) {
            out.extend(words_of(&p.text));
        }
    }
    out
}

/// The words of the cells a model range limited to columns covers on the model's reading: in the
/// table its start is in, from its start's row to its end's (none when its end isn't in that
/// table), the cells wholly inside its columns.
fn model_cells(d: &Document, r: &PermRange) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let (Some([first, last]), Some((tp, from, _))) = (r.cols, r.start.path.cell()) else { return out };
    let n = tp.0.len();
    if r.end.story != r.start.story || r.end.path.0.len() < n + 3 || !r.end.path.0.starts_with(&tp.0) {
        return out;
    }
    let to = r.end.path.0[n] as usize;
    let Some(t) = d.table(r.start.story, &tp) else { return out };
    let (first, last) = (u64::from(first), u64::from(last));
    for row in from..=to.min(t.rows.len().saturating_sub(1)) {
        for (c, cell) in t.rows[row].cells.iter().enumerate() {
            let g = t.grid_col(row, c) as u64;
            if first <= g && g + cell.span() as u64 <= last + 1 {
                for b in &cell.blocks {
                    each_para(b, 0, &mut |p| out.extend(words_of(&p.text)));
                }
            }
        }
    }
    out
}

/// The model's stories, in the order of [`Markup::stories`]: the body, the header, the glossary
/// entries (`None` for one the model doesn't have); each as a document and the story in it.
fn stories(d: &Document, entries: usize) -> Vec<Option<(Document, StoryRef)>> {
    let header = d.parts.iter().find(|(_, p)| p.kind == PartKind::Header).map(|(id, _)| (d.clone(), StoryRef::Part(*id)));
    let mut out = vec![Some((d.clone(), StoryRef::Body)), header];
    for i in 0..entries {
        out.push(d.glossary.iter().find(|e| e.name == format!("g{i}")).map(|e| {
            let mut g = Document::default();
            g.body = e.blocks.clone();
            (g, StoryRef::Body)
        }));
    }
    out
}

/// Reads the case's package, then saves and reads it twice: the three models.
fn rounds(m: &Markup) -> [(&'static str, Document); 3] {
    let a = craft_docx::read(&m.package()).unwrap();
    let b = craft_docx::read(&craft_docx::write(&a).unwrap()).unwrap();
    let c = craft_docx::read(&craft_docx::write(&b).unwrap()).unwrap();
    [("as read", a), ("after one save", b), ("after two saves", c)]
}

fn story_name(i: usize) -> String {
    ["body", "header"].get(i).map_or_else(|| format!("glossary entry g{}", i - 2), |s| s.to_string())
}

/// Every model range against its pair in the file: the problems found.
fn wider(src: &[Src], docs: &[(&str, Document)]) -> Vec<String> {
    let mut problems = Vec::new();
    for (round, d) in docs {
        for (i, story) in stories(d, src.len() - 2).iter().enumerate() {
            let Some((doc, s)) = story else { continue };
            let src = &src[i];
            let mut editors = BTreeSet::new();
            for r in doc.perm_ranges().iter().filter(|r| r.start.story == *s) {
                let place = format!("{}, {round}: the range for {}", story_name(i), r.editor);
                if !editors.insert(r.editor.clone()) {
                    problems.push(format!("{place} is there twice"));
                }
                let Some(start) = src.marks.iter().position(|m| m.start && m.editor == r.editor) else {
                    problems.push(format!("{place} isn't in the file"));
                    continue;
                };
                let Some(&end) = src.pairs.get(&start) else {
                    problems.push(format!("{place} has an end, but its start has none in the file"));
                    continue;
                };
                let (text, want) = (words_between(doc, *s, &r.start, &r.end), src.text(start, end));
                if !text.is_subset(&want) {
                    problems
                        .push(format!("{place}'s markers are around {:?}, beyond the file's {want:?}", text.difference(&want).collect::<Vec<_>>()));
                }
                if src.marks[start].cols.is_some() != r.cols.is_some() {
                    problems.push(format!("{place} has the columns {:?}, the file's start {:?}", r.cols, src.marks[start].cols));
                }
                let (cells, want) = (model_cells(doc, r), src.cells(start, end));
                if !cells.is_subset(&want) {
                    problems.push(format!(
                        "{place} (columns {:?}) covers the cells of {:?}, beyond the file's {want:?}",
                        r.cols,
                        cells.difference(&want).collect::<Vec<_>>()
                    ));
                }
            }
        }
    }
    problems
}

/// Every valid range without columns in the file against the model's: the problems found.
fn changed(src: &[Src], docs: &[(&str, Document)]) -> Vec<String> {
    let mut problems = Vec::new();
    for (round, d) in docs {
        for (i, story) in stories(d, src.len() - 2).iter().enumerate() {
            let src = &src[i];
            let valid: Vec<(usize, usize)> = src.pairs.iter().map(|(s, e)| (*s, *e)).filter(|(s, e)| src.valid(*s, *e)).collect();
            if valid.is_empty() {
                continue;
            }
            let Some((doc, s)) = story else {
                problems.push(format!("{}, {round}: the story is gone, with its ranges", story_name(i)));
                continue;
            };
            let ranges: Vec<PermRange> = doc.perm_ranges().into_iter().filter(|r| r.start.story == *s).collect();
            let kept = story_words(doc, *s);
            for (start, end) in valid {
                let editor = &src.marks[start].editor;
                let want: BTreeSet<String> = src.text(start, end).intersection(&kept).cloned().collect();
                let got: Vec<BTreeSet<String>> =
                    ranges.iter().filter(|r| &r.editor == editor).map(|r| words_between(doc, *s, &r.start, &r.end)).collect();
                if got != [want.clone()] {
                    problems.push(format!("{}, {round}: the range for {editor} covers {got:?}, want [{want:?}]", story_name(i)));
                }
            }
        }
    }
    problems
}

/// The cases' runner: a fixed seed unless `PROPTEST_RNG_SEED` sets one, the shared case count
/// (`PROPTEST_CASES`), and no failure files: a failing case becomes a named test.
fn config() -> ProptestConfig {
    let base = ProptestConfig { failure_persistence: None, ..ProptestConfig::default() };
    let rng_seed = match base.rng_seed {
        RngSeed::Random => RngSeed::Fixed(0x7065_726d_7321),
        seed => seed,
    };
    ProptestConfig { rng_seed, ..base }
}

proptest! {
    #![proptest_config(config())]

    /// No range covers anything its pair in the file doesn't, as read and through two saves: no
    /// word outside the text between its markers, and with columns, no cell outside the ones they
    /// cover; and a valid range without columns covers the same text as in the file (the words a
    /// model keeps). One test for both, so each case is read and saved once; even so, each case
    /// reads a document and saves it twice, so the test takes seconds, bounded by the shared case
    /// count.
    #[test]
    fn permission_ranges_never_cover_more_than_the_file_gives_them_and_valid_ones_keep_their_text(g in document()) {
        let m = Markup::of(&g);
        let src: Vec<Src> = m.stories().into_iter().map(Src::read).collect();
        let docs = rounds(&m);
        let mut problems = wider(&src, &docs);
        problems.extend(changed(&src, &docs));
        prop_assert!(problems.is_empty(), "{}\nbody: {}\nheader: {}\nglossary: {:?}", problems.join("\n"), m.body, m.header, m.glossary);
    }
}
