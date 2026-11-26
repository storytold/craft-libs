//! `TextSelectTool`: selecting text on a rendered page (Acrobat's Select tool, a craft-ui canvas tool): drag across
//! glyphs in reading order, Alt-drag (or the app's Column Select tool) for the glyphs inside a rectangle,
//! double-click a word, triple-click a line, four clicks the page; the selection highlighted. Find matches are painted
//! by [`TextMarks`], a canvas control the app shows in the page body under any tool.
//!
//! The page's text comes from the app ([`PageTextSource`]: PdfCraft's extracted text layer); the layer keeps no text.

use std::ops::{Range, RangeInclusive};

use craft_behavior::kurbo::{Point, Rect};
use craft_behavior::{CancelReason, Cursor, CursorIcon, Metrics, Sense};
use craft_ui::canvas::{CanvasTool, ToolAction, ToolContext, ToolKey, ToolPath, ToolPointer};
use craft_ui::contract::{Common, Component, Cx, MeasureCx, PaintUi, Role};
use craft_ui::primitives::{Draw, PaintShape, StrokeWidth};
use craft_ui::style::{ComponentStyle, StyleColor, StyleCx};
use egui::{CornerRadius, Pos2, Response, Vec2, pos2};

use super::view::PlacedPage;

#[cfg(feature = "suite")]
pub(crate) mod stories;

/// A page's text as the selection reads it, glyph by glyph in reading order. Rectangles are page points (origin
/// top-left).
pub trait PageTextSource {
    /// The number of glyphs on the page.
    fn glyph_count(&self) -> usize;
    /// The glyph nearest `p` within a line's height, if any: where a press or drag lands.
    fn glyph_at(&self, p: Point) -> Option<usize>;
    /// The glyph whose box holds `p`, if any (exactly, no tolerance): where the pointer is over text and shows the
    /// I-beam.
    fn glyph_box_at(&self, p: Point) -> Option<usize>;
    /// One rectangle per line piece of `glyphs`.
    fn line_rects(&self, glyphs: Range<usize>) -> Vec<Rect>;
    /// The glyphs whose boxes `rect` holds, in reading order.
    fn glyphs_in(&self, rect: Rect) -> Vec<usize>;
    /// The word around a glyph, as a glyph range.
    fn word_at(&self, glyph: usize) -> Option<Range<usize>>;
    /// The line around a glyph, as a glyph range.
    fn line_at(&self, glyph: usize) -> Option<Range<usize>>;
    /// The text of `glyphs` as Copy puts it on the clipboard (lines joined with newlines).
    fn text_of(&self, glyphs: Range<usize>) -> String;
    /// The text of a column selection, one line per row.
    fn column_text(&self, glyphs: &[usize]) -> String;
    /// The rows of the page between glyphs `anchor` and `head`, for a drag released at `end_x`; `None` when the
    /// page uses reading order or does not supply visual rows.
    fn rows_between(&self, anchor: usize, head: usize, end_x: f64) -> Option<Vec<usize>> {
        let _ = (anchor, head, end_x);
        None
    }
}

/// What a selection covers: a run from its page on to any other, or a column on its page.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum TextSpan {
    /// Reading order, between two glyphs (inclusive), anchor where the drag started.
    #[non_exhaustive]
    Run {
        /// Index of the glyph where the drag started.
        anchor: usize,
        /// Index of the glyph the drag has reached.
        head: usize,
        /// The page of the glyph the drag has reached; the selection's page is where it started.
        head_page: usize,
        /// Where on its line a same-page drag was released, in page points.
        end_x: Option<f64>,
    },
    /// The glyphs inside a rectangle (page points), drawn from the corner the drag started at.
    #[non_exhaustive]
    Column {
        /// Corner of the rectangle where the drag started, in page points.
        from: Point,
        /// Corner of the rectangle where the drag has reached, in page points.
        to: Point,
    },
}

impl TextSpan {
    /// A run from the glyph `anchor` on the selection's page to the glyph `head` on page `head_page`.
    pub fn run(anchor: usize, head_page: usize, head: usize) -> Self {
        TextSpan::Run { anchor, head, head_page, end_x: None }
    }

