use super::palette::{Bin, HistKey, Used, centers, median_cut, nearest_lab};
use super::{Mode, PIXELS_PER_SHARE, Palette, Quantized, Raster, TRANSPARENT, TraceParams, luma, oklab, over_rows};
use crate::parallel_map;

/// `q` with its blend pixels settled, and the colours that leaves unused dropped.
///
/// A pixel on an edge is a blend of the colours on its two sides, and the palette colour nearest
/// to a blend can be a third colour altogether (halfway from navy to yellow lies an olive that is
/// nearer to a green elsewhere in the picture), which would trace as a sliver of that colour along
/// the edge. A blend pixel (see [`own_colours`]) whose colour is none of those of the picture's
/// own pixels within two pixels of it takes the side it is nearer to of the pair of those colours
/// it is best a blend of, when that explains it twice as well as its own colour does. With one
/// colour around it, it keeps its own.
pub(super) fn settled(q: Quantized, img: &Raster, own: &[bool]) -> Quantized {
    settled_by(q, img, own, PIXELS_PER_SHARE)
}

/// [`settled`], with the rows worked on in bands of about `per_share` pixels (see [`over_rows`]).
fn settled_by(mut q: Quantized, img: &Raster, own: &[bool], per_share: usize) -> Quantized {
    let (w, h) = (img.width as usize, img.height as usize);
    if q.labels.len() != w * h || own.len() != w * h {
        return q;
    }
    // Each pixel is settled by the labels as they were before any label changed. Each band of rows
    // keeps its changes (see [`Changes`]), and they are made once every band is done.
    let before = &q.labels;
    let color = |l: u16| q.palette.get(l as usize).map(|c| c.map(f64::from));
    let dist = |a: [f64; 3], b: [f64; 3]| (0..3).map(|k| (a[k] - b[k]).powi(2)).sum::<f64>().sqrt();
    let bands: Vec<(usize, Changes)> = over_rows(w, h, per_share, |rows| {
        let (first, end) = (rows.start * w, rows.end * w);
        let mut band = Changes::new(end - first);
        for i in first..end {
            let mine_l = before[i];
            if own[i] || mine_l == TRANSPARENT {
                continue;
            }
            let (x, y) = (i % w, i / w);
            // The colours of the picture's own pixels within two pixels (seldom more than three).
            let mut near: Vec<u16> = Vec::new();
            for yy in y.saturating_sub(2)..=(y + 2).min(h - 1) {
                for xx in x.saturating_sub(2)..=(x + 2).min(w - 1) {
                    let j = yy * w + xx;
                    if own[j] && before[j] != TRANSPARENT && !near.contains(&before[j]) {
                        near.push(before[j]);
                    }
                }
            }
            let (Some(p), Some(mine)) = (img.rgba.get(i * 4..i * 4 + 3).map(|p| [f64::from(p[0]), f64::from(p[1]), f64::from(p[2])]), color(mine_l))
            else {
                continue;
            };
            if near.len() < 2 || near.contains(&mine_l) {
                continue;
            }
            let mut best: Option<(f64, u16)> = None;
            for (n, &a) in near.iter().enumerate() {
                for &b in &near[n + 1..] {
                    let (Some(ca), Some(cb)) = (color(a), color(b)) else { continue };
                    let d = [0, 1, 2].map(|k| cb[k] - ca[k]);
                    let len2: f64 = d.iter().map(|v| v * v).sum();
                    let t = if len2 > 0.0 { ((0..3).map(|k| (p[k] - ca[k]) * d[k]).sum::<f64>() / len2).clamp(0.0, 1.0) } else { 0.0 };
                    let off = dist(p, [0, 1, 2].map(|k| ca[k] + t * d[k]));
                    if best.is_none_or(|x| off < x.0) {
                        best = Some((off, if t < 0.5 { a } else { b }));
                    }
                }
            }
            if let Some((off, to)) = best
                && off < 0.5 * dist(p, mine)
            {
                band.set(i - first, to, || before.get(first..end).unwrap_or_default().to_vec());
            }
        }
        if band.is_empty() { vec![] } else { vec![(first, band)] }
    });
    if bands.is_empty() {
        return q;
    }
    for (first, band) in bands {
        band.each(|at, to| {
            if let Some(l) = q.labels.get_mut(first + at) {
                *l = to;
            }
        });
    }
    compact(q.labels, q.palette)
}

/// The labels a pass changes in one part of a picture, `len` pixels long, counting each pixel from
/// the part's first. They are kept as pairs of a pixel and its new label until the pairs would take
/// more memory than a copy of the part's labels, and as that copy, with the changes made, after
/// that. The pairs take at most as much memory as the copy, and a part holds both only while it
/// moves the pairs into the copy.
struct Changes {
    len: usize,
    few: Vec<(usize, u16)>,
    copy: Option<Vec<u16>>,
}

impl Changes {
    fn new(len: usize) -> Self {
        Changes { len, few: vec![], copy: None }
    }

    /// Changes the label of the pixel `at` to `to`. `labels` gives the part's labels as they were
    /// before any changed, for the copy.
    fn set(&mut self, at: usize, to: u16, labels: impl FnOnce() -> Vec<u16>) {
        if let Some(copy) = &mut self.copy {
            if let Some(l) = copy.get_mut(at) {
                *l = to;
            }
            return;
        }
        // The most pairs that take no more memory than a copy of the part's labels. The pairs grow
        // by doubling up to that many, and no further.
        let most = self.len * size_of::<u16>() / size_of::<(usize, u16)>();
        if self.few.len() < most {
            if self.few.len() == self.few.capacity() {
                self.few.reserve_exact(self.few.capacity().max(4).min(most - self.few.len()));
            }
            self.few.push((at, to));
            return;
        }
        let mut copy = labels();
        for (at, to) in std::mem::take(&mut self.few).into_iter().chain([(at, to)]) {
            if let Some(l) = copy.get_mut(at) {
                *l = to;
            }
        }
        self.copy = Some(copy);
    }

    fn is_empty(&self) -> bool {
        self.few.is_empty() && self.copy.is_none()
    }

