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

use std::collections::BTreeMap;
use std::io::Read;

use craft_wordmodel::para::OBJ;
use craft_wordmodel::{Block, Blocks, Document, InlineObject};

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
    ("before a cell that starts with a table", true, |m| {
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
                    problems.push(format!("{name}, {round}: the {kind} range {range:?} covers {got:?}, the file {words:?}"));
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

const COLUMN_CASES: [ColumnCase; 6] = [
    ("one column, the start in the row", StartAt::Row, r#" w:colFirst="1" w:colLast="1""#, Some([1, 1])),
    ("two columns, the start in the first cell", StartAt::Cell, r#" w:colFirst="0" w:colLast="1""#, Some([0, 1])),
    ("no columns", StartAt::Row, "", None),
    // The spec says to ignore such a pair; it is saved as the file has it.
    ("the first column after the last", StartAt::Row, r#" w:colFirst="2" w:colLast="1""#, Some([2, 1])),
    // Non-conformant without its pair: it still limits the range to that column.
    ("colFirst alone", StartAt::Cell, r#" w:colFirst="2""#, Some([2, 2])),
    ("values that are not column numbers", StartAt::Row, r#" w:colFirst="two" w:colLast="-1""#, None),
];

/// The tags named `name` in `xml`, without their angle brackets.
fn tags<'a>(xml: &'a str, name: &'a str) -> impl Iterator<Item = &'a str> {
    xml.split('<')
        .skip(1)
        .map(|piece| piece.split('>').next().unwrap_or(""))
        .filter(move |tag| tag.split(|c: char| c.is_whitespace() || c == '/').next() == Some(name))
}

/// A permission range limited to some of a table's columns keeps them, and its rows, as read and
/// through two saves; a range without them gains none, and stays a range over whole rows.
#[test]
fn a_permission_range_keeps_its_table_columns_and_gains_none_through_two_saves() {
    let mut problems = Vec::new();
    for (name, at, attrs, want) in &COLUMN_CASES {
        let body = column_body(*at, attrs);
        let words = covered(&file_tokens(&body), PERM, COLUMNS);
        assert!(!words.is_empty(), "{name}: the fixture marks no text");
        let saved_cols = want.map_or([String::new(), String::new()], |c| c.map(|v| v.to_string()));
        let mut d = craft_docx::read(&package(&body)).unwrap();
        for (round, save) in [("as read", Some("first save")), ("after one save", Some("second save")), ("after two saves", None)] {
            let cols: Vec<Option<[u32; 2]>> = d.perm_ranges().iter().map(|r| r.cols).collect();
            if cols != [*want] {
                problems.push(format!("{name}, {round}: ranges with the columns {cols:?}, want [{want:?}]"));
            }
            let mut tokens = Vec::new();
            read_tokens(&d, &d.body, &mut BTreeMap::new(), &mut tokens);
            let got = covered(&tokens, PERM, COLUMNS);
            if got != words {
                problems.push(format!("{name}, {round}: the range covers {got:?}, the file {words:?}"));
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
