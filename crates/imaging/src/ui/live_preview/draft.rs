//! [`DraftScale`]: LightCraft's learned size of a slider drag's whole-frame draft.

use std::collections::VecDeque;

/// The size of a slider drag's whole-frame draft, chosen when the drag starts and kept until it
/// ends (a change in the middle would make the resolution jump), unless its full-size drafts are
/// far over budget. Opt-in per app: LightCraft uses it, PhotoCraft keeps its Low Resolution
/// Previews switch ([`ProxyPlan`](super::ProxyPlan)).
///
/// Rules (LightCraft's `DragScale`, constants exact):
/// - A drag renders at full size (1.0) when the GPU draws the view and the (item, control) pair
///   is not known slow; otherwise at [`SMALL`](Self::SMALL) (0.6).
/// - A draft's time is its render plus the frame that shows it (the upload is part of it); a
///   host frame time over 1000 ms counts as unknown (0). Each render key is timed once.
/// - Mid-drag give-up: with more than 3 full-size samples whose median after the first exceeds
///   2 × [`BUDGET_MS`](Self::BUDGET_MS) (16 ms), the drag drops to [`SMALL`](Self::SMALL).
/// - At the drag's end, a drag timed at full size with a median after the first over
///   1.25 × [`BUDGET_MS`](Self::BUDGET_MS) (20 ms) counts against its pair: a first time makes it
///   suspect (the next drag stays full size), a second slow for 30 s, and each slow retry doubles
///   the wait up to 240 s; a drag within it forgets the verdict. A drag never timed at full size
///   changes nothing.
/// - Verdicts are per item (another photo is timed afresh); at most 256 pairs are kept, the one
///   remembered longest ago forgotten first.
///
/// Times: `now` is the host's clock in seconds (egui's `input.time`), durations in milliseconds.
#[derive(Clone, Debug, Default)]
pub struct DraftScale {
    /// The drag running now: (item, control) and its scale.
    current: Option<((u64, String), f32)>,
    /// How long the current drag's full-size drafts took to show (ms), at most 1024.
    samples: Vec<f64>,
    /// The render key last timed.
    seen: Option<u64>,
    /// A shown draft's render time, waiting for the frame time of the pass that uploaded it.
    pending: Option<f64>,
    /// Pairs found over budget, the one remembered longest ago first.
    verdicts: VecDeque<((u64, String), Verdict)>,
}

/// Full-size drafts after the first that a drag times before it may give up on full size.
const GIVE_UP_AFTER: usize = 3;
/// Draft times kept per drag.
const SAMPLES_KEPT: usize = 1024;
/// Over this many times the budget, a drag gives up on full size.
const GIVE_UP_FACTOR: f64 = 2.0;
/// Pairs kept.
const VERDICTS_KEPT: usize = 256;
/// A host frame time over this (ms) counts as unknown.
const FRAME_MS_KNOWN: f64 = 1000.0;
/// A drag counts against its pair only over this (ms): a quarter over the budget.
const VERDICT_MS: f64 = DraftScale::BUDGET_MS * 1.25;
/// The wait (s) after a pair's second slow drag, before a drag tries full size again.
const RETRY_AFTER_S: f64 = 30.0;
/// The longest wait (s).
const RETRY_MAX_S: f64 = 240.0;

/// What [`DraftScale`] remembers about the drags of one control on one item.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Verdict {
    /// One drag was over budget at full size: the next stays full size.
    Suspect,
    /// Two in a row were: drafts are small until `until` (s); `wait` is how long this holds (s).
    Slow { until: f64, wait: f64 },
}

impl DraftScale {
    /// The scale of a draft that is not full size.
    pub const SMALL: f32 = 0.6;

    /// The slider budget per draft, in milliseconds.
    pub const BUDGET_MS: f64 = 16.0;

    fn verdict(&self, key: &(u64, String)) -> Option<Verdict> {
        self.verdicts.iter().find(|(k, _)| k == key).map(|(_, v)| *v)
    }

    /// Sets or (`None`) forgets a pair's verdict. A new pair forgets the one remembered longest
    /// ago when the list is full.
    fn set_verdict(&mut self, key: (u64, String), verdict: Option<Verdict>) {
        let at = self.verdicts.iter().position(|(k, _)| *k == key);
        match (at, verdict) {
            (Some(i), Some(v)) => {
                if let Some(entry) = self.verdicts.get_mut(i) {
                    entry.1 = v;
                }
            }
            (Some(i), None) => {
                self.verdicts.remove(i);
            }
            (None, Some(v)) => {
                while self.verdicts.len() >= VERDICTS_KEPT {
                    self.verdicts.pop_front();
                }
                self.verdicts.push_back((key, v));
            }
            (None, None) => {}
        }
    }

