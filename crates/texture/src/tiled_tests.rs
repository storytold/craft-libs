//! Height to normal and ORM packing over a read-only adapter of 256 × 256 copy-on-write tiles of
//! interleaved two-channel U8, U16, F16 or F32 samples, with absent tiles reading a default pixel.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::normal::{HeightNormalOptions, NormalConvention, encode_normal, normal_from_neighborhood, try_height_to_normal};
use crate::packing::{OrmSources, try_pack_orm};
use crate::{Image, Limits, ScalarSource, SourceError};

const TILE: u32 = 256;
/// Channels per stored pixel; the adapter reads channel 0.
const CHANNELS: usize = 2;

#[derive(Clone, Copy, Debug)]
enum SampleType {
    U8,
    U16,
    F16,
    F32,
}

impl SampleType {
    fn size(self) -> usize {
        match self {
            SampleType::U8 => 1,
            SampleType::U16 | SampleType::F16 => 2,
            SampleType::F32 => 4,
        }
    }

    fn encode(self, v: f32, out: &mut Vec<u8>) {
        match self {
            SampleType::U8 => out.push((v.clamp(0.0, 1.0) * 255.0).round() as u8),
            SampleType::U16 => out.extend(((v.clamp(0.0, 1.0) * 65535.0).round() as u16).to_le_bytes()),
            SampleType::F16 => out.extend(half::f16::from_f32(v).to_bits().to_le_bytes()),
            SampleType::F32 => out.extend(v.to_le_bytes()),
        }
    }

    /// The linear value of one stored sample, without a transfer curve.
    fn decode(self, b: &[u8]) -> f32 {
        match (self, b) {
            (SampleType::U8, [v, ..]) => f32::from(*v) / 255.0,
            (SampleType::U16, [a, b, ..]) => f32::from(u16::from_le_bytes([*a, *b])) / 65535.0,
            (SampleType::F16, [a, b, ..]) => half::f16::from_bits(u16::from_le_bytes([*a, *b])).to_f32(),
            (SampleType::F32, [a, b, c, d, ..]) => f32::from_le_bytes([*a, *b, *c, *d]),
            _ => panic!("short sample"),
        }
    }
}

/// A tiled surface read through [`ScalarSource`] without copying it.
struct Tiled {
    w: u32,
    h: u32,
    ty: SampleType,
    tiles: BTreeMap<(u32, u32), Arc<Vec<u8>>>,
    /// One stored pixel, read where a tile is absent.
    default: Vec<u8>,
    reads: AtomicUsize,
}

impl Tiled {
    /// A `w × h` surface whose channel 0 holds `f(x, y)` and channel 1 holds a value that must not
    /// be read. The tiles in `absent` are not stored.
    fn new(w: u32, h: u32, ty: SampleType, f: impl Fn(u32, u32) -> f32, absent: &[(u32, u32)], default: f32) -> Tiled {
        let mut tiles = BTreeMap::new();
        for ty_ in 0..h.div_ceil(TILE) {
            for tx in 0..w.div_ceil(TILE) {
                if absent.contains(&(tx, ty_)) {
                    continue;
                }
                let mut bytes = Vec::with_capacity((TILE * TILE) as usize * CHANNELS * ty.size());
                for y in ty_ * TILE..(ty_ + 1) * TILE {
                    for x in tx * TILE..(tx + 1) * TILE {
                        ty.encode(if x < w && y < h { f(x, y) } else { 0.0 }, &mut bytes);
                        ty.encode(0.37, &mut bytes);
                    }
                }
                tiles.insert((tx, ty_), Arc::new(bytes));
            }
        }
        let mut px = Vec::new();
        ty.encode(default, &mut px);
        ty.encode(0.37, &mut px);
        Tiled { w, h, ty, tiles, default: px, reads: AtomicUsize::new(0) }
    }

    /// The sample at `(x, y)`, looked up on its own.
    fn value(&self, x: u32, y: u32) -> f32 {
        match self.tiles.get(&(x / TILE, y / TILE)) {
            Some(t) => {
                let i = (((y % TILE) * TILE + x % TILE) as usize) * CHANNELS * self.ty.size();
                self.ty.decode(&t[i..])
            }
            None => self.ty.decode(&self.default),
        }
    }

    /// The same samples as one untiled gray image.
    fn untiled(&self) -> Image {
        Image::gray(self.w, self.h, |x, y| self.value(x, y))
    }
}

impl ScalarSource for Tiled {
    fn dimensions(&self) -> (u32, u32) {
        (self.w, self.h)
    }

    /// Walk the tile segments of the row, holding each tile while its segment is decoded.
    fn read_row(&self, y: u32, x0: u32, out: &mut [f32]) -> Result<(), SourceError> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        if y >= self.h || x0 as usize + out.len() > self.w as usize {
            return Err(SourceError("outside the surface".into()));
        }
        let stride = CHANNELS * self.ty.size();
        let mut x = x0;
        let mut rest = out;
        while !rest.is_empty() {
            let n = ((TILE - x % TILE) as usize).min(rest.len());
            let (seg, tail) = rest.split_at_mut(n);
            match self.tiles.get(&(x / TILE, y / TILE)) {
                Some(tile) => {
                    let start = (((y % TILE) * TILE + x % TILE) as usize) * stride;
                    for (o, px) in seg.iter_mut().zip(tile[start..start + n * stride].chunks_exact(stride)) {
                        *o = self.ty.decode(px);
                    }
                }
                None => seg.fill(self.ty.decode(&self.default)),
            }
            x += n as u32;
            rest = tail;
        }
        Ok(())
    }
}

