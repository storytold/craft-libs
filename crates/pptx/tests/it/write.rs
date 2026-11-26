//! The writer, read back with the reader.

use super::common;

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

const NS_A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const NS_P: &str = "http://schemas.openxmlformats.org/presentationml/2006/main";
const NS_THM15: &str = "http://schemas.microsoft.com/office/thememl/2012/main";

/// The namespace of every element named `local` in `xml`, resolved through the declarations in
/// scope (`""` for none).
fn namespaces_of(xml: &str, local: &str) -> Vec<String> {
    fn rec(e: &craft_xml::El, scope: &mut Vec<(String, String)>, local: &str, out: &mut Vec<String>) {
        let n = scope.len();
        scope.extend(e.ns_decls().map(|(p, u)| (p.to_string(), u.to_string())));
        if e.local() == local {
            out.push(scope.iter().rev().find(|(p, _)| p == e.prefix()).map(|(_, u)| u.clone()).unwrap_or_default());
        }
        for c in e.elements() {
            rec(c, scope, local, out);
        }
        scope.truncate(n);
    }
    let root = craft_xml::Options::default().strict().parse(xml.as_bytes()).unwrap_or_else(|e| panic!("not well-formed: {e}\n{xml}"));
    let mut out = vec![];
    rec(&root, &mut vec![], local, &mut out);
    out
}

/// A theme with a custom colour list and a theme family id, with the `a:` prefix.
const PREFIXED_THEME: &str = r#"<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" name="Harbour"><a:themeElements><a:clrScheme name="Harbour"><a:dk1><a:srgbClr val="000000"/></a:dk1><a:lt1><a:srgbClr val="FFFFFF"/></a:lt1><a:dk2><a:srgbClr val="1F1F1F"/></a:dk2><a:lt2><a:srgbClr val="EEEEEE"/></a:lt2><a:accent1><a:srgbClr val="123456"/></a:accent1><a:accent2><a:srgbClr val="222222"/></a:accent2><a:accent3><a:srgbClr val="333333"/></a:accent3><a:accent4><a:srgbClr val="444444"/></a:accent4><a:accent5><a:srgbClr val="555555"/></a:accent5><a:accent6><a:srgbClr val="666666"/></a:accent6><a:hlink><a:srgbClr val="0000FF"/></a:hlink><a:folHlink><a:srgbClr val="800080"/></a:folHlink></a:clrScheme><a:fontScheme name="Harbour"><a:majorFont><a:latin typeface="Lato"/><a:ea typeface=""/><a:cs typeface=""/></a:majorFont><a:minorFont><a:latin typeface="Calibri"/><a:ea typeface=""/><a:cs typeface=""/></a:minorFont></a:fontScheme><a:fmtScheme name="Harbour"><a:fillStyleLst/><a:lnStyleLst/><a:effectStyleLst><a:effectStyle><a:effectLst/><a:scene3d><a:camera prst="orthographicFront"/><a:lightRig rig="threePt" dir="t"/></a:scene3d><a:sp3d><a:bevelT w="63500" h="25400"/></a:sp3d></a:effectStyle><a:effectStyle><a:effectLst/></a:effectStyle><a:effectStyle><a:effectLst/></a:effectStyle></a:effectStyleLst><a:bgFillStyleLst/></a:fmtScheme></a:themeElements><a:objectDefaults/><a:extraClrSchemeLst/><a:custClrLst><a:custClr name="Brand"><a:srgbClr val="C00000"/></a:custClr></a:custClrLst><a:extLst><a:ext uri="{05A4C25C-085E-4340-85A3-A5531E510DB2}"><thm15:themeFamily xmlns:thm15="http://schemas.microsoft.com/office/thememl/2012/main" name="Harbour" id="{62F939B6-93AF-4DB8-9C6B-D6C7DFDC589F}" vid="{4A3C46E8-61CC-4603-A589-7422A47A8E4A}"/></a:ext></a:extLst></a:theme>"#;

/// The deck with its theme part replaced by `theme`.
fn with_theme(theme: &str) -> Vec<u8> {
    let pptx = craft_pptx::export(&deck(), &Plain).unwrap();
    common::rewrite(&pptx, |name, data| Some(if name == "ppt/theme/theme1.xml" { theme.as_bytes().to_vec() } else { data.to_vec() }), &[])
}

