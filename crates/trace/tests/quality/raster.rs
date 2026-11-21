//! A small scanline rasteriser and image filters for the quality harness: test inputs are drawn
//! from ground-truth vector shapes here, never loaded from files.

use craft_trace::{Raster, TracedGradient};
use kurbo::{BezPath, PathEl, Point};

/// The closed polylines of `path` (one per subpath), flattened within `tol`.
pub fn polylines(path: &BezPath, tol: f64) -> Vec<Vec<Point>> {
    let (mut out, mut cur) = (vec![], vec![]);
    kurbo::flatten(path.iter(), tol, |el| match el {
        PathEl::MoveTo(p) => {
            if cur.len() > 1 {
                out.push(std::mem::take(&mut cur));
            }
            cur.clear();
            cur.push(p);
        }
        PathEl::LineTo(p) => cur.push(p),
        _ => {}
    });
    if cur.len() > 1 {
        out.push(cur);
    }
    out
}

/// Adds the non-zero-winding coverage (0–1 per pixel) of `path` to `cov` (`w × h`); returns the
/// rows touched. Five sub-scanlines per row, exact along x.
pub fn coverage_into(path: &BezPath, w: usize, h: usize, cov: &mut [f32]) -> (usize, usize) {
    const SS: usize = 5;
    let mut edges = vec![];
    for p in polylines(path, 0.02) {
        for i in 0..p.len() {
            let (a, b) = (p[i], p[(i + 1) % p.len()]);
            if a.y < b.y {
                edges.push((a, b, 1));
            } else if b.y < a.y {
                edges.push((b, a, -1));
            }
        }
    }
    let (lo, hi) = edges.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), e| (lo.min(e.0.y), hi.max(e.1.y)));
    if edges.is_empty() {
        return (0, 0);
    }
    let (r0, r1) = (lo.floor().clamp(0.0, h as f64) as usize, hi.ceil().clamp(0.0, h as f64) as usize);
    let mut xs: Vec<(f64, i32)> = vec![];
    for row in r0..r1 {
        for k in 0..SS {
            let y = row as f64 + (k as f64 + 0.5) / SS as f64;
            xs.clear();
            xs.extend(edges.iter().filter(|(a, b, _)| a.y <= y && y < b.y).map(|(a, b, d)| (a.x + (y - a.y) * (b.x - a.x) / (b.y - a.y), *d)));
            xs.sort_by(|p, q| p.0.total_cmp(&q.0));
            let (mut wind, mut start) = (0, 0.0);
            for &(x, d) in &xs {
                let before = wind;
                wind += d;
                if before == 0 {
                    start = x;
                } else if wind == 0 {
                    let (x0, x1) = (start.clamp(0.0, w as f64), x.clamp(0.0, w as f64));
                    for px in x0.floor() as usize..(x1.ceil() as usize).min(w) {
                        cov[row * w + px] += ((x1.min(px as f64 + 1.0) - x0.max(px as f64)).max(0.0) / SS as f64) as f32;
                    }
                }
            }
        }
    }
    (r0, r1)
}

/// The coverage of `path` alone.
pub fn coverage(path: &BezPath, w: usize, h: usize) -> Vec<f32> {
    let mut cov = vec![0.0; w * h];
    coverage_into(path, w, h, &mut cov);
    cov
}

/// Paints `scene` (bottom first) over `bg`, anti-aliased, the way a renderer shows a trace.
pub fn render(scene: &[(BezPath, [u8; 3])], bg: [u8; 3], w: usize, h: usize) -> Raster {
    let scene: Vec<(BezPath, [u8; 3], Option<&TracedGradient>)> = scene.iter().map(|(p, c)| (p.clone(), *c, None)).collect();
    render_painted(&scene, bg, w, h)
}

/// [`render`], with the paths that have a gradient filled with it.
pub fn render_painted(scene: &[(BezPath, [u8; 3], Option<&TracedGradient>)], bg: [u8; 3], w: usize, h: usize) -> Raster {
    let mut buf = vec![bg.map(f32::from); w * h];
    let mut cov = vec![0.0f32; w * h];
    for (path, color, gradient) in scene {
        let (r0, r1) = coverage_into(path, w, h, &mut cov);
        for i in r0 * w..r1 * w {
            let a = cov[i].min(1.0);
            if a > 0.0 {
                let color = match gradient {
                    Some(g) => g.color_at(Point::new((i % w) as f64 + 0.5, (i / w) as f64 + 0.5)).map(|v| v as f32),
                    None => color.map(f32::from),
                };
                for ch in 0..3 {
                    buf[i][ch] += (color[ch] - buf[i][ch]) * a;
                }
                cov[i] = 0.0;
            }
        }
    }
    from_rgb(w, h, &buf)
}

