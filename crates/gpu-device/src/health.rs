//! Device health: notices a lost wgpu device or an uncaptured wgpu error, so callers stop issuing
//! GPU work and reopen a device or fall back, instead of panicking.
//!
//! wgpu's default uncaptured-error handler panics, and `Device::poll` panics on a lost device
//! whatever the handler. [`DeviceHealth::watch`] replaces the handler with one that records the
//! fault and registers the device-lost callback, and the polls here run only while the device is
//! healthy, catching the poll's panic as a last resort. Every GPU entry point checks
//! [`DeviceHealth::fault`] first.
//!
//! Builds for the web too: the callbacks only touch atomics and a mutex.

use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

#[cfg(not(target_arch = "wasm32"))]
use std::time::{Duration, Instant};

/// Outcome of a bounded wait for submitted GPU work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WaitOutcome {
    /// The submitted work finished before the deadline.
    Finished,
    /// The deadline passed (or the device faulted) before the work finished.
    TimedOut,
    /// The caller canceled the wait before the work finished.
    Canceled,
}

/// Why the GPU cannot be used any more.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fault {
    /// The device was lost (driver reset, the OS ended a submission, `Device::destroy`).
    Lost(String),
    /// An uncaptured wgpu error (out of memory, validation, internal): the GPU output cannot be
    /// trusted, and on most drivers the device is about to be lost anyway.
    Error(String),
}

impl Fault {
    /// Whether the device itself was lost (rather than an error reported).
    pub fn is_lost(&self) -> bool {
        matches!(self, Fault::Lost(_))
    }

    /// An out-of-memory error on a device that still works: freeing memory, calling
    /// [`DeviceHealth::clear_out_of_memory`] with the count read from
    /// [`DeviceHealth::out_of_memory_count`] and trying again can succeed, unlike after a loss or
    /// a validation error. Polls and waits keep running meanwhile. Only wgpu's own out-of-memory
    /// error is recorded with exactly [`OUT_OF_MEMORY`]; a validation error that mentions memory
    /// is not one.
    pub fn is_out_of_memory(&self) -> bool {
        matches!(self, Fault::Error(s) if s == OUT_OF_MEMORY)
    }

    /// The driver's or wgpu's message.
    pub fn detail(&self) -> &str {
        match self {
            Fault::Lost(s) | Fault::Error(s) => s,
        }
    }
}

impl fmt::Display for Fault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Fault::Lost(s) if s.is_empty() => f.write_str("GPU device lost"),
            Fault::Lost(s) => write!(f, "GPU device lost: {s}"),
            Fault::Error(s) => write!(f, "GPU error: {s}"),
        }
    }
}

/// The message an out-of-memory [`Fault::Error`] carries (wgpu's `Error::OutOfMemory`).
pub const OUT_OF_MEMORY: &str = "Out of Memory";

/// The state wgpu's callbacks write and callers read, without a lock: flags in atomics and each
/// fault's text in a [`OnceLock`], whose `get` does not wait. A `set` that races another `set`
/// of the same slot waits only for that one store.
#[derive(Default)]
struct Inner {
    /// The first device loss.
    lost: OnceLock<String>,
    /// Out-of-memory errors seen.
    out_of_memory_seen: AtomicU64,
    /// Out-of-memory errors the caller cleared ([`DeviceHealth::clear_out_of_memory`]).
    out_of_memory_cleared: AtomicU64,
    /// The first other error.
    error: OnceLock<String>,
    errors: AtomicU64,
    /// wgpu itself reported the device lost (see [`DeviceHealth::device_lost`]).
    device_lost: AtomicBool,
}

/// Shared health flag of one wgpu device. Cheap to clone; every clone sees the same state.
#[derive(Clone, Default)]
pub struct DeviceHealth(Arc<Inner>);

impl fmt::Debug for DeviceHealth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DeviceHealth").field("fault", &self.fault()).field("errors", &self.errors()).finish()
    }
}

impl DeviceHealth {
    /// A healthy flag not attached to a device (tests, CPU-only callers).
    pub fn new() -> Self {
        Self::default()
    }

