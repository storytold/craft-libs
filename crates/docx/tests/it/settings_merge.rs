//! Settings a save used to drop: `word/settings.xml` is merged into the file's original, so
//! settings the model doesn't hold (compatibility options, rsids, document variables, proofing
//! state, "remove personal information"…) survive, and owned ones follow the model. Hand-written
//! fixtures (from the spec), no files produced by Word.

use std::collections::BTreeMap;
use std::io::{Read, Write};

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

fn with_settings(settings: &str) -> Vec<u8> {
    let sources = format!(
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId9" Type="{REL}/mailMergeSource" Target="file:///__wordcraft_missing_source__/letters.xlsx" TargetMode="External"/></Relationships>"#
    );
    package(
        "<w:p><w:r><w:t>Text</w:t></w:r></w:p>",
        &[("rId1", &format!("{REL}/settings"), "settings.xml")],
        &[],
        &[("word/settings.xml", settings), ("word/_rels/settings.xml.rels", &sources)],
    )
}

fn pos(s: &str, needle: &str) -> usize {
    s.find(needle).unwrap_or_else(|| panic!("{needle} missing from {s}"))
}

#[test]
fn unmodelled_settings_survive_a_save_and_owned_ones_follow_the_model() {
    let mut doc = craft_docx::read(&with_settings(SETTINGS)).unwrap();
    assert!(doc.settings.track_changes);
    doc.settings.track_changes = false;
    let files = save(&doc);
    let s = text(&files, "word/settings.xml");
    // Unmodelled children and the original root are kept byte for byte. The regenerated zoom
    // still has its original 120 percent value when the model has not changed it.
    for kept in [
        r#"<w:zoom w:percent="120"/>"#,
        r#"<w:proofState w:spelling="clean" w:grammar="clean"/>"#,
        r#"w:name="overrideTableStyleFontSizeAndJustification""#,
        r#"w:name="enableOpenTypeFeatures""#,
        r#"<w:docVar w:name="ClauseSet" w:val="B"/>"#,
        r#"<w:rsidRoot w:val="00A1B2C3"/>"#,
        r#"<m:mathFont m:val="Cambria Math"/>"#,
        r#"<w:themeFontLang w:val="pl-PL"/>"#,
        r#"<w:decimalSymbol w:val=","/>"#,
        r#"<w:listSeparator w:val=";"/>"#,
        r#"<w14:docId w14:val="1A2B3C4D"/>"#,
        r#"mc:Ignorable="w14 w15""#,
    ] {
        assert!(s.contains(kept), "{kept} lost: {s}");
    }
    // Owned: Track Changes was turned off; one compatibility mode. Mail merge now carries its
    // source with newly allocated relationship IDs rather than leaving dangling references.
    assert!(!s.contains("trackRevisions"), "{s}");
    assert_eq!(s.matches("compatibilityMode").count(), 1, "{s}");
    assert!(s.contains("mailMerge"), "{s}");
    assert!(text(&files, "word/_rels/settings.xml.rels").contains("file:///__wordcraft_missing_source__/letters.xlsx"));
    // The privacy options are the model's (#1062): written once each, and gone when turned off.
    assert_eq!(s.matches("removePersonalInformation").count(), 1, "{s}");
    assert_eq!(s.matches("removeDateAndTime").count(), 1, "{s}");
    // CT_Settings order holds for kept and owned children.
    assert!(pos(&s, "removePersonalInformation") < pos(&s, "defaultTabStop"));
    assert!(pos(&s, "defaultTabStop") < pos(&s, "<w:compat>"));
    assert!(pos(&s, "<w:compat>") < pos(&s, "<w:rsids>"));
    // Turning Track Changes back on inserts it at its place.
    doc.settings.track_changes = true;
    let s = text(&save(&doc), "word/settings.xml");
    assert!(pos(&s, "proofState") < pos(&s, "<w:trackRevisions/>") && pos(&s, "<w:trackRevisions/>") < pos(&s, "defaultTabStop"), "{s}");
    let mut private = doc.clone();
    private.settings.remove_personal_info = false;
    private.settings.remove_date_time = false;
    let p = text(&save(&private), "word/settings.xml");
    assert!(!p.contains("removePersonalInformation") && !p.contains("removeDateAndTime"), "{p}");
    // A second generation is identical.
    let again = craft_docx::read(&craft_docx::write(&doc).unwrap()).unwrap();
    assert_eq!(text(&save(&again), "word/settings.xml"), s);
}

