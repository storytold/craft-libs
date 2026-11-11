//! `Title`: the document title the bar shows in the free gap between its leading content and its trailing end.

use crate::contract::{BodyCx, Props as _, WordSetCx as _};
use crate::display::{Elide, Text};
use crate::layout::{Placed, arrange};
use crate::shell::{ShellWords, slots};
use craft_behavior::Arrangement;
use egui::{Rect, Ui, Vec2, vec2};

/// The title of the window's document, with the mark that says it has unsaved changes.
///
/// The title is centred on the window's centre line when it fits between the leading content and the trailing end,
/// slides toward the nearer end when the centre line would put it over one, and is cut with an ellipsis when it does
/// not fit at all (the cut title shows whole in a tooltip).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Title {
    text: String,
    unsaved: bool,
}

impl Title {
    /// A title for a document with no unsaved changes.
    pub fn new(text: impl Into<String>) -> Self {
        Title { text: text.into(), unsaved: false }
    }

    /// A title for a document with unsaved changes.
    pub fn unsaved(text: impl Into<String>) -> Self {
        Title { text: text.into(), unsaved: true }
    }

    /// The text shown: the title, followed by an asterisk while the document has unsaved changes.
    pub fn shown(&self, words: &ShellWords) -> String {
        if self.unsaved { words.unsaved_title.fill(&[("title", self.text.as_str())]) } else { self.text.clone() }
    }

    /// Whether the document has unsaved changes.
    pub(super) fn is_unsaved(&self) -> bool {
        self.unsaved
    }

    /// Where a title `width` wide goes in a gap `free` wide, as (offset from the gap's start, width), given the
    /// window's centre line `centre` from the same start: on the centre line when it fits, else slid to the nearer
    /// end, else the whole gap.
    pub fn span(centre: f32, free: f32, width: f32) -> (f32, f32) {
        let free = free.max(0.0);
        if width >= free {
            return (0.0, free);
        }
        ((centre - width / 2.0).clamp(0.0, free - width), width)
    }

    /// Shows the title in the rest of `ui`, which is the bar body's row after the leading content. `body_width` is the
    /// body's whole width and `centre` the window's centre line from the body's start edge.
    pub(super) fn show_in_gap(&self, ui: &mut Ui, cx: &BodyCx<'_>, body_width: f32, centre: f32, margin: f32) {
        let words = cx.words::<ShellWords>();
        let mut text = Text::new(self.shown(&words)).elide(Elide::End);
        if self.unsaved {
            text = text.description(words.unsaved.as_str());
        }
        let gap = Gap { body_width, centre, margin };
        arrange(ui, &gap, &mut [Placed::new("title", text)]);
    }
}

/// The free gap: the room after the leading content, less a margin at both ends.
struct Gap {
    body_width: f32,
    centre: f32,
    margin: f32,
}

impl Arrangement for Gap {
    fn place(&self, room: Rect, sizes: &[Vec2], out: &mut Vec<Rect>) -> Vec2 {
        let size = sizes.first().copied().unwrap_or(Vec2::ZERO);
        let free = room.width() - 2.0 * self.margin;
        // The room ends where the body does, so its start lies this far from the body's.
        let room_start = self.body_width - room.width();
        let (from, width) = Title::span(self.centre - room_start - self.margin, free, size.x);
        out.push(Rect::from_min_size(room.min + vec2(self.margin + from, (room.height() - size.y) / 2.0), vec2(width, size.y)));
        vec2(room.width(), size.y)
    }
}

/// The window's centre line from the start edge of the bar body: the middle of the title slot where the bar sits in
/// a shell, else the middle of the body.
pub(super) fn centre_line(cx: &BodyCx<'_>, rtl: bool) -> f32 {
    let body = cx.rect;
    let middle = cx
        .shell()
        .and_then(|shell| shell.regions().slots.iter().find(|(key, _)| *key == slots::TITLE).map(|(_, rect)| rect.center().x))
        .unwrap_or_else(|| body.center().x);
    if rtl { body.right() - middle } else { middle - body.left() }
}