    /// `f` of each changed pixel and its new label (of every pixel of the part, once it is a copy).
    fn each(self, mut f: impl FnMut(usize, u16)) {
        match self.copy {
            Some(copy) => copy.into_iter().enumerate().for_each(|(at, to)| f(at, to)),
            None => self.few.into_iter().for_each(|(at, to)| f(at, to)),
        }
    }
}

/// The most colors (Color) or grays (Grayscale) a palette may ask for to have its blend pixels
/// settled by [`settled_wide`] rather than [`settled`]. The limited-color presets ask for 3 to 16
/// colors.
const LIMITED_COLORS: u32 = 16;

/// Whether a trace with `params` settles its blend pixels with [`settled_wide`]: Grayscale, and
/// Color with a Limited palette, of at most [`LIMITED_COLORS`] colors.
pub(super) fn settles_wide(params: &TraceParams) -> bool {
    match params.mode {
        Mode::Grayscale => params.colors <= LIMITED_COLORS,
        Mode::Color | Mode::Logo => params.palette == Palette::Limited && params.colors <= LIMITED_COLORS,
        Mode::BlackAndWhite => false,
    }
}

/// How far (px, across, along or diagonally) [`settled_wide`] looks for the picture's own pixels
/// around a blend pixel: blur spreads an edge over several pixels, and its two sides can lie this
/// far apart.
const BLEND_REACH: usize = 8;

/// How far (px, across, along or diagonally) an own pixel of its color keeps a blend pixel's color
/// in [`settled_wide`]: where three colors meet, the blend is of its color too.
const OWN_NEAR: usize = 4;

/// How many of the colors within reach of a blend pixel, nearest to its color first,
/// [`settled_wide`] pairs up.
const NEAR_COLORS: usize = 8;

/// How many columns at a time [`settled_wide`] keeps the colors within reach for, so that its
/// memory does not grow with the width of the picture.
const BLEND_STRIP: usize = 7;

/// How far apart (squared levels) the distances of a blend pixel off the lines between two pairs
/// of colors may be for [`settled_wide`] to count them as equal: rounding leaves a gray that lies
/// between two grays about 1e-28 off the line between them.
const SAME_FIT: f64 = 1e-6;

/// How much nearer (squared) to the line between a pair of colors than to its own color a blend
/// pixel must lie for [`settled_wide`] to settle it: half as far, as [`settled`] asks.
const BETTER_FIT: f64 = 0.25;

/// How far off the line between a pair of colors (as a share of their distance apart) a blend pixel
/// that lies between the two may be for [`settled_wide`] to settle it whatever its own color.
const ON_THE_LINE: f64 = 0.15;

/// [`settled`] for a limited palette (see [`settles_wide`]), which also settles the in-between
/// tones of an edge blurred over several pixels: without it they trace as a ring of a third color
/// around the edge.
///
/// A blend pixel (see [`own_colours`]) whose color is none of those of the picture's own pixels
/// within [`OWN_NEAR`] pixels of it takes the side it is nearer to of the pair of the colors of own
/// pixels within [`BLEND_REACH`] pixels that it is best a blend of (of two pairs that explain it
/// equally well, the closer one), when that explains it twice as well as its own color does, or
/// when it lies between the two and off the line between them by less than [`ON_THE_LINE`] of
/// their distance apart. Only the [`NEAR_COLORS`] colors nearest to its color are paired. It keeps
/// its own color when that color is one of the pair, or when one color is all there is around it.
/// With `gray` (Grayscale), a pixel's color is its gray (see [`luma`]).
pub(super) fn settled_wide(q: Quantized, img: &Raster, own: &[bool], gray: bool) -> Quantized {
    settled_wide_by(q, img, own, gray, PIXELS_PER_SHARE)
}

/// [`settled_wide`], with its strips of columns worked on in groups of about `per_share` pixels
/// (at least one strip), spread over the available cores (one thread on the web).
fn settled_wide_by(mut q: Quantized, img: &Raster, own: &[bool], gray: bool, per_share: usize) -> Quantized {
    let (w, h) = (img.width as usize, img.height as usize);
    if q.labels.len() != w * h || own.len() != w * h || w == 0 || h == 0 {
        return q;
    }
    // Each pixel is settled by the labels as they were before any label changed. Each group of
    // strips keeps its changes (see [`Changes`]), and they are made once every group is done.
    let strip = BLEND_STRIP.min(w).max(1);
    let per_group = (per_share / (strip * h).max(1)).max(1);
    let cols = strip.saturating_mul(per_group);
    let groups: Vec<std::ops::Range<usize>> = (0..w).step_by(cols).map(|x| x..x.saturating_add(cols).min(w)).collect();
    let settle = |cols: &std::ops::Range<usize>| (cols.clone(), settled_wide_strips(&q, img, own, gray, cols.clone()));
    let mut settled: Vec<(std::ops::Range<usize>, Changes)> =
        if groups.len() <= 1 { groups.iter().map(settle).collect() } else { parallel_map(&groups, settle) };
    settled.retain(|(_, changes)| !changes.is_empty());
    if settled.is_empty() {
        return q;
    }
    for (cols, group) in settled {
        let gw = cols.len().max(1);
        group.each(|at, to| {
            if let Some(l) = q.labels.get_mut((at / gw) * w + cols.start + at % gw) {
                *l = to;
            }
        });
    }
    compact(q.labels, q.palette)
}

