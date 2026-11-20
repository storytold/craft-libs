//! Theme font slots of Latin text (`w:rFonts/@w:asciiTheme`, `@w:hAnsiTheme`, ISO/IEC 29500-1
//! §17.3.2.26). Saved as the font they name, the text looked the same in Word but no longer
//! followed a change of the theme's fonts. Saving writes a slot back only while it still names the
//! text's font in the document's theme, so a font chosen later is never hidden by it.

use std::io::{Read, Write};

const THEME: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" name="T"><a:themeElements><a:fontScheme name="T"><a:majorFont><a:latin typeface="Georgia"/><a:ea typeface=""/><a:cs typeface=""/></a:majorFont><a:minorFont><a:latin typeface="Verdana"/><a:ea typeface="SimSun"/><a:cs typeface="Arial"/><a:font script="Jpan" typeface="MS Mincho"/><a:font script="Hans" typeface="DengXian"/></a:minorFont></a:fontScheme></a:themeElements></a:theme>"#;

fn docx(body: &str, styles: &str) -> Vec<u8> {
    docx_with_theme(body, styles, THEME)
}

fn docx_with_theme(body: &str, styles: &str, theme: &str) -> Vec<u8> {
    let root = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    let doc_rels = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme" Target="theme/theme1.xml"/></Relationships>"#;
    let doc = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{body}</w:body></w:document>"#
    );
    let styles = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">{styles}</w:styles>"#
    );
    let parts = [
        ("_rels/.rels", root.to_string()),
        ("word/document.xml", doc),
        ("word/_rels/document.xml.rels", doc_rels.to_string()),
        ("word/styles.xml", styles),
        ("word/theme/theme1.xml", theme.to_string()),
    ];
    let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, xml) in &parts {
        zw.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
        zw.write_all(xml.as_bytes()).unwrap();
    }
    zw.finish().unwrap().into_inner()
}

fn part(bytes: &[u8], name: &str) -> String {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut s = String::new();
    z.by_name(name).unwrap().read_to_string(&mut s).unwrap();
    s
}

fn rfonts(xml: &str) -> Vec<String> {
    xml.match_indices("<w:rFonts").map(|(i, _)| xml[i..].split('>').next().unwrap_or("").to_string()).collect()
}

// As pandoc and Word write them: the minor font in the defaults, the major one on a run.
const STYLES: &str = r#"<w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:asciiTheme="minorHAnsi" w:hAnsiTheme="minorHAnsi" w:eastAsiaTheme="minorEastAsia" w:cstheme="minorBidi"/></w:rPr></w:rPrDefault></w:docDefaults>"#;
const BODY: &str = r#"<w:p><w:r><w:rPr><w:rFonts w:asciiTheme="majorHAnsi" w:hAnsiTheme="majorHAnsi"/></w:rPr><w:t>Heading text</w:t></w:r></w:p>"#;

