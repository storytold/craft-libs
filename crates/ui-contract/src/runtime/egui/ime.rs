//! Resetting and interrupting the OS composition, and the IME events and output.

use super::input::Arrived;
use egui::{Context, Event, ImeEvent, Rect, ViewportId};

/// An IME event as the engine reads it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Raw {
    Preedit {
        text: String,
        cursor_chars: Option<std::ops::Range<usize>>,
    },
    Commit(String),
    /// The input method deletes characters around the caret or the marked text.
    DeleteSurrounding {
        before: usize,
        after: usize,
    },
    /// The same, counted in bytes of UTF-8 text (winit on macOS and Wayland).
    DeleteSurroundingBytes {
        before: usize,
        after: usize,
    },
    /// The OS finished the reset with this request id.
    ResetCompleted(egui::ImeResetId),
    /// The integration dropped the reset request with this id; no completion follows.
    ResetRetired(egui::ImeResetId),
}

/// Removes the IME events from `events`, and the other events `drop` names, and returns the IME events in order, each
/// with its place: how many of the events left in `events` arrived before it. The key stage numbers the events it
/// reads by the same places (`keys::input_hook`), so an editor gets the two in the order they arrived.
pub(crate) fn take_events(events: &mut Vec<Arrived>, take: bool, drop: impl Fn(&Event) -> bool) -> Vec<(Raw, usize)> {
    let mut taken = Vec::new();
    let mut kept = 0;
    events.retain(|Arrived { event, .. }| {
        if let Some(raw) = take.then(|| raw_of(event)).flatten() {
            taken.extend(raw.map(|raw| (raw, kept)));
            return false;
        }
        let keep = !drop(event);
        kept += usize::from(keep);
        keep
    });
    taken
}

/// What the IME engine reads from `event`: `None` for an event that is not the engine's, `Some(None)` for one it takes
/// and ignores.
fn raw_of(event: &Event) -> Option<Option<Raw>> {
    let raw = match event {
        Event::Ime(ime) => match ime {
            ImeEvent::Preedit { text, active_range_chars } => Some(Raw::Preedit { text: text.clone(), cursor_chars: active_range_chars.clone() }),
            ImeEvent::Commit(text) => Some(Raw::Commit(text.clone())),
            ImeEvent::DeleteSurrounding { before_chars, after_chars } => Some(Raw::DeleteSurrounding { before: *before_chars, after: *after_chars }),
            ImeEvent::DeleteSurroundingBytes { before_bytes, after_bytes } => {
                Some(Raw::DeleteSurroundingBytes { before: *before_bytes, after: *after_bytes })
            }
            _ => None,
        },
        Event::ImeResetCompleted { request_id } => Some(Raw::ResetCompleted(*request_id)),
        Event::ImeResetRetired { request_id } => Some(Raw::ResetRetired(*request_id)),
        _ => return None,
    };
    Some(raw)
}

/// What the engine writes into `PlatformOutput.ime`.
pub(crate) struct Output {
    pub owner: egui::Id,
    pub field: Rect,
    pub caret: Rect,
    /// Interrupt the OS composition and ask for a reset.
    pub interrupt: bool,
    /// The context generation the tracker gave the owner: it changes when the owner, its epoch, its focus or its
    /// purpose changes, and stays for ordinary edits.
    pub generation: u64,
    pub password: bool,
    /// The bounded committed text around the caret, or `Unavailable`.
    pub surrounding: egui::output::IMESurroundingText,
    /// Whether the input method made the change to `surrounding`, as the tracker proved it from the receipts.
    pub cause: egui::output::ImeChangeCause,
}

/// Writes the pass's IME output; egui reads it only when it is `Some` at the end of the pass. Returns the reset
/// request an interrupt published: its completion is the one that matches (`None` when the ids ran out).
pub(crate) fn write(ctx: &Context, viewport: ViewportId, out: Output) -> Option<egui::ImeResetId> {
    let reset = out.interrupt.then(|| ctx.request_ime_reset(viewport)).flatten();
    ctx.output_mut(|output| {
        let mut ime = egui::output::IMEOutput::new(
            out.owner,
            out.generation,
            if out.password { egui::IMEPurpose::Password } else { egui::IMEPurpose::Normal },
            out.field,
            out.caret,
        );
        ime.should_interrupt_composition = out.interrupt;
        ime.reset_request = reset;
        ime.surrounding_text = out.surrounding;
        ime.change_cause = out.cause;
        output.ime = Some(ime);
    });
    reset
}

/// Drops what `TextEdit` wrote in a pass whose IME output the engine writes.
pub(crate) fn clear(ctx: &Context) {
    ctx.output_mut(|output| output.ime = None);
}
