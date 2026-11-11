//! Integration tests of craft-pdf: contracts.

#![cfg(feature = "raster")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use craft_pdf::{
    interpret::InterpreterSettings,
    raster::{self, Options, RenderCache},
};
use std::sync::Arc;

fn pdf(content: &str, extra: &str) -> Arc<Vec<u8>> {
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 40 20] /Contents 4 0 R {extra} >>"),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
    ];
    assemble(&objects)
}

fn assemble(objects: &[String]) -> Arc<Vec<u8>> {
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = vec![0];
    for (i, body) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
    }
    let start = bytes.len();
    bytes.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len()).as_bytes());
    for offset in offsets.iter().skip(1) {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{start}\n%%EOF\n", offsets.len()).as_bytes());
    Arc::new(bytes)
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn cropped_rotated_geometry_and_tile_match_full_raster() {
    let doc = craft_pdf::open(pdf("1 0 0 rg 0 0 40 20 re f", "/CropBox [10 0 40 20] /Rotate 90"), None).unwrap();
    let page = &doc.pages()[0];
    assert_eq!(raster::dimensions(page, &Options::default()).unwrap(), [20, 30]);
    let full = raster::render(page, &RenderCache::new(), &InterpreterSettings::default(), &Options::default()).unwrap();
    let tile = raster::render(
        page,
        &RenderCache::new(),
        &InterpreterSettings::default(),
        &Options { size: Some([5, 5]), offset: [2.0, 3.0], ..Default::default() },
    )
    .unwrap();
    for row in 0..5 {
        assert_eq!(&tile.bytes()[row * 20..(row + 1) * 20], &full.bytes()[((row + 3) * 20 + 2) * 4..((row + 3) * 20 + 7) * 4]);
    }
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn alpha_is_premultiplied_exactly_once() {
    let doc = craft_pdf::open(pdf("/G gs 0 0 1 rg 0 0 40 20 re f", "/Resources << /ExtGState << /G << /ca 0.5 >> >> >>"), None).unwrap();
    let r = raster::render(&doc.pages()[0], &RenderCache::new(), &InterpreterSettings::default(), &Options::default()).unwrap();
    assert_eq!(&r.bytes()[0..4], &[0, 0, 128, 128]);
}
