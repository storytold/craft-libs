//! Boundary following on the pixel-crack grid.
//!
//! Vertices are pixel corners `(x, y)` with `0 ≤ x ≤ w`, `0 ≤ y ≤ h`. Every edge between an inside
//! and an outside pixel becomes a directed edge with the inside pixel on its right (y down), so
//! outer boundaries run clockwise and holes counter-clockwise. At a saddle (two inside pixels
//! touching only diagonally) the walk turns right, which keeps diagonal neighbours apart — the
//! same 4-connectivity used to label components.

/// A closed boundary loop.
#[derive(Clone, Debug, PartialEq)]
pub struct Loop {
    /// Corner vertices in order (the loop closes back to the first).
    pub pts: Vec<(i32, i32)>,
    /// Twice the signed area (positive = outer boundary, negative = hole).
    pub area2: i64,
}

/// One 4-connected component of the mask: its outer loop and holes.
#[derive(Clone, Debug, PartialEq)]
pub struct Component {
    pub outer: Loop,
    pub holes: Vec<Loop>,
    /// Inside pixel count.
    pub pixels: usize,
}

const R: u8 = 1;
const D: u8 = 2;
const L: u8 = 4;
const U: u8 = 8;

fn right_of(d: u8) -> u8 {
    match d {
        R => D,
        D => L,
        L => U,
        _ => R,
    }
}

fn step(d: u8) -> (i32, i32) {
    match d {
        R => (1, 0),
        D => (0, 1),
        L => (-1, 0),
        _ => (0, -1),
    }
}

/// 4-connected component labels (`u32::MAX` outside) and per-component pixel counts.
pub(crate) fn label(mask: &[bool], w: usize, h: usize) -> (Vec<u32>, Vec<usize>) {
    let mut comp = vec![u32::MAX; w * h];
    let mut sizes = Vec::new();
    // (Indices as `u32`: a mask has at most 64 million pixels, and this stack can hold them all.)
    let mut stack: Vec<u32> = Vec::new();
    for s in 0..w * h {
        if !mask[s] || comp[s] != u32::MAX {
            continue;
        }
        let id = sizes.len() as u32;
        let mut n = 0usize;
        comp[s] = id;
        stack.push(s as u32);
        while let Some(i) = stack.pop() {
            let i = i as usize;
            n += 1;
            let (x, y) = (i % w, i / w);
            let mut go = |j: usize| {
                if mask[j] && comp[j] == u32::MAX {
                    comp[j] = id;
                    stack.push(j as u32);
                }
            };
            if x > 0 {
                go(i - 1);
            }
            if x + 1 < w {
                go(i + 1);
            }
            if y > 0 {
                go(i - w);
            }
            if y + 1 < h {
                go(i + w);
            }
        }
        sizes.push(n);
    }
    (comp, sizes)
}

/// Trace every component of `mask` (row-major, `w × h`; a mask of another length gives nothing).
///
/// A walk around the mask's pixels with the inside on the right. Each turn is worked out during the
/// walk from the two pixels ahead of the current corner: turn right if the one ahead on the right
/// is outside, go straight if the one ahead on the left is, else turn left (so at a saddle the walk
/// turns right and diagonal pixels stay apart). A loop starts at the top edge of an inside pixel
/// whose upper neighbour is outside, found by scanning the pixels in order; one flag per pixel
/// records whether the walk has been along its top edge, so that each loop is walked once.
pub fn trace_mask(mask: &[bool], w: usize, h: usize) -> Vec<Component> {
    if w.checked_mul(h) != Some(mask.len()) {
        return vec![];
    }
    let (comp, sizes) = label(mask, w, h);
    let inside = |x: i32, y: i32| x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h && mask.get(y as usize * w + x as usize) == Some(&true);
    // The pixels ahead of corner (x, y) going `d`: on the right of the way and on its left.
    let ahead = |x: i32, y: i32, d: u8| {
        let (nw, ne, se, sw) = ((x - 1, y - 1), (x, y - 1), (x, y), (x - 1, y));
        let (r, l) = match d {
            R => (se, ne),
            D => (sw, se),
            L => (nw, sw),
            _ => (ne, nw),
        };
        (inside(r.0, r.1), inside(l.0, l.1))
    };
    // A loop has fewer edges than the grid has corners, four times over.
    let longest = 4 * (w + 1) * (h + 1);
    let mut walked = vec![false; w * h];
    let mut outers: Vec<Option<Loop>> = vec![None; sizes.len()];
    let mut holes: Vec<Vec<Loop>> = vec![Vec::new(); sizes.len()];
    for (s, (&on, &owner)) in mask.iter().zip(&comp).enumerate() {
        // Every loop has a rightward edge (the top edge of some inside pixel); start there so the
        // loop closes exactly (right-turn rule at the start corner, too).
        if !on || walked.get(s) != Some(&false) || (s >= w && mask.get(s - w) == Some(&true)) {
            continue;
        }
        let (sx, sy) = ((s % w) as i32, (s / w) as i32);
        if let Some(v) = walked.get_mut(s) {
            *v = true;
        }
        let mut pts = vec![(sx, sy)];
        let (mut x, mut y) = (sx + 1, sy);
        let mut d = R;
        let mut area2: i64 = 0;
        let mut prev = (sx, sy);
        for _ in 0..longest {
            area2 += i64::from(prev.0) * i64::from(y) - i64::from(x) * i64::from(prev.1);
            prev = (x, y);
            let (right, left) = ahead(x, y, d);
            let next = if !right {
                right_of(d)
            } else if !left {
                d
            } else {
                // left turn
                right_of(right_of(right_of(d)))
            };
            if (x, y) == (sx, sy) && next == R {
                break;
            }
            if next == R
                && let Some(v) = walked.get_mut(y as usize * w + x as usize)
            {
                *v = true;
            }
            // Keep only turn vertices.
            if next != d {
                pts.push((x, y));
            }
            let (dx, dy) = step(next);
            x += dx;
            y += dy;
            d = next;
        }
        let l = Loop { pts, area2 };
        if l.area2 > 0 {
            if let Some(o) = outers.get_mut(owner as usize) {
                *o = Some(l);
            }
        } else if let Some(hs) = holes.get_mut(owner as usize) {
            hs.push(l);
        }
    }
    let mut out = Vec::with_capacity(sizes.len());
    for ((o, hs), &pixels) in outers.into_iter().zip(holes).zip(&sizes) {
        if let Some(outer) = o {
            out.push(Component { outer, holes: hs, pixels });
        }
    }
    out
}

