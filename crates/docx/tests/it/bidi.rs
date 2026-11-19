//! Right-to-left and complex-script OOXML: `w:bidi` paragraphs, `w:rtl` / `w:cs` runs, the
//! complex-script font, size, bold and italic, and the bidi language. Fixtures are hand-written
//! from ECMA-376 Part 1 §17.3.1.6 (bidi), §17.3.2 (run properties).

use std::io::{Read, Write};

use craft_wordmodel::props::{Align, CharProps, ParaProps};
use craft_wordmodel::{Document, Paragraph, para_block};

fn zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, bytes) in entries {
        zw.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
        zw.write_all(bytes).unwrap();
    }
    zw.finish().unwrap().into_inner()
}

/// A package whose body is `body`.
fn docx(body: &str) -> Vec<u8> {
    let root = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    let doc = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{body}</w:body></w:document>"#
    );
    zip(&[("_rels/.rels", root.as_bytes()), ("word/document.xml", doc.as_bytes())])
}

fn paras(d: &Document) -> Vec<&Paragraph> {
    d.body.iter().filter_map(|b| b.as_para()).collect()
}

/// `word/document.xml` of a written package.
fn document_xml(bytes: &[u8]) -> String {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut s = String::new();
    z.by_name("word/document.xml").unwrap().read_to_string(&mut s).unwrap();
    s
}

/// Persian text with a ZWNJ (نیم‌فاصله), Persian digits, Latin and punctuation.
const FA: &str = "می‌خواهم نسخهٔ ۲٫۵ از WordCraft را (امروز) نصب کنم.";

fn persian_run() -> CharProps {
    CharProps {
        rtl: Some(true),
        font_cs: Some("B Nazanin".into()),
        size_cs: Some(14.0),
        bold_cs: Some(true),
        italic_cs: Some(false),
        lang_bidi: Some("fa-IR".into()),
        ..Default::default()
    }
}

#[test]
fn persian_paragraph_round_trips() {
    let mut rtl = Paragraph::with_text(FA, persian_run());
    rtl.props = ParaProps { bidi: Some(true), align: Some(Align::Justify), ..Default::default() };
    let ltr = Paragraph::with_text("English with فارسی inside", CharProps { cs: Some(true), ..Default::default() });
    let mut d = Document::new();
    d.body = vec![para_block(rtl.clone()), para_block(ltr.clone())];
    let bytes = craft_docx::write(&d).unwrap();
    let r = craft_docx::read(&bytes).unwrap();
    let ps = paras(&r);
    assert_eq!(ps[0].text, FA, "Persian text, ZWNJ and digits survive byte for byte");
    assert_eq!(ps[0].props.bidi, Some(true));
    assert_eq!(ps[0].props.align, Some(Align::Justify));
    assert_eq!(ps[0].runs[0].props, persian_run());
    assert_eq!(ps[1].text, ltr.text);
    assert_eq!(ps[1].props.bidi, None);
    assert_eq!(ps[1].runs[0].props.cs, Some(true));
}

