//! Shared internals for the dialog/window chrome helpers: the mirrored body layout and a
//! temporary direction scope. Kept private to `direction` so the generic direction-core
//! surface stays in `mod.rs`.

use egui::{Align, Context, Layout};

use super::{Direction, HasDirection, apply, is_rtl};

/// The body layout for dialogs and window bodies: in a right-to-left interface
/// `top_down(Align::Max)` makes egui prefer right-to-left, which right-aligns block children
/// and flips every nested `ui.horizontal*` row; left to right it is egui's default.
pub(crate) fn body_layout(d: &impl HasDirection) -> Layout {
    if is_rtl(d) { Layout::top_down(Align::Max) } else { Layout::top_down(Align::Min) }
}

/// Run with the context's direction temporarily `dir` — both the stored direction and egui's
/// `Style::right_to_left` flag — restored when the guard drops.
pub(crate) fn direction_scope(ctx: &Context, dir: Direction) -> impl Drop + use<> {
    struct Restore {
        ctx: Context,
        previous: Direction,
    }
    impl Drop for Restore {
        fn drop(&mut self) {
            apply(&self.ctx, self.previous);
        }
    }
    let previous = ctx.direction();
    apply(ctx, dir);
    Restore { ctx: ctx.clone(), previous }
}
