//! The Select tool on a canvas of placed pages: runs dragged across pages, click units and their drags, and Shift-click,
//! through the canvas's tool host and the fixture's pointer.

use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;

use craft_behavior::kurbo::{Point, Rect};
use craft_pages::ui::fixtures::demo_placed;
use craft_pages::ui::{PageTextSource, PageTexts, PlacedPage, TextSelectState, TextSelectTool, TextSelection, TextSpan};
use craft_ui::canvas::{Canvas, ToolHost, tool_pass};
use craft_ui::contract::ShowContainer as _;
use craft_ui::testing::{Fixture, pointer, trial};
use egui::{Modifiers, Pos2, vec2};

const GLYPH_WIDTH: f64 = 10.0;
const LINE_HEIGHT: f64 = 14.0;
const PAGES: usize = 3;

/// Lines of fixed-width glyphs at the top of a page.
struct Grid {
    lines: &'static [&'static str],
    reverse_cells: bool,
}

impl Grid {
    fn ranges(&self) -> Vec<Range<usize>> {
        let mut start = 0;
        self.lines
            .iter()
            .map(|line| {
                let range = start..start + line.chars().count();
                start = range.end;
                range
            })
            .collect()
    }

    fn chars(&self) -> Vec<char> {
        self.lines.concat().chars().collect()
    }

    fn line_of(&self, glyph: usize) -> Option<(usize, Range<usize>)> {
        self.ranges().into_iter().enumerate().find(|(_, range)| range.contains(&glyph))
    }

    fn box_of(&self, glyph: usize) -> Option<Rect> {
        let (line, range) = self.line_of(glyph)?;
        let column = glyph - range.start;
        let column = if self.reverse_cells && line == 0 {
            if column < 3 {
                column + 4
            } else if column >= 4 {
                column - 4
            } else {
                column
            }
        } else {
            column
        } as f64;
        Some(Rect::new(column * GLYPH_WIDTH, line as f64 * LINE_HEIGHT, (column + 1.0) * GLYPH_WIDTH, (line + 1) as f64 * LINE_HEIGHT))
    }

    /// The parts of `glyphs` on each line, one string per line that has any.
    fn rows(&self, glyphs: impl Fn(usize) -> bool) -> String {
        let chars = self.chars();
        let rows: Vec<String> =
            self.ranges().into_iter().map(|range| range.filter(|&glyph| glyphs(glyph)).map(|glyph| chars[glyph]).collect()).collect();
        rows.into_iter().filter(|row: &String| !row.is_empty()).collect::<Vec<_>>().join("\n")
    }
}

impl PageTextSource for Grid {
    fn glyph_count(&self) -> usize {
        self.ranges().last().map_or(0, |range| range.end)
    }

    fn glyph_at(&self, p: Point) -> Option<usize> {
        if self.reverse_cells {
            return self.glyph_box_at(p);
        }
        if p.y < 0.0 {
            return None;
        }
        let range = self.ranges().into_iter().nth((p.y / LINE_HEIGHT) as usize)?;
        let column = ((p.x / GLYPH_WIDTH).max(0.0) as usize).min(range.len().checked_sub(1)?);
        Some(range.start + column)
    }

    fn glyph_box_at(&self, p: Point) -> Option<usize> {
        (0..self.glyph_count()).find(|&glyph| self.box_of(glyph).is_some_and(|rect| rect.contains(p)))
    }

    fn line_rects(&self, glyphs: Range<usize>) -> Vec<Rect> {
        self.ranges()
            .into_iter()
            .filter_map(|range| {
                let (start, end) = (glyphs.start.max(range.start), glyphs.end.min(range.end));
                if start >= end {
                    return None;
                }
                Some(self.box_of(start)?.union(self.box_of(end - 1)?))
            })
            .collect()
    }

    fn glyphs_in(&self, rect: Rect) -> Vec<usize> {
        (0..self.glyph_count()).filter(|&glyph| self.box_of(glyph).is_some_and(|glyph| rect.contains(glyph.center()))).collect()
    }

    fn word_at(&self, glyph: usize) -> Option<Range<usize>> {
        let chars = self.chars();
        let (_, line) = self.line_of(glyph)?;
        let word = |at: usize| chars[at] != ' ';
        if !word(glyph) {
            return None;
        }
        let start = (line.start..glyph).rev().take_while(|&at| word(at)).last().unwrap_or(glyph);
        let end = (glyph..line.end).take_while(|&at| word(at)).last().unwrap_or(glyph) + 1;
        Some(start..end)
    }

