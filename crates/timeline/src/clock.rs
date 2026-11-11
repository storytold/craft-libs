//! The playback clock and the view that follows the playhead.
//!
//! [`PlaybackClock`] turns play, stop, speed, loops and stop points into the playhead time at
//! `now`. The app passes `now` (monotonic seconds) on every call: egui's `input().time` is the
//! previous full pass's value during a logic-only pass (EffectCraft `clock.rs:3`), and tests
//! drive a manual clock. An audio device that knows where it is drives the clock through
//! [`PlaybackClock::sync_external`].
//!
//! [`Follow`] scrolls a [`TimeView`] with the playhead while playing, unless something else moved
//! the view during playback.

use crate::time::{Tick, TimeRange, TimeView};

/// How long playback waits for the first frames before its clock starts anyway, in seconds
/// (FilmCraft `frames.rs:442`). The frames it waits for are [`crate::frames::PREROLL_FRAMES`].
pub const PREROLL_TIMEOUT_S: f64 = 0.5;

/// Seconds an external clock may stand still while playing before the wall clock takes over (a
/// device that stopped consuming samples; FilmCraft `audio.rs:7` `AUDIO_STALL_S`).
pub const EXTERNAL_STALL_S: f64 = 1.0;

/// [`FollowMode::Page`] pages when the playhead passes this share of the width and puts it at
/// [`PAGE_MARGIN`] from the left (FilmCraft `timeline.rs:513`).
const PAGE_EDGE: f64 = 0.95;
const PAGE_MARGIN: f64 = 0.05;
/// The middle of the view, where the centred modes hold the playhead.
const CENTRE: f64 = 0.5;

/// Where playback is.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum PlaybackState {
    #[default]
    /// Not playing.
    Stopped,
    /// Play was pressed; the clock waits for the first frames since `since` (seconds).
    Prerolling {
        /// The wall-clock time, in seconds, when Play was pressed.
        since: f64,
    },
    /// Frames are playing and the playhead advances.
    Playing,
}

/// What a clock step crossed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClockEvent {
    /// Preroll ended (frames ready or the timeout passed): the clock runs from this pass.
    Started,
    /// The playhead passed the loop's end and went back to its start.
    Looped,
    /// Forward playback reached the stop point and stopped there.
    Stopped(Tick),
}

/// The transport's clock: play at a speed (negative plays backwards; J/K/L shuttle speeds),
/// stop, preroll, loop over a range, stop at a point (Play In to Out), and an external clock.
///
/// ```ignore
/// // On Space:
/// clock.toggle(now, doc.playhead());
/// // Each pass:
/// let ready = frames.preroll_ready(&key, clock.speed(), last_frame);
/// for event in clock.step(now, ready) { /* Looped: request the loop start's frames */ }
/// doc.set_playhead(clock.position(now));
/// ```
#[derive(Clone, Debug, Default)]
pub struct PlaybackClock {
    state: PlaybackState,
    speed: f64,
    loop_range: Option<TimeRange>,
    stop_at: Option<Tick>,
    /// Monotonic time (s) and timeline time when the clock (re)started.
    anchor_time: f64,
    anchor: Tick,
    /// The external clock's last reading, and when (monotonic s) it last moved.
    external: Option<(Tick, f64)>,
}

/// The playhead after a loop's end or start has been passed: back inside `range`.
fn folded(raw: Tick, range: TimeRange) -> Tick {
    range.start + Tick((raw - range.start).0.rem_euclid(range.duration.0))
}

impl PlaybackClock {
    /// A stopped clock at normal speed.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the clock is stopped, prerolling or playing.
    pub fn state(&self) -> PlaybackState {
        self.state
    }

    /// Playing or prerolling.
    pub fn is_playing(&self) -> bool {
        self.state != PlaybackState::Stopped
    }

    /// The playback speed as a multiple of normal speed.
    pub fn speed(&self) -> f64 {
        self.speed
    }

    /// The unfolded playhead at `now` while playing: the external clock's reading while it moves,
    /// continued on the wall clock once it has stood still for [`EXTERNAL_STALL_S`].
    fn raw(&self, now: f64) -> Tick {
        let (from, since) = match self.external {
            Some((reading, moved)) if now < moved + EXTERNAL_STALL_S => return reading,
            Some((reading, moved)) => (reading, moved + EXTERNAL_STALL_S),
            None => (self.anchor, self.anchor_time),
        };
        from + Tick::from_seconds_f64((now - since) * self.speed)
    }

    /// Stop at `at` without asking where the playhead is.
    fn halt(&mut self, now: f64, at: Tick) {
        self.state = PlaybackState::Stopped;
        self.anchor = at;
        self.anchor_time = now;
        self.external = None;
        self.stop_at = None;
    }

