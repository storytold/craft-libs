//! A row given no width lays its units out at zero, never in a NaN room. The row is short, so it probes its bodies for
//! their leasts in a room unbounded along x; a body there that scrolls vertically keeps that axis unbounded for what
//! it holds, so a centred row inside it and the stack in that row never take a width from an infinite rect.

use craft_ui::buttons::Button;
use craft_ui::contract::{RowAlign, Scroll, Show as _, ShowContainer as _};
use craft_ui::layout_components::{Row, Stack};
use craft_ui::testing::{Fixture, trial};
use egui::{Rect, Vec2};
use std::cell::Cell;
use std::rc::Rc;

#[trial]
fn a_row_given_no_width_lays_out_its_stack_child_with_finite_rects() {
    // The stack's rect from the last real run.
    let seen = Rc::new(Cell::new(Rect::NOTHING));
    let out = Rc::clone(&seen);
    let mut fixture = Fixture::new(move |ui| {
        let out = Rc::clone(&out);
        ui.allocate_ui(Vec2::new(0.0, 300.0), move |ui| {
            Row::new().fill().show(ui, |ui, _| {
                Stack::new().scroll(Scroll::vertical()).show(ui, |ui, _| {
                    Row::new().fill_width().align(RowAlign::Center).show(ui, |ui, _| {
                        let stack = Stack::new().show(ui, |ui, _| {
                            Button::new("Tool").show(ui);
                        });
                        if !ui.is_sizing_pass() {
                            out.set(stack.output.rect);
                        }
                    });
                });
                Button::new("Canvas").show(ui);
            });
        });
    });
    fixture.room(Vec2::new(400.0, 300.0)).settle();
    let rect = seen.get();
    assert!(rect != Rect::NOTHING, "the stack showed");
    assert!(rect.min.is_finite() && rect.max.is_finite(), "the stack's rect is finite: {rect:?}");
}