    /// The column from the page point `from` to the page point `to`.
    pub fn column(from: Point, to: Point) -> Self {
        TextSpan::Column { from, to }
    }
}

/// A text selection: a run in reading order from its page on to any other, or a column on its page.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct TextSelection {
    /// Page index, 0-based: where the selection started.
    pub page: usize,
    /// The selected glyphs.
    pub span: TextSpan,
}

impl TextSelection {
    /// The selection `span`, started on page `page`.
    pub fn new(page: usize, span: TextSpan) -> Self {
        TextSelection { page, span }
    }

    /// A run's ends as `(page, glyph)`, anchor then head; `None` for a column.
    fn run_ends(&self) -> Option<((usize, usize), (usize, usize))> {
        let TextSpan::Run { anchor, head, head_page, .. } = self.span else { return None };
        Some(((self.page, anchor), (head_page, head)))
    }

    /// A run's ends in reading order, first then last: `(page, glyph)` pairs compare in reading order.
    fn reading_order(&self) -> Option<((usize, usize), (usize, usize))> {
        let (anchor, head) = self.run_ends()?;
        Some(if anchor <= head { (anchor, head) } else { (head, anchor) })
    }

    /// The pages the selection covers, from its first to its last.
    pub fn pages(&self) -> RangeInclusive<usize> {
        match self.reading_order() {
            Some(((first, _), (last, _))) => first..=last,
            None => self.page..=self.page,
        }
    }

    /// The glyphs of the selection on `page`: the rest of its first page, every page between whole, its last page up
    /// to its last glyph, whichever way the drag went. `None` when it covers nothing there, and for a column, whose
    /// glyphs are not one range ([`rects_on`](Self::rects_on) and [`text`](Self::text) read it).
    pub fn glyphs_on(&self, page: usize, text: &dyn PageTextSource) -> Option<Range<usize>> {
        let ((first_page, first), (last_page, last)) = self.reading_order()?;
        if !(first_page..=last_page).contains(&page) {
            return None;
        }
        // The indices may be stale (the page's text was read again) or hostile: every one is clamped to the page.
        let count = text.glyph_count();
        let start = if page == first_page { first.min(count) } else { 0 };
        let end = if page == last_page { last.saturating_add(1).min(count) } else { count };
        Some(start..end).filter(|glyphs| !glyphs.is_empty())
    }

    /// The selection on `page` as rectangles, one per line piece (for highlighting and markup quads).
    // pdfcraft: crates/ui-egui/src/canvas/selection.rs:29 `Selection::rects`.
    pub fn rects_on(&self, page: usize, text: &dyn PageTextSource) -> Vec<Rect> {
        match self.span {
            TextSpan::Column { from, to } if page == self.page => glyph_rects(text, &text.glyphs_in(Rect::from_points(from, to))),
            TextSpan::Column { .. } => Vec::new(),
            TextSpan::Run { .. } => match self.rows_on(page, text) {
                Some(rows) => glyph_rects(text, &rows),
                None => self.glyphs_on(page, text).map(|glyphs| text.line_rects(glyphs)).unwrap_or_default(),
            },
        }
    }

    /// The glyphs a one-page run selects by rows, when the page says so.
    fn rows_on(&self, page: usize, text: &dyn PageTextSource) -> Option<Vec<usize>> {
        let TextSpan::Run { anchor, head, end_x: Some(end_x), .. } = self.span else { return None };
        let ((first, _), (last, _)) = self.reading_order()?;
        (first == page && last == page).then(|| text.rows_between(anchor, head, end_x)).flatten()
    }

    /// The selected text on `page`, one of the pages the selection covers.
    fn text_on(&self, page: usize, text: &dyn PageTextSource) -> String {
        match self.span {
            TextSpan::Column { from, to } => text.column_text(&text.glyphs_in(Rect::from_points(from, to))),
            TextSpan::Run { .. } => match self.rows_on(page, text) {
                Some(rows) => text.column_text(&rows),
                None => self.glyphs_on(page, text).map(|glyphs| text.text_of(glyphs)).unwrap_or_default(),
            },
        }
    }

