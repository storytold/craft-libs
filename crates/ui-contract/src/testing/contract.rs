//! What the contract trials read: the decision records and findings of the pipeline.

use crate::runtime::inspect::{self, Finding};
use egui::{Context, Id};

/// The steps the latest show of `id` ran, in order.
pub fn steps(ctx: &Context, id: Id) -> Vec<&'static str> {
    inspect::steps_of(ctx, id)
}

/// Every finding reported on `ctx`.
pub fn findings(ctx: &Context) -> Vec<Finding> {
    inspect::findings(ctx)
}

/// The canvas-control types that have shown on `ctx`.
pub fn canvas_controls(ctx: &Context) -> Vec<&'static str> {
    inspect::canvas_controls(ctx)
}

/// The reveal request addressed to the scroll owner `owner`, once.
pub fn take_reveal(ctx: &Context, owner: Id) -> Option<egui::Rect> {
    crate::contract::take_reveal(ctx, owner)
}

/// Queues a reveal of `rect` for the scroll owner `owner`, as a component that asked for it leaves it for the owner's
/// next pass.
pub fn queue_reveal(ctx: &Context, owner: Id, rect: egui::Rect) {
    crate::contract::queue_reveal(ctx, owner, rect);
}

/// An edit response with no events, for a trial component that has no `cx.edit_response()` to call.
pub fn edit_response<T>(response: egui::Response) -> crate::contract::EditResponse<T> {
    crate::contract::EditResponse::new(response, craft_behavior::EditEvents::new(), crate::contract::Validity::Ok)
}

/// Whether `response` reports a click, by pointer or by keyboard activation.
pub fn clicked(response: &egui::Response) -> bool {
    crate::runtime::egui::fixture::clicked(response)
}

/// Whether `response` reports a primary-button pointer click.
pub fn clicked_by_primary(response: &egui::Response) -> bool {
    crate::runtime::egui::fixture::clicked_by_primary(response)
}

/// Places a clickable egui widget in `rect` by hand, the way a bypass of the components does.
pub fn raw_widget(ui: &mut egui::Ui, rect: egui::Rect) {
    crate::runtime::egui::fixture::raw_widget(ui, rect);
}

/// Allocates a widget of `size` in `ui`.
pub fn allocate(ui: &mut egui::Ui, size: egui::Vec2) {
    crate::runtime::egui::fixture::allocate_blocker(ui, size);
}