#[test]
fn changed_zoom_survives_merged_settings_and_reads_back_as_page_width() {
    let old_zoom = r#"<w:zoom w:percent="120"/>"#;
    let updated_zoom = r#"<w:zoom w:val="bestFit" w:percent="145"/>"#;
    for original in [SETTINGS.to_string(), SETTINGS.replace(old_zoom, "")] {
        let mut doc = craft_docx::read(&with_settings(&original)).unwrap();
        assert!(doc.passthrough.contains_key("wordcraft:settings.orig"));
        doc.settings.zoom = Some(craft_wordmodel::Zoom { percent: 145, fit: Some(craft_wordmodel::ZoomFit::PageWidth) });
        let bytes = craft_docx::write(&doc).unwrap();
        let settings = text(&unzip(&bytes), "word/settings.xml");
        assert_eq!(settings.matches("<w:zoom ").count(), 1, "{settings}");
        assert!(settings.contains(updated_zoom), "{settings}");
        // The retained root and unrelated child bytes still belong to the original file.
        let root_end = original.find("><w:").unwrap() + 1;
        assert!(settings.starts_with(&original[..root_end]), "{settings}");
        for kept in [
            r#"<w:proofState w:spelling="clean" w:grammar="clean"/>"#,
            r#"<w:docVars><w:docVar w:name="ClauseSet" w:val="B"/></w:docVars>"#,
            r#"<w:rsids><w:rsidRoot w:val="00A1B2C3"/><w:rsid w:val="00A1B2C3"/></w:rsids>"#,
        ] {
            assert!(settings.contains(kept), "{kept} lost: {settings}");
        }
        assert!(pos(&settings, "<w:zoom ") < pos(&settings, "<w:proofState"));
        let again = craft_docx::read(&bytes).unwrap();
        let zoom = again.settings.zoom.unwrap();
        assert_eq!(zoom.percent, 145);
        assert!(matches!(zoom.fit, Some(craft_wordmodel::ZoomFit::PageWidth)));
        assert_eq!(text(&save(&again), "word/settings.xml"), settings);
    }
}

#[test]
fn settings_written_by_wordcraft_keep_no_original() {
    let doc = craft_wordmodel::Document::new();
    let back = craft_docx::read(&craft_docx::write(&doc).unwrap()).unwrap();
    assert!(!back.passthrough.contains_key("wordcraft:settings.orig"), "{:?}", back.passthrough.keys());
    assert_eq!(text(&save(&back), "word/settings.xml"), text(&save(&doc), "word/settings.xml"));
}

