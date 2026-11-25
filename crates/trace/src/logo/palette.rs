//! Finding a logo's flat colours, and telling flat-colour art from gradients and photos.

use super::unmix::Px;

/// A colour with every channel at or above this is white, which is always a class of its own.
pub(crate) const WHITE_MIN: u8 = 245;

/// `#rrggbb` or `rrggbb`.
pub(crate) fn parse_hex(s: &str) -> Option<[u8; 3]> {
    let h = s.trim().trim_start_matches('#');
    if h.len() != 6 || !h.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let v = |i: usize| u8::from_str_radix(h.get(i..i + 2)?, 16).ok();
    Some([v(0)?, v(2)?, v(4)?])
}

fn rgb255(p: &Px) -> [f64; 3] {
    let a = f64::from(p[3]).max(1e-6);
    [f64::from(p[0]) / a * 255.0, f64::from(p[1]) / a * 255.0, f64::from(p[2]) / a * 255.0]
}

/// Opaque pixels whose 3×3 neighbourhood is uniform (variance below `max_var`): edge pixels, which
/// are blends, do not get a vote.
fn solid_pixels(pix: &[Px], w: usize, h: usize, max_var: f64) -> Vec<[f64; 3]> {
    let mut out = Vec::new();
    if w < 3 || h < 3 {
        return pix.iter().filter(|p| p[3] > 0.98).map(rgb255).collect();
    }
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let Some(c) = pix.get(y * w + x) else { continue };
            if c[3] <= 0.98 {
                continue;
            }
            let (mut s, mut s2, mut cnt) = ([0.0f64; 3], [0.0f64; 3], 0.0f64);
            let mut opaque = true;
            for dy in 0..3 {
                for dx in 0..3 {
                    let Some(q) = pix.get((y + dy - 1) * w + (x + dx - 1)) else { continue };
                    if q[3] <= 0.98 {
                        opaque = false;
                    }
                    let v = rgb255(q);
                    for k in 0..3 {
                        s[k] += v[k];
                        s2[k] += v[k] * v[k];
                    }
                    cnt += 1.0;
                }
            }
            if !opaque || cnt < 9.0 {
                continue;
            }
            let var: f64 = (0..3).map(|k| s2[k] / cnt - (s[k] / cnt).powi(2)).sum();
            if var < max_var {
                out.push(rgb255(c));
            }
        }
    }
    out
}

fn dist2(a: [f64; 3], b: [f64; 3]) -> f64 {
    (0..3).map(|k| (a[k] - b[k]).powi(2)).sum()
}

struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((self.0 >> 33) as f64) / ((1u64 << 31) as f64)
    }
}

fn kmeans(px: &[[f64; 3]], k: usize, seed: u64) -> (Vec<[f64; 3]>, Vec<usize>, f64) {
    let mut rng = Lcg(seed);
    let mut c: Vec<[f64; 3]> = vec![px[(rng.next() * px.len() as f64) as usize % px.len()]];
    while c.len() < k {
        let d: Vec<f64> = px.iter().map(|p| c.iter().map(|q| dist2(*p, *q)).fold(f64::MAX, f64::min)).collect();
        let total: f64 = d.iter().sum();
        if total <= 0.0 {
            break;
        }
        let mut r = rng.next() * total;
        let mut pick = px.len() - 1;
        for (i, v) in d.iter().enumerate() {
            r -= v;
            if r <= 0.0 {
                pick = i;
                break;
            }
        }
        c.push(px[pick]);
    }
    let mut lab = vec![0usize; px.len()];
    for _ in 0..30 {
        for (i, p) in px.iter().enumerate() {
            lab[i] = (0..c.len()).min_by(|&a, &b| dist2(*p, c[a]).total_cmp(&dist2(*p, c[b]))).unwrap_or(0);
        }
        let mut sum = vec![[0.0f64; 3]; c.len()];
        let mut cnt = vec![0usize; c.len()];
        for (p, &l) in px.iter().zip(&lab) {
            for j in 0..3 {
                sum[l][j] += p[j];
            }
            cnt[l] += 1;
        }
        for (i, ci) in c.iter_mut().enumerate() {
            if cnt[i] > 0 {
                *ci = [sum[i][0] / cnt[i] as f64, sum[i][1] / cnt[i] as f64, sum[i][2] / cnt[i] as f64];
            }
        }
    }
    let cost = px.iter().zip(&lab).map(|(p, &l)| dist2(*p, c[l])).sum();
    (c, lab, cost)
}

