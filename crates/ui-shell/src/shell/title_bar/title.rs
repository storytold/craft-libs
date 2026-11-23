//! `Title`: the document title the bar shows in the free gap between its leading content and its trailing end.

use crate::buttons::Button;
use crate::contract::{BodyCx, Props as _, WordSetCx as _, clicked};
use crate::display::{Elide, Text};
use crate::layout::{Placed, arrange};
use crate::runtime::env::Label;
use crate::shell::{ShellWords, slots};
use craft_behavior::Arrangement;
use craft_theme::Variant;
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
    /// The tooltip of a title that is a button; `None` for a title that is text.
    action: Option<Label>,
}

impl Title {
    /// A title for a document with no unsaved changes.
    pub fn new(text: impl Into<String>) -> Self {
        Title { text: text.into(), unsaved: false, action: None }
    }

    /// A title for a document with unsaved changes.
    pub fn unsaved(text: impl Into<String>) -> Self {
        Title { text: text.into(), unsaved: true, action: None }
    }

    /// Makes the title a focusable button named by the shown title, with `tooltip`; the bar reports its activation in
    /// `TitleBarOutput::title_activated`. Without it the title is text.
    #[must_use]
    pub fn action(mut self, tooltip: impl Into<Label>) -> Self {
        self.action = Some(tooltip.into());
        self
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
    /// body's whole width and `centre` the window's centre line from the body's start edge. Returns whether a title with
    /// an action was activated in this pass.
    pub(super) fn show_in_gap(&self, ui: &mut Ui, cx: &BodyCx<'_>, body_width: f32, centre: f32, margin: f32) -> bool {
        let words = cx.words::<ShellWords>();
        let gap = Gap { body_width, centre, margin };
        let description = self.unsaved.then(|| words.unsaved.as_str().to_owned());
        match &self.action {
            Some(tooltip) => {
                // A ghost button has no fill until hovered, so the title reads as text; its label is cut with an
                // ellipsis when the gap is too narrow, as the text is.
                let mut button = Button::new(self.shown(&words)).variant(Variant::GHOST).tooltip(tooltip.clone());
                if let Some(description) = description {
                    button = button.description(description);
                }
                let mut placed = [Placed::new("title", button)];
                arrange(ui, &gap, &mut placed);
                placed.first().and_then(Placed::response).is_some_and(clicked)
            }
            None => {
                let mut text = Text::new(self.shown(&words)).elide(Elide::End);
                if let Some(description) = description {
                    text = text.description(description);
                }
                arrange(ui, &gap, &mut [Placed::new("title", text)]);
                false
            }
        }
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
        // The title takes no width of its own: it lies in what the leading content leaves, so a member that fills the
        // row (a document tab strip) takes the free width and the title the rest, if any.
        vec2(0.0, size.y)
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
