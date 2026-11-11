//! [`NotesEditor`] (N3.18): a multi-line rich-text box for speaker or page notes (PowerPoint's Notes pane): the
//! notes' bold, italic, underline and colour runs drawn while editing, bullets and numbers at each paragraph's start
//! with wrapped lines under the text, not under the bullet; typing keeps the formatting around the caret. Read-only,
//! the same component is the presenter's notes pane.
//!
//! The notes are the app's; [`NotesText`] is the shape the editor reads and writes.

#[cfg(feature = "screen-testing")]
mod stories;

mod text;

use craft_behavior::{EditCommand, EditorInput, Granularity, Metrics, NavMove, PointerButton, PressPhase, Sense, TextPos};
use craft_ui::contract::{
    Animate as _, Commands as _, Common, Component, Cx, Focus as _, MeasureCx, NoValue, Outcome, PaintUi, PartsCx, Pointer as _, Props as _,
    Remember as _, Role, WordSetCx as _,
};
use craft_ui::fonts::FontRef;
use craft_ui::primitives::{DocRun, DocText, EditorLine, EditorPurpose, EditorRun, EditorState, PaintShape, StrokeWidth, TextRun};
use craft_ui::runtime::commands::ClipItem;
use craft_ui::runtime::env::Label;
use craft_ui::runtime::focus::KeyUse;
use craft_ui::runtime::pending::Pending;
use craft_ui::style::{ComponentStyle, DataColor, StyleColor, StyleCx, TypeRole};
use egui::epaint::text::CharIndex;
use egui::{Pos2, Rect, Response, Vec2};

use super::editor::word_boundary;
use crate::lists::line_starts;
pub use text::{NotesParagraph, NotesRun, NotesText};
use text::{locate, markers_of, paragraph_range, splice};

/// The largest indent, points.
const MAX_INDENT: f32 = 400.0;
/// The notes' default text size in document points: indents are in document points and scale by the shown text
/// size over this size.
const DOCUMENT_SIZE: f32 = 12.0;
/// The width the editor measures at its zoom of 1 when its parent gives no room, document points.
const DEFAULT_WIDTH: f32 = 168.0;
/// The lines of the body role the editor measures at least.
const MIN_LINES: f32 = 4.0;

craft_ui::words! {
    /// The notes pane's words (PowerPoint's).
    #[non_exhaustive]
    pub struct NotesLabels {
        notes: "notes.notes" = "Notes";
        placeholder: "notes.placeholder" = "Click to add notes";
    }
}

/// What the editor did this pass.
#[derive(Debug)]
#[non_exhaustive]
pub struct NotesEditorOutput {
    /// The editor's response.
    pub response: Response,
    /// The notes changed this pass: the app stores them (one undo step per typing run).
    pub changed: bool,
}

impl Outcome for NotesEditorOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The notes' look: the body type at the pane's zoom, the text and caret colours and the selection fill. Indents are
/// in document points and scale with the text size.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct NotesEditorStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The text colour.
    ink: StyleColor,
    /// The placeholder's colour: the muted text role.
    muted: StyleColor,
    /// The selection fill.
    selection: StyleColor,
    /// The size of the body text, points.
    body_size: f32,
    /// The body role's line height, points.
    line_height: f32,
    /// The largest indent, points.
    max_indent: f32,
}

/// The position of the caret and the selection anchor in chars of [`NotesText::plain`].
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Cursor {
    caret: usize,
    anchor: usize,
    /// The pass time of the last edit or move: the blink restarts from it.
    blink_start: f64,
}

impl Cursor {
    /// The caret and anchor after the pointer reached char `at`: a press (`Some((clicks, shift))`) places the caret,
    /// Shift extends from the anchor, a double press selects the word and a triple press the paragraph; a drag
    /// (`None`) moves the caret from the anchor.
    fn point(&mut self, plain: &str, at: usize, press: Option<(u32, bool)>) {
        let Some((clicks, shift)) = press else {
            self.caret = at;
            return;
        };
        match clicks {
            0 | 1 => {
                self.caret = at;
                if !shift {
                    self.anchor = at;
                }
            }
            2 => {
                self.caret = word_boundary(plain, at, true);
                self.anchor = word_boundary(plain, self.caret, false);
            }
            _ => {
                let range = paragraph_range(plain, locate(plain, at).0);
                self.anchor = range.start;
                self.caret = range.end;
            }
        }
    }
}

