//! A container's measure: its body's children measured under its layout, plus its pad, from this pass's content.

use craft_behavior::Proposal;
use craft_ui::contract::{Gap, Pad, Props as _, Show, ShowContainer};
use craft_ui::layout_components::Stack;
use craft_ui::testing::layout::Block;
use craft_ui::testing::{Frames, trial};
use egui::{Rect, Ui, Vec2};
use std::cell::Cell;

const BLOCK: Vec2 = Vec2::new(40.0, 20.0);

fn stack() -> Stack<'static> {
    Stack::new().gap(Gap::Section).pad(Pad::Regular).id_salt("measured")
}

fn blocks(ui: &mut Ui, count: usize) {
    for salt in 0..count {
        Block::new(BLOCK).id_salt(salt).show(ui);
    }
}

/// A stack of three fixed-size children measures what `show` takes for it: their sum and gaps plus the pad on the
/// main axis, the widest plus the pad across it.
#[trial]
fn a_stack_measures_what_it_shows() {
    let mut frames = Frames::new();
    let (measured, shown, span) = (Cell::new(Vec2::ZERO), Cell::new(Rect::NOTHING), Cell::new(Rect::NOTHING));
    frames.frame(vec![], |ui| {
        measured.set(stack().measure_in(ui, Proposal::Ideal, |ui, _| blocks(ui, 3)));
        let mut placed = Rect::NOTHING;
        let output = stack().show(ui, |ui, _| {
            for salt in 0..3 {
                placed = placed.union(Block::new(BLOCK).id_salt(salt).show(ui).rect);
            }
        });
        shown.set(output.output.rect);
        span.set(placed);
    });
    let (measured, shown, span) = (measured.get(), shown.get(), span.get());
    assert!(shown.contains_rect(span) && shown.height() > span.height(), "the stack pads its children: {shown:?} {span:?}");
    assert_eq!(measured, shown.size(), "the measure is the size the stack shows at");
}
