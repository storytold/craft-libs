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

// Undo or Redo inside the nudge pause ends the run first: the nudges are a step of their own, which drops what was
// undone before them.
#[trial]
fn undo_or_redo_inside_the_nudge_pause_keeps_the_nudges_as_their_own_step() {
    let at = |s: &TransformSession<Warp>| s.quad()[0];
    let mut s = session().nudge_pause(Some(1.0));
    drag(&mut s, moved(doc(30.0, 0.0)), 0.0);
    drag(&mut s, moved(doc(60.0, 0.0)), 0.1);
    assert!(s.undo());
    nudge(&mut s, moved(doc(31.0, 0.0)), 0.2);
    assert!(!s.redo(), "the nudge dropped the undone drag");
    assert!(near(at(&s), Point::new(31.0, 0.0)), "{:?}", at(&s));
    assert!(s.undo(), "the nudge is a step");
    assert!(near(at(&s), Point::new(30.0, 0.0)), "{:?}", at(&s));
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

#[derive(Clone, Debug, PartialEq)]
struct Framed(Rect);

impl TransformWarp for Framed {
    fn is_identity(&self) -> bool {
        false
    }

    fn frame(&self) -> Option<Rect> {
        Some(self.0)
    }
}

/// A warp with its own frame takes the session's frame and box in the step that enters it; leaving the warp, Undo and
/// Redo give the frame and the box back in step with the warp.
#[trial]
fn a_warp_with_its_own_frame_takes_the_frame_and_gives_it_back() {
    let inner = Rect::new(20.0, 10.0, 120.0, 60.0);
    let mut s = TransformSession::<Framed>::new(FRAME);
    s.apply(TransformEdit::Warp(Framed(inner)), 0.0);
    s.settle(0.0, false);
    assert_eq!((s.rect(), s.state().quad), (inner, BoxState::from_rect(inner).quad));
    assert!(!s.is_untouched());
    assert!(s.undo());
    assert_eq!((s.rect(), s.state().quad), (FRAME, BoxState::from_rect(FRAME).quad), "undo restores frame and box with the warp");
    assert!(s.redo());
    assert_eq!(s.rect(), inner);
    s.apply(TransformEdit::LeaveWarp, 1.0);
    assert_eq!((s.rect(), s.state().quad), (FRAME, BoxState::from_rect(FRAME).quad));
}

// The revision moves with every change to what Undo, Redo and the Warp phase report, and stays through a drag's moves
// and the view-like edits, so an app keys its Edit menu on it.
#[trial]
fn the_revision_follows_the_undo_steps_and_the_warp_phase() {
    let mut s = session();
    let start = s.revision();
    let id = GestureId::new(1);
    s.apply(TransformEdit::Box(begin(id)), 0.0);
    s.apply(TransformEdit::Box(update(id, moved(doc(5.0, 0.0)))), 0.0);
    s.apply(TransformEdit::Box(update(id, moved(doc(9.0, 0.0)))), 0.0);
    assert_eq!(s.revision(), start, "a drag's moves record no step");
    s.apply(TransformEdit::Box(commit(id, moved(doc(9.0, 0.0)))), 0.0);
    s.settle(0.0, false);
    let recorded = s.revision();
    assert!(recorded != start && s.can_undo());
    assert!(s.undo());
    let undone = s.revision();
    assert!(undone != recorded && s.can_redo());
    s.apply(TransformEdit::Warp(Warp(false)), 0.0);
    let warped = s.revision();
    assert!(warped != undone && s.warp().is_some());
    s.apply(TransformEdit::Warp(Warp(true)), 0.0);
    assert_eq!(s.revision(), warped, "a warp edit inside the phase opens no phase");
    s.apply(TransformEdit::LeaveWarp, 0.0);
    assert!(s.revision() != warped && s.warp().is_none());
}