    /// The draft scale for this pass, or `None` without a drag. `drag` is the item (the host's id
    /// of the photo or document) and the control being dragged; `gpu` whether the GPU draws this
    /// view; `shown` the key of the whole-frame render on screen (a render from before the drag
    /// is not one of its drafts). A drag of another pair, or `None`, ends the running drag first
    /// (its verdict is updated at `now`).
    pub fn scale(&mut self, drag: Option<(u64, &str)>, gpu: bool, shown: Option<u64>, now: f64) -> Option<f32> {
        // lightcraft: crates/ui-egui/src/region.rs:176 `DragScale::scale`, :99 `CPU_DRAFT_SCALE` (0.6).
        let Some((item, control)) = drag else {
            self.finish(now);
            return None;
        };
        match &self.current {
            Some(((i, c), s)) if *i == item && c == control => return Some(*s),
            Some(_) => self.finish(now),
            None => {}
        }
        let key = (item, control.to_string());
        let small = matches!(self.verdict(&key), Some(Verdict::Slow { until, .. }) if now < until);
        let s = if gpu && !small { 1.0 } else { Self::SMALL };
        self.current = Some((key, s));
        self.seen = shown;
        Some(s)
    }

    /// Once per pass: `key` names the whole-frame render on screen and `render_ms` how long it
    /// took; `last_frame_ms` how long the previous pass took, painting included (`None` without a
    /// host report, e.g. headless). Times the current drag's full-size drafts and gives up on full
    /// size when they are far over budget (the type's rules).
    pub fn observe(&mut self, key: u64, render_ms: f64, last_frame_ms: Option<f64>) {
        // lightcraft: crates/ui-egui/src/region.rs:199 `observe`, :103 `DRAFT_BUDGET_MS` (16), :107
        // `GIVE_UP_FACTOR` (2), :110 `GIVE_UP_AFTER` (3), :118 `FRAME_MS_KNOWN` (1000), :288
        // `median_after_first`.
        let full = matches!(self.current, Some((_, s)) if s >= 1.0);
        if !full {
            self.pending = None;
            return;
        }
        if let Some(render) = self.pending.take() {
            let frame = last_frame_ms.filter(|ms| ms.is_finite() && (0.0..=FRAME_MS_KNOWN).contains(ms)).unwrap_or(0.0);
            if self.samples.len() < SAMPLES_KEPT {
                self.samples.push(render + frame);
            }
            if self.samples.len() > GIVE_UP_AFTER
                && median_after_first(&self.samples).is_some_and(|m| m > Self::BUDGET_MS * GIVE_UP_FACTOR)
                && let Some((_, s)) = self.current.as_mut()
            {
                *s = Self::SMALL;
                return;
            }
        }
        if self.seen == Some(key) {
            return;
        }
        self.seen = Some(key);
        if render_ms.is_finite() && render_ms >= 0.0 {
            self.pending = Some(render_ms);
        }
    }

    /// The drag ended: updates the pair's verdict from the drag's full-size drafts (the type's
    /// rules) and clears the drag. [`scale`](Self::scale) with `None` calls it.
    fn finish(&mut self, now: f64) {
        // lightcraft: crates/ui-egui/src/region.rs:264 `finish`, :122 `VERDICT_MS` (16 × 1.25), :127
        // `RETRY_AFTER_S` (30), :130 `RETRY_MAX_S` (240), :239 `set_verdict`, :114 `VERDICTS_KEPT`
        // (256).
        if let Some((key, _)) = self.current.take()
            && let Some(median) = median_after_first(&self.samples)
        {
            let verdict = if median > VERDICT_MS {
                let wait = match self.verdict(&key) {
                    None => None,
                    Some(Verdict::Suspect) => Some(RETRY_AFTER_S),
                    Some(Verdict::Slow { wait, .. }) => Some((wait * 2.0).min(RETRY_MAX_S)),
                };
                Some(wait.map_or(Verdict::Suspect, |wait| Verdict::Slow { until: now + wait, wait }))
            } else {
                None
            };
            self.set_verdict(key, verdict);
        }
        self.samples.clear();
        self.seen = None;
        self.pending = None;
    }
}

/// The median of a drag's draft times (ms), leaving out the first draft (it rebuilds the stage
/// caches the others reuse); `None` before a second draft.
fn median_after_first(samples: &[f64]) -> Option<f64> {
    let mut rest: Vec<f64> = samples.iter().skip(1).copied().collect();
    rest.sort_by(f64::total_cmp);
    rest.get(rest.len() / 2).copied()
}
