//! Auto Mask on a constructed scene like a photo it failed on: a white object, shaded from bright
//! on its left to the wall's own brightness on its right (only its warmer colour tells them apart
//! there), the object's dark shadow on the wall right under it, and a mid-grey wall around both.
//! The brush is wide enough to reach all three. With its centre on the object, the paint should
//! fill the object solidly to its edges and stay off the shadow and the wall; with its centre on
//! the shadow, fill the shadow and stay off the object and the wall; and where the paint ends
//! inside the brush's reach it should be at the object's edge, not at the brush's footprint.
//!
//! `cargo test -p craft-develop --lib scene_tests -- --nocapture` prints the numbers.

use super::*;
use crate::geometry::Frame;
use crate::masks::brush_dabs;
use craft_develop_model::{BrushStroke, DevelopSettings};

/// Regions of the scene, in units of a 400 × 300 frame (scaled for larger renders).
const OBJ: (f64, f64, f64, f64) = (60.0, 40.0, 230.0, 170.0);
const SHADOW: (f64, f64, f64, f64) = (80.0, 170.0, 260.0, 230.0);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Region {
    Object,
    Shadow,
    Wall,
}

fn region_at(x: f64, y: f64) -> Region {
    let inside = |r: (f64, f64, f64, f64)| x >= r.0 && x < r.2 && y >= r.1 && y < r.3;
    if inside(OBJ) {
        Region::Object
    } else if inside(SHADOW) {
        Region::Shadow
    } else {
        Region::Wall
    }
}

