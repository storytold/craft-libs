//! `craft_pdf::testing::Pdf`, the builder other crates' tests reach through `test-support`: a file
//! it builds opens in the object layer with every object where the cross-reference table says.
#![cfg(feature = "test-support")]
#![allow(clippy::unwrap_used, clippy::panic)]

use std::sync::Arc;

use craft_pdf::cos::{Document, ObjRef, Object, XrefEntry};
use craft_pdf::testing::Pdf;

fn one_page() -> Pdf {
    Pdf::new()
        .version("1.4")
        .obj("<< /Type /Catalog /Pages 2 0 R >>")
        .obj("<< /Type /Pages /Kids [3 0 R] /Count 1 >>")
        .obj("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 300] /Contents 4 0 R >>")
        .stream("", b"0 0 50 50 re f")
        .trailer_entry("/Info 5 0 R")
        .obj("<< /Title (Fixture) >>")
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn a_built_file_opens_without_repair() {
    let bytes = one_page().build();
    assert!(bytes.starts_with(b"%PDF-1.4\n"));
    assert!(bytes.ends_with(b"%%EOF\n"));

    let doc = Document::open(Arc::new(bytes)).unwrap();
    assert!(!doc.reconstructed(), "the cross-reference table is exact: {:?}", doc.repair_log());
    assert_eq!(doc.version(), "1.4");
    assert_eq!(doc.root(), Some(ObjRef::new(1, 0)));
    for num in 1..=5 {
        assert!(matches!(doc.xref_entry(num), Some(XrefEntry::InFile { generation: 0, .. })), "object {num} is in the table");
        assert!(doc.xref_entry_is_exact(num), "object {num} is at its table offset");
    }
    assert_eq!(doc.xref_entry(6), None);
    let Object::Stream(contents) = &*doc.get(ObjRef::new(4, 0)) else { panic!("object 4 is the content stream") };
    assert_eq!(contents.decoded().unwrap(), b"0 0 50 50 re f");
    assert_eq!(doc.trailer().get(b"Info"), Some(&Object::Ref(ObjRef::new(5, 0))));
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn next_obj_names_the_object_added_next() {
    let pdf = one_page();
    assert_eq!(pdf.next_obj(), 6);
    let doc = Document::open(Arc::new(pdf.obj("<< /Marker true >>").build())).unwrap();
    assert!(matches!(doc.xref_entry(6), Some(XrefEntry::InFile { .. })));
    assert!(doc.xref_entry_is_exact(6));
    assert_eq!(doc.dict(&doc.get(ObjRef::new(6, 0))).and_then(|d| d.get(b"Marker").cloned()), Some(Object::Bool(true)));
}
