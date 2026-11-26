//! The notes' rich-text data model: runs, paragraphs, the flat character sequence and char-range helpers.

use std::ops::Range;

use egui::Color32;

use crate::lists::{ListMarker, ListParagraph, markers};

/// A run of notes text with its formatting.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct NotesRun {
    /// The run's text.
    pub text: String,
    /// Bold.
    pub bold: bool,
    /// Italic.
    pub italic: bool,
    /// Underline.
    pub underline: bool,
    /// Strikethrough.
    pub strikethrough: bool,
    /// `None`: the editor's text colour.
    pub color: Option<Color32>,
}

impl NotesRun {
    /// A plain run of `text`.
    pub fn new(text: impl Into<String>) -> Self {
        NotesRun { text: text.into(), ..NotesRun::default() }
    }

    /// The run bold (`on`) or not.
    #[must_use]
    pub fn bold(mut self, on: bool) -> Self {
        self.bold = on;
        self
    }

    /// The run italic (`on`) or not.
    #[must_use]
    pub fn italic(mut self, on: bool) -> Self {
        self.italic = on;
        self
    }

    /// The run underlined (`on`) or not.
    #[must_use]
    pub fn underline(mut self, on: bool) -> Self {
        self.underline = on;
        self
    }

    /// The run struck through (`on`) or not.
    #[must_use]
    pub fn strikethrough(mut self, on: bool) -> Self {
        self.strikethrough = on;
        self
    }

    /// The run in `color` (`None`: the editor's text colour).
    #[must_use]
    pub fn color(mut self, color: Option<Color32>) -> Self {
        self.color = color;
        self
    }
}

/// One paragraph: its runs, list level and marker, and its margins (points; `None` takes
/// [`crate::lists::LEVEL_INDENT`] per level).
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct NotesParagraph {
    /// The runs of the paragraph, in order.
    pub runs: Vec<NotesRun>,
    /// List level; 0 is the outermost.
    pub level: u8,
    /// The list marker, if the paragraph is a list item.
    pub marker: Option<ListMarker>,
    /// Start-edge margin of the lines after the first, in points.
    pub margin: Option<f64>,
    /// First-line indent in points, added to `margin`; negative hangs the first line.
    pub indent: Option<f64>,
}

impl NotesParagraph {
    /// A paragraph of `runs`, at level 0 without a list marker.
    pub fn new(runs: Vec<NotesRun>) -> Self {
        NotesParagraph { runs, ..NotesParagraph::default() }
    }

    /// The paragraph at list `level`, with `marker` if it is a list item.
    #[must_use]
    pub fn list(mut self, level: u8, marker: Option<ListMarker>) -> Self {
        self.level = level;
        self.marker = marker;
        self
    }

    /// The start-edge `margin` of the lines after the first and the first-line `indent`, in points.
    #[must_use]
    pub fn margins(mut self, margin: Option<f64>, indent: Option<f64>) -> Self {
        self.margin = margin;
        self.indent = indent;
        self
    }
}

/// The notes as the editor sees them.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct NotesText {
    /// The paragraphs, in order.
    pub paragraphs: Vec<NotesParagraph>,
}

impl NotesText {
    /// Notes of `paragraphs`.
    pub fn new(paragraphs: Vec<NotesParagraph>) -> Self {
        NotesText { paragraphs }
    }

    /// Plain paragraphs, one per line of `text`.
    pub(super) fn from_plain(text: &str) -> NotesText {
        let paragraph = |line: &str| NotesParagraph {
            runs: if line.is_empty() { Vec::new() } else { vec![NotesRun { text: line.to_owned(), ..NotesRun::default() }] },
            ..NotesParagraph::default()
        };
        NotesText { paragraphs: text.split('\n').map(paragraph).collect() }
    }

    /// The plain text, paragraphs joined with `\n` (what the text field edits).
    pub fn plain(&self) -> String {
        let paragraphs: Vec<String> = self.paragraphs.iter().map(|p| p.runs.iter().map(|r| r.text.as_str()).collect()).collect();
        paragraphs.join("\n")
    }

    /// The notes as they read with `text` typed in as if in place: only the span between the
    /// longest common prefix and suffix of [`plain`](Self::plain) and `text` changes (so one
    /// insertion, a deletion, or a selection replaced by typing all count), untouched text keeps
    /// its runs and paragraph settings, and the new text takes the formatting where it lands.
    /// Line endings in `text` are read as `\n` first. `None` when the two share no prefix and no
    /// suffix: the app then replaces the notes with plain paragraphs.
    pub fn typed(&self, text: &str) -> Option<NotesText> {
        let old: Vec<char> = self.plain().chars().collect();
        let new: Vec<char> = text.replace("\r\n", "\n").replace('\r', "\n").chars().collect();
        if old == new {
            return Some(self.clone());
        }
        let prefix = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
        let room = old.len().min(new.len()) - prefix;
        let suffix = old.iter().rev().zip(new.iter().rev()).take(room).take_while(|(a, b)| a == b).count();
        if prefix == 0 && suffix == 0 {
            return None;
        }
        let mut flat = Flat::of(self);
        flat.replace(prefix..old.len() - suffix, &new[prefix..new.len() - suffix]);
        Some(flat.into_notes())
    }
}

