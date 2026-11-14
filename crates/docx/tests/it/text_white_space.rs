//! White space at the ends of a `w:t` that does not say `xml:space="preserve"` is not text.
//!
//! Measured in Word for Windows 16.0.20625 with "A", the element under test and "B" in runs of
//! their own (9 pt Arial, a space is 2.5 pt): one or three spaces, three spaces written as
//! `&#32;`, spaces around a tab or a line feed, and three spaces with `xml:space="default"` all
//! leave "AB"; " x ", "x   " and "   x" leave "AxB"; "x  y" keeps both spaces; two no-break
//! spaces and two ideographic spaces are kept. With `xml:space="preserve"` three spaces are
//! 7.5 pt and " x " is "A x B".
use craft_wordmodel::Block;

const W: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main""#;
const ROOT: &str = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;

/// The text of the one paragraph "A", `middle` (the `w:t` elements between) and "B" are read as.
fn read(middle: &str) -> String {
    let body = format!("<w:p><w:r><w:t>A</w:t></w:r><w:r>{middle}</w:r><w:r><w:t>B</w:t></w:r></w:p>");
    let doc = format!(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document {W}><w:body>{body}</w:body></w:document>"#);
    let bytes = crate::fixtures::zip(&[("_rels/.rels", ROOT.as_bytes()), ("word/document.xml", doc.as_bytes())]);
    let d = craft_docx::read(&bytes).unwrap();
    let Some(Block::Para(p)) = d.body.first().map(|b| &**b) else { panic!("a paragraph") };
    p.text.clone()
}

#[test]
fn white_space_at_the_ends_of_a_text_element_is_text_only_when_preserved() {
    let cases = [
        // Nothing but white space: nothing is left.
        ("<w:t> </w:t>", "AB"),
        ("<w:t>   </w:t>", "AB"),
        ("<w:t>&#32;&#32;&#32;</w:t>", "AB"),
        (r#"<w:t xml:space="default">   </w:t>"#, "AB"),
        ("<w:t> &#9; </w:t>", "AB"),
        ("<w:t> &#10; </w:t>", "AB"),
        // At either end of text: dropped; between words: kept.
        ("<w:t> x </w:t>", "AxB"),
        ("<w:t>x   </w:t>", "AxB"),
        ("<w:t>   x</w:t>", "AxB"),
        ("<w:t>x  y</w:t>", "Ax  yB"),
        ("<w:t> x  y </w:t>", "Ax  yB"),
        // Not XML white space: kept.
        ("<w:t>&#160;&#160;</w:t>", "A\u{a0}\u{a0}B"),
        ("<w:t>&#12288;&#12288;</w:t>", "A\u{3000}\u{3000}B"),
        // Preserved.
        (r#"<w:t xml:space="preserve">   </w:t>"#, "A   B"),
        (r#"<w:t xml:space="preserve"> x </w:t>"#, "A x B"),
        (r#"<w:t xml:space="preserve"> </w:t>"#, "A B"),
        // Each element by itself: the second one's own spaces go, the first one's stay.
        (r#"<w:t xml:space="preserve"> x </w:t><w:t> y </w:t>"#, "A x yB"),
    ];
    for (middle, text) in cases {
        assert_eq!(read(middle), text, "{middle}");
    }
}

/// The writer marks every text element as preserved, so what it saves comes back whole.
#[test]
fn what_the_writer_saves_is_read_back_with_its_spaces() {
    let d = craft_wordmodel::Document::from_text("  two before, two after  ");
    let again = craft_docx::read(&craft_docx::write(&d).unwrap()).unwrap();
    let Some(Block::Para(p)) = again.body.first().map(|b| &**b) else { panic!("a paragraph") };
    assert_eq!(p.text, "  two before, two after  ");
}
