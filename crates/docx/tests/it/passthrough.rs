//! What a save keeps from a Word file beyond the document model (settings: `settings_merge.rs`):
//! package parts WordCraft doesn't write (carried verbatim),
//! comment durable ids and UTC dates, localized default styles, latent styles, list identities
//! and extended properties. Hand-written fixtures (from the spec), no files produced by Word.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::sync::Arc;

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const R_NS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

fn zip(entries: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, bytes) in entries {
        zw.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
        zw.write_all(bytes).unwrap();
    }
    zw.finish().unwrap().into_inner()
}

fn unzip(bytes: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut out = BTreeMap::new();
    for i in 0..z.len() {
        let mut f = z.by_index(i).unwrap();
        let mut b = Vec::new();
        f.read_to_end(&mut b).unwrap();
        assert!(out.insert(f.name().unwrap().to_string(), b).is_none(), "duplicate part {}", f.name().unwrap());
    }
    out
}

fn text(files: &BTreeMap<String, Vec<u8>>, name: &str) -> String {
    String::from_utf8(files.get(name).unwrap_or_else(|| panic!("no {name} in {:?}", files.keys())).clone()).unwrap()
}

fn rels(list: &[(&str, &str, &str)]) -> Vec<u8> {
    let mut s =
        String::from(r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#);
    for (id, ty, target) in list {
        s.push_str(&format!(r#"<Relationship Id="{id}" Type="{ty}" Target="{target}"/>"#));
    }
    s.push_str("</Relationships>");
    s.into_bytes()
}

/// A package: `body`, document relationships, root relationships after the main document's,
/// and extra parts.
fn package(body: &str, doc_rels: &[(&str, &str, &str)], root_rels: &[(&str, &str, &str)], extra: &[(&str, &str)]) -> Vec<u8> {
    let doc = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="{W}" xmlns:r="{R_NS}" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml"><w:body>{body}</w:body></w:document>"#
    );
    let mut root = vec![("rIdMain", "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument", "word/document.xml")];
    root.extend_from_slice(root_rels);
    let mut entries: Vec<(&str, Vec<u8>)> =
        vec![("_rels/.rels", rels(&root)), ("word/document.xml", doc.into_bytes()), ("word/_rels/document.xml.rels", rels(doc_rels))];
    for (p, c) in extra {
        entries.push((p, c.as_bytes().to_vec()));
    }
    zip(&entries)
}

fn save(doc: &craft_wordmodel::Document) -> BTreeMap<String, Vec<u8>> {
    unzip(&craft_docx::write(doc).expect("write"))
}

const SETTINGS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml" xmlns:w15="http://schemas.microsoft.com/office/word/2012/wordml" mc:Ignorable="w14 w15"><w:zoom w:percent="120"/><w:removePersonalInformation/><w:removeDateAndTime/><w:proofState w:spelling="clean" w:grammar="clean"/><w:mailMerge><w:mainDocumentType w:val="formLetters"/><w:dataSource r:id="rId9"/></w:mailMerge><w:trackRevisions/><w:defaultTabStop w:val="708"/><w:hyphenationZone w:val="425"/><w:characterSpacingControl w:val="doNotCompress"/><w:compat><w:compatSetting w:name="compatibilityMode" w:uri="http://schemas.microsoft.com/office/word" w:val="15"/><w:compatSetting w:name="overrideTableStyleFontSizeAndJustification" w:uri="http://schemas.microsoft.com/office/word" w:val="1"/><w:compatSetting w:name="enableOpenTypeFeatures" w:uri="http://schemas.microsoft.com/office/word" w:val="1"/></w:compat><w:docVars><w:docVar w:name="ClauseSet" w:val="B"/></w:docVars><w:rsids><w:rsidRoot w:val="00A1B2C3"/><w:rsid w:val="00A1B2C3"/></w:rsids><m:mathPr><m:mathFont m:val="Cambria Math"/><m:smallFrac m:val="0"/></m:mathPr><w:themeFontLang w:val="pl-PL"/><w:decimalSymbol w:val=","/><w:listSeparator w:val=";"/><w14:docId w14:val="1A2B3C4D"/></w:settings>"#;

fn pos(s: &str, needle: &str) -> usize {
    s.find(needle).unwrap_or_else(|| panic!("{needle} missing from {s}"))
}

const CT: &str = r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Default Extension="jpeg" ContentType="image/jpeg"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/webSettings.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.webSettings+xml"/><Override PartName="/word/stylesWithEffects.xml" ContentType="application/vnd.ms-word.stylesWithEffects+xml"/><Override PartName="/word/people.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.people+xml"/><Override PartName="/word/webextensions/taskpanes.xml" ContentType="application/vnd.ms-office.webextensiontaskpanes+xml"/><Override PartName="/word/webextensions/webextension1.xml" ContentType="application/vnd.ms-office.webextension+xml"/><Override PartName="/word/unknownPart.xml" ContentType="application/x-unknown+xml"/></Types>"#;

#[test]
fn unmodelled_parts_are_carried_with_relationships_and_content_types() {
    let web = r#"<w:webSettings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:optimizeForBrowser/><w:allowPNG/></w:webSettings>"#;
    let swe =
        r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:style w:type="paragraph" w:styleId="X"/></w:styles>"#;
    let people = r#"<w15:people xmlns:w15="http://schemas.microsoft.com/office/word/2012/wordml"><w15:person w15:author="Ann"><w15:presenceInfo w15:providerId="None" w15:userId="Ann"/></w15:person></w15:people>"#;
    let panes = r#"<wetp:taskpanes xmlns:wetp="http://schemas.microsoft.com/office/webextensions/taskpanes/2010/11" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><wetp:taskpane dockstate="right" visibility="0" width="350" row="1"><wetp:webextensionref r:id="rId1"/></wetp:taskpane></wetp:taskpanes>"#;
    let ext = r#"<we:webextension xmlns:we="http://schemas.microsoft.com/office/webextensions/webextension/2010/11" id="{00000000-0000-0000-0000-000000000001}"><we:reference id="wa0" version="1.0.0.0" store="en-US" storeType="OMEX"/></we:webextension>"#;
    let unknown = r#"<x:thing xmlns:x="urn:example:future"/>"#;
    let thumb = "\u{ff}\u{d8}\u{ff}JPEG";
    let panes_rels =
        String::from_utf8(rels(&[("rId1", "http://schemas.microsoft.com/office/2011/relationships/webextension", "webextension1.xml")])).unwrap();
    let bytes = package(
        r#"<w:p><w:r><w:t>Body</w:t></w:r></w:p>"#,
        &[
            ("rId1", &format!("{REL}/webSettings"), "webSettings.xml"),
            ("rId2", "http://schemas.microsoft.com/office/2007/relationships/stylesWithEffects", "stylesWithEffects.xml"),
            ("rId3", "http://schemas.microsoft.com/office/2011/relationships/people", "people.xml"),
            ("rId4", "urn:example:future-relationship", "unknownPart.xml"),
        ],
        &[
            ("rIdT", "http://schemas.openxmlformats.org/package/2006/relationships/metadata/thumbnail", "docProps/thumbnail.jpeg"),
            ("rIdW", "http://schemas.microsoft.com/office/2011/relationships/webextensiontaskpanes", "word/webextensions/taskpanes.xml"),
        ],
        &[
            ("[Content_Types].xml", CT),
            ("word/webSettings.xml", web),
            ("word/stylesWithEffects.xml", swe),
            ("word/people.xml", people),
            ("word/unknownPart.xml", unknown),
            ("docProps/thumbnail.jpeg", thumb),
            ("word/webextensions/taskpanes.xml", panes),
            ("word/webextensions/_rels/taskpanes.xml.rels", &panes_rels),
            ("word/webextensions/webextension1.xml", ext),
        ],
    );
    let doc = craft_docx::read(&bytes).unwrap();
    let files = save(&doc);
    let input = unzip(&bytes);
    for part in [
        "word/webSettings.xml",
        "word/stylesWithEffects.xml",
        "word/people.xml",
        "word/unknownPart.xml",
        "docProps/thumbnail.jpeg",
        "word/webextensions/taskpanes.xml",
        "word/webextensions/webextension1.xml",
    ] {
        assert_eq!(files.get(part), input.get(part), "{part} not carried verbatim");
    }
    let doc_rels = text(&files, "word/_rels/document.xml.rels");
    for ty in ["/webSettings\"", "/stylesWithEffects\"", "/people\"", "urn:example:future-relationship"] {
        assert_eq!(doc_rels.matches(ty).count(), 1, "{ty}: {doc_rels}");
    }
    let root_rels = text(&files, "_rels/.rels");
    assert!(root_rels.contains(r#"Target="docProps/thumbnail.jpeg""#) && root_rels.contains("webextensiontaskpanes"), "{root_rels}");
    assert!(text(&files, "word/webextensions/_rels/taskpanes.xml.rels").contains("webextension1.xml"));
    let ct = text(&files, "[Content_Types].xml");
    for (part, ty) in [
        ("/word/webSettings.xml", "webSettings+xml"),
        ("/word/stylesWithEffects.xml", "stylesWithEffects+xml"),
        ("/word/people.xml", "people+xml"),
        ("/word/webextensions/taskpanes.xml", "webextensiontaskpanes+xml"),
        ("/docProps/thumbnail.jpeg", "image/jpeg"),
    ] {
        let entry = ct.split("<Override ").find(|o| o.contains(&format!(r#"PartName="{part}""#))).unwrap_or_else(|| panic!("{part}: {ct}"));
        assert!(entry.contains(ty), "{part}: {entry}");
    }
    // Saved again: the same parts, still once each (`unzip` rejects duplicates).
    let again = save(&craft_docx::read(&craft_docx::write(&doc).unwrap()).unwrap());
    assert_eq!(again.get("word/webSettings.xml"), input.get("word/webSettings.xml"));
    assert_eq!(text(&again, "word/_rels/document.xml.rels").matches("/webSettings\"").count(), 1);
}

#[test]
fn parts_the_writer_owns_or_the_body_refers_to_are_not_carried() {
    // A second styles part, and an unknown part the body refers to by id: neither is carried
    // (the first would contradict the written styles, the second's element isn't kept).
    let bytes = package(
        r#"<w:p><w:r><w:t>Body</w:t></w:r><w:customThing r:id="rId8"/></w:p>"#,
        &[("rId7", &format!("{REL}/styles"), "other-styles.xml"), ("rId8", "urn:example:referenced", "referenced.xml")],
        &[],
        &[
            ("word/other-styles.xml", r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"/>"#),
            ("word/referenced.xml", "<x/>"),
        ],
    );
    let files = save(&craft_docx::read(&bytes).unwrap());
    assert!(!files.contains_key("word/other-styles.xml") && !files.contains_key("word/referenced.xml"), "{:?}", files.keys());
    assert_eq!(text(&files, "word/_rels/document.xml.rels").matches("/styles\"").count(), 1);
}

fn comments_package(with_ids: bool) -> Vec<u8> {
    let comments = r#"<w:comments xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml"><w:comment w:id="0" w:author="Ann" w:date="2026-01-02T10:00:00" w:initials="A"><w:p w14:paraId="11111111"><w:r><w:t>First</w:t></w:r></w:p></w:comment><w:comment w:id="1" w:author="Ben" w:date="2026-01-03T11:00:00" w:initials="B"><w:p w14:paraId="22222222"><w:r><w:t>Reply</w:t></w:r></w:p></w:comment></w:comments>"#;
    let ex = r#"<w15:commentsEx xmlns:w15="http://schemas.microsoft.com/office/word/2012/wordml"><w15:commentEx w15:paraId="11111111" w15:done="0"/><w15:commentEx w15:paraId="22222222" w15:paraIdParent="11111111" w15:done="0"/></w15:commentsEx>"#;
    let ids = r#"<w16cid:commentsIds xmlns:w16cid="http://schemas.microsoft.com/office/word/2016/wordml/cid"><w16cid:commentId w16cid:paraId="11111111" w16cid:durableId="5A1B2C3D"/><w16cid:commentId w16cid:paraId="22222222" w16cid:durableId="0F00BA12"/></w16cid:commentsIds>"#;
    let cex = r#"<w16cex:commentsExtensible xmlns:w16cex="http://schemas.microsoft.com/office/word/2018/wordml/cex"><w16cex:commentExtensible w16cex:durableId="5A1B2C3D" w16cex:dateUtc="2026-01-02T09:00:00Z"/><w16cex:commentExtensible w16cex:durableId="0F00BA12" w16cex:dateUtc="2026-01-03T10:00:00Z"/></w16cex:commentsExtensible>"#;
    let body = r#"<w:p><w:commentRangeStart w:id="0"/><w:commentRangeStart w:id="1"/><w:r><w:t>Text</w:t></w:r><w:commentRangeEnd w:id="0"/><w:r><w:rPr><w:rStyle w:val="Odwoaniedokomentarza"/></w:rPr><w:commentReference w:id="0"/></w:r><w:commentRangeEnd w:id="1"/><w:r><w:rPr><w:rStyle w:val="Odwoaniedokomentarza"/></w:rPr><w:commentReference w:id="1"/></w:r></w:p>"#;
    let styles = r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:style w:type="paragraph" w:default="1" w:styleId="Normalny"><w:name w:val="Normal"/></w:style><w:style w:type="character" w:default="1" w:styleId="Domylnaczcionkaakapitu"><w:name w:val="Default Paragraph Font"/></w:style><w:style w:type="character" w:styleId="Odwoaniedokomentarza"><w:name w:val="annotation reference"/><w:basedOn w:val="Domylnaczcionkaakapitu"/><w:rPr><w:sz w:val="16"/></w:rPr></w:style></w:styles>"#;
    let mut doc_rels = vec![
        ("rId1", format!("{REL}/comments"), "comments.xml"),
        ("rId2", "http://schemas.microsoft.com/office/2011/relationships/commentsExtended".to_string(), "commentsExtended.xml"),
        ("rId5", format!("{REL}/styles"), "styles.xml"),
    ];
    let mut extra = vec![("word/comments.xml", comments), ("word/commentsExtended.xml", ex), ("word/styles.xml", styles)];
    if with_ids {
        doc_rels.push(("rId3", "http://schemas.microsoft.com/office/2016/09/relationships/commentsIds".into(), "commentsIds.xml"));
        doc_rels.push(("rId4", "http://schemas.microsoft.com/office/2018/08/relationships/commentsExtensible".into(), "commentsExtensible.xml"));
        extra.push(("word/commentsIds.xml", ids));
        extra.push(("word/commentsExtensible.xml", cex));
    }
    let doc_rels: Vec<(&str, &str, &str)> = doc_rels.iter().map(|(a, b, c)| (*a, b.as_str(), *c)).collect();
    package(body, &doc_rels, &[], &extra)
}

/// (paragraph id → durable id) from commentsIds, and (durable id → UTC date) from
/// commentsExtensible, as written.
fn written_ids(files: &BTreeMap<String, Vec<u8>>) -> (Vec<(String, String)>, Vec<(String, String)>) {
    let attr = |el: &str, name: &str| -> Option<String> { el.split(&format!("{name}=\"")).nth(1)?.split('"').next().map(str::to_string) };
    let ids = text(files, "word/commentsIds.xml")
        .split("<w16cid:commentId ")
        .skip(1)
        .filter_map(|e| Some((attr(e, "w16cid:paraId")?, attr(e, "w16cid:durableId")?)))
        .collect();
    let dates = text(files, "word/commentsExtensible.xml")
        .split("<w16cex:commentExtensible ")
        .skip(1)
        .filter_map(|e| Some((attr(e, "w16cex:durableId")?, attr(e, "w16cex:dateUtc").unwrap_or_default())))
        .collect();
    (ids, dates)
}

#[test]
fn comment_durable_ids_and_utc_dates_survive_and_match_the_written_paragraph_ids() {
    let doc = craft_docx::read(&comments_package(true)).unwrap();
    let c: Vec<_> = doc.comments.values().collect();
    assert_eq!(c[0].durable_id.as_deref(), Some("5A1B2C3D"));
    assert_eq!(c[1].date_utc.as_deref(), Some("2026-01-03T10:00:00Z"));
    let files = save(&doc);
    let (ids, dates) = written_ids(&files);
    assert_eq!(ids.iter().map(|(_, d)| d.as_str()).collect::<Vec<_>>(), ["5A1B2C3D", "0F00BA12"]);
    // Each paragraph id is the one the comment's paragraph got in comments.xml.
    let comments = text(&files, "word/comments.xml");
    for (pid, _) in &ids {
        assert!(comments.contains(&format!(r#"w14:paraId="{pid}""#)), "{pid}: {comments}");
    }
    assert_eq!(dates, [("5A1B2C3D".into(), "2026-01-02T09:00:00Z".into()), ("0F00BA12".into(), "2026-01-03T10:00:00Z".into())]);
    let rels = text(&files, "word/_rels/document.xml.rels");
    assert!(rels.contains("relationships/commentsIds") && rels.contains("relationships/commentsExtensible"), "{rels}");
    // Read back: the same durable ids and dates.
    let back = craft_docx::read(&craft_docx::write(&doc).unwrap()).unwrap();
    assert_eq!(
        back.comments.values().map(|c| c.durable_id.clone()).collect::<Vec<_>>(),
        doc.comments.values().map(|c| c.durable_id.clone()).collect::<Vec<_>>()
    );
    assert_eq!(
        back.comments.values().map(|c| c.date_utc.clone()).collect::<Vec<_>>(),
        doc.comments.values().map(|c| c.date_utc.clone()).collect::<Vec<_>>()
    );
}

#[test]
fn comments_without_durable_ids_get_unique_ones() {
    let doc = craft_docx::read(&comments_package(false)).unwrap();
    let (ids, dates) = written_ids(&save(&doc));
    assert_eq!(ids.len(), 2);
    assert_ne!(ids[0].1, ids[1].1);
    for (_, d) in &ids {
        let v = u32::from_str_radix(d, 16).unwrap();
        assert!(v != 0 && v < 0x7FFF_FFFF, "{d}");
    }
    // Local dates without a zone aren't passed off as UTC.
    assert!(dates.iter().all(|(_, utc)| utc.is_empty()), "{dates:?}");
}

/// The reactions namespace ([MS-OREACTXML]).
const CR: &str = "http://schemas.microsoft.com/office/comments/2020/reactions";

/// commentsExtensible.xml with one Like on the first comment, its namespaces declared on the
/// root, as Word writes it.
const LIKE_CEX: &str = r#"<w16cex:commentsExtensible xmlns:w16cex="http://schemas.microsoft.com/office/word/2018/wordml/cex" xmlns:w16="http://schemas.microsoft.com/office/word/2018/wordml" xmlns:cr="http://schemas.microsoft.com/office/comments/2020/reactions" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" mc:Ignorable="w16 cr"><w16cex:commentExtensible w16cex:durableId="5A1B2C3D" w16cex:dateUtc="2026-01-02T09:00:00Z"><w16cex:extLst><w16:ext w16:uri="{CE6994B0-6A32-4C9F-8C6B-6E91EDA988CE}"><cr:reactions><cr:reaction reactionType="1"><cr:reactionInfo dateUtc="2026-01-04T06:16:20Z"><cr:user userId="ann@example.com" userProvider="None" userName="Ann"/></cr:reactionInfo></cr:reaction></cr:reactions></w16:ext></w16cex:extLst></w16cex:commentExtensible><w16cex:commentExtensible w16cex:durableId="0F00BA12" w16cex:dateUtc="2026-01-03T10:00:00Z"/></w16cex:commentsExtensible>"#;

/// [`comments_package`] with `cex` as its commentsExtensible.xml, read.
fn comments_with_cex(cex: &str) -> craft_wordmodel::Document {
    let files = unzip(&comments_package(true));
    let entries: Vec<(&str, Vec<u8>)> =
        files.iter().map(|(k, v)| (k.as_str(), if k == "word/commentsExtensible.xml" { cex.as_bytes().to_vec() } else { v.clone() })).collect();
    craft_docx::read(&zip(&entries)).unwrap()
}

fn liked_comments() -> craft_wordmodel::Document {
    comments_with_cex(LIKE_CEX)
}

/// The written commentsExtensible.xml read by namespace, whatever prefixes it uses.
fn written_cex(files: &BTreeMap<String, Vec<u8>>) -> craft_xml::El {
    let namespaces = [
        ("cex", "http://schemas.microsoft.com/office/word/2018/wordml/cex"),
        ("w16", "http://schemas.microsoft.com/office/word/2018/wordml"),
        ("cr", CR),
    ];
    let canonical = craft_xml::Canonical { namespaces: &namespaces, unknown: "?", element: None, attr_value: None };
    craft_xml::Options::canonical(&canonical).strict().parse(text(files, "word/commentsExtensible.xml").as_bytes()).unwrap()
}

#[test]
fn a_comments_reactions_survive_open_and_save() {
    let doc = liked_comments();
    for files in [save(&doc), save(&craft_docx::read(&craft_docx::write(&doc).unwrap()).unwrap())] {
        let cex = written_cex(&files);
        let comments: Vec<_> = cex.children("cex:commentExtensible").collect();
        let reaction = comments[0].find("cr:reaction").unwrap_or_else(|| panic!("{}", text(&files, "word/commentsExtensible.xml")));
        assert_eq!(reaction.attr("reactionType"), Some("1"));
        let user = reaction.find("cr:user").unwrap();
        assert_eq!((user.attr("userId"), user.attr("userName")), (Some("ann@example.com"), Some("Ann")));
        assert!(reaction.find("cr:reactionInfo").unwrap().attr("dateUtc").is_some());
        assert!(comments[1].elements().next().is_none(), "the comment without reactions gets none");
    }
}

#[test]
fn reactions_follow_the_privacy_settings() {
    let mut doc = liked_comments();
    doc.settings.remove_date_time = true;
    let files = save(&doc);
    let cex = written_cex(&files);
    let reaction = cex.find("cr:reaction").unwrap();
    assert!(reaction.find("cr:user").is_some());
    assert!(cex.descendants().all(|e| e.attr_local("dateUtc").is_none()), "{}", text(&files, "word/commentsExtensible.xml"));
    // The people who reacted are personal information.
    doc.settings.remove_personal_info = true;
    let files = save(&doc);
    assert!(written_cex(&files).find("cr:reaction").is_none(), "{}", text(&files, "word/commentsExtensible.xml"));
}

/// Kept markup takes its namespaces along, wherever the file declared them; markup using a prefix
/// nothing declares isn't kept, as a save could only write it as markup no reader accepts.
#[test]
fn kept_extensions_carry_their_namespaces_and_undeclared_prefixes_are_not_kept() {
    let cex = format!(
        r#"<x:commentsExtensible xmlns:x="http://schemas.microsoft.com/office/word/2018/wordml/cex"><x:commentExtensible x:durableId="5A1B2C3D" xmlns:r="{CR}"><x:extLst><ext xmlns="http://schemas.microsoft.com/office/word/2018/wordml" uri="{{CE6994B0-6A32-4C9F-8C6B-6E91EDA988CE}}"><r:reactions><r:reaction reactionType="2"/></r:reactions></ext></x:extLst></x:commentExtensible><x:commentExtensible x:durableId="0F00BA12" x:dateUtc="2026-01-03T10:00:00Z"><x:extLst><nope:thing/></x:extLst></x:commentExtensible></x:commentsExtensible>"#
    );
    let doc = comments_with_cex(&cex);
    let c: Vec<_> = doc.comments.values().collect();
    assert!(c[1].extensions.is_none() && c[1].date_utc.is_some(), "{:?}", c[1]);
    let files = save(&doc);
    let cex = written_cex(&files);
    let reaction = cex.find("cr:reaction").unwrap_or_else(|| panic!("{}", text(&files, "word/commentsExtensible.xml")));
    assert_eq!(reaction.attr("reactionType"), Some("2"));
}

/// A prefix kept markup only lists as ignorable, declared outside it, goes along too.
#[test]
fn kept_extensions_carry_the_namespaces_their_own_ignorable_lists() {
    let cex = format!(
        r#"<x:commentsExtensible xmlns:x="http://schemas.microsoft.com/office/word/2018/wordml/cex" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:zz="urn:zz" xmlns:r="{CR}"><x:commentExtensible x:durableId="5A1B2C3D"><x:extLst mc:Ignorable="zz"><r:reactions><r:reaction reactionType="2"/></r:reactions></x:extLst></x:commentExtensible></x:commentsExtensible>"#
    );
    let files = save(&comments_with_cex(&cex));
    let written = text(&files, "word/commentsExtensible.xml");
    assert!(written.contains(r#"mc:Ignorable="zz""#), "{written}");
    assert!(written.contains(r#"xmlns:zz="urn:zz""#), "{written}");
}

/// A prefix the file marks ignorable outside kept markup stays ignorable in it when the markup
/// declares it again with the same namespace, and not when it binds it to another one.
#[test]
fn kept_extensions_keep_an_outer_ignorable_prefix_they_declare_again_alike() {
    for (same, markup) in kept_marks_q_ignorable("", r#"xmlns:q="urn:q""#) {
        assert!(same, "{markup}");
    }
    for (other, markup) in kept_marks_q_ignorable("", r#"xmlns:q="urn:other""#) {
        assert!(!other, "{markup}");
    }
}

/// A prefix the file marks ignorable outside kept markup stays ignorable in it when the markup
/// binds the file's Markup Compatibility prefix to another namespace.
#[test]
fn kept_extensions_keep_an_outer_ignorable_prefix_when_they_rebind_mc() {
    for (kept, markup) in kept_marks_q_ignorable("", r#"xmlns:mc="urn:vendor""#) {
        assert!(kept, "{markup}");
    }
}

/// The root marks a namespace ignorable, not a prefix: kept markup doesn't mark the namespace
/// a comment element binds that prefix to instead, through a save and a second round trip.
#[test]
fn kept_extensions_do_not_mark_ignorable_a_prefix_their_comment_rebinds() {
    for (marked, markup) in kept_marks_q_ignorable(r#"xmlns:q="urn:b""#, "") {
        assert!(!marked && markup.contains(r#"xmlns:q="urn:b""#), "{markup}");
    }
}

/// The root marks the namespace `q` names ignorable, and `alias` names it too: when the comment
/// element binds `q` to another namespace, kept markup naming `alias` (here in
/// `mc:ProcessContent`, which needs its prefix ignorable) still marks that namespace ignorable,
/// under `alias`, through a save and a second round trip.
#[test]
fn kept_extensions_mark_ignorable_under_another_prefix_a_namespace_whose_prefix_their_comment_rebinds() {
    let cex = format!(
        r#"<x:commentsExtensible xmlns:x="http://schemas.microsoft.com/office/word/2018/wordml/cex" xmlns:mc="{MCE}" xmlns:q="urn:old" xmlns:alias="urn:old" mc:Ignorable="q" xmlns:r="{CR}"><x:commentExtensible x:durableId="5A1B2C3D" xmlns:q="urn:b"><x:extLst mc:ProcessContent="alias:note"><alias:note><r:reactions/></alias:note></x:extLst></x:commentExtensible></x:commentsExtensible>"#
    );
    for (ext, markup) in kept_markup(&cex) {
        let ignorable = ignorable_namespaces(&ext);
        assert!(ignorable.contains(&"urn:old") && !ignorable.contains(&"urn:b"), "{markup}");
        assert!(ext.ns_decls().any(|d| d == ("alias", "urn:old")), "{markup}");
    }
}

/// As above, with `alias` declared only on markup inside the `extLst`: the kept element declares
/// the namespace itself and marks it ignorable, under `alias` or, when the comment element binds
/// `alias` to something else, under a prefix of its own, through a save and a second round trip.
#[test]
fn kept_extensions_mark_ignorable_a_namespace_only_their_inner_markup_names() {
    for comment_alias in ["", r#"xmlns:alias="urn:c""#] {
        let cex = format!(
            r#"<x:commentsExtensible xmlns:x="http://schemas.microsoft.com/office/word/2018/wordml/cex" xmlns:mc="{MCE}" xmlns:q="urn:old" mc:Ignorable="q" xmlns:r="{CR}"><x:commentExtensible x:durableId="5A1B2C3D" xmlns:q="urn:b" {comment_alias}><x:extLst><r:reactions xmlns:alias="urn:old" mc:ProcessContent="alias:note"><alias:note/></r:reactions></x:extLst></x:commentExtensible></x:commentsExtensible>"#
        );
        for (ext, markup) in kept_markup(&cex) {
            let ignorable = ignorable_namespaces(&ext);
            assert!(ignorable.contains(&"urn:old"), "{markup}");
            assert!(!ignorable.contains(&"urn:b") && !ignorable.contains(&"urn:c"), "{markup}");
            let inner = ext.find("r:reactions").unwrap_or_else(|| panic!("{markup}"));
            assert!(inner.ns_decls().any(|d| d == ("alias", "urn:old")), "{markup}");
        }
    }
}

/// As above, with only a default namespace naming it, on the `extLst` or on markup inside it:
/// the kept element declares the namespace under a prefix and marks it ignorable, through a save
/// and a second round trip.
#[test]
fn kept_extensions_mark_ignorable_a_namespace_only_a_default_namespace_names() {
    let inside = r#"<x:extLst><r:reactions><note xmlns="urn:old"/></r:reactions></x:extLst>"#;
    let on_ext = r#"<x:extLst xmlns="urn:old"><note/></x:extLst>"#;
    for ext in [inside, on_ext] {
        let cex = format!(
            r#"<x:commentsExtensible xmlns:x="http://schemas.microsoft.com/office/word/2018/wordml/cex" xmlns:mc="{MCE}" xmlns:q="urn:old" mc:Ignorable="q" xmlns:r="{CR}"><x:commentExtensible x:durableId="5A1B2C3D" xmlns:q="urn:b">{ext}</x:commentExtensible></x:commentsExtensible>"#
        );
        for (kept, markup) in kept_markup(&cex) {
            let ignorable = ignorable_namespaces(&kept);
            assert!(ignorable.contains(&"urn:old") && !ignorable.contains(&"urn:b"), "{markup}");
            let default = kept.ns_decls().chain(kept.descendants().flat_map(craft_xml::El::ns_decls)).find(|(p, _)| p.is_empty());
            assert_eq!(default, Some(("", "urn:old")), "the note stays in urn:old: {markup}");
        }
    }
}

const MCE: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";

/// The extension kept from commentsExtensible.xml `cex`, as read and after a save and a second
/// read: each parsed, and as written.
fn kept_markup(cex: &str) -> Vec<(craft_xml::El, String)> {
    let doc = comments_with_cex(cex);
    let back = craft_docx::read(&craft_docx::write(&doc).unwrap()).unwrap();
    [doc, back]
        .iter()
        .map(|d| {
            let markup = d.comments.values().find_map(|c| c.extensions.clone()).unwrap();
            (craft_xml::Options::default().strict().parse(markup.as_bytes()).unwrap(), markup)
        })
        .collect()
}

/// The namespaces kept markup `ext` marks ignorable by Markup Compatibility, by its own
/// declarations.
fn ignorable_namespaces(ext: &craft_xml::El) -> Vec<&str> {
    let mce: Vec<&str> = ext.ns_decls().filter(|(_, u)| *u == MCE).map(|(p, _)| p).collect();
    let tokens = ext.attrs.iter().filter(|(k, _)| k.split_once(':').is_some_and(|(p, local)| local == "Ignorable" && mce.contains(&p)));
    tokens.flat_map(|(_, v)| v.split_whitespace()).filter_map(|t| ext.ns_decls().find(|(p, _)| *p == t).map(|(_, u)| u)).collect()
}

/// Whether the extension kept from an `extLst` carrying `ext_decls`, in a comment element
/// carrying `comment_decls`, under a root that marks `q` ignorable, marks `q` ignorable by
/// Markup Compatibility, as read and after a save and a second read; each with the kept markup.
fn kept_marks_q_ignorable(comment_decls: &str, ext_decls: &str) -> Vec<(bool, String)> {
    let cex = format!(
        r#"<x:commentsExtensible xmlns:x="http://schemas.microsoft.com/office/word/2018/wordml/cex" xmlns:mc="{MCE}" xmlns:q="urn:q" mc:Ignorable="q" xmlns:r="{CR}"><x:commentExtensible x:durableId="5A1B2C3D" {comment_decls}><x:extLst {ext_decls}><r:reactions q:note="n"/></x:extLst></x:commentExtensible></x:commentsExtensible>"#
    );
    kept_markup(&cex)
        .into_iter()
        .map(|(ext, markup)| {
            let mce: Vec<&str> = ext.ns_decls().filter(|(_, u)| *u == MCE).map(|(p, _)| p).collect();
            let marked = ext.attrs.iter().any(|(k, v)| {
                k.split_once(':').is_some_and(|(p, local)| local == "Ignorable" && mce.contains(&p)) && v.split_whitespace().any(|t| t == "q")
            });
            (marked, markup)
        })
        .collect()
}

/// The namespace `prefix` names on the first element called `name` in `root`, by the
/// declarations on the way down.
fn binding_at(root: &craft_xml::El, name: &str, prefix: &str) -> Option<String> {
    fn find(e: &craft_xml::El, name: &str, prefix: &str, scope: &mut Vec<(String, String)>) -> Option<Option<String>> {
        let mark = scope.len();
        scope.extend(e.ns_decls().map(|(p, u)| (p.to_string(), u.to_string())));
        let found = if e.name == name {
            Some(scope.iter().rev().find(|(p, _)| p == prefix).map(|(_, u)| u.clone()))
        } else {
            e.elements().find_map(|k| find(k, name, prefix, scope))
        };
        scope.truncate(mark);
        found
    }
    find(root, name, prefix, &mut Vec::new()).flatten()
}

/// A prefix kept markup names only in a Markup Compatibility attribute (`mc:Choice/@Requires`,
/// `mc:MustUnderstand`, `mc:ProcessContent`, `mc:PreserveElements`, `mc:PreserveAttributes`),
/// declared outside it, still names the same namespace where it's used after a save and a
/// second round trip. A prefix nothing declares isn't kept.
#[test]
fn kept_extensions_carry_the_namespaces_their_compatibility_attributes_name() {
    let cases = [
        ("mc:Choice", r#"<mc:AlternateContent><mc:Choice Requires="q"><r:reactions/></mc:Choice><mc:Fallback/></mc:AlternateContent>"#),
        ("r:reactions", r#"<r:reactions mc:MustUnderstand="q"/>"#),
        ("r:reactions", r#"<r:reactions mc:ProcessContent="q:box"/>"#),
        ("r:reactions", r#"<r:reactions mc:PreserveElements="q:*"/>"#),
        ("r:reactions", r#"<r:reactions mc:PreserveAttributes="q:note"/>"#),
    ];
    let cex = |declare: &str, ext: &str| {
        format!(
            r#"<x:commentsExtensible xmlns:x="http://schemas.microsoft.com/office/word/2018/wordml/cex" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:r="{CR}"{declare}><x:commentExtensible x:durableId="5A1B2C3D" x:dateUtc="2026-01-02T09:00:00Z"><x:extLst>{ext}</x:extLst></x:commentExtensible></x:commentsExtensible>"#
        )
    };
    for (name, ext) in cases {
        let doc = comments_with_cex(&cex(r#" xmlns:q="urn:q""#, ext));
        for files in [save(&doc), save(&craft_docx::read(&craft_docx::write(&doc).unwrap()).unwrap())] {
            let written = text(&files, "word/commentsExtensible.xml");
            let root = craft_xml::Options::default().strict().parse(written.as_bytes()).unwrap();
            assert_eq!(binding_at(&root, name, "q").as_deref(), Some("urn:q"), "{ext}: {written}");
        }
        let c = comments_with_cex(&cex("", ext)).comments.values().next().cloned().unwrap();
        assert!(c.extensions.is_none() && c.date_utc.is_some(), "{ext}: {c:?}");
    }
}

/// Leaving the dates out keeps a namespace whose prefix happens to be `dateUtc`, so what uses it
/// still opens.
#[test]
fn leaving_dates_out_keeps_a_namespace_named_date_utc() {
    let cex = format!(
        r#"<x:commentsExtensible xmlns:x="http://schemas.microsoft.com/office/word/2018/wordml/cex" xmlns:r="{CR}"><x:commentExtensible x:durableId="5A1B2C3D"><x:extLst xmlns:dateUtc="urn:d"><r:reactions><r:reaction reactionType="2"><dateUtc:item dateUtc="2026-01-04T06:16:20Z"/></r:reaction></r:reactions></x:extLst></x:commentExtensible></x:commentsExtensible>"#
    );
    let mut doc = comments_with_cex(&cex);
    doc.settings.remove_date_time = true;
    let back = craft_docx::read(&craft_docx::write(&doc).unwrap()).unwrap();
    let kept = back.comments.values().find_map(|c| c.extensions.clone()).unwrap_or_default();
    assert!(kept.contains("dateUtc:item") && !kept.contains("2026-01-04"), "{kept}");
}

/// A local `w:date` with a `Z` (older files have no `dateUtc`) isn't saved as the UTC time either,
/// so it doesn't shift by the user's offset after a round trip.
#[test]
fn a_local_date_with_a_z_is_not_saved_as_utc() {
    let mut doc = craft_docx::read(&comments_package(false)).unwrap();
    for c in doc.comments.values_mut() {
        c.date.push('Z');
    }
    let files = save(&doc);
    let (_, dates) = written_ids(&files);
    assert!(dates.iter().all(|(_, utc)| utc.is_empty()), "{dates:?}");
    let back = craft_docx::read(&craft_docx::write(&doc).unwrap()).unwrap();
    assert!(back.comments.values().all(|c| c.date_utc.is_none() && c.date.ends_with('Z')));
}

#[test]
fn comment_references_use_the_documents_annotation_reference_style() {
    let files = save(&craft_docx::read(&comments_package(true)).unwrap());
    let body = text(&files, "word/document.xml");
    assert!(body.contains(r#"<w:rStyle w:val="Odwoaniedokomentarza"/>"#), "{body}");
    assert!(!body.contains(r#"w:val="CommentReference""#), "{body}");
    // A document without that style keeps the English id.
    let mut doc = craft_wordmodel::Document::new();
    Arc::make_mut(&mut doc.styles).styles.retain(|s| !s.name.eq_ignore_ascii_case("annotation reference"));
    let rebuilt = craft_docx::read(&comments_package(true)).unwrap();
    doc.body = rebuilt.body.clone();
    doc.parts = rebuilt.parts.clone();
    doc.comments = rebuilt.comments.clone();
    assert!(text(&save(&doc), "word/document.xml").contains(r#"w:val="CommentReference""#));
}

#[test]
fn localized_default_styles_keep_their_default_flag() {
    let styles = r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:latentStyles w:defLockedState="0" w:defUIPriority="99" w:defSemiHidden="0" w:defUnhideWhenUsed="0" w:defQFormat="0" w:count="376"><w:lsdException w:name="Normal" w:uiPriority="0" w:qFormat="1"/><w:lsdException w:name="heading 1" w:uiPriority="9" w:qFormat="1"/></w:latentStyles><w:style w:type="paragraph" w:default="1" w:styleId="Normalny"><w:name w:val="Normal"/></w:style><w:style w:type="character" w:default="1" w:styleId="Domylnaczcionkaakapitu"><w:name w:val="Default Paragraph Font"/></w:style><w:style w:type="table" w:default="1" w:styleId="Standardowy"><w:name w:val="Normal Table"/></w:style><w:style w:type="numbering" w:default="1" w:styleId="Bezlisty"><w:name w:val="No List"/></w:style></w:styles>"#;
    let bytes =
        package("<w:p><w:r><w:t>x</w:t></w:r></w:p>", &[("rId1", &format!("{REL}/styles"), "styles.xml")], &[], &[("word/styles.xml", styles)]);
    let doc = craft_docx::read(&bytes).unwrap();
    let s = text(&save(&doc), "word/styles.xml");
    for id in ["Domylnaczcionkaakapitu", "Standardowy", "Bezlisty"] {
        assert!(s.contains(&format!(r#"w:default="1" w:styleId="{id}""#)) || s.contains(&format!(r#"w:styleId="{id}" w:default="1""#)), "{id}: {s}");
    }
    assert_eq!(s.matches(r#"w:default="1""#).count(), 4, "{s}");
    // Latent styles come back as they were, before the styles.
    assert!(s.contains(r#"<w:lsdException w:name="heading 1" w:uiPriority="9" w:qFormat="1"/>"#), "{s}");
    assert!(pos(&s, "<w:latentStyles") < pos(&s, "<w:style "), "{s}");
    // A new document still flags the English defaults.
    let fresh = text(&save(&craft_wordmodel::Document::new()), "word/styles.xml");
    assert!(fresh.contains(r#"w:styleId="Normal" w:default="1""#) || fresh.contains(r#"w:default="1" w:styleId="Normal""#), "{fresh}");
}

#[test]
fn list_identities_survive_and_a_copied_definition_gets_none() {
    let numbering = r#"<w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:abstractNum w:abstractNumId="0"><w:nsid w:val="1F2E3D4C"/><w:multiLevelType w:val="hybridMultilevel"/><w:tmpl w:val="0415000F"/><w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="decimal"/><w:lvlText w:val="%1."/></w:lvl></w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num></w:numbering>"#;
    let bytes = package(
        r#"<w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="1"/></w:numPr></w:pPr><w:r><w:t>x</w:t></w:r></w:p>"#,
        &[("rId1", &format!("{REL}/numbering"), "numbering.xml")],
        &[],
        &[("word/numbering.xml", numbering)],
    );
    let mut doc = craft_docx::read(&bytes).unwrap();
    let n = text(&save(&doc), "word/numbering.xml");
    assert!(n.contains(r#"<w:nsid w:val="1F2E3D4C"/><w:multiLevelType w:val="hybridMultilevel"/><w:tmpl w:val="0415000F"/>"#), "{n}");
    // A copy of the definition (as the engine makes when restarting a list) doesn't reuse the nsid.
    let mut copy = doc.numbering.abstracts[0].clone();
    copy.id = 7;
    Arc::make_mut(&mut doc.numbering).abstracts.push(copy);
    let n = text(&save(&doc), "word/numbering.xml");
    assert_eq!(n.matches("1F2E3D4C").count(), 1, "{n}");
}

#[test]
fn malformed_numbering_levels_are_skipped_instead_of_becoming_level_zero() {
    let numbering = r#"<w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:abstractNum w:abstractNumId="0"><w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="decimal"/><w:lvlText w:val="%1."/></w:lvl><w:lvl w:ilvl="-1"><w:numFmt w:val="bullet"/><w:lvlText w:val="bad"/></w:lvl><w:lvl w:ilvl="9"><w:numFmt w:val="bullet"/><w:lvlText w:val="bad"/></w:lvl><w:lvl w:ilvl="999999999999999999999"><w:numFmt w:val="bullet"/><w:lvlText w:val="bad"/></w:lvl></w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="0"/><w:lvlOverride w:ilvl="-1"><w:startOverride w:val="42"/></w:lvlOverride><w:lvlOverride w:ilvl="9"><w:startOverride w:val="43"/></w:lvlOverride><w:lvlOverride><w:startOverride w:val="44"/></w:lvlOverride></w:num></w:numbering>"#;
    let body = r#"<w:p><w:pPr><w:numPr><w:numId w:val="1"/></w:numPr></w:pPr><w:r><w:t>default level</w:t></w:r></w:p><w:p><w:pPr><w:numPr><w:ilvl w:val="2"/><w:numId w:val="1"/></w:numPr></w:pPr><w:r><w:t>valid level</w:t></w:r></w:p><w:p><w:pPr><w:numPr><w:ilvl w:val="-1"/><w:numId w:val="1"/></w:numPr></w:pPr><w:r><w:t>negative level</w:t></w:r></w:p><w:p><w:pPr><w:numPr><w:ilvl w:val="9"/><w:numId w:val="1"/></w:numPr></w:pPr><w:r><w:t>out of range</w:t></w:r></w:p><w:p><w:pPr><w:numPr><w:ilvl w:val="999999999999999999999"/><w:numId w:val="1"/></w:numPr></w:pPr><w:r><w:t>overflow</w:t></w:r></w:p>"#;
    let bytes = package(body, &[("rId1", &format!("{REL}/numbering"), "numbering.xml")], &[], &[("word/numbering.xml", numbering)]);
    let doc = craft_docx::read(&bytes).unwrap();
    let paragraphs: Vec<_> = doc
        .body
        .iter()
        .filter_map(|block| match &**block {
            craft_wordmodel::Block::Para(p) => Some(p),
            _ => None,
        })
        .collect();
    assert_eq!(paragraphs.len(), 5);
    assert!(doc.numbering.level_is_defined(1, 0));
    assert!(!doc.numbering.level_is_defined(1, 2), "effective defaults remain distinguishable from source definitions");
    assert!(doc.numbering.level(1, 2).is_some(), "Word's effective default level remains available");
    assert_eq!(paragraphs[0].props.numbering.map(|n| (n.num, n.level)), Some((1, 0)), "an omitted level defaults to zero");
    assert_eq!(paragraphs[1].props.numbering.map(|n| (n.num, n.level)), Some((1, 2)), "a valid level is preserved");
    for p in &paragraphs[2..] {
        assert_eq!(p.props.numbering, None, "malformed explicit level is not treated as a real list: {}", p.text);
    }
    let level_zero = doc.numbering.level(1, 0).expect("the valid level zero");
    assert_eq!(level_zero.format, craft_wordmodel::section::NumFormat::Decimal);
    assert_eq!(level_zero.start, 1, "invalid level overrides do not affect level zero");
}

#[test]
fn extended_properties_are_kept_without_words_counts_and_personal_ones_follow_the_privacy_setting() {
    let app = r#"<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/extended-properties" xmlns:vt="http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes"><Template>Normal.dotm</Template><TotalTime>42</TotalTime><Pages>107</Pages><Words>40000</Words><Characters>250000</Characters><Application>Microsoft Office Word</Application><DocSecurity>0</DocSecurity><Lines>2000</Lines><Paragraphs>1700</Paragraphs><ScaleCrop>false</ScaleCrop><Manager>M</Manager><Company>Firma</Company><LinksUpToDate>false</LinksUpToDate><CharactersWithSpaces>290000</CharactersWithSpaces><SharedDoc>false</SharedDoc><HyperlinkBase>https://example.com/</HyperlinkBase><HyperlinksChanged>false</HyperlinksChanged><AppVersion>16.0000</AppVersion></Properties>"#;
    let make = |settings: &str| {
        let mut entries = vec![("docProps/app.xml", app)];
        let mut doc_rels = vec![];
        let s;
        if !settings.is_empty() {
            s = settings.to_string();
            entries.push(("word/settings.xml", s.as_str()));
            doc_rels.push(("rId1", format!("{REL}/settings"), "settings.xml"));
        }
        let doc_rels: Vec<(&str, &str, &str)> = doc_rels.iter().map(|(a, b, c)| (*a, b.as_str(), *c)).collect();
        let b = package(
            "<w:p><w:r><w:t>x</w:t></w:r></w:p>",
            &doc_rels,
            &[("rIdA", "http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties", "docProps/app.xml")],
            &entries,
        );
        text(&save(&craft_docx::read(&b).unwrap()), "docProps/app.xml")
    };
    let a = make("");
    for kept in [
        "<Template>Normal.dotm</Template>",
        "<Company>Firma</Company>",
        "<Manager>M</Manager>",
        "<HyperlinkBase>https://example.com/</HyperlinkBase>",
    ] {
        assert!(a.contains(kept), "{kept}: {a}");
    }
    for gone in ["Pages", "Characters", "Lines", "TotalTime", "AppVersion", "Microsoft Office Word", "40000"] {
        assert!(!a.contains(gone), "{gone}: {a}");
    }
    assert!(a.contains("<Application>WordCraft</Application>"));
    // This privacy fixture has no mail-merge source relationships; do not include a dangling source.
    let settings = SETTINGS.replace(r#"<w:mailMerge><w:mainDocumentType w:val="formLetters"/><w:dataSource r:id="rId9"/></w:mailMerge>"#, "");
    let private = make(&settings);
    assert!(!private.contains("Company") && !private.contains("Manager"), "{private}");
    assert!(private.contains("<Template>Normal.dotm</Template>"));
}

/// `name` with its letters' case flipped by the bits of `i`.
fn spelling(name: &str, i: u32) -> String {
    let mut bit = 0;
    name.chars()
        .map(|c| {
            if !c.is_ascii_alphabetic() {
                return c;
            }
            bit += 1;
            if i >> (bit - 1) & 1 == 1 { c.to_ascii_uppercase() } else { c.to_ascii_lowercase() }
        })
        .collect()
}

#[test]
fn a_part_reached_under_many_spellings_is_kept_and_written_once() {
    // Part names are case-insensitive (ECMA-376 Part 2 §6.2.2.3): 64 relationships naming one picture
    // and one unmodelled part in 64 casings are two parts, not 128 copies on save (#3704).
    let ns = r#"xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture""#;
    let n = 64;
    let mut body = String::new();
    let mut doc_rels = Vec::new();
    for i in 0..n {
        body.push_str(&format!(
            r#"<w:p><w:r><w:drawing><wp:inline {ns}><wp:extent cx="9525" cy="9525"/><wp:docPr id="{}" name="p"/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic><pic:blipFill><a:blip r:embed="rIdP{i}"/></pic:blipFill></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>"#,
            i + 1
        ));
        doc_rels.push((format!("rIdP{i}"), format!("{REL}/image"), spelling("media/image1.png", i)));
        doc_rels.push((format!("rIdU{i}"), "urn:example:future-relationship".to_string(), spelling("unknownPart.xml", i)));
    }
    let doc_rels: Vec<(&str, &str, &str)> = doc_rels.iter().map(|(a, b, c)| (a.as_str(), b.as_str(), c.as_str())).collect();
    let picture = format!("\u{89}PNG\r\n\u{1a}\n{}", "0".repeat(64 * 1024));
    let unknown = format!(r#"<x:thing xmlns:x="urn:example:future">{}</x:thing>"#, "x".repeat(64 * 1024));
    let ct = CT.replace("<Default Extension=\"jpeg\"", "<Default Extension=\"png\" ContentType=\"image/png\"/><Default Extension=\"jpeg\"");
    let bytes =
        package(&body, &doc_rels, &[], &[("[Content_Types].xml", &ct), ("word/media/image1.png", &picture), ("word/unknownPart.xml", &unknown)]);
    let doc = craft_docx::read(&bytes).unwrap();
    assert_eq!(doc.media.len(), 1, "{:?}", doc.media.keys());
    let files = save(&doc);
    let named = |s: &str| files.keys().filter(|k| k.to_ascii_lowercase().contains(s)).count();
    assert_eq!(named("unknownpart"), 1, "{:?}", files.keys());
    assert_eq!(files.keys().filter(|k| k.starts_with("word/media/")).count(), 1, "{:?}", files.keys());
    let written: usize = files.values().map(Vec::len).sum();
    assert!(written < 4 * 64 * 1024, "{written} bytes written");
}

#[test]
fn embedded_fonts_load_and_save_when_the_package_spells_the_font_table_in_capitals() {
    // Part names are case-insensitive: a package storing `WORD/FONTTABLE.XML` and its fonts in
    // capitals, related in lower case, loaded no fonts, and a save dropped the table and every
    // font (review of #3872).
    let table = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><w:fonts xmlns:w="{W}" xmlns:r="{R_NS}"><w:font w:name="Quill Sans"><w:embedRegular r:id="rId1"/></w:font></w:fonts>"#
    );
    let table_rels = rels(&[("rId1", &format!("{REL}/font"), "fonts/font1.odttf")]);
    let font_bytes = vec![9u8; 2048];
    let ct = CT.replace(
        "<Default Extension=\"jpeg\"",
        "<Default Extension=\"odttf\" ContentType=\"application/vnd.openxmlformats-officedocument.obfuscatedFont\"/><Default Extension=\"jpeg\"",
    );
    let mut entries: Vec<(&str, Vec<u8>)> = vec![
        ("_rels/.rels", rels(&[("rIdMain", &format!("{REL}/officeDocument"), "word/document.xml")])),
        ("[Content_Types].xml", ct.into_bytes()),
        (
            "word/document.xml",
            format!(r#"<w:document xmlns:w="{W}"><w:body><w:p><w:r><w:t>Body</w:t></w:r></w:p></w:body></w:document>"#).into_bytes(),
        ),
        ("word/_rels/document.xml.rels", rels(&[("rId1", &format!("{REL}/fontTable"), "fontTable.xml")])),
    ];
    entries.push(("WORD/FONTTABLE.XML", table.into_bytes()));
    entries.push(("WORD/_RELS/FONTTABLE.XML.RELS", table_rels));
    entries.push(("WORD/FONTS/FONT1.ODTTF", font_bytes.clone()));
    let doc = craft_docx::read(&zip(&entries)).unwrap();
    let fonts = craft_docx::embedded_fonts(&doc);
    assert_eq!(fonts.len(), 1, "the font loads");
    assert_eq!(fonts[0].data, font_bytes);
    // And through a save and a reopen.
    let saved = craft_docx::write(&doc).unwrap();
    let files = unzip(&saved);
    assert!(files.keys().any(|k| k.to_ascii_lowercase().ends_with("fonttable.xml")), "{:?}", files.keys());
    let again = craft_docx::read(&saved).unwrap();
    let fonts = craft_docx::embedded_fonts(&again);
    assert_eq!(fonts.len(), 1, "the font survives a save: {:?}", files.keys());
    assert_eq!(fonts[0].data, font_bytes);
}

#[test]
fn comment_ids_keep_sparse_threads_reservations_and_missing_parts() {
    use craft_wordmodel::{Comment, Document, Paragraph, Part, PartKind, para_block};
    let mut doc = Document::from_text("Original thread fixture.");
    // New IDs must skip the exact durable ID reserved by a later comment, even one whose
    // missing part keeps it out of commentsIds.xml. Duplicate imported IDs are reassigned.
    let rows = [
        (3, None, Some(90), false, true),
        (11, Some("5a1b2c3d"), Some(90), true, true),
        (25, Some("5A1B2C3D"), Some(999), false, true),
        (90, Some("10000001"), None, true, true),
        (999, Some("10000002"), None, false, false),
        (u32::MAX, Some("not-hex"), Some(3), false, true),
    ];
    for (id, durable, parent, resolved, has_part) in rows {
        let part = id;
        if has_part {
            doc.parts.insert(
                part,
                Part {
                    kind: PartKind::Comment,
                    blocks: vec![para_block(Paragraph::with_text(&format!("Comment {id} β"), Default::default()))],
                    ..Default::default()
                },
            );
        }
        doc.comments.insert(id, Comment { part, parent, resolved, durable_id: durable.map(str::to_string), ..Default::default() });
    }
    let files = save(&doc);
    let (ids, _) = written_ids(&files);
    assert_eq!(ids.iter().map(|(_, id)| id.as_str()).collect::<Vec<_>>(), ["10000003", "5A1B2C3D", "10000004", "10000001", "10000005"]);
    let ex = text(&files, "word/commentsExtended.xml");
    assert_eq!(ex.matches("<w15:commentEx ").count(), 5);
    assert!(ex.contains(r#"w15:paraId="10000001" w15:paraIdParent="10000004" w15:done="0""#));
    assert!(ex.contains(r#"w15:paraId="10000002" w15:paraIdParent="10000004" w15:done="1""#));
    assert!(ex.contains(r#"w15:paraId="10000003" w15:done="0""#), "a missing parent part has no paragraph ID");
    assert!(ex.contains(r#"w15:paraId="10000006" w15:paraIdParent="10000001" w15:done="0""#));
    let back = craft_docx::read(&craft_docx::write(&doc).unwrap()).unwrap();
    // Reader IDs need not match the sparse input IDs. Check the thread by authored text.
    let comment = |text: &str| back.comments.iter().find(|(_, c)| back.plain_text(craft_wordmodel::StoryRef::Part(c.part)).trim() == text).unwrap();
    let (root, _) = comment("Comment 90 β");
    assert_eq!(comment("Comment 3 β").1.parent, Some(*root));
    assert_eq!(comment("Comment 11 β").1.parent, Some(*root));
    assert!(comment("Comment 11 β").1.resolved);
    let (first, _) = comment("Comment 3 β");
    assert_eq!(comment(&format!("Comment {} β", u32::MAX)).1.parent, Some(*first));
}
