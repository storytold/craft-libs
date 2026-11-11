//! `TransformSession`: results, gestures, undo, presets, the warp phase, the mode menu and the settings.

use craft_behavior::geometry::quad::BoxState;
use craft_behavior::kurbo::{Point, Rect, Vec2};
use craft_behavior::{EditEvent, GestureId, Mixed};
use craft_draw::ui::transform::{TransformEdit, TransformSession, TransformWarp};
use craft_ui::testing::trial;
use std::sync::Arc;

fn begin<T>(id: GestureId) -> EditEvent<T> {
    EditEvent::begin(id, Mixed::Mixed, Arc::from(""), None)
}

fn update<T>(id: GestureId, value: T) -> EditEvent<T> {
    EditEvent::update(id, value)
}

fn commit<T>(id: GestureId, value: T) -> EditEvent<T> {
    EditEvent::commit(id, Mixed::Mixed, value)
}

fn set<T>(value: T) -> EditEvent<T> {
    EditEvent::set(Mixed::Mixed, value, Arc::from(""), None)
}

#[derive(Clone, Debug, PartialEq)]
struct Warp(bool);

impl TransformWarp for Warp {
    fn is_identity(&self) -> bool {
        self.0
    }
}

const FRAME: Rect = Rect::new(0.0, 0.0, 200.0, 100.0);

fn session() -> TransformSession<Warp> {
    TransformSession::new(FRAME)
}

/// A document displacement.
fn doc(x: f64, y: f64) -> Vec2 {
    Vec2::new(x, y)
}

fn moved(by: Vec2) -> BoxState {
    BoxState::from_rect(FRAME).moved(by)
}

/// A whole drag to `state`, settled.
fn drag(s: &mut TransformSession<Warp>, state: BoxState, now: f64) {
    let id = GestureId::new(1);
    s.apply(TransformEdit::Box(begin(id)), now);
    s.apply(TransformEdit::Box(update(id, state)), now);
    s.apply(TransformEdit::Box(commit(id, state)), now);
    s.settle(now, false);
}

fn nudge(s: &mut TransformSession<Warp>, state: BoxState, now: f64) {
    s.apply(TransformEdit::Box(set(state)), now);
    s.settle(now, false);
}

fn near(a: Point, b: Point) -> bool {
    (a - b).hypot() < 1e-6
}

// A new session has no result and commits as CloseUntouched; a moved box gives Apply with the
// frame, the box and the interpolation; a mode or interpolation change alone stays untouched.

// Discard during a running gesture puts back the box the gesture began on.

// WarpRequested answers NeedWarp with the current frame and box; Warp(w) enters the Warp phase;
// one undo goes back to the Box phase with the box as it was.

// LeaveWarp drops the warp and returns to the Box phase with the box unchanged.

// The menu lists Free Transform, Scale, Rotate, Skew, Distort, Perspective, Warp, separator, the
// three rotations, separator, the two flips; the current mode is checked; Distort and Perspective
// are disabled when the content refuses distort; every pick applies as its edit.

// ReferencePoint(false) keeps the pivot where it is: a later preset and the box rules still turn
// and scale about it.

// `box_rules` with `legacy` gives ScaleRule::Free, without it Proportional; the aspect link and
// the session's mode and allow carry over; the rotate step is 15.

// Two nudges within the pause share one step; a drag between them breaks the run; with no pause
// every nudge is its own step.
#[trial]
fn nudges_within_the_pause_fold_into_one_step() {
    let step = |dx: f64| moved(doc(dx, 0.0));
    let steps = |s: &mut TransformSession<Warp>| {
        let mut n = 0;
        while s.undo() {
            n += 1;
        }
        n
    };

    let mut folded = session().nudge_pause(Some(1.0));
    nudge(&mut folded, step(1.0), 0.0);
    nudge(&mut folded, step(2.0), 0.5);
    folded.settle(5.0, false);
    assert_eq!(steps(&mut folded), 1);

    let mut broken = session().nudge_pause(Some(1.0));
    nudge(&mut broken, step(1.0), 0.0);
    drag(&mut broken, step(30.0), 0.2);
    nudge(&mut broken, step(31.0), 0.4);
    broken.settle(5.0, false);
    assert_eq!(steps(&mut broken), 3);

    let mut plain = session();
    nudge(&mut plain, step(1.0), 0.0);
    nudge(&mut plain, step(2.0), 0.1);
    assert_eq!(steps(&mut plain), 2);
}

// Between a Box Begin and its Commit, undo and redo return false and change nothing.

// `homography` maps the frame's corners onto the box's corners and its centre into the box; a box
// with three corners on a line gives None.
#[trial]
fn the_homography_maps_the_frame_onto_the_box() {
    let mut s = TransformSession::<()>::new(Rect::new(10.0, 20.0, 210.0, 120.0));
    let target =
        BoxState::new([Point::new(0.0, 0.0), Point::new(300.0, 10.0), Point::new(280.0, 200.0), Point::new(-20.0, 150.0)], Point::new(100.0, 80.0));
    s.apply(TransformEdit::Box(set(target)), 0.0);
    let h = s.homography().expect("a box");
    let corners = [Point::new(10.0, 20.0), Point::new(210.0, 20.0), Point::new(210.0, 120.0), Point::new(10.0, 120.0)];
    for (corner, expected) in corners.into_iter().zip(target.quad) {
        assert!(near(h.apply(corner), expected), "{corner:?}");
    }
    assert!(target.contains(h.apply(Point::new(110.0, 70.0))));

    let line = BoxState::new([Point::new(0.0, 0.0), Point::new(10.0, 0.0), Point::new(20.0, 0.0), Point::new(30.0, 0.0)], Point::ZERO);
    s.apply(TransformEdit::Box(set(line)), 0.0);
    assert!(s.homography().is_none());
}
