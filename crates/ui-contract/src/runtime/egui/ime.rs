//! Resetting and interrupting the OS composition, and the IME events and output.

use egui::{Context, Event, ImeEvent, Rect, ViewportId};

/// An IME event as the engine reads it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Raw {
    Preedit {
        text: String,
        cursor_chars: Option<std::ops::Range<usize>>,
    },
    Commit(String),
    /// The OS finished the reset the engine asked for.
    ResetCompleted,
}

/// Removes the IME events from `events` and returns them in order.
pub(crate) fn take_events(events: &mut Vec<Event>) -> Vec<Raw> {
    let mut taken = Vec::new();
    events.retain(|event| match event {
        Event::Ime(ime) => {
            match ime {
                ImeEvent::Preedit { text, active_range_chars } => {
                    taken.push(Raw::Preedit { text: text.clone(), cursor_chars: active_range_chars.clone() })
                }
                ImeEvent::Commit(text) => taken.push(Raw::Commit(text.clone())),
                _ => {}
            }
            false
        }
        Event::ImeResetCompleted { .. } => {
            taken.push(Raw::ResetCompleted);
            false
        }
        _ => true,
    });
    taken
}

/// What the engine writes into `PlatformOutput.ime`.
pub(crate) struct Output {
    pub owner: egui::Id,
    pub field: Rect,
    pub caret: Rect,
    /// Interrupt the OS composition and ask for a reset.
    pub interrupt: bool,
    /// Changes when the owner changes.
    pub generation: u64,
    pub password: bool,
}

/// Writes the pass's IME output; egui reads it only when it is `Some` at the end of the pass.
pub(crate) fn write(ctx: &Context, viewport: ViewportId, out: Output) {
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
        output.ime = Some(ime);
    });
}

/// Drops what `TextEdit` wrote: the engine is the one writer.
pub(crate) fn clear(ctx: &Context) {
    ctx.output_mut(|output| output.ime = None);
}
