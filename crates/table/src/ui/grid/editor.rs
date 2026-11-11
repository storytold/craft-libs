//! The in-cell editor of the grid: the draft, its modes and formula point mode.

use craft_behavior::{EditorInput, Granularity, MoveMods, NavMove, TextPos};
use std::ops::Range;

use super::{CellAt, CellRange, absolute_range_reference};

/// The characters after which a formula expects a reference.
const OPERATORS: &str = "=+-*/(,^&<>:;";

/// How a cell is open.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Mode {
    /// Not open.
    #[default]
    Ready,
    /// Typing replaced the content: the arrows move to another cell.
    Enter,
    /// F2 or a double click: the arrows move the caret.
    Edit,
}

/// A single-line draft with a caret, a selection anchor and the composition in progress.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Draft {
    text: String,
    caret: usize,
    anchor: Option<usize>,
    preedit: Option<String>,
    revision: u64,
}

fn previous(text: &str, at: usize) -> usize {
    text[..at].char_indices().next_back().map_or(0, |(i, _)| i)
}

fn next(text: &str, at: usize) -> usize {
    text[at..].chars().next().map_or(text.len(), |c| at + c.len_utf8())
}

fn word_start(text: &str, at: usize) -> usize {
    let head = &text[..at];
    let trimmed = head.trim_end().len();
    head[..trimmed].trim_end_matches(char::is_alphanumeric).len()
}

fn word_end(text: &str, at: usize) -> usize {
    let tail = &text[at..];
    let skipped = tail.len() - tail.trim_start().len();
    let rest = &tail[skipped..];
    at + skipped + (rest.len() - rest.trim_start_matches(char::is_alphanumeric).len())
}

impl Draft {
    pub(super) fn text(&self) -> &str {
        &self.text
    }

    pub(super) fn set_text(&mut self, text: &str) {
        self.text = text.to_owned();
        self.caret = self.text.len();
        self.anchor = None;
        self.preedit = None;
        self.revision += 1;
    }

    pub(super) fn revision(&self) -> u64 {
        self.revision
    }

    pub(super) fn caret(&self) -> usize {
        self.caret
    }

    /// The text as shown: the composition sits at the caret.
    pub(super) fn shown(&self) -> String {
        match &self.preedit {
            Some(preedit) => format!("{}{preedit}{}", &self.text[..self.caret], &self.text[self.caret..]),
            None => self.text.clone(),
        }
    }

    pub(super) fn caret_shown(&self) -> usize {
        self.caret + self.preedit.as_ref().map_or(0, String::len)
    }

    pub(super) fn selection(&self) -> Option<(TextPos, TextPos)> {
        self.anchor.filter(|&anchor| anchor != self.caret).map(|anchor| (TextPos::new(0, anchor), TextPos::new(0, self.caret)))
    }

    fn selected(&self) -> Option<Range<usize>> {
        self.anchor.filter(|&anchor| anchor != self.caret).map(|anchor| anchor.min(self.caret)..anchor.max(self.caret))
    }

    fn insert(&mut self, text: &str) {
        let text = text.replace(['\n', '\r'], "");
        if let Some(range) = self.selected() {
            self.text.replace_range(range.clone(), "");
            self.caret = range.start;
        }
        self.anchor = None;
        self.text.insert_str(self.caret, &text);
        self.caret += text.len();
        self.revision += 1;
    }

    fn delete(&mut self, forward: bool, by: Granularity) {
        if let Some(range) = self.selected() {
            self.text.replace_range(range.clone(), "");
            self.caret = range.start;
            self.anchor = None;
            self.revision += 1;
            return;
        }
        let (from, to) = match (forward, by) {
            (false, Granularity::Word) => (word_start(&self.text, self.caret), self.caret),
            (false, Granularity::Line | Granularity::Paragraph | Granularity::Document) => (0, self.caret),
            (false, Granularity::Unit) => (previous(&self.text, self.caret), self.caret),
            (true, Granularity::Word) => (self.caret, word_end(&self.text, self.caret)),
            (true, Granularity::Line | Granularity::Paragraph | Granularity::Document) => (self.caret, self.text.len()),
            (true, Granularity::Unit) => (self.caret, next(&self.text, self.caret)),
            (_, _) => (self.caret, self.caret),
        };
        if from < to {
            self.text.replace_range(from..to, "");
            self.caret = from;
            self.revision += 1;
        }
    }

