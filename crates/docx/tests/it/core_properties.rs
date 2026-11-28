//! keep the language, status, identifier, version and last-printed core properties.

use std::io::Read;

#[test]
fn core_properties_beyond_the_usual_ones_survive_a_save() {
    let mut d = craft_wordmodel::Document::new();
    d.body = vec![craft_wordmodel::para_block(craft_wordmodel::Paragraph::with_text("Text", Default::default()))];
    d.core.language = "es-ES".into();
    d.core.content_status = "Final".into();
    d.core.identifier = "EXP-2026/014".into();
    d.core.version = "1.2".into();
    d.core.last_printed = "2026-11-02T09:30:00Z".into();
    let bytes = craft_docx::write(&d).unwrap();
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
    let mut core = String::new();
    z.by_name("docProps/core.xml").unwrap().read_to_string(&mut core).unwrap();
    for tag in ["<dc:language>es-ES</dc:language>", "<cp:contentStatus>Final</cp:contentStatus>", "<cp:version>1.2</cp:version>"] {
        assert!(core.contains(tag), "{tag} in {core}");
    }
    let c = craft_docx::read(&bytes).unwrap().core;
    assert_eq!(
        (c.language.as_str(), c.content_status.as_str(), c.identifier.as_str(), c.version.as_str(), c.last_printed.as_str()),
        ("es-ES", "Final", "EXP-2026/014", "1.2", "2026-11-02T09:30:00Z")
    );
}
