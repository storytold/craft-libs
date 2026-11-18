//! Guo–Hall A1 thinning (Guo & Hall 1989, doi:10.1145/62065.62074).
//! Neighbourhoods are judged together in each sub-iteration; only changed ones are revisited.

use super::Grid;

/// Guo–Hall A1's test for pixel with neighbours `p` (P2..P9) in sub-iteration `step`.
fn guo_hall_removable(p: &[bool; 8], step: usize) -> bool {
    let [p2, p3, p4, p5, p6, p7, p8, p9] = *p;
    let c = usize::from(!p2 && (p3 || p4)) + usize::from(!p4 && (p5 || p6)) + usize::from(!p6 && (p7 || p8)) + usize::from(!p8 && (p9 || p2));
    let n1 = usize::from(p9 || p2) + usize::from(p3 || p4) + usize::from(p5 || p6) + usize::from(p7 || p8);
    let n2 = usize::from(p2 || p3) + usize::from(p4 || p5) + usize::from(p6 || p7) + usize::from(p8 || p9);
    let m = if step == 0 { (p6 || p7 || !p9) && p8 } else { (p2 || p3 || !p5) && p4 };
    c == 1 && (2..=3).contains(&n1.min(n2)) && !m
}

/// The Guo–Hall rounds alone (no staircase removal). Bounded: at most `w + h` rounds.
///
/// Only pixels whose neighbourhood changed since they were last judged in a sub-iteration are
/// judged again (a pixel's removability depends on its 8 neighbours only): in sub-iteration 0,
/// neighbours of every pixel removed in the last round; in sub-iteration 1, neighbours of those
/// removed in the last round's sub-iteration 1 and this round's sub-iteration 0. The result is
/// the plain algorithm's (tested against it below).
fn guo_hall_rounds(g: &mut Grid) {
    let n = g.on.len();
    let ring = g.offsets();
    let mut mark = vec![0u32; n];
    let mut stamp = 0u32;
    let mut around = |g: &Grid, removed: &[&[usize]]| -> Vec<usize> {
        // Two stamps per round; the crop cap keeps this well below u32::MAX.
        stamp += 1;
        let mut out = vec![];
        for &t in removed.iter().flat_map(|r| r.iter()) {
            for o in ring {
                let i = t.wrapping_add_signed(o);
                if i < n && g.on.get(i) == Some(&true) && g.is_interior(i) && mark.get(i) != Some(&stamp) {
                    if let Some(m) = mark.get_mut(i) {
                        *m = stamp;
                    }
                    out.push(i);
                }
            }
        }
        out.sort_unstable();
        out
    };
    let (mut prev0, mut prev1): (Vec<usize>, Vec<usize>) = (vec![], vec![]);
    for round in 0..g.w + g.h {
        let cand0: Vec<usize> =
            if round == 0 { (0..n).filter(|&i| g.on.get(i) == Some(&true) && g.is_interior(i)).collect() } else { around(g, &[&prev0, &prev1]) };
        let rem0: Vec<usize> = cand0.into_iter().filter(|&i| guo_hall_removable(&g.ring(i), 0)).collect();
        for &i in &rem0 {
            if let Some(on) = g.on.get_mut(i) {
                *on = false;
            }
        }
        let cand1: Vec<usize> =
            if round == 0 { (0..n).filter(|&i| g.on.get(i) == Some(&true) && g.is_interior(i)).collect() } else { around(g, &[&prev1, &rem0]) };
        let rem1: Vec<usize> = cand1.into_iter().filter(|&i| guo_hall_removable(&g.ring(i), 1)).collect();
        for &i in &rem1 {
            if let Some(on) = g.on.get_mut(i) {
                *on = false;
            }
        }
        if rem0.is_empty() && rem1.is_empty() {
            break;
        }
        (prev0, prev1) = (rem0, rem1);
    }
}

/// Thins `g` in place (Guo–Hall, then staircase removal); see the module docs.
pub(super) fn thin(g: &mut Grid) {
    guo_hall_rounds(g);
    staircase(g);
}

