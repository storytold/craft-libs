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

/// egui's longest movement that still counts as a click.
pub fn max_click_dist(ctx: &Context) -> f32 {
    fixture::max_click_dist(ctx)
}

/// Sets egui's longest press that still counts as a click.
pub fn set_max_click_duration(ctx: &Context, seconds: f64) {
    fixture::set_max_click_duration(ctx, seconds);
}
