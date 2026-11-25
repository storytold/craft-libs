//! Reference frame storage, motion vector prediction, and half-sample / quarter-sample motion
//! compensation (ISO/IEC 14496-2 §7.6–7.7).

use super::headers::VopType;

/// Table 7-6: sixteen-division rounding table for deriving a 4:2:0 chrominance motion vector
/// from the sum of four 8×8 luminance motion vectors in 4MV mode (§7.6.1.6).
const ROUNDTAB_16: [i32; 16] = [0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 2, 2];

/// An 8-bit sample plane at macroblock-aligned dimensions (`w × h`).
#[derive(Clone, Debug)]
pub(crate) struct Plane {
    pub w: usize,
    pub h: usize,
    pub data: Vec<u8>,
}

impl Plane {
    pub fn new(w: usize, h: usize, fill: u8) -> Self {
        Self { w, h, data: vec![fill; w * h] }
    }

    /// Sample at `(x, y)`, clamped to the plane boundaries (`[0, w - 1]` × `[0, h - 1]`).
    #[inline]
    pub fn at(&self, x: i32, y: i32) -> i32 {
        let cx = x.clamp(0, self.w as i32 - 1) as usize;
        let cy = y.clamp(0, self.h as i32 - 1) as usize;
        i32::from(self.data[cy * self.w + cx])
    }

    /// Crop the top-left `crop_w × crop_h` samples into a tightly packed vector.
    pub fn crop(&self, crop_w: usize, crop_h: usize) -> Vec<u8> {
        let mut out = Vec::with_capacity(crop_w * crop_h);
        for y in 0..crop_h {
            let row = &self.data[y * self.w..y * self.w + crop_w];
            out.extend_from_slice(row);
        }
        out
    }
}

/// Per-macroblock metadata retained for spatial MV prediction and B-VOP direct mode.
#[derive(Clone, Copy, Debug)]
pub(crate) struct MbMeta {
    /// True if the macroblock was coded with `not_coded == 1` in a P/S-VOP.
    pub not_coded: bool,
    /// True if the macroblock was intra-coded.
    pub intra: bool,
    /// Motion vectors of the 4 8×8 luminance blocks (in half-pel units, or quarter-pel when `quarter_sample` is on).
    /// For 1MV macroblocks, all four entries hold the same vector; for intra/skipped, `(0, 0)`.
    pub mvs: [(i32, i32); 4],
    /// Video packet slice index (`0, 1, ...`), used to isolate prediction across resync markers.
    pub slice_id: usize,
}

impl Default for MbMeta {
    fn default() -> Self {
        Self { not_coded: true, intra: false, mvs: [(0, 0); 4], slice_id: 0 }
    }
}

/// A decoded macroblock-aligned frame held as a reference or ready for display output.
#[derive(Clone, Debug)]
pub(crate) struct RefFrame {
    pub width: u32,
    pub height: u32,
    pub y: Plane,
    pub u: Plane,
    pub v: Plane,
    pub mbs: Vec<MbMeta>,
    pub vop_type: VopType,
    pub time_ticks: i64,
    pub pts: i64,
    pub top_field_first: bool,
}

impl RefFrame {
    pub fn new(width: u32, height: u32, mb_w: usize, mb_h: usize, vop_type: VopType, time_ticks: i64, pts: i64) -> Self {
        Self {
            width,
            height,
            y: Plane::new(mb_w * 16, mb_h * 16, 0),
            u: Plane::new(mb_w * 8, mb_h * 8, 128),
            v: Plane::new(mb_w * 8, mb_h * 8, 128),
            mbs: vec![MbMeta::default(); mb_w * mb_h],
            vop_type,
            time_ticks,
            pts,
            top_field_first: false,
        }
    }
}

#[inline]
fn median(a: i32, b: i32, c: i32) -> i32 {
    a.max(b).min(a.min(b).max(c))
}

/// Fetch candidate motion vector from macroblock `(cx, cy)`, block `cb`, returning `None` if
/// `(cx, cy)` lies outside the VOP or belongs to a different video packet (`slice_id`).
#[inline]
fn cand_mv(mbs: &[MbMeta], mb_w: usize, mb_h: usize, cx: isize, cy: isize, cb: usize, slice_id: usize) -> Option<(i32, i32)> {
    if cx < 0 || cy < 0 || (cx as usize) >= mb_w || (cy as usize) >= mb_h {
        return None;
    }
    let idx = (cy as usize) * mb_w + (cx as usize);
    let m = &mbs[idx];
    if m.slice_id != slice_id {
        return None;
    }
    Some(m.mvs[cb])
}