/// Up to `max_colors` flat colours, most common first. White is left out (it is always a class of
/// its own), as are colours that make up under 0.1% of the solid pixels (noise).
///
/// Line art (thin strokes on paper) has no solid area of any colour but the paper's: a stroke one
/// or two pixels wide never fills a uniform 3 × 3 neighbourhood, so its colour is a few pixels in
/// tens of thousands of paper and is cut as noise. Where the solid pixels give no colour, the
/// colours are read from the strokes' cores instead ([`line_art_palette`]).
pub(crate) fn auto_palette(pix: &[Px], w: usize, h: usize, max_colors: usize) -> Vec<[u8; 3]> {
    let mut px = solid_pixels(pix, w, h, 20.0);
    if px.len() < max_colors.max(2) * 20 {
        px = pix.iter().filter(|p| p[3] > 0.9).map(rgb255).collect();
    }
    let found = clustered(px, max_colors);
    if !found.is_empty() {
        return found.into_iter().map(|(c, _)| c).collect();
    }
    line_art_palette(pix, w, h, max_colors)
}

/// How much darker than white (in its darkest channel) a colour must be to be ink rather than the
/// paper's grain or a light smudge.
const INK_DEPTH: f64 = 48.0;

/// The least share of the opaque pixels that ink must cover for a page with no solid colour but
/// white to be line art rather than blank paper with a few specks or marks on it.
const MIN_INK_SHARE: f64 = 0.01;

/// Whether `c` is ink: at least [`INK_DEPTH`] darker than white in one channel.
fn is_ink(c: [f64; 3]) -> bool {
    c.iter().any(|&v| v <= 255.0 - INK_DEPTH)
}

/// The colours of line art, read from its strokes' cores ([`ink_cores`]): nothing when ink covers
/// under [`MIN_INK_SHARE`] of the image, and the lighter shades the anti-aliasing of a stroke's
/// edge leaves as cores of their own folded into the ink they are a blend of ([`fold_fringes`]).
fn line_art_palette(pix: &[Px], w: usize, h: usize, max_colors: usize) -> Vec<[u8; 3]> {
    let (mut opaque, mut ink) = (0usize, 0usize);
    for p in pix.iter().filter(|p| p[3] > 0.98) {
        opaque += 1;
        if is_ink(rgb255(p)) {
            ink += 1;
        }
    }
    if (ink as f64) < MIN_INK_SHARE * opaque as f64 {
        return Vec::new();
    }
    fold_fringes(clustered(ink_cores(pix, w, h), max_colors))
}

/// The opaque ink pixels ([`is_ink`]) that are at least as far from white as each of their four
/// neighbours: the middle of every stroke, where its colour is least blended with the paper's
/// (and every pixel of a wider area of one colour).
fn ink_cores(pix: &[Px], w: usize, h: usize) -> Vec<[f64; 3]> {
    let white = [255.0; 3];
    let ink = |i: usize| pix.get(i).filter(|p| p[3] > 0.98).map(|p| dist2(rgb255(p), white)).unwrap_or(0.0);
    let mut out = Vec::new();
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let Some(p) = pix.get(i).filter(|p| p[3] > 0.98) else { continue };
            let c = rgb255(p);
            if !is_ink(c) {
                continue;
            }
            let d = dist2(c, white);
            let near = [(x > 0).then(|| i - 1), (x + 1 < w).then_some(i + 1), (y > 0).then(|| i - w), (y + 1 < h).then_some(i + w)];
            if near.into_iter().flatten().all(|j| ink(j) <= d) {
                out.push(c);
            }
        }
    }
    out
}

/// How far (in RGB) a colour may lie from the line between white and an ink and still be a blend
/// of the two.
const FRINGE_APART: f64 = 20.0;

