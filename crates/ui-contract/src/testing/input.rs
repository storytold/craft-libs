//! Key events and the egui input options, for trials that drive frames themselves.

use crate::runtime::egui::fixture;
use egui::{Context, Event, Key, Modifiers};

/// A key press event with `modifiers`.
pub fn key_press(key: Key, modifiers: Modifiers) -> Event {
    fixture::key_press(key, modifiers)
}

/// The auto-repeat event of a held `key`.
pub fn key_repeat(key: Key, modifiers: Modifiers) -> Event {
    fixture::key_repeat(key, modifiers)
}

/// An event of typed `text`.
pub fn text(text: &str) -> Event {
    fixture::text_event(text)
}

/// Whether a press of `key` is still in this pass's input: no component claimed it.
pub fn key_reached(ctx: &Context, key: Key) -> bool {
    fixture::key_reached(ctx, key)
}

/// Gives egui back the option the runtime turns off at install: `Cmd` with `=` or `-` zooms the interface.
pub fn allow_zoom_keys(ctx: &Context) {
    fixture::allow_zoom_keys(ctx);
}

/// How far a press may move and still be a click, in points: the framework's slop.
pub fn slop(ctx: &Context) -> f32 {
    crate::runtime::frame::slop(ctx)
}