/// A theme written with a default namespace instead of the `a:` prefix: the parts kept as
/// written (custom colours, theme family id) stay in the DrawingML namespace on save.
#[test]
fn a_default_namespace_theme_keeps_its_kept_parts_in_the_drawingml_namespace() {
    let theme = PREFIXED_THEME.replace("<a:", "<").replace("</a:", "</").replace("xmlns:a=", "xmlns=");
    assert!(!theme.contains("a:"), "{theme}");
    let p = craft_pptx::import(&with_theme(&theme), &Plain).unwrap();
    assert_eq!(p.masters[0].theme.colors.get(SchemeSlot::Accent1), Rgba::rgb(0x12, 0x34, 0x56));
    let once = craft_pptx::export(&p, &Plain).unwrap();
    let xml = part(&once, "ppt/theme/theme1.xml");
    // The writer may add an `a:extLst` of its own to the colour scheme, so every one is checked.
    for local in ["objectDefaults", "custClrLst", "custClr", "extLst", "ext", "srgbClr"] {
        let ns = namespaces_of(&xml, local);
        assert!(!ns.is_empty() && ns.iter().all(|n| n == NS_A), "{local}: {ns:?}\n{xml}");
    }
    assert_eq!(namespaces_of(&xml, "themeFamily"), [NS_THM15], "{xml}");
    // The effect style's 3-D is kept as written too.
    assert_eq!(namespaces_of(&xml, "scene3d"), [NS_A], "{xml}");
    assert_eq!(namespaces_of(&xml, "sp3d"), [NS_A], "{xml}");
    // The second save writes the same kept parts.
    let kept = |x: &str| x[x.find("</a:themeElements>").expect("themeElements")..].to_string();
    let twice = craft_pptx::export(&craft_pptx::import(&once, &Plain).unwrap(), &Plain).unwrap();
    assert_eq!(kept(&part(&twice, "ppt/theme/theme1.xml")), kept(&xml));
}

/// The same theme with the `a:` prefix: the kept parts are written as before (each declaring
/// the `a:` it uses), with no default namespace added.
#[test]
fn a_prefixed_theme_keeps_its_kept_parts_unchanged() {
    let p = craft_pptx::import(&with_theme(PREFIXED_THEME), &Plain).unwrap();
    let xml = part(&craft_pptx::export(&p, &Plain).unwrap(), "ppt/theme/theme1.xml");
    assert!(!xml.contains(r#" xmlns=""#), "{xml}");
    let a = format!(r#"xmlns:a="{NS_A}""#);
    let kept = format!(
        r#"<a:objectDefaults {a}/><a:extraClrSchemeLst {a}/><a:custClrLst {a}><a:custClr name="Brand"><a:srgbClr val="C00000"/></a:custClr></a:custClrLst><a:extLst {a}><a:ext uri="{{05A4C25C-085E-4340-85A3-A5531E510DB2}}"><thm15:themeFamily xmlns:thm15="{NS_THM15}" name="Harbour" id="{{62F939B6-93AF-4DB8-9C6B-D6C7DFDC589F}}" vid="{{4A3C46E8-61CC-4603-A589-7422A47A8E4A}}"/></a:ext></a:extLst></a:theme>"#
    );
    assert!(xml.contains(&kept), "{xml}");
}

/// The deck with part `name` (root `root`) given a default PresentationML namespace and an
/// unprefixed extension list, saved again: the part as written.
fn default_namespace_extensions(name: &str, root: &str) -> String {
    let pptx = craft_pptx::export(&deck(), &Plain).unwrap();
    let ext = r#"<extLst><ext uri="{11111111-2222-3333-4444-555555555555}"><t:data xmlns:t="urn:test" v="1"/></ext></extLst>"#;
    let (open, close) = (format!("<{root} "), format!("</{root}>"));
    let doc = common::rewrite(
        &pptx,
        |n, data| {
            if n != name {
                return Some(data.to_vec());
            }
            let xml = String::from_utf8(data.to_vec()).unwrap();
            assert!(xml.contains(&open) && xml.contains(&close) && !xml.contains("extLst"), "{xml}");
            Some(xml.replacen(&open, &format!(r#"<{root} xmlns="{NS_P}" "#), 1).replace(&close, &format!("{ext}{close}")).into_bytes())
        },
        &[],
    );
    let p = craft_pptx::import(&doc, &Plain).unwrap();
    part(&craft_pptx::export(&p, &Plain).unwrap(), name)
}

/// The namespaces of the `ext` elements in `xml` other than DrawingML's `a:ext` (an extent).
fn extension_namespaces(xml: &str) -> Vec<String> {
    namespaces_of(xml, "ext").into_iter().filter(|n| n != NS_A).collect()
}

/// A slide written with a default PresentationML namespace: its kept extension list stays in
/// that namespace under the writer's `p:extLst`.
#[test]
fn a_default_namespace_slide_keeps_its_extensions_in_the_presentationml_namespace() {
    let xml = default_namespace_extensions("ppt/slides/slide1.xml", "p:sld");
    assert!(xml.contains("{11111111-2222-3333-4444-555555555555}"), "{xml}");
    assert_eq!(extension_namespaces(&xml), [NS_P], "{xml}");
    assert_eq!(namespaces_of(&xml, "data"), ["urn:test"], "{xml}");
}

/// The same for a layout and a master, whose kept extension list is written back through
/// `ext_children`.
#[test]
fn a_default_namespace_layout_and_master_keep_their_extensions_in_the_presentationml_namespace() {
    for (name, root) in [("ppt/slideLayouts/slideLayout1.xml", "p:sldLayout"), ("ppt/slideMasters/slideMaster1.xml", "p:sldMaster")] {
        let xml = default_namespace_extensions(name, root);
        assert!(xml.contains("{11111111-2222-3333-4444-555555555555}"), "{name}: {xml}");
        assert_eq!(extension_namespaces(&xml), [NS_P], "{name}: {xml}");
        assert_eq!(namespaces_of(&xml, "data"), ["urn:test"], "{name}: {xml}");
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