/// `groups` (colours with their counts, most common first) with each colour that is a blend of white
/// and a darker colour of the list (it lies on the line between them, within [`FRINGE_APART`])
/// folded into that colour: the anti-aliased edge of a thin stroke, or a stroke thinner than a
/// pixel, is lighter than its ink but no colour of its own. The ink keeps its colour.
fn fold_fringes(mut groups: Vec<([u8; 3], usize)>) -> Vec<[u8; 3]> {
    // Lightest first, so a blend of a blend goes to the darkest ink.
    let depth = |c: [u8; 3]| c.iter().map(|&v| 255.0 - f64::from(v)).sum::<f64>();
    groups.sort_by(|a, b| depth(a.0).total_cmp(&depth(b.0)));
    let mut i = 0;
    while i < groups.len() {
        let a = groups[i].0.map(|v| 255.0 - f64::from(v));
        // The ink `a` is the closest blend of, if any: (its index, the distance off the line).
        let mut into: Option<(usize, f64)> = None;
        for (j, g) in groups.iter().enumerate().skip(i + 1) {
            let b = g.0.map(|v| 255.0 - f64::from(v));
            let bb: f64 = b.iter().map(|v| v * v).sum();
            if bb <= 0.0 {
                continue;
            }
            let t = (0..3).map(|k| a[k] * b[k]).sum::<f64>() / bb;
            if !(0.0..1.0).contains(&t) {
                continue;
            }
            let off = (0..3).map(|k| (a[k] - t * b[k]).powi(2)).sum::<f64>().sqrt();
            if off <= FRINGE_APART && into.is_none_or(|(_, o)| off < o) {
                into = Some((j, off));
            }
        }
        match into {
            Some((j, _)) => {
                let n = groups[i].1;
                groups[j].1 += n;
                groups.remove(i);
            }
            None => i += 1,
        }
    }
    groups.sort_by_key(|g| std::cmp::Reverse(g.1));
    groups.into_iter().map(|(c, _)| c).collect()
}

/// Up to `max_colors` colours that `px` clusters into, with how many of `px` each has, most common
/// first; white and noise left out.
fn clustered(mut px: Vec<[f64; 3]>, max_colors: usize) -> Vec<([u8; 3], usize)> {
    if px.is_empty() {
        return Vec::new();
    }
    if px.len() > 20_000 {
        let stride = px.len() / 20_000 + 1;
        px = px.into_iter().step_by(stride).collect();
    }
    let k = max_colors.clamp(1, 12).min(px.len());
    let best = (0..4u64).map(|s| kmeans(&px, k, 0x9e37_79b9 + s * 7919)).min_by(|a, b| a.2.total_cmp(&b.2));
    let Some((cent, lab, _)) = best else { return Vec::new() };
    let mut cnt = vec![0usize; cent.len()];
    for &l in &lab {
        if let Some(c) = cnt.get_mut(l) {
            *c += 1;
        }
    }
    let total = lab.len().max(1);
    let mut groups: Vec<([f64; 3], usize)> =
        cent.iter().zip(&cnt).filter(|(c, n)| **n * 1000 > total && !c.iter().all(|&v| v >= f64::from(WHITE_MIN))).map(|(c, &n)| (*c, n)).collect();
    // Centres closer than 20 are one colour split by noise.
    loop {
        let mut merged = false;
        'outer: for i in 0..groups.len() {
            for j in (i + 1)..groups.len() {
                if dist2(groups[i].0, groups[j].0) < 400.0 {
                    let (a, b) = (groups[i], groups[j]);
                    let n = (a.1 + b.1) as f64;
                    let c = [0, 1, 2].map(|k| (a.0[k] * a.1 as f64 + b.0[k] * b.1 as f64) / n);
                    groups[i] = (c, a.1 + b.1);
                    groups.remove(j);
                    merged = true;
                    break 'outer;
                }
            }
        }
        if !merged {
            break;
        }
    }
    groups.sort_by_key(|g| std::cmp::Reverse(g.1));
    groups.iter().map(|(c, n)| (c.map(|v| v.round().clamp(0.0, 255.0) as u8), *n)).collect()
}

/// How far apart (in RGB) two tones of one colour may be: a scan's paper is a tenth darker in one
/// corner than in another.
const TONES_APART: f64 = 60.0;

