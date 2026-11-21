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
        let day = |d: &Option<hayro_syntax::object::DateTime>| d.as_ref().map(|d| (d.year, d.month, d.day));
        assert_eq!(day(&pdf.metadata().creation_date), Some((2026, 10, 9)));
        assert_eq!(day(&pdf.metadata().modification_date), Some((2026, 10, 12)));
        assert!(text.contains("<xmp:CreateDate>2026-10-09") && text.contains("<xmp:ModifyDate>2026-10-12"), "XMP dates");
    }
}

/// Without a modification date, the creation date is written as both.
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn the_creation_date_is_the_modification_date_when_none_is_given() {
    let created = Some(backend::metadata::DateTime::new(2026).month(10).day(9));
    let mut writer = Writer::new(Default::default(), Metadata { created, ..Default::default() });
    writer.document_mut().start_page_with(PageSettings::from_wh(40.0, 20.0).unwrap()).finish();
    let pdf = hayro_syntax::Pdf::new(std::sync::Arc::new(writer.finish().unwrap())).unwrap();
    let day = |d: &Option<hayro_syntax::object::DateTime>| d.as_ref().map(|d| (d.year, d.month, d.day));
    assert_eq!(day(&pdf.metadata().modification_date), Some((2026, 10, 9)));
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

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn only_web_and_mail_link_targets_become_uri_links() {
    use backend::annotation::{Annotation, LinkAnnotation};
    use hayro_syntax::object::{Array, Dict, String as PdfString};
    let urls = [
        " https://example.com/deck ",
        "mailto:someone@example.com",
        "javascript:alert(1)",
        "data:text/html,<script>alert(1)</script>",
        "file://attacker.example/share/x",
        "\\\\attacker.example\\share\\x",
        "//attacker.example/share/x",
        "ms-msdt:/id PCWDiagnostic",
        "search-ms:query=x&crumb=location:\\\\attacker.example\\share",
    ];
    let settings = backend::SerializeSettings { configuration: write::configuration(PdfVersion::Pdf17, None).unwrap(), ..Default::default() };
    let mut writer = Writer::new(settings, Metadata::default());
    let mut page = writer.document_mut().start_page_with(PageSettings::new(Size::from_wh(200.0, 400.0).unwrap()));
    for (i, url) in (0u8..).zip(urls) {
        let rect = Rect::from_xywh(10.0, 10.0 + 30.0 * f32::from(i), 100.0, 20.0).unwrap();
        if let Some(target) = write::uri_target(url) {
            page.add_annotation(Annotation::new_link(LinkAnnotation::new(rect, target), None));
        }
    }
    page.finish();
    let bytes = writer.finish().unwrap();
    let pdf = hayro_syntax::Pdf::new(std::sync::Arc::new(bytes)).unwrap();
    let annotations = pdf.pages()[0].raw().get::<Array<'_>>(b"Annots").unwrap();
    let uris: Vec<String> = annotations
        .iter::<Dict<'_>>()
        .filter_map(|a| a.get::<Dict<'_>>(b"A")?.get::<PdfString<'_>>(b"URI"))
        .map(|s| String::from_utf8_lossy(s.as_bytes()).into_owned())
        .collect();
    assert_eq!(uris, ["https://example.com/deck", "mailto:someone@example.com"]);
}

// Original pixel recipes from issue #561 (OPUZZLES); no app, font or image assets.
#[cfg(feature = "filters")]
mod row_prediction {
    use super::*;
    use std::io::{Read, Write};
    use std::sync::Arc;

    use backend::color::device_n::{Colorant, DeviceNSpace, Process};
    use backend::image::{BitsPerComponent, CustomImage, Image, ImageColorspace};
    use flate2::{Compression, read::ZlibDecoder, write::ZlibEncoder};
    use hayro_syntax::object::{Array, Dict, Name, Object, Stream};

    // Original spec-built64x32 RGBA16 PNG: the custom RGB16/alpha recipe below,
    // big-endian samples; PNG color type6, no interlace, Up rows (tag2).
    // Generated only with Python stdlib struct/zlib (including chunk CRCs); no image codec/assets.
    const PNG16: &[u8] = b"\x89\x50\x4e\x47\x0d\x0a\x1a\x0a\x00\x00\x00\x0d\x49\x48\x44\x52\x00\x00\x00\x40\x00\x00\x00\x20\
        \x10\x06\x00\x00\x00\xf2\x0d\xa2\xc7\x00\x00\x01\x7d\x49\x44\x41\x54\x78\xda\xed\xd2\x21\x48\x43\
        \x51\x00\x85\xe1\xbb\x87\xc2\x34\xe8\x40\xc3\x0c\xca\xd0\xe0\x82\xc2\x34\xe8\x40\xc3\x2c\x32\xb0\
        \xac\x08\xc3\xe2\x82\x61\x16\x19\x58\x5e\x11\x86\xc5\x05\xc3\x2c\x32\xb0\xcc\x30\x50\x8b\x0a\x06\
        \x57\x44\xb1\xcc\x20\xa8\x45\x05\x83\x2b\xa2\x58\x66\x10\xd4\xf2\xe4\xf1\x07\x45\xb3\x96\x7b\x4e\
        \xb8\x61\xef\x83\xc1\xbd\xbf\x63\xbc\x80\xf1\x8c\x31\xfe\xe9\x79\xc1\x66\xff\x34\x26\xd4\xea\x9f\
        \xc6\x0b\xb7\xfb\x67\xc0\x44\x3a\x71\xd1\x2e\x5c\xac\x07\x17\xef\xc3\x25\xa2\xb8\xe4\x20\x2e\x35\
        \x8c\x4b\x8f\xe2\x32\xe3\xb8\xec\x04\x2e\x37\x89\x73\xa7\x70\xf9\x14\xae\x30\x8d\x2b\xce\xe0\x4a\
        \xb3\xb8\xf2\x1c\x6e\x6b\x1e\xb7\xb7\x80\x3b\x5c\xc4\x1d\xbb\xb8\xda\x12\xee\x62\x19\x77\xb3\x82\
        \xbb\x5f\xc5\x3d\xae\xe1\x1a\xeb\xb8\xb7\x0d\x9c\x29\xe3\x82\x15\x5c\x68\x07\x17\xde\xc5\x45\x0e\
        \x70\xd1\x2a\x2e\x76\x84\x8b\x9f\xe2\x12\x35\x5c\xf2\x1c\x97\xba\xc2\xa5\xaf\x71\x99\x3b\x5c\xb6\
        \x8e\xcb\x3d\xe0\xdc\x67\x5c\xbe\x81\x2b\xbc\xe2\x8a\x1f\xb8\x92\x87\xdb\x6c\xc2\x6d\xb7\xe0\xf6\
        \xdb\x70\xd5\x0e\xdc\x49\x18\x77\xd6\x8d\xbb\xec\xc5\xdd\xf6\xe3\xea\x03\xb8\xa7\x21\xdc\xcb\x08\
        \xee\x7d\xcc\x71\x02\x5f\x73\x7e\x9c\xce\xaf\x5f\xe5\xfe\xd7\x39\x7f\xfe\xbf\xdf\xbf\x6b\x16\x4e\
        \x01\x28\x00\x4d\x01\x68\x0a\x40\xb3\xf3\x59\x14\x80\x52\xd3\x14\x80\xa6\x00\x34\x05\xa0\x59\xf8\
        \x1c\x0a\x40\xc9\x69\x0a\x40\x53\x00\x9a\x02\xd0\x2c\x7c\x06\x05\xa0\xf4\x34\x05\xa0\x29\x00\x4d\
        \x01\x68\x16\x5e\xbf\x02\x50\x82\x9a\x02\xd0\x14\x80\xa6\x00\x34\x0b\xaf\x5d\x01\x28\x45\x4d\x01\
        \x68\x0a\x40\x53\x00\x9a\x85\xd7\xfd\x09\x61\x07\x13\x9b\x10\xb0\x38\x81\x00\x00\x00\x00\x49\x45\
        \x4e\x44\xae\x42\x60\x82";

    #[derive(Clone, Hash)]
    struct Samples {
        color: Vec<u8>,
        alpha: Option<Vec<u8>>,
        size: (u32, u32),
        bits: BitsPerComponent,
        space: ImageColorspace,
    }

    impl CustomImage for Samples {
        fn color_channel(&self) -> &[u8] {
            &self.color
        }
        fn alpha_channel(&self) -> Option<&[u8]> {
            self.alpha.as_deref()
        }
        fn bits_per_component(&self) -> BitsPerComponent {
            self.bits
        }
        fn size(&self) -> (u32, u32) {
            self.size
        }
        fn icc_profile(&self) -> Option<&[u8]> {
            None
        }
        fn color_space(&self) -> ImageColorspace {
            self.space
        }
        fn device_n_space(&self) -> Option<DeviceNSpace> {
            (self.space == ImageColorspace::DeviceN)
                .then(|| DeviceNSpace::new([Colorant::Process(Process::Magenta), Colorant::Process(Process::Cyan)]).unwrap())
        }
    }

    struct Row {
        name: &'static str,
        samples: Samples,
        rgba: bool,
        png: bool,
        must_predict: bool,
        must_keep_raw: bool,
    }

    struct Benefit {
        name: &'static str,
        ascii: bool,
        predictor: Option<i32>,
        selected: usize,
        unpredicted: usize,
    }

    fn components(space: ImageColorspace) -> usize {
        match space {
            ImageColorspace::Luma => 1,
            ImageColorspace::Rgb => 3,
            ImageColorspace::Cmyk => 4,
            ImageColorspace::DeviceN => 2,
        }
    }

    fn bpc(bits: BitsPerComponent) -> i32 {
        match bits {
            BitsPerComponent::Eight => 8,
            BitsPerComponent::Sixteen => 16,
        }
    }

    fn raw_deflate(data: &[u8]) -> Vec<u8> {
        // The existing writer's raw control: the same zlib level, with no row transform.
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::new(6));
        encoder.write_all(data).unwrap();
        encoder.finish().unwrap()
    }

    fn image(row: &Row) -> Image {
        if row.png {
            Image::from_png(PNG16.to_vec().into(), false).unwrap()
        } else if row.rgba {
            let rgba = row.samples.color.as_chunks::<3>().0.iter().flat_map(|rgb| [rgb[0], rgb[1], rgb[2], 255]).collect();
            Image::from_rgba8(rgba, row.samples.size.0, row.samples.size.1)
        } else {
            Image::from_custom(row.samples.clone(), false).unwrap()
        }
    }

    fn export(row: &Row, ascii: bool) -> Vec<u8> {
        let settings = backend::SerializeSettings {
            ascii_compatible: ascii,
            no_device_cs: false,
            configuration: write::configuration(PdfVersion::Pdf17, None).unwrap(),
            ..Default::default()
        };
        let mut writer = Writer::new(settings, Metadata::default());
        let mut page = writer.document_mut().start_page_with(PageSettings::from_wh(100.0, 80.0).unwrap());
        let mut surface = page.surface();
        surface.draw_image(image(row), Size::from_wh(100.0, 80.0).unwrap());
        surface.finish();
        page.finish();
        writer.finish().unwrap()
    }

    fn check_plane(stream: &Stream<'_>, expected: &[u8], samples: &Samples, colors: usize, ascii: bool) -> Option<i32> {
        let dict = stream.dict();
        assert_eq!(dict.get::<Name<'_>>(b"Subtype").unwrap().as_str(), "Image");
        assert_eq!(dict.get::<i32>(b"Width"), Some(samples.size.0 as i32));
        assert_eq!(dict.get::<i32>(b"Height"), Some(samples.size.1 as i32));
        assert_eq!(dict.get::<i32>(b"BitsPerComponent"), Some(bpc(samples.bits)));
        assert_eq!(stream.decoded().unwrap().as_ref(), expected, "all samples, including 16-bit byte rollover");

        let params = if ascii {
            let filters: Vec<_> = dict.get::<Array<'_>>(b"Filter").unwrap().iter::<Name<'_>>().map(|n| n.as_str().to_owned()).collect();
            assert_eq!(filters, ["ASCIIHexDecode", "FlateDecode"]);
            dict.get::<Array<'_>>(b"DecodeParms").map(|array| {
                let slots: Vec<_> = array.iter::<Object<'_>>().collect();
                assert_eq!(slots.len(), 2, "one DecodeParms slot per filter");
                assert!(matches!(slots[0], Object::Null(_)), "ASCIIHex has no parameters");
                slots[1].clone().into_dict().unwrap()
            })
        } else {
            assert_eq!(dict.get::<Name<'_>>(b"Filter").unwrap().as_str(), "FlateDecode");
            dict.get::<Dict<'_>>(b"DecodeParms")
        };
        params.map(|params| {
            let predictor = params.get::<i32>(b"Predictor").unwrap();
            assert_eq!(predictor, 12);
            assert_eq!(params.get::<i32>(b"Colors"), Some(colors as i32));
            assert_eq!(params.get::<i32>(b"Columns"), Some(samples.size.0 as i32));
            assert_eq!(params.get::<i32>(b"BitsPerComponent"), Some(bpc(samples.bits)));
            if !ascii {
                let encoded = stream.raw_data();
                let mut tagged_rows = Vec::new();
                ZlibDecoder::new(encoded.as_ref()).read_to_end(&mut tagged_rows).unwrap();
                let stride = samples.size.0 as usize * colors * (bpc(samples.bits) as usize / 8) + 1;
                assert_eq!(tagged_rows.len(), stride * samples.size.1 as usize);
                assert!(tagged_rows.chunks_exact(stride).all(|row| row[0] == 2), "PNG Up tag on every row");
            }
            predictor
        })
    }

    fn check_row(row: &Row, ascii: bool) -> Option<Benefit> {
        let pdf = hayro_syntax::Pdf::new(Arc::new(export(row, ascii))).unwrap();
        let page = &pdf.pages()[0];
        assert_eq!(page.render_dimensions(), (100.0, 80.0));
        let resources = &page.resources().x_objects;
        let keys: Vec<_> = resources.keys().collect();
        assert_eq!(keys.len(), 1, "{}: one drawn image", row.name);
        let stream = resources.get::<Stream<'_>>(keys[0].as_ref()).unwrap();
        let predictor = check_plane(&stream, &row.samples.color, &row.samples, components(row.samples.space), ascii);
        let dict = stream.dict();
        if row.samples.space == ImageColorspace::DeviceN {
            let cs: Vec<_> = dict.get::<Array<'_>>(b"ColorSpace").unwrap().iter::<Object<'_>>().collect();
            assert_eq!(cs[0].clone().into_name().unwrap().as_str(), "DeviceN");
            let names: Vec<_> = cs[1].clone().into_array().unwrap().iter::<Name<'_>>().map(|n| n.as_str().to_owned()).collect();
            assert_eq!(names, ["Magenta", "Cyan"], "preserve colorant/sample order");
        } else {
            let name = match row.samples.space {
                ImageColorspace::Luma => "DeviceGray",
                ImageColorspace::Rgb => "DeviceRGB",
                ImageColorspace::Cmyk => "DeviceCMYK",
                ImageColorspace::DeviceN => unreachable!(),
            };
            assert_eq!(dict.get::<Name<'_>>(b"ColorSpace").unwrap().as_str(), name);
        }
        if let Some(alpha) = &row.samples.alpha {
            let mask = dict.get::<Stream<'_>>(b"SMask").unwrap();
            assert_eq!(mask.dict().get::<Name<'_>>(b"ColorSpace").unwrap().as_str(), "DeviceGray");
            check_plane(&mask, alpha, &row.samples, 1, ascii);
        } else {
            assert!(dict.get::<Object<'_>>(b"SMask").is_none(), "{}: no extra alpha plane", row.name);
        }
        if row.must_keep_raw {
            assert_eq!(predictor, None, "{}: prediction must not enlarge tiny/noisy images", row.name);
            let raw = raw_deflate(&row.samples.color);
            let expected_length = if ascii { raw.len() * 2 + raw.len() / 35 } else { raw.len() };
            assert_eq!(stream.raw_data().len(), expected_length, "{}: retain raw deflation", row.name);
        }
        row.must_predict.then(|| {
            let raw = raw_deflate(&row.samples.color);
            Benefit {
                name: row.name,
                ascii,
                predictor,
                // Compare like encoded payloads. Production separately accounts for DecodeParms
                // overhead; the tiny/noisy rows above check its raw fallback.
                selected: stream.raw_data().len(),
                unpredicted: if ascii { raw.len() * 2 + raw.len() / 35 } else { raw.len() },
            }
        })
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn preserves_samples_and_compresses_redundant_rows() {
        let mut rows = Vec::new();
        for pattern in [false, true] {
            let mut color = Vec::with_capacity(1000 * 800 * 3);
            for y in 0..800u32 {
                for x in 0..1000u32 {
                    let rgb = if pattern {
                        [((x * 31 + y * 17) % 256) as u8, ((x * 7 + y * 53) % 256) as u8, ((x * 11 + y * 23) % 256) as u8]
                    } else {
                        [(x * 255 / 999) as u8, (y * 255 / 799) as u8, ((x + y) * 127 / 899) as u8]
                    };
                    color.extend_from_slice(&rgb);
                }
            }
            rows.push(Row {
                name: if pattern { "repeating RGB" } else { "gradient RGB" },
                samples: Samples { color, alpha: None, size: (1000, 800), bits: BitsPerComponent::Eight, space: ImageColorspace::Rgb },
                rgba: true,
                png: false,
                must_predict: true,
                must_keep_raw: false,
            });
        }
        for space in [ImageColorspace::Luma, ImageColorspace::Rgb, ImageColorspace::Cmyk, ImageColorspace::DeviceN] {
            for bits in [BitsPerComponent::Eight, BitsPerComponent::Sixteen] {
                let mut color = Vec::new();
                let mut alpha = Vec::new();
                for y in 0..32u16 {
                    for x in 0..64u16 {
                        for c in 0..components(space) {
                            // Carries cross both low- and high-byte boundaries between rows.
                            let sample = [0x00ffu16, 0x0100, 0xff00, 0xffff, 0][(x as usize + c) % 5].wrapping_add(y * 257);
                            if bits == BitsPerComponent::Sixteen {
                                color.extend_from_slice(&sample.to_be_bytes());
                            } else {
                                color.push(sample as u8);
                            }
                        }
                        let sample = x.wrapping_mul(2053).wrapping_add(y * 257);
                        if bits == BitsPerComponent::Sixteen {
                            alpha.extend_from_slice(&sample.to_be_bytes());
                        } else {
                            alpha.push(sample as u8);
                        }
                    }
                }
                rows.push(Row {
                    name: if bits == BitsPerComponent::Sixteen && space == ImageColorspace::Luma {
                        "16-bit gray rollover"
                    } else {
                        "custom color and alpha"
                    },
                    samples: Samples { color, alpha: Some(alpha), size: (64, 32), bits, space },
                    rgba: false,
                    png: false,
                    must_predict: bits == BitsPerComponent::Sixteen && space == ImageColorspace::Luma,
                    must_keep_raw: false,
                });
            }
        }
        let mut random = 0x561u32;
        let noisy = (0..37 * 29 * 3)
            .map(|_| {
                random ^= random << 13;
                random ^= random >> 17;
                random ^= random << 5;
                random as u8
            })
            .collect();
        for (name, color, size) in [("tiny RGB", vec![7, 129, 253], (1, 1)), ("noisy RGB", noisy, (37, 29))] {
            rows.push(Row {
                name,
                samples: Samples { color, alpha: None, size, bits: BitsPerComponent::Eight, space: ImageColorspace::Rgb },
                rgba: true,
                png: false,
                must_predict: false,
                must_keep_raw: true,
            });
        }

        let png_samples = rows
            .iter()
            .find(|row| row.samples.space == ImageColorspace::Rgb && row.samples.bits == BitsPerComponent::Sixteen)
            .unwrap()
            .samples
            .clone();
        rows.push(Row {
            name: "PNG16 RGB and alpha rollover",
            samples: png_samples,
            rgba: false,
            png: true,
            must_predict: true,
            must_keep_raw: false,
        });

        let mut benefits = Vec::new();
        for row in &rows {
            for ascii in [false, true] {
                if let Some(benefit) = check_row(row, ascii) {
                    benefits.push(benefit);
                }
            }
        }
        // All rows' decoded samples, metadata, masks, ASCII chains and fallback checks run
        // before main's expected failure: it has no row predictor on the gradient.
        for benefit in benefits {
            assert_eq!(benefit.predictor, Some(12), "{} (ASCII={}): sampled rows need PNG Up prediction", benefit.name, benefit.ascii);
            assert!(
                benefit.selected < benefit.unpredicted,
                "{} (ASCII={}): predicted payload must beat raw-deflated payload",
                benefit.name,
                benefit.ascii
            );
        }
    }
}