/// The labels that [`settled_wide`] changes in the columns `cols` (whole strips of [`BLEND_STRIP`]
/// columns, or the picture's last columns), each pixel counted row by row across those columns.
fn settled_wide_strips(q: &Quantized, img: &Raster, own: &[bool], gray: bool, cols: std::ops::Range<usize>) -> Changes {
    let (w, h) = (img.width as usize, img.height as usize);
    let gw = cols.len();
    let before = &q.labels;
    let colors: Vec<[f64; 3]> = q.palette.iter().map(|c| c.map(f64::from)).collect();
    let dist2 = |a: [f64; 3], b: [f64; 3]| (0..3).map(|k| (a[k] - b[k]).powi(2)).sum::<f64>();
    // The colors of the own pixels within reach of each pixel, as bits (one word per 64 palette
    // colors), a strip of columns at a time: along its row for the rows within reach (a window
    // sliding along each row, kept for the last 2 × reach + 1 rows), then across those rows.
    let words = q.palette.len().div_ceil(64).max(1);
    let span = 2 * BLEND_REACH + 1;
    let strip = BLEND_STRIP.min(w).max(1);
    let pixel = |p: &[u8]| if gray { [f64::from(luma([p[0], p[1], p[2], p[3]])); 3] } else { [f64::from(p[0]), f64::from(p[1]), f64::from(p[2])] };
    let mut settled = Changes::new(gw * h);
    let mut rows = vec![0u64; span * strip * words];
    let mut counts = vec![0u32; q.palette.len()];
    let mut window = vec![0u64; words];
    let mut bits = vec![0u64; words];
    // The bits of row `y` for the columns `x0..x1` (at most `strip` of them).
    let mut row_colors = |y: usize, x0: usize, x1: usize, rows: &mut [u64]| {
        let Some(out) = rows.get_mut((y % span) * strip * words..(y % span + 1) * strip * words) else { return };
        counts.fill(0);
        window.fill(0);
        // The own pixel at `x` enters the window (`add`) or leaves it.
        let step = |counts: &mut [u32], window: &mut [u64], x: usize, add: bool| {
            let i = y * w + x;
            let l = before[i] as usize;
            if !own[i] || l >= counts.len() {
                return;
            }
            if add {
                counts[l] += 1;
                window[l / 64] |= 1 << (l % 64);
            } else {
                counts[l] -= 1;
                if counts[l] == 0 {
                    window[l / 64] &= !(1 << (l % 64));
                }
            }
        };
        let first = x0.saturating_sub(BLEND_REACH);
        for x in first..(x0 + BLEND_REACH).min(w) {
            step(&mut counts, &mut window, x, true);
        }
        for x in x0..x1 {
            if x + BLEND_REACH < w {
                step(&mut counts, &mut window, x + BLEND_REACH, true);
            }
            if let Some(gone) = x.checked_sub(BLEND_REACH + 1).filter(|&g| g >= first) {
                step(&mut counts, &mut window, gone, false);
            }
            if let Some(o) = out.get_mut((x - x0) * words..(x - x0 + 1) * words) {
                o.copy_from_slice(&window);
            }
        }
    };
    let mut near: Vec<u16> = Vec::new();
    let mut by_distance: Vec<(f64, u16)> = Vec::new();
    for (x0, y) in cols.clone().step_by(strip).flat_map(|x0| (0..h).map(move |y| (x0, y))) {
        let x1 = (x0 + strip).min(cols.end);
        if y == 0 {
            for yy in 0..BLEND_REACH.min(h) {
                row_colors(yy, x0, x1, &mut rows);
            }
        }
        if y + BLEND_REACH < h {
            row_colors(y + BLEND_REACH, x0, x1, &mut rows);
        }
        'pixels: for x in x0..x1 {
            let i = y * w + x;
            let mine_l = before[i];
            if own[i] || mine_l == TRANSPARENT {
                continue;
            }
            // Its color is the picture's own here.
            for yy in y.saturating_sub(OWN_NEAR)..=(y + OWN_NEAR).min(h - 1) {
                for xx in x.saturating_sub(OWN_NEAR)..=(x + OWN_NEAR).min(w - 1) {
                    if own[yy * w + xx] && before[yy * w + xx] == mine_l {
                        continue 'pixels;
                    }
                }
            }
            bits.fill(0);
            for yy in y.saturating_sub(BLEND_REACH)..=(y + BLEND_REACH).min(h - 1) {
                let at = (yy % span) * strip * words + (x - x0) * words;
                for (b, r) in bits.iter_mut().zip(rows.get(at..at + words).unwrap_or_default()) {
                    *b |= r;
                }
            }
            near.clear();
            for (k, &b) in bits.iter().enumerate() {
                let mut b = b;
                while b != 0 {
                    let l = (k * 64 + b.trailing_zeros() as usize) as u16;
                    b &= b - 1;
                    near.push(l);
                }
            }
            let (Some(p), Some(&mine)) = (img.rgba.get(i * 4..i * 4 + 4).map(pixel), colors.get(mine_l as usize)) else {
                continue;
            };
            if near.iter().filter(|&&l| l != mine_l).count() < 2 {
                continue;
            }
            // In busy art many colors lie within reach: only the ones nearest to its color are paired.
            // Each distance is worked out once, and only the nearest few are sorted (nearest first,
            // then by index).
            if near.len() > NEAR_COLORS {
                by_distance.clear();
                by_distance.extend(near.iter().map(|&l| (colors.get(l as usize).map_or(f64::INFINITY, |&c| dist2(p, c)), l)));
                let order = |a: &(f64, u16), b: &(f64, u16)| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1));
                by_distance.select_nth_unstable_by(NEAR_COLORS, order);
                by_distance.truncate(NEAR_COLORS);
                by_distance.sort_unstable_by(order);
                near.clear();
                near.extend(by_distance.iter().map(|&(_, l)| l));
            }
            // The pair it is best a blend of, by the square of how far it lies off the line between them,
            // and of two pairs it lies equally far off, the closer pair (in grays it lies on the line
            // between every pair of grays darker and lighter than it).
            let mut best: Option<(f64, [u16; 2], u16, f64, bool)> = None;
            for (n, &a) in near.iter().enumerate() {
                let Some(&ca) = colors.get(a as usize) else { continue };
                let pa = [p[0] - ca[0], p[1] - ca[1], p[2] - ca[2]];
                for &b in &near[n + 1..] {
                    let Some(&cb) = colors.get(b as usize) else { continue };
                    let d = [cb[0] - ca[0], cb[1] - ca[1], cb[2] - ca[2]];
                    let len2 = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
                    let along = if len2 > 0.0 { (pa[0] * d[0] + pa[1] * d[1] + pa[2] * d[2]) / len2 } else { 0.0 };
                    let t = along.clamp(0.0, 1.0);
                    let off2 = (pa[0] - t * d[0]).powi(2) + (pa[1] - t * d[1]).powi(2) + (pa[2] - t * d[2]).powi(2);
                    if best.is_none_or(|x| off2 < x.0 - SAME_FIT || (off2 <= x.0 + SAME_FIT && len2 < x.3)) {
                        best = Some((off2, [a, b], if t < 0.5 { a } else { b }, len2, along > 0.0 && along < 1.0));
                    }
                }
            }
            // A pixel already of one of the two colors it is best a blend of stays as it is.
            if let Some((off2, pair, to, apart2, between)) = best
                && !pair.contains(&mine_l)
                && (off2 < BETTER_FIT * dist2(p, mine) || (between && off2 < ON_THE_LINE * ON_THE_LINE * apart2))
            {
                settled.set(y * gw + x - cols.start, to, || {
                    (0..h).flat_map(|yy| before.get(yy * w + cols.start..yy * w + cols.end).unwrap_or_default()).copied().collect()
                });
            }
        }
    }
    settled
}

