//! The room a story that fills both axes is shown in.

use crate::layout::flow_room;
use crate::runtime::env::Env;
use craft_theme::Tokens;
use egui::{Rect, Ui};

/// The stage's height, in control heights (`size.control.md`): 360 in Compact.
const HEIGHT_IN_CONTROLS: f32 = 15.0;

/// The stage's least width, in control heights: 240 in Compact.
const WIDTH_MIN_IN_CONTROLS: f32 = 10.0;

/// The stage's greatest width, in control heights: 720 in Compact.
const WIDTH_MAX_IN_CONTROLS: f32 = 30.0;

/// The height of a story's stage.
fn stage_height(tokens: &Tokens) -> f32 {
    HEIGHT_IN_CONTROLS * tokens.size.control.md
}

/// The least width of a story's stage, which is also the least width of a column of a story grid.
pub fn least_stage_width(tokens: &Tokens) -> f32 {
    WIDTH_MIN_IN_CONTROLS * tokens.size.control.md
}

/// Allocates the stage of a story of a canvas control or a root that fills both axes, and returns its rect: the width
/// left in `ui` between the least and greatest stage width, and the stage height, all in control heights. Show the
/// story's root over it with `show_at`.
pub fn stage(ui: &mut Ui) -> Rect {
    let height = stage_height(&Env::of(ui).tokens);
    stage_fitting(ui, |_| height)
}

/// The height of the room `ui` offers a story whose root has no content height and fills it (a window, a full-window
/// canvas): the room its card's view gives, or the stage height where the room is unbounded.
pub fn room_height(ui: &Ui) -> f32 {
    let env = Env::of(ui);
    let room = flow_room(ui, &env).y;
    if room.is_finite() { room } else { stage_height(&env.tokens) }
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
    let env = Env::of(ui);
    let tokens = &env.tokens;
    let room = flow_room(ui, &env).x;
    let room = if room.is_finite() { room } else { ui.available_width() };
    room.clamp(least_stage_width(tokens), WIDTH_MAX_IN_CONTROLS * tokens.size.control.md)
}
