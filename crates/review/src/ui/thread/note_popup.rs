//! The note pop-up: a modeless window over a note's messages that keeps its size on screen at every zoom.

use std::collections::HashMap;

use craft_ui::contract::SizeLimits;
use craft_ui::runtime::env::Label;
use egui::{Response, Ui, vec2};

/// One message of a note pop-up.
#[derive(Clone, Debug, PartialEq)]
pub struct NotePopupMessage<'a> {
    /// The message's id.
    pub id: u64,
    /// The message's title line.
    pub title: &'a str,
    /// The author's name.
    pub author: &'a str,
    /// The message text.
    pub text: &'a str,
    /// Unix seconds of the last edit or of the creation; a message with no date shows none.
    pub modified: Option<i64>,
    /// How deep a reply sits; 0 is the comment.
    pub depth: usize,
    /// Whether the message can be edited.
    pub editable: bool,
}

/// The pop-ups' reply drafts, one per note.
#[derive(Clone, Debug, Default)]
pub struct NoteDrafts {
    #[allow(dead_code)] // P3(pdf-comments-w3): CM-C17
    drafts: HashMap<u64, String>,
}

/// The smallest size of a note pop-up, in points; it has no largest size.
pub const NOTE_POPUP_LIMITS: SizeLimits = SizeLimits::at_least(vec2(160.0, 96.0));

/// What a note pop-up did this pass.
#[derive(Debug)]
#[non_exhaustive]
pub struct NotePopupOutput {
    /// The pop-up's response.
    pub response: Response,
    /// The window's new rectangle, reported when a move or resize commits.
    pub placed: Option<egui::Rect>,
}

/// A modeless note pop-up window over a note's messages. It keeps its size on screen at every zoom, so it takes no
/// zoom scale; a move or resize is reported as [`NotePopupOutput::placed`].
#[must_use = "show the pop-up with `show`"]
#[allow(dead_code)] // P3(pdf-comments-w3): CM-C18
pub struct NotePopup<'a> {
    id: u64,
    label: Label,
    rect: egui::Rect,
    messages: &'a [NotePopupMessage<'a>],
    drafts: &'a mut NoteDrafts,
}

impl<'a> NotePopup<'a> {
    /// The pop-up `id` named `label` at `rect` over `messages`, keeping its reply drafts in `drafts`.
    pub fn new(id: u64, label: impl Into<Label>, rect: egui::Rect, messages: &'a [NotePopupMessage<'a>], drafts: &'a mut NoteDrafts) -> Self {
        NotePopup { id, label: label.into(), rect, messages, drafts }
    }

    /// The note's colour, straight RGB.
    #[must_use]
    #[allow(dead_code)] // P3(pdf-comments-w3): CM-C18
    pub fn color(self, color: Option<[u8; 3]>) -> Self {
        let _ = color;
        todo!("CM-C18")
    }

    /// Whether the note shows its checkmark.
    #[must_use]
    #[allow(dead_code)] // P3(pdf-comments-w3): CM-C18
    pub fn mark(self, mark: bool) -> Self {
        let _ = mark;
        todo!("CM-C18")
    }

    /// Whether the note is the selected one.
    #[must_use]
    #[allow(dead_code)] // P3(pdf-comments-w3): CM-C18
    pub fn selected(self, selected: bool) -> Self {
        let _ = selected;
        todo!("CM-C18")
    }

    /// The current time in Unix seconds, which the ages count from.
    #[must_use]
    #[allow(dead_code)] // P3(pdf-comments-w3): CM-C18
    pub fn now(self, unix_seconds: i64) -> Self {
        let _ = unix_seconds;
        todo!("CM-C18")
    }

    /// Whether the document allows commenting: replying and editing.
    #[must_use]
    #[allow(dead_code)] // P3(pdf-comments-w3): CM-C18
    pub fn allowed(self, allowed: bool) -> Self {
        let _ = allowed;
        todo!("CM-C18")
    }

    /// Shows the pop-up.
    #[allow(dead_code)] // P3(pdf-comments-w3): CM-C18
    pub fn show(self, ui: &mut Ui) -> NotePopupOutput {
        let _ = ui;
        todo!("CM-C18")
    }
}