/// Whether `img` is continuous tone (a photo, a gradient) rather than flat art, however noisy
/// or unevenly lit (a scan): whether eight colours leave much of it unsaid. Flat art is a few
/// colours and what noise and lighting make of them, and eight colours come within a few levels
/// of every pixel; a photo's tones spread all through its range, and eight colours stay far from
/// most of them (more than 6 levels, and more than a tenth of how far a single colour stays).
/// A gray picture's tones lie along one line instead of through the whole color cube, and eight
/// colors come nearer to them. A neutral picture (19 in 20 of its opaque pixels with their
/// channels within 8 levels of each other) is therefore also continuous tone when eight colors
/// stay more than 0.065 as far as a single color does. So is a sepia, single-hue or duotone
/// picture whose tones follow one curve through the color cube (19 in 20 of its opaque pixels on
/// it, see [`on_one_curve`]). Neither is, however far eight colors stay, when sixteen colors stay
/// at most a tenth as far as eight do. That exception holds for flat art of up to sixteen clean
/// levels, gray or tinted, where sixteen colors come within about a level of every pixel. On gray
/// and tinted photos sixteen colors stay 0.18 to 0.57 as far as eight do, and on most anti-aliased
/// or noisy gray flat art 0.24 to 0.5 as far, so the exception holds for neither.
pub(crate) fn continuous_tone(img: &Raster) -> bool {
    let hk = HistKey::new(img);
    let mut bins = vec![Bin::default(); 1 << 15];
    let (mut opaque, mut neutral) = (0u64, 0u64);
    for p in img.rgba.as_chunks::<4>().0 {
        if p[3] >= 128 {
            bins[hk.key(p[0], p[1], p[2])].add(p);
            opaque += 1;
            if p[0].max(p[1]).max(p[2]) - p[0].min(p[1]).min(p[2]) <= 8 {
                neutral += 1;
            }
        }
    }
    let used: Vec<Used> = bins.iter().enumerate().filter(|(_, b)| b.n > 0).map(|(i, b)| (i, b.mean(), b.n as f64)).collect();
    // The root-mean-square distance (levels) from the pixels to the nearest of `k` colours.
    let unsaid = |k: usize| {
        let colors = centers(&median_cut(&used, k, 0.0));
        let (mut sum, mut n) = (0.0, 0.0);
        for (_, c, weight) in &used {
            let d2 = colors.iter().map(|m| (0..3).map(|ch| (m[ch] - c[ch]).powi(2)).sum::<f64>()).fold(f64::INFINITY, f64::min);
            sum += d2 * weight;
            n += weight;
        }
        if n > 0.0 { (sum / n).sqrt() } else { 0.0 }
    };
    let eight = unsaid(8);
    if eight <= 6.0 {
        return false;
    }
    let one = unsaid(1);
    if eight <= 0.065 * one {
        return false;
    }
    let one_line = || 20 * neutral >= 19 * opaque || 20 * on_one_curve(img) >= 19 * opaque;
    if eight > 0.1 * one {
        return !(unsaid(16) <= 0.1 * eight && one_line());
    }
    unsaid(16) > 0.1 * eight && one_line()
}

/// How far (levels) a pixel may lie from the tone curve of its picture and still count as on it,
/// for [`on_one_curve`].
const CURVE_OFF: f64 = 6.0;

/// How long (levels) each stretch along a picture's main color direction is whose median color is a
/// point of its tone curve, for [`on_one_curve`].
const CURVE_STEP: f64 = 8.0;

/// How far (levels) the colors of a picture can lie from their mean along any direction: more than
/// the length of the color cube's diagonal (255 × √3).
const CURVE_REACH: f64 = 448.0;