    fn move_by(&mut self, step: NavMove, mods: MoveMods) {
        let to = match (step, mods.by) {
            (NavMove::Back, Granularity::Word) => word_start(&self.text, self.caret),
            (NavMove::Forward, Granularity::Word) => word_end(&self.text, self.caret),
            (NavMove::Back, _) => match (self.selected(), mods.extend) {
                (Some(range), false) => range.start,
                _ => previous(&self.text, self.caret),
            },
            (NavMove::Forward, _) => match (self.selected(), mods.extend) {
                (Some(range), false) => range.end,
                _ => next(&self.text, self.caret),
            },
            (NavMove::Start | NavMove::Up | NavMove::PageUp, _) => 0,
            (NavMove::End | NavMove::Down | NavMove::PageDown, _) => self.text.len(),
            _ => self.caret,
        };
        if mods.extend {
            self.anchor.get_or_insert(self.caret);
        } else {
            self.anchor = None;
        }
        self.caret = to;
    }

    /// Applies an editor input to the draft; false when the input does nothing to a single line.
    pub(super) fn apply(&mut self, input: EditorInput) {
        match input {
            EditorInput::Text(text) => {
                self.preedit = None;
                self.insert(&text);
            }
            EditorInput::Delete { forward, by, .. } => self.delete(forward, by),
            EditorInput::DeleteSurrounding { before, after, .. } => {
                for _ in 0..before {
                    self.delete(false, Granularity::Unit);
                }
                for _ in 0..after {
                    self.delete(true, Granularity::Unit);
                }
            }
            EditorInput::Move(step, mods) => self.move_by(step, mods),
            EditorInput::Select { anchor, focus, .. } => {
                self.anchor = Some(anchor.byte.min(self.text.len()));
                self.caret = focus.byte.min(self.text.len());
            }
            EditorInput::Preedit { text, .. } => self.preedit = Some(text.to_string()).filter(|text| !text.is_empty()),
            EditorInput::ImeCommit => self.preedit = None,
            EditorInput::Cancel => self.anchor = None,
            _ => {}
        }
    }

    /// Replaces the text from `from` to the end with `reference` (point mode).
    pub(super) fn point_at(&mut self, from: usize, reference: &str) {
        self.text.truncate(from.min(self.text.len()));
        self.text.push_str(reference);
        self.caret = self.text.len();
        self.anchor = None;
        self.revision += 1;
    }
}

/// The reference cursor of formula point mode: the cell it started at, the range it names and where in the draft the
/// reference starts.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Point {
    pub(super) anchor: CellAt<u32>,
    pub(super) range: CellRange<u32>,
    pub(super) from: usize,
}

/// The cell being edited and its draft.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct EditState {
    pub(super) mode: Mode,
    pub(super) cell: Option<CellAt<u32>>,
    pub(super) draft: Draft,
    pub(super) point: Option<Point>,
    /// End was pressed and no arrow has followed: the next arrow jumps to the edge of the data region.
    pub(super) end_mode: bool,
}

impl EditState {
    /// Whether the draft is a formula waiting for a reference.
    pub(super) fn pointing(&self) -> bool {
        let text = self.draft.text();
        self.mode != Mode::Ready && (self.point.is_some() || text.starts_with('=') && text.ends_with(|c: char| OPERATORS.contains(c)))
    }

    pub(super) fn open(&mut self, cell: CellAt<u32>, mode: Mode, text: &str) {
        self.mode = mode;
        self.cell = Some(cell);
        self.point = None;
        self.draft.set_text(text);
    }

    pub(super) fn close(&mut self) {
        self.mode = Mode::Ready;
        self.point = None;
    }

    /// Moves the reference cursor to `to` and writes its reference into the draft. `extend` grows the range from the
    /// cell the cursor started at (Shift+arrow).
    pub(super) fn point_to(&mut self, to: CellAt<u32>, extend: bool) {
        let from = self.point.as_ref().map_or(self.draft.caret(), |point| point.from);
        let anchor = match (&self.point, extend) {
            (Some(point), true) => point.anchor,
            _ => to,
        };
        let range = CellRange::cells(anchor, to);
        self.draft.point_at(from, &absolute_range_reference(range.clone()));
        self.point = Some(Point { anchor, range, from });
    }
}
