//! Edge-preserving smoothing of continuous-tone images before they are quantised.
//!
//! A photo's tones run into each other, and its noise is as large as the step from one palette
//! colour to the next: quantised as it is, noise decides pixel by pixel which of two neighbouring
//! colours a pixel gets, and the trace is tens of thousands of specks. Each pixel is averaged with
//! its neighbours instead, each weighted by how alike it is (a bilateral filter: C. Tomasi and
//! R. Manduchi, "Bilateral Filtering for Gray and Color Images", 1998), so that noise within a tone
//! is averaged away while an edge between two tones stays where and as sharp as it was. Three
//! rounds, each along rows and then along columns, three pixels at a time.

use crate::{Raster, parallel_map};

/// Colours this far apart (levels, all channels together) count half as much as equal ones.
const ALIKE: f64 = 20.0;

/// How much a neighbour counts, by the squared distance between the two colours (all channels
/// together): `1 / (1 + d2 / ALIKE²)²`. Distances are whole numbers, so every value a pass can ask
/// for is worked out once here instead of once per pixel and neighbour.
fn weight_table() -> Vec<f64> {
    (0..=3 * 255 * 255usize).map(|d2| 1.0 / (1.0 + d2 as f64 / (ALIKE * ALIKE)).powi(2)).collect()
}

/// `v` rounded to the nearest level, halves up. The same as `v.round() as u8` for `0 <= v <= 255`,
/// without the library call that `round` is on targets that lack a rounding instruction.
fn level(v: f64) -> u8 {
    let whole = v as u8;
    whole.saturating_add(u8::from(v - f64::from(whole) >= 0.5))
}

/// The pixel at column `x` of `row`.
fn at(row: &[u8], x: usize) -> Option<&[u8]> {
    row.get(x * 4..x * 4 + 4)
}

/// One pass along rows (`dx = 1`) or columns (`dx = 0`), weighting by `weights` (see `weight_table`).
fn pass(src: &[u8], w: usize, h: usize, along_rows: bool, weights: &[f64]) -> Vec<u8> {
    let rows: Vec<usize> = (0..h).collect();
    let row = |y: usize| src.get(y * w * 4..(y + 1) * w * 4).unwrap_or_default();
    let done = parallel_map(&rows, |&y| {
        let cur = row(y);
        let (above, below) = (y.checked_sub(1).map(row), (y + 1 < h).then(|| row(y + 1)));
        let mut out = Vec::with_capacity(w * 4);
        for (x, p) in cur.as_chunks::<4>().0.iter().enumerate() {
            if p[3] < 128 {
                out.extend_from_slice(p);
                continue;
            }
            let near = if along_rows {
                [x.checked_sub(1).and_then(|x| at(cur, x)), at(cur, x + 1)]
            } else {
                [above.and_then(|r| at(r, x)), below.and_then(|r| at(r, x))]
            };
            let (mut sum, mut weight) = ([f64::from(p[0]), f64::from(p[1]), f64::from(p[2])], 1.0);
            for q in near.into_iter().flatten().filter(|q| q[3] >= 128) {
                let d = |k: usize| i32::from(q[k]) - i32::from(p[k]);
                let d2 = (d(0) * d(0) + d(1) * d(1) + d(2) * d(2)) as usize;
                let wt = weights.get(d2).copied().unwrap_or(0.0);
                for k in 0..3 {
                    sum[k] += wt * f64::from(q[k]);
                }
                weight += wt;
            }
            out.extend(sum.map(|v| level(v / weight)));
            out.push(p[3]);
        }
        out
    });
    done.concat()
}