    /// Watch `device`: its device-lost callback and uncaptured errors mark the returned flag
    /// instead of panicking. Replaces any handler set before (wgpu keeps one of each).
    pub fn watch(device: &wgpu::Device) -> Self {
        let health = Self::new();
        let lost = health.clone();
        // Called by wgpu with its own locks held: only record, never call back into wgpu.
        device.set_device_lost_callback(move |reason, message| {
            let detail = match (reason, message.is_empty()) {
                (wgpu::DeviceLostReason::Destroyed, true) => "device destroyed".to_string(),
                (_, true) => "lost by the driver".to_string(),
                (_, false) => message,
            };
            log::error!("wgpu device lost ({reason:?}): {detail}");
            lost.0.device_lost.store(true, Ordering::Release);
            lost.mark(Fault::Lost(detail));
        });
        let errors = health.clone();
        device.on_uncaptured_error(Arc::new(move |e: wgpu::Error| errors.uncaptured(&e)));
        health
    }

    /// Record an uncaptured wgpu error.
    fn uncaptured(&self, e: &wgpu::Error) {
        self.0.errors.fetch_add(1, Ordering::Relaxed);
        log::error!("uncaptured wgpu error: {e}");
        self.mark(fault_of(e));
    }

    /// Record an error an error scope caught.
    ///
    /// The uncaptured-error handler never sees a scoped error, so without this a watched device
    /// would keep being handed work after it failed. Polls first: a lost device is a
    /// [`Fault::Lost`], and its callback may not have run yet (it runs on a poll). The poll does
    /// not block.
    pub fn caught(&self, device: &wgpu::Device, error: &wgpu::Error) {
        if self.lost() {
            return;
        }
        #[cfg(not(target_arch = "wasm32"))]
        self.poll(device);
        #[cfg(target_arch = "wasm32")]
        let _ = device;
        if self.lost() {
            return;
        }
        self.mark(fault_of(error));
    }

    fn lost(&self) -> bool {
        self.fault().is_some_and(|f| f.is_lost())
    }

    /// Mark the device unusable. The first fault is kept; a later device loss replaces an error,
    /// and a later out-of-memory error is reported over another error, so a caller still sees
    /// that freeing memory may help after a validation error in the same frame.
    pub fn mark(&self, fault: Fault) {
        if fault.is_out_of_memory() {
            self.0.out_of_memory_seen.fetch_add(1, Ordering::AcqRel);
            return;
        }
        // A slot already set keeps its first text.
        let _ = match fault {
            Fault::Lost(text) => self.0.lost.set(text),
            Fault::Error(text) => self.0.error.set(text),
        };
    }

    /// Out-of-memory errors recorded so far. Read it when handling an out-of-memory fault, and
    /// pass it to [`clear_out_of_memory`](Self::clear_out_of_memory) once memory was freed.
    pub fn out_of_memory_count(&self) -> u64 {
        self.0.out_of_memory_seen.load(Ordering::Acquire)
    }

    /// Clear the out-of-memory errors up to `count` (from
    /// [`out_of_memory_count`](Self::out_of_memory_count)) after the caller freed memory. One
    /// recorded after `count` was read stays. The device counts as healthy again unless it was
    /// lost or another error was recorded, which [`fault`](Self::fault) then reports.
    pub fn clear_out_of_memory(&self, count: u64) {
        let count = count.min(self.0.out_of_memory_seen.load(Ordering::Acquire));
        self.0.out_of_memory_cleared.fetch_max(count, Ordering::AcqRel);
    }

    /// Whether an out-of-memory error is recorded and not cleared.
    fn out_of_memory(&self) -> bool {
        let cleared = self.0.out_of_memory_cleared.load(Ordering::Acquire);
        self.0.out_of_memory_seen.load(Ordering::Acquire) > cleared
    }

    /// Whether the device may still be used: no fault was recorded, or only out-of-memory errors
    /// that [`clear_out_of_memory`](Self::clear_out_of_memory) cleared.
    pub fn is_ok(&self) -> bool {
        self.0.lost.get().is_none() && self.0.error.get().is_none() && !self.out_of_memory()
    }

    /// Whether polls and waits still run: the device was not lost and reported no error other
    /// than out of memory. A poll reclaims the staging memory that recovering from an
    /// out-of-memory error needs.
    fn pollable(&self) -> bool {
        self.0.lost.get().is_none() && self.0.error.get().is_none()
    }

    /// Why the device cannot be used, if it cannot.
    pub fn fault(&self) -> Option<Fault> {
        if let Some(text) = self.0.lost.get() {
            return Some(Fault::Lost(text.clone()));
        }
        if self.out_of_memory() {
            return Some(Fault::Error(OUT_OF_MEMORY.into()));
        }
        self.0.error.get().map(|text| Fault::Error(text.clone()))
    }

