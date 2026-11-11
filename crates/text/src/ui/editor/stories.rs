//! Stories of the text editor: a caret, a selection and marked text on a canvas. The story is the app: it lays the
//! text out in monospace cells and paints it under the editor.

use std::ops::Range;

use craft_behavior::kurbo::Point;
use craft_behavior::{CanvasView, Metrics, Mixed, Sense};
use craft_ui::canvas::{Canvas, canvas_space};
use craft_ui::contract::{Bind, Common, Component, Cx, MeasureCx, PaintUi, Role, Show as _, ShowContainer as _, Validity};
use craft_ui::fonts::FontRef;
use craft_ui::primitives::{DocRun, DocText};
use craft_ui::runtime::env::Message;
use craft_ui::style::{ComponentStyle, DataColor, StyleColor, StyleCx};
use craft_ui::testing::{Drive, StoryData, StoryOutput, stage_fitting, story};
use egui::{Pos2, Rect, Response, Ui, Vec2};

use super::{CaretLine, EditKind, HostLine, HostRun, TextEditor, TextEditorHost, TextSelection};

/// The width of a char's cell, in document units.
const CELL: f64 = 10.0;
/// The height of a row, in document units.
const LINE: f64 = 24.0;
/// The size of the text, in document units.
const TEXT_SIZE: f32 = 16.0;

/// A text in a monospace layout: char `c` of row `r` sits in the cell at x = 10c, y = 24r. Rows break at `\n` and
/// wrap after the last space that fits `columns` cells; a wrapped row keeps the spaces it ends with.
struct MonoHost {
    text: String,
    revision: u64,
    columns: usize,
}

impl MonoHost {
    /// The chars of each laid-out row, without line breaks.
    fn rows(&self) -> Vec<Range<usize>> {
        let chars: Vec<char> = self.text.chars().collect();
        let columns = self.columns.max(1);
        let mut rows = Vec::new();
        let mut start = 0;
        loop {
            let end = chars[start..].iter().position(|c| *c == '\n').map_or(chars.len(), |at| start + at);
            let mut row = start;
            while end - row > columns {
                let limit = row + columns;
                let cut = (row + 1..=limit).rev().find(|at| chars[at - 1] == ' ').unwrap_or(limit);
                rows.push(row..cut);
                row = cut;
            }
            rows.push(row..end);
            if end == chars.len() {
                return rows;
            }
            start = end + 1;
        }
    }

    /// The row an offset starts: at a soft wrap, the row after it.
    fn row_of(rows: &[Range<usize>], offset: usize) -> usize {
        rows.iter().rposition(|row| row.start <= offset).unwrap_or(0)
    }

    /// The pieces of `range`, one per row it covers: (row, first column, end column).
    fn pieces(&self, range: Range<usize>) -> Vec<(usize, usize, usize)> {
        self.rows()
            .iter()
            .enumerate()
            .filter_map(|(at, row)| {
                let (from, to) = (range.start.max(row.start), range.end.min(row.end));
                (from < to).then(|| (at, from - row.start, to - row.start))
            })
            .collect()
    }

    /// The text of each row.
    fn row_texts(&self) -> Vec<String> {
        self.rows().into_iter().map(|row| self.text.chars().skip(row.start).take(row.len()).collect()).collect()
    }

    /// The size of the laid-out text, in document units.
    fn extent(&self) -> Vec2 {
        self.extent_in_columns(0)
    }

    /// The size of the laid-out text with one more cell at the end of the widest row, where the caret after its last
    /// char goes.
    fn extent_with_caret_cell(&self) -> Vec2 {
        self.extent_in_columns(1)
    }

    fn extent_in_columns(&self, extra: usize) -> Vec2 {
        let rows = self.rows();
        let widest = rows.iter().map(ExactSizeIterator::len).max().unwrap_or(0) + extra;
        Vec2::new((widest as f64 * CELL) as f32, (rows.len() as f64 * LINE) as f32)
    }
}

impl TextEditorHost for MonoHost {
    fn text(&self) -> &str {
        &self.text
    }

    fn revision(&self) -> u64 {
        self.revision
    }

    fn replace(&mut self, range: Range<usize>, text: &str, _: EditKind) -> bool {
        let byte = |chars: usize| self.text.char_indices().nth(chars).map_or(self.text.len(), |(at, _)| at);
        let (from, to) = (byte(range.start), byte(range.end));
        self.text.replace_range(from..to, text);
        self.revision += 1;
        true
    }