    /// The selected text, as Copy puts it on the clipboard: the covered pages' parts joined with newlines. `None`
    /// while a covered page is unread ([`unread_pages`](Self::unread_pages) says which).
    // pdfcraft: crates/ui-egui/src/canvas/selection.rs:37 `Selection::text`.
    pub fn text(&self, texts: &dyn PageTexts) -> Option<String> {
        let parts = self.pages().map(|page| texts.page(page).map(|text| self.text_on(page, text))).collect::<Option<Vec<_>>>()?;
        Some(parts.into_iter().filter(|part| !part.is_empty()).collect::<Vec<_>>().join("\n"))
    }

    /// The covered pages whose text is not read yet.
    pub fn unread_pages(&self, texts: &dyn PageTexts) -> Vec<usize> {
        self.pages().filter(|&page| texts.page(page).is_none()).collect()
    }
}

/// One rectangle per line piece of `glyphs`, taken as runs of consecutive indices.
fn glyph_rects(text: &dyn PageTextSource, glyphs: &[usize]) -> Vec<Rect> {
    runs(glyphs).into_iter().flat_map(|run| text.line_rects(run)).collect()
}

/// `glyphs` as ranges of consecutive indices.
fn runs(glyphs: &[usize]) -> Vec<Range<usize>> {
    let mut runs: Vec<Range<usize>> = Vec::new();
    for &glyph in glyphs {
        match runs.last_mut() {
            Some(run) if run.end == glyph => run.end = run.end.saturating_add(1),
            _ => runs.push(glyph..glyph.saturating_add(1)),
        }
    }
    runs
}

/// The glyphs `clicks` quick clicks select around `glyph`: 2 the word, 3 the line, 4 or more the page.
// pdfcraft: crates/ui-egui/src/canvas/selection.rs:47 `click_unit`.
pub fn click_unit(text: &dyn PageTextSource, clicks: u32, glyph: usize) -> Option<Range<usize>> {
    match clicks {
        2 => text.word_at(glyph),
        3 => text.line_at(glyph),
        4.. => Some(0..text.glyph_count()).filter(|page| !page.is_empty()),
        _ => None,
    }
}

/// A highlight drawn with the selection: find matches, the current one stronger.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct TextMark {
    /// Page index, 0-based.
    pub page: usize,
    /// The glyphs to highlight.
    pub glyphs: Range<usize>,
    /// Whether this is the current find match, drawn stronger.
    pub current: bool,
}

impl TextMark {
    /// The match of `glyphs` on page `page`, the current match when `current`.
    pub fn new(page: usize, glyphs: Range<usize>, current: bool) -> Self {
        TextMark { page, glyphs, current }
    }
}

/// The text of the document's pages, for the tool: the pages whose text the app has (PdfCraft extracts it in the
/// background; a page without it yet selects nothing).
pub trait PageTexts {
    /// The text of page `page`, if it has been extracted yet.
    fn page(&self, page: usize) -> Option<&dyn PageTextSource>;
}

/// The tool's state, kept by the app per document view: the selection (view state, applied in place) and the gesture
/// in progress.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct TextSelectState {
    /// The selection, if any.
    pub selection: Option<TextSelection>,
    /// The drag in progress: the selection's page and where it started (a run from the anchor to the pressed glyph,
    /// or the column's first corner).
    gesture: Option<(usize, TextSpan)>,
    /// The press's click count, and the selection from before the gesture (Esc brings it back).
    clicks: u32,
    before: Option<TextSelection>,
    dragged: bool,
}

/// The colours and radii of the selection and the find marks.
#[derive(Clone, Debug, PartialEq)]
struct MarkLook {
    selection: StyleColor,
    found: StyleColor,
    current: StyleColor,
    radius: CornerRadius,
    /// How far the current match grows past its glyphs.
    current_grow: f32,
    /// The accent stroke: the column selection's outline and the underline of spoken text.
    column_outline: StyleColor,
}