/// Gap #5: `w:doNotTrackMoves` (ECMA-376 §17.15.1.29) turns Track moves off; it is read, written
/// back right after `w:trackRevisions` (in a merged part and a generated one), and left out
/// while moves are tracked.
#[test]
fn do_not_track_moves_round_trips() {
    let original = SETTINGS.replace("<w:trackRevisions/>", "<w:trackRevisions/><w:doNotTrackMoves/>");
    let mut doc = craft_docx::read(&with_settings(&original)).unwrap();
    assert!(!doc.settings.track_moves);
    let s = text(&save(&doc), "word/settings.xml");
    assert_eq!(s.matches("<w:doNotTrackMoves/>").count(), 1, "{s}");
    assert!(pos(&s, "<w:trackRevisions/>") < pos(&s, "<w:doNotTrackMoves/>") && pos(&s, "<w:doNotTrackMoves/>") < pos(&s, "<w:defaultTabStop"));
    assert!(s.contains("ClauseSet"), "a merged part: {s}");
    doc.settings.track_moves = true;
    let s = text(&save(&doc), "word/settings.xml");
    assert!(!s.contains("doNotTrackMoves"), "{s}");

    // `w:val="false"` keeps moves tracked.
    let off = SETTINGS.replace("<w:trackRevisions/>", r#"<w:trackRevisions/><w:doNotTrackMoves w:val="false"/>"#);
    assert!(craft_docx::read(&with_settings(&off)).unwrap().settings.track_moves);

    // A document of our own: absent by default, written when off, and read back.
    let mut doc = craft_wordmodel::Document::new();
    assert!(doc.settings.track_moves);
    assert!(!text(&save(&doc), "word/settings.xml").contains("doNotTrackMoves"));
    doc.settings.track_moves = false;
    let s = text(&save(&doc), "word/settings.xml");
    assert!(s.contains("<w:doNotTrackMoves/>"), "{s}");
    assert!(!craft_docx::read(&craft_docx::write(&doc).unwrap()).unwrap().settings.track_moves);
}

#[test]
fn cleared_compatibility_mode_removes_it_from_merged_settings() {
    let mut doc = craft_docx::read(&with_settings(SETTINGS)).unwrap();
    assert_eq!(doc.settings.compat_mode, Some(15));
    assert!(doc.passthrough.contains_key("wordcraft:settings.orig"));

    // Case 1: compat_mode is set to None, and no other compat switch is enabled.
    doc.settings.compat_mode = None;
    let files = save(&doc);
    let s = text(&files, "word/settings.xml");
    assert!(!s.contains("compatibilityMode"), "expected mode removed: {s}");
    assert!(s.contains("<w:compat>"), "expected w:compat preserved for unowned settings: {s}");
    assert!(s.contains("overrideTableStyleFontSizeAndJustification"), "{s}");
    assert!(s.contains("enableOpenTypeFeatures"), "{s}");

    let reopened = craft_docx::read(&craft_docx::write(&doc).unwrap()).unwrap();
    assert_eq!(reopened.settings.compat_mode, None);

    // Case 2: compat_mode is set to None, but another switch is enabled.
    doc.settings.do_not_expand_shift_return = true;
    let files2 = save(&doc);
    let s2 = text(&files2, "word/settings.xml");
    assert!(!s2.contains("compatibilityMode"), "expected mode removed: {s2}");
    assert!(s2.contains("<w:compat>"), "{s2}");
    assert!(s2.contains("<w:doNotExpandShiftReturn/>"), "expected switch preserved: {s2}");
    assert!(s2.contains("overrideTableStyleFontSizeAndJustification"), "{s2}");
    assert!(s2.contains("enableOpenTypeFeatures"), "{s2}");

    let reopened2 = craft_docx::read(&craft_docx::write(&doc).unwrap()).unwrap();
    assert_eq!(reopened2.settings.compat_mode, None);
    assert!(reopened2.settings.do_not_expand_shift_return);
}

/// Hide spelling errors and Hide grammar errors in this document only (`w:hideSpellingErrors`,
/// `w:hideGrammaticalErrors`) are read, written once each (before `w:proofState` in a merged
/// part), written in a generated part, and left out when they are turned off.
#[test]
fn hidden_proofing_errors_round_trip() {
    let original = SETTINGS.replace("<w:proofState", r#"<w:hideSpellingErrors/><w:hideGrammaticalErrors w:val="1"/><w:proofState"#);
    let mut doc = craft_docx::read(&with_settings(&original)).unwrap();
    assert!(doc.settings.hide_spelling_errors && doc.settings.hide_grammar_errors);
    let s = text(&save(&doc), "word/settings.xml");
    assert_eq!(s.matches("hideSpellingErrors").count(), 1, "{s}");
    assert_eq!(s.matches("hideGrammaticalErrors").count(), 1, "{s}");
    assert!(pos(&s, "<w:hideSpellingErrors/>") < pos(&s, "<w:hideGrammaticalErrors/>"));
    assert!(pos(&s, "<w:hideGrammaticalErrors/>") < pos(&s, "<w:proofState"), "{s}");
    assert!(s.contains("ClauseSet"), "a merged part: {s}");
    doc.settings.hide_spelling_errors = false;
    doc.settings.hide_grammar_errors = false;
    let s = text(&save(&doc), "word/settings.xml");
    assert!(!s.contains("hideSpellingErrors") && !s.contains("hideGrammaticalErrors"), "{s}");

    // Turned on in a part that lacks them: inserted once each, before `w:proofState`.
    let mut doc = craft_docx::read(&with_settings(SETTINGS)).unwrap();
    assert!(!doc.settings.hide_spelling_errors && !doc.settings.hide_grammar_errors);
    doc.settings.hide_spelling_errors = true;
    doc.settings.hide_grammar_errors = true;
    let s = text(&save(&doc), "word/settings.xml");
    assert_eq!(s.matches("hideSpellingErrors").count(), 1, "{s}");
    assert_eq!(s.matches("hideGrammaticalErrors").count(), 1, "{s}");
    assert!(pos(&s, "removeDateAndTime") < pos(&s, "<w:hideSpellingErrors/>"), "{s}");
    assert!(pos(&s, "<w:hideGrammaticalErrors/>") < pos(&s, "<w:proofState"), "{s}");

    // `w:val="false"` and `w:val="0"` show them.
    let off = SETTINGS.replace("<w:proofState", r#"<w:hideSpellingErrors w:val="false"/><w:hideGrammaticalErrors w:val="0"/><w:proofState"#);
    let read = craft_docx::read(&with_settings(&off)).unwrap();
    assert!(!read.settings.hide_spelling_errors && !read.settings.hide_grammar_errors);
    let s = text(&save(&read), "word/settings.xml");
    assert!(!s.contains("hideSpellingErrors") && !s.contains("hideGrammaticalErrors"), "{s}");

    // A document of our own: absent by default, written when on, and read back.
    let mut doc = craft_wordmodel::Document::new();
    assert!(!text(&save(&doc), "word/settings.xml").contains("hideSpellingErrors"));
    doc.settings.hide_grammar_errors = true;
    let s = text(&save(&doc), "word/settings.xml");
    assert!(s.contains("<w:hideGrammaticalErrors/>") && !s.contains("hideSpellingErrors"), "{s}");
    let back = craft_docx::read(&craft_docx::write(&doc).unwrap()).unwrap();
    assert!(back.settings.hide_grammar_errors && !back.settings.hide_spelling_errors);
}
