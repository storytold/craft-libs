//! Page-level settings (ODF 1.3 §17.2 `style:page-layout-properties`, §16.29
//! `text:notes-configuration`, §16.30 `text:linenumbering-configuration`): page number format,
//! page colour and borders, footnote and endnote numbering, and line numbering, read into the
//! document's and its sections' settings and written back.

use crate::text::tests::{NS_T, package};
use crate::text::{export, import};
use craft_wordmodel::Document;
use craft_wordmodel::props::{BorderStyle, Rgb};
use craft_wordmodel::section::{LineNumberRestart, NoteRestart, NumFormat};

fn open() -> Document {
    let styles = format!(
        r##"<office:document-styles {NS_T}><office:styles>
<text:notes-configuration text:note-class="footnote" style:num-format="a" text:start-value="4" text:footnotes-position="page" text:start-numbering-at="page"/>
<text:notes-configuration text:note-class="endnote" style:num-format="I" text:start-value="0"/>
<text:linenumbering-configuration text:number-lines="true" text:offset="0.25in" style:num-format="1" text:number-position="left" text:increment="5" text:restart-on-page="true"/>
</office:styles><office:automatic-styles><style:page-layout style:name="pm1"><style:page-layout-properties fo:page-width="8.5in" fo:page-height="11in" fo:margin-top="1in" fo:margin-bottom="1in" fo:margin-left="1in" fo:margin-right="1in" style:num-format="i" fo:background-color="#ffeecc" fo:border="0.5pt solid #ff0000" fo:padding="10pt"/></style:page-layout></office:automatic-styles><office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm1"/></office:master-styles></office:document-styles>"##
    );
    let content = format!(
        r#"<office:document-content {NS_T}><office:body><office:text><text:p>Body.</text:p></office:text></office:body></office:document-content>"#
    );
    import(&package(&content, Some(&styles))).unwrap()
}

/// What these settings come to: (page number format, page colour, a page border's style, width,
/// colour and space, footnote format, start and restart, endnote format, line numbering).
#[allow(clippy::type_complexity)]
fn settings(
    d: &Document,
) -> (
    NumFormat,
    Option<Rgb>,
    Option<(BorderStyle, f32, Option<Rgb>, f32)>,
    NumFormat,
    Option<u32>,
    Option<NoteRestart>,
    NumFormat,
    Option<(u32, f32, LineNumberRestart)>,
) {
    let sec = &d.last_section;
    let top = sec.page_borders.and_then(|b| b.top).map(|b| (b.style, b.width, b.color, b.space));
    let lines = sec.line_numbers.as_ref().map(|l| (l.count_by, l.distance, l.restart));
    (
        sec.page_num_format,
        d.settings.page_color,
        top,
        d.settings.footnote_format,
        d.settings.footnote_pr.num_start,
        d.settings.footnote_pr.num_restart,
        d.settings.endnote_format,
        lines,
    )
}

#[test]
fn page_settings_open() {
    assert_eq!(
        settings(&open()),
        (
            NumFormat::LowerRoman,
            Some(Rgb(0xff, 0xee, 0xcc)),
            // At the margin, 1in from the edge (WordCraft measures from the edge).
            Some((BorderStyle::Single, 0.5, Some(Rgb(255, 0, 0)), 72.0)),
            NumFormat::LowerLetter,
            // ODF counts from 0: start value 4 is the fifth.
            Some(5),
            Some(NoteRestart::EachPage),
            NumFormat::UpperRoman,
            Some((5, 18.0, LineNumberRestart::Page)),
        )
    );
}

#[test]
fn the_text_starts_inside_a_page_border() {
    // ODF's margin is the border's distance from the edge; the text is inside its padding.
    let d = open();
    assert_eq!(d.last_section.margin_left, 72.0 + 0.5 + 10.0);
    assert_eq!(d.last_section.margin_top, 72.0 + 0.5 + 10.0);
}

#[test]
fn page_settings_round_trip() {
    let d = open();
    assert_eq!(settings(&import(&export(&d).unwrap()).unwrap()), settings(&d));
    // Defaults stay defaults.
    let plain = Document::new();
    assert_eq!(settings(&import(&export(&plain).unwrap()).unwrap()), settings(&plain));
}

#[test]
fn hostile_page_settings_do_not_panic() {
    let styles = format!(
        r#"<office:document-styles {NS_T}><office:styles><text:notes-configuration text:note-class="footnote" text:start-value="99999999999" style:num-format="zz"/><text:linenumbering-configuration text:number-lines="true" text:increment="-4" text:offset="1e30in"/></office:styles><office:automatic-styles><style:page-layout style:name="pm1"><style:page-layout-properties fo:border="1e30pt solid" fo:padding="-5in" fo:background-color="transparent"/></style:page-layout></office:automatic-styles><office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm1"/></office:master-styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {NS_T}><office:body><office:text><text:p>x</text:p></office:text></office:body></office:document-content>"#
    );
    let d = import(&package(&content, Some(&styles))).unwrap();
    let s = settings(&d);
    assert!(s.4.is_none_or(|n| n <= craft_wordmodel::section::MAX_NOTE_START), "{s:?}");
    assert!(s.7.as_ref().is_none_or(|l| l.0 >= 1 && l.1.is_finite() && l.1 <= 1584.0), "{s:?}");
    let _ = export(&d).unwrap();
}