impl MarkLook {
    fn of(cx: &StyleCx<'_>) -> Self {
        let colors = &cx.tokens.color;
        MarkLook {
            selection: cx.color(colors.state.text_selection),
            found: cx.color(colors.state.r#match),
            current: cx.color(colors.state.match_current),
            radius: cx.field().radius,
            current_grow: cx.tokens.stroke.thin,
            column_outline: cx.color(colors.accent.solid),
        }
    }
}

/// One filled mark, `rect` grown by `grow`: the one fill of every mark.
fn fill_mark(draw: &Draw<'_>, look: &MarkLook, rect: egui::Rect, fill: StyleColor, grow: f32) {
    draw.fill(&PaintShape::Rect(rect.expand(grow), look.radius), fill);
}

/// Paints the selection, find and area marks of one placed page: the selection fill, the match fills, the current match
/// grown. The one painter of all: [`TextMarks`] paints the matches and areas and the tool the selection through it.
/// Without `text` (a page whose text is not there, or none at all) only the `areas` paint.
fn paint_marks(
    draw: &Draw<'_>,
    look: &MarkLook,
    placed: &PlacedPage,
    text: Option<&dyn PageTextSource>,
    selection: Option<&TextSelection>,
    marks: &[TextMark],
    areas: &[MarkArea],
) {
    let (top, bottom) = placed.band;
    let band = egui::Rect::from_min_max(pos2(placed.rect.min.x, placed.rect.min.y + top), pos2(placed.rect.max.x, placed.rect.min.y + bottom));
    let screen = |r: Rect| egui::Rect::from_two_pos(placed.to_screen(Point::new(r.x0, r.y0)), placed.to_screen(Point::new(r.x1, r.y1)));
    draw.clipped(band, |draw| {
        if let Some(text) = text {
            for mark in marks.iter().filter(|mark| mark.page == placed.page) {
                let (fill, grow) = if mark.current { (look.current, look.current_grow) } else { (look.found, 0.0) };
                for rect in text.line_rects(mark.glyphs.clone()) {
                    fill_mark(draw, look, screen(rect), fill, grow);
                }
            }
        }
        for area in areas.iter().filter(|area| area.page == placed.page) {
            let rect = screen(area.rect);
            let (fill, grow) = match area.kind {
                MarkKind::Selection => (look.selection, 0.0),
                MarkKind::Found => (look.found, 0.0),
                MarkKind::Current | MarkKind::Spoken => (look.current, look.current_grow),
            };
            fill_mark(draw, look, rect, fill, grow);
            if area.kind == MarkKind::Spoken {
                draw.stroke(&PaintShape::Path(vec![pos2(rect.min.x, rect.max.y), rect.max]), StrokeWidth::Thin, look.column_outline);
            }
        }
        if let (Some(text), Some(selection)) = (text, selection) {
            for rect in selection.rects_on(placed.page, text) {
                fill_mark(draw, look, screen(rect), look.selection, 0.0);
            }
        }
    });
}

/// Acrobat's Select tool on the page view, a craft-ui [`CanvasTool`] the app runs in its `ToolHost` over the page
/// view's canvas; built per pass over the pass's placed pages and the app's [`TextSelectState`].
///
/// - Press and drag with the primary button selects ([`TextSpan::Run`], anchored at the press's glyph, its head on
///   whichever page the pointer is over); with Alt held (or [`column`](Self::column) on: the Column Select tool) it
///   selects a column on the pressed page, its rectangle outlined while dragging. The press's click count is
///   [`ToolPointer::clicks`]: 2 the word, 3 the line, 4 the page ([`click_unit`]); dragging after 2 or 3 extends by
///   whole units onto other pages too, the page unit stays on its page. Shift-click extends the run from its anchor
///   onto any page. The cursor is the I-beam over glyphs, the arrow elsewhere. A press off any glyph clears the
///   selection.
/// - Keys while the canvas has the keyboard: Shift with the arrows moves a run's head on its page by a glyph (Shift
///   and Alt by a word; left and right swap in a right-to-left interface), Escape clears. Select All and Copy are the
///   app's commands (they read [`TextSelection::text`]).
/// - Its command form is the app's Select All command: the tool selects, so it is not freehand.
/// - The selection is reported to assistive technology through the page view's accessibility bridge.
pub struct TextSelectTool<'a> {
    pages: &'a [PlacedPage],
    texts: &'a dyn PageTexts,
    state: &'a mut TextSelectState,
    column: bool,
    changed: bool,
}

