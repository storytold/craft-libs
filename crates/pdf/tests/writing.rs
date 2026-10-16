#![cfg(feature = "write")]
#![allow(clippy::unwrap_used, clippy::panic)]

use backend::configure::{Archival, PdfVersion};
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
fn writes_pages_metadata_and_selected_version_without_a_renderer() {
    for compress in [false, true] {
        let bytes = document(compress);
        assert!(bytes.starts_with(b"%PDF-1.6"));
        let text = String::from_utf8_lossy(&bytes);
        for value in ["Synthetic export", "Test author", "Craft writer contract", "Original synthetic artwork"] {
            assert!(text.contains(value), "missing {value}");
        }
        let pdf = hayro_syntax::Pdf::new(std::sync::Arc::new(bytes.clone())).unwrap();
        let sizes: Vec<_> = pdf.pages().iter().map(|page| page.render_dimensions()).collect();
        assert_eq!(sizes, vec![(80.0, 40.0), (30.0, 60.0)]);
    }
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn fixed_input_does_not_read_a_clock_or_change_pdf_bytes() {
    assert_eq!(document(false), document(false));
    assert_eq!(document(true), document(true));
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn document_language_is_preserved_in_catalog_and_xmp_when_supplied() {
    use hayro_syntax::object::{Dict, String as PdfString};

    for language in [None, Some("es-CR"), Some("ja")] {
        let mut writer = Writer::new_with_language(Default::default(), Metadata::default(), language.map(str::to_owned));
        writer.document_mut().start_page_with(PageSettings::from_wh(40.0, 20.0).unwrap()).finish();
        let bytes = writer.finish().unwrap();
        let pdf = hayro_syntax::Pdf::new(std::sync::Arc::new(bytes.clone())).unwrap();
        let catalog = pdf.xref().get::<Dict<'_>>(pdf.xref().root_id()).unwrap();
        let actual = catalog.get::<PdfString<'_>>(b"Lang");
        assert_eq!(actual.as_ref().map(|value| value.as_bytes()), language.map(str::as_bytes));

        let xmp = String::from_utf8_lossy(&bytes);
        if let Some(language) = language {
            assert!(xmp.contains("<dc:language>"));
            assert!(xmp.contains(&format!("<rdf:li>{language}</rdf:li>")));
        } else {
            assert!(!xmp.contains("<dc:language>"));
        }
    }
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn incompatible_archival_version_is_a_configuration_error() {
    assert!(matches!(write::configuration(PdfVersion::Pdf17, Some(Archival::A1_B)), Err(WriteError::Configuration(_))));
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn serialization_preserves_archival_validation_diagnostics() {
    let settings =
        backend::SerializeSettings { configuration: write::configuration(PdfVersion::Pdf17, Some(Archival::A2_B)).unwrap(), ..Default::default() };
    // Deliberately missing the metadata required by the selected archival standard.
    let mut writer = Writer::new(settings, Metadata::default());
    writer.document_mut().start_page_with(PageSettings::from_wh(40.0, 20.0).unwrap()).finish();
    let Err(WriteError::Serialization(error)) = writer.finish() else { panic!("expected archival validation errors") };
    assert!(matches!(*error, backend::error::KrillaError::Validation(_)));
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn invalid_fonts_and_variations_are_fallible() {
    assert!(matches!(write::font(b"not a font", 0, None), Err(WriteError::InvalidFont)));
    for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert!(matches!(write::font(&[], 0, Some(&[(*b"wght", value)])), Err(WriteError::FontVariations)));
    }
}

#[derive(Debug, PartialEq, Eq)]
enum AppError {
    InvalidPage,
    Panic,
    Writer,
}

impl From<WriteError> for AppError {
    fn from(error: WriteError) -> Self {
        if matches!(error, WriteError::BackendPanic(_)) { Self::Panic } else { Self::Writer }
    }
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn export_guard_preserves_application_errors() {
    let result: Result<(), AppError> = write::guard(|| Err(AppError::InvalidPage));
    assert_eq!(result, Err(AppError::InvalidPage));
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_native_drawing_panic_is_an_error_and_a_fresh_export_still_works() {
    let result: Result<(), AppError> = write::guard(|| {
        let mut writer = Writer::new(Default::default(), Metadata::default());
        let _page = writer.document_mut().start_page_with(PageSettings::from_wh(40.0, 20.0).unwrap());
        panic!("synthetic app drawing failure");
    });
    assert_eq!(result, Err(AppError::Panic));
    assert!(!document(false).is_empty());
}

#[cfg(feature = "write-pdf")]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn embedded_pages_can_be_written_without_interpretation_or_rasterization() {
    let source = backend::pdf::Pdf::new(std::sync::Arc::new(document(false))).unwrap();
    let source = backend::pdf::PdfDocument::new(std::sync::Arc::new(source));
    let mut writer = Writer::new(Default::default(), Metadata::default());
    let size = Size::from_wh(80.0, 40.0).unwrap();
    let mut page = writer.document_mut().start_page_with(PageSettings::new(size));
    let mut surface = page.surface();
    surface.draw_pdf_page(&source, size, 0);
    surface.finish();
    page.finish();
    let bytes = writer.finish().unwrap();
    let pdf = hayro_syntax::Pdf::new(std::sync::Arc::new(bytes.clone())).unwrap();
    assert_eq!(pdf.pages().len(), 1);
    assert_eq!(pdf.pages()[0].render_dimensions(), (80.0, 40.0));
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn native_unbalanced_surface_cleanup_survives_in_a_subprocess() {
    const CHILD: &str = "CRAFT_PDF_SURFACE_PANIC_PROBE";
    if std::env::var_os(CHILD).is_some() {
        let result: Result<(), AppError> = write::guard(|| {
            let mut writer = Writer::new(Default::default(), Metadata::default());
            let mut page = writer.document_mut().start_page_with(PageSettings::from_wh(40.0, 20.0).unwrap());
            let mut surface = page.surface();
            surface.push_transform(&backend::geom::Transform::from_translate(2.0, 3.0));
            let mut clip = PathBuilder::new();
            clip.push_rect(Rect::from_xywh(0.0, 0.0, 20.0, 10.0).unwrap());
            surface.push_clip_path(&clip.finish().unwrap(), &backend::paint::FillRule::NonZero);
            surface.push_isolated();
            surface.start_tagged(backend::tagging::ContentTag::Other);
            panic!("synthetic app drawing failure with open surface state");
        });
        assert_eq!(result, Err(AppError::Panic));
        assert!(!document(false).is_empty());
        return;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "native_unbalanced_surface_cleanup_survives_in_a_subprocess", "--nocapture"])
        .env(CHILD, "1")
        .output()
        .unwrap();
    assert!(output.status.success(), "child failed: {:?}\n{}", output.status, String::from_utf8_lossy(&output.stderr));
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn native_panic_diagnostics_preserve_bounded_unicode_text() {
    let result: Result<(), WriteError> = write::guard(|| panic!("synthetic app export failure"));
    assert!(matches!(result, Err(WriteError::BackendPanic(message)) if message == "synthetic app export failure"));
    let result: Result<(), WriteError> = write::guard(|| std::panic::panic_any("界".repeat(2048)));
    assert!(matches!(result, Err(WriteError::BackendPanic(message)) if message == "界".repeat(1024)));
    let result: Result<(), WriteError> = write::guard(|| std::panic::panic_any(7_u32));
    assert!(matches!(result, Err(WriteError::BackendPanic(message)) if message == "non-string panic payload"));
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn empty_metadata_stays_absent_and_populated_metadata_stays_present() {
    let export = |metadata| {
        let mut writer = Writer::new(Default::default(), metadata);
        writer.document_mut().start_page_with(PageSettings::from_wh(40.0, 20.0).unwrap()).finish();
        writer.finish().unwrap()
    };
    let empty = export(Metadata { title: Some(String::new()), description: Some(String::new()), ..Default::default() });
    assert_eq!(empty, export(Metadata::default()));
    let empty = String::from_utf8_lossy(&empty);
    for key in [
        "/Title",
        "/Author",
        "/Keywords",
        "/Creator",
        "/Producer",
        "/Subject",
        "/CreationDate",
        "<dc:title>",
        "<dc:creator>",
        "<pdf:Keywords>",
        "<dc:description>",
        "<xmp:CreatorTool>",
    ] {
        assert!(!empty.contains(key), "unexpected empty metadata {key}");
    }
    let full = document(false);
    let full = String::from_utf8_lossy(&full);
    for key in [
        "/Title",
        "/Author",
        "/Keywords",
        "/Creator",
        "/Producer",
        "/Subject",
        "/CreationDate",
        "<dc:title>",
        "<dc:creator>",
        "<pdf:Keywords>",
        "<dc:description>",
        "<xmp:CreatorTool>",
    ] {
        assert!(full.contains(key), "missing populated metadata {key}");
    }
    for title in [None, Some(String::new())] {
        let settings = backend::SerializeSettings {
            configuration: write::configuration(PdfVersion::Pdf17, Some(Archival::A2_B)).unwrap(),
            ..Default::default()
        };
        let mut writer = Writer::new(settings, Metadata { title, ..Default::default() });
        writer.document_mut().start_page_with(PageSettings::from_wh(40.0, 20.0).unwrap()).finish();
        assert!(matches!(writer.finish(), Err(WriteError::Serialization(_))), "empty title must not bypass archival validation");
    }
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

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn empty_language_is_absent_and_keeps_required_language_validation() {
    let mut outputs = Vec::new();
    for language in [None, Some(String::new())] {
        let mut writer = Writer::new_with_language(Default::default(), Metadata::default(), language.clone());
        writer.document_mut().start_page_with(PageSettings::from_wh(40.0, 20.0).unwrap()).finish();
        outputs.push(writer.finish().unwrap());
        let settings = backend::SerializeSettings {
            configuration: write::configuration(PdfVersion::Pdf17, Some(Archival::A2_A)).unwrap(),
            ..Default::default()
        };
        let mut writer = Writer::new_with_language(settings, Metadata::default(), language);
        writer.document_mut().start_page_with(PageSettings::from_wh(40.0, 20.0).unwrap()).finish();
        let Err(WriteError::Serialization(error)) = writer.finish() else { panic!("expected archival errors") };
        let backend::error::KrillaError::Validation(errors) = *error else { panic!("expected validation diagnostics") };
        assert!(
            errors.iter().any(|(error, _)| matches!(error, backend::configure::ValidationError::NoDocumentLanguage)),
            "empty language must not bypass archival language validation"
        );
    }
    assert_eq!(outputs[0], outputs[1], "empty language and absent language must serialize identically");
}
