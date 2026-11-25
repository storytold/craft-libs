//! PdfCraft hayro patch (34) and hayro-interpret patch (112), storytold/pdfcraft#7397: every
//! scene the renderer draws (the page, soft masks, tiling cells, image masks, ink passes) is kept
//! to a coverage budget well below the 2^31 values a strip can index, and the interpreter stops
//! drawing once its device has no room left (`Device::out_of_room`), saying the page was cut
//! (`InterpreterWarning::ContentTruncated`). Forms that draw the next form twice, twenty deep
//! (PDFBOX-5415), held 5.4 GB for 76 s before; they now stop at about 240 MB. Adapted from
//! PdfCraft's `coverage_budget` renderer tests.

use crate::{InterpreterSettings, Pdf, RenderCache, RenderSettings, render};
use hayro_interpret::{
    BlendMode, ClipPath, Context, Device, GlyphDrawMode, Image, InterpreterCache,
    InterpreterWarning, Paint, PathDrawMode, SoftMask, font::Glyph, interpret_page,
};
use kurbo::{Affine, BezPath, Rect};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use vello_cpu::color::palette::css::WHITE;

/// A blue circle, filled.
const FILLED_CIRCLE: &str = "0 0 1 rg 100 160 m 133 160 160 133 160 100 c 160 67 133 40 100 40 c 67 40 40 67 40 100 c 40 133 67 160 100 160 c f";

/// A page whose form `F0` draws `F1` twice, `F1` draws `F2` twice, and so on `levels` deep; the
/// last form fills a circle. The page draws `F0` once, so the circle is filled 2^`levels` times.
fn doubling_forms(levels: usize) -> Vec<u8> {
    doubling_forms_then(levels, FILLED_CIRCLE, "")
}

/// [`doubling_forms`] with the last form painting `leaf` instead, and the page painting `after`
/// once the forms are done.
fn doubling_forms_then(levels: usize, leaf: &str, after: &str) -> Vec<u8> {
    doubling_forms_page(
        levels,
        leaf,
        &format!("/F0 Do {after}"),
        "/XObject << /F0 5 0 R >>",
    )
}

/// A page with `content` and `resources` (inside its resource dictionary), and the forms of
/// [`doubling_forms`] from object 5 (`F0`) on, the last one painting `leaf`.
fn doubling_forms_page(levels: usize, leaf: &str, content: &str, resources: &str) -> Vec<u8> {
    let mut objs: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        // Object 4 is the page's content; forms start at 5.
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Contents 4 0 R /Resources << {resources} >> >>"
        ),
        format!(
            "<< /Length {} >>\nstream\n{content}\nendstream",
            content.len()
        ),
    ];
    for level in 0..=levels {
        let (body, resources) = if level == levels {
            (leaf.to_string(), String::new())
        } else {
            (
                "/F Do /F Do".to_string(),
                format!("/Resources << /XObject << /F {} 0 R >> >>", 6 + level),
            )
        };
        objs.push(format!(
            "<< /Type /XObject /Subtype /Form /BBox [0 0 200 200] {resources} /Length {} >>\nstream\n{body}\nendstream",
            body.len()
        ));
    }
    pdf_of(&objs)
}

/// A PDF of the objects `objs`, numbered from 1, with a cross-reference table.
fn pdf_of(objs: &[String]) -> Vec<u8> {
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes(),
    );
    for o in offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objs.len() + 1
        )
        .as_bytes(),
    );
    out
}

/// Interpreter settings whose warning sink sets the flag it returns on `ContentTruncated`.
fn truncation_settings() -> (InterpreterSettings, Arc<AtomicBool>) {
    let truncated = Arc::new(AtomicBool::new(false));
    let seen = truncated.clone();
    let settings = InterpreterSettings {
        warning_sink: Arc::new(move |w| {
            if matches!(w, InterpreterWarning::ContentTruncated) {
                seen.store(true, Ordering::Relaxed);
            }
        }),
        ..Default::default()
    };
    (settings, truncated)
}

