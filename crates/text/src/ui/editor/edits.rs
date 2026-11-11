//! The editing rules of [`TextEditor`](super::TextEditor): one pass of inputs applied to the host's text, the
//! selection and the marked text. Offsets are chars of the host's text.

use std::hash::{Hash, Hasher};
use std::ops::Range;

use craft_behavior::kurbo::Point;
use craft_behavior::{Granularity, MoveMods, NavMove};

use super::{EditKind, TextEditorHost, TextSelection};
use crate::quotes::{QuoteStyle, smart_quotes};

/// Most clicks in a run that select more: 2 a word, 3 a line, 4 a paragraph, 5 all text.
const MAX_CLICKS: u32 = 5;

/// What the editor keeps between passes of one editing session.
#[derive(Clone, Debug, Default)]
pub(super) struct Session {
    /// The marked text's chars in the host's text while composing.
    pub preedit: Option<Range<usize>>,
    /// The converted segment within the marked text, in chars from its start.
    pub preedit_active: Option<Range<usize>>,
    /// The marked text as the editor last wrote it.
    pub preedit_text: String,
    /// A drag is extending the selection by the unit the press selected (the click run's count).
    pub dragging: Option<u32>,
    /// The chars the press selected, which a drag by word, line or paragraph keeps selected.
    pub drag_origin: Range<usize>,
    /// The pass time of the last edit, move or press: the caret blink restarts from it.
    pub blink_start: f64,
    /// Length and hash of the host's text as the last pass left it.
    pub seen: Option<(usize, u64)>,
    /// The selection the last pass left, to tell the app's own selection changes.
    pub last_selection: Option<TextSelection>,
}

/// What a pass of inputs did.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Done {
    pub changed: bool,
    pub commit: bool,
    pub cancel: bool,
}

/// Length and hash of `text`, to tell whether it changed between passes.
pub(super) fn fingerprint(text: &str) -> (usize, u64) {
    let mut hasher = std::hash::DefaultHasher::new();
    text.hash(&mut hasher);
    (text.len(), hasher.finish())
}

/// The chars of `text` in `range`.
pub(super) fn char_slice(text: &str, range: &Range<usize>) -> String {
    text.chars().skip(range.start).take(range.len()).collect()
}

/// The chars of the paragraph holding char `c`, without its line break.
fn paragraph_of(chars: &[char], c: usize) -> Range<usize> {
    let is_break = |ch: &char| matches!(ch, '\n' | '\r');
    let start = chars[..c].iter().rposition(is_break).map_or(0, |i| i + 1);
    let end = chars[c..].iter().position(is_break).map_or(chars.len(), |i| c + i);
    start..end
}

/// The selection a run of `count` quick clicks makes at char `at`: 1 a caret, 2 the word (letters and digits)
/// around it, 3 the laid-out line `line`, 4 the paragraph up to its line break, 5 all of `text`. Offsets past the
/// text are clamped.
pub fn click_selection(text: &str, at: usize, line: Option<Range<usize>>, count: u32) -> Range<usize> {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let c = at.min(n);
    match count {
        2 => {
            let word = |i: usize| chars.get(i).is_some_and(|ch| ch.is_alphanumeric());
            let (mut a, mut b) = (c, c);
            while a > 0 && word(a - 1) {
                a -= 1;
            }
            while word(b) {
                b += 1;
            }
            a..b
        }
        3 => line.map_or_else(|| paragraph_of(&chars, c), |l| l.start.min(n)..l.end.min(n)),
        4 => paragraph_of(&chars, c),
        5 => 0..n,
        _ => c..c,
    }
}

/// The selection unit of a click that is the `clicks`-th press of its run: 1 a caret, then 2 a word up to 5 all
/// text; the sixth press starts over at 1.
pub fn click_count(clicks: u32) -> u32 {
    (clicks.max(1) - 1) % MAX_CLICKS + 1
}

/// The next word edge from char `from`: forward skips non-word chars then word chars (letters and digits), backward
/// the same leftward in logical order.
pub fn word_boundary(text: &str, from: usize, forward: bool) -> usize {
    let chars: Vec<char> = text.chars().collect();
    let mut i = from.min(chars.len());
    if forward {
        while i < chars.len() && !chars[i].is_alphanumeric() {
            i += 1;
        }
        while i < chars.len() && chars[i].is_alphanumeric() {
            i += 1;
        }
    } else {
        while i > 0 && !chars[i - 1].is_alphanumeric() {
            i -= 1;
        }
        while i > 0 && chars[i - 1].is_alphanumeric() {
            i -= 1;
        }
    }
    i
}

/// The move a key means in text that runs vertically: Up is the previous char, Down the next, Back the next column
/// and Forward the previous one (right-to-left columns). Other moves and horizontal text unchanged.
pub(super) fn flow_move(step: NavMove, vertical: bool) -> NavMove {
    if !vertical {
        return step;
    }
    match step {
        NavMove::Up => NavMove::Back,
        NavMove::Down => NavMove::Forward,
        NavMove::Back => NavMove::Down,
        NavMove::Forward => NavMove::Up,
        other => other,
    }
}

