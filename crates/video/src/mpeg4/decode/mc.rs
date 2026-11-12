//! Motion-compensated prediction: half-sample (§7.6.2.1) and quarter-sample (§7.6.2.2)
//! interpolation, field references, and the warping of global motion compensation (§7.8).
//!
//! Reference samples outside the decoded picture repeat the nearest edge sample (§7.6.4), so
//! every vector is valid and reads are clamped, never out of bounds. The edge is that of the whole
//! macroblocks, not the VOP's width and height, as Xvid and FFmpeg pad their references.

/// One plane of a decoded picture, sized to whole macroblocks.
#[derive(Clone, Debug)]
pub(super) struct Plane {
    pub data: Vec<u8>,
    pub stride: usize,
    pub rows: usize,
}

impl Plane {
    pub(super) fn new(stride: usize, rows: usize) -> Plane {
        Plane { data: vec![128; stride * rows], stride, rows }
    }

    /// The whole plane (`field` None) or one field of it (0 top, 1 bottom) as a reference.
    pub(super) fn view(&self, field: Option<usize>) -> View<'_> {
        View { data: &self.data, stride: self.stride, w: self.stride.max(1) as i32, h: self.rows.max(1) as i32, field: field.map(|f| f as i32) }
    }
}

/// A reference picture or field, read with clamped coordinates. A field is a view of the padded
/// frame: its lines past the picture's edge repeat the frame's edge line, whichever field that
/// line belongs to.
#[derive(Clone, Copy)]
pub(super) struct View<'a> {
    data: &'a [u8],
    stride: usize,
    w: i32,
    h: i32,
    field: Option<i32>,
}

impl View<'_> {
    #[inline]
    fn at(&self, x: i32, y: i32) -> i32 {
        let y = match self.field {
            None => y,
            Some(p) => y.saturating_mul(2).saturating_add(p),
        };
        let x = x.clamp(0, self.w - 1) as usize;
        let y = y.clamp(0, self.h - 1) as usize;
        i32::from(self.data.get(y * self.stride + x).copied().unwrap_or(128))
    }

    /// The `w`×`h` samples (at most 17×17) from `(x, y)`, one row of `out` per row.
    fn fetch(&self, x: i32, y: i32, w: usize, h: usize, out: &mut Block) {
        let w = w.min(17);
        let inside = x >= 0 && i64::from(x) + w as i64 <= i64::from(self.w);
        for (j, full) in out.iter_mut().take(h).enumerate() {
            let dst = &mut full[..w];
            let yy = y.saturating_add(j as i32);
            let yy = match self.field {
                None => yy,
                Some(p) => yy.saturating_mul(2).saturating_add(p),
            };
            let row = yy.clamp(0, self.h - 1) as usize * self.stride;
            // Rows the block covers whole are copied; only blocks over the left or right edge clamp
            // each sample.
            match self.data.get(row..row + self.stride) {
                Some(line) if inside => {
                    let x = x as usize;
                    // A fixed-length copy where the line allows: the compiler inlines it.
                    match line.get(x..x + 17) {
                        Some(src) => full.copy_from_slice(src),
                        None => dst.copy_from_slice(&line[x..x + w]),
                    }
                }
                Some(line) => {
                    for (i, d) in dst.iter_mut().enumerate() {
                        let xx = x.saturating_add(i as i32).clamp(0, self.w - 1) as usize;
                        *d = line.get(xx).copied().unwrap_or(128);
                    }
                }
                None => dst.fill(128),
            }
        }
    }
}

/// Reference samples of one block and its interpolation margin, row by row.
type Block = [[u8; 17]; 17];