    fn line_at(&self, glyph: usize) -> Option<Range<usize>> {
        self.line_of(glyph).map(|(_, range)| range)
    }

    fn text_of(&self, glyphs: Range<usize>) -> String {
        self.rows(|glyph| glyphs.contains(&glyph))
    }

    fn column_text(&self, glyphs: &[usize]) -> String {
        self.rows(|glyph| glyphs.contains(&glyph))
    }

    fn rows_between(&self, anchor: usize, head: usize, _: f64) -> Option<Vec<usize>> {
        self.reverse_cells.then(|| (anchor.min(head)..=anchor.max(head)).collect())
    }
}

/// The document's pages, `None` where the text is not read yet.
struct Doc(Vec<Option<Grid>>);

impl PageTexts for Doc {
    fn page(&self, page: usize) -> Option<&dyn PageTextSource> {
        self.0.get(page)?.as_ref().map(|grid| grid as &dyn PageTextSource)
    }
}

/// Three pages of text; the middle one unread unless `middle_read`.
fn doc(middle_read: bool) -> Rc<Doc> {
    Rc::new(Doc(vec![
        Some(Grid { lines: &["one two three", "four five six"], reverse_cells: false }),
        middle_read.then_some(Grid { lines: &["seven eight", "nine ten"], reverse_cells: false }),
        Some(Grid { lines: &["eleven twelve", "thirteen"], reverse_cells: false }),
    ]))
}

/// The selection's glyphs on each page.
fn glyphs(selection: &TextSelection, doc: &Doc) -> Vec<Option<Range<usize>>> {
    (0..PAGES).map(|page| doc.page(page).and_then(|text| selection.glyphs_on(page, text))).collect()
}

struct Shown {
    state: TextSelectState,
    placed: Vec<PlacedPage>,
}

/// A canvas showing three placed pages, the Select tool run on its tool pass as an app runs it.
struct Rig {
    fixture: Fixture,
    shown: Rc<RefCell<Shown>>,
}

impl Rig {
    fn new(doc: Rc<Doc>) -> Self {
        let shown = Rc::new(RefCell::new(Shown { state: TextSelectState::default(), placed: Vec::new() }));
        let seen = Rc::clone(&shown);
        let mut fixture = Fixture::new(move |ui| {
            let mut guard = seen.borrow_mut();
            let Shown { state, placed } = &mut *guard;
            let texts: &dyn PageTexts = &*doc;
            let _ = Canvas::new("Pages").show(ui, |ui, body| {
                *placed = demo_placed(PAGES, body.rect.min + vec2(20.0, 20.0));
                if let Some(pass) = tool_pass(&body.env) {
                    let _ = ToolHost::new().run(ui, &pass, &mut TextSelectTool::new(placed.as_slice(), texts, &mut *state));
                }
            });
        });
        fixture.room(vec2(700.0, 1760.0));
        fixture.settle();
        Rig { fixture, shown }
    }

    /// The screen point in the middle of glyph `column` of line `line` on page `page`.
    fn at(&self, page: usize, line: usize, column: usize) -> Pos2 {
        let shown = self.shown.borrow();
        let placed = shown.placed.get(page).expect("the page is placed");
        placed.to_screen(Point::new((column as f64 + 0.5) * GLYPH_WIDTH, (line as f64 + 0.5) * LINE_HEIGHT))
    }

    fn selection(&self) -> TextSelection {
        self.shown.borrow().state.selection.expect("a selection")
    }

    /// Presses `clicks` times at `at` (one click run: the same point, a frame apart), holds the last press, drags to
    /// `to` in four steps and releases.
    fn click_drag(&mut self, at: Pos2, clicks: usize, to: Pos2) {
        self.fixture.frame(vec![pointer::moved(at)]);
        for _ in 1..clicks {
            self.fixture.frame(vec![pointer::press(at)]);
            self.fixture.frame(vec![pointer::release(at)]);
        }
        self.fixture.frame(vec![pointer::press(at)]);
        for step in 1..=4 {
            self.fixture.frame(vec![pointer::moved(at + (to - at) * (step as f32 / 4.0))]);
        }
        self.fixture.frame(vec![pointer::release(to)]);
        self.fixture.frame(Vec::new());
    }

    fn drag(&mut self, from: Pos2, to: Pos2) {
        self.click_drag(from, 1, to);
    }

