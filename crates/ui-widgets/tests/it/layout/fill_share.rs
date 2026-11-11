//! Along a container's main axis, a child that fills takes the room its siblings leave, those drawn before it and
//! those drawn after it.

use craft_behavior::ColumnWidth;
use craft_ui::buttons::Button;
use craft_ui::contract::{Show as _, ShowContainer as _};
use craft_ui::layout_components::{Grid, Stack};
use craft_ui::testing::{Fixture, trial};
use egui::{Rect, Vec2};
use std::cell::Cell;
use std::rc::Rc;

const TOLERANCE: f32 = 0.5;
static PANES: [ColumnWidth; 2] = [ColumnWidth::fill(0.5), ColumnWidth::fill(0.5)];

#[trial]
fn a_fill_child_leaves_room_for_later_siblings() {
    const ROOM: Vec2 = Vec2::new(400.0, 400.0);
    let seen = Rc::new(Cell::new((Rect::NOTHING, Rect::NOTHING, Rect::NOTHING)));
    let out = Rc::clone(&seen);
    let mut fixture = Fixture::new(move |ui| {
        let stack = Stack::new().fill().show(ui, |ui, _| {
            let grid = Grid::new(&PANES).fill().show(ui, |ui, _| {
                Stack::new().fill().show(ui, |ui, _| {
                    let _ = Button::new("Table").show(ui);
                });
                Stack::new().fill().show(ui, |ui, _| {
                    let _ = Button::new("Detail").show(ui);
                });
            });
            let pane = Stack::new().show(ui, |ui, _| {
                let _ = Button::new("Keyboard one").show(ui);
                let _ = Button::new("Keyboard two").show(ui);
            });
            (grid.output.rect, pane.output.rect)
        });
        if let Some((grid, pane)) = stack.inner {
            out.set((stack.output.rect, grid, pane));
        }
    });
    fixture.room(ROOM).settle();
    let (stack, grid, pane) = seen.get();
    let dump = format!("stack {stack:?}, grid {grid:?}, pane {pane:?}");
    assert!(pane.height() > 0.0 && stack.contains_rect(pane.shrink(TOLERANCE)), "the pane is inside the stack: {dump}");
    let gap = pane.min.y - grid.max.y;
    assert!(gap >= -TOLERANCE, "the pane starts after the grid: {dump}");
    assert!((grid.height() + gap + pane.height() - stack.height()).abs() <= TOLERANCE, "the grid takes what the pane leaves: {dump}");
}
