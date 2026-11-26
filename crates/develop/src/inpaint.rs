//! Content-aware hole filling (inpainting) for the Remove tool.
//!
//! * [`complete`]: multi-scale patch-based completion (Wexler, Shechtman, Irani 2007) with the
//!   nearest-neighbour field found by PatchMatch (Barnes et al. 2009). See [`patchmatch`].
//! * [`membrane_fill`]: smooth harmonic fill, used to seed the coarsest level and as a fallback.
//!
//! Buffers are interleaved `w × h × ch` floats in any colour model plus a `w × h` hole mask.
//! Everything is deterministic (fixed-seed RNG, results independent of the thread count) and never
//! panics on mismatched sizes: such calls return `None` or the input unchanged.
//!
//! Sources: C. Barnes, E. Shechtman, A. Finkelstein, D. B. Goldman, *PatchMatch: A Randomized
//! Correspondence Algorithm for Structural Image Editing*, SIGGRAPH 2009; Y. Wexler, E. Shechtman,
//! M. Irani, *Space-Time Completion of Video*, PAMI 2007.
//!
//! Ported from PhotoCraft (`crates/algo/src/inpaint.rs`, `poisson.rs`, MIT OR Apache-2.0, same
//! organisation; see `NOTICE`).

pub mod membrane;
pub mod patchmatch;

pub use membrane::membrane_fill;
pub use patchmatch::{CompleteParams, complete, complete_with};

/// A cancelled [`complete_with`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cancelled;

impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("cancelled")
    }
}

impl std::error::Error for Cancelled {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finite_extreme_membrane_boundaries_stay_finite() {
        let img = [f32::MAX, 0.0, f32::MAX];
        let out = membrane_fill(3, 1, 1, &img, &[false, true, false]);
        assert_eq!(out, vec![f32::MAX; 3]);
        assert_eq!(membrane_fill(3, 1, 1, &[-f32::MAX, 0.0, f32::MAX], &[false, true, false]), vec![-f32::MAX, 0.0, f32::MAX]);
        // Also exercise the multigrid restriction and prolongation at extreme scale.
        let hole: Vec<bool> = (0..256).map(|i| i % 16 > 0 && i % 16 < 15 && i / 16 > 0 && i / 16 < 15).collect();
        assert_eq!(membrane_fill(16, 16, 1, &[f32::MAX; 256], &hole), vec![f32::MAX; 256]);
        let img: Vec<f32> = (0..256).map(|i| if i % 16 < 8 { -f32::MAX } else { f32::MAX }).collect();
        let out = membrane_fill(16, 16, 1, &img, &hole);
        for ((&actual, &original), &unknown) in out.iter().zip(&img).zip(&hole) {
            assert!(actual.is_finite() && (-f32::MAX..=f32::MAX).contains(&actual));
            if !unknown {
                assert_eq!(actual.to_bits(), original.to_bits());
            }
        }
    }

    #[test]
    fn cancellation_precedes_an_oversized_patch_result() {
        let mut hole = vec![false; 32 * 32];
        hole[16 * 32 + 16] = true;
        let p = CompleteParams { patch_radius: 16, ..CompleteParams::default() };
        assert_eq!(complete_with(32, 32, 1, &[0.5; 32 * 32], &hole, &p, &|| true), Err(Cancelled));
    }

    #[test]
    fn hostile_sizes_do_not_panic() {
        assert_eq!(complete(4, 4, 3, &[0.0; 5], &[true; 16], &CompleteParams::default()), None);
        assert_eq!(complete(4, 4, 0, &[], &[true; 16], &CompleteParams::default()), None);
        assert_eq!(complete(usize::MAX, 2, 3, &[0.0; 6], &[true; 2], &CompleteParams::default()), None);
        assert_eq!(membrane_fill(4, 4, 1, &[0.5; 3], &[true; 16]), vec![0.5; 3]);
        // A hole covering everything: nothing to copy from.
        assert_eq!(complete(16, 16, 1, &[0.5; 256], &[true; 256], &CompleteParams::default()), None);
        // A tiny image smaller than a patch.
        let _ = complete(2, 2, 1, &[0.5; 4], &[true, false, false, false], &CompleteParams::default());
    }

    #[test]
    fn a_patch_radius_no_image_could_hold_finds_nothing_to_copy() {
        let mut hole = vec![false; 32 * 32];
        hole[16 * 32 + 16] = true;
        for patch_radius in [16, usize::MAX / 2, usize::MAX] {
            let p = CompleteParams { patch_radius, ..CompleteParams::default() };
            assert_eq!(complete(32, 32, 1, &[0.5; 32 * 32], &hole, &p), None, "radius {patch_radius}");
        }
        // Cancelled is still cancelled, not "nothing to copy from".
        let p = CompleteParams { patch_radius: 16, ..CompleteParams::default() };
        assert_eq!(complete_with(32, 32, 1, &[0.5; 32 * 32], &hole, &p, &|| true), Err(Cancelled));
        // A radius the image has room for still fills.
        let p = CompleteParams { patch_radius: 5, ..CompleteParams::default() };
        assert!(complete(32, 32, 1, &[0.5; 32 * 32], &hole, &p).is_some());
    }

    #[test]
    fn an_image_with_no_pixels_is_returned_without_walking_its_rows() {
        let p = CompleteParams::default();
        assert_eq!(complete_with(0, usize::MAX, 1, &[], &[], &p, &|| false), Ok(Some(Vec::new())));
        assert_eq!(complete_with(usize::MAX, 0, 3, &[], &[], &p, &|| false), Ok(Some(Vec::new())));
    }

    /// Whichever poll says "cancelled", and only that one, the call ends in `Err`: also when it was a
    /// worker's poll, which can stop the worker but not return.
    #[test]
    fn one_cancelled_poll_anywhere_cancels_the_call() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let (w, h) = (48usize, 48usize);
        let img: Vec<f32> = (0..w * h).map(|i| ((i % w) as f32 * 0.37).sin() * 0.5 + 0.5).collect();
        let hole: Vec<bool> = (0..w * h).map(|i| (i % w).abs_diff(24) < 5 && (i / w).abs_diff(24) < 5).collect();
        let p = CompleteParams::default();
        let polls = AtomicUsize::new(0);
        let count = || {
            polls.fetch_add(1, Ordering::Relaxed);
            false
        };
        assert!(matches!(complete_with(w, h, 1, &img, &hole, &p, &count), Ok(Some(_))));
        let total = polls.load(Ordering::Relaxed);
        assert!(total > 8, "the scene polls in the workers too: {total}");
        // Cancelling at the first, a quarter, the middle, three quarters and the last polls, workers' polls included.
        let mut picks = vec![0, 1, 2, total / 4, total / 2, 3 * total / 4, total - 3, total - 2, total - 1];
        picks.dedup();
        for nth in picks {
            let polls = AtomicUsize::new(0);
            let once = || polls.fetch_add(1, Ordering::Relaxed) == nth;
            assert_eq!(complete_with(w, h, 1, &img, &hole, &p, &once), Err(Cancelled), "poll {nth} of {total}");
        }
    }

    #[test]
    fn cancellation_stops() {
        let img = vec![0.5f32; 64 * 64];
        let hole: Vec<bool> = (0..64 * 64usize).map(|i| (i % 64).abs_diff(32) < 4 && (i / 64).abs_diff(32) < 4).collect();
        assert_eq!(complete_with(64, 64, 1, &img, &hole, &CompleteParams::default(), &|| true), Err(Cancelled));
    }
}
