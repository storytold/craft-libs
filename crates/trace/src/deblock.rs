//! The 8 x 8 block edges of a block-coded image (a JPEG, or a copy of one saved in another format)
//! smoothed away before it is traced.
//!
//! A JPEG codes each 8 x 8 block on its own and rounds each block's brightness in coarse steps, which
//! leaves a small step at every block edge. Where an area's tone changes slowly, as a dark ground does,
//! those steps decide which of two palette colors a whole block gets, and the trace follows the block
//! edges as a staircase. The block grid is found in the image itself: the mean step between
//! neighboring pixels peaks at one position in every eight, across and down, at every block edge and
//! not only at every other one (a pattern of 16 px squares has that too). Where it does, the step
//! at each block edge is spread evenly over the eight pixels around it when it is small and those
//! pixels hold no edge. Larger steps are left as they are. How large a step is smoothed, and whether
//! each row keeps part of its own step, depends on the trace's mode and number of colors (see
//! [`Strength`]). The deblocking filters of ITU-T H.263
//! Annex J and ISO/IEC 14496-2 (MPEG-4 Part 2) Annex F.3.1 treat block edges the same way: small steps
//! between blocks become ramps, and large ones are kept.

use std::sync::Mutex;

use crate::{Raster, parallel_map, quantize};

/// The block size of a JPEG.
const BLOCK: usize = 8;
/// Steps between neighboring pixels of this many levels or more are edges, left out when the grid is
/// measured. No block-edge step this large is smoothed.
const EDGE: f64 = 16.0;
/// The mean step at the block edges must be this many times the mean step at the other positions,
/// across and down, for the image to count as block-coded.
const PEAK: f64 = 1.5;
/// The smallest limit on the block-edge steps that are smoothed (levels, after the image's own slope
/// is taken out); the limit is under `EDGE` too.
const LEAST: f64 = 2.5;
/// Fixed-point scale of the working copy: sixteenths of a level.
const SCALE: f64 = 16.0;

/// How the block edges are smoothed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Strength {
    /// The largest block-edge step that is smoothed: this many times the extra step the block edges
    /// show on average, at least `LEAST` and under `EDGE`.
    reach: f64,
    /// A block edge with a step of this many levels or more between neighbors in the pixels around
    /// it is left as it is.
    guard: f64,
    /// How far one row's step may differ from its stretch's mean step and still be smoothed, as a
    /// share of the limit. The rest of a row's own step is kept.
    row_share: f64,
}

/// For a trace in black and white, which thresholds the image without smoothing it first: small
/// steps only, each row with its own.
pub(crate) const BLACK_AND_WHITE: Strength = Strength { reach: 2.0, guard: 8.0, row_share: 0.5 };
/// For a trace of up to 16 colors: each row with its own step.
pub(crate) const FEW_COLORS: Strength = Strength { reach: 8.0, guard: 13.0, row_share: 0.5 };
/// For a trace of more than 16 colors: larger steps, but only each stretch's mean step, so that
/// every row of a stretch moves by the same amount.
pub(crate) const MANY_COLORS: Strength = Strength { reach: 4.0, guard: EDGE, row_share: 0.0 };

/// Where the block edges of a block-coded image are, and how large their steps are.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Grid {
    /// A block edge lies between columns `x - 1` and `x` when `x % 8 == across`.
    across: usize,
    /// A block edge lies between rows `y - 1` and `y` when `y % 8 == down`.
    down: usize,
    /// The extra step the block edges show on average over the other positions, across or down,
    /// whichever is smaller.
    extra: f64,
}

/// Positions counted when the grid is measured: two blocks, so that a pattern of 16 px or more
/// (a checkerboard, cells of a picture enlarged 16 times), whose edges fall on every other block
/// edge only, is not taken for blocks.
const SPAN: usize = 2 * BLOCK;

