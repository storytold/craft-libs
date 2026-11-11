//! `ThumbRow` rules: the order thumbs keep, split halves, wrapping.

use craft_behavior::{Mixed, MoveMods, NavMove};
use craft_ui::contract::Bind;
use craft_ui::testing::trial;
use craft_ui::value_drag::{Anchor, RowOp, Thumb, ThumbOp, ThumbRow};

fn forward(thumb: usize) -> RowOp {
    RowOp::new(thumb, ThumbOp::Move(NavMove::Forward, MoveMods::PLAIN))
}

/// In an anchored row the last thumb stays within the span past the first and the thumbs keep their order round
/// the period.
#[trial]
fn an_anchored_row_keeps_its_span_and_order() {
    let thumbs = [Thumb::new("a"), Thumb::new("b"), Thumb::new("c"), Thumb::new("d")];
    let row = ThumbRow::new(Bind::given(Mixed::each([340.0, 10.0, 50.0, 80.0])), 0.0..=360.0).step(1.0).anchor(Anchor::new(0, 100.0)).thumbs(thumbs);
    let rules = row.rules();
    let at = Mixed::each([340.0, 10.0, 50.0, 80.0]);
    // The last thumb is 100 past the first at most: 340 + 100 = 80 round the period.
    assert_eq!(rules.apply(&forward(3), &at), at);
    // A middle thumb stops at the next one.
    assert_eq!(rules.apply(&RowOp::new(1, ThumbOp::Set(70.0)), &at), Mixed::each([340.0, 50.0, 50.0, 80.0]));
}
