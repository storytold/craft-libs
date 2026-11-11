//! Stories of the text selection and the find marks.

use std::ops::Range;

use craft_behavior::kurbo::{Point, Rect};
use craft_ui::canvas::Canvas;
use craft_ui::contract::{Show as _, ShowContainer as _};
use craft_ui::testing::{StoryData, StoryOutput, story};
use egui::{Pos2, Ui};

use super::{PageTextSource, PageTexts, TextMark, TextMarks, TextSelection, TextSpan};
use crate::ui::fixtures::demo_placed;

const LINES: [&str; 3] = ["The quick brown fox", "jumps over the lazy dog", "and keeps running"];
const GLYPH_WIDTH: f64 = 10.0;
const LINE_HEIGHT: f64 = 14.0;

/// Three lines of fixed-width glyphs at the top of the page.
struct Lines;

impl Lines {
    fn starts() -> Vec<usize> {
        LINES.iter().scan(0, |at, line| Some(std::mem::replace(at, *at + line.chars().count()))).collect()
    }

    fn line_of(glyph: usize) -> usize {
        Self::starts().iter().rposition(|start| *start <= glyph).unwrap_or(0)
    }

    fn range_of(line: usize) -> Range<usize> {
        let start = Self::starts()[line];
        start..start + LINES[line].chars().count()
    }

    fn box_of(glyph: usize) -> Rect {
        let line = Self::line_of(glyph);
        let column = glyph - Self::range_of(line).start;
        Rect::new(column as f64 * GLYPH_WIDTH, line as f64 * LINE_HEIGHT, (column + 1) as f64 * GLYPH_WIDTH, (line + 1) as f64 * LINE_HEIGHT)
    }
}

impl PageTextSource for Lines {
    fn glyph_count(&self) -> usize {
        LINES.iter().map(|line| line.chars().count()).sum()
    }

    fn glyph_at(&self, p: Point) -> Option<usize> {
        let line = ((p.y / LINE_HEIGHT).floor().max(0.0) as usize).min(LINES.len() - 1);
        let range = Self::range_of(line);
        let column = ((p.x / GLYPH_WIDTH).floor().max(0.0) as usize).min(range.len() - 1);
        Some(range.start + column)
    }

    fn glyph_box_at(&self, p: Point) -> Option<usize> {
        (0..self.glyph_count()).find(|&glyph| Self::box_of(glyph).contains(p))
    }

    fn line_rects(&self, glyphs: Range<usize>) -> Vec<Rect> {
        (0..LINES.len())
            .filter_map(|line| {
                let range = Self::range_of(line);
                let (start, end) = (glyphs.start.max(range.start), glyphs.end.min(range.end));
                (start < end).then(|| Self::box_of(start).union(Self::box_of(end - 1)))
            })
            .collect()
    }

    fn glyphs_in(&self, rect: Rect) -> Vec<usize> {
        (0..self.glyph_count()).filter(|&glyph| rect.contains(Self::box_of(glyph).center())).collect()
    }

    fn word_at(&self, glyph: usize) -> Option<Range<usize>> {
        let text: Vec<char> = LINES.join("").chars().collect();
        let line = Self::range_of(Self::line_of(glyph));
        let word = |at: usize| text[at] != ' ';
        word(glyph).then(|| {
            let start = (line.start..glyph).rev().take_while(|&at| word(at)).last().unwrap_or(glyph);
            let end = (glyph..line.end).take_while(|&at| word(at)).last().unwrap_or(glyph) + 1;
            start..end
        })
    }

    fn line_at(&self, glyph: usize) -> Option<Range<usize>> {
        Some(Self::range_of(Self::line_of(glyph)))
    }

    fn text_of(&self, glyphs: Range<usize>) -> String {
        LINES.join("").chars().skip(glyphs.start).take(glyphs.len()).collect()
    }

    fn column_text(&self, glyphs: &[usize]) -> String {
        glyphs.iter().map(|&glyph| self.text_of(glyph..glyph + 1)).collect()
    }
}

impl PageTexts for Lines {
    fn page(&self, _: usize) -> Option<&dyn PageTextSource> {
        Some(self)
    }
}

/// Shows `marks` and `selection` over one page placed at the canvas's top left.
fn on_page(ui: &mut Ui, marks: &[TextMark], selection: Option<TextSelection>) -> StoryOutput {
    let pages = demo_placed(1, Pos2::new(24.0, 24.0));
    let lines = Lines;
    let mut control = Some(TextMarks::new(&pages, &lines, marks));
    let stage = craft_ui::testing::stage(ui);
    let shown = Canvas::new("Page").show_at(ui, stage, |ui, body| {
        control.take().map(|control| match selection {
            Some(selection) => control.selection(selection).show_at(ui, body.rect),
            None => control.show_at(ui, body.rect),
        })
    });
    shown.inner.flatten().map(StoryOutput::from).unwrap_or_default()
}

#[story(TextMarks<'static>, covers = [Default])]
fn find_marks(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let marks = [TextMark::new(0, 4..9, false), TextMark::new(0, 38..42, true), TextMark::new(0, 60..63, false)];
    on_page(ui, &marks, None)
}

#[story(TextMarks<'static>, covers = [Default])]
fn run_selected(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    on_page(ui, &[], Some(TextSelection::new(0, TextSpan::run(4, 26))))
}

#[story(TextMarks<'static>, covers = [Default])]
fn column_selected(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let (from, to) = (Point::new(15.0, 2.0), Point::new(65.0, 40.0));
    on_page(ui, &[], Some(TextSelection::new(0, TextSpan::column(from, to))))
}