fn hash(x: usize, y: usize, seed: u64) -> f32 {
    let mut v =
        (x as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (y as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F) ^ seed.wrapping_mul(0x1656_67B1_9E37_79F9);
    v ^= v >> 29;
    v = v.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    v ^= v >> 32;
    (v % 10_000) as f32 / 5_000.0 - 1.0
}

/// The scene at `k` times 400 × 300: (colours, log luminance, region of each pixel).
fn scene(k: f64) -> (Rgb32f, Plane, Vec<Region>, usize, usize) {
    let (w, h) = ((400.0 * k) as usize, (300.0 * k) as usize);
    let wall = 0.2f32;
    let warm = [1.08f32, 0.99, 0.82];
    let neutral = [1.04f32, 0.99, 0.91];
    let mut regions = vec![Region::Wall; w * h];
    let img = Rgb32f::from_fn(w, h, |x, y| {
        let (u, v) = ((x as f64 + 0.5) / k, (y as f64 + 0.5) / k);
        let r = region_at(u, v);
        let (lum, chroma) = match r {
            // two stops over the wall on the lit side, falling steeply to the wall's brightness
            Region::Object => {
                let t = ((u - OBJ.0) / (OBJ.2 - OBJ.0)) as f32;
                (wall * (2.0 * (1.0 - t * t * t)).exp2(), warm)
            }
            Region::Shadow => (wall * (-2.5f32).exp2(), neutral),
            Region::Wall => (wall, neutral),
        };
        // a little noise, in proportion to the pixel (finer at the larger size, as a sensor's)
        let n = 0.04 / k.sqrt() as f32;
        let g = (n * hash(x, y, 1)).exp2();
        std::array::from_fn(|c| lum * chroma[c] * g * (1.0 + 0.3 * n * hash(x, y, 2 + c as u64)))
    });
    for y in 0..h {
        for x in 0..w {
            regions[y * w + x] = region_at((x as f64 + 0.5) / k, (y as f64 + 0.5) / k);
        }
    }
    let l = img.map(crate::local::log_lum);
    (img, l, regions, w, h)
}

fn stroke(points: &[(f64, f64)], auto_mask: bool) -> BrushStroke {
    BrushStroke {
        points: points.iter().map(|p| Point::new(p.0 / 400.0, p.1 / 300.0)).collect(),
        size: 0.08,
        feather: 50.0,
        flow: 100.0,
        auto_mask,
        ..BrushStroke::default()
    }
}

fn painted(s: &BrushStroke, img: &Rgb32f, l: &Plane, w: usize, h: usize, model: AutoMaskModel) -> Plane {
    let f = Frame::new(w, h, &DevelopSettings::default(), true);
    let mut r = StrokeRaster::with_model(w, h, model);
    r.update(&brush_dabs(s, &f, w, h), img, l);
    r.to_plane()
}

/// Centre on the object, near its shaded right side and its bottom edge over the shadow, the
/// brush's circle reaching over the wall and the shadow.
const ON_OBJECT: [(f64, f64); 6] = [(200.0, 110.0), (218.0, 125.0), (220.0, 150.0), (205.0, 160.0), (185.0, 158.0), (210.0, 140.0)];
/// Centre on the shadow, the circle reaching over the object above it and the wall below.
const ON_SHADOW: [(f64, f64); 4] = [(110.0, 190.0), (160.0, 185.0), (200.0, 192.0), (245.0, 188.0)];

#[derive(Debug, Default)]
struct Measure {
    /// The target's pixels the plain brush paints fully (2 px or more inside its edges): the share
    /// painted 0.9 or more.
    cover: f32,
    /// Mean paint on each other region where the plain brush paints at all (2 px or more from it).
    leak: Vec<(Region, f32)>,
    /// Where the paint crosses 0.5 well inside the brush's reach (the plain brush paints 0.6 or
    /// more there, so it is not the brush's own edge): the mean distance (px) to the target's true
    /// edge.
    boundary: f32,
}

fn measure(k: f64, pts: &[(f64, f64)], target: Region, model: AutoMaskModel) -> Measure {
    let (img, l, regions, w, h) = scene(k);
    let plain = painted(&stroke(pts, false), &img, &l, w, h, model);
    let a = painted(&stroke(pts, true), &img, &l, w, h, model);
    let at = |x: i64, y: i64| (x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h).then(|| regions[y as usize * w + x as usize]);
    // distance of each pixel to the nearest pixel of another region (small scenes: brute force
    // over a window)
    let near_edge = |x: usize, y: usize, d: i64| {
        let r = regions[y * w + x];
        (-d..=d).any(|dy| (-d..=d).any(|dx| at(x as i64 + dx, y as i64 + dy).is_some_and(|o| o != r)))
    };
    let edge_px: Vec<(usize, usize)> = (1..h - 1)
        .flat_map(|y| (1..w - 1).map(move |x| (x, y)))
        .filter(|&(x, y)| {
            let in_t = regions[y * w + x] == target;
            in_t && [(1i64, 0i64), (-1, 0), (0, 1), (0, -1)].iter().any(|(dx, dy)| at(x as i64 + dx, y as i64 + dy).is_some_and(|o| o != target))
        })
        .collect();
    let (mut n, mut full) = (0f32, 0f32);
    let mut leak: Vec<(Region, f32, f32)> =
        [Region::Object, Region::Shadow, Region::Wall].into_iter().filter(|r| *r != target).map(|r| (r, 0.0, 0.0)).collect();
    let (mut bn, mut bsum) = (0f32, 0f32);
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let i = y * w + x;
            if plain.data[i] < 0.05 {
                continue;
            }
            let r = regions[i];
            let edge = near_edge(x, y, (2.0 * k).ceil() as i64);
            if r == target && !edge && plain.data[i] >= 0.95 {
                n += 1.0;
                full += f32::from(u8::from(a.data[i] >= 0.9));
            } else if r != target
                && !edge
                && let Some(e) = leak.iter_mut().find(|e| e.0 == r)
            {
                e.1 += a.data[i];
                e.2 += 1.0;
            }
            let crosses = a.data[i] >= 0.5
                && [1, w].iter().any(|&o| a.data.get(i + o).is_some_and(|v| *v < 0.5) || a.data.get(i - o).is_some_and(|v| *v < 0.5));
            if crosses && plain.data[i] >= 0.6 {
                let d = edge_px
                    .iter()
                    .map(|&(ex, ey)| ((ex as f32 - x as f32).powi(2) + (ey as f32 - y as f32).powi(2)).sqrt())
                    .fold(f32::INFINITY, f32::min);
                bn += 1.0;
                bsum += d / k as f32;
            }
        }
    }
    Measure {
        cover: full / n.max(1.0),
        leak: leak.into_iter().map(|(r, s, c)| (r, s / c.max(1.0))).collect(),
        boundary: if bn > 0.0 { bsum / bn } else { f32::INFINITY },
    }
}

