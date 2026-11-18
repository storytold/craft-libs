//! a table's partial cell margins keep the other sides.

use std::io::Write;

#[test]
fn a_left_only_table_margin_keeps_the_other_sides() {
    let ns = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main""#;
    let root = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="r1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    let rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="s1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>"#;
    let styles = format!(
        r#"<w:styles {ns}><w:style w:type="table" w:styleId="Grid"><w:name w:val="Table Grid"/><w:tblPr><w:tblCellMar><w:top w:w="20" w:type="dxa"/><w:left w:w="108" w:type="dxa"/><w:bottom w:w="40" w:type="dxa"/><w:right w:w="160" w:type="dxa"/></w:tblCellMar></w:tblPr></w:style></w:styles>"#
    );
    let table = |style: &str| {
        format!(
            r#"<w:tbl><w:tblPr>{style}<w:tblCellMar><w:left w:w="107" w:type="dxa"/></w:tblCellMar></w:tblPr><w:tblGrid><w:gridCol w:w="4320"/></w:tblGrid><w:tr><w:tc><w:p><w:r><w:t>Cell</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:p/>"#
        )
    };
    let document = format!(r#"<w:document {ns}><w:body>{}{}</w:body></w:document>"#, table(r#"<w:tblStyle w:val="Grid"/>"#), table(""));
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, data) in [
        ("_rels/.rels", root),
        ("word/_rels/document.xml.rels", rels),
        ("word/document.xml", document.as_str()),
        ("word/styles.xml", styles.as_str()),
    ] {
        zip.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(data.as_bytes()).unwrap();
    }
    let doc = craft_docx::read(&zip.finish().unwrap().into_inner()).unwrap();
    let margins: Vec<_> = doc.body.iter().filter_map(|b| b.as_table()).map(|t| t.props.cell_margins).collect();
    assert_eq!(
        margins,
        vec![Some([1.0, 5.35, 2.0, 8.0]), Some([0.0, 5.35, 0.0, 0.0])],
        "the left side is the table's, the rest the style's (none without one: the file has no default table style)"
    );
}
