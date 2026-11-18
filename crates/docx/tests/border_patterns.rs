//! Distinct border styles survive the shared DOCX reader/writer.
use craft_wordmodel::props::{Border, BorderStyle, Borders};
use craft_wordmodel::{Block, CharProps, Document, Paragraph, Rgb, para_block};

fn rt(doc: &Document) -> Document {
    craft_docx::read(&craft_docx::write(doc).unwrap()).unwrap()
}

/// Each border style keeps its own `w:val` through a save: `dashSmallGap` stays
/// `dashSmallGap` (it was saved as `dashed`, a different pattern in Word).
#[test]
fn border_styles_keep_their_pattern() {
    for style in [BorderStyle::Dotted, BorderStyle::Dashed, BorderStyle::DashSmallGap, BorderStyle::DotDash] {
        let b = Border { style, width: 0.5, color: Some(Rgb(0, 0, 0)), space: 1.0 };
        let mut p = Paragraph::with_text("Rule", CharProps::default());
        p.props.borders = Some(Borders { bottom: Some(b), ..Borders::default() });
        let mut d = Document::new();
        d.body = vec![para_block(p)];
        let back = rt(&d);
        let Block::Para(q) = &*back.body[0] else { panic!("a paragraph") };
        assert_eq!(q.props.borders.and_then(|b| b.bottom).map(|b| b.style), Some(style), "{style:?} round-trips");
    }
    assert_eq!(BorderStyle::from_ooxml("dashSmallGap"), BorderStyle::DashSmallGap);
    assert_eq!(BorderStyle::DashSmallGap.ooxml(), "dashSmallGap");
}
