//! What the contract trials read: the decision records and findings of the pipeline.

use crate::runtime::inspect::{self, Finding};
use egui::{Context, Id};

/// The steps the latest inspected show of `id` ran, in order. Wrap the show in
/// [`crate::runtime::inspect::record`] to collect its steps.
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

/// Places a clickable egui widget in `rect` by hand, the way a bypass of the components does.
pub fn raw_widget(ui: &mut egui::Ui, rect: egui::Rect) {
    crate::runtime::egui::fixture::raw_widget(ui, rect);
}

/// Allocates a widget of `size` in `ui`.
pub fn allocate(ui: &mut egui::Ui, size: egui::Vec2) {
    crate::runtime::egui::fixture::allocate_blocker(ui, size);
}

thread_local! {
    static PARTS_CHECK: std::cell::RefCell<Option<Vec<&'static str>>> = const { std::cell::RefCell::new(None) };
}

/// Run a story with the parts declaration check enabled, and return any offending component types.
pub fn check_parts_declarations(run: impl FnOnce()) -> Vec<&'static str> {
    struct Reset(Option<Vec<&'static str>>);
    impl Drop for Reset {
        fn drop(&mut self) {
            PARTS_CHECK.with(|check| *check.borrow_mut() = self.0.take());
        }
    }
    let previous = PARTS_CHECK.with(|check| check.replace(Some(Vec::new())));
    let _reset = Reset(previous);
    run();
    PARTS_CHECK.with(|check| check.borrow_mut().take().unwrap_or_default())
}

pub(crate) fn checking_parts() -> bool {
    PARTS_CHECK.with(|check| check.borrow().is_some())
}

pub(crate) fn bad_parts(component: &'static str) {
    PARTS_CHECK.with(|check| {
        if let Some(bad) = check.borrow_mut().as_mut() {
            bad.push(component);
        }
    });
}
