//! Theme colour references of WordprocessingML colours (ECMA-376 Part 1, 5th edition: §17.3.2.6
//! `w:color`, §17.3.2.40 `w:u`, §17.3.4 `CT_Border`, §17.3.5 `CT_Shd`, §17.18.97
//! `ST_ThemeColor`, §17.18.98 `ST_UcharHexNumber`). A `w:themeColor` (or a shading's
//! `w:themeFill`) supersedes the fixed colour stored beside it, which only a reader without the
//! theme shows; a save that writes the resolved colour instead freezes the text to it, and the
//! colour no longer follows the document's theme (storytold/wordcraft#6256).
//!
//! The packages are written here from strings, for these tests; the stored colours deliberately
//! differ from what their theme references resolve to, so a save that drops a reference, or
//! replaces a stored colour with the resolved one, shows.

use std::collections::BTreeMap;
use std::io::{Read, Write};

use craft_wordmodel::props::{Border, TextColor};
use craft_wordmodel::{Document, Rgb};

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const PKG_RELS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

/// A twelve-colour theme of our own.
fn theme() -> String {
    theme_with("336699")
}

/// [`theme`] with another first accent colour.
fn theme_with(accent1: &str) -> String {
    let slots = [
        ("dk1", "142638"),
        ("lt1", "E9F1F7"),
        ("dk2", "314457"),
        ("lt2", "DDE6EF"),
        ("accent1", accent1),
        ("accent2", "8A4B32"),
        ("accent3", "28725A"),
        ("accent4", "77558B"),
        ("accent5", "B27935"),
        ("accent6", "426E87"),
        ("hlink", "3359AD"),
        ("folHlink", "775577"),
    ];
    let colours: String = slots.iter().map(|(n, v)| format!(r#"<a:{n}><a:srgbClr val="{v}"/></a:{n}>"#)).collect();
    let fonts: String = ["majorFont", "minorFont"]
        .iter()
        .map(|n| format!(r#"<a:{n}><a:latin typeface="Liberation Sans"/><a:ea typeface=""/><a:cs typeface=""/></a:{n}>"#))
        .collect();
    let fill = r#"<a:solidFill><a:srgbClr val="E9F1F7"/></a:solidFill>"#.repeat(3);
    let line = r#"<a:ln w="12700"><a:solidFill><a:srgbClr val="142638"/></a:solidFill></a:ln>"#.repeat(3);
    let effects = "<a:effectStyle><a:effectLst/></a:effectStyle>".repeat(3);
    format!(
        r#"<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" name="Synthetic colour reference probe"><a:themeElements><a:clrScheme name="Synthetic twelve">{colours}</a:clrScheme><a:fontScheme name="Synthetic fonts">{fonts}</a:fontScheme><a:fmtScheme name="Synthetic format"><a:fillStyleLst>{fill}</a:fillStyleLst><a:lnStyleLst>{line}</a:lnStyleLst><a:effectStyleLst>{effects}</a:effectStyleLst><a:bgFillStyleLst>{fill}</a:bgFillStyleLst></a:fmtScheme></a:themeElements></a:theme>"#
    )
}

const STYLES: &str = r#"<w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style><w:style w:type="paragraph" w:styleId="SyntheticThemeStyle"><w:name w:val="Synthetic Theme Style"/><w:basedOn w:val="Normal"/><w:rPr><w:color w:val="445566" w:themeColor="accent6" w:themeTint="33"/></w:rPr></w:style>"#;

/// A run, paragraph, table, cell and page border and shading each in a theme colour, every one
/// with a tint or shade; two shadings with a theme colour on both channels; and a run in a fixed
/// colour and one in the automatic colour beside them.
const BODY: &str = r#"<w:p><w:pPr><w:pBdr><w:top w:val="single" w:sz="8" w:color="456789" w:themeColor="accent5" w:themeTint="66"/></w:pBdr><w:shd w:val="clear" w:color="102030" w:fill="203040" w:themeColor="accent4" w:themeShade="80" w:themeFill="accent3" w:themeFillTint="99"/></w:pPr><w:r><w:rPr><w:color w:val="A1B2C3" w:themeColor="accent1" w:themeShade="BF"/><w:u w:val="single" w:color="112233" w:themeColor="accent2" w:themeTint="66"/><w:bdr w:val="single" w:sz="8" w:color="234567" w:themeColor="accent3" w:themeShade="80"/><w:shd w:val="clear" w:color="auto" w:fill="345678" w:themeFill="accent2" w:themeFillTint="80"/></w:rPr><w:t>Synthetic themed run</w:t></w:r></w:p><w:p><w:pPr><w:pStyle w:val="SyntheticThemeStyle"/></w:pPr><w:r><w:t>Synthetic styled run</w:t></w:r></w:p><w:p><w:r><w:rPr><w:color w:val="123456"/></w:rPr><w:t>Literal control</w:t></w:r><w:r><w:rPr><w:color w:val="auto"/></w:rPr><w:t>Automatic control</w:t></w:r></w:p><w:tbl><w:tblPr><w:tblW w:w="4800" w:type="dxa"/><w:tblBorders><w:left w:val="single" w:sz="8" w:color="56789A" w:themeColor="accent6" w:themeShade="AA"/></w:tblBorders><w:shd w:val="clear" w:color="auto" w:fill="607080" w:themeFill="accent5" w:themeFillShade="CC"/></w:tblPr><w:tblGrid><w:gridCol w:w="4800"/></w:tblGrid><w:tr><w:tc><w:tcPr><w:tcW w:w="4800" w:type="dxa"/><w:tcBorders><w:bottom w:val="single" w:sz="8" w:color="6789AB" w:themeColor="accent1" w:themeTint="80"/></w:tcBorders><w:shd w:val="clear" w:color="304050" w:fill="405060" w:themeColor="accent5" w:themeTint="40" w:themeFill="accent4" w:themeFillShade="80"/></w:tcPr><w:p><w:r><w:t>Synthetic shaded cell</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:p/><w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="720" w:footer="720" w:gutter="0"/><w:pgBorders><w:bottom w:val="single" w:sz="8" w:color="789ABC" w:themeColor="accent2" w:themeShade="99"/></w:pgBorders></w:sectPr>"#;

/// A package of `body` and `styles`, with the theme above unless `with_theme` is false (and then
/// no relationship or content type names one either).
fn package(body: &str, styles: &str, with_theme: bool) -> Vec<u8> {
    package_with(body, styles, with_theme.then(theme).as_deref(), "", None)
}

/// A package of `body`, `styles`, `theme` (none: no theme part), the `settings` given, and a
/// nested WordprocessingML document `chunk` as `rChunk` (for a `w:altChunk`).
fn package_with(body: &str, styles: &str, theme: Option<&str>, settings: &str, chunk: Option<&[u8]>) -> Vec<u8> {
    let decl = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#;
    let mut rels = format!(
        r#"<Relationship Id="rStyles" Type="{REL}/styles" Target="styles.xml"/><Relationship Id="rSettings" Type="{REL}/settings" Target="settings.xml"/>"#
    );
    let mut types = String::new();
    if theme.is_some() {
        rels.push_str(&format!(r#"<Relationship Id="rTheme" Type="{REL}/theme" Target="theme/theme1.xml"/>"#));
        types.push_str(r#"<Override PartName="/word/theme/theme1.xml" ContentType="application/vnd.openxmlformats-officedocument.theme+xml"/>"#);
    }
    if chunk.is_some() {
        rels.push_str(&format!(r#"<Relationship Id="rChunk" Type="{REL}/aFChunk" Target="afchunk.docx"/>"#));
        types.push_str(
            r#"<Default Extension="docx" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>"#,
        );
    }
    let mut parts: Vec<(String, Vec<u8>)> = vec![
        ("_rels/.rels".into(), format!(r#"{decl}<Relationships xmlns="{PKG_RELS}"><Relationship Id="rMain" Type="{REL}/officeDocument" Target="word/document.xml"/></Relationships>"#).into_bytes()),
        ("word/document.xml".into(), format!(r#"{decl}<w:document xmlns:w="{W}" xmlns:r="{REL}"><w:body>{body}</w:body></w:document>"#).into_bytes()),
        ("word/styles.xml".into(), format!(r#"{decl}<w:styles xmlns:w="{W}">{styles}</w:styles>"#).into_bytes()),
        ("word/settings.xml".into(), format!(r#"{decl}<w:settings xmlns:w="{W}">{settings}</w:settings>"#).into_bytes()),
        ("word/_rels/document.xml.rels".into(), format!(r#"{decl}<Relationships xmlns="{PKG_RELS}">{rels}</Relationships>"#).into_bytes()),
        (
            "[Content_Types].xml".into(),
            format!(
                r#"{decl}<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/><Override PartName="/word/settings.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml"/>{types}</Types>"#
            )
            .into_bytes(),
        ),
    ];
    if let Some(theme) = theme {
        parts.push(("word/theme/theme1.xml".into(), format!("{decl}{theme}").into_bytes()));
    }
    if let Some(chunk) = chunk {
        parts.push(("word/afchunk.docx".into(), chunk.to_vec()));
    }
    let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, bytes) in &parts {
        zw.start_file(name.as_str(), zip::write::SimpleFileOptions::default()).unwrap();
        zw.write_all(bytes).unwrap();
    }
    zw.finish().unwrap().into_inner()
}

fn entry(pkg: &[u8], name: &str) -> Option<Vec<u8>> {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(pkg)).unwrap();
    let mut f = z.by_name(name).ok()?;
    let mut bytes = Vec::new();
    f.read_to_end(&mut bytes).unwrap();
    Some(bytes)
}

/// `name` parsed with its WordprocessingML names prefixed `w:`, whatever prefix the part uses.
fn part(pkg: &[u8], name: &str) -> craft_xml::El {
    let canonical = craft_xml::Canonical::new(&[("w", W)]);
    let options = craft_xml::Options { names: craft_xml::Names::Canonical(&canonical), ..Default::default() };
    options.parse(&entry(pkg, name).unwrap_or_else(|| panic!("{name} is missing"))).unwrap()
}

/// The colour attributes of `e`: its fixed colours (`w:val`, `w:color`, `w:fill`) and its theme
/// references (`w:theme…`). Other attributes, a border's width and spacing, are left out.
fn colour_attrs(e: &craft_xml::El) -> BTreeMap<String, String> {
    ["w:val", "w:color", "w:fill", "w:themeColor", "w:themeTint", "w:themeShade", "w:themeFill", "w:themeFillTint", "w:themeFillShade"]
        .iter()
        .filter_map(|a| Some((a.to_string(), e.attr(a)?.to_string())))
        .collect()
}

fn nth<'a>(e: &'a craft_xml::El, name: &'a str, i: usize) -> &'a craft_xml::El {
    e.children(name).nth(i).unwrap_or_else(|| panic!("no {name} #{i} in {}", e.name))
}

/// Each colour owner of the fixture, found by its place in the document and not by searching (a
/// reference written only inside a tracked change's old properties must not count), with its
/// colour attributes.
fn owners(pkg: &[u8]) -> Vec<(&'static str, BTreeMap<String, String>)> {
    let doc = part(pkg, "word/document.xml");
    let body = doc.child("w:body").unwrap();
    let p0 = nth(body, "w:p", 0);
    let ppr = p0.child("w:pPr").unwrap();
    let rpr = nth(p0, "w:r", 0).child("w:rPr").unwrap();
    let controls = nth(body, "w:p", 2);
    let tblpr = body.child("w:tbl").unwrap().child("w:tblPr").unwrap();
    let tcpr = body.child("w:tbl").unwrap().child("w:tr").unwrap().child("w:tc").unwrap().child("w:tcPr").unwrap();
    let styles = part(pkg, "word/styles.xml");
    let style = styles.children("w:style").find(|s| s.attr("w:styleId") == Some("SyntheticThemeStyle")).unwrap();
    let path = |e: &craft_xml::El, names: &[&str]| e.path(names).map(colour_attrs).unwrap_or_default();
    vec![
        ("run colour", path(rpr, &["w:color"])),
        ("run underline", path(rpr, &["w:u"])),
        ("run border", path(rpr, &["w:bdr"])),
        ("run shading", path(rpr, &["w:shd"])),
        ("paragraph shading", path(ppr, &["w:shd"])),
        ("paragraph top border", path(ppr, &["w:pBdr", "w:top"])),
        ("table shading", path(tblpr, &["w:shd"])),
        ("table left border", path(tblpr, &["w:tblBorders", "w:left"])),
        ("cell shading", path(tcpr, &["w:shd"])),
        ("cell bottom border", path(tcpr, &["w:tcBorders", "w:bottom"])),
        ("page bottom border", path(body, &["w:sectPr", "w:pgBorders", "w:bottom"])),
        ("style colour", path(style, &["w:rPr", "w:color"])),
        ("fixed colour", path(nth(controls, "w:r", 0), &["w:rPr", "w:color"])),
        ("automatic colour", path(nth(controls, "w:r", 1), &["w:rPr", "w:color"])),
    ]
}

/// The colours the document is shown in, at the same owners.
fn shown(d: &Document) -> Vec<(&'static str, Option<TextColor>)> {
    let paras: Vec<_> = d.body.iter().filter_map(|b| b.as_para()).collect();
    let table = d.body.iter().find_map(|b| b.as_table()).unwrap();
    let cell = &table.rows[0].cells[0];
    let run = &paras[0].runs[0].props;
    let rgb = |c: Option<Rgb>| c.map(TextColor::Rgb);
    let line = |b: Option<Border>| b.and_then(|b| b.color).map(TextColor::Rgb);
    let style = d.styles.get("SyntheticThemeStyle").unwrap();
    vec![
        ("run colour", run.color),
        ("run underline", rgb(run.underline_color)),
        ("run border", line(run.border)),
        ("run shading", rgb(run.shading)),
        ("paragraph shading", rgb(paras[0].props.shading)),
        ("paragraph top border", line(paras[0].props.borders.and_then(|b| b.top))),
        ("table shading", rgb(table.props.shading)),
        ("table left border", line(table.props.borders.and_then(|b| b.left))),
        ("cell shading", rgb(cell.props.shading)),
        ("cell bottom border", line(cell.props.borders.and_then(|b| b.bottom))),
        ("page bottom border", line(d.last_section.page_borders.and_then(|b| b.bottom))),
        ("style colour", style.chr.color),
        ("fixed colour", paras[2].runs[0].props.color),
        ("automatic colour", paras[2].runs[1].props.color),
    ]
}

#[test]
fn theme_colour_references_and_their_stored_colours_survive_two_saves() {
    let input = package(BODY, STYLES, true);
    let expected = owners(&input);
    let themed = expected.iter().filter(|(_, a)| a.keys().any(|k| k.starts_with("w:theme"))).count();
    let references = expected.iter().flat_map(|(_, a)| a.keys()).filter(|k| *k == "w:themeColor" || *k == "w:themeFill").count();
    assert_eq!((themed, references), (12, 14), "the fixture's twelve themed colours and their fourteen references");
    let first = craft_docx::read(&input).unwrap();
    let paint = shown(&first);
    assert_ne!(paint[0].1, Some(TextColor::Rgb(Rgb(0xA1, 0xB2, 0xC3))), "the run is shown in its theme colour, not the stored one");
    let mut doc = first;
    for save in 1..=2 {
        let saved = craft_docx::write(&doc).unwrap();
        let got = owners(&saved);
        for ((name, want), (_, have)) in expected.iter().zip(&got) {
            assert_eq!(have, want, "{name}, save {save}: the theme reference, its tint or shade and the stored colour");
        }
        doc = craft_docx::read(&saved).unwrap();
        assert_eq!(shown(&doc), paint, "save {save}: the colours shown");
        let texts: Vec<String> = doc.body.iter().filter_map(|b| b.as_para()).map(|p| p.text.to_string()).collect();
        assert_eq!(texts[..3], ["Synthetic themed run", "Synthetic styled run", "Literal controlAutomatic control"], "save {save}");
        assert_eq!(doc.body.iter().filter_map(|b| b.as_para()).nth(1).and_then(|p| p.props.style.clone()).as_deref(), Some("SyntheticThemeStyle"));
    }
}

/// The paragraphs of a saved body, each with its text and its runs' properties.
fn saved_runs(pkg: &[u8]) -> Vec<(String, Vec<craft_xml::El>)> {
    let doc = part(pkg, "word/document.xml");
    let body = doc.child("w:body").unwrap().clone();
    body.children("w:p")
        .map(|p| {
            let runs: Vec<_> = p.children("w:r").collect();
            let text = runs.iter().filter_map(|r| r.child("w:t")).map(|t| t.text().into_owned()).collect();
            (text, runs.iter().map(|r| r.child("w:rPr").cloned().unwrap_or_else(|| craft_xml::El::new("w:rPr"))).collect())
        })
        .collect()
}

/// The colour attributes of the element `name` in the properties of the run whose paragraph is
/// `text` (its first run).
fn saved(pkg: &[u8], text: &str, name: &str) -> BTreeMap<String, String> {
    let runs = saved_runs(pkg);
    let (_, props) = runs.iter().find(|(t, _)| t == text).unwrap_or_else(|| panic!("no paragraph {text:?}"));
    props[0].child(name).map(colour_attrs).unwrap_or_default()
}

fn attrs(list: &[(&str, &str)]) -> BTreeMap<String, String> {
    list.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

/// Without a theme colour scheme, theme colours show their stored colours, and a save writes a
/// theme of its own: written back, the references would take that theme's colours once read
/// again. They are saved as the colours shown, which read the same after every save.
#[test]
fn a_document_without_a_theme_saves_its_theme_colours_as_shown() {
    let mut doc = craft_docx::read(&package(BODY, STYLES, false)).unwrap();
    let paint = shown(&doc);
    assert_eq!(paint[0].1, Some(TextColor::Rgb(Rgb(0xA1, 0xB2, 0xC3))), "the run shows its stored colour");
    for save in 1..=2 {
        doc = craft_docx::read(&craft_docx::write(&doc).unwrap()).unwrap();
        assert_eq!(shown(&doc), paint, "save {save}");
    }
}

/// A colour set since the file was read is saved as set: one changed in place, as an older
/// caller might without touching its theme colour record, and one set to the colour it already
/// looked like (it was chosen). A border whose width alone changed keeps its theme colour.
#[test]
fn a_colour_set_since_it_was_read_is_saved_as_set() {
    let mut d = craft_docx::read(&package(BODY, STYLES, true)).unwrap();
    let craft_wordmodel::Block::Para(p) = std::sync::Arc::make_mut(&mut d.body[0]) else { panic!("a paragraph") };
    let run = &mut p.runs[0].props;
    run.color = Some(TextColor::Rgb(Rgb(1, 2, 3)));
    let underline = run.underline_color;
    run.set_underline_color(underline);
    if let Some(b) = run.border.as_mut() {
        b.width = 2.0;
    }
    let out = craft_docx::write(&d).unwrap();
    let got: BTreeMap<_, _> = owners(&out).into_iter().collect();
    assert_eq!(got["run colour"], attrs(&[("w:val", "010203")]));
    let hex = underline.map(|c| c.hex()).unwrap_or_default();
    assert_eq!(got["run underline"], attrs(&[("w:val", "single"), ("w:color", &hex)]));
    let input: BTreeMap<_, _> = input_owners().into_iter().collect();
    assert_eq!(got["run border"], input["run border"], "its colour is as read");
    assert_eq!(got["run shading"], input["run shading"], "left alone");
}

fn input_owners() -> Vec<(&'static str, BTreeMap<String, String>)> {
    owners(&package(BODY, STYLES, true))
}

/// Each form a theme colour takes is saved as read: a tint and a shade together, a name the
/// colour scheme mapping gives, a shading pattern with a theme colour on both colours, and a
/// `nil` shading. A theme colour that `ST_ThemeColor` doesn't have, or `none`, names none: the
/// stored colour is shown and saved alone. A reference without the stored colour `w:color`
/// requires gets the colour shown.
#[test]
fn each_form_of_a_theme_colour_is_saved_as_read() {
    let run = |text: &str, rpr: &str| format!(r#"<w:p><w:r><w:rPr>{rpr}</w:rPr><w:t>{text}</w:t></w:r></w:p>"#);
    let forms = [
        ("tint and shade", r#"<w:color w:val="A1B2C3" w:themeColor="accent1" w:themeTint="99" w:themeShade="BF"/>"#),
        ("mapped", r#"<w:color w:val="000000" w:themeColor="text1"/>"#),
        ("none", r#"<w:color w:val="123456" w:themeColor="none"/>"#),
        ("unknown", r#"<w:color w:val="A1B2C3" w:themeColor="futureSlot" w:themeShade="BF"/>"#),
        ("no stored colour", r#"<w:color w:themeColor="accent2"/>"#),
        (
            "pattern",
            r#"<w:shd w:val="pct25" w:color="auto" w:fill="FFFFFF" w:themeColor="accent1" w:themeFill="background1" w:themeFillShade="F2"/>"#,
        ),
        ("nil", r#"<w:shd w:val="nil" w:fill="FFFFFF" w:themeFill="accent2"/>"#),
    ];
    let body: String = forms.iter().map(|(t, rpr)| run(t, rpr)).collect();
    let mapping = r#"<w:clrSchemeMapping w:bg1="light1" w:t1="accent2" w:bg2="light2" w:t2="dark2" w:accent1="accent1" w:accent2="accent2" w:accent3="accent3" w:accent4="accent4" w:accent5="accent5" w:accent6="accent6" w:hyperlink="hyperlink" w:followedHyperlink="followedHyperlink"/>"#;
    let input = package_with(&body, STYLES, Some(&theme()), mapping, None);
    let d = craft_docx::read(&input).unwrap();
    let out = craft_docx::write(&craft_docx::read(&craft_docx::write(&d).unwrap()).unwrap()).unwrap();
    for (text, rpr) in forms {
        let name = if rpr.starts_with("<w:shd") { "w:shd" } else { "w:color" };
        let read = saved(&input, text, name);
        let want = match text {
            "none" => attrs(&[("w:val", "123456")]),
            "unknown" => attrs(&[("w:val", "A1B2C3")]),
            "no stored colour" => {
                let p = d.body.iter().filter_map(|b| b.as_para()).find(|p| p.text == text).unwrap();
                let Some(TextColor::Rgb(c)) = p.runs[0].props.color else { panic!("a colour") };
                attrs(&[("w:val", &c.hex()), ("w:themeColor", "accent2")])
            }
            _ => read,
        };
        assert_eq!(saved(&out, text, name), want, "{text}");
    }
    let p = d.body.iter().filter_map(|b| b.as_para()).find(|p| p.text == "unknown").unwrap();
    assert_eq!(p.runs[0].props.color, Some(TextColor::Rgb(Rgb(0xA1, 0xB2, 0xC3))), "an unknown theme colour shows the stored one");
}

/// The page colour (`w:background`, §17.2.1) follows its theme colour as text does, and is saved
/// as read.
#[test]
fn a_page_colour_in_a_theme_colour_is_shown_in_it_and_saved_as_read() {
    let background = r#"<w:background w:color="FFFFFF" w:themeColor="accent2" w:themeShade="BF"/>"#;
    let mut input = package(BODY, STYLES, true);
    let doc = String::from_utf8(entry(&input, "word/document.xml").unwrap()).unwrap().replace("<w:body>", &format!("{background}<w:body>"));
    input = rezip(&input, "word/document.xml", doc.as_bytes());
    let d = craft_docx::read(&input).unwrap();
    let accent = craft_wordmodel::props::ThemeRef { color: craft_wordmodel::props::ThemeColor::Accent2, tint: None, shade: Some(0xBF) };
    assert_eq!(d.settings.page_color, accent.resolve(&d.settings.theme_colors, None));
    assert_ne!(d.settings.page_color, Some(Rgb(0xFF, 0xFF, 0xFF)));
    let out = craft_docx::write(&d).unwrap();
    let saved = part(&out, "word/document.xml").child("w:background").map(colour_attrs);
    assert_eq!(saved, part(&input, "word/document.xml").child("w:background").map(colour_attrs));
}

/// A clear pattern over an automatic fill paints nothing (`CT_Shd`, §17.3.5), whatever its
/// pattern colour names. Given a theme colour, such a shading is read with its record, and the
/// two move as one: a style based on a shaded style, with that shading, shows none, before and
/// after a save.
#[test]
fn a_shading_that_paints_nothing_with_a_theme_colour_replaces_the_inherited_one() {
    let styles = format!(
        r#"{STYLES}<w:style w:type="paragraph" w:styleId="SyntheticShaded"><w:name w:val="Synthetic Shaded"/><w:basedOn w:val="Normal"/><w:pPr><w:shd w:val="clear" w:color="auto" w:fill="F2E6A0"/></w:pPr></w:style><w:style w:type="paragraph" w:styleId="SyntheticCleared"><w:name w:val="Synthetic Cleared"/><w:basedOn w:val="SyntheticShaded"/><w:pPr><w:shd w:val="clear" w:color="auto" w:themeColor="accent1" w:fill="auto"/></w:pPr></w:style>"#
    );
    let para = |style: &str| format!(r#"<w:p><w:pPr><w:pStyle w:val="{style}"/></w:pPr><w:r><w:t>{style}</w:t></w:r></w:p>"#);
    let body = format!("{}{}", para("SyntheticShaded"), para("SyntheticCleared"));
    let first = craft_docx::read(&package(&body, &styles, true)).unwrap();
    let again = craft_docx::read(&craft_docx::write(&first).unwrap()).unwrap();
    for (when, d) in [("read", &first), ("saved and read again", &again)] {
        let shading = |text: &str| {
            let p = d.body.iter().filter_map(|b| b.as_para()).find(|p| p.text == text).unwrap();
            d.styles.resolve_para(&p.props).shading
        };
        assert!(shading("SyntheticShaded").is_some(), "{when}: the based-on style is shaded");
        assert_eq!(shading("SyntheticCleared"), None, "{when}: the derived style's shading replaces it");
    }
}

/// `pkg` with the part `name` replaced by `bytes`.
fn rezip(pkg: &[u8], name: &str, bytes: &[u8]) -> Vec<u8> {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(pkg)).unwrap();
    let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for i in 0..z.len() {
        let mut f = z.by_index(i).unwrap();
        let entry_name = f.name().unwrap().to_string();
        let mut content = Vec::new();
        f.read_to_end(&mut content).unwrap();
        zw.start_file(entry_name.as_str(), zip::write::SimpleFileOptions::default()).unwrap();
        zw.write_all(if entry_name == name { bytes } else { &content }).unwrap();
    }
    zw.finish().unwrap().into_inner()
}

/// A tracked formatting change keeps the theme colours of both sides: the colour now, and the
/// one before the change (`w:rPrChange`), which share their reader and writer.
#[test]
fn a_tracked_colour_change_keeps_the_theme_colours_of_both_sides() {
    let body = r#"<w:p><w:r><w:rPr><w:color w:val="112233" w:themeColor="accent1" w:themeShade="BF"/><w:rPrChange w:id="1" w:author="Tester" w:date="2026-01-01T00:00:00Z"><w:rPr><w:color w:val="445566" w:themeColor="accent2" w:themeTint="80"/></w:rPr></w:rPrChange></w:rPr><w:t>Changed</w:t></w:r></w:p>"#;
    let input = package(body, STYLES, true);
    let out = craft_docx::write(&craft_docx::read(&input).unwrap()).unwrap();
    let rpr = |pkg: &[u8]| {
        let runs = saved_runs(pkg);
        let props = runs[0].1[0].clone();
        let old = props.path(&["w:rPrChange", "w:rPr", "w:color"]).map(colour_attrs);
        (props.child("w:color").map(colour_attrs), old)
    };
    assert_eq!(rpr(&out), rpr(&input));
}

/// A nested document (`w:altChunk`) keeps its theme colours only when its theme is its
/// container's: saved in the container, they would name the container's colours. Under another
/// theme they are saved as the colours they are shown in.
#[test]
fn a_nested_document_keeps_its_theme_colours_only_under_the_same_theme() {
    let chunk_body = r#"<w:p><w:r><w:rPr><w:color w:val="A1B2C3" w:themeColor="accent1" w:themeShade="BF"/></w:rPr><w:t>Nested</w:t></w:r></w:p>"#;
    let outer = r#"<w:p><w:r><w:t>Outer</w:t></w:r></w:p><w:altChunk r:id="rChunk"/><w:sectPr/>"#;
    for (accent1, keeps) in [("336699", true), ("993366", false)] {
        let chunk = package_with(chunk_body, STYLES, Some(&theme_with(accent1)), "", None);
        let alone = craft_docx::read(&chunk).unwrap();
        let input = package_with(outer, STYLES, Some(&theme()), "", Some(&chunk));
        let d = craft_docx::read(&input).unwrap();
        let nested = |d: &Document| d.body.iter().filter_map(|b| b.as_para()).find(|p| p.text == "Nested").map(|p| p.runs[0].props.color);
        assert_eq!(nested(&d), nested(&alone), "shown as on its own, under {accent1}");
        let got = saved(&craft_docx::write(&d).unwrap(), "Nested", "w:color");
        let want = if keeps {
            attrs(&[("w:val", "A1B2C3"), ("w:themeColor", "accent1"), ("w:themeShade", "BF")])
        } else {
            let Some(Some(TextColor::Rgb(c))) = nested(&d) else { panic!("a colour") };
            attrs(&[("w:val", &c.hex())])
        };
        assert_eq!(got, want, "under {accent1}");
    }
}

/// Word 2003 XML has no theme colours: a document saved as it gets the colours shown, not the
/// fixed colours stored beside the references it can't carry.
#[test]
fn word_2003_xml_gets_the_colours_shown() {
    let d = craft_docx::read(&package(BODY, STYLES, true)).unwrap();
    let root = craft_xml::parse(&craft_docx::write_wordml2003(&d).unwrap()).unwrap();
    let run =
        root.descendants().find(|e| e.name == "w:r" && e.child("w:t").is_some_and(|t| t.text() == "Synthetic themed run")).expect("the themed run");
    let Some(TextColor::Rgb(c)) = shown(&d)[0].1 else { panic!("a colour") };
    assert_eq!(run.path(&["w:rPr", "w:color"]).and_then(|e| e.attr("w:val")), Some(c.hex().as_str()));
}
