//! `ThumbRow` rules: the order thumbs keep, split halves, wrapping.

use craft_behavior::{Mixed, MoveMods, NavMove};
use craft_ui::contract::Bind;
use craft_ui::testing::trial;
use craft_ui::value_drag::{Anchor, Neighbours, Order, RowOp, Thumb, ThumbOp, ThumbRow};

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

/// Two split pairs ordered black below white, as a Blend If bar's handles: in a row that pushes, setting the black
/// high half past the white pair pushes the white pair along and the order holds; in a row that stops, the half stops
/// at the white pair.
#[trial]
fn a_pushing_row_moves_the_thumbs_ahead_and_a_stopping_row_stops_at_them() {
    let ordered = |below, above| Order::between(below, above, 0.0);
    let thumbs = || {
        [
            Thumb::new("black low").split().order(ordered(None, Some(2))),
            Thumb::new("black high").order(ordered(Some(0), Some(2))),
            Thumb::new("white low").split().order(ordered(Some(1), None)),
            Thumb::new("white high").order(ordered(Some(1), None)),
        ]
    };
    let at = Mixed::each([10.0, 20.0, 100.0, 120.0]);
    let set = RowOp::new(1, ThumbOp::Set(150.0));
    let pushing = ThumbRow::new(Bind::given(at), 0.0..=255.0).step(1.0).thumbs(thumbs()).neighbours(Neighbours::Push);
    assert_eq!(pushing.rules().apply(&set, &at), Mixed::each([10.0, 150.0, 150.0, 150.0]));
    let stopping = ThumbRow::new(Bind::given(at), 0.0..=255.0).step(1.0).thumbs(thumbs());
    assert_eq!(stopping.rules().apply(&set, &at), Mixed::each([10.0, 100.0, 100.0, 120.0]));
}