impl<'a> View<'a> {
    /// The 17-sample lines from `(x, y)` of a block of `h` ≤ 17 rows, read in place when the block
    /// lies inside the reference, else fetched with clamping into `buf`.
    fn lines<'b>(&self, x: i32, y: i32, w: usize, h: usize, buf: &'b mut Block) -> [&'b [u8; 17]; 17]
    where
        'a: 'b,
    {
        let h = h.min(17);
        let (step, base) = match self.field {
            None => (1, i64::from(y)),
            Some(p) => (2, 2 * i64::from(y) + i64::from(p)),
        };
        let last = base + step * (h as i64 - 1);
        let inside = x >= 0
            && i64::from(x) + 17 <= i64::from(self.w)
            && base >= 0
            && last < i64::from(self.h)
            && (last as usize) * self.stride + x as usize + 17 <= self.data.len();
        if inside {
            let data: &'b [u8] = self.data;
            let (x, base, step) = (x as usize, base as usize, step as usize);
            let mut out = [&buf[0]; 17];
            for (j, o) in out.iter_mut().enumerate().take(h) {
                let at = (base + step * j) * self.stride + x;
                if let Some(line) = data.get(at..at + 17).and_then(|l| <&[u8; 17]>::try_from(l).ok()) {
                    *o = line;
                }
            }
            return out;
        }
        self.fetch(x, y, w, h, buf);
        std::array::from_fn(|j| &buf[j])
    }
}

/// A block of predicted samples: `w`×`h` at `out[off..]` with stride `stride`.
pub(super) struct Dst<'a> {
    pub out: &'a mut [u8],
    pub stride: usize,
    pub w: usize,
    pub h: usize,
}

impl Dst<'_> {
    #[inline]
    fn set(&mut self, x: usize, y: usize, v: i32) {
        if let Some(p) = self.out.get_mut(y * self.stride + x) {
            *p = v.clamp(0, 255) as u8;
        }
    }

    /// Store the rows of a `w`×`h` block (at most the destination's size), each row computed by
    /// `f` from its index into a 16-sample line.
    #[inline]
    fn rows(&mut self, w: usize, h: usize, mut f: impl FnMut(usize, &mut [u8; 16])) {
        let (w, stride) = (w.min(self.w).min(16), self.stride.max(1));
        let mut line = [0u8; 16];
        for (j, row) in self.out.chunks_mut(stride).take(h.min(self.h)).enumerate() {
            f(j, &mut line);
            match (w, row.get_mut(..w)) {
                (16, Some(r)) => r.copy_from_slice(&line),
                (8, Some(r)) => r.copy_from_slice(&line[..8]),
                _ => {
                    let n = w.min(row.len());
                    row[..n].copy_from_slice(&line[..n]);
                }
            }
        }
    }
}

/// Half-sample bilinear prediction of `dst` from `(x, y)` displaced by `mv` (half samples).
pub(super) fn half(src: View, x: i32, y: i32, mv: [i32; 2], rounding: bool, dst: &mut Dst) {
    let (ix, iy) = (x.saturating_add(mv[0] >> 1), y.saturating_add(mv[1] >> 1));
    let (fx, fy) = (mv[0] & 1 != 0, mv[1] & 1 != 0);
    let (w, h) = (dst.w.min(16), dst.h.min(16));
    let mut buf: Block = [[0; 17]; 17];
    let s = src.lines(ix, iy, w + 1, h + 1, &mut buf);
    let r = u16::from(rounding);
    let u = |v: u8| u16::from(v);
    // The bilinear averages stay within 0..=255, so the narrowing casts are exact.
    match (fx, fy) {
        (false, false) => dst.rows(w, h, |j, o| o.copy_from_slice(&s[j][..16])),
        (true, false) => dst.rows(w, h, |j, o| {
            let a = s[j];
            for i in 0..16 {
                o[i] = ((u(a[i]) + u(a[i + 1]) + 1 - r) >> 1) as u8;
            }
        }),
        (false, true) => dst.rows(w, h, |j, o| {
            let (a, b) = (s[j], s[(j + 1).min(16)]);
            for i in 0..16 {
                o[i] = ((u(a[i]) + u(b[i]) + 1 - r) >> 1) as u8;
            }
        }),
        (true, true) => dst.rows(w, h, |j, o| {
            let (a, b) = (s[j], s[(j + 1).min(16)]);
            for i in 0..16 {
                o[i] = ((u(a[i]) + u(a[i + 1]) + u(b[i]) + u(b[i + 1]) + 2 - r) >> 2) as u8;
            }
        }),
    }
}

