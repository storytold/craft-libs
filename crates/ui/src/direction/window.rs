//! Floating-window chrome: the window body scope with its LTR sizing pass, first-show placement
//! and min-size clamping. See [`super::dialog`] for the mirror-exactly-once rule.

use egui::{Align, InnerResponse, Layout, Rect, Ui, pos2};

use super::{
    Direction, HasDirection, is_rtl,
    scope::{body_layout, direction_scope},
};

/// Wrap a floating window's (or dialog's) content. Call once at the top of each
/// `egui::Window` body closure. In a right-to-left interface the sizing pass runs in an LTR
/// scope — under the mirrored body layout the measured `min_rect` right-anchors and grows
/// leftward, so width-reading rows would balloon the window and the slack would land on the
/// reading-end side — and only the visible pass runs under the mirrored body layout. Left to
/// right the content draws unchanged.
pub fn window_body<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    if is_rtl(ui) {
        if ui.is_sizing_pass() {
            let _ltr = direction_scope(ui.ctx(), Direction::Ltr);
            let inner = add(ui);
            InnerResponse::new(inner, ui.response())
        } else {
            ui.with_layout(body_layout(ui), add)
        }
    } else {
        let inner = add(ui);
        InnerResponse::new(inner, ui.response())
    }
}

/// A resizable host's contents (a modal dialog body): in a right-to-left interface its sizing
/// pass runs left-to-right — stored direction, egui's direction flag and the body layout — so
/// the content measures its natural minimum left-anchored exactly as in LTR, and the enforced
/// minimum is direction-independent. Non-sizing frames and LTR run unchanged.
pub fn sizing_pass_ltr<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    if is_rtl(ui) && ui.is_sizing_pass() {
        let _ltr = direction_scope(ui.ctx(), Direction::Ltr);
        ui.with_layout(Layout::top_down(Align::Min), add).inner
    } else {
        add(ui)
    }
}

/// First-show x of a floating panel window `width` wide, `inset` px inside the canvas's
/// reading-end edge, shifted `cascade` px further toward the reading end (cascading panels;
/// negative cascades run back toward the canvas centre). Clamped to keep 8 px of the window on
/// the canvas on both sides.
pub fn float_window_x(d: &impl HasDirection, canvas: Rect, width: f32, inset: f32, cascade: f32) -> f32 {
    if is_rtl(d) {
        (canvas.left() + inset - cascade).min(canvas.right() - 8.0 - width).max(canvas.left() + 8.0)
    } else {
        (canvas.right() - width - inset + cascade).max(canvas.left() + 8.0)
    }
}

/// Clamp a resizable window's rect to exactly `min`, keeping the anchored edge fixed: the right
/// edge in a right-to-left interface (the resize grip is on the left), the left edge otherwise.
/// The top edge is fixed in both directions. The returned rect's size is exactly `min`, so
/// shrink drags are honoured in both directions.
pub fn clamp_window_min_size(d: &impl HasDirection, rect: Rect, min: egui::Vec2) -> Rect {
    if is_rtl(d) { Rect::from_min_size(pos2(rect.right() - min.x, rect.top()), min) } else { Rect::from_min_size(rect.min, min) }
}
