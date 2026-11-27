//! Decoration-free indentation for dialog fields and inspector subgroups.

use egui::{InnerResponse, Rect, Ui, UiBuilder, pos2};

/// Physical left/right padding around a scope, in logical points.
///
/// Unlike a tree indent, this has no guide line and does not require a vertical parent. It
/// supports fractional theme spacing without changing global `Spacing::indent` or visuals.
/// The child inherits layout, enabled state and clipping; choose an inner layout as usual when
/// placing a vertical set of fields inside a horizontal parent. Translation, RTL padding policy,
/// widget identity and whether controls are enabled remain the application's responsibility.
#[derive(Clone, Copy, Debug, Default)]
pub struct ContentInset {
    left: f32,
    right: f32,
}

impl ContentInset {
    /// Indent from the left. Negative/non-finite values become zero; finite values are capped
    /// at one million points and further restricted to the available width when shown.
    pub fn new(left: f32) -> Self {
        Self { left: amount(left), right: 0.0 }
    }

    /// Set the physical right inset in logical points.
    pub fn right(mut self, right: f32) -> Self {
        self.right = amount(right);
        self
    }

    /// Allocate the child's used space and return its value and response. The left inset takes
    /// priority in narrow parents; the right inset uses the remaining width. Insets introduce
    /// no vertical gap or frame. Repeated controls can use an enclosing `Ui::push_id`.
    pub fn show<R>(self, ui: &mut Ui, add_contents: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
        let mut rect = ui.available_rect_before_wrap();
        let left = self.left.min(rect.width().max(0.0));
        rect.min.x += left;
        let right = self.right.min(rect.width().max(0.0));
        rect.max.x -= right;
        ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
            let inner = add_contents(ui);
            // Wrapping uses available width, but the parent consumes only the used rectangle.
            // Reserve both physical insets after layout without changing content positions/IDs.
            let used = ui.min_rect();
            ui.expand_to_include_rect(Rect::from_min_max(pos2(used.left() - left, used.top()), pos2(used.right() + right, used.bottom())));
            inner
        })
    }
}

fn amount(value: f32) -> f32 {
    if value.is_finite() { value.clamp(0.0, 1_000_000.0) } else { 0.0 }
}
