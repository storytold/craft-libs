//! Pictures in cells and dynamic-array metadata: written, read back, and hostile metadata.

use craft_xlsx::testing::workbook;
use craft_xlsx::{CellPicture, Document, Error, Limits, Package};

fn png(w: u32, h: u32, tag: u8) -> Vec<u8> {
    let mut b = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
    b.extend(w.to_be_bytes());
    b.extend(h.to_be_bytes());
    b.extend([8, 6, 0, 0, 0, tag, 0, 0, 0]);
    b
}

fn new_doc() -> Document {
    Document::read(&workbook(&[("S", r#"<row r="1"><c r="A1" vm="1" t="e"><v>#VALUE!</v></c></row>"#)], &[]), &Limits::default()).unwrap()
}

fn part(doc: &Document, name: &str) -> String {
    doc.parts.iter().find(|p| p.name == name).map(|p| String::from_utf8_lossy(&p.bytes).into_owned()).unwrap_or_default()
}

#[test]
fn pictures_round_trip() {
    let pictures = vec![
        CellPicture { image: png(10, 10, 1), alt: "a & b".into() },
        CellPicture { image: png(20, 5, 2), alt: String::new() },
        CellPicture { image: png(10, 10, 1), alt: "again".into() },
    ];
    let mut doc = new_doc();
    doc.add_cell_metadata(&pictures, true).unwrap();
    assert!(matches!(doc.add_cell_metadata(&pictures, false), Err(Error::Invalid(_))));
    let back = Document::read(&doc.write().unwrap(), &Limits::default()).unwrap();
    assert_eq!(back.cell_pictures, pictures.iter().cloned().map(Some).collect::<Vec<_>>());
    assert_eq!(back.warnings, ["no styles part"]);
    assert!(back.parts.iter().any(|p| p.name == "xl/media/cellImage1.png"));
    let meta = part(&back, "xl/metadata.xml");
    assert!(meta.contains(r#"<metadataTypes count="2"><metadataType name="XLDAPR""#) && meta.contains(r#"<rc t="2" v="2"/>"#));
    // Kept as read on a second write.
    let again = Document::read(&back.write().unwrap(), &Limits::default()).unwrap();
    assert_eq!(again.cell_pictures, back.cell_pictures);
}

/// The dynamic-array metadata alone, as GridCraft writes it.
#[test]
fn dynamic_arrays_only() {
    let mut doc = new_doc();
    doc.add_cell_metadata(&[], true).unwrap();
    assert_eq!(
        part(&doc, "xl/metadata.xml"),
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<metadata xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" xmlns:xda=\"http://schemas.microsoft.com/office/spreadsheetml/2017/dynamicarray\"><metadataTypes count=\"1\"><metadataType name=\"XLDAPR\" minSupportedVersion=\"120000\" copy=\"1\" pasteAll=\"1\" pasteValues=\"1\" merge=\"1\" splitFirst=\"1\" rowColShift=\"1\" clearFormats=\"1\" clearComments=\"1\" assign=\"1\" coerce=\"1\" cellMeta=\"1\"/></metadataTypes><futureMetadata name=\"XLDAPR\" count=\"1\"><bk><extLst><ext uri=\"{bdbb8cdc-fa1e-496e-a857-3c3f30c029c3}\"><xda:dynamicArrayProperties fDynamic=\"1\" fCollapsed=\"0\"/></ext></extLst></bk></futureMetadata><cellMetadata count=\"1\"><bk><rc t=\"1\" v=\"0\"/></bk></cellMetadata></metadata>"
    );
    let bytes = doc.write().unwrap();
    let mut p = Package::open(&bytes, &Limits::default()).unwrap();
    assert_eq!(p.cell_pictures().unwrap(), Vec::new());
}

/// Rewrites one part of a written workbook.
fn with_part(bytes: &[u8], name: &str, edit: impl Fn(String) -> String) -> Vec<u8> {
    let mut doc = Document::read(bytes, &Limits::default()).unwrap();
    let p = doc.parts.iter_mut().find(|p| p.name == name).unwrap();
    p.bytes = edit(String::from_utf8_lossy(&p.bytes).into_owned()).into_bytes();
    doc.write().unwrap()
}

#[test]
fn hostile_metadata_reads_what_it_can() {
    let mut doc = new_doc();
    doc.add_cell_metadata(&[CellPicture { image: png(4, 4, 0), alt: "x".into() }], false).unwrap();
    let good = doc.write().unwrap();
    let pictures = |b: &[u8]| {
        let mut p = Package::open(b, &Limits::default()).unwrap();
        let pics = p.cell_pictures().unwrap();
        (pics.iter().map(Option::is_some).collect::<Vec<_>>(), p.warnings().to_vec())
    };
    assert_eq!(pictures(&good).0, [true]);

    // A value block pointing past the future metadata, and one with no rich-value record.
    let b = with_part(&good, "xl/metadata.xml", |m| {
        m.replace(
            r#"<valueMetadata count="1"><bk><rc t="1" v="0"/></bk>"#,
            r#"<valueMetadata count="2"><bk><rc t="1" v="9"/></bk><bk><rc t="7" v="0"/></bk>"#,
        )
    });
    assert_eq!(pictures(&b).0, [false, false]);
    // Two XLRICHVALUE types: nothing is trusted.
    let b = with_part(&good, "xl/metadata.xml", |m| m.replace("</metadataTypes>", r#"<metadataType name="XLRICHVALUE"/></metadataTypes>"#));
    let (got, warnings) = pictures(&b);
    assert!(got.is_empty() && warnings.iter().any(|w| w.contains("two XLRICHVALUE")));
    // A rich value naming a relationship that is not there, and a count that lies.
    let b = with_part(&good, "xl/richData/rdrichvalue.xml", |m| m.replace("<v>0</v>", "<v>5</v>").replace(r#"count="1""#, r#"count="3""#));
    let (got, warnings) = pictures(&b);
    assert_eq!(got, [false]);
    assert!(warnings.iter().any(|w| w.contains("count")));
    // Not a picture: the image bytes are not PNG.
    let b = with_part(&good, "xl/media/cellImage1.png", |_| "GIF89a".into());
    assert_eq!(pictures(&b).0, [false]);
    // More records than the limit is refused.
    let many = "<bk><rc t=\"1\" v=\"0\"/></bk>".repeat(craft_xlsx::MAX_RECORDS + 1);
    let b = with_part(&good, "xl/metadata.xml", |m| m.replace(r#"<bk><rc t="1" v="0"/></bk>"#, &many));
    let mut p = Package::open(&b, &Limits::default()).unwrap();
    assert!(matches!(p.cell_pictures(), Err(Error::TooLarge(_))));
}
