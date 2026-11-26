//! `PenSource`: the tablet's readings, which the runtime's one pointer feed pairs with each pointer move.

use crate::runtime::pass_state;
use craft_behavior::PointerSample;
use egui::Context;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

/// The most readings kept between two passes: a stalled app drops the oldest rather than grow without bound.
const MOST_QUEUED: usize = 4096;

/// What the tablet hooks have written and the pass has not yet read.
#[derive(Default)]
struct Readings {
    queued: Vec<PointerSample>,
    current: Option<PointerSample>,
    /// The last reading of the last batch drained: the first move of the next batch starts from it.
    before: Option<PointerSample>,
    barrel: bool,
    /// The pen tip went down with the barrel button held: the pointer feed reports the contact as the secondary
    /// button until the tip lifts, also when the barrel button comes up first.
    barrel_press: bool,
}

/// A shared handle the app's platform tablet hooks write into and the runtime drains in each pass that carries pointer
/// events. Clones share one queue.
#[derive(Clone, Default)]
pub struct PenSource {
    readings: Arc<Mutex<Readings>>,
}

impl PenSource {
    /// The readings; a hook that panicked while it held them leaves values that are still readings.
    fn lock(&self) -> MutexGuard<'_, Readings> {
        self.readings.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Queues one tablet reading in order; `None` says the pen left the tablet.
    pub fn set(&self, sample: Option<PointerSample>) {
        let mut readings = self.lock();
        let sample = sample.map(PointerSample::sanitized);
        if let Some(sample) = sample {
            if readings.queued.len() >= MOST_QUEUED {
                readings.queued.remove(0);
            }
            readings.queued.push(sample);
        }
        readings.current = sample;
    }

    /// Sets whether the barrel button is down.
    pub fn set_barrel(&self, down: bool) {
        self.lock().barrel = down;
    }

    /// Takes the readings queued since the last drain, in order, with the last reading of the drain before. A pen
    /// reports pressure 0 while it hovers and as it lifts (X11, macOS), which is no contact: a batch that also holds a
    /// contact reading drops its pressure-0 readings, so a stroke's last moves keep the pressure the pen left with and
    /// a tap its own (#1798).
    pub(crate) fn drain(&self) -> (Option<PointerSample>, Vec<PointerSample>) {
        let mut readings = self.lock();
        let mut samples = std::mem::take(&mut readings.queued);
        if samples.iter().any(|sample| sample.pressure > 0.0) {
            samples.retain(|sample| sample.pressure > 0.0);
        }
        let before = readings.before;
        if let Some(&last) = samples.last() {
            readings.before = Some(last);
        }
        (before, samples)
    }

    /// The latest reading, if the pen is on the tablet.
    pub fn current(&self) -> Option<PointerSample> {
        self.lock().current
    }

    /// Whether the barrel button is down.
    pub(crate) fn barrel(&self) -> bool {
        self.lock().barrel
    }

    /// Whether the contact under way went down with the barrel button held.
    pub(crate) fn barrel_press(&self) -> bool {
        self.lock().barrel_press
    }

    /// Records whether the contact under way reports as the secondary button.
    pub(crate) fn set_barrel_press(&self, down: bool) {
        self.lock().barrel_press = down;
    }
}

/// The pen source `install` was given, in the context's pass state.
#[derive(Default)]
pub(crate) struct InstalledPen(Option<PenSource>);

/// Keeps `source` as the context's pen feed; `None` reports no pen.
pub(crate) fn install(ctx: &Context, source: Option<PenSource>) {
    pass_state::with::<InstalledPen, _>(ctx, |pen| pen.0 = source);
}

/// The context's pen feed, when the app gave one.
pub(crate) fn source(ctx: &Context) -> Option<PenSource> {
    pass_state::with::<InstalledPen, _>(ctx, |pen| pen.0.clone())
}

/// What one pass reads of the pen: the readings since the last drain, the current one and the barrel button.
pub(crate) struct PenPass {
    /// The last reading before the pass's own.
    before: Option<PointerSample>,
    samples: Vec<PointerSample>,
    /// The pen's reading for the pass as a whole (its press and release): the latest reading, or the pass's last
    /// contact reading when the pen lifted (or reads pressure 0) by the time the pass runs, as after a tap shorter than
    /// a pass; `None` once the pen left the tablet with no contact in the pass.
    pub current: Option<PointerSample>,
    /// The barrel button is down.
    pub barrel: bool,
    /// The contact under way went down with the barrel button held.
    pub barrel_press: bool,
}

impl PenPass {
    /// Drains `source` for a pass whose events carry pointer events: its moves take the readings.
    pub(crate) fn read(source: &PenSource) -> Self {
        let (before, samples) = source.drain();
        let latest = source.current();
        let contact = samples.last().copied().filter(|sample| sample.pressure > 0.0);
        let current = match contact {
            Some(contact) if latest.is_none_or(|latest| latest.pressure <= 0.0) => Some(contact),
            _ => latest,
        };
        PenPass { before, samples, current, barrel: source.barrel(), barrel_press: source.barrel_press() }
    }

    /// The live state of `source` with no readings, for a pass with no pointer event: the queue stays for the pass
    /// that carries the moves.
    pub(crate) fn live(source: &PenSource) -> Self {
        PenPass { before: None, samples: Vec::new(), current: source.current(), barrel: source.barrel(), barrel_press: source.barrel_press() }
    }

    /// The pass of an injected step that carries the pen's reading `sample`: the one reading of the pass, on the
    /// tablet, with the barrel button up.
    pub(crate) fn injected(sample: PointerSample) -> Self {
        let sample = sample.sanitized();
        PenPass { before: None, samples: vec![sample], current: Some(sample), barrel: false, barrel_press: false }
    }

    /// Whether the pen reported in the pass or is on the tablet: the pointer is the pen.
    pub(crate) fn active(&self) -> bool {
        self.current.is_some() || !self.samples.is_empty()
    }

    /// The reading of move `nr` of the pass's `moves` pointer moves. The pen reports at its own rate: the pass's
    /// readings, after the last reading before them, spread evenly over its moves, so as many moves as readings take
    /// one reading each and the last move takes the latest. A move between two readings takes the pressure and tilt
    /// between them, the rotation between them the short way round, and the nearer one's end. A pass with no new
    /// reading repeats the current one.
    pub(crate) fn paired(&self, nr: usize, moves: usize) -> Option<PointerSample> {
        let Some(&first) = self.samples.first() else { return self.current };
        let start = self.before.unwrap_or(first);
        let count = self.samples.len();
        // Point 0 is the reading before the pass, point i the pass's reading i - 1.
        let point = |i: usize| if i == 0 { start } else { self.samples[i.min(count) - 1] };
        let moves = moves.max(1);
        let at = ((nr + 1).min(moves) * count) as f32 / moves as f32;
        let below = at.floor() as usize;
        let (a, b, t) = (point(below), point(below + 1), at - below as f32);
        let lerp = |from: f32, to: f32| from + (to - from) * t;
        let turn = std::f32::consts::TAU;
        let rotation = (b.rotation - a.rotation + turn * 1.5).rem_euclid(turn) - turn / 2.0;
        let mut sample = if t < 0.5 { a } else { b };
        sample.pressure = lerp(a.pressure, b.pressure);
        sample.tilt = [lerp(a.tilt[0], b.tilt[0]), lerp(a.tilt[1], b.tilt[1])];
        sample.rotation = a.rotation + rotation * t;
        Some(sample.sanitized())
    }
}