    /// Uncaptured wgpu errors seen so far.
    pub fn errors(&self) -> u64 {
        self.0.errors.load(Ordering::Relaxed)
    }

    /// Whether wgpu itself reported the device lost: its device-lost callback ran, or polling it
    /// panicked. A fault recorded with [`mark`](Self::mark) is not one. After a real loss nothing
    /// drawn with the device reaches the screen, the window's own frames included.
    pub fn device_lost(&self) -> bool {
        self.0.device_lost.load(Ordering::Acquire)
    }

    /// Non-blocking poll that runs even after a fault was recorded, so a device that is lost
    /// later (a driver reset while the app sat idle, or the PC slept) is noticed before the next
    /// frame is drawn with it. Skipped once [`device_lost`](Self::device_lost).
    pub fn probe(&self, device: &wgpu::Device) {
        if self.device_lost() {
            return;
        }
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| device.poll(wgpu::PollType::Poll))).is_err() {
            self.lost_by_poll();
        }
    }

    /// `Device::poll` panicked: wgpu found the device lost.
    fn lost_by_poll(&self) {
        self.0.device_lost.store(true, Ordering::Release);
        self.mark(Fault::Lost("the device stopped responding".into()));
    }

    /// Block until `index` (or all submitted work) finished, unless the device was lost or
    /// reported an error other than out of memory. Returns whether the work finished with the
    /// device still usable: `false` when it was lost or reported such an error, before or during
    /// the wait, and when the poll itself failed, so the work may still be running. After an
    /// out-of-memory error the wait still runs. A panic in `Device::poll` is caught and recorded
    /// as a loss. Native only: the web has no blocking wait.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn wait(&self, device: &wgpu::Device, index: Option<wgpu::SubmissionIndex>) -> bool {
        if !self.pollable() {
            return false;
        }
        let poll =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| device.poll(wgpu::PollType::Wait { submission_index: index, timeout: None })));
        match poll {
            Ok(Ok(_)) => self.pollable(),
            Ok(Err(e)) => {
                log::warn!("wgpu poll: {e}");
                false
            }
            Err(_) => {
                self.lost_by_poll();
                false
            }
        }
    }

    /// Non-blocking poll (reclaims staging memory), skipped once the device was lost or reported
    /// an error other than out of memory.
    pub fn poll(&self, device: &wgpu::Device) {
        if !self.pollable() {
            return;
        }
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| device.poll(wgpu::PollType::Poll))).is_err() {
            self.lost_by_poll();
        }
    }

    /// Like [`Self::wait`], but bounded: returns whether the work up to `index` finished within
    /// `timeout`. To wait for everything submitted so far, pass the index of an empty submission
    /// (`queue.submit([])`); work submitted after it does not extend the wait. A timeout marks the
    /// device faulted, so later work stops going to a GPU that no longer answers. A submission
    /// index the device never issued returns `false` and leaves the device healthy. A zero
    /// `timeout`, or one that passes before the first poll, returns `false` and leaves the device
    /// healthy. A device that was already lost, or that reported an error other than out of
    /// memory, returns `false` unchanged.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn wait_timeout(&self, device: &wgpu::Device, index: wgpu::SubmissionIndex, timeout: Duration) -> bool {
        self.wait_timeout_cancelable(device, index, timeout, &|| false) == WaitOutcome::Finished
    }

    /// [`Self::wait_timeout`] that polls in slices of at most 100 ms and checks `canceled` between
    /// them, keeping the same overall deadline. Every slice waits for the same `index`. A slice
    /// that times out means the work has not finished yet; only the overall deadline (or a poll
    /// that panics) marks the device faulted.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn wait_timeout_cancelable(
        &self,
        device: &wgpu::Device,
        index: wgpu::SubmissionIndex,
        timeout: Duration,
        canceled: &(dyn Fn() -> bool + Sync),
    ) -> WaitOutcome {
        if !self.pollable() {
            return WaitOutcome::TimedOut;
        }
        let mut wrong_index = false;
        let mut polled = false;
        let start = Instant::now();
        let elapsed = || start.elapsed();
        let outcome = wait_slices(timeout, canceled, elapsed, |slice| {
            polled = true;
            let poll = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                device.poll(wgpu::PollType::Wait { submission_index: Some(index.clone()), timeout: Some(slice) })
            }));
            match poll {
                Ok(Ok(_)) => {
                    if self.pollable() {
                        SlicePoll::Done
                    } else {
                        SlicePoll::Failed
                    }
                }
                Ok(Err(wgpu::PollError::Timeout)) => SlicePoll::Pending,
                // A submission index the device never issued: the caller's mistake, not the GPU's.
                Ok(Err(e)) => {
                    log::warn!("wgpu poll: {e}");
                    wrong_index = true;
                    SlicePoll::Failed
                }
                Err(_) => {
                    self.lost_by_poll();
                    SlicePoll::Failed
                }
            }
        });
        // A deadline that passed before the first poll says nothing about the device.
        if outcome == WaitOutcome::TimedOut && polled && !wrong_index && self.pollable() {
            self.mark(Fault::Lost(format!("GPU wait timed out after {:.1} s", timeout.as_secs_f64())));
        }
        outcome
    }
}