/// Combine three optional spatial motion vector candidates `(A, B, C)` according to ISO/IEC 14496-2 §7.6.5:
/// if exactly one candidate is valid (inside VOP and same video packet), that candidate is used;
/// otherwise invalid candidates become `(0, 0)` and the component-wise median is taken.
#[inline]
fn combine_mv_candidates(a: Option<(i32, i32)>, b: Option<(i32, i32)>, c: Option<(i32, i32)>) -> (i32, i32) {
    match (a, b, c) {
        (Some(v), None, None) | (None, Some(v), None) | (None, None, Some(v)) => v,
        _ => {
            let va = a.unwrap_or((0, 0));
            let vb = b.unwrap_or((0, 0));
            let vc = c.unwrap_or((0, 0));
            (median(va.0, vb.0, vc.0), median(va.1, vb.1, vc.1))
        }
    }
}

/// Spatial motion vector predictor for 1MV macroblock `(mx, my)` or block `b` (`0..4`) in 4MV mode (§7.6.5, Figure 7-16).
pub(crate) fn mv_predictor(mbs: &[MbMeta], grid: (usize, usize), mb: (usize, usize), block: usize, slice_id: usize) -> (i32, i32) {
    let (mb_w, mb_h) = grid;
    let (mx, my) = mb;
    let (ix, iy) = (mx as isize, my as isize);
    let cur = my * mb_w + mx;
    let (a, b, c) = match block {
        0 => (
            cand_mv(mbs, mb_w, mb_h, ix - 1, iy, 1, slice_id),
            cand_mv(mbs, mb_w, mb_h, ix, iy - 1, 2, slice_id),
            cand_mv(mbs, mb_w, mb_h, ix + 1, iy - 1, 2, slice_id),
        ),
        1 => (Some(mbs[cur].mvs[0]), cand_mv(mbs, mb_w, mb_h, ix, iy - 1, 3, slice_id), cand_mv(mbs, mb_w, mb_h, ix + 1, iy - 1, 2, slice_id)),
        2 => (cand_mv(mbs, mb_w, mb_h, ix - 1, iy, 3, slice_id), Some(mbs[cur].mvs[0]), Some(mbs[cur].mvs[1])),
        _ => (Some(mbs[cur].mvs[2]), Some(mbs[cur].mvs[0]), Some(mbs[cur].mvs[1])),
    };
    combine_mv_candidates(a, b, c)
}

/// Convert a 16×16 luminance motion vector to a 4:2:0 chrominance motion vector in half-sample units (§7.6.2.2, §7.6.2.3).
/// When `quarter_sample` is true, each luma component is first divided by 2 (truncated towards zero) to half-sample units.
#[inline]
pub(crate) fn chroma_mv_1mv(mv: (i32, i32), quarter_sample: bool) -> (i32, i32) {
    let h = if quarter_sample { (mv.0 / 2, mv.1 / 2) } else { mv };
    ((h.0 >> 1) | (h.0 & 1), (h.1 >> 1) | (h.1 & 1))
}

#[inline]
fn div_round_16(sum: i32) -> i32 {
    if sum == 0 {
        return 0;
    }
    let abs = sum.abs();
    let mag = (abs >> 4) * 2 + ROUNDTAB_16[(abs & 0xF) as usize];
    if sum < 0 { -mag } else { mag }
}

/// Convert four 8×8 luminance motion vectors to a 4:2:0 chrominance motion vector in half-sample units (Table 7-6, §7.6.1.6, §7.6.2.3).
#[inline]
pub(crate) fn chroma_mv_4mv(mvs: &[(i32, i32); 4], quarter_sample: bool) -> (i32, i32) {
    let mut sx = 0i32;
    let mut sy = 0i32;
    for &(vx, vy) in mvs {
        let hx = if quarter_sample { vx / 2 } else { vx };
        let hy = if quarter_sample { vy / 2 } else { vy };
        sx += hx;
        sy += hy;
    }
    (div_round_16(sx), div_round_16(sy))
}

/// Half-sample bilinear motion-compensated prediction of a `size × size` block at `(x, y)` (§7.6.2.1).
pub(crate) fn predict_halfpel(plane: &Plane, x: i32, y: i32, mv: (i32, i32), size: usize, rounding: bool, out: &mut [u8]) {
    let (ix, iy) = (x + (mv.0 >> 1), y + (mv.1 >> 1));
    let (hx, hy) = (mv.0 & 1, mv.1 & 1);
    let r = i32::from(rounding);
    for j in 0..size {
        for i in 0..size {
            let (sx, sy) = (ix + i as i32, iy + j as i32);
            let a = plane.at(sx, sy);
            let v = match (hx, hy) {
                (0, 0) => a,
                (1, 0) => (a + plane.at(sx + 1, sy) + 1 - r) >> 1,
                (0, _) => (a + plane.at(sx, sy + 1) + 1 - r) >> 1,
                _ => (a + plane.at(sx + 1, sy) + plane.at(sx, sy + 1) + plane.at(sx + 1, sy + 1) + 2 - r) >> 2,
            };
            out[j * size + i] = v as u8;
        }
    }
}

/// Block-edge mirror index for the 8-tap quarter-sample filter over `0..=limit` (§7.6.2.3).
#[inline]
fn mirror_idx(idx: isize, limit: isize) -> usize {
    if idx < 0 {
        (-idx - 1).min(limit) as usize
    } else if idx > limit {
        (2 * limit - idx + 1).max(0) as usize
    } else {
        idx as usize
    }
}

