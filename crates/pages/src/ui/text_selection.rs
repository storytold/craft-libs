//! `TextSelectTool`: selecting text on a rendered page (Acrobat's Select tool, a craft-ui canvas tool): drag across
//! glyphs in reading order, Alt-drag (or the app's Column Select tool) for the glyphs inside a rectangle,
//! double-click a word, triple-click a line, four clicks the page; the selection highlighted. Find matches are painted
//! by [`TextMarks`], a canvas control the app shows in the page body under any tool.
//!
//! The page's text comes from the app ([`PageTextSource`]: PdfCraft's extracted text layer); the layer keeps no text.

use std::ops::Range;

use craft_behavior::kurbo::{Point, Rect};
use craft_behavior::{CancelReason, Cursor, CursorIcon, Metrics, Sense};
use craft_ui::canvas::{CanvasTool, ToolAction, ToolContext, ToolKey, ToolPath, ToolPointer};
use craft_ui::contract::{Common, Component, Cx, MeasureCx, PaintUi, Role};
use craft_ui::primitives::{Draw, PaintShape, StrokeWidth};
use craft_ui::style::{ComponentStyle, StyleColor, StyleCx};
use egui::{CornerRadius, Pos2, Response, Vec2, pos2};

use super::view::PlacedPage;

#[cfg(feature = "screen-testing")]
mod stories;

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
}

/// What is selected on one page.
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
    /// A run from the glyph `anchor` to the glyph `head`.
    pub fn run(anchor: usize, head: usize) -> Self {
        TextSpan::Run { anchor, head }
    }

    /// The column from the page point `from` to the page point `to`.
    pub fn column(from: Point, to: Point) -> Self {
        TextSpan::Column { from, to }
    }
}

/// A text selection on one page.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct TextSelection {
    /// Page index, 0-based.
    pub page: usize,
    /// The selected glyphs on that page.
    pub span: TextSpan,
}

impl TextSelection {
    /// The selection `span` on page `page`.
    pub fn new(page: usize, span: TextSpan) -> Self {
        TextSelection { page, span }
    }

    /// The selection as rectangles, one per line piece (for highlighting and markup quads).
    // pdfcraft: crates/ui-egui/src/canvas/selection.rs:29 `Selection::rects`.
    pub fn rects(&self, text: &dyn PageTextSource) -> Vec<Rect> {
        match self.span {
            TextSpan::Run { anchor, head } => text.line_rects(run_range(anchor, head)),
            TextSpan::Column { from, to } => {
                let glyphs = text.glyphs_in(Rect::from_points(from, to));
                runs(&glyphs).into_iter().flat_map(|run| text.line_rects(run)).collect()
            }
        }
    }

    /// The selected text, as Copy puts it on the clipboard.
    // pdfcraft: crates/ui-egui/src/canvas/selection.rs:37 `Selection::text`.
    pub fn text(&self, text: &dyn PageTextSource) -> String {
        match self.span {
            TextSpan::Run { anchor, head } => text.text_of(run_range(anchor, head)),
            TextSpan::Column { from, to } => text.column_text(&text.glyphs_in(Rect::from_points(from, to))),
        }
    }
}

/// The glyphs of a run, both ends included.
fn run_range(anchor: usize, head: usize) -> Range<usize> {
    anchor.min(head)..anchor.max(head).saturating_add(1)
}

