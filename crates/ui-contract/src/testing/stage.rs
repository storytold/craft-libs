//! The room a story that fills both axes is shown in.

use crate::layout::flow_room;
use crate::runtime::env::Env;
use egui::{Rect, Ui};

/// Allocates the stage of a story of a canvas control or a root that fills both axes, and returns its rect: the width
/// left in `ui` between the least and greatest stage width, and the stage height, all from the size tokens. Show the
/// story's root over it with `show_at`.
pub fn stage(ui: &mut Ui) -> Rect {
    let height = Env::of(ui).tokens.tokens().size.story.height;
    stage_fitting(ui, |_| height)
}

/// The height of the room `ui` offers a story whose root has no content height and fills it (a window, a full-window
/// canvas): the room its card's view gives, or the stage height where the room is unbounded.
pub fn room_height(ui: &Ui) -> f32 {
    let room = flow_room(ui).y;
    if room.is_finite() { room } else { Env::of(ui).tokens.tokens().size.story.height }
}

/// Allocates a stage as wide as [`stage`]'s and as tall as the story's content at that width, which `height` gives,
/// and returns its rect: the stage of a canvas whose document is only as tall as its content (a text being edited).
pub fn stage_fitting(ui: &mut Ui, height: impl FnOnce(f32) -> f32) -> Rect {
    let width = stage_width(ui);
    crate::runtime::egui::alloc::space(ui, egui::vec2(width, height(width)))
}

/// The width of the stage [`stage_fitting`] allocates: the width left in `ui` between the least and greatest stage
/// width. A story whose content measures itself at a width (through `ui`) reads it before allocating.
pub fn stage_width(ui: &Ui) -> f32 {
    let story = Env::of(ui).tokens.tokens().size.story;
    let room = flow_room(ui).x;
    let room = if room.is_finite() { room } else { ui.available_width() };
    room.clamp(story.width_min, story.width_max)
}
