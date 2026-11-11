//! `EditorInput`: the one ordered input stream the IME engine produces and an editor part reads (18.4).

use crate::intent::{EditCommand, Granularity, MoveMods, NavMove, TextPos};
use std::ops::Range;
use std::sync::Arc;

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
    /// Characters around the caret, deleted for the IME.
    #[non_exhaustive]
    DeleteSurrounding {
        /// Characters before the caret.
        before: usize,
        /// Characters after the caret.
        after: usize,
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
