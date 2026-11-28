//! The embedding app sets its name once per process. Exercise the setter in a fresh copy of the test binary.

use std::io::Read;

#[test]
fn the_embedding_app_names_itself_in_app_xml() {
    const CHILD: &str = "CRAFT_DOCX_APP_NAME_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new(std::env::current_exe().expect("DOCX test executable"))
            .args(["--exact", "application_name::the_embedding_app_names_itself_in_app_xml", "--nocapture"])
            .env(CHILD, "1")
            .output()
            .expect("run the application name test in a fresh process");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success() && stdout.contains("application name verified"), "{stdout}\n{stderr}");
        return;
    }
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
    println!("application name verified");
}
