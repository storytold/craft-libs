//! The reader on small packages written by hand.

use craft_opc::{PackageOut, RelsOut};
use craft_pptx::{Defaults, Geom, LayoutType, Master, PhType, Plain, Presentation, ShapeKind, Size};

const NS: &str = r#"xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main""#;
const CT_PRES: &str = "application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml";

/// A package from (part, content type, xml) and (part, [(kind, target)]) relationships.
fn package(parts: &[(&str, &str, String)], rels: &[(&str, &[(&str, &str)])]) -> Vec<u8> {
    let mut out = PackageOut::default();
    for (name, ct, xml) in parts {
        if ct.is_empty() {
            out.binary(name, "image/png", xml.clone().into_bytes());
        } else {
            out.xml_part(name, ct, xml.clone().into_bytes());
        }
    }
    for (part, list) in rels {
        let mut r = RelsOut::default();
        for (kind, target) in *list {
            r.add_new(kind, target, false);
        }
        out.rels(part, &r);
    }
    out.finish().unwrap()
}

fn sp(id: u32, name: &str, ph: &str, pr: &str, text: &str) -> String {
    format!(
        r#"<p:sp><p:nvSpPr><p:cNvPr id="{id}" name="{name}"/><p:cNvSpPr/><p:nvPr>{ph}</p:nvPr></p:nvSpPr><p:spPr>{pr}</p:spPr><p:txBody><a:bodyPr/><a:p><a:r><a:t>{text}</a:t></a:r></a:p></p:txBody></p:sp>"#
    )
}

const XFRM: &str = r#"<a:xfrm><a:off x="12700" y="25400"/><a:ext cx="1270000" cy="635000"/></a:xfrm>"#;