/// The editor, built per pass over the app's notes.
///
/// - Text at the body type role × [`zoom`](NotesEditor::zoom), wrapped to the width; each paragraph laid out at its
///   indent, its marker ([`crate::lists::markers`]) drawn at its first line's start.
/// - An empty editor shows the placeholder; the editor is named by the notes word.
/// - Read-only (`Props::read_only`) it filters input to what changes no text and is the presenter's view.
/// - Bold, italic and underline toggles are the app's commands, not the editor's.
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
#[component(content_painter)]
pub struct NotesEditor<'a> {
    #[common]
    common: Common,
    notes: &'a mut NotesText,
    /// The notes pane's zoom (PowerPoint's Zoom of the notes pane), a factor over the body type role's size; 1 by
    /// default.
    #[prop(quantity = "zoom factor")]
    zoom: f32,
    changed: bool,
}

impl<'a> NotesEditor<'a> {
    /// An editor named `label` over `notes`, shown at the body size.
    pub fn new(label: impl Into<Label>, notes: &'a mut NotesText) -> Self {
        NotesEditor { common: Common::new(), notes, zoom: 1.0, changed: false }.label(label)
    }
}
impl NotesEditor<'_> {
    /// The char of the notes under `pos`: the paragraph whose rows hold the point (the last one below them all), then
    /// the galley's character at the point, laid out as `paint` lays it out.
    fn char_at(&self, cx: &Cx<'_, Self>, pos: Pos2) -> usize {
        let style = &cx.style;
        let area = cx.content_rect;
        let size = style.body_size * self.zoom;
        let scale = size / DOCUMENT_SIZE;
        let ink = DataColor::new(style.ink.egui());
        let font = FontRef::proportional();
        let mut y = area.top();
        let mut at = 0usize;
        let mut hit = 0usize;
        for (index, paragraph) in self.notes.paragraphs.iter().enumerate() {
            let (_, margin) = line_starts(paragraph.level, paragraph.margin, paragraph.indent);
            let origin = area.left() + indent(style, margin, scale);
            let runs = paragraph_runs(paragraph, &font, size, ink);
            let mut text = DocText::new(&runs);
            text.wrap = Some((area.width() - (origin - area.left())).max(0.0));
            let layout = cx.layout_doc(&text);
            let length: usize = paragraph.runs.iter().map(|run| run.text.chars().count()).sum();
            if index == 0 || pos.y >= y {
                hit = at + layout.galley.cursor_from_pos(pos - Pos2::new(origin, y)).index.0.min(length);
            }
            y += layout.size().y.max(size);
            at += length + 1;
        }
        hit
    }

    /// Applies one input to the notes' plain text and the cursor; returns whether the text changed.
    fn apply(&self, plain: &mut String, cursor: &mut Cursor, input: EditorInput, clip: Option<&str>) -> bool {
        let chars = plain.chars().count();
        cursor.caret = cursor.caret.min(chars);
        cursor.anchor = cursor.anchor.min(chars);
        let selected = cursor.caret.min(cursor.anchor)..cursor.caret.max(cursor.anchor);
        let insert = |plain: &mut String, cursor: &mut Cursor, text: &str| {
            let text = text.replace("\r\n", "\n").replace('\r', "\n");
            splice(plain, selected.clone(), &text);
            cursor.caret = selected.start + text.chars().count();
            cursor.anchor = cursor.caret;
            true
        };
        match input {
            EditorInput::Text(text) => insert(plain, cursor, &text),
            EditorInput::Newline => insert(plain, cursor, "\n"),
            EditorInput::Tab => insert(plain, cursor, "\t"),
            EditorInput::Edit(EditCommand::Paste) => clip.is_some_and(|text| insert(plain, cursor, text)),
            EditorInput::Edit(EditCommand::Cut) if !selected.is_empty() => insert(plain, cursor, ""),
            EditorInput::Edit(EditCommand::SelectAll) => {
                cursor.anchor = 0;
                cursor.caret = chars;
                false
            }
            EditorInput::Delete { forward, by, .. } => {
                let range = if !selected.is_empty() {
                    selected
                } else {
                    let at = cursor.caret;
                    let to = match (forward, by) {
                        (false, Granularity::Line | Granularity::Paragraph | Granularity::Document) => {
                            paragraph_range(plain, locate(plain, at).0).start
                        }
                        (false, Granularity::Word) => word_boundary(plain, at, false),
                        (false, _) => at.saturating_sub(1),
                        (true, Granularity::Unit) => (at + 1).min(chars),
                        (true, _) => word_boundary(plain, at, true),
                    };
                    to.min(at)..to.max(at)
                };
                if range.is_empty() {
                    return false;
                }
                splice(plain, range.clone(), "");
                cursor.caret = range.start;
                cursor.anchor = range.start;
                true
            }
            EditorInput::DeleteSurrounding { before, after, .. } => {
                let (from, to) = (selected.start.saturating_sub(before), (selected.end + after).min(chars));
                if from == selected.start && to == selected.end {
                    return false;
                }
                splice(plain, selected.end..to, "");
                splice(plain, from..selected.start, "");
                cursor.caret = from + selected.len();
                cursor.anchor = from;
                true
            }
            EditorInput::Move(step, mods) => {
                let (index, column) = locate(plain, cursor.caret);
                let to = match step {
                    NavMove::Back | NavMove::Forward => {
                        let forward = step == NavMove::Forward;
                        if mods.by != Granularity::Unit {
                            word_boundary(plain, cursor.caret, forward)
                        } else if !selected.is_empty() && !mods.extend {
                            if forward { selected.end } else { selected.start }
                        } else if forward {
                            (cursor.caret + 1).min(chars)
                        } else {
                            cursor.caret.saturating_sub(1)
                        }
                    }
                    NavMove::Up | NavMove::Down if matches!(mods.by, Granularity::Paragraph | Granularity::Document) => {
                        if step == NavMove::Up {
                            0
                        } else {
                            chars
                        }
                    }
                    NavMove::Up | NavMove::Down => {
                        let paragraphs = plain.split('\n').count();
                        let target = if step == NavMove::Up { index.checked_sub(1) } else { Some(index + 1).filter(|next| *next < paragraphs) };
                        target.map_or(cursor.caret, |target| {
                            let range = paragraph_range(plain, target);
                            range.start + column.min(range.len())
                        })
                    }
                    NavMove::Start | NavMove::End => {
                        if mods.by == Granularity::Document {
                            if step == NavMove::End { chars } else { 0 }
                        } else {
                            let range = paragraph_range(plain, index);
                            if step == NavMove::End { range.end } else { range.start }
                        }
                    }
                    _ => cursor.caret,
                };
                cursor.caret = to;
                if !mods.extend {
                    cursor.anchor = to;
                }
                false
            }
            EditorInput::Select { anchor, focus, .. } => {
                let at = |pos: TextPos| {
                    let range = paragraph_range(plain, pos.line);
                    let line: String = plain.chars().skip(range.start).take(range.len()).collect();
                    range.start + line.get(..pos.byte.min(line.len())).map_or(0, |head| head.chars().count())
                };
                cursor.anchor = at(anchor);
                cursor.caret = at(focus);
                false
            }
            _ => false,
        }
    }
}

