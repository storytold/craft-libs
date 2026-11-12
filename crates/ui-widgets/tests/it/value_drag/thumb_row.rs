//! `ThumbRow` rules: the order thumbs keep, split halves, wrapping; and what a press on the row moves.

use craft_behavior::{Mixed, MotionMode, MoveMods, NavMove};
use craft_ui::contract::{Bind, Props as _, Show as _};
use craft_ui::testing::{Fixture, pointer, trial};
use craft_ui::value_drag::{Anchor, Neighbours, OffThumbs, Order, RowOp, Thumb, ThumbOp, ThumbRow};
use egui::{Modifiers, Pos2, Rangef, Rect, Vec2};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

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
/// high half past the white pair moves the black pair there and pushes the white pair along, and the order holds; in
/// a row that stops, the black pair stops at the white pair.
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
    assert_eq!(pushing.rules().apply(&set, &at), Mixed::each([140.0, 150.0, 150.0, 150.0]));
    let stopping = ThumbRow::new(Bind::given(at), 0.0..=255.0).step(1.0).thumbs(thumbs());
    assert_eq!(stopping.rules().apply(&set, &at), Mixed::each([90.0, 100.0, 100.0, 120.0]));
}

/// A key, the wheel or a set value on either half of a split thumb moves the pair and keeps the spread, up to where
/// the pair meets the end; with Alt (the fine flag) the focused half moves alone, by the plain step.
#[trial]
fn a_split_thumb_moves_as_a_pair_and_with_alt_as_one_half() {
    let at = Mixed::each([10.0, 20.0]);
    let row = ThumbRow::new(Bind::given(at), 0.0..=255.0).step(1.0).thumbs([Thumb::new("low").split(), Thumb::new("high")]);
    let rules = row.rules();
    assert_eq!(rules.apply(&forward(1), &at), Mixed::each([11.0, 21.0]));
    let alt = RowOp::new(1, ThumbOp::Move(NavMove::Forward, MoveMods::PLAIN.fine(true)));
    assert_eq!(rules.apply(&alt, &at), Mixed::each([10.0, 21.0]));
    assert_eq!(rules.apply(&RowOp::new(0, ThumbOp::Set(100.0)), &at), Mixed::each([100.0, 110.0]));
    assert_eq!(rules.apply(&RowOp::new(0, ThumbOp::Set(250.0)), &at), Mixed::each([245.0, 255.0]));
}

const FIELD: Rect = Rect { min: Pos2::new(20.0, 20.0), max: Pos2::new(276.0, 50.0) };

/// A split thumb shown in `FIELD` over 0..=255, holding its values and where its thumbs travel.
struct Split {
    values: RefCell<[Mixed<f64>; 2]>,
    travel: Cell<Rangef>,
    off: OffThumbs,
}

impl Split {
    /// The split thumb at `values` in a row whose presses off the thumbs do `off`, shown and settled.
    fn rig(values: [f64; 2], off: OffThumbs) -> (Rc<Split>, Fixture) {
        let split = Rc::new(Split { values: RefCell::new(Mixed::each(values)), travel: Cell::new(Rangef::NOTHING), off });
        let shown = Rc::clone(&split);
        let mut fixture = Fixture::new(move |ui| shown.show(ui));
        fixture.motion(MotionMode::Off).settle();
        (split, fixture)
    }

    fn show(&self, ui: &mut egui::Ui) {
        let thumbs = [Thumb::new("low").split(), Thumb::new("high")];
        let mut values = self.values.borrow_mut();
        let row = ThumbRow::new(Bind::bound(&mut *values), 0.0..=255.0).step(1.0).thumbs(thumbs).off_thumbs(self.off).label("Blend");
        self.travel.set(row.show_at(ui, FIELD).travel);
    }

    /// The point on the row at `value`.
    fn at(&self, value: f64) -> Pos2 {
        let travel = self.travel.get();
        Pos2::new(travel.min + travel.span() * (value / 255.0) as f32, FIELD.center().y)
    }

    fn values(&self) -> [f64; 2] {
        let values = *self.values.borrow();
        values.map(|value| value.same().copied().unwrap_or(f64::NAN))
    }
}

/// Presses at `from`, moves `by` along the row in four frames, with Alt held from the third on when `alt`, and
/// releases.
fn drag(fixture: &mut Fixture, from: Pos2, by: f32, alt: bool) {
    fixture.frame(vec![pointer::moved(from)]);
    fixture.frame(vec![pointer::press(from)]);
    for step in 1..=4 {
        if alt && step == 3 {
            fixture.hold(Modifiers::ALT);
        }
        fixture.frame(vec![pointer::moved(from + Vec2::new(by * step as f32 / 4.0, 0.0))]);
    }
    fixture.frame(vec![pointer::release(from + Vec2::new(by, 0.0))]);
    fixture.hold(Modifiers::NONE).settle();
}

/// The thumb that has the focus.
fn focused(fixture: &Fixture) -> Option<u64> {
    fixture.focused_stop().and_then(|(_, part)| part).and_then(|part| part.index)
}

/// A drag on a half of a split thumb moves the pair; once Alt is held, the half alone, from where the drag began.
#[trial]
fn a_drag_moves_a_split_pair_and_with_alt_held_its_half() {
    let (split, mut fixture) = Split::rig([100.0, 150.0], OffThumbs::Jump);
    drag(&mut fixture, split.at(150.0), 20.0, false);
    let [low, high] = split.values();
    assert!(high > 150.0 && high - low == 50.0, "the pair did not move together: {low}, {high}");
    drag(&mut fixture, split.at(high), 20.0, true);
    let [alone_low, alone_high] = split.values();
    assert!(alone_low == low && alone_high > high, "the high half did not move alone: {alone_low}, {alone_high}");
}

/// A press on the triangle of coinciding halves takes the half on its side of the tip, even as it drags toward the
/// other side.
#[trial]
fn a_press_on_coinciding_halves_takes_the_half_on_its_side_of_the_tip() {
    for (side, half) in [(-3.0_f32, 0), (3.0, 1)] {
        let (split, mut fixture) = Split::rig([100.0, 100.0], OffThumbs::Jump);
        drag(&mut fixture, split.at(100.0) + Vec2::new(side, 0.0), -20.0 * side.signum(), false);
        assert_eq!(focused(&fixture), Some(half), "a press {side} points from the tip took another half");
    }
}

/// A tap off the thumbs focuses the nearest thumb in a row whose presses off them take the nearest, and does nothing
/// in a row that ignores them; neither moves a thumb.
#[trial]
fn a_tap_off_the_thumbs_focuses_the_nearest_or_does_nothing_as_the_row_says() {
    for (off, expected) in [(OffThumbs::Nearest, Some(1)), (OffThumbs::Ignore, None)] {
        let (split, mut fixture) = Split::rig([100.0, 150.0], off);
        fixture.click_at(split.at(220.0)).settle();
        assert_eq!(split.values(), [100.0, 150.0], "a tap in a {off:?} row moved a thumb");
        assert_eq!(focused(&fixture), expected, "a tap in a {off:?} row focused another thumb");
    }
}