/// How many of the opaque pixels (an alpha of 128 or more) of `img` lie on its tone curve: within
/// [`CURVE_OFF`] of it, measured across its main color direction, and within [`CURVE_OFF`] of the
/// plane its colors spread along most. The main color direction is the direction through the color
/// cube along which the opaque pixels' colors spread the most. The tone curve goes through the
/// median color (the median of each channel) of the opaque pixels in each stretch of [`CURVE_STEP`]
/// levels along that direction, which a few pixels of another color in the stretch do not move. The
/// colors of a sepia, single-hue or duotone photo follow one such curve, as a gray photo's lie along
/// the line of grays, while a color photo's spread across it. A curve with a bend, as a duotone's,
/// stays within the plane, while art in several unrelated colors has colors off it. None are on
/// the curve when there is no main color direction (no opaque pixels, or all of them one color).
fn on_one_curve(img: &Raster) -> u64 {
    let opaque = || img.rgba.as_chunks::<4>().0.iter().filter(|p| p[3] >= 128).map(|p| [p[0], p[1], p[2]].map(f64::from));
    let (mut n, mut sum, mut products) = (0.0, [0.0; 3], [[0.0; 3]; 3]);
    for c in opaque() {
        n += 1.0;
        for i in 0..3 {
            sum[i] += c[i];
            for j in 0..3 {
                products[i][j] += c[i] * c[j];
            }
        }
    }
    if n == 0.0 {
        return 0;
    }
    let mean = sum.map(|s| s / n);
    let covariance: [[f64; 3]; 3] = std::array::from_fn(|i| std::array::from_fn(|j| products[i][j] / n - mean[i] * mean[j]));
    let Some(axis) = main_direction(&covariance) else { return 0 };
    let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    // The direction across the main one along which the colors spread the most, and the direction
    // across both: off the plane the colors spread along most. None when they lie on one line.
    let first = dot(covariance.map(|row| dot(row, axis)), axis);
    let rest: [[f64; 3]; 3] = std::array::from_fn(|i| std::array::from_fn(|j| covariance[i][j] - first * axis[i] * axis[j]));
    let off_plane = main_direction(&rest).and_then(|b| {
        let n = [axis[1] * b[2] - axis[2] * b[1], axis[2] * b[0] - axis[0] * b[2], axis[0] * b[1] - axis[1] * b[0]];
        let len = dot(n, n).sqrt();
        (len > 1e-9).then(|| n.map(|v| v / len))
    });
    let stretches = (2.0 * CURVE_REACH / CURVE_STEP) as usize;
    let stretch = |c: [f64; 3]| {
        let along = dot([0, 1, 2].map(|k| c[k] - mean[k]), axis);
        (((along + CURVE_REACH) / CURVE_STEP).floor().max(0.0) as usize).min(stretches - 1)
    };
    // How many pixels of each stretch have each level in each channel.
    let mut levels = vec![[[0u32; 256]; 3]; stretches];
    for p in img.rgba.as_chunks::<4>().0.iter().filter(|p| p[3] >= 128) {
        if let Some(l) = levels.get_mut(stretch([p[0], p[1], p[2]].map(f64::from))) {
            for (counts, &v) in l.iter_mut().zip(p) {
                counts[usize::from(v)] += 1;
            }
        }
    }
    let middles: Vec<[f64; 3]> = levels
        .iter()
        .map(|l| {
            l.map(|counts| {
                let half = counts.iter().map(|&c| u64::from(c)).sum::<u64>().div_ceil(2);
                let mut seen = 0u64;
                counts
                    .iter()
                    .position(|&c| {
                        seen += u64::from(c);
                        seen >= half && seen > 0
                    })
                    .map_or(0.0, |v| v as f64)
            })
        })
        .collect();
    opaque()
        .filter(|&c| {
            let Some(mid) = middles.get(stretch(c)) else { return false };
            let d = [0, 1, 2].map(|k| c[k] - mid[k]);
            let along = dot(d, axis);
            let off = off_plane.map_or(0.0, |o| dot([0, 1, 2].map(|k| c[k] - mean[k]), o));
            dot(d, d) - along * along <= CURVE_OFF * CURVE_OFF && off.abs() <= CURVE_OFF
        })
        .count() as u64
}

/// The unit direction of the largest spread of a covariance matrix `m` (its main eigenvector),
/// found in a fixed 64 steps of power iteration from its longest row. None when it has no spread.
fn main_direction(m: &[[f64; 3]; 3]) -> Option<[f64; 3]> {
    let norm = |v: [f64; 3]| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    let mut v = *m.iter().max_by(|a, b| norm(**a).total_cmp(&norm(**b)))?;
    for _ in 0..64 {
        let len = norm(v);
        if len <= 1e-9 {
            return None;
        }
        let unit = v.map(|x| x / len);
        v = [0, 1, 2].map(|i| (0..3).map(|j| m[i][j] * unit[j]).sum());
    }
    let len = norm(v);
    (len > 1e-9).then(|| v.map(|x| x / len))
}

/// Drop unused palette entries and merge duplicates.
pub(super) fn compact(mut labels: Vec<u16>, palette: Vec<[u8; 3]>) -> Quantized {
    let mut used = vec![false; palette.len()];
    for &l in &labels {
        if l != TRANSPARENT {
            used[l as usize] = true;
        }
    }
    let mut remap = vec![0u16; palette.len()];
    let mut out: Vec<[u8; 3]> = vec![];
    for (i, c) in palette.iter().enumerate() {
        if !used[i] {
            continue;
        }
        remap[i] = match out.iter().position(|o| o == c) {
            Some(j) => j as u16,
            None => {
                out.push(*c);
                (out.len() - 1) as u16
            }
        };
    }
    for l in &mut labels {
        if *l != TRANSPARENT {
            *l = remap[*l as usize];
        }
    }
    Quantized { labels, palette: out }
}

