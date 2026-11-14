//! Motion search for frame prediction: integer pattern search from predictor candidates, then
//! half-sample refinement. Costs are SAD + λ·(motion vector bits).

use crate::mpeg2::mc::{Block, Frame, predict};

use super::tables::{enc_tables, wrap_delta};

pub(crate) struct Search<'a> {
    pub reference: &'a Frame,
    /// The source macroblock's luma (16×16).
    pub src: &'a [u8; 256],
    pub mbx: usize,
    pub mby: usize,
    pub f_code: u8,
    /// Largest integer displacement searched, in samples.
    pub range: i32,
    /// Motion vector predictor (half-sample units).
    pub pmv: [i32; 2],
    pub lambda: u32,
}

/// The chosen vector (half-sample units) and its SAD.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Found {
    pub mv: [i32; 2],
    pub sad: u32,
}

/// Steps of the integer pattern search, coarse to fine.
const STEPS: [i32; 4] = [8, 4, 2, 1];
/// Moves per step before moving on (bounds the search on pathological content).
const MAX_MOVES: usize = 16;

impl Search<'_> {
    fn origin(&self) -> (i32, i32) {
        ((self.mbx * 16) as i32, (self.mby * 16) as i32)
    }

    /// Integer displacements allowed per axis: inside the reference and the f_code range.
    fn bounds(&self) -> ([i32; 2], [i32; 2]) {
        let f = 1i32 << self.f_code.saturating_sub(1);
        let (ox, oy) = self.origin();
        let (w, h) = (self.reference.w as i32, self.reference.h as i32);
        let lo_lim = -(8 * f).min(self.range);
        let hi_lim = (8 * f - 1).min(self.range);
        ([lo_lim.max(-ox), lo_lim.max(-oy)], [hi_lim.min(w - 16 - ox), hi_lim.min(h - 16 - oy)])
    }

    /// A half-sample vector the bitstream can carry whose prediction stays inside the reference.
    fn valid(&self, mv: [i32; 2]) -> bool {
        let f = 1i32 << self.f_code.saturating_sub(1);
        let (ox, oy) = self.origin();
        let size = [self.reference.w as i32, self.reference.h as i32];
        let o = [ox, oy];
        (0..2).all(|c| {
            let start = o[c] + (mv[c] >> 1);
            mv[c] >= -16 * f && mv[c] < 16 * f && start >= 0 && start + 16 + (mv[c] & 1) <= size[c]
        })
    }

    fn mv_cost(&self, mv: [i32; 2]) -> u32 {
        let t = enc_tables();
        let bits: u32 = (0..2).map(|c| t.motion_bits(wrap_delta(mv[c], self.pmv[c], self.f_code), self.f_code)).sum();
        self.lambda * bits
    }

    fn sad_int(&self, d: [i32; 2]) -> u32 {
        let (ox, oy) = self.origin();
        let stride = self.reference.w;
        let x = (ox + d[0]) as usize;
        let y = (oy + d[1]) as usize;
        let plane = &self.reference.planes[0];
        let mut sad = 0u32;
        for r in 0..16 {
            let at = (y + r) * stride + x;
            let Some(row) = plane.get(at..at + 16) else { return u32::MAX };
            for (a, b) in row.iter().zip(&self.src[r * 16..r * 16 + 16]) {
                sad += u32::from(a.abs_diff(*b));
            }
        }
        sad
    }

    fn sad_half(&self, mv: [i32; 2]) -> u32 {
        let (ox, oy) = self.origin();
        let mut tmp = [0u8; 256];
        let blk = Block { x: ox + (mv[0] >> 1), y: oy + (mv[1] >> 1), hx: mv[0] & 1 != 0, hy: mv[1] & 1 != 0, bw: 16, bh: 16 };
        predict(&mut tmp, 0, 16, self.reference.plane(0, None), blk, false);
        tmp.iter().zip(self.src.iter()).map(|(a, b)| u32::from(a.abs_diff(*b))).sum()
    }

    /// Best vector among the candidates (half-sample units), refined.
    pub fn run(&self, candidates: &[[i32; 2]]) -> Found {
        let (lo, hi) = self.bounds();
        let clamp = |d: [i32; 2]| [d[0].clamp(lo[0], hi[0].max(lo[0])), d[1].clamp(lo[1], hi[1].max(lo[1]))];
        let eval = |d: [i32; 2]| {
            let sad = self.sad_int(d);
            (sad.saturating_add(self.mv_cost([2 * d[0], 2 * d[1]])), sad)
        };
        let mut best_d = [0, 0];
        let (mut best_cost, mut best_sad) = eval(best_d);
        for c in candidates {
            let d = clamp([c[0] >> 1, c[1] >> 1]);
            if d != best_d {
                let (cost, sad) = eval(d);
                if cost < best_cost {
                    (best_d, best_cost, best_sad) = (d, cost, sad);
                }
            }
        }
        for step in STEPS.into_iter().filter(|&s| s <= self.range.max(1)) {
            for _ in 0..MAX_MOVES {
                let mut moved = false;
                for off in [[step, 0], [-step, 0], [0, step], [0, -step]] {
                    let d = clamp([best_d[0] + off[0], best_d[1] + off[1]]);
                    if d == best_d {
                        continue;
                    }
                    let (cost, sad) = eval(d);
                    if cost < best_cost {
                        (best_d, best_cost, best_sad) = (d, cost, sad);
                        moved = true;
                    }
                }
                if !moved {
                    break;
                }
            }
        }
        // half-sample refinement around the integer result
        let centre = [2 * best_d[0], 2 * best_d[1]];
        let mut best = Found { mv: centre, sad: best_sad };
        for dy in -1..=1 {
            for dx in -1..=1 {
                let mv = [centre[0] + dx, centre[1] + dy];
                if (dx == 0 && dy == 0) || !self.valid(mv) {
                    continue;
                }
                let sad = self.sad_half(mv);
                let cost = sad.saturating_add(self.mv_cost(mv));
                if cost < best_cost {
                    best_cost = cost;
                    best = Found { mv, sad };
                }
            }
        }
        best
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A textured reference and a source macroblock cut from it at a known offset.
    fn scene(dx: i32, dy: i32) -> (Frame, [u8; 256]) {
        let mut f = Frame::new(64, 64, 32, 32);
        for y in 0..64 {
            for x in 0..64 {
                let (x, y) = (x as f32, y as f32);
                f.planes[0][y as usize * 64 + x as usize] =
                    (128.0 + 60.0 * (x * 0.31).sin() * (y * 0.23).cos() + 30.0 * ((x + y) * 0.13).sin()) as u8;
            }
        }
        let mut src = [0u8; 256];
        for r in 0..16 {
            for c in 0..16 {
                src[r * 16 + c] = f.planes[0][(16 + dy + r as i32) as usize * 64 + (16 + dx + c as i32) as usize];
            }
        }
        (f, src)
    }

    #[test]
    fn finds_an_integer_displacement() {
        let (f, src) = scene(3, -2);
        let s = Search { reference: &f, src: &src, mbx: 1, mby: 1, f_code: 3, range: 15, pmv: [0, 0], lambda: 1 };
        let found = s.run(&[]);
        assert_eq!(found, Found { mv: [6, -4], sad: 0 });
    }

    #[test]
    fn stays_inside_the_reference() {
        let (f, src) = scene(0, 0);
        for (mbx, mby) in [(0, 0), (3, 3), (0, 3)] {
            let s = Search { reference: &f, src: &src, mbx, mby, f_code: 4, range: 63, pmv: [0, 0], lambda: 0 };
            let found = s.run(&[[-200, -200], [200, 200]]);
            assert!(s.valid(found.mv), "{found:?} at {mbx},{mby}");
        }
    }
}