/// Renders page 1 at 72 dpi with the default settings, returning its RGBA pixels, whether it said
/// content was left out, and the paths the renderer was asked to draw (`crate::paths_drawn`).
fn render_counted(pdf: Vec<u8>) -> (Vec<u8>, bool, u64) {
    let parsed = Pdf::new(pdf).expect("the fixture parses");
    let (settings, truncated) = truncation_settings();
    let before = crate::paths_drawn();
    let pixmap = render(
        parsed.pages().get(0).expect("the fixture has one page"),
        &RenderCache::new(),
        &settings,
        &RenderSettings {
            bg_color: WHITE,
            ..Default::default()
        },
    );
    let drawn = crate::paths_drawn() - before;
    (
        pixmap.data_as_u8_slice().to_vec(),
        truncated.load(Ordering::Relaxed),
        drawn,
    )
}

/// The share of the pixels of `px` that are not white.
fn inked(px: &[u8]) -> f64 {
    let px = px.as_chunks::<4>().0;
    px.iter().filter(|p| p[0] < 250).count() as f64 / px.len() as f64
}

/// Pixels of `px` that are mostly red.
fn red(px: &[u8]) -> usize {
    px.as_chunks::<4>()
        .0
        .iter()
        .filter(|p| p[0] > 200 && p[2] < 50)
        .count()
}

/// A fill of a hundred stripes one unit wide across the 200 x 200 page, two units apart. Every
/// tile of the canvas holds an edge of one, so one fill stores coverage for the whole canvas
/// (40,000 values at 72 dpi).
fn stripes() -> String {
    let stripes: Vec<String> = (0..100).map(|i| format!("{}.5 0 1 200 re", i * 2)).collect();
    format!("0 g {} f", stripes.join(" "))
}

/// The leaves a page of forms drawing the next form twice, thirteen deep, paints: 2^13 fills of
/// [`stripes`], each storing coverage for the whole canvas.
const BOMB_LEAVES: u64 = 1 << 13;

/// The floor the tests below put under every scene's coverage budget on their thread, in place
/// of the default 2^26 (`MIN_COVERAGE_BUDGET`): 52 fills of [`stripes`] reach it, where the
/// default takes 1,677, which an unoptimised build spends minutes drawing. The budget of a
/// 200 x 200 scene is this floor, as it is the default one at full size
/// (`the_coverage_budget_grows_with_the_canvas`).
const TEST_FLOOR: usize = 1 << 21;

/// Lowers the coverage budget's floor to [`TEST_FLOOR`] on this thread.
fn lower_the_floor() {
    crate::renderer::TEST_COVERAGE_FLOOR.with(|floor| floor.set(Some(TEST_FLOOR)));
}

/// About how many fills of [`stripes`] the coverage budget of a 200 x 200 canvas holds, at the
/// 40,000 values each stores (vello_cpu also keeps room for twice the canvas, so a couple fewer
/// fit).
fn stripes_within_the_budget() -> u64 {
    crate::coverage_budget(200, 200) as u64 / 40_000
}

/// Forms that draw the next form twice fill the rasterizer's coverage long before the page's
/// nested-paint budget runs out. The page now stops at its coverage budget, and the
/// interpreter stops drawing once the renderer has no room: the fills drawn are those the budget
/// holds, not every leaf, what fitted is drawn, and the page says the rest was left out.
#[test]
fn forms_drawing_the_next_twice_stop_at_the_coverage_budget() {
    lower_the_floor();
    assert_eq!(
        crate::coverage_budget(200, 200),
        TEST_FLOOR,
        "a small canvas gets the floor"
    );
    let held = stripes_within_the_budget();
    assert!(
        held < BOMB_LEAVES / 4,
        "the page is past its budget: {held} of {BOMB_LEAVES} leaves fit"
    );

    let (px, cut, drawn) = render_counted(doubling_forms_then(13, &stripes(), ""));
    assert!(cut, "the page says content was left out");
    assert!(
        drawn <= held + 1,
        "drawing stops at the budget: {drawn} paths drawn, {held} fit, {BOMB_LEAVES} leaves"
    );
    assert!(
        drawn + 2 >= held,
        "what the budget holds is drawn: {drawn} paths drawn, {held} fit"
    );
    assert!(inked(&px) > 0.4, "the stripes that fitted show: {}", inked(&px));
}

