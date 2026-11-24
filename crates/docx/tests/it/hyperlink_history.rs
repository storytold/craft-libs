//! Hyperlink history attribute (`w:hyperlink/@w:history`, ECMA-376 Part 1 §17.16.22): absent,
//! `"0"`, and `"1"` round-trip without injecting `w:history="1"`. Fixtures are hand-written from
//! the spec.

use std::io::Read;

use craft_wordmodel::{Document, InlineObject, PartKind};

use crate::fixtures::zip;

const W_NS: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#;
const ROOT_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
const RELS_OPEN: &str =
    r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#;

fn rels_xml(entries: &[(&str, &str, &str, bool)]) -> String {
    let mut s = String::from(RELS_OPEN);
    for (id, ty, target, external) in entries {
        let mode = if *external { r#" TargetMode="External""# } else { "" };
        s.push_str(&format!(
            r#"<Relationship Id="{id}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/{ty}" Target="{target}"{mode}/>"#
        ));
    }
    s.push_str("</Relationships>");
    s
}

fn docx(body: &str, doc_rels: &[(&str, &str, &str, bool)], extra: &[(&str, &str)]) -> Vec<u8> {
    let doc = format!(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document {W_NS}><w:body>{body}</w:body></w:document>"#);
    let r = rels_xml(doc_rels);
    let mut entries: Vec<(&str, &[u8])> =
        vec![("_rels/.rels", ROOT_RELS.as_bytes()), ("word/document.xml", doc.as_bytes()), ("word/_rels/document.xml.rels", r.as_bytes())];
    for (path, content) in extra {
        entries.push((path, content.as_bytes()));
    }
    zip(&entries)
}

fn part(bytes: &[u8], name: &str) -> String {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut s = String::new();
    z.by_name(name).unwrap().read_to_string(&mut s).unwrap();
    s
}

fn hyperlink_open_tags(xml: &str) -> Vec<&str> {
    xml.match_indices("<w:hyperlink ")
        .map(|(i, _)| {
            let rest = &xml[i..];
            let end = rest.find('>').unwrap();
            &rest[..=end]
        })
        .collect()
}

fn history_of(doc: &Document, text: &str) -> Option<bool> {
    for b in &doc.body {
        if let Some(p) = b.as_para()
            && let Some(off) = p.text.find(text)
        {
            return p.props_of_char(off).link_history;
        }
    }
    panic!("text {text:?} not found");
}

#[test]
fn three_state_round_trip_for_external_and_internal_links() {
    let body = concat!(
        r#"<w:p><w:bookmarkStart w:id="0" w:name="bm"/><w:r><w:t>Target</w:t></w:r><w:bookmarkEnd w:id="0"/></w:p>"#,
        r#"<w:p><w:hyperlink r:id="rIdExt"><w:r><w:t>ext-omit</w:t></w:r></w:hyperlink></w:p>"#,
        r#"<w:p><w:hyperlink r:id="rIdExt" w:history="0" w:tooltip="Zero tip"><w:r><w:t>ext-zero</w:t></w:r></w:hyperlink></w:p>"#,
        r#"<w:p><w:hyperlink r:id="rIdExt" w:history="1"><w:r><w:rPr><w:b/></w:rPr><w:t>ext-one</w:t></w:r></w:hyperlink></w:p>"#,
        r#"<w:p><w:hyperlink w:anchor="bm" w:tooltip="Omit tip"><w:r><w:t>int-omit</w:t></w:r></w:hyperlink></w:p>"#,
        r#"<w:p><w:hyperlink w:anchor="bm" w:history="0"><w:r><w:t>int-zero</w:t></w:r></w:hyperlink></w:p>"#,
        r#"<w:p><w:hyperlink w:anchor="bm" w:history="1" w:tooltip="One tip"><w:r><w:t>int-one</w:t></w:r></w:hyperlink></w:p>"#,
    );
    let bytes = docx(body, &[("rIdExt", "hyperlink", "https://example.org/", true)], &[]);
    let first = craft_docx::read(&bytes).unwrap();
    let expected = [
        ("ext-omit", None),
        ("ext-zero", Some(false)),
        ("ext-one", Some(true)),
        ("int-omit", None),
        ("int-zero", Some(false)),
        ("int-one", Some(true)),
    ];
    for (text, want) in expected {
        assert_eq!(history_of(&first, text), want, "{text}");
    }

    let saved = craft_docx::write(&first).unwrap();
    let xml = part(&saved, "word/document.xml");
    let tags = hyperlink_open_tags(&xml);
    assert_eq!(tags.len(), 6, "{xml}");
    assert!(tags[0].contains("r:id=\"") && !tags[0].contains("w:history=") && !tags[0].contains("w:tooltip="), "{}", tags[0]);
    assert!(tags[1].contains("r:id=\"") && tags[1].contains(r#"w:history="0" w:tooltip="Zero tip""#), "{}", tags[1]);
    assert!(tags[2].contains("r:id=\"") && tags[2].contains(r#"w:history="1""#) && !tags[2].contains("w:tooltip="), "{}", tags[2]);
    assert_eq!(tags[3], r#"<w:hyperlink w:anchor="bm" w:tooltip="Omit tip">"#);
    assert_eq!(tags[4], r#"<w:hyperlink w:anchor="bm" w:history="0">"#);
    assert_eq!(tags[5], r#"<w:hyperlink w:anchor="bm" w:history="1" w:tooltip="One tip">"#);
    // Plain runs inside hyperlinks with `w:history` must not acquire an empty `w:rPr`.
    assert!(!xml.contains("<w:rPr></w:rPr>") && !xml.contains("<w:rPr/>"), "{xml}");
    assert_eq!(xml.matches("<w:rPr>").count(), 1, "{xml}");

    let back = craft_docx::read(&saved).unwrap();
    for (text, want) in expected {
        assert_eq!(history_of(&back, text), want, "re-read {text}");
    }
    assert_eq!(back.body, first.body);
}

#[test]
fn no_edit_save_preserves_history_across_body_and_header() {
    let body = concat!(
        r#"<w:p><w:hyperlink r:id="rIdBody"><w:r><w:t>body-omit</w:t></w:r></w:hyperlink></w:p>"#,
        r#"<w:p><w:hyperlink r:id="rIdBody" w:history="0"><w:r><w:t>body-zero</w:t></w:r></w:hyperlink></w:p>"#,
        r#"<w:sectPr><w:headerReference w:type="default" r:id="rIdHdr"/></w:sectPr>"#,
    );
    let header = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:hdr {W_NS}><w:p><w:hyperlink r:id="rIdHLink"><w:r><w:t>hdr-omit</w:t></w:r></w:hyperlink></w:p><w:p><w:hyperlink r:id="rIdHLink" w:history="0"><w:r><w:t>hdr-zero</w:t></w:r></w:hyperlink></w:p></w:hdr>"#
    );
    let header_rels = rels_xml(&[("rIdHLink", "hyperlink", "https://example.org/header", true)]);
    let initial = docx(
        body,
        &[("rIdBody", "hyperlink", "https://example.org/body", true), ("rIdHdr", "header", "header1.xml", false)],
        &[("word/header1.xml", &header), ("word/_rels/header1.xml.rels", &header_rels)],
    );

    let mut doc = craft_docx::read(&initial).unwrap();
    for round in 0..2 {
        let saved = craft_docx::write(&doc).unwrap();
        for part_name in ["word/document.xml", "word/header1.xml"] {
            let xml = part(&saved, part_name);
            assert!(!xml.contains(r#"w:history="1""#), "round {round} {part_name}: {xml}");
            let tags = hyperlink_open_tags(&xml);
            assert_eq!(tags.len(), 2, "round {round} {part_name}: {xml}");
            assert!(!tags[0].contains("w:history="), "round {round} {part_name}: {}", tags[0]);
            assert!(tags[1].contains(r#"w:history="0""#), "round {round} {part_name}: {}", tags[1]);
        }
        doc = craft_docx::read(&saved).unwrap();
        assert_eq!(history_of(&doc, "body-omit"), None);
        assert_eq!(history_of(&doc, "body-zero"), Some(false));
        let hdr = doc.parts.values().find(|p| p.kind == PartKind::Header).unwrap();
        assert_eq!(hdr.blocks[0].as_para().unwrap().props_of_char(0).link_history, None);
        assert_eq!(hdr.blocks[1].as_para().unwrap().props_of_char(0).link_history, Some(false));
    }
}

#[test]
fn st_on_off_spellings_and_invalid_values() {
    let cases = [
        ("true", Some(true), Some(r#"w:history="1""#)),
        ("on", Some(true), Some(r#"w:history="1""#)),
        ("false", Some(false), Some(r#"w:history="0""#)),
        ("off", Some(false), Some(r#"w:history="0""#)),
        (" 0 ", Some(false), Some(r#"w:history="0""#)),
        ("yes", None, None),
        ("", None, None),
        ("True", None, None),
    ];
    let mut body = String::new();
    for (i, &(raw, _, _)) in cases.iter().enumerate() {
        body.push_str(&format!(r#"<w:p><w:hyperlink r:id="rId1" w:history="{raw}"><w:r><w:t>item-{i}</w:t></w:r></w:hyperlink></w:p>"#));
    }
    // A nested hyperlink replaces the outer history flag even when its own attribute is absent.
    body.push_str(
        r#"<w:p><w:hyperlink r:id="rId1" w:history="1"><w:hyperlink r:id="rId2"><w:r><w:t>nested</w:t></w:r></w:hyperlink></w:hyperlink></w:p>"#,
    );
    let bytes = docx(&body, &[("rId1", "hyperlink", "https://example.org/a", true), ("rId2", "hyperlink", "https://example.org/b", true)], &[]);
    let doc = craft_docx::read(&bytes).unwrap();
    for (i, &(raw, want, _)) in cases.iter().enumerate() {
        assert_eq!(history_of(&doc, &format!("item-{i}")), want, "raw={raw:?}");
    }
    assert_eq!(history_of(&doc, "nested"), None);

    let xml = part(&craft_docx::write(&doc).unwrap(), "word/document.xml");
    let tags = hyperlink_open_tags(&xml);
    assert_eq!(tags.len(), cases.len() + 1, "{xml}");
    for (&(raw, _, attr), tag) in cases.iter().zip(&tags) {
        match attr {
            Some(a) => assert!(tag.contains(a), "raw={raw:?}: {tag}"),
            None => assert!(!tag.contains("w:history="), "raw={raw:?}: {tag}"),
        }
    }
    assert!(!tags[cases.len()].contains("w:history="), "{}", tags[cases.len()]);
}

#[test]
fn adjacent_hyperlinks_with_different_history_stay_separate() {
    let body = concat!(
        "<w:p>",
        r#"<w:hyperlink r:id="rId1" w:history="0"><w:r><w:t>zero</w:t></w:r></w:hyperlink>"#,
        r#"<w:hyperlink r:id="rId1" w:history="1"><w:r><w:t>one</w:t></w:r></w:hyperlink>"#,
        r#"<w:hyperlink r:id="rId1"><w:r><w:t>omit</w:t></w:r></w:hyperlink>"#,
        "</w:p>",
    );
    let bytes = docx(body, &[("rId1", "hyperlink", "https://example.org/", true)], &[]);
    let first = craft_docx::read(&bytes).unwrap();
    let saved = craft_docx::write(&first).unwrap();
    let xml = part(&saved, "word/document.xml");
    let tags = hyperlink_open_tags(&xml);
    assert_eq!(tags.len(), 3, "{xml}");
    assert!(tags[0].contains(r#"w:history="0""#), "{}", tags[0]);
    assert!(tags[1].contains(r#"w:history="1""#), "{}", tags[1]);
    assert!(!tags[2].contains("w:history="), "{}", tags[2]);

    let back = craft_docx::read(&saved).unwrap();
    assert_eq!(history_of(&back, "zero"), Some(false));
    assert_eq!(history_of(&back, "one"), Some(true));
    assert_eq!(history_of(&back, "omit"), None);
}

#[test]
fn hyperlink_wrapping_ruby_preserves_history_without_splitting() {
    let body = concat!(
        r#"<w:p><w:hyperlink r:id="rId1" w:history="0">"#,
        r#"<w:r><w:t xml:space="preserve">prefix </w:t></w:r>"#,
        r#"<w:r><w:ruby><w:rubyPr><w:rubyAlign w:val="distributeSpace"/><w:hps w:val="10"/>"#,
        r#"<w:hpsRaise w:val="18"/><w:hpsBaseText w:val="20"/><w:lid w:val="ja-JP"/></w:rubyPr>"#,
        "<w:rt><w:r><w:t>かんじ</w:t></w:r></w:rt><w:rubyBase><w:r><w:t>漢字</w:t></w:r></w:rubyBase>",
        "</w:ruby></w:r></w:hyperlink></w:p>",
    );
    let bytes = docx(body, &[("rId1", "hyperlink", "https://example.org/", true)], &[]);
    let first = craft_docx::read(&bytes).unwrap();
    let p = first.body[0].as_para().unwrap();
    let ruby_off = p.object_offsets()[0];
    assert!(matches!(p.object_at(ruby_off), Some(InlineObject::Ruby { .. })));
    assert_eq!(p.props_of_char(0).link_history, Some(false));
    assert_eq!(p.props_of_char(ruby_off).link_history, Some(false));

    let saved = craft_docx::write(&first).unwrap();
    let xml = part(&saved, "word/document.xml");
    let tags = hyperlink_open_tags(&xml);
    assert_eq!(tags.len(), 1, "{xml}");
    assert!(tags[0].contains(r#"w:history="0""#), "{}", tags[0]);
    let open = xml.find("<w:hyperlink ").unwrap();
    let ruby = xml.find("<w:ruby>").unwrap();
    let close = xml.find("</w:hyperlink>").unwrap();
    assert!(open < ruby && ruby < close, "{xml}");

    let back = craft_docx::read(&saved).unwrap();
    let bp = back.body[0].as_para().unwrap();
    let back_ruby_off = bp.object_offsets()[0];
    assert!(matches!(bp.object_at(back_ruby_off), Some(InlineObject::Ruby { .. })));
    assert_eq!(bp.props_of_char(0).link_history, Some(false));
    assert_eq!(bp.props_of_char(back_ruby_off).link_history, Some(false));
}
