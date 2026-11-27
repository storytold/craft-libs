//! Cross fill in a row: a row that takes its height from its content holds a child that fills its height at the
//! child's measured height, and shows it across the line the row settles on, so the child never sets the line.

use craft_ui::buttons::Button;
use craft_ui::contract::{Show as _, ShowContainer as _};
use craft_ui::display::Divider;
use craft_ui::layout_components::{Row, Stack};
use craft_ui::testing::{Frames, last_pass_measuring_runs, trial};
use egui::Rect;
use std::cell::Cell;

const SNAP: f32 = 1.0;

/// The rects of a content-height row, its stack of buttons, its vertical divider and its lone button.
#[derive(Clone, Copy)]
struct Rects {
    row: Rect,
    stack: Rect,
    divider: Rect,
    button: Rect,
}

/// Shows the row for `n` frames in a column given the screen.
fn shown(frames: &mut Frames, n: usize, seen: &Cell<Option<Rects>>) {
    frames.idle(n, |ui| {
        Stack::new().fill().show_at(ui, ui.available_rect_before_wrap(), |ui, _| {
            let parts = Cell::new((Rect::NOTHING, Rect::NOTHING, Rect::NOTHING));
            let row = Row::new().show(ui, |ui, _| {
                let stack = Stack::new().show(ui, |ui, _| {
                    for label in ["Bevel", "Stroke", "Inner Shadow", "Drop Shadow"] {
                        Button::new(label).show(ui);
                    }
                });
                let divider = Divider::vertical().show(ui).rect;
                let button = Button::new("Preview").show(ui).rect;
                parts.set((stack.output.rect, divider, button));
            });
            let (stack, divider, button) = parts.get();
            seen.set(Some(Rects { row: row.output.rect, stack, divider, button }));
        });
    });
}

#[trial]
fn a_vertical_divider_spans_the_line_of_a_content_height_row_without_setting_it() {
    let mut frames = Frames::new();
    let seen = Cell::new(None);
    let check = |seen: &Cell<Option<Rects>>, when: &str| {
        let Rects { row, stack, divider, button } = seen.get().expect("the row shows");
        assert!(
            (divider.top() - row.top()).abs() <= SNAP && (divider.bottom() - row.bottom()).abs() <= SNAP,
            "{when}: the divider {divider:?} spans the row {row:?}"
        );
        assert!((row.height() - stack.height()).abs() <= SNAP, "{when}: the row {row:?} is as tall as its tallest child {stack:?}");
        assert!(button.height() < row.height() - SNAP, "{when}: a button keeps its own height {button:?} in the row {row:?}");
        assert!(button.top() >= row.top() - SNAP && button.bottom() <= row.bottom() + SNAP, "{when}: the button {button:?} sits in the line");
    };
    shown(&mut frames, 4, &seen);
    check(&seen, "after four frames");
    shown(&mut frames, 4, &seen);
    check(&seen, "once settled");
    assert!(last_pass_measuring_runs(&frames.ctx).is_empty(), "a settled frame measures nothing");
}