/// For one direction, from the mean step between neighboring opaque pixels at each of 16 positions:
/// the position (of eight) where it peaks at both block edges of the span, the extra step those
/// show over the other positions, and whether there is no step at all at the other positions; `None`
/// when no position stands out.
fn peak(sum: &[f64; SPAN], count: &[u64; SPAN]) -> Option<(usize, f64, bool)> {
    if count.contains(&0) {
        return None;
    }
    let mean: Vec<f64> = sum.iter().zip(count).map(|(&s, &n)| s / n as f64).collect();
    let pair = |at: usize| (mean.get(at).copied().unwrap_or(0.0), mean.get(at + BLOCK).copied().unwrap_or(0.0));
    let at = (0..BLOCK).max_by(|&a, &b| {
        let (pa, pb) = (pair(a), pair(b));
        pa.0.min(pa.1).total_cmp(&pb.0.min(pb.1))
    })?;
    let (first, second) = pair(at);
    let others = || mean.iter().enumerate().filter(|&(i, _)| i != at && i != at + BLOCK).map(|(_, &m)| m);
    let rest = others().sum::<f64>() / (SPAN - 2) as f64;
    (first.min(second) > PEAK * rest).then_some((at, (first + second) / 2.0 - rest, others().all(|m| m == 0.0)))
}

/// The block grid of `img`, if it has one.
fn grid(img: &Raster) -> Option<Grid> {
    let (w, h) = (img.width as usize, img.height as usize);
    if w < 2 * SPAN || h < 2 * SPAN || img.rgba.len() != w * h * 4 {
        return None;
    }
    let px = img.rgba.as_chunks::<4>().0;
    let luma = |p: &[u8; 4]| (p[3] == 255).then(|| f64::from(quantize::luma(*p)));
    let (mut across, mut down) = (([0.0; SPAN], [0u64; SPAN]), ([0.0; SPAN], [0u64; SPAN]));
    for y in 0..h {
        for x in 0..w {
            let Some(here) = px.get(y * w + x).and_then(luma) else { continue };
            let add = |bins: &mut ([f64; SPAN], [u64; SPAN]), at: usize, other: Option<f64>| {
                if let Some(step) = other.map(|o| (here - o).abs()).filter(|&s| s < EDGE) {
                    bins.0[at % SPAN] += step;
                    bins.1[at % SPAN] += 1;
                }
            };
            if x > 0 {
                add(&mut across, x, px.get(y * w + x - 1).and_then(luma));
            }
            if y > 0 {
                add(&mut down, y, px.get((y - 1) * w + x).and_then(luma));
            }
        }
    }
    let (across, extra_across, flat_across) = peak(&across.0, &across.1)?;
    let (down, extra_down, flat_down) = peak(&down.0, &down.1)?;
    // Blocks that are each one flat color, with no step at all inside them, are cells of a picture
    // enlarged eight times, which stay as they are.
    if flat_across && flat_down {
        return None;
    }
    Some(Grid { across, down, extra: extra_across.min(extra_down) })
}

/// Smooths one stretch of a block edge in `band` (values in sixteenths of a level, 3 channels per
/// pixel): `lines` (1 to 8) lines that cross the edge, the pixel at position `i` (0 to 7; the edge lies between
/// 3 and 4) of line `l` starting at `band[index(l, i)]`, where `opaque(l, i)` says whether it is fully
/// opaque. The stretch's step is the mean over its lines of the step across the edge less the slope
/// on either side. A stretch of opaque pixels whose step is under `limit` on every channel, with no
/// step of `strength.guard` or more between neighbors in its pixels (across, the edge itself aside, or
/// along), has that step spread over the eight pixels of each line: with the line's own difference
/// from it, to at most `strength.row_share` of `limit` either way, each pixel moves by the step times
/// 3.5/8, 2.5/8, 1.5/8 or 0.5/8 by its distance from the edge, so the pixels in the middle of the
/// blocks hardly move.
fn smooth_edge(
    band: &mut [i16],
    lines: usize,
    index: impl Fn(usize, usize) -> usize,
    opaque: impl Fn(usize, usize) -> bool,
    limit: f64,
    strength: Strength,
) {
    if lines == 0 || lines > BLOCK || !(0..lines).all(|l| (0..BLOCK).all(|i| opaque(l, i))) {
        return;
    }
    let at = |l: usize, i: usize, k: usize| f64::from(band.get(index(l, i) + k).copied().unwrap_or(0)) / SCALE;
    // Each line's step on each channel, and the largest step within the pixels around the edge.
    let mut steps = [[0.0; 3]; BLOCK];
    let steps = steps.get_mut(..lines).unwrap_or_default();
    let mut inner = 0.0f64;
    for (l, step) in steps.iter_mut().enumerate() {
        for (k, s) in step.iter_mut().enumerate() {
            let p: [f64; BLOCK] = std::array::from_fn(|i| at(l, i, k));
            *s = (p[4] - p[3]) - ((p[3] - p[2]) + (p[5] - p[4])) / 2.0;
            for i in (0..BLOCK - 1).filter(|&i| i != 3) {
                inner = inner.max((p[i + 1] - p[i]).abs());
            }
            // And from this line to the next: a line along the edge is an edge too.
            if l + 1 < lines {
                for (i, &here) in p.iter().enumerate() {
                    inner = inner.max((at(l + 1, i, k) - here).abs());
                }
            }
        }
    }
    let mean: [f64; 3] = std::array::from_fn(|k| steps.iter().map(|s| s[k]).sum::<f64>() / lines as f64);
    if inner >= strength.guard || mean.iter().any(|m| m.abs() >= limit) {
        return;
    }
    let own = limit * strength.row_share;
    for (l, step) in steps.iter().enumerate() {
        for k in 0..3 {
            let step = mean[k] + (step[k] - mean[k]).clamp(-own, own);
            for i in 0..BLOCK / 2 {
                let shift = (step * (3.5 - i as f64) / BLOCK as f64 * SCALE).round() as i16;
                if let Some(p) = band.get_mut(index(l, 3 - i) + k) {
                    *p = p.saturating_add(shift);
                }
                if let Some(p) = band.get_mut(index(l, 4 + i) + k) {
                    *p = p.saturating_sub(shift);
                }
            }
        }
    }
}