    fn click(&mut self, at: Pos2) {
        self.fixture.frame(vec![pointer::moved(at)]);
        self.fixture.frame_held(vec![pointer::press(at)]);
        self.fixture.frame(vec![pointer::release(at)]);
        self.fixture.frame(Vec::new());
    }

    /// Waits past the double-click time, so the next press starts a new click run.
    fn pause(&mut self) {
        self.fixture.advance(2.0);
        self.fixture.frame(Vec::new());
    }
}

/// A run dragged from page 0 to page 2 takes the rest of page 0, page 1 whole and page 2 up to the head, whichever way
/// it was dragged; its text waits for an unread middle page; stale and hostile indices are clamped to each page.
#[trial]
fn a_run_dragged_across_pages_takes_the_rest_of_the_first_the_middle_whole_and_the_start_of_the_last() {
    let read = doc(true);
    let mut rig = Rig::new(Rc::clone(&read));
    // "five" on page 0's second line (glyph 18), to the "e" of "twelve" on page 2 (glyph 9).
    let (five, twelve) = (rig.at(0, 1, 5), rig.at(2, 0, 9));
    let whole = "five six\nseven eight\nnine ten\neleven twe";

    rig.drag(five, twelve);
    let forward = rig.selection();
    assert_eq!(forward.page, 0, "the selection starts where the drag did");
    assert!(matches!(forward.span, TextSpan::Run { anchor: 18, head: 9, head_page: 2, end_x: None, .. }), "{forward:?}");
    assert_eq!(forward.pages(), 0..=2);
    assert_eq!(glyphs(&forward, &read), [Some(18..26), Some(0..19), Some(0..10)]);
    assert_eq!(forward.text(&*read).as_deref(), Some(whole));

    rig.pause();
    rig.drag(twelve, five);
    let back = rig.selection();
    assert_eq!(back.page, 2);
    assert!(matches!(back.span, TextSpan::Run { anchor: 9, head: 18, head_page: 0, .. }), "{back:?}");
    assert_eq!(back.pages(), 0..=2);
    assert_eq!(glyphs(&back, &read), glyphs(&forward, &read), "the same glyphs in reading order");
    assert_eq!(back.text(&*read).as_deref(), Some(whole));

    // The middle page not read yet: the drag still reaches page 2, and Copy waits for page 1.
    let unread = doc(false);
    let mut rig = Rig::new(Rc::clone(&unread));
    rig.drag(five, twelve);
    let waiting = rig.selection();
    assert_eq!(waiting.pages(), 0..=2);
    assert_eq!(glyphs(&waiting, &unread), [Some(18..26), None, Some(0..10)]);
    assert_eq!(waiting.text(&*unread), None);
    assert_eq!(waiting.unread_pages(&*unread), vec![1]);
    assert_eq!(forward.unread_pages(&*read), Vec::<usize>::new());

    // Hostile and stale indices: clamped to each page's glyphs, nothing past the end.
    let hostile = TextSelection::new(0, TextSpan::run(usize::MAX, 2, usize::MAX));
    assert_eq!(glyphs(&hostile, &read), [None, Some(0..19), Some(0..21)]);
    assert_eq!(hostile.text(&*read).as_deref(), Some("seven eight\nnine ten\neleven twelve\nthirteen"));
    let stale = TextSelection::new(1, TextSpan::run(30, 1, 40));
    assert_eq!(glyphs(&stale, &read), [None, None, None]);
    assert_eq!(stale.text(&*read).as_deref(), Some(""));
    let past_end = TextSelection::new(1, TextSpan::run(3, 1, 400));
    assert_eq!(glyphs(&past_end, &read), [None, Some(3..19), None]);
}