#[test]
fn latin_theme_slots_survive_a_save() {
    let d = craft_docx::read(&docx(BODY, STYLES)).unwrap();
    assert_eq!(d.styles.default_chr.font.as_deref(), Some("Verdana"), "layout still reads the font the slot names");
    assert_eq!(d.styles.default_chr.font_ascii_theme, Some(craft_wordmodel::props::ThemeFont::MinorHAnsi));
    let run = &d.body[0].as_para().unwrap().runs[0];
    assert_eq!((run.props.font.as_deref(), run.props.font_hansi_theme), (Some("Georgia"), Some(craft_wordmodel::props::ThemeFont::MajorHAnsi)));

    let out = craft_docx::write(&d).unwrap();
    let styles = rfonts(&part(&out, "word/styles.xml"));
    assert!(styles.iter().any(|f| f.contains(r#"w:asciiTheme="minorHAnsi""#) && f.contains(r#"w:hAnsiTheme="minorHAnsi""#)), "{styles:?}");
    let doc = rfonts(&part(&out, "word/document.xml"));
    assert!(doc.iter().any(|f| f.contains(r#"w:asciiTheme="majorHAnsi""#) && f.contains(r#"w:hAnsiTheme="majorHAnsi""#)), "{doc:?}");
    // And again after a second round trip.
    let again = craft_docx::read(&out).unwrap();
    assert_eq!(again.styles.default_chr.font_ascii_theme, Some(craft_wordmodel::props::ThemeFont::MinorHAnsi));
}

#[test]
fn a_font_chosen_later_is_not_hidden_by_the_slot() {
    let mut d = craft_docx::read(&docx(BODY, STYLES)).unwrap();
    // Set directly, as the font commands do, without touching the slots.
    if let Some(craft_wordmodel::Block::Para(p)) = d.body.get_mut(0).map(std::sync::Arc::make_mut) {
        p.runs[0].props.font = Some("Courier New".into());
    }
    let doc = rfonts(&part(&craft_docx::write(&d).unwrap(), "word/document.xml"));
    assert!(doc.iter().any(|f| f.contains(r#"w:ascii="Courier New""#)), "{doc:?}");
    assert!(!doc.iter().any(|f| f.contains("w:asciiTheme") || f.contains("w:hAnsiTheme")), "the slot would show Georgia in Word: {doc:?}");
}

#[test]
fn a_theme_font_change_without_the_text_drops_the_slot() {
    let mut d = craft_docx::read(&docx(BODY, STYLES)).unwrap();
    // The theme's minor font changes but the defaults keep Verdana: the slot no longer names it.
    d.settings.minor_font = "Arial".into();
    let styles = rfonts(&part(&craft_docx::write(&d).unwrap(), "word/styles.xml"));
    assert!(!styles.iter().any(|f| f.contains(r#"w:asciiTheme="minorHAnsi""#)), "{styles:?}");
    assert!(styles.iter().any(|f| f.contains(r#"w:ascii="Verdana""#)), "{styles:?}");
}

#[test]
fn a_named_font_overlaid_replaces_the_latin_slots() {
    use craft_wordmodel::props::CharProps;
    let mut base = CharProps {
        font: Some("Verdana".into()),
        font_ascii_theme: Some(craft_wordmodel::props::ThemeFont::MinorHAnsi),
        font_hansi_theme: Some(craft_wordmodel::props::ThemeFont::MinorHAnsi),
        ..Default::default()
    };
    base.overlay(&CharProps { font: Some("Courier New".into()), ..Default::default() });
    assert_eq!((base.font_ascii_theme, base.font_hansi_theme), (None, None));
    let mut themed = CharProps { font: Some("Verdana".into()), ..Default::default() };
    themed.overlay(&CharProps {
        font: Some("Georgia".into()),
        font_ascii_theme: Some(craft_wordmodel::props::ThemeFont::MajorHAnsi),
        ..Default::default()
    });
    assert_eq!(themed.font_ascii_theme, Some(craft_wordmodel::props::ThemeFont::MajorHAnsi));
}

/// The theme's East Asian, complex-script and per-script fonts are written back with it: the
/// `w:eastAsiaTheme` and `w:cstheme` slots a save keeps resolve through them in Word, and an
/// empty `a:ea` would leave them naming no font.
#[test]
fn the_themes_east_asian_and_script_fonts_survive_a_save() {
    let out = craft_docx::write(&craft_docx::read(&docx(BODY, STYLES)).unwrap()).unwrap();
    let theme = part(&out, "word/theme/theme1.xml");
    let minor = theme.split("<a:minorFont>").nth(1).and_then(|m| m.split("</a:minorFont>").next()).unwrap_or_default();
    for want in [
        r#"<a:ea typeface="SimSun"/>"#,
        r#"<a:cs typeface="Arial"/>"#,
        r#"<a:font script="Jpan" typeface="MS Mincho"/>"#,
        r#"<a:font script="Hans" typeface="DengXian"/>"#,
    ] {
        assert!(minor.contains(want), "{want} missing from the minor font: {theme}");
    }
    let major = theme.split("<a:majorFont>").nth(1).and_then(|m| m.split("</a:majorFont>").next()).unwrap_or_default();
    assert!(major.contains(r#"<a:ea typeface=""/>"#) && !major.contains("a:font "), "the major font keeps its empty faces: {theme}");
}

/// The minor font's part of a saved theme.
fn minor_font(out: &[u8]) -> String {
    let theme = part(out, "word/theme/theme1.xml");
    theme.split("<a:minorFont>").nth(1).and_then(|m| m.split("</a:minorFont>").next()).unwrap_or_default().to_string()
}

/// A per-script font for a script called `cs` stays a per-script font: it is not taken for the
/// theme's `a:cs` face.
#[test]
fn a_script_named_cs_is_not_the_cs_face() {
    let theme = THEME.replace(r#"<a:font script="Jpan""#, r#"<a:font script="cs" typeface="Tahoma"/><a:font script="Jpan""#);
    let minor = minor_font(&craft_docx::write(&craft_docx::read(&docx_with_theme(BODY, STYLES, &theme)).unwrap()).unwrap());
    assert!(minor.contains(r#"<a:cs typeface="Arial"/>"#), "{minor}");
    assert!(minor.contains(r#"<a:font script="cs" typeface="Tahoma"/>"#), "{minor}");
}

/// The 256 per-script fonts kept are 256 usable ones: empty default faces don't use them up.
#[test]
fn empty_default_faces_dont_count_against_the_script_limit() {
    let scripts: String = (0..256).map(|i| format!(r#"<a:font script="S{i}" typeface="F{i}"/>"#)).collect();
    let theme = THEME.replace(
        r#"<a:ea typeface="SimSun"/><a:cs typeface="Arial"/><a:font script="Jpan" typeface="MS Mincho"/><a:font script="Hans" typeface="DengXian"/>"#,
        &format!(r#"<a:ea typeface=""/><a:cs typeface=""/>{scripts}"#),
    );
    let minor = minor_font(&craft_docx::write(&craft_docx::read(&docx_with_theme(BODY, STYLES, &theme)).unwrap()).unwrap());
    assert!(minor.contains(r#"<a:font script="S255" typeface="F255"/>"#), "{minor}");
}

/// Latin slots naming different faces: the model has one Latin font, the ASCII one, so only the
/// slot that still names it is written; the other is left out rather than made to name a face
/// it didn't.
#[test]
fn a_latin_slot_naming_another_face_is_not_written() {
    let body = r#"<w:p><w:r><w:rPr><w:rFonts w:asciiTheme="minorHAnsi" w:hAnsiTheme="majorHAnsi"/></w:rPr><w:t>Mixed</w:t></w:r></w:p>"#;
    let d = craft_docx::read(&docx(body, STYLES)).unwrap();
    assert_eq!(d.body[0].as_para().unwrap().runs[0].props.font.as_deref(), Some("Verdana"));
    let doc = rfonts(&part(&craft_docx::write(&d).unwrap(), "word/document.xml"));
    let run = doc.iter().find(|f| f.contains("Verdana")).unwrap_or_else(|| panic!("{doc:?}"));
    assert!(run.contains(r#"w:asciiTheme="minorHAnsi""#) && !run.contains("w:hAnsiTheme"), "{run}");
}

/// A long script id (a language tag, which DrawingML allows) doesn't use up a place the
/// mappings font resolution takes need: the last of 256 short ones is still saved.
#[test]
fn a_language_tag_script_doesnt_push_out_a_short_one() {
    let short: String = (0..255).map(|i| format!(r#"<a:font script="S{i}" typeface="F{i}"/>"#)).collect();
    let theme = THEME.replace(
        r#"<a:font script="Jpan" typeface="MS Mincho"/><a:font script="Hans" typeface="DengXian"/>"#,
        &format!(r#"<a:font script="zh-Hant-TW" typeface="PMingLiU"/>{short}<a:font script="Arab" typeface="Times New Roman"/>"#),
    );
    let minor = minor_font(&craft_docx::write(&craft_docx::read(&docx_with_theme(BODY, STYLES, &theme)).unwrap()).unwrap());
    assert!(minor.contains(r#"<a:font script="Arab" typeface="Times New Roman"/>"#), "{minor}");
    assert!(minor.contains(r#"<a:font script="zh-Hant-TW" typeface="PMingLiU"/>"#), "{minor}");
}