/// The formatting of one character.
type Style = NotesRun;

/// One element of the notes as a flat sequence: a character with its formatting, or the break
/// that starts the next paragraph (carrying that paragraph's own settings).
#[derive(Clone)]
enum Item {
    Char(char, Style),
    Break(NotesParagraph),
}

/// The notes as the sequence of characters and paragraph breaks that `plain` writes out.
struct Flat {
    first: NotesParagraph,
    items: Vec<Item>,
}

impl Flat {
    fn of(notes: &NotesText) -> Flat {
        let bare = |p: &NotesParagraph| NotesParagraph { runs: Vec::new(), ..p.clone() };
        let mut flat = Flat { first: notes.paragraphs.first().map(bare).unwrap_or_default(), items: Vec::new() };
        for (i, p) in notes.paragraphs.iter().enumerate() {
            if i > 0 {
                flat.items.push(Item::Break(bare(p)));
            }
            for run in &p.runs {
                let style = NotesRun { text: String::new(), ..run.clone() };
                flat.items.extend(run.text.chars().map(|c| Item::Char(c, style.clone())));
            }
        }
        flat
    }

    /// Replaces the items in `span` with `text`: the text takes the formatting of the character
    /// before it in its paragraph, else of the one after; a typed line break starts a paragraph
    /// with the settings of the one it splits; a deleted break joins the paragraphs under the
    /// first one's settings.
    fn replace(&mut self, span: std::ops::Range<usize>, text: &[char]) {
        let paragraph_start = |items: &[Item], at: usize| items[..at].iter().rposition(|i| matches!(i, Item::Break(_))).map_or(0, |b| b + 1);
        let style = {
            let from = paragraph_start(&self.items, span.start);
            let before = (span.start > from).then(|| &self.items[span.start - 1]);
            let after = self.items.get(span.end).filter(|_| span.start == from);
            match before.or(after) {
                Some(Item::Char(_, style)) => style.clone(),
                _ => Style::default(),
            }
        };
        let split = {
            let from = paragraph_start(&self.items, span.start);
            match from {
                0 => self.first.clone(),
                b => match &self.items[b - 1] {
                    Item::Break(p) => p.clone(),
                    Item::Char(..) => NotesParagraph::default(),
                },
            }
        };
        let typed: Vec<Item> = text.iter().map(|&c| if c == '\n' { Item::Break(split.clone()) } else { Item::Char(c, style.clone()) }).collect();
        self.items.splice(span, typed);
    }

    fn into_notes(self) -> NotesText {
        let mut paragraphs = Vec::new();
        let mut current = self.first;
        for item in self.items {
            match item {
                Item::Char(c, style) => match current.runs.last_mut() {
                    Some(run) if NotesRun { text: String::new(), ..run.clone() } == style => run.text.push(c),
                    _ => current.runs.push(NotesRun { text: c.to_string(), ..style }),
                },
                Item::Break(next) => paragraphs.push(std::mem::replace(&mut current, next)),
            }
        }
        paragraphs.push(current);
        NotesText { paragraphs }
    }
}
pub(super) fn markers_of(notes: &NotesText) -> Vec<Option<String>> {
    let list: Vec<ListParagraph> = notes
        .paragraphs
        .iter()
        .map(|p| ListParagraph { level: p.level, marker: p.marker.clone(), empty: p.runs.iter().all(|r| r.text.is_empty()) })
        .collect();
    markers(&list)
}

/// The chars of paragraph `at` of `plain`'s lines.
pub(super) fn paragraph_range(plain: &str, at: usize) -> Range<usize> {
    let mut start = 0;
    for (index, line) in plain.split('\n').enumerate() {
        let end = start + line.chars().count();
        if index == at {
            return start..end;
        }
        start = end + 1;
    }
    start..start
}

/// The paragraph holding char `c` and the char's offset within it.
pub(super) fn locate(plain: &str, c: usize) -> (usize, usize) {
    let mut start = 0;
    for (index, line) in plain.split('\n').enumerate() {
        let end = start + line.chars().count();
        if c <= end {
            return (index, c - start);
        }
        start = end + 1;
    }
    (0, 0)
}

/// Replaces the chars `range` of `plain` with `with`.
pub(super) fn splice(plain: &mut String, range: Range<usize>, with: &str) {
    let byte = |chars: usize, text: &str| text.char_indices().nth(chars).map_or(text.len(), |(b, _)| b);
    let (from, to) = (byte(range.start, plain), byte(range.end, plain));
    plain.replace_range(from..to, with);
}
