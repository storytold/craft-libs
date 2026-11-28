//! Tests of the writer: new packages read back by the reader.
//!
//! 본 제품은 한글과컴퓨터의 한글 문서 파일(.hwp) 공개 문서를 참고하여 개발하였습니다.

use crate::xml;

use super::SaveReport;

const TEST_ZIP: craft_zip::Limits = craft_zip::Limits { max_entries: 65_536, max_total_bytes: 1 << 30 };

/// A new package of `doc`, and what the writer left out.
fn new_hwpx(doc: &craft_wordmodel::Document) -> Result<(Vec<u8>, SaveReport), crate::HwpxError> {
    super::new::from_document(doc, None)
}

/// A part of a package, as text.
pub(crate) fn zip_part(bytes: &[u8], name: &str) -> String {
    let mut z = craft_zip::Archive::open(bytes, TEST_ZIP).unwrap();
    String::from_utf8(z.read(name, u64::MAX).unwrap().unwrap()).unwrap()
}

/// A document that never was an HWPX becomes a package the reader (and Hancom's viewer) opens:
/// text, character shapes, paragraph shapes and page breaks come back.
#[test]
fn new_hwpx_holds_the_document() {
    let mut doc = craft_wordmodel::Document::from_text("첫 문단 & <기호>\n둘째 문단");
    if let Some(craft_wordmodel::Block::Para(p)) = doc.body.first_mut().map(std::sync::Arc::make_mut) {
        for r in &mut p.runs {
            r.props.bold = Some(true);
            r.props.size = Some(20.0);
        }
        p.props.align = Some(craft_wordmodel::Align::Center);
        p.text.push(craft_wordmodel::para::PAGE_BREAK);
        if let Some(r) = p.runs.last_mut() {
            r.len += craft_wordmodel::para::PAGE_BREAK.len_utf8();
        }
    }
    let (bytes, report) = new_hwpx(&doc).unwrap();
    assert!(report.skipped.is_empty(), "{:?}", report.skipped);
    let mut z = craft_zip::Archive::open(&bytes, TEST_ZIP).unwrap();
    assert_eq!(z.entry_name(0).unwrap().as_deref(), Some("mimetype"));
    assert_eq!(z.is_stored("mimetype"), Some(true));
    let back = crate::read(&bytes).unwrap();
    let paras: Vec<&craft_wordmodel::Paragraph> = back.body.iter().filter_map(|b| b.as_para()).collect();
    let first = paras.iter().find(|p| p.text.contains("첫 문단")).expect("the first paragraph");
    assert!(first.text.contains("첫 문단 & <기호>"));
    assert_eq!(first.props.align, Some(craft_wordmodel::Align::Center));
    assert!(first.runs.iter().filter(|r| r.len > 0).all(|r| r.props.bold == Some(true) && r.props.size == Some(20.0)), "{:?}", first.runs);
    let at = paras.iter().position(|p| p.text.contains("둘째 문단")).expect("the second paragraph");
    // HWPX breaks a page before a paragraph: the break ends the first paragraph's text and
    // starts the page of what follows
    assert!(paras.get(1..=at).unwrap_or(&[]).iter().any(|p| p.props.page_break_before == Some(true)), "no page break before the second paragraph");
}

/// A run naming no East Asian font is written with the font the caller gives for Hangul beside
/// its own, in the Hangul slot of its character shape; without one, with its own font.
#[test]
fn new_hwpx_hangul_takes_the_callers_font() {
    let hangul_face = |hangul_font: crate::write::HangulFont| {
        let doc = craft_wordmodel::Document::from_text("한글 문단");
        let (bytes, _) = super::new::from_document(&doc, hangul_font).unwrap();
        let head = xml::parse(zip_part(&bytes, "Contents/header.xml").as_bytes(), false).unwrap();
        let sec = xml::parse(zip_part(&bytes, "Contents/section0.xml").as_bytes(), true).unwrap();
        // the Hangul text's run -> its character shape -> the fonts it names
        let run = sec.iter("hp:run").find(|r| r.iter("hp:t").any(|t| t.text.contains('한'))).expect("the Hangul run");
        let shape = run.get("charPrIDRef").expect("a character shape");
        let char_pr = head.iter("hh:charPr").find(|c| c.get("id") == Some(shape)).expect("the character shape");
        let font_ref = char_pr.find("hh:fontRef").expect("its fonts");
        let face = |lang: &str, slot: &str| {
            let id = font_ref.get(slot).expect("a font");
            let list = head.iter("hh:fontface").find(|f| f.get("lang") == Some(lang)).expect("the fonts");
            list.iter("hh:font").find(|f| f.get("id") == Some(id)).and_then(|f| f.get("face")).expect("the font").to_string()
        };
        (face("HANGUL", "hangul"), face("LATIN", "latin"))
    };
    let (hangul, latin) = hangul_face(Some(|_| Some("Hangul Stand-in".into())));
    assert_eq!(hangul, "Hangul Stand-in");
    assert_ne!(latin, hangul, "only the Hangul slot takes it");
    let (hangul, latin) = hangul_face(None);
    assert_eq!(hangul, latin);
}