    /// Loop over `range` while playing (Premiere's Loop button with In and Out set; the whole
    /// sequence without them). `None` stops looping.
    pub fn set_loop(&mut self, range: Option<TimeRange>) {
        self.loop_range = range;
    }

    /// Forward playback stops at `at` (Play In to Out, Play from Playhead to Out Point).
    pub fn set_stop_at(&mut self, at: Option<Tick>) {
        self.stop_at = at;
    }

    /// Start playing from `from` at `speed` (1.0 normal). With `preroll` the state is
    /// [`PlaybackState::Prerolling`] until [`step`](Self::step) sees the frames ready or
    /// [`PREROLL_TIMEOUT_S`] pass; without it the clock runs at once. Playing again while playing
    /// changes the speed and re-anchors at the current position (shuttle).
    // filmcraft: crates/ui-egui/src/app/playback.rs:18 `play` (anchor time and tick, preroll).
    pub fn play(&mut self, now: f64, from: Tick, speed: f64, preroll: bool) {
        let from = if self.state == PlaybackState::Playing { self.position(now) } else { from };
        self.speed = speed;
        self.anchor = from;
        self.anchor_time = now;
        self.external = None;
        self.state = if preroll { PlaybackState::Prerolling { since: now } } else { PlaybackState::Playing };
    }

    /// Stop and return where the playhead stopped.
    // filmcraft: crates/ui-egui/src/app/playback.rs:97 `stop`.
    pub fn stop(&mut self, now: f64) -> Tick {
        let at = self.position(now);
        self.state = PlaybackState::Stopped;
        self.anchor = at;
        self.anchor_time = now;
        self.external = None;
        self.stop_at = None;
        at
    }

    /// Space: stop when playing (returns the position), else play at 1.0 from `from` with
    /// preroll.
    // filmcraft: crates/ui-egui/src/app/playback.rs:10 `toggle_play`.
    pub fn toggle(&mut self, now: f64, from: Tick) -> Option<Tick> {
        if self.is_playing() {
            Some(self.stop(now))
        } else {
            self.play(now, from, 1.0, true);
            None
        }
    }

    /// Move the playhead to `t` while playing (a click in the ruler): re-anchors there; the
    /// state is unchanged.
    pub fn seek(&mut self, now: f64, t: Tick) {
        self.anchor = t;
        self.anchor_time = now;
        self.external = None;
    }

    /// The playhead at `now`: the anchor while stopped or prerolling; while playing the anchor
    /// plus `(now − anchor_time) × speed`, or the external clock's reading while it moves, folded
    /// into the loop range and held at the stop point.
    pub fn position(&self, now: f64) -> Tick {
        match self.state {
            PlaybackState::Stopped | PlaybackState::Prerolling { .. } => self.anchor,
            PlaybackState::Playing => {
                let raw = self.raw(now);
                match self.loop_range.filter(|r| !r.is_empty()) {
                    Some(range) if self.speed > 0.0 && raw >= range.end() => folded(raw, range),
                    Some(range) if self.speed < 0.0 && raw < range.start => folded(raw, range),
                    Some(_) => raw,
                    None => match self.stop_at {
                        Some(at) if self.speed > 0.0 && raw >= at => at,
                        _ if raw < Tick::ZERO => Tick::ZERO,
                        _ => raw,
                    },
                }
            }
        }
    }

    /// Advance the state machine for this pass. `frames_ready` is whether the frames preroll
    /// waits for are there ([`FrameServer::preroll_ready`](crate::frames::FrameServer::preroll_ready)).
    /// Returns what happened this pass, in order (a long pass can loop and then stop).
    pub fn step(&mut self, now: f64, frames_ready: bool) -> Vec<ClockEvent> {
        let mut events = Vec::new();
        if let PlaybackState::Prerolling { since } = self.state {
            if !(frames_ready || now - since >= PREROLL_TIMEOUT_S) {
                return events;
            }
            self.state = PlaybackState::Playing;
            self.anchor_time = now;
            events.push(ClockEvent::Started);
        }
        if self.state != PlaybackState::Playing {
            return events;
        }
        let raw = self.raw(now);
        if let Some(range) = self.loop_range.filter(|r| !r.is_empty()) {
            if self.speed > 0.0 && raw >= range.end() || self.speed < 0.0 && raw < range.start {
                self.anchor = folded(raw, range);
                self.anchor_time = now;
                self.external = None;
                events.push(ClockEvent::Looped);
            }
        } else if let Some(at) = self.stop_at.filter(|at| self.speed > 0.0 && raw >= *at) {
            self.halt(now, at);
            events.push(ClockEvent::Stopped(at));
        } else if self.speed < 0.0 && raw <= Tick::ZERO {
            self.halt(now, Tick::ZERO);
            events.push(ClockEvent::Stopped(Tick::ZERO));
        }
        events
    }