/// What one bounded poll reported.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SlicePoll {
    Done,
    Pending,
    Failed,
}

/// Wait for `total`, polling in slices no longer than 100 ms and checking `canceled` between
/// them. `elapsed` gives the time since the wait started. A slice that times out is pending, not
/// a failure; only the overall deadline or a failed poll ends the wait early.
#[cfg(not(target_arch = "wasm32"))]
fn wait_slices(
    total: Duration,
    canceled: &(dyn Fn() -> bool + Sync),
    mut elapsed: impl FnMut() -> Duration,
    mut poll: impl FnMut(Duration) -> SlicePoll,
) -> WaitOutcome {
    const SLICE: Duration = Duration::from_millis(100);
    loop {
        if canceled() {
            return WaitOutcome::Canceled;
        }
        let remaining = total.saturating_sub(elapsed());
        if remaining.is_zero() {
            return WaitOutcome::TimedOut;
        }
        match poll(remaining.min(SLICE)) {
            SlicePoll::Done => return WaitOutcome::Finished,
            SlicePoll::Pending => {}
            SlicePoll::Failed => return WaitOutcome::TimedOut,
        }
    }
}

/// The whole message of wgpu's `DeviceError::Lost`.
const LOST_CAUSE: &str = "Parent device is lost";

/// Whether one of the errors that caused `e` (its `source` chain) is wgpu's `DeviceError::Lost`:
/// an error whose whole message is [`LOST_CAUSE`]. A resource label is part of another error's
/// message, so a label never matches.
fn caused_by_loss(e: &wgpu::Error) -> bool {
    let source: &(dyn std::error::Error + 'static) = match e {
        wgpu::Error::Validation { source, .. } | wgpu::Error::Internal { source, .. } => source.as_ref(),
        _ => return false,
    };
    std::iter::successors(Some(source), |c| c.source()).any(|c| c.to_string().trim().eq_ignore_ascii_case(LOST_CAUSE))
}

/// The fault a wgpu error stands for. An error caused by a lost device ([`caused_by_loss`])
/// follows a device loss whose callback may not have run yet (it runs on the next poll), so it
/// counts as a loss rather than as an error. An error about a destroyed buffer or texture is an
/// error. Only wgpu's own out-of-memory error is [`OUT_OF_MEMORY`]. The text keeps the causes
/// ([`one_line`]).
fn fault_of(e: &wgpu::Error) -> Fault {
    let text = e.to_string();
    if matches!(e, wgpu::Error::OutOfMemory { .. }) {
        Fault::Error(OUT_OF_MEMORY.into())
    } else if caused_by_loss(e) {
        Fault::Lost(one_line(&text))
    } else {
        Fault::Error(one_line(&text))
    }
}