/// Text boxes that keep naming the same stories (each level two boxes of the next story, 20
/// levels deep: about a million expansions within the depth limit) are written within a budget,
/// the rest reported. Before, every expansion was written again.
#[test]
fn new_hwpx_text_boxes_naming_the_same_stories_are_bounded() {
    use craft_wordmodel::{CharProps, InlineObject, Paragraph, PartKind, para::ShapeKind, para_block};
    let boxes = |story: Option<u32>| {
        let mut p = Paragraph::with_text("x", CharProps::default());
        if let Some(story) = story {
            for _ in 0..2 {
                let shape = InlineObject::Shape {
                    kind: ShapeKind::TextBox,
                    w: 50.0,
                    h: 20.0,
                    fill: None,
                    stroke: None,
                    stroke_width: 0.0,
                    float: Default::default(),
                    story: Some(story),
                    freeform: None,
                    effects: Default::default(),
                    extra: Default::default(),
                };
                p.insert_object(0, shape, &CharProps::default()).unwrap();
            }
        }
        vec![para_block(p)]
    };
    let mut doc = craft_wordmodel::Document::from_text("body");
    let mut next = None;
    for _ in 0..20 {
        let id = doc.add_part(PartKind::TextBox, Vec::new());
        if let Some(part) = doc.parts.get_mut(&id) {
            part.blocks = boxes(next);
        }
        next = Some(id);
    }
    doc.body.extend(boxes(next));
    let started = std::time::Instant::now();
    let (_, report) = new_hwpx(&doc).unwrap();
    assert!(report.skipped.iter().any(|s| s.contains("past the most one file writes")), "{report:?}");
    assert!(started.elapsed().as_secs() < 60, "bounded: {:?}", started.elapsed());
}

/// A first-page header with content and no default header (HWPX has no first-page header of its
/// own) is reported as not written. Before, nothing was written and nothing reported.
#[test]
fn new_hwpx_reports_a_first_page_header_alone() {
    use craft_wordmodel::{CharProps, Paragraph, PartKind, para_block};
    let mut doc = craft_wordmodel::Document::from_text("body");
    let first = doc.add_part(PartKind::Header, vec![para_block(Paragraph::with_text("first page only", CharProps::default()))]);
    doc.last_section.title_page = true;
    doc.last_section.headers.first = Some(first);
    doc.last_section.headers.default = None;
    let (_, report) = new_hwpx(&doc).unwrap();
    assert!(report.skipped.iter().any(|s| s.contains("first-page headers")), "{report:?}");
}

/// A table of 10,001 rows (the document model allows 32,767) written to a new HWPX reads back
/// whole. Before, reading stopped at 10,000 rows and the last row's text was gone.
#[test]
fn new_hwpx_long_tables_read_back_whole() {
    let mut doc = craft_wordmodel::Document::from_text("before");
    let mut t = craft_wordmodel::Table::new(10_001, 1, 200.0);
    if let Some(cell) = t.rows.last_mut().and_then(|r| r.cells.first_mut()) {
        cell.blocks = vec![craft_wordmodel::para_block(craft_wordmodel::Paragraph::with_text("last row", craft_wordmodel::CharProps::default()))];
    }
    doc.body.push(std::sync::Arc::new(craft_wordmodel::Block::Table(t)));
    let (bytes, report) = new_hwpx(&doc).unwrap();
    assert!(report.skipped.is_empty(), "{report:?}");
    let back = crate::read(&bytes).unwrap();
    let t = back.body.iter().find_map(|b| b.as_table()).expect("the table");
    assert_eq!(t.rows.len(), 10_001);
    let last = t.rows.last().and_then(|r| r.cells.first()).and_then(|c| c.blocks.first()).and_then(|b| b.as_para());
    assert_eq!(last.map(|p| p.text.as_str()), Some("last row"));
}

