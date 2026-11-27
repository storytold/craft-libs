//! A picture's hyperlink (#5267): Word keeps it on the drawing (`wp:docPr/a:hlinkClick`,
//! ECMA-376 Part 1 §20.4.2.5) rather than in a `w:hyperlink` around the run. Original
//! synthetic packages; the 2×2 picture is generated in code.
use craft_wordmodel::{CharProps, Document, InlineObject, Pos};
use std::io::{Read, Write};
use std::sync::Arc;

fn png() -> Vec<u8> {
    let mut out = Vec::new();
    image::RgbaImage::from_pixel(2, 2, image::Rgba([20, 90, 200, 255]))
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .unwrap();
    out
}

/// An inline picture's run; `link` goes into its `wp:docPr`.
fn picture_run(id: u32, link: &str) -> String {
    format!(
        r#"<w:r><w:drawing><wp:inline distT="0" distB="0" distL="0" distR="0"><wp:extent cx="254000" cy="254000"/><wp:docPr id="{id}" name="Picture {id}">{link}</wp:docPr><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic><pic:nvPicPr><pic:cNvPr id="0" name="square.png"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed="rIdPic"/></pic:blipFill><pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="254000" cy="254000"/></a:xfrm><a:prstGeom prst="rect"/></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r>"#
    )
}

