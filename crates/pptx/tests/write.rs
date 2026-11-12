//! The writer, read back with the reader.

use std::sync::Arc;

use craft_pptx::table::{Cell, Table};
use craft_pptx::{
    ColorRef, Defaults, Geom, MediaId, Plain, Presentation, Rgba, SchemeSlot, Shape, ShapeId, ShapeKind, Size, Slide, SlideId, TextBody, Xfrm,
};

/// A deck with the plain master and one slide on its layout.
fn deck() -> Presentation {
    let mut p = Presentation::default();
    let (m, last) = Plain.master(p.slide_size, 0);
    let layout = m.layouts[0].id;
    p.masters.push(Arc::new(m));
    p.next_id = last;
    let id = SlideId(p.alloc_id().unwrap());
    p.slides.push(Arc::new(Slide { id, layout, ..Default::default() }));
    p
}

fn add(p: &mut Presentation, mut s: Shape) -> ShapeId {
    s.id = ShapeId(p.alloc_id().unwrap());
    let id = s.id;
    Arc::make_mut(&mut p.slides[0]).shapes.push(s);
    id
}

fn bytes_of(pptx: &[u8], name: &str) -> Vec<u8> {
    craft_opc::Package::open(pptx, &craft_pptx::LIMITS).unwrap().get(name).unwrap().to_vec()
}

fn part(pptx: &[u8], name: &str) -> String {
    String::from_utf8(bytes_of(pptx, name)).unwrap()
}

fn round_trip(p: &Presentation) -> Presentation {
    let bytes = craft_pptx::export(p, &Plain).unwrap();
    craft_pptx::import(&bytes, &Plain).unwrap()
}

#[test]
fn shapes_survive_writing_and_reading() {
    let mut p = deck();
    let png = p.add_media("dot.png", "image/png", b"\x89PNG not really".to_vec()).unwrap();
    let mut text = TextBody::from_text("Hello\nWorld");
    text.body.anchor = Some(craft_pptx::text::Anchor::Bottom);
    let a = add(
        &mut p,
        Shape {
            name: "Box".into(),
            xfrm: Some(Xfrm::new(10.0, 20.0, 200.0, 100.0)),
            geom: Geom::Preset { name: "roundRect".into(), adj: vec![25000.0] },
            fill: Some(craft_pptx::Fill::solid(ColorRef::rgb(Rgba::rgb(0x12, 0x34, 0x56)))),
            text: Some(text),
            descr: "alt".into(),
            ..Default::default()
        },
    );
    let b = add(
        &mut p,
        Shape {
            name: "Picture".into(),
            xfrm: Some(Xfrm::new(300.0, 20.0, 50.0, 50.0)),
            kind: ShapeKind::Picture { fill: craft_pptx::style::PictureFill { media: png, ..Default::default() } },
            ..Default::default()
        },
    );
    add(&mut p, Shape { name: "Line".into(), kind: ShapeKind::Connector { start: Some((a, 1)), end: Some((b, 3)) }, ..Default::default() });
    let mut t = Table::new(2, 2, 200.0, 30.0);
    t.rows[1].cells[1] = Cell::new("B2");
    add(&mut p, Shape { name: "Table".into(), xfrm: Some(Xfrm::new(0.0, 200.0, 200.0, 60.0)), kind: ShapeKind::Table(t), ..Default::default() });

    let q = round_trip(&p);
    assert!(q.validate().is_empty(), "{:?}", q.validate());
    let s = &q.slides[0].shapes;
    assert_eq!(s.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["Box", "Picture", "Line", "Table"]);
    assert_eq!(s[0].xfrm, p.slides[0].shapes[0].xfrm);
    assert_eq!(s[0].geom, p.slides[0].shapes[0].geom);
    assert_eq!(s[0].fill, p.slides[0].shapes[0].fill);
    assert_eq!(s[0].descr, "alt");
    assert_eq!(s[0].text.as_ref().map(|t| (t.text(), t.body.anchor)), Some(("Hello\nWorld".into(), Some(craft_pptx::text::Anchor::Bottom))));
    let ShapeKind::Picture { fill } = &s[1].kind else { panic!("not a picture") };
    assert_eq!(q.media(fill.media).map(|m| m.data.as_slice()), Some(&b"\x89PNG not really"[..]));
    assert_eq!(s[2].kind, ShapeKind::Connector { start: Some((s[0].id, 1)), end: Some((s[1].id, 3)) });
    let ShapeKind::Table(t) = &s[3].kind else { panic!("not a table") };
    assert_eq!(t.rows[1].cells[1].text.text(), "B2");
}

#[test]
fn a_deck_without_masters_gets_the_callers() {
    let mut p = Presentation::default();
    p.slides.push(Arc::new(Slide { id: SlideId(1), ..Default::default() }));
    let bytes = craft_pptx::export(&p, &Plain).unwrap();
    let q = craft_pptx::import(&bytes, &Plain).unwrap();
    assert_eq!(q.masters.len(), 1);
    assert_eq!(q.slides.len(), 1);
    assert_eq!(q.slides[0].layout, q.masters[0].layouts[0].id);
}

