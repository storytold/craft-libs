//! Counting-allocator verification that `craft-blitter` performs zero heap allocations.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use craft_blitter::{AlphaMode, BlendMode, CompositeOp, blit_rgba8_opts, blit_rgba16_opts, blit_rgba32f_opts};

struct CountingAlloc;

static COUNTING: AtomicBool = AtomicBool::new(false);
static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

// SAFETY: `CountingAlloc` delegates every allocation and deallocation directly to `std::alloc::System`
// with identical `Layout` and pointer arguments, only incrementing an atomic counter when enabled.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if COUNTING.load(Ordering::Relaxed) {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        // SAFETY: Forwarded unchanged to `System::alloc`.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if COUNTING.load(Ordering::Relaxed) {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        // SAFETY: Forwarded unchanged to `System::alloc_zeroed`.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if COUNTING.load(Ordering::Relaxed) {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        // SAFETY: Forwarded unchanged to `System::realloc`.
        unsafe { System.realloc(ptr, layout, new_size) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: Forwarded unchanged to `System::dealloc`.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

fn count_allocations<F: FnOnce()>(f: F) -> usize {
    ALLOCATIONS.store(0, Ordering::SeqCst);
    COUNTING.store(true, Ordering::SeqCst);
    f();
    COUNTING.store(false, Ordering::SeqCst);
    ALLOCATIONS.load(Ordering::SeqCst)
}

#[test]
fn zero_heap_allocations_across_serial_and_warm_parallel_blits() {
    // 1. Sanity check: verify the counting allocator catches a real heap allocation.
    let probe = count_allocations(|| {
        let v: Vec<u8> = std::hint::black_box(Vec::with_capacity(64));
        std::hint::black_box(&v);
    });
    assert!(probe >= 1, "counting allocator failed to record Vec allocation");

    // 2. Cold serial blitting (< PAR_THRESHOLD = 4096 pixels): zero allocations across all 32 modes,
    // both alpha conventions, and both Porter-Duff operators.
    let mut dst8 = [128u8; 256 * 4];
    let src8 = [200u8; 256 * 4];
    let mut dst16 = [32768u16; 256 * 4];
    let src16 = [50000u16; 256 * 4];
    let mut dst32 = [[0.5f32, 0.4, 0.3, 0.9]; 256];
    let src32 = [[0.8f32, 0.2, 0.6, 0.7]; 256];

    let serial_allocs = count_allocations(|| {
        for mode in BlendMode::ALL {
            for alpha in [AlphaMode::Straight, AlphaMode::Premultiplied] {
                for op in [CompositeOp::SourceOver, CompositeOp::SourceAtop] {
                    blit_rgba8_opts(&mut dst8, &src8, mode, 0.75, alpha, op);
                    blit_rgba16_opts(&mut dst16, &src16, mode, 0.75, alpha, op);
                    blit_rgba32f_opts(&mut dst32, &src32, mode, 0.75, false, alpha, op);
                    blit_rgba32f_opts(&mut dst32, &src32, mode, 0.75, true, alpha, op);
                }
            }
        }
    });
    assert_eq!(serial_allocs, 0, "serial blitting allocated {serial_allocs} times");

    // 3. Parallel blitting (>= PAR_THRESHOLD = 4096 pixels): warm Rayon's global worker threads,
    // then verify steady-state parallel blitting performs zero heap allocations.
    let mut big_dst8 = vec![100u8; 65536 * 4];
    let big_src8 = vec![180u8; 65536 * 4];
    let mut big_dst16 = vec![25000u16; 65536 * 4];
    let big_src16 = vec![45000u16; 65536 * 4];
    let mut big_dst32 = vec![[0.3f32, 0.5, 0.7, 1.0]; 65536];
    let big_src32 = vec![[0.7f32, 0.2, 0.4, 0.8]; 65536];

    // Warm Rayon's worker threads and their thread-local epoch handles before counting.
    for _ in 0..8 {
        blit_rgba8_opts(&mut big_dst8, &big_src8, BlendMode::Normal, 0.5, AlphaMode::Straight, CompositeOp::SourceOver);
    }

    let par_allocs = count_allocations(|| {
        for mode in [BlendMode::Normal, BlendMode::Multiply, BlendMode::SoftLight, BlendMode::Hue] {
            blit_rgba8_opts(&mut big_dst8, &big_src8, mode, 0.75, AlphaMode::Straight, CompositeOp::SourceOver);
            blit_rgba16_opts(&mut big_dst16, &big_src16, mode, 0.75, AlphaMode::Straight, CompositeOp::SourceOver);
            blit_rgba32f_opts(&mut big_dst32, &big_src32, mode, 0.75, false, AlphaMode::Straight, CompositeOp::SourceOver);
        }
    });
    assert_eq!(par_allocs, 0, "warm parallel blitting allocated {par_allocs} times");
}
