//! What WordCraft doesn't model survives a save that doesn't change it: a paragraph nothing
//! edited is written back as it was read, with the definitions it names (`craft_wordmodel::kept`,
//! `odt::kept`).
//!
//! Every sample here (and any sample file added for the no-edit loss gate) is written by us for
//! the purpose, with our own text: never a file or text taken from LibreOffice, a bug tracker,
//! another application's templates or anyone else's document, so every sample can be committed.

use crate::text::tests::{NS_T, content_of, package, styles_of};
use crate::text::{export, import};
use craft_wordmodel::Pos;

/// A paragraph with an element and an attribute WordCraft doesn't read, in automatic styles
/// naming a common style WordCraft doesn't model either, and a font face.
const CONTENT: &str = r#"<office:font-face-decls><style:font-face style:name="Odd Font" svg:font-family="'Odd Font'"/></office:font-face-decls><office:automatic-styles><style:style style:name="P1" style:family="paragraph" style:parent-style-name="Plaque"><style:paragraph-properties fo:margin-left="1in" loext:odd="yes"/></style:style><style:style style:name="T1" style:family="text"><style:text-properties style:font-name="Odd Font" fo:font-weight="bold"/></style:style></office:automatic-styles>"#;
const PARA: &str = r#"<text:p text:style-name="P1" x:unknown="1">Keep <text:span text:style-name="T1">this</text:span><text:hidden-text text:condition="ooow:1" text:string-value="secret"/> as it is.</text:p>"#;

fn doc() -> craft_wordmodel::Document {
    let styles = format!(
        r#"<office:document-styles {NS_T}><office:styles><style:style style:name="Plaque" style:family="paragraph" style:class="odd"><style:text-properties fo:font-size="14pt"/></style:style></office:styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {NS_T} xmlns:x="urn:example:x" xmlns:loext="urn:org:documentfoundation:names:experimental:office:xmlns:loext:1.0">{CONTENT}<office:body><office:text>{PARA}<text:p>Other.</text:p></office:text></office:body></office:document-content>"#
    );
    import(&package(&content, Some(&styles))).unwrap()
}