impl<'a> TextSelectTool<'a> {
    /// A tool over the placed `pages`, reading text from `texts` and keeping its gesture in `state`.
    pub fn new(pages: &'a [PlacedPage], texts: &'a dyn PageTexts, state: &'a mut TextSelectState) -> Self {
        TextSelectTool { pages, texts, state, column: false, changed: false }
    }

    /// The Column Select tool is active.
    pub fn column(mut self, on: bool) -> Self {
        self.column = on;
        self
    }

    /// Whether the selection changed this pass.
    pub fn changed(&self) -> bool {
        self.changed
    }

    /// The shown part of the page under `at`.
    fn page_at(&self, at: Pos2) -> Option<&'a PlacedPage> {
        let pages: &'a [PlacedPage] = self.pages;
        pages.iter().find(|placed| {
            let (top, bottom) = placed.band;
            placed.rect.contains(at) && at.y >= placed.rect.min.y + top && at.y <= placed.rect.min.y + bottom
        })
    }

    fn set(&mut self, selection: Option<TextSelection>) {
        if self.state.selection != selection {
            self.state.selection = selection;
            self.changed = true;
        }
    }

    fn end_gesture(&mut self) {
        self.state.gesture = None;
        self.state.clicks = 0;
        self.state.before = None;
        self.state.dragged = false;
    }

    /// The run a 2-, 3- or 4-click drag selects from the unit pressed (around glyph `origin` on `page`) to glyph `hit`
    /// on `head_page`: whole units both ways in `(page, glyph)` order; the four-click page unit stays on its page.
    /// `None` for a single-click drag, or when the pressed glyph has no unit.
    fn unit_run(&self, page: usize, origin: usize, head_page: usize, hit: usize, end_x: f64) -> Option<TextSpan> {
        let clicks = self.state.clicks;
        if clicks < 2 {
            return None;
        }
        let unit = click_unit(self.texts.page(page)?, clicks, origin).filter(|unit| !unit.is_empty())?;
        if clicks >= 4 {
            return Some(TextSpan::run(unit.start, page, unit.end - 1));
        }
        let reached = self
            .texts
            .page(head_page)
            .and_then(|text| click_unit(text, clicks, hit))
            .filter(|unit| !unit.is_empty())
            .unwrap_or(hit..hit.saturating_add(1));
        if head_page == page && end_x.is_finite() {
            let text = self.texts.page(page)?;
            let outer = |range: &Range<usize>| {
                let first = range.start;
                let last = range.end - 1;
                let a = text.line_rects(first..first.saturating_add(1)).into_iter().next()?;
                let b = text.line_rects(last..last.saturating_add(1)).into_iter().next()?;
                Some(if a.center().x <= b.center().x { ((first, a), (last, b)) } else { ((last, b), (first, a)) })
            };
            if let (Some((left, right)), Some((to_left, to_right))) = (outer(&unit), outer(&reached)) {
                let dy = to_left.1.center().y - left.1.center().y;
                let later =
                    if dy.abs() > left.1.height().min(to_left.1.height()) / 2.0 { dy > 0.0 } else { to_left.1.center().x >= left.1.center().x };
                let (anchor, head) = if later { (left.0, to_right.0) } else { (right.0, to_left.0) };
                if text.rows_between(anchor, head, end_x).is_some() {
                    return Some(TextSpan::Run { anchor, head, head_page, end_x: Some(end_x) });
                }
            }
        }
        let span = if (head_page, reached.start) < (page, unit.start) {
            TextSpan::run(unit.end - 1, head_page, reached.start)
        } else if head_page == page {
            TextSpan::run(unit.start, page, (reached.end - 1).max(unit.end - 1))
        } else {
            TextSpan::run(unit.start, head_page, reached.end - 1)
        };
        Some(match span {
            TextSpan::Run { anchor, head, head_page, .. } if head_page == page && end_x.is_finite() => {
                TextSpan::Run { anchor, head, head_page, end_x: Some(end_x) }
            }
            _ => span,
        })
    }

    /// Where a single-click drag that stayed on its page was released, in page points: the end
    /// [`PageTextSource::rows_between`] reads. `None` for a click, a unit drag, a column and a run across pages.
    fn released_past_row(&self, pointer: &ToolPointer) -> Option<f64> {
        let (page, TextSpan::Run { head: pressed, head_page: pressed_page, .. }) = self.state.gesture? else { return None };
        let ((selected, _), head) = self.state.selection?.run_ends()?;
        if self.state.clicks != 1 || selected != page || head.0 != page || head == (pressed_page, pressed) {
            return None;
        }
        let placed = self.page_at(pointer.pos).filter(|placed| placed.page == page)?;
        let at = placed.to_page(pointer.pos);
        at.x.is_finite().then_some(at.x)
    }
}

