//! Integration tests of craft-pdf: contracts.

#![cfg(feature = "raster")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use craft_pdf::{
    interpret::InterpreterSettings,
    raster::{self, Options, RenderCache},
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
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

fn doubling_forms_pdf(levels: usize, leaf: &str, after: &str) -> Arc<Vec<u8>> {
    let content = format!("/F0 Do {after}");
    let mut objs = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Contents 4 0 R /Resources << /XObject << /F0 5 0 R >> >> >>".into(),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
    ];
    for level in 0..=levels {
        let (body, resources) = if level == levels {
            (leaf.to_owned(), String::new())
        } else {
            ("/F Do /F Do".to_owned(), format!("/Resources << /XObject << /F {} 0 R >> >>", 6 + level))
        };
        objs.push(format!("<< /Type /XObject /Subtype /Form /BBox [0 0 200 200] {resources} /Length {} >>\nstream\n{body}\nendstream", body.len()));
    }
    assemble(&objs)
}

fn render_with_coverage_budget(bytes: Arc<Vec<u8>>, max_coverage: Option<usize>) -> (raster::Raster, usize) {
    let doc = craft_pdf::open(bytes, None).unwrap();
    let warnings = Arc::new(AtomicUsize::new(0));
    let count = warnings.clone();
    let settings = InterpreterSettings {
        warning_sink: Arc::new(move |w| {
            if matches!(w, craft_pdf::interpret::InterpreterWarning::ContentTruncated) {
                count.fetch_add(1, Ordering::Relaxed);
            }
        }),
        ..Default::default()
    };
    let raster =
        raster::render(&doc.pages()[0], &RenderCache::new(), &settings, &Options { background: [255; 4], max_coverage, ..Default::default() })
            .unwrap();
    (raster, warnings.load(Ordering::Relaxed))
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn max_coverage_omits_excess_fills_preserving_earlier_content_and_warns() {
    let circle = "0 0 1 rg 100 160 m 133 160 160 133 160 100 c 160 67 133 40 100 40 c 67 40 40 67 40 100 c 40 133 67 160 100 160 c f";
    let marker = "1 0 0 rg 170.5 10.5 20 20 re f";
    let bytes = doubling_forms_pdf(10, circle, marker);
    let blue = |px: &[u8]| px.as_chunks::<4>().0.iter().filter(|p| p[2] > 200 && p[0] < 50).count();
    let red = |px: &[u8]| px.as_chunks::<4>().0.iter().filter(|p| p[0] > 200 && p[2] < 50).count();

    let (whole, whole_warnings) = render_with_coverage_budget(bytes.clone(), None);
    assert!(!whole.diagnostics.content_truncated);
    assert_eq!(whole_warnings, 0);
    assert!(blue(whole.bytes()) > 10_000);
    assert!(red(whole.bytes()) > 50);

    let (kept, kept_warnings) = render_with_coverage_budget(bytes, Some(200_000));
    assert!(kept.diagnostics.content_truncated);
    assert!(kept_warnings > 0);
    assert_eq!(kept.bytes().len(), whole.bytes().len());
    assert!(blue(kept.bytes()) > 10_000);
    assert_eq!(red(kept.bytes()), 0);
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn max_coverage_omits_excess_strokes_and_fast_rects() {
    let cases = [
        (
            "0 0 1 RG 4 w 100 160 m 133 160 160 133 160 100 c 160 67 133 40 100 40 c 67 40 40 67 40 100 c 40 133 67 160 100 160 c S",
            "1 0 0 RG 6 w 170 10 m 180 30 190 10 195 30 c S",
        ),
        ("0 0 1 rg 40.5 40.5 119 119 re f", "1 0 0 rg 170.5 10.5 20 20 re f"),
    ];
    let red = |px: &[u8]| px.as_chunks::<4>().0.iter().filter(|p| p[0] > 200 && p[2] < 50).count();

    for (leaf, marker) in cases {
        let bytes = doubling_forms_pdf(8, leaf, marker);
        let (whole, whole_warnings) = render_with_coverage_budget(bytes.clone(), None);
        assert!(!whole.diagnostics.content_truncated);
        assert_eq!(whole_warnings, 0);
        assert!(red(whole.bytes()) > 50);

        let (kept, kept_warnings) = render_with_coverage_budget(bytes, Some(200_000));
        assert!(kept.diagnostics.content_truncated);
        assert!(kept_warnings > 0);
        assert_eq!(kept.bytes().len(), whole.bytes().len());
        assert_eq!(red(kept.bytes()), 0);
    }
}

/// A soft mask set outside a form, whose group draws that same form (an exporter's one shared cover form), is finite
/// nesting, not a form drawing itself: the mask draws the form whole and the page is not reported truncated.
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn a_soft_mask_drawing_the_form_it_masks_draws_it_whole() {
    let content = "0 0 1 rg /G gs /X Do";
    let cover = "0 0 40 20 re f";
    let mask = "1 g /X Do";
    let bytes = assemble(&[
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 40 20] /Contents 4 0 R /Resources << /XObject << /X 5 0 R >> /ExtGState << /G << /SMask << /S /Luminosity /G 6 0 R >> >> >> >> >>".into(),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
        format!("<< /Type /XObject /Subtype /Form /BBox [0 0 40 20] /Length {} >>\nstream\n{cover}\nendstream", cover.len()),
        format!(
            "<< /Type /XObject /Subtype /Form /BBox [0 0 40 20] /Group << /S /Transparency >> /Resources << /XObject << /X 5 0 R >> >> /Length {} >>\nstream\n{mask}\nendstream",
            mask.len()
        ),
    ]);
    let (r, warnings) = render_with_coverage_budget(bytes, None);
    assert!(!r.diagnostics.content_truncated, "the mask's form is not a form drawing itself");
    assert_eq!(warnings, 0);
    let blue = r.bytes().as_chunks::<4>().0.iter().filter(|p| p[2] > 200 && p[0] < 50).count();
    assert!(blue > r.bytes().len() / 4 / 2, "the white mask shows the blue cover");
}
