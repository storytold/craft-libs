//! PdfCraft patch (89) tests: a form that draws itself through a soft mask, a tiling pattern or
//! a Type 3 glyph set inside it is skipped, and the page is not reported truncated. A soft mask,
//! tiling pattern or Type 3 glyph set outside a form draws that form in full when the device
//! interprets it while drawing the form. The tests of a form that draws itself directly or
//! through another form are in `context.rs`.

use crate::context::content_was_truncated;
use crate::font::{Glyph, Type3Glyph};
use crate::pattern::Pattern;
use crate::{
    BlendMode, ClipPath, Context, Device, GlyphDrawMode, Image, InterpreterCache,
    InterpreterSettings, InterpreterWarning, Paint, PathDrawMode, SoftMask, interpret_page,
};
use hayro_syntax::Pdf;
use kurbo::{Affine, BezPath, Rect};
use std::sync::{Arc, Mutex};

/// A PDF with one 100 by 100 page whose resources are `resources` and whose content is
/// `content`, followed by `objects` as objects 5, 6 and so on.
fn pdf(resources: &str, content: &str, objects: &[String]) -> Pdf {
    let mut bodies = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Count 1 /Kids [3 0 R] >>".to_string(),
        format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Contents 4 0 R /Resources {resources} >>"),
        stream("", content),
    ];
    bodies.extend_from_slice(objects);
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in bodies.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", bodies.len() + 1).as_bytes());
    for offset in offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", bodies.len() + 1).as_bytes(),
    );
    Pdf::new(out).expect("synthetic pdf")
}

fn stream(entries: &str, content: &str) -> String {
    format!("<< /Length {} {entries} >>\nstream\n{content}\nendstream", content.len())
}

/// A form 10 by 10 that draws `content` with the resources of whatever draws it.
fn form(content: &str) -> String {
    stream("/Type /XObject /Subtype /Form /BBox [0 0 10 10]", content)
}

/// A soft mask group that draws `content` with `resources`.
fn mask_group(resources: &str, content: &str) -> String {
    stream(
        &format!("/Type /XObject /Subtype /Form /BBox [0 0 100 100] /Group << /S /Transparency >> /Resources {resources}"),
        content,
    )
}

/// A colored tiling pattern whose cell draws `content` with `resources`.
fn tiling(resources: &str, content: &str) -> String {
    stream(
        &format!(
            "/Type /Pattern /PatternType 1 /PaintType 1 /TilingType 1 /BBox [0 0 10 10] /XStep 10 /YStep 10 /Resources {resources}"
        ),
        content,
    )
}

/// A Type 3 font whose glyph `a` is drawn by the procedure in object `proc`, with `resources`.
fn type3(resources: &str, proc: usize) -> String {
    format!(
        "<< /Type /Font /Subtype /Type3 /FontBBox [0 0 1000 1000] /FontMatrix [0.001 0 0 0.001 0 0] /CharProcs << /a {proc} 0 R >> /Encoding << /Type /Encoding /Differences [97 /a] >> /FirstChar 97 /LastChar 97 /Widths [1000] /Resources {resources} >>"
    )
}

/// The paths a device was given, by where they were drawn.
#[derive(Default, Debug, PartialEq)]
struct Drawn {
    paths: usize,
    in_masks: usize,
    in_patterns: usize,
    in_glyphs: usize,
}

/// A device that interprets every soft mask it is given, every tiling pattern it fills with and
/// every Type 3 glyph it draws, each into a device of its own, at the time it is given them. With
/// `defer_glyphs`, it keeps the first Type 3 glyph and interprets it at the next path instead.
#[derive(Default)]
struct Recorder<'a> {
    drawn: Drawn,
    defer_glyphs: bool,
    deferred: Option<Box<Type3Glyph<'a>>>,
}

impl Recorder<'_> {
    fn absorb(&mut self, inner: &Recorder<'_>) -> usize {
        let d = &inner.drawn;
        self.drawn.in_masks += d.in_masks;
        self.drawn.in_patterns += d.in_patterns;
        self.drawn.in_glyphs += d.in_glyphs;
        d.paths
    }
}

impl<'a> Device<'a> for Recorder<'a> {
    fn set_soft_mask(&mut self, mask: Option<SoftMask<'a>>) {
        if let Some(mask) = mask {
            let mut inner = Recorder::default();
            mask.interpret(&mut inner);
            self.drawn.in_masks += self.absorb(&inner);
        }
    }

    fn set_blend_mode(&mut self, _: BlendMode) {}

    fn draw_path(&mut self, _: &BezPath, transform: Affine, paint: &Paint<'a>, _: &PathDrawMode) {
        self.drawn.paths += 1;
        if let Paint::Pattern(pattern) = paint
            && let Pattern::Tiling(tiling) = &**pattern
        {
            let mut inner = Recorder::default();
            tiling.interpret(&mut inner, transform, false);
            self.drawn.in_patterns += self.absorb(&inner);
        }
        if let Some(glyph) = self.deferred.take() {
            let mut inner = Recorder::default();
            glyph.interpret(&mut inner, Affine::IDENTITY, Affine::IDENTITY, paint);
            self.drawn.in_glyphs += self.absorb(&inner);
        }
    }

    fn push_clip_path(&mut self, _: &ClipPath) {}

    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {}

    fn draw_glyph(&mut self, glyph: &Glyph<'a>, transform: Affine, glyph_transform: Affine, paint: &Paint<'a>, _: &GlyphDrawMode) {
        let Glyph::Type3(glyph) = glyph else {
            return;
        };
        if self.defer_glyphs {
            self.deferred = Some(glyph.clone());
            return;
        }
        let mut inner = Recorder::default();
        glyph.interpret(&mut inner, transform, glyph_transform, paint);
        self.drawn.in_glyphs += self.absorb(&inner);
    }

    fn draw_image(&mut self, _: Image<'a, '_>, _: Affine) {}

    fn pop_clip_path(&mut self) {}

    fn pop_transparency_group(&mut self) {}
}

