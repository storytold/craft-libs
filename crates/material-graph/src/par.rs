// SPDX-License-Identifier: MIT OR Apache-2.0
//! Data-parallel helpers for the hot paths (per-pixel passes, `crate::tex::par_map`, and the paint stack's stamping
//! and tile compositing, which uses them from here), on rayon.
//!
//! - Work runs on the pool of the calling thread's [`crate::context::EvalContext`] when it names one, else on the pool
//!   the caller is in (rayon's global pool outside any). The caller's context is current on every thread that runs a
//!   piece, so cancel checks and profile spans inside the pieces see it. Work that is a single run is not handed to a
//!   pool at all: the calling thread does it.
//! - Items are split at fixed indices (runs of a given length), never by thread count, and each piece is handed to
//!   whichever thread is free, so uneven items (tiles with many dabs, tiles with none) balance.
//!
//! Results are deterministic: each item is visited exactly once with its index, and what a visit does may depend
//! only on its item and index, never on which thread runs it or when.
//!
//! Unlike `crate::tex::par_map` these never stop early for a cancelled evaluation, so callers always get complete
//! results.

use rayon::prelude::*;

/// Threads besides the caller that a call may use: the pool's threads less one.
pub fn helpers() -> usize {
    crate::context::with_current(|ec| match ec.and_then(|ec| ec.pool.as_ref()) {
        Some(pool) => pool.current_num_threads(),
        None => rayon::current_num_threads(),
    })
    .saturating_sub(1)
}

/// Calls `f(index, item)` for every item, spread over the available threads.
pub fn for_each_mut<T: Send, F: Fn(usize, &mut T) + Sync>(items: &mut [T], f: F) {
    let run = items.len().div_ceil((helpers() + 1) * 4).max(1);
    for_each_run_mut(items, run, |first, chunk| {
        for (i, it) in chunk.iter_mut().enumerate() {
            f(first + i, it);
        }
    });
}

/// Calls `f(first_index, run)` for consecutive runs of at most `run` items covering `items` (split at multiples of
/// `run`), spread over the available threads.
pub fn for_each_run_mut<T: Send, F: Fn(usize, &mut [T]) + Sync>(items: &mut [T], run: usize, f: F) {
    let run = run.max(1);
    if items.len() <= run {
        // one run: no other thread to wake
        if !items.is_empty() {
            f(0, items);
        }
        return;
    }
    let ec = crate::context::current();
    let ec = ec.as_ref();
    crate::context::in_pool(ec, || {
        items.par_chunks_mut(run).enumerate().for_each(|(ri, chunk)| crate::context::scoped_opt(ec, || f(ri * run, chunk)));
    });
}

#[cfg(test)]
mod tests {
    #[test]
    fn visits_every_item_once_with_its_index() {
        for n in [0, 1, 2, 7, 1000] {
            let mut v = vec![0usize; n];
            super::for_each_mut(&mut v, |i, x| *x += i + 1);
            assert!(v.iter().enumerate().all(|(i, &x)| x == i + 1), "{n} items");
        }
    }

    #[test]
    fn runs_cover_the_items_in_order_and_respect_their_length() {
        for (n, run) in [(10, 3), (10, 10), (10, 64), (4097, 4096), (5, 0)] {
            let mut v = vec![usize::MAX; n];
            super::for_each_run_mut(&mut v, run, |first, chunk| {
                assert!(chunk.len() <= run.max(1));
                chunk.iter_mut().enumerate().for_each(|(i, x)| *x = first + i);
            });
            assert!(v.iter().enumerate().all(|(i, &x)| x == i), "{n} items in runs of {run}");
        }
    }

    #[test]
    fn pieces_see_the_callers_context() {
        use crate::context::{Cancel, EvalContext, cancelled, scoped};
        let ec = EvalContext::with_cancel(Cancel::new());
        ec.cancel.cancel();
        let mut v = vec![false; 64];
        scoped(&ec, || super::for_each_run_mut(&mut v, 1, |_, x| x[0] = cancelled()));
        assert!(v.iter().all(|x| *x), "every piece must see the cancelled context");
        let pool = std::sync::Arc::new(rayon::ThreadPoolBuilder::new().num_threads(3).build().unwrap());
        let ec = EvalContext { pool: Some(pool), ..EvalContext::default() };
        let mut threads = vec![0usize; 64];
        scoped(&ec, || super::for_each_run_mut(&mut threads, 1, |_, x| x[0] = rayon::current_num_threads()));
        assert!(threads.iter().all(|n| *n == 3), "pieces run on the context's pool");
    }
}