/// wgpu's multi-line error text on one line: the heading and each cause, joined with ": ",
/// without wgpu's "Caused by:" line, cut to 200 characters.
fn one_line(s: &str) -> String {
    const MAX: usize = 200;
    let mut out = String::new();
    let mut count = 0;
    let lines = s.lines().map(str::trim).filter(|l| !l.is_empty() && !l.eq_ignore_ascii_case("Caused by:"));
    for (i, line) in lines.enumerate() {
        let separator = if i == 0 { "" } else { ": " };
        for c in separator.chars().chain(line.chars()) {
            if count == MAX {
                out.push('…');
                return out;
            }
            out.push(c);
            count += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_fault_wins_but_loss_replaces_an_error() {
        let h = DeviceHealth::new();
        assert!(h.is_ok());
        assert_eq!(h.fault(), None);
        h.mark(Fault::Error("out of memory".into()));
        assert!(!h.is_ok());
        let c = h.clone();
        c.mark(Fault::Error("second".into()));
        assert_eq!(h.fault(), Some(Fault::Error("out of memory".into())));
        c.mark(Fault::Lost("reset".into()));
        assert_eq!(h.fault(), Some(Fault::Lost("reset".into())));
        c.mark(Fault::Lost("later".into()));
        assert_eq!(h.fault().map(|f| f.to_string()).as_deref(), Some("GPU device lost: reset"));
        // a recorded loss is not wgpu reporting the device lost
        assert!(!h.device_lost());
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn wait_slices_cancel_timeout_and_completion() {
        let never = || false;
        // a clock that does not move: only the polls end these waits
        let still = || Duration::ZERO;
        assert_eq!(wait_slices(Duration::ZERO, &never, still, |_| SlicePoll::Done), WaitOutcome::TimedOut);
        assert_eq!(wait_slices(Duration::from_secs(1), &never, still, |_| SlicePoll::Done), WaitOutcome::Finished);
        assert_eq!(wait_slices(Duration::from_secs(1), &never, still, |_| SlicePoll::Failed), WaitOutcome::TimedOut);

        let mut polls = 0;
        let outcome = wait_slices(Duration::from_secs(1), &never, still, |_| {
            polls += 1;
            if polls < 3 { SlicePoll::Pending } else { SlicePoll::Done }
        });
        assert_eq!(outcome, WaitOutcome::Finished);
        assert_eq!(polls, 3, "a timed-out slice is retried, not treated as a failure");

        let canceled = || true;
        let mut called = false;
        assert_eq!(
            wait_slices(Duration::from_secs(1), &canceled, still, |_| {
                called = true;
                SlicePoll::Done
            }),
            WaitOutcome::Canceled
        );
        assert!(!called, "cancellation is checked before polling");
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn wait_slices_end_at_the_deadline_of_a_moving_clock() {
        let never = || false;
        let now = std::cell::Cell::new(Duration::ZERO);
        let mut slices = Vec::new();
        let clock = || now.get();
        let outcome = wait_slices(Duration::from_millis(250), &never, clock, |slice| {
            slices.push(slice);
            now.set(now.get() + slice);
            SlicePoll::Pending
        });
        assert_eq!(outcome, WaitOutcome::TimedOut);
        assert_eq!(slices, [Duration::from_millis(100), Duration::from_millis(100), Duration::from_millis(50)]);
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn wait_slices_observes_cancel_while_pending() {
        let flag = AtomicBool::new(false);
        let canceled = || flag.load(Ordering::SeqCst);
        let mut polls = 0;
        let still = || Duration::ZERO;
        let outcome = wait_slices(Duration::from_secs(1), &canceled, still, |_| {
            polls += 1;
            flag.store(true, Ordering::SeqCst);
            SlicePoll::Pending
        });
        assert_eq!(outcome, WaitOutcome::Canceled);
        assert_eq!(polls, 1, "a cancel set during a pending poll ends the wait");
    }

    #[test]
    fn fault_text() {
        assert_eq!(Fault::Lost(String::new()).to_string(), "GPU device lost");
        assert_eq!(Fault::Error("x".into()).to_string(), "GPU error: x");
        assert_eq!(one_line("\n  Buffer 'pc' is invalid\nmore"), "Buffer 'pc' is invalid: more");
        assert_eq!(one_line(&"a".repeat(300)).chars().count(), 201);
        assert_eq!(one_line(&"a".repeat(200)), "a".repeat(200));
        assert_eq!(one_line(&format!("{}\nb", "a".repeat(199))), format!("{}:…", "a".repeat(199)));
        assert!(Fault::Error("Out of Memory".into()).is_out_of_memory());
        assert!(!Fault::Error("Validation Error".into()).is_out_of_memory());
        assert!(!Fault::Error("Validation Error: buffer allocation would run out of memory".into()).is_out_of_memory());
        let h = DeviceHealth::new();
        h.mark(Fault::Error("Validation Error".into()));
        h.mark(Fault::Error(OUT_OF_MEMORY.into()));
        assert!(h.fault().is_some_and(|f| f.is_out_of_memory()));
        h.mark(Fault::Lost("reset".into()));
        h.mark(Fault::Error(OUT_OF_MEMORY.into()));
        assert!(h.fault().is_some_and(|f| f.is_lost()));
        assert!(!Fault::Lost("out of memory".into()).is_out_of_memory());
    }

    #[test]
    fn out_of_memory_keeps_polls_running_until_cleared() {
        let h = DeviceHealth::new();
        h.mark(Fault::Error(OUT_OF_MEMORY.into()));
        assert!(!h.is_ok());
        assert!(h.pollable(), "polls reclaim the memory that recovery needs");
        let seen = h.out_of_memory_count();
        // one recorded while the caller freed memory is not cleared with the earlier ones
        h.mark(Fault::Error(OUT_OF_MEMORY.into()));
        h.clear_out_of_memory(seen);
        assert!(h.fault().is_some_and(|f| f.is_out_of_memory()));
        h.clear_out_of_memory(h.out_of_memory_count());
        assert!(h.is_ok());
        assert_eq!(h.fault(), None);
        // a count larger than the ones recorded does not hide later errors
        h.clear_out_of_memory(u64::MAX);
        h.mark(Fault::Error(OUT_OF_MEMORY.into()));
        assert!(!h.is_ok());
        h.clear_out_of_memory(h.out_of_memory_count());
        // an earlier error is still there once the out-of-memory error is cleared
        h.mark(Fault::Error("Validation Error".into()));
        assert!(!h.pollable());
        h.mark(Fault::Error(OUT_OF_MEMORY.into()));
        assert!(!h.pollable(), "a later out-of-memory error does not undo the earlier error");
        h.clear_out_of_memory(h.out_of_memory_count());
        assert_eq!(h.fault(), Some(Fault::Error("Validation Error".into())));
        assert!(!h.is_ok() && !h.pollable());
        let lost = DeviceHealth::new();
        lost.mark(Fault::Error(OUT_OF_MEMORY.into()));
        lost.mark(Fault::Lost("reset".into()));
        assert!(!lost.pollable());
    }

    #[test]
    fn marks_from_several_threads_keep_the_loss() {
        let h = DeviceHealth::new();
        let threads: Vec<_> = (0..8)
            .map(|i| {
                let h = h.clone();
                std::thread::spawn(move || h.mark(if i == 5 { Fault::Lost("reset".into()) } else { Fault::Error(format!("e{i}")) }))
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        assert_eq!(h.fault(), Some(Fault::Lost("reset".into())));
    }

    /// An error and the errors that caused it, outermost first, as wgpu-core chains them.
    #[derive(Debug)]
    struct Cause(&'static str, Option<Box<Cause>>);

    impl fmt::Display for Cause {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str(self.0)
        }
    }

    impl std::error::Error for Cause {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            self.1.as_deref().map(|c| c as _)
        }
    }

    /// A validation error with the causes `chain` and wgpu's description of them.
    fn validation(chain: &[&'static str]) -> wgpu::Error {
        let source = chain.iter().rev().fold(None, |inner, text| Some(Box::new(Cause(text, inner))));
        let mut description = String::from("Validation Error\n\nCaused by:\n");
        for c in chain {
            description.push_str(&format!("    {c}\n"));
        }
        wgpu::Error::Validation { source: source.unwrap_or_else(|| Box::new(Cause("", None))), description }
    }

    #[test]
    fn only_a_lost_device_counts_as_a_loss() {
        let lost = validation(&["In Device::create_buffer", "Parent device is lost"]);
        assert!(fault_of(&lost).is_lost(), "{:?}", fault_of(&lost));
        let destroyed = validation(&["In Queue::submit", "Buffer 'pc' has been destroyed"]);
        // the stored text names what failed, not only wgpu's heading
        assert_eq!(fault_of(&destroyed), Fault::Error("Validation Error: In Queue::submit: Buffer 'pc' has been destroyed".into()));
        assert_eq!(fault_of(&lost), Fault::Lost("Validation Error: In Device::create_buffer: Parent device is lost".into()));
        let texture = validation(&["Texture with 'frame' label has been destroyed"]);
        assert!(!fault_of(&texture).is_lost());
        // a label is the caller's text, not the cause, even on a line of its own
        let labeled = validation(&["In Queue::submit", "Buffer 'device is lost' has been destroyed"]);
        assert_eq!(fault_of(&labeled), Fault::Error("Validation Error: In Queue::submit: Buffer 'device is lost' has been destroyed".into()));
        for label in ["Buffer 'Parent device is lost' has been destroyed", "Buffer 'a\nParent device is lost\nb' has been destroyed"] {
            let labeled = validation(&["In Queue::submit", label]);
            assert!(!fault_of(&labeled).is_lost(), "{label:?}");
        }
    }
}
