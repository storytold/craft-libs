#![cfg(feature = "interpret")]
#![allow(clippy::unwrap_used, clippy::panic)]

use craft_pdf::interpret::{
    BlendMode, CancellationCheck, ClipPath, Context, Device, GlyphDrawMode, Image, InterpreterCache, InterpreterSettings, Paint, PathDrawMode,
    SoftMask, TransformExt, font::Glyph,
};
use kurbo::{Affine, BezPath, Rect};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

#[derive(Default)]
struct Counter {
    paths: usize,
    glyphs: usize,
    annotation_depth: i32,
    stop_at_annotations: Option<Arc<AtomicBool>>,
}

impl Device<'_> for Counter {
    fn begin_annotations(&mut self) {
        self.annotation_depth += 1;
        if let Some(stop) = &self.stop_at_annotations {
            stop.store(true, Ordering::Relaxed);
        }
    }
    fn end_annotations(&mut self) {
        self.annotation_depth -= 1;
    }
    fn set_soft_mask(&mut self, _: Option<SoftMask<'_>>) {}
    fn set_blend_mode(&mut self, _: BlendMode) {}
    fn draw_path(&mut self, _: &BezPath, _: Affine, _: &Paint<'_>, _: &PathDrawMode) {
        self.paths += 1;
    }
    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'_>>, _: BlendMode) {}
    fn draw_glyph(&mut self, _: &Glyph<'_>, _: Affine, _: Affine, _: &Paint<'_>, _: &GlyphDrawMode) {
        self.glyphs += 1;
    }
    fn draw_image(&mut self, _: Image<'_, '_>, _: Affine) {}
    fn pop_clip_path(&mut self) {}
    fn pop_transparency_group(&mut self) {}
}

fn fixture(content: &str, form: &str) -> craft_pdf::syntax::Pdf {
    fixture_with_annotations(content, form, "")
}

fn fixture_with_annotations(content: &str, form: &str, annotations: &str) -> craft_pdf::syntax::Pdf {
    let bytes = format!(
        "%PDF-1.7\n1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj\n\
         2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj\n\
         3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 40 20] /Resources << /Font << /F1 5 0 R >> /XObject << /F0 6 0 R >> >> /Contents 4 0 R {annotations} >> endobj\n\
         4 0 obj << /Length {} >> stream\n{content}\nendstream endobj\n\
         5 0 obj << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> endobj\n\
         6 0 obj << /Type /XObject /Subtype /Form /BBox [0 0 40 20] /Resources << >> /Length {} >> stream\n{form}\nendstream endobj\n\
         trailer << /Root 1 0 R >>\n%%EOF\n",
        content.len(),
        form.len(),
    );
    craft_pdf::open_slice(bytes.as_bytes(), None).unwrap()
}

fn execute(pdf: &craft_pdf::syntax::Pdf, settings: InterpreterSettings) -> Counter {
    let page = pdf.pages().first().unwrap();
    let cache = InterpreterCache::new();
    let mut context = Context::new(page.initial_transform(true).to_kurbo(), Rect::new(0.0, 0.0, 40.0, 20.0), &cache, page.xref(), settings);
    let mut counter = Counter::default();
    craft_pdf::interpret_page(page, &mut context, &mut counter).unwrap();
    counter
}

