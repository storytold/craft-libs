//! `EditorInput`: the one ordered input stream the IME engine produces and an editor part reads (18.4).

use crate::intent::{EditCommand, Granularity, MoveMods, NavMove, TextPos};
use std::ops::Range;
use std::sync::Arc;

/// What an input method counts the text around the caret in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SurroundingUnit {
    /// Characters.
    Chars,
    /// Bytes of UTF-8 text.
    Bytes,
}

impl SurroundingUnit {
    /// The characters `n` of this unit cover at the end of `text`, the editor's text before the deletion. A character
    /// the range only partly covers stays.
    pub fn chars_before(self, n: usize, text: &str) -> usize {
        match self {
            SurroundingUnit::Chars => n,
            SurroundingUnit::Bytes => {
                let from = text.len().saturating_sub(n);
                text.char_indices().filter(|(at, _)| *at >= from).count()
            }
        }
    }

    /// The characters `n` of this unit cover at the start of `text`, the editor's text after the deletion. A
    /// character the range only partly covers stays.
    pub fn chars_after(self, n: usize, text: &str) -> usize {
        match self {
            SurroundingUnit::Chars => n,
            SurroundingUnit::Bytes => text.char_indices().take_while(|(at, c)| at + c.len_utf8() <= n).count(),
        }
    }
}

/// One input of an editor part.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum EditorInput {
    /// Committed typing, possibly several characters.
    Text(Arc<str>),
    /// A logical move; `MoveMods.by` gives the unit.
    Move(NavMove, MoveMods),
    /// Delete or Backspace.
    #[non_exhaustive]
    Delete {
        /// Delete toward the reading end.
        forward: bool,
        /// How much to delete.
        by: Granularity,
    },
    /// Text around the caret or the marked text, deleted for the IME.
    #[non_exhaustive]
    DeleteSurrounding {
        /// How much lies before the caret, in `unit`.
        before: usize,
        /// How much lies after the caret, in `unit`.
        after: usize,
        /// What `before` and `after` count.
        unit: SurroundingUnit,
    },
    /// AccessKit `SetTextSelection`.
    #[non_exhaustive]
    Select {
        /// The anchor.
        anchor: TextPos,
        /// The focus.
        focus: TextPos,
    },
    /// Text shown inline and not yet part of the document.
    #[non_exhaustive]
    Preedit {
        /// The preedit text.
        text: Arc<str>,
        /// The IME's cursor, as a byte range of `text`.
        cursor: Option<Range<usize>>,
    },
    /// The preedit became text; delivered as `Text` right after.
    ImeCommit,
    /// An edit command from the command queue.
    Edit(EditCommand),
    /// Enter.
    Newline,
    /// Tab.
    Tab,
    /// Escape.
    Cancel,
}

impl EditorInput {
    /// A `Delete` of `by`.
    pub const fn delete(forward: bool, by: Granularity) -> Self {
        EditorInput::Delete { forward, by }
    }

    /// A `DeleteSurrounding` of `before` and `after` characters.
    pub const fn delete_surrounding(before: usize, after: usize) -> Self {
        EditorInput::DeleteSurrounding { before, after, unit: SurroundingUnit::Chars }
    }

    /// A `DeleteSurrounding` of `before` and `after` bytes of UTF-8 text, as winit reports it.
    pub const fn delete_surrounding_bytes(before: usize, after: usize) -> Self {
        EditorInput::DeleteSurrounding { before, after, unit: SurroundingUnit::Bytes }
    }

    /// A `Preedit` of `text` with the IME's cursor.
    pub fn preedit(text: impl Into<Arc<str>>, cursor: Option<Range<usize>>) -> Self {
        EditorInput::Preedit { text: text.into(), cursor }
    }

    /// A `Select` of `anchor` to `focus`.
    pub const fn select(anchor: TextPos, focus: TextPos) -> Self {
        EditorInput::Select { anchor, focus }
    }

    /// Whether the input changes no text: a read-only editor receives only these (18.4).
    pub const fn is_read_only_safe(&self) -> bool {
        match self {
            EditorInput::Move(..) | EditorInput::Select { .. } | EditorInput::Cancel => true,
            EditorInput::Edit(command) => matches!(command, EditCommand::Copy | EditCommand::SelectAll),
            _ => false,
        }
    }
}