fn deck() -> Vec<u8> {
    let pres = format!(
        r#"<p:presentation {NS} firstSlideNum="3"><p:sldMasterIdLst><p:sldMasterId id="2147483648" r:id="rId1"/></p:sldMasterIdLst><p:sldIdLst><p:sldId id="256" r:id="rId2"/><p:sldId id="257" r:id="rId3"/></p:sldIdLst><p:sldSz cx="9144000" cy="6858000"/><p:notesSz cx="6858000" cy="9144000"/><p:custShowLst><p:custShow name="Short" id="0"><p:sldLst><p:sld r:id="rId2"/></p:sldLst></p:custShow></p:custShowLst><p:extLst><p:ext uri="{{521415D9-36F7-43E2-AB2F-B90AF26B5E84}}"><p14:sectionLst xmlns:p14="http://schemas.microsoft.com/office/powerpoint/2010/main"><p14:section name="Intro" id="{{0}}"><p14:sldIdLst><p14:sldId id="256"/></p14:sldIdLst></p14:section></p14:sectionLst></p:ext></p:extLst></p:presentation>"#
    );
    let master = format!(
        r#"<p:sldMaster {NS}><p:cSld name="Main"><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/>{}{}</p:spTree></p:cSld><p:clrMap bg1="lt1" tx1="dk1" bg2="lt2" tx2="dk2" accent1="accent1" accent2="accent2" accent3="accent3" accent4="accent4" accent5="accent5" accent6="accent6" hlink="hlink" folHlink="folHlink"/><p:sldLayoutIdLst><p:sldLayoutId id="2147483649" r:id="rId1"/></p:sldLayoutIdLst><p:txStyles><p:titleStyle><a:lvl1pPr><a:defRPr sz="4400"/></a:lvl1pPr></p:titleStyle><p:bodyStyle/><p:otherStyle/></p:txStyles></p:sldMaster>"#,
        sp(2, "Title", r#"<p:ph type="title"/>"#, XFRM, ""),
        sp(3, "Body", r#"<p:ph type="body" idx="1"/>"#, XFRM, "")
    );
    let layout = format!(
        r#"<p:sldLayout {NS} type="title"><p:cSld name="Title Slide"><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/>{}</p:spTree></p:cSld></p:sldLayout>"#,
        sp(2, "Title", r#"<p:ph type="ctrTitle"/>"#, &format!(r#"{XFRM}<a:prstGeom prst="ellipse"><a:avLst/></a:prstGeom>"#), "")
    );
    let shapes = [
        sp(5, "Title 1", r#"<p:ph type="ctrTitle"/>"#, "", "Hello"),
        sp(6, "Box", "", &format!(r#"{XFRM}<a:prstGeom prst="roundRect"><a:avLst><a:gd name="adj" fmla="val 30000"/></a:avLst></a:prstGeom><a:solidFill><a:srgbClr val="FF0000"/></a:solidFill>"#), "Text"),
        format!(r#"<p:pic><p:nvPicPr><p:cNvPr id="7" name="Picture" descr="A dot"/><p:cNvPicPr/><p:nvPr/></p:nvPicPr><p:blipFill><a:blip r:embed="rId2"/><a:stretch><a:fillRect/></a:stretch></p:blipFill><p:spPr>{XFRM}<a:prstGeom prst="rect"><a:avLst/></a:prstGeom></p:spPr></p:pic>"#),
        r#"<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="8" name="Table"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr><p:xfrm><a:off x="0" y="0"/><a:ext cx="2540000" cy="508000"/></p:xfrm><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/table"><a:tbl><a:tblPr firstRow="1"/><a:tblGrid><a:gridCol w="1270000"/><a:gridCol w="1270000"/></a:tblGrid><a:tr h="254000"><a:tc><a:txBody><a:bodyPr/><a:p><a:r><a:t>A1</a:t></a:r></a:p></a:txBody><a:tcPr/></a:tc><a:tc><a:txBody><a:bodyPr/><a:p><a:r><a:t>B1</a:t></a:r></a:p></a:txBody><a:tcPr/></a:tc></a:tr></a:tbl></a:graphicData></a:graphic></p:graphicFrame>"#.to_string(),
        format!(r#"<p:grpSp><p:nvGrpSpPr><p:cNvPr id="9" name="Group"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="127000" cy="127000"/><a:chOff x="0" y="0"/><a:chExt cx="254000" cy="254000"/></a:xfrm></p:grpSpPr>{}</p:grpSp>"#, sp(10, "Inner", "", XFRM, "In")),
        r#"<p:cxnSp><p:nvCxnSpPr><p:cNvPr id="11" name="Line"/><p:cNvCxnSpPr><a:stCxn id="6" idx="1"/><a:endCxn id="10" idx="3"/></p:cNvCxnSpPr><p:nvPr/></p:nvCxnSpPr><p:spPr><a:prstGeom prst="straightConnector1"><a:avLst/></a:prstGeom></p:spPr></p:cxnSp>"#.to_string(),
    ];
    let slide = format!(
        r#"<p:sld {NS} show="0"><p:cSld name="First"><p:custDataLst><p:tags r:id="rId3"/></p:custDataLst><p:bg><p:bgPr><a:solidFill><a:srgbClr val="00FF00"/></a:solidFill><a:effectLst/></p:bgPr></p:bg><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/>{}</p:spTree></p:cSld></p:sld>"#,
        shapes.concat()
    );
    let theme = r#"<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" name="Harbour"><a:themeElements><a:clrScheme name="Harbour"><a:dk1><a:srgbClr val="000000"/></a:dk1><a:lt1><a:srgbClr val="FFFFFF"/></a:lt1><a:dk2><a:srgbClr val="1F1F1F"/></a:dk2><a:lt2><a:srgbClr val="EEEEEE"/></a:lt2><a:accent1><a:srgbClr val="123456"/></a:accent1><a:accent2><a:srgbClr val="222222"/></a:accent2><a:accent3><a:srgbClr val="333333"/></a:accent3><a:accent4><a:srgbClr val="444444"/></a:accent4><a:accent5><a:srgbClr val="555555"/></a:accent5><a:accent6><a:srgbClr val="666666"/></a:accent6><a:hlink><a:srgbClr val="0000FF"/></a:hlink><a:folHlink><a:srgbClr val="800080"/></a:folHlink></a:clrScheme><a:fontScheme name="Harbour"><a:majorFont><a:latin typeface="Lato"/></a:majorFont><a:minorFont><a:latin typeface="Source Sans 3"/></a:minorFont></a:fontScheme></a:themeElements></a:theme>"#;
    let tags = r#"<p:tagLst xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:tag name="ADDIN" val="42"/></p:tagLst>"#;
    let core = r#"<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>Quarterly</dc:title><dc:creator>Ann</dc:creator><cp:revision>4</cp:revision></cp:coreProperties>"#;
    package(
        &[
            ("ppt/presentation.xml", CT_PRES, pres),
            ("ppt/slideMasters/slideMaster1.xml", "application/vnd.openxmlformats-officedocument.presentationml.slideMaster+xml", master),
            ("ppt/slideLayouts/slideLayout1.xml", "application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml", layout),
            ("ppt/slides/slide1.xml", "application/vnd.openxmlformats-officedocument.presentationml.slide+xml", slide),
            ("ppt/theme/theme1.xml", "application/vnd.openxmlformats-officedocument.theme+xml", theme.into()),
            ("ppt/tags/tag1.xml", "application/vnd.openxmlformats-officedocument.presentationml.tags+xml", tags.into()),
            ("ppt/media/image1.png", "", "png bytes".into()),
            ("docProps/core.xml", "application/vnd.openxmlformats-package.core-properties+xml", core.into()),
        ],
        &[
            (
                "",
                &[
                    ("officeDocument", "ppt/presentation.xml"),
                    ("http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties", "docProps/core.xml"),
                ],
            ),
            (
                "ppt/presentation.xml",
                &[("slideMaster", "slideMasters/slideMaster1.xml"), ("slide", "slides/slide1.xml"), ("slide", "slides/slide2.xml")],
            ),
            ("ppt/slideMasters/slideMaster1.xml", &[("slideLayout", "../slideLayouts/slideLayout1.xml"), ("theme", "../theme/theme1.xml")]),
            ("ppt/slideLayouts/slideLayout1.xml", &[("slideMaster", "../slideMasters/slideMaster1.xml")]),
            (
                "ppt/slides/slide1.xml",
                &[("slideLayout", "../slideLayouts/slideLayout1.xml"), ("image", "../media/image1.png"), ("tags", "../tags/tag1.xml")],
            ),
        ],
    )
}

fn shape<'a>(p: &'a Presentation, name: &str) -> &'a craft_pptx::Shape {
    let mut found = None;
    craft_pptx::walk(&p.slides[0].shapes, &mut |s, _| {
        if s.name == name && found.is_none() {
            found = Some(s);
        }
    });
    found.unwrap_or_else(|| panic!("no shape {name}"))
}

#[test]
fn a_small_deck_reads_into_the_model() {
    let bytes = deck();
    assert!(craft_pptx::sniff(&bytes));
    let (p, lost) = craft_pptx::import_checked(&bytes, &Plain).unwrap();
    assert_eq!(lost, ["ppt/slides/slide2.xml"], "the missing slide is reported and left out");
    assert_eq!(p.slide_size, Size::new(720.0, 540.0));
    assert_eq!(p.first_slide_number, 3);
    assert_eq!(p.props.title, "Quarterly");
    assert_eq!(p.props.author, "Ann");
    assert_eq!(p.props.revision, 4);

    let m = &p.masters[0];
    assert_eq!(m.name, "Main");
    assert_eq!(m.theme.name, "Harbour");
    assert_eq!(m.theme.fonts.major.latin, "Lato");
    assert_eq!(m.title_style.level(0).and_then(|l| l.run.size), Some(44.0));
    assert_eq!(m.layouts.len(), 1);
    assert_eq!(m.layouts[0].kind, LayoutType::Title);

    let s = &p.slides[0];
    assert_eq!(s.layout, m.layouts[0].id);
    assert!(s.hidden);
    assert_eq!(s.name, "First");
    assert_eq!(s.tags, [("ADDIN".to_string(), "42".to_string())]);
    assert!(matches!(&s.background, Some(craft_pptx::Background::Fill { .. })));
    assert_eq!(p.sections.len(), 1);
    assert_eq!(p.sections[0].slides, [s.id]);
    assert_eq!(p.custom_shows[0].name, "Short");
    assert_eq!(p.custom_shows[0].slides, [s.id]);

    assert!(p.validate().is_empty(), "{:?}", p.validate());
    assert!(p.next_id >= 11);

    let (theme, _) = craft_pptx::import_theme(&bytes).unwrap();
    assert_eq!(theme.name, "Harbour");
}

#[test]
fn the_shapes_of_a_small_deck_read_into_the_model() {
    let p = craft_pptx::import(&deck(), &Plain).unwrap();
    // A placeholder with no geometry of its own takes its layout placeholder's.
    let title = shape(&p, "Title 1");
    assert_eq!(title.ph_type(), Some(PhType::CtrTitle));
    assert_eq!(title.geom, Geom::preset("ellipse"));
    assert_eq!(title.text.as_ref().map(|t| t.text()), Some("Hello".into()));

    let b = shape(&p, "Box");
    assert!(matches!(&b.geom, Geom::Preset { name, adj } if name == "roundRect" && adj == &[30000.0]));
    assert_eq!(b.xfrm.map(|x| (x.x, x.y, x.w, x.h)), Some((1.0, 2.0, 100.0, 50.0)));
    assert!(b.fill.is_some());
    assert_eq!(b.text.as_ref().and_then(|t| t.body.anchor), Some(craft_pptx::text::Anchor::Top), "shape text sits at the top");

    let pic = shape(&p, "Picture");
    assert_eq!(pic.descr, "A dot");
    let ShapeKind::Picture { fill } = &pic.kind else { panic!("not a picture: {:?}", pic.kind) };
    let media = p.media(fill.media).unwrap();
    assert_eq!((media.name.as_str(), media.content_type.as_str(), media.data.as_slice()), ("image1.png", "image/png", &b"png bytes"[..]));

    let ShapeKind::Table(t) = &shape(&p, "Table").kind else { panic!("not a table") };
    assert_eq!(t.cols, [100.0, 100.0]);
    assert!(t.first_row);
    assert_eq!(t.rows[0].cells[1].text.text(), "B1");

    let g = shape(&p, "Group");
    assert_eq!(g.children().len(), 1);
    let inner = shape(&p, "Inner").id;
    let ShapeKind::Connector { start, end } = shape(&p, "Line").kind else { panic!("not a connector") };
    assert_eq!(start, Some((b.id, 1)));
    assert_eq!(end, Some((inner, 3)));
}

/// A file with no readable master gets the caller's.
#[test]
fn a_deck_without_a_master_takes_the_callers() {
    struct Mine;
    impl Defaults for Mine {
        fn master(&self, size: Size, after: u32) -> (Master, u32) {
            let (mut m, last) = Plain.master(size, after);
            m.name = "Mine".into();
            (m, last)
        }
    }
    let pres = format!(r#"<p:presentation {NS}><p:sldIdLst><p:sldId id="256" r:id="rId1"/></p:sldIdLst></p:presentation>"#);
    let slide = format!(r#"<p:sld {NS}><p:cSld><p:spTree>{}</p:spTree></p:cSld></p:sld>"#, sp(4, "Box", "", XFRM, "x"));
    let bytes = package(
        &[
            ("ppt/presentation.xml", CT_PRES, pres),
            ("ppt/slides/slide1.xml", "application/vnd.openxmlformats-officedocument.presentationml.slide+xml", slide),
        ],
        &[("", &[("officeDocument", "ppt/presentation.xml")]), ("ppt/presentation.xml", &[("slide", "slides/slide1.xml")])],
    );
    let p = craft_pptx::import(&bytes, &Mine).unwrap();
    assert_eq!(p.masters.len(), 1);
    assert_eq!(p.masters[0].name, "Mine");
    assert_eq!(p.slides[0].layout, p.masters[0].layouts[0].id);
    assert!(p.validate().is_empty(), "{:?}", p.validate());
}

#[test]
fn other_files_are_refused() {
    assert!(!craft_pptx::sniff(b"not a zip"));
    assert!(craft_pptx::import(b"not a zip", &Plain).is_err());
    let doc = package(
        &[("word/document.xml", "application/xml", "<w:document xmlns:w=\"w\"/>".into())],
        &[("", &[("officeDocument", "word/document.xml")])],
    );
    assert!(craft_pptx::sniff(&doc), "a package, but not a presentation");
    assert!(matches!(craft_pptx::import(&doc, &Plain), Err(craft_pptx::Error::NotPptx(_))));
    assert!(craft_pptx::import_theme(&doc).is_err());
}

/// Groups nested past 32 levels are cut there; the rest of the slide is read.
#[test]
fn deep_group_nesting_is_cut() {
    let open = r#"<p:grpSp><p:nvGrpSpPr><p:cNvPr id="0" name="G"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/>"#;
    let tree = format!("{}{}{}", open.repeat(40), sp(4, "Deep", "", XFRM, "x"), "</p:grpSp>".repeat(40));
    let pres = format!(r#"<p:presentation {NS}><p:sldIdLst><p:sldId id="256" r:id="rId1"/></p:sldIdLst></p:presentation>"#);
    let slide = format!(r#"<p:sld {NS}><p:cSld><p:spTree>{tree}{}</p:spTree></p:cSld></p:sld>"#, sp(5, "Top", "", XFRM, "y"));
    let bytes = package(
        &[
            ("ppt/presentation.xml", CT_PRES, pres),
            ("ppt/slides/slide1.xml", "application/vnd.openxmlformats-officedocument.presentationml.slide+xml", slide),
        ],
        &[("", &[("officeDocument", "ppt/presentation.xml")]), ("ppt/presentation.xml", &[("slide", "slides/slide1.xml")])],
    );
    let p = craft_pptx::import(&bytes, &Plain).unwrap();
    let mut depth = 0;
    craft_pptx::walk(&p.slides[0].shapes, &mut |_, d| depth = depth.max(d));
    assert_eq!(depth, 32, "33 levels of groups, the last one empty");
    assert!(p.slides[0].shapes.iter().any(|s| s.name == "Top"));
}