fn package(body: &str) -> Vec<u8> {
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture"><w:body>{body}<w:sectPr/></w:body></w:document>"#
    );
    let rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    let doc_rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rIdPic" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/square.png"/><Relationship Id="rIdWeb" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="https://example.com/picture" TargetMode="External"/><Relationship Id="rIdRun" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="https://example.com/run" TargetMode="External"/></Relationships>"#;
    let types = r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="png" ContentType="image/png"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#;
    let png = png();
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, bytes) in [
        ("[Content_Types].xml", types.as_bytes()),
        ("_rels/.rels", rels.as_bytes()),
        ("word/document.xml", xml.as_bytes()),
        ("word/_rels/document.xml.rels", doc_rels.as_bytes()),
        ("word/media/square.png", png.as_slice()),
    ] {
        zip.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

/// The link on each picture of the first paragraph, in order.
fn picture_links(doc: &Document) -> Vec<Option<String>> {
    let p = doc.body[0].as_para().unwrap();
    p.object_offsets()
        .into_iter()
        .filter(|off| matches!(p.object_at(*off), Some(InlineObject::Image { .. })))
        .map(|off| p.props_of_char(off).link.clone())
        .collect()
}

fn part(bytes: &[u8], name: &str) -> String {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut s = String::new();
    z.by_name(name).unwrap().read_to_string(&mut s).unwrap();
    s
}

#[test]
fn a_link_on_a_pictures_drawing_is_read() {
    let body = format!(
        "<w:p>{}{}{}<w:hyperlink r:id=\"rIdRun\">{}</w:hyperlink></w:p>",
        // Word's way: the link on the drawing.
        picture_run(1, r#"<a:hlinkClick r:id="rIdWeb"/>"#),
        // A click naming a relationship that is no hyperlink, and one naming none.
        picture_run(2, r#"<a:hlinkClick r:id="rIdPic"/>"#),
        picture_run(3, r#"<a:hlinkClick r:id="rIdMissing"/>"#),
        // A link around the run wins over the drawing's.
        picture_run(4, r#"<a:hlinkClick r:id="rIdWeb"/>"#),
    );
    let doc = craft_docx::read(&package(&body)).unwrap();
    assert_eq!(picture_links(&doc), [Some("https://example.com/picture".to_string()), None, None, Some("https://example.com/run".to_string())]);
}

#[test]
fn a_linked_picture_is_saved_with_its_link_on_the_drawing_too() {
    let mut doc = Document::new();
    doc.media.insert("square.png".into(), Arc::new(png()));
    let picture = || InlineObject::Image {
        media: "square.png".into(),
        w: 20.0,
        h: 20.0,
        alt: String::new(),
        float: Default::default(),
        crop: [0.0; 4],
        ole: None,
        fx: Default::default(),
    };
    let linked = |l: &str| CharProps { link: Some(l.to_string()), ..Default::default() };
    // A place in this document stays a `w:hyperlink w:anchor` only: `a:hlinkClick` names a relationship.
    doc.insert_object(&Pos::body(0, 0), picture(), &linked("#Spot")).unwrap();
    doc.insert_object(&Pos::body(0, 0), picture(), &linked("https://example.com/saved")).unwrap();
    let bytes = craft_docx::write(&doc).unwrap();
    let xml = part(&bytes, "word/document.xml");
    let clicks: Vec<&str> = xml.match_indices("<a:hlinkClick ").map(|(i, _)| &xml[i..]).collect();
    assert_eq!(clicks.len(), 1, "{xml}");
    let rid = clicks[0].split('"').nth(1).unwrap();
    let docpr = xml.find("<wp:docPr ").unwrap();
    assert!(docpr < xml.find("<a:hlinkClick ").unwrap() && xml.find("<a:hlinkClick ").unwrap() < xml.find("</wp:docPr>").unwrap(), "{xml}");
    let rels = part(&bytes, "word/_rels/document.xml.rels");
    assert!(rels.contains(&format!(r#"Id="{rid}""#)) && rels.contains(r#"Target="https://example.com/saved""#), "{rels}");
    // The run's own hyperlink names the same relationship, so the address is stored once.
    assert_eq!(rels.matches("https://example.com/saved").count(), 1, "{rels}");
    let back = craft_docx::read(&bytes).unwrap();
    assert_eq!(picture_links(&back), [Some("https://example.com/saved".to_string()), Some("#Spot".to_string())]);
    // The drawing's link alone (as Word writes it) reads back the same.
    let without_run_link = xml.replace(&format!(r#"<w:hyperlink r:id="{rid}" w:history="1">"#), "").replacen("</w:hyperlink>", "", 1);
    assert_ne!(without_run_link, xml);
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes.as_slice())).unwrap();
    let mut out = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for i in 0..z.len() {
        let mut f = z.by_index(i).unwrap();
        let name = f.name().unwrap().to_string();
        let mut data = Vec::new();
        f.read_to_end(&mut data).unwrap();
        if name == "word/document.xml" {
            data = without_run_link.clone().into_bytes();
        }
        out.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        out.write_all(&data).unwrap();
    }
    let edited = out.finish().unwrap().into_inner();
    assert_eq!(picture_links(&craft_docx::read(&edited).unwrap())[0], Some("https://example.com/saved".to_string()));
}

// Original synthetic VML package for #2912, reported by denstu473; no reference-app asset.
#[test]
fn a_vml_picture_keeps_its_crop_when_saved() {
    fn image_crop(doc: &Document, original_png: &[u8]) -> [f32; 4] {
        let paragraph = doc.body[0].as_para().expect("the synthetic picture's paragraph");
        assert_eq!(paragraph.objects.len(), 1, "the VML picture must be decoded before testing crop");
        let InlineObject::Image { media, w, h, alt, crop, fx, .. } = &paragraph.objects[0] else {
            panic!("the synthetic object must be an image");
        };
        assert!((*w - 24.0).abs() < 0.001 && (*h - 12.0).abs() < 0.001, "picture size changed: {w} x {h}");
        assert_eq!(alt, "cropped square");
        assert_eq!(doc.media.get(media).expect("the picture's embedded media").as_slice(), original_png);
        assert_eq!(*fx, craft_wordmodel::picfx::PicFx::new(40_000, 0, 0));
        *crop
    }
    fn crops_near(actual: [f32; 4], expected: [f32; 4], tolerance: f32) -> bool {
        actual.into_iter().zip(expected).all(|(a, b)| (a - b).abs() <= tolerance)
    }

    let original_png = png();
    let mut lost = Vec::new();
    // Crop slots are left, top, right, bottom. These distinct fractions are exactly binary.
    let positive = [0.125, 0.0625, 0.25, 0.5];
    for (label, attributes, expected, saved_rect) in [
        ("default", "", [0.0; 4], None),
        (
            "report fixed fractions",
            r#"cropleft="8192f" croptop="3277f" cropright="16384f" cropbottom="6554f""#,
            [8192.0f32 / 65_536.0, 3277.0 / 65_536.0, 16384.0 / 65_536.0, 6554.0 / 65_536.0],
            Some(r#"<a:srcRect l="12500" t="5000" r="25000" b="10001"/>"#),
        ),
        (
            "fixed fractions",
            r#"cropleft="8192f" croptop="4096f" cropright="16384f" cropbottom="32768f""#,
            positive,
            Some(r#"<a:srcRect l="12500" t="6250" r="25000" b="50000"/>"#),
        ),
        (
            "decimal fractions",
            r#"cropleft="0.125" croptop="0.0625" cropright="0.25" cropbottom="0.5""#,
            positive,
            Some(r#"<a:srcRect l="12500" t="6250" r="25000" b="50000"/>"#),
        ),
        (
            "percentage fractions",
            r#"cropleft="12.5%" croptop="6.25%" cropright="25%" cropbottom="50%""#,
            positive,
            Some(r#"<a:srcRect l="12500" t="6250" r="25000" b="50000"/>"#),
        ),
        (
            "bounded fractions",
            r#"cropleft="-25%" croptop="125%" cropright="not-a-fraction" cropbottom="NaN""#,
            [0.0, 1.0, 0.0, 0.0],
            Some(r#"<a:srcRect l="0" t="100000" r="0" b="0"/>"#),
        ),
    ] {
        // The local namespace keeps the existing package helper and every old fixture intact.
        let body = format!(
            r#"<w:p><w:r><w:pict xmlns:v="urn:schemas-microsoft-com:vml"><v:shape style="width:24pt;height:12pt" alt="cropped square"><v:imagedata r:id="rIdPic" gain="1" blacklevel="0.2" {attributes}/></v:shape></w:pict></w:r></w:p>"#
        );
        let doc = craft_docx::read(&package(&body)).unwrap();
        let read_crop = image_crop(&doc, &original_png);
        let saved = craft_docx::write(&doc).unwrap();
        let xml = part(&saved, "word/document.xml");
        let reopened = craft_docx::read(&saved).unwrap();
        let reopened_crop = image_crop(&reopened, &original_png);

        if let Some(saved_rect) = saved_rect {
            if !crops_near(read_crop, expected, 0.000_001) {
                lost.push(format!("{label}: read crop {read_crop:?}"));
            }
            if !xml.contains(saved_rect) {
                lost.push(format!("{label}: saved srcRect"));
            }
            // DrawingML uses 100000 units per fraction; allow its rounding on a real re-read.
            if !crops_near(reopened_crop, expected, 0.000_01) {
                lost.push(format!("{label}: reopened crop {reopened_crop:?}"));
            }
        } else {
            // The zero/default control must work before any positive-crop regression is reported.
            assert_eq!(read_crop, [0.0; 4]);
            assert!(!xml.contains("<a:srcRect"), "{xml}");
            assert_eq!(reopened_crop, [0.0; 4]);
        }
    }
    assert!(lost.is_empty(), "VML picture crops were lost: {lost:?}");
}

// Original Transitional DOCX/VML: ISO 29500 VML shape/group href attributes, with
// independent literal targets at each level. No reference-app file or click oracle.
#[test]
fn vml_objects_keep_their_own_hyperlinks() {
    fn check_images(doc: &Document, object: &InlineObject, original_png: &[u8], out: &mut Vec<(String, f32, f32)>) {
        match object {
            InlineObject::Image { media, w, h, alt, crop, .. } => {
                assert_eq!(doc.media.get(media).expect("the original embedded picture").as_slice(), original_png);
                assert_eq!(*crop, [0.0; 4], "a hyperlink must not invent a crop");
                out.push((alt.clone(), *w, *h));
            }
            InlineObject::Group { children, .. } => {
                for child in children {
                    check_images(doc, &child.obj, original_png, out);
                }
            }
            InlineObject::Shape { kind, w, h, .. } => {
                assert_eq!(*kind, craft_wordmodel::para::ShapeKind::Rectangle);
                assert!((*w - 5.0).abs() < 0.001 && (*h - 7.0).abs() < 0.001);
            }
            other => panic!("the original picture fixture produced another object: {other:?}"),
        }
    }
    fn top_links(doc: &Document) -> Vec<Option<String>> {
        let paragraph = doc.body[1].as_para().expect("the VML paragraph");
        paragraph.object_offsets().into_iter().map(|off| paragraph.props_of_char(off).link.clone()).collect()
    }

    fn model_links(doc: &Document) -> Vec<(Vec<usize>, Option<String>)> {
        fn member_links(object: &InlineObject, path: &[usize], out: &mut Vec<(Vec<usize>, Option<String>)>) {
            if let InlineObject::Group { children, .. } = object {
                for (i, child) in children.iter().enumerate() {
                    let mut path = path.to_vec();
                    path.push(i);
                    out.push((path.clone(), child.link.clone()));
                    member_links(&child.obj, &path, out);
                }
            }
        }
        let paragraph = doc.body[1].as_para().unwrap();
        let mut out = Vec::new();
        for (i, off) in paragraph.object_offsets().into_iter().enumerate() {
            out.push((vec![i], paragraph.props_of_char(off).link.clone()));
            member_links(paragraph.object_at(off).unwrap(), &[i], &mut out);
        }
        out
    }
    fn xml_link(nv: Option<&craft_xml::El>, rels: &craft_xml::El) -> Option<String> {
        let id = nv?.child("a:hlinkClick")?.attr("r:id")?;
        let rel = rels.children("Relationship").find(|r| r.attr("Id") == Some(id)).expect("every click must name a saved relationship");
        assert_eq!(rel.attr("Type"), Some("http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink"));
        assert_eq!(rel.attr("TargetMode"), Some("External"));
        Some(rel.attr("Target").expect("the click target").to_string())
    }
    fn saved_member_links(group: &craft_xml::El, path: &[usize], rels: &craft_xml::El, out: &mut Vec<(Vec<usize>, Option<String>)>) {
        for (i, child) in group.elements().filter(|e| matches!(e.name.as_str(), "pic:pic" | "wps:wsp" | "wpg:grpSp")).enumerate() {
            let mut path = path.to_vec();
            path.push(i);
            let nv = match child.name.as_str() {
                "pic:pic" => child.child("pic:nvPicPr").and_then(|p| p.child("pic:cNvPr")),
                "wps:wsp" => child.child("wps:cNvPr"),
                _ => child.child("wpg:cNvPr"),
            };
            out.push((path.clone(), xml_link(nv, rels)));
            if child.name == "wpg:grpSp" {
                saved_member_links(child, &path, rels, out);
            }
        }
    }

    fn check_group_geometry(current: &Document) {
        let InlineObject::Group { children, .. } = &current.body[1].as_para().unwrap().objects[3] else { panic!() };
        assert_eq!(children.len(), 3);
        assert!((children[0].x - 3.0).abs() < 0.001 && (children[0].y - 4.0).abs() < 0.001);
        assert!((children[2].x - 20.0).abs() < 0.001 && (children[2].y - 5.0).abs() < 0.001);
        let InlineObject::Group { children, float, ch_w, ch_h, .. } = &children[2].obj else { panic!("the linked nested group must remain a group") };
        assert_eq!((*ch_w, *ch_h), (10.0, 10.0));
        assert_eq!(float.spin(), craft_wordmodel::geom::Spin::new(30.0, true, false));
        assert_eq!(children.len(), 1);
        assert!((children[0].x - 1.0).abs() < 0.001 && (children[0].y - 2.0).abs() < 0.001);
    }

    let vml = r##"<w:p><w:r><w:pict xmlns:v="urn:schemas-microsoft-com:vml">
<v:shape id="TopFirst" style="width:24pt;height:12pt" alt="first" href="https://example.com/TopFirst"><v:imagedata r:id="rIdPic"/></v:shape>
<v:shape id="Unlinked" style="width:16pt;height:8pt" alt="unlinked"><v:imagedata r:id="rIdPic"/></v:shape>
<v:shape id="TopLast" style="width:12pt;height:6pt" alt="last" href="https://example.com/TopLast"><v:imagedata r:id="rIdPic"/></v:shape>
<v:group id="Outer" style="width:40pt;height:20pt" coordsize="40,20" href="https://example.com/Outer">
<v:shape id="Child" style="left:3pt;top:4pt;width:6pt;height:4pt" alt="child" href="https://example.com/Child"><v:imagedata r:id="rIdPic"/></v:shape>
<v:rect id="ChildShape" style="left:12pt;top:2pt;width:5pt;height:7pt" fillcolor="#123456" href="https://example.com/Shape"/>
<v:group id="Nested" style="left:20pt;top:5pt;width:10pt;height:10pt;rotation:30;flip:x" coordsize="10,10" href="https://example.com/Nested">
<v:shape id="Fragment" style="left:1pt;top:2pt;width:2pt;height:3pt" alt="fragment" href="#Spot"><v:imagedata r:id="rIdPic"/></v:shape>
</v:group></v:group></w:pict></w:r></w:p>"##;
    // A known DrawingML link is a positive control in the same package, with the
    // same generated media and relationship table as the independently linked VML.
    let body = format!("<w:p>{}</w:p>{vml}", picture_run(10, r#"<a:hlinkClick r:id="rIdWeb"/>"#));
    let original_png = png();
    let doc = craft_docx::read(&package(&body)).expect("the original Transitional package must read");
    let saved = craft_docx::write(&doc).expect("the supported picture/group content must save");
    let reopened = craft_docx::read(&saved).expect("the normalized DOCX must reopen");
    let expected_images = [("first", 24.0, 12.0), ("unlinked", 16.0, 8.0), ("last", 12.0, 6.0), ("child", 6.0, 4.0), ("fragment", 2.0, 3.0)];

    // All payload, ordering, geometry and positive/unlinked controls run before
    // the missing-target assertion; neither a dropped object nor bad media is red proof.
    for current in [&doc, &reopened] {
        assert_eq!(current.body.len(), 2);
        assert_eq!(picture_links(current), [Some("https://example.com/picture".to_string())]);
        let paragraph = current.body[1].as_para().expect("the VML paragraph remains a paragraph");
        assert_eq!(paragraph.objects.len(), 4, "three top-level pictures and their following group");
        let InlineObject::Group { w, h, .. } = &paragraph.objects[3] else {
            panic!("the last VML object must remain a group");
        };
        assert!((*w - 40.0).abs() < 0.001 && (*h - 20.0).abs() < 0.001);
        let mut images = Vec::new();
        for object in &paragraph.objects {
            check_images(current, object, &original_png, &mut images);
        }
        assert_eq!(images.len(), expected_images.len(), "every original picture must remain");
        for ((alt, w, h), (expected_alt, expected_w, expected_h)) in images.iter().zip(expected_images) {
            assert_eq!(alt, expected_alt, "picture order/identity changed");
            assert!((w - expected_w).abs() < 0.001 && (h - expected_h).abs() < 0.001, "{alt}: {w} x {h}");
        }
        assert_eq!(top_links(current)[1], None, "the unlinked neighbor must stay unlinked");
    }
    let rels = part(&saved, "word/_rels/document.xml.rels");
    assert!(rels.contains(r#"Target="https://example.com/picture""#), "the DrawingML control's relationship must survive");

    let expected_top = [Some("https://example.com/TopFirst"), None, Some("https://example.com/TopLast"), Some("https://example.com/Outer")];
    let mut lost = Vec::new();
    for (phase, current) in [("read", &doc), ("reopened", &reopened)] {
        for (index, (actual, expected)) in top_links(current).iter().zip(expected_top).enumerate() {
            if actual.as_deref() != expected {
                lost.push(format!("{phase} top-level object {index}: {actual:?} instead of {expected:?}"));
            }
        }
    }
    // These are necessary persistence checks using today's APIs, not a claim that
    // relationship presence proves nested path-to-target association or activation.
    for (label, target) in [
        ("first", "https://example.com/TopFirst"),
        ("last", "https://example.com/TopLast"),
        ("outer group", "https://example.com/Outer"),
        ("group child", "https://example.com/Child"),
        ("group shape", "https://example.com/Shape"),
        ("nested group", "https://example.com/Nested"),
        ("fragment data", "#Spot"),
    ] {
        if !rels.contains(&format!(r#"Target="{target}""#)) {
            lost.push(format!("saved {label} target: {target}"));
        }
    }
    let expected: Vec<_> = [
        (vec![0], Some("https://example.com/TopFirst")),
        (vec![1], None),
        (vec![2], Some("https://example.com/TopLast")),
        (vec![3], Some("https://example.com/Outer")),
        (vec![3, 0], Some("https://example.com/Child")),
        (vec![3, 1], Some("https://example.com/Shape")),
        (vec![3, 2], Some("https://example.com/Nested")),
        (vec![3, 2, 0], Some("#Spot")),
    ]
    .into_iter()
    .map(|(path, link)| (path, link.map(str::to_string)))
    .collect();
    for current in [&doc, &reopened] {
        assert_eq!(model_links(current), expected, "each target must remain on its own object path");
        check_group_geometry(current);
    }
    let xml = craft_xml::parse(part(&saved, "word/document.xml").as_bytes()).unwrap();
    let relationships = craft_xml::parse(rels.as_bytes()).unwrap();
    let paragraph = xml.child("w:body").unwrap().children("w:p").nth(1).unwrap();
    let mut xml_links = Vec::new();
    for (i, drawing) in paragraph.find_all("w:drawing").enumerate() {
        let frame = drawing.child("wp:inline").or_else(|| drawing.child("wp:anchor")).unwrap();
        xml_links.push((vec![i], xml_link(frame.child("wp:docPr"), &relationships)));
        if let Some(group) = frame.child("a:graphic").and_then(|g| g.child("a:graphicData")).and_then(|d| d.child("wpg:wgp")) {
            saved_member_links(group, &[i], &relationships, &mut xml_links);
        }
    }
    assert_eq!(xml_links, expected, "saved cNvPr/docPr clicks must name the target at the same path");
    assert!(lost.is_empty(), "VML object hyperlinks were lost: {lost:?}");
}

#[test]
fn vml_hyperlink_templates_and_outer_runs_keep_their_scope() {
    let cases = [
        (r##"type="#Linked""##, "", Some("https://example.com/Template")),
        (r##"type="#Linked" href="https://example.com/Direct""##, "", Some("https://example.com/Direct")),
        (r##"type="#Linked" href="   ""##, "", None),
        (r#"href="  https://example.com/Trimmed  ""#, "", Some("https://example.com/Trimmed")),
        ("", r#"o:href="https://example.com/ImageSource" r:href="rIdWeb""#, None),
    ];
    let mut shapes = String::new();
    for (attributes, image_attributes, _) in cases {
        shapes
            .push_str(&format!(r#"<v:shape style="width:24pt;height:12pt" {attributes}><v:imagedata r:id="rIdPic" {image_attributes}/></v:shape>"#,));
    }
    let body = format!(
        r#"<w:p><w:r><w:pict xmlns:v="urn:schemas-microsoft-com:vml" xmlns:o="urn:schemas-microsoft-com:office:office"><v:shapetype id="Linked" href="  https://example.com/Template  "/>{shapes}</w:pict></w:r></w:p><w:p><w:hyperlink r:id="rIdRun"><w:r><w:pict xmlns:v="urn:schemas-microsoft-com:vml"><v:group style="width:24pt;height:12pt" coordsize="24,12" href="https://example.com/Group"><v:shape style="width:24pt;height:12pt" href="https://example.com/Child"><v:imagedata r:id="rIdPic"/></v:shape></v:group></w:pict></w:r></w:hyperlink></w:p>"#,
    );
    let doc = craft_docx::read(&package(&body)).unwrap();
    let saved = craft_docx::write(&doc).unwrap();
    let reopened = craft_docx::read(&saved).unwrap();
    let expected: Vec<_> = cases.into_iter().map(|(_, _, link)| link.map(str::to_string)).collect();
    for current in [&doc, &reopened] {
        assert_eq!(picture_links(current), expected, "presence-aware template/direct href precedence");
        let paragraph = current.body[1].as_para().unwrap();
        assert_eq!(paragraph.objects.len(), 1);
        assert_eq!(paragraph.props_of_char(paragraph.object_offsets()[0]).link.as_deref(), Some("https://example.com/run"));
        let InlineObject::Group { children, .. } = &paragraph.objects[0] else { panic!() };
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].link.as_deref(), Some("https://example.com/Child"), "the run overrides the group, not its child");
    }

    // A shape type may be declared inside the group rather than beside it. These
    // original inputs use unique IDs and keep image-source URLs out of click lookup.
    let local_cases = [(r##"type="#GroupLocal""##, Some("https://example.com/GroupTemplate")), (r##"type="#NotSelected""##, None)];
    let mut local_shapes = String::new();
    for (attributes, _) in local_cases {
        local_shapes.push_str(&format!(
            r#"<v:shape style="width:6pt;height:5pt" {attributes}><v:imagedata r:id="rIdPic" o:href="https://example.com/ImageSource" r:href="rIdWeb"/></v:shape>"#,
        ));
    }
    let local_body = format!(
        r##"<w:p><w:r><w:pict xmlns:v="urn:schemas-microsoft-com:vml" xmlns:o="urn:schemas-microsoft-com:office:office" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:other="urn:original-test:unselected"><mc:AlternateContent><mc:Choice Requires="other"><v:shapetype id="NotSelected" href="https://example.com/Unselected"/></mc:Choice><mc:Fallback/></mc:AlternateContent><v:group style="width:36pt;height:10pt" coordsize="36,10"><v:shapetype id="GroupLocal" href="  https://example.com/GroupTemplate  "/>{local_shapes}<v:group style="left:12pt;top:5pt;width:6pt;height:5pt" coordsize="6,5" href="https://example.com/NestedTemplateGroup"><v:shapetype id="DeepLocal" href="https://example.com/DeepTemplate"/><v:shape type="#DeepLocal" style="width:6pt;height:5pt"><v:imagedata r:id="rIdPic"/></v:shape></v:group></v:group></w:pict></w:r></w:p>"##,
    );
    let local_doc = craft_docx::read(&package(&local_body)).unwrap();
    let local_saved = craft_docx::write(&local_doc).unwrap();
    let local_reopened = craft_docx::read(&local_saved).unwrap();
    let original_png = png();
    for current in [&local_doc, &local_reopened] {
        assert_eq!(current.body.len(), 1);
        let paragraph = current.body[0].as_para().unwrap();
        assert_eq!(paragraph.objects.len(), 1);
        assert_eq!(paragraph.props_of_char(0).link, None, "a template must not become its containing group's link");
        let InlineObject::Group { children, .. } = &paragraph.objects[0] else { panic!() };
        assert_eq!(children.len(), local_cases.len() + 1);
        for (child, (_, expected)) in children.iter().zip(local_cases) {
            let InlineObject::Image { media, w, h, .. } = &child.obj else { panic!() };
            assert_eq!(current.media.get(media).unwrap().as_slice(), original_png.as_slice());
            assert!((*w - 6.0).abs() < 0.001 && (*h - 5.0).abs() < 0.001);
            assert_eq!(child.link.as_deref(), expected, "group-local template href");
        }
        let nested = &children[local_cases.len()];
        assert_eq!(nested.link.as_deref(), Some("https://example.com/NestedTemplateGroup"));
        let InlineObject::Group { children: nested_children, .. } = &nested.obj else { panic!() };
        assert_eq!(nested_children.len(), 1);
        assert_eq!(nested_children[0].link.as_deref(), Some("https://example.com/DeepTemplate"), "nested group-local template href");
    }
}

fn replacing_picture_part(bytes: &[u8], name: &str, replacement: &str) -> Vec<u8> {
    let mut input = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut output = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for i in 0..input.len() {
        let mut file = input.by_index(i).unwrap();
        let entry = file.name().unwrap().to_string();
        let mut data = Vec::new();
        file.read_to_end(&mut data).unwrap();
        if entry == name {
            data = replacement.as_bytes().to_vec();
        }
        output.start_file(entry, zip::write::SimpleFileOptions::default()).unwrap();
        output.write_all(&data).unwrap();
    }
    output.finish().unwrap().into_inner()
}

#[test]
fn group_picture_links_require_their_own_external_hyperlink_relationship() {
    let body = r#"<w:p><w:r><w:pict xmlns:v="urn:schemas-microsoft-com:vml"><v:group style="width:24pt;height:12pt" coordsize="24,12"><v:shape style="width:24pt;height:12pt" href="https://example.com/Child"><v:imagedata r:id="rIdPic"/></v:shape></v:group></w:pict></w:r></w:p>"#;
    let doc = craft_docx::read(&package(body)).unwrap();
    let saved = craft_docx::write(&doc).unwrap();
    let xml = part(&saved, "word/document.xml");
    let rels = part(&saved, "word/_rels/document.xml.rels");
    let tree = craft_xml::parse(xml.as_bytes()).unwrap();
    let nv = tree.find("pic:pic").unwrap().child("pic:nvPicPr").unwrap().child("pic:cNvPr").unwrap();
    let click = nv.child("a:hlinkClick").expect("the valid child's click must be on its cNvPr");
    let id = click.attr("r:id").unwrap();
    let original_png = png();
    let check = |bytes: &[u8], expected: Option<&str>| {
        let doc = craft_docx::read(bytes).unwrap();
        let paragraph = doc.body[0].as_para().unwrap();
        assert_eq!(paragraph.objects.len(), 1);
        assert_eq!(paragraph.props_of_char(0).link, None, "an unlinked group must not borrow its child's link");
        let InlineObject::Group { children, .. } = &paragraph.objects[0] else { panic!() };
        assert_eq!(children.len(), 1);
        let InlineObject::Image { media, w, h, .. } = &children[0].obj else { panic!() };
        assert_eq!(doc.media.get(media).unwrap().as_slice(), original_png.as_slice());
        assert!((*w - 24.0).abs() < 0.001 && (*h - 12.0).abs() < 0.001);
        assert_eq!(children[0].link.as_deref(), expected);
    };
    check(&saved, Some("https://example.com/Child"));
    let type_name = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink";
    for modified in [
        rels.replace(type_name, "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image"),
        rels.replace(" TargetMode=\"External\"", ""),
        rels.replace("Target=\"https://example.com/Child\"", "Target=\"   \""),
        rels.replace("https://example.com/Child", &format!("https://example.com/{}", "x".repeat(8 << 10))),
    ] {
        assert_ne!(modified, rels);
        check(&replacing_picture_part(&saved, "word/_rels/document.xml.rels", &modified), None);
    }
    let missing = xml.replace(&format!(r#"r:id="{id}""#), r#"r:id="rIdAbsent""#);
    assert_ne!(missing, xml);
    check(&replacing_picture_part(&saved, "word/document.xml", &missing), None);
    // A click in an unrelated extension is not the picture's nonvisual properties.
    let mut unrelated = tree.clone();
    let picture = unrelated
        .child_mut("w:body")
        .unwrap()
        .child_mut("w:p")
        .unwrap()
        .child_mut("w:r")
        .unwrap()
        .child_mut("w:drawing")
        .unwrap()
        .child_mut("wp:inline")
        .unwrap()
        .child_mut("a:graphic")
        .unwrap()
        .child_mut("a:graphicData")
        .unwrap()
        .child_mut("wpg:wgp")
        .unwrap()
        .child_mut("pic:pic")
        .unwrap();
    picture.child_mut("pic:nvPicPr").unwrap().child_mut("pic:cNvPr").unwrap().children.clear();
    let extension = craft_xml::parse(format!(r#"<a:extLst xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><a:ext uri="urn:original-test:unrelated"><a:hlinkClick r:id="{id}"/></a:ext></a:extLst>"#).as_bytes()).unwrap();
    picture.child_mut("pic:spPr").unwrap().push(extension);
    check(&replacing_picture_part(&saved, "word/document.xml", &unrelated.to_xml()), None);
}

#[test]
fn linked_groups_share_one_retained_member_budget() {
    fn count(object: &InlineObject) -> usize {
        match object {
            InlineObject::Group { children, .. } => children.iter().map(|c| 1 + count(&c.obj)).sum(),
            _ => 0,
        }
    }
    fn input(per_group: usize) -> Vec<u8> {
        let leaf = r#"<v:shape style="width:1pt;height:1pt"><v:imagedata r:id="rIdPic"/></v:shape>"#.repeat(per_group);
        package(&format!(
            r#"<w:p><w:r><w:pict xmlns:v="urn:schemas-microsoft-com:vml"><v:group style="width:24pt;height:12pt" coordsize="24,12"><v:group style="width:12pt;height:12pt" coordsize="12,12" href="https://example.com/Left">{leaf}</v:group><v:group style="left:12pt;width:12pt;height:12pt" coordsize="12,12" href="https://example.com/Right">{leaf}</v:group></v:group></w:pict></w:r></w:p>"#,
        ))
    }
    let small = craft_docx::read(&input(1)).unwrap();
    assert_eq!(count(&small.body[0].as_para().unwrap().objects[0]), 4, "two linked groups and two picture leaves");
    let saved_small = craft_docx::write(&small).unwrap();
    let reopened_small = craft_docx::read(&saved_small).unwrap();
    assert_eq!(count(&reopened_small.body[0].as_para().unwrap().objects[0]), 4);
    let max = craft_wordmodel::para::MAX_GROUP_CHILDREN;
    assert_eq!(max, 1000, "this original fixture is bounded to the existing limit plus two nodes");
    let large = craft_docx::read(&input(500)).unwrap();
    assert_eq!(count(&large.body[0].as_para().unwrap().objects[0]), max, "VML must count linked nodes and leaves together");
    // Independently construct the same 1002-node DrawingML input from our small
    // original save, so the VML reader's own bound cannot hide a DML budget regression.
    let mut xml = craft_xml::parse(part(&saved_small, "word/document.xml").as_bytes()).unwrap();
    let group = xml
        .child_mut("w:body")
        .unwrap()
        .child_mut("w:p")
        .unwrap()
        .child_mut("w:r")
        .unwrap()
        .child_mut("w:drawing")
        .unwrap()
        .child_mut("wp:inline")
        .unwrap()
        .child_mut("a:graphic")
        .unwrap()
        .child_mut("a:graphicData")
        .unwrap()
        .child_mut("wpg:wgp")
        .unwrap();
    for child in group.elements_mut().filter(|e| e.name == "wpg:grpSp") {
        let picture = child.child("pic:pic").unwrap().clone();
        child.children.retain(|n| !matches!(n, craft_xml::Node::El(e) if e.name == "pic:pic"));
        for _ in 0..500 {
            child.push(picture.clone());
        }
    }
    let dml = replacing_picture_part(&saved_small, "word/document.xml", &xml.to_xml());
    let doc = craft_docx::read(&dml).unwrap();
    assert_eq!(count(&doc.body[0].as_para().unwrap().objects[0]), max, "DrawingML must share the same root-wide bound");
}

/// An omitted linked group returns its reserved slot before the following drawable is read.
#[test]
fn empty_linked_groups_release_the_last_member_slot() {
    let max = craft_wordmodel::para::MAX_GROUP_CHILDREN;
    let before = r#"<v:shape style="width:1pt;height:1pt" alt="before-empty"><v:imagedata r:id="rIdPic"/></v:shape>"#;
    let after = r#"<v:shape style="width:1pt;height:1pt" alt="after-empty"><v:imagedata r:id="rIdPic"/></v:shape>"#;
    let empty = r#"<v:group style="width:1pt;height:1pt" coordsize="1,1" href="https://example.com/Empty"/>"#;
    let input = |members: &str| {
        package(&format!(
            r#"<w:p><w:r><w:pict xmlns:v="urn:schemas-microsoft-com:vml"><v:group style="width:24pt;height:12pt" coordsize="24,12">{members}</v:group></w:pict></w:r></w:p>"#,
        ))
    };
    let check = |doc: &Document| {
        let InlineObject::Group { children, .. } = &doc.body[0].as_para().unwrap().objects[0] else {
            panic!("the original root group must remain");
        };
        assert_eq!(children.len(), max, "the empty linked group must return the last member slot");
        assert!(children.iter().all(|child| matches!(&child.obj, InlineObject::Image { .. }) && child.link.is_none()));
        assert!(
            matches!(&children.last().unwrap().obj, InlineObject::Image { alt, .. } if alt == "after-empty"),
            "the drawable after the omitted group must survive"
        );
    };
    let vml = input(&format!("{}{empty}{after}", before.repeat(max - 1)));
    check(&craft_docx::read(&vml).unwrap());

    // Save a small, nonempty linked group to obtain normalized DrawingML with its relationship.
    // Then remove its drawable and fill the root independently of the VML boundary input.
    let seed =
        input(&format!(r#"<v:group style="width:1pt;height:1pt" coordsize="1,1" href="https://example.com/Empty">{before}</v:group>{after}"#,));
    let saved = craft_docx::write(&craft_docx::read(&seed).unwrap()).unwrap();
    let mut xml = craft_xml::parse(part(&saved, "word/document.xml").as_bytes()).unwrap();
    let group = xml
        .child_mut("w:body")
        .unwrap()
        .child_mut("w:p")
        .unwrap()
        .child_mut("w:r")
        .unwrap()
        .child_mut("w:drawing")
        .unwrap()
        .child_mut("wp:inline")
        .unwrap()
        .child_mut("a:graphic")
        .unwrap()
        .child_mut("a:graphicData")
        .unwrap()
        .child_mut("wpg:wgp")
        .unwrap();
    let mut empty = group.child("wpg:grpSp").unwrap().clone();
    let filler = empty.child("pic:pic").unwrap().clone();
    let last = group.child("pic:pic").unwrap().clone();
    assert!(empty.child("wpg:cNvPr").unwrap().child("a:hlinkClick").is_some(), "exercise the linked-group reservation path");
    empty.children.retain(|n| !matches!(n, craft_xml::Node::El(e) if e.name == "pic:pic"));
    group.children.retain(|n| !matches!(n, craft_xml::Node::El(e) if matches!(e.name.as_str(), "wpg:grpSp" | "pic:pic")));
    for _ in 0..max - 1 {
        group.push(filler.clone());
    }
    group.push(empty);
    group.push(last);
    let dml = replacing_picture_part(&saved, "word/document.xml", &xml.to_xml());
    check(&craft_docx::read(&dml).unwrap());
}

#[test]
fn linked_group_coordinates_keep_their_scale() {
    fn image_rect(object: &InlineObject, x: f32, scale: f32) -> (f32, f32) {
        match object {
            InlineObject::Image { w, .. } => (x, w * scale),
            InlineObject::Group { w, ch_w, children, .. } => {
                assert_eq!(children.len(), 1);
                let inner = scale * w / ch_w;
                image_rect(&children[0].obj, x + children[0].x * inner, inner)
            }
            _ => panic!("expected a picture or its group"),
        }
    }
    let picture = picture_run(1, "");
    let picture = &picture[picture.find("<pic:pic>").unwrap()..picture.find("</pic:pic>").unwrap() + "</pic:pic>".len()];
    let picture = picture
        .replace(r#"<a:off x="0" y="0"/><a:ext cx="254000" cy="254000"/>"#, r#"<a:off x="114300000" y="0"/><a:ext cx="12700000" cy="12700000"/>"#);
    for linked in [false, true] {
        let vml_link = if linked { r#"href="https://example.com/subgroup""# } else { "" };
        let dml_link = if linked { r#"<a:hlinkClick r:id="rIdWeb"/>"# } else { "" };
        let vml = format!(
            r#"<w:p><w:r><w:pict xmlns:v="urn:schemas-microsoft-com:vml"><v:group style="width:200pt;height:200pt" coordsize="200,200"><v:group style="width:100pt;height:100pt" coordsize="10000,10000" coordorigin="-7000,0" {vml_link}><v:shape style="left:9000;width:1000;height:1000"><v:imagedata r:id="rIdPic"/></v:shape></v:group></v:group></w:pict></w:r></w:p>"#,
        );
        let dml = format!(
            r#"<w:p><w:r><w:drawing><wp:inline><wp:extent cx="2540000" cy="2540000"/><wp:docPr id="1" name="Group"/><a:graphic><a:graphicData uri="http://schemas.microsoft.com/office/word/2010/wordprocessingGroup"><wpg:wgp xmlns:wpg="http://schemas.microsoft.com/office/word/2010/wordprocessingGroup"><wpg:cNvGrpSpPr/><wpg:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="2540000" cy="2540000"/><a:chOff x="0" y="0"/><a:chExt cx="2540000" cy="2540000"/></a:xfrm></wpg:grpSpPr><wpg:grpSp><wpg:cNvPr id="2" name="Subgroup">{dml_link}</wpg:cNvPr><wpg:cNvGrpSpPr/><wpg:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="1270000" cy="1270000"/><a:chOff x="-88900000" y="0"/><a:chExt cx="127000000" cy="127000000"/></a:xfrm></wpg:grpSpPr>{picture}</wpg:grpSp></wpg:wgp></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>"#,
        );
        for body in [vml, dml] {
            let doc = craft_docx::read(&package(&body)).unwrap();
            let reopened = craft_docx::read(&craft_docx::write(&doc).unwrap()).unwrap();
            for current in [&doc, &reopened] {
                let (x, w) = image_rect(&current.body[0].as_para().unwrap().objects[0], 0.0, 1.0);
                assert!((x - 160.0).abs() < 0.01, "the rebased offset must be scaled before clipping: {x}");
                assert!((w - 10.0).abs() < 0.01, "the picture keeps its size in the same coordinate system: {w}");
            }
        }
    }
}

#[test]
fn inherited_drawing_targets_share_a_document_byte_budget() {
    let target = format!("https://example.com/{}", "x".repeat((8 << 10) - "https://example.com/".len()));
    let leaf = r##"<v:shape type="#Target" style="width:1pt;height:1pt"><v:imagedata r:id="rIdPic"/></v:shape>"##.repeat(1000);
    let paragraph = format!(
        r#"<w:p><w:r><w:pict xmlns:v="urn:schemas-microsoft-com:vml"><v:shapetype id="Target" href="{target}"/><v:group style="width:100pt;height:100pt" coordsize="100,100">{leaf}</v:group></w:pict></w:r></w:p>"#,
    );
    let doc = craft_docx::read(&package(&paragraph.repeat(9))).unwrap();
    let mut bytes = 0;
    let mut retained = 0;
    let mut dropped = 0;
    for block in &doc.body {
        let InlineObject::Group { children, .. } = &block.as_para().unwrap().objects[0] else { panic!() };
        assert_eq!(children.len(), 1000, "a target budget must not discard the drawing");
        for child in children {
            assert!(matches!(child.obj, InlineObject::Image { .. }));
            if let Some(link) = &child.link {
                assert_eq!(link, &target);
                bytes += link.len();
                retained += 1;
            } else {
                dropped += 1;
            }
        }
    }
    assert!(retained > 0 && dropped > 0, "the same budget must span every group in the document");
    assert!(bytes <= 64 << 20);
}

#[test]
fn native_ink_keeps_its_vml_preview_target_without_overriding_its_own() {
    for own_target in [false, true] {
        let input = super::fixtures::ink_docx(true, false);
        let mut xml = part(&input, "word/document.xml");
        xml = xml.replace(r#"<v:shape id="preview""#, r#"<v:shape id="preview" href="https://example.com/preview""#);
        let target = format!("https://example.com/{}", "x".repeat((8 << 10) - "https://example.com/".len()));
        let leaf = r##"<v:shape type="#Later" style="width:1pt;height:1pt"><v:imagedata r:id="rPreview"/></v:shape>"##.repeat(1000);
        let later = format!(r#"<v:group style="width:1pt;height:1pt" coordsize="1,1">{leaf}</v:group>"#).repeat(9);
        xml = xml.replace("</w:pict>", &format!(r#"<v:shapetype id="Later" href="{target}"/>{later}</w:pict>"#));
        let mut input = replacing_picture_part(&input, "word/document.xml", &xml);
        if own_target {
            let xml = xml.replace(
                r#"<wp:docPr id="1" name="Study ink"/>"#,
                r#"<wp:docPr id="1" name="Study ink"><a:hlinkClick r:id="rNativeLink"/></wp:docPr>"#,
            );
            let rels = part(&input, "word/_rels/document.xml.rels").replace(
                "</Relationships>",
                r#"<Relationship Id="rNativeLink" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="https://example.com/native" TargetMode="External"/></Relationships>"#,
            );
            input = replacing_picture_part(&input, "word/document.xml", &xml);
            input = replacing_picture_part(&input, "word/_rels/document.xml.rels", &rels);
        }
        let mut doc = craft_docx::read(&input).unwrap();
        for _ in 0..2 {
            let paragraph = doc.body[1].as_para().unwrap();
            let InlineObject::Graphic { graphic, .. } = &paragraph.objects[0] else { panic!() };
            assert_eq!(graphic.kind.noun(), "ink");
            assert!(graphic.source.is_some() && !graphic.items.is_empty());
            let expected = if own_target { "https://example.com/native" } else { "https://example.com/preview" };
            assert_eq!(paragraph.props_of_char(0).link.as_deref(), Some(expected));
            doc = craft_docx::read(&craft_docx::write(&doc).unwrap()).unwrap();
        }
    }
}

#[test]
fn deepest_linked_vml_tree_survives_flow_and_docx() {
    fn check(doc: &Document) {
        let paragraph = doc.body[0].as_para().unwrap();
        assert_eq!(paragraph.objects.len(), 1);
        assert_eq!(paragraph.props_of_char(0).link.as_deref(), Some("https://example.com/root"));
        let mut object = &paragraph.objects[0];
        for level in 0..=8 {
            let InlineObject::Group { children, .. } = object else { panic!("group at level {level}") };
            assert_eq!(children.len(), 1);
            let expected = if level == 8 { "#last-picture".to_string() } else { format!("https://example.com/level{}", level + 1) };
            assert_eq!(children[0].link.as_deref(), Some(expected.as_str()));
            object = &children[0].obj;
        }
        let InlineObject::Image { media, w, h, .. } = object else { panic!("deepest picture") };
        assert_eq!(doc.media.get(media).unwrap().as_slice(), png().as_slice());
        assert!((*w - 6.0).abs() < 0.001 && (*h - 5.0).abs() < 0.001);
    }
    let mut members = r##"<v:shape style="width:6pt;height:5pt" href="#last-picture"><v:imagedata r:id="rIdPic"/></v:shape>"##.to_string();
    for level in (1..=8).rev() {
        members = format!(r#"<v:group style="width:12pt;height:8pt" coordsize="12,8" href="https://example.com/level{level}">{members}</v:group>"#,);
    }
    let body = format!(
        r#"<w:p><w:r><w:pict xmlns:v="urn:schemas-microsoft-com:vml"><v:group style="width:12pt;height:8pt" coordsize="12,8" href="https://example.com/root">{members}</v:group></w:pict></w:r></w:p>"#,
    );
    let imported = craft_docx::read(&package(&body)).unwrap();
    check(&imported);
    let flow = craft_flow::from_doc_with(&imported, craft_flow::FlowOpts { boxes: true, paras: true, ..Default::default() });
    let rebuilt = craft_flow::to_doc(&flow);
    check(&rebuilt);
    let reopened = craft_docx::read(&craft_docx::write(&rebuilt).unwrap()).unwrap();
    check(&reopened);
}

/// Original Flow callers used the mark for a textless paragraph's single drawing.
#[test]
fn a_single_flow_picture_keeps_its_paragraph_mark_link() {
    const MARK: &str = "https://example.com/paragraph";
    const OWN: &str = "https://example.com/object";
    let bytes = Arc::new(png());
    for (links, text, expected) in [
        (vec![None], false, vec![Some(MARK)]),
        (vec![Some(OWN)], false, vec![Some(OWN)]),
        (vec![Some("")], false, vec![None]),
        (vec![None, None], false, vec![None, None]),
        (vec![Some(OWN), None], false, vec![Some(OWN), None]),
        (vec![None, Some(OWN)], false, vec![None, Some(OWN)]),
        (vec![None], true, vec![None]),
    ] {
        let inlines = links
            .into_iter()
            .map(|link| {
                let mut picture = craft_flow::make_img(bytes.clone(), Some(12.0), Some(6.0), "original picture").unwrap();
                picture.link = link.map(str::to_string);
                craft_flow::Inline::Image(picture)
            })
            .collect();
        let mut paragraph =
            craft_flow::Para { inlines, mark: Some(Box::new(CharProps { link: Some(MARK.into()), ..Default::default() })), ..Default::default() };
        if text {
            paragraph.push_text("beside the picture", &Default::default());
        }
        let flow = craft_flow::Flow { blocks: vec![craft_flow::FBlock::Para(paragraph)], ..Default::default() };
        let model = craft_flow::to_doc(&flow);
        let reopened = craft_docx::read(&craft_docx::write(&model).unwrap()).unwrap();
        let expected: Vec<_> = expected.into_iter().map(|target| target.map(str::to_string)).collect();
        for current in [&model, &reopened] {
            assert_eq!(
                picture_links(current),
                expected,
                "only the legacy single drawing inherits the mark; explicit targets and neighbors keep their own scope"
            );
            let paragraph = current.body[0].as_para().unwrap();
            assert_eq!(paragraph.objects.len(), expected.len());
            for object in &paragraph.objects {
                let InlineObject::Image { media, w, h, .. } = object else { panic!("the picture remains a picture") };
                assert_eq!(current.media.get(media).unwrap().as_slice(), bytes.as_slice());
                assert!((*w - 12.0).abs() < 0.001 && (*h - 6.0).abs() < 0.001, "the original picture size is retained");
            }
        }
    }
}

/// The legacy paragraph target has the same byte limit as an object's own target.
#[test]
fn a_single_flow_picture_bounds_its_inherited_mark_target() {
    const OWN: &str = "https://example.com/object";
    let prefix = "https://example.com/";
    let remaining = craft_flow::MAX_LINK - prefix.len();
    let ascii = format!("{prefix}{}", "a".repeat(remaining));
    let utf8 = format!("{prefix}{}{}", "é".repeat(remaining / 2), "a".repeat(remaining % 2));
    let ascii_over = format!("{ascii}a");
    let utf8_over = format!("{utf8}a");
    let bytes = Arc::new(png());
    for (mark, links, expected) in [
        (ascii.as_str(), vec![None], vec![Some(ascii.as_str())]),
        (ascii_over.as_str(), vec![None], vec![None]),
        (utf8.as_str(), vec![None], vec![Some(utf8.as_str())]),
        (utf8_over.as_str(), vec![None], vec![None]),
        (ascii_over.as_str(), vec![Some(OWN)], vec![Some(OWN)]),
        (ascii_over.as_str(), vec![Some("")], vec![None]),
        (ascii_over.as_str(), vec![Some(" \t ")], vec![None]),
        (ascii_over.as_str(), vec![Some(ascii_over.as_str())], vec![None]),
        (OWN, vec![Some(utf8_over.as_str())], vec![None]),
        (ascii_over.as_str(), vec![None, None], vec![None, None]),
        (ascii_over.as_str(), vec![Some(OWN), None], vec![Some(OWN), None]),
        (ascii_over.as_str(), vec![None, Some(OWN)], vec![None, Some(OWN)]),
    ] {
        let inlines = links
            .into_iter()
            .map(|link| {
                let mut picture = craft_flow::make_img(bytes.clone(), Some(12.0), Some(6.0), "original picture").unwrap();
                picture.link = link.map(str::to_string);
                craft_flow::Inline::Image(picture)
            })
            .collect();
        let paragraph =
            craft_flow::Para { inlines, mark: Some(Box::new(CharProps { link: Some(mark.into()), ..Default::default() })), ..Default::default() };
        let flow = craft_flow::Flow { blocks: vec![craft_flow::FBlock::Para(paragraph)], ..Default::default() };
        let expected: Vec<_> = expected.into_iter().map(|target| target.map(str::to_string)).collect();
        let check = |current: &Document| {
            assert_eq!(picture_links(current), expected, "each drawing keeps only its own or eligible inherited target");
            let paragraph = current.body[0].as_para().unwrap();
            assert_eq!(paragraph.objects.len(), expected.len());
            for object in &paragraph.objects {
                let InlineObject::Image { media, w, h, .. } = object else { panic!("the picture remains a picture") };
                assert_eq!(current.media.get(media).unwrap().as_slice(), bytes.as_slice());
                assert!((*w - 12.0).abs() < 0.001 && (*h - 6.0).abs() < 0.001, "the original picture size is retained");
            }
        };
        let model = craft_flow::to_doc(&flow);
        check(&model);
        let reopened = craft_docx::read(&craft_docx::write(&model).unwrap()).unwrap();
        check(&reopened);
    }
}