/// `palette` with the tones of one colour made one colour. Two flat colours leave nothing between
/// them but the pixels of their shared edges; where a scan's paper is unevenly lit, or a wash
/// shades off, the pixels between two of its tones are as many as those at either: such a pair is
/// one colour (their mean, by pixels), however many tones of it were found.
pub(crate) fn fold_tones(pix: &[Px], mut palette: Vec<[u8; 3]>) -> Vec<[u8; 3]> {
    let apart = |a: [u8; 3], b: [u8; 3]| dist2(a.map(f64::from), b.map(f64::from)).sqrt();
    loop {
        let close: Vec<(usize, usize)> = (0..palette.len())
            .flat_map(|i| (i + 1..palette.len()).map(move |j| (i, j)))
            .filter(|&(i, j)| palette.get(i).zip(palette.get(j)).is_some_and(|(a, b)| apart(*a, *b) <= TONES_APART))
            .collect();
        if close.is_empty() {
            return palette;
        }
        // Each opaque pixel goes to its nearest colour; for a close pair, the pixels of either are
        // counted by where they lie from one to the other: at one, between, at the other.
        let mut thirds = vec![[0usize; 3]; close.len()];
        for p in pix.iter().filter(|p| p[3] > 0.9) {
            let c = rgb255(p);
            let Some(near) = (0..palette.len()).min_by(|&i, &j| dist2(c, palette[i].map(f64::from)).total_cmp(&dist2(c, palette[j].map(f64::from))))
            else {
                continue;
            };
            for (k, &(i, j)) in close.iter().enumerate() {
                if near == i || near == j {
                    let (a, b) = (palette[i].map(f64::from), palette[j].map(f64::from));
                    let t = (0..3).map(|ch| (c[ch] - a[ch]) * (b[ch] - a[ch])).sum::<f64>() / dist2(a, b).max(1e-9);
                    thirds[k][if t < 1.0 / 3.0 {
                        0
                    } else if t <= 2.0 / 3.0 {
                        1
                    } else {
                        2
                    }] += 1;
                }
            }
        }
        // The pair with the most between it, if that is no fewer than a third of what the lesser
        // end has.
        let filled = |t: &[usize; 3]| t[1] as f64 / (t[0].min(t[2]).max(1)) as f64;
        let Some((k, _)) = thirds.iter().enumerate().filter(|(_, t)| filled(t) >= 1.0 / 3.0).max_by(|a, b| filled(a.1).total_cmp(&filled(b.1)))
        else {
            return palette;
        };
        let ((i, j), t) = (close[k], thirds[k]);
        let (na, nb) = ((t[0] + t[1] / 2).max(1) as f64, (t[2] + t[1] / 2).max(1) as f64);
        palette[i] = [0, 1, 2].map(|ch| ((f64::from(palette[i][ch]) * na + f64::from(palette[j][ch]) * nb) / (na + nb)).round() as u8);
        palette.remove(j);
    }
}

/// The color of the ground a flat bar two pixels wide lies on, if the pixel at (`x`, `y`) is one of
/// the bar's. Across the bar (down or across the image) the pixel and one neighbor are the bar, and
/// the pixels on either side of the two are the ground; along the bar, three pixels in a row that
/// include the pixel (at either end or in the middle) are bar and ground in the same way, so all
/// six pixels of a bar three pixels long count. Every bar pixel lies within √12, about 3.5 (in
/// RGB), of the pixel's color; the ground's pixels lie within 10 (in RGB) of the first one and at
/// least 25 from the pixel's color. `at` is the opaque color at a pixel, `None` outside the image or where it is
/// not opaque.
fn bar_ground(at: &impl Fn(isize, isize) -> Option<[f64; 3]>, x: isize, y: isize, c: [f64; 3]) -> Option<[f64; 3]> {
    let bar = |q: Option<[f64; 3]>| q.is_some_and(|q| dist2(q, c) <= 12.0);
    for ((ax, ay), (bx, by)) in [((1, 0), (0, 1)), ((0, 1), (1, 0))] {
        for s in [-1, 1] {
            // Across the bar: ground, the pixel, its partner, ground.
            let Some(g) = at(x + 2 * s * bx, y + 2 * s * by) else { continue };
            let ground = |q: Option<[f64; 3]>| q.is_some_and(|q| dist2(q, g) < 100.0 && dist2(q, c) >= 625.0);
            // Along the bar: three pixels in a row, the pixel at either end of them or in the middle.
            let flat = (-1..=1).any(|o| {
                (o - 1..=o + 1).all(|k| {
                    let (px, py) = (x + k * ax, y + k * ay);
                    bar(at(px, py))
                        && bar(at(px + s * bx, py + s * by))
                        && ground(at(px - s * bx, py - s * by))
                        && ground(at(px + 2 * s * bx, py + 2 * s * by))
                })
            });
            if flat {
                return Some(g);
            }
        }
    }
    None
}

