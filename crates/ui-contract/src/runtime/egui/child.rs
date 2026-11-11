//! Child `Ui`s: env scopes and gates (`ui.disable()`, a scope's own direction and layout), `new_child`,
//! `UiBuilder::scope_id`, `accessibility_parent`, the egui layout of each `BodyLayout` and sense.

use egui::{Align, Layout, Rect, Ui, UiBuilder};

/// The physical layout of a body with no layout of its own: top down, items hugging the reading start.
fn stack_layout(right_to_left: bool) -> Layout {
    Layout::top_down(if right_to_left { Align::Max } else { Align::Min })
}

/// The physical layout of a `Row` body: left to right, right to left under RTL, items allocated from the top; the
/// row's placement (`layout::place_in_cell`) puts them on its line.
pub(crate) fn row_layout(right_to_left: bool, wrap: bool) -> Layout {
    let layout = if right_to_left { Layout::right_to_left(Align::Min) } else { Layout::left_to_right(Align::Min) };
    layout.with_main_wrap(wrap)
}

/// The layout of the children `PaintUi` builds: left to right with items on the top line, never mirrored, because
/// children sit at physical rects the composite computed.
pub(crate) fn pinned_layout() -> Layout {
    Layout::left_to_right(Align::Min)
}

/// Runs `body` in a child `Ui` with an explicit layout and returns what it produced with the rect the child
/// took in `ui`. `rect` bounds the child; `None` gives it the rest of the parent.
pub(crate) fn block<R>(ui: &mut Ui, rect: Option<Rect>, layout: Layout, body: impl FnOnce(&mut Ui) -> R) -> (R, Rect) {
    let mut builder = UiBuilder::new().layout(layout);
    if let Some(rect) = rect {
        builder = builder.max_rect(rect);
    }
    let out = ui.scope_builder(builder, body);
    (out.inner, out.response.rect)
}

/// The physical layout of a `Stack` or `Grid` body for `right_to_left`.
pub(crate) fn column_layout(right_to_left: bool) -> Layout {
    stack_layout(right_to_left)
}

/// Runs `body` in a child `Ui`. `disabled` disables the child and everything below it. `direction` gives the
/// child a cloned style carrying that direction and an explicit layout, never the parent's.
pub(crate) fn scope<R>(ui: &mut Ui, disabled: bool, direction: Option<bool>, body: impl FnOnce(&mut Ui) -> R) -> R {
    let mut builder = UiBuilder::new();
    if let Some(right_to_left) = direction {
        let mut style = (**ui.style()).clone();
        style.right_to_left = right_to_left;
        builder = builder.layout(stack_layout(right_to_left)).style(style);
    }
    ui.scope_builder(builder, |child| {
        if disabled {
            child.disable();
        }
        body(child)
    })
    .inner
}

/// Runs `f` on a child `Ui` of `ui`.
#[cfg_attr(not(feature = "testing"), expect(dead_code, reason = "used by the pipeline (F3)"))]
pub(crate) fn scoped<R>(ui: &mut Ui, f: impl FnOnce(&mut Ui) -> R) -> R {
    ui.scope(f).inner
}
