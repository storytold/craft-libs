//! Deep nesting: a component's run keeps only what it reads after its children on the stack.

use craft_ui::contract::{Props as _, Show as _, ShowContainer as _};
use craft_ui::layout::BodyLayout;
use craft_ui::testing::layout::Block;
use craft_ui::testing::{Frames, trial};
use craft_ui_test_kit::contract::Panel;
use egui::{Ui, Vec2};
use std::cell::Cell;

/// Containers sixteen deep, rows inside stacks, show on a thread with a 512 KiB stack: their measuring runs and their
/// real runs both stay within it.
#[trial]
fn sixteen_nested_containers_show_on_a_512_kib_stack() {
    let reached = std::thread::Builder::new()
        .stack_size(512 * 1024)
        .spawn(|| {
            let reached = Cell::new(0);
            let mut frames = Frames::new();
            frames.idle(2, |ui| nest(ui, 16, &reached));
            reached.get()
        })
        .expect("the thread starts")
        .join()
        .expect("the frames run");
    assert!(reached > 0, "the innermost body ran");
}

fn nest(ui: &mut Ui, depth: usize, reached: &Cell<usize>) {
    let layout = if depth.is_multiple_of(2) { BodyLayout::Stack } else { BodyLayout::row() };
    Panel::new().layout(layout).id_salt(depth).show(ui, |ui, _| {
        if depth == 1 {
            Block::new(Vec2::splat(10.0)).show(ui);
            reached.set(reached.get() + 1);
        } else {
            nest(ui, depth - 1, reached);
        }
    });
}