/// The same forms as a page's soft mask: the mask is drawn as a scene of its own, which has the
/// same budget, and drawing it stops there too, keeping what fitted. Its stripes are white, so
/// the mask lets the page's blue fill through where they are drawn.
#[test]
fn forms_drawing_the_next_twice_in_a_soft_mask_stop_at_its_coverage_budget() {
    lower_the_floor();
    let white_stripes = stripes().replacen("0 g", "1 g", 1);
    let pdf = doubling_forms_page(
        13,
        &white_stripes,
        "/M gs 0 0 1 rg 0 0 200 200 re f",
        "/ExtGState << /M << /SMask << /S /Luminosity /G 5 0 R >> >> >>",
    );
    let held = stripes_within_the_budget();
    let (px, cut, drawn) = render_counted(pdf);
    assert!(cut, "the page says content was left out");
    // The mask's fills, and the page's own fill under it.
    assert!(
        drawn <= held + 2,
        "drawing the mask stops at its budget: {drawn} paths drawn, {held} fit, {BOMB_LEAVES} leaves"
    );
    assert!(
        drawn + 2 >= held,
        "what the mask's budget holds is drawn: {drawn} paths drawn, {held} fit"
    );
    assert!(
        inked(&px) > 0.4,
        "the fill shows through the stripes that fitted: {}",
        inked(&px)
    );
}

/// Nested forms within the budget draw every leaf and say nothing was left out.
#[test]
fn nested_forms_within_the_budget_draw_every_leaf() {
    lower_the_floor();
    let (px, cut, drawn) = render_counted(doubling_forms(6));
    assert!(!cut, "nothing is left out");
    assert_eq!(drawn, 64, "every leaf is drawn");
    assert!(inked(&px) > 0.1, "the circle shows: {}", inked(&px));
    let (_, cut, drawn) = render_counted(doubling_forms_then(5, &stripes(), ""));
    assert!(!cut, "32 canvas-wide fills fit");
    assert_eq!(drawn, 32, "every leaf is drawn");
}

/// The coverage budget is the floor for a small canvas and grows with a large one.
#[test]
fn the_coverage_budget_grows_with_the_canvas() {
    assert_eq!(crate::coverage_budget(1, 1), 1 << 26);
    assert_eq!(
        crate::coverage_budget(2048, 2048),
        1 << 26,
        "16 values a pixel is the floor here"
    );
    assert_eq!(crate::coverage_budget(4096, 4096), 16 * 4096 * 4096);
    assert!(
        crate::coverage_budget(u16::MAX, u16::MAX) > 1 << 31,
        "vello_cpu keeps it to 2^31"
    );
}

/// A device with room for `room` paths, counting those it is handed.
struct Room {
    room: usize,
    paths: usize,
}

