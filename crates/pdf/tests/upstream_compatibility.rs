//! Observable resource, object-depth and text-paint compatibility contracts.
#![cfg(feature = "read")]
#![allow(clippy::unwrap_used, clippy::panic)]

fn object(bytes: &mut Vec<u8>, number: usize, body: &[u8]) -> usize {
    let offset = bytes.len();
    bytes.extend_from_slice(format!("{number} 0 obj\n").as_bytes());
    bytes.extend_from_slice(body);
    bytes.extend_from_slice(b"\nendobj\n");
    offset
}

fn assemble(objects: &[Vec<u8>]) -> Vec<u8> {
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let offsets: Vec<_> = objects.iter().enumerate().map(|(i, body)| object(&mut bytes, i + 1, body)).collect();
    let start = bytes.len();
    bytes.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1).as_bytes());
    for offset in offsets {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(format!("trailer << /Size {} /Root 1 0 R >>\nstartxref\n{start}\n%%EOF\n", objects.len() + 1).as_bytes());
    bytes
}

mod page_trees {
    use super::assemble;
    use craft_pdf::syntax::object::{FromBytes, Name};
    use craft_pdf::syntax::page::{Page, Rotation};

    fn assert_page(page: &Page<'_>, id: i32, media: [f64; 4], crop: [f64; 4], rotation: i32, user_unit: f32) {
        assert_eq!(page.raw().get::<i32>(b"ContractPage"), Some(id));
        let actual = page.media_box();
        assert_eq!([actual.x0, actual.y0, actual.x1, actual.y1], media);
        let actual = page.crop_box();
        assert_eq!([actual.x0, actual.y0, actual.x1, actual.y1], crop);
        let actual = match page.rotation() {
            Rotation::None => 0,
            Rotation::Horizontal => 90,
            Rotation::Flipped => 180,
            Rotation::FlippedHorizontal => 270,
        };
        assert_eq!(actual, rotation);
        assert_eq!(page.user_unit(), user_unit);
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn cyclic_page_trees_keep_valid_sibling_order_and_deduplicate_leaves() {
        let pdf = craft_pdf::open_slice(
            &assemble(&[
                b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
                b"<< /Type /Pages /Count 3 /Kids [3 0 R 6 0 R 5 0 R 4 0 R 6 0 R] /MediaBox [0 0 100 200] /CropBox [1 2 90 180] /Rotate 90 /Resources << /Font << /Root << /BaseFont /Helvetica >> >> >> >>".to_vec(),
                b"<< /Type /Pages /Parent 2 0 R /Count 2 /Kids [2 0 R 5 0 R 4 0 R 5 0 R] /MediaBox [0 0 300 400] /CropBox [10 20 280 370] /Rotate 180 /Resources << /Font << /Local << /BaseFont /Courier >> >> >> >>".to_vec(),
                b"<< /Type /Page /Parent 3 0 R /ContractPage 4 /Rotate 270 /UserUnit 2 /Resources << >> >>".to_vec(),
                b"<< /Type /Page /Parent 3 0 R /ContractPage 5 >>".to_vec(),
                b"<< /Type /Page /Parent 2 0 R /ContractPage 6 >>".to_vec(),
            ]),
            None,
        )
        .unwrap();
        assert_eq!(pdf.pages().len(), 3, "cycles and repeated references must not hide or duplicate valid pages");
        assert_page(&pdf.pages()[0], 5, [0.0, 0.0, 300.0, 400.0], [10.0, 20.0, 280.0, 370.0], 180, 1.0);
        assert_page(&pdf.pages()[1], 4, [0.0, 0.0, 300.0, 400.0], [10.0, 20.0, 280.0, 370.0], 270, 2.0);
        assert_page(&pdf.pages()[2], 6, [0.0, 0.0, 100.0, 200.0], [1.0, 2.0, 90.0, 180.0], 90, 1.0);
        let root = Name::from_bytes(b"/Root").unwrap();
        let local = Name::from_bytes(b"/Local").unwrap();
        assert!(pdf.pages()[0].resources().get_font(&local).is_some());
        assert!(pdf.pages()[0].resources().get_font(&root).is_none());
        assert!(pdf.pages()[1].resources().get_font(&local).is_none());
        assert!(pdf.pages()[1].resources().get_font(&root).is_none());
        assert!(pdf.pages()[2].resources().get_font(&root).is_some());
        assert!(pdf.pages()[2].resources().get_font(&local).is_none());
    }
}
