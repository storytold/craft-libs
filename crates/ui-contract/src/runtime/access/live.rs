//! Announcements: transient text goes through `announce`, flushed at the end of each pass as one status node and
//! returned in `FrameOutput` for tests.

use super::Politeness;
use crate::runtime::egui::{access as adapter, pass};
use crate::runtime::frame::{self, PerViewport};
use crate::runtime::memory::global;
use egui::{Context, ViewportId};

/// What the frame has announced so far in one viewport.
#[derive(Default)]
struct Announced {
    frame: u64,
    texts: Vec<(String, Politeness)>,
    /// The announcements of the previous frame, and whether its status text carried the repeat marker.
    previous: String,
    previous_marked: bool,
    /// The same for this frame, as its last pass wrote it.
    current: String,
    current_marked: bool,
}

/// Ends the text of a status that repeats the previous frame's: some platform adapters raise a live-region event
/// only when the live node's text changes. Alternates, so three equal frames in a row still differ pairwise.
const REPEAT_MARK: char = '\u{200b}';

/// Announces `text`; for code without a context. Components call `cx.announce`. The same announcement made again
/// in the frame (the code ran in a discarded pass too) is kept once.
pub fn announce(ctx: &Context, text: impl Into<String>, politeness: Politeness) {
    if !crate::runtime::measure::effect(ctx, "announce") {
        return;
    }
    let text = text.into();
    let viewport = pass::current_viewport(ctx);
    let new = PerViewport::<Announced>::with(ctx, viewport, |announced| {
        let seen = announced.texts.iter().any(|(old, old_politeness)| *old == text && *old_politeness == politeness);
        if !seen {
            announced.texts.push((text.clone(), politeness));
        }
        !seen
    });
    if new {
        frame::announce(ctx, text);
    }
}

/// The begin hook: the first pass of a frame starts with no announcements; a discarded pass of the frame keeps them,
/// so the pass that is shown still carries the status node.
pub(crate) fn begin_pass(ctx: &Context) {
    let viewport = pass::current_viewport(ctx);
    let frame = pass::frame_nr(ctx, viewport);
    PerViewport::<Announced>::with(ctx, viewport, |announced| {
        if announced.frame != frame {
            announced.frame = frame;
            announced.texts.clear();
            announced.previous = std::mem::take(&mut announced.current);
            announced.previous_marked = std::mem::take(&mut announced.current_marked);
        }
    });
}

/// The end hook: the status node of the viewport, one live node every frame that holds the frame's announcements,
/// empty when there are none.
pub(crate) fn end_pass(ctx: &Context) {
    let viewport = pass::current_viewport(ctx);
    let (text, assertive) = PerViewport::<Announced>::with(ctx, viewport, |announced| {
        let assertive = announced.texts.iter().any(|(_, politeness)| *politeness == Politeness::Assertive);
        let plain = announced.texts.iter().map(|(text, _)| text.as_str()).collect::<Vec<_>>().join(". ");
        let marked = !plain.is_empty() && plain == announced.previous && !announced.previous_marked;
        let mut text = plain.clone();
        if marked {
            text.push(REPEAT_MARK);
        }
        announced.current = plain;
        announced.current_marked = marked;
        (text, assertive)
    });
    adapter::write_status(ctx, status_id(viewport), &text, assertive);
}

fn status_id(viewport: ViewportId) -> egui::Id {
    global("access.status", viewport)
}
