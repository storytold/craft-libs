//! Small workbooks built in code, for tests and examples.

use craft_opc::{PackageOut, RelsOut};

const MAIN_NS: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const R_NS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

/// An `.xlsx` with the given sheets, each `(name, the rows inside <sheetData>)`, and shared
/// strings. Names and strings are written as given: escape them first if they need it.
pub fn workbook(sheets: &[(&str, &str)], strings: &[&str]) -> Vec<u8> {
    let mut out = PackageOut::default();
    let mut root = RelsOut::default();
    root.add("officeDocument", "xl/workbook.xml", false);
    out.rels("", &root);
    let mut rels = RelsOut::default();
    let mut list = String::new();
    for (i, (name, rows)) in sheets.iter().enumerate() {
        let n = i + 1;
        let part = format!("xl/worksheets/sheet{n}.xml");
        let id = rels.add("worksheet", &format!("worksheets/sheet{n}.xml"), false);
        list.push_str(&format!(r#"<sheet name="{name}" sheetId="{n}" r:id="{id}"/>"#));
        out.xml_part(
            &part,
            "application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml",
            format!(r#"<worksheet xmlns="{MAIN_NS}"><sheetData>{rows}</sheetData></worksheet>"#).into_bytes(),
        );
    }
    if !strings.is_empty() {
        rels.add("sharedStrings", "sharedStrings.xml", false);
        let items: String = strings.iter().map(|s| format!("<si><t>{s}</t></si>")).collect();
        out.xml_part(
            "xl/sharedStrings.xml",
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml",
            format!(r#"<sst xmlns="{MAIN_NS}" count="{0}" uniqueCount="{0}">{items}</sst>"#, strings.len()).into_bytes(),
        );
    }
    out.rels("xl/workbook.xml", &rels);
    out.xml_part(
        "xl/workbook.xml",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml",
        format!(r#"<workbook xmlns="{MAIN_NS}" xmlns:r="{R_NS}"><sheets>{list}</sheets></workbook>"#).into_bytes(),
    );
    out.finish().unwrap_or_default()
}

/// A ZIP archive of the given entries, stored as named (no content types are added).
pub fn zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let entries: Vec<(String, Vec<u8>)> = entries.iter().map(|(n, b)| (n.to_string(), b.to_vec())).collect();
    craft_opc::zip_entries(&entries).unwrap_or_default()
}
