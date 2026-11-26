//! A document followed by another file's data, as a file recovered from a damaged disk can be:
//! both readers find the document at its own end, as other viewers do.
#![cfg(all(feature = "read", feature = "cos"))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use craft_pdf::cos::{Document, Object};
use craft_pdf::testing::Pdf;

/// A one-page document 200 points wide, and its length.
fn document() -> (Vec<u8>, usize) {
    let document = Pdf::new()
        .version("1.4")
        .obj("<< /Type /Catalog /Pages 2 0 R >>")
        .obj("<< /Type /Pages /Kids [3 0 R] /Count 1 >>")
        .obj("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 100] >>")
        .build();
    let end = document.len();
    (document, end)
}

/// What a program's text holds after a fragment: a `startxref` template, so the file's last
/// `startxref` names nothing.
const PROGRAM_TEXT: &[u8] = b"\xff\xfe\0/Producer (%s)\nxref\n0 %d\ntrailer\n<< /Size %d /Root %d 0 R >>\nstartxref\n%ld\n%%%%EOF\n\0\0";

/// The width of each page in the page tree, as the object layer reads it.
fn object_layer_widths(doc: &Document) -> Vec<Option<i64>> {
    let catalog = doc.get(doc.root().expect("a root"));
    let pages = catalog.as_dict().and_then(|c| c.get(b"Pages").cloned()).map(|p| doc.resolve(&p)).expect("a page tree");
    let kids = pages.as_dict().and_then(|p| p.get(b"Kids").cloned()).map(|k| doc.resolve(&k)).expect("kids");
    let Object::Array(kids) = &*kids else { panic!("kids are an array") };
    kids.iter()
        .map(|kid| {
            let page = doc.resolve(kid);
            let media_box = page.as_dict().and_then(|p| p.get(b"MediaBox").cloned()).map(|b| doc.resolve(&b));
            match media_box.as_deref() {
                Some(Object::Array(b)) => b.get(2).and_then(|w| match w {
                    Object::Int(w) => Some(*w),
                    _ => None,
                }),
                _ => None,
            }
        })
        .collect()
}

/// The width of each page, as the renderer reads it.
fn renderer_widths(file: &[u8]) -> Vec<f64> {
    let pdf = craft_pdf::open_slice(file, None).expect("the renderer finds a document");
    pdf.pages()
        .iter()
        .map(|p| {
            let b = p.media_box();
            b.x1 - b.x0
        })
        .collect()
}

fn says_where_the_document_ends(doc: &Document, end: usize) -> bool {
    doc.repair_log().iter().any(|l| l.contains(&format!("ends at byte {end}")))
}

/// Fragments whose objects reuse the document's numbers and are none of a catalog, page tree or
/// page: rebuilding the whole file finds no document at all.
#[test]
fn a_document_followed_by_other_data_is_read_up_to_its_own_end() {
    let (mut file, end) = document();
    file.extend_from_slice(b"\0\0\x13\x88 fragment\n1 0 obj\n<< /Length 4 >>\nstream\nabcd\nendstream\nendobj\n");
    file.extend_from_slice(b"2 0 obj\n(not a page tree)\nendobj\n3 0 obj\n[1 2 3]\nendobj\n");
    file.extend_from_slice(PROGRAM_TEXT);

    let doc = Document::open(Arc::new(file.clone())).expect("the object layer finds the document");
    assert_eq!(object_layer_widths(&doc), [Some(200)], "{:?}", doc.repair_log());
    assert!(says_where_the_document_ends(&doc, end), "{:?}", doc.repair_log());
    assert_eq!(renderer_widths(&file), [200.0]);
}

/// Fragments of another document, with its own catalog and two pages under the same numbers:
/// rebuilding the whole file would show that document, or a mix of the two.
#[test]
fn another_documents_fragments_after_the_end_are_not_mixed_in() {
    let (mut file, end) = document();
    file.extend_from_slice(b"\0\0\x13\x88 fragment\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
    file.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>\nendobj\n");
    file.extend_from_slice(b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 600 100] >>\nendobj\n");
    file.extend_from_slice(b"4 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 600 100] >>\nendobj\n");
    file.extend_from_slice(PROGRAM_TEXT);

    let doc = Document::open(Arc::new(file.clone())).expect("the object layer opens the file");
    assert_eq!(object_layer_widths(&doc), [Some(200)], "the document's own page: {:?}", doc.repair_log());
    assert!(says_where_the_document_ends(&doc, end), "{:?}", doc.repair_log());
    assert_eq!(renderer_widths(&file), [200.0], "the renderer reads the same page");
}

/// An update appended after the document whose own `startxref` is damaged is no other file's
/// data: the file is rebuilt as before, and the update's page wins.
#[test]
fn an_appended_update_with_a_damaged_end_still_reads_the_update() {
    let (mut file, _) = document();
    file.extend_from_slice(b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 500 100] >>\nendobj\n");
    file.extend_from_slice(b"startxref\nnot an offset\n%%EOF\n");

    let doc = Document::open(Arc::new(file.clone())).expect("the object layer opens the file");
    assert_eq!(object_layer_widths(&doc), [Some(500)], "the update's page: {:?}", doc.repair_log());
    assert!(!doc.repair_log().iter().any(|l| l.contains("ends at byte")), "{:?}", doc.repair_log());
    assert_eq!(renderer_widths(&file), [500.0], "the renderer reads the same page");
}

/// An appended update whose damaged `startxref` follows an object header with a stray number
/// before it (`42 3 0 obj`), which a rebuild skips: still an update, not another file's data, so
/// the update's page wins in both readers.
#[test]
fn an_appended_update_after_a_stray_number_still_reads_the_update() {
    let (mut file, _) = document();
    file.extend_from_slice(b"42 3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 500 100] >>\nendobj\n");
    file.extend_from_slice(b"startxref\nnot an offset\n%%EOF\n");

    let doc = Document::open(Arc::new(file.clone())).expect("the object layer opens the file");
    assert_eq!(object_layer_widths(&doc), [Some(500)], "the update's page: {:?}", doc.repair_log());
    assert!(!doc.repair_log().iter().any(|l| l.contains("ends at byte")), "{:?}", doc.repair_log());
    assert_eq!(renderer_widths(&file), [500.0], "the renderer reads the same page");
}