#[test]
fn bidi_markup_is_written_in_schema_order() {
    let mut p =
        Paragraph::with_text(FA, CharProps { bold: Some(true), italic: Some(true), size: Some(12.0), font: Some("Arial".into()), ..persian_run() });
    p.props.bidi = Some(true);
    let mut d = Document::new();
    d.body = vec![para_block(p)];
    let xml = document_xml(&craft_docx::write(&d).unwrap());
    assert!(xml.contains("<w:bidi/>"), "paragraph direction: {xml}");
    assert!(xml.contains(r#"w:cs="B Nazanin""#), "complex-script font");
    assert!(xml.contains(r#"<w:szCs w:val="28"/>"#), "complex-script size in half points");
    assert!(xml.contains(r#"w:bidi="fa-IR""#), "bidi language");
    // CT_RPr is a sequence: b, bCs, i, iCs … sz, szCs … rtl, cs, lang.
    let pos = |t: &str| xml.find(t).unwrap_or_else(|| panic!("{t} missing: {xml}"));
    assert!(pos("<w:rFonts") < pos("<w:b/>"));
    assert!(pos("<w:b/>") < pos("<w:bCs/>") && pos("<w:bCs/>") < pos("<w:i/>"));
    assert!(pos("<w:i/>") < pos(r#"<w:iCs w:val="0"/>"#));
    assert!(pos(r#"<w:sz w:val="24"/>"#) < pos("<w:szCs"));
    assert!(pos("<w:rtl/>") < pos("<w:lang"));
}

/// Thai text WordCraft draws in the complex-script formatting is written `w:cs`, since Word draws
/// Thai outside a complex-script run in the run's other font and `w:sz` (measured in Word for
/// Windows). Only the Thai is marked: spaces and Latin text keep the run's own formatting.
#[test]
fn thai_text_is_written_complex_script_where_its_formatting_differs() {
    let text = "ทดสอบ ภาษาไทย ABC 123";
    let drawn = |d: &Document| -> Vec<(String, f32, bool)> {
        let p = paras(d)[0];
        let mut out = Vec::new();
        let mut at = 0;
        for r in &p.runs {
            let rc = d.styles.resolve_char(p.props.style.as_deref(), &r.props);
            for ch in p.text[at..at + r.len].chars() {
                let c = if rc.uses_complex(ch) { rc.complex() } else { rc.clone() };
                out.push((c.font, c.size, c.bold));
            }
            at += r.len;
        }
        out
    };
    let doc = |defaults: CharProps, run: CharProps| {
        let mut d = Document::new();
        std::sync::Arc::make_mut(&mut d.styles).default_chr = defaults;
        d.body = vec![para_block(Paragraph::with_text(text, run))];
        d
    };
    let differing = CharProps {
        font: Some("Angsana New".into()),
        font_cs: Some("Tahoma".into()),
        size: Some(22.0),
        size_cs: Some(11.0),
        bold_cs: Some(true),
        ..Default::default()
    };
    let d = doc(differing.clone(), CharProps::default());
    let bytes = craft_docx::write(&d).unwrap();
    let xml = document_xml(&bytes);
    assert_eq!(xml.matches("<w:cs/>").count(), 2, "the two Thai words, not the space, Latin or digits: {xml}");
    let again = craft_docx::read(&bytes).unwrap();
    assert_eq!(paras(&again)[0].text, text);
    assert_eq!(drawn(&again), drawn(&d), "drawn the same after a save");
    let thai = drawn(&again)[0].clone();
    assert_eq!(thai, ("Tahoma".to_string(), 11.0, true));

    // A tracked formatting change gets the same mark in its "before" copy.
    let tracked = CharProps {
        fmt_change: craft_wordmodel::props::PropChange::boxed(1, CharProps { italic: Some(true), ..Default::default() }),
        ..Default::default()
    };
    let xml = document_xml(&craft_docx::write(&doc(differing.clone(), tracked)).unwrap());
    assert_eq!(xml.matches("<w:cs/>").count(), 4, "{xml}");

    // Formatting that is the same either way, and runs that say whether they are complex
    // script, are written as they are.
    let same = CharProps { font: Some("TH Sarabun New".into()), font_cs: Some("TH Sarabun New".into()), size: Some(16.0), ..Default::default() };
    for (name, d) in [
        ("same formatting", doc(same, CharProps::default())),
        ("run cs off", doc(differing.clone(), CharProps { cs: Some(false), ..Default::default() })),
    ] {
        let xml = document_xml(&craft_docx::write(&d).unwrap());
        assert!(!xml.contains("<w:cs/>"), "{name}: {xml}");
        assert_eq!(xml.matches("<w:r>").count() + xml.matches("<w:r ").count(), 1, "{name}: one run: {xml}");
    }
}

/// What Word counts for a run being complex script, measured in Word for Windows: its table
/// style's `w:cs` and run formatting do count, a character style's `w:cs` (or `w:cs w:val="0"`)
/// doesn't, for Latin and Thai text alike. Text is written so Word draws it as WordCraft does.
#[test]
fn text_is_written_complex_script_by_what_word_counts() {
    use craft_wordmodel::{Block, Cell, Row, Style, StyleKind, Table};
    use std::sync::Arc;
    let text = "ทดสอบ ABC";
    // Each character's font, size and bold as WordCraft draws it, a table cell's with the table
    // style's run formatting.
    let drawn = |d: &Document| -> Vec<(String, f32, bool)> {
        let (p, table) = match &*d.body[0] {
            Block::Table(t) => (t.rows[0].cells[0].blocks[0].as_para().unwrap(), d.styles.table_cells_chr(t).map(|c| c[0][0].clone())),
            b => (b.as_para().unwrap(), None),
        };
        let mut out = Vec::new();
        for (range, props) in p.run_ranges() {
            let rc = d.styles.resolve_char_in(p.props.style.as_deref(), table.as_ref(), props);
            for ch in p.text[range].chars() {
                let c = if rc.uses_complex(ch) { rc.complex() } else { rc.clone() };
                out.push((c.font, c.size, c.bold));
            }
        }
        out
    };
    let doc = |defaults: CharProps, styles: Vec<Style>, run: CharProps, table: Option<&str>| {
        let mut d = Document::new();
        let sheet = Arc::make_mut(&mut d.styles);
        sheet.default_chr = defaults;
        for s in styles {
            sheet.upsert(s);
        }
        let para = para_block(Paragraph::with_text(text, run));
        d.body = vec![match table {
            Some(id) => {
                let mut t = Table { grid: vec![300.0], ..Default::default() };
                t.props.style = Some(id.into());
                let cell = Cell {
                    props: craft_wordmodel::props::CellProps { span: 1, ..Default::default() },
                    blocks: vec![para],
                    controls: Default::default(),
                };
                t.rows = vec![Row { props: Default::default(), cells: vec![cell], controls: Default::default() }];
                Arc::new(Block::Table(t))
            }
            None => para,
        }];
        d
    };
    let style = |id: &str, kind: StyleKind, chr: CharProps| Style { id: id.into(), name: id.into(), kind, chr, ..Default::default() };
    let tahoma = CharProps { font: Some("Tahoma".into()), font_cs: Some("Tahoma".into()), ..Default::default() };
    let differing =
        CharProps { font: Some("Angsana New".into()), font_cs: Some("Tahoma".into()), size: Some(10.0), size_cs: Some(20.0), ..Default::default() };
    let cs_on = CharProps { cs: Some(true), ..Default::default() };
    let in_char_style = |id: &str| CharProps { style: Some(id.into()), ..Default::default() };
    let cases = [
        // A table style's sizes make the two formattings differ: the Thai word is marked.
        (
            "table style sizes",
            doc(
                tahoma,
                vec![style("T", StyleKind::Table, CharProps { size: Some(20.0), size_cs: Some(10.0), ..Default::default() })],
                CharProps::default(),
                Some("T"),
            ),
            1,
        ),
        // A table style's `w:cs` makes the run complex script for Word too: nothing to mark.
        ("table style cs", doc(differing.clone(), vec![style("T", StyleKind::Table, cs_on.clone())], CharProps::default(), Some("T")), 0),
        // A character style's `w:cs` doesn't count in Word: the whole run, Latin too, is marked.
        ("character style cs", doc(differing.clone(), vec![style("CsOn", StyleKind::Character, cs_on.clone())], in_char_style("CsOn"), None), 1),
    ];
    for (name, d, marks) in cases {
        let bytes = craft_docx::write(&d).unwrap();
        let xml = document_xml(&bytes);
        assert_eq!(xml.matches("<w:cs/>").count(), marks, "{name}: {xml}");
        assert_eq!(drawn(&craft_docx::read(&bytes).unwrap()), drawn(&d), "{name}: drawn the same after a save");
    }
    // A character style's `w:cs w:val="0"` doesn't turn off its paragraph style's `w:cs` in Word:
    // the text WordCraft draws in the run's other formatting is written `w:cs w:val="0"`.
    let normal = Style { id: "Normal".into(), name: "Normal".into(), kind: StyleKind::Paragraph, chr: cs_on, ..Default::default() };
    let cs_off = style("CsOff", StyleKind::Character, CharProps { cs: Some(false), ..Default::default() });
    let d = doc(differing, vec![normal, cs_off], in_char_style("CsOff"), None);
    let bytes = craft_docx::write(&d).unwrap();
    let xml = document_xml(&bytes);
    assert!(xml.contains(r#"<w:cs w:val="0"/>"#), "{xml}");
    assert_eq!(drawn(&craft_docx::read(&bytes).unwrap()), drawn(&d), "drawn the same after a save");
}

#[test]
fn reads_word_style_bidi_markup() {
    // What a right-to-left paragraph looks like in a Word document: w:bidi, logical
    // alignment and indents (left = start), runs with w:rtl and the complex-script values.
    let body = r#"
<w:p>
  <w:pPr><w:bidi/><w:ind w:left="720" w:right="0"/><w:jc w:val="left"/></w:pPr>
  <w:r><w:rPr><w:rFonts w:ascii="Times New Roman" w:hAnsi="Times New Roman" w:cs="B Lotus"/><w:b/><w:bCs/><w:sz w:val="24"/><w:szCs w:val="32"/><w:rtl/><w:lang w:val="en-US" w:bidi="fa-IR"/></w:rPr><w:t>سلام دنیا</w:t></w:r>
  <w:r><w:rPr><w:rtl/></w:rPr><w:t xml:space="preserve"> </w:t></w:r>
  <w:r><w:t>Word 2025</w:t></w:r>
</w:p>
<w:p><w:pPr><w:bidi w:val="0"/></w:pPr><w:r><w:rPr><w:cs/><w:iCs/></w:rPr><w:t>x</w:t></w:r></w:p>"#;
    let d = craft_docx::read(&docx(body)).unwrap();
    let ps = paras(&d);
    assert_eq!(ps[0].text, "سلام دنیا Word 2025");
    assert_eq!(ps[0].props.bidi, Some(true));
    assert_eq!(ps[0].props.indent_left, Some(36.0), "w:left is the start indent");
    assert_eq!(ps[0].props.align, Some(Align::Left), "jc=left is the start edge");
    let fa = &ps[0].runs[0].props;
    assert_eq!(fa.font.as_deref(), Some("Times New Roman"));
    assert_eq!(fa.font_cs.as_deref(), Some("B Lotus"));
    assert_eq!((fa.size, fa.size_cs), (Some(12.0), Some(16.0)));
    // Both explicit values survive, even when they happen to be equal.
    assert_eq!((fa.bold, fa.bold_cs, fa.rtl), (Some(true), Some(true), Some(true)));
    assert_eq!((fa.lang.as_deref(), fa.lang_bidi.as_deref()), (Some("en-US"), Some("fa-IR")));
    assert_eq!(ps[1].props.bidi, Some(false));
    let x = &ps[1].runs[0].props;
    assert_eq!((x.cs, x.italic_cs, x.italic), (Some(true), Some(true), None));
    // The resolved complex-script formatting is what Persian characters are drawn with.
    let rc = d.styles.resolve_char(None, fa);
    let cs = rc.complex();
    assert_eq!((cs.font.as_str(), cs.size, cs.bold), ("B Lotus", 16.0, true));
    assert!(rc.uses_complex('س') && rc.uses_complex('W'), "every character of an rtl run is complex script");
}

#[test]
fn hostile_bidi_markup_is_bounded() {
    let long = "x".repeat(10_000);
    let body = format!(
        r#"<w:p><w:pPr><w:bidi w:val="banana"/></w:pPr><w:r><w:rPr><w:rFonts w:cs="{long}"/><w:szCs w:val="-5"/><w:szCs w:val="99999999"/><w:lang w:bidi="{long}"/><w:rtl w:val="maybe"/></w:rPr><w:t>ok</w:t></w:r></w:p>"#
    );
    let d = craft_docx::read(&docx(&body)).unwrap();
    let c = &paras(&d)[0].runs[0].props;
    assert_eq!(c.font_cs, None, "absurd font names are dropped");
    assert_eq!(c.lang_bidi, None);
    assert!(c.size_cs.is_none_or(|s| (1.0..=1638.0).contains(&s)));
}

/// Right-to-left tables (`w:tblPr/w:bidiVisual`, ECMA-376 Part 1 §17.4.1) and sections
/// (`w:sectPr/w:bidi`, §17.6.1) read, write in schema order and round-trip (#362).
#[test]
fn rtl_tables_and_sections_round_trip() {
    let body = r#"
<w:tbl>
  <w:tblPr><w:bidiVisual/><w:tblW w:w="0" w:type="auto"/><w:tblInd w:w="360" w:type="dxa"/></w:tblPr>
  <w:tblGrid><w:gridCol w:w="2000"/><w:gridCol w:w="2000"/></w:tblGrid>
  <w:tr><w:tc><w:p><w:r><w:t>اول</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>دوم</w:t></w:r></w:p></w:tc></w:tr>
</w:tbl>
<w:tbl>
  <w:tblPr><w:bidiVisual w:val="0"/></w:tblPr>
  <w:tblGrid><w:gridCol w:w="2000"/></w:tblGrid>
  <w:tr><w:tc><w:p/></w:tc></w:tr>
</w:tbl>
<w:p/>
<w:sectPr><w:cols w:num="2" w:space="720"/><w:bidi/></w:sectPr>"#;
    let d = craft_docx::read(&docx(body)).unwrap();
    let tables: Vec<&craft_wordmodel::Table> = d.body.iter().filter_map(|b| b.as_table()).collect();
    assert_eq!(tables.len(), 2);
    assert!(tables[0].props.bidi_visual, "w:bidiVisual reads as a right-to-left table");
    assert_eq!(tables[0].props.indent, Some(18.0), "the indent is kept (measured from the right)");
    assert!(!tables[1].props.bidi_visual, "w:val=0 turns it off");
    assert!(d.last_section.rtl, "w:bidi in sectPr is a right-to-left section");
    let bytes = craft_docx::write(&d).unwrap();
    let xml = document_xml(&bytes);
    assert_eq!(xml.matches("<w:bidiVisual/>").count(), 1, "{xml}");
    // CT_TblPr is a sequence: tblStyle, tblpPr, tblOverlap, bidiVisual, … tblW, jc, … tblInd.
    let pos = |t: &str| xml.find(t).unwrap_or_else(|| panic!("{t} missing: {xml}"));
    assert!(pos("<w:bidiVisual/>") < pos("<w:tblW"));
    let r = craft_docx::read(&bytes).unwrap();
    let again: Vec<bool> = r.body.iter().filter_map(|b| b.as_table()).map(|t| t.props.bidi_visual).collect();
    assert_eq!(again, vec![true, false]);
    assert!(r.last_section.rtl && r.last_section.columns.count == 2);
}

/// A package whose body is `body` and whose styles part holds `styles`.
fn docx_with_styles(body: &str, styles: &str) -> Vec<u8> {
    let ns = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main""#;
    let root = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    let rels = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rIdS" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>"#;
    let doc = format!(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document {ns}><w:body>{body}</w:body></w:document>"#);
    let st = format!(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:styles {ns}>{styles}</w:styles>"#);
    zip(&[
        ("_rels/.rels", root.as_bytes()),
        ("word/_rels/document.xml.rels", rels.as_bytes()),
        ("word/document.xml", doc.as_bytes()),
        ("word/styles.xml", st.as_bytes()),
    ])
}

/// Thai text draws in the complex-script formatting only where Word draws it so (measured in Word
/// for Windows; run defaults Angsana New 22 pt, complex script Tahoma 11 pt bold). Without
/// `w:cs` or `w:rtl` on the run, its paragraph style or the run defaults, Thai text measures as
/// Angsana New 22 pt, whatever its language, `w:hint` or character style.
#[test]
fn thai_text_takes_the_complex_script_formatting_only_in_a_complex_script_run() {
    let thai = "ทดสอบ ภาษาไทย";
    let defaults = |extra: &str| {
        format!(
            r#"<w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii="Angsana New" w:hAnsi="Angsana New" w:cs="Tahoma"/><w:bCs/><w:sz w:val="44"/><w:szCs w:val="22"/>{extra}</w:rPr></w:rPrDefault></w:docDefaults>
<w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style>
<w:style w:type="paragraph" w:styleId="CsPara"><w:name w:val="CsPara"/><w:basedOn w:val="Normal"/><w:rPr><w:cs/></w:rPr></w:style>
<w:style w:type="character" w:styleId="CsChar"><w:name w:val="CsChar"/><w:rPr><w:cs/></w:rPr></w:style>
<w:style w:type="character" w:styleId="CsOff"><w:name w:val="CsOff"/><w:rPr><w:cs w:val="0"/></w:rPr></w:style>
<w:style w:type="character" w:styleId="RtlChar"><w:name w:val="RtlChar"/><w:rPr><w:rtl/></w:rPr></w:style>
<w:style w:type="table" w:styleId="CsTable"><w:name w:val="CsTable"/><w:rPr><w:cs/></w:rPr></w:style>
<w:style w:type="table" w:styleId="SizeTable"><w:name w:val="SizeTable"/><w:rPr><w:sz w:val="30"/><w:szCs w:val="16"/></w:rPr></w:style>"#
        )
    };
    let latin = ("Angsana New", 22.0, false);
    let complex = ("Tahoma", 11.0, true);
    let run = |rpr: &str| format!(r#"<w:p><w:r><w:rPr>{rpr}</w:rPr><w:t xml:space="preserve">{thai}</w:t></w:r></w:p>"#);
    let styled = |p: &str, rpr: &str| {
        format!(r#"<w:p><w:pPr><w:pStyle w:val="{p}"/></w:pPr><w:r><w:rPr>{rpr}</w:rPr><w:t xml:space="preserve">{thai}</w:t></w:r></w:p>"#)
    };
    let in_table = |style: &str| {
        format!(
            r#"<w:tbl><w:tblPr><w:tblStyle w:val="{style}"/></w:tblPr><w:tblGrid><w:gridCol w:w="6000"/></w:tblGrid><w:tr><w:tc>{}</w:tc></w:tr></w:tbl>"#,
            run("")
        )
    };
    let cs_off = r#"<w:rStyle w:val="CsOff"/>"#;
    let cases: Vec<(&str, String, String, (&str, f32, bool))> = vec![
        ("plain", defaults(""), run(""), latin),
        ("bidi language", defaults(""), run(r#"<w:lang w:val="th-TH" w:bidi="th-TH"/>"#), latin),
        ("hint", defaults(""), run(r#"<w:rFonts w:hint="cs"/>"#), latin),
        ("character style", defaults(""), run(r#"<w:rStyle w:val="CsChar"/>"#), latin),
        ("character style rtl", defaults(""), run(r#"<w:rStyle w:val="RtlChar"/>"#), latin),
        ("run cs", defaults(""), run("<w:cs/>"), complex),
        ("run rtl", defaults(""), run("<w:rtl/>"), complex),
        ("paragraph style", defaults(""), styled("CsPara", ""), complex),
        ("paragraph style, run cs off", defaults(""), styled("CsPara", r#"<w:cs w:val="0"/>"#), latin),
        ("run defaults", defaults("<w:cs/>"), run(""), complex),
        // A character style's `w:cs w:val="0"` doesn't turn the paragraph style's or the run
        // defaults' off either.
        ("paragraph style, character style cs off", defaults(""), styled("CsPara", cs_off), complex),
        ("run defaults, character style cs off", defaults("<w:cs/>"), run(cs_off), complex),
        // A table style's `w:cs` counts, and its sizes are the run's.
        ("table style", defaults(""), in_table("CsTable"), complex),
        ("table style sizes", defaults(""), in_table("SizeTable"), ("Angsana New", 15.0, false)),
    ];
    let check = |d: &Document, name: &str, want: (&str, f32, bool), when: &str| {
        let (p, table) = match &*d.body[0] {
            craft_wordmodel::Block::Table(t) => {
                (t.rows[0].cells[0].blocks[0].as_para().unwrap(), d.styles.table_cells_chr(t).map(|c| c[0][0].clone()))
            }
            b => (b.as_para().unwrap(), None),
        };
        let rc = d.styles.resolve_char_in(p.props.style.as_deref(), table.as_ref(), &p.runs[0].props);
        for ch in p.text.chars() {
            let drawn = if rc.uses_complex(ch) { rc.complex() } else { rc.clone() };
            assert_eq!((drawn.font.as_str(), drawn.size, drawn.bold), want, "{name} {when}: {ch:?}");
        }
        let c = &p.runs[0].props;
        assert_eq!((c.font_cs.as_deref(), c.size_cs, c.bold_cs), (None, None, None), "{name} {when}: nothing is pinned");
    };
    for (name, styles, body, want) in &cases {
        let d = craft_docx::read(&docx_with_styles(body, styles)).unwrap();
        check(&d, name, *want, "read");
        let again = craft_docx::read(&craft_docx::write(&d).unwrap()).unwrap();
        check(&again, name, *want, "after a save");
    }
    // Latin text and right-to-left text are left as they were.
    let body = r#"<w:p><w:r><w:t>Latin only</w:t></w:r></w:p><w:p><w:r><w:t>سلام</w:t></w:r></w:p>"#;
    let d = craft_docx::read(&docx_with_styles(body, &defaults(""))).unwrap();
    for p in paras(&d) {
        assert_eq!(p.runs[0].props.cs, None, "{}", p.text);
    }
    // Latin text follows the same rule (measured): a character style's `w:cs` or `w:rtl` leaves it in the
    // run's own formatting, a character style's `w:cs w:val="0"` under a paragraph style's
    // `w:cs` in the complex-script formatting.
    let latin_in =
        |p: &str, rpr: &str| format!(r#"<w:p><w:pPr><w:pStyle w:val="{p}"/></w:pPr><w:r><w:rPr>{rpr}</w:rPr><w:t>Latin</w:t></w:r></w:p>"#);
    let body =
        latin_in("Normal", r#"<w:rStyle w:val="CsChar"/>"#) + &latin_in("CsPara", cs_off) + &latin_in("Normal", r#"<w:rStyle w:val="RtlChar"/>"#);
    let d = craft_docx::read(&docx_with_styles(&body, &defaults(""))).unwrap();
    let again = craft_docx::read(&craft_docx::write(&d).unwrap()).unwrap();
    for (d, when) in [(&d, "read"), (&again, "after a save")] {
        let drawn: Vec<_> = paras(d)
            .iter()
            .map(|p| {
                let rc = d.styles.resolve_char(p.props.style.as_deref(), &p.runs[0].props);
                let c = if rc.uses_complex('L') { rc.complex() } else { rc };
                (c.font, c.size)
            })
            .collect();
        assert_eq!(drawn, [("Angsana New".to_string(), 22.0), ("Tahoma".to_string(), 11.0), ("Angsana New".to_string(), 22.0)], "{when}");
    }
}

/// Measured in Word for Windows: Thai text as a field's result follows the same rule as ordinary
/// text, also when the reader keeps the field as one object with its result.
#[test]
fn a_fields_thai_result_takes_the_complex_script_formatting_only_in_a_complex_script_run() {
    let styles = r#"<w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii="Angsana New" w:hAnsi="Angsana New" w:cs="Tahoma"/><w:sz w:val="20"/><w:szCs w:val="40"/></w:rPr></w:rPrDefault></w:docDefaults>
<w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style>"#;
    let field = |rpr: &str| {
        let r = |inner: &str| format!("<w:r><w:rPr>{rpr}</w:rPr>{inner}</w:r>");
        format!(
            "<w:p>{}{}{}{}{}</w:p>",
            r(r#"<w:fldChar w:fldCharType="begin"/>"#),
            r(r#"<w:instrText xml:space="preserve"> QUOTE "x" </w:instrText>"#),
            r(r#"<w:fldChar w:fldCharType="separate"/>"#),
            r("<w:t>ทดสอบ</w:t>"),
            r(r#"<w:fldChar w:fldCharType="end"/>"#)
        )
    };
    let drawn = |d: &Document| {
        let p = paras(d)[0];
        let at = p.text.find(craft_wordmodel::para::OBJ).expect("the field is one object");
        let run = p.runs.iter().scan(0, |end, r| {
            let start = *end;
            *end += r.len;
            Some((start, r))
        });
        let (_, run) = run.into_iter().find(|(start, r)| *start <= at && at < start + r.len).unwrap();
        let rc = d.styles.resolve_char(p.props.style.as_deref(), &run.props);
        if rc.uses_complex('ท') { (rc.font_cs.clone(), rc.size_cs) } else { (rc.font.clone(), rc.size) }
    };
    for (what, rpr, want) in [("nothing", "", ("Angsana New", 10.0)), ("w:cs", "<w:cs/>", ("Tahoma", 20.0))] {
        let d = craft_docx::read(&docx_with_styles(&field(rpr), styles)).unwrap();
        assert_eq!(drawn(&d), (want.0.to_string(), want.1), "a run with {what}: read");
        let again = craft_docx::read(&craft_docx::write(&d).unwrap()).unwrap();
        assert_eq!(drawn(&again), (want.0.to_string(), want.1), "a run with {what}: after a save");
    }
}

/// A tracked formatting change's "before" copy is marked by what it draws itself. Here Normal says
/// `w:cs` and the run's character style says `w:cs w:val="0"`, which Word doesn't count, so the
/// run is written `w:cs w:val="0"`; before the change the run had no character style and was
/// complex script by its paragraph style, so rejecting the change in Word must leave it so.
#[test]
fn a_tracked_changes_before_copy_is_marked_by_its_own_formatting() {
    use craft_wordmodel::props::PropChange;
    use craft_wordmodel::{Revision, RevisionKind, Style, StyleKind};
    use std::sync::Arc;
    let mut d = Document::new();
    let sheet = Arc::make_mut(&mut d.styles);
    sheet.default_chr =
        CharProps { font: Some("Angsana New".into()), font_cs: Some("Tahoma".into()), size: Some(10.0), size_cs: Some(20.0), ..Default::default() };
    let cs = |on: bool| CharProps { cs: Some(on), ..Default::default() };
    sheet.upsert(Style { id: "Normal".into(), name: "Normal".into(), kind: StyleKind::Paragraph, chr: cs(true), ..Default::default() });
    sheet.upsert(Style { id: "CsOff".into(), name: "CsOff".into(), kind: StyleKind::Character, chr: cs(false), ..Default::default() });
    d.revisions.push(Revision { kind: RevisionKind::Insert, author: "A".into(), date: String::new(), move_name: None });
    let run = CharProps { style: Some("CsOff".into()), fmt_change: PropChange::boxed(0, CharProps::default()), ..Default::default() };
    d.body = vec![para_block(Paragraph::with_text("ABC", run))];
    let xml = document_xml(&craft_docx::write(&d).unwrap());
    // The text's run (the paragraph mark's properties come first and have a change of their own).
    let run = xml.split("<w:r>").find(|r| r.contains(">ABC</w:t>")).unwrap_or_else(|| panic!("no run: {xml}"));
    let (now, before) = run.split_once("<w:rPrChange").expect("the change is written");
    assert!(now.contains(r#"<w:cs w:val="0"/>"#), "the run as WordCraft draws it: {now}");
    assert!(!before.contains(r#"<w:cs w:val="0"/>"#), "the before copy stays complex script: {before}");
    // Read back, the before copy draws as it did.
    let r = craft_docx::read(&craft_docx::write(&d).unwrap()).unwrap();
    let old = paras(&r)[0].runs[0].props.fmt_change.as_ref().map(|c| c.old.clone()).expect("kept");
    assert!(r.styles.resolve_char(Some("Normal"), &old).uses_complex('A'));
}

/// A tracked formatting change's "before" copy is marked where it needs it, even where the run
/// itself needs nothing. Here the run has no character style, so only its Thai letter is marked;
/// before the change it had a character style saying `w:cs`, which Word doesn't count, so WordCraft
/// drew all of it, the space and Latin text too, in the complex-script formatting. Rejecting the
/// change in Word must bring that back.
#[test]
fn a_tracked_changes_before_copy_is_marked_where_only_it_needs_it() {
    use craft_wordmodel::props::PropChange;
    use craft_wordmodel::{Revision, RevisionKind, Style, StyleKind};
    use std::sync::Arc;
    let mut d = Document::new();
    let sheet = Arc::make_mut(&mut d.styles);
    sheet.default_chr =
        CharProps { font: Some("Angsana New".into()), font_cs: Some("Tahoma".into()), size: Some(10.0), size_cs: Some(20.0), ..Default::default() };
    sheet.upsert(Style {
        id: "CsOn".into(),
        name: "CsOn".into(),
        kind: StyleKind::Character,
        chr: CharProps { cs: Some(true), ..Default::default() },
        ..Default::default()
    });
    d.revisions.push(Revision { kind: RevisionKind::Insert, author: "A".into(), date: String::new(), move_name: None });
    let before = CharProps { style: Some("CsOn".into()), ..Default::default() };
    let run = CharProps { fmt_change: PropChange::boxed(0, before), ..Default::default() };
    d.body = vec![para_block(Paragraph::with_text("\u{0E17} ABC", run))];
    let bytes = craft_docx::write(&d).unwrap();
    let xml = document_xml(&bytes);
    let run = xml.split("<w:r>").find(|r| r.contains("ABC</w:t>")).unwrap_or_else(|| panic!("no run: {xml}"));
    let (now, before) = run.split_once("<w:rPrChange").expect("the change is written");
    assert!(!now.contains("<w:cs"), "the Latin text needs nothing itself: {now}");
    assert!(before.contains("<w:cs/>"), "its before copy is complex script as WordCraft drew it: {before}");
    // As Word reads the before copy (a character style's `w:cs` doesn't count), each character
    // takes the complex-script formatting, as WordCraft drew it.
    let r = craft_docx::read(&bytes).unwrap();
    let p = paras(&r)[0];
    for (range, props) in p.run_ranges() {
        let old = props.fmt_change.as_ref().map(|c| CharProps { style: None, ..c.old.clone() }).expect("kept");
        let rc = r.styles.resolve_char(None, &old);
        for ch in p.text[range].chars() {
            assert!(rc.uses_complex(ch), "{ch:?} in {:?}", p.text);
        }
    }
}