#[test]
fn a_missing_picture_is_written_as_a_transparent_stand_in() {
    let mut p = deck();
    add(
        &mut p,
        Shape {
            kind: ShapeKind::Picture { fill: craft_pptx::style::PictureFill { media: MediaId(999), ..Default::default() } },
            ..Default::default()
        },
    );
    let bytes = craft_pptx::export(&p, &Plain).unwrap();
    let png = bytes_of(&bytes, "ppt/media/image1.png");
    assert!(png.starts_with(b"\x89PNG\r\n\x1a\n") && png.ends_with(b"IEND\xAE\x42\x60\x82"), "{png:?}");
}

/// The caller's name and text styles go into the file.
#[test]
fn the_callers_defaults_are_written() {
    struct Acme;
    impl Defaults for Acme {
        fn master(&self, size: Size, after: u32) -> (craft_pptx::Master, u32) {
            Plain.master(size, after)
        }
        fn default_text_style(&self) -> craft_pptx::text::ListStyle {
            let mut ls = craft_pptx::text::ListStyle::default();
            ls.set(
                0,
                craft_pptx::text::LevelStyle { run: craft_pptx::text::RunProps { size: Some(26.0), ..Default::default() }, ..Default::default() },
            );
            ls
        }
        fn application(&self) -> &str {
            "Acme"
        }
    }
    let mut p = deck();
    let t = Table { style: "light1-accent2".into(), ..Table::new(1, 1, 100.0, 20.0) };
    add(&mut p, Shape { kind: ShapeKind::Table(t), ..Default::default() });
    let bytes = craft_pptx::export(&p, &Acme).unwrap();
    assert!(part(&bytes, "docProps/app.xml").contains("<Application>Acme</Application>"));
    assert!(part(&bytes, "ppt/tableStyles.xml").contains("styleName=\"Acme "), "{}", part(&bytes, "ppt/tableStyles.xml"));
    let q = craft_pptx::import(&bytes, &Plain).unwrap();
    assert_eq!(q.default_text_style.level(0).and_then(|l| l.run.size), Some(26.0));
    assert!(part(&craft_pptx::export(&p, &Plain).unwrap(), "docProps/app.xml").contains("<Application>craft-pptx</Application>"));
}

/// From DeckCraft's `crates/pptx/src/lib.rs`.
#[test]
fn table_direction_and_column_order_survive_two_saves() {
    for rtl in [Some(true), Some(false), None] {
        let mut p = deck();
        let mut t = Table::new(1, 2, 200.0, 30.0);
        t.rtl = rtl;
        t.rows[0].cells[0] = Cell::new("First column");
        t.rows[0].cells[1] = Cell::new("Second column");
        add(&mut p, Shape { kind: ShapeKind::Table(t), ..Default::default() });
        let mut q = round_trip(&p);
        for _ in 0..2 {
            let Some(ShapeKind::Table(t)) = q.slides[0].shapes.first().map(|s| &s.kind) else { panic!("no table") };
            assert_eq!(t.rtl, rtl);
            assert_eq!(t.rows[0].cells[0].text.paragraphs[0].runs[0].text, "First column");
            assert_eq!(t.rows[0].cells[1].text.paragraphs[0].runs[0].text, "Second column");
            q = round_trip(&q);
        }
    }
}

/// From DeckCraft's `crates/pptx/src/lib.rs`: the theme of a written deck, and of a `.thmx`
/// holding that theme part.
#[test]
fn reads_the_theme_of_a_pptx_and_of_a_thmx() {
    let mut p = deck();
    let m = Arc::make_mut(&mut p.masters[0]);
    m.theme.name = "Harbour".into();
    m.theme.fonts.major.latin = "Lato".into();
    m.theme.colors.set(SchemeSlot::Accent1, Rgba::rgb(0x12, 0x34, 0x56));
    let pptx = craft_pptx::export(&p, &Plain).unwrap();
    let mut out = craft_opc::PackageOut::default();
    out.xml_part(
        "theme/theme/theme1.xml",
        "application/vnd.openxmlformats-officedocument.theme+xml",
        part(&pptx, "ppt/theme/theme1.xml").into_bytes(),
    );
    let mut rels = craft_opc::RelsOut::default();
    rels.add("officeDocument", "theme/theme/theme1.xml", false);
    out.rels("", &rels);
    let thmx = out.finish().unwrap();
    for bytes in [pptx, thmx] {
        let (t, _) = craft_pptx::import_theme(&bytes).unwrap();
        assert_eq!(t.name, "Harbour");
        assert_eq!(t.fonts.major.latin, "Lato");
        assert_eq!(t.colors.get(SchemeSlot::Accent1), Rgba::rgb(0x12, 0x34, 0x56));
    }
}

#[test]
fn templates_and_shows_differ_in_the_main_content_type() {
    let p = deck();
    for (ext, ct) in [("potx", "template.main+xml"), ("ppsx", "slideshow.main+xml"), ("pptx", "presentation.main+xml")] {
        let bytes = craft_pptx::export_as(&p, &Plain, ext).unwrap();
        assert!(part(&bytes, "[Content_Types].xml").contains(ct), "{ext}");
        assert_eq!(craft_pptx::import(&bytes, &Plain).unwrap().slides.len(), 1);
    }
}