    fn hit(&mut self, at: Point) -> (usize, usize) {
        let rows = self.rows();
        let line = ((at.y / LINE).floor().max(0.0) as usize).min(rows.len() - 1);
        let row = &rows[line];
        ((row.start + (at.x / CELL).round().max(0.0) as usize).min(row.end), line)
    }

    fn caret(&mut self, offset: usize, line: Option<usize>) -> Option<CaretLine> {
        let rows = self.rows();
        let line = line.unwrap_or_else(|| Self::row_of(&rows, offset)).min(rows.len() - 1);
        let x = offset.saturating_sub(rows[line].start) as f64 * CELL;
        Some(CaretLine { top: Point::new(x, line as f64 * LINE), bottom: Point::new(x, (line + 1) as f64 * LINE) })
    }

    fn selection_quads(&mut self, range: Range<usize>) -> Vec<[Point; 4]> {
        self.pieces(range)
            .into_iter()
            .map(|(line, from, to)| {
                let (x0, x1) = (from as f64 * CELL, to as f64 * CELL);
                let (y0, y1) = (line as f64 * LINE, (line + 1) as f64 * LINE);
                [Point::new(x0, y0), Point::new(x1, y0), Point::new(x1, y1), Point::new(x0, y1)]
            })
            .collect()
    }

    fn underline_segments(&mut self, range: Range<usize>) -> Vec<[Point; 2]> {
        self.pieces(range)
            .into_iter()
            .map(|(line, from, to)| {
                let y = (line + 1) as f64 * LINE - 3.0;
                [Point::new(from as f64 * CELL, y), Point::new(to as f64 * CELL, y)]
            })
            .collect()
    }

    fn lines(&mut self) -> Vec<HostLine> {
        self.rows()
            .into_iter()
            .zip(self.row_texts())
            .enumerate()
            .map(|(line, (chars, text))| {
                let count = chars.len();
                let positions: Vec<f32> = (0..count).map(|at| (at as f64 * CELL) as f32).collect();
                let (y0, y1) = (line as f64 * LINE, (line + 1) as f64 * LINE);
                let rect = Rect::from_min_max(Pos2::new(0.0, y0 as f32), Pos2::new((count as f64 * CELL) as f32, y1 as f32));
                let runs = vec![HostRun { bytes: 0..text.len(), rtl: false, positions, widths: vec![CELL as f32; count] }];
                HostLine { chars, rect, baseline: rect.center().y + TEXT_SIZE / 3.0, runs, text }
            })
            .collect()
    }

    fn line_range(&mut self, line: usize) -> Range<usize> {
        let rows = self.rows();
        rows[line.min(rows.len() - 1)].clone()
    }

    fn line_step(&mut self, offset: usize, lines: i32) -> usize {
        let rows = self.rows();
        let line = Self::row_of(&rows, offset);
        let target = (line as i32 + lines).clamp(0, rows.len() as i32 - 1) as usize;
        (rows[target].start + (offset - rows[line].start)).min(rows[target].end)
    }

    fn line_edge(&mut self, offset: usize, end: bool) -> usize {
        let rows = self.rows();
        let row = &rows[Self::row_of(&rows, offset)];
        if end { row.end } else { row.start }
    }
}

/// The style of the story's text.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
struct MonoTextStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The text colour.
    ink: StyleColor,
}

/// The app's paint of the host's text: each char centred in its cell, through the canvas view.
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
#[component(unregistered)]
struct MonoText {
    #[common]
    common: Common,
    rows: Vec<String>,
}

impl Component for MonoText {
    const ROLE: Role = Role::None;
    const PARTS: bool = false;
    const STOPS: bool = false;
    const INPUT: craft_behavior::Input = craft_behavior::Input::Display;
    const SHOWS_LABEL: bool = false;
    const OVERLAYS: bool = false;
    type Style = MonoTextStyle;
    type Output = Response;

