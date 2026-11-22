//! Times single edits and iteration on `SharedVec` against a shared `Vec` copied on write.
//!
//! `cargo run --release -p craft-shared-vec --example bench`
//!
//! Each edit sets one element of a list that a snapshot still holds, as an undo history does: the
//! `Vec` is `Arc<Vec<_>>` changed with `Arc::make_mut`, which copies every element, and the
//! `SharedVec` copies the blocks on the edited path. The copied bytes are counted the same way for
//! both: the whole vector for `Vec`, and `bytes_not_in` for `SharedVec`.

use std::hint::black_box;
use std::sync::Arc;
use std::time::{Duration, Instant};

use craft_shared_vec::SharedVec;

/// The element type: one handle, as a document tree's child list holds.
type Handle = Arc<u64>;

const EDITS: usize = 2_000;

fn per_edit(total: Duration) -> f64 {
    total.as_secs_f64() * 1e9 / EDITS as f64
}

fn edits(count: usize) {
    let values: Vec<Handle> = (0..count as u64).map(Arc::new).collect();
    let step = count / EDITS + 1;

    let mut vec = Arc::new(values.clone());
    let mut vec_bytes = 0usize;
    let start = Instant::now();
    for edit in 0..EDITS {
        let snapshot = Arc::clone(&vec);
        let list = Arc::make_mut(&mut vec);
        if let Some(slot) = list.get_mut(edit * step % count) {
            *slot = Arc::new(edit as u64);
        }
        vec_bytes += list.capacity() * size_of::<Handle>();
        black_box(&snapshot);
    }
    let vec_time = start.elapsed();

    let mut shared: SharedVec<Handle> = values.into();
    let mut shared_bytes = 0usize;
    let mut shared_time = Duration::ZERO;
    for edit in 0..EDITS {
        let snapshot = shared.clone();
        let start = Instant::now();
        let _ = shared.set(edit * step % count, Arc::new(edit as u64));
        shared_time += start.elapsed();
        shared_bytes += shared.bytes_not_in(&snapshot);
        black_box(&snapshot);
    }

    println!(
        "{count:>7} elements | Vec {:>10.0} ns/edit {:>9} B/edit | SharedVec {:>7.0} ns/edit {:>6} B/edit",
        per_edit(vec_time),
        vec_bytes / EDITS,
        per_edit(shared_time),
        shared_bytes / EDITS,
    );
}

fn iteration(count: usize) {
    let values: Vec<u64> = (0..count as u64).collect();
    let shared: SharedVec<u64> = values.iter().copied().collect();
    let rounds = (20_000_000 / count).max(1);
    let time = |f: &dyn Fn() -> u64| {
        let start = Instant::now();
        for _ in 0..rounds {
            black_box(f());
        }
        start.elapsed().as_secs_f64() * 1e9 / (rounds * count) as f64
    };
    let vec_ns = time(&|| values.iter().fold(0u64, |sum, value| sum.wrapping_add(*value)));
    let iter_ns = time(&|| shared.iter().fold(0u64, |sum, value| sum.wrapping_add(*value)));
    let rev_ns = time(&|| shared.iter().rev().fold(0u64, |sum, value| sum.wrapping_add(*value)));
    let chunk_ns = time(&|| shared.chunks().flatten().fold(0u64, |sum, value| sum.wrapping_add(*value)));
    // A `for` loop takes one element at a time through `next`.
    let vec_loop_ns = time(&|| {
        let mut sum = 0u64;
        for value in &values {
            sum = sum.wrapping_add(black_box(*value));
        }
        sum
    });
    let loop_ns = time(&|| {
        let mut sum = 0u64;
        for value in &shared {
            sum = sum.wrapping_add(black_box(*value));
        }
        sum
    });
    println!(
        "{count:>7} elements | ns/element: fold Vec {vec_ns:.2}, SharedVec {iter_ns:.2}, reversed {rev_ns:.2}, chunks {chunk_ns:.2} | for loop Vec {vec_loop_ns:.2}, SharedVec {loop_ns:.2}"
    );
}

fn main() {
    println!("One edit beside a kept snapshot, {EDITS} edits:");
    for count in [1_000, 10_000, 100_000] {
        edits(count);
    }
    println!("Iteration:");
    for count in [1_000, 10_000, 100_000] {
        iteration(count);
    }
}
