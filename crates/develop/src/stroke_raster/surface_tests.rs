//! Auto Mask on a constructed scene like a face it failed on: one smoothly shaded surface (skin),
//! its brightness falling by more than a stop and a half from one side to the other with a gentle
//! bulge of light, its colour drifting a little, fine noise, freckles and a few soft creases, on a
//! background of another colour (a blanket) beyond a crisp edge. With the brush's centre
//! wandering over the surface, everything of it within the brush's reach should be painted (the
//! shading, drift, freckles and creases are no edges), and nothing past its edge.
//!
//! `cargo test -p craft-develop --lib surface_tests -- --nocapture` prints the numbers.

use super::*;
use crate::geometry::Frame;
use crate::masks::brush_dabs;
use craft_develop_model::{BrushStroke, DevelopSettings};

fn hash(x: usize, y: usize, seed: u64) -> f32 {
    let mut v =
        (x as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (y as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F) ^ seed.wrapping_mul(0x1656_67B1_9E37_79F9);
    v ^= v >> 29;
    v = v.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    v ^= v >> 32;
    (v % 10_000) as f32 / 5_000.0 - 1.0
}

/// Whether point `(u, v)` (units of a 400 × 300 frame) is on the surface: an ellipse.
fn on_surface(u: f64, v: f64) -> bool {
    ((u - 200.0) / 120.0).powi(2) + ((v - 150.0) / 95.0).powi(2) < 1.0
}

/// A crease: (centre, direction, half length, depth EV, width).
type Crease = ((f64, f64), (f64, f64), f64, f32, f64);

/// Soft creases: (centre, direction, half length, depth EV, width) in frame units. The first, deep
/// as a closed eye's, runs right across the surface, the side past it (from the brush's centre on
/// [`ONE_SIDE`]) [`PAST_CREASE_EV`] darker: the paint must cross it.
const CREASES: [Crease; 5] = [
    ((200.0, 150.0), (0.6, -0.8), 140.0, 1.1, 4.0),
    ((165.0, 125.0), (0.8, 0.6), 35.0, 0.7, 3.0),
    ((225.0, 170.0), (1.0, -0.2), 40.0, 0.6, 4.0),
    ((190.0, 195.0), (0.3, 1.0), 22.0, 0.8, 2.5),
    ((255.0, 115.0), (0.0, 1.0), 28.0, 0.5, 6.0),
];

/// How much darker the surface is past the first crease (a face's far side, turned from the
/// light).
const PAST_CREASE_EV: f32 = 0.6;

/// A nostril: (centre, radius), a stop and a half darker and redder.
const NOSTRIL: ((f64, f64), f64) = ((215.0, 160.0), 4.0);

/// Blushes: (centre, radius) in frame units, where the skin turns redder.
const BLUSHES: [((f64, f64), f64); 2] = [((150.0, 175.0), 22.0), ((250.0, 150.0), 18.0)];

/// The scene at `k` times 400 × 300: (colours, log luminance, whether each pixel is the surface).
fn scene(k: f64) -> (Rgb32f, Plane, Vec<bool>, usize, usize) {
    let (w, h) = ((400.0 * k) as usize, (300.0 * k) as usize);
    let blanket = [0.82f32, 0.97, 1.3];
    let mut inside = vec![false; w * h];
    let img = Rgb32f::from_fn(w, h, |x, y| {
        let (u, v) = ((x as f64 + 0.5) / k, (y as f64 + 0.5) / k);
        let (lum, chroma) = if on_surface(u, v) {
            // a rounded form lit from the upper left: about 2 EV from its lit side to its shaded
            // rim, falling faster toward the rim, as a head's
            let (nx, ny) = ((u - 200.0) / 120.0, (v - 150.0) / 95.0);
            let nz = (1.0 - nx * nx - ny * ny).max(0.0).sqrt();
            let lit = (-0.55 * nx - 0.35 * ny + 0.76 * nz).max(0.05) as f32;
            let t = ((u - 80.0) / 240.0) as f32;
            let mut ev = 0.9 + 1.1 * lit.log2().max(-1.5) - 0.6 * t;
            // freckles: small dark spots
            let (fu, fv) = ((u / 9.0).floor(), (v / 9.0).floor());
            let fx = fu * 9.0 + 4.5 + 3.0 * f64::from(hash(fu as usize, fv as usize, 7));
            let fy = fv * 9.0 + 4.5 + 3.0 * f64::from(hash(fu as usize, fv as usize, 8));
            if hash(fu as usize, fv as usize, 9) > 0.6 && (u - fx).hypot(v - fy) < 1.3 {
                ev -= 0.3;
            }
            for (q, ((cx, cy), (dx, dy), half, depth, width)) in CREASES.into_iter().enumerate() {
                let n = dx.hypot(dy);
                let (dx, dy) = (dx / n, dy / n);
                let along = (u - cx) * dx + (v - cy) * dy;
                let across = -(u - cx) * dy + (v - cy) * dx;
                let fade = (1.0 - (along / half).powi(2)).max(0.0) as f32;
                ev -= depth * fade * (-(across / width).powi(2) as f32).exp();
                if q == 0 {
                    ev -= PAST_CREASE_EV / (1.0 + (-across / width).exp()) as f32;
                }
            }
            let nostril = (-((u - NOSTRIL.0.0).powi(2) + (v - NOSTRIL.0.1).powi(2)) / NOSTRIL.1.powi(2)).exp() as f32;
            ev -= 1.5 * nostril;
            // warm skin drifting a little cooler and pinker over the surface
            let s = ((v - 35.0) / 230.0) as f32;
            let mut warm = [1.27 - 0.06 * s, 0.91 + 0.01 * s, 0.84 + 0.07 * s + 0.03 * t];
            for ((bx, by), br) in BLUSHES {
                let b = (-((u - bx).powi(2) + (v - by).powi(2)) / (br * br)).exp() as f32;
                warm = [warm[0] + 0.13 * b, warm[1] - 0.03 * b, warm[2] - 0.05 * b];
            }
            warm = [warm[0] + 0.3 * nostril, warm[1] - 0.08 * nostril, warm[2] - 0.2 * nostril];
            (0.18 * ev.exp2(), warm)
        } else {
            (0.18 * 0.6f32.exp2() * (0.03 * ((u + v) / 60.0).sin() as f32).exp2(), blanket)
        };
        let n = 0.05 / k.sqrt() as f32;
        let g = (n * hash(x, y, 1)).exp2();
        std::array::from_fn(|c| lum * chroma[c] * g * (1.0 + 0.3 * n * hash(x, y, 2 + c as u64)))
    });
    for y in 0..h {
        for x in 0..w {
            inside[y * w + x] = on_surface((x as f64 + 0.5) / k, (y as f64 + 0.5) / k);
        }
    }
    let l = img.map(crate::local::log_lum);
    (img, l, inside, w, h)
}

fn stroke(points: &[(f64, f64)], size: f64, auto_mask: bool) -> BrushStroke {
    BrushStroke {
        points: points.iter().map(|p| Point::new(p.0 / 400.0, p.1 / 300.0)).collect(),
        size,
        feather: 50.0,
        flow: 100.0,
        auto_mask,
        ..BrushStroke::default()
    }
}

fn painted(s: &BrushStroke, img: &Rgb32f, l: &Plane, w: usize, h: usize) -> Plane {
    let f = Frame::new(w, h, &DevelopSettings::default(), true);
    let mut r = StrokeRaster::with_model(w, h, AutoMaskModel::Smoothed);
    r.update(&brush_dabs(s, &f, w, h), img, l);
    r.to_plane()
}

/// The centre wandering over the surface: over the lit side, across creases, into the shade.
const WANDER: [(f64, f64); 9] =
    [(135.0, 140.0), (160.0, 105.0), (200.0, 95.0), (240.0, 120.0), (262.0, 160.0), (232.0, 195.0), (185.0, 190.0), (155.0, 165.0), (205.0, 145.0)];
/// The centre staying on the lit side of the long crease, the brush reaching over it.
const ONE_SIDE: [(f64, f64); 6] = [(140.0, 135.0), (162.0, 104.0), (192.0, 96.0), (178.0, 122.0), (152.0, 150.0), (186.0, 118.0)];
/// Near the edge, the brush reaching over the blanket.
const NEAR_EDGE: [(f64, f64); 5] = [(100.0, 150.0), (110.0, 115.0), (140.0, 82.0), (185.0, 66.0), (240.0, 68.0)];

#[derive(Debug)]
pub(super) struct SurfaceMeasure {
    /// Of the surface's pixels the plain brush paints fully (and 2 px or more inside its edge), the
    /// share painted 0.9 or more.
    pub cover: f32,
    /// Mean paint on the background where the plain brush paints (2 px or more from the edge).
    pub leak: f32,
    /// Mean distance (px of a 400 × 300 frame) from where the paint crosses 0.5 well inside the
    /// brush's reach (the plain brush painting 0.9 or more there) to the surface's edge.
    pub boundary: f32,
}

pub(super) fn measure_surface(k: f64, pts: &[(f64, f64)], size: f64) -> SurfaceMeasure {
    let (img, l, inside, w, h) = scene(k);
    let plain = painted(&stroke(pts, size, false), &img, &l, w, h);
    let a = painted(&stroke(pts, size, true), &img, &l, w, h);
    if let Ok(dir) = std::env::var("CRAFT_SURFACE_OUT") {
        // the scene with the paint over it in red, the plain brush's reach it misses in cyan
        let mut out = format!("P6\n{w} {h}\n255\n").into_bytes();
        for i in 0..w * h {
            let p = img.data[i].map(|v| (v.max(0.0).powf(1.0 / 2.2) * 255.0).min(255.0));
            let (al, gap) = (a.data[i] * 0.55, (plain.data[i] - a.data[i]).max(0.0) * 0.5);
            let q = [p[0] * (1.0 - al) + 255.0 * al, p[1] * (1.0 - al), p[2] * (1.0 - al)];
            out.extend_from_slice(&[(q[0] * (1.0 - gap)) as u8, (q[1] * (1.0 - gap) + 255.0 * gap) as u8, (q[2] * (1.0 - gap) + 255.0 * gap) as u8]);
        }
        let _ = std::fs::write(format!("{dir}/surface-{}-{size}-{k}.ppm", pts.len()), out);
    }
    let at = |x: i64, y: i64| (x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h).then(|| inside[y as usize * w + x as usize]);
    let near_edge = |x: usize, y: usize, d: i64| {
        let r = inside[y * w + x];
        (-d..=d).any(|dy| (-d..=d).any(|dx| at(x as i64 + dx, y as i64 + dy).is_some_and(|o| o != r)))
    };
    let edge_px: Vec<(usize, usize)> = (1..h - 1)
        .flat_map(|y| (1..w - 1).map(move |x| (x, y)))
        .filter(|&(x, y)| {
            inside[y * w + x] && [(1i64, 0i64), (-1, 0), (0, 1), (0, -1)].iter().any(|(dx, dy)| at(x as i64 + dx, y as i64 + dy) == Some(false))
        })
        .collect();
    let (mut n, mut full, mut ln, mut lsum, mut bn, mut bsum) = (0f32, 0f32, 0f32, 0f32, 0f32, 0f32);
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let i = y * w + x;
            if plain.data[i] < 0.05 {
                continue;
            }
            let edge = near_edge(x, y, (2.0 * k).ceil() as i64);
            if inside[i] && !edge && plain.data[i] >= 0.95 {
                n += 1.0;
                full += f32::from(u8::from(a.data[i] >= 0.9));
            } else if !inside[i] && !edge {
                ln += 1.0;
                lsum += a.data[i];
            }
            let crosses = a.data[i] >= 0.5
                && [1, w].iter().any(|&o| a.data.get(i + o).is_some_and(|v| *v < 0.5) || a.data.get(i - o).is_some_and(|v| *v < 0.5));
            // (where the plain brush paints solidly: not where the paint fades with the brush's
            // feather)
            if crosses && plain.data[i] >= 0.9 {
                let d = edge_px
                    .iter()
                    .map(|&(ex, ey)| ((ex as f32 - x as f32).powi(2) + (ey as f32 - y as f32).powi(2)).sqrt())
                    .fold(f32::INFINITY, f32::min);
                bn += 1.0;
                bsum += d / k as f32;
            }
        }
    }
    SurfaceMeasure { cover: full / n.max(1.0), leak: lsum / ln.max(1.0), boundary: if bn > 0.0 { bsum / bn } else { 0.0 } }
}