/// 1D 8-tap FIR half-sample interpolation `[-1, 3, -6, 20, 20, -6, 3, -1]` with block-edge mirroring (§7.6.2.3).
#[inline]
fn fir_halfpel_1d(samples: &[i32], count: usize, rounding: i32, out: &mut [i32]) {
    let limit = count as isize;
    for k in 0..count {
        let ik = k as isize;
        let s0 = samples[k] + samples[k + 1];
        let s1 = samples[mirror_idx(ik - 1, limit)] + samples[mirror_idx(ik + 2, limit)];
        let s2 = samples[mirror_idx(ik - 2, limit)] + samples[mirror_idx(ik + 3, limit)];
        let s3 = samples[mirror_idx(ik - 3, limit)] + samples[mirror_idx(ik + 4, limit)];
        out[k] = ((20 * s0 - 6 * s1 + 3 * s2 - s3 + 16 - rounding) >> 5).clamp(0, 255);
    }
}

/// Quarter-sample luminance motion-compensated prediction of a `size × size` (`16` or `8`) block
/// at `(x, y)` with quarter-pel motion vector `mv` (ISO/IEC 14496-2 §7.6.2.3).
pub(crate) fn predict_qpel(plane: &Plane, x: i32, y: i32, mv: (i32, i32), size: usize, rounding: bool, out: &mut [u8]) {
    let (ix, iy) = (x + (mv.0 >> 2), y + (mv.1 >> 2));
    let (qx, qy) = (mv.0 & 3, mv.1 & 3);
    let r = i32::from(rounding);

    if qx == 0 && qy == 0 {
        for j in 0..size {
            for i in 0..size {
                out[j * size + i] = plane.at(ix + i as i32, iy + j as i32) as u8;
            }
        }
        return;
    }

    // Fetch (size + 1) x (size + 1) integer grid.
    let stride = size + 1;
    let mut grid = [0i32; 17 * 17];
    for j in 0..=size {
        for i in 0..=size {
            grid[j * stride + i] = plane.at(ix + i as i32, iy + j as i32);
        }
    }

    // Horizontal pass producing `size` columns across `size + 1` rows.
    let mut h_rows = [0i32; 17 * 16];
    let mut half_row = [0i32; 16];
    for j in 0..=size {
        let row = &grid[j * stride..j * stride + stride];
        match qx {
            0 => {
                for i in 0..size {
                    h_rows[j * size + i] = row[i];
                }
            }
            2 => {
                fir_halfpel_1d(row, size, r, &mut half_row[..size]);
                h_rows[j * size..j * size + size].copy_from_slice(&half_row[..size]);
            }
            1 => {
                fir_halfpel_1d(row, size, r, &mut half_row[..size]);
                for i in 0..size {
                    h_rows[j * size + i] = (row[i] + half_row[i] + 1 - r) >> 1;
                }
            }
            _ => {
                fir_halfpel_1d(row, size, r, &mut half_row[..size]);
                for i in 0..size {
                    h_rows[j * size + i] = (row[i + 1] + half_row[i] + 1 - r) >> 1;
                }
            }
        }
    }

    if qy == 0 {
        for j in 0..size {
            for i in 0..size {
                out[j * size + i] = h_rows[j * size + i] as u8;
            }
        }
        return;
    }

    // Vertical pass over each of the `size` columns.
    let mut col = [0i32; 17];
    let mut half_col = [0i32; 16];
    for i in 0..size {
        for j in 0..=size {
            col[j] = h_rows[j * size + i];
        }
        fir_halfpel_1d(&col[..=size], size, r, &mut half_col[..size]);
        for j in 0..size {
            let v = match qy {
                2 => half_col[j],
                1 => (col[j] + half_col[j] + 1 - r) >> 1,
                _ => (col[j + 1] + half_col[j] + 1 - r) >> 1,
            };
            out[j * size + i] = v as u8;
        }
    }
}

/// Predict a luminance block (`size` = 16 or 8) using either quarter-pel or half-pel interpolation.
#[inline]
pub(crate) fn predict_luma(plane: &Plane, origin: (i32, i32), mv: (i32, i32), size: usize, quarter_sample: bool, rounding: bool, out: &mut [u8]) {
    if quarter_sample {
        predict_qpel(plane, origin.0, origin.1, mv, size, rounding, out);
    } else {
        predict_halfpel(plane, origin.0, origin.1, mv, size, rounding, out);
    }
}

/// Average two predicted sample buffers (`(a + b + 1) >> 1`, ISO/IEC 14496-2 §7.6.9).
#[inline]
pub(crate) fn average_pred(a: &[u8], b: &[u8], out: &mut [u8]) {
    for ((o, &va), &vb) in out.iter_mut().zip(a).zip(b) {
        *o = ((u16::from(va) + u16::from(vb) + 1) >> 1) as u8;
    }
}