/// `glyphs` as ranges of consecutive indices.
fn runs(glyphs: &[usize]) -> Vec<Range<usize>> {
    let mut runs: Vec<Range<usize>> = Vec::new();
    for &glyph in glyphs {
        match runs.last_mut() {
            Some(run) if run.end == glyph => run.end += 1,
            _ => runs.push(glyph..glyph + 1),
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
    /// The drag in progress: the page and where it started (glyph, or page point for a column).
    gesture: Option<(usize, TextSpan)>,
    /// The press's click count, and the selection from before the gesture (Esc brings it back).
    clicks: u32,
    before: Option<TextSelection>,
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

/// Paints the selection and find marks of one placed page: the selection fill, the match fills, the current match
/// grown. The one painter of both: [`TextMarks`] paints the matches and the tool the selection through it.
fn paint_marks(
    draw: &Draw<'_>,
    look: &MarkLook,
    placed: &PlacedPage,
    text: &dyn PageTextSource,
    selection: Option<&TextSelection>,
    marks: &[TextMark],
) {
    let (top, bottom) = placed.band;
    let band = egui::Rect::from_min_max(pos2(placed.rect.min.x, placed.rect.min.y + top), pos2(placed.rect.max.x, placed.rect.min.y + bottom));
    let screen = |r: Rect| egui::Rect::from_two_pos(placed.to_screen(Point::new(r.x0, r.y0)), placed.to_screen(Point::new(r.x1, r.y1)));
    draw.clipped(band, |draw| {
        for mark in marks.iter().filter(|mark| mark.page == placed.page) {
            let (fill, grow) = if mark.current { (look.current, look.current_grow) } else { (look.found, 0.0) };
            for rect in text.line_rects(mark.glyphs.clone()) {
                draw.fill(&PaintShape::Rect(screen(rect).expand(grow), look.radius), fill);
            }
        }
        if let Some(selection) = selection.filter(|selection| selection.page == placed.page) {
            for rect in selection.rects(text) {
                draw.fill(&PaintShape::Rect(screen(rect), look.radius), look.selection);
            }
        }
    });
}

/// Acrobat's Select tool on the page view, a craft-ui [`CanvasTool`] the app runs in its `ToolHost` over the page
/// view's canvas; built per pass over the pass's placed pages and the app's [`TextSelectState`].
///
/// - Press and drag with the primary button selects ([`TextSpan::Run`], anchored at the press's glyph); with Alt held
///   (or [`column`](Self::column) on: the Column Select tool) it selects a column, its rectangle outlined while
///   dragging. The press's click count is [`ToolPointer::clicks`]: 2 the word, 3 the line, 4 the page
///   ([`click_unit`]). The cursor is the I-beam over glyphs, the arrow elsewhere. A press off any glyph clears the
///   selection.
/// - Keys while the canvas has the keyboard: Shift with the arrows extends a run by a glyph (Shift and Alt by a word;
///   left and right swap in a right-to-left interface), Escape clears. Select
///   All and Copy are the app's commands (they read [`TextSelection::text`]).
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
        let at = placed.to_page(pointer.pos);
        let before = self.state.selection;
        if self.column || pointer.mods.alt {
            self.state.gesture = Some((placed.page, TextSpan::Column { from: at, to: at }));
            self.state.clicks = 1;
            self.state.before = before;
            self.set(Some(TextSelection { page: placed.page, span: TextSpan::Column { from: at, to: at } }));
            return ToolAction::Claimed;
        }
        let Some(glyph) = text.glyph_at(at) else {
            self.set(None);
            return ToolAction::Done;
        };
        let clicks = pointer.clicks.max(1);
        let extended = before.filter(|selection| pointer.mods.shift && clicks < 2 && selection.page == placed.page);
        let (span, start) = match (clicks, extended) {
            (2.., _) => match click_unit(text, clicks, glyph).filter(|unit| !unit.is_empty()) {
                Some(unit) => (TextSpan::Run { anchor: unit.start, head: unit.end - 1 }, TextSpan::Run { anchor: glyph, head: glyph }),
                None => {
                    self.set(None);
                    return ToolAction::Done;
                }
            },
            (_, Some(TextSelection { span: TextSpan::Run { anchor, .. }, .. })) => {
                (TextSpan::Run { anchor, head: glyph }, TextSpan::Run { anchor, head: anchor })
            }
            _ => (TextSpan::Run { anchor: glyph, head: glyph }, TextSpan::Run { anchor: glyph, head: glyph }),
        };
        self.state.gesture = Some((placed.page, start));
        self.state.clicks = clicks;
        self.state.before = before;
        self.set(Some(TextSelection { page: placed.page, span }));
        ToolAction::Claimed
    }

    fn drag(&mut self, pointer: &ToolPointer, _cx: &mut ToolContext<'_>) {
        let (pages, texts) = (self.pages, self.texts);
        let Some((page, start)) = self.state.gesture else { return };
        let Some(selection) = self.state.selection.filter(|selection| selection.page == page) else { return };
        let (Some(placed), Some(text)) = (pages.iter().find(|placed| placed.page == page), texts.page(page)) else { return };
        let at = placed.to_page(pointer.pos);
        let span = match (selection.span, start) {
            (TextSpan::Column { from, .. }, _) => TextSpan::Column { from, to: at },
            (TextSpan::Run { anchor, .. }, TextSpan::Run { anchor: origin, .. }) => {
                let Some(hit) = text.glyph_at(at) else { return };
                let clicks = self.state.clicks;
                match click_unit(text, clicks, origin).filter(|unit| !unit.is_empty()) {
                    Some(unit) if clicks >= 2 => {
                        let reached = click_unit(text, clicks, hit).filter(|unit| !unit.is_empty()).unwrap_or(hit..hit + 1);
                        if reached.start < unit.start {
                            TextSpan::Run { anchor: unit.end - 1, head: reached.start }
                        } else {
                            TextSpan::Run { anchor: unit.start, head: (reached.end - 1).max(unit.end - 1) }
                        }
                    }
                    _ => TextSpan::Run { anchor, head: hit },
                }
            }
            (span, _) => span,
        };
        self.set(Some(TextSelection { page, span }));
    }

    fn release(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        self.drag(pointer, cx);
        if let Some(selection) = self.state.selection
            && matches!(selection.span, TextSpan::Column { .. })
            && self.texts.page(selection.page).is_none_or(|text| selection.text(text).is_empty())
        {
            self.set(None);
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
                let TextSpan::Run { anchor, head } = selection.span else { return false };
                let Some(text) = self.texts.page(selection.page) else { return false };
                let count = text.glyph_count();
                let vertical = direction.y != 0.0;
                let forward = if vertical { direction.y > 0.0 } else { (direction.x > 0.0) != cx.direction().is_rtl() };
                let moved = if vertical {
                    line_step(text, head, forward)
                } else if mods.alt {
                    word_step(text, head, forward, count)
                } else if forward {
                    head + 1
                } else {
                    head.saturating_sub(1)
                };
                self.set(Some(TextSelection { page: selection.page, span: TextSpan::Run { anchor, head: moved.min(count.saturating_sub(1)) } }));
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
            paint_marks(draw, &look, placed, text, self.state.selection.as_ref(), &[]);
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
        (head + 1..count).find_map(|glyph| text.word_at(glyph).map(|word| word.end - 1).filter(|end| *end > head)).unwrap_or(head)
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
}

impl<'a> TextMarks<'a> {
    /// The `marks` over the placed `pages`, whose text `texts` gives.
    pub fn new(pages: &'a [PlacedPage], texts: &'a dyn PageTexts, marks: &'a [TextMark]) -> Self {
        TextMarks { common: Common::new(), pages, texts, marks, selection: None }
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
            let Some(text) = self.texts.page(placed.page) else { continue };
            paint_marks(&draw, &cx.style.look, placed, text, self.selection.as_ref(), self.marks);
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}