/// A case: its name, the stroke's points, and the region its centre is on.
type Case<'a> = (&'a str, &'a [(f64, f64)], Region);

#[test]
fn auto_mask_fills_the_object_or_its_shadow_to_their_edges() {
    let cases: [Case<'_>; 2] = [("centre on object", &ON_OBJECT, Region::Object), ("centre on shadow", &ON_SHADOW, Region::Shadow)];
    eprintln!("{:<18} {:>5} | {:>6} {:>18} {:>18} {:>9}", "case", "model", "cover", "leak", "leak", "boundary");
    for (name, pts, target) in cases {
        for model in [AutoMaskModel::PerPixel, AutoMaskModel::Smoothed] {
            let m = measure(1.0, pts, target, model);
            let leaks: Vec<String> = m.leak.iter().map(|(r, v)| format!("{r:?} {v:.3}")).collect();
            eprintln!(
                "{name:<18} {:>5} | {:>6.3} {:>18} {:>18} {:>7.2}px",
                if model == AutoMaskModel::PerPixel { "v1-8" } else { "v9" },
                m.cover,
                leaks[0],
                leaks[1],
                m.boundary
            );
            if model == AutoMaskModel::Smoothed {
                assert!(m.cover >= 0.97, "{name}: the target filled solidly ({})", m.cover);
                for (r, v) in &m.leak {
                    assert!(*v < 0.03, "{name}: no paint on the {r:?} ({v})");
                }
                assert!(m.boundary < 1.5, "{name}: the paint ends at the edge ({} px off)", m.boundary);
            }
        }
    }
}

/// The scene rendered at 2.5 times the size (as an export is to the preview) paints the same shape.
#[test]
fn auto_mask_paints_the_same_shape_at_preview_and_full_size() {
    for (pts, _) in [(&ON_OBJECT[..], Region::Object), (&ON_SHADOW[..], Region::Shadow)] {
        let (img, l, _, w, h) = scene(1.0);
        let small = painted(&stroke(pts, true), &img, &l, w, h, AutoMaskModel::Smoothed);
        let (img, l, _, bw, bh) = scene(2.5);
        let big = painted(&stroke(pts, true), &img, &l, bw, bh, AutoMaskModel::Smoothed);
        // the large one sampled at the small one's pixel centres
        let (mut differ, mut painted_px) = (0f32, 0f32);
        for y in 0..h {
            for x in 0..w {
                let (bx, by) = (((x as f64 + 0.5) * 2.5) as usize, ((y as f64 + 0.5) * 2.5) as usize);
                let (s, b) = (small.data[y * w + x] >= 0.5, big.data[by * bw + bx] >= 0.5);
                painted_px += f32::from(u8::from(s || b));
                differ += f32::from(u8::from(s != b));
            }
        }
        if let Ok(dir) = std::env::var("CRAFT_SCENE_OUT") {
            let mut out = format!("P6\n{w} {h}\n255\n").into_bytes();
            for y in 0..h {
                for x in 0..w {
                    let (bx, by) = (((x as f64 + 0.5) * 2.5) as usize, ((y as f64 + 0.5) * 2.5) as usize);
                    let p = img.data[by * bw + bx].map(|v| (v.max(0.0).powf(1.0 / 2.2) * 200.0).min(255.0) as u8);
                    let (s, b) = (small.data[y * w + x] >= 0.5, big.data[by * bw + bx] >= 0.5);
                    out.extend_from_slice(&match (s, b) {
                        (true, true) => [200, 0, 0],
                        (true, false) => [0, 255, 0],
                        (false, true) => [0, 0, 255],
                        _ => p,
                    });
                }
            }
            let _ = std::fs::write(format!("{dir}/scene-{}.ppm", pts.len()), out);
        }
        let share = differ / painted_px.max(1.0);
        eprintln!("preview vs full size: {:.1} % of the painted pixels differ", 100.0 * share);
        assert!(share < 0.04, "the shapes differ over {share}");
    }
}