// pdfcraft: crates/ui-egui/src/canvas.rs:1470 `text_layer`, :1547 `text_pointer_input`.
impl CanvasTool for TextSelectTool<'_> {
    fn cursor(&self, cx: &ToolContext<'_>) -> Cursor {
        let Some(pointer) = cx.pointer else { return Cursor::System(CursorIcon::Default) };
        if self.column || cx.mods.alt {
            return Cursor::System(CursorIcon::Crosshair);
        }
        let over_glyph = self
            .page_at(pointer.pos)
            .and_then(|placed| self.texts.page(placed.page).map(|text| text.glyph_box_at(placed.to_page(pointer.pos))))
            .is_some_and(|glyph| glyph.is_some());
        Cursor::System(if over_glyph { CursorIcon::Text } else { CursorIcon::Default })
    }

    fn press(&mut self, pointer: &ToolPointer, _cx: &mut ToolContext<'_>) -> ToolAction {
        let texts = self.texts;
        let Some(placed) = self.page_at(pointer.pos) else {
            self.set(None);
            return ToolAction::Done;
        };
        let Some(text) = texts.page(placed.page) else {
            self.set(None);
            return ToolAction::Done;
        };
        let (page, at) = (placed.page, placed.to_page(pointer.pos));
        let before = self.state.selection;
        self.state.dragged = false;
        if self.column || pointer.mods.alt {
            self.state.gesture = Some((page, TextSpan::column(at, at)));
            self.state.clicks = 1;
            self.state.before = before;
            self.set(Some(TextSelection::new(page, TextSpan::column(at, at))));
            return ToolAction::Claimed;
        }
        let Some(glyph) = text.glyph_at(at) else {
            self.set(None);
            return ToolAction::Done;
        };
        let clicks = pointer.clicks.max(1);
        let extended = before.filter(|_| pointer.mods.shift && clicks < 2).and_then(|selection| selection.run_ends());
        let (selection, start) = match (clicks, extended) {
            (2.., _) => match click_unit(text, clicks, glyph).filter(|unit| !unit.is_empty()) {
                Some(unit) => (
                    TextSelection::new(page, TextSpan::run(unit.start, page, unit.end - 1)),
                    TextSelection::new(page, TextSpan::run(glyph, page, glyph)),
                ),
                None => {
                    self.set(None);
                    return ToolAction::Done;
                }
            },
            (_, Some(((from, anchor), _))) => {
                let run = TextSelection::new(from, TextSpan::run(anchor, page, glyph));
                (run, run)
            }
            _ => {
                let run = TextSelection::new(page, TextSpan::run(glyph, page, glyph));
                (run, run)
            }
        };
        self.state.gesture = Some((start.page, start.span));
        self.state.clicks = clicks;
        self.state.before = before;
        self.set(Some(selection));
        ToolAction::Claimed
    }

    fn drag(&mut self, pointer: &ToolPointer, _cx: &mut ToolContext<'_>) {
        self.state.dragged = true;
        let (pages, texts) = (self.pages, self.texts);
        let Some((page, start)) = self.state.gesture else { return };
        let Some(selection) = self.state.selection.filter(|selection| selection.page == page) else { return };
        let span = match (selection.span, start) {
            (TextSpan::Column { from, .. }, _) => {
                let Some(placed) = pages.iter().find(|placed| placed.page == page) else { return };
                TextSpan::column(from, placed.to_page(pointer.pos))
            }
            (TextSpan::Run { anchor, .. }, TextSpan::Run { anchor: origin, .. }) => {
                // The head follows the pointer onto whichever page it is over; off every page, or over a page whose
                // text is not read yet, the selection keeps its last value.
                let Some(placed) = self.page_at(pointer.pos) else { return };
                let at = placed.to_page(pointer.pos);
                let Some(hit) = texts.page(placed.page).and_then(|text| text.glyph_at(at)) else { return };
                self.unit_run(page, origin, placed.page, hit, at.x).unwrap_or(TextSpan::run(anchor, placed.page, hit))
            }
            _ => return,
        };
        self.set(Some(TextSelection::new(page, span)));
    }

    fn release(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        let dragged = self.state.dragged;
        if dragged {
            self.drag(pointer, cx);
        }
        let column_click = pointer.mods.shift
            && !dragged
            && self.state.before.is_some_and(|selection| matches!(selection.span, TextSpan::Column { .. }))
            && self.state.clicks == 1;
        if column_click {
            self.set(None);
            self.end_gesture();
            return;
        }
        let end_x = self.released_past_row(pointer);
        let selection = self.state.selection;
        match selection {
            Some(selection @ TextSelection { span: TextSpan::Column { .. }, .. })
                if selection.text(self.texts).is_none_or(|text| text.is_empty()) =>
            {
                self.set(None);
            }
            Some(TextSelection { page, span: TextSpan::Run { anchor, head, head_page, .. } }) if end_x.is_some() => {
                self.set(Some(TextSelection::new(page, TextSpan::Run { anchor, head, head_page, end_x })));
            }
            _ => {}
        }
        self.end_gesture();
    }

    fn cancel(&mut self, _reason: CancelReason, _cx: &mut ToolContext<'_>) {
        if self.state.gesture.is_some() {
            let before = self.state.before;
            self.set(before);
        }
        self.end_gesture();
    }

    fn key(&mut self, key: &ToolKey, cx: &mut ToolContext<'_>) -> bool {
        let Some(selection) = self.state.selection else { return false };
        match key {
            ToolKey::Cancel => {
                self.set(None);
                true
            }
            ToolKey::Nudge { direction, mods, .. } if mods.shift => {
                // The head moves on its own page and stops at that page's first and last glyph.
                let Some(((_, anchor), (head_page, head))) = selection.run_ends() else { return false };
                let Some(text) = self.texts.page(head_page) else { return false };
                let count = text.glyph_count();
                let vertical = direction.y != 0.0;
                let forward = if vertical { direction.y > 0.0 } else { (direction.x > 0.0) != cx.direction().is_rtl() };
                let moved = if vertical {
                    line_step(text, head, forward)
                } else if mods.alt {
                    word_step(text, head, forward, count)
                } else if forward {
                    head.saturating_add(1)
                } else {
                    head.saturating_sub(1)
                };
                self.set(Some(TextSelection::new(selection.page, TextSpan::run(anchor, head_page, moved.min(count.saturating_sub(1))))));
                true
            }
            _ => false,
        }
    }

    fn paint(&self, draw: &Draw<'_>, cx: &ToolContext<'_>) {
        let Some(style) = cx.style() else { return };
        let look = MarkLook::of(style);
        for placed in self.pages {
            let Some(text) = self.texts.page(placed.page) else { continue };
            paint_marks(draw, &look, placed, Some(text), self.state.selection.as_ref(), &[], &[]);
        }
        if let (Some((page, _)), Some(TextSelection { page: selected, span: TextSpan::Column { from, to } })) =
            (self.state.gesture, self.state.selection)
            && page == selected
            && let Some(placed) = self.pages.iter().find(|placed| placed.page == page)
        {
            let rect = egui::Rect::from_two_pos(placed.to_screen(from), placed.to_screen(to));
            draw.stroke(&PaintShape::Rect(rect, CornerRadius::ZERO), StrokeWidth::Thin, look.column_outline);
        }
    }

    fn command_form(&self) -> ToolPath {
        ToolPath::Command("the Select All command")
    }
}

