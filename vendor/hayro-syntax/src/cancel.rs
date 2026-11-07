//! Cancellation scoped to one PDF open, with no thread-local state.

use crate::sync::{Arc, Mutex, MutexExt};
use core::fmt;

/// An owned cancellation callback, called synchronously on the opening thread.
///
/// Capture shared state, such as an `Arc<AtomicBool>`, to request cancellation from another
/// thread. A request is latched separately for each open. Parsed PDFs do not retain or call
/// the callback after that open returns.
#[derive(Clone)]
pub struct Cancellation(Arc<dyn Fn() -> bool + Send + Sync>);

impl Cancellation {
    /// Own a callback that returns true when the current operation should stop.
    pub fn new(check: impl Fn() -> bool + Send + Sync + 'static) -> Self {
        Self(Arc::new(check))
    }

    /// Poll the callback. An opening operation latches a true result in its own control.
    pub fn requested(&self) -> bool {
        (self.0)()
    }
}

impl fmt::Debug for Cancellation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Cancellation").finish_non_exhaustive()
    }
}

#[derive(Debug)]
struct State {
    callback: Option<Cancellation>,
    stopped: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct Control(Arc<Inner>);

#[derive(Debug)]
struct Inner {
    state: Mutex<State>,
    #[cfg(feature = "std")]
    active: core::sync::atomic::AtomicBool,
    #[cfg(not(feature = "std"))]
    active: core::cell::Cell<bool>,
}

impl Control {
    pub(crate) fn new(callback: &Cancellation) -> Self {
        Self(Arc::new(Inner {
            state: Mutex::new(State { callback: Some(callback.clone()), stopped: false }),
            #[cfg(feature = "std")]
            active: core::sync::atomic::AtomicBool::new(true),
            #[cfg(not(feature = "std"))]
            active: core::cell::Cell::new(true),
        }))
    }

    pub(crate) fn active(&self) -> bool {
        #[cfg(feature = "std")]
        {
            self.0.active.load(core::sync::atomic::Ordering::Relaxed)
        }
        #[cfg(not(feature = "std"))]
        {
            self.0.active.get()
        }
    }

    pub(crate) fn check(&self) -> Option<()> {
        if !self.active() {
            return Some(());
        }
        // Never invoke caller code under the cancellation-state lock.
        let callback = {
            let state = self.0.state.get();
            if state.stopped {
                return None;
            }
            state.callback.clone()
        };
        if callback.is_some_and(|c| c.requested()) {
            self.0.state.get().stopped = true;
            None
        } else {
            Some(())
        }
    }

    fn finish(&self) {
        let callback = self.0.state.get().callback.take();
        #[cfg(feature = "std")]
        self.0.active.store(false, core::sync::atomic::Ordering::Relaxed);
        #[cfg(not(feature = "std"))]
        self.0.active.set(false);
        drop(callback);
    }
}

pub(crate) fn check(control: Option<&Control>) -> Option<()> {
    control.map_or(Some(()), Control::check)
}

pub(crate) struct Checkpoints<'a> {
    control: Option<&'a Control>,
    next: usize,
}

impl<'a> Checkpoints<'a> {
    pub(crate) fn new(control: Option<&'a Control>) -> Self {
        Self { control, next: 0 }
    }

    pub(crate) fn at(&mut self, work: usize) -> Option<()> {
        if self.control.is_some() && work >= self.next {
            check(self.control)?;
            self.next = work.saturating_add(64 * 1024);
        }
        Some(())
    }
}

/// Remove callback captures on success, failure and unwinding. The xref may keep an inert
/// control after a successful open, so deferred reads do not poll a completed operation.
pub(crate) struct Scope(pub(crate) Control);

impl Drop for Scope {
    fn drop(&mut self) {
        self.0.finish();
    }
}

#[cfg(feature = "unsafe")]
pub(crate) struct Input<'a, R> {
    inner: R,
    control: Option<&'a Control>,
    left: usize,
}

#[cfg(feature = "unsafe")]
impl<'a, R> Input<'a, R> {
    pub(crate) fn new(inner: R, control: Option<&'a Control>) -> Self {
        Self { inner, control, left: 0 }
    }
}

#[cfg(feature = "unsafe")]
impl<R: std::io::Read> std::io::Read for Input<'_, R> {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        if self.control.is_none() {
            return self.inner.read(bytes);
        }
        if bytes.is_empty() {
            return Ok(0);
        }
        if self.left == 0 {
            check(self.control).ok_or_else(|| std::io::Error::other("PDF opening canceled"))?;
            self.left = 64 * 1024;
        }
        let len = self.left.min(bytes.len());
        let n = self.inner.read(&mut bytes[..len])?;
        self.left -= n;
        Ok(n)
    }
}

#[cfg(test)]
pub(crate) fn stop_at(n: usize) -> (Control, alloc::sync::Arc<core::sync::atomic::AtomicUsize>) {
    use core::sync::atomic::{AtomicUsize, Ordering};
    let polls = alloc::sync::Arc::new(AtomicUsize::new(0));
    let counter = polls.clone();
    let cancel = Cancellation::new(move || counter.fetch_add(1, Ordering::Relaxed) + 1 == n);
    (Control::new(&cancel), polls)
}