/// Tables nested past the writer's depth limit are left out and reported. Before, they and
/// their cells' text went without a notice.
#[test]
fn new_hwpx_reports_tables_nested_too_deep() {
    use craft_wordmodel::{Block, Table};
    let mut table = Table::new(1, 1, 100.0);
    for _ in 0..40 {
        let mut outer = Table::new(1, 1, 100.0);
        if let Some(cell) = outer.rows.first_mut().and_then(|r| r.cells.first_mut()) {
            cell.blocks = vec![std::sync::Arc::new(Block::Table(table))];
        }
        table = outer;
    }
    let mut doc = craft_wordmodel::Document::from_text("before");
    doc.body.push(std::sync::Arc::new(Block::Table(table)));
    let (_, report) = new_hwpx(&doc).unwrap();
    assert!(report.skipped.iter().any(|s| s.contains("tables nested too deep")), "{report:?}");
}

/// A freeform of two paths inside groups as deep as the writer goes (where no group is left to
/// hold its paths apart) writes its first path and reports the other. Before, the second path
/// went without a notice.
#[test]
fn new_hwpx_reports_freeform_paths_at_the_group_limit() {
    use craft_wordmodel::freeform::{FreePath, Freeform};
    use craft_wordmodel::para::ShapeKind;
    use craft_wordmodel::{CharProps, InlineObject, Paragraph, para_block};
    let path = |x: f32| FreePath { pts: vec![[x, 0.0], [x + 50.0, 50.0]], closed: false };
    let ff = Freeform { w: 100.0, h: 100.0, paths: vec![path(0.0), path(20.0)], ..Default::default() };
    let mut obj = InlineObject::Shape {
        kind: ShapeKind::Freeform,
        w: 50.0,
        h: 50.0,
        fill: None,
        stroke: Some(craft_wordmodel::props::Rgb(0, 0, 0)),
        stroke_width: 1.0,
        float: Default::default(),
        story: None,
        freeform: Some(std::sync::Arc::new(ff)),
        effects: Default::default(),
        extra: Default::default(),
    };
    for _ in 0..8 {
        obj = InlineObject::Group {
            w: 50.0,
            h: 50.0,
            float: Default::default(),
            ch_w: 50.0,
            ch_h: 50.0,
            children: vec![craft_wordmodel::para::GroupChild { x: 0.0, y: 0.0, obj }],
            canvas: None,
        };
    }
    let mut p = Paragraph::with_text("x", CharProps::default());
    p.insert_object(0, obj, &CharProps::default()).unwrap();
    let mut doc = craft_wordmodel::Document::from_text("");
    doc.body.clear();
    doc.body.push(para_block(p));
    let (_, report) = new_hwpx(&doc).unwrap();
    assert!(report.skipped.iter().any(|s| s.contains("freeform paths past the first")), "{report:?}");
}

/// A text box whose story holds a text box showing that same story: written once, the inner
/// one left out and reported. Before, each story started its depth over and the writing
/// recursed until the stack overflowed.
#[test]
fn new_hwpx_a_text_box_holding_itself_is_left_out() {
    use craft_wordmodel::{CharProps, InlineObject, Paragraph, PartKind, para::ShapeKind, para_block};
    let box_para = |story: u32| {
        let shape = InlineObject::Shape {
            kind: ShapeKind::TextBox,
            w: 50.0,
            h: 20.0,
            fill: None,
            stroke: None,
            stroke_width: 0.0,
            float: Default::default(),
            story: Some(story),
            freeform: None,
            effects: Default::default(),
            extra: Default::default(),
        };
        let mut p = Paragraph::with_text("상자 글", CharProps::default());
        p.insert_object(0, shape, &CharProps::default()).unwrap();
        vec![para_block(p)]
    };
    let mut doc = craft_wordmodel::Document::from_text("본문");
    let id = doc.add_part(PartKind::TextBox, Vec::new());
    if let Some(part) = doc.parts.get_mut(&id) {
        part.blocks = box_para(id);
    }
    doc.body.extend(box_para(id));
    let (bytes, report) = new_hwpx(&doc).unwrap();
    assert!(zip_part(&bytes, "Contents/section0.xml").contains("상자 글"));
    assert!(report.skipped.iter().any(|s| s.contains("nested in themselves")), "{report:?}");
}