/// Where the stretches of eight lines start along a block edge `len` lines long, when they start at
/// the lines with `l % 8 == at`: the first, from line 0, can be shorter. Each as (first line, count).
fn stretches(len: usize, at: usize) -> Vec<(usize, usize)> {
    let mut out: Vec<(usize, usize)> = if at > 0 { vec![(0, at.min(len))] } else { vec![] };
    out.extend((at..len).step_by(BLOCK).map(|s| (s, BLOCK.min(len - s))));
    out
}

/// `f` of each of `bands` (with what it needs to know about the band), which it may change, spread
/// over the available cores.
fn each_band<K: Copy + Send, T: Send>(bands: Vec<(K, &mut [T])>, f: impl Fn(K, &mut [T]) + Sync) {
    let bands: Vec<Mutex<(K, &mut [T])>> = bands.into_iter().map(Mutex::new).collect();
    parallel_map(&bands, |band| {
        // Each band is locked by one worker only, so the lock is never held elsewhere.
        if let Ok(mut band) = band.lock() {
            let (first, values) = &mut *band;
            f(*first, values);
        }
    });
}

/// The first block edge (of positions `at`, `at + 8`, ...) with four pixels before it.
fn first_edge(at: usize) -> usize {
    if at >= BLOCK / 2 { at } else { at + BLOCK }
}

/// Smooths the block edges of `v`, rows `top` on of the image (`w` pixels of 3 channels, in
/// sixteenths of a level, from the first row of a stretch of rows to the last row of one), where
/// `opaque(y, x)` is set: the edges between columns `x - 1` and `x` with `x % 8 == across` in stretches
/// of eight rows that start at the rows with `y % 8 == down`, then the edges between rows `e - 1` and
/// `e` for each `e` of `edges` in stretches of eight columns that start at the columns with
/// `x % 8 == across` (see `smooth_edge`).
fn smooth_strip(
    v: &mut [i16],
    top: usize,
    w: usize,
    grid: &Grid,
    edges: &[usize],
    opaque: impl Fn(usize, usize) -> bool,
    (limit, strength): (f64, Strength),
) {
    let row = w * 3;
    if w < BLOCK || row == 0 {
        return;
    }
    for (at, rows) in stretches(v.len() / row, (grid.down + BLOCK - top % BLOCK) % BLOCK) {
        let Some(band) = v.get_mut(at * row..(at + rows) * row) else { continue };
        for e in (first_edge(grid.across)..=w - BLOCK / 2).step_by(BLOCK) {
            smooth_edge(band, rows, |l, i| (l * w + e - BLOCK / 2 + i) * 3, |l, i| opaque(top + at + l, e - BLOCK / 2 + i), limit, strength);
        }
    }
    for &e in edges {
        let Some(band) = e.checked_sub(top + BLOCK / 2).and_then(|at| v.get_mut(at * row..(at + BLOCK) * row)) else { continue };
        for (left, columns) in stretches(w, grid.across) {
            smooth_edge(band, columns, |l, i| (i * w + left + l) * 3, |l, i| opaque(e - BLOCK / 2 + i, left + l), limit, strength);
        }
    }
}