/// Colours the current palette cannot explain. `residual` is each pixel's error after the best blend
/// of the known classes (see `unmix`); a solid run of pixels (a pixel and its four neighbours agree)
/// with a large error is a colour that is missing, such as the small accent that was too few
/// pixels to be found at first. A flat bar two pixels wide on a ground within 25 of a known color
/// ([`bar_ground`]) has no such run: its pixels count when their error is large, and also when their
/// color lies less than two thirds as far from that known color as from every other known color (a
/// gray bar on white beside black art), where the bar would otherwise be traced as the ground.
/// Groups of at least three such pixels at least 25 away from every known color are returned, most
/// common first.
pub(crate) fn unexplained_colours(pix: &[Px], w: usize, h: usize, residual: &[f32], known: &[[u8; 3]]) -> Vec<[u8; 3]> {
    let mut pal: Vec<[f64; 3]> = known.iter().map(|c| c.map(f64::from)).collect();
    pal.push([255.0, 255.0, 255.0]);
    let at = |x: usize, y: usize| pix.get(y * w + x).filter(|p| p[3] > 0.98).map(rgb255);
    let at_signed = |x: isize, y: isize| {
        let (x, y) = (usize::try_from(x).ok()?, usize::try_from(y).ok()?);
        if x < w && y < h { at(x, y) } else { None }
    };
    let mut groups: Vec<([f64; 3], usize)> = Vec::new();
    for y in 1..h.saturating_sub(1) {
        for x in 1..w.saturating_sub(1) {
            let large = residual.get(y * w + x).copied().unwrap_or(0.0) >= 0.0064;
            let Some(c) = at(x, y) else { continue };
            if pal.iter().any(|q| dist2(*q, c) < 625.0) {
                continue;
            }
            let uniform = [(x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)].iter().all(|&(xx, yy)| at(xx, yy).is_some_and(|q| dist2(q, c) < 100.0));
            let missing = match (uniform, isize::try_from(x), isize::try_from(y)) {
                (true, ..) => large,
                (false, Ok(sx), Ok(sy)) => bar_ground(&at_signed, sx, sy, c).is_some_and(|g| {
                    // The known color of the ground, and every other known color more than 1.5 times as far from the bar.
                    let Some(ground) = pal.iter().copied().find(|q| dist2(*q, g) < 625.0) else { return false };
                    large || pal.iter().filter(|q| dist2(**q, ground) >= 625.0).all(|q| 2.25 * dist2(c, ground) < dist2(c, *q))
                }),
                _ => false,
            };
            if !missing {
                continue;
            }
            match groups.iter_mut().find(|(m, _)| dist2(*m, c) < 400.0) {
                Some((m, n)) => {
                    for k in 0..3 {
                        m[k] = (m[k] * *n as f64 + c[k]) / (*n as f64 + 1.0);
                    }
                    *n += 1;
                }
                None => groups.push((c, 1)),
            }
        }
    }
    groups.retain(|g| g.1 >= 3);
    groups.sort_by_key(|g| std::cmp::Reverse(g.1));
    groups.iter().map(|(c, _)| c.map(|v| v.round().clamp(0.0, 255.0) as u8)).collect()
}

