//! Admission and reusable buffer storage shared by all parallel pictures of a decoder.

use crate::hevc::decode::slicedec::{WinBufs, WindowSizes};
use std::any::Any;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

const DEFAULT_BUDGET: u64 = crate::budget::DECODE_MEMORY_BUDGET / 8;

struct CachedBuffers {
    bytes: u64,
    buffers: Box<dyn Any + Send>,
}

struct State {
    limit: u64,
    reserved: u64,
    retained: u64,
    cache: Vec<CachedBuffers>,
}

impl State {
    fn trim_cache(&mut self, allowed: u64) {
        while self.retained > allowed {
            let Some(cached) = self.cache.pop() else { break };
            self.retained -= cached.bytes;
        }
    }
}

pub(super) struct ScratchBudget {
    max_cached: usize,
    state: Mutex<State>,
}

impl ScratchBudget {
    pub(super) fn new(max_cached: usize) -> Self {
        Self { max_cached, state: Mutex::new(State { limit: DEFAULT_BUDGET, reserved: 0, retained: 0, cache: Vec::new() }) }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(super) fn set_limit(&self, limit: u64) {
        let mut state = self.lock();
        state.limit = limit;
        let allowed = limit.saturating_sub(state.reserved);
        state.trim_cache(allowed);
    }

    pub(super) fn reserve(self: &Arc<Self>, bytes: usize) -> Option<Reservation> {
        let mut state = self.lock();
        let reserved = state.reserved.checked_add(bytes as u64)?;
        let allowed = state.limit.checked_sub(reserved)?;
        state.trim_cache(allowed);
        state.reserved = reserved;
        Some(Reservation { budget: self.clone(), bytes })
    }

    pub(super) fn take_bufs<S: Send + 'static>(&self, sizes: WindowSizes) -> Option<WinBufs<S>> {
        let mut state = self.lock();
        let i = state.cache.iter().rposition(|c| c.buffers.downcast_ref::<WinBufs<S>>().is_some_and(|b| b.matches(sizes)))?;
        let cached = state.cache.swap_remove(i);
        state.retained -= cached.bytes;
        cached.buffers.downcast().ok().map(|buffers| *buffers)
    }

    pub(super) fn clear_cache(&self) {
        let mut state = self.lock();
        state.cache = Vec::new();
        state.retained = 0;
    }
}

/// Owns the budget of a picture until its workers are all done with their windows.
pub(super) struct Reservation {
    budget: Arc<ScratchBudget>,
    bytes: usize,
}

impl Reservation {
    /// Transfer the backing allocations from this picture's reservation to the idle cache.
    pub(super) fn put_bufs<S: Send + 'static>(&mut self, buffers: WinBufs<S>) {
        let Some(storage) = buffers.allocated_bytes() else { return };
        let Some(bytes) = storage.checked_add(size_of::<WinBufs<S>>() + size_of::<CachedBuffers>()) else { return };
        let bytes = bytes as u64;
        let mut state = self.budget.lock();
        let Some(remaining) = self.bytes.checked_sub(storage) else { return };
        let Some(retained) = state.retained.checked_add(bytes) else { return };
        let reserved = state.reserved - storage as u64;
        if state.cache.len() < self.budget.max_cached && reserved.checked_add(retained).is_some_and(|total| total <= state.limit) {
            state.cache.push(CachedBuffers { bytes, buffers: Box::new(buffers) });
            state.retained = retained;
            state.reserved = reserved;
            self.bytes = remaining;
        }
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        self.budget.lock().reserved -= self.bytes as u64;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reservation_and_idle_buffers_share_the_limit() {
        let sizes = WindowSizes { planes: [256, 128, 128], blocks: 8 };
        let storage = sizes.bytes::<u8>().unwrap();
        let cached = storage + size_of::<WinBufs<u8>>() + size_of::<CachedBuffers>();
        let budget = Arc::new(ScratchBudget::new(8));
        budget.set_limit((storage + cached) as u64);
        let first = budget.reserve(storage).unwrap();
        let mut second = budget.reserve(storage).unwrap();
        assert!(budget.reserve(storage).is_none());

        second.put_bufs(WinBufs::<u8>::with_sizes(sizes));
        assert_eq!(budget.lock().retained, cached as u64);
        assert_eq!(budget.lock().reserved, storage as u64);
        drop(second);

        let third = budget.reserve(storage).unwrap();
        assert!(budget.take_bufs::<u8>(sizes).is_none());
        assert!(budget.reserve(storage).is_none());
        drop((first, third));
        assert_eq!(budget.lock().reserved, 0);
    }
}