/// Indices of a line of `n + 1` samples padded with three mirrored samples before and four after
/// (§7.6.2.2): entry `k` holds sample `k - 3`, `p(-1 - i)` before the line and `p(2n + 1 - i)`
/// after it.
const fn mirror_index(n: usize) -> [usize; 24] {
    let mut m = [0usize; 24];
    let mut k = 0;
    while k < 24 {
        let (i, n) = (k as isize - 3, n as isize);
        let v = if i < 0 {
            -1 - i
        } else if i > n {
            2 * n + 1 - i
        } else {
            i
        };
        m[k] = if v < 0 {
            0
        } else if v > n {
            n as usize
        } else {
            v as usize
        };
        k += 1;
    }
    m
}

const MIRROR_8: [usize; 24] = mirror_index(8);
const MIRROR_16: [usize; 24] = mirror_index(16);

fn mirror_for(n: usize) -> &'static [usize; 24] {
    if n > 8 { &MIRROR_16 } else { &MIRROR_8 }
}

/// The 8-tap half-sample filter of §7.6.2.2 on eight consecutive samples, between the fourth and
/// the fifth.
#[inline(always)]
fn qtap(t: [i32; 8], r: i32) -> i32 {
    let v = 20 * (t[3] + t[4]) - 6 * (t[2] + t[5]) + 3 * (t[1] + t[6]) - (t[0] + t[7]);
    ((v + 16 - r) >> 5).clamp(0, 255)
}

/// Quarter-sample luminance prediction of an `n`×`n` block (`n` 8 or 16), `mv` in quarter
/// samples. The filter mirrors the samples at the block's edges.
pub(super) fn quarter(src: View, x: i32, y: i32, mv: [i32; 2], rounding: bool, dst: &mut Dst) {
    let n = dst.w.min(16);
    let rows = dst.h.min(16);
    let (ix, iy) = (x.saturating_add(mv[0] >> 2), y.saturating_add(mv[1] >> 2));
    let (dx, dy) = ((mv[0] & 3) as usize, (mv[1] & 3) as usize);
    let r = i32::from(rounding);
    let mut buf: Block = [[0; 17]; 17];
    let s = src.lines(ix, iy, n + 1, rows + 1, &mut buf);
    // Horizontal pass over rows + 1 rows: the value at x + dx/4.
    let mut t = [[0i32; 16]; 17];
    let hm = mirror_for(n);
    for (&row, out) in s.iter().zip(t.iter_mut()).take(rows + 1) {
        if dx == 0 {
            for (o, &p) in out.iter_mut().zip(row) {
                *o = i32::from(p);
            }
            continue;
        }
        let mut e = [0i32; 24];
        for (e, &k) in e.iter_mut().zip(hm) {
            *e = i32::from(row[k.min(16)]);
        }
        // e[i + 3] is sample i of the row.
        for i in 0..16 {
            let h = qtap([e[i], e[i + 1], e[i + 2], e[i + 3], e[i + 4], e[i + 5], e[i + 6], e[i + 7]], r);
            out[i] = match dx {
                1 => (e[i + 3] + h + 1 - r) >> 1,
                2 => h,
                _ => (h + e[i + 4] + 1 - r) >> 1,
            };
        }
    }
    if dy == 0 {
        dst.rows(n, rows, |j, o| {
            for (o, &v) in o.iter_mut().zip(&t[j]) {
                *o = v.clamp(0, 255) as u8;
            }
        });
        return;
    }
    // Vertical pass, mirrored at the block's rows.
    let vm = mirror_for(rows);
    dst.rows(n, rows, |j, o| {
        let row = |d: usize| &t[vm[(j + d).min(23)].min(16)];
        let (t0, t1, t2, t3, t4, t5, t6, t7) = (row(0), row(1), row(2), row(3), row(4), row(5), row(6), row(7));
        for i in 0..16 {
            let f = qtap([t0[i], t1[i], t2[i], t3[i], t4[i], t5[i], t6[i], t7[i]], r);
            let v = match dy {
                1 => (t3[i] + f + 1 - r) >> 1,
                2 => f,
                _ => (f + t4[i] + 1 - r) >> 1,
            };
            o[i] = v.clamp(0, 255) as u8;
        }
    });
}