impl Component for NotesEditor<'_> {
    const ROLE: Role = Role::MULTILINE_TEXT_INPUT;
    const INPUT: craft_ui::contract::Input = craft_ui::contract::Input::Editor;
    const KEYS: KeyUse = KeyUse::CODE;
    type Style = NotesEditorStyle;
    type Output = NotesEditorOutput;

    fn style(cx: &StyleCx<'_>, _: NotesEditorStyleKey) -> NotesEditorStyle {
        NotesEditorStyle {
            metrics: Metrics::new(),
            ink: cx.color(cx.tokens.color.text.normal),
            muted: cx.color(cx.tokens.color.text.muted),
            selection: cx.color(cx.tokens.color.state.text_selection),
            body_size: cx.tokens.typography.body.size,
            line_height: cx.tokens.typography.body.line_height,
            max_indent: MAX_INDENT,
        }
    }

    fn sense(&self) -> Sense {
        Sense::CLICK | Sense::DRAG | Sense::FOCUS | Sense::TAKE_FOCUS
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let style = cx.style;
        let size = style.body_size * self.zoom;
        let width = cx.room.unwrap_or(DEFAULT_WIDTH * self.zoom);
        let scale = size / DOCUMENT_SIZE;
        let font = FontRef::proportional();
        let ink = DataColor::new(style.ink.egui());
        let height: f32 = self
            .notes
            .paragraphs
            .iter()
            .map(|paragraph| {
                let (_, margin) = line_starts(paragraph.level, paragraph.margin, paragraph.indent);
                let runs = paragraph_runs(paragraph, &font, size, ink);
                let mut text = DocText::new(&runs);
                text.wrap = Some((width - indent(style, margin, scale)).max(0.0));
                cx.layout_doc(&text).size().y.max(size)
            })
            .sum();
        Vec2::new(width, height.max(style.line_height * self.zoom * MIN_LINES))
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        cx.editor(EDITOR, rect, EditorPurpose::Text);
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let inputs: Vec<EditorInput> = cx.editor_input().collect();
        let press = *cx.press();
        let pointed = match press.phase {
            PressPhase::Down { start, .. } if start.button == PointerButton::Primary => {
                Some((self.char_at(cx, start.origin), Some((start.clicks, press.mods.shift))))
            }
            PressPhase::DragStart { pos, .. } | PressPhase::Drag { pos, .. } => Some((self.char_at(cx, pos), None)),
            _ => None,
        };
        if inputs.is_empty() && pointed.is_none() {
            return;
        }
        let clip = if inputs.iter().any(|input| matches!(input, EditorInput::Edit(EditCommand::Paste))) {
            match cx.clipboard().paste() {
                Pending::Ready(content) => content.text.as_ref().map(|text| text.to_string()),
                _ => None,
            }
        } else {
            None
        };
        let now = cx.now();
        let mut cursor = *cx.memory::<Cursor>();
        let mut plain = self.notes.plain();
        if let Some((at, press)) = pointed {
            cursor.point(&plain, at, press);
        }
        let mut copied = None;
        let mut text_changed = false;
        for input in inputs {
            if matches!(input, EditorInput::Edit(EditCommand::Copy | EditCommand::Cut)) {
                let range = cursor.caret.min(cursor.anchor)..cursor.caret.max(cursor.anchor);
                copied = Some(plain.chars().skip(range.start).take(range.len()).collect::<String>()).filter(|text| !text.is_empty());
            }
            text_changed |= self.apply(&mut plain, &mut cursor, input, clip.as_deref());
        }
        if text_changed {
            *self.notes = self.notes.typed(&plain).unwrap_or_else(|| NotesText::from_plain(&plain));
            self.changed = true;
        }
        cursor.blink_start = now;
        *cx.memory::<Cursor>() = cursor;
        if let Some(text) = copied {
            cx.clipboard().put(&[ClipItem::Text(text.into())]);
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let painter = ui.content_painter(cx);
        let draw = ui.draw();
        let style = &cx.style;
        let area = cx.content_rect;
        let size = style.body_size * self.zoom;
        let scale = size / DOCUMENT_SIZE;
        let ink_color = style.ink.egui();
        let ink = DataColor::new(ink_color);
        let cursor = cx.recall::<Cursor>();
        let plain = self.notes.plain();
        let chars = plain.chars().count();
        let selected = cursor.caret.min(cursor.anchor).min(chars)..cursor.caret.max(cursor.anchor).min(chars);
        let marks = markers_of(self.notes);
        let font = FontRef::proportional();

        let mut y = area.top();
        let mut at = 0usize;
        let mut editor_lines: Vec<(String, Rect, f32)> = Vec::new();
        let mut caret_rect = Rect::from_min_size(area.min, Vec2::new(0.0, size));
        for (paragraph, marker) in self.notes.paragraphs.iter().zip(&marks) {
            let (first, margin) = line_starts(paragraph.level, paragraph.margin, paragraph.indent);
            let (first, margin) = (indent(style, first, scale), indent(style, margin, scale));
            let origin = area.left() + margin;
            let runs = paragraph_runs(paragraph, &font, size, ink);
            let mut text = DocText::new(&runs);
            text.wrap = Some((area.width() - (origin - area.left())).max(0.0));
            let layout = painter.layout(&text);
            let length: usize = paragraph.runs.iter().map(|run| run.text.chars().count()).sum();
            let range = at..at + length;
            let top_left = Pos2::new(origin, y);
            // Selection bands under the text, one per row the selection touches.
            if !selected.is_empty() && selected.start <= range.end && selected.end >= range.start {
                let (from, to) = (selected.start.max(range.start) - range.start, selected.end.min(range.end) - range.start);
                for band in selection_bands(&layout.galley, from, to) {
                    let band = band.translate(top_left.to_vec2());
                    draw.fill(
                        &PaintShape::Polygon(vec![band.left_top(), band.right_top(), band.right_bottom(), band.left_bottom()]),
                        style.selection,
                    );
                }
            }
            painter.painter.galley(top_left, layout.galley.clone(), ink_color);
            if let Some(marker) = marker {
                let marker_run = [DocRun::new(marker, font.clone(), size, ink)];
                let laid = painter.layout(&DocText::new(&marker_run));
                painter.painter.galley(Pos2::new(area.left() + first.min(margin), y), laid.galley, ink_color);
            }
            if (range.start..=range.end).contains(&cursor.caret.min(chars)) {
                let cursor_pos = layout.galley.pos_from_cursor(egui::text::CCursor::new(cursor.caret.min(chars) - range.start));
                caret_rect = Rect::from_min_max(top_left + cursor_pos.min.to_vec2(), top_left + cursor_pos.max.to_vec2());
            }
            let height = layout.size().y;
            editor_lines.push((
                paragraph.runs.iter().map(|run| run.text.as_str()).collect(),
                Rect::from_min_size(top_left, Vec2::new(layout.size().x, height)),
                top_left.y + layout.first_baseline,
            ));
            y += height.max(size);
            at = range.end + 1;
        }
        if self.notes.paragraphs.iter().all(|p| p.runs.iter().all(|r| r.text.is_empty())) && !cx.is_focused() {
            let words = cx.words::<NotesLabels>();
            draw.text(area, &TextRun::new(words.placeholder.as_str()).role(TypeRole::Body).color(style.muted));
        }
        if cx.is_focused() && selected.is_empty() && cx.blink(cursor.blink_start) {
            draw.stroke(&PaintShape::Path(vec![caret_rect.center_top(), caret_rect.center_bottom()]), StrokeWidth::Thin, style.ink);
        }
        let runs: Vec<[EditorRun<'_>; 1]> = editor_lines.iter().map(|(text, _, _)| [EditorRun::new(0..text.len(), false, &[], &[])]).collect();
        let lines: Vec<EditorLine<'_>> =
            editor_lines.iter().zip(&runs).map(|((text, rect, baseline), runs)| EditorLine::new(text, *rect, *baseline, runs)).collect();
        let position = |c: usize| {
            let (line, column) = locate(&plain, c.min(chars));
            let text = editor_lines.get(line).map_or("", |l| l.0.as_str());
            TextPos::new(line, text.char_indices().nth(column).map_or(text.len(), |(b, _)| b))
        };
        let mut state = EditorState::new(caret_rect, &lines, position(cursor.caret), revision(&plain));
        state.selection = (!selected.is_empty()).then(|| (position(cursor.anchor), position(cursor.caret)));
        cx.editor_state(state);
    }

    fn finish(self, cx: Cx<'_, Self>) -> NotesEditorOutput {
        NotesEditorOutput { response: cx.response.clone(), changed: self.changed }
    }
}

/// The indent of `points` document points at `scale`, within the largest indent.
fn indent(style: &NotesEditorStyle, points: f64, scale: f32) -> f32 {
    if points.is_finite() { points.clamp(0.0, f64::from(style.max_indent)) as f32 * scale } else { 0.0 }
}

/// The runs of `paragraph` at `size`: one document run per styled run, or one empty run for an empty paragraph so
/// it keeps a line.
fn paragraph_runs<'a>(paragraph: &'a NotesParagraph, font: &FontRef, size: f32, ink: DataColor) -> Vec<DocRun<'a>> {
    let runs: Vec<DocRun<'a>> = paragraph
        .runs
        .iter()
        .filter(|run| !run.text.is_empty())
        .map(|run| {
            let mut doc = DocRun::new(&run.text, font.clone(), size, run.color.map_or(ink, DataColor::new));
            if run.italic {
                doc = doc.italic();
            }
            if run.bold {
                doc = doc.bold();
            }
            if run.underline {
                doc = doc.underline();
            }
            if run.strikethrough {
                doc = doc.strikethrough();
            }
            doc
        })
        .collect();
    if runs.is_empty() { vec![DocRun::new("", font.clone(), size, ink)] } else { runs }
}

/// A number that changes with the notes' text, for the input method's revision.
fn revision(plain: &str) -> u64 {
    use std::hash::{Hash as _, Hasher as _};
    let mut hasher = std::hash::DefaultHasher::new();
    plain.hash(&mut hasher);
    hasher.finish()
}

/// The selection of the chars `from..to` of a laid-out paragraph as one band per galley row it touches, in the
/// galley's coordinates.
fn selection_bands(galley: &egui::Galley, from: usize, to: usize) -> Vec<Rect> {
    let mut bands = Vec::new();
    let mut start = 0usize;
    for placed in &galley.rows {
        let row = &placed.row;
        let length = row.char_count_excluding_newline().0;
        let (a, b) = (from.max(start), to.min(start + length));
        if a < b {
            let (x0, x1) = (placed.pos.x + row.x_offset(CharIndex(a - start)), placed.pos.x + row.x_offset(CharIndex(b - start)));
            let rect = placed.rect();
            bands.push(Rect::from_x_y_ranges(x0.min(x1)..=x0.max(x1), rect.y_range()));
        }
        start += placed.char_count_including_newline().0;
    }
    bands
}

/// The part key of the editor part.
const EDITOR: craft_behavior::PartKey = craft_behavior::PartKey::new("editor");