#[cfg(test)]
mod walk_tests {
    use super::*;

    /// The walk this module had before: outgoing-direction bits stored per corner in a raster
    /// pass, then followed. Kept to check that [`trace_mask`] gives the same loops.
    fn trace_mask_stored(mask: &[bool], w: usize, h: usize) -> Vec<Component> {
        assert_eq!(mask.len(), w * h);
        let (comp, sizes) = label(mask, w, h);
        let vw = w + 1;
        let mut dirs = vec![0u8; vw * (h + 1)];
        let inside = |x: isize, y: isize| x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h && mask[y as usize * w + x as usize];
        for y in 0..h {
            for x in 0..w {
                if !mask[y * w + x] {
                    continue;
                }
                let (xi, yi) = (x as isize, y as isize);
                if !inside(xi, yi - 1) {
                    dirs[y * vw + x] |= R;
                }
                if !inside(xi + 1, yi) {
                    dirs[y * vw + x + 1] |= D;
                }
                if !inside(xi, yi + 1) {
                    dirs[(y + 1) * vw + x + 1] |= L;
                }
                if !inside(xi - 1, yi) {
                    dirs[(y + 1) * vw + x] |= U;
                }
            }
        }
        let mut outers: Vec<Option<Loop>> = vec![None; sizes.len()];
        let mut holes: Vec<Vec<Loop>> = vec![Vec::new(); sizes.len()];
        for start in 0..dirs.len() {
            // Every loop has an R edge (the top edge of some inside pixel); start there so the loop
            // closes exactly (right-turn rule at the start vertex, too).
            while dirs[start] & R != 0 {
                let (sx, sy) = ((start % vw) as i32, (start / vw) as i32);
                let owner = comp[sy as usize * w + sx as usize];
                dirs[start] &= !R;
                let mut pts = vec![(sx, sy)];
                let (mut x, mut y) = (sx + 1, sy);
                let mut d = R;
                let mut area2: i64 = 0;
                let mut prev = (sx, sy);
                loop {
                    area2 += prev.0 as i64 * y as i64 - x as i64 * prev.1 as i64;
                    prev = (x, y);
                    let v = y as usize * vw + x as usize;
                    let at_start = (x, y) == (sx, sy);
                    let mut cand = dirs[v];
                    if at_start {
                        cand |= R;
                    }
                    let next = if cand.count_ones() == 1 {
                        cand
                    } else if cand & right_of(d) != 0 {
                        right_of(d)
                    } else if cand & d != 0 {
                        d
                    } else {
                        // left turn
                        right_of(right_of(right_of(d)))
                    };
                    if at_start && next == R {
                        break;
                    }
                    if next == 0 || dirs[v] & next == 0 {
                        // Malformed (cannot happen for a valid mask); stop safely.
                        break;
                    }
                    dirs[v] &= !next;
                    // Keep only turn vertices.
                    if next != d {
                        pts.push((x, y));
                    }
                    let (dx, dy) = step(next);
                    x += dx;
                    y += dy;
                    d = next;
                }
                let l = Loop { pts, area2 };
                if owner == u32::MAX {
                    continue;
                }
                if l.area2 > 0 {
                    outers[owner as usize] = Some(l);
                } else {
                    holes[owner as usize].push(l);
                }
            }
        }
        let mut out = Vec::with_capacity(sizes.len());
        for (i, o) in outers.into_iter().enumerate() {
            if let Some(outer) = o {
                out.push(Component { outer, holes: std::mem::take(&mut holes[i]), pixels: sizes[i] });
            }
        }
        out
    }

    /// The walk computing its turns from the pixels gives exactly the stored-direction walk's
    /// loops: on every 4 × 4 mask, and on seeded 23 × 17 masks of several densities.
    #[test]
    fn the_walk_equals_the_stored_direction_walk() {
        for bits in 0u32..(1 << 16) {
            let mask: Vec<bool> = (0..16).map(|i| bits >> i & 1 == 1).collect();
            assert_eq!(trace_mask(&mask, 4, 4), trace_mask_stored(&mask, 4, 4), "{bits:016b}");
        }
        for seed in 0..400u32 {
            let (w, h) = (23, 17);
            let mask: Vec<bool> = (0..w * h)
                .map(|i| ((i as u32).wrapping_mul(2_654_435_761) ^ seed.wrapping_mul(40_503)).wrapping_mul(2_246_822_519) >> 28 < (seed % 15) + 1)
                .collect();
            assert_eq!(trace_mask(&mask, w, h), trace_mask_stored(&mask, w, h), "seed {seed}");
        }
    }

    /// A mask of the wrong length gives nothing instead of a panic.
    #[test]
    fn a_mask_of_the_wrong_length_gives_nothing() {
        assert!(trace_mask(&[true; 3], 2, 2).is_empty());
        assert!(trace_mask(&[true; 4], usize::MAX, 2).is_empty());
    }
}