/// Removes staircase pixels (see the module docs). Sequential, in scan order, until stable
/// (at most 4 passes).
fn staircase(g: &mut Grid) {
    for _ in 0..4 {
        let mut changed = false;
        for i in 0..g.on.len() {
            // Three or more neighbours: a corner of a staircase always has them, while a pixel
            // with two touching neighbours may be the end of a line two pixels thick (removing
            // it would eat that line from its end, pixel by pixel).
            if g.on.get(i) != Some(&true) || g.degree(i) < 3 {
                continue;
            }
            let p = g.ring(i);
            // Yokoi's x1..x8 = E, NE, N, NW, W, SW, S, SE.
            let x = [p[2], p[1], p[0], p[7], p[6], p[5], p[4], p[3]];
            let off = |k: usize| !x.get(k % 8).copied().unwrap_or(false);
            let c8: usize = [0usize, 2, 4, 6].iter().map(|&k| usize::from(off(k)) - usize::from(off(k) && off(k + 1) && off(k + 2))).sum();
            if c8 == 1 {
                if let Some(on) = g.on.get_mut(i) {
                    *on = false;
                }
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skel(w: usize, h: usize, f: impl Fn(usize, usize) -> bool) -> Grid {
        let m: Vec<bool> = (0..w * h).map(|i| f(i % w, i / w)).collect();
        let mut g = Grid::from_mask(&m, w, h).unwrap();
        thin(&mut g);
        g
    }

    #[test]
    fn bars_thin_to_lines() {
        for thick in 1..7 {
            let g = skel(40, 12, |x, y| (3..37).contains(&x) && (3..3 + thick).contains(&y));
            let on: Vec<usize> = (0..g.on.len()).filter(|&i| g.on[i]).collect();
            assert!(on.len() >= 20, "thick {thick}: {} pixels", on.len());
            // Corner spurs may remain at the square ends (the graph prunes them).
            assert!(on.iter().filter(|&&i| g.degree(i) == 1).count() >= 2, "thick {thick}");
        }
    }

    #[test]
    fn diagonal_lines_two_to_five_pixels_thick_keep_their_length() {
        for thick in 2..6usize {
            for mirror in [false, true] {
                let g = skel(60, 60, |x, y| {
                    let x = if mirror { 59 - x } else { x };
                    (5..55).contains(&y) && x + 1 >= y && x < y + thick
                });
                let on: Vec<usize> = (0..g.on.len()).filter(|&i| g.on[i]).collect();
                let (lo, hi) = (on.iter().map(|&i| i / g.w).min().unwrap(), on.iter().map(|&i| i / g.w).max().unwrap());
                assert!(hi - lo >= 46, "thick {thick}, mirror {mirror}: shortened to {} rows", hi - lo + 1);
                assert!(on.iter().all(|&i| g.degree(i) <= 2), "thick {thick}: not one pixel wide");
                assert_eq!(on.iter().filter(|&&i| g.degree(i) == 1).count(), 2);
                let mut seen = std::collections::HashSet::new();
                let mut stack = vec![on[0]];
                while let Some(i) = stack.pop() {
                    if seen.insert(i) {
                        stack.extend(g.neighbours(i));
                    }
                }
                assert_eq!(seen.len(), on.len(), "disconnected diagonal");
            }
        }
    }

    /// Plain Guo–Hall: every pixel judged every sub-iteration.
    fn reference(g: &mut Grid) {
        loop {
            let mut any = false;
            for step in 0..2 {
                let picked: Vec<usize> =
                    (0..g.on.len()).filter(|&i| g.on.get(i) == Some(&true) && g.is_interior(i) && guo_hall_removable(&g.ring(i), step)).collect();
                any |= !picked.is_empty();
                for i in picked {
                    if let Some(on) = g.on.get_mut(i) {
                        *on = false;
                    }
                }
            }
            if !any {
                break;
            }
        }
    }

    #[test]
    fn the_active_set_gives_the_plain_algorithms_result() {
        let mut seed = 11u64;
        let mut rnd = || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (seed >> 33) as usize
        };
        for case in 0..200 {
            let (w, h) = (3 + rnd() % 40, 3 + rnd() % 40);
            let blobs: Vec<(usize, usize, usize)> = (0..1 + rnd() % 6).map(|_| (rnd() % w, rnd() % h, 1 + rnd() % 9)).collect();
            let m: Vec<bool> = (0..w * h)
                .map(|i| {
                    let (x, y) = ((i % w) as i64, (i / w) as i64);
                    blobs.iter().any(|&(bx, by, r)| (x - bx as i64).pow(2) + (y - by as i64).pow(2) <= (r * r) as i64)
                })
                .collect();
            let (mut a, mut b) = (Grid::from_mask(&m, w, h).unwrap(), Grid::from_mask(&m, w, h).unwrap());
            guo_hall_rounds(&mut a);
            reference(&mut b);
            assert!(a.on == b.on, "case {case}");
        }
    }
}