/// `q` with the semi-transparent pixels along the cut edge of `img` given the color beside them,
/// and the colors that leaves unused dropped.
///
/// A traced pixel with an alpha of 128 to 254 and a pixel above, below, left or right of it with an
/// alpha under 128 is on the cut edge. When its color is the color of a fully opaque pixel within
/// two pixels of it, or a darker shade of that color (see [`edge_blend_of`]), it is that color
/// blended with the transparent background, and its own color can lie nearest to an unrelated
/// palette color, which would trace as a sliver of that color along the edge. Such a pixel takes
/// the color that most of those opaque pixels have, and of colors that tie, the one nearest to its
/// own color. Only the pixels it reaches through pixels of an alpha of 128 or more, going up, down,
/// left or right, count, so that art across a transparent gap is left out. It keeps its own color
/// when one of the fully opaque pixels it reaches has it, or when its color is a blend of none of
/// them, as with a thin semi-transparent outline of another color. Pixels outside the image do not
/// count as transparent. With `gray` (Grayscale), a pixel's color is its gray (see [`luma`]).
/// Returns which cut-edge pixels are a blend of the color of a fully opaque pixel they reach (see
/// [`edge_blend_of`]), whether or not they took that color (empty when `q` does not match `img`).
pub(crate) fn settle_cut_edges(q: &mut Quantized, img: &Raster, gray: bool) -> Vec<bool> {
    let (w, h) = (img.width as usize, img.height as usize);
    let px = img.rgba.as_chunks::<4>().0;
    if q.labels.len() != w * h || px.len() != w * h {
        return Vec::new();
    }
    let mut blended = vec![false; w * h];
    let color = |p: &[u8; 4]| if gray { [luma(*p); 3] } else { [p[0], p[1], p[2]] };
    let edge = cut_edge(img);
    let mut changed = false;
    let mut stack: Vec<(usize, usize)> = Vec::new();
    for (i, blend) in blended.iter_mut().enumerate() {
        let mine = q.labels[i];
        if mine == TRANSPARENT || !edge.get(i).copied().unwrap_or(false) {
            continue;
        }
        let Some(p) = px.get(i) else { continue };
        let own = oklab(color(p).map(f64::from));
        // The colors of the fully opaque pixels within two pixels that it reaches and is a blend
        // of, and how many have each.
        let mut near: Vec<(u16, usize)> = Vec::new();
        let mut keep = false;
        reached_opaque(img, (i % w, i / w), &mut stack, |j| {
            let l = q.labels[j];
            if l != TRANSPARENT {
                keep |= l == mine;
                if px.get(j).is_some_and(|b| edge_blend_of(own, color(b), p[3])) {
                    match near.iter_mut().find(|n| n.0 == l) {
                        Some(n) => n.1 += 1,
                        None => near.push((l, 1)),
                    }
                }
            }
        });
        *blend = !near.is_empty();
        if keep || near.is_empty() {
            continue;
        }
        let most = near.iter().map(|n| n.1).max().unwrap_or(0);
        let tied: Vec<(u16, [f64; 3])> =
            near.iter().filter(|n| n.1 == most).filter_map(|n| Some((n.0, oklab(q.palette.get(n.0 as usize)?.map(f64::from))))).collect();
        let tied_labs: Vec<[f64; 3]> = tied.iter().map(|t| t.1).collect();
        let Some(&(to, _)) = tied.get(nearest_lab(&tied_labs, own)) else { continue };
        q.labels[i] = to;
        changed = true;
    }
    if changed {
        *q = compact(std::mem::take(&mut q.labels), std::mem::take(&mut q.palette));
    }
    blended
}

/// Calls `visit` with the index of each fully opaque pixel of `img` within two pixels of the pixel
/// at `at` that it reaches through pixels of an alpha of 128 or more, going up, down, left or
/// right. `stack` is scratch space.
fn reached_opaque(img: &Raster, at: (usize, usize), stack: &mut Vec<(usize, usize)>, mut visit: impl FnMut(usize)) {
    let (w, h) = (img.width as usize, img.height as usize);
    let px = img.rgba.as_chunks::<4>().0;
    let alpha = |j: usize| px.get(j).map_or(0, |p| p[3]);
    let (x, y) = at;
    if x >= w || y >= h {
        return;
    }
    let (x0, y0, x1, y1) = (x.saturating_sub(2), y.saturating_sub(2), (x + 2).min(w - 1), (y + 2).min(h - 1));
    let mut reached = [[false; 5]; 5];
    stack.clear();
    stack.push((x, y));
    if let Some(r) = reached.get_mut(y - y0).and_then(|row| row.get_mut(x - x0)) {
        *r = true;
    }
    while let Some((cx, cy)) = stack.pop() {
        let j = cy * w + cx;
        if alpha(j) == 255 {
            visit(j);
        }
        for (nx, ny) in [(cx.wrapping_sub(1), cy), (cx + 1, cy), (cx, cy.wrapping_sub(1)), (cx, cy + 1)] {
            if nx < x0 || nx > x1 || ny < y0 || ny > y1 || alpha(ny * w + nx) < 128 {
                continue;
            }
            let Some(r) = reached.get_mut(ny - y0).and_then(|row| row.get_mut(nx - x0)) else { continue };
            if !*r {
                *r = true;
                stack.push((nx, ny));
            }
        }
    }
}

/// Which pixels of `img` are on its cut edge: an alpha of 128 to 254, and a pixel above, below,
/// left or right of it with an alpha under 128. Pixels outside the image do not count as
/// transparent.
fn cut_edge(img: &Raster) -> Vec<bool> {
    let (w, h) = (img.width as usize, img.height as usize);
    let px = img.rgba.as_chunks::<4>().0;
    let alpha = |j: usize| px.get(j).map_or(0, |p| p[3]);
    (0..px.len())
        .map(|i| {
            let (x, y) = (i % w.max(1), i / w.max(1));
            (128..=254).contains(&alpha(i))
                && ((x > 0 && alpha(i - 1) < 128)
                    || (x + 1 < w && alpha(i + 1) < 128)
                    || (y > 0 && alpha(i - w) < 128)
                    || (y + 1 < h && alpha(i + w) < 128))
        })
        .collect()
}

/// How many steps in from the cut edge (see [`cut_edge`]) the semi-transparent pixels that choose
/// no color reach.
const CUT_EDGE_BAND: usize = 2;