/// Chrominance vector (half samples) of a luminance vector of one 16×16 or field block.
pub(super) fn chroma_mv_1(mv: [i32; 2], quarter_sample: bool) -> [i32; 2] {
    mv.map(|v| {
        let v = if quarter_sample { v / 2 } else { v };
        (v >> 1) | (v & 1)
    })
}

/// Chrominance vector of four 8×8 luminance vectors (§7.6.3, Table 7-6 rounding).
pub(super) fn chroma_mv_4(mvs: &[[i32; 2]; 4], quarter_sample: bool) -> [i32; 2] {
    const ROUND: [i32; 16] = [0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 2, 2];
    // The sum is taken in `i64`: four extreme vectors exceed `i32`.
    std::array::from_fn(|c| {
        let sum: i64 = mvs.iter().map(|m| i64::from(if quarter_sample { m[c] / 2 } else { m[c] })).sum();
        let a = sum.unsigned_abs();
        let v = i64::from(ROUND[(a & 15) as usize]) + ((a >> 4) << 1) as i64;
        let v = if sum < 0 { -v } else { v };
        v.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
    })
}

/// Luminance prediction of a block at `(x, y)` with `mv` in the VOP's vector units.
pub(super) fn luma(src: View, x: i32, y: i32, mv: [i32; 2], quarter_sample: bool, rounding: bool, dst: &mut Dst) {
    if quarter_sample {
        quarter(src, x, y, mv, rounding, dst);
    } else {
        half(src, x, y, mv, rounding, dst);
    }
}

/// Global motion compensation parameters of an S-VOP (§7.8.4), in 1/`s` sample units.
#[derive(Clone, Debug)]
pub(super) struct Gmc {
    /// log2 of `s`, the warping precision (1 to 4).
    pub shift: u32,
    pub points: u8,
    /// `(i0', j0')`.
    pub p0: [i64; 2],
    /// `(i1'', j1'')` and `(i2'', j2'')`, the virtual points.
    pub v1: [i64; 2],
    pub v2: [i64; 2],
    /// log2 of the virtual width and height, `W'` and `H'`.
    pub alpha: u32,
    pub beta: u32,
    pub rounding: bool,
}

/// Division with rounding to the nearest integer, halves away from zero (the "//" of the
/// standard).
pub(super) fn div_round(a: i64, b: i64) -> i64 {
    if b == 0 {
        return 0;
    }
    let h = b.abs() / 2;
    if (a < 0) != (b < 0) { (a - h) / b } else { (a + h) / b }
}

/// Floor division by a power of two with rounding to nearest, halves up.
#[inline]
fn shr_round(a: i64, shift: u32) -> i64 {
    if shift == 0 { a } else { (a + (1 << (shift - 1))) >> shift }
}

