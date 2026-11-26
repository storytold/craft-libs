//! Tests of the outline tracer. Each uses a small `MaskSource` over a `Vec<f32>`
//! written in the test.

use super::*;
use craft_ui::testing::trial;

/// A mask over a row-major `Vec<f32>` at the origin.
struct Plane {
    size: [u32; 2],
    data: Vec<f32>,
}

impl Plane {
    fn blank(w: u32, h: u32) -> Self {
        Self { size: [w, h], data: vec![0.0; (w * h) as usize] }
    }

    fn fill(&mut self, x0: u32, y0: u32, x1: u32, y1: u32) {
        for y in y0..y1 {
            for x in x0..x1 {
                self.data[(y * self.size[0] + x) as usize] = 1.0;
            }
        }
    }
}

impl MaskSource for Plane {
    fn size(&self) -> [u32; 2] {
        self.size
    }

    fn revision(&self) -> u64 {
        0
    }

    fn content_bounds(&self) -> MaskRect {
        MaskRect::of_size(self.size)
    }

    fn read_row(&self, y: i32, x0: i32, step: u32, out: &mut [f32]) {
        for (i, v) in out.iter_mut().enumerate() {
            let x = x0 + i32::try_from(i).unwrap() * i32::try_from(step).unwrap();
            let inside = x >= 0 && y >= 0 && (x as u32) < self.size[0] && (y as u32) < self.size[1];
            *v = if inside { self.data[y as usize * self.size[0] as usize + x as usize] } else { 0.0 };
        }
    }
}

// Segments of a known 4 × 3 block mask: two merged horizontal runs, two merged vertical runs, one
// padding cell found at a mask touching `bounds`.
#[trial]
fn trace_merges_runs_and_pads_one_cell() {
    let mut mask = Plane::blank(4, 3);
    mask.fill(0, 0, 4, 3);
    let mut segments = outline(&mask, MaskRect::of_size([4, 3]), 1);
    segments.sort_unstable();
    let mut expected = vec![([0, 0], [4, 0]), ([0, 3], [4, 3]), ([0, 0], [0, 3]), ([4, 0], [4, 3])];
    expected.sort_unstable();
    assert_eq!(segments, expected);
    assert!(outline(&mask, MaskRect::default(), 1).is_empty());
}

// A one-pixel-wide line traced at step 8 still gives segments (the coarse-cell edge sample), and
// every segment end stays inside `bounds`.
#[trial]
fn trace_coarse_cells_keep_one_pixel_selections() {
    let mut mask = Plane::blank(64, 64);
    mask.fill(1, 0, 2, 64);
    let bounds = MaskRect::new(1, 0, 2, 64);
    let segments = outline(&mask, bounds, 8);
    assert!(!segments.is_empty());
    for (a, b) in segments {
        for p in [a, b] {
            assert!(p[0] >= bounds.x0 && p[0] <= bounds.x1 && p[1] >= bounds.y0 && p[1] <= bounds.y1);
        }
    }
}

// A 6 × 6 block with a 2 × 2 hole traces the outer square and the hole's own square; bounds of
// zero or negative size trace nothing.
#[trial]
fn a_hole_traces_its_own_loop_and_a_hostile_view_traces_nothing() {
    let mut mask = Plane::blank(6, 6);
    mask.fill(0, 0, 6, 6);
    for y in 2..4 {
        for x in 2..4 {
            mask.data[y * 6 + x] = 0.0;
        }
    }
    let mut segments = outline(&mask, MaskRect::of_size([6, 6]), 1);
    segments.sort_unstable();
    let mut expected = vec![
        ([0, 0], [6, 0]),
        ([0, 6], [6, 6]),
        ([0, 0], [0, 6]),
        ([6, 0], [6, 6]),
        ([2, 2], [4, 2]),
        ([2, 4], [4, 4]),
        ([2, 2], [2, 4]),
        ([4, 2], [4, 4]),
    ];
    expected.sort_unstable();
    assert_eq!(segments, expected);
    for hostile in
        [MaskRect::new(3, 0, 3, 6), MaskRect::new(0, 3, 6, 3), MaskRect::new(6, 6, 0, 0), MaskRect::new(i32::MAX, i32::MAX, i32::MIN, i32::MIN)]
    {
        assert!(outline(&mask, hostile, 1).is_empty(), "{hostile:?} traces nothing");
    }
}

// `edges_of_grid` gives PhotoCraft's Content-Aware Fill unit edges for an L-shaped grid, and
// `None` past the limit.