/// Draws the page of `pdf` and returns what was drawn and whether the page was reported
/// truncated.
fn draw(pdf: &Pdf, defer_glyphs: bool) -> (Drawn, bool) {
    let cache = InterpreterCache::new();
    let warnings = Arc::new(Mutex::new(Vec::new()));
    let sink = warnings.clone();
    let settings = InterpreterSettings {
        warning_sink: Arc::new(move |warning| sink.lock().expect("warnings").push(warning)),
        ..InterpreterSettings::default()
    };
    let mut ctx = Context::new(Affine::IDENTITY, Rect::new(0.0, 0.0, 100.0, 100.0), &cache, pdf.xref(), settings);
    let mut device = Recorder { defer_glyphs, ..Recorder::default() };
    let page = &pdf.pages()[0];
    interpret_page(page, &mut ctx, &mut device);
    let warnings = warnings.lock().expect("warnings");
    let truncated = warnings.iter().any(|w| matches!(w, InterpreterWarning::ContentTruncated));
    assert_eq!(truncated, content_was_truncated());
    (device.drawn, truncated)
}

const SQUARE: &str = "0 0 10 10 re f";

// A form that names itself in its own resources is a reference cycle, which hayro-syntax reads
// as a missing object. The forms below have no resources of their own and reach themselves
// through the page's.

#[test]
fn a_soft_mask_set_outside_a_form_draws_that_form() {
    // The mask is set on the page and draws the form the page then draws; the device interprets
    // it while it draws that form.
    let pdf = pdf(
        "<< /XObject << /F 5 0 R >> /ExtGState << /GS << /SMask << /S /Alpha /G 6 0 R >> >> >> >>",
        "/GS gs /F Do",
        &[form(SQUARE), mask_group("<< /XObject << /F 5 0 R >> >>", "/F Do")],
    );
    let (drawn, truncated) = draw(&pdf, false);
    assert!(drawn.in_masks > 0, "{drawn:?}");
    assert_eq!(drawn.paths, 1);
    assert!(!truncated);
}

#[test]
fn a_soft_mask_set_inside_a_form_that_it_draws_skips_that_form() {
    let pdf = pdf(
        "<< /XObject << /F 5 0 R >> /ExtGState << /GS << /SMask << /S /Alpha /G 6 0 R >> >> >> >>",
        "/F Do",
        &[form("/GS gs 0 0 10 10 re f"), mask_group("<< /XObject << /F 5 0 R >> >>", "/F Do")],
    );
    let (drawn, truncated) = draw(&pdf, false);
    assert_eq!(drawn, Drawn { paths: 1, ..Drawn::default() });
    assert!(!truncated);
}

#[test]
fn a_tiling_pattern_set_outside_a_form_draws_that_form() {
    let pdf = pdf(
        "<< /XObject << /F 5 0 R >> /Pattern << /P 6 0 R >> >>",
        "/Pattern cs /P scn /F Do",
        &[form(SQUARE), tiling("<< /XObject << /F 5 0 R >> >>", "/F Do")],
    );
    let (drawn, truncated) = draw(&pdf, false);
    assert!(drawn.in_patterns > 0, "{drawn:?}");
    assert!(!truncated);
}

#[test]
fn a_tiling_pattern_set_inside_a_form_that_it_draws_skips_that_form() {
    let pdf = pdf(
        "<< /XObject << /F 5 0 R >> /Pattern << /P 6 0 R >> >>",
        "/F Do",
        &[form("/Pattern cs /P scn 0 0 10 10 re f"), tiling("<< /XObject << /F 5 0 R >> >>", "/F Do")],
    );
    let (drawn, truncated) = draw(&pdf, false);
    assert_eq!(drawn, Drawn { paths: 1, ..Drawn::default() });
    assert!(!truncated);
}

#[test]
fn a_type3_glyph_shown_outside_a_form_draws_that_form() {
    // The device keeps the glyph shown on the page and interprets it while it draws the form.
    let pdf = pdf(
        "<< /XObject << /F 5 0 R >> /Font << /T 6 0 R >> >>",
        "BT /T 10 Tf (a) Tj ET /F Do",
        &[form(SQUARE), type3("<< /XObject << /F 5 0 R >> >>", 7), stream("", "1000 0 d0 /F Do")],
    );
    let (drawn, truncated) = draw(&pdf, true);
    assert!(drawn.in_glyphs > 0, "{drawn:?}");
    assert!(!truncated);
}

#[test]
fn a_type3_glyph_shown_inside_a_form_that_it_draws_skips_that_form() {
    let pdf = pdf(
        "<< /XObject << /F 5 0 R >> /Font << /T 6 0 R >> >>",
        "/F Do",
        &[
            form("BT /T 10 Tf (a) Tj ET 0 0 10 10 re f"),
            type3("<< /XObject << /F 5 0 R >> >>", 7),
            stream("", "1000 0 d0 /F Do"),
        ],
    );
    let (drawn, truncated) = draw(&pdf, false);
    assert_eq!(drawn, Drawn { paths: 1, ..Drawn::default() });
    assert!(!truncated);
}