impl Gmc {
    /// Set up the warp of a `width`×`height` VOP from the decoded trajectory.
    pub(super) fn new(width: u32, height: u32, points: u8, accuracy: u8, warp: &[(i32, i32); 3], rounding: bool) -> Gmc {
        let shift = u32::from(accuracy) + 1;
        let s = 1i64 << shift;
        let r = 16 / s;
        let (w, h) = (i64::from(width), i64::from(height));
        let d = |k: usize| [i64::from(warp[k].0), i64::from(warp[k].1)];
        let (d0, d1, d2) = (d(0), d(1), d(2));
        // i0' = (s/2)(2·i0 + du0), with (i0, j0) = (0, 0), (i1, j1) = (W, 0), (i2, j2) = (0, H).
        let half = s / 2;
        let p0 = [half * d0[0], half * d0[1]];
        let p1 = [half * (2 * w + d0[0] + d1[0]), half * (d0[1] + d1[1])];
        let p2 = [half * (d0[0] + d2[0]), half * (2 * h + d0[1] + d2[1])];
        let alpha = (0..).find(|&a| 1i64 << a >= w).unwrap_or(13);
        let beta = (0..).find(|&b| 1i64 << b >= h).unwrap_or(13);
        let (w2, h2) = (1i64 << alpha, 1i64 << beta);
        let v1 =
            [16 * w2 + div_round((w - w2) * (r * p0[0]) + w2 * (r * p1[0] - 16 * w), w), div_round((w - w2) * (r * p0[1]) + w2 * (r * p1[1]), w)];
        let v2 =
            [div_round((h - h2) * (r * p0[0]) + h2 * (r * p2[0]), h), 16 * h2 + div_round((h - h2) * (r * p0[1]) + h2 * (r * p2[1] - 16 * h), h)];
        Gmc { shift, points, p0, v1, v2, alpha, beta, rounding }
    }

    fn r(&self) -> i64 {
        16 >> self.shift
    }

    /// Warped position of luminance sample `(i, j)`, in 1/s units.
    pub(super) fn luma_pos(&self, i: i64, j: i64) -> [i64; 2] {
        let s = 1i64 << self.shift;
        let r = self.r();
        let [i0, j0] = self.p0;
        match self.points {
            0 | 1 => [i0 + s * i, j0 + s * j],
            2 => {
                let a = -r * i0 + self.v1[0];
                let b = r * j0 - self.v1[1];
                let c = -r * j0 + self.v1[1];
                let sh = self.alpha + 4 - self.shift;
                [i0 + shr_round(a * i + b * j, sh), j0 + shr_round(c * i + a * j, sh)]
            }
            _ => {
                let (w2, h2) = (1i64 << self.alpha, 1i64 << self.beta);
                let sh = self.alpha + self.beta + 4 - self.shift;
                [
                    i0 + shr_round((-r * i0 + self.v1[0]) * h2 * i + (-r * i0 + self.v2[0]) * w2 * j, sh),
                    j0 + shr_round((-r * j0 + self.v1[1]) * h2 * i + (-r * j0 + self.v2[1]) * w2 * j, sh),
                ]
            }
        }
    }

    /// Warped position of chrominance sample `(ic, jc)`, in 1/s chroma units.
    pub(super) fn chroma_pos(&self, ic: i64, jc: i64) -> [i64; 2] {
        let s = 1i64 << self.shift;
        let r = self.r();
        let [i0, j0] = self.p0;
        match self.points {
            0 | 1 => {
                let half = |v: i64| (v >> 1) | (v & 1);
                [half(i0) + s * ic, half(j0) + s * jc]
            }
            2 => {
                let w2 = 1i64 << self.alpha;
                let a = -r * i0 + self.v1[0];
                let b = r * j0 - self.v1[1];
                let c = -r * j0 + self.v1[1];
                let sh = self.alpha + 6 - self.shift;
                let (x, y) = (4 * ic + 1, 4 * jc + 1);
                [shr_round(a * x + b * y + 2 * w2 * r * i0 - 16 * w2, sh), shr_round(c * x + a * y + 2 * w2 * r * j0 - 16 * w2, sh)]
            }
            _ => {
                let (w2, h2) = (1i64 << self.alpha, 1i64 << self.beta);
                let sh = self.alpha + self.beta + 6 - self.shift;
                let (x, y) = (4 * ic + 1, 4 * jc + 1);
                let k = 2 * w2 * h2;
                [
                    shr_round((-r * i0 + self.v1[0]) * h2 * x + (-r * i0 + self.v2[0]) * w2 * y + k * r * i0 - 16 * w2 * h2, sh),
                    shr_round((-r * j0 + self.v1[1]) * h2 * x + (-r * j0 + self.v2[1]) * w2 * y + k * r * j0 - 16 * w2 * h2, sh),
                ]
            }
        }
    }