/// `img` with the noise within its tones smoothed away and its edges kept.
pub(crate) fn smoothed(img: &Raster) -> Raster {
    let (w, h) = (img.width as usize, img.height as usize);
    let mut rgba = img.rgba.clone();
    if rgba.len() == w * h * 4 {
        let weights = weight_table();
        for _ in 0..3 {
            rgba = pass(&pass(&rgba, w, h, true, &weights), w, h, false, &weights);
        }
    }
    Raster { width: img.width, height: img.height, rgba }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noise_goes_and_an_edge_stays() {
        // Left half about 60, right half about 160, with noise of ±6 on both.
        let img = Raster::from_fn(40, 20, |x, y| {
            let mut n = x.wrapping_mul(0x9E37_79B1) ^ y.wrapping_mul(0x85EB_CA77);
            n ^= n >> 15;
            n = n.wrapping_mul(0x2C1B_3C6D);
            let n = ((n >> 12) % 13) as u8;
            let v = if x < 20 { 54 + n } else { 154 + n };
            [v, v, v, 255]
        });
        let out = smoothed(&img);
        let spread = |r: &Raster, x0: u32, x1: u32| {
            let v: Vec<u8> = (0..20).flat_map(|y| (x0..x1).map(move |x| (x, y))).map(|(x, y)| r.pixel(x, y)[0]).collect();
            v.iter().max().unwrap() - v.iter().min().unwrap()
        };
        assert_eq!(spread(&img, 0, 20), 12);
        assert!(spread(&out, 0, 20) <= 6 && spread(&out, 20, 40) <= 6, "{} {}", spread(&out, 0, 20), spread(&out, 20, 40));
        // The edge is still one pixel sharp, where it was.
        assert!(out.pixel(19, 10)[0] < 70 && out.pixel(20, 10)[0] > 150);
    }

    /// The filter as it was before the weight table: the reference the fast pass must match byte for byte.
    fn reference(src: &[u8], w: usize, h: usize, along_rows: bool) -> Vec<u8> {
        let px = |x: usize, y: usize| src.get((y * w + x) * 4..(y * w + x) * 4 + 4);
        let mut out = Vec::with_capacity(w * h * 4);
        for y in 0..h {
            for x in 0..w {
                let Some(p) = px(x, y) else { continue };
                if p[3] < 128 {
                    out.extend_from_slice(p);
                    continue;
                }
                let near = if along_rows {
                    [(x > 0).then(|| (x - 1, y)), (x + 1 < w).then(|| (x + 1, y))]
                } else {
                    [(y > 0).then(|| (x, y - 1)), (y + 1 < h).then(|| (x, y + 1))]
                };
                let (mut sum, mut weight) = ([f64::from(p[0]), f64::from(p[1]), f64::from(p[2])], 1.0);
                for q in near.into_iter().flatten().filter_map(|(x, y)| px(x, y)).filter(|q| q[3] >= 128) {
                    let d2: f64 = (0..3).map(|k| (f64::from(q[k]) - f64::from(p[k])).powi(2)).sum();
                    let wt = 1.0 / (1.0 + d2 / (ALIKE * ALIKE)).powi(2);
                    for k in 0..3 {
                        sum[k] += wt * f64::from(q[k]);
                    }
                    weight += wt;
                }
                out.extend(sum.map(|v| (v / weight).round() as u8));
                out.push(p[3]);
            }
        }
        out
    }

    fn reference_smoothed(img: &Raster) -> Vec<u8> {
        let (w, h) = (img.width as usize, img.height as usize);
        let mut rgba = img.rgba.clone();
        for _ in 0..3 {
            rgba = reference(&reference(&rgba, w, h, true), w, h, false);
        }
        rgba
    }

    fn hash(x: u32, y: u32, salt: u32) -> u32 {
        let mut n = x.wrapping_mul(0x9E37_79B1) ^ y.wrapping_mul(0x85EB_CA77) ^ salt.wrapping_mul(0xC2B2_AE3D);
        n ^= n >> 15;
        n = n.wrapping_mul(0x2C1B_3C6D);
        n ^= n >> 13;
        n.wrapping_mul(0x297A_2D39) ^ (n >> 16)
    }

    #[test]
    fn same_bytes_as_the_unoptimised_filter() {
        let inputs: Vec<(&str, Raster)> = vec![
            ("random colours", Raster::from_fn(61, 47, |x, y| [hash(x, y, 1) as u8, (hash(x, y, 2) >> 3) as u8, (hash(x, y, 3) >> 5) as u8, 255])),
            (
                "photo-like",
                Raster::from_fn(90, 70, |x, y| {
                    let base = ((x as f64 / 9.0).sin() * 60.0 + (y as f64 / 13.0).cos() * 60.0 + 128.0) as i32;
                    let v = (base + (hash(x, y, 4) % 9) as i32 - 4).clamp(0, 255) as u8;
                    [v, (i32::from(v) / 2 + 60) as u8, 255 - v, 255]
                }),
            ),
            ("gradient", Raster::from_fn(80, 33, |x, y| [(x * 3) as u8, (y * 7) as u8, (x + y) as u8, 255])),
            ("flat", Raster::from_fn(20, 20, |_, _| [40, 90, 200, 255])),
            ("extremes", Raster::from_fn(32, 32, |x, y| if hash(x, y, 5).is_multiple_of(2) { [0, 0, 0, 255] } else { [255, 255, 255, 255] })),
            (
                "holes and half alpha",
                Raster::from_fn(50, 40, |x, y| {
                    [hash(x, y, 6) as u8, (hash(x, y, 7) >> 8) as u8, 77, [0, 127, 128, 255, 255][(hash(x, y, 8) % 5) as usize]]
                }),
            ),
            ("one column", Raster::from_fn(1, 30, |_, y| [(y * 8) as u8, 5, 9, 255])),
            ("one row", Raster::from_fn(30, 1, |x, _| [(x * 8) as u8, 5, 9, 255])),
            ("one pixel", Raster::from_fn(1, 1, |_, _| [1, 2, 3, 255])),
            ("two by two", Raster::from_fn(2, 2, |x, y| [(x * 100) as u8, (y * 100) as u8, 3, 255])),
            ("wide", Raster::from_fn(257, 3, |x, y| [x as u8, (x / 2) as u8, y as u8, 255])),
        ];
        for (name, img) in &inputs {
            assert_eq!(smoothed(img).rgba, reference_smoothed(img), "{name}");
        }
    }

    #[test]
    fn same_bytes_for_every_pair_of_grey_levels() {
        // Every pixel next to every possible neighbour, one table for all of them (the picture
        // tests above run the three rounds).
        let weights = weight_table();
        for p in 0..=255u8 {
            for q in 0..=255u8 {
                // Grey, and a mixed pair, along a row (2x1) and along a column (1x2).
                for src in [[p, p, p, 255, q, q, q, 255], [p, q, p ^ q, 255, q, p, q ^ 0x5a, 255]] {
                    assert_eq!(pass(&src, 2, 1, true, &weights), reference(&src, 2, 1, true), "{p} {q}");
                    assert_eq!(pass(&src, 1, 2, false, &weights), reference(&src, 1, 2, false), "{p} {q}");
                }
            }
        }
    }

    #[test]
    fn level_is_round_for_every_value_that_can_occur() {
        for v in
            [0.0, 0.499_999_999_999_999_94, 0.5, 0.500_000_000_000_000_1, 1.499_999_999_999_999_8, 1.5, 127.5, 254.5, 254.999_999_999_999_97, 255.0]
        {
            assert_eq!(level(v), v.round() as u8, "{v}");
        }
        // Values a hair either side of every half, where a wrong rounding would show.
        for k in 0..255 {
            let half = f64::from(k) + 0.5;
            for v in [half, f64::from_bits(half.to_bits() - 1), f64::from_bits(half.to_bits() + 1)] {
                assert_eq!(level(v), v.round() as u8, "{v}");
            }
        }
        for i in 0..=255 * 4096u32 {
            let v = f64::from(i) / 4096.0;
            assert_eq!(level(v), v.round() as u8, "{v}");
        }
    }

    #[test]
    fn transparent_pixels_and_odd_sizes_pass_through() {
        let img = Raster::from_fn(3, 1, |x, _| [10 * x as u8, 0, 0, if x == 1 { 0 } else { 255 }]);
        let out = smoothed(&img);
        assert_eq!(out.pixel(1, 0), [10, 0, 0, 0]);
        assert_eq!((out.width, out.height, out.rgba.len()), (3, 1, 12));
        let broken = Raster { width: 4, height: 4, rgba: vec![0; 7] };
        assert_eq!(smoothed(&broken).rgba.len(), 7);
    }
}