/// Draws ground-truth art: each pixel is the average of `K × K` hard samples of the top-most
/// shape, so abutting shapes blend with each other on their shared edge and never with the
/// background behind them (which painting them one after another would let through).
pub fn render_truth(scene: &[(BezPath, [u8; 3])], bg: [u8; 3], w: usize, h: usize, k: usize) -> Raster {
    let (bw, bh) = (w * k, h * k);
    let mut top = vec![bg; bw * bh];
    let mut cov = vec![0.0f32; bw * bh];
    for (path, color) in scene {
        let mut big = path.clone();
        big.apply_affine(kurbo::Affine::scale(k as f64));
        let (r0, r1) = coverage_into(&big, bw, bh, &mut cov);
        for i in r0 * bw..r1 * bw {
            if cov[i] >= 0.5 {
                top[i] = *color;
            }
            cov[i] = 0.0;
        }
    }
    let mut buf = vec![[0.0f32; 3]; w * h];
    for y in 0..bh {
        for x in 0..bw {
            for ch in 0..3 {
                buf[(y / k) * w + x / k][ch] += f32::from(top[y * bw + x][ch]) / (k * k) as f32;
            }
        }
    }
    from_rgb(w, h, &buf)
}

fn from_rgb(w: usize, h: usize, buf: &[[f32; 3]]) -> Raster {
    let mut rgba = Vec::with_capacity(w * h * 4);
    for p in buf {
        rgba.extend(p.map(|v| v.round().clamp(0.0, 255.0) as u8));
        rgba.push(255);
    }
    Raster::new(w as u32, h as u32, rgba).unwrap()
}

/// A deterministic hash of a pixel position and a salt, as a value in `-1.0..1.0`.
pub fn noise(x: u32, y: u32, salt: u32) -> f64 {
    let mut v = x.wrapping_mul(0x9E37_79B1) ^ y.wrapping_mul(0x85EB_CA77) ^ salt.wrapping_mul(0xC2B2_AE3D);
    v ^= v >> 15;
    v = v.wrapping_mul(0x2C1B_3C6D);
    v ^= v >> 12;
    v = v.wrapping_mul(0x297A_2D39);
    v ^= v >> 15;
    f64::from(v) / f64::from(u32::MAX) * 2.0 - 1.0
}

fn map(img: &Raster, f: impl Fn(u32, u32, [f64; 3]) -> [f64; 3]) -> Raster {
    Raster::from_fn(img.width, img.height, |x, y| {
        let p = img.pixel(x, y);
        let c = f(x, y, [f64::from(p[0]), f64::from(p[1]), f64::from(p[2])]);
        [c[0].round().clamp(0.0, 255.0) as u8, c[1].round().clamp(0.0, 255.0) as u8, c[2].round().clamp(0.0, 255.0) as u8, 255]
    })
}

/// A slight blur (the 1-4-6-4-1 binomial kernel each way, about one pixel of sigma): a resized or
/// re-sampled image.
pub fn blur(img: &Raster) -> Raster {
    const K: [f64; 5] = [1.0, 4.0, 6.0, 4.0, 1.0];
    let (w, h) = (img.width as i64, img.height as i64);
    let pass = |src: &Raster, dx: i64, dy: i64| {
        map(src, |x, y, _| {
            let mut s = [0.0; 3];
            for (k, wt) in K.iter().enumerate() {
                let o = k as i64 - 2;
                let p = src.pixel((i64::from(x) + o * dx).clamp(0, w - 1) as u32, (i64::from(y) + o * dy).clamp(0, h - 1) as u32);
                for ch in 0..3 {
                    s[ch] += f64::from(p[ch]) * wt / 16.0;
                }
            }
            s
        })
    };
    pass(&pass(img, 1, 0), 0, 1)
}

/// Compression-like noise: an offset per 8 × 8 block plus ringing-sized noise per pixel.
pub fn noisy(img: &Raster) -> Raster {
    map(img, |x, y, c| {
        let block = 3.0 * noise(x / 8, y / 8, 7);
        [0, 1, 2].map(|ch| c[ch] + block + 5.0 * noise(x, y, 11 + ch as u32))
    })
}

/// The colour a scan gives a clean colour, before shading, blur and grain: ink on cream paper.
pub fn scan_color(c: [u8; 3]) -> [u8; 3] {
    const PAPER: [f64; 3] = [236.0, 230.0, 214.0];
    const INK: [f64; 3] = [34.0, 32.0, 44.0];
    [0, 1, 2].map(|ch| (INK[ch] + (PAPER[ch] - INK[ch]) * f64::from(c[ch]) / 255.0).round() as u8)
}

/// Scanned ink on paper: the colours of [`scan_color`], uneven lighting, a slight blur and grain.
pub fn scanned(img: &Raster) -> Raster {
    let (w, h) = (f64::from(img.width), f64::from(img.height));
    let toned = map(img, |x, y, c| {
        let s = scan_color(c.map(|v| v as u8));
        let shade = 1.0 - 0.07 * (f64::from(x) / w) - 0.05 * ((f64::from(y) / h) * 3.0).sin().abs();
        s.map(|v| f64::from(v) * shade)
    });
    map(&blur(&toned), |x, y, c| {
        let g = 4.0 * noise(x, y, 23);
        c.map(|v| v + g)
    })
}