#[test]
fn an_unedited_paragraph_is_written_as_it_was_read() {
    let d = doc();
    let saved = export(&d).unwrap();
    let c = content_of(&saved);
    assert!(c.contains(PARA), "{c}");
    // What it names, as read: its automatic styles and font face, the common style its style is
    // based on.
    assert!(c.contains(r#"<style:style style:name="P1" style:family="paragraph" style:parent-style-name="Plaque"><style:paragraph-properties fo:margin-left="1in" loext:odd="yes"/></style:style>"#), "{c}");
    assert!(c.contains(r#"<style:style style:name="T1" style:family="text">"#), "{c}");
    assert!(c.contains(r#"style:name="Odd Font""#), "{c}");
    let s = styles_of(&saved);
    assert!(s.contains(r#"style:name="Plaque""#), "{s}");
    // WordCraft's own automatic styles can't take the same names.
    assert!(!c.contains(r#"style:name="P1" style:family="paragraph"><"#) || c.matches(r#"style:name="P1""#).count() == 1, "{c}");
    // And it reads back the same.
    let back = import(&saved).unwrap();
    assert_eq!(back.body[0].as_para().unwrap().text, d.body[0].as_para().unwrap().text);
    // Saved again, still as read.
    assert!(content_of(&export(&back).unwrap()).contains(PARA));
}

#[test]
fn an_edited_paragraph_is_written_from_the_model() {
    let mut d = doc();
    d.insert_text(&Pos::body(0, 0), "Now ", &Default::default()).unwrap();
    let c = content_of(&export(&d).unwrap());
    assert!(!c.contains("text:hidden-text") && !c.contains("x:unknown"), "{c}");
    assert!(c.contains("Now Keep"), "{c}");
    // The paragraph nobody touched still is as read.
    assert!(c.contains("<text:p>Other.</text:p>"), "{c}");
    // WordCraft's names keep clear of the file's.
    assert!(c.contains(r#"text:style-name="wcP"#) || !c.contains(r#"style:name="P1""#), "{c}");
}

#[test]
fn a_style_edited_in_wordcraft_is_written_as_edited() {
    // A custom style a kept paragraph names: WordCraft's version, not the one read.
    let styles = format!(
        r#"<office:document-styles {NS_T}><office:styles><style:style style:name="Note" style:family="paragraph"><style:text-properties fo:font-size="9pt"/></style:style></office:styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {NS_T}><office:body><office:text><text:p text:style-name="Note">Small.</text:p></office:text></office:body></office:document-content>"#
    );
    let mut d = import(&package(&content, Some(&styles))).unwrap();
    let id = d.body[0].as_para().unwrap().props.style.clone().unwrap();
    if let Some(st) = std::sync::Arc::make_mut(&mut d.styles).styles.iter_mut().find(|s| s.id == id) {
        st.chr.size = Some(20.0);
    }
    let saved = export(&d).unwrap();
    assert!(content_of(&saved).contains(r#"<text:p text:style-name="Note">Small.</text:p>"#));
    let s = styles_of(&saved);
    assert!(s.contains(r#"fo:font-size="20pt""#) && !s.contains(r#"fo:font-size="9pt""#), "{s}");
}

#[test]
fn a_kept_sequence_is_declared() {
    let content = format!(
        r#"<office:document-content {NS_T}><office:body><office:text><text:p>Figure <text:sequence text:name="Figure" text:formula="ooow:Figure+1" style:num-format="1">1</text:sequence>: a cat</text:p></office:text></office:body></office:document-content>"#
    );
    let d = import(&package(&content, None)).unwrap();
    let c = content_of(&export(&d).unwrap());
    assert!(c.contains(r#"<text:sequence-decl text:display-outline-level="0" text:name="Figure"/>"#), "{c}");
}

#[test]
fn hostile_kept_markup_does_not_panic() {
    for body in [
        r#"<text:p text:style-name="P1">a<text:span text:style-name="P1">b</text:span></text:p>"#,
        r#"<text:p text:style-name="">x</text:p><text:p text:style-name="&amp;&lt;">y</text:p>"#,
        r#"<text:p><text:sequence text:name="">1</text:sequence><text:sequence>2</text:sequence></text:p>"#,
        // Sections left open, closed twice, inside a list, empty.
        r#"<text:section text:name="a"><text:p>x</text:p>"#,
        r#"<text:p>y</text:p></text:section></text:section><text:p>z</text:p>"#,
        r#"<text:list><text:list-item><text:section text:name="b"><text:p>w</text:p></text:section></text:list-item></text:list>"#,
        r#"<text:section text:name="c"/><text:section text:name="d"></text:section><text:p>v</text:p>"#,
    ] {
        let content = format!(
            r#"<office:document-content {NS_T}><office:automatic-styles><style:style style:name="P1" style:family="paragraph" style:parent-style-name="P1" style:list-style-name="P1"/></office:automatic-styles><office:body><office:text>{body}</office:text></office:body></office:document-content>"#
        );
        let d = import(&package(&content, None)).unwrap();
        let saved = export(&d).unwrap();
        let _ = import(&saved).unwrap();
    }
}

#[test]
fn unedited_style_definitions_are_written_as_read() {
    // What the model has no place for (an unused list style, the footnote configuration, a
    // style attribute nobody reads) survives while nobody changes a style.
    let styles = format!(
        r#"<office:document-styles {NS_T}><office:styles><style:default-style style:family="paragraph"><style:paragraph-properties style:tab-stop-distance="0.5in"/></style:default-style><style:style style:name="Standard" style:family="paragraph" style:class="text" x:odd="1"><style:paragraph-properties fo:text-align="justify"/></style:style><text:list-style style:name="Unused"><text:list-level-style-bullet text:level="1" text:bullet-char="✓"/></text:list-style><text:notes-configuration text:note-class="footnote" style:num-format="a" text:start-value="0" text:footnotes-position="page" text:start-numbering-at="document"/></office:styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {NS_T}><office:body><office:text><text:p text:style-name="Standard">Text.</text:p></office:text></office:body></office:document-content>"#
    );
    let pkg = package(&content, Some(&styles.replace("<office:document-styles ", r#"<office:document-styles xmlns:x="urn:example:x" "#)));
    let mut d = import(&pkg).unwrap();
    let s = styles_of(&export(&d).unwrap());
    assert!(s.contains(r#"<text:list-style style:name="Unused">"#), "{s}");
    assert!(s.contains(r#"x:odd="1""#) && s.contains(r#"xmlns:x="urn:example:x""#), "{s}");
    assert_eq!(s.matches("<text:notes-configuration text:note-class=\"footnote\"").count(), 1, "{s}");
    assert_eq!(s.matches(r#"<style:style style:name="Standard""#).count(), 1, "{s}");
    // A style changed in WordCraft: that one is WordCraft's, the others still the file's.
    if let Some(st) = std::sync::Arc::make_mut(&mut d.styles).styles.iter_mut().find(|s| s.id == "Quote") {
        st.chr.bold = Some(true);
    }
    let s = styles_of(&export(&d).unwrap());
    assert!(s.contains(r#"x:odd="1""#), "{s}");
    assert!(
        s.split(r#"<style:style style:name="Quotations""#)
            .nth(1)
            .is_some_and(|q| q.split("</style:style>").next().is_some_and(|q| q.contains(r#"fo:font-weight="bold""#))),
        "{s}"
    );
    assert_eq!(s.matches(r#"<style:style style:name="Standard""#).count(), 1, "{s}");
}

#[test]
fn a_file_a_kept_definition_names_is_written_with_it() {
    // LibreOffice's templates fill pages with a bitmap their styles name (`draw:fill-image`);
    // written as read without the picture, LibreOffice refused the whole file.
    let png = [b"\x89PNG\r\n\x1a\n".as_slice(), &[3u8; 64]].concat();
    let styles = format!(
        r#"<office:document-styles {NS_T} xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" xmlns:xlink="http://www.w3.org/1999/xlink"><office:styles><draw:fill-image draw:name="Paper" xlink:href="Pictures/paper.png" xlink:type="simple"/><style:style style:name="Standard" style:family="paragraph"/></office:styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {NS_T}><office:body><office:text><text:p>Text.</text:p></office:text></office:body></office:document-content>"#
    );
    let pkg = crate::text::tests::zip_of(&[("content.xml", content.as_bytes()), ("styles.xml", styles.as_bytes()), ("Pictures/paper.png", &png)]);
    let d = import(&pkg).unwrap();
    let saved = export(&d).unwrap();
    assert!(styles_of(&saved).contains(r#"xlink:href="Pictures/paper.png""#));
    let mut z = crate::Package::open(&saved, &crate::Limits::DEFAULT).unwrap();
    let got = z.read("Pictures/paper.png").unwrap().unwrap();
    assert_eq!(got, png);
    let m = crate::text::tests::entry(&saved, "META-INF/manifest.xml");
    assert!(m.contains(r#"manifest:full-path="Pictures/paper.png""#), "{m}");
}

/// A picture as LibreOffice writes one it draws as a shape: a frame with a text style, and in its
/// `draw:image` an empty paragraph (ODF 1.3 §10.4.4 lets a picture hold text).
const SHAPE_PICTURE: &str = r#"<draw:frame text:anchor-type="paragraph" draw:z-index="0" draw:name="Banner" draw:style-name="gr1" draw:text-style-name="P1" svg:width="19cm" svg:height="3cm" svg:x="-3cm" svg:y="0cm"><draw:image xlink:href="Pictures/x.png" xlink:type="simple" xlink:show="embed" xlink:actuate="onLoad" draw:mime-type="image/png"><text:p/></draw:image></draw:frame>"#;
const AFTER_PICTURE: &str = r#"<text:p text:style-name="P2">Text after it.</text:p>"#;

/// A package whose text is `body`, with the styles and the picture [`SHAPE_PICTURE`] names.
fn shape_picture_doc(body: &str) -> (Vec<u8>, Vec<u8>) {
    let png = [b"\x89PNG\r\n\x1a\n".as_slice(), &[5u8; 64]].concat();
    let content = format!(
        r#"<office:document-content {NS_T} xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0" xmlns:xlink="http://www.w3.org/1999/xlink"><office:automatic-styles><style:style style:name="P1" style:family="paragraph"/><style:style style:name="P2" style:family="paragraph"><style:paragraph-properties fo:margin-top="3.4cm"/></style:style><style:style style:name="gr1" style:family="graphic"><style:graphic-properties style:wrap="none" style:horizontal-pos="from-left" style:horizontal-rel="paragraph" style:vertical-pos="from-top" style:vertical-rel="paragraph"/></style:style></office:automatic-styles><office:body><office:text>{body}</office:text></office:body></office:document-content>"#
    );
    (crate::text::tests::zip_of(&[("content.xml", content.as_bytes()), ("Pictures/x.png", &png)]), png)
}

#[test]
fn a_picture_holding_an_empty_paragraph_stays_in_its_own() {
    let first = format!(r#"<text:p text:style-name="Standard">{SHAPE_PICTURE}</text:p>"#);
    let (pkg, png) = shape_picture_doc(&format!("{first}{AFTER_PICTURE}"));
    let d = import(&pkg).unwrap();
    // Two paragraphs, as written: the picture in the first, the text in the second. The picture's
    // own paragraph used to end the one around it, and the picture joined the next one.
    assert_eq!(d.body.len(), 2);
    let (a, b) = (d.body[0].as_para().unwrap(), d.body[1].as_para().unwrap());
    assert_eq!(a.objects.len(), 1, "the picture is in the first paragraph");
    assert!(b.objects.is_empty());
    assert_eq!(b.text, "Text after it.");

    let saved = export(&d).unwrap();
    let c = content_of(&saved);
    assert!(c.contains(&format!("{first}\n{AFTER_PICTURE}")), "as read: {c}");
    for def in [r#"style:name="P1""#, r#"style:name="P2""#, r#"style:name="gr1""#] {
        assert!(c.contains(def), "{def}: {c}");
    }
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(saved.clone())).unwrap();
    let mut got = Vec::new();
    std::io::Read::read_to_end(&mut z.by_name("Pictures/x.png").unwrap(), &mut got).unwrap();
    assert_eq!(got, png);
    // Saved again, still as read.
    assert!(content_of(&export(&import(&saved).unwrap()).unwrap()).contains(&format!("{first}\n{AFTER_PICTURE}")));
}

#[test]
fn a_grouped_picture_holding_an_empty_paragraph_stays_in_its_own() {
    // LibreOffice writes every picture in a group this way.
    let member = SHAPE_PICTURE.replace(r#" text:anchor-type="paragraph" draw:z-index="0""#, "");
    let first = format!(
        r#"<text:p text:style-name="Standard"><draw:g text:anchor-type="paragraph" draw:z-index="0" draw:style-name="gr1">{member}<draw:rect draw:style-name="gr1" svg:width="2cm" svg:height="1cm" svg:x="0cm" svg:y="4cm"><text:p/></draw:rect></draw:g></text:p>"#
    );
    let (pkg, _) = shape_picture_doc(&format!("{first}{AFTER_PICTURE}"));
    let d = import(&pkg).unwrap();
    assert_eq!(d.body.len(), 2);
    assert_eq!(d.body[0].as_para().unwrap().objects.len(), 1, "the group is in the first paragraph");
    assert_eq!(d.body[1].as_para().unwrap().text, "Text after it.");
    let c = content_of(&export(&d).unwrap());
    assert!(c.contains(&format!("{first}\n{AFTER_PICTURE}")), "{c}");
}

#[test]
fn a_picture_holding_a_paragraph_in_a_cell_or_a_text_box_stays_in_its_own() {
    let inner = format!(r#"<text:p text:style-name="Standard">Before {SHAPE_PICTURE} after.</text:p>"#);
    let cell =
        format!(r#"<table:table><table:table-column/><table:table-row><table:table-cell>{inner}</table:table-cell></table:table-row></table:table>"#);
    let boxed = format!(
        r#"<text:p><draw:frame text:anchor-type="paragraph" svg:width="5cm" svg:height="5cm"><draw:text-box>{inner}</draw:text-box></draw:frame></text:p>"#
    );
    let (pkg, _) = shape_picture_doc(&format!("{cell}{boxed}"));
    let d = import(&pkg).unwrap();
    let craft_wordmodel::Block::Table(t) = &*d.body[0] else { panic!("a table: {:?}", d.body[0]) };
    let in_cell = &t.rows[0].cells[0].blocks;
    assert_eq!(in_cell.len(), 1);
    assert_eq!(in_cell[0].as_para().unwrap().text, "Before \u{FFFC} after.");
    let saved = content_of(&export(&d).unwrap());
    assert_eq!(saved.matches(inner.as_str()).count(), 2, "as read: {saved}");
}

/// All the text of `blocks`, paragraph by paragraph.
fn texts(blocks: &[std::sync::Arc<craft_wordmodel::Block>]) -> Vec<String> {
    blocks.iter().filter_map(|b| b.as_para()).map(|p| p.text.clone()).collect()
}

#[test]
fn words_in_a_picture_are_not_lost() {
    // LibreOffice draws them on the picture. Skipped like an empty paragraph, they were only in the
    // paragraph's markup as read: gone once the paragraph was edited, or saved to another format.
    let held = SHAPE_PICTURE.replace("<text:p/>", r#"<text:p text:style-name="P1">Words on it</text:p>"#);
    let first = format!(r#"<text:p text:style-name="Standard">{held}</text:p>"#);
    let (pkg, _) = shape_picture_doc(&format!("{first}{AFTER_PICTURE}"));
    let mut d = import(&pkg).unwrap();
    assert!(texts(&d.body).iter().any(|t| t.contains("Words on it")), "{:?}", texts(&d.body));
    assert!(d.plain_text(craft_wordmodel::StoryRef::Body).contains("Words on it"));
    d.kept.clear();
    assert!(content_of(&export(&d).unwrap()).contains("Words on it"));
}

#[test]
fn a_picture_in_a_header_holding_an_empty_paragraph_stays_in_its_own_and_words_stay() {
    let header = |held: &str| {
        let styles = format!(
            r#"<office:document-styles {NS_T} xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0" xmlns:xlink="http://www.w3.org/1999/xlink"><office:automatic-styles><style:page-layout style:name="pm1"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm"/></style:page-layout></office:automatic-styles><office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm1"><style:header><text:p>Head {held} line</text:p></style:header></style:master-page></office:master-styles></office:document-styles>"#
        );
        let png = [b"\x89PNG\r\n\x1a\n".as_slice(), &[5u8; 64]].concat();
        let content = format!(
            r#"<office:document-content {NS_T}><office:body><office:text><text:p>Body</text:p></office:text></office:body></office:document-content>"#
        );
        let pkg = crate::text::tests::zip_of(&[("content.xml", content.as_bytes()), ("styles.xml", styles.as_bytes()), ("Pictures/x.png", &png)]);
        let d = import(&pkg).unwrap();
        let part = d.parts.values().find(|p| p.kind == craft_wordmodel::PartKind::Header).expect("a header");
        texts(&part.blocks)
    };
    // An empty paragraph in the picture doesn't end the header's.
    assert_eq!(header(SHAPE_PICTURE), vec!["Head \u{FFFC} line".to_string()]);
    // Words in one are read.
    let held = SHAPE_PICTURE.replace("<text:p/>", "<text:p>Words on it</text:p>");
    assert!(header(&held).iter().any(|t| t.contains("Words on it")), "{:?}", header(&held));
}

/// The paragraphs of a document whose first paragraph holds [`SHAPE_PICTURE`] with `inside` in
/// place of its empty paragraph, followed by [`AFTER_PICTURE`].
fn with_in_picture(inside: &str) -> craft_wordmodel::Document {
    let held = SHAPE_PICTURE.replace("<text:p/>", inside);
    let (pkg, _) = shape_picture_doc(&format!(r#"<text:p text:style-name="Standard">A {held} B</text:p>{AFTER_PICTURE}"#));
    import(&pkg).unwrap()
}

#[test]
fn words_in_a_picture_written_as_cdata_or_after_a_comment_are_not_lost() {
    // Read as the parser reads them, not as the bytes look: CDATA is text, and a comment naming
    // the picture's end doesn't end it.
    for inside in ["<text:p><![CDATA[Words on it]]></text:p>", "<!-- </draw:image> --><text:p>Words on it</text:p>"] {
        let mut d = with_in_picture(inside);
        assert!(texts(&d.body).iter().any(|t| t.contains("Words on it")), "{inside}: {:?}", texts(&d.body));
        assert!(d.plain_text(craft_wordmodel::StoryRef::Body).contains("Words on it"), "{inside}");
        d.kept.clear();
        assert!(content_of(&export(&d).unwrap()).contains("Words on it"), "{inside}");
    }
}

#[test]
fn an_empty_picture_with_a_greater_than_sign_in_its_markup_stays_in_its_own() {
    // A `>` in an attribute or a comment isn't a word.
    for inside in [r#"<text:p text:style-name="a&gt;b" text:odd="1 > 0"/>"#, "<!-- a > b --><text:p/>", "<text:p><![CDATA[ ]]></text:p>"] {
        let d = with_in_picture(inside);
        assert_eq!(texts(&d.body), ["A \u{FFFC} B", "Text after it."], "{inside}");
    }
}

#[test]
fn many_pictures_without_their_ends_are_read_in_linear_time() {
    // Each picture's contents are looked at only so far: a picture whose end is far off, or never
    // comes, isn't looked through to the end of the file once for every picture.
    let run = |n: usize| {
        let stray = "<draw:image></x>".repeat(n);
        let open = "<draw:image>".repeat(n);
        let (pkg, _) = shape_picture_doc(&format!(
            r#"<text:p><draw:frame svg:width="1cm" svg:height="1cm">{stray}</draw:frame></text:p><text:p><draw:frame svg:width="1cm" svg:height="1cm">{open}</draw:frame></text:p>"#
        ));
        let t = std::time::Instant::now();
        assert!(import(&pkg).is_ok());
        t.elapsed()
    };
    let took = run(20_000);
    assert!(took.as_secs_f32() < 3.0, "{took:?}");
}

/// An unused namesake's definition written back as read (`MyCustom` beside an edited
/// `My_20_Custom`) keeps the picture it names in the package.
#[test]
fn an_unused_namesakes_picture_stays_when_the_used_style_is_edited() {
    use std::sync::Arc;
    let png = [b"\x89PNG\r\n\x1a\n".as_slice(), &[5u8; 64]].concat();
    let styles = format!(
        r##"<office:document-styles {NS_T} xmlns:xlink="http://www.w3.org/1999/xlink"><office:styles><style:style style:name="Standard" style:family="paragraph" style:class="text"/><style:style style:name="My_20_Custom" style:family="paragraph" style:parent-style-name="Standard"><style:text-properties fo:font-size="9pt"/></style:style><style:style style:name="MyCustom" style:family="paragraph" style:parent-style-name="Standard"><style:paragraph-properties><style:background-image xlink:href="Pictures/only.png" xlink:type="simple"/></style:paragraph-properties></style:style></office:styles></office:document-styles>"##
    );
    let content = format!(
        r#"<office:document-content {NS_T}><office:body><office:text><text:p text:style-name="My_20_Custom">Text</text:p></office:text></office:body></office:document-content>"#
    );
    let pkg = crate::text::tests::zip_of(&[("content.xml", content.as_bytes()), ("styles.xml", styles.as_bytes()), ("Pictures/only.png", &png)]);
    let mut d = import(&pkg).unwrap();
    let id = d.body[0].as_para().unwrap().props.style.clone().unwrap();
    Arc::make_mut(&mut d.styles).get_mut(&id).unwrap().chr.size = Some(21.0);
    let saved = export(&d).unwrap();
    let s = styles_of(&saved);
    assert!(def_named(&s, "MyCustom").is_some_and(|d| d.contains(r#"xlink:href="Pictures/only.png""#)), "{s}");
    let mut z = crate::Package::open(&saved, &crate::Limits::DEFAULT).unwrap();
    assert_eq!(z.read("Pictures/only.png").unwrap(), Some(png));
    let m = crate::text::tests::entry(&saved, "META-INF/manifest.xml");
    assert!(m.contains(r#"manifest:full-path="Pictures/only.png""#), "{m}");
}

/// A file defining what WordCraft doesn't model: a gradient, a paragraph style no text of its
/// own uses, a default style's writing mode, and a frame style naming them.
fn file_with_definitions() -> Vec<u8> {
    let ns = crate::text::tests::NS_T;
    let draw =
        r#"xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0""#;
    let styles = format!(
        r##"<office:document-styles {ns} {draw}><office:styles><draw:gradient draw:name="Sunset" draw:style="linear" draw:start-color="#ff8800" draw:end-color="#ffff00"/><style:default-style style:family="paragraph"><style:paragraph-properties style:writing-mode="page" style:line-break="strict"/><style:text-properties fo:language="de" fo:country="DE"/></style:default-style><style:style style:name="Standard" style:family="paragraph" style:class="text"/><style:style style:name="Text_20_body" style:display-name="Text body" style:family="paragraph" style:parent-style-name="Standard"><style:paragraph-properties fo:margin-bottom="7pt"/></style:style><style:style style:name="Eigen" style:display-name="Eigen" style:family="paragraph" style:parent-style-name="Standard"><style:text-properties fo:font-size="9pt"/></style:style><style:style style:name="Frame" style:family="graphic"><style:graphic-properties style:wrap="dynamic" style:vertical-pos="top"/></style:style></office:styles></office:document-styles>"##
    );
    let content = format!(
        r#"<office:document-content {ns} {draw}><office:automatic-styles><style:style style:name="fr1" style:family="graphic" style:parent-style-name="Frame"><style:graphic-properties draw:fill="gradient" draw:fill-gradient-name="Sunset"/></style:style></office:automatic-styles><office:body><office:text><text:p text:style-name="Text_20_body">Text<draw:frame draw:style-name="fr1" text:anchor-type="paragraph" svg:width="2in" svg:height="1in"><draw:text-box><text:p>In</text:p></draw:text-box></draw:frame></text:p><text:p text:style-name="Eigen">Own</text:p></office:text></office:body></office:document-content>"#
    );
    crate::text::tests::package(&content, Some(&styles))
}

#[test]
fn changing_one_style_leaves_the_files_other_definitions_alone() {
    use std::sync::Arc;
    let bytes = file_with_definitions();
    let as_read = |saved: &[u8]| {
        let s = crate::text::tests::entry(saved, "styles.xml");
        assert!(s.contains(r#"draw:gradient draw:name="Sunset""#), "its gradient: {s}");
        assert!(s.contains(r#"fo:margin-bottom="7pt""#), "a style WordCraft doesn't model: {s}");
        assert!(s.contains(r#"style:wrap="dynamic""#), "what its frames derive from: {s}");
        assert!(s.contains(r#"style:writing-mode="page""#) && s.contains(r#"fo:language="de""#), "its default style: {s}");
        s
    };
    // A style no text uses.
    let mut d = crate::text::import(&bytes).unwrap();
    Arc::make_mut(&mut d.styles).get_mut("Quote").unwrap().chr.size = Some(13.0);
    let saved = crate::text::export(&d).unwrap();
    let s = as_read(&saved);
    assert!(s.contains(r#"fo:font-size="13pt""#), "the change: {s}");
    assert_eq!(crate::text::import(&saved).unwrap().styles.get("Quote").unwrap().chr.size, Some(13.0));
    // A style the file defined: that one is WordCraft's now, the others still the file's.
    let mut d = crate::text::import(&bytes).unwrap();
    Arc::make_mut(&mut d.styles).get_mut("Eigen").unwrap().chr.size = Some(21.0);
    let saved = crate::text::export(&d).unwrap();
    let s = as_read(&saved);
    assert!(s.contains(r#"fo:font-size="21pt""#) && !s.contains(r#"fo:font-size="9pt""#), "{s}");
    // A new list (`para.numbering`): the styles as read, and the list there.
    let mut d = crate::text::import(&bytes).unwrap();
    Arc::make_mut(&mut d.numbering).add_list(craft_wordmodel::numbering::ListKind::Bullet);
    as_read(&crate::text::export(&d).unwrap());
    // Normal changed: the default style and Standard are WordCraft's.
    let mut d = crate::text::import(&bytes).unwrap();
    Arc::make_mut(&mut d.styles).default_chr.size = Some(15.0);
    let s = crate::text::tests::entry(&crate::text::export(&d).unwrap(), "styles.xml");
    assert!(s.contains(r#"fo:font-size="15pt""#), "{s}");
    assert!(s.contains(r#"draw:gradient draw:name="Sunset""#) && s.contains(r#"fo:margin-bottom="7pt""#), "the rest stays: {s}");
    // Nothing changed: the file's own, and none of WordCraft's over them.
    let d = crate::text::import(&bytes).unwrap();
    as_read(&crate::text::export(&d).unwrap());
}

/// A style deleted in WordCraft stays deleted: the file's own definition doesn't come back with
/// the definitions that stay as read.
#[test]
fn a_deleted_style_stays_deleted() {
    use std::sync::Arc;
    let mut d = crate::text::import(&file_with_definitions()).unwrap();
    assert!(d.styles.get("Eigen").is_some());
    Arc::make_mut(&mut d.styles).remove("Eigen");
    d.restyle("Eigen", None);
    let saved = crate::text::export(&d).unwrap();
    let s = crate::text::tests::entry(&saved, "styles.xml");
    assert!(!s.contains(r#"style:name="Eigen""#), "{s}");
    assert!(s.contains(r#"draw:gradient draw:name="Sunset""#) && s.contains(r#"fo:margin-bottom="7pt""#), "the rest stays: {s}");
    assert!(!crate::text::tests::entry(&saved, "content.xml").contains("Eigen"));
    assert!(crate::text::import(&saved).unwrap().styles.get("Eigen").is_none());
}

/// A file's style whose name has a space (`My_20_Custom`, WordCraft's `MyCustom`) is one style. An
/// edit reaches its definition under the file's name, a deletion removes it, and left alone it
/// stays as read with no second definition. A style nothing uses any more keeps its edit.
#[test]
fn a_style_named_with_a_space_is_the_files_style() {
    use std::sync::Arc;
    let s = r#"<style:style style:name="Standard" style:family="paragraph" style:class="text"/><style:style style:name="Base_20_Style" style:display-name="Base Style" style:family="paragraph" style:parent-style-name="Standard"/><style:style style:name="My_20_Custom" style:display-name="My Custom" style:family="paragraph" style:parent-style-name="Base_20_Style"><style:text-properties fo:font-size="9pt"/></style:style>"#;
    let styles = format!(r#"<office:document-styles {NS_T}><office:styles>{s}</office:styles></office:document-styles>"#);
    let content = format!(
        r#"<office:document-content {NS_T}><office:body><office:text><text:p text:style-name="My_20_Custom">One</text:p><text:p text:style-name="My_20_Custom">Two</text:p><text:p text:style-name="Base_20_Style">Three</text:p></office:text></office:body></office:document-content>"#
    );
    let bytes = package(&content, Some(&styles));
    let open = || import(&bytes).unwrap();
    let def = |xml: &str, name: &str| xml.split("<style:style ").find(|d| d.starts_with(&format!(r#"style:name="{name}""#))).map(String::from);
    let id = open().body[0].as_para().unwrap().props.style.clone().unwrap();

    let saved = styles_of(&export(&open()).unwrap());
    assert!(def(&saved, "My_20_Custom").is_some_and(|d| d.contains(r#"fo:font-size="9pt""#)), "as read: {saved}");
    assert!(def(&saved, "MyCustom").is_none(), "no second one: {saved}");

    // Edited: the paragraphs written as read name the file's definition, so that takes the edit.
    let mut d = open();
    Arc::make_mut(&mut d.styles).get_mut(&id).unwrap().chr.size = Some(21.0);
    let bytes_saved = export(&d).unwrap();
    let saved = styles_of(&bytes_saved);
    let mine = def(&saved, "My_20_Custom").unwrap_or_else(|| panic!("the file's name: {saved}"));
    assert!(mine.contains(r#"fo:font-size="21pt""#) && !saved.contains(r#"fo:font-size="9pt""#), "{saved}");
    assert!(mine.contains(r#"style:parent-style-name="Base_20_Style""#), "its parent, by the file's name: {mine}");
    assert!(content_of(&bytes_saved).contains(r#"text:style-name="My_20_Custom""#));
    let back = import(&bytes_saved).unwrap();
    assert_eq!(back.styles.resolve_char(back.body[0].as_para().unwrap().props.style.as_deref(), &Default::default()).size, 21.0);

    // Deleted: it stays deleted.
    let mut d = open();
    Arc::make_mut(&mut d.styles).remove(&id);
    d.restyle(&id, None);
    let bytes_saved = export(&d).unwrap();
    let saved = styles_of(&bytes_saved);
    assert!(def(&saved, "My_20_Custom").is_none() && def(&saved, "MyCustom").is_none(), "{saved}");
    assert!(def(&saved, "Base_20_Style").is_some(), "the others stay: {saved}");
    assert!(import(&bytes_saved).unwrap().styles.get(&id).is_none());

    // Nothing uses it any more, and it is edited: the edit is saved, not the style lost.
    let mut d = open();
    d.restyle(&id, None);
    Arc::make_mut(&mut d.styles).get_mut(&id).unwrap().chr.size = Some(21.0);
    let saved = styles_of(&export(&d).unwrap());
    assert!(def(&saved, "My_20_Custom").is_some_and(|m| m.contains(r#"fo:font-size="21pt""#)), "{saved}");
}

/// Two of the file's names WordCraft has as one style (`My_20_Custom` and `My_Custom` are both
/// `MyCustom`): edited, each is written under its own name, neither lost nor doubled.
#[test]
fn two_names_for_one_style_both_stay_when_it_is_edited() {
    use std::sync::Arc;
    let s = r#"<style:style style:name="Standard" style:family="paragraph" style:class="text"/><style:style style:name="My_20_Custom" style:family="paragraph" style:parent-style-name="Standard"><style:text-properties fo:font-size="9pt"/></style:style><style:style style:name="My_Custom" style:family="paragraph" style:parent-style-name="Standard"><style:text-properties fo:font-size="9pt"/></style:style>"#;
    let styles = format!(r#"<office:document-styles {NS_T}><office:styles>{s}</office:styles></office:document-styles>"#);
    let content = format!(
        r#"<office:document-content {NS_T}><office:body><office:text><text:p text:style-name="My_20_Custom">One</text:p><text:p text:style-name="My_Custom">Two</text:p></office:text></office:body></office:document-content>"#
    );
    let mut d = import(&package(&content, Some(&styles))).unwrap();
    let id = d.body[0].as_para().unwrap().props.style.clone().unwrap();
    Arc::make_mut(&mut d.styles).get_mut(&id).unwrap().chr.size = Some(21.0);
    let saved = styles_of(&export(&d).unwrap());
    for name in ["My_20_Custom", "My_Custom"] {
        assert_eq!(saved.matches(&format!(r#"style:name="{name}""#)).count(), 1, "{name}: {saved}");
        let def = saved.split(&format!(r#"style:name="{name}""#)).nth(1).and_then(|d| d.split("</style:style>").next()).unwrap();
        assert!(def.contains(r#"fo:font-size="21pt""#), "{name}: {def}");
    }
}

/// A parent's name goes by its family: a text style's other name isn't a paragraph style's.
#[test]
fn a_rewritten_styles_parent_keeps_its_own_familys_name() {
    use std::sync::Arc;
    let s = r#"<style:style style:name="Standard" style:family="paragraph" style:class="text"/><style:style style:name="BaseStyle" style:family="paragraph" style:parent-style-name="Standard"><style:text-properties fo:font-size="30pt"/></style:style><style:style style:name="Base_20_Style" style:family="text"><style:text-properties fo:font-weight="bold"/></style:style><style:style style:name="My_20_Custom" style:family="paragraph" style:parent-style-name="BaseStyle"><style:text-properties fo:font-weight="bold"/></style:style>"#;
    let styles = format!(r#"<office:document-styles {NS_T}><office:styles>{s}</office:styles></office:document-styles>"#);
    let content = format!(
        r#"<office:document-content {NS_T}><office:body><office:text><text:p text:style-name="My_20_Custom">One</text:p><text:p>Two <text:span text:style-name="Base_20_Style">three</text:span></text:p></office:text></office:body></office:document-content>"#
    );
    let mut d = import(&package(&content, Some(&styles))).unwrap();
    let id = d.body[0].as_para().unwrap().props.style.clone().unwrap();
    Arc::make_mut(&mut d.styles).get_mut(&id).unwrap().chr.italic = Some(true);
    let bytes = export(&d).unwrap();
    let saved = styles_of(&bytes);
    let mine = saved.split("<style:style ").find(|t| t.starts_with(r#"style:name="My_20_Custom""#)).and_then(|t| t.split('>').next()).unwrap();
    assert!(mine.contains(r#"style:parent-style-name="BaseStyle""#), "{mine}");
    let back = import(&bytes).unwrap();
    let p = back.body[0].as_para().unwrap();
    assert_eq!(back.styles.resolve_char(p.props.style.as_deref(), &Default::default()).size, 30.0, "inherited");
}

/// Whether `xml` reads as XML: tags close in order and every attribute value unescapes.
fn reads_as_xml(xml: &str) -> bool {
    // craft-xml's strict read refuses unbalanced tags, malformed attributes and unknown
    // references, but takes a bare `&` as itself: each `&` must start a reference too.
    let refs = xml.split('&').skip(1).all(|r| {
        let name = r.split(';').next().unwrap_or("");
        r.contains(';') && !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'#')
    });
    let mut events = craft_xml::Options::default().strict().events(xml.as_bytes());
    loop {
        match events.next() {
            Ok(Some(_)) => {}
            Ok(None) => return refs,
            Err(_) => return false,
        }
    }
}

/// A file's paragraph style `styles` (markup inside `office:styles`) and paragraphs naming
/// `used`, opened.
fn opened(styles: &str, used: &[&str]) -> craft_wordmodel::Document {
    let styles = format!(r#"<office:document-styles {NS_T}><office:styles>{styles}</office:styles></office:document-styles>"#);
    let body: String = used.iter().map(|n| format!(r#"<text:p text:style-name="{n}">Text</text:p>"#)).collect();
    let content =
        format!(r#"<office:document-content {NS_T}><office:body><office:text>{body}</office:text></office:body></office:document-content>"#);
    import(&package(&content, Some(&styles))).unwrap()
}

/// The definition of the style named `name` (as written, escaped) in `xml`, up to its end.
fn def_named(xml: &str, name: &str) -> Option<String> {
    xml.split("<style:style ")
        .find(|d| d.starts_with(&format!(r#"style:name="{name}""#)))
        .and_then(|d| d.split("</style:style>").next())
        .map(String::from)
}

/// A file's style whose name has `&`, `"` or `<` in it, edited, is written under its name escaped.
#[test]
fn an_edited_styles_name_is_written_escaped() {
    use std::sync::Arc;
    for name in ["R&amp;D", "Say&quot;Hi", "A&lt;B"] {
        let s = format!(
            r#"<style:style style:name="Standard" style:family="paragraph" style:class="text"/><style:style style:name="{name}" style:family="paragraph" style:parent-style-name="Standard"><style:text-properties fo:font-size="9pt"/></style:style>"#
        );
        let mut d = opened(&s, &[name]);
        let id = d.body[0].as_para().unwrap().props.style.clone().unwrap();
        Arc::make_mut(&mut d.styles).get_mut(&id).unwrap().chr.size = Some(21.0);
        let bytes = export(&d).unwrap();
        let saved = styles_of(&bytes);
        assert!(reads_as_xml(&saved), "{name}: {saved}");
        assert!(def_named(&saved, name).is_some_and(|d| d.contains(r#"fo:font-size="21pt""#)), "{name}: {saved}");
        let back = import(&bytes).unwrap();
        assert_eq!(back.styles.resolve_char(back.body[0].as_para().unwrap().props.style.as_deref(), &Default::default()).size, 21.0, "{name}");
    }
}

/// An edited style's parent is the file's style the import read it from, not another of the
/// file's styles WordCraft would have under the same id (`BaseStyle`, `Base_20_Style`).
#[test]
fn a_rewritten_styles_parent_is_the_one_it_was_read_from() {
    use std::sync::Arc;
    let s = r#"<style:style style:name="Standard" style:family="paragraph" style:class="text"/><style:style style:name="BaseStyle" style:family="paragraph" style:parent-style-name="Standard"><style:text-properties fo:font-size="30pt"/></style:style><style:style style:name="Base_20_Style" style:family="paragraph" style:parent-style-name="Standard"><style:text-properties fo:font-size="9pt"/></style:style><style:style style:name="My_20_Custom" style:family="paragraph" style:parent-style-name="BaseStyle"/>"#;
    let mut d = opened(s, &["My_20_Custom"]);
    let id = d.body[0].as_para().unwrap().props.style.clone().unwrap();
    Arc::make_mut(&mut d.styles).get_mut(&id).unwrap().chr.italic = Some(true);
    let bytes = export(&d).unwrap();
    let saved = styles_of(&bytes);
    let mine = def_named(&saved, "My_20_Custom").unwrap_or_else(|| panic!("{saved}"));
    assert!(mine.contains(r#"style:parent-style-name="BaseStyle""#), "{mine}");
    assert!(def_named(&saved, "Base_20_Style").is_some_and(|d| d.contains(r#"fo:font-size="9pt""#)), "as read: {saved}");
    let back = import(&bytes).unwrap();
    assert_eq!(back.styles.resolve_char(back.body[0].as_para().unwrap().props.style.as_deref(), &Default::default()).size, 30.0, "inherited");
}

/// A style nothing in the file uses keeps its own definition when a used one WordCraft has
/// under the same id (`My_20_Custom`, `My_Custom`) is edited.
#[test]
fn an_unused_style_sharing_an_edited_ones_id_keeps_its_definition() {
    use std::sync::Arc;
    let s = r##"<style:style style:name="Standard" style:family="paragraph" style:class="text"/><style:style style:name="My_20_Custom" style:family="paragraph" style:parent-style-name="Standard"><style:text-properties fo:font-size="9pt"/></style:style><style:style style:name="My_Custom" style:family="paragraph" style:parent-style-name="Standard"><style:text-properties fo:font-size="30pt" fo:color="#ff0000"/></style:style>"##;
    let mut d = opened(s, &["My_20_Custom"]);
    let id = d.body[0].as_para().unwrap().props.style.clone().unwrap();
    Arc::make_mut(&mut d.styles).get_mut(&id).unwrap().chr.size = Some(21.0);
    let saved = styles_of(&export(&d).unwrap());
    assert!(def_named(&saved, "My_20_Custom").is_some_and(|d| d.contains(r#"fo:font-size="21pt""#)), "{saved}");
    let theirs = def_named(&saved, "My_Custom").unwrap_or_else(|| panic!("{saved}"));
    assert!(theirs.contains(r#"fo:font-size="30pt""#) && theirs.contains("#ff0000"), "as read: {theirs}");
}

/// A style nothing in the file uses, named as WordCraft names a used one (`MyCustom` beside
/// `My_20_Custom`, which WordCraft has as `MyCustom`), keeps its definition when the used one is
/// edited or deleted.
#[test]
fn an_unused_style_named_as_wordcraft_names_an_edited_one_keeps_its_definition() {
    use std::sync::Arc;
    let s = r##"<style:style style:name="Standard" style:family="paragraph" style:class="text"/><style:style style:name="My_20_Custom" style:family="paragraph" style:parent-style-name="Standard"><style:text-properties fo:font-size="9pt"/></style:style><style:style style:name="MyCustom" style:family="paragraph" style:parent-style-name="Standard"><style:text-properties fo:font-size="30pt" fo:color="#ff0000"/></style:style>"##;
    let as_read = |saved: &str| {
        let theirs = def_named(saved, "MyCustom").unwrap_or_else(|| panic!("{saved}"));
        assert!(theirs.contains(r#"fo:font-size="30pt""#) && theirs.contains("#ff0000"), "as read: {theirs}");
    };

    let mut d = opened(s, &["My_20_Custom"]);
    let id = d.body[0].as_para().unwrap().props.style.clone().unwrap();
    Arc::make_mut(&mut d.styles).get_mut(&id).unwrap().chr.size = Some(21.0);
    let saved = styles_of(&export(&d).unwrap());
    assert!(def_named(&saved, "My_20_Custom").is_some_and(|d| d.contains(r#"fo:font-size="21pt""#)), "{saved}");
    as_read(&saved);

    let mut d = opened(s, &["My_20_Custom"]);
    Arc::make_mut(&mut d.styles).remove(&id);
    d.restyle(&id, None);
    let saved = styles_of(&export(&d).unwrap());
    assert!(def_named(&saved, "My_20_Custom").is_none(), "{saved}");
    as_read(&saved);
}

/// A paragraph written from the model names the file's style it was read with (`My_20_Custom`),
/// not a style of the file's that WordCraft's name for it would land on (`MyCustom`, unused).
#[test]
fn an_edited_paragraph_keeps_its_styles_file_name_beside_an_unused_namesake() {
    use std::sync::Arc;
    let s = r##"<style:style style:name="Standard" style:family="paragraph" style:class="text"/><style:style style:name="My_20_Custom" style:family="paragraph" style:parent-style-name="Standard"><style:text-properties fo:font-size="9pt"/></style:style><style:style style:name="MyCustom" style:family="paragraph" style:parent-style-name="Standard"><style:text-properties fo:font-size="30pt" fo:color="#ff0000"/></style:style>"##;
    for (edit_style, size) in [(false, 9.0), (true, 21.0)] {
        let mut d = opened(s, &["My_20_Custom"]);
        let id = d.body[0].as_para().unwrap().props.style.clone().unwrap();
        if edit_style {
            Arc::make_mut(&mut d.styles).get_mut(&id).unwrap().chr.size = Some(21.0);
        }
        d.insert_text(&Pos::body(0, 0), "New ", &Default::default()).unwrap();
        let bytes = export(&d).unwrap();
        let c = content_of(&bytes);
        assert!(c.contains("New Text"), "{c}");
        assert!(!c.contains(r#"style-name="MyCustom""#), "{c}");
        let back = import(&bytes).unwrap();
        let got = back.styles.resolve_char(back.body[0].as_para().unwrap().props.style.as_deref(), &Default::default()).size;
        assert_eq!(got, size, "edited style {edit_style}: {c}\n{}", styles_of(&bytes));
    }
}

/// A style made after reading (`NewChild`), based on a file's style beside an unused namesake
/// (`My_20_Custom`, WordCraft's `MyCustom`, beside `MyCustom`), derives from the file's style it
/// was based on.
#[test]
fn a_new_style_based_on_one_beside_an_unused_namesake_derives_from_it() {
    use std::sync::Arc;
    let s = r##"<style:style style:name="Standard" style:family="paragraph" style:class="text"/><style:style style:name="My_20_Custom" style:family="paragraph" style:parent-style-name="Standard"><style:text-properties fo:font-size="9pt"/></style:style><style:style style:name="MyCustom" style:family="paragraph" style:parent-style-name="Standard"><style:text-properties fo:font-size="30pt" fo:color="#ff0000"/></style:style>"##;
    let mut d = opened(s, &["My_20_Custom"]);
    let id = d.body[0].as_para().unwrap().props.style.clone().unwrap();
    let mut child = d.styles.get(&id).unwrap().clone();
    (child.id, child.name, child.based_on, child.next) = ("NewChild".into(), "NewChild".into(), Some(id.clone()), None);
    (child.para, child.chr) = Default::default();
    Arc::make_mut(&mut d.styles).upsert(child);
    d.restyle(&id, Some("NewChild".to_string()));
    let bytes = export(&d).unwrap();
    let saved = styles_of(&bytes);
    let new = saved.split("<style:style ").find(|t| t.starts_with(r#"style:name="NewChild""#)).and_then(|t| t.split('>').next());
    assert!(new.is_some_and(|n| n.contains(r#"style:parent-style-name="My_20_Custom""#)), "{saved}");
    let theirs = def_named(&saved, "MyCustom").unwrap_or_else(|| panic!("{saved}"));
    assert!(theirs.contains(r#"fo:font-size="30pt""#), "as read: {theirs}");
    let back = import(&bytes).unwrap();
    let got = back.styles.resolve_char(back.body[0].as_para().unwrap().props.style.as_deref(), &Default::default()).size;
    assert_eq!(got, 9.0, "{saved}\n{}", content_of(&bytes));
}

/// A style made after reading (`NewChild`), based on a file's style WordCraft names otherwise
/// (`Base_20_Style`, WordCraft's `BaseStyle`), derives from the file's style, with what WordCraft
/// doesn't model of it.
#[test]
fn a_new_style_based_on_a_renamed_one_derives_from_the_files() {
    use std::sync::Arc;
    let s = r#"<style:style style:name="Standard" style:family="paragraph" style:class="text"/><style:style style:name="Base_20_Style" style:display-name="Base Style" style:family="paragraph" style:parent-style-name="Standard"><style:paragraph-properties style:writing-mode="tb-rl"/><style:text-properties fo:font-size="30pt"/></style:style>"#;
    let mut d = opened(s, &["Base_20_Style"]);
    let id = d.body[0].as_para().unwrap().props.style.clone().unwrap();
    let mut child = d.styles.get(&id).unwrap().clone();
    (child.id, child.name, child.based_on, child.next) = ("NewChild".into(), "NewChild".into(), Some(id.clone()), None);
    (child.para, child.chr) = Default::default();
    Arc::make_mut(&mut d.styles).upsert(child);
    d.restyle(&id, Some("NewChild".to_string()));
    let bytes = export(&d).unwrap();
    let saved = styles_of(&bytes);
    let new = saved.split("<style:style ").find(|t| t.starts_with(r#"style:name="NewChild""#)).and_then(|t| t.split('>').next());
    assert!(new.is_some_and(|n| n.contains(r#"style:parent-style-name="Base_20_Style""#)), "{saved}");
    assert!(def_named(&saved, "Base_20_Style").is_some_and(|d| d.contains(r#"style:writing-mode="tb-rl""#)), "as read: {saved}");
    let back = import(&bytes).unwrap();
    let got = back.styles.resolve_char(back.body[0].as_para().unwrap().props.style.as_deref(), &Default::default()).size;
    assert_eq!(got, 30.0, "{saved}");
}

/// The same for a character style: edited text's automatic style derives from the file's
/// character style it was read with (`My_20_Accent`), not an unused namesake (`MyAccent`).
#[test]
fn edited_text_keeps_its_character_styles_file_name_beside_an_unused_namesake() {
    let s = r##"<style:style style:name="Standard" style:family="paragraph" style:class="text"/><style:style style:name="My_20_Accent" style:family="text"><style:text-properties fo:font-size="9pt"/></style:style><style:style style:name="MyAccent" style:family="text"><style:text-properties fo:font-size="30pt" fo:color="#ff0000"/></style:style>"##;
    let styles = format!(r#"<office:document-styles {NS_T}><office:styles>{s}</office:styles></office:document-styles>"#);
    let content = format!(
        r#"<office:document-content {NS_T}><office:body><office:text><text:p>One <text:span text:style-name="My_20_Accent">two</text:span></text:p></office:text></office:body></office:document-content>"#
    );
    let mut d = import(&package(&content, Some(&styles))).unwrap();
    d.insert_text(&Pos::body(0, 0), "New ", &Default::default()).unwrap();
    let c = content_of(&export(&d).unwrap());
    assert!(c.contains("New One"), "{c}");
    assert!(c.contains(r#"style:parent-style-name="My_20_Accent""#) && !c.contains(r#"="MyAccent""#), "{c}");
}

/// An edited style the file names as WordCraft does (`Eigen`), whose parent WordCraft names
/// otherwise (`Base_20_Style`, WordCraft's `BaseStyle`), still derives from the file's parent.
#[test]
fn an_edited_style_under_its_own_name_keeps_its_parents_file_name() {
    use std::sync::Arc;
    let s = r#"<style:style style:name="Standard" style:family="paragraph" style:class="text"/><style:style style:name="Base_20_Style" style:display-name="Base Style" style:family="paragraph" style:parent-style-name="Standard" style:class="text"><style:paragraph-properties style:writing-mode="tb-rl"/><style:text-properties fo:font-size="30pt"/></style:style><style:style style:name="Eigen" style:family="paragraph" style:parent-style-name="Base_20_Style"/>"#;
    let mut d = opened(s, &["Eigen"]);
    let id = d.body[0].as_para().unwrap().props.style.clone().unwrap();
    Arc::make_mut(&mut d.styles).get_mut(&id).unwrap().chr.italic = Some(true);
    let bytes = export(&d).unwrap();
    let saved = styles_of(&bytes);
    let mine = def_named(&saved, "Eigen").unwrap_or_else(|| panic!("{saved}"));
    assert!(mine.contains(r#"style:parent-style-name="Base_20_Style""#), "{mine}");
    assert!(def_named(&saved, "Base_20_Style").is_some_and(|d| d.contains(r#"style:writing-mode="tb-rl""#)), "as read: {saved}");
    let back = import(&bytes).unwrap();
    assert_eq!(back.styles.resolve_char(back.body[0].as_para().unwrap().props.style.as_deref(), &Default::default()).size, 30.0, "inherited");
}

/// A character style WordCraft read under another id (`AccentChar`, as a paragraph style has
/// `Accent`), edited, is written under the file's name: the text written as read names that.
#[test]
fn an_edited_character_style_read_under_another_id_keeps_the_files_name() {
    use std::sync::Arc;
    let s = r#"<style:style style:name="Standard" style:family="paragraph" style:class="text"/><style:style style:name="Accent" style:family="paragraph" style:parent-style-name="Standard"/><style:style style:name="Accent" style:family="text"><style:text-properties fo:font-size="9pt"/></style:style>"#;
    let styles = format!(r#"<office:document-styles {NS_T}><office:styles>{s}</office:styles></office:document-styles>"#);
    let content = format!(
        r#"<office:document-content {NS_T}><office:body><office:text><text:p text:style-name="Accent">One <text:span text:style-name="Accent">two</text:span></text:p></office:text></office:body></office:document-content>"#
    );
    let mut d = import(&package(&content, Some(&styles))).unwrap();
    let id =
        d.styles.styles.iter().find(|s| s.kind == craft_wordmodel::styles::StyleKind::Character && s.name == "Accent").map(|s| s.id.clone()).unwrap();
    Arc::make_mut(&mut d.styles).get_mut(&id).unwrap().chr.size = Some(21.0);
    let saved = styles_of(&export(&d).unwrap());
    let text = saved
        .split("<style:style ")
        .find(|d| d.starts_with(r#"style:name="Accent""#) && d.split('>').next().is_some_and(|h| h.contains(r#"style:family="text""#)))
        .and_then(|d| d.split("</style:style>").next())
        .unwrap_or_else(|| panic!("{saved}"));
    assert!(text.contains(r#"fo:font-size="21pt""#), "{text}");
}

/// Many of the file's styles WordCraft has under one id (`My-Custom`, `My.Custom`…), one edited:
/// editing the shared id rewrites only the definitions that were read into it.
#[test]
fn many_styles_sharing_an_id_keep_unedited_definitions() {
    use std::sync::Arc;
    let names: Vec<String> =
        (0..6_000u32).map(|i| format!("My{}Custom", (0..13).map(|b| if i >> b & 1 == 1 { '-' } else { '.' }).collect::<String>())).collect();
    let mut s = String::from(r#"<style:style style:name="Standard" style:family="paragraph" style:class="text"/>"#);
    for n in &names {
        s.push_str(&format!(r#"<style:style style:name="{n}" style:family="paragraph" style:parent-style-name="Standard" style:next-style-name="{n}"><style:text-properties fo:font-size="9pt"/></style:style>"#));
    }
    let used: Vec<&str> = names.iter().take(500).map(String::as_str).collect();
    let mut d = opened(&s, &used);
    let id = d.body[0].as_para().unwrap().props.style.clone().unwrap();
    Arc::make_mut(&mut d.styles).get_mut(&id).unwrap().chr.size = Some(21.0);
    let saved = styles_of(&export(&d).unwrap());
    assert_eq!(saved.matches(r#"fo:font-size="21pt""#).count(), 500, "the ones read into it");
    assert_eq!(saved.matches(r#"fo:font-size="9pt""#).count(), 5_500, "the others as read");
}

#[test]
fn the_files_font_declarations_are_kept() {
    // A reader without a font picks its substitute by how the file declares it. WordCraft's own
    // guess at a font's kind replaced the file's, and a document came back from a save with no
    // edits 13 pages shorter in LibreOffice on a computer without its font.
    let ns = NS_T;
    let face = r#"<style:font-face style:name="Odd Roman" svg:font-family="&apos;Odd Roman&apos;" style:font-family-generic="roman"/>"#;
    let styles = format!(
        r#"<office:document-styles {ns}><office:font-face-decls>{face}</office:font-face-decls><office:styles><style:default-style style:family="paragraph"><style:text-properties style:font-name="Odd Roman" fo:font-size="12pt"/></style:default-style></office:styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {ns}><office:font-face-decls>{face}</office:font-face-decls><office:automatic-styles><style:style style:name="P1" style:family="paragraph"><style:text-properties style:font-name="Odd Roman"/></style:style></office:automatic-styles><office:body><office:text><text:p text:style-name="P1">Text</text:p></office:text></office:body></office:document-content>"#
    );
    let check = |saved: &[u8], what: &str| {
        for part in [styles_of(saved), content_of(saved)] {
            assert!(part.contains(face), "{what}: {part}");
            assert_eq!(part.matches(r#"<style:font-face style:name="Odd Roman""#).count(), 1, "{what}: declared once: {part}");
        }
    };
    let mut d = import(&package(&content, Some(&styles))).unwrap();
    check(&export(&d).unwrap(), "no edits");
    // Text edited, written from the model: still the file's declaration.
    if let Some(craft_wordmodel::Block::Para(p)) = d.body.get_mut(0).map(std::sync::Arc::make_mut) {
        p.text.push('!');
        if let Some(r) = p.runs.last_mut() {
            r.len += 1;
        }
    }
    check(&export(&d).unwrap(), "an edit");
}

/// A font's name spelled another way in XML (`O'Brien`, `O&#39;Brien`) is the same font: its
/// declaration is the file's, once.
#[test]
fn a_fonts_name_spelled_another_way_is_declared_once_as_the_file_has_it() {
    for spelled in ["O'Brien", "O&#39;Brien"] {
        let face = format!(r#"<style:font-face style:name="{spelled}" svg:font-family="O&apos;Brien" style:font-family-generic="roman"/>"#);
        let styles = format!(
            r#"<office:document-styles {NS_T}><office:font-face-decls>{face}</office:font-face-decls><office:styles><style:default-style style:family="paragraph"><style:text-properties style:font-name="{spelled}" fo:font-size="12pt"/></style:default-style></office:styles></office:document-styles>"#
        );
        let content = format!(
            r#"<office:document-content {NS_T}><office:font-face-decls>{face}</office:font-face-decls><office:automatic-styles><style:style style:name="T1" style:family="text"><style:text-properties style:font-name="{spelled}"/></style:style></office:automatic-styles><office:body><office:text><text:p><text:span text:style-name="T1">One</text:span></text:p></office:text></office:body></office:document-content>"#
        );
        let mut d = import(&package(&content, Some(&styles))).unwrap();
        // Text edited, written from the model.
        if let Some(craft_wordmodel::Block::Para(p)) = d.body.get_mut(0).map(std::sync::Arc::make_mut) {
            p.text.push('!');
            if let Some(r) = p.runs.last_mut() {
                r.len += 1;
            }
        }
        let saved = export(&d).unwrap();
        for part in [styles_of(&saved), content_of(&saved)] {
            assert!(part.contains(&face), "{spelled}: the file's: {part}");
            let is_it = |t: &&str| {
                let name = t.split('>').next().and_then(|tag| tag.split("style:name=\"").nth(1)).and_then(|v| v.split('"').next());
                name.is_some_and(|n| n == "O'Brien" || n == "O&#39;Brien" || n == "O&apos;Brien")
            };
            assert_eq!(part.split("<style:font-face ").skip(1).filter(is_it).count(), 1, "{spelled}: declared once: {part}");
        }
    }
}

/// A face named other than its family (`F1`, `Mystery`): text written from the model names the
/// family, and that declaration says what the file's does, not WordCraft's guess at the font's kind.
#[test]
fn a_face_named_other_than_its_family_gives_the_family_the_files_declaration() {
    let face =
        r#"<style:font-face style:name="F1" svg:font-family="&apos;Mystery&apos;" style:font-family-generic="roman" style:font-pitch="variable"/>"#;
    let styles = format!(
        r#"<office:document-styles {NS_T}><office:font-face-decls>{face}</office:font-face-decls><office:styles><style:default-style style:family="paragraph"><style:text-properties fo:font-size="12pt"/></style:default-style></office:styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {NS_T}><office:font-face-decls>{face}</office:font-face-decls><office:automatic-styles><style:style style:name="T1" style:family="text"><style:text-properties style:font-name="F1"/></style:style></office:automatic-styles><office:body><office:text><text:p><text:span text:style-name="T1">One</text:span></text:p></office:text></office:body></office:document-content>"#
    );
    let mut d = import(&package(&content, Some(&styles))).unwrap();
    if let Some(craft_wordmodel::Block::Para(p)) = d.body.get_mut(0).map(std::sync::Arc::make_mut) {
        p.text.push('!');
        if let Some(r) = p.runs.last_mut() {
            r.len += 1;
        }
    }
    let saved = export(&d).unwrap();
    let part = content_of(&saved);
    assert!(part.contains(r#"style:font-name="Mystery""#), "{part}");
    let mystery = part.split("<style:font-face ").find(|t| t.starts_with(r#"style:name="Mystery""#)).and_then(|t| t.split("/>").next());
    let mystery = mystery.unwrap_or_else(|| panic!("declared: {part}"));
    assert!(mystery.contains(r#"style:font-family-generic="roman""#), "{mystery}");
}

/// Only declarations are declarations: not a script's text that reads like one. And a face's name
/// is its own attribute, not another with `style:name` in its name.
#[test]
fn a_face_is_found_and_named_by_its_own_markup() {
    let script =
        r#"<office:scripts><office:script script:language="x"><![CDATA[<style:font-face style:name="Mystery" />]]></office:script></office:scripts>"#;
    let face = r#"<style:font-face xstyle:name="F1" style:name="F1" svg:font-family="&apos;Mystery&apos;" style:font-family-generic="roman"/>"#;
    let styles = format!(
        r#"<office:document-styles {NS_T}><office:font-face-decls>{face}</office:font-face-decls><office:styles><style:default-style style:family="paragraph"><style:text-properties fo:font-size="12pt"/></style:default-style></office:styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {NS_T}>{script}<office:font-face-decls>{face}</office:font-face-decls><office:automatic-styles><style:style style:name="T1" style:family="text"><style:text-properties style:font-name="F1"/></style:style></office:automatic-styles><office:body><office:text><text:p><text:span text:style-name="T1">One</text:span></text:p></office:text></office:body></office:document-content>"#
    );
    let mut d = import(&package(&content, Some(&styles))).unwrap();
    if let Some(craft_wordmodel::Block::Para(p)) = d.body.get_mut(0).map(std::sync::Arc::make_mut) {
        p.text.push('!');
        if let Some(r) = p.runs.last_mut() {
            r.len += 1;
        }
    }
    let saved = export(&d).unwrap();
    let part = content_of(&saved);
    assert!(part.contains(script), "the script as it was: {part}");
    let decls = part.split("<office:font-face-decls>").last().and_then(|t| t.split("</office:font-face-decls>").next()).unwrap();
    assert!(decls.contains(r#"xstyle:name="F1" style:name="Mystery""#), "{decls}");
    assert!(import(&saved).is_ok());
}

/// A font declaration in single quotes is the file's too: it replaces WordCraft's, and isn't
/// declared a second time.
#[test]
fn a_font_declaration_in_single_quotes_is_kept() {
    let ns = format!(r#"{NS_T} xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0""#);
    let face = r#"<style:font-face style:name='Odd Roman' svg:font-family='Odd Roman' style:font-family-generic='roman'/>"#;
    let styles = format!(
        r#"<office:document-styles {ns}><office:font-face-decls>{face}</office:font-face-decls><office:styles><style:default-style style:family="paragraph"><style:text-properties style:font-name="Odd Roman" fo:font-size="12pt"/></style:default-style></office:styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {ns}><office:font-face-decls>{face}</office:font-face-decls><office:automatic-styles><style:style style:name="P1" style:family="paragraph"><style:text-properties style:font-name="Odd Roman"/></style:style></office:automatic-styles><office:body><office:text><text:p text:style-name="P1">x</text:p></office:text></office:body></office:document-content>"#
    );
    let saved = export(&import(&package(&content, Some(&styles))).unwrap()).unwrap();
    for part in [styles_of(&saved), content_of(&saved)] {
        assert!(part.contains(face), "{part}");
        assert_eq!(part.matches("<style:font-face").count(), part.matches("<style:font-face style:name=").count());
        assert_eq!(part.matches(r#"style:name="Odd Roman""#).count(), 0, "declared once: {part}");
        assert_eq!(part.matches("style:name='Odd Roman'").count(), 1, "declared once: {part}");
    }
}

/// A name space only the declarations' container states goes with the declaration: without it the
/// saved part isn't well-formed in its name spaces.
#[test]
fn a_name_space_on_the_font_declarations_goes_with_them() {
    let ns = format!(r#"{NS_T} xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0""#);
    let decls = r#"<office:font-face-decls xmlns:x="urn:example:x"><style:font-face style:name="Odd Roman" svg:font-family="Odd Roman" x:odd="1"/></office:font-face-decls>"#;
    let styles = format!(
        r#"<office:document-styles {ns}>{decls}<office:styles><style:default-style style:family="paragraph"><style:text-properties style:font-name="Odd Roman" fo:font-size="12pt"/></style:default-style></office:styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {ns}>{decls}<office:automatic-styles><style:style style:name="P1" style:family="paragraph"><style:text-properties style:font-name="Odd Roman"/></style:style></office:automatic-styles><office:body><office:text><text:p text:style-name="P1">x</text:p></office:text></office:body></office:document-content>"#
    );
    let saved = export(&import(&package(&content, Some(&styles))).unwrap()).unwrap();
    for part in [styles_of(&saved), content_of(&saved)] {
        assert!(part.contains(r#"x:odd="1""#), "{part}");
        assert!(part.contains(r#"xmlns:x="urn:example:x""#), "{part}");
    }
}

/// Line numbering changed in the first of two sections is saved: the file's own definition stays
/// only while the section the save writes it from is as read (#5702 review).
#[test]
fn line_numbering_changed_in_the_first_section_is_saved() {
    let styles = format!(
        r#"<office:document-styles {NS_T}><office:styles><text:linenumbering-configuration text:number-lines="true" text:offset="0.25in" style:num-format="1" text:number-position="left" text:increment="5" text:restart-on-page="true"/><style:style style:name="Opening" style:family="paragraph" style:master-page-name="Other"/></office:styles><office:automatic-styles><style:page-layout style:name="pm1"><style:page-layout-properties fo:page-width="8.5in" fo:page-height="11in"/></style:page-layout><style:page-layout style:name="pm2"><style:page-layout-properties fo:page-width="11in" fo:page-height="8.5in"/></style:page-layout></office:automatic-styles><office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm1"/><style:master-page style:name="Other" style:page-layout-name="pm2"/></office:master-styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {NS_T}><office:body><office:text><text:p>One.</text:p><text:p text:style-name="Opening">Two.</text:p></office:text></office:body></office:document-content>"#
    );
    let bytes = package(&content, Some(&styles));
    let mut d = import(&bytes).unwrap();
    assert_eq!(d.sections().len(), 2, "two sections");
    let first = d.body.iter_mut().find_map(|b| match std::sync::Arc::make_mut(b) {
        craft_wordmodel::Block::Para(p) => p.section.as_deref_mut(),
        _ => None,
    });
    let first = first.expect("the first section");
    first.line_numbers.as_mut().expect("line numbering as read").count_by = 3;
    let s = styles_of(&export(&d).unwrap());
    assert!(s.contains(r#"text:increment="3""#) && !s.contains(r#"text:increment="5""#), "the change: {s}");
}

#[test]
fn rewriting_shared_style_names_scales_linearly_in_name_lookups() {
    use std::{
        cell::Cell,
        collections::{BTreeMap, BTreeSet},
        sync::Arc,
    };
    let lookups = |n: usize| {
        let defs: BTreeMap<String, Arc<str>> = (0..n).map(|i| (format!("common:Name{i}"), Arc::from("<style:style/>"))).collect();
        let ids = defs.keys().map(|key| (key.clone(), "Shared".to_string())).collect();
        let changed = BTreeSet::from(["common:Shared".to_string()]);
        let calls = Cell::new(0usize);
        let ours = |id: &str| {
            calls.set(calls.get() + 1);
            id.to_string()
        };
        let rewritten = super::super::kept::rewritten_defs(
            r#"<office:styles><style:style style:name="Shared" style:family="paragraph"/></office:styles>"#,
            &defs,
            &changed,
            &ids,
            &ours,
        );
        assert_eq!(rewritten.len(), n);
        calls.get()
    };
    let small = lookups(16);
    let large = lookups(128);
    assert!(small > 0);
    assert!(large <= 8 * small, "name lookups grew faster than the input: {small} -> {large}");
}
