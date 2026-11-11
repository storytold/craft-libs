//! Pointer events and gestures for trials that drive frames themselves.

use crate::runtime::egui::fixture;
use crate::testing::Frames;
use egui::{Event, Pos2, Ui, Vec2};

/// The pointer moving to `pos`.
pub fn moved(pos: Pos2) -> Event {
    fixture::pointer_moved(pos)
}

/// The primary button going down at `pos`.
pub fn press(pos: Pos2) -> Event {
    fixture::primary_button(pos, true)
}

/// The primary button going up at `pos`.
pub fn release(pos: Pos2) -> Event {
    fixture::primary_button(pos, false)
}

/// The pointer leaving the window.
pub fn gone() -> Event {
    fixture::pointer_gone()
}

/// The window losing focus.
pub fn window_blurred() -> Event {
    fixture::window_blurred()
}

/// A vertical wheel event of `dy` points.
pub fn wheel(dy: f32) -> Event {
    fixture::wheel_event(dy)
}

/// A mouse wheel turning `dy` lines at the pointer.
pub fn wheel_lines(dy: f32) -> Event {
    fixture::wheel_lines_event(dy)
}

/// The secondary button going down or up at `pos`.
pub fn secondary(pos: Pos2, pressed: bool) -> Event {
    fixture::secondary_button(pos, pressed)
}

/// The middle button going down or up at `pos`.
pub fn middle(pos: Pos2, pressed: bool) -> Event {
    fixture::middle_button(pos, pressed)
}

/// A vertical wheel event of `dy` points with Ctrl or Cmd held, which zooms.
pub fn zoom_wheel(dy: f32) -> Event {
    fixture::zoom_wheel_event(dy)
}

/// Hover in one frame, press in the next, release in the next, then one idle frame.
pub fn click(frames: &mut Frames, at: Pos2, show: &mut impl FnMut(&mut Ui)) {
    frames.frame(vec![moved(at)], &mut *show);
    frames.frame(vec![press(at)], &mut *show);
    frames.frame(vec![release(at)], &mut *show);
    frames.frame(vec![], &mut *show);
}

/// A touchpad tap: hover in one frame, then press and release inside the next. `gone` adds the pointer
/// leaving, as a touch screen reports a lift.
pub fn tap(frames: &mut Frames, at: Pos2, gone_after: bool, show: &mut impl FnMut(&mut Ui)) {
    frames.frame(vec![moved(at)], &mut *show);
    let mut events = vec![press(at), release(at)];
    if gone_after {
        events.push(gone());
    }
    frames.frame(events, &mut *show);
    frames.frame(vec![], &mut *show);
}

/// Press at `from`, move to `from + by` in four steps over four frames, release.
pub fn drag(frames: &mut Frames, from: Pos2, by: Vec2, show: &mut impl FnMut(&mut Ui)) {
    frames.frame(vec![moved(from)], &mut *show);
    frames.frame(vec![press(from)], &mut *show);
    for step in 1..=4 {
        frames.frame(vec![moved(from + by * (step as f32 / 4.0))], &mut *show);
    }
    frames.frame(vec![release(from + by)], &mut *show);
    frames.frame(vec![], &mut *show);
}

/// Asks the pointer service to end every captured press in the running pass, as Escape does. Call it from the
/// frame closure before the roots show.
pub fn cancel_captured_presses(ctx: &egui::Context) {
    crate::runtime::pointer::cancel_press(ctx, craft_behavior::PressCancel::Escape);
}

/// Whether an open press holds the pointer in the running viewport, as Escape reads it.
pub fn press_captured(ctx: &egui::Context) -> bool {
    crate::runtime::pointer::press_captured(ctx)
}
