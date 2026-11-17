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
    fn cancellation_stops() {
        let img = vec![0.5f32; 64 * 64];
        let hole: Vec<bool> = (0..64 * 64usize).map(|i| (i % 64).abs_diff(32) < 4 && (i / 64).abs_diff(32) < 4).collect();
        assert_eq!(complete_with(64, 64, 1, &img, &hole, &CompleteParams::default(), &|| true), Err(Cancelled));
    }
}
