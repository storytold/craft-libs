//! `HoverMask`: an object's coverage, reduced for the hover tint.

use craft_behavior::kurbo::Rect;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// An object's coverage reduced for the hover tint: at most [`HoverMask::MAX_TEXELS`] texels on its long side,
/// sampled once when the mask is made. Cheap to clone (the texels are shared) and `Send + Sync`, so a lookup job can
/// make it off the UI thread and the app can keep it with its result. The outline uploads its texture once per mask
/// and colour.
#[derive(Clone, Debug)]
pub struct HoverMask {
    /// The mask's extent in document units: the texture's quad.
    bounds: Rect,
    /// Texels across and down.
    size: [usize; 2],
    /// Coverage 0..=255 per texel, row by row.
    coverage: Arc<[u8]>,
    /// Unique per mask made, for the texture store.
    generation: u64,
}

impl PartialEq for HoverMask {
    /// Two masks are equal when they are the same sampling (clones of one mask).
    fn eq(&self, other: &Self) -> bool {
        self.generation == other.generation
    }
}

impl HoverMask {
    /// The longest side of the tint texture: the outline is a preview, not the selection.
    pub const MAX_TEXELS: usize = 768;

    /// The mask over `bounds` (document units): `coverage(x, y)` (0..=1, at a document point) is read at the top-left
    /// corner of each texel's cell, with one texel per document unit until the long side passes
    /// [`MAX_TEXELS`](Self::MAX_TEXELS), then `ceil(long side / 768)` units per texel. Coverage outside 0..=1 is
    /// clamped; one that is not finite reads as 0. Empty or non-finite `bounds` give a 1 × 1 mask with no coverage.
    pub fn sampled(bounds: Rect, coverage: impl Fn(f64, f64) -> f32) -> HoverMask {
        static GENERATIONS: AtomicU64 = AtomicU64::new(0);
        let generation = GENERATIONS.fetch_add(1, Ordering::Relaxed);
        let finite = bounds.x0.is_finite() && bounds.y0.is_finite() && bounds.x1.is_finite() && bounds.y1.is_finite();
        if !finite || bounds.width() <= 0.0 || bounds.height() <= 0.0 {
            return HoverMask { bounds, size: [1, 1], coverage: Arc::from(vec![0u8]), generation };
        }
        let (width, height) = (bounds.width().ceil().max(1.0), bounds.height().ceil().max(1.0));
        let step = (width.max(height) / Self::MAX_TEXELS as f64).ceil().max(1.0);
        let (across, down) = ((width / step).ceil() as usize, (height / step).ceil() as usize);
        let mut texels = Vec::with_capacity(across * down);
        for y in 0..down {
            for x in 0..across {
                let covered = coverage(bounds.x0 + x as f64 * step, bounds.y0 + y as f64 * step);
                let covered = if covered.is_finite() { covered.clamp(0.0, 1.0) } else { 0.0 };
                texels.push((covered * 255.0).round() as u8);
            }
        }
        HoverMask { bounds, size: [across, down], coverage: Arc::from(texels), generation }
    }

    /// The mask's extent in document units.
    pub fn bounds(&self) -> Rect {
        self.bounds
    }

    /// Texels across and down.
    pub fn size(&self) -> [usize; 2] {
        self.size
    }

    /// The coverage of texel `(x, y)`, 0..=1; 0 outside the mask.
    pub fn coverage(&self, x: usize, y: usize) -> f32 {
        let [width, height] = self.size;
        if x >= width || y >= height {
            return 0.0;
        }
        f32::from(self.coverage[y * width + x]) / 255.0
    }

    /// Whether texel `(x, y)` is on the mask's edge: covered at 0.5 or more, and on the texture's border or beside a
    /// texel (left, right, above, below) covered less than 0.5.
    pub fn is_edge(&self, x: usize, y: usize) -> bool {
        let [width, height] = self.size;
        let on = |x: usize, y: usize| self.coverage(x, y) >= 0.5;
        on(x, y) && (x == 0 || y == 0 || x + 1 == width || y + 1 == height || !on(x - 1, y) || !on(x + 1, y) || !on(x, y - 1) || !on(x, y + 1))
    }

    /// The key of the mask's texture in the texture store.
    pub(super) fn generation(&self) -> u64 {
        self.generation
    }
}