#[test]
fn auto_mask_fills_a_shaded_drifting_surface_within_reach_and_stops_at_its_edge() {
    eprintln!("{:<10} {:>5} {:>4} | {:>6} {:>6} {:>9}", "case", "size", "k", "cover", "leak", "boundary");
    let mut failed = Vec::new();
    for (name, pts) in [("wander", &WANDER[..]), ("one side", &ONE_SIDE[..]), ("near edge", &NEAR_EDGE[..])] {
        for size in [0.08, 0.16] {
            for k in [1.0, 2.5] {
                let m = measure_surface(k, pts, size);
                eprintln!("{name:<10} {size:>5} {k:>4} | {:>6.3} {:>6.3} {:>7.2}px", m.cover, m.leak, m.boundary);
                if m.cover < 0.9 || m.leak >= 0.03 || m.boundary >= 1.5 {
                    failed.push(format!("{name} {size} {k}: {m:?}"));
                }
            }
        }
    }
    // (within reach: covered 0.9 or more; past the edge: under 0.03; the paint's edge within 1.5
    // px of the surface's)
    assert!(failed.is_empty(), "{failed:#?}");
}

/// A cream object against a grey wall almost as bright, as a cake's shaded side against the wall
/// behind it: the edge is mostly one of colour (the wall a little cooler), with a thin darker rim
/// just inside it, a slant and a slow wave, and sensor noise on both sides. Its edge at row `y`
/// (pixel units of the 480 × 360 frame): the object is left of it.
fn wall_edge(y: f64) -> f64 {
    290.0 + 0.3 * (y - 180.0) + 8.0 * (y / 45.0).sin()
}

