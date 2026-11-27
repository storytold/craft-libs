//! Comment and permission ranges whose markers sit in a table's structure, outside its cells, are
//! kept: each covers the same text after reading as in the file, and each save writes it as a start
//! followed by its end.
//!
//! ECMA-376 Part 1 (5th edition, 2016), §A.1 (`wml.xsd`): `CT_Tbl` takes `EG_RangeMarkupElements`
//! before `w:tblPr` (comment range markers, not permission ones); `EG_ContentRowContent` takes
//! `EG_RunLevelElts` (comment and permission range markers) around and between rows, and `CT_Row`
//! takes them around and between cells (`EG_ContentCellContent`). Row- and cell-level `w:sdt` and
//! `w:customXml` hold the same groups again (`CT_SdtContentRow`, `CT_CustomXmlRow`,
//! `CT_SdtContentCell`, `CT_CustomXmlCell`), and `mc:AlternateContent` (Part 3) can stand for any of
//! them. A `w:permStart` with no `w:permEnd` after it, or an end with no start before it, makes a
//! document non-conformant (§17.13.7.1, §17.13.7.2); a comment range marker without its other end
//! is the comment's single anchor point (§17.13.4.3, §17.13.4.4), so the span it marked is lost.
//!
//! A permission range can also be limited to some of a table's columns (`w:colFirst`, `w:colLast`,
//! §17.13.7.2): its start goes in the first row it covers, its end at the end of the last, and the
//! attributes name the first and last column it covers in each of those rows. Without them, a range
//! in a table "shall apply to the entire table", so a range that loses them covers more cells.
//!
//! Synthetic fixtures written for these tests from the schema.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::sync::Arc;

use craft_wordmodel::para::OBJ;
use craft_wordmodel::props::CharProps;
use craft_wordmodel::walk::each_para;
use craft_wordmodel::{Block, Blocks, Document, InlineObject, Paragraph, Path, PermRange, StoryRef, para_block};

use crate::fixtures::zip;

const W_NS: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml""#;
const CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/comments.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.comments+xml"/></Types>"#;
const ROOT_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
const DOC_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments" Target="comments.xml"/></Relationships>"#;

/// The range (of each kind) that starts in the paragraph before the table and ends at the place a
/// case tests, and the one that starts there and ends in the paragraph after the table: named by
/// the comment's author and the permission range's editor.
const ENDS: &str = "ends-here";
const STARTS: &str = "starts-here";
const COMMENT: &str = "comment";
const PERM: &str = "permission";