    /// An external clock (the audio device's played-sample count, as a time) read at `now`. While
    /// it keeps moving it is the playhead; when it stands still for [`EXTERNAL_STALL_S`] while
    /// playing, the wall clock takes over from its last reading (FilmCraft's audio clock,
    /// `lib.rs:141`).
    pub fn sync_external(&mut self, position: Tick, now: f64) {
        if self.state != PlaybackState::Playing {
            return;
        }
        match self.external {
            Some((last, _)) if last == position => {}
            _ => self.external = Some((position, now)),
        }
    }
}

/// How the view follows the playhead while playing. SoundCraft's five Scrolling modes; FilmCraft's
/// Timeline Playback Auto-Scrolling is `None` (No Scroll), `Page` (Page Scroll) and `Center`
/// (Smooth Scroll).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum FollowMode {
    /// The view stays where it is.
    None,
    /// The view stays while playing and jumps to the playhead when playback stops.
    AfterPlayback,
    /// When the playhead passes 95 % of the width (or is left of the view) the view moves so it
    /// is 5 % from the left.
    #[default]
    Page,
    /// The playhead stays in the middle from the first frame; the content moves under it.
    Continuous,
    /// The playhead moves to the middle, then stays there while the content moves.
    Center,
}

/// The follow state of one view: whether something other than the follow moved the view during
/// playback (a pan, the scroll bar, the overview), which holds the follow until the playhead is
/// back in view or playback stops or starts.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Follow {
    hold: bool,
    /// The view start seen last pass and the start the follow last asked for.
    last_start: Option<Tick>,
    last_target: Option<Tick>,
    /// Whether the last pass was playing, to reveal the playhead once when it stops.
    was_playing: bool,
}

impl Follow {
    /// Forget the hold: on play, on stop, and when the user picks a scrolling mode again.
    pub fn reset(&mut self) {
        *self = Follow::default();
    }

    /// Whether the follow is held.
    pub fn is_held(&self) -> bool {
        self.hold
    }

    /// The new view start for this pass, if the view should move. `playing` is the clock's; a
    /// view whose start differs from both the start seen last pass and the follow's own last
    /// target was moved by something else, so the follow holds and returns `None`. A playhead
    /// back in view clears the hold. While stopped, [`FollowMode::AfterPlayback`] reveals the
    /// playhead once (on the pass playback stopped).
    // soundcraft: crates/ui-egui/src/edit_window/scroll.rs:121 `follow_update` (the hold);
    // filmcraft: crates/ui-egui/src/panels/timeline.rs:513 (the 95 % / 5 % page rule and the
    // centred smooth scroll).
    pub fn update(&mut self, mode: FollowMode, playing: bool, view: &TimeView, width: f32, playhead: Tick) -> Option<Tick> {
        let was_playing = std::mem::replace(&mut self.was_playing, playing);
        let start = view.start;
        let moved_by_other = self.last_start.is_some_and(|last| last != start) && self.last_target != Some(start);
        self.last_start = Some(start);
        if !playing {
            self.hold = false;
            self.last_target = None;
            return (was_playing && mode == FollowMode::AfterPlayback).then(|| view.scroll_to_reveal(playhead, width)).flatten();
        }
        if !was_playing {
            self.hold = false;
        }
        let follows = matches!(mode, FollowMode::Page | FollowMode::Continuous | FollowMode::Center);
        let pps = view.points_per_second;
        let (width, x) = (f64::from(width), (playhead - start).seconds() * pps);
        let in_view = (0.0..=width).contains(&x);
        if in_view {
            self.hold = false;
        } else if follows && moved_by_other {
            self.hold = true;
        }
        if !follows || self.hold || !(pps > 0.0 && pps.is_finite()) {
            return None;
        }
        let behind = |share: f64| playhead - Tick::from_seconds_f64(width * share / pps);
        let target = match mode {
            FollowMode::Page => (x > width * PAGE_EDGE || x < 0.0).then(|| behind(PAGE_MARGIN)),
            FollowMode::Center => (x > width * CENTRE || x < 0.0).then(|| behind(CENTRE)),
            FollowMode::Continuous => Some(behind(CENTRE)),
            FollowMode::None | FollowMode::AfterPlayback => None,
        }?
        .max(Tick::ZERO);
        self.last_target = Some(target);
        (target != start).then_some(target)
    }
}
