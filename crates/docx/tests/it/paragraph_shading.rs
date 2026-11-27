//! Original OOXML examples from ECMA-376 CT_Shd/ST_Shd. Automatic is a distinct model setting,
//! not a claim about a renderer's chosen color. MS-OI29500 §2.1.553 says paragraph nil is ignored.

use std::io::Write;

use craft_wordmodel::{Document, Rgb};

const YELLOW: Rgb = Rgb(255, 255, 0);
const BLUE: Rgb = Rgb(0, 0, 255);
const ACCENT: Rgb = Rgb(0x22, 0x44, 0x66);

fn paragraph(style: &str, shading: &str, text: &str) -> String {
    format!(r#"<w:p><w:pPr><w:pStyle w:val="{style}"/>{shading}</w:pPr><w:r><w:t>{text}</w:t></w:r></w:p>"#)
}

fn package(body: &str, with_theme: bool) -> Vec<u8> {
    let document =
        format!(r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{body}</w:body></w:document>"#);
    let root = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="document" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    let theme_rel = if with_theme {
        r#"<Relationship Id="theme" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme" Target="theme/theme1.xml"/>"#
    } else {
        ""
    };
    let rels = format!(
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="styles" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>{theme_rel}</Relationships>"#
    );
    let styles = r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
<w:style w:type="paragraph" w:styleId="Shaded"><w:name w:val="Shaded"/><w:basedOn w:val="Normal"/><w:pPr><w:shd w:val="clear" w:fill="FFFF00"/></w:pPr></w:style>
<w:style w:type="paragraph" w:styleId="Automatic"><w:name w:val="Automatic"/><w:basedOn w:val="Shaded"/><w:pPr><w:shd w:val="clear" w:fill="auto"/></w:pPr></w:style>
</w:styles>"#;
    let theme_type = if with_theme {
        r#"<Override PartName="/word/theme/theme1.xml" ContentType="application/vnd.openxmlformats-officedocument.theme+xml"/>"#
    } else {
        ""
    };
    let types = format!(
        r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/>{theme_type}</Types>"#
    );
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, text) in [
        ("[Content_Types].xml", types.as_str()),
        ("_rels/.rels", root),
        ("word/document.xml", document.as_str()),
        ("word/_rels/document.xml.rels", rels.as_str()),
        ("word/styles.xml", styles),
    ] {
        zip.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(text.as_bytes()).unwrap();
    }
    if with_theme {
        let theme = r#"<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" name="Original"><a:themeElements><a:clrScheme name="Original"><a:dk1><a:srgbClr val="000000"/></a:dk1><a:lt1><a:srgbClr val="FFFFFF"/></a:lt1><a:dk2><a:srgbClr val="111111"/></a:dk2><a:lt2><a:srgbClr val="EEEEEE"/></a:lt2><a:accent1><a:srgbClr val="224466"/></a:accent1><a:accent2><a:srgbClr val="335577"/></a:accent2><a:accent3><a:srgbClr val="446688"/></a:accent3><a:accent4><a:srgbClr val="557799"/></a:accent4><a:accent5><a:srgbClr val="6688AA"/></a:accent5><a:accent6><a:srgbClr val="7799BB"/></a:accent6><a:hlink><a:srgbClr val="0000FF"/></a:hlink><a:folHlink><a:srgbClr val="800080"/></a:folHlink></a:clrScheme></a:themeElements></a:theme>"#;
        zip.start_file("word/theme/theme1.xml", zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(theme.as_bytes()).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

fn fills(doc: &Document) -> Vec<Option<Rgb>> {
    doc.body.iter().filter_map(|block| block.as_para()).map(|p| doc.styles.resolve_para(&p.props).shading).collect()
}

fn reopen(doc: &Document) -> Document {
    craft_docx::read(&craft_docx::write(doc).unwrap()).unwrap()
}

#[test]
fn automatic_shading_overrides_inherited_color() {
    // This entire module uses the pre-existing public API, including on the f8 production baseline.
    let body = [
        paragraph("Shaded", "", "inherit"),
        paragraph("Shaded", r#"<w:shd w:val="clear" w:fill="0000FF"/>"#, "fixed"),
        paragraph("Shaded", r#"<w:shd w:val="nil" w:fill="0000FF"/>"#, "ignored nil"),
        paragraph("Shaded", r#"<w:shd w:val="clear" w:fill="auto"/>"#, "automatic"),
        paragraph("Shaded", r#"<w:shd w:val="clear"/>"#, "default automatic background"),
    ]
    .concat();
    let doc = craft_docx::read(&package(&body, false)).unwrap();
    let got = fills(&doc);
    assert_eq!(got.len(), 5);
    assert_eq!(got[0], Some(YELLOW), "an absent direct property inherits");
    assert_eq!(got[1], Some(BLUE), "clear with a fixed background remains colored");
    assert_eq!(got[2], Some(YELLOW), "paragraph nil is ignored");
    assert_eq!(got[3], None, "explicit automatic paragraph background overrides inherited fixed RGB");
    assert_eq!(got[4], None, "clear's omitted background defaults to automatic");
    assert_eq!(fills(&reopen(&doc)), got, "save/read retains the distinction");
}

#[test]
fn invalid_or_unresolved_shading_keeps_inherited_color() {
    for shading in [
        r#"<w:shd/>"#,
        r#"<w:shd w:fill="0000FF"/>"#,
        r#"<w:shd w:val="bogus" w:fill="0000FF"/>"#,
        r#"<w:shd w:val="clear" w:fill="ZZZZZZ"/>"#,
        r#"<w:shd w:val="clear" w:fill="123"/>"#,
        r#"<w:shd w:val="clear" w:themeFill="accent1"/>"#,
        r#"<w:shd w:val="clear" w:themeFill="accent1" w:fill="0000FF"/>"#,
        r#"<w:shd w:val="clear" w:themeFill="unknown"/>"#,
        r#"<w:shd w:val="nil" w:fill="0000FF"/>"#,
        r#"<w:shd w:val="pct20" w:fill="auto"/>"#,
    ] {
        let doc = craft_docx::read(&package(&paragraph("Shaded", shading, "control"), false)).unwrap();
        assert_eq!(fills(&doc), [Some(YELLOW)], "{shading}");
        assert_eq!(fills(&reopen(&doc)), [Some(YELLOW)], "saved {shading}");
    }
}

#[test]
fn fixed_and_theme_backgrounds_stay_colored() {
    let body = [
        paragraph("Shaded", r#"<w:shd w:val="clear" w:fill="0000FF"/>"#, "fixed"),
        paragraph("Shaded", r#"<w:shd w:val="clear" w:themeFill="accent1"/>"#, "theme"),
        paragraph("Shaded", r#"<w:shd w:val="clear" w:themeFill="accent1" w:fill="0000FF"/>"#, "theme wins"),
        paragraph("Shaded", r#"<w:shd w:val="clear" w:themeFill="accent1" w:fill="ZZZZZZ"/>"#, "theme ignores bad fallback"),
        paragraph("Shaded", r#"<w:shd w:val="clear" w:themeFill="accent1" w:fill="123"/>"#, "theme ignores short fallback"),
        paragraph("Shaded", r#"<w:shd w:val="clear" w:themeFill="accent1" w:fill="auto"/>"#, "theme overrides automatic fallback"),
    ]
    .concat();
    let doc = craft_docx::read(&package(&body, true)).unwrap();
    let expected = [Some(BLUE), Some(ACCENT), Some(ACCENT), Some(ACCENT), Some(ACCENT), Some(ACCENT)];
    assert_eq!(fills(&doc), expected);
    assert_eq!(fills(&reopen(&doc)), expected);
}

#[test]
fn automatic_style_shading_survives_save() {
    let body = [
        paragraph("Automatic", "", "style automatic"),
        paragraph("Automatic", r#"<w:shd w:val="clear" w:fill="0000FF"/>"#, "direct color"),
        paragraph("Automatic", r#"<w:shd w:val="nil"/>"#, "nil leaves style automatic"),
    ]
    .concat();
    let doc = craft_docx::read(&package(&body, false)).unwrap();
    let expected = [None, Some(BLUE), None];
    assert_eq!(fills(&doc), expected);
    assert_eq!(fills(&reopen(&doc)), expected);
}

#[test]
fn review_paragraph_shading_theme_none_uses_its_plain_fill() {
    // ST_ThemeColor's none is no theme color, rather than an unresolved named theme.
    let color = Rgb(0xC0, 0x20, 0x30);
    for with_theme in [false, true] {
        let body = [
            paragraph("Shaded", "", "inherit"),
            paragraph("Shaded", r#"<w:shd w:val="clear" w:fill="C02030" w:themeFill="none"/>"#, "plain color"),
            paragraph("Shaded", r#"<w:shd w:val="clear" w:fill="auto" w:themeFill="none"/>"#, "plain automatic"),
            paragraph("Shaded", r#"<w:shd w:val="clear" w:fill="ZZZZZZ" w:themeFill="none"/>"#, "invalid plain fill"),
            paragraph("Shaded", r#"<w:shd w:val="clear" w:fill="C02030" w:themeFill="accent1"/>"#, "named theme"),
            paragraph("Shaded", r#"<w:shd w:val="clear" w:fill="C02030" w:themeFill="unknown"/>"#, "invalid named theme"),
            paragraph("Shaded", r#"<w:shd w:val="clear" w:fill="ZZZZZZ" w:themeFill="accent1"/>"#, "theme precedes fallback"),
        ]
        .concat();
        let doc = craft_docx::read(&package(&body, with_theme)).unwrap();
        let named = Some(if with_theme { ACCENT } else { YELLOW });
        let expected = [Some(YELLOW), Some(color), None, Some(YELLOW), named, Some(YELLOW), named];
        assert_eq!(fills(&doc), expected, "themeFill none uses plain RGB/auto; with_theme={with_theme}");
        assert_eq!(fills(&reopen(&doc)), expected, "saved plain RGB/auto; with_theme={with_theme}");
    }
}
