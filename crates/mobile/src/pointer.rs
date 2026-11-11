//! Which touch drives egui's pointer, and when.
//!
//! A finger's press is held back for [`HOLD`]: a second finger landing meanwhile makes the touches a
//! gesture (a two- or three-finger tap, a pinch, a pan), and the press never reaches the app. Without
//! the hold, the first finger of every gesture pressed the pointer at once and a brush painted a dab
//! there: a new edit, which also emptied the redo history before a three-finger-tap redo could run.
//! A finger that moves past [`SLOP`] or lifts presses at once, so strokes and taps on controls aren't
//! delayed in practice. A stylus is never held: it is never a gesture.

use std::time::{Duration, Instant};

use egui::{Event, Pos2, TouchPhase};

use crate::host::Touch;

/// How long a finger's press waits for a second finger. Fingers of one gesture land within a few
/// tens of milliseconds of each other.
pub(crate) const HOLD: Duration = Duration::from_millis(80);
/// How far a held finger may move (points) before it is a stroke, not the start of a gesture.
pub(crate) const SLOP: f32 = 6.0;

#[derive(Clone, Copy, Debug)]
struct Held {
    id: u64,
    pos: Pos2,
    at: Instant,
}

/// The touch that drives egui's pointer: the first one down, until it lifts.
#[derive(Debug, Default)]
pub(crate) struct TouchPointer {
    held: Option<Held>,
    driving: Option<u64>,
    /// Touches down now.
    down: u32,
    /// The touches down are a gesture: none of them presses until they have all lifted.
    gesture: bool,
}

impl TouchPointer {
    /// The pointer events this touch stands for. `immediate` for a stylus: no hold.
    pub(crate) fn touch(&mut self, t: Touch, now: Instant, immediate: bool) -> Vec<Event> {
        let events = self.pointer_events(t, now, immediate);
        match t.phase {
            TouchPhase::Start => self.down += 1,
            TouchPhase::End | TouchPhase::Cancel => {
                self.down = self.down.saturating_sub(1);
                if self.down == 0 {
                    self.gesture = false;
                }
            }
            TouchPhase::Move => {}
        }
        events
    }

    fn pointer_events(&mut self, t: Touch, now: Instant, immediate: bool) -> Vec<Event> {
        match t.phase {
            TouchPhase::Start => {
                if self.gesture {
                    return Vec::new();
                }
                if self.held.take().is_some() {
                    self.gesture = true;
                    // A second finger: the touches are a gesture, and nothing presses.
                    return Vec::new();
                }
                if self.driving.is_some() {
                    return Vec::new();
                }
                if immediate {
                    self.driving = Some(t.id);
                    return press(t.pos);
                }
                self.held = Some(Held { id: t.id, pos: t.pos, at: now });
                Vec::new()
            }
            TouchPhase::Move => match self.held {
                Some(h) if h.id == t.id => {
                    if (t.pos - h.pos).length() <= SLOP {
                        return Vec::new();
                    }
                    self.held = None;
                    self.driving = Some(t.id);
                    let mut events = press(h.pos);
                    events.push(Event::PointerMoved(t.pos));
                    events
                }
                _ if self.driving == Some(t.id) => vec![Event::PointerMoved(t.pos)],
                _ => Vec::new(),
            },
            TouchPhase::End => match self.held {
                Some(h) if h.id == t.id => {
                    // Lifted within the hold, alone: a tap.
                    self.held = None;
                    let mut events = press(h.pos);
                    events.extend(release(t.pos));
                    events
                }
                _ if self.driving == Some(t.id) => {
                    self.driving = None;
                    release(t.pos)
                }
                _ => Vec::new(),
            },
            TouchPhase::Cancel => {
                if self.held.is_some_and(|h| h.id == t.id) {
                    self.held = None;
                }
                if self.driving == Some(t.id) {
                    self.driving = None;
                    return vec![Event::PointerGone];
                }
                Vec::new()
            }
        }
    }

    /// Press a held finger whose hold is over: it wasn't a gesture.
    pub(crate) fn tick(&mut self, now: Instant) -> Vec<Event> {
        match self.held {
            Some(h) if now.duration_since(h.at) >= HOLD => {
                self.held = None;
                self.driving = Some(h.id);
                press(h.pos)
            }
            _ => Vec::new(),
        }
    }