/// A two-column table holding `content` (its rows).
fn table(content: &str) -> String {
    format!(r#"<w:tbl><w:tblPr/><w:tblGrid><w:gridCol w:w="2400"/><w:gridCol w:w="2400"/></w:tblGrid>{content}</w:tbl>"#)
}

fn row(content: &str) -> String {
    format!("<w:tr>{content}</w:tr>")
}

fn cell(word: &str) -> String {
    format!(r#"<w:tc><w:tcPr/><w:p><w:r><w:t xml:space="preserve">{word} </w:t></w:r></w:p></w:tc>"#)
}

fn row_a() -> String {
    row(&(cell("a1") + &cell("a2")))
}

fn row_b() -> String {
    row(&(cell("b1") + &cell("b2")))
}

fn sdt(content: &str) -> String {
    format!("<w:sdt><w:sdtPr/><w:sdtContent>{content}</w:sdtContent></w:sdt>")
}

fn custom_xml(content: &str) -> String {
    format!(r#"<w:customXml w:element="part">{content}</w:customXml>"#)
}

/// Alternate content choosing `content`; the fallback (no text, no markers) shows if it is read
/// instead.
fn alternate(content: &str, fallback: &str) -> String {
    format!(r#"<mc:AlternateContent><mc:Choice Requires="w14">{content}</mc:Choice><mc:Fallback>{fallback}</mc:Fallback></mc:AlternateContent>"#)
}

/// Where each case puts the markers `m`, and whether it has permission ranges (`w:tbl` takes no
/// permission markers before `w:tblPr`).
type Case = (&'static str, bool, fn(&str) -> String);

/// The case whose markers come before a cell that starts with a table (its words `n1`, `n2`).
const NESTED: &str = "before a cell that starts with a table";

const CASES: [Case; 21] = [
    ("in the table before its properties", false, |m| {
        format!(r#"<w:tbl>{m}<w:tblPr/><w:tblGrid><w:gridCol w:w="2400"/><w:gridCol w:w="2400"/></w:tblGrid>{}{}</w:tbl>"#, row_a(), row_b())
    }),
    ("in a row before its first cell", true, |m| table(&(row_a() + &row(&format!("{m}{}{}", cell("b1"), cell("b2")))))),
    ("between a row's cells", true, |m| table(&(row_a() + &row(&format!("{}{m}{}", cell("b1"), cell("b2")))))),
    ("in a row after its last cell", true, |m| table(&(row(&format!("{}{}{m}", cell("a1"), cell("a2"))) + &row_b()))),
    ("between rows", true, |m| table(&format!("{}{m}{}", row_a(), row_b()))),
    ("after the last row", true, |m| table(&format!("{}{}{m}", row_a(), row_b()))),
    ("in a row's content control before the row", true, |m| table(&(row_a() + &sdt(&format!("{m}{}", row_b()))))),
    // The control still closes after its row, not after the markers.
    ("in a row's content control after the row", true, |m| table(&(sdt(&format!("{}{m}", row_a())) + &row_b()))),
    // A control with no row in it goes, as before; its markers stay.
    ("in a content control between rows holding no row", true, |m| table(&format!("{}{}{}", row_a(), sdt(m), row_b()))),
    ("in a row's custom XML", true, |m| table(&(row_a() + &custom_xml(&format!("{m}{}", row_b()))))),
    ("in a row's alternate content", true, |m| {
        table(&(row_a() + &alternate(&format!("{m}{}", row_b()), &row("<w:tc><w:p/></w:tc><w:tc><w:p/></w:tc>"))))
    }),
    ("in a cell's content control before the cell", true, |m| table(&(row_a() + &row(&(cell("b1") + &sdt(&format!("{m}{}", cell("b2")))))))),
    ("in a cell's content control after the cell", true, |m| table(&(row_a() + &row(&(sdt(&format!("{}{m}", cell("b1"))) + &cell("b2")))))),
    ("in a cell's custom XML", true, |m| table(&(row_a() + &row(&(cell("b1") + &custom_xml(&format!("{m}{}", cell("b2")))))))),
    ("in a cell's alternate content", true, |m| {
        table(&(row_a() + &row(&(cell("b1") + &alternate(&format!("{m}{}", cell("b2")), "<w:tc><w:p/></w:tc>")))))
    }),
    // A row with no cells isn't kept: its markers go on to the next cell, or end the table.
    ("in a row with no cells between rows", true, |m| table(&format!("{}{}{}", row_a(), row(m), row_b()))),
    ("in a row with no cells after the last row", true, |m| table(&format!("{}{}{}", row_a(), row_b(), row(m)))),
    // No row is kept, so no table: its markers go on to the paragraph after it.
    ("in a table whose only row has no cells", true, |m| table(&row(m))),
    // A legacy merge's continuation cell is skipped unread.
    ("before a merged cell's continuation", true, |m| {
        let first = r#"<w:tc><w:tcPr><w:hMerge w:val="restart"/></w:tcPr><w:p><w:r><w:t xml:space="preserve">b1 </w:t></w:r></w:p></w:tc>"#;
        table(&(row_a() + &row(&format!(r#"{first}{m}<w:tc><w:tcPr><w:hMerge/></w:tcPr><w:p/></w:tc>"#))))
    }),
    // A damaged cell (§17.4.65): nothing in it to hold the markers.
    ("before a cell with no paragraph", true, |m| table(&(row_a() + &row(&format!("{}{m}<w:tc><w:tcPr/></w:tc>", cell("b1")))))),
    (NESTED, true, |m| {
        let inner = format!(r#"<w:tbl><w:tblPr/><w:tblGrid><w:gridCol w:w="1200"/></w:tblGrid>{}{}</w:tbl>"#, row(&cell("n1")), row(&cell("n2")));
        table(&(row_a() + &row(&format!("{m}<w:tc><w:tcPr/>{inner}<w:p/></w:tc>{}", cell("b2")))))
    }),
];

/// The fixture's body: a paragraph, the case's table, a paragraph. The `ENDS` ranges start in the
/// first paragraph and end at the case's place; the `STARTS` ranges start there and end in the
/// last paragraph, where the comments' references are. Every annotation has an id of its own
/// (§17.13.4.3). With `marked` false, no range markers.
fn body(case: &Case, marked: bool) -> String {
    let (_, perms, place) = case;
    let perms = *perms && marked;
    let on = |s: &str, keep: bool| if keep { s.to_string() } else { String::new() };
    let starts = on(r#"<w:commentRangeStart w:id="1"/>"#, marked) + &on(&format!(r#"<w:permStart w:id="3" w:ed="{ENDS}"/>"#), perms);
    let m = on(r#"<w:commentRangeEnd w:id="1"/>"#, marked)
        + &on(r#"<w:permEnd w:id="3"/>"#, perms)
        + &on(r#"<w:commentRangeStart w:id="2"/>"#, marked)
        + &on(&format!(r#"<w:permStart w:id="4" w:ed="{STARTS}"/>"#), perms);
    let ends = on(r#"<w:commentRangeEnd w:id="2"/>"#, marked) + &on(r#"<w:permEnd w:id="4"/>"#, perms);
    format!(
        r#"<w:p><w:r><w:t xml:space="preserve">x </w:t></w:r>{starts}<w:r><w:t xml:space="preserve">before </w:t></w:r></w:p>{}<w:p><w:r><w:t xml:space="preserve">after </w:t></w:r>{ends}<w:r><w:t xml:space="preserve">y </w:t></w:r><w:r><w:commentReference w:id="1"/></w:r><w:r><w:commentReference w:id="2"/></w:r></w:p>"#,
        place(&m)
    )
}

fn package(body: &str) -> Vec<u8> {
    let doc = format!(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document {W_NS}><w:body>{body}<w:sectPr/></w:body></w:document>"#);
    let comments = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:comments {W_NS}><w:comment w:id="1" w:author="{ENDS}" w:initials="E"><w:p/></w:comment><w:comment w:id="2" w:author="{STARTS}" w:initials="S"><w:p/></w:comment></w:comments>"#
    );
    zip(&[
        ("[Content_Types].xml", CONTENT_TYPES.as_bytes()),
        ("_rels/.rels", ROOT_RELS.as_bytes()),
        ("word/document.xml", doc.as_bytes()),
        ("word/_rels/document.xml.rels", DOC_RELS.as_bytes()),
        ("word/comments.xml", comments.as_bytes()),
    ])
}

fn document_xml(docx: &[u8]) -> String {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(docx)).unwrap();
    let mut s = String::new();
    z.by_name("word/document.xml").unwrap().read_to_string(&mut s).unwrap();
    s
}

/// A word, or a range's start or end: (kind, name).
enum Token {
    Word(String),
    Start(&'static str, String),
    End(&'static str, String),
}

/// The value of attribute `name` in a tag's text.
fn attr(tag: &str, name: &str) -> String {
    let key = format!("{name}=\"");
    tag.find(&key).map(|i| tag[i + key.len()..].split('"').next().unwrap_or("").to_string()).unwrap_or_default()
}

/// The words and range markers of the fixture's own markup, in document order.
fn file_tokens(xml: &str) -> Vec<Token> {
    let comment = |id: &str| (if id == "1" { ENDS } else { STARTS }).to_string();
    let mut perms = BTreeMap::new();
    let mut out = Vec::new();
    for piece in xml.split('<').skip(1) {
        let (tag, text) = piece.split_once('>').unwrap_or((piece, ""));
        let id = attr(tag, "w:id");
        match tag.split(|c: char| c.is_whitespace() || c == '/').next().unwrap_or("") {
            "w:t" => out.extend(text.split_whitespace().map(|w| Token::Word(w.to_string()))),
            "w:commentRangeStart" => out.push(Token::Start(COMMENT, comment(&id))),
            "w:commentRangeEnd" => out.push(Token::End(COMMENT, comment(&id))),
            "w:permStart" => {
                let editor = attr(tag, "w:ed");
                perms.insert(id, editor.clone());
                out.push(Token::Start(PERM, editor));
            }
            "w:permEnd" => out.extend(perms.get(&id).map(|e| Token::End(PERM, e.clone()))),
            _ => {}
        }
    }
    out
}

/// The words and range markers of `blocks` as read, in document order through table cells.
fn read_tokens(d: &Document, blocks: &Blocks, perms: &mut BTreeMap<u32, String>, out: &mut Vec<Token>) {
    let author = |id: &u32| d.comments.get(id).map(|c| c.author.clone()).unwrap_or_default();
    for b in blocks {
        match &**b {
            Block::Para(p) => {
                let mut objects = p.objects.iter();
                let mut word = String::new();
                for c in p.text.chars() {
                    if c != OBJ && !c.is_whitespace() {
                        word.push(c);
                        continue;
                    }
                    if !word.is_empty() {
                        out.push(Token::Word(std::mem::take(&mut word)));
                    }
                    if c != OBJ {
                        continue;
                    }
                    match objects.next() {
                        Some(InlineObject::CommentStart { id }) => out.push(Token::Start(COMMENT, author(id))),
                        Some(InlineObject::CommentEnd { id }) => out.push(Token::End(COMMENT, author(id))),
                        Some(InlineObject::PermStart { id, editor, .. }) => {
                            perms.insert(*id, editor.clone());
                            out.push(Token::Start(PERM, editor.clone()));
                        }
                        Some(InlineObject::PermEnd { id }) => out.extend(perms.get(id).map(|e| Token::End(PERM, e.clone()))),
                        _ => {}
                    }
                }
                if !word.is_empty() {
                    out.push(Token::Word(word));
                }
            }
            Block::Table(t) => {
                for r in &t.rows {
                    for c in &r.cells {
                        read_tokens(d, &c.blocks, perms, out);
                    }
                }
            }
        }
    }
}

/// The words the range `name` of `kind` covers: from its start to its end, or to the end of the
/// document when it has none.
fn covered(tokens: &[Token], kind: &str, name: &str) -> Vec<String> {
    let mut inside = false;
    let mut out = Vec::new();
    for t in tokens {
        match t {
            Token::Start(k, n) if *k == kind && n == name => inside = true,
            Token::End(k, n) if *k == kind && n == name => inside = false,
            Token::Word(w) if inside => out.push(w.clone()),
            _ => {}
        }
    }
    out
}

/// Comment and permission ranges in a saved body that aren't one start followed by one end, and
/// how many of each kind there are.
fn unpaired(xml: &str) -> (Vec<String>, BTreeMap<&'static str, usize>) {
    let mut ranges: BTreeMap<(&'static str, String), Vec<bool>> = BTreeMap::new();
    for piece in xml.split('<').skip(1) {
        let tag = piece.split('>').next().unwrap_or("");
        let (kind, end) = match tag.split(|c: char| c.is_whitespace() || c == '/').next().unwrap_or("") {
            "w:commentRangeStart" => (COMMENT, false),
            "w:commentRangeEnd" => (COMMENT, true),
            "w:permStart" => (PERM, false),
            "w:permEnd" => (PERM, true),
            _ => continue,
        };
        ranges.entry((kind, attr(tag, "w:id"))).or_default().push(end);
    }
    let mut counts = BTreeMap::new();
    let mut bad = Vec::new();
    for ((kind, id), ends) in ranges {
        *counts.entry(kind).or_default() += 1;
        if ends != [false, true] {
            let marks: Vec<&str> = ends.iter().map(|e| if *e { "end" } else { "start" }).collect();
            bad.push(format!("{kind} range w:id={id} is saved as {}", marks.join(", ")));
        }
    }
    (bad, counts)
}

/// Comment and permission ranges that end, or start, at each place outside a table's cells where
/// the schema allows their markers cover the same text when read as in the file, and are saved as
/// matched pairs, through two saves.
///
/// About a second in a debug build: each place's file is read three times and saved twice.
#[test]
fn ranges_marked_between_table_rows_and_cells_cover_the_same_text_and_save_as_pairs() {
    let mut problems = Vec::new();
    for case in &CASES {
        let (name, perms, _) = case;
        let body = body(case, true);
        let file = file_tokens(&body);
        let kinds: &[&str] = if *perms { &[COMMENT, PERM] } else { &[COMMENT] };
        let ranges: Vec<(&str, &str, Vec<String>)> = kinds.iter().flat_map(|k| [ENDS, STARTS].map(|n| (*k, n, covered(&file, k, n)))).collect();
        assert!(ranges.iter().all(|(_, _, words)| !words.is_empty()), "{name}: the fixture marks no text");
        let mut d = craft_docx::read(&package(&body)).unwrap();
        for (round, save) in [("as read", Some("first save")), ("after one save", Some("second save")), ("after two saves", None)] {
            let mut tokens = Vec::new();
            read_tokens(&d, &d.body, &mut BTreeMap::new(), &mut tokens);
            for (kind, range, words) in &ranges {
                let got = covered(&tokens, kind, range);
                if &got != words {
                    problems.push(format!("{name}, {round}: the {kind} range {range:?} covers {got:?}, want {words:?}"));
                }
            }
            let Some(save) = save else { break };
            let saved = craft_docx::write(&d).unwrap();
            let (bad, counts) = unpaired(&document_xml(&saved));
            problems.extend(bad.into_iter().map(|b| format!("{name}, {save}: {b}")));
            for kind in kinds {
                let n = counts.get(kind).copied().unwrap_or(0);
                if n != 2 {
                    problems.push(format!("{name}, {save}: {n} {kind} ranges saved, want 2"));
                }
            }
            d = craft_docx::read(&saved).unwrap();
        }
    }
    assert!(problems.is_empty(), "{} problems:\n{}", problems.len(), problems.join("\n"));
}

/// The tables in `blocks` without their text: rows, cells with their spans, content controls, and
/// tables inside cells.
fn layout(blocks: &Blocks) -> String {
    let mut s = String::new();
    for b in blocks {
        if let Block::Table(t) = &**b {
            s += &format!("table {:?} [", t.controls);
            for r in &t.rows {
                s += &format!("row {:?} [", r.controls);
                for c in &r.cells {
                    s += &format!("cell {} {:?} [{}] ", c.props.span, c.controls, layout(&c.blocks));
                }
                s += "] ";
            }
            s += "] ";
        }
    }
    s
}

/// Range markers outside a table's cells change none of its rows, cells or content controls: the
/// table reads as it does without them.
#[test]
fn range_markers_between_table_rows_and_cells_change_no_rows_cells_or_controls() {
    for case in &CASES {
        let read = |marked| layout(&craft_docx::read(&package(&body(case, marked))).unwrap().body);
        assert_eq!(read(true), read(false), "{}", case.0);
    }
}

/// The editor the column cases' permission range names.
const COLUMNS: &str = "columns";

/// Where a column case's permission range starts. It always ends in `w:tr` after the second
/// row's last cell, as in the spec's example (§17.13.7.2).
#[derive(Clone, Copy)]
enum StartAt {
    /// In the first row, before its first cell.
    Row,
    /// In the first row's first cell, before its paragraph (the spec's example).
    Cell,
}

/// A three-by-three table of the words `a0` … `c2` between two paragraphs, with a permission range
/// over its first two rows that starts as `at` says and has the column attributes `cols`.
fn column_body(at: StartAt, cols: &str) -> String {
    let start = format!(r#"<w:permStart w:id="5" w:ed="{COLUMNS}"{cols}/>"#);
    let cells = |r: char, from: usize| (from..3).map(|c| cell(&format!("{r}{c}"))).collect::<String>();
    let first = match at {
        StartAt::Row => format!("{start}{}", cells('a', 0)),
        StartAt::Cell => format!(r#"<w:tc><w:tcPr/>{start}<w:p><w:r><w:t xml:space="preserve">a0 </w:t></w:r></w:p></w:tc>{}"#, cells('a', 1)),
    };
    let grid = r#"<w:gridCol w:w="1600"/>"#.repeat(3);
    format!(
        r#"<w:p><w:r><w:t xml:space="preserve">before </w:t></w:r></w:p><w:tbl><w:tblPr/><w:tblGrid>{grid}</w:tblGrid>{}{}{}</w:tbl><w:p><w:r><w:t xml:space="preserve">after </w:t></w:r></w:p>"#,
        row(&first),
        row(&format!(r#"{}<w:permEnd w:id="5"/>"#, cells('b', 0))),
        row(&cells('c', 0)),
    )
}

/// A column case: its name, where its range starts, the column attributes its start has in the
/// file, and the columns it should keep (`None`: none).
type ColumnCase = (&'static str, StartAt, &'static str, Option<[u32; 2]>);

const COLUMN_CASES: [ColumnCase; 7] = [
    ("one column, the start in the row", StartAt::Row, r#" w:colFirst="1" w:colLast="1""#, Some([1, 1])),
    ("two columns, the start in the first cell", StartAt::Cell, r#" w:colFirst="0" w:colLast="1""#, Some([0, 1])),
    ("no columns", StartAt::Row, "", None),
    // The spec says to ignore such a pair; it is saved as the file has it.
    ("the first column after the last", StartAt::Row, r#" w:colFirst="2" w:colLast="1""#, Some([2, 1])),
    // Non-conformant without its pair: it still limits the range to that column.
    ("colFirst alone", StartAt::Cell, r#" w:colFirst="2""#, Some([2, 2])),
    // `ST_DecimalNumber` is an `xsd:integer`: a sign, leading zeros and spaces around it are allowed.
    ("an integer's other spellings", StartAt::Row, r#" w:colFirst=" +1 " w:colLast="01""#, Some([1, 1])),
    ("the largest column number the model holds", StartAt::Row, r#" w:colFirst="4294967295" w:colLast="4294967295""#, Some([u32::MAX, u32::MAX])),
];

/// The tags named `name` in `xml`, without their angle brackets.
fn tags<'a>(xml: &'a str, name: &'a str) -> impl Iterator<Item = &'a str> {
    xml.split('<')
        .skip(1)
        .map(|piece| piece.split('>').next().unwrap_or(""))
        .filter(move |tag| tag.split(|c: char| c.is_whitespace() || c == '/').next() == Some(name))
}

/// The cells a range limited to columns covers, worked out from the model on its own reading of
/// §17.13.7.2: the table its start is in, the rows from its start's to its end's, and in each the
/// cells wholly inside its columns, as (row, first grid column). `None` when it has no columns or
/// its end isn't in that table.
fn range_cells(d: &Document, r: &PermRange) -> Option<(Path, BTreeSet<(usize, usize)>)> {
    let [first, last] = r.cols?;
    let (table_path, start_row, _) = r.start.path.cell()?;
    let n = table_path.0.len();
    if r.end.story != r.start.story || r.end.path.0.len() < n + 3 || !r.end.path.0.starts_with(&table_path.0) {
        return None;
    }
    let end_row = *r.end.path.0.get(n)? as usize;
    let t = d.table(r.start.story, &table_path)?;
    let mut cells = BTreeSet::new();
    for row in start_row..=end_row {
        for (c, cell) in t.rows.get(row)?.cells.iter().enumerate() {
            let g = t.grid_col(row, c);
            if first as usize <= g && g + cell.span().max(1) <= last as usize + 1 {
                cells.insert((row, g));
            }
        }
    }
    Some((table_path, cells))
}

/// Every cell in rows `rows` and grid columns `cols`, as [`range_cells`] gives them.
fn grid(rows: std::ops::RangeInclusive<usize>, cols: std::ops::RangeInclusive<usize>) -> BTreeSet<(usize, usize)> {
    rows.flat_map(|r| cols.clone().map(move |c| (r, c))).collect()
}

/// The permission markers anywhere in the body, in document order: (start or end, id).
fn body_perm_markers(d: &Document) -> Vec<(bool, u32)> {
    let mut out = Vec::new();
    for b in &d.body {
        each_para(b, 0, &mut |p| {
            for o in &p.objects {
                match o {
                    InlineObject::PermStart { id, .. } => out.push((true, *id)),
                    InlineObject::PermEnd { id } => out.push((false, *id)),
                    _ => {}
                }
            }
        });
    }
    out
}

/// A permission range limited to some of a table's columns keeps them, and the cells they cover,
/// as read and through two saves; a range without them gains none.
#[test]
fn a_permission_range_keeps_its_table_columns_and_gains_none_through_two_saves() {
    let mut problems = Vec::new();
    for (name, at, attrs, want) in &COLUMN_CASES {
        let body = column_body(*at, attrs);
        let words = covered(&file_tokens(&body), PERM, COLUMNS);
        assert!(!words.is_empty(), "{name}: the fixture marks no text");
        let saved_cols = want.map_or([String::new(), String::new()], |c| c.map(|v| v.to_string()));
        // Columns that name some of the table's: the first two rows' cells in them.
        let cells = want.filter(|[a, b]| a <= b && *b < 3).map(|[a, b]| (Path::top(1), grid(0..=1, a as usize..=b as usize)));
        let mut d = craft_docx::read(&package(&body)).unwrap();
        for (round, save) in [("as read", Some("first save")), ("after one save", Some("second save")), ("after two saves", None)] {
            let ranges = d.perm_ranges();
            let cols: Vec<Option<[u32; 2]>> = ranges.iter().map(|r| r.cols).collect();
            if cols != [*want] {
                problems.push(format!("{name}, {round}: ranges with the columns {cols:?}, want [{want:?}]"));
            }
            if cells.is_some() && ranges.first().and_then(|r| range_cells(&d, r)) != cells {
                problems.push(format!("{name}, {round}: covers the cells {:?}, want {cells:?}", ranges.first().and_then(|r| range_cells(&d, r))));
            }
            let mut tokens = Vec::new();
            read_tokens(&d, &d.body, &mut BTreeMap::new(), &mut tokens);
            let got = covered(&tokens, PERM, COLUMNS);
            if got != words {
                problems.push(format!("{name}, {round}: the range's markers bound {got:?}, the file's {words:?}"));
            }
            let Some(save) = save else { break };
            let saved = craft_docx::write(&d).unwrap();
            let xml = document_xml(&saved);
            let starts: Vec<[String; 2]> = tags(&xml, "w:permStart").map(|t| [attr(t, "w:colFirst"), attr(t, "w:colLast")]).collect();
            if starts != [saved_cols.clone()] {
                problems.push(format!("{name}, {save}: saved starts with the columns {starts:?}, want [{saved_cols:?}]"));
            }
            let (bad, _) = unpaired(&xml);
            problems.extend(bad.into_iter().map(|b| format!("{name}, {save}: {b}")));
            d = craft_docx::read(&saved).unwrap();
        }
    }
    assert!(problems.is_empty(), "{} problems:\n{}", problems.len(), problems.join("\n"));
}

/// A permission range whose column attributes are there but aren't column numbers the model can
/// hold is dropped, start and end, and nothing of it is saved: read as a range without columns it
/// would apply to the entire table (§17.13.7.2). One good value isn't copied to the other.
#[test]
fn a_permission_range_whose_columns_are_not_column_numbers_is_dropped() {
    let mut problems = Vec::new();
    for (name, attrs) in [
        ("not a number", r#" w:colFirst="two" w:colLast="1""#),
        ("negative", r#" w:colFirst="1" w:colLast="-1""#),
        ("past what the model holds", r#" w:colFirst="0" w:colLast="4294967296""#),
    ] {
        for at in [StartAt::Row, StartAt::Cell] {
            let d = craft_docx::read(&package(&column_body(at, attrs))).unwrap();
            let left = (d.perm_ranges().len(), body_perm_markers(&d).len());
            let xml = document_xml(&craft_docx::write(&d).unwrap());
            let saved = (tags(&xml, "w:permStart").count(), tags(&xml, "w:permEnd").count());
            if left != (0, 0) || saved != (0, 0) {
                problems.push(format!("{name}: (ranges, markers) {left:?} as read, (starts, ends) {saved:?} saved; want none"));
            }
        }
    }
    assert!(problems.is_empty(), "{} problems:\n{}", problems.len(), problems.join("\n"));
}

/// A table of `cols` grid columns whose rows are `rows` (row `r`'s cells read `r0`, `r1`…), between
/// a paragraph before it and one after it; `marks(i)` gives the markers row `i` has before its
/// cells and after them.
fn grid_body(cols: usize, rows: &str, marks: impl Fn(usize) -> (String, String)) -> String {
    let mut table = String::new();
    for (i, r) in rows.chars().enumerate() {
        let (before, after) = marks(i);
        let cells: String = (0..cols).map(|c| cell(&format!("{r}{c}"))).collect();
        table += &row(&format!("{before}{cells}{after}"));
    }
    let widths = r#"<w:gridCol w:w="400"/>"#.repeat(cols);
    format!(
        r#"<w:p><w:r><w:t xml:space="preserve">before </w:t></w:r></w:p><w:tbl><w:tblPr/><w:tblGrid>{widths}</w:tblGrid>{table}</w:tbl><w:p><w:r><w:t xml:space="preserve">after </w:t></w:r></w:p>"#
    )
}

/// A range limited to columns `cols`, started before row `from`'s cells and ended after row
/// `to`'s, for [`grid_body`].
fn column_marks(cols: &str, from: usize, to: usize) -> impl Fn(usize) -> (String, String) {
    let start = format!(r#"<w:permStart w:id="5" w:ed="{COLUMNS}"{cols}/>"#);
    move |i| (if i == from { start.clone() } else { String::new() }, if i == to { r#"<w:permEnd w:id="5"/>"#.to_string() } else { String::new() })
}

/// A table of 65 grid columns, wider than the 63 the reader keeps, whose one row holds `cells`; a
/// range limited to columns `cols` starts before them and ends after them.
fn wide_body(cells: &str, cols: &str) -> String {
    let marks = column_marks(cols, 0, 0);
    let (start, end) = marks(0);
    let row = if cells.starts_with("<w:trPr>") {
        let (pr, rest) = cells.split_at(cells.find("</w:trPr>").map_or(0, |i| i + 9));
        format!("<w:tr>{pr}{start}{rest}{end}</w:tr>")
    } else {
        row(&format!("{start}{cells}{end}"))
    };
    let grid = r#"<w:gridCol w:w="400"/>"#.repeat(65);
    format!(
        r#"<w:p><w:r><w:t xml:space="preserve">before </w:t></w:r></w:p><w:tbl><w:tblPr/><w:tblGrid>{grid}</w:tblGrid>{row}</w:tbl><w:p><w:r><w:t xml:space="preserve">after </w:t></w:r></w:p>"#
    )
}

/// A range limited to columns in a table whose grid the reader changes keeps naming the cells it
/// names in the file, of those the reader keeps: the reader keeps 63 of a table's columns, and cuts
/// a span or `w:gridBefore` past them, which moves the cells after them to other columns. A range
/// whose columns name kept cells stays as it is; one whose columns run past the kept ones is cut to
/// them; one that names a moved cell follows it; one that names no kept cell, or would now name a
/// cell it doesn't name in the file, is dropped, start and end. It never names a column the table
/// no longer has (which would apply it to the entire table, §17.13.7.2), nor an unrelated cell.
#[test]
fn a_column_range_in_a_grid_the_reader_changes_keeps_the_cells_it_names_or_is_dropped() {
    let cols = |a: u32, b: u32| format!(r#" w:colFirst="{a}" w:colLast="{b}""#);
    // Two rows of 65 cells, the range over both.
    let wide = |a, b| grid_body(65, "ab", column_marks(&cols(a, b), 0, 1));
    // One row whose cut span or omitted columns move its cell `moved` from column 64 to 63.
    let span = |a, b| {
        let cells = format!(r#"<w:tc><w:tcPr><w:gridSpan w:val="64"/></w:tcPr><w:p><w:r><w:t>wide</w:t></w:r></w:p></w:tc>{}"#, cell("moved"));
        wide_body(&cells, &cols(a, b))
    };
    let omitted = |a, b| wide_body(&format!(r#"<w:trPr><w:gridBefore w:val="64"/></w:trPr>{}"#, cell("moved")), &cols(a, b));
    // Each case: the columns and cells it keeps (`None`: dropped).
    let cases: [(&str, String, Option<([u32; 2], BTreeSet<(usize, usize)>)>); 7] = [
        ("columns kept", wide(2, 3), Some(([2, 3], grid(0..=1, 2..=3)))),
        ("columns kept and not", wide(60, 64), Some(([60, 62], grid(0..=1, 60..=62)))),
        ("no column kept", wide(64, 64), None),
        ("a cut span, the column the moved cell takes", span(63, 63), None),
        ("a cut span, the moved cell's column", span(64, 64), Some(([63, 63], grid(0..=0, 63..=63)))),
        ("cut omitted columns, the column the moved cell takes", omitted(63, 63), None),
        ("cut omitted columns, the moved cell's column", omitted(64, 64), Some(([63, 63], grid(0..=0, 63..=63)))),
    ];
    let mut problems = Vec::new();
    for (name, body, want) in cases {
        let mut d = craft_docx::read(&package(&body)).unwrap();
        let kept = d.table(StoryRef::Body, &Path::top(1)).map_or(0, |t| t.cols());
        assert!(kept < 65, "{name}: the reader keeps every column; this test needs it not to");
        for round in ["as read", "after one save", "after two saves"] {
            let ranges = d.perm_ranges();
            let cols: Vec<Option<[u32; 2]>> = ranges.iter().map(|r| r.cols).collect();
            let markers = body_perm_markers(&d).len();
            match &want {
                Some((c, g)) => {
                    let cells = (Path::top(1), g.clone());
                    let got = ranges.first().and_then(|r| range_cells(&d, r));
                    if cols != [Some(*c)] || got.as_ref() != Some(&cells) {
                        problems.push(format!("{name}, {round}: ranges with the columns {cols:?} over {got:?}, want [{c:?}] over {cells:?}"));
                    }
                }
                None if !ranges.is_empty() || markers != 0 => {
                    problems.push(format!("{name}, {round}: ranges with the columns {cols:?} and {markers} markers left, want none"));
                }
                None => {}
            }
            if round != "after two saves" {
                d = craft_docx::read(&craft_docx::write(&d).unwrap()).unwrap();
            }
        }
    }
    assert!(problems.is_empty(), "{} problems:\n{}", problems.len(), problems.join("\n"));
}

/// A permission start read in a table's structure counts only with an end of its own: with none,
/// or with another start of its `w:id` open (which end closes which is unclear), it is dropped
/// with any end it has, as before such markers were read, and nothing of it is saved. A start in a
/// paragraph stays, with the end the file gives it: an end closes the latest start of its `w:id`
/// still open. One before the table start, whose end that start takes, is left with none, and makes
/// nothing editable.
#[test]
fn a_start_in_a_tables_structure_without_an_end_of_its_own_is_dropped() {
    let plain = column_body(StartAt::Row, "");
    let end = r#"<w:permEnd w:id="5"/>"#;
    let paragraph_start = r#"<w:permStart w:id="5" w:ed="paragraph"/>"#;
    let before = r#"<w:p><w:r><w:t xml:space="preserve">before "#;
    let b0 = r#"<w:p><w:r><w:t xml:space="preserve">b0 "#;
    // Each case: its ranges (editor, words), and the starts and ends left in the body and saved.
    let cases: [(&str, String, &[(&str, &[&str])], (usize, usize)); 4] = [
        ("no end", plain.replace(end, ""), &[], (0, 0)),
        ("a second start with its id", plain.replace(end, &format!(r#"<w:permStart w:id="5" w:ed="again"/>{end}"#)), &[], (0, 0)),
        ("a paragraph's start with its id before it", plain.replacen(before, &format!("<w:p>{paragraph_start}{}", &before[5..]), 1), &[], (1, 0)),
        (
            "a paragraph's start with its id after it",
            plain.replacen(b0, &format!("<w:p>{paragraph_start}{}", &b0[5..]), 1),
            &[("paragraph", &["b0", "b1", "b2"])],
            (1, 1),
        ),
    ];
    let mut problems = Vec::new();
    for (name, body, want, left) in cases {
        let mut d = craft_docx::read(&package(&body)).unwrap();
        for round in ["as read", "after one save", "after two saves"] {
            let editors: Vec<String> = d.perm_ranges().iter().map(|r| r.editor.clone()).collect();
            let mut tokens = Vec::new();
            read_tokens(&d, &d.body, &mut BTreeMap::new(), &mut tokens);
            let got: Vec<(String, Vec<String>)> = editors.iter().map(|e| (e.clone(), covered(&tokens, PERM, e))).collect();
            let expect: Vec<(String, Vec<String>)> = want.iter().map(|(e, w)| (e.to_string(), w.iter().map(|w| w.to_string()).collect())).collect();
            let markers = body_perm_markers(&d);
            let marks = (markers.iter().filter(|(start, _)| *start).count(), markers.iter().filter(|(start, _)| !*start).count());
            if got != expect || marks != left {
                problems.push(format!("{name}, {round}: ranges {got:?} and (starts, ends) {marks:?}, want {expect:?} and {left:?}"));
            }
            if round != "after two saves" {
                let saved = craft_docx::write(&d).unwrap();
                let xml = document_xml(&saved);
                let (starts, ends) = (tags(&xml, "w:permStart").count(), tags(&xml, "w:permEnd").count());
                if (starts, ends) != left {
                    problems.push(format!("{name}, saved {round}: {starts} starts and {ends} ends, want {left:?}"));
                }
                d = craft_docx::read(&saved).unwrap();
            }
        }
    }
    assert!(problems.is_empty(), "{} problems:\n{}", problems.len(), problems.join("\n"));
}

/// Deleting the row a column range ends in, or moving it before the row it starts in, leaves its
/// start without an end after it, and so does deleting the paragraph a range ends in: nothing is
/// editable through it, then or after saving. An unmatched start doesn't run on to the end of its
/// story.
#[test]
fn a_range_whose_end_is_deleted_or_moved_before_its_start_makes_nothing_editable() {
    let table = grid_body(3, "abcde", column_marks(r#" w:colFirst="1" w:colLast="1""#, 0, 3));
    let paragraphs = r#"<w:p><w:permStart w:id="1" w:edGrp="everyone"/><w:r><w:t>one</w:t></w:r></w:p><w:p><w:r><w:t>two</w:t></w:r><w:permEnd w:id="1"/></w:p><w:p><w:r><w:t>three</w:t></w:r></w:p>"#;
    let cases: [(&str, &str, fn(&mut Document)); 3] = [
        ("the last row of a column range deleted", &table, |d| d.table_mut(StoryRef::Body, &Path::top(1)).unwrap().delete_row(3)),
        ("the last row of a column range moved first", &table, |d| {
            let t = d.table_mut(StoryRef::Body, &Path::top(1)).unwrap();
            let last = t.rows.remove(3);
            t.rows.insert(0, last);
        }),
        ("the paragraph a range ends in deleted", paragraphs, |d| {
            d.body.remove(1);
        }),
    ];
    let mut problems = Vec::new();
    for (name, body, edit) in cases {
        let mut d = craft_docx::read(&package(body)).unwrap();
        assert_eq!(d.perm_ranges().len(), 1, "{name}: the fixture has one range");
        edit(&mut d);
        for round in ["edited", "after one save", "after two saves"] {
            let ranges = d.perm_ranges();
            if !ranges.is_empty() {
                problems.push(format!("{name}, {round}: {} editable ranges, want none: {ranges:?}", ranges.len()));
            }
            if round != "after two saves" {
                d = craft_docx::read(&craft_docx::write(&d).unwrap()).unwrap();
            }
        }
    }
    assert!(problems.is_empty(), "{} problems:\n{}", problems.len(), problems.join("\n"));
}

/// A column range whose first cell starts with a nested table keeps its start in its own table:
/// the start goes to the cell's own paragraph after the nested table, not into the nested table,
/// and no paragraph is added for it beyond the one a save gives a cell that ends with a table. So
/// its columns still name the outer table's, as read and through two saves.
#[test]
fn a_column_ranges_start_stays_in_its_own_table_when_its_first_cell_starts_with_a_nested_one() {
    let inner = format!(r#"<w:tbl><w:tblPr/><w:tblGrid><w:gridCol w:w="1200"/></w:tblGrid>{}</w:tbl>"#, row(&cell("nested")));
    let a0 = r#"<w:p><w:r><w:t xml:space="preserve">a0 "#;
    let mut problems = Vec::new();
    let a0_para = r#"<w:p><w:r><w:t xml:space="preserve">a0 </w:t></w:r></w:p>"#;
    for (name, at, nested_only) in [
        ("the start in the row", StartAt::Row, false),
        ("the start in the cell", StartAt::Cell, false),
        // A damaged cell (§17.4.65): the nested table is all it holds.
        ("the start in the row, the cell holding only the nested table", StartAt::Row, true),
    ] {
        let body = column_body(at, r#" w:colFirst="1" w:colLast="1""#).replacen(a0, &format!("{inner}{a0}"), 1);
        let body = if nested_only { body.replacen(a0_para, "", 1) } else { body };
        let unmarked =
            body.replace(r#"<w:permStart w:id="5" w:ed="columns" w:colFirst="1" w:colLast="1"/>"#, "").replace(r#"<w:permEnd w:id="5"/>"#, "");
        let first_cell_blocks = |d: &Document| d.table(StoryRef::Body, &Path::top(1)).map_or(0, |t| t.rows[0].cells[0].blocks.len());
        // As saved: a cell that ends with a table is given a paragraph after it.
        let saved = craft_docx::write(&craft_docx::read(&package(&unmarked)).unwrap()).unwrap();
        let blocks = first_cell_blocks(&craft_docx::read(&saved).unwrap());
        let mut d = craft_docx::read(&package(&body)).unwrap();
        for round in ["as read", "after one save", "after two saves"] {
            let ranges = d.perm_ranges();
            let tables: Vec<_> = ranges.iter().map(|r| (r.start.path.cell().map(|c| c.0), r.end.path.cell().map(|c| c.0))).collect();
            let cells = ranges.first().and_then(|r| range_cells(&d, r));
            if tables != [(Some(Path::top(1)), Some(Path::top(1)))] || cells != Some((Path::top(1), grid(0..=1, 1..=1))) {
                problems.push(format!(
                    "{name}, {round}: start and end in the tables {tables:?}, covering {cells:?}; want the outer table, its column 1 in rows 0–1"
                ));
            }
            if first_cell_blocks(&d) != blocks {
                problems.push(format!("{name}, {round}: the first cell holds {} blocks, {blocks} without the range", first_cell_blocks(&d)));
            }
            if round != "after two saves" {
                d = craft_docx::read(&craft_docx::write(&d).unwrap()).unwrap();
            }
        }
    }
    assert!(problems.is_empty(), "{} problems:\n{}", problems.len(), problems.join("\n"));
}

/// Splitting a table inside a column range closes the range at the end of the first table and
/// opens it again, with an id of its own, at the start of the second: each table holds its own
/// pair, over the same column of its own rows, as split and through two saves. Neither range
/// crosses from one table to the other.
#[test]
fn splitting_a_table_inside_a_column_range_gives_each_table_a_range_of_its_own() {
    let body = grid_body(3, "abcde", column_marks(r#" w:colFirst="1" w:colLast="1""#, 0, 3));
    let mut d = craft_docx::read(&package(&body)).unwrap();
    let mut next = d.next_perm_id();
    let tail = d.table_mut(StoryRef::Body, &Path::top(1)).unwrap().split_off(2, &mut next);
    d.body.insert(2, para_block(Paragraph::with_text("between", CharProps::default())));
    d.body.insert(3, Arc::new(Block::Table(tail)));
    let mut problems = Vec::new();
    for round in ["split", "after one save", "after two saves"] {
        let ranges = d.perm_ranges();
        let ids: BTreeSet<u32> = ranges.iter().map(|r| r.id).collect();
        let cells: Vec<_> = ranges.iter().map(|r| range_cells(&d, r)).collect();
        let want = [Some((Path::top(1), grid(0..=1, 1..=1))), Some((Path::top(3), grid(0..=1, 1..=1)))];
        if ranges.len() != 2 || ids.len() != 2 || cells != want || ranges.iter().any(|r| r.cols != Some([1, 1])) {
            problems.push(format!(
                "{round}: ranges {ranges:?} covering {cells:?}; want two, with ids of their own, each over column 1 of its own table's two rows"
            ));
        }
        if round != "after two saves" {
            d = craft_docx::read(&craft_docx::write(&d).unwrap()).unwrap();
        }
    }
    assert!(problems.is_empty(), "{} problems:\n{}", problems.len(), problems.join("\n"));
}

/// The words between a permission range's markers, in document order.
fn range_words(d: &Document, r: &PermRange) -> Vec<String> {
    let mut text = String::new();
    for path in d.paths_between(&r.start, &r.end) {
        let Some(p) = d.para(r.start.story, &path) else { continue };
        let from = if path == r.start.path { r.start.off } else { 0 };
        let to = if path == r.end.path { r.end.off } else { p.text.len() };
        text.push_str(&p.text.get(from..to).unwrap_or("").replace(OBJ, " "));
        text.push(' ');
    }
    text.split_whitespace().map(str::to_string).collect()
}

/// An end that closes no start in the file closes none as read: a permission start before a table
/// nested in its cell keeps the end the file gives it, inside that table, and an end with its `w:id`
/// after the outer table opens nothing. A range without columns keeps its place in the text, so it
/// still covers the nested table's text up to its end. A range limited to columns has its start
/// after the nested table, where its columns name its own table's, and so after its end: it is
/// dropped. Either way no later text becomes editable, as read and through two saves.
#[test]
fn a_late_end_that_closes_no_start_in_the_file_makes_nothing_after_it_editable() {
    let inner = r#"<w:tbl><w:tblPr/><w:tblGrid><w:gridCol w:w="1200"/></w:tblGrid><w:tr><w:tc><w:p><w:r><w:t>inside</w:t></w:r><w:permEnd w:id="5"/></w:p></w:tc></w:tr></w:tbl>"#;
    let mut problems = Vec::new();
    for (cols, want) in [("", vec!["inside"]), (r#" w:colFirst="0" w:colLast="0""#, vec![])] {
        let start = format!(r#"<w:permStart w:id="5" w:ed="alpha"{cols}/>"#);
        for (place, in_row) in [("in the row", true), ("in the cell", false)] {
            let (row_start, cell_start) = if in_row { (start.as_str(), "") } else { ("", start.as_str()) };
            let outer =
                table(&row(&format!(r#"{row_start}<w:tc>{cell_start}{inner}<w:p><w:r><w:t>outside</w:t></w:r></w:p></w:tc>{}"#, cell("other"))));
            let body = format!(r#"<w:p><w:r><w:t>before</w:t></w:r></w:p>{outer}<w:p><w:r><w:t>after</w:t></w:r><w:permEnd w:id="5"/></w:p>"#);
            let mut d = craft_docx::read(&package(&body)).unwrap();
            for round in ["as read", "after one save", "after two saves"] {
                let got: Vec<Vec<String>> = d.perm_ranges().iter().map(|r| range_words(&d, r)).collect();
                let expect: Vec<Vec<String>> = if want.is_empty() { Vec::new() } else { vec![want.iter().map(|w| w.to_string()).collect()] };
                if got != expect {
                    problems.push(format!("start {place}, columns {cols:?}, {round}: ranges over {got:?}, want {expect:?}"));
                }
                if round != "after two saves" {
                    d = craft_docx::read(&craft_docx::write(&d).unwrap()).unwrap();
                }
            }
        }
    }
    assert!(problems.is_empty(), "{} problems:\n{}", problems.len(), problems.join("\n"));
}

/// A range limited to columns whose end sits between two rows, or in a row with no cells after the
/// last row it covers, ends in that row: as read and through two saves it covers its column in the
/// rows from its start's to that one, and not the next row's cell.
#[test]
fn a_column_ranges_end_between_rows_ends_the_row_before_it() {
    let start = format!(r#"<w:permStart w:id="5" w:ed="{COLUMNS}" w:colFirst="1" w:colLast="1"/>"#);
    let cells = |r: char| (0..3).map(|c| cell(&format!("{r}{c}"))).collect::<String>();
    let end = r#"<w:permEnd w:id="5"/>"#;
    let mut problems = Vec::new();
    for (name, between) in [("between rows", end.to_string()), ("in a row with no cells", row(end))] {
        let rows = format!("{}{}{between}{}", row(&format!("{start}{}", cells('a'))), row(&cells('b')), row(&cells('c')));
        let widths = r#"<w:gridCol w:w="1600"/>"#.repeat(3);
        let body = format!(
            r#"<w:p><w:r><w:t xml:space="preserve">before </w:t></w:r></w:p><w:tbl><w:tblPr/><w:tblGrid>{widths}</w:tblGrid>{rows}</w:tbl><w:p><w:r><w:t xml:space="preserve">after </w:t></w:r></w:p>"#
        );
        let mut d = craft_docx::read(&package(&body)).unwrap();
        for round in ["as read", "after one save", "after two saves"] {
            let cells: Vec<_> = d.perm_ranges().iter().map(|r| range_cells(&d, r)).collect();
            let want = [Some((Path::top(1), grid(0..=1, 1..=1)))];
            if cells != want {
                problems.push(format!("{name}, {round}: ranges covering {cells:?}, want {want:?}"));
            }
            if round != "after two saves" {
                d = craft_docx::read(&craft_docx::write(&d).unwrap()).unwrap();
            }
        }
    }
    assert!(problems.is_empty(), "{} problems:\n{}", problems.len(), problems.join("\n"));
}

/// Markers of a range limited to columns that come after a story's last table, with no paragraph
/// after it, stay out of that table, in the paragraph a save gives a story that ends with a table:
/// the range doesn't become one over that table's cells, whether it starts after the table too
/// (and covers nothing) or in it (and ends outside it).
#[test]
fn a_column_ranges_markers_after_a_storys_last_table_stay_out_of_it() {
    let start = format!(r#"<w:permStart w:id="5" w:ed="{COLUMNS}" w:colFirst="1" w:colLast="1"/>"#);
    let end = r#"<w:permEnd w:id="5"/>"#;
    let widths = r#"<w:gridCol w:w="1600"/>"#.repeat(3);
    let cells = |r: char| (0..3).map(|c| cell(&format!("{r}{c}"))).collect::<String>();
    let mut problems = Vec::new();
    for (name, first, after) in
        [("both after the table", String::new(), format!("{start}{end}")), ("only its end after it", start.clone(), end.to_string())]
    {
        let body = format!(
            r#"<w:p><w:r><w:t xml:space="preserve">before </w:t></w:r></w:p><w:tbl><w:tblPr/><w:tblGrid>{widths}</w:tblGrid>{}{}</w:tbl>{after}"#,
            row(&format!("{first}{}", cells('a'))),
            row(&cells('b'))
        );
        let mut d = craft_docx::read(&package(&body)).unwrap();
        for round in ["as read", "after one save", "after two saves"] {
            let ranges = d.perm_ranges();
            let cells: Vec<_> = ranges.iter().filter_map(|r| range_cells(&d, r)).collect();
            if !cells.is_empty() {
                problems.push(format!("{name}, {round}: ranges {ranges:?} cover the cells {cells:?}, want none"));
            }
            if round != "after two saves" {
                d = craft_docx::read(&craft_docx::write(&d).unwrap()).unwrap();
            }
        }
    }
    assert!(problems.is_empty(), "{} problems:\n{}", problems.len(), problems.join("\n"));
}

/// A range limited to columns whose start is in a table nested in a cell that has no rows is
/// dropped, start and end: the reader keeps no such table, and in the outer table, where the start
/// would go, its columns would name that table's cells, which the file's range doesn't cover.
#[test]
fn a_column_range_started_in_a_table_with_no_rows_is_dropped() {
    let start = format!(r#"<w:permStart w:id="5" w:ed="{COLUMNS}" w:colFirst="0" w:colLast="1"/>"#);
    let empty = format!(r#"<w:tbl><w:tblPr/><w:tblGrid><w:gridCol w:w="1200"/></w:tblGrid>{start}</w:tbl>"#);
    let outer = table(&row(&format!(r#"<w:tc>{empty}<w:p/></w:tc><w:tc><w:permEnd w:id="5"/><w:p><w:r><w:t>other</w:t></w:r></w:p></w:tc>"#)));
    let mut d =
        craft_docx::read(&package(&format!(r#"<w:p><w:r><w:t>before</w:t></w:r></w:p>{outer}<w:p><w:r><w:t>after</w:t></w:r></w:p>"#))).unwrap();
    let mut problems = Vec::new();
    for round in ["as read", "after one save", "after two saves"] {
        let (ranges, markers) = (d.perm_ranges(), body_perm_markers(&d));
        if !ranges.is_empty() || !markers.is_empty() {
            problems.push(format!("{round}: ranges {ranges:?} and markers {markers:?}, want none"));
        }
        if round != "after two saves" {
            d = craft_docx::read(&craft_docx::write(&d).unwrap()).unwrap();
        }
    }
    assert!(problems.is_empty(), "{} problems:\n{}", problems.len(), problems.join("\n"));
}

/// The id `Document::next_perm_id` gives is one no permission marker in the glossary has: a table
/// split with it, in a document read with a glossary entry holding a permission range, gives the
/// new table's range an id of its own, and the saved body and glossary share none.
#[test]
fn splitting_a_table_takes_a_permission_id_the_glossary_doesnt_have() {
    let mut d = craft_docx::read(&package(&grid_body(3, "abcde", column_marks(r#" w:colFirst="1" w:colLast="1""#, 0, 3)))).unwrap();
    let mut entry = Paragraph::with_text("entry", CharProps::default());
    entry
        .insert_object(0, InlineObject::PermStart { id: 1, group: String::new(), editor: "glossary".into(), cols: None }, &CharProps::default())
        .unwrap();
    entry.insert_object(entry.len(), InlineObject::PermEnd { id: 1 }, &CharProps::default()).unwrap();
    d.glossary.push(craft_wordmodel::BuildingBlock {
        name: "entry".into(),
        gallery: "docParts".into(),
        category: "General".into(),
        blocks: vec![para_block(entry)],
        ..Default::default()
    });
    let mut d = craft_docx::read(&craft_docx::write(&d).unwrap()).unwrap();
    let ids_in = |blocks: &Blocks| {
        let mut ids = BTreeSet::new();
        for b in blocks {
            each_para(b, 0, &mut |p| {
                for o in &p.objects {
                    if let InlineObject::PermStart { id, .. } | InlineObject::PermEnd { id } = o {
                        ids.insert(*id);
                    }
                }
            });
        }
        ids
    };
    let glossary: BTreeSet<u32> = d.glossary.iter().flat_map(|e| ids_in(&e.blocks)).collect();
    assert!(!glossary.is_empty(), "the glossary entry has no permission range as read");
    let mut next = d.next_perm_id();
    let given = next;
    let tail = d.table_mut(StoryRef::Body, &Path::top(1)).unwrap().split_off(2, &mut next);
    d.body.insert(2, para_block(Paragraph::with_text("between", CharProps::default())));
    d.body.insert(3, Arc::new(Block::Table(tail)));
    assert_eq!(d.perm_ranges().len(), 2, "the split gives each table a range");
    assert!(!glossary.contains(&given), "next_perm_id gave {given}, which the glossary has: {glossary:?}");
    // Saved, the body's ids and the glossary's are apart.
    let saved = craft_docx::write(&d).unwrap();
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(saved.as_slice())).unwrap();
    let names: Vec<String> = z.file_names().filter_map(|n| n.ok().map(|n| n.to_string())).collect();
    let glossary_part =
        names.into_iter().find(|n| n.starts_with("word/glossary") && !n.contains("Styles") && !n.contains("Numbering") && n.ends_with(".xml"));
    let mut part_ids = |name: &str| {
        let mut xml = String::new();
        z.by_name(name).unwrap().read_to_string(&mut xml).unwrap();
        ["w:permStart", "w:permEnd"].iter().flat_map(|t| tags(&xml, t).map(|tag| attr(tag, "w:id")).collect::<Vec<_>>()).collect::<BTreeSet<String>>()
    };
    let (body, glossary) = (part_ids("word/document.xml"), part_ids(&glossary_part.expect("the saved package has a glossary part")));
    assert!(!glossary.is_empty() && body.is_disjoint(&glossary), "saved, the body's permission ids {body:?} and the glossary's {glossary:?} meet");
}

/// A range limited to columns that cover only the first part of a merged cell (`w:hMerge`, the
/// reader's one cell spanning both) covers none of it: in the file the merged cell isn't wholly
/// inside those columns, and the range doesn't take wider columns to reach it. As read and through
/// two saves it keeps its columns and covers no cell of that row.
#[test]
fn a_column_range_over_part_of_a_merged_cell_covers_none_of_it() {
    let merged = format!(
        r#"<w:tc><w:tcPr><w:hMerge w:val="restart"/></w:tcPr><w:p><w:r><w:t>first</w:t></w:r></w:p></w:tc><w:tc><w:tcPr><w:hMerge/></w:tcPr><w:p/></w:tc>{}"#,
        cell("next")
    );
    let marks = column_marks(r#" w:colFirst="0" w:colLast="0""#, 0, 0);
    let (start, end) = marks(0);
    let widths = r#"<w:gridCol w:w="1600"/>"#.repeat(3);
    let body = format!(
        r#"<w:p><w:r><w:t xml:space="preserve">before </w:t></w:r></w:p><w:tbl><w:tblPr/><w:tblGrid>{widths}</w:tblGrid>{}</w:tbl><w:p><w:r><w:t xml:space="preserve">after </w:t></w:r></w:p>"#,
        row(&format!("{start}{merged}{end}"))
    );
    let mut d = craft_docx::read(&package(&body)).unwrap();
    let mut problems = Vec::new();
    for round in ["as read", "after one save", "after two saves"] {
        let ranges = d.perm_ranges();
        let got: Vec<_> = ranges.iter().map(|r| (r.cols, range_cells(&d, r))).collect();
        let want = [(Some([0, 0]), Some((Path::top(1), BTreeSet::new())))];
        if got != want {
            problems.push(format!("{round}: ranges with the columns and cells {got:?}, want {want:?}"));
        }
        if round != "after two saves" {
            d = craft_docx::read(&craft_docx::write(&d).unwrap()).unwrap();
        }
    }
    assert!(problems.is_empty(), "{} problems:\n{}", problems.len(), problems.join("\n"));
}