/// Numbered and bulleted lists become HWP paragraph headings, and paragraph styles keep their
/// names; both read back.
#[test]
fn new_hwpx_writes_lists_and_styles() {
    use craft_wordmodel::props::NumRef;
    let mut doc = craft_wordmodel::Document::from_text("제목\n하나\n둘\n점");
    let num = std::sync::Arc::make_mut(&mut doc.numbering).add_list(craft_wordmodel::ListKind::Numbered);
    let bul = std::sync::Arc::make_mut(&mut doc.numbering).add_list(craft_wordmodel::ListKind::Bullet);
    let props = [(Some("Heading1"), None), (None, Some((num, 0))), (None, Some((num, 1))), (None, Some((bul, 0)))];
    for (b, (style, list)) in doc.body.iter_mut().zip(props) {
        if let craft_wordmodel::Block::Para(p) = std::sync::Arc::make_mut(b) {
            p.props.style = style.map(String::from);
            p.props.numbering = list.map(|(num, level)| NumRef { num, level });
        }
    }
    let (bytes, report) = new_hwpx(&doc).unwrap();
    assert!(report.skipped.is_empty(), "{:?}", report.skipped);
    let z = bytes.clone();
    let head = zip_part(&z, "Contents/header.xml");
    assert!(head.contains(r#"<hh:heading type="NUMBER" idRef="1" level="0"/>"#), "{head}");
    assert!(head.contains(r#"<hh:heading type="NUMBER" idRef="1" level="1"/>"#), "{head}");
    assert!(head.contains(r#"<hh:heading type="BULLET" idRef="1" level="0"/>"#), "{head}");
    assert!(head.contains(r#"numFormat="LATIN_SMALL" charPrIDRef="4294967295" checkable="0">^2.</hh:paraHead>"#), "{head}");
    assert!(head.contains(r#"<hh:bullet id="1" char="•""#), "{head}");
    assert!(head.contains(r#"name="heading 1""#) || head.contains(r#"name="Heading 1""#), "{head}");
    let back = crate::read(&bytes).unwrap();
    let listed = back.body.iter().filter_map(|b| b.as_para()).filter(|p| p.props.numbering.is_some()).count();
    assert_eq!(listed, 3);
}

/// Tables become HWPX tables: size, merged cells, cell borders and fill, and they read back.
#[test]
fn new_hwpx_writes_tables() {
    let mut doc = craft_wordmodel::Document::from_text("표 앞");
    let mut t = craft_wordmodel::Table::new(2, 3, 300.0);
    if let Some(row) = t.rows.first_mut() {
        row.cells.truncate(2);
        if let Some(c) = row.cells.first_mut() {
            c.props.span = 2;
            c.props.shading = Some(craft_wordmodel::Rgb(0xEE, 0xCC, 0xAA));
        }
    }
    doc.body.insert(1, std::sync::Arc::new(craft_wordmodel::Block::Table(t)));
    doc.body.push(std::sync::Arc::new(craft_wordmodel::Block::Para(craft_wordmodel::Paragraph::default())));
    let (bytes, report) = new_hwpx(&doc).unwrap();
    assert!(report.skipped.is_empty(), "{:?}", report.skipped);
    let z = bytes.clone();
    let (sec, head) = (zip_part(&z, "Contents/section0.xml"), zip_part(&z, "Contents/header.xml"));
    assert!(sec.contains(r#"rowCnt="2" colCnt="3""#), "{sec}");
    assert!(sec.contains(r#"<hp:cellSpan colSpan="2" rowSpan="1"/>"#), "{sec}");
    assert_eq!(sec.matches("<hp:tc ").count(), 5);
    assert!(head.contains(r##"faceColor="#EECCAA""##), "{head}");
    let back = crate::read(&bytes).unwrap();
    let t = back.body.iter().find_map(|b| b.as_table()).expect("the table");
    let first = t.rows.first().and_then(|r| r.cells.first()).expect("a cell");
    assert_eq!(first.props.span, 2);
    assert_eq!(first.props.shading, Some(craft_wordmodel::Rgb(0xEE, 0xCC, 0xAA)));
}

/// A continued cell under a cell that doesn't start a merge (a row inserted inside a merge)
/// joins that cell instead of leaving a hole in the table.
#[test]
fn new_hwpx_joins_a_continued_cell_to_the_one_above() {
    let mut doc = craft_wordmodel::Document::from_text("표");
    let mut t = craft_wordmodel::Table::new(3, 2, 200.0);
    if let Some(c) = t.rows.first_mut().and_then(|r| r.cells.first_mut()) {
        c.props.vmerge = craft_wordmodel::props::VMerge::Restart;
    }
    if let Some(c) = t.rows.get_mut(2).and_then(|r| r.cells.first_mut()) {
        c.props.vmerge = craft_wordmodel::props::VMerge::Continue;
    }
    doc.body.insert(1, std::sync::Arc::new(craft_wordmodel::Block::Table(t)));
    doc.body.push(std::sync::Arc::new(craft_wordmodel::Block::Para(craft_wordmodel::Paragraph::default())));
    let (bytes, _) = new_hwpx(&doc).unwrap();
    let z = bytes;
    let sec = zip_part(&z, "Contents/section0.xml");
    assert_eq!(sec.matches("<hp:tc ").count(), 5, "{sec}");
    assert!(sec.contains(r#"<hp:cellAddr colAddr="0" rowAddr="1"/><hp:cellSpan colSpan="1" rowSpan="2"/>"#), "{sec}");
}

/// A drawing inline in the story of a text box that is a group member is placed in the story's
/// text, with its own size and position, not as another member of the group. Before, it took
/// the box's member state and lost them.
#[test]
fn new_hwpx_text_box_story_in_a_group_places_its_own_drawings() {
    use craft_wordmodel::{CharProps, InlineObject, Paragraph, PartKind, para::ShapeKind, para_block};
    let shape = |kind: ShapeKind, story: Option<u32>| InlineObject::Shape {
        kind,
        w: 20.0,
        h: 10.0,
        fill: None,
        stroke: Some(craft_wordmodel::props::Rgb(0, 0, 0)),
        stroke_width: 1.0,
        float: Default::default(),
        story,
        freeform: None,
        effects: Default::default(),
        extra: Default::default(),
    };
    let mut doc = craft_wordmodel::Document::from_text("");
    let mut inner = Paragraph::with_text("상자", CharProps::default());
    inner.insert_object(0, shape(ShapeKind::Rectangle, None), &CharProps::default()).unwrap();
    let id = doc.add_part(PartKind::TextBox, vec![para_block(inner)]);
    // shown at twice its own coordinates
    let group = InlineObject::Group {
        w: 100.0,
        h: 100.0,
        float: Default::default(),
        ch_w: 50.0,
        ch_h: 50.0,
        children: vec![craft_wordmodel::para::GroupChild { x: 0.0, y: 0.0, obj: shape(ShapeKind::TextBox, Some(id)) }],
        canvas: None,
    };
    let mut p = Paragraph::with_text("x", CharProps::default());
    p.insert_object(0, group, &CharProps::default()).unwrap();
    doc.body.clear();
    doc.body.push(para_block(p));
    let (bytes, _) = new_hwpx(&doc).unwrap();
    let sec = zip_part(&bytes, "Contents/section0.xml");
    let text = &sec[sec.find("<hp:drawText").unwrap()..sec.find("</hp:drawText>").unwrap()];
    assert!(text.contains("<hp:sz ") && text.contains("<hp:pos "), "the story's drawing has its own size and place: {text}");
}

/// A hyperlink in a document saved as a new .hwpx keeps its text, and the save reports that its
/// destination is not written. Before, the destination went without a notice.
#[test]
fn new_hwpx_reports_hyperlink_destinations() {
    use craft_wordmodel::{CharProps, Paragraph, para_block};
    let linked = CharProps { link: Some("https://example.com/".into()), ..CharProps::default() };
    let mut doc = craft_wordmodel::Document::from_text("");
    doc.body.clear();
    doc.body.push(para_block(Paragraph::with_text("연결된 글", linked)));
    let (bytes, report) = new_hwpx(&doc).unwrap();
    assert!(zip_part(&bytes, "Contents/section0.xml").contains("연결된 글"));
    assert!(report.skipped.iter().any(|s| s.contains("hyperlink destinations")), "{report:?}");
}

/// A table row without a height of its own is written with the cell as tall as its margins: its
/// text's estimated height goes only into the table's size. Written as the cell's height it came
/// back as the row's height on reopening, so every save and reopen made such rows taller.
#[test]
fn table_rows_without_a_height_stay_without_one() {
    let mut doc = craft_wordmodel::Document::from_text("표 앞");
    let mut t = craft_wordmodel::Table::new(1, 1, 200.0);
    if let Some(c) = t.rows.first_mut().and_then(|r| r.cells.first_mut()) {
        c.blocks = craft_wordmodel::Document::from_text(&["여러 줄의 칸 글"; 12].join("\n")).body;
    }
    doc.body.insert(1, std::sync::Arc::new(craft_wordmodel::Block::Table(t)));
    doc.body.push(std::sync::Arc::new(craft_wordmodel::Block::Para(craft_wordmodel::Paragraph::default())));
    let (bytes, _) = new_hwpx(&doc).unwrap();
    let sec = xml::parse(zip_part(&bytes, "Contents/section0.xml").as_bytes(), false).unwrap();
    let height = |tag: &str| -> i64 { sec.iter(tag).next().and_then(|e| e.get("height")).and_then(|h| h.parse().ok()).expect(tag) };
    let (cell, size) = (height("hp:cellSz"), height("hp:sz"));
    assert!(cell < size / 12, "the cell is its margins tall, not the text's estimate: {cell} of {size}");
    let back = crate::read(&bytes).unwrap();
    let t = back.body.iter().find_map(|b| b.as_table()).expect("the table");
    assert!(t.rows.iter().all(|r| r.props.height.is_none()), "no row height on reopening");
}

/// What a save leaves out is logged by the public writer (a new package naming a hyperlink's
/// text but not its destination).
#[test]
fn what_isnt_written_is_logged() {
    use craft_wordmodel::{CharProps, Paragraph};
    let linked = CharProps { link: Some("https://example.com/".into()), ..CharProps::default() };
    let mut doc = craft_wordmodel::Document::from_text("");
    doc.body = vec![std::sync::Arc::new(craft_wordmodel::Block::Para(Paragraph::with_text("연결된 글", linked)))];
    let (bytes, warnings) = crate::test_log::warnings(|| crate::write(&doc));
    assert!(bytes.is_ok());
    assert!(warnings.iter().any(|w| w.contains("hyperlink destinations")), "{warnings:?}");
}

/// A turned object is written as Hancom's viewer turns it (2026-11-25): hp:sz is the box around
/// the turned object, placed where it lies, hp:curSz the object's own size, and the drawing
/// matrices turn it. Before, the matrices were left unturned and the viewer drew it upright.
#[test]
fn new_hwpx_writes_a_turned_object_in_the_box_around_it() {
    use craft_wordmodel::{
        CharProps, InlineObject, Paragraph,
        para::{Anchor, Float, ShapeKind, Wrap},
        para_block,
    };
    let mut float = Float { wrap: Wrap::InFrontOfText, h_rel: Anchor::Column, v_rel: Anchor::Paragraph, x: 10.0, y: 50.0, ..Float::default() };
    float.set_spin(craft_wordmodel::geom::Spin::new(90.0, false, false));
    let shape = InlineObject::Shape {
        kind: ShapeKind::Rectangle,
        w: 100.0,
        h: 40.0,
        fill: None,
        stroke: Some(craft_wordmodel::props::Rgb(0, 0, 0)),
        stroke_width: 1.0,
        float,
        story: None,
        freeform: None,
        effects: Default::default(),
        extra: Default::default(),
    };
    let mut doc = craft_wordmodel::Document::from_text("");
    let mut p = Paragraph::with_text("x", CharProps::default());
    p.insert_object(0, shape, &CharProps::default()).unwrap();
    doc.body = vec![para_block(p)];
    let (bytes, _) = new_hwpx(&doc).unwrap();
    let sec = xml::parse(zip_part(&bytes, "Contents/section0.xml").as_bytes(), false).unwrap();
    let rect = sec.iter("hp:rect").next().expect("the shape");
    let size = |tag: &str| -> (i64, i64) {
        let e = rect.find(tag).expect(tag);
        (e.get("width").unwrap().parse().unwrap(), e.get("height").unwrap().parse().unwrap())
    };
    let ((fw, fh), (cw, ch)) = (size("hp:sz"), size("hp:curSz"));
    assert!(fw < fh && cw > ch, "frame upright, own size wide: {:?}", ((fw, fh), (cw, ch)));
    let rot = rect.find("hp:renderingInfo").and_then(|r| r.find("hc:rotMatrix")).expect("rotMatrix");
    assert_ne!(rot.get("e1"), Some("1"), "the matrix turns it");
    let pos = rect.find("hp:pos").unwrap();
    let off = |k: &str| -> i64 { pos.get(k).unwrap().parse().unwrap() };
    // the frame's centre is the object's centre
    assert!((off("horzOffset") + fw / 2 - (1000 + cw / 2)).abs() <= 1, "{}", off("horzOffset"));
    assert!((off("vertOffset") + fh / 2 - (5000 + ch / 2)).abs() <= 1, "{}", off("vertOffset"));
}

/// Offsets left of or above their anchor are written as Hancom writes them, unsigned 32-bit (its
/// viewer reads a minus sign as 0: 2026-11-25), and read back where they were; one above its
/// paragraph isn't held in the body (held, the viewer draws it at the paragraph's top).
#[test]
fn new_hwpx_writes_offsets_before_their_anchor_as_hancom_does() {
    use craft_wordmodel::{
        CharProps, InlineObject, Paragraph,
        para::{Anchor, Float, ShapeKind, Wrap},
        para_block,
    };
    let float = Float { wrap: Wrap::InFrontOfText, h_rel: Anchor::Column, v_rel: Anchor::Paragraph, x: -20.0, y: -30.0, ..Float::default() };
    let shape = InlineObject::Shape {
        kind: ShapeKind::Rectangle,
        w: 40.0,
        h: 20.0,
        fill: None,
        stroke: Some(craft_wordmodel::props::Rgb(0, 0, 0)),
        stroke_width: 1.0,
        float,
        story: None,
        freeform: None,
        effects: Default::default(),
        extra: Default::default(),
    };
    let mut doc = craft_wordmodel::Document::from_text("");
    let mut p = Paragraph::with_text("x", CharProps::default());
    p.insert_object(0, shape, &CharProps::default()).unwrap();
    doc.body = vec![para_block(p)];
    let (bytes, _) = new_hwpx(&doc).unwrap();
    let sec = xml::parse(zip_part(&bytes, "Contents/section0.xml").as_bytes(), false).unwrap();
    let pos = sec.iter("hp:pos").next().expect("the shape's place");
    // -20 pt and -30 pt in HWPUNIT, as 32-bit two's complement
    assert_eq!(pos.get("horzOffset"), Some((-2000i32 as u32).to_string().as_str()));
    assert_eq!(pos.get("vertOffset"), Some((-3000i32 as u32).to_string().as_str()));
    assert_eq!(pos.get("flowWithText"), Some("0"), "not held at the paragraph's top");
    let back = crate::read(&bytes).unwrap();
    let f = back.body.iter().filter_map(|b| b.as_para()).flat_map(|p| p.objects.iter()).find_map(|o| match o {
        InlineObject::Shape { float, .. } => Some(float.clone()),
        _ => None,
    });
    assert!(f.as_ref().is_some_and(|f| (f.x + 20.0).abs() < 0.01 && (f.y + 30.0).abs() < 0.01), "read back where it was: {f:?}");
}

/// Characters XML 1.0 has no place for (U+0000 in a title, other C0 controls in text) are left
/// out of the parts. Before, a NUL in the title went into `content.hpf`, which then didn't parse.
#[test]
fn new_hwpx_leaves_out_characters_xml_cannot_hold() {
    let mut doc = craft_wordmodel::Document::from_text("가\u{1}나");
    doc.core.title = "제목\u{0}".into();
    let (bytes, _) = new_hwpx(&doc).unwrap();
    for part in ["Contents/content.hpf", "Contents/section0.xml"] {
        let xml = zip_part(&bytes, part);
        assert!(!xml.chars().any(|c| c < ' ' && !matches!(c, '\t' | '\n' | '\r')), "{part}");
        assert!(xml::parse(xml.as_bytes(), false).is_ok(), "{part} parses");
    }
    assert!(crate::read(&bytes).unwrap().core.title.starts_with("제목"));
}

/// A paragraph that doesn't say whether Hangul words wrap whole (one from another format, or a
/// new one) is written breaking them between syllables, as the model has it and an app lays it
/// out; one that says so keeps what it says. Before, unset was written as wrapping whole, and
/// the document's lines broke otherwise in Hancom than where it was edited.
#[test]
fn new_hwpx_writes_hangul_word_wrapping_as_the_model_has_it() {
    use craft_wordmodel::{CharProps, Paragraph, para_block};
    let para = |text: &str, keep: Option<bool>| {
        let mut p = Paragraph::with_text(text, CharProps::default());
        p.props.keep_hangul_words = keep;
        para_block(p)
    };
    let mut doc = craft_wordmodel::Document::from_text("");
    doc.body = vec![para("비어 있음", None), para("어절", Some(true)), para("글자", Some(false))];
    let (bytes, _) = new_hwpx(&doc).unwrap();
    let back = crate::read(&bytes).unwrap();
    let keep: Vec<Option<bool>> = back.body.iter().filter_map(|b| b.as_para()).map(|p| p.props.keep_hangul_words).collect();
    assert_eq!(keep, [Some(false), Some(true), Some(false)]);
}

/// A mirrored picture is written mirrored by its drawing matrices as well as its hp:flip, as
/// Hancom's viewer mirrors it only by the matrices (2026-11-25): the scale matrix is negative on
/// that axis and the translation moves it back by its width. Before, only hp:flip said so.
#[test]
fn new_hwpx_mirrors_a_picture_in_its_matrices() {
    use craft_wordmodel::{
        CharProps, InlineObject, Paragraph,
        para::{Float, Wrap},
        para_block,
    };
    let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
    png.extend(1u32.to_be_bytes());
    png.extend(1u32.to_be_bytes());
    png.extend([8, 6, 0, 0, 0]);
    let mut float = Float { wrap: Wrap::InFrontOfText, ..Float::default() };
    float.set_spin(craft_wordmodel::geom::Spin::new(0.0, true, false));
    let pic =
        InlineObject::Image { media: "p.png".into(), w: 40.0, h: 20.0, alt: String::new(), float, crop: [0.0; 4], fx: Default::default(), ole: None };
    let mut doc = craft_wordmodel::Document::from_text("");
    doc.media.insert("p.png".into(), std::sync::Arc::new(png));
    let mut p = Paragraph::with_text("x", CharProps::default());
    p.insert_object(0, pic, &CharProps::default()).unwrap();
    doc.body = vec![para_block(p)];
    let (bytes, _) = new_hwpx(&doc).unwrap();
    let sec = xml::parse(zip_part(&bytes, "Contents/section0.xml").as_bytes(), false).unwrap();
    let pic = sec.iter("hp:pic").next().expect("the picture");
    assert_eq!(pic.find("hp:flip").and_then(|f| f.get("horizontal")), Some("1"));
    let ri = pic.find("hp:renderingInfo").expect("renderingInfo");
    let num = |m: &str, k: &str| -> f64 { ri.find(m).and_then(|e| e.get(k)).and_then(|v| v.parse().ok()).unwrap_or(f64::NAN) };
    assert!(num("hc:scaMatrix", "e1") < 0.0, "scaled by -1 across");
    assert!(num("hc:scaMatrix", "e5") > 0.0, "not down");
    assert_eq!(num("hc:transMatrix", "e3"), 4000.0, "moved back by its width");
    let back = crate::read(&bytes).unwrap();
    let flipped = back.body.iter().filter_map(|b| b.as_para()).flat_map(|p| p.objects.iter()).find_map(|o| match o {
        InlineObject::Image { float, .. } => Some(float.flip_h),
        _ => None,
    });
    assert!(flipped.is_some(), "read back");
}

/// A turned group is written upright and reported. Written turned (as a picture is), Hancom's
/// viewer turned its frame but stretched its members into it unturned (2026-11-25).
#[test]
fn new_hwpx_writes_a_turned_group_upright_and_says_so() {
    use craft_wordmodel::{
        CharProps, InlineObject, Paragraph,
        para::{Float, GroupChild, ShapeKind, Wrap},
        para_block,
    };
    let rect = InlineObject::Shape {
        kind: ShapeKind::Rectangle,
        w: 40.0,
        h: 20.0,
        fill: None,
        stroke: Some(craft_wordmodel::props::Rgb(0, 0, 0)),
        stroke_width: 1.0,
        float: Float::default(),
        story: None,
        freeform: None,
        effects: Default::default(),
        extra: Default::default(),
    };
    let mut float = Float { wrap: Wrap::InFrontOfText, ..Float::default() };
    float.set_spin(craft_wordmodel::geom::Spin::new(90.0, false, false));
    let group = InlineObject::Group {
        w: 80.0,
        h: 20.0,
        float,
        ch_w: 80.0,
        ch_h: 20.0,
        children: vec![GroupChild { x: 0.0, y: 0.0, obj: rect.clone() }, GroupChild { x: 40.0, y: 0.0, obj: rect }],
        canvas: None,
    };
    let mut doc = craft_wordmodel::Document::from_text("");
    let mut p = Paragraph::with_text("x", CharProps::default());
    p.insert_object(0, group, &CharProps::default()).unwrap();
    doc.body = vec![para_block(p)];
    let (bytes, report) = new_hwpx(&doc).unwrap();
    assert!(report.skipped.iter().any(|s| s.contains("turned groups")), "{report:?}");
    let sec = xml::parse(zip_part(&bytes, "Contents/section0.xml").as_bytes(), false).unwrap();
    let c = sec.iter("hp:container").next().expect("the group");
    assert_eq!(c.find("hp:rotationInfo").and_then(|r| r.get("angle")), Some("0"), "upright");
    let sz = c.find("hp:sz").expect("its size");
    assert_eq!((sz.get("width"), sz.get("height")), (Some("8000"), Some("2000")), "its own size, not a turned box");
}
