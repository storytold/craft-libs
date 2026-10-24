//! Taking a frame's key input while the dialog records, before the app's dispatcher or egui's
//! widgets see it.

use ::egui::{Event, EventFilter, Id, InputState, Ui};

use crate::chord::{Chord, Key, Mods, Platform};
use crate::egui::{Clip, clipboard_chord, clipboard_kind};

/// The keys a recording widget keeps from egui's focus navigation, so Tab, arrows and Escape are
/// recorded instead of moving focus.
pub(crate) const FOCUS_LOCK: EventFilter = EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: true };

/// Holds egui's focus on `id` with [`FOCUS_LOCK`]; call each frame the widget records.
pub(crate) fn lock_focus(ui: &Ui, id: Id) {
    ui.memory_mut(|m| m.set_focus_lock_filter(id, FOCUS_LOCK));
}

/// Gives Tab, arrows and Escape back to egui's focus navigation. egui keeps a focused widget's
/// filter until focus moves, so a widget that recorded calls this each frame it has focus and
/// does not record.
pub(crate) fn unlock_focus(ui: &Ui, id: Id) {
    ui.memory_mut(|m| m.set_focus_lock_filter(id, EventFilter::default()));
}

/// Removes every key, text, IME and clipboard event from `input` and returns the chords pressed,
/// in order. Repeats, releases and modifier-only presses give no chord. Copy, Cut and Paste
/// count as the presses that cause them (Cmd+C, Cmd+X, Cmd+V, or Shift+Delete and Shift+Insert),
/// once when a backend sends both the press and the event.
pub(crate) fn take(input: &mut InputState, platform: Platform) -> Vec<Chord> {
    let frame_mods = Mods::from_egui(&input.modifiers, platform);
    let mut chords = Vec::new();
    let mut kept = Vec::with_capacity(input.events.len());
    let mut clipboard_press: Option<Clip> = None;
    for event in std::mem::take(&mut input.events) {
        match &event {
            Event::Key { key, pressed, repeat, modifiers, .. } => {
                let Some(key) = Key::from_egui(*key) else { continue };
                if *pressed && !*repeat {
                    let held = Mods::from_egui(modifiers, platform);
                    if let Some(clip) = clipboard_kind(key, held) {
                        clipboard_press = Some(clip);
                    }
                    chords.push(Chord::new(held, key));
                }
            }
            Event::Copy | Event::Cut | Event::Paste(_) => {
                let (key, held, clip) = clipboard_chord(&event, frame_mods);
                if clipboard_press == Some(clip) {
                    clipboard_press = None;
                } else {
                    chords.push(Chord::new(held, key));
                }
            }
            Event::Text(_) | Event::Ime(_) => {}
            _ => kept.push(event),
        }
    }
    input.events = kept;
    chords
}

#[cfg(test)]
mod tests {
    use ::egui::{Context, Key as EguiKey, Modifiers, RawInput};

    use super::*;

    fn run(events: Vec<Event>) -> (Vec<String>, usize) {
        let ctx = Context::default();
        let mut out = (Vec::new(), 0);
        let mut full = ctx.run_ui(RawInput { events, ..RawInput::default() }, |ui| {
            let chords = ui.ctx().input_mut(|i| take(i, Platform::Mac));
            out = (chords.iter().map(Chord::to_string).collect(), ui.ctx().input(|i| i.events.len()));
        });
        full.textures_delta.clear();
        out
    }

    fn press(key: EguiKey, modifiers: Modifiers) -> Event {
        Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }
    }

    #[test]
    fn takes_presses_and_clipboard_events_and_leaves_nothing() {
        let cmd = Modifiers::MAC_CMD;
        let events = vec![
            press(EguiKey::E, cmd),
            Event::Text("e".into()),
            Event::Key { key: EguiKey::E, physical_key: None, pressed: false, repeat: false, modifiers: cmd },
            press(EguiKey::Tab, Modifiers::NONE),
            // The web build: the press, then the event it causes, counted once.
            press(EguiKey::C, cmd),
            Event::Copy,
            // egui-winit: only the event.
            Event::Paste("x".into()),
            Event::PointerGone,
        ];
        let (chords, left) = run(events);
        assert_eq!(chords, ["Cmd+E", "Tab", "Cmd+C", "Cmd+V"]);
        assert_eq!(left, 1, "only the pointer event stays");
    }
}