impl<'a> Device<'a> for Room {
    fn set_soft_mask(&mut self, _: Option<SoftMask<'a>>) {}
    fn set_blend_mode(&mut self, _: BlendMode) {}
    fn draw_path(&mut self, _: &BezPath, _: Affine, _: &Paint<'a>, _: &PathDrawMode) {
        self.paths += 1;
    }
    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {}
    fn draw_glyph(
        &mut self,
        _: &Glyph<'a>,
        _: Affine,
        _: Affine,
        _: &Paint<'a>,
        _: &GlyphDrawMode,
    ) {
    }
    fn draw_image(&mut self, _: Image<'a, '_>, _: Affine) {}
    fn pop_clip_path(&mut self) {}
    fn pop_transparency_group(&mut self) {}
    fn out_of_room(&self) -> bool {
        self.paths >= self.room
    }
}

/// Interprets page 1 of `pdf` into a [`Room`] with room for `room` paths, returning the paths it
/// was handed and whether the page said content was left out.
fn interpret_into_room(pdf: Vec<u8>, room: usize) -> (usize, bool) {
    let pdf = Pdf::new(pdf).expect("the fixture parses");
    let page = pdf.pages().get(0).expect("the fixture has one page");
    let (settings, truncated) = truncation_settings();
    let cache = InterpreterCache::new();
    let mut ctx = Context::new(
        Affine::IDENTITY,
        Rect::new(0.0, 0.0, 200.0, 200.0),
        &cache,
        page.xref(),
        settings,
    );
    let mut device = Room { room, paths: 0 };
    interpret_page(page, &mut ctx, &mut device);
    (device.paths, truncated.load(Ordering::Relaxed))
}

/// The interpreter stops drawing once its device has no room left, as it does when cancelled,
/// and says content was left out: 2^12 leaves, room for 100.
#[test]
fn the_interpreter_stops_when_its_device_has_no_room() {
    let (paths, cut) = interpret_into_room(doubling_forms(12), 100);
    assert_eq!(paths, 100, "nothing is drawn past the device's room");
    assert!(cut, "the page says content was left out");

    let (paths, cut) = interpret_into_room(doubling_forms(12), usize::MAX);
    assert_eq!(paths, 1 << 12, "with room, every leaf is drawn");
    assert!(!cut, "nothing is left out");
}

/// A device with no room from the start stops before reading the page and says content was left
/// out: the page's first window holds no operator yet, so only the check after it sees the device
/// full.
#[test]
fn a_device_with_no_room_from_the_start_says_the_page_was_left_out() {
    let (paths, cut) = interpret_into_room(doubling_forms(2), 0);
    assert_eq!(paths, 0, "nothing is drawn");
    assert!(cut, "the page says content was left out");
}

/// A page filled with a 200 x 200 tiling pattern whose cell paints `fills - 1` black fills of
/// [`stripes`] and then fills the whole cell red (the rasterizer asks the same room of every
/// path, a canvas' worth): its pixels and whether it said content was left out.
fn stripes_cell(fills: usize) -> (Vec<u8>, bool) {
    let black = stripes();
    let mut cell: Vec<&str> = vec![black.as_str(); fills.saturating_sub(1)];
    cell.push("1 0 0 rg 0 0 200 200 re f");
    let cell = cell.join("\n");
    let content = "/Pattern cs /P0 scn 0 0 200 200 re f";
    let objs = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Contents 4 0 R /Resources << /Pattern << /P0 5 0 R >> >> >>".to_string(),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
        format!(
            "<< /Type /Pattern /PatternType 1 /PaintType 1 /TilingType 1 /BBox [0 0 200 200] /XStep 200 /YStep 200 /Resources << >> /Length {} >>\nstream\n{cell}\nendstream",
            cell.len()
        ),
    ];
    let (px, cut, _) = render_counted(pdf_of(&objs));
    (px, cut)
}

/// A tiling cell whose last fill is the first its coverage budget leaves out says the page was
/// left out. Every other fill past the budget is caught by the check before the next operator;
/// after the last one there was none, so the cell was drawn without it and nothing was said.
/// The cell's fills are counted to the one the budget first refuses, by its colour: the red
/// last fill covers the cell until the cell has one fill too many.
#[test]
fn a_tiling_cell_whose_last_fill_is_left_out_says_so() {
    lower_the_floor();
    let held = stripes_within_the_budget() as usize;
    let mut first_refused = None;
    for fills in held.saturating_sub(4)..held + 4 {
        let (pixels, cut) = stripes_cell(fills);
        if red(&pixels) == 0 {
            first_refused = Some((fills, cut));
            break;
        }
        assert!(!cut, "{fills} fills fit and nothing is said to be left out");
    }
    let (fills, cut) = first_refused.expect("the budget refuses a fill near the estimate");
    assert!(
        fills > held.saturating_sub(4),
        "the cell with the fewest fills, {fills}, draws its last one"
    );
    assert!(
        cut,
        "the cell's last fill, the {fills}th, is left out and the page says so"
    );
}