/// The edges between rows smoothed in one piece of work, with the stretches of rows around them.
const STRIP: usize = 8;

/// `img` with its block edges smoothed away as `strength` says, or `None` when it shows no block grid.
/// The image is worked on in strips of rows, each with the stretches of rows its `STRIP` edges between
/// rows need, so only the copy it returns is as large as the image.
pub(crate) fn deblocked(img: &Raster, strength: Strength) -> Option<Raster> {
    let grid = grid(img)?;
    let limit = (strength.reach * grid.extra).clamp(LEAST, EDGE);
    let (w, h) = (img.width as usize, img.height as usize);
    let px = img.rgba.as_chunks::<4>().0;
    let opaque = |y: usize, x: usize| px.get(y * w + x).is_some_and(|p| p[3] == 255);
    let edges: Vec<usize> = (first_edge(grid.down)..=h.saturating_sub(BLOCK / 2)).step_by(BLOCK).collect();
    let groups: Vec<&[usize]> = edges.chunks(STRIP).collect();
    // The rows each strip writes: from four rows above its first edge (the top of the image for the
    // first strip) to four rows below its last (the bottom of the image for the last strip).
    let mut rgba = img.rgba.clone();
    let mut bands = vec![];
    let mut rest: &mut [u8] = &mut rgba;
    let mut above = 0;
    for (n, group) in groups.iter().enumerate() {
        let below = if n + 1 == groups.len() { h } else { group.last().map_or(h, |e| e + BLOCK / 2) };
        let (band, after) = std::mem::take(&mut rest).split_at_mut_checked(below.checked_sub(above)? * w * 4)?;
        bands.push(((above, *group), band));
        rest = after;
        above = below;
    }
    // The first row of the stretch of rows that holds row `y`, and the row after the stretch.
    let stretch = |y: usize| {
        let first = if y < grid.down { 0 } else { y - (y - grid.down) % BLOCK };
        (first, if y < grid.down { grid.down } else { first + BLOCK }.min(h))
    };
    each_band(bands, |(above, group), band| {
        let below = above + band.len() / (w * 4);
        let (first, last) = (stretch(above).0, stretch(below.saturating_sub(1)).1);
        let Some(source) = px.get(first * w..last * w) else { return };
        let mut v: Vec<i16> = source.iter().flat_map(|p| [p[0], p[1], p[2]].map(|c| i16::from(c) * SCALE as i16)).collect();
        smooth_strip(&mut v, first, w, &grid, group, opaque, (limit, strength));
        let skip = (above - first) * w;
        for (p, c) in band.as_chunks_mut::<4>().0.iter_mut().zip(v.as_chunks::<3>().0.iter().skip(skip)) {
            for k in 0..3 {
                p[k] = ((i32::from(c[k]) + 8).div_euclid(16)).clamp(0, 255) as u8;
            }
        }
    });
    Some(Raster { width: img.width, height: img.height, rgba })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A number in -1..1 that looks random, from `x`, `y` and `k`.
    fn jitter(x: u32, y: u32, k: u32) -> f64 {
        let mut n = x.wrapping_mul(0x9E37_79B1) ^ y.wrapping_mul(0x85EB_CA77) ^ k.wrapping_mul(0xC2B2_AE3D);
        n ^= n >> 15;
        n = n.wrapping_mul(0x2C1B_3C6D);
        n ^= n >> 12;
        f64::from(n % 2001) / 1000.0 - 1.0
    }

    /// A dark ground that brightens slowly to the right, with a level of noise and, when `blocks`,
    /// every 8 x 8 block (counted from `(dx, dy)`) moved by up to 2 levels, as a JPEG leaves it.
    fn ground(w: u32, h: u32, blocks: bool, dx: u32, dy: u32) -> Raster {
        Raster::from_fn(w, h, |x, y| {
            let offset = if blocks { 2.0 * jitter((x + dx) / 8, (y + dy) / 8, 9) } else { 0.0 };
            let v = 20.0 + 30.0 * f64::from(x) / f64::from(w) + offset + jitter(x, y, 0);
            let v = v.round().clamp(0.0, 255.0) as u8;
            [v, v, v, 255]
        })
    }

    /// The mean step between neighboring columns at the block edges (`x % 8 == across`) and elsewhere.
    fn steps(img: &Raster, across: u32) -> (f64, f64) {
        let (mut on, mut off) = ((0.0, 0), (0.0, 0));
        for y in 0..img.height {
            for x in 1..img.width {
                let d = (f64::from(img.pixel(x, y)[0]) - f64::from(img.pixel(x - 1, y)[0])).abs();
                let bin = if x % 8 == across { &mut on } else { &mut off };
                bin.0 += d;
                bin.1 += 1;
            }
        }
        (on.0 / f64::from(on.1), off.0 / f64::from(off.1))
    }

    #[test]
    fn the_block_grid_is_found_where_the_blocks_are() {
        let g = grid(&ground(160, 120, true, 0, 0)).unwrap();
        assert_eq!((g.across, g.down), (0, 0));
        // The same image with 3 columns and 5 rows cut off: its block edges are at x % 8 == 5, y % 8 == 3.
        let g = grid(&ground(160, 120, true, 3, 5)).unwrap();
        assert_eq!((g.across, g.down), (5, 3));
        assert!(g.extra > 0.5, "{}", g.extra);
    }

    #[test]
    fn an_image_without_blocks_is_left_as_it_is() {
        assert_eq!(grid(&ground(160, 120, false, 0, 0)), None);
        assert!(deblocked(&ground(160, 120, false, 0, 0), FEW_COLORS).is_none());
        // A picture of four flat colors enlarged eight times has steps only at its cells' edges, and
        // none within them; two of its colors are 6 levels apart.
        let colors = [[30, 30, 36, 255], [36, 30, 30, 255], [200, 60, 40, 255], [240, 220, 120, 255]];
        let cells = Raster::from_fn(160, 120, |x, y| colors[((jitter(x / 8, y / 8, 1) + 1.0) * 2.0).min(3.0) as usize]);
        assert_eq!(grid(&cells), None);
        // So does a picture of many shades enlarged eight times, as a photo's would.
        let shades = Raster::from_fn(160, 120, |x, y| {
            let v = (40.0 + 0.6 * f64::from(x / 8 * 8) + 0.5 * f64::from(y / 8 * 8) + 3.0 * jitter(x / 8, y / 8, 4)).round() as u8;
            [v, v.saturating_sub(10), v.saturating_add(10), 255]
        });
        assert_eq!(grid(&shades), None);
        // A noisy checkerboard of 16 px squares 10 levels apart has its steps on every other block edge.
        let squares = Raster::from_fn(160, 120, |x, y| {
            let v = (20.0 + 10.0 * f64::from((x / 16 + y / 16) % 3) + 4.0 * jitter(x, y, 2)).round() as u8;
            [v, v, v, 255]
        });
        assert_eq!(grid(&squares), None);
        // Too small to hold two spans of 16 px each way.
        assert_eq!(grid(&ground(31, 120, true, 0, 0)), None);
    }

    #[test]
    fn the_steps_at_block_edges_become_ramps() {
        for ((dx, dy), strength) in [((0, 0), FEW_COLORS), ((3, 5), FEW_COLORS), ((0, 0), MANY_COLORS), ((0, 0), BLACK_AND_WHITE)] {
            let img = ground(160, 120, true, dx, dy);
            let across = (8 - dx % 8) % 8;
            let (on, off) = steps(&img, across);
            assert!(on > 2.0 * off, "before: {on} at the block edges, {off} elsewhere");
            let out = deblocked(&img, strength).unwrap();
            let (on, off) = steps(&out, across);
            assert!(on < 1.25 * off, "after: {on} at the block edges, {off} elsewhere");
            // The ground's own slope stays: 30 levels across the image.
            let mean = |r: &Raster, x: u32| (0..r.height).map(|y| f64::from(r.pixel(x, y)[0])).sum::<f64>() / f64::from(r.height);
            assert!((mean(&out, 156) - mean(&out, 4) - (mean(&img, 156) - mean(&img, 4))).abs() < 0.5);
        }
    }

    #[test]
    fn stretches_cover_every_line_once() {
        assert_eq!(stretches(20, 0), [(0, 8), (8, 8), (16, 4)]);
        assert_eq!(stretches(20, 3), [(0, 3), (3, 8), (11, 8), (19, 1)]);
        assert_eq!(stretches(2, 3), [(0, 2)]);
    }

    #[test]
    fn with_many_colors_every_row_of_a_stretch_moves_alike() {
        let img = ground(160, 120, true, 0, 0);
        let v: Vec<i16> = img.rgba.chunks(4).flat_map(|p| [p[0], p[1], p[2]].map(|c| i16::from(c) * SCALE as i16)).collect();
        // How far each pixel moves when the edges between columns are smoothed.
        let shifts = |strength: Strength| -> Vec<i16> {
            let mut out = v.clone();
            for top in (0..120).step_by(8) {
                let band = &mut out[top * 160 * 3..(top + 8) * 160 * 3];
                for e in (8..=156).step_by(8) {
                    smooth_edge(band, 8, |l, i| (l * 160 + e - 4 + i) * 3, |_, _| true, 4.0, strength);
                }
            }
            out.iter().zip(&v).map(|(o, i)| o - i).step_by(3).collect()
        };
        let (many, few) = (shifts(MANY_COLORS), shifts(FEW_COLORS));
        let differ = |s: &[i16]| {
            (0..160)
                .flat_map(|x| (0..120).step_by(8).map(move |top| (x, top)))
                .filter(|&(x, top)| (top..top + 8).any(|y| s[y * 160 + x] != s[top * 160 + x]))
                .count()
        };
        assert!(many.iter().filter(|&&s| s != 0).count() > 160 * 120 / 2);
        assert_eq!(differ(&many), 0);
        // With few colors, each row keeps part of its own step.
        assert!(differ(&few) > 160 * 15 / 2, "{}", differ(&few));
    }

    #[test]
    fn edges_and_fine_lines_stay_sharp() {
        // Black lines 1 to 4 px wide and white gaps between them on a block-coded mid-gray ground, one
        // line on a block edge.
        let base = ground(160, 120, true, 0, 0);
        let lines = [(16, 1), (31, 2), (40, 3), (61, 4)];
        let img = Raster::from_fn(160, 120, |x, y| {
            if lines.iter().any(|&(at, wide)| (at..at + wide).contains(&x)) {
                [0, 0, 0, 255]
            } else {
                let v = base.pixel(x, y)[0] + 80;
                [v, v, v, 255]
            }
        });
        let out = deblocked(&img, FEW_COLORS).unwrap();
        for y in 0..120 {
            for &(at, wide) in &lines {
                for x in at..at + wide {
                    assert_eq!(out.pixel(x, y), [0, 0, 0, 255], "line pixel ({x}, {y})");
                }
                // The ground beside a line moves by no more than a block's offset (the edges between
                // rows in its column are smoothed; the one across the line is not).
                for x in [at - 1, at + wide] {
                    let moved = (i32::from(out.pixel(x, y)[0]) - i32::from(img.pixel(x, y)[0])).abs();
                    assert!(moved <= 3, "beside the line, ({x}, {y}) moved {moved}");
                }
            }
        }
    }

    #[test]
    fn transparent_pixels_keep_their_colors() {
        let base = ground(160, 120, true, 0, 0);
        let img = Raster::from_fn(160, 120, |x, y| {
            let mut p = base.pixel(x, y);
            if x >= 100 {
                p[3] = if x < 104 { 128 } else { 0 };
            }
            p
        });
        let out = deblocked(&img, FEW_COLORS).unwrap();
        for y in 0..120 {
            for x in 100..160 {
                assert_eq!(out.pixel(x, y), img.pixel(x, y), "({x}, {y})");
            }
        }
        assert_eq!(out.rgba.chunks(4).map(|p| p[3]).collect::<Vec<_>>(), img.rgba.chunks(4).map(|p| p[3]).collect::<Vec<_>>());
    }
}