/// About normal noise from three hashes, unit spread.
fn gauss(x: usize, y: usize, seed: u64) -> f32 {
    (hash(x, y, seed) + hash(x, y, seed + 101) + hash(x, y, seed + 202)) * 1.0
}

/// The wall scene, the wall `wall_ev` brighter than the object's shaded edge: colours, log
/// luminance (480 × 360).
fn wall_scene(wall_ev: f32) -> (Rgb32f, Plane, usize, usize) {
    let (w, h) = (480usize, 360usize);
    let img = Rgb32f::from_fn(w, h, |x, y| {
        let (u, v) = (x as f64 + 0.5, y as f64 + 0.5);
        let e = wall_edge(v);
        // (the edge as soft as a lens draws it: a pixel and a half)
        let t = (1.0 / (1.0 + ((u - e) / 0.8).exp())) as f32;
        let cream = [1.065f32, 0.99, 0.83];
        let grey = [1.04f32, 0.99, 0.91];
        // the object shaded toward its edge, down to the wall's brightness there
        let obj_ev = 0.15 + 0.6 * ((e - u) / 200.0).clamp(0.0, 1.0) as f32 - 0.5 * (-((e - u - 2.0) / 1.5).powi(2) as f32).exp();
        let ev = t * obj_ev + (1.0 - t) * wall_ev;
        let ch: [f32; 3] = std::array::from_fn(|c| t * cream[c] + (1.0 - t) * grey[c]);
        // sensor noise: 0.04 EV of luminance, a little of colour
        let g = (0.04 * gauss(x, y, 11) / 1.7).exp2();
        std::array::from_fn(|c| 0.18 * ev.exp2() * g * ch[c] * (1.0 + 0.006 * gauss(x, y, 21 + c as u64) / 1.7))
    });
    let l = img.map(crate::local::log_lum);
    (img, l, w, h)
}

