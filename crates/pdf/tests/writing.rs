//! Integration tests of craft-pdf: writing.

#![cfg(feature = "write")]
#![allow(clippy::unwrap_used, clippy::panic)]

use backend::configure::PdfVersion;
use backend::geom::{PathBuilder, Rect, Size};
use backend::page::PageSettings;
use backend::paint::Fill;
use craft_pdf::write::{self, Metadata, WriteError, Writer, backend};

fn document(compress: bool) -> Vec<u8> {
    let metadata = Metadata {
        creator: "Craft writer contract".into(),
        producer: "Craft writer contract".into(),
        title: Some("Synthetic export".into()),
        authors: vec!["Test author".into()],
        description: Some("Original synthetic artwork".into()),
        keywords: vec!["craft".into(), "pdf".into()],
        created: Some(backend::metadata::DateTime::new(2026).month(10).day(9)),
        modified: Some(backend::metadata::DateTime::new(2026).month(10).day(12)),
    };
    let settings = backend::SerializeSettings {
        compress_content_streams: compress,
        configuration: write::configuration(PdfVersion::Pdf16, None).unwrap(),
        ..Default::default()
    };
    let mut writer = Writer::new(settings, metadata);
    for (width, height) in [(80.0, 40.0), (30.0, 60.0)] {
        let mut page = writer.document_mut().start_page_with(PageSettings::new(Size::from_wh(width, height).unwrap()));
        let mut surface = page.surface();
        let mut path = PathBuilder::new();
        path.push_rect(Rect::from_xywh(2.0, 3.0, 15.0, 12.0).unwrap());
        surface.set_fill(Some(Fill { paint: backend::color::rgb::Color::new(20, 80, 160).into(), ..Default::default() }));
        surface.draw_path(&path.finish().unwrap());
        surface.finish();
        page.finish();
    }
    writer.finish().unwrap()
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn fixed_input_does_not_read_a_clock_or_change_pdf_bytes() {
    assert_eq!(document(false), document(false));
    assert_eq!(document(true), document(true));
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn retaining_a_writer_after_page_unwind_cannot_serialize_missing_pages() {
    for mode in ["page", "surface", "explicit finish"] {
        let settings = backend::SerializeSettings { enable_tagging: false, ..Default::default() };
        let mut writer = Writer::new(settings, Metadata::default());
        writer.document_mut().start_page_with(PageSettings::from_wh(40.0, 20.0).unwrap()).finish();
        let result: Result<(), WriteError> = if mode == "page" {
            write::guard(|| {
                let _page = writer.document_mut().start_page_with(PageSettings::from_wh(40.0, 20.0).unwrap());
                panic!("synthetic failed middle page");
            })
        } else {
            // Keep the page outside the narrow guard: successful later page cleanup
            // must not clear poison from an unwound or explicitly failed surface.
            let mut page = writer.document_mut().start_page_with(PageSettings::from_wh(40.0, 20.0).unwrap());
            let result = write::guard(|| {
                let mut surface = page.surface();
                surface.push_transform(&backend::geom::Transform::from_translate(2.0, 3.0));
                if mode == "explicit finish" {
                    surface.finish();
                    return Ok(());
                }
                panic!("synthetic failed middle surface");
            });
            page.finish();
            result
        };
        assert!(matches!(result, Err(WriteError::BackendPanic(_))));
        writer.document_mut().start_page_with(PageSettings::from_wh(40.0, 20.0).unwrap()).finish();
        assert!(
            matches!(writer.finish(), Err(WriteError::Serialization(error)) if matches!(*error, backend::error::KrillaError::Poisoned)),
            "a retained failed writer must reject serialization after {mode}"
        );
    }
}