/// Which pixels of `img` choose the palette's colors: the opaque ones (an alpha of 128 or more),
/// except the pixels that [`settle_cut_edges`] gives the color beside them, and the pixels with an
/// alpha of 128 to 254 reached from those in at most [`CUT_EDGE_BAND`] steps up, down, left or
/// right through such pixels whose color is a blend of the same kind. The pixels it gives the color
/// beside them are the ones on the cut edge (see [`cut_edge`]) whose color is the color of a fully
/// opaque pixel that they reach blended with the transparent background (see [`edge_blend_of`]).
/// Counted, these pixels could add a second shade of a color to the palette. A thin
/// semi-transparent outline of another color still chooses. When no pixel is left to choose, every
/// opaque pixel chooses. With `gray` (Grayscale), a pixel's color is its gray (see [`luma`]), as in
/// [`settle_cut_edges`].
pub(super) fn choosing_pixels(img: &Raster, gray: bool) -> Vec<bool> {
    let (w, n) = (img.width as usize, img.rgba.len() / 4);
    let px = img.rgba.as_chunks::<4>().0;
    let semi = |j: usize| px.get(j).is_some_and(|p| (128..=254).contains(&p[3]));
    let color = |p: &[u8; 4]| if gray { [luma(*p); 3] } else { [p[0], p[1], p[2]] };
    let edge = cut_edge(img);
    let mut stack: Vec<(usize, usize)> = Vec::new();
    // Whether the color of pixel `i` is the color of a fully opaque pixel it reaches blended with
    // the transparent background.
    let mut blend_of_art = |i: usize| {
        let Some(p) = px.get(i) else { return false };
        let own = oklab(color(p).map(f64::from));
        let mut blend = false;
        reached_opaque(img, (i % w.max(1), i / w.max(1)), &mut stack, |j| {
            blend |= px.get(j).is_some_and(|b| edge_blend_of(own, color(b), p[3]));
        });
        blend
    };
    let mut left_out: Vec<bool> = (0..n).map(|i| edge.get(i).copied().unwrap_or(false) && blend_of_art(i)).collect();
    let mut front: Vec<usize> = (0..left_out.len()).filter(|&i| left_out.get(i).copied().unwrap_or(false)).collect();
    for _ in 0..CUT_EDGE_BAND {
        let mut next = Vec::new();
        for i in front {
            let (x, y) = (i % w.max(1), i / w.max(1));
            for j in [(x > 0).then(|| i - 1), (x + 1 < w).then(|| i + 1), (y > 0).then(|| i - w), (i + w < n).then(|| i + w)].into_iter().flatten() {
                if left_out.get(j) == Some(&false) && semi(j) && blend_of_art(j) {
                    if let Some(out) = left_out.get_mut(j) {
                        *out = true;
                    }
                    next.push(j);
                }
            }
        }
        front = next;
    }
    let choose: Vec<bool> = px.iter().zip(&left_out).map(|(p, &out)| p[3] >= 128 && !out).collect();
    if choose.iter().any(|&c| c) { choose } else { px.iter().map(|p| p[3] >= 128).collect() }
}

/// How far (Oklab × 100) the color of a pixel on a cut edge may lie from the color beside it, or
/// from a darker shade of that color, and still count as a blend of it with the transparent
/// background.
const EDGE_BLEND: f64 = 8.0;

