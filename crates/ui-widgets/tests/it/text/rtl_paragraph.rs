//! Right-to-left paragraphs: a line that starts with a Latin word keeps the paragraph's direction, and wrapped lines
//! share the right edge, where right-to-left reading starts.

use craft_ui::contract::Show as _;
use craft_ui::display::Text;
use craft_ui::fonts::{FontRef, I18nScripts, SystemFonts, UiFonts, deflated_default_definitions};
use craft_ui::runtime::env::Direction;
use craft_ui::testing::{Fixture, trial};
use egui::Vec2;

/// The framework's Arabic test face, for the Arabic script these paragraphs are written in.
const ARABIC: &[u8] = include_bytes!("../../../../ui-contract/tests/fonts/NotoSansArabic-test.ttf");

/// A right-to-left fixture showing `text` as a `Text` in a column `width` wide, set in the interface face and the
/// Arabic test face.
fn paragraph(text: &'static str, width: f32) -> Fixture {
    let mut fixture = Fixture::new(move |ui| {
        let _ = Text::new(text).show(ui);
    });
    let egui_face = |name: &str| (**deflated_default_definitions().font_data.get(name).expect("an egui default face")).clone();
    UiFonts::new(&I18nScripts)
        .app_face("ui", egui_face("Ubuntu-Light"), &[FontRef::proportional()])
        .app_face("arabic", egui::FontData::from_static(ARABIC), &[FontRef::proportional()])
        .system_fonts(SystemFonts::Off)
        .install(fixture.ctx());
    fixture.direction(Direction::Rtl).room(Vec2::new(width, 300.0));
    fixture.settle();
    fixture
}

/// A painted row: its top, its text in reading order, and each glyph's character, screen left and advance.
type PaintedRow = (f32, String, Vec<(char, f32, f32)>);

/// Each painted row of the text shape whose text holds `part`: its top, its text in reading order, and each glyph's
/// character, screen left and advance.
fn painted_rows(fixture: &mut Fixture, part: &str) -> Vec<PaintedRow> {
    let shapes = fixture.frame_shapes();
    let shown = shapes
        .iter()
        .find_map(|shape| match shape {
            egui::Shape::Text(shown) if shown.galley.text().contains(part) => Some(shown.clone()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("a text holding {part:?} paints"));
    shown
        .galley
        .rows
        .iter()
        .map(|placed| {
            let left = shown.pos.x + placed.pos.x;
            let glyphs: Vec<(char, f32, f32)> = placed.row.glyphs.iter().map(|glyph| (glyph.chr, left + glyph.pos.x, glyph.advance_width)).collect();
            (shown.pos.y + placed.pos.y, glyphs.iter().map(|(chr, ..)| *chr).collect(), glyphs)
        })
        .collect()
}

/// Whether `chr` is a letter of the Arabic block, which Arabic and Persian are written in.
fn is_arabic(chr: char) -> bool {
    ('\u{0600}'..='\u{06FF}').contains(&chr)
}

/// A line of a right-to-left paragraph that starts with a Latin word keeps the paragraph's direction: the word reads
/// first, at the right, not at the left as a line of its own direction would put it.
#[trial]
fn a_right_to_left_paragraph_starting_with_latin_keeps_its_base() {
    let mut fixture = paragraph("این نظر\nPDF درباره فایل است", 300.0);
    let rows = painted_rows(&mut fixture, "PDF");
    let (_, text, glyphs) = rows.iter().find(|(_, text, _)| text.contains("PDF")).expect("a line holds PDF");
    assert!(text.starts_with("PDF"), "the line starts with the Latin word: {text:?}");
    let p = glyphs.iter().find(|(chr, ..)| *chr == 'P').map(|(_, x, _)| *x).expect("a Latin P");
    let arabic: Vec<f32> = glyphs.iter().filter(|(chr, ..)| is_arabic(*chr)).map(|(_, x, _)| *x).collect();
    assert!(!arabic.is_empty(), "the line has Arabic letters: {text:?}");
    assert!(arabic.iter().all(|x| p > *x), "the line stays right to left, the P right of the Arabic letters: {glyphs:?}");
}

/// A right-to-left paragraph wraps against the right edge of its column: its lines end on one right edge inside the
/// column, and the first words read on the top line.
#[trial]
fn wrapped_right_to_left_lines_align_to_the_right_edge() {
    const WIDTH: f32 = 120.0;
    let persian = "این یک نظر طولانی به زبان فارسی است که باید در چند خط شکسته شود";
    let mut fixture = paragraph(persian, WIDTH);
    let rows = painted_rows(&mut fixture, "این");
    assert!(rows.len() > 1, "the paragraph wraps: {rows:?}");
    // Where each line's ink spans: a space at a line's end takes no room at the edge.
    let spans: Vec<(f32, f32)> = rows
        .iter()
        .map(|(_, _, glyphs)| {
            let ink = glyphs.iter().filter(|(chr, ..)| !chr.is_whitespace());
            ink.fold((f32::INFINITY, f32::NEG_INFINITY), |(left, right), (_, x, advance)| (left.min(*x), right.max(x + advance)))
        })
        .collect();
    let right = spans.iter().map(|(_, right)| *right).fold(f32::NEG_INFINITY, f32::max);
    // Glyphs are placed on whole device pixels and their advances are not, so line ends differ by under a pixel.
    let pixel = 1.0 / fixture.ctx().pixels_per_point();
    for ((left, end), (_, text, _)) in spans.iter().zip(&rows) {
        assert!((end - right).abs() <= pixel, "{text:?} ends at the right edge {right}: {end}");
        assert!(*left >= -0.5, "{text:?} stays inside the column: {left}");
    }
    assert!(right <= WIDTH + 0.5, "the lines stay inside the column: {right}");
    let top = rows.iter().min_by(|a, b| a.0.total_cmp(&b.0)).expect("a line");
    assert!(top.1.starts_with("این"), "the first words read on the top line: {:?}", top.1);
}
