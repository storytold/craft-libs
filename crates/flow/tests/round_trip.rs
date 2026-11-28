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

#[test]
fn drop_cap_not_repeated_after_page_break() {
    let mut doc = craft_wordmodel::Document::default();
    let mut para = craft_wordmodel::Paragraph::with_text("Before\x0cAfter", craft_wordmodel::props::CharProps::default());
    para.props.drop_cap = Some(3);
    para.props.drop_cap_distance = Some(72.0);
    para.props.shading = Some(craft_wordmodel::Rgb(255, 0, 0));
    doc.body = vec![std::sync::Arc::new(craft_wordmodel::Block::Para(para))];

    let flow = craft_flow::from_doc_with(&doc, craft_flow::FlowOpts { paras: true, ..Default::default() });
    let paras: Vec<_> = flow.blocks.iter().filter_map(|b| if let FBlock::Para(p) = b { Some(p) } else { None }).collect();
    assert_eq!(paras.len(), 2);
    // First paragraph keeps drop cap and shading
    assert_eq!(paras[0].extra.as_ref().and_then(|x| x.drop_cap), Some(3));
    assert_eq!(paras[0].extra.as_ref().and_then(|x| x.drop_cap_distance), Some(72.0));
    assert_eq!(paras[0].extra.as_ref().and_then(|x| x.shading), Some(craft_wordmodel::Rgb(255, 0, 0)));
    // Continuation clears drop cap, but preserves shading
    assert_eq!(paras[1].extra.as_ref().and_then(|x| x.drop_cap), None);
    assert_eq!(paras[1].extra.as_ref().and_then(|x| x.drop_cap_distance), None);
    assert_eq!(paras[1].extra.as_ref().and_then(|x| x.shading), Some(craft_wordmodel::Rgb(255, 0, 0)));
}

