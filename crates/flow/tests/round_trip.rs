//! A flow made into a document and read back keeps what the text formats express: kinds,
//! lists, alignment, character formatting, links, tables, pictures and the metadata.

use craft_flow::{Cell, FBlock, FTable, Flow, Fmt, Inline, Kind, ListInfo, Meta, Para, from_doc, make_img, to_doc};
use craft_wordmodel::Align;

/// A 1×1 PNG.
const PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 0x0D, 0x49, 0x48, 0x44, 0x52, 0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0, 0x1F, 0x15, 0xC4,
    0x89, 0, 0, 0, 0x0D, 0x49, 0x44, 0x41, 0x54, 0x78, 0xDA, 0x63, 0xFC, 0xCF, 0xC0, 0x50, 0x0F, 0, 0x04, 0x85, 0x01, 0x80, 0x84, 0xA9, 0x8C, 0x21,
    0, 0, 0, 0, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
];

fn para(kind: Kind, inlines: Vec<Inline>) -> Para {
    Para { kind, inlines, ..Default::default() }
}

fn text(s: &str, f: Fmt) -> Inline {
    Inline::Text(s.into(), f)
}

fn sample() -> Flow {
    let bold = Fmt { bold: true, ..Default::default() };
    let link = Fmt { link: Some("https://example.com/x".into()), ..Default::default() };
    let mut centred =
        para(Kind::Normal, vec![text("Centred ", Fmt::default()), text("bold", bold.clone()), text(" and ", Fmt::default()), text("a link", link)]);
    centred.align = Some(Align::Center);
    let item = |s: &str, ordered: bool, level: u8| Para {
        list: Some(ListInfo { ordered, level, ..Default::default() }),
        ..para(Kind::Normal, vec![text(s, Fmt::default())])
    };
    let cell = |s: &str, header: bool| Cell {
        blocks: vec![FBlock::Para(para(Kind::Normal, vec![text(s, Fmt::default())]))],
        colspan: 1,
        rowspan: 1,
        header,
        ..Default::default()
    };
    let table = FTable { rows: vec![vec![cell("A", true), cell("B", true)], vec![cell("1", false), cell("2", false)]], ..Default::default() };
    let img = make_img(PNG.to_vec(), Some(36.0), None, "tiny").map(Inline::Image).into_iter().collect();
    Flow {
        blocks: vec![
            FBlock::Para(para(Kind::Heading(1), vec![text("Main heading", Fmt::default())])),
            FBlock::Para(centred),
            FBlock::Para(item("first", false, 0)),
            FBlock::Para(item("nested", true, 1)),
            FBlock::Para(para(Kind::Quote, vec![text("Quoted.", Fmt::default())])),
            FBlock::Para(para(Kind::Code, vec![text("let x = 1;", Fmt { code: true, ..Default::default() })])),
            FBlock::Table(table),
            FBlock::Para(para(Kind::Normal, img)),
        ],
        meta: Meta { title: "Sample".into(), author: "Ada".into(), ..Default::default() },
    }
}

fn paras(blocks: &[FBlock]) -> Vec<&Para> {
    blocks.iter().filter_map(|b| if let FBlock::Para(p) = b { Some(p) } else { None }).collect()
}

#[test]
fn a_flow_survives_a_document() {
    let flow = sample();
    let doc = to_doc(&flow);
    assert_eq!((doc.core.title.as_str(), doc.core.creator.as_str()), ("Sample", "Ada"));
    assert_eq!(doc.media.len(), 1, "the picture is in the document");
    let back = from_doc(&doc);
    let (want, got) = (paras(&flow.blocks), paras(&back.blocks));
    assert_eq!(got.len(), want.len());
    for (w, g) in want.iter().zip(&got) {
        assert_eq!((g.kind, g.list.map(|l| (l.ordered, l.level)), g.align), (w.kind, w.list.map(|l| (l.ordered, l.level)), w.align), "{}", w.text());
        assert_eq!(g.text(), w.text());
    }
    let fmt_of = |p: &Para, s: &str| {
        p.inlines.iter().find_map(|i| {
            if let Inline::Text(t, f) = i
                && t == s
            {
                Some(f.clone())
            } else {
                None
            }
        })
    };
    assert!(fmt_of(got[1], "bold").is_some_and(|f| f.bold));
    assert_eq!(fmt_of(got[1], "a link").and_then(|f| f.link).as_deref(), Some("https://example.com/x"));
    assert!(matches!(got[6].inlines.as_slice(), [Inline::Image(i)] if i.alt == "tiny" && (i.w - 36.0).abs() < 0.01));
    let Some(FBlock::Table(t)) = back.blocks.iter().find(|b| matches!(b, FBlock::Table(_))) else { panic!("no table") };
    assert_eq!(t.rows.len(), 2);
    assert_eq!(t.rows[1].iter().map(Cell::text).collect::<Vec<_>>(), ["1", "2"]);
}

#[test]
fn pictures_are_sniffed_and_sized_from_their_headers() {
    assert_eq!(craft_flow::sniff_image(PNG), Some("png"));
    assert_eq!(craft_flow::image_px(PNG), Some((1, 1)));
    assert_eq!(craft_flow::sniff_image(b"not a picture"), None);
    assert!(make_img(b"not a picture".to_vec(), None, None, "").is_none());
    // A picture wider than the column is shrunk to it, keeping its proportions.
    let wide = make_img(PNG.to_vec(), Some(1000.0), Some(500.0), "").unwrap();
    assert_eq!((wide.w, wide.h), (468.0, 234.0));
}
