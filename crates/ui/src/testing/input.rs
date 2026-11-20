//! Pointer, key, text and wheel input for an egui_kittest [`Harness`], with a fixed number of
//! frames per step so a test can count on when the app sees each event.
//!
//! Each function puts its events straight into the harness's input for the frame it belongs to
//! and runs exactly the frames listed below. The counts hold only when no events are waiting in
//! the harness's queue: [`Harness::step`] gives each event queued with [`Harness::event`] (or a
//! kittest node action) a frame of its own, so a step's first frame then runs once per queued
//! event, with the step's own events in the first of them. Run the harness after queuing events
//! and before calling these functions.
//!
//! | Step | Frames | Events, frame by frame |
//! |------|--------|------------------------|
//! | [`move_to`] | 1 | pointer moved |
//! | [`press`] | 2 | modifiers, pointer moved, button down; nothing |
//! | [`release`] | 2 | modifiers, pointer moved, button up; modifiers released |
//! | [`click`] | 3 | modifiers, pointer moved, button down; button up; modifiers released |
//! | [`drag`] | `steps` + 3 | modifiers, pointer moved, primary down; one move per step; primary up; modifiers released |
//! | [`key`] | 2 | modifiers, key down, key up; modifiers released |
//! | [`type_text`] | 2 | text; nothing |
//! | [`wheel`] | 2 | modifiers, wheel; modifiers released |
//!
//! The modifiers are sent as [`Event::ModifiersChanged`] and in each pointer, key and wheel event.
//! [`press`] leaves them held, so moves between a [`press`] and its [`release`] see them too; the
//! other steps release them in their last frame. A drag with another button is a [`press`], some
//! [`move_to`] calls and a [`release`].
//!
//! ```no_run
//! use craft_ui::testing::input;
//! use egui::{Modifiers, PointerButton, pos2, vec2};
//!
//! let mut harness = egui_kittest::Harness::new_ui(|ui| _ = ui.button("Open"));
//! input::click(&mut harness, pos2(20.0, 12.0), PointerButton::Primary, Modifiers::NONE);
//! input::drag(&mut harness, pos2(20.0, 40.0), pos2(120.0, 40.0), 4, Modifiers::SHIFT);
//! input::wheel(&mut harness, vec2(0.0, -48.0), Modifiers::COMMAND);
//! ```

#[cfg(test)]
mod tests;

use egui::{Event, Key, Modifiers, MouseWheelSource, MouseWheelUnit, PointerButton, Pos2, TouchPhase, Vec2};
use egui_kittest::Harness;

/// The most moves [`drag`] sends; a larger `steps` is reduced to this.
pub const MAX_DRAG_STEPS: usize = 1024;

/// Run one frame with `events` as its input.
fn frame<State>(harness: &mut Harness<'_, State>, events: impl IntoIterator<Item = Event>) {
    harness.input_mut().events.extend(events);
    harness.step();
}

fn button(pos: Pos2, button: PointerButton, pressed: bool, modifiers: Modifiers) -> Event {
    Event::PointerButton { pos, button, pressed, modifiers }
}

/// Move the pointer to `pos`. 1 frame.
pub fn move_to<State>(harness: &mut Harness<'_, State>, pos: Pos2) {
    frame(harness, [Event::PointerMoved(pos)]);
}

/// Press `pointer` at `pos` with `modifiers` held, and keep both held. 2 frames.
pub fn press<State>(harness: &mut Harness<'_, State>, pos: Pos2, pointer: PointerButton, modifiers: Modifiers) {
    frame(harness, [Event::ModifiersChanged(modifiers), Event::PointerMoved(pos), button(pos, pointer, true, modifiers)]);
    frame(harness, []);
}

/// Release `pointer` at `pos`, then release the modifiers. 2 frames.
pub fn release<State>(harness: &mut Harness<'_, State>, pos: Pos2, pointer: PointerButton, modifiers: Modifiers) {
    frame(harness, [Event::ModifiersChanged(modifiers), Event::PointerMoved(pos), button(pos, pointer, false, modifiers)]);
    frame(harness, [Event::ModifiersChanged(Modifiers::NONE)]);
}

/// Click `pointer` at `pos` with `modifiers` held. 3 frames.
pub fn click<State>(harness: &mut Harness<'_, State>, pos: Pos2, pointer: PointerButton, modifiers: Modifiers) {
    frame(harness, [Event::ModifiersChanged(modifiers), Event::PointerMoved(pos), button(pos, pointer, true, modifiers)]);
    frame(harness, [button(pos, pointer, false, modifiers)]);
    frame(harness, [Event::ModifiersChanged(Modifiers::NONE)]);
}

/// Drag with the primary button from `from` to `to` in `steps` even moves, the last one at `to`.
/// `steps` is at least 1 and at most [`MAX_DRAG_STEPS`]. `steps` + 3 frames.
pub fn drag<State>(harness: &mut Harness<'_, State>, from: Pos2, to: Pos2, steps: usize, modifiers: Modifiers) {
    let steps = steps.clamp(1, MAX_DRAG_STEPS);
    let primary = PointerButton::Primary;
    frame(harness, [Event::ModifiersChanged(modifiers), Event::PointerMoved(from), button(from, primary, true, modifiers)]);
    for step in 1..=steps {
        let pos = if step == steps { to } else { from.lerp(to, step as f32 / steps as f32) };
        frame(harness, [Event::PointerMoved(pos)]);
    }
    frame(harness, [button(to, primary, false, modifiers)]);
    frame(harness, [Event::ModifiersChanged(Modifiers::NONE)]);
}

/// Press and release `key` with `modifiers` held, in one frame, as a platform reports a quick
/// key press. The events carry no physical key and are not repeats. 2 frames.
pub fn key<State>(harness: &mut Harness<'_, State>, key: Key, modifiers: Modifiers) {
    let event = |pressed| Event::Key { key, physical_key: None, pressed, repeat: false, modifiers };
    frame(harness, [Event::ModifiersChanged(modifiers), event(true), event(false)]);
    frame(harness, [Event::ModifiersChanged(Modifiers::NONE)]);
}

/// Type `text` into the focused widget as one text event. Empty text sends no event. 2 frames.
pub fn type_text<State>(harness: &mut Harness<'_, State>, text: &str) {
    frame(harness, (!text.is_empty()).then(|| Event::Text(text.to_owned())));
    frame(harness, []);
}

/// Turn the wheel by `delta` points with `modifiers` held. The event's phase is
/// [`TouchPhase::Move`] and its source [`MouseWheelSource::Unknown`]. A positive `y` moves the
/// content down, as egui defines it. 2 frames.
pub fn wheel<State>(harness: &mut Harness<'_, State>, delta: Vec2, modifiers: Modifiers) {
    let event = Event::MouseWheel { unit: MouseWheelUnit::Point, delta, phase: TouchPhase::Move, source: MouseWheelSource::Unknown, modifiers };
    frame(harness, [Event::ModifiersChanged(modifiers), event]);
    frame(harness, [Event::ModifiersChanged(Modifiers::NONE)]);
}
