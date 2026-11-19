//! A document that brings no font table of its own (one built in code, or read from another
//! format) gets one naming the fonts it uses, with their generic family and pitch: a reader
//! without Arial then draws a sans, not its own serif default.
use std::collections::BTreeMap;
use std::io::Read;
use std::sync::Arc;

use craft_wordmodel::props::CharProps;
use craft_wordmodel::table::Table;
use craft_wordmodel::{Block, Document, Paragraph};

fn unzip(bytes: &[u8]) -> BTreeMap<String, String> {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut out = BTreeMap::new();
    for i in 0..z.len() {
        let mut f = z.by_index(i).unwrap();
        let mut s = String::new();
        if f.read_to_string(&mut s).is_ok() {
            out.insert(f.name().unwrap().to_string(), s);
        }
    }
    out
}

/// The `w:font` element for `name` in `table`.
fn font<'a>(table: &'a str, name: &str) -> &'a str {
    let open = format!("<w:font w:name=\"{name}\">");
    let at = table.find(&open).unwrap_or_else(|| panic!("no {name} in {table}"));
    let rest = &table[at..];
    &rest[..rest.find("</w:font>").unwrap()]
}

fn append(p: &mut Paragraph, s: &str, props: &CharProps) {
    let at = p.len();
    p.insert_text(at, s, props).unwrap();
}

fn run(text: &str, font: &str) -> Paragraph {
    Paragraph::with_text(text, CharProps { font: Some(font.into()), ..Default::default() })
}

#[test]
fn a_built_document_names_its_fonts_with_their_family_and_pitch() {
    let mut d = Document::new();
    let mut table = Table::new(1, 1, 200.0);
    if let Some(c) = table.rows.first_mut().and_then(|r| r.cells.first_mut()) {
        c.blocks = vec![Arc::new(Block::Para(run("cell", "Courier New")))];
    }
    let mut symbol = Paragraph::new();
    append(&mut symbol, "\u{F0B7}", &CharProps { sym_font: Some("Symbol".into()), ..Default::default() });
    let mut serif = run("serif", "Times New Roman");
    append(&mut serif, " east", &CharProps { font_ea: Some("MS Mincho".into()), ..Default::default() });
    d.body =
        vec![Arc::new(Block::Para(run("sans", "Arial"))), Arc::new(Block::Table(table)), Arc::new(Block::Para(serif)), Arc::new(Block::Para(symbol))];
    let files = unzip(&craft_docx::write(&d).unwrap());
    let t = files.get("word/fontTable.xml").unwrap_or_else(|| panic!("no font table in {:?}", files.keys()));
    assert!(font(t, "Arial").contains(r#"<w:family w:val="swiss"/><w:pitch w:val="variable"/>"#), "{t}");
    assert!(font(t, "Times New Roman").contains(r#"<w:family w:val="roman"/>"#), "{t}");
    assert!(font(t, "Courier New").contains(r#"<w:family w:val="modern"/><w:pitch w:val="fixed"/>"#), "a table cell's font: {t}");
    assert!(font(t, "MS Mincho").contains("<w:family "), "an East Asian font: {t}");
    assert!(font(t, "Symbol").contains(r#"<w:charset w:val="02"/>"#), "a symbol font has the symbol set: {t}");
    // The built-in styles' fonts are there too.
    let body = d.styles.default_chr.font.clone().unwrap();
    font(t, &body);
    assert_eq!(t.matches(r#"<w:font w:name="Arial">"#).count(), 1, "each font once");
    // Related from the document and typed.
    assert!(
        files["word/_rels/document.xml.rels"].contains(r#"relationships/fontTable" Target="fontTable.xml""#),
        "{}",
        files["word/_rels/document.xml.rels"]
    );
    assert!(files["[Content_Types].xml"].contains("wordprocessingml.fontTable+xml"));
    // The reader takes the families back: Times New Roman is a serif.
    let back = craft_docx::read(&craft_docx::write(&d).unwrap()).unwrap();
    assert!(back.settings.roman_fonts.iter().any(|f| f == "Times New Roman"), "{:?}", back.settings.roman_fonts);
    assert!(!back.settings.roman_fonts.iter().any(|f| f == "Arial"));
    // Saved again, the table read back is written as it was.
    let again = unzip(&craft_docx::write(&back).unwrap());
    assert_eq!(again.get("word/fontTable.xml"), Some(t));
}

#[test]
fn a_file_font_table_family_wins_over_the_name() {
    // A file's table classes "Fancy Face" as roman: the generated table, written when that one is
    // gone, keeps it a serif.
    let mut d = Document::new();
    d.body = vec![Arc::new(Block::Para(run("x", "Fancy Face")))];
    d.settings.roman_fonts = vec!["Fancy Face".into()];
    let files = unzip(&craft_docx::write(&d).unwrap());
    assert!(font(&files["word/fontTable.xml"], "Fancy Face").contains(r#"<w:family w:val="roman"/>"#));
}

#[test]
fn hostile_font_counts_and_names_are_bounded() {
    let mut d = Document::new();
    let mut p = Paragraph::new();
    for i in 0..5000 {
        append(&mut p, "x", &CharProps { font: Some(format!("Face {i}")), ..Default::default() });
    }
    append(&mut p, "y", &CharProps { font: Some("N".repeat(10_000)), ..Default::default() });
    d.body = vec![Arc::new(Block::Para(p))];
    let files = unzip(&craft_docx::write(&d).unwrap());
    let t = &files["word/fontTable.xml"];
    assert!(t.matches("<w:font ").count() <= 1000, "{}", t.matches("<w:font ").count());
    assert!(!t.contains(&"N".repeat(300)));
}