/// Whether `own` (Oklab), the color of a pixel with an alpha of `alpha`, is `beside` (sRGB) blended
/// with the transparent background: whether it lies within [`EDGE_BLEND`] of the line in Oklab from
/// `beside` to `beside` with each channel scaled by `alpha` / 510.
fn edge_blend_of(own: [f64; 3], beside: [u8; 3], alpha: u8) -> bool {
    let to = oklab(beside.map(f64::from));
    let k = f64::from(alpha) / 510.0;
    let from = oklab(beside.map(|v| f64::from(v) * k));
    let d = [to[0] - from[0], to[1] - from[1], to[2] - from[2]];
    let len2 = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
    let t = if len2 > 0.0 { ((0..3).map(|c| (own[c] - from[c]) * d[c]).sum::<f64>() / len2).clamp(0.0, 1.0) } else { 0.0 };
    let off2: f64 = (0..3).map(|c| (own[c] - from[c] - t * d[c]).powi(2)).sum();
    off2 <= EDGE_BLEND * EDGE_BLEND
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Rings of four colors around a point off the center, blended over three pixels at each edge,
    /// with a little noise, and a band of half-transparent pixels along the bottom.
    fn rings(w: u32, h: u32) -> Raster {
        let colors = [[200.0, 30.0, 40.0], [20.0, 60.0, 180.0], [240.0, 210.0, 50.0], [25.0, 25.0, 25.0]];
        Raster::from_fn(w, h, |x, y| {
            let r = (f64::from(x) - 0.4 * f64::from(w)).hypot(f64::from(y) - 0.55 * f64::from(h)) / 9.0;
            let (ring, t) = (r.floor() as usize, ((r.fract() - 0.8) / 0.2).clamp(0.0, 1.0));
            let (a, b) = (colors[ring % 4], colors[(ring + 1) % 4]);
            let noise = f64::from((x * 7 + y * 13) % 5) - 2.0;
            let c = [0, 1, 2].map(|k| (a[k] + t * (b[k] - a[k]) + noise).clamp(0.0, 255.0) as u8);
            [c[0], c[1], c[2], if y + 2 >= h { 160 } else { 255 }]
        })
    }

    #[test]
    fn changes_are_copied_out_once_their_pairs_would_take_more_memory_than_the_labels() {
        let labels: Vec<u16> = (0..64).collect();
        let pairs = labels.len() * size_of::<u16>() / size_of::<(usize, u16)>();
        let mut changes = Changes::new(labels.len());
        for at in 0..pairs {
            changes.set(at * 3, 99, || labels.clone());
        }
        assert!(changes.copy.is_none());
        assert!(changes.few.capacity() <= pairs, "the pairs take no more memory than a copy of the labels");
        changes.set(63, 98, || labels.clone());
        assert!(changes.few.is_empty() && changes.copy.is_some());
        changes.set(1, 97, || labels.clone());
        let mut out = labels.clone();
        changes.each(|at, to| out[at] = to);
        let mut expected = labels.clone();
        (0..pairs).for_each(|at| expected[at * 3] = 99);
        (expected[63], expected[1]) = (98, 97);
        assert_eq!(out, expected);
    }

    #[test]
    fn blend_pixels_settle_the_same_however_the_picture_is_shared_out() {
        // Odd sizes, so that the last band of rows and the last strip of columns are short.
        let img = rings(173, 97);
        let all = img.width as usize * img.height as usize;
        let opaque: Vec<bool> = img.rgba.as_chunks::<4>().0.iter().map(|p| p[3] >= 128).collect();
        let own = super::super::own_colours_among(&img, &opaque);
        // Two colors for each blend, and an olive that the blends lie nearer to than to their sides,
        // each pixel labeled with the nearest of them (none for a transparent one).
        let palette = vec![[200, 30, 40], [20, 60, 180], [240, 210, 50], [25, 25, 25], [120, 120, 60]];
        let labels = img
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .map(|p| {
                let d = |c: &[u8; 3]| (0..3).map(|k| (i32::from(p[k]) - i32::from(c[k])).pow(2)).sum::<i32>();
                (0..palette.len()).min_by_key(|&i| d(&palette[i])).map_or(TRANSPARENT, |i| i as u16)
            })
            .collect();
        let q = Quantized { labels, palette };
        // One row (or one strip) per share, a few rows per share, and the whole picture as one.
        let shares = [1, 3 * 173 + 5, all];
        for (name, settle) in [
            ("settled", &(|s| settled_by(q.clone(), &img, &own, s)) as &dyn Fn(usize) -> Quantized),
            ("settled_wide", &|s| settled_wide_by(q.clone(), &img, &own, false, s)),
            ("settled_wide gray", &|s| settled_wide_by(q.clone(), &img, &own, true, s)),
        ] {
            let out: Vec<Quantized> = shares.iter().map(|&s| settle(s)).collect();
            assert_ne!(out[0], q, "{name} settles some blend pixels");
            assert!(out.iter().all(|o| *o == out[0]), "{name}");
        }
    }

    #[test]
    fn blend_pixels_settle_the_same_when_most_of_a_share_changes() {
        // Columns of red and blue own pixels, two blend pixels between each pair: one nearer to red
        // and one nearer to blue, both labeled with an olive. Half the pixels of each band of rows and
        // of each group of strips change, more than the eighth at which a share's changes are copied
        // out (see [`Changes`]). 44 columns, so that the last strip of 7 is short.
        let (w, h) = (44u32, 5u32);
        let (red, blue) = ([200u8, 30, 40], [20u8, 60, 180]);
        let mix = |t: f64| [0, 1, 2].map(|k| (f64::from(red[k]) + t * (f64::from(blue[k]) - f64::from(red[k]))).round() as u8);
        let img = Raster::from_fn(w, h, |x, _| {
            let c = [red, mix(0.3), mix(0.7), blue][x as usize % 4];
            [c[0], c[1], c[2], 255]
        });
        let own: Vec<bool> = (0..w * h).map(|i| matches!(i % w % 4, 0 | 3)).collect();
        let labels = (0..w * h).map(|i| [0, 2, 2, 1][(i % w % 4) as usize]).collect();
        let q = Quantized { labels, palette: vec![red, blue, [120, 120, 60]] };
        let expected = Quantized { labels: (0..w * h).map(|i| [0, 0, 1, 1][(i % w % 4) as usize]).collect(), palette: vec![red, blue] };
        let all = (w * h) as usize;
        for s in [1, 3 * 7 * h as usize, all] {
            assert_eq!(settled_by(q.clone(), &img, &own, s), expected, "settled, {s} pixels per share");
            assert_eq!(settled_wide_by(q.clone(), &img, &own, false, s), expected, "settled_wide, {s} pixels per share");
        }
    }

    #[test]
    fn a_picture_with_no_pixels_settles_to_itself() {
        let q = Quantized { labels: vec![], palette: vec![[0, 0, 0], [255, 255, 255]] };
        for (w, h) in [(u32::MAX, 0), (0, u32::MAX)] {
            let img = Raster::new(w, h, vec![]).unwrap();
            assert_eq!(settled_by(q.clone(), &img, &[], PIXELS_PER_SHARE), q, "settled, {w} x {h}");
            assert_eq!(settled_wide_by(q.clone(), &img, &[], false, PIXELS_PER_SHARE), q, "settled_wide, {w} x {h}");
        }
    }

    #[test]
    fn a_gray_blend_takes_the_closer_of_two_pairs_it_lies_between_despite_rounding() {
        // Gray 105 in the middle, with own pixels of gray 50 to its left, 200 to its right and black
        // along the top row, and a palette gray of 120 of its own. It lies between 0 and 200 exactly
        // and between 50 and 200 to within rounding: it takes 50, the nearer of the closer pair.
        let gray = |x: u32, y: u32| match (x, y) {
            (8, 8) => 105,
            (_, 0) => 0,
            (0..=8, _) => 50,
            _ => 200,
        };
        let img = Raster::from_fn(17, 17, |x, y| {
            let v = gray(x, y);
            [v, v, v, 255]
        });
        let palette = vec![[0, 0, 0], [50, 50, 50], [200, 200, 200], [120, 120, 120]];
        let labels: Vec<u16> = (0..17 * 17)
            .map(|i| match gray(i % 17, i / 17) {
                0 => 0,
                50 => 1,
                200 => 2,
                _ => 3,
            })
            .collect();
        let own: Vec<bool> = labels.iter().map(|&l| l != 3).collect();
        let q = settled_wide(Quantized { labels, palette }, &img, &own, true);
        let middle = q.labels[8 * 17 + 8];
        assert_eq!(q.palette[usize::from(middle)], [50, 50, 50]);
    }

    #[test]
    fn a_blend_pixel_beyond_the_end_of_a_pair_keeps_its_nearest_color() {
        // A column of [92, 92, 92] between own pixels of [100, 100, 100] to its left and
        // [200, 200, 200] to its right, with a palette color of [90, 90, 90]: it lies beyond the
        // darker end of the pair, not between the two, and its own color is the nearer, so it
        // keeps it.
        let value = |x: u32| match x {
            0..=7 => 100,
            8 => 92,
            _ => 200,
        };
        let img = Raster::from_fn(17, 17, |x, _| {
            let v = value(x);
            [v, v, v, 255]
        });
        let palette = vec![[100, 100, 100], [200, 200, 200], [90, 90, 90]];
        let labels: Vec<u16> = (0..17 * 17u32)
            .map(|i| match value(i % 17) {
                100 => 0,
                200 => 1,
                _ => 2,
            })
            .collect();
        let own: Vec<bool> = labels.iter().map(|&l| l != 2).collect();
        let q = settled_wide(Quantized { labels, palette }, &img, &own, false);
        assert_eq!(q.palette[usize::from(q.labels[8 * 17 + 8])], [90, 90, 90]);
    }
}