/// Share of solid pixels within a small colour distance of a palette colour (white included).
/// About 1.0 for flat-colour art; far lower for gradients, shading and photos.
pub(crate) fn flat_fraction(pix: &[Px], w: usize, h: usize, palette: &[[u8; 3]]) -> f64 {
    let px = solid_pixels(pix, w, h, 60.0);
    if px.is_empty() {
        return 0.0;
    }
    let mut pal: Vec<[f64; 3]> = palette.iter().map(|c| c.map(f64::from)).collect();
    pal.push([255.0, 255.0, 255.0]);
    let near = px.iter().filter(|p| pal.iter().any(|q| dist2(**p, *q) < 22.0 * 22.0)).count();
    near as f64 / px.len() as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blocks(w: usize, h: usize, f: impl Fn(usize, usize) -> [u8; 4]) -> Vec<Px> {
        (0..w * h)
            .map(|i| {
                let p = f(i % w, i / w);
                let a = f32::from(p[3]) / 255.0;
                [f32::from(p[0]) / 255.0 * a, f32::from(p[1]) / 255.0 * a, f32::from(p[2]) / 255.0 * a, a]
            })
            .collect()
    }

    #[test]
    fn hex_parsing() {
        assert_eq!(parse_hex("#6C60F6"), Some([0x6c, 0x60, 0xf6]));
        assert_eq!(parse_hex("ea4335"), Some([0xea, 0x43, 0x35]));
        for bad in ["", "#12345", "#1234567", "#gg0000", "é2345z", "  ", "+1+2+3", "#+1+2+3"] {
            assert_eq!(parse_hex(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn three_flat_colours_are_found_exactly() {
        let pix = blocks(60, 20, |x, _| match x / 20 {
            0 => [200, 30, 30, 255],
            1 => [30, 160, 60, 255],
            _ => [60, 70, 180, 255],
        });
        let mut pal = auto_palette(&pix, 60, 20, 6);
        pal.sort();
        let mut want = vec![[200, 30, 30], [30, 160, 60], [60, 70, 180]];
        want.sort();
        assert_eq!(pal, want);
        assert!(flat_fraction(&pix, 60, 20, &pal) > 0.99);
    }

    #[test]
    fn white_is_never_in_the_palette_and_transparency_is_ignored() {
        let pix = blocks(40, 20, |x, _| {
            if x < 10 {
                [255, 255, 255, 255]
            } else if x < 20 {
                [0, 0, 0, 0]
            } else {
                [10, 120, 200, 255]
            }
        });
        let pal = auto_palette(&pix, 40, 20, 4);
        assert_eq!(pal, vec![[10, 120, 200]]);
    }

    #[test]
    fn a_gradient_is_not_flat() {
        let pix = blocks(64, 16, |x, _| [(x * 4) as u8, 255 - (x * 4) as u8, 128, 255]);
        let pal = auto_palette(&pix, 64, 16, 6);
        assert!(flat_fraction(&pix, 64, 16, &pal) < 0.93, "gradient scored flat");
    }

    #[test]
    fn edge_shades_fold_into_their_ink_and_other_colours_stay() {
        // A blue ink, a blend of it with white (its anti-aliased edge), a red ink and an orange
        // that lies on no ink's line from white.
        let groups = vec![([30, 90, 200], 50), ([143, 173, 228], 40), ([210, 40, 50], 30), ([240, 150, 40], 5)];
        assert_eq!(fold_fringes(groups), vec![[30, 90, 200], [210, 40, 50], [240, 150, 40]]);
    }

    #[test]
    fn a_bar_two_pixels_wide_and_three_long_is_an_unexplained_color() {
        // A red bar two pixels wide on white beside a black block, three and four pixels long,
        // lying across and down the image. Only the bar three pixels long needs its end pixels
        // counted to reach the three pixels a color needs.
        const RED: [u8; 3] = [210, 40, 50];
        for long in [3, 4] {
            for across in [true, false] {
                let (w, h) = (16, 12);
                let pix = blocks(w, h, |x, y| {
                    let (along, side) = if across { (x, y) } else { (y, x) };
                    if x < 3 {
                        [0, 0, 0, 255]
                    } else if (6..6 + long).contains(&along) && (5..7).contains(&side) {
                        [RED[0], RED[1], RED[2], 255]
                    } else {
                        [255, 255, 255, 255]
                    }
                });
                let residual = vec![1.0; w * h];
                assert_eq!(unexplained_colours(&pix, w, h, &residual, &[[0, 0, 0]]), vec![RED], "{long} long, across: {across}");
            }
        }
    }

    #[test]
    fn nothing_to_find_in_empty_or_blank_images() {
        assert!(auto_palette(&[], 0, 0, 6).is_empty());
        let clear = blocks(10, 10, |_, _| [0, 0, 0, 0]);
        assert!(auto_palette(&clear, 10, 10, 6).is_empty());
        assert_eq!(flat_fraction(&clear, 10, 10, &[]), 0.0);
        let white = blocks(10, 10, |_, _| [255, 255, 255, 255]);
        assert!(auto_palette(&white, 10, 10, 6).is_empty());
    }
}
