//! An app that embeds the writer names itself in `docProps/app.xml` (its own test binary: the
//! name is set once per process).

use std::io::Read;

#[test]
fn the_embedding_app_names_itself_in_app_xml() {
    assert_eq!(craft_docx::application_name(), "WordCraft", "the default");
    craft_docx::set_application_name("Example Writer");
    craft_docx::set_application_name("Other");
    assert_eq!(craft_docx::application_name(), "Example Writer", "only the first call counts");
    let mut d = craft_wordmodel::Document::new();
    d.body = vec![craft_wordmodel::para_block(craft_wordmodel::Paragraph::with_text("Text", Default::default()))];
    let bytes = craft_docx::write(&d).unwrap();
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
    let mut app = String::new();
    z.by_name("docProps/app.xml").unwrap().read_to_string(&mut app).unwrap();
    assert!(app.contains("<Application>Example Writer</Application>"), "{app}");
}