    /// Bilinear sample of `src` at `pos` (1/s units).
    fn sample(&self, src: View, pos: [i64; 2]) -> i32 {
        let s = 1i64 << self.shift;
        let clampc = |v: i64| v.clamp(-(1 << 20), 1 << 20) as i32;
        let (ip, jp) = (clampc(pos[0] >> self.shift), clampc(pos[1] >> self.shift));
        let (ri, rj) = ((pos[0] & (s - 1)) as i32, (pos[1] & (s - 1)) as i32);
        let s = s as i32;
        let top = (s - ri) * src.at(ip, jp) + ri * src.at(ip + 1, jp);
        let bot = (s - ri) * src.at(ip, jp + 1) + ri * src.at(ip + 1, jp + 1);
        let v = (s - rj) * top + rj * bot;
        (v + (s * s) / 2 - i32::from(self.rounding)) >> (2 * self.shift)
    }

    /// Predict the luminance (`chroma` false, 16×16) or a chrominance (8×8) block of macroblock
    /// `(mx, my)`.
    pub(super) fn predict(&self, src: View, mx: usize, my: usize, chroma: bool, dst: &mut Dst) {
        let n = if chroma { 8 } else { 16 };
        for j in 0..n {
            for i in 0..n {
                let (x, y) = ((mx * n + i) as i64, (my * n + j) as i64);
                let pos = if chroma { self.chroma_pos(x, y) } else { self.luma_pos(x, y) };
                dst.set(i, j, self.sample(src, pos));
            }
        }
    }