#[test]
fn line_numbering_is_on_unless_it_says_otherwise() {
    // ODF 1.3 §19.845: `text:number-lines` defaults to true; LibreOffice leaves it out.
    for (attr, on) in [("", true), (r#"text:number-lines="false""#, false)] {
        let styles = format!(
            r#"<office:document-styles {NS_T}><office:styles><text:linenumbering-configuration {attr} text:increment="5"/></office:styles><office:automatic-styles><style:page-layout style:name="pm1"><style:page-layout-properties fo:page-width="8.5in" fo:page-height="11in"/></style:page-layout></office:automatic-styles><office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm1"/></office:master-styles></office:document-styles>"#
        );
        let content = format!(
            r#"<office:document-content {NS_T}><office:body><office:text><text:p>x</text:p></office:text></office:body></office:document-content>"#
        );
        let d = import(&package(&content, Some(&styles))).unwrap();
        assert_eq!(d.last_section.line_numbers.is_some(), on, "{attr}");
    }
}

#[test]
fn a_page_layout_without_a_size_is_the_default_pages_size() {
    // LibreOffice's templates often state margins but no page size: it is the default paper's
    // (the user's region's in WordCraft, as LibreOffice uses its own default).
    let styles = format!(
        r#"<office:document-styles {}><office:automatic-styles><style:page-layout style:name="pm1"><style:page-layout-properties fo:margin-top="1in" fo:margin-bottom="1in" fo:margin-left="1.25in" fo:margin-right="1.25in"/></style:page-layout></office:automatic-styles><office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm1"/></office:master-styles></office:document-styles>"#,
        crate::text::tests::NS_T
    );
    let content = format!(
        "<office:document-content {}><office:body><office:text><text:p>x</text:p></office:text></office:body></office:document-content>",
        crate::text::tests::NS_T
    );
    let bytes = crate::text::tests::package(&content, Some(&styles));
    let a4 = craft_wordmodel::SectionProps { page_w: 595.3, page_h: 841.9, ..Default::default() };
    let d = crate::text::tests::import_on_ext("odt", &bytes, &a4).unwrap().unwrap();
    assert_eq!((d.last_section.page_w, d.last_section.page_h), (595.3, 841.9));
    assert_eq!((d.last_section.margin_top, d.last_section.margin_left), (72.0, 90.0));
    // A stated size is the file's.
    let sized = styles.replace(r#"<style:page-layout-properties "#, r#"<style:page-layout-properties fo:page-width="5in" fo:page-height="7in" "#);
    let d = crate::text::tests::import_on_ext("odt", &crate::text::tests::package(&content, Some(&sized)), &a4).unwrap().unwrap();
    assert_eq!((d.last_section.page_w, d.last_section.page_h), (360.0, 504.0));
}

#[test]
fn a_document_without_a_page_layout_has_the_default_paper_and_no_margins() {
    // No styles.xml at all: the region's paper (the standard leaves the size to the
    // application), and no margins (the standard leaves them open too; 0 is XSL's initial value).
    // It is laid out as an ODF document.
    let content = format!(
        "<office:document-content {}><office:body><office:text><text:p>x</text:p></office:text></office:body></office:document-content>",
        crate::text::tests::NS_T
    );
    let a4 = craft_wordmodel::SectionProps { page_w: 595.3, page_h: 841.9, ..Default::default() };
    let d = crate::text::tests::import_on_ext("odt", &crate::text::tests::package(&content, None), &a4).unwrap().unwrap();
    let s = &d.last_section;
    assert_eq!((s.page_w, s.page_h), (595.3, 841.9));
    assert_eq!([s.margin_top, s.margin_bottom, s.margin_left, s.margin_right], [0.0; 4], "{s:?}");
    assert!(d.settings.odf.is_some());
}

#[test]
fn a_page_layout_stating_nothing_has_no_margins() {
    // LibreOffice's CV template: a page layout with no properties at all. Its margins are XSL's
    // initial value, 0.
    let ns = crate::text::tests::NS_T;
    let styles = format!(
        r#"<office:document-styles {ns}><office:automatic-styles><style:page-layout style:name="Mpm1"><style:header-style/><style:footer-style/></style:page-layout></office:automatic-styles><office:master-styles><style:master-page style:name="Standard" style:page-layout-name="Mpm1"/></office:master-styles></office:document-styles>"#
    );
    let content =
        format!("<office:document-content {ns}><office:body><office:text><text:p>x</text:p></office:text></office:body></office:document-content>");
    let d = crate::text::tests::import_ext("odt", &crate::text::tests::package(&content, Some(&styles))).unwrap().unwrap();
    let s = &d.last_section;
    assert_eq!([s.margin_top, s.margin_bottom, s.margin_left, s.margin_right], [0.0; 4]);
    // One margin stated: that one is the file's.
    let one = styles.replace("<style:header-style/>", r#"<style:page-layout-properties fo:margin-left="1in"/><style:header-style/>"#);
    let d = crate::text::tests::import_ext("odt", &crate::text::tests::package(&content, Some(&one))).unwrap().unwrap();
    assert_eq!((d.last_section.margin_left, d.last_section.margin_top), (72.0, 0.0));
}

#[test]
fn a_page_background_picture_is_read_and_written() {
    // LibreOffice's report templates: a picture centred on the page, not repeated.
    let png = {
        let img = image::RgbImage::from_pixel(8, 4, image::Rgb([200, 30, 30]));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    };
    let ns = crate::text::tests::NS_T;
    let layout = |repeat: &str| {
        format!(
            r#"<office:document-styles {ns} xmlns:xlink="http://www.w3.org/1999/xlink"><office:automatic-styles><style:page-layout style:name="pm1"><style:page-layout-properties fo:page-width="8.5in" fo:page-height="11in"><style:background-image xlink:href="Pictures/map.png" style:position="center center" {repeat}/></style:page-layout-properties></style:page-layout></office:automatic-styles><office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm1"/></office:master-styles></office:document-styles>"#
        )
    };
    let content =
        format!("<office:document-content {ns}><office:body><office:text><text:p>x</text:p></office:text></office:body></office:document-content>");
    for (repeat, fill) in [(r#"style:repeat="no-repeat""#, "center"), (r#"style:repeat="stretch""#, "frame"), ("", "tile")] {
        let styles = layout(repeat);
        let pkg = crate::text::tests::zip_of(&[("content.xml", content.as_bytes()), ("styles.xml", styles.as_bytes()), ("Pictures/map.png", &png)]);
        let d = crate::text::import(&pkg).unwrap();
        let pic = d.settings.page_picture.as_ref().expect("a page picture");
        assert_eq!(pic.fill_type, fill);
        assert_eq!(d.media.get(&pic.media).map(|b| b.as_slice()), Some(png.as_slice()));
        // Written back in the page layout, the picture in the package.
        let saved = crate::text::export(&d).unwrap();
        let s = crate::text::tests::entry(&saved, "styles.xml");
        let want_repeat = match fill {
            "center" => "no-repeat",
            "frame" => "stretch",
            _ => "repeat",
        };
        assert!(s.contains(&format!(r#"style:repeat="{want_repeat}""#)) && s.contains("Pictures/page-background.png"), "{s}");
        let again = crate::text::import(&saved).unwrap();
        assert_eq!(again.settings.page_picture.as_ref().map(|p| p.fill_type.as_str()), Some(fill));
    }
}

#[test]
fn a_page_borders_padding_keeps_the_text_where_it_was() {
    // Design › Page Borders around a page with 1 inch margins: the border 24 pt from the edge,
    // 45 pt of padding inside it (more than Word's 31 pt border spacing, which is measured
    // differently): the text stays 72 pt from the edge, written and read back.
    let mut d = Document::new();
    d.body = vec![craft_wordmodel::para_block(craft_wordmodel::Paragraph::with_text("x", Default::default()))];
    let edge = craft_wordmodel::props::Border { style: BorderStyle::Single, width: 3.0, color: Some(Rgb(0x2f, 0x55, 0x97)), space: 24.0 };
    d.last_section.page_borders =
        Some(craft_wordmodel::props::Borders { top: Some(edge), bottom: Some(edge), left: Some(edge), right: Some(edge), ..Default::default() });
    let back = crate::text::import(&crate::text::export(&d).unwrap()).unwrap();
    let s = &back.last_section;
    for m in [s.margin_top, s.margin_bottom, s.margin_left, s.margin_right] {
        assert!((m - 72.0).abs() < 0.05, "{s:?}");
    }
    assert_eq!(s.page_borders.and_then(|b| b.left).map(|b| (b.width, b.space)), Some((3.0, 24.0)));
    // storytold/wordcraft#5065: a page border of width 0 (a DOCX one without `w:sz`) is still
    // drawn, at the thinnest line, rather than lost: ODF has no border that is drawn and still
    // takes no room.
    let thin = craft_wordmodel::props::Border { width: 0.0, ..edge };
    d.last_section.page_borders = Some(craft_wordmodel::props::Borders { left: Some(thin), ..Default::default() });
    let styles = crate::text::tests::styles_of(&crate::text::export(&d).unwrap());
    let thinnest = crate::text::writer::pt(craft_wordmodel::props::Border::THINNEST);
    assert!(styles.contains(&format!(r#"fo:border-left="{thinnest} solid"#)), "{styles}");
}

/// A file with what WordCraft doesn't model of a page: how its pages are used, the footnote line,
/// a header's background.
fn page_file(extra_layouts: &str, masters: &str, body: &str) -> Vec<u8> {
    let ns = crate::text::tests::NS_T;
    let styles = format!(
        r##"<office:document-styles {ns}><office:automatic-styles><style:page-layout style:name="pm1" style:page-usage="mirrored"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm" fo:margin-top="2cm" fo:margin-bottom="2cm" fo:margin-left="3cm" fo:margin-right="2cm" style:print-orientation="portrait"><style:footnote-sep style:width="0.05cm" style:color="#ff0000"/></style:page-layout-properties><style:header-style><style:header-footer-properties fo:min-height="1cm" fo:margin-bottom="0.5cm" fo:background-color="#eeffee" style:dynamic-spacing="false"/></style:header-style></style:page-layout>{extra_layouts}</office:automatic-styles><office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm1"><style:header><text:p>Kopf</text:p></style:header></style:master-page>{masters}</office:master-styles></office:document-styles>"##
    );
    let content = format!("<office:document-content {ns}><office:body><office:text>{body}</office:text></office:body></office:document-content>");
    crate::text::tests::package(&content, Some(&styles))
}

#[test]
fn a_page_setup_change_leaves_the_rest_of_the_files_page_definitions() {
    // Changing one margin wrote every page layout and master page from the model, so a header's
    // background and the footnote line were gone in LibreOffice.
    let bytes = page_file("", "", "<text:p>x</text:p>");
    let mut d = crate::text::import(&bytes).unwrap();
    d.last_section.margin_left = 100.0;
    let saved = crate::text::export(&d).unwrap();
    let s = crate::text::tests::styles_of(&saved);
    assert!(s.contains(r#"style:page-layout style:name="pm1" style:page-usage="mirrored""#), "the file's layout: {s}");
    assert!(s.contains(r##"style:color="#ff0000""##) && s.contains(r##"fo:background-color="#eeffee""##), "what WordCraft doesn't model: {s}");
    assert!(s.contains(r#"fo:margin-left="100pt""#) && !s.contains(r#"fo:margin-left="3cm""#), "the change: {s}");
    assert!(s.contains(r#"fo:page-width="21cm""#) && s.contains(r#"fo:margin-right="2cm""#), "the rest, in the file's units: {s}");
    assert!(
        s.contains("<text:p>Kopf</text:p>") && s.contains(r#"style:master-page style:name="Standard" style:page-layout-name="pm1""#),
        "its header: {s}"
    );
    assert_eq!(crate::text::import(&saved).unwrap().last_section.margin_left, 100.0);
    // Size and orientation.
    let mut d = crate::text::import(&bytes).unwrap();
    let (w, h) = (d.last_section.page_h, d.last_section.page_w);
    (d.last_section.page_w, d.last_section.page_h) = (w, h);
    let s = crate::text::tests::styles_of(&crate::text::export(&d).unwrap());
    assert!(s.contains(r#"style:print-orientation="landscape""#) && s.contains(r##"style:color="#ff0000""##), "{s}");
    // Columns, in the place the schema gives them, before the footnote line.
    let mut d = crate::text::import(&bytes).unwrap();
    d.last_section.columns.count = 2;
    let saved = crate::text::export(&d).unwrap();
    let s = crate::text::tests::styles_of(&saved);
    assert!(s.find("<style:columns").is_some_and(|c| s.find("<style:footnote-sep").is_some_and(|f| c < f)), "{s}");
    assert_eq!(crate::text::import(&saved).unwrap().last_section.columns.count, 2);
    // Changed back, nothing is left of the change.
    let mut d = crate::text::import(&bytes).unwrap();
    let was = d.last_section.margin_left;
    d.last_section.margin_left = 50.0;
    d.last_section.margin_left = was;
    assert!(crate::text::tests::styles_of(&crate::text::export(&d).unwrap()).contains(r#"fo:margin-left="3cm""#));
}

#[test]
fn a_first_pages_change_goes_on_to_the_pages_after_it() {
    // WordCraft reads a first page style and the pages after it as one section.
    let second = r#"<style:page-layout style:name="pm2"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm" fo:margin-left="3cm"/></style:page-layout>"#;
    let first = r#"<style:master-page style:name="First" style:page-layout-name="pm2" style:next-style-name="Standard"/>"#;
    let bytes = page_file(second, first, "<text:p>x</text:p>");
    let mut d = crate::text::import(&bytes).unwrap();
    d.last_section.margin_left = 100.0;
    let s = crate::text::tests::styles_of(&crate::text::export(&d).unwrap());
    assert!(s.contains(r#"fo:margin-left="100pt""#) && s.contains(r##"style:color="#ff0000""##), "{s}");
}

#[test]
fn a_first_pages_header_measure_stays_off_later_pages_without_one() {
    // A first page with a header, and later pages with none: a top margin change moves the first
    // page's header band, and gives the later pages' layout no header measurements.
    let ns = crate::text::tests::NS_T;
    let styles = format!(
        r#"<office:document-styles {ns}><office:automatic-styles><style:page-layout style:name="later"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm" fo:margin-top="2cm" fo:margin-bottom="2cm" fo:margin-left="2cm" fo:margin-right="2cm"/><style:header-style/><style:footer-style/></style:page-layout><style:page-layout style:name="first"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm" fo:margin-top="3cm" fo:margin-bottom="2cm" fo:margin-left="2cm" fo:margin-right="2cm"/><style:header-style><style:header-footer-properties fo:min-height="3cm" fo:margin-bottom="0.4cm"/></style:header-style><style:footer-style/></style:page-layout></office:automatic-styles><office:master-styles><style:master-page style:name="Standard" style:page-layout-name="later"/><style:master-page style:name="First" style:page-layout-name="first" style:next-style-name="Standard"><style:header><text:p>Kopf</text:p></style:header></style:master-page></office:master-styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {ns}><office:automatic-styles><style:style style:name="P1" style:family="paragraph" style:master-page-name="First"/></office:automatic-styles><office:body><office:text><text:p text:style-name="P1">x</text:p></office:text></office:body></office:document-content>"#
    );
    let bytes = crate::text::tests::package(&content, Some(&styles));
    let mut d = crate::text::import(&bytes).unwrap();
    assert!(d.last_section.margin_top > 150.0, "the header's reach is the top margin: {}", d.last_section.margin_top);
    d.last_section.margin_top = 150.0;
    let s = crate::text::tests::styles_of(&crate::text::export(&d).unwrap());
    let later = s.split(r#"style:name="later""#).nth(1).and_then(|r| r.split("</style:page-layout>").next()).unwrap();
    assert!(later.contains("<style:header-style/>") && later.contains(r#"fo:margin-top="2cm""#), "later pages: {later}");
    let first = s.split(r#"style:name="first""#).nth(1).and_then(|r| r.split("</style:page-layout>").next()).unwrap();
    assert!(first.contains(r#"fo:margin-top="3cm""#) && !first.contains(r#"fo:min-height="3cm""#), "first page: {first}");
}

#[test]
fn later_pages_header_keeps_its_distance_when_the_first_page_has_none() {
    // A first page with no header, and later pages with one: the later pages' top margin measures
    // to the header, not the text, so the first page's new text margin isn't theirs.
    let ns = crate::text::tests::NS_T;
    let styles = format!(
        r#"<office:document-styles {ns}><office:automatic-styles><style:page-layout style:name="later"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm" fo:margin-top="1cm" fo:margin-bottom="2cm" fo:margin-left="2cm" fo:margin-right="2cm"/><style:header-style><style:header-footer-properties fo:min-height="1cm"/></style:header-style></style:page-layout><style:page-layout style:name="first"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm" fo:margin-top="2cm" fo:margin-bottom="2cm" fo:margin-left="2cm" fo:margin-right="2cm"/><style:header-style/></style:page-layout></office:automatic-styles><office:master-styles><style:master-page style:name="Standard" style:page-layout-name="later"><style:header><text:p>Kopf</text:p></style:header></style:master-page><style:master-page style:name="First" style:page-layout-name="first" style:next-style-name="Standard"/></office:master-styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {ns}><office:automatic-styles><style:style style:name="P1" style:family="paragraph" style:master-page-name="First"/></office:automatic-styles><office:body><office:text><text:p text:style-name="P1">x</text:p></office:text></office:body></office:document-content>"#
    );
    let bytes = crate::text::tests::package(&content, Some(&styles));
    let mut d = crate::text::import(&bytes).unwrap();
    d.last_section.margin_top = 100.0;
    let saved = crate::text::export(&d).unwrap();
    let s = crate::text::tests::styles_of(&saved);
    let later = s.split(r#"style:name="later""#).nth(1).and_then(|r| r.split("</style:page-layout>").next()).unwrap();
    assert!(later.contains(r#"fo:margin-top="1cm""#) && later.contains(r#"fo:min-height="71.65pt""#), "later pages: {later}");
    let first = s.split(r#"style:name="first""#).nth(1).and_then(|r| r.split("</style:page-layout>").next()).unwrap();
    assert!(first.contains(r#"fo:margin-top="100pt""#) && first.contains("<style:header-style/>"), "first page: {first}");
    // Reopened, the change is still there.
    let back = crate::text::import(&saved).unwrap();
    assert!((back.last_section.margin_top - 100.0).abs() < 0.01, "{}", back.last_section.margin_top);
}

#[test]
fn a_bands_spacing_counts_once_when_the_page_setup_changes() {
    // The band's height and its spacing together reach the text. Written as the height alone,
    // with the spacing again after it, each save moved the text down by the spacing.
    let ns = crate::text::tests::NS_T;
    let styles = format!(
        r#"<office:document-styles {ns}><office:automatic-styles><style:page-layout style:name="pm1"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm" fo:margin-top="1cm" fo:margin-bottom="1cm" fo:margin-left="2cm" fo:margin-right="2cm"/><style:header-style><style:header-footer-properties fo:min-height="1cm" fo:margin-bottom="1cm"/></style:header-style><style:footer-style><style:header-footer-properties fo:min-height="1cm" fo:margin-top="0.5cm"/></style:footer-style></style:page-layout></office:automatic-styles><office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm1"><style:header><text:p>Kopf</text:p></style:header><style:footer><text:p>Fuss</text:p></style:footer></style:master-page></office:master-styles></office:document-styles>"#
    );
    let content =
        format!("<office:document-content {ns}><office:body><office:text><text:p>x</text:p></office:text></office:body></office:document-content>");
    let bytes = crate::text::tests::package(&content, Some(&styles));
    // The file's page layout, changed.
    let mut d = crate::text::import(&bytes).unwrap();
    let was = d.last_section.margin_top;
    d.last_section.margin_top = 150.0;
    d.last_section.margin_bottom = 120.0;
    let saved = crate::text::export(&d).unwrap();
    let s = crate::text::tests::styles_of(&saved);
    assert!(s.contains(r#"fo:margin-bottom="1cm""#) && s.contains(r#"fo:margin-top="0.5cm""#), "the spacing stays: {s}");
    let back = crate::text::import(&saved).unwrap();
    assert!((back.last_section.margin_top - 150.0).abs() < 0.02, "{}", back.last_section.margin_top);
    assert!((back.last_section.margin_bottom - 120.0).abs() < 0.02, "{}", back.last_section.margin_bottom);
    // Less room than the spacing takes it from the spacing.
    let mut d = crate::text::import(&bytes).unwrap();
    d.last_section.margin_top = d.last_section.header + 10.0;
    let back = crate::text::import(&crate::text::export(&d).unwrap()).unwrap();
    assert!((back.last_section.margin_top - (back.last_section.header + 10.0)).abs() < 0.02, "{}", back.last_section.margin_top);
    // WordCraft's own page layout, written whole when the kind of page changes.
    let mut d = crate::text::import(&bytes).unwrap();
    d.last_section.page_num_start = Some(5);
    let back = crate::text::import(&crate::text::export(&d).unwrap()).unwrap();
    assert!((back.last_section.margin_top - was).abs() < 0.02, "{} after {was}", back.last_section.margin_top);
    let again = crate::text::import(&crate::text::export(&back).unwrap()).unwrap();
    assert!((again.last_section.margin_top - was).abs() < 0.02, "{} after {was}", again.last_section.margin_top);
}

/// A header's spacing changed where its text starts doesn't move: the band's height gives way.
#[test]
fn a_change_of_only_a_bands_spacing_leaves_the_text_where_it_was() {
    let ns = crate::text::tests::NS_T;
    let styles = format!(
        r#"<office:document-styles {ns}><office:automatic-styles><style:page-layout style:name="pm1"><style:page-layout-properties fo:page-width="500pt" fo:page-height="700pt" fo:margin-top="20pt" fo:margin-bottom="20pt" fo:margin-left="20pt" fo:margin-right="20pt"/><style:header-style><style:header-footer-properties fo:min-height="40pt" fo:margin-bottom="10pt"/></style:header-style></style:page-layout></office:automatic-styles><office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm1"><style:header><text:p>h</text:p></style:header></style:master-page></office:master-styles></office:document-styles>"#
    );
    let content =
        format!("<office:document-content {ns}><office:body><office:text><text:p>x</text:p></office:text></office:body></office:document-content>");
    let mut d = crate::text::import(&crate::text::tests::package(&content, Some(&styles))).unwrap();
    let was = d.last_section.margin_top;
    d.last_section.header_gap = 20.0;
    let back = crate::text::import(&crate::text::export(&d).unwrap()).unwrap();
    assert!((back.last_section.margin_top - was).abs() < 0.02, "{} after {was}", back.last_section.margin_top);
}

#[test]
fn a_fixed_height_band_stays_one() {
    let ns = crate::text::tests::NS_T;
    let styles = format!(
        r#"<office:document-styles {ns}><office:automatic-styles><style:page-layout style:name="pm1"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm" fo:margin-top="1cm" fo:margin-bottom="1cm" fo:margin-left="2cm" fo:margin-right="2cm"/><style:header-style><style:header-footer-properties svg:height="1cm"/></style:header-style></style:page-layout></office:automatic-styles><office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm1"><style:header><text:p>Kopf</text:p></style:header></style:master-page></office:master-styles></office:document-styles>"#
    );
    let content =
        format!("<office:document-content {ns}><office:body><office:text><text:p>x</text:p></office:text></office:body></office:document-content>");
    let bytes = crate::text::tests::package(&content, Some(&styles));
    let mut d = crate::text::import(&bytes).unwrap();
    d.last_section.margin_top = 100.0;
    let saved = crate::text::export(&d).unwrap();
    let s = crate::text::tests::styles_of(&saved);
    let layout = s.split(r#"style:name="pm1""#).nth(1).and_then(|r| r.split("</style:page-layout>").next()).unwrap();
    assert!(layout.contains(r#"svg:height="71.65pt""#) && !layout.contains("fo:min-height"), "{layout}");
    assert!((crate::text::import(&saved).unwrap().last_section.margin_top - 100.0).abs() < 0.02);
}

#[test]
fn later_pages_with_their_own_margin_keep_it() {
    // A first page set apart: what it changes isn't what the later pages say.
    let ns = crate::text::tests::NS_T;
    let styles = format!(
        r#"<office:document-styles {ns}><office:automatic-styles><style:page-layout style:name="later"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm" fo:margin-top="2cm" fo:margin-bottom="2cm" fo:margin-left="2cm" fo:margin-right="2cm"/></style:page-layout><style:page-layout style:name="first"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm" fo:margin-top="5cm" fo:margin-bottom="2cm" fo:margin-left="4cm" fo:margin-right="2cm"/></style:page-layout></office:automatic-styles><office:master-styles><style:master-page style:name="Standard" style:page-layout-name="later"/><style:master-page style:name="First" style:page-layout-name="first" style:next-style-name="Standard"/></office:master-styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {ns}><office:automatic-styles><style:style style:name="P1" style:family="paragraph" style:master-page-name="First"/></office:automatic-styles><office:body><office:text><text:p text:style-name="P1">x</text:p></office:text></office:body></office:document-content>"#
    );
    let bytes = crate::text::tests::package(&content, Some(&styles));
    let mut d = crate::text::import(&bytes).unwrap();
    d.last_section.margin_top = 100.0;
    d.last_section.margin_left = 90.0;
    d.last_section.margin_right = 80.0;
    let s = crate::text::tests::styles_of(&crate::text::export(&d).unwrap());
    let layout = |name: &str| {
        let key = format!(r#"style:name="{name}""#);
        s.split(key.as_str()).nth(1).and_then(|r| r.split("</style:page-layout>").next()).unwrap().to_string()
    };
    let (first, later) = (layout("first"), layout("later"));
    assert!(first.contains(r#"fo:margin-top="100pt""#) && first.contains(r#"fo:margin-left="90pt""#), "{first}");
    assert!(later.contains(r#"fo:margin-top="2cm""#) && later.contains(r#"fo:margin-left="2cm""#), "{later}");
    // Where both said the same, the change is the later pages' too.
    assert!(first.contains(r#"fo:margin-right="80pt""#) && later.contains(r#"fo:margin-right="80pt""#), "{first} {later}");
}

#[test]
fn sections_sharing_a_page_layout_can_still_differ() {
    // One section changed: the page layout they share can't say both, so each gets its own.
    let style = r#"<office:automatic-styles><style:style style:name="P1" style:family="paragraph" style:master-page-name="Standard"/></office:automatic-styles>"#;
    let ns = crate::text::tests::NS_T;
    let content = format!(
        r#"<office:document-content {ns}>{style}<office:body><office:text><text:p>one</text:p><text:p text:style-name="P1">two</text:p></office:text></office:body></office:document-content>"#
    );
    let styles = r##"<office:automatic-styles><style:page-layout style:name="pm1"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm" fo:margin-left="3cm" fo:margin-right="2cm" fo:margin-top="2cm" fo:margin-bottom="2cm"/></style:page-layout></office:automatic-styles><office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm1"/></office:master-styles>"##;
    let bytes = crate::text::tests::package(&content, Some(&format!("<office:document-styles {ns}>{styles}</office:document-styles>")));
    let mut d = crate::text::import(&bytes).unwrap();
    assert!(d.body.iter().filter_map(|b| b.as_para()).any(|p| p.section.is_some()), "two sections");
    d.last_section.margin_left = 100.0;
    let back = crate::text::import(&crate::text::export(&d).unwrap()).unwrap();
    assert_eq!(back.last_section.margin_left, 100.0);
    let first = back.body.iter().filter_map(|b| b.as_para()).find_map(|p| p.section.as_ref()).map(|s| s.margin_left);
    assert!(first.is_some_and(|m| (m - 3.0 * 72.0 / 2.54).abs() < 0.1), "{first:?}");
}

#[test]
fn hostile_page_definitions_do_not_stop_a_page_setup_change() {
    let layouts =
        r#"<style:page-layout style:name="odd"><style:page-layout-properties fo:margin-left='1in' fo:margin-left2="x" a="b"/></style:page-layout>"#;
    let masters = r#"<style:master-page style:name="A" style:page-layout-name="odd" style:next-style-name="B"/><style:master-page style:name="B" style:page-layout-name="odd" style:next-style-name="A"/><style:master-page style:name="C" style:page-layout-name="none" style:next-style-name="C"/>"#;
    let bytes = page_file(layouts, masters, "<text:p>x</text:p>");
    let mut d = crate::text::import(&bytes).unwrap();
    d.last_section.margin_left = f32::NAN;
    let _ = crate::text::export(&d).unwrap();
    d.last_section.margin_left = 1e30;
    d.last_section.page_w = -5.0;
    let saved = crate::text::export(&d).unwrap();
    assert!(crate::text::import(&saved).is_ok());
}

/// A margin changed in the same save as the watermark or odd and even pages: the margin alone could
/// stay a change to the file's page layouts, but those two aren't in them, and would be lost.
#[test]
fn a_margin_changed_with_the_watermark_or_odd_and_even_pages_keeps_them() {
    let file = |left: &str| {
        let styles = format!(
            r#"<office:document-styles {NS_T}><office:automatic-styles><style:page-layout style:name="pm1" style:page-usage="mirrored"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm" fo:margin-top="2cm" fo:margin-bottom="2cm" fo:margin-left="2cm" fo:margin-right="2cm"/><style:header-style><style:header-footer-properties fo:min-height="1cm"/></style:header-style></style:page-layout></office:automatic-styles><office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm1"><style:header><text:p>Right head</text:p></style:header>{left}</style:master-page></office:master-styles></office:document-styles>"#
        );
        let content = format!(
            r#"<office:document-content {NS_T}><office:body><office:text><text:p>x</text:p></office:text></office:body></office:document-content>"#
        );
        package(&content, Some(&styles))
    };
    let mut d = import(&file("")).unwrap();
    d.last_section.margin_left += 30.0;
    d.settings.watermark = Some(craft_wordmodel::Watermark { text: "DRAFT".into(), ..Default::default() });
    let saved = export(&d).unwrap();
    let s = crate::text::tests::entry(&saved, "styles.xml");
    assert!(s.contains("PowerPlusWaterMarkObject") && s.contains("DRAFT"), "the watermark: {s}");
    let back = import(&saved).unwrap();
    assert_eq!(back.settings.watermark.map(|w| w.text).as_deref(), Some("DRAFT"));
    assert!((back.last_section.margin_left - d.last_section.margin_left).abs() < 0.5, "the margin: {s}");

    let mut d = import(&file("<style:header-left><text:p>Left head</text:p></style:header-left>")).unwrap();
    assert!(d.settings.even_odd_headers);
    d.last_section.margin_left += 30.0;
    d.settings.even_odd_headers = false;
    let saved = export(&d).unwrap();
    let s = crate::text::tests::entry(&saved, "styles.xml");
    assert!(!s.contains("<style:header-left"), "the left pages' header: {s}");
    let back = import(&saved).unwrap();
    assert!(!back.settings.even_odd_headers);
    assert!((back.last_section.margin_left - d.last_section.margin_left).abs() < 0.5, "the margin: {s}");
}

/// XML allows space around an attribute's `=`, and either quote. A change replaces such an
/// attribute: a second one by that name would make a file no reader can open.
#[test]
fn a_page_setup_change_replaces_an_attribute_however_the_file_writes_it() {
    let ns = crate::text::tests::NS_T;
    let styles = format!(
        r#"<office:document-styles {ns}><office:automatic-styles><style:page-layout style:name="pm1"><style:page-layout-properties style:print-orientation=' fo:margin-left="9cm"' fo:page-width = "21cm" fo:page-height= '29.7cm' fo:margin-top = "2cm" fo:margin-bottom ="2cm" fo:margin-left= '3cm' fo:margin-right
 =
 "2cm"/></style:page-layout></office:automatic-styles><office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm1"/></office:master-styles></office:document-styles>"#
    );
    let content =
        format!("<office:document-content {ns}><office:body><office:text><text:p>x</text:p></office:text></office:body></office:document-content>");
    let mut d = import(&package(&content, Some(&styles))).unwrap();
    let s = &mut d.last_section;
    (s.margin_top, s.margin_bottom, s.margin_left, s.margin_right) = (100.0, 90.0, 80.0, 70.0);
    let saved = export(&d).unwrap();
    let written = crate::text::tests::entry(&saved, "styles.xml");
    let layout = written.split(r#"style:name="pm1""#).nth(1).and_then(|r| r.split("</style:page-layout>").next()).unwrap();
    for (name, value) in [("fo:margin-top", "100pt"), ("fo:margin-bottom", "90pt"), ("fo:margin-right", "70pt")] {
        assert_eq!(layout.matches(name).count(), 1, "{name} once: {layout}");
        assert!(layout.contains(&format!(r#"{name}="{value}""#)), "{name}: {layout}");
    }
    assert!(
        layout.contains(r#"fo:margin-left="80pt""#) && layout.contains(r#"' fo:margin-left="9cm"'"#),
        "the value that looks like one stays: {layout}"
    );
    let back = import(&saved).unwrap().last_section;
    assert_eq!((back.margin_top, back.margin_bottom, back.margin_left, back.margin_right), (100.0, 90.0, 80.0, 70.0), "{layout}");
}

/// Later pages follow a border change only where they said what the first pages did: what else
/// they say of their border and padding stays, stated side by side or all at once.
#[test]
fn a_border_change_keeps_what_later_pages_say_of_the_rest() {
    let ns = crate::text::tests::NS_T;
    let page =
        r#"fo:page-width="21cm" fo:page-height="29.7cm" fo:margin-top="2cm" fo:margin-bottom="2cm" fo:margin-left="2cm" fo:margin-right="2cm""#;
    let (black, red) = ("1pt solid #000000", "3pt solid #ff0000");
    let changed = |later: &str| {
        let styles = format!(
            r#"<office:document-styles {ns}><office:automatic-styles><style:page-layout style:name="later"><style:page-layout-properties {page} {later}/></style:page-layout><style:page-layout style:name="first"><style:page-layout-properties {page} fo:border="{black}" fo:padding="10pt"/></style:page-layout></office:automatic-styles><office:master-styles><style:master-page style:name="Standard" style:page-layout-name="later"/><style:master-page style:name="First" style:page-layout-name="first" style:next-style-name="Standard"/></office:master-styles></office:document-styles>"#
        );
        let content = format!(
            r#"<office:document-content {ns}><office:automatic-styles><style:style style:name="P1" style:family="paragraph" style:master-page-name="First"/></office:automatic-styles><office:body><office:text><text:p text:style-name="P1">x</text:p></office:text></office:body></office:document-content>"#
        );
        let mut d = import(&package(&content, Some(&styles))).unwrap();
        let top = d.last_section.page_borders.as_mut().and_then(|b| b.top.as_mut()).expect("a top border");
        (top.width, top.color) = (2.0, Some(Rgb(0, 0, 255)));
        let s = crate::text::tests::styles_of(&export(&d).unwrap());
        let layout = |name: &str| {
            let key = format!(r#"style:name="{name}""#);
            s.split(key.as_str()).nth(1).and_then(|r| r.split("</style:page-layout>").next()).unwrap().to_string()
        };
        (layout("first"), layout("later"))
    };
    let has = |layout: &str, attr: &str, value: &str| assert!(layout.contains(&format!(r#"{attr}="{value}""#)), "{attr}={value}: {layout}");
    let blue = "2pt solid #0000ff";

    // Their top said what the first pages' did; their other sides and their padding differ.
    let (first, later) = changed(&format!(r#"fo:border="{red}" fo:border-top="{black}" fo:padding="20pt""#));
    has(&first, "fo:border-top", blue);
    has(&first, "fo:border-left", black);
    has(&later, "fo:border-top", blue);
    for side in ["bottom", "left", "right"] {
        has(&later, &format!("fo:border-{side}"), red);
    }
    for side in ["top", "bottom", "left", "right"] {
        has(&later, &format!("fo:padding-{side}"), "20pt");
    }
    assert!(!later.contains(r#"fo:border=""#) && !later.contains(r#"fo:padding=""#), "{later}");

    // Their border said what the first pages' did, all at once; their padding differs.
    let (_, later) = changed(&format!(r#"fo:border="{black}" fo:padding="20pt""#));
    has(&later, "fo:border-top", blue);
    has(&later, "fo:border-left", black);
    has(&later, "fo:padding-left", "20pt");
}

/// A page layout written as read with a page setup change in another (the later pages' background
/// picture) keeps the package file it names.
#[test]
fn a_patched_page_setup_keeps_the_files_its_page_layouts_name() {
    let png = [b"\x89PNG\r\n\x1a\n".as_slice(), &[7u8; 64]].concat();
    let page =
        r#"fo:page-width="21cm" fo:page-height="29.7cm" fo:margin-top="2cm" fo:margin-bottom="2cm" fo:margin-left="2cm" fo:margin-right="2cm""#;
    let styles = format!(
        r#"<office:document-styles {NS_T} xmlns:xlink="http://www.w3.org/1999/xlink"><office:automatic-styles><style:page-layout style:name="pmfirst"><style:page-layout-properties {page}/></style:page-layout><style:page-layout style:name="pmnext"><style:page-layout-properties {page}><style:background-image xlink:href="Pictures/later.png" xlink:type="simple"/></style:page-layout-properties></style:page-layout></office:automatic-styles><office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pmnext"/><style:master-page style:name="First_20_Page" style:page-layout-name="pmfirst" style:next-style-name="Standard"/></office:master-styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {NS_T}><office:automatic-styles><style:style style:name="P1" style:family="paragraph" style:master-page-name="First_20_Page"/></office:automatic-styles><office:body><office:text><text:p text:style-name="P1">x</text:p></office:text></office:body></office:document-content>"#
    );
    let pkg = crate::text::tests::zip_of(&[("content.xml", content.as_bytes()), ("styles.xml", styles.as_bytes()), ("Pictures/later.png", &png)]);
    let mut d = import(&pkg).unwrap();
    d.last_section.margin_left += 20.0;
    let saved = export(&d).unwrap();
    let s = crate::text::tests::entry(&saved, "styles.xml");
    assert!(s.contains(r#"xlink:href="Pictures/later.png""#), "{s}");
    let mut z = crate::Package::open(&saved, &crate::Limits::DEFAULT).unwrap();
    assert_eq!(z.read("Pictures/later.png").unwrap(), Some(png));
    let m = crate::text::tests::entry(&saved, "META-INF/manifest.xml");
    assert!(m.contains(r#"manifest:full-path="Pictures/later.png""#), "{m}");
}

/// A first page with no header before later pages with one, both stating one margin: a change to
/// the top margin moves the later pages' text by as much, and a page number format both state goes
/// on to the later pages.
#[test]
fn later_pages_with_a_header_follow_a_first_page_without_one() {
    let page = r#"fo:page-width="21cm" fo:page-height="29.7cm" fo:margin-top="2cm" fo:margin-bottom="2cm" fo:margin-left="2cm" fo:margin-right="2cm" style:num-format="1""#;
    let styles = format!(
        r#"<office:document-styles {NS_T}><office:automatic-styles><style:page-layout style:name="pmfirst"><style:page-layout-properties {page}/></style:page-layout><style:page-layout style:name="pmnext"><style:page-layout-properties {page}/><style:header-style><style:header-footer-properties fo:min-height="0.5cm" fo:margin-bottom="0.2cm"/></style:header-style></style:page-layout></office:automatic-styles><office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pmnext"><style:header><text:p>H</text:p></style:header></style:master-page><style:master-page style:name="First_20_Page" style:page-layout-name="pmfirst" style:next-style-name="Standard"/></office:master-styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {NS_T}><office:automatic-styles><style:style style:name="P1" style:family="paragraph" style:master-page-name="First_20_Page"/></office:automatic-styles><office:body><office:text><text:p text:style-name="P1">x</text:p></office:text></office:body></office:document-content>"#
    );
    let open = || import(&package(&content, Some(&styles))).unwrap();
    let layout = |s: &str, name: &str| {
        s.split(&format!(r#"style:name="{name}""#)).nth(1).and_then(|r| r.split("</style:page-layout>").next()).unwrap().to_string()
    };

    let mut d = open();
    d.last_section.margin_top += 57.0;
    let s = crate::text::tests::entry(&export(&d).unwrap(), "styles.xml");
    let (first, later) = (layout(&s, "pmfirst"), layout(&s, "pmnext"));
    assert!(first.contains(r#"fo:margin-top="113.69pt""#), "{first}");
    // Its header stays where it is and takes the room, as any page with a header's does.
    assert!(later.contains(r#"fo:margin-top="2cm""#) && later.contains(r#"fo:margin-bottom="0.2cm""#), "{later}");
    assert!(later.contains(r#"fo:min-height="71.17pt""#), "the text 57 pt lower: {later}");

    let mut d = open();
    d.last_section.page_num_format = NumFormat::UpperRoman;
    let s = crate::text::tests::entry(&export(&d).unwrap(), "styles.xml");
    assert!(layout(&s, "pmfirst").contains(r#"style:num-format="I""#), "{s}");
    assert!(layout(&s, "pmnext").contains(r#"style:num-format="I""#), "{s}");
}

/// Later pages showing no header whose layout still has a header's measurements: their text starts
/// at the margin, so a top margin change moves it by as much, not past the unused header.
#[test]
fn later_pages_without_a_header_ignore_their_layouts_header_measurements() {
    let page =
        r#"fo:page-width="21cm" fo:page-height="29.7cm" fo:margin-top="20pt" fo:margin-bottom="20pt" fo:margin-left="2cm" fo:margin-right="2cm""#;
    let styles = format!(
        r#"<office:document-styles {NS_T}><office:automatic-styles><style:page-layout style:name="pmfirst"><style:page-layout-properties {page}/></style:page-layout><style:page-layout style:name="pmnext"><style:page-layout-properties {page}/><style:header-style><style:header-footer-properties fo:min-height="40pt" fo:margin-bottom="10pt"/></style:header-style></style:page-layout></office:automatic-styles><office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pmnext"/><style:master-page style:name="First_20_Page" style:page-layout-name="pmfirst" style:next-style-name="Standard"/></office:master-styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {NS_T}><office:automatic-styles><style:style style:name="P1" style:family="paragraph" style:master-page-name="First_20_Page"/></office:automatic-styles><office:body><office:text><text:p text:style-name="P1">x</text:p></office:text></office:body></office:document-content>"#
    );
    let mut d = import(&package(&content, Some(&styles))).unwrap();
    assert!((d.last_section.margin_top - 20.0).abs() < 0.01, "{}", d.last_section.margin_top);
    d.last_section.margin_top = 30.0;
    let s = crate::text::tests::entry(&export(&d).unwrap(), "styles.xml");
    let later = s.split(r#"style:name="pmnext""#).nth(1).and_then(|r| r.split("</style:page-layout>").next()).unwrap();
    assert!(later.contains(r#"fo:margin-top="30pt""#), "the text 10 pt lower: {later}");
    assert!(later.contains(r#"fo:min-height="40pt""#) && later.contains(r#"fo:margin-bottom="10pt""#), "its header's style as read: {later}");
}

/// A file listing another master page before `Standard`, its first paragraph naming none: WordCraft
/// reads the first pages from the first one listed, so a change to them is still there when the
/// saved file is read again.
#[test]
fn a_change_to_the_first_pages_lands_on_the_master_page_they_were_read_from() {
    let layouts = r#"<style:page-layout style:name="pmX"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm" fo:margin-left="5cm"/></style:page-layout>"#;
    let styles = format!(
        r#"<office:document-styles {NS_T}><office:automatic-styles><style:page-layout style:name="pm1"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm" fo:margin-left="2cm"/></style:page-layout>{layouts}</office:automatic-styles><office:master-styles><style:master-page style:name="X" style:page-layout-name="pmX"/><style:master-page style:name="Standard" style:page-layout-name="pm1"/></office:master-styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {NS_T}><office:body><office:text><text:p>x</text:p></office:text></office:body></office:document-content>"#
    );
    let mut d = import(&package(&content, Some(&styles))).unwrap();
    assert!((d.last_section.margin_left - 5.0 * 72.0 / 2.54).abs() < 0.1, "X's: {}", d.last_section.margin_left);
    d.last_section.margin_left = 100.0;
    let back = import(&export(&d).unwrap()).unwrap();
    assert_eq!(back.last_section.margin_left, 100.0);
}

/// Many sections using one long chain of master pages with large headers: a page setup change
/// reads each master page once, not once for every section using it.
#[test]
fn a_page_setup_change_reads_each_master_page_once() {
    use std::collections::BTreeMap;
    use std::sync::Arc;
    let header = format!("<style:header>{}</style:header>", "<text:p>h</text:p>".repeat(400));
    let mut defs: BTreeMap<String, Arc<str>> = BTreeMap::new();
    let mut order = Vec::new();
    defs.insert(
        "sauto:pm1|page-layout".into(),
        r#"<style:page-layout style:name="pm1"><style:page-layout-properties fo:margin-left="1in"/></style:page-layout>"#.into(),
    );
    order.push("sauto:pm1|page-layout".to_string());
    for i in 0..64 {
        let next = format!("M{}", (i + 1) % 64);
        let key = format!("master:M{i}");
        defs.insert(
            key.clone(),
            format!(
                r#"<style:master-page style:name="M{i}" style:page-layout-name="pm1" style:next-style-name="{next}">{header}</style:master-page>"#
            )
            .into(),
        );
        order.push(key);
    }
    let layout = r#"<style:page-layout style:name="layout"><style:page-layout-properties fo:margin-left="72pt"/></style:page-layout>"#;
    let changes: Vec<_> = (0..2_000).map(|_| crate::text::kept::PageChange { master: Some("M0"), then: layout, now: layout }).collect();
    let t = std::time::Instant::now();
    assert!(crate::text::kept::page_layout_patches(&defs, &order, &changes).is_some_and(|p| p.is_empty()));
    assert!(t.elapsed() < std::time::Duration::from_secs(2), "{:?}", t.elapsed());
}

/// A page layout with a header's style but no properties of its own: a margin change gives it
/// them first, where ODF 1.3 §16.5 puts them.
#[test]
fn a_page_layouts_new_properties_go_before_its_bands_styles() {
    let styles = format!(
        r#"<office:document-styles {NS_T}><office:automatic-styles><style:page-layout style:name="pm1"><style:header-style><style:header-footer-properties fo:min-height="1cm"/></style:header-style></style:page-layout></office:automatic-styles><office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm1"/></office:master-styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {NS_T}><office:body><office:text><text:p>x</text:p></office:text></office:body></office:document-content>"#
    );
    let mut d = import(&package(&content, Some(&styles))).unwrap();
    d.last_section.margin_left = 100.0;
    let s = crate::text::tests::styles_of(&export(&d).unwrap());
    let (props, band) = (s.find("<style:page-layout-properties"), s.find("<style:header-style"));
    assert!(props.zip(band).is_some_and(|(p, b)| p < b), "{s}");
}

/// A master page showing only a left pages' header: WordCraft reads its pages as having none, so a
/// top margin change leaves the header's style alone.
#[test]
fn a_left_pages_header_alone_is_no_header_for_a_margin_change() {
    let styles = format!(
        r#"<office:document-styles {NS_T}><office:automatic-styles><style:page-layout style:name="pm1"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm" fo:margin-top="2cm"/><style:header-style><style:header-footer-properties fo:min-height="1cm" fo:margin-bottom="0.5cm"/></style:header-style></style:page-layout></office:automatic-styles><office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm1"><style:header-left><text:p>L</text:p></style:header-left></style:master-page></office:master-styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {NS_T}><office:body><office:text><text:p>x</text:p></office:text></office:body></office:document-content>"#
    );
    let mut d = import(&package(&content, Some(&styles))).unwrap();
    d.last_section.margin_top += 20.0;
    let s = crate::text::tests::styles_of(&export(&d).unwrap());
    assert!(s.contains(r#"fo:min-height="1cm""#) && s.contains(r#"fo:margin-bottom="0.5cm""#), "{s}");
    let back = import(&export(&d).unwrap()).unwrap().last_section.margin_top;
    assert!((back - d.last_section.margin_top).abs() < 0.01, "{back}");
}

/// An ODT document with a page background and an active header containing a logo
/// preserves the page background (both modern draw:fill and legacy fallback) and the header logo (#5803).
#[test]
fn background_plus_small_header_logo_keeps_background() {
    let png = {
        let img = image::RgbImage::from_pixel(8, 4, image::Rgb([200, 30, 30]));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    };
    let logo_png = {
        let img = image::RgbImage::from_pixel(4, 4, image::Rgb([30, 200, 30]));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    };
    let mut d = Document::new();
    let bg_media = d.add_media(png, "png");
    d.settings.page_picture = Some(craft_wordmodel::PagePicture { media: bg_media, fill_type: "frame".into(), title: "PageBackground".into() });
    let logo_media = d.add_media(logo_png, "png");
    let mut hp = craft_wordmodel::Paragraph::new();
    hp.text = format!("Company {}", craft_wordmodel::para::OBJ);
    hp.objects.push(craft_wordmodel::InlineObject::Image {
        media: logo_media,
        w: 50.0,
        h: 20.0,
        alt: "Logo".into(),
        float: Default::default(),
        crop: [0.0; 4],
        fx: Default::default(),
        ole: None,
    });
    hp.runs = vec![
        craft_wordmodel::para::Run { len: 8, props: Default::default() },
        craft_wordmodel::para::Run { len: craft_wordmodel::para::OBJ.len_utf8(), props: Default::default() },
    ];
    d.parts.insert(
        1,
        craft_wordmodel::Part { kind: craft_wordmodel::PartKind::Header, blocks: vec![craft_wordmodel::para_block(hp)], body: Default::default() },
    );
    d.last_section.headers.default = Some(1);

    let saved = export(&d).unwrap();
    let s = crate::text::tests::entry(&saved, "styles.xml");
    // Verify both draw:fill="bitmap" and style:background-image are emitted consistently
    assert!(s.contains("draw:fill=\"bitmap\""), "page layout must declare draw:fill=\"bitmap\": {s}");
    assert!(s.contains("style:background-image"), "legacy background-image fallback must be present: {s}");
    assert!(s.contains("<draw:fill-image"), "draw:fill-image must be declared in office:styles: {s}");

    // Reading the saved ODT back preserves page_picture
    let back = import(&saved).unwrap();
    assert!(back.settings.page_picture.is_some(), "saved ODT must preserve page background picture");
}

/// An ODT document with a page background and a full-page behind-text header picture
/// preserves both the header picture and the page background on export and re-import (#5803).
#[test]
fn full_page_header_picture_with_background_round_trips() {
    let bg_png = {
        let img = image::RgbImage::from_pixel(8, 4, image::Rgb([200, 30, 30]));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    };
    let header_png = {
        let img = image::RgbImage::from_pixel(4, 8, image::Rgb([30, 30, 200]));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    };
    let mut d = Document::new();
    let bg_media = d.add_media(bg_png.clone(), "png");
    d.settings.page_picture = Some(craft_wordmodel::PagePicture { media: bg_media, fill_type: "frame".into(), title: "PageBackground".into() });
    let header_media = d.add_media(header_png.clone(), "png");
    let mut hp = craft_wordmodel::Paragraph::new();
    hp.text = format!("{}", craft_wordmodel::para::OBJ);
    hp.objects.push(craft_wordmodel::InlineObject::Image {
        media: header_media,
        w: 595.0,
        h: 842.0,
        alt: "Full page header picture".into(),
        float: craft_wordmodel::para::Float { wrap: craft_wordmodel::para::Wrap::BehindText, ..Default::default() },
        crop: [0.0; 4],
        fx: Default::default(),
        ole: None,
    });
    hp.runs = vec![craft_wordmodel::para::Run { len: hp.text.len(), props: Default::default() }];
    d.parts.insert(
        1,
        craft_wordmodel::Part { kind: craft_wordmodel::PartKind::Header, blocks: vec![craft_wordmodel::para_block(hp)], body: Default::default() },
    );
    d.last_section.headers.default = Some(1);

    let saved = export(&d).unwrap();
    let s = crate::text::tests::entry(&saved, "styles.xml");
    assert!(s.contains("draw:fill=\"bitmap\""), "page layout must declare draw:fill=\"bitmap\": {s}");
    assert!(s.contains("style:background-image"), "legacy background-image fallback must be present: {s}");
    assert!(s.contains("<draw:fill-image"), "draw:fill-image must be declared in office:styles: {s}");

    let fill_name =
        s.split("draw:fill-image-name=\"").nth(1).and_then(|x| x.split('"').next()).expect("styles.xml must contain draw:fill-image-name");
    let draw_fill_pattern = format!("<draw:fill-image draw:name=\"{fill_name}\" xlink:href=\"");
    let draw_href = s.split(&draw_fill_pattern).nth(1).and_then(|x| x.split('"').next()).expect("draw:fill-image must define xlink:href");
    let legacy_href = s
        .split("<style:background-image xlink:href=\"")
        .nth(1)
        .and_then(|x| x.split('"').next())
        .expect("legacy background-image must define xlink:href");
    assert_eq!(draw_href, legacy_href, "modern and legacy background image references must match");

    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(&saved)).unwrap();
    let mut bg_entry = archive.by_name(draw_href).expect("packaged background picture must exist");
    let mut bg_bytes = Vec::new();
    std::io::Read::read_to_end(&mut bg_entry, &mut bg_bytes).unwrap();
    assert_eq!(bg_bytes, bg_png, "packaged background image bytes must match");

    let back = import(&saved).unwrap();
    let bg = back.settings.page_picture.as_ref().expect("page background picture must survive round trip");
    assert_eq!(back.media.get(&bg.media).map(|m| m.as_slice()), Some(bg_png.as_slice()), "imported background bytes must match");
    let header_part = back.last_section.headers.default.and_then(|id| back.parts.get(&id)).expect("header part must survive round trip");
    let header_img_media = header_part.blocks.iter().find_map(|b| {
        b.as_para().and_then(|p| {
            p.objects.iter().find_map(|o| if let craft_wordmodel::InlineObject::Image { media, .. } = o { Some(media.as_str()) } else { None })
        })
    });
    assert!(header_img_media.is_some(), "header part must contain image object");
    let header_img_media = header_img_media.unwrap();
    assert_eq!(back.media.get(header_img_media).map(|m| m.as_slice()), Some(header_png.as_slice()), "imported header image bytes must match");
}

/// A table cell style with a background image survives the kept styles path (#5803).
#[test]
fn cell_style_picture_survives_kept_path() {
    let png = {
        let img = image::RgbImage::from_pixel(8, 4, image::Rgb([200, 30, 30]));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    };
    let ns = crate::text::tests::NS_T;
    let styles = format!(
        r#"<office:document-styles {ns}><office:styles><style:style style:name="CellBg" style:family="table-cell"><style:table-cell-properties><style:background-image xlink:href="Pictures/cell-bg.png" xlink:type="simple" xlink:actuate="onLoad"/></style:table-cell-properties></style:style></office:styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {ns}><office:body><office:text><text:p>Cell test</text:p></office:text></office:body></office:document-content>"#
    );
    let pkg = crate::text::tests::zip_of(&[("content.xml", content.as_bytes()), ("styles.xml", styles.as_bytes()), ("Pictures/cell-bg.png", &png)]);
    let d = import(&pkg).unwrap();
    let saved = export(&d).unwrap();
    let s = crate::text::tests::entry(&saved, "styles.xml");
    assert!(s.contains("style:background-image"), "cell style background-image must survive on kept path: {s}");
    assert!(s.contains("Pictures/cell-bg.png"), "cell background picture path must survive: {s}");
}

/// An ODT document with a text-only header retains its legacy page background image
/// and writes the image into the package archive.
#[test]
fn text_header_keeps_page_background_picture() {
    let png = {
        let img = image::RgbImage::from_pixel(8, 4, image::Rgb([200, 30, 30]));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    };
    let mut d = Document::new();
    let bg_media = d.add_media(png, "png");
    d.settings.page_picture = Some(craft_wordmodel::PagePicture { media: bg_media, fill_type: "frame".into(), title: String::new() });
    let mut hp = craft_wordmodel::Paragraph::new();
    hp.text = "Header text".into();
    hp.runs = vec![craft_wordmodel::para::Run { len: hp.text.len(), props: Default::default() }];
    d.parts.insert(
        1,
        craft_wordmodel::Part { kind: craft_wordmodel::PartKind::Header, blocks: vec![craft_wordmodel::para_block(hp)], body: Default::default() },
    );
    d.last_section.headers.default = Some(1);

    let saved = export(&d).unwrap();
    let s = crate::text::tests::entry(&saved, "styles.xml");
    assert!(s.contains("style:background-image"), "page background must be written when header has no picture: {s}");
    assert!(s.contains("Pictures/page-background.png"));
}

/// Every picture written into the ODT zip archive must be referenced by styles.xml or content.xml (#5803).
#[test]
fn no_unreferenced_images_left_in_package() {
    let png = {
        let img = image::RgbImage::from_pixel(8, 4, image::Rgb([200, 30, 30]));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    };
    let ns = crate::text::tests::NS_T;
    let nsd = r#"xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" xmlns:xlink="http://www.w3.org/1999/xlink""#;
    // An imported ODT with modern draw:fill="bitmap" and draw:fill-image
    let styles = format!(
        r#"<office:document-styles {ns} {nsd}><office:styles><draw:fill-image draw:name="Bg1" xlink:href="Pictures/original.png" xlink:type="simple"/><style:style style:name="Standard" style:family="paragraph"/></office:styles><office:automatic-styles><style:page-layout style:name="pm1"><style:page-layout-properties fo:page-width="8.5in" fo:page-height="11in" draw:fill="bitmap" draw:fill-image-name="Bg1"><style:background-image xlink:href="Pictures/original.png" xlink:type="simple" xlink:actuate="onLoad" style:repeat="stretch"/></style:page-layout-properties></style:page-layout></office:automatic-styles><office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm1"/></office:master-styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {ns}><office:body><office:text><text:p>Body text.</text:p></office:text></office:body></office:document-content>"#
    );
    let pkg = crate::text::tests::zip_of(&[("content.xml", content.as_bytes()), ("styles.xml", styles.as_bytes()), ("Pictures/original.png", &png)]);
    let mut d = import(&pkg).unwrap();

    // Now insert a header picture into the document, changing page setup
    let header_media = d.add_media(png.clone(), "png");
    let mut hp = craft_wordmodel::Paragraph::new();
    hp.text = format!("{}", craft_wordmodel::para::OBJ);
    hp.objects.push(craft_wordmodel::InlineObject::Image {
        media: header_media,
        w: 100.0,
        h: 50.0,
        alt: "Header logo".into(),
        float: Default::default(),
        crop: [0.0; 4],
        fx: Default::default(),
        ole: None,
    });
    hp.runs = vec![craft_wordmodel::para::Run { len: hp.text.len(), props: Default::default() }];
    d.parts.insert(
        1,
        craft_wordmodel::Part { kind: craft_wordmodel::PartKind::Header, blocks: vec![craft_wordmodel::para_block(hp)], body: Default::default() },
    );
    d.last_section.headers.default = Some(1);

    let saved = export(&d).unwrap();
    let styles_xml = crate::text::tests::entry(&saved, "styles.xml");
    let content_xml = crate::text::tests::entry(&saved, "content.xml");

    // Verify modern draw:fill bitmap and legacy style:background-image resolve to the same image target
    let fill_name =
        styles_xml.split("draw:fill-image-name=\"").nth(1).and_then(|s| s.split('"').next()).expect("styles.xml must contain draw:fill-image-name");
    let draw_fill_pattern = format!("<draw:fill-image draw:name=\"{fill_name}\" xlink:href=\"");
    let draw_href = styles_xml.split(&draw_fill_pattern).nth(1).and_then(|s| s.split('"').next()).expect("draw:fill-image must define xlink:href");
    let legacy_href = styles_xml
        .split("<style:background-image xlink:href=\"")
        .nth(1)
        .and_then(|s| s.split('"').next())
        .expect("legacy background-image must define xlink:href");
    assert_eq!(draw_href, legacy_href, "modern and legacy background image references must match");

    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(&saved)).unwrap();
    let file_names: Vec<String> = (0..archive.len()).map(|i| archive.by_index(i).unwrap().name().unwrap().to_string()).collect();

    let pic_files: Vec<&String> = file_names.iter().filter(|n| n.starts_with("Pictures/")).collect();
    assert!(!pic_files.is_empty(), "package should contain picture files");

    for pic in pic_files {
        let is_referenced = styles_xml.contains(pic.as_str()) || content_xml.contains(pic.as_str());
        assert!(is_referenced, "package image {pic} must be referenced in styles.xml or content.xml: styles={styles_xml}");
    }
}

/// An inactive header (e.g. `first` when `title_page` is false, or `even` when `even_odd_headers`
/// is false) containing an image does NOT suppress the page background picture (#5803).
#[test]
fn inactive_header_picture_does_not_suppress_page_background() {
    let png = {
        let img = image::RgbImage::from_pixel(8, 4, image::Rgb([200, 30, 30]));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    };
    let mut d = Document::new();
    let bg_media = d.add_media(png.clone(), "png");
    d.settings.page_picture = Some(craft_wordmodel::PagePicture { media: bg_media, fill_type: "frame".into(), title: String::new() });

    let header_media = d.add_media(png, "png");
    let mut hp = craft_wordmodel::Paragraph::new();
    hp.text = format!("{}", craft_wordmodel::para::OBJ);
    hp.objects.push(craft_wordmodel::InlineObject::Image {
        media: header_media,
        w: 100.0,
        h: 100.0,
        alt: "Inactive header picture".into(),
        float: Default::default(),
        crop: [0.0; 4],
        fx: Default::default(),
        ole: None,
    });
    hp.runs = vec![craft_wordmodel::para::Run { len: hp.text.len(), props: Default::default() }];
    d.parts.insert(
        100,
        craft_wordmodel::Part { kind: craft_wordmodel::PartKind::Header, blocks: vec![craft_wordmodel::para_block(hp)], body: Default::default() },
    );

    // Point headers.first to the header with picture, but title_page is FALSE.
    d.last_section.headers.first = Some(100);
    d.last_section.title_page = false;

    let saved = export(&d).unwrap();
    let s = crate::text::tests::entry(&saved, "styles.xml");
    assert!(s.contains("style:background-image"), "background-image must be preserved when header picture is in an inactive header: {s}");
    assert!(s.contains("Pictures/page-background.png"));
}

/// A noninitial continuous section header containing a picture does NOT produce a master page
/// and does not suppress the page background (#5803).
#[test]
fn continuous_section_header_picture_does_not_suppress_page_background() {
    let png = {
        let img = image::RgbImage::from_pixel(8, 4, image::Rgb([200, 30, 30]));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    };
    let mut d = Document::new();
    let bg_media = d.add_media(png.clone(), "png");
    d.settings.page_picture = Some(craft_wordmodel::PagePicture { media: bg_media, fill_type: "frame".into(), title: String::new() });

    // Body with a first section ending paragraph
    let mut p1 = craft_wordmodel::Paragraph::new();
    p1.text = "First paragraph in section 1".into();
    p1.runs = vec![craft_wordmodel::para::Run { len: p1.text.len(), props: Default::default() }];
    p1.section = Some(Box::new(craft_wordmodel::SectionProps::default()));
    d.body.push(craft_wordmodel::para_block(p1));

    // Second section (last_section) is continuous
    let header_media = d.add_media(png, "png");
    let mut hp = craft_wordmodel::Paragraph::new();
    hp.text = format!("{}", craft_wordmodel::para::OBJ);
    hp.objects.push(craft_wordmodel::InlineObject::Image {
        media: header_media,
        w: 100.0,
        h: 100.0,
        alt: "Continuous section header picture".into(),
        float: Default::default(),
        crop: [0.0; 4],
        fx: Default::default(),
        ole: None,
    });
    hp.runs = vec![craft_wordmodel::para::Run { len: hp.text.len(), props: Default::default() }];
    d.parts.insert(
        200,
        craft_wordmodel::Part { kind: craft_wordmodel::PartKind::Header, blocks: vec![craft_wordmodel::para_block(hp)], body: Default::default() },
    );

    d.last_section.start = craft_wordmodel::section::SectionStart::Continuous;
    d.last_section.headers.default = Some(200);

    let saved = export(&d).unwrap();
    let s = crate::text::tests::entry(&saved, "styles.xml");
    assert!(
        s.contains("style:background-image"),
        "page background must not be suppressed by continuous section header that emits no master page: {s}"
    );
    assert!(s.contains("Pictures/page-background.png"));
}

/// A non-continuous `last_section` without any body blocks following the section break
/// does not produce an exported page and does not suppress the page background (#5803).
#[test]
fn trailing_empty_last_section_header_picture_does_not_suppress_page_background() {
    let png = {
        let img = image::RgbImage::from_pixel(8, 4, image::Rgb([200, 30, 30]));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    };
    let mut d = Document::new();
    let bg_media = d.add_media(png.clone(), "png");
    d.settings.page_picture = Some(craft_wordmodel::PagePicture { media: bg_media, fill_type: "frame".into(), title: String::new() });

    // Body with a single paragraph ending section 1 (nothing follows it)
    let mut p1 = craft_wordmodel::Paragraph::new();
    p1.text = "Sole paragraph ending section 1".into();
    p1.runs = vec![craft_wordmodel::para::Run { len: p1.text.len(), props: Default::default() }];
    p1.section = Some(Box::new(craft_wordmodel::SectionProps::default()));
    d.body.push(craft_wordmodel::para_block(p1));

    // last_section is NextPage and has a header with a picture, but has no blocks following
    let header_media = d.add_media(png, "png");
    let mut hp = craft_wordmodel::Paragraph::new();
    hp.text = format!("{}", craft_wordmodel::para::OBJ);
    hp.objects.push(craft_wordmodel::InlineObject::Image {
        media: header_media,
        w: 100.0,
        h: 100.0,
        alt: "Unused trailing section header picture".into(),
        float: Default::default(),
        crop: [0.0; 4],
        fx: Default::default(),
        ole: None,
    });
    hp.runs = vec![craft_wordmodel::para::Run { len: hp.text.len(), props: Default::default() }];
    d.parts.insert(
        300,
        craft_wordmodel::Part { kind: craft_wordmodel::PartKind::Header, blocks: vec![craft_wordmodel::para_block(hp)], body: Default::default() },
    );

    d.last_section.start = craft_wordmodel::section::SectionStart::NextPage;
    d.last_section.headers.default = Some(300);

    let saved = export(&d).unwrap();
    let s = crate::text::tests::entry(&saved, "styles.xml");
    assert!(s.contains("style:background-image"), "page background must not be suppressed by an unexported trailing last_section header: {s}");
    assert!(s.contains("Pictures/page-background.png"));
}

/// When a page background picture is replaced with a new image while retaining an existing
/// fill-image name that belongs to a graphic style, export allocates a distinct fill name,
/// references it in page layouts, and preserves the retained definition and image (#5803).
#[test]
fn replaced_page_picture_allocates_distinct_fill_name_and_keeps_retained_def() {
    let orig_png = {
        let img = image::RgbImage::from_pixel(8, 4, image::Rgb([100, 200, 100]));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    };
    let new_png = {
        let img = image::RgbImage::from_pixel(4, 8, image::Rgb([200, 100, 200]));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    };
    let ns = crate::text::tests::NS_T;
    let nsd = "xmlns:draw=\"urn:oasis:names:tc:opendocument:xmlns:drawing:1.0\"";
    let styles = format!(
        r#"<office:document-styles {ns} {nsd}><office:styles><draw:fill-image draw:name="SharedBg" xlink:href="Pictures/original.png" xlink:type="simple"/><style:style style:name="Graphic1" style:family="graphic"><style:graphic-properties draw:fill="bitmap" draw:fill-image-name="SharedBg"/></style:style></office:styles><office:automatic-styles><style:page-layout style:name="pm1"><style:page-layout-properties fo:page-width="8.5in" fo:page-height="11in" draw:fill="bitmap" draw:fill-image-name="SharedBg"><style:background-image xlink:href="Pictures/original.png" xlink:type="simple" xlink:actuate="onLoad" style:repeat="stretch"/></style:page-layout-properties></style:page-layout></office:automatic-styles><office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm1"/></office:master-styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {ns} {nsd}><office:body><office:text><text:p>Body text</text:p></office:text></office:body></office:document-content>"#
    );
    let pkg =
        crate::text::tests::zip_of(&[("content.xml", content.as_bytes()), ("styles.xml", styles.as_bytes()), ("Pictures/original.png", &orig_png)]);
    let mut d = import(&pkg).unwrap();
    let new_media = d.add_media(new_png.clone(), "png");
    d.settings.page_picture = Some(craft_wordmodel::PagePicture { media: new_media, fill_type: "stretch".into(), title: "SharedBg".into() });

    let saved = export(&d).unwrap();
    let s = crate::text::tests::entry(&saved, "styles.xml");
    assert!(s.contains(r#"<draw:fill-image draw:name="SharedBg" xlink:href="Pictures/original.png""#), "original fill-image must be preserved: {s}");
    assert!(s.contains(r#"<draw:fill-image draw:name="SharedBg-1""#), "distinct fill-image definition must be allocated: {s}");
    assert!(s.contains(r#"draw:fill-image-name="SharedBg-1""#), "page layout must reference distinct fill-image name: {s}");

    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(&saved)).unwrap();
    assert!(archive.by_name("Pictures/original.png").is_ok(), "Pictures/original.png must be in package");
    assert!(archive.by_name("Pictures/page-background.png").is_ok(), "Pictures/page-background.png must be in package");
}

/// The pictures in package `bytes` under `Pictures/`, sorted.
fn picture_paths(bytes: &[u8]) -> Vec<String> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut out: Vec<String> =
        (0..archive.len()).map(|i| archive.by_index(i).unwrap().name().unwrap().to_string()).filter(|n| n.starts_with("Pictures/")).collect();
    out.sort();
    out
}

/// Saving a page background picture and opening it again keeps one picture and one fill-image
/// definition: the kept file is reused while its bytes are the document's, not renamed (#5856).
#[test]
fn page_background_round_trips_do_not_grow_the_package() {
    let png = {
        let img = image::RgbImage::from_pixel(8, 4, image::Rgb([200, 30, 30]));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    };
    let mut d = Document::new();
    let media = d.add_media(png, "png");
    d.settings.page_picture = Some(craft_wordmodel::PagePicture { media, fill_type: "frame".into(), title: "PageBackground".into() });
    let mut current = d;
    for generation in 1..=4 {
        let saved = export(&current).unwrap();
        let styles = crate::text::tests::entry(&saved, "styles.xml");
        assert_eq!(styles.matches("<draw:fill-image ").count(), 1, "generation {generation} must keep one fill image: {styles}");
        assert_eq!(picture_paths(&saved), ["Pictures/page-background.png"], "generation {generation} must keep one picture");
        current = import(&saved).unwrap();
    }
}

/// An untouched LibreOffice page background (fill image `Bg1` in `Pictures/original.png`) is
/// written back as it was: one definition, one file, no renamed copies (#5856).
#[test]
fn an_unchanged_libreoffice_page_background_is_written_back_once() {
    let png = {
        let img = image::RgbImage::from_pixel(8, 4, image::Rgb([200, 30, 30]));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    };
    let ns = crate::text::tests::NS_T;
    let nsd = r#"xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" xmlns:xlink="http://www.w3.org/1999/xlink""#;
    let styles = format!(
        r#"<office:document-styles {ns} {nsd}><office:styles><draw:fill-image draw:name="Bg1" xlink:href="Pictures/original.png" xlink:type="simple"/><style:style style:name="Standard" style:family="paragraph"/></office:styles><office:automatic-styles><style:page-layout style:name="pm1"><style:page-layout-properties fo:page-width="8.5in" fo:page-height="11in" draw:fill="bitmap" draw:fill-image-name="Bg1"><style:background-image xlink:href="Pictures/original.png" xlink:type="simple" xlink:actuate="onLoad" style:repeat="stretch"/></style:page-layout-properties></style:page-layout></office:automatic-styles><office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm1"/></office:master-styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {ns}><office:body><office:text><text:p>Body text.</text:p></office:text></office:body></office:document-content>"#
    );
    let pkg = crate::text::tests::zip_of(&[("content.xml", content.as_bytes()), ("styles.xml", styles.as_bytes()), ("Pictures/original.png", &png)]);
    let d = import(&pkg).unwrap();
    let saved = export(&d).unwrap();
    let s = crate::text::tests::entry(&saved, "styles.xml");
    assert_eq!(s.matches("<draw:fill-image ").count(), 1, "one fill image, as read: {s}");
    assert!(s.contains(r#"<draw:fill-image draw:name="Bg1" xlink:href="Pictures/original.png""#), "the file's fill image is reused: {s}");
    assert!(!s.contains("page-background"), "no second picture is named for the background: {s}");
    assert_eq!(picture_paths(&saved), ["Pictures/original.png"], "one picture, as read");
}

/// A kept paragraph style both sides name (`Standard`) keeps the picture its background refers
/// to: when styles are written as read, the file's definition wins and its file must stay
/// (#5856).
#[test]
fn common_style_background_picture_survives_styles_as_read() {
    let png = {
        let img = image::RgbImage::from_pixel(8, 4, image::Rgb([200, 30, 30]));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    };
    let ns = crate::text::tests::NS_T;
    let nsd = "xmlns:xlink=\"http://www.w3.org/1999/xlink\"";
    let styles = format!(
        r#"<office:document-styles {ns} {nsd}><office:styles><style:style style:name="Standard" style:family="paragraph"><style:paragraph-properties><style:background-image xlink:href="Pictures/para.png" xlink:type="simple" xlink:actuate="onLoad"/></style:paragraph-properties></style:style></office:styles><office:automatic-styles><style:page-layout style:name="pm1"><style:page-layout-properties fo:page-width="8.5in" fo:page-height="11in"/></style:page-layout></office:automatic-styles><office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm1"/></office:master-styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {ns}><office:body><office:text><text:p>Body text.</text:p></office:text></office:body></office:document-content>"#
    );
    let pkg = crate::text::tests::zip_of(&[("content.xml", content.as_bytes()), ("styles.xml", styles.as_bytes()), ("Pictures/para.png", &png)]);
    let d = import(&pkg).unwrap();
    let saved = export(&d).unwrap();
    let s = crate::text::tests::entry(&saved, "styles.xml");
    assert!(s.contains("Pictures/para.png"), "the style's background reference must stay: {s}");
    assert_eq!(picture_paths(&saved), ["Pictures/para.png"], "the referenced picture must be written");
}

/// A DOCX texture name (`o:title`, "Blue tissue paper") is not an XML NCName: the fill image it
/// names is written with one (#5856).
#[test]
fn a_texture_name_is_written_as_an_ncname() {
    let png = {
        let img = image::RgbImage::from_pixel(8, 4, image::Rgb([200, 30, 30]));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    };
    let mut d = Document::new();
    let media = d.add_media(png, "png");
    d.settings.page_picture = Some(craft_wordmodel::PagePicture { media, fill_type: "frame".into(), title: "Blue tissue paper".into() });
    let saved = export(&d).unwrap();
    let s = crate::text::tests::entry(&saved, "styles.xml");
    assert!(s.contains(r#"<draw:fill-image draw:name="Blue_tissue_paper""#), "the fill name is an NCName: {s}");
    assert!(s.contains(r#"draw:fill-image-name="Blue_tissue_paper""#), "the page layout names it: {s}");
    let back = import(&saved).unwrap();
    assert_eq!(back.settings.page_picture.as_ref().map(|p| p.title.as_str()), Some("Blue_tissue_paper"));
}

/// Fill-image names a hostile file repeats with suffixes cost one lookup each, not a scan of
/// every definition per attempt (#5856).
#[test]
fn many_colliding_fill_image_names_export_in_linear_time() {
    let bg = {
        let img = image::RgbImage::from_pixel(8, 4, image::Rgb([200, 30, 30]));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    };
    let other = {
        let img = image::RgbImage::from_pixel(4, 4, image::Rgb([30, 200, 30]));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    };
    let replacement = {
        let img = image::RgbImage::from_pixel(2, 6, image::Rgb([30, 30, 200]));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    };
    const N: usize = 12_000;
    let ns = crate::text::tests::NS_T;
    let nsd = r#"xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" xmlns:xlink="http://www.w3.org/1999/xlink""#;
    let mut defs = String::with_capacity(N * 90);
    for i in 0..N {
        let name = if i == 0 { "X".to_string() } else { format!("X-{i}") };
        let href = if i == 0 { "Pictures/bg.png" } else { "Pictures/other.png" };
        defs.push_str(&format!(r#"<draw:fill-image draw:name="{name}" xlink:href="{href}" xlink:type="simple"/>"#));
    }
    let styles = format!(
        r#"<office:document-styles {ns} {nsd}><office:styles>{defs}</office:styles><office:automatic-styles><style:page-layout style:name="pm1"><style:page-layout-properties fo:page-width="8.5in" fo:page-height="11in" draw:fill="bitmap" draw:fill-image-name="X"><style:background-image xlink:href="Pictures/bg.png" xlink:type="simple" xlink:actuate="onLoad" style:repeat="stretch"/></style:page-layout-properties></style:page-layout></office:automatic-styles><office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm1"/></office:master-styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {ns}><office:body><office:text><text:p>Body text.</text:p></office:text></office:body></office:document-content>"#
    );
    let pkg = crate::text::tests::zip_of(&[
        ("content.xml", content.as_bytes()),
        ("styles.xml", styles.as_bytes()),
        ("Pictures/bg.png", &bg),
        ("Pictures/other.png", &other),
    ]);
    let mut d = import(&pkg).unwrap();
    // The user replaces the picture: its fill-image name now meets every kept name.
    let media = d.add_media(replacement, "png");
    d.settings.page_picture = Some(craft_wordmodel::PagePicture { media, fill_type: "stretch".into(), title: "X".into() });
    let start = std::time::Instant::now();
    let saved = export(&d).unwrap();
    assert!(start.elapsed() < std::time::Duration::from_secs(5), "export took {:?}", start.elapsed());
    let s = crate::text::tests::entry(&saved, "styles.xml");
    assert!(s.contains(r#"<draw:fill-image draw:name="X-12000""#), "a free fill-image name is allocated: {s}");
}

/// When modern `draw:fill="bitmap"` references a picture missing from the package,
/// import falls back to legacy `<style:background-image>` if present and valid (#1784).
#[test]
fn legacy_background_fallback_when_modern_bitmap_missing() {
    let legacy_png = {
        let img = image::RgbImage::from_pixel(8, 4, image::Rgb([10, 150, 20]));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    };
    let ns = crate::text::tests::NS_T;
    let nsd = r#"xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" xmlns:xlink="http://www.w3.org/1999/xlink""#;
    let styles = format!(
        r#"<office:document-styles {ns} {nsd}><office:styles><draw:fill-image draw:name="MissingBg" xlink:href="Pictures/missing.png" xlink:type="simple"/><style:style style:name="Standard" style:family="paragraph"/></office:styles><office:automatic-styles><style:page-layout style:name="pm1"><style:page-layout-properties fo:page-width="8.5in" fo:page-height="11in" draw:fill="bitmap" draw:fill-image-name="MissingBg"><style:background-image xlink:href="Pictures/legacy.png" xlink:type="simple" xlink:actuate="onLoad" style:repeat="stretch"/></style:page-layout-properties></style:page-layout></office:automatic-styles><office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm1"/></office:master-styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {ns}><office:body><office:text><text:p>Body text.</text:p></office:text></office:body></office:document-content>"#
    );
    let pkg =
        crate::text::tests::zip_of(&[("content.xml", content.as_bytes()), ("styles.xml", styles.as_bytes()), ("Pictures/legacy.png", &legacy_png)]);
    let d = import(&pkg).unwrap();
    let pic = d.settings.page_picture.as_ref().expect("legacy fallback background picture must be imported");
    assert_eq!(d.media.get(&pic.media).map(|b| b.as_slice()), Some(legacy_png.as_slice()));
    assert_eq!(pic.fill_type, "frame");
}

/// When modern `draw:fill="bitmap"` references a file with non-image bytes,
/// import falls back to legacy `<style:background-image>` if present and valid (#1784).
#[test]
fn legacy_background_fallback_when_modern_bitmap_non_image() {
    let legacy_png = {
        let img = image::RgbImage::from_pixel(8, 4, image::Rgb([10, 150, 20]));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    };
    let corrupt_data = b"This is plain text and definitely not a valid image format.";
    let ns = crate::text::tests::NS_T;
    let nsd = r#"xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" xmlns:xlink="http://www.w3.org/1999/xlink""#;
    let styles = format!(
        r#"<office:document-styles {ns} {nsd}><office:styles><draw:fill-image draw:name="CorruptBg" xlink:href="Pictures/corrupt.bin" xlink:type="simple"/><style:style style:name="Standard" style:family="paragraph"/></office:styles><office:automatic-styles><style:page-layout style:name="pm1"><style:page-layout-properties fo:page-width="8.5in" fo:page-height="11in" draw:fill="bitmap" draw:fill-image-name="CorruptBg"><style:background-image xlink:href="Pictures/legacy.png" xlink:type="simple" xlink:actuate="onLoad" style:repeat="stretch"/></style:page-layout-properties></style:page-layout></office:automatic-styles><office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm1"/></office:master-styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {ns}><office:body><office:text><text:p>Body text.</text:p></office:text></office:body></office:document-content>"#
    );
    let pkg = crate::text::tests::zip_of(&[
        ("content.xml", content.as_bytes()),
        ("styles.xml", styles.as_bytes()),
        ("Pictures/corrupt.bin", corrupt_data),
        ("Pictures/legacy.png", &legacy_png),
    ]);
    let d = import(&pkg).unwrap();
    let pic = d.settings.page_picture.as_ref().expect("legacy fallback background picture must be imported when modern is non-image");
    assert_eq!(d.media.get(&pic.media).map(|b| b.as_slice()), Some(legacy_png.as_slice()));
    assert_eq!(pic.fill_type, "frame");
}
