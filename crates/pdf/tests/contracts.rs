#![cfg(feature = "raster")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use craft_pdf::{
    interpret::InterpreterSettings,
    raster::{self, Error, Options, RenderCache},
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

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
fn transparent_and_white_backgrounds_are_explicit() {
    let doc = craft_pdf::open(pdf("0 0 1 rg 0 0 20 20 re f", ""), None).unwrap();
    let page = &doc.pages()[0];
    let settings = InterpreterSettings::default();
    let a = raster::render(page, &RenderCache::new(), &settings, &Options::default()).unwrap();
    assert_eq!((a.width, a.height), (40, 20));
    assert_eq!(&a.bytes()[0..4], &[0, 0, 255, 255]);
    assert_eq!(&a.bytes()[120..124], &[0, 0, 0, 0]);
    let b = raster::render(page, &RenderCache::new(), &settings, &Options { background: [255; 4], ..Default::default() }).unwrap();
    assert_eq!(&b.bytes()[120..124], &[255; 4]);
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
fn user_unit_scales_rotated_crop_geometry_and_paint_together() {
    let doc = craft_pdf::open(pdf("1 0 0 rg 0 0 40 20 re f", "/CropBox [10 0 40 20] /Rotate 90 /UserUnit 2"), None).unwrap();
    let page = &doc.pages()[0];
    assert_eq!(raster::dimensions(page, &Options::default()).unwrap(), [40, 60]);
    let full = raster::render(page, &RenderCache::new(), &InterpreterSettings::default(), &Options::default()).unwrap();
    assert_eq!((full.width, full.height), (40, 60));
    assert!(full.bytes().as_chunks::<4>().0.iter().all(|pixel| *pixel == [255, 0, 0, 255]));
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn subpixel_page_extent_still_produces_visible_pixels() {
    let doc = craft_pdf::open(pdf("1 0 0 rg 0 0 40 20 re f", "/CropBox [0 0 40 0.2]"), None).unwrap();
    let raster =
        raster::render(&doc.pages()[0], &RenderCache::new(), &InterpreterSettings::default(), &Options { scale: [0.5; 2], ..Default::default() })
            .unwrap();
    assert_eq!((raster.width, raster.height), (20, 1));
    assert!(raster.bytes().as_chunks::<4>().0.iter().any(|pixel| pixel[3] > 0));
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn rounded_up_viewport_draws_the_partial_edge_pixel() {
    let doc = craft_pdf::open(pdf("0 0 1 rg 0 0 40 20 re f", ""), None).unwrap();
    let raster = raster::render(
        &doc.pages()[0],
        &RenderCache::new(),
        &InterpreterSettings::default(),
        &Options { scale: [0.51; 2], size: Some([21, 11]), ..Default::default() },
    )
    .unwrap();
    assert_eq!((raster.width, raster.height), (21, 11));
    let corner = raster.bytes().as_chunks::<4>().0.last().unwrap();
    assert!(corner[3] > 0 && corner[3] < 255, "partial edge pixel: {corner:?}");
    assert_eq!(corner[2], corner[3]);
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn hostile_options_are_rejected_before_buffer_changes() {
    let doc = craft_pdf::open(pdf("", ""), None).unwrap();
    let page = &doc.pages()[0];
    let cache = RenderCache::new();
    for scale in [f32::NAN, f32::INFINITY, -1.0, 0.0] {
        let mut buffer = [123; 4];
        assert_eq!(
            raster::render_into(page, &cache, &InterpreterSettings::default(), &Options { scale: [scale, 1.0], ..Default::default() }, &mut buffer),
            Err(Error::InvalidOptions)
        );
        assert_eq!(buffer, [123; 4]);
    }
    for size in [[0, 1], [8193, 1], [8192, 8192], [u32::MAX, u32::MAX]] {
        assert_eq!(raster::dimensions(page, &Options { size: Some(size), ..Default::default() }), Err(Error::OutputLimit));
    }
    assert_eq!(raster::render_into(page, &cache, &InterpreterSettings::default(), &Options::default(), &mut [0; 4]), Err(Error::BufferLength));
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn pre_cancelled_requests_never_draw() {
    let doc = craft_pdf::open(pdf("0 0 1 rg 0 0 40 20 re f", ""), None).unwrap();
    let settings = InterpreterSettings { cancelled: Some(Arc::new(AtomicBool::new(true))), ..Default::default() };
    let mut bytes = vec![7; 40 * 20 * 4];
    assert_eq!(raster::render_into(&doc.pages()[0], &RenderCache::new(), &settings, &Options::default(), &mut bytes), Err(Error::Cancelled));
    assert!(bytes.iter().all(|b| *b == 7));
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn injected_font_resolver_is_used() {
    let doc = craft_pdf::open(
        pdf("BT /F1 12 Tf 2 5 Td (ABC) Tj ET", "/Resources << /Font << /F1 << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> >> >>"),
        None,
    )
    .unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let original = InterpreterSettings::default().font_resolver;
    let settings = InterpreterSettings {
        font_resolver: Arc::new(move |query| {
            count.fetch_add(1, Ordering::Relaxed);
            original(query)
        }),
        ..Default::default()
    };
    raster::render(&doc.pages()[0], &RenderCache::new(), &settings, &Options::default()).unwrap();
    assert!(calls.load(Ordering::Relaxed) > 0);
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn decode_warnings_reach_caller_and_result() {
    let doc = craft_pdf::open(pdf("q 10 0 0 10 0 0 cm BI /W 32769 /H 8192 /BPC 8 /CS /RGB ID x EI Q", ""), None).unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let settings = InterpreterSettings {
        warning_sink: Arc::new(move |_| {
            count.fetch_add(1, Ordering::Relaxed);
        }),
        ..Default::default()
    };
    let result = raster::render(&doc.pages()[0], &RenderCache::new(), &settings, &Options::default()).unwrap();
    assert!(result.diagnostics.image_decode_failed);
    assert!(calls.load(Ordering::Relaxed) > 0);
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg(not(target_arch = "wasm32"))]
fn backend_callback_panic_is_an_error() {
    let doc = craft_pdf::open(
        pdf("BT /F1 12 Tf 2 5 Td (ABC) Tj ET", "/Resources << /Font << /F1 << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> >> >>"),
        None,
    )
    .unwrap();
    let settings = InterpreterSettings { font_resolver: Arc::new(|_| panic!("synthetic backend failure")), ..Default::default() };
    let error = raster::render(&doc.pages()[0], &RenderCache::new(), &settings, &Options::default()).unwrap_err();
    assert_eq!(error, Error::BackendPanic);
    assert_eq!(error.to_string(), "PDF renderer failed internally");
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn alpha_is_premultiplied_exactly_once() {
    let doc = craft_pdf::open(pdf("/G gs 0 0 1 rg 0 0 40 20 re f", "/Resources << /ExtGState << /G << /ca 0.5 >> >> >>"), None).unwrap();
    let r = raster::render(&doc.pages()[0], &RenderCache::new(), &InterpreterSettings::default(), &Options::default()).unwrap();
    assert_eq!(&r.bytes()[0..4], &[0, 0, 128, 128]);
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn annotation_policy_and_duplicate_ocg_names_remain_independent() {
    let content = "/OC /Left BDC 1 0 0 rg 0 0 20 20 re f EMC /OC /Right BDC 0 0 1 rg 20 0 20 20 re f EMC";
    let appearance = "0 1 0 rg 0 0 40 20 re f";
    let doc=craft_pdf::open(assemble(&[
        "<< /Type /Catalog /Pages 2 0 R /OCProperties << /OCGs [7 0 R 8 0 R] /D << /BaseState /ON >> >> >>".into(),
        "<< /Type /Pages /Count 1 /Kids [3 0 R] >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 40 20] /Contents 4 0 R /Annots [5 0 R] /Resources << /Properties << /Left 7 0 R /Right 8 0 R >> >> >>".into(),
        format!("<< /Length {} >>\nstream\n{content}\nendstream",content.len()),
        "<< /Type /Annot /Subtype /Square /Rect [0 0 40 20] /F 4 /AP << /N 6 0 R >> >>".into(),
        format!("<< /Type /XObject /Subtype /Form /BBox [0 0 40 20] /Length {} >>\nstream\n{appearance}\nendstream",appearance.len()),
        "<< /Type /OCG /Name (Duplicate) >>".into(),
        "<< /Type /OCG /Name (Duplicate) >>".into(),
    ]),None).unwrap();
    let page = &doc.pages()[0];
    let settings = InterpreterSettings { render_annotations: false, ocg_overrides: Arc::new(vec![(7, 0, false)]), ..Default::default() };
    let hidden = raster::render(page, &RenderCache::new(), &settings, &Options::default()).unwrap();
    assert_eq!(&hidden.bytes()[0..4], &[0; 4]);
    assert_eq!(&hidden.bytes()[120..124], &[0, 0, 255, 255]);
    let visible = raster::render(page, &RenderCache::new(), &InterpreterSettings::default(), &Options::default()).unwrap();
    assert_eq!(&visible.bytes()[0..4], &[0, 255, 0, 255]);
    let comments_hidden =
        raster::render(page, &RenderCache::new(), &InterpreterSettings { hide_comments: true, ..Default::default() }, &Options::default()).unwrap();
    assert_eq!(&comments_hidden.bytes()[0..4], &[255, 0, 0, 255]);
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn colour_replacement_is_explicit_and_preserves_alpha() {
    let doc = craft_pdf::open(pdf("/G gs 0 0 0 rg 0 0 40 20 re f", "/Resources << /ExtGState << /G << /ca 0.5 >> >> >>"), None).unwrap();
    let page = &doc.pages()[0];
    let cache = RenderCache::new();
    let original = raster::render(page, &cache, &InterpreterSettings::default(), &Options::default()).unwrap();
    assert_eq!(&original.bytes()[0..4], &[0, 0, 0, 128]);
    let replacement = raster::ColorReplacement { text: [255, 0, 0], background: [255; 3], only_black: true, line_art: true };
    let changed =
        raster::render(page, &cache, &InterpreterSettings::default(), &Options { color_replacement: Some(replacement), ..Default::default() })
            .unwrap();
    assert_eq!(&changed.bytes()[0..4], &[128, 0, 0, 128]);
    let restored = raster::render(page, &cache, &InterpreterSettings::default(), &Options::default()).unwrap();
    assert_eq!(original.bytes(), restored.bytes());
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn malformed_stream_completion_warning_reaches_raster_and_caller() {
    let doc = craft_pdf::open(
        assemble(&[
            "<< /Type /Catalog /Pages 2 0 R >>".into(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 40 20] /Contents 4 0 R >>".into(),
            "<< /Length 1 /Filter /FlateDecode >>\nstream\nx\nendstream".into(),
        ]),
        None,
    )
    .unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let settings = InterpreterSettings {
        warning_sink: Arc::new(move |w| {
            if matches!(w, craft_pdf::interpret::InterpreterWarning::ContentTruncated) {
                count.fetch_add(1, Ordering::Relaxed);
            }
        }),
        ..Default::default()
    };
    let result = raster::render(&doc.pages()[0], &RenderCache::new(), &settings, &Options::default()).unwrap();
    assert!(result.diagnostics.content_truncated);
    assert!(calls.load(Ordering::Relaxed) > 0);
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn preview_settings_and_cache_preserve_final_image_quality() {
    let content = "q 10 0 0 10 20 5 cm /Im0 Do Q q 0.5 0 0 0.5 2 2 cm /Im0 Do Q";
    let pixels = "A".repeat(4 * 4 * 3);
    let doc = craft_pdf::open(assemble(&[
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 40 20] /Contents 4 0 R /Resources << /XObject << /Im0 5 0 R >> >> >>".into(),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
        format!("<< /Type /XObject /Subtype /Image /Width 4 /Height 4 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Length {} >>\nstream\n{pixels}\nendstream", pixels.len()),
    ]), None).unwrap();
    let page = &doc.pages()[0];
    let cache = RenderCache::new();
    let preview = raster::render(page, &cache, &InterpreterSettings { preview: true, ..Default::default() }, &Options::default()).unwrap();
    let final_after_preview = raster::render(page, &cache, &InterpreterSettings::default(), &Options::default()).unwrap();
    let fresh_final = raster::render(page, &RenderCache::new(), &InterpreterSettings::default(), &Options::default()).unwrap();
    assert_ne!(preview.bytes(), fresh_final.bytes(), "the tiny image is skipped only for a preview");
    assert_eq!(final_after_preview.bytes(), fresh_final.bytes(), "preview cache must not degrade final output");
    assert!(fresh_final.bytes().as_chunks::<4>().0.iter().filter(|p| **p == [0x41, 0x41, 0x41, 255]).count() > 50);
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn document_decode_limit_warns_for_pages_and_nested_painting() {
    // ASCIIHex wraps a RunLength packet of 128 spaces. The valid stream is cut to
    // 32 bytes only by the lowered document policy, with no malformed operator.
    let filter = "/Filter [/ASCIIHexDecode /RunLengthDecode]";
    let encoded = "812080";
    for kind in ["page", "form", "pattern", "type3"] {
        let (content, resources, extras) = match kind {
            "page" => (encoded.to_owned(), String::new(), Vec::new()),
            "form" => ("/Fm Do".into(), "/Resources << /XObject << /Fm 5 0 R >> >>".into(), vec![
                format!("<< /Type /XObject /Subtype /Form /BBox [0 0 40 20] /Length {} {filter} >>\nstream\n{encoded}\nendstream", encoded.len())
            ]),
            "pattern" => ("/Pattern cs /P scn 0 0 40 20 re f".into(), "/Resources << /Pattern << /P 5 0 R >> >>".into(), vec![
                format!("<< /Type /Pattern /PatternType 1 /PaintType 1 /TilingType 1 /BBox [0 0 40 20] /XStep 40 /YStep 20 /Resources << >> /Length {} {filter} >>\nstream\n{encoded}\nendstream", encoded.len())
            ]),
            _ => ("BT /F 10 Tf 1 0 0 1 10 10 Tm (A) Tj ET".into(), "/Resources << /Font << /F 5 0 R >> >>".into(), vec![
                "<< /Type /Font /Subtype /Type3 /FontBBox [0 0 500 500] /FontMatrix [0.001 0 0 0.001 0 0] /CharProcs << /A 6 0 R >> /Encoding << /Type /Encoding /Differences [65 /A] >> /FirstChar 65 /LastChar 65 /Widths [500] /Resources << >> >>".into(),
                format!("<< /Length {} {filter} >>\nstream\n{encoded}\nendstream", encoded.len()),
            ]),
        };
        let content_filter = if kind == "page" { filter } else { "" };
        let mut objects = vec![
            "<< /Type /Catalog /Pages 2 0 R >>".into(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
            format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 40 20] /Contents 4 0 R {resources} >>"),
            format!("<< /Length {} {content_filter} >>\nstream\n{content}\nendstream", content.len()),
        ];
        objects.extend(extras);
        let data = assemble(&objects);
        for low in [false, true] {
            let decode_limits = if low { craft_pdf::syntax::DecodeLimits::default().with_max_decoded_stream(32) } else { Default::default() };
            let options = craft_pdf::OpenOptions { decode_limits, ..Default::default() };
            let doc = craft_pdf::open_with_options(data.clone(), None, &options).unwrap();
            let calls = Arc::new(AtomicUsize::new(0));
            let count = calls.clone();
            let settings = InterpreterSettings {
                warning_sink: Arc::new(move |w| {
                    if matches!(w, craft_pdf::interpret::InterpreterWarning::ContentTruncated) {
                        count.fetch_add(1, Ordering::Relaxed);
                    }
                }),
                ..Default::default()
            };
            let result = raster::render(&doc.pages()[0], &RenderCache::new(), &settings, &Options::default()).unwrap();
            assert_eq!(result.diagnostics.content_truncated, low, "{kind}: low={low}");
            assert_eq!(calls.load(Ordering::Relaxed) > 0, low, "{kind}: caller warning");
        }
    }
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn failed_form_decode_reports_incomplete_content() {
    let doc = craft_pdf::open(
        assemble(&[
            "<< /Type /Catalog /Pages 2 0 R >>".into(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 40 20] /Contents 4 0 R /Resources << /XObject << /Fm 5 0 R >> >> >>".into(),
            "<< /Length 6 >>\nstream\n/Fm Do\nendstream".into(),
            "<< /Type /XObject /Subtype /Form /BBox [0 0 40 20] /Length 1 /Filter /DCTDecode >>\nstream\nx\nendstream".into(),
        ]),
        None,
    )
    .unwrap();
    let result = raster::render(&doc.pages()[0], &RenderCache::new(), &InterpreterSettings::default(), &Options::default()).unwrap();
    assert!(result.diagnostics.content_truncated);
}
