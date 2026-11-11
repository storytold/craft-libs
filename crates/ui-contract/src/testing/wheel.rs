//! The wheel service from a trial's side: registering takers and reading their shares.

use crate::runtime::egui::fixture;
use crate::runtime::wheel;
use craft_behavior::WheelTaker;
use egui::{Context, Id, Vec2};

/// Registers `taker` for the next pass's split.
pub fn register(ctx: &Context, taker: WheelTaker<Id>) {
    wheel::register(ctx, taker);
}

/// The scroll `id` takes this pass, if it has a share.
pub fn take(ctx: &Context, id: Id) -> Option<Vec2> {
    wheel::take(ctx, id).map(|share| share.scroll)
}

/// The whole share `id` takes this pass: scroll, zoom and the pointer a zoom is about.
pub fn take_share(ctx: &Context, id: Id) -> Option<craft_behavior::Wheel> {
    wheel::take(ctx, id)
}

/// Gives key focus to the widget `id`.
pub fn focus(ctx: &Context, id: Id) {
    fixture::focus(ctx, id);
}

/// The taker of last pass a reveal of `rect` goes to: the innermost scroll owner that meets it.
pub fn nearest_scroll_owner(ctx: &Context, rect: egui::Rect) -> Option<Id> {
    wheel::nearest_scroll_owner(ctx, rect)
}
