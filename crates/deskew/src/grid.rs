//! The image reduced to an ink-level grid, with any dark frame touching the edge removed.

use crate::image::Image;

/// The longest side of the grid. Larger images are subsampled; text lines survive it.
const GRID_SIDE: u32 = 1600;

/// Grid levels at or above this are ink.
pub(crate) const INK: u8 = 128;

/// Ink levels, 0 (paper) to 255 (full ink), on a regular grid over the image.
pub(crate) struct Grid {
    pub width: usize,
    pub height: usize,
    pub levels: Vec<u8>,
}

impl Grid {
    /// Sample `image` every `step` pixels, `step` chosen so the longer side is at most
    /// [`GRID_SIDE`], then clear the frame (see [`Grid::clear_frame`]).
    pub(crate) fn sample(image: &Image<'_>, background: &[u32]) -> Self {
        let step = image.width().max(image.height()).div_ceil(GRID_SIDE).max(1);
        let width = image.width().div_ceil(step) as usize;
        let height = image.height().div_ceil(step) as usize;
        let mut levels = Vec::with_capacity(width.saturating_mul(height));
        for y in (0..image.height()).step_by(step as usize) {
            for x in (0..image.width()).step_by(step as usize) {
                levels.push(image.ink_level(x, y, background));
            }
        }
        let mut grid = Self { width, height, levels };
        grid.clear_frame();
        grid
    }

    pub(crate) fn level(&self, x: usize, y: usize) -> u8 {
        self.levels.get(y.saturating_mul(self.width).saturating_add(x)).copied().unwrap_or(0)
    }

    /// Clear every ink pixel 8-connected to ink within 1% of the grid's longer side of an
    /// edge: a scanner's dark border, a punched hole or a shadow along the edge. Such a frame is
    /// square to the image, not to the content, and would pull the measured angle towards zero.
    fn clear_frame(&mut self) {
        let (w, h) = (self.width, self.height);
        let reach = (w.max(h) / 100).max(1);
        for y in 0..h {
            let edge_row = y < reach || y.saturating_add(reach) >= h;
            for x in 0..w {
                if edge_row || x < reach || x.saturating_add(reach) >= w {
                    self.clear_component(x, y);
                }
            }
        }
    }

    /// Clear the 8-connected ink component at `(x, y)`, a horizontal run at a time: each run
    /// is cleared whole, and each ink run touching it in the rows above and below is queued
    /// once. Every queued run is cleared before it is queued again, so the queue never holds
    /// more entries than the grid has pixels.
    fn clear_component(&mut self, x: usize, y: usize) {
        let mut runs = vec![(x, y)];
        while let Some((seed, row)) = runs.pop() {
            if !self.is_ink(seed, row) {
                continue;
            }
            let (mut first, mut last) = (seed, seed);
            while first > 0 && self.is_ink(first - 1, row) {
                first -= 1;
            }
            while self.is_ink(last + 1, row) {
                last += 1;
            }
            for x in first..=last {
                self.take(x, row);
            }
            let reach = first.saturating_sub(1)..=last.saturating_add(1);
            for next in [row.checked_sub(1), row.checked_add(1).filter(|&r| r < self.height)].into_iter().flatten() {
                let mut x = *reach.start();
                while x <= *reach.end() {
                    if self.is_ink(x, next) {
                        runs.push((x, next));
                        while self.is_ink(x, next) {
                            x += 1;
                        }
                    }
                    x += 1;
                }
            }
        }
    }

    fn is_ink(&self, x: usize, y: usize) -> bool {
        x < self.width && y < self.height && self.level(x, y) >= INK
    }

    /// Clear `(x, y)` if it is ink, saying whether it was.
    fn take(&mut self, x: usize, y: usize) -> bool {
        let at = y.saturating_mul(self.width).saturating_add(x);
        match self.levels.get_mut(at) {
            Some(level) if *level >= INK => {
                *level = 0;
                true
            }
            _ => false,
        }
    }
}