/// The head one word on from `head`, or back.
fn word_step(text: &dyn PageTextSource, head: usize, forward: bool, count: usize) -> usize {
    if forward {
        (head.saturating_add(1)..count).find_map(|glyph| text.word_at(glyph).map(|word| word.end - 1).filter(|end| *end > head)).unwrap_or(head)
    } else {
        (0..head).rev().find_map(|glyph| text.word_at(glyph).map(|word| word.start).filter(|start| *start < head)).unwrap_or(head)
    }
}

/// The head on the next line (or the one before), at the same distance from the line's start.
fn line_step(text: &dyn PageTextSource, head: usize, forward: bool) -> usize {
    let Some(line) = text.line_at(head) else { return head };
    let other = if forward {
        (line.end < text.glyph_count()).then(|| text.line_at(line.end)).flatten()
    } else {
        line.start.checked_sub(1).and_then(|before| text.line_at(before))
    };
    other.filter(|other| !other.is_empty()).map_or(head, |other| (other.start + (head - line.start)).min(other.end - 1))
}

/// The style of the find marks.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct TextMarksStyle {
    #[style(metrics)]
    metrics: Metrics,
    look: MarkLook,
}

/// The find marks of the document's pages, one fill per match and a stronger one for the current match, painted over
/// the page view's body under any tool (find marks show under the Hand tool too). Paint only, and not in the
/// accessibility tree: the find panel reports the matches.
///
/// With [`selection`](Self::selection) it also paints a text selection, for a tool other than the Select tool.
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
#[component(canvas_control)]
pub struct TextMarks<'a> {
    #[common]
    common: Common,
    pages: &'a [PlacedPage],
    texts: &'a dyn PageTexts,
    marks: &'a [TextMark],
    /// The text selection to paint with the marks.
    #[prop]
    selection: Option<TextSelection>,
    /// The areas to mark, in page points: selection, find and Read Aloud marks that are not glyph ranges.
    #[prop]
    areas: &'a [MarkArea],
}