/// Quick clicks select the word, the line and the page on the press; a 2- or 3-click drag extends by whole units onto
/// other pages both ways, the page unit stays on its page; Shift-click extends a run from its anchor onto any page.
#[trial]
fn click_units_and_their_drags_and_shift_click_extend_a_run() {
    let doc = doc(true);
    let mut rig = Rig::new(Rc::clone(&doc));
    let text = |rig: &Rig| rig.selection().text(&*doc).expect("every page is read");

    // The "w" of "two" on page 0: each press of the run selects before its release.
    let two = rig.at(0, 0, 5);
    rig.fixture.frame(vec![pointer::moved(two)]);
    for (clicks, selected) in [(1, "w"), (2, "two"), (3, "one two three"), (4, "one two three\nfour five six")] {
        rig.fixture.frame(vec![pointer::press(two)]);
        assert_eq!(text(&rig), selected, "{clicks} quick clicks, on the press");
        rig.fixture.frame(vec![pointer::release(two)]);
    }
    rig.fixture.frame(Vec::new());

    // A double-click on "eight" (page 1) dragged onto "thirteen" (page 2): whole words, anchored at the word's start.
    rig.pause();
    rig.click_drag(rig.at(1, 0, 7), 2, rig.at(2, 1, 2));
    let words = rig.selection();
    assert_eq!(words.page, 1);
    assert!(matches!(words.span, TextSpan::Run { anchor: 6, head: 20, head_page: 2, end_x: None, .. }), "{words:?}");
    assert_eq!(text(&rig), "eight\nnine ten\neleven twelve\nthirteen");

    // Dragged back onto "one" (page 0): anchored at the word's end, the head at the reached word's start.
    rig.pause();
    rig.click_drag(rig.at(1, 0, 7), 2, rig.at(0, 0, 1));
    let back = rig.selection();
    assert!(matches!(back.span, TextSpan::Run { anchor: 10, head: 0, head_page: 0, .. }), "{back:?}");
    assert_eq!(glyphs(&back, &doc), [Some(0..26), Some(0..11), None]);
    assert_eq!(text(&rig), "one two three\nfour five six\nseven eight");

    // A triple-click on page 0's first line dragged onto page 1's first: whole lines.
    rig.pause();
    rig.click_drag(rig.at(0, 0, 1), 3, rig.at(1, 0, 2));
    assert_eq!(glyphs(&rig.selection(), &doc), [Some(0..26), Some(0..11), None]);

    // Four clicks on page 1 dragged onto page 2: the page unit stays on its page.
    rig.pause();
    rig.click_drag(rig.at(1, 0, 2), 4, rig.at(2, 0, 2));
    assert_eq!(rig.selection().pages(), 1..=1);
    assert_eq!(text(&rig), "seven eight\nnine ten");

    // A run on page 0 from "five", then Shift-clicks: onto page 2, and back before the anchor on page 0.
    rig.pause();
    rig.drag(rig.at(0, 1, 5), rig.at(0, 1, 7));
    assert_eq!(text(&rig), "fiv");
    rig.pause();
    rig.fixture.hold(Modifiers::SHIFT);
    rig.click(rig.at(2, 0, 9));
    let extended = rig.selection();
    assert_eq!(extended.page, 0, "Shift-click keeps the anchor's page");
    assert!(matches!(extended.span, TextSpan::Run { anchor: 18, head: 9, head_page: 2, .. }), "{extended:?}");
    assert_eq!(text(&rig), "five six\nseven eight\nnine ten\neleven twe");
    rig.pause();
    rig.click(rig.at(0, 0, 1));
    rig.fixture.hold(Modifiers::NONE);
    assert_eq!(rig.selection().pages(), 0..=0);
    assert_eq!(text(&rig), "ne two three\nfour f");
}

#[trial]
fn same_page_drag_keeps_its_visual_endpoint() {
    let doc = doc(true);
    let mut rig = Rig::new(doc);
    rig.drag(rig.at(0, 0, 1), rig.at(0, 0, 8));
    assert!(matches!(rig.selection().span, TextSpan::Run { end_x: Some(_), .. }));
}

#[trial]
fn same_page_word_drag_keeps_the_visual_endpoint_and_both_words() {
    let mut rig = Rig::new(doc(true));
    rig.click_drag(rig.at(0, 0, 1), 2, rig.at(0, 0, 9));
    assert!(matches!(rig.selection().span, TextSpan::Run { anchor: 0, head: 12, end_x: Some(_), .. }));
}

#[trial]
fn a_word_drag_follows_visual_cell_order_when_reading_order_is_reversed() {
    let doc = Rc::new(Doc(vec![Some(Grid { lines: &["one two"], reverse_cells: true }), None, None]));
    let mut rig = Rig::new(doc);
    rig.click_drag(rig.at(0, 0, 1), 2, rig.at(0, 0, 5));
    assert!(matches!(rig.selection().span, TextSpan::Run { anchor: 4, head: 2, end_x: Some(_), .. }));
}

#[trial]
fn shift_click_after_a_column_clears_the_selection() {
    let mut rig = Rig::new(doc(true));
    rig.shown.borrow_mut().state.selection = Some(TextSelection::new(0, TextSpan::column(Point::new(0.0, 0.0), Point::new(30.0, 20.0))));
    rig.fixture.hold(Modifiers::SHIFT);
    rig.click(rig.at(0, 0, 5));
    assert!(rig.shown.borrow().state.selection.is_none());
}