fn normals<S: ScalarSource + ?Sized>(src: &S, options: &HeightNormalOptions) -> Image {
    match try_height_to_normal(src, options, &Limits::default()) {
        Ok(img) => img,
        Err(e) => panic!("normal map failed: {e}"),
    }
}

/// A rough, non-planar height in `0..=1`.
fn bumpy(x: u32, y: u32) -> f32 {
    ((x * 7 + y * 13 + x * y) % 17) as f32 / 16.0
}

/// Normals and ORM values over tiles of `ty` match the same samples untiled, and the samples
/// decode to `0.2`, `0.6` and `1.0` within `tol`.
fn check_sample_type(ty: SampleType, tol: f32) {
    let height = Tiled::new(300, 270, ty, bumpy, &[(1, 1)], 0.5);
    let opts = HeightNormalOptions { strength: 3.0, ..HeightNormalOptions::default() };
    let tiled = normals(&height, &opts);
    assert_eq!(tiled, normals(&height.untiled(), &opts), "{ty:?}");
    // An absent tile reads its default pixel, which is flat.
    assert_eq!(tiled.get(280, 260, false), [0.5, 0.5, 1.0, 1.0]);

    let ao = Tiled::new(300, 270, ty, |_, _| 0.2, &[], 0.0);
    let rough = Tiled::new(300, 270, ty, |_, _| 0.6, &[], 0.0);
    let metal = Tiled::new(300, 270, ty, |_, _| 1.0, &[(0, 0)], 0.6);
    let s = OrmSources { occlusion: Some(&ao), roughness: Some(&rough), metallic: Some(&metal), ..OrmSources::default() };
    let Ok(orm) = try_pack_orm(&s, &Limits::default()) else { panic!("packing {ty:?}") };
    // Metallic's tile (0, 0) is absent and reads its default, 0.6.
    for (x, y, m) in [(0u32, 0u32, 0.6f32), (299, 269, 1.0), (256, 3, 1.0)] {
        let p = orm.get(i64::from(x), i64::from(y), false);
        let want = [0.2, 0.6, m, 1.0];
        assert!(p.iter().zip(want).all(|(a, b)| (a - b).abs() <= tol), "{ty:?} ({x}, {y}): {p:?} vs {want:?}");
        assert_eq!(p, [ao.value(x, y), rough.value(x, y), metal.value(x, y), 1.0], "{ty:?} ({x}, {y})");
    }
    assert!((orm.get(1, 1, false)[0] - 0.2).abs() <= tol && (orm.get(1, 1, false)[1] - 0.6).abs() <= tol);
}

#[test]
fn u8_tiles_give_the_untiled_normal_and_orm() {
    check_sample_type(SampleType::U8, 1e-7);
}

#[test]
fn u16_tiles_give_the_untiled_normal_and_orm() {
    check_sample_type(SampleType::U16, 1e-7);
}

#[test]
fn f16_tiles_give_the_untiled_normal_and_orm() {
    check_sample_type(SampleType::F16, 1e-3);
    // Binary16 rounds 0.2 to 0.199951171875.
    assert_eq!(SampleType::F16.decode(&half::f16::from_f32(0.2).to_bits().to_le_bytes()), 0.199_951_17);
}

#[test]
fn f32_tiles_give_the_untiled_normal_and_orm() {
    check_sample_type(SampleType::F32, 0.0);
}

#[test]
fn storage_tile_seams_read_the_neighbor_tile() {
    let (a, b, s) = (0.011f32, -0.004f32, 4.0f32);
    let plane = |x: u32, y: u32| a * x as f32 + b * y as f32;
    let height = Tiled::new(300, 300, SampleType::F32, plane, &[], 0.0);
    let n = normals(&height, &HeightNormalOptions { strength: s, ..HeightNormalOptions::default() });
    let untiled = normals(&height.untiled(), &HeightNormalOptions { strength: s, ..HeightNormalOptions::default() });
    let len = ((a * s).powi(2) + (b * s).powi(2) + 1.0).sqrt();
    let want = [-a * s / len, b * s / len, 1.0 / len].map(|c| 0.5 * c + 0.5);
    for (x, y) in [(255, 100), (256, 100), (100, 255), (100, 256), (255, 255), (256, 256), (255, 256), (256, 255)] {
        let p = n.get(x, y, false);
        assert!(p.iter().zip(want).all(|(p, w)| (p - w).abs() < 1e-4), "({x}, {y}): {p:?} vs {want:?}");
        assert_eq!(p, untiled.get(x, y, false), "({x}, {y})");
        let h = |dx: i64, dy: i64| height.value((x + dx) as u32, (y + dy) as u32);
        let hood = [[h(-1, -1), h(0, -1), h(1, -1)], [h(-1, 0), h(0, 0), h(1, 0)], [h(-1, 1), h(0, 1), h(1, 1)]];
        let [r, g, b] = encode_normal(normal_from_neighborhood(hood, s), NormalConvention::OpenGl);
        assert_eq!(p, [r, g, b, 1.0], "({x}, {y}) reads both tiles");
    }
}

#[test]
fn rows_are_read_as_needed_without_a_full_copy() {
    let height = Tiled::new(300, 270, SampleType::U16, bumpy, &[], 0.0);
    let _ = normals(&height, &HeightNormalOptions::default());
    let reads = height.reads.load(Ordering::Relaxed);
    // Each row once; with the `parallel` feature each band of 64 rows also reads the row on
    // either side of it.
    #[cfg(not(feature = "parallel"))]
    assert_eq!(reads, 270);
    #[cfg(feature = "parallel")]
    assert!((270..=270 + 2 * 270usize.div_ceil(64)).contains(&reads), "{reads}");
}