    /// The vector of a GMC macroblock as a neighbour's predictor and for direct mode: the mean
    /// warped displacement of its luminance samples, in half or quarter samples, limited to the
    /// range of `fcode`.
    pub(super) fn average_mv(&self, mx: usize, my: usize, quarter_sample: bool, fcode: u8) -> [i32; 2] {
        let s = 1i64 << self.shift;
        let mut sum = [0i64; 2];
        for j in 0..16 {
            for i in 0..16 {
                let (x, y) = ((mx * 16 + i) as i64, (my * 16 + j) as i64);
                let p = self.luma_pos(x, y);
                sum[0] += p[0] - s * x;
                sum[1] += p[1] - s * y;
            }
        }
        let unit = if quarter_sample { 4 } else { 2 };
        let len = 16i64 << fcode;
        sum.map(|v| div_round(v * unit, 256 * s).clamp(-len, len - 1) as i32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plane(w: usize, h: usize, f: impl Fn(usize, usize) -> u8) -> Plane {
        let mut p = Plane::new(w, h);
        for y in 0..h {
            for x in 0..w {
                p.data[y * w + x] = f(x, y);
            }
        }
        p
    }

    #[test]
    fn half_sample_averages_with_rounding_control() {
        let p = plane(32, 32, |x, _| (x * 3) as u8);
        let mut out = [0u8; 4];
        let mut d = Dst { out: &mut out, stride: 2, w: 2, h: 2 };
        half(p.view(None), 4, 4, [1, 0], false, &mut d);
        // (12 + 15 + 1) >> 1 = 14
        assert_eq!(out[0], 14);
        let mut d = Dst { out: &mut out, stride: 2, w: 2, h: 2 };
        half(p.view(None), 4, 4, [1, 0], true, &mut d);
        assert_eq!(out[0], 13);
    }

    #[test]
    fn reads_outside_the_picture_repeat_the_edge() {
        let p = plane(16, 16, |x, y| (x + 16 * y) as u8);
        let mut out = [0u8; 4];
        let mut d = Dst { out: &mut out, stride: 2, w: 2, h: 2 };
        half(p.view(None), 0, 0, [-1000, -1000], false, &mut d);
        assert_eq!(out, [0; 4]);
        let mut d = Dst { out: &mut out, stride: 2, w: 2, h: 2 };
        half(p.view(None), 0, 0, [i32::MAX / 4, i32::MAX / 4], false, &mut d);
        assert_eq!(out, [255; 4]);
    }

    #[test]
    fn quarter_sample_of_a_flat_area_is_flat_and_integer_positions_copy() {
        let p = plane(48, 48, |_, _| 77);
        let mut out = [0u8; 256];
        for mv in [[1, 3], [2, 2], [3, 1], [-5, 6]] {
            let mut d = Dst { out: &mut out, stride: 16, w: 16, h: 16 };
            quarter(p.view(None), 16, 16, mv, false, &mut d);
            assert!(out.iter().all(|&v| v == 77), "{mv:?}");
        }
        let p = plane(48, 48, |x, y| (x * 5 + y * 3) as u8);
        let mut d = Dst { out: &mut out, stride: 16, w: 16, h: 16 };
        quarter(p.view(None), 16, 16, [4, -8], false, &mut d);
        assert_eq!(out[0], ((17 * 5) + 14 * 3) as u8);
    }

    #[test]
    fn quarter_sample_filter_is_linear_inside_a_ramp() {
        // A horizontal ramp: the half-sample value inside the block is the midpoint.
        let p = plane(64, 32, |x, _| (x * 4) as u8);
        let mut out = [0u8; 64];
        let mut d = Dst { out: &mut out, stride: 8, w: 8, h: 8 };
        quarter(p.view(None), 16, 8, [2, 0], false, &mut d);
        // Away from the mirrored edges the filter reproduces the ramp.
        assert_eq!(out[3], (16 * 4 + 3 * 4 + 2) as u8);
    }

    #[test]
    fn chroma_vector_rounding() {
        assert_eq!(chroma_mv_1([1, 3], false), [1, 1]);
        assert_eq!(chroma_mv_1([-1, -3], false), [-1, -1]);
        assert_eq!(chroma_mv_1([4, 5], false), [2, 3]);
        assert_eq!(chroma_mv_4(&[[1, 0]; 4], false), [1, 0]);
        assert_eq!(chroma_mv_4(&[[2, -2]; 4], false), [1, -1]);
        assert_eq!(chroma_mv_4(&[[8, 0], [0, 0], [0, 0], [0, 0]], false), [1, 0]);
    }

    #[test]
    fn gmc_without_motion_is_the_identity() {
        let g = Gmc::new(64, 48, 3, 2, &[(0, 0); 3], false);
        for (i, j) in [(0, 0), (17, 5), (63, 47)] {
            assert_eq!(g.luma_pos(i, j), [8 * i, 8 * j]);
        }
        let g = Gmc::new(64, 48, 2, 3, &[(0, 0); 3], false);
        assert_eq!(g.luma_pos(10, 20), [160, 320]);
        assert_eq!(g.chroma_pos(3, 4), [48, 64]);
        assert_eq!(g.average_mv(1, 1, false, 1), [0, 0]);
    }

    #[test]
    fn gmc_translation_moves_every_sample() {
        // One point, accuracy 1/4 (s = 4): du = 3 half samples is 6 quarter samples.
        let g = Gmc::new(64, 48, 1, 1, &[(3, -2), (0, 0), (0, 0)], false);
        assert_eq!(g.luma_pos(5, 5), [20 + 6, 20 - 4]);
        assert_eq!(g.average_mv(0, 0, false, 1), [3, -2]);
        assert_eq!(g.average_mv(0, 0, true, 1), [6, -4]);
    }
}
