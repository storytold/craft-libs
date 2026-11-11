//! The playback clock and the follow.

use super::clock::{ClockEvent, PREROLL_TIMEOUT_S, PlaybackClock, PlaybackState};
use super::time::{Tick, TimeRange};
use craft_ui::testing::trial;

fn secs(s: f64) -> Tick {
    Tick::from_seconds_f64(s)
}

fn playing(from: f64, speed: f64) -> PlaybackClock {
    let mut clock = PlaybackClock::new();
    clock.play(0.0, secs(from), speed, false);
    clock
}

// position advances by (now - start) * speed, backwards at negative speed.

// Prerolling until frames_ready or 0.5 s, then Started.
#[trial]
fn preroll_waits_for_frames_or_the_timeout() {
    let mut clock = PlaybackClock::new();
    assert_eq!(clock.toggle(1.0, secs(5.0)), None);
    assert_eq!(clock.state(), PlaybackState::Prerolling { since: 1.0 });
    assert!(clock.step(1.2, false).is_empty());
    assert_eq!(clock.position(1.4), secs(5.0), "the playhead waits");
    assert_eq!(clock.step(1.3, true), [ClockEvent::Started]);
    assert_eq!(clock.position(2.3), secs(6.0), "the clock runs from the pass that started it");

    let mut clock = PlaybackClock::new();
    clock.play(0.0, Tick::ZERO, 1.0, true);
    assert!(clock.step(PREROLL_TIMEOUT_S - 0.01, false).is_empty());
    assert_eq!(clock.step(PREROLL_TIMEOUT_S, false), [ClockEvent::Started]);
    assert_eq!(clock.toggle(1.0, Tick::ZERO), Some(secs(0.5)));
}

// Looped at the loop end, Stopped at stop_at.
#[trial]
fn loops_and_stops_at_the_stop_point() {
    let mut clock = playing(1.0, 1.0);
    clock.set_loop(Some(TimeRange::from_bounds(secs(1.0), secs(3.0))));
    assert_eq!(clock.position(2.5), secs(1.5), "folded into the loop before the pass sees it");
    assert_eq!(clock.step(2.5, true), [ClockEvent::Looped]);
    assert!(clock.step(3.0, true).is_empty());
    assert_eq!(clock.position(3.0), secs(2.0));

    let mut clock = playing(0.0, 1.0);
    clock.set_stop_at(Some(secs(2.0)));
    assert!(clock.step(1.0, true).is_empty());
    assert_eq!(clock.position(5.0), secs(2.0));
    assert_eq!(clock.step(5.0, true), [ClockEvent::Stopped(secs(2.0))]);
    assert_eq!(clock.state(), PlaybackState::Stopped);
    assert_eq!(clock.position(9.0), secs(2.0));
}

// Center centres once there, Continuous from the first frame.