#[test]
fn vml_hyperlinks_survive_flow_for_roots_and_nested_members() {
    use craft_flow::{BoxLook, FlowOpts, Member, from_doc_with};
    use craft_wordmodel::{Block, CharProps, para::ShapeKind};

    let image = |target: Option<&str>| {
        let mut img = make_img(PNG.to_vec(), Some(12.0), Some(8.0), "original tiny picture").unwrap();
        img.link = target.map(str::to_string);
        img.crop = [0.125, 0.25, 0.0, 0.0];
        Inline::Image(img)
    };
    let nested = Inline::Group {
        w: 10.0,
        h: 10.0,
        float: None,
        link: None,
        members: vec![Member { x: 2.0, y: 1.0, inline: image(None), link: Some("#nested-place".into()) }],
    };
    let group = Inline::Group {
        w: 40.0,
        h: 30.0,
        float: None,
        link: Some("https://example.com/group".into()),
        members: vec![
            Member { x: 1.0, y: 2.0, inline: image(None), link: Some("https://example.com/child".into()) },
            Member { x: 8.0, y: 3.0, inline: nested, link: Some("https://example.com/nested-group".into()) },
            Member { x: 4.0, y: 5.0, inline: image(None), link: None },
        ],
    };
    let shape = Inline::Shape {
        kind: ShapeKind::Rectangle,
        w: 9.0,
        h: 6.0,
        float: None,
        blocks: Vec::new(),
        look: BoxLook::default(),
        path: None,
        link: Some("https://example.com/shape".into()),
    };
    let mut paragraph = para(Kind::Normal, vec![group, image(Some("https://example.com/picture")), shape, image(None)]);
    paragraph.mark = Some(Box::new(CharProps { link: Some("https://example.com/not-the-object".into()), ..Default::default() }));
    let flow = Flow { blocks: vec![FBlock::Para(paragraph)], ..Default::default() };
    let doc = to_doc(&flow);
    let Block::Para(p) = &*doc.body[0] else { panic!("drawing paragraph") };
    assert_eq!(
        p.runs.iter().map(|r| r.props.link.as_deref()).collect::<Vec<_>>(),
        [Some("https://example.com/group"), Some("https://example.com/picture"), Some("https://example.com/shape"), None],
        "each root's target comes from that object, not the paragraph mark"
    );
    let craft_wordmodel::para::InlineObject::Group { children, .. } = &p.objects[0] else { panic!("group") };
    assert_eq!(children[0].link.as_deref(), Some("https://example.com/child"));
    assert_eq!(children[1].link.as_deref(), Some("https://example.com/nested-group"));
    assert!(children[2].link.is_none());
    let back = from_doc_with(&doc, FlowOpts { boxes: true, paras: true, ..Default::default() });
    let got = paras(&back.blocks);
    let [Inline::Group { link, members, .. }, Inline::Image(picture), Inline::Shape { link: shape_link, .. }, Inline::Image(unlinked)] =
        got[0].inlines.as_slice()
    else {
        panic!("all four drawings survive")
    };
    assert_eq!(link.as_deref(), Some("https://example.com/group"));
    assert_eq!(picture.link.as_deref(), Some("https://example.com/picture"));
    assert_eq!(shape_link.as_deref(), Some("https://example.com/shape"));
    assert!(unlinked.link.is_none());
    assert_eq!(picture.data.as_slice(), PNG);
    assert_eq!((picture.w, picture.h, picture.crop), (12.0, 8.0, [0.125, 0.25, 0.0, 0.0]));
    assert_eq!((members[0].x, members[0].y), (1.0, 2.0));
    assert_eq!(members[0].link.as_deref(), Some("https://example.com/child"));
    assert_eq!(members[1].link.as_deref(), Some("https://example.com/nested-group"));
    assert!(members[2].link.is_none());
    let Inline::Group { link: own, members: inner, .. } = &members[1].inline else { panic!("nested group") };
    assert!(own.is_none(), "the nested group's target is carried by its Member, not duplicated on descendants");
    assert_eq!(inner[0].link.as_deref(), Some("#nested-place"));

    fn check_depth_boundary(image: impl Fn(Option<&str>) -> Inline) {
        // DOCX retains an outer group at depth 0 and eight nested subgroups. Flow must
        // keep that complete tree rather than lose its last drawable member.
        fn linked_chain(leaf: Inline, deepest: usize) -> Inline {
            let mut inline = leaf;
            for level in (0..=deepest).rev() {
                let target = if level == deepest { "#last-picture".to_string() } else { format!("https://example.com/level{}", level + 1) };
                inline = Inline::Group {
                    w: 12.0,
                    h: 8.0,
                    float: None,
                    link: (level == 0).then(|| "https://example.com/deep-root".into()),
                    members: vec![Member { x: 0.0, y: 0.0, inline, link: Some(target) }],
                };
            }
            inline
        }
        let boundary = Flow { blocks: vec![FBlock::Para(para(Kind::Normal, vec![linked_chain(image(None), 8)]))], ..Default::default() };
        let boundary_doc = to_doc(&boundary);
        let boundary_para = boundary_doc.body[0].as_para().unwrap();
        assert_eq!(boundary_para.objects.len(), 1, "root plus eight subgroups remains drawable");
        assert_eq!(boundary_para.props_of_char(0).link.as_deref(), Some("https://example.com/deep-root"));
        let mut object = &boundary_para.objects[0];
        for level in 0..=8 {
            let craft_wordmodel::para::InlineObject::Group { children, .. } = object else { panic!("group at level {level}") };
            assert_eq!(children.len(), 1);
            let target = if level == 8 { "#last-picture".to_string() } else { format!("https://example.com/level{}", level + 1) };
            assert_eq!(children[0].link.as_deref(), Some(target.as_str()), "document target at level {level}");
            object = &children[0].obj;
        }
        let craft_wordmodel::para::InlineObject::Image { media, .. } = object else { panic!("last picture") };
        assert_eq!(boundary_doc.media.get(media).unwrap().as_slice(), PNG);
        let boundary_back = from_doc_with(&boundary_doc, FlowOpts { boxes: true, paras: true, ..Default::default() });
        let boundary_paras = paras(&boundary_back.blocks);
        assert_eq!(boundary_paras[0].inlines.len(), 1);
        let mut inline = &boundary_paras[0].inlines[0];
        for level in 0..=8 {
            let Inline::Group { members, link, .. } = inline else { panic!("flow group at level {level}") };
            assert_eq!(link.as_deref(), (level == 0).then_some("https://example.com/deep-root"));
            assert_eq!(members.len(), 1);
            let target = if level == 8 { "#last-picture".to_string() } else { format!("https://example.com/level{}", level + 1) };
            assert_eq!(members[0].link.as_deref(), Some(target.as_str()), "flow target at level {level}");
            inline = &members[0].inline;
        }
        let Inline::Image(last) = inline else { panic!("last flow picture") };
        assert_eq!(last.data.as_slice(), PNG);

        // One further subgroup still exceeds the existing limit. Its root's separate
        // drawable sibling survives, so dropping the over-deep branch cannot pass vacuously.
        let mut beyond = linked_chain(image(None), 9);
        let Inline::Group { members, .. } = &mut beyond else { panic!() };
        members.push(Member { x: 0.0, y: 0.0, inline: image(None), link: Some("https://example.com/sibling".into()) });
        let beyond_flow = Flow { blocks: vec![FBlock::Para(para(Kind::Normal, vec![beyond]))], ..Default::default() };
        let beyond_doc = to_doc(&beyond_flow);
        let beyond_para = beyond_doc.body[0].as_para().unwrap();
        assert_eq!(beyond_para.objects.len(), 1);
        let craft_wordmodel::para::InlineObject::Group { children, .. } = &beyond_para.objects[0] else { panic!() };
        assert_eq!(children.len(), 1, "the ninth subgroup is still excluded");
        assert_eq!(children[0].link.as_deref(), Some("https://example.com/sibling"));
        assert!(matches!(&children[0].obj, craft_wordmodel::para::InlineObject::Image { .. }));
    }
    check_depth_boundary(image);

    fn check_oversized_drawing_links(image: impl Fn(Option<&str>) -> Inline) {
        // Construct valid drawable inputs first; to_doc's own budget must not hide
        // the from_doc boundary being checked by this regression.
        let group =
            Inline::Group { w: 12.0, h: 8.0, float: None, link: None, members: vec![Member { x: 0.0, y: 0.0, inline: image(None), link: None }] };
        let input = Flow {
            blocks: vec![FBlock::Para(para(Kind::Normal, vec![group, image(None), image(Some("https://example.com/kept"))]))],
            ..Default::default()
        };
        let mut doc = to_doc(&input);
        let mut paragraph = doc.body[0].as_para().unwrap().clone();
        let offsets = paragraph.object_offsets();
        let oversized = "x".repeat(craft_flow::MAX_LINK + 1);
        paragraph.format(offsets[0], offsets[0] + craft_wordmodel::para::OBJ.len_utf8(), &|props| props.link = Some(oversized.clone())).unwrap();
        paragraph.format(offsets[1], offsets[1] + craft_wordmodel::para::OBJ.len_utf8(), &|props| props.link = Some(oversized.clone())).unwrap();
        let craft_wordmodel::para::InlineObject::Group { children, .. } = &mut paragraph.objects[0] else { panic!() };
        children[0].link = Some(oversized);
        doc.body = vec![Block::Para(paragraph).into()];
        let back = from_doc_with(&doc, FlowOpts { boxes: true, paras: true, ..Default::default() });
        let got = paras(&back.blocks);
        let [Inline::Group { link, members, .. }, Inline::Image(unlinked), Inline::Image(kept)] = got[0].inlines.as_slice() else {
            panic!("all drawings remain")
        };
        assert!(link.is_none(), "oversized group target is dropped");
        assert_eq!(members.len(), 1);
        assert!(members[0].link.is_none(), "oversized member target is dropped");
        assert!(unlinked.link.is_none(), "oversized root image target is dropped");
        assert_eq!(kept.link.as_deref(), Some("https://example.com/kept"));
        assert_eq!(kept.data.as_slice(), PNG);
    }
    check_oversized_drawing_links(image);
}