/// One pass of edits over the host's text.
pub(super) struct Edits<'a> {
    pub host: &'a mut dyn TextEditorHost,
    pub session: &'a mut Session,
    pub selection: TextSelection,
    pub smart_quotes: Option<QuoteStyle>,
    pub esc_commits: bool,
    pub done: Done,
}

impl Edits<'_> {
    fn chars(&self) -> usize {
        self.host.text().chars().count()
    }

    /// The selection as a range within the text.
    pub fn clamped(&self) -> Range<usize> {
        let chars = self.chars();
        let range = self.selection.range();
        range.start.min(chars)..range.end.min(chars)
    }

    fn clamp_selection(&mut self) {
        let chars = self.chars();
        self.selection = TextSelection { caret: self.selection.caret.min(chars), anchor: self.selection.anchor.min(chars) };
    }

    /// Ends the composition in the editor's state, keeping the marked text as typed.
    pub fn end_composition(&mut self) {
        self.session.preedit = None;
        self.session.preedit_active = None;
    }

    /// Whether the marked range still holds the text the composition wrote.
    fn preedit_is_current(&self) -> bool {
        let text = self.host.text();
        self.session.preedit.as_ref().is_some_and(|r| r.end <= text.chars().count() && char_slice(text, r) == self.session.preedit_text)
    }

    /// Reconciles the editor with a text changed from outside (undo, redo, a panel edit): the selection is clamped
    /// and a marked range that no longer holds the composition's text is dropped.
    pub fn reconcile(&mut self) {
        if self.session.seen.is_some_and(|seen| seen != fingerprint(self.host.text())) && !self.preedit_is_current() {
            self.end_composition();
        }
        self.clamp_selection();
    }

    /// Replace `range` through the host and put the caret after the new text.
    fn replace(&mut self, range: Range<usize>, text: &str, kind: EditKind) -> bool {
        if !self.host.replace(range.clone(), text, kind) {
            return false;
        }
        self.selection = TextSelection::caret_at(range.start + text.chars().count());
        true
    }

    /// Replace the selection with `text` (line endings become `\n`). An application edit ends a live composition.
    pub fn insert_text(&mut self, text: &str, kind: EditKind) -> bool {
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        let range = self.clamped();
        if range.is_empty() && text.is_empty() {
            return false;
        }
        if !self.replace(range, &text, kind) {
            return false;
        }
        self.end_composition();
        true
    }

    /// Typed text: straight quotes follow the style, when there is one.
    pub fn insert_typed(&mut self, text: &str) -> bool {
        let Some(style) = self.smart_quotes else {
            return self.insert_text(text, EditKind::Typed);
        };
        let start = self.selection.range().start;
        let before = start.checked_sub(1).and_then(|i| self.host.text().chars().nth(i));
        let text = smart_quotes(text, before, style);
        self.insert_text(&text, EditKind::Typed)
    }

    /// The selected chars, for the clipboard; `None` with an empty selection.
    pub fn selected_text(&self) -> Option<String> {
        let range = self.clamped();
        (!range.is_empty()).then(|| char_slice(self.host.text(), &range))
    }

    /// Delete the selection ([`EditKind::Deleted`]); the caller has copied it.
    pub fn delete_selection(&mut self) -> bool {
        let range = self.clamped();
        !range.is_empty() && self.replace(range, "", EditKind::Deleted)
    }

    /// New marked text (or its end, with empty `text`). Returns the range of marked text that ended, for a commit
    /// to replace.
    pub fn preedit(&mut self, text: &str, active: Option<Range<usize>>) -> Option<Range<usize>> {
        let chars = self.chars();
        let marked = self.session.preedit.take().filter(|r| r.end <= chars);
        let active = active.filter(|a| a.end <= text.chars().count());
        if text.is_empty() {
            self.session.preedit_active = None;
            return marked;
        }
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        let range = marked.unwrap_or_else(|| self.clamped());
        if self.replace(range.clone(), &text, EditKind::Composing) {
            self.session.preedit = Some(range.start..range.start + text.chars().count());
            self.session.preedit_text = text;
            self.session.preedit_active = active;
            self.done.changed = true;
        }
        None
    }

    /// The text a composition committed, replacing its `marked` range.
    pub fn commit_marked(&mut self, marked: Range<usize>, text: &str) {
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        if self.replace(marked, &text, EditKind::Committed) {
            self.done.changed = true;
        }
    }

    /// The marked text that ended without a commit is deleted.
    pub fn drop_marked(&mut self, marked: Range<usize>) {
        if self.replace(marked, "", EditKind::Composing) {
            self.done.changed = true;
        }
    }

    /// An input method's delete-surrounding: the chars around the marked text, or the selection.
    pub fn delete_around(&mut self, before: usize, after: usize) -> bool {
        let chars = self.chars();
        let range = self.session.preedit.clone().filter(|r| r.end <= chars).unwrap_or_else(|| self.clamped());
        let (start, end) = (range.start, range.end);
        let to = end.saturating_add(after).min(chars);
        let from = start.saturating_sub(before);
        if from >= start && end >= to {
            return false;
        }
        // The trailing run first, so the leading run's offsets stay valid.
        if end < to && !self.host.replace(end..to, "", EditKind::Deleted) {
            return false;
        }
        if from < start && !self.host.replace(from..start, "", EditKind::Deleted) {
            return false;
        }
        let shift = start - from;
        self.selection = TextSelection { caret: self.selection.caret.saturating_sub(shift), anchor: self.selection.anchor.saturating_sub(shift) };
        if let Some(r) = self.session.preedit.as_mut() {
            *r = r.start.saturating_sub(shift)..r.end.saturating_sub(shift);
        }
        true
    }

    /// Move the caret to `to` (clamped), extending the selection with `extend`.
    fn move_caret(&mut self, to: usize, extend: bool) {
        self.selection.caret = to.min(self.chars());
        if !extend {
            self.selection.anchor = self.selection.caret;
        }
    }

    /// A logical move of the caret.
    pub fn motion(&mut self, step: NavMove, mods: MoveMods) {
        self.clamp_selection();
        let text = self.host.text().to_owned();
        let chars = text.chars().count();
        let caret = self.selection.caret;
        let range = self.selection.range();
        let step = flow_move(step, self.host.vertical());
        let to = match step {
            NavMove::Back | NavMove::Forward => {
                let forward = step == NavMove::Forward;
                if mods.by != Granularity::Unit {
                    word_boundary(&text, caret, forward)
                } else if !range.is_empty() && !mods.extend {
                    if forward { range.end } else { range.start }
                } else if forward {
                    caret + 1
                } else {
                    caret.saturating_sub(1)
                }
            }
            NavMove::Up | NavMove::Down => {
                let up = step == NavMove::Up;
                if matches!(mods.by, Granularity::Paragraph | Granularity::Document) {
                    if up { 0 } else { chars }
                } else {
                    self.host.line_step(caret, if up { -1 } else { 1 })
                }
            }
            NavMove::Start | NavMove::End => {
                let end = step == NavMove::End;
                if mods.by == Granularity::Document { if end { chars } else { 0 } } else { self.host.line_edge(caret, end) }
            }
            NavMove::PageUp => 0,
            NavMove::PageDown => chars,
            _ => caret,
        };
        self.move_caret(to, mods.extend);
    }

    /// Backspace or Delete.
    pub fn delete(&mut self, forward: bool, by: Granularity) -> bool {
        self.clamp_selection();
        let text = self.host.text().to_owned();
        let chars = text.chars().count();
        let range = self.clamped();
        let range = if range.is_empty() {
            let at = range.start;
            let to = match (forward, by) {
                (false, Granularity::Line | Granularity::Paragraph | Granularity::Document) => self.host.line_edge(at, false),
                (false, Granularity::Word) => word_boundary(&text, at, false),
                (false, _) => at.saturating_sub(1),
                (true, Granularity::Unit) => (at + 1).min(chars),
                (true, _) => word_boundary(&text, at, true),
            };
            to.min(at)..to.max(at)
        } else {
            range
        };
        if range.is_empty() {
            return false;
        }
        self.replace(range, "", EditKind::Deleted)
    }

    /// Select everything.
    pub fn select_all(&mut self) {
        self.selection = TextSelection { anchor: 0, caret: self.chars() };
    }

    /// A primary press at `at` (document units), the `clicks`-th of its run, at pass time `time`: places the caret
    /// (Shift extends), selects by the click run's count and starts a drag.
    pub fn press(&mut self, at: Point, shift: bool, clicks: u32, time: f64) {
        self.end_composition();
        let (offset, line) = self.host.hit(at);
        let chars = self.chars();
        let offset = offset.min(chars);
        let count = click_count(clicks);
        self.selection.caret = offset;
        if !shift {
            self.selection.anchor = offset;
        }
        self.session.drag_origin = offset..offset;
        if count >= 2 {
            let line = (count == 3).then(|| self.host.line_range(line));
            let range = click_selection(self.host.text(), offset, line, count);
            self.selection = TextSelection { anchor: range.start, caret: range.end };
            self.session.drag_origin = range;
        }
        self.session.dragging = Some(count);
        self.session.blink_start = time;
    }

    /// The pointer moved with the button held after a press: extend the selection by the press's unit.
    pub fn drag(&mut self, at: Point) {
        let Some(count) = self.session.dragging else { return };
        let (offset, line) = self.host.hit(at);
        let offset = offset.min(self.chars());
        if count <= 1 {
            self.selection.caret = offset;
            return;
        }
        let line = (count == 3).then(|| self.host.line_range(line));
        let range = click_selection(self.host.text(), offset, line, count);
        let origin = self.session.drag_origin.clone();
        self.selection = if range.start < origin.start {
            TextSelection { anchor: origin.end, caret: range.start }
        } else {
            TextSelection { anchor: origin.start, caret: range.end.max(origin.end) }
        };
    }
}