/// What a [`MarkArea`] marks; each kind has its `MarkLook` fill.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum MarkKind {
    /// A selection.
    Selection,
    /// A find match.
    Found,
    /// The current find match.
    Current,
    /// The text Read Aloud speaks.
    Spoken,
}

/// A rectangle of a page to mark, in page points.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct MarkArea {
    /// The page index, 0-based.
    pub page: usize,
    /// The area in page points.
    pub rect: craft_behavior::kurbo::Rect,
    /// What it marks.
    pub kind: MarkKind,
}

impl MarkArea {
    /// The area `rect` of `page`, marking `kind`.
    pub fn new(page: usize, rect: craft_behavior::kurbo::Rect, kind: MarkKind) -> Self {
        MarkArea { page, rect, kind }
    }
}

/// No page has text: what [`TextMarks::over`] reads, so only its areas paint.
struct NoTexts;

impl PageTexts for NoTexts {
    fn page(&self, _: usize) -> Option<&dyn PageTextSource> {
        None
    }
}

impl<'a> TextMarks<'a> {
    /// The `marks` over the placed `pages`, whose text `texts` gives.
    pub fn new(pages: &'a [PlacedPage], texts: &'a dyn PageTexts, marks: &'a [TextMark]) -> Self {
        TextMarks { common: Common::new(), pages, texts, marks, selection: None, areas: &[] }
    }

    /// Marks for [`areas`](Self::areas) alone over the placed `pages`, with no text source.
    pub fn over(pages: &'a [PlacedPage]) -> Self {
        TextMarks { common: Common::new(), pages, texts: &NoTexts, marks: &[], selection: None, areas: &[] }
    }
}

impl Component for TextMarks<'_> {
    const ROLE: Role = Role::None;
    const OVERLAYS: bool = false;
    const PARTS: bool = false;
    const STOPS: bool = false;
    const INPUT: craft_behavior::Input = craft_behavior::Input::Display;
    const SHOWS_LABEL: bool = false;
    type Style = TextMarksStyle;
    type Output = Response;

    fn style(cx: &StyleCx<'_>, _: TextMarksStyleKey) -> TextMarksStyle {
        TextMarksStyle { metrics: Metrics::new(), look: MarkLook::of(cx) }
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::ZERO
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let draw = ui.draw();
        for placed in self.pages {
            paint_marks(&draw, &cx.style.look, placed, self.texts.page(placed.page), self.selection.as_ref(), self.marks, self.areas);
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}