#[derive(Debug)]
struct EdgeMeasure {
    /// Mean of where the paint crosses 0.5 along each row, less the edge's place (px).
    offset: f32,
    /// How far that crossing wanders about its own running mean over 9 rows (RMS, px): a torn,
    /// ragged edge wanders, a clean one follows the photo's.
    rough: f32,
    /// The most it wanders from the edge's place, less the mean offset (px).
    worst: f32,
    /// Rows the paint crossed no edge near the photo's (it stopped short, or ran on).
    lost: usize,
    /// Mean paint on the wall 3 px or more past the edge where the plain brush paints.
    leak: f32,
    /// Wall pixels 3 px or more past the edge painted over 0.05 (specks, haze).
    specks: usize,
    /// Object pixels 3 px or more inside the edge the plain brush paints solidly, painted under
    /// 0.9 (pinholes).
    holes: usize,
    /// Separate pieces of paint (0.5 or more), 8-connected.
    pieces: usize,
    /// The stroke painted a point at a time against painted at once: the most they differ.
    live: f32,
}

fn measure_wall_edge(size: f64, wall_ev: f32) -> EdgeMeasure {
    let (img, l, w, h) = wall_scene(wall_ev);
    let f = Frame::new(w, h, &DevelopSettings::default(), true);
    let r = size * 480.0;
    // the brush's centre inside the object, its circle reaching over the edge
    let pts: Vec<(f64, f64)> = (0..=12).map(|i| 50.0 + 260.0 * f64::from(i) / 12.0).map(|y| (wall_edge(y) - 0.45 * r, y)).collect();
    let mk = |auto: bool| BrushStroke {
        points: pts.iter().map(|p| Point::new(p.0 / 480.0, p.1 / 360.0)).collect(),
        size,
        feather: 50.0,
        flow: 100.0,
        auto_mask: auto,
        ..BrushStroke::default()
    };
    let s = mk(true);
    let plain = painted(&mk(false), &img, &l, w, h);
    let mut once = StrokeRaster::with_model(w, h, AutoMaskModel::Smoothed);
    once.update(&brush_dabs(&s, &f, w, h), &img, &l);
    let a = once.to_plane();
    let mut live = StrokeRaster::with_model(w, h, AutoMaskModel::Smoothed);
    for n in 1..=s.points.len() {
        let part = BrushStroke { points: s.points[..n].to_vec(), ..s.clone() };
        live.update(&brush_dabs(&part, &f, w, h), &img, &l);
    }
    let live_d = live.to_plane().data.iter().zip(&a.data).map(|(p, q)| (p - q).abs()).fold(0f32, f32::max);
    if let Ok(dir) = std::env::var("CRAFT_SURFACE_OUT") {
        let mut out = format!("P6\n{w} {h}\n255\n").into_bytes();
        for i in 0..w * h {
            let p = img.data[i].map(|v| (v.max(0.0).powf(1.0 / 2.2) * 255.0).min(255.0));
            let al = a.data[i] * 0.55;
            out.extend_from_slice(&[(p[0] * (1.0 - al) + 255.0 * al) as u8, (p[1] * (1.0 - al)) as u8, (p[2] * (1.0 - al)) as u8]);
        }
        let _ = std::fs::write(format!("{dir}/wall-{size}-{wall_ev}.ppm"), out);
    }
    // where the paint crosses 0.5 on each row the plain brush paints solidly across the edge
    let mut d = Vec::new();
    let mut lost = 0;
    for y in 0..h {
        let e = wall_edge(y as f64 + 0.5);
        let (lo, hi) = ((e - 8.0).max(0.0) as usize, ((e + 8.0) as usize).min(w - 1));
        if ((e - 4.0) as usize..=(e + 4.0) as usize).any(|x| plain.data.get(y * w + x).is_none_or(|v| *v < 0.9)) {
            continue;
        }
        // (the last crossing from paint to none, interpolated)
        let cross = (lo..hi).rev().find(|&x| a.data[y * w + x] >= 0.5 && a.data[y * w + x + 1] < 0.5).map(|x| {
            let (p, q) = (a.data[y * w + x], a.data[y * w + x + 1]);
            x as f64 + 0.5 + f64::from((p - 0.5) / (p - q))
        });
        match cross {
            Some(c) if (c - e).abs() < 7.0 => d.push((c - e) as f32),
            _ => lost += 1,
        }
    }
    let n = d.len().max(1) as f32;
    let offset = d.iter().sum::<f32>() / n;
    let mut rough = 0f32;
    for i in 0..d.len() {
        let (a0, a1) = (i.saturating_sub(4), (i + 5).min(d.len()));
        let m = d[a0..a1].iter().sum::<f32>() / (a1 - a0) as f32;
        rough += (d[i] - m).powi(2);
    }
    let rough = (rough / n).sqrt();
    let worst = d.iter().map(|v| (v - offset).abs()).fold(0f32, f32::max);
    let (mut ln, mut lsum, mut specks, mut holes) = (0f32, 0f32, 0, 0);
    for y in 0..h {
        let e = wall_edge(y as f64 + 0.5);
        for x in 0..w {
            let (u, i) = (x as f64 + 0.5, y * w + x);
            if u >= e + 3.0 && plain.data[i] > 0.0 {
                ln += 1.0;
                lsum += a.data[i];
                specks += usize::from(a.data[i] > 0.05);
            }
            if u <= e - 3.0 && plain.data[i] >= 0.95 && a.data[i] < 0.9 {
                holes += 1;
            }
        }
    }
    // pieces of paint: flood fill
    let mut seen = vec![false; w * h];
    let mut pieces = 0;
    for start in 0..w * h {
        if seen[start] || a.data[start] < 0.5 {
            continue;
        }
        pieces += 1;
        let mut stack = vec![start];
        seen[start] = true;
        while let Some(i) = stack.pop() {
            let (x, y) = ((i % w) as i64, (i / w) as i64);
            for (dx, dy) in [(-1i64, -1i64), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)] {
                let (nx, ny) = (x + dx, y + dy);
                if nx < 0 || ny < 0 || nx >= w as i64 || ny >= h as i64 {
                    continue;
                }
                let j = ny as usize * w + nx as usize;
                if !seen[j] && a.data[j] >= 0.5 {
                    seen[j] = true;
                    stack.push(j);
                }
            }
        }
    }
    EdgeMeasure { offset, rough, worst, lost, leak: lsum / ln.max(1.0), specks, holes, pieces, live: live_d }
}

/// The paint's edge against a wall almost as bright as the object follows the photo's edge
/// cleanly: no tears, no haze or specks past it, no pinholes inside, one piece, and the stroke
/// painted live is the stroke rendered at once.
#[test]
fn auto_mask_edge_against_a_wall_as_bright_is_clean() {
    let mut failed = Vec::new();
    for (size, wall_ev) in [(0.05, 0.0), (0.1, 0.0), (0.05, -1.0), (0.1, -1.0)] {
        let m = measure_wall_edge(size, wall_ev);
        eprintln!("wall {wall_ev} EV, brush {size}: {m:?}");
        if m.offset.abs() > 1.0
            || m.rough > 0.3
            || m.worst > 1.2
            || m.lost > 0
            || m.leak > 0.005
            || m.specks > 0
            || m.holes > 0
            || m.pieces != 1
            || m.live > 1e-4
        {
            failed.push(format!("{size} {wall_ev}: {m:?}"));
        }
    }
    assert!(failed.is_empty(), "{failed:#?}");
}