    fn style(cx: &StyleCx<'_>, _: MonoTextStyleKey) -> MonoTextStyle {
        MonoTextStyle { metrics: Metrics::new(), ink: cx.color(cx.tokens.color.text.normal) }
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::ZERO
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let Some(space) = canvas_space(&cx.env) else { return };
        let draw = ui.draw();
        let ink = DataColor::new(cx.style.ink.egui());
        let size = TEXT_SIZE * space.view.zoom as f32;
        for (line, text) in self.rows.iter().enumerate() {
            for (column, c) in text.chars().enumerate() {
                let centre = space.view.to_screen(Point::new((column as f64 + 0.5) * CELL, (line as f64 + 0.5) * LINE));
                let run = [DocRun::new(c.to_string(), FontRef::monospace(), size, ink)];
                let layout = cx.layout_doc(&DocText::new(&run));
                draw.galley(centre - layout.size() / 2.0, &layout);
            }
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

/// The text the stories edit.
const SAMPLE: &str = "The quick brown fox\njumps over the lazy dog";

/// The editor over `text` with the selection bound to `bind`, on a canvas as tall as the text wrapped at the stage's
/// width, whose view puts document (0, 0) at its top left. The host's edits (typed, pasted and composed text) are
/// written back to `text`, which the story keeps between passes.
fn over_canvas(ui: &mut Ui, text: &mut String, mut bind: Bind<'_, TextSelection>) -> StoryOutput {
    let mut host = MonoHost { text: std::mem::take(text), revision: 0, columns: 1 };
    let room = stage_fitting(ui, |width| {
        // One cell stays free at the end of a row for the caret after its last char.
        host.columns = ((f64::from(width) / CELL).floor() as usize).saturating_sub(1);
        host.extent().y
    });
    let view = CanvasView::new(room, Point::new(f64::from(room.width()) / 2.0, f64::from(room.height()) / 2.0), 1.0);
    let shown = Canvas::new("Document").view(view).show_at(ui, room, |ui, _| {
        let bounds = Rect::from_min_size(view.to_screen(Point::ZERO), host.extent_with_caret_cell());
        MonoText { common: Common::new(), rows: host.row_texts() }.show_at(ui, bounds);
        TextEditor::new("Text", &mut host, bind.reborrowed()).show_at(ui, bounds)
    });
    *text = host.text;
    shown.inner.map(StoryOutput::from).unwrap_or_default()
}

/// The editor over `text` with `selection` on a canvas.
fn on_canvas(ui: &mut Ui, data: &mut StoryData, text: &str, selection: TextSelection) -> StoryOutput {
    let mut host = data.value("text", text.to_owned()).clone();
    let selection = data.value("selection", selection);
    let out = over_canvas(ui, &mut host, Bind::bound(&mut *selection));
    *data.value("text", String::new()) = host;
    out
}

/// The two lines with "quick" selected.
#[story(TextEditor<'static>, covers = [Default], drive = Drive::Never)]
fn canvas(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    on_canvas(ui, data, SAMPLE, TextSelection::new(4, 9))
}

#[story(TextEditor<'static>, covers = [], drive = Drive::Type("sample"))]
fn caret(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    on_canvas(ui, data, SAMPLE, TextSelection::caret_at(5))
}

#[story(TextEditor<'static>, covers = [LongText], drive = Drive::Never)]
fn long_text(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    on_canvas(ui, data, &"A line of text that wraps across the frame. ".repeat(10), TextSelection::caret_at(0))
}

#[story(TextEditor<'static>, covers = [], drive = Drive::Never)]
fn selection(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    on_canvas(ui, data, SAMPLE, TextSelection::new(10, 25))
}

#[story(TextEditor<'static>, covers = [], drive = Drive::Never)]
fn composing(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    on_canvas(ui, data, "The quick brown fox かな", TextSelection::caret_at(22))
}

#[story(TextEditor<'static>, covers = [Mixed], drive = Drive::Never)]
fn selections_that_differ(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mut host = data.value("text", SAMPLE.to_owned()).clone();
    let selection = data.value("selection", Mixed::<TextSelection>::Mixed);
    over_canvas(ui, &mut host, Bind::bound_mixed(&mut *selection))
}

#[story(TextEditor<'static>, covers = [Invalid], drive = Drive::Never)]
fn selection_past_the_text(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mut host = data.value("text", SAMPLE.to_owned()).clone();
    let end = host.chars().count();
    let selection = data.value("selection", TextSelection::caret_at(end + 1));
    let inside = move |selection: &TextSelection| {
        if selection.caret > end { Validity::Refuse(Message::new("The selection is past the text")) } else { Validity::Ok }
    };
    over_canvas(ui, &mut host, Bind::bound(&mut *selection).check(inside))
}