fn budget(units: usize) -> CancellationCheck {
    let left = AtomicUsize::new(units);
    CancellationCheck::new(move || {
        let mut current = left.load(Ordering::Relaxed);
        loop {
            let Some(next) = current.checked_sub(1) else { return true };
            match left.compare_exchange_weak(current, next, Ordering::Relaxed, Ordering::Relaxed) {
                Ok(_) => return false,
                Err(actual) => current = actual,
            }
        }
    })
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn cancellation_is_sticky_and_shared_by_clones() {
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    let check = CancellationCheck::new(move || observed.fetch_add(1, Ordering::Relaxed) == 1);
    let clone = check.clone();
    assert!(!check.is_cancelled());
    assert!(clone.is_cancelled());
    assert!(check.is_cancelled());
    assert!(clone.is_cancelled());
    assert_eq!(calls.load(Ordering::Relaxed), 2, "the callback would return false again without the shared latch");
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn absent_checkpoints_keep_complete_paths_and_one_long_text_string() {
    let content = format!("{}BT /F1 10 Tf 0 0 Td ({}) Tj ET", "0 0 1 1 re f\n".repeat(80), "A".repeat(4096));
    let counter = execute(&fixture(&content, ""), InterpreterSettings::default());
    assert_eq!(counter.paths, 80);
    assert_eq!(counter.glyphs, 4096);
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn a_work_budget_stops_inside_nested_form_operators() {
    let check = budget(64);
    let settings = InterpreterSettings { cancellation_check: Some(check.clone()), ..Default::default() };
    let counter = execute(&fixture("/F0 Do", &"0 0 1 1 re f\n".repeat(4096)), settings);
    assert!(counter.paths > 0 && counter.paths < 4096, "{} paths", counter.paths);
    assert!(check.is_cancelled());
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn a_work_budget_stops_inside_one_long_text_operator() {
    let content = format!("BT /F1 10 Tf 0 0 Td ({}) Tj ET", "A".repeat(4096));
    let check = budget(64);
    let settings = InterpreterSettings { cancellation_check: Some(check.clone()), ..Default::default() };
    let counter = execute(&fixture(&content, ""), settings);
    assert!(counter.glyphs > 0 && counter.glyphs < 4096, "{} glyphs", counter.glyphs);
    assert!(check.is_cancelled());
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn a_numeric_text_array_is_checked_before_its_final_glyph() {
    let content = format!("BT /F1 10 Tf 0 0 Td [{}(A)] TJ ET", "1 ".repeat(4096));
    let check = budget(64);
    let settings = InterpreterSettings { cancellation_check: Some(check.clone()), ..Default::default() };
    let counter = execute(&fixture(&content, ""), settings);
    assert_eq!(counter.glyphs, 0);
    assert!(check.is_cancelled());
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn a_later_context_cannot_reset_the_shared_budget() {
    let pdf = fixture(&"0 0 1 1 re f\n".repeat(4096), "");
    let settings = InterpreterSettings { cancellation_check: Some(budget(64)), ..Default::default() };
    assert!(execute(&pdf, settings.clone()).paths > 0);
    assert_eq!(execute(&pdf, settings).paths, 0);
}

#[cfg(feature = "raster")]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn raster_never_reports_partial_budget_output_as_success() {
    use craft_pdf::raster::{self, Error, Options, RenderCache};
    let pdf = fixture(&"0 0 1 1 re f\n".repeat(4096), "");
    let settings = InterpreterSettings { cancellation_check: Some(budget(64)), ..Default::default() };
    let page = pdf.pages().first().unwrap();
    assert!(matches!(raster::render(page, &RenderCache::new(), &settings, &Options::default()), Err(Error::Cancelled)));
}

#[cfg(all(feature = "raster", not(target_arch = "wasm32")))]
#[test]
fn raster_checkpoint_panics_are_guarded_before_drawing_or_allocation() {
    use craft_pdf::raster::{self, Error, Options, RenderCache};
    let pdf = fixture("0 0 1 1 re f", "");
    let settings =
        InterpreterSettings { cancellation_check: Some(CancellationCheck::new(|| panic!("synthetic checkpoint failure"))), ..Default::default() };
    let page = pdf.pages().first().unwrap();
    assert!(matches!(raster::render(page, &RenderCache::new(), &settings, &Options::default()), Err(Error::BackendPanic)));
    let mut buffer = vec![0; 40 * 20 * 4];
    assert_eq!(raster::render_into(page, &RenderCache::new(), &settings, &Options::default(), &mut buffer), Err(Error::BackendPanic));
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn cancellation_balances_the_annotation_device_scope() {
    let pdf = fixture_with_annotations("", "", "/Annots [<< /Subtype /Highlight /Rect [0 0 10 10] >>]");
    let page = pdf.pages().first().unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let observed = stop.clone();
    let settings =
        InterpreterSettings { cancellation_check: Some(CancellationCheck::new(move || observed.load(Ordering::Relaxed))), ..Default::default() };
    let cache = InterpreterCache::new();
    let mut context = Context::new(page.initial_transform(true).to_kurbo(), Rect::new(0.0, 0.0, 40.0, 20.0), &cache, page.xref(), settings);
    let mut counter = Counter { stop_at_annotations: Some(stop.clone()), ..Default::default() };
    craft_pdf::interpret_page(page, &mut context, &mut counter).unwrap();
    assert!(stop.load(Ordering::Relaxed));
    assert_eq!(counter.annotation_depth, 0, "cancellation must close the device scope opened by the updated backend");
}