    /// When a held press is due, for scheduling a frame.
    pub(crate) fn due(&self) -> Option<Instant> {
        self.held.map(|h| h.at + HOLD)
    }

    /// Whether a touch has the pointer, or may be about to.
    pub(crate) fn touching(&self) -> bool {
        self.held.is_some() || self.driving.is_some()
    }
}

fn press(pos: Pos2) -> Vec<Event> {
    vec![
        Event::PointerMoved(pos),
        Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
    ]
}

/// A finger doesn't hover: after the release the pointer leaves, so nothing stays highlighted.
fn release(pos: Pos2) -> Vec<Event> {
    vec![Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE }, Event::PointerGone]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(id: u64, phase: TouchPhase, x: f32) -> Touch {
        Touch { id, phase, pos: Pos2::new(x, 0.0), force: 0.0 }
    }

    fn presses(events: &[Event]) -> Vec<bool> {
        events.iter().filter_map(|e| if let Event::PointerButton { pressed, .. } = e { Some(*pressed) } else { None }).collect()
    }

    /// The bug found on a Galaxy Tab S8+: the first finger of a three-finger tap pressed the
    /// pointer, a brush dab became a new edit, and redo had nothing left to redo.
    #[test]
    fn the_fingers_of_a_multi_finger_tap_never_press() {
        let (mut p, t0) = (TouchPointer::default(), Instant::now());
        let mut all = p.touch(t(1, TouchPhase::Start, 0.0), t0, false);
        all.extend(p.touch(t(2, TouchPhase::Start, 50.0), t0 + Duration::from_millis(20), false));
        all.extend(p.touch(t(3, TouchPhase::Start, 90.0), t0 + Duration::from_millis(30), false));
        all.extend(p.tick(t0 + Duration::from_millis(200)));
        for id in 1..=3 {
            all.extend(p.touch(t(id, TouchPhase::End, 0.0), t0 + Duration::from_millis(60), false));
        }
        assert!(presses(&all).is_empty(), "{all:?}");
        assert!(!p.touching());
    }

    #[test]
    fn a_finger_that_moves_presses_at_once_where_it_landed() {
        let (mut p, t0) = (TouchPointer::default(), Instant::now());
        assert_eq!(p.touch(t(1, TouchPhase::Start, 0.0), t0, false), Vec::<Event>::new());
        assert_eq!(p.touch(t(1, TouchPhase::Move, 3.0), t0, false), Vec::<Event>::new(), "within the slop: still held");
        let events = p.touch(t(1, TouchPhase::Move, 20.0), t0, false);
        assert_eq!(presses(&events), [true]);
        assert!(events.contains(&Event::PointerMoved(Pos2::new(0.0, 0.0))), "the stroke starts where the finger landed");
        assert_eq!(presses(&p.touch(t(1, TouchPhase::End, 20.0), t0, false)), [false]);
    }

    #[test]
    fn a_quick_single_tap_still_clicks_and_a_held_finger_presses_when_the_hold_ends() {
        let (mut p, t0) = (TouchPointer::default(), Instant::now());
        p.touch(t(1, TouchPhase::Start, 0.0), t0, false);
        assert_eq!(presses(&p.touch(t(1, TouchPhase::End, 0.0), t0 + Duration::from_millis(30), false)), [true, false]);

        p.touch(t(2, TouchPhase::Start, 0.0), t0, false);
        assert_eq!(p.due(), Some(t0 + HOLD));
        assert_eq!(p.tick(t0 + Duration::from_millis(40)), Vec::<Event>::new());
        assert_eq!(presses(&p.tick(t0 + HOLD)), [true]);
        assert_eq!(p.due(), None);
    }

    #[test]
    fn a_stylus_presses_at_once_and_a_cancelled_touch_never_releases() {
        let (mut p, t0) = (TouchPointer::default(), Instant::now());
        assert_eq!(presses(&p.touch(t(1, TouchPhase::Start, 0.0), t0, true)), [true]);
        let cancelled = p.touch(t(1, TouchPhase::Cancel, 0.0), t0, true);
        assert_eq!(cancelled, [Event::PointerGone]);
    }
}
