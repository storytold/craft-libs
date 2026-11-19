//! Speaker notes keep their run and paragraph formatting: a bold heading, indented points with
//! custom bullets, plain lines.

mod common;

use std::sync::Arc;

use common::{export, import, part, rewrite};
use craft_pptx::text::{Bullet, ParaProps, TextBody};

const NOTES: &str = "ppt/notesSlides/notesSlide1.xml";

/// A notes slide written the way PowerPoint writes one (Slovak text): a bold heading, a plain
/// line, two level-2 points with a hanging indent and a Wingdings square bullet (one mixing a bold
/// label and plain text), a level-2 paragraph with buNone, and a closing plain line.
const NOTES_XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:notes xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/><a:chOff x="0" y="0"/><a:chExt cx="0" cy="0"/></a:xfrm></p:grpSpPr><p:sp><p:nvSpPr><p:cNvPr id="2" name="Slide Image Placeholder 1"/><p:cNvSpPr><a:spLocks noGrp="1" noRot="1" noChangeAspect="1"/></p:cNvSpPr><p:nvPr><p:ph type="sldImg"/></p:nvPr></p:nvSpPr><p:spPr/></p:sp><p:sp><p:nvSpPr><p:cNvPr id="3" name="Notes Placeholder 2"/><p:cNvSpPr><a:spLocks noGrp="1"/></p:cNvSpPr><p:nvPr><p:ph type="body" idx="1"/></p:nvPr></p:nvSpPr><p:spPr/><p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:rPr lang="sk-SK" b="1" dirty="0"/><a:t>Metodický prehľad</a:t></a:r></a:p><a:p><a:r><a:rPr lang="sk-SK" dirty="0"/><a:t>Žiaci pracujú vo dvojiciach.</a:t></a:r></a:p><a:p><a:pPr marL="285750" lvl="1" indent="-285750"><a:buFont typeface="Wingdings" panose="05000000000000000000" pitchFamily="2" charset="2"/><a:buChar char="&#167;"/></a:pPr><a:r><a:rPr lang="sk-SK" b="1" dirty="0"/><a:t>Cieľ: </a:t></a:r><a:r><a:rPr lang="sk-SK" dirty="0"/><a:t>vysvetliť cyklus</a:t></a:r></a:p><a:p><a:pPr marL="285750" lvl="1" indent="-285750"><a:buFont typeface="Wingdings" panose="05000000000000000000" pitchFamily="2" charset="2"/><a:buChar char="&#167;"/></a:pPr><a:r><a:rPr lang="sk-SK" i="1" dirty="0"/><a:t>Reflexia</a:t></a:r></a:p><a:p><a:pPr marL="285750" lvl="1" indent="-285750"><a:buNone/></a:pPr><a:r><a:rPr lang="sk-SK" dirty="0"/><a:t>Bez odrážky</a:t></a:r></a:p><a:p><a:r><a:rPr lang="sk-SK" dirty="0"/><a:t>Pomôcky: počítač</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:notes>"#;

/// A deck with one notes slide, its notes replaced by [`NOTES_XML`].
fn deck() -> Vec<u8> {
    let mut p = common::deck();
    Arc::make_mut(&mut p.slides[0]).notes = TextBody::from_text("placeholder");
    let bytes = export(&p);
    assert!(part(&bytes, NOTES).contains("placeholder"), "the export has no {NOTES}");
    rewrite(&bytes, |name, data| Some(if name == NOTES { NOTES_XML.as_bytes().to_vec() } else { data.to_vec() }), &[])
}

fn assert_formatted(notes: &TextBody) {
    assert_eq!(notes.text(), "Metodický prehľad\nŽiaci pracujú vo dvojiciach.\nCieľ: vysvetliť cyklus\nReflexia\nBez odrážky\nPomôcky: počítač");
    let [head, plain, goal, reflect, unbulleted, closing] = notes.paragraphs.as_slice() else { panic!("six paragraphs: {notes:?}") };
    assert_eq!(head.runs[0].props.bold, Some(true), "bold heading");
    for para in [head, plain, closing] {
        assert_eq!((para.level, &para.props), (0, &ParaProps::default()), "{:?}", para.text());
    }
    assert_eq!(plain.runs[0].props.bold, None);
    for para in [goal, reflect] {
        assert_eq!(para.level, 1, "level 2");
        assert_eq!(para.props.margin_left, Some(22.5), "marL 285750 EMU");
        assert_eq!(para.props.indent, Some(-22.5), "hanging indent");
        assert_eq!(para.props.bullet, Some(Bullet::Char { char: "§".into() }));
        assert_eq!(para.props.bullet_font.as_deref(), Some("Wingdings"));
    }
    assert_eq!((goal.runs[0].text.as_str(), goal.runs[0].props.bold), ("Cieľ: ", Some(true)), "bold label");
    assert_eq!((goal.runs[1].text.as_str(), goal.runs[1].props.bold), ("vysvetliť cyklus", None), "then plain");
    assert_eq!(reflect.runs[0].props.italic, Some(true));
    assert_eq!((unbulleted.level, unbulleted.props.bullet.clone(), unbulleted.props.margin_left), (1, Some(Bullet::None), Some(22.5)));
}

#[test]
fn notes_import_keeps_bold_levels_and_indents() {
    let p = import(&deck());
    assert_formatted(&p.slides[0].notes);
}

#[test]
fn notes_formatting_survives_a_round_trip() {
    let p = import(&deck());
    let again = import(&export(&p));
    assert_formatted(&again.slides[0].notes);
    // And the written XML says so, for PowerPoint.
    let xml = part(&export(&again), NOTES);
    assert!(xml.contains(r#"b="1""#), "{xml}");
    assert!(xml.contains(r#"lvl="1""#) && xml.contains(r#"marL="285750""#) && xml.contains(r#"indent="-285750""#), "{xml}");
    assert!(xml.contains(r#"<a:buFont typeface="Wingdings""#) && xml.contains("<a:buChar") && xml.contains("<a:buNone/>"), "{xml}");
}
