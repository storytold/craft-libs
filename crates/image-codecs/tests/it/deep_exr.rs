//! Deep EXR: a synthetic generator builds deep chunks per the OpenEXR file layout
//! specification (NONE, RLE and ZIPS × scanline and tile), the oracle tests check the
//! compositing math against hand-computed values, and malformed inputs must error, never
//! panic. With the `corpus` feature, the OpenEXR project's deep test images (`corpus/exr/`,
//! each checked against its pinned SHA-256 before decoding) are checked against the ID
//! manifests their sidecars list.

use craft_image_codecs::CodecError;
use craft_image_codecs::f16;
use craft_image_codecs::{Limits, decode, decode_deep_exr};
use sha2::{Digest, Sha256};
use std::io::Read as _;
use std::io::Write as _;

// ---------------------------------------------------------------------------------------------
// A minimal deep-EXR writer, mirroring what the reference files show on disk:
// version 2 | deep bit (and the tile bit for tiles), one header with the mandatory
// attributes in alphabetical order, a chunk offset table, then deep chunks of
// [y | tile coords][u64 packed table][u64 packed data][u64 unpacked data][table][data].
// The table holds u32 sample counts, cumulative within each row of the block; the sample
// data is grouped per row, then per channel, in pixel order (the layout the OpenEXR library
// writes). NONE stores raw; RLE and ZIPS separate even/odd bytes, apply the +128 predictor,
// then RLE packs runs and ZIPS deflates.

fn attr(out: &mut Vec<u8>, name: &str, ty: &str, payload: &[u8]) {
    out.extend_from_slice(name.as_bytes());
    out.push(0);
    out.extend_from_slice(ty.as_bytes());
    out.push(0);
    out.extend_from_slice(&(payload.len() as i32).to_le_bytes());
    out.extend_from_slice(payload);
}

fn samples_to_differences(buf: &mut [u8]) {
    if let Some(&first) = buf.first() {
        let mut prev = i16::from(first);
        for b in buf.iter_mut().skip(1) {
            let v = i16::from(*b);
            *b = (v - prev + 128) as u8;
            prev = v;
        }
    }
}

fn separate_bytes_fragments(buf: &mut [u8]) {
    let copy = buf.to_vec();
    let half = copy.len().div_ceil(2);
    for i in 0..copy.len() {
        buf[if i % 2 == 0 { i / 2 } else { half + i / 2 }] = copy[i];
    }
}

fn rle_bytes(src: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < src.len() {
        let mut run = 1;
        while i + run < src.len() && run < 128 && src[i + run] == src[i] {
            run += 1;
        }
        // OpenEXR tokens: `n >= 0` repeats the next byte n + 1 times, `n < 0` copies -n bytes.
        if run > 1 {
            out.push((run - 1) as u8);
            out.push(src[i]);
            i += run;
        } else {
            let start = i;
            i += 1;
            while i < src.len() && (i - start) < 127 && src[i] != src[i - 1] {
                i += 1;
            }
            let n = i - start;
            out.push((-(n as i32)) as u8);
            out.extend_from_slice(&src[start..start + n]);
        }
    }
    out
}

fn compress(kind: u8, raw: &[u8]) -> Vec<u8> {
    match kind {
        1 => {
            // Like ZIPS, RLE packs the byte-split, predicted stream.
            let mut b = raw.to_vec();
            separate_bytes_fragments(&mut b);
            samples_to_differences(&mut b);
            let c = rle_bytes(&b);
            if c.len() < raw.len() { c } else { raw.to_vec() }
        }
        2 => {
            let mut b = raw.to_vec();
            separate_bytes_fragments(&mut b);
            samples_to_differences(&mut b);
            let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
            // Writing to a Vec never fails; the encoder errors surface in `finish`.
            #[allow(clippy::unwrap_used)]
            z.write_all(&b).unwrap();
            #[allow(clippy::unwrap_used)]
            let c = z.finish().unwrap();
            if c.len() < raw.len() { c } else { raw.to_vec() }
        }
        _ => raw.to_vec(),
    }
}

/// One channel: a name (the caller must pass channels in alphabetical order, as chlist
/// requires) and the sample type as in the chlist attribute (0 = u32, 1 = half, 2 = float).
struct Chan {
    name: &'static str,
    ty: i32,
}

/// `counts[p]` samples per pixel; `sample(p, ci)` returns channel `ci`'s values for pixel
/// `p` (its length must be `counts[p]`). `tile` switches between deepscanline and deeptile.
#[allow(clippy::too_many_arguments)]
fn gen_deep(
    w: usize,
    h: usize,
    channels: &[Chan],
    compression: u8,
    tile: Option<(usize, usize)>,
    counts: &[u32],
    sample: &dyn Fn(usize, usize) -> Vec<f32>,
) -> Vec<u8> {
    gen_deep_tables(w, h, channels, compression, tile, counts, sample, false)
}

/// [`gen_deep`]; with `full_tile_tables` every tile's count table is stored at the full tile
/// size, as the OpenEXR library writes edge tiles: the block's entries first (in rows of its
/// clipped width), then zeros.
#[allow(clippy::too_many_arguments)]
fn gen_deep_tables(
    w: usize,
    h: usize,
    channels: &[Chan],
    compression: u8,
    tile: Option<(usize, usize)>,
    counts: &[u32],
    sample: &dyn Fn(usize, usize) -> Vec<f32>,
    full_tile_tables: bool,
) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&0x0131_2f76u32.to_le_bytes());
    // Single-part deep files (deepscanline and deeptile alike) carry only the deep-data
    // flag; the tiling lives in the `tiles` attribute and the `type` string.
    out.extend_from_slice(&(2u32 | 0x800).to_le_bytes());

    let mut chlist = Vec::new();
    for c in channels {
        chlist.extend_from_slice(c.name.as_bytes());
        chlist.push(0);
        chlist.extend_from_slice(&c.ty.to_le_bytes());
        chlist.push(0); // pLinear
        chlist.extend_from_slice(&[0u8; 3]);
        chlist.extend_from_slice(&1i32.to_le_bytes());
        chlist.extend_from_slice(&1i32.to_le_bytes());
    }
    chlist.push(0);
    attr(&mut out, "channels", "chlist", &chlist);
    attr(&mut out, "compression", "compression", &[compression]);
    // box2i is inclusive: the data window of a w×h image at the origin ends at (w-1, h-1).
    let dw: Vec<u8> = [0i32, 0, w as i32 - 1, h as i32 - 1].iter().flat_map(|v| v.to_le_bytes()).collect();
    attr(&mut out, "dataWindow", "box2i", &dw);
    attr(&mut out, "displayWindow", "box2i", &dw);
    attr(&mut out, "lineOrder", "lineOrder", &[0]);
    attr(&mut out, "pixelAspectRatio", "float", &1f32.to_le_bytes());
    attr(&mut out, "screenWindowCenter", "v2f", &[0f32.to_le_bytes(), 0f32.to_le_bytes()].concat());
    attr(&mut out, "screenWindowWidth", "float", &1f32.to_le_bytes());
    if let Some((tw, th)) = tile {
        let mut td = Vec::new();
        td.extend_from_slice(&(tw as u32).to_le_bytes());
        td.extend_from_slice(&(th as u32).to_le_bytes());
        td.push(0); // one level, round down
        attr(&mut out, "tiles", "tiledesc", &td);
    }
    attr(&mut out, "type", "string", if tile.is_some() { b"deeptile" } else { b"deepscanline" });
    attr(&mut out, "version", "int", &1i32.to_le_bytes());
    out.push(0); // end of the header

    // The blocks: scan lines (one line each) or the level-0 tiles.
    struct Block {
        pixels: Vec<usize>,
        /// Pixels per row: the count table restarts and the sample data is grouped per row.
        row_w: usize,
    }
    let blocks: Vec<Block> = match tile {
        None => (0..h).map(|y| Block { pixels: (y * w..y * w + w).collect(), row_w: w }).collect(),
        Some((tw, th)) => {
            let tx = w.div_ceil(tw);
            let ty = h.div_ceil(th);
            let mut v = Vec::new();
            for by in 0..ty {
                for bx in 0..tx {
                    let mut px = Vec::new();
                    for yy in (by * th)..((by * th + th).min(h)) {
                        for xx in (bx * tw)..((bx * tw + tw).min(w)) {
                            px.push(yy * w + xx);
                        }
                    }
                    v.push(Block { pixels: px, row_w: (bx * tw + tw).min(w) - bx * tw });
                }
            }
            v
        }
    };

    let table_len = |b: &Block| b.pixels.len() * 4;
    let data_len = |b: &Block| -> usize {
        let samples: u64 = b.pixels.iter().map(|&p| counts[p] as u64).sum();
        channels.iter().map(|c| samples as usize * if c.ty == 1 { 2 } else { 4 }).sum()
    };

    // Build every chunk, remember the offsets, then write the offset table first.
    let mut chunks: Vec<(usize, Vec<u8>)> = Vec::new(); // (block index, framed chunk)
    for (bi, b) in blocks.iter().enumerate() {
        let mut c = Vec::new();
        match tile {
            None => {
                let y = (bi % h) as i32;
                c.extend_from_slice(&y.to_le_bytes());
            }
            Some((tw, _)) => {
                let tx = w.div_ceil(tw);
                let (bx, by) = (bi % tx, bi / tx);
                c.extend_from_slice(&(bx as i32).to_le_bytes());
                c.extend_from_slice(&(by as i32).to_le_bytes());
                c.extend_from_slice(&0i32.to_le_bytes());
                c.extend_from_slice(&0i32.to_le_bytes());
            }
        }
        let mut table = Vec::with_capacity(b.pixels.len() * 4);
        for row in b.pixels.chunks(b.row_w) {
            let mut acc = 0u32;
            for &p in row {
                acc += counts[p];
                table.extend_from_slice(&acc.to_le_bytes());
            }
        }
        assert_eq!(table.len(), table_len(b));
        if let (true, Some((tw, th))) = (full_tile_tables, tile) {
            table.resize(tw * th * 4, 0);
        }
        let mut data = Vec::new();
        for row in b.pixels.chunks(b.row_w) {
            for (ci, ch) in channels.iter().enumerate() {
                for &p in row {
                    for v in sample(p, ci) {
                        match ch.ty {
                            1 => data.extend_from_slice(&f16::from_f32(v).to_le_bytes()),
                            0 => data.extend_from_slice(&(v as u32).to_le_bytes()),
                            _ => data.extend_from_slice(&v.to_le_bytes()),
                        }
                    }
                }
            }
        }
        let ct = compress(compression, &table);
        let cd = compress(compression, &data);
        c.extend_from_slice(&(ct.len() as u64).to_le_bytes());
        c.extend_from_slice(&(cd.len() as u64).to_le_bytes());
        c.extend_from_slice(&(data.len() as u64).to_le_bytes());
        c.extend_from_slice(&ct);
        c.extend_from_slice(&cd);
        assert_eq!(data.len(), data_len(b));
        chunks.push((bi, c));
    }
    let mut pos = out.len() + chunks.len() * 8;
    let mut table = Vec::new();
    for (_, c) in &chunks {
        table.extend_from_slice(&(pos as u64).to_le_bytes());
        pos += c.len();
    }
    out.extend_from_slice(&table);
    for (_, c) in chunks {
        out.extend_from_slice(&c);
    }
    out
}

/// RGBA float channels in chlist (alphabetical) order, no Z (everything composites).
const RGBA_NO_Z: [Chan; 4] = [Chan { name: "A", ty: 2 }, Chan { name: "B", ty: 2 }, Chan { name: "G", ty: 2 }, Chan { name: "R", ty: 2 }];

/// RGBA(Z) float channels in chlist (alphabetical) order.
const RGBAZ: [Chan; 5] =
    [Chan { name: "A", ty: 2 }, Chan { name: "B", ty: 2 }, Chan { name: "G", ty: 2 }, Chan { name: "R", ty: 2 }, Chan { name: "Z", ty: 2 }];

fn sample_rgba(p: usize, ci: usize) -> Vec<f32> {
    // One sample per pixel: a small checkerboard-ish pattern with varying depth. Deep
    // colours are premultiplied, so the stored RGB values include the 0.5 alpha.
    let v = [0.25f32, 0.5, 0.75, 1.0][p % 4];
    match ci {
        0 => vec![0.5],             // A
        1 => vec![0.125 * v],       // B = straight 0.25v, premultiplied
        2 => vec![0.25 * v],        // G = straight 0.5v, premultiplied
        3 => vec![0.5 * v],         // R = straight v, premultiplied
        _ => vec![10.0 + p as f32], // Z
    }
}

const COUNTS_ONE: &[u32] = &[1; 16]; // for a 4x4 image

fn rgba_flat(bytes: &[u8]) -> Vec<f32> {
    let img = decode(bytes).expect("decodes");
    let s = img.to_f32_samples().expect("f32");
    assert_eq!(img.layout(), craft_image_codecs::ChannelLayout::Rgba);
    assert_eq!(img.sample_type(), craft_image_codecs::SampleType::F32);
    assert!(img.warnings.iter().any(|w| matches!(w, craft_image_codecs::DecodeWarning::DeepFlattened { .. })));
    s
}

// ---------------------------------------------------------------------------------------------
// Oracles

/// One sample per pixel: the flat image is the stored colour un-premultiplied.
#[test]
fn single_sample_matches_flat() {
    let bytes = gen_deep(4, 4, &RGBAZ, 0, None, COUNTS_ONE, &sample_rgba);
    let px = rgba_flat(&bytes);
    for p in 0..16 {
        let v = [0.25f32, 0.5, 0.75, 1.0][p % 4];
        let (r, g, b, a) = (px[p * 4], px[p * 4 + 1], px[p * 4 + 2], px[p * 4 + 3]);
        assert!((r - v).abs() < 1e-6, "r at {p}: {r}");
        assert!((g - 0.5 * v).abs() < 1e-6, "g at {p}: {g}");
        assert!((b - 0.25 * v).abs() < 1e-6, "b at {p}: {b}");
        assert!((a - 0.5).abs() < 1e-6, "a at {p}: {a}");
    }
}

/// Two samples stored back-to-front: the front (smaller Z) covers the back. Hand-computed:
/// front a=0.5, premultiplied colour (0.5, 0.25, 0); back a=0.25, premultiplied (0.25, 0.25, 0.25).
/// Over (front first): a = 0.5 + (1−0.5)·0.25 = 0.625; c = (0.5 + 0.5·0.25, 0.25 + 0.5·0.25,
/// 0 + 0.5·0.25) = (0.625, 0.375, 0.125); straight = c / a = (1.0, 0.6, 0.2).
#[test]
fn z_order_composites_front_over_back() {
    // pixel 0: samples [(z=9, back), (z=1, front)] — the file order is reversed on purpose.
    let counts = vec![2u32; 16];
    let sample = |p: usize, ci: usize| -> Vec<f32> {
        if p != 0 {
            let v = sample_rgba(p, ci);
            return vec![v[0], v[0]];
        }
        match ci {
            0 => vec![0.25, 0.5],  // A: back first
            1 => vec![0.25, 0.0],  // B
            2 => vec![0.25, 0.25], // G
            3 => vec![0.25, 0.5],  // R
            _ => vec![9.0, 1.0],   // Z
        }
    };
    let bytes = gen_deep(4, 4, &RGBAZ, 0, None, &counts, &sample);
    let px = rgba_flat(&bytes);
    let a = 0.5f32 + 0.5 * 0.25;
    assert!((px[3] - a).abs() < 1e-6, "alpha {}", px[3]);
    assert!((px[0] - 0.625 / a).abs() < 1e-6, "r {}", px[0]);
    assert!((px[1] - 0.375 / a).abs() < 1e-6, "g {}", px[1]);
    assert!((px[2] - 0.125 / a).abs() < 1e-6, "b {}", px[2]);
}

/// A fully opaque front sample hides everything behind it.
#[test]
fn opaque_front_hides_back() {
    let counts = vec![3u32; 16];
    let sample = |p: usize, ci: usize| -> Vec<f32> {
        if p != 1 {
            let v = sample_rgba(p, ci);
            return vec![v[0], v[0], v[0]];
        }
        match ci {
            0 => vec![1.0, 0.9, 1.0], // front opaque, middle partial, back opaque
            1 => vec![0.0, 1.0, 1.0],
            2 => vec![1.0, 1.0, 1.0],
            3 => vec![1.0, 0.0, 0.0],
            _ => vec![1.0, 5.0, 9.0],
        }
    };
    let bytes = gen_deep(4, 4, &RGBAZ, 0, None, &counts, &sample);
    let px = rgba_flat(&bytes);
    let (r, g, b, a) = (px[4], px[5], px[6], px[7]);
    assert!((r - 1.0).abs() < 1e-6 && (g - 1.0).abs() < 1e-6 && (b - 0.0).abs() < 1e-6, "colour ({r},{g},{b})");
    assert!((a - 1.0).abs() < 1e-6, "alpha {a}");
}

/// Deep EXR is HDR: colour above 1.0 survives the composite (it used to be clamped), and a
/// tiled file whose edge tiles are narrower than the tile size decodes like the scan lines.
#[test]
fn hdr_colour_is_kept_and_edge_tiles_match_scanlines() {
    let (w, h) = (7, 5);
    let counts: Vec<u32> = (0..w * h).map(|p| (p as u32 * 3) % 4).collect();
    let sample = |p: usize, ci: usize| -> Vec<f32> {
        (0..(p as u32 * 3) % 4).map(|k| [1.0, 0.25 * k as f32, 0.5, 2.0 + p as f32 / 8.0, 10.0 - k as f32][ci]).collect()
    };
    for compression in [0u8, 1, 2] {
        let scan = rgba_flat(&gen_deep(w, h, &RGBAZ, compression, None, &counts, &sample));
        let tiled = rgba_flat(&gen_deep(w, h, &RGBAZ, compression, Some((3, 2)), &counts, &sample));
        assert_eq!(scan, tiled, "compression {compression}");
        // Pixel 1 has 3 opaque samples; the front one (k = 2, Z = 8) has R = 2.125.
        assert!((scan[4] - 2.125).abs() < 1e-5, "compression {compression}: R {}", scan[4]);
    }
}

/// Pixels without samples are transparent black, and the structured counts say so.
#[test]
fn empty_pixels_are_transparent() {
    let mut counts = vec![0u32; 16];
    counts[5] = 1;
    let bytes = gen_deep(4, 4, &RGBAZ, 0, None, &counts, &|p, ci| {
        if p == 5 { vec![[1.0f32, 0.5, 0.5, 0.5, 1.0][ci]] } else { Vec::new() }
    });
    let px = rgba_flat(&bytes);
    for p in 0..16 {
        let base = p * 4;
        if p == 5 {
            assert!((px[base + 3] - 1.0).abs() < 1e-6);
        } else {
            assert!(px[base..base + 4].iter().all(|v| *v == 0.0), "pixel {p} not empty");
        }
    }
    let deep = decode_deep_exr(&bytes, &Limits::none()).expect("deep");
    assert_eq!(deep.total_samples(), 1);
    assert_eq!(deep.sample_range(5).end, 1);
}

/// RLE, ZIPS and NONE decode to identical pixels and identical structured samples.
#[test]
fn rle_and_zips_match_uncompressed() {
    let counts: Vec<u32> = (0..16).map(|p| if p % 3 == 0 { 2 } else { 1 }).collect();
    let sample = |p: usize, ci: usize| -> Vec<f32> {
        if counts[p] == 2 {
            match ci {
                0 => vec![0.75, 0.25],
                1 => vec![0.1, 0.2],
                2 => vec![0.3, 0.4],
                3 => vec![0.5, 0.6],
                _ => vec![7.0, 3.0],
            }
        } else {
            sample_rgba(p, ci)
        }
    };
    let base = gen_deep(4, 4, &RGBAZ, 0, None, &counts, &sample);
    let px = rgba_flat(&base);
    let deep_base = decode_deep_exr(&base, &Limits::none()).expect("deep");
    for kind in [1u8, 2] {
        let bytes = gen_deep(4, 4, &RGBAZ, kind, None, &counts, &sample);
        let other = rgba_flat(&bytes);
        for (i, (a, b)) in px.iter().zip(other.iter()).enumerate() {
            assert!((a - b).abs() < 1e-6, "compression {kind}, sample {i}: {a} vs {b}");
        }
        let deep = decode_deep_exr(&bytes, &Limits::none()).expect("deep");
        assert_eq!(deep.counts, deep_base.counts);
        for (a, b) in deep.channels.iter().zip(deep_base.channels.iter()) {
            assert_eq!(a.name, b.name);
            assert_eq!(a.samples, b.samples, "channel {}", a.name);
        }
    }
}

/// Deep tiles decode to the same pixels as deep scanlines of the same data.
#[test]
fn tiles_match_scanlines() {
    let counts = vec![1u32, 0, 2, 1, 1, 1, 0, 1, 2, 1, 1, 0, 1, 1, 1, 1];
    let sample = |p: usize, ci: usize| -> Vec<f32> {
        if counts[p] == 2 {
            match ci {
                0 => vec![0.5, 0.5],
                1 => vec![0.2, 0.1],
                2 => vec![0.3, 0.3],
                3 => vec![0.4, 0.4],
                _ => vec![2.0, 8.0],
            }
        } else if counts[p] > 0 {
            vec![[0.5, 0.2, 0.4, 0.7, 4.0][ci]]
        } else {
            Vec::new()
        }
    };
    let sl = gen_deep(4, 4, &RGBAZ, 0, None, &counts, &sample);
    let tiled = gen_deep(4, 4, &RGBAZ, 0, Some((2, 2)), &counts, &sample);
    let a = rgba_flat(&sl);
    let b = rgba_flat(&tiled);
    for (i, (x, y)) in a.iter().zip(b.iter()).enumerate() {
        assert!((x - y).abs() < 1e-6, "tile sample {i}: {x} vs {y}");
    }
}

/// Half-float channels come back as F16 samples with exact values.
#[test]
fn half_channels_round_trip() {
    let channels =
        [Chan { name: "A", ty: 1 }, Chan { name: "B", ty: 1 }, Chan { name: "G", ty: 1 }, Chan { name: "R", ty: 1 }, Chan { name: "Z", ty: 1 }];
    let bytes = gen_deep(4, 4, &channels, 2, None, COUNTS_ONE, &|p, _ci| {
        let v = f16::from_f32(0.5).to_f32();
        vec![v * (p as f32 + 1.0) / 4.0]
    });
    let deep = decode_deep_exr(&bytes, &Limits::none()).expect("deep");
    for c in &deep.channels {
        assert_eq!(c.sample, craft_image_codecs::SampleType::F16, "{}", c.name);
    }
    let a = deep.channel("A").expect("A");
    assert_eq!(a.samples.len(), 16);
}

/// Without an alpha channel the front-most sample wins, opaque; luminance maps to gray.
#[test]
fn without_alpha_front_sample_is_opaque() {
    let channels = [Chan { name: "B", ty: 2 }, Chan { name: "G", ty: 2 }, Chan { name: "R", ty: 2 }, Chan { name: "Z", ty: 2 }];
    let counts = vec![2u32; 16];
    let bytes = gen_deep(4, 4, &channels, 0, None, &counts, &|p, ci| {
        if p % 2 == 0 {
            match ci {
                0 => vec![0.9, 0.1],
                1 => vec![0.2, 0.8],
                2 => vec![0.4, 0.6],
                _ => vec![2.0, 6.0],
            }
        } else {
            vec![[0.0, 0.0, 1.0, 1.0][ci], [0.0, 0.0, 1.0, 9.0][ci]]
        }
    });
    let img = decode(&bytes).expect("decodes");
    assert_eq!(img.layout(), craft_image_codecs::ChannelLayout::Rgb);
    let px = img.to_f32_samples().expect("f32");
    // Even pixels: the front sample (z=2) is (r 0.4, g 0.2, b 0.9).
    assert!((px[0] - 0.4).abs() < 1e-6 && (px[1] - 0.2).abs() < 1e-6 && (px[2] - 0.9).abs() < 1e-6);
}

/// Grayscale deep files (Y + Z) flatten to opaque gray, Y + A + Z keeps its alpha: the alpha
/// must land in the GrayA alpha slot, not a luminance copy (that was the bug).
#[test]
fn grayscale_deep_keeps_its_alpha() {
    let channels = [Chan { name: "A", ty: 2 }, Chan { name: "Y", ty: 2 }, Chan { name: "Z", ty: 2 }];
    let bytes = gen_deep(4, 4, &channels, 0, None, COUNTS_ONE, &|p, ci| {
        match ci {
            0 => vec![0.5],      // A
            1 => vec![0.5],      // Y, premultiplied: straight 1.0 × 0.5
            _ => vec![p as f32], // Z
        }
    });
    let img = decode(&bytes).expect("decodes");
    assert_eq!(img.layout(), craft_image_codecs::ChannelLayout::GrayA);
    let px = img.to_f32_samples().expect("f32");
    for (p, s) in px.chunks(2).enumerate() {
        let (y, a) = (s[0], s[1]);
        assert!((a - 0.5).abs() < 1e-6, "pixel {p}: alpha {a}, want 0.5");
        assert!((y - 1.0).abs() < 1e-6, "pixel {p}: luminance {y}, want the un-premultiplied 1.0");
    }

    // Without an alpha channel the same file is opaque gray.
    let channels = [Chan { name: "Y", ty: 2 }, Chan { name: "Z", ty: 2 }];
    let bytes = gen_deep(4, 4, &channels, 0, None, COUNTS_ONE, &|p, ci| match ci {
        0 => vec![0.4],
        _ => vec![p as f32],
    });
    let img = decode(&bytes).expect("decodes");
    assert_eq!(img.layout(), craft_image_codecs::ChannelLayout::Gray);
    let px = img.to_f32_samples().expect("f32");
    for (p, v) in px.iter().enumerate() {
        assert!((v - 0.4).abs() < 1e-6, "pixel {p}: {v}, want 0.4");
    }
}

/// decode_deep returns the counts and channels in file order.
#[test]
fn decode_deep_structure() {
    let bytes = gen_deep(4, 4, &RGBAZ, 0, None, COUNTS_ONE, &sample_rgba);
    let deep = decode_deep_exr(&bytes, &Limits::none()).expect("deep");
    assert_eq!((deep.width, deep.height), (4, 4));
    assert_eq!(deep.channels.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["A", "B", "G", "R", "Z"]);
    assert_eq!(deep.counts.len(), 17);
    assert_eq!(deep.counts[0], 0);
    assert_eq!(deep.total_samples(), 16);
    for p in 0..16 {
        assert_eq!(deep.sample_range(p).end - deep.sample_range(p).start, 1);
    }
    assert_eq!(deep.channel("Z").expect("Z").samples[5], 15.0);
}

// ---------------------------------------------------------------------------------------------
// Hostile input: every damaged file errors, nothing panics.

fn truncate_sweep(bytes: &[u8]) {
    // Cut at every offset (cap the count for speed) — a cut file must error, not panic.
    let step = (bytes.len() / 96).max(1);
    let mut i = step;
    while i < bytes.len() {
        let cut = &bytes[..i];
        let _ = decode_deep_exr(cut, &Limits::none());
        let _ = decode(cut);
        i += step;
    }
}

#[test]
fn truncated_files_error_never_panic() {
    for kind in [0u8, 1, 2] {
        let bytes = gen_deep(4, 4, &RGBAZ, kind, None, COUNTS_ONE, &sample_rgba);
        truncate_sweep(&bytes);
    }
    let tiled = gen_deep(4, 4, &RGBAZ, 1, Some((2, 2)), COUNTS_ONE, &sample_rgba);
    truncate_sweep(&tiled);
}

#[test]
fn hostile_sizes_and_offsets_error() {
    let bytes = gen_deep(4, 4, &RGBAZ, 0, None, COUNTS_ONE, &sample_rgba);
    // A tiny sample-data budget rejects the file.
    let tight = Limits { max_alloc: 64, ..Limits::none() };
    assert!(matches!(decode_deep_exr(&bytes, &tight), Err(CodecError::LimitExceeded(_))));
    // A non-deep file is unsupported for decode_deep.
    let flat = craft_image_codecs::encode(
        &craft_image_codecs::Image::from_u8(2, 2, craft_image_codecs::ChannelLayout::Rgba, [128; 16].to_vec()).expect("img"),
        craft_image_codecs::Format::OpenExr,
        &Default::default(),
    )
    .expect("encode");
    assert!(matches!(decode_deep_exr(&flat, &Limits::none()), Err(CodecError::Unsupported { .. })));
    // The last chunk's y coordinate destroyed: out of the data window.
    let mut broken = bytes.clone();
    let header_end = header_end_of(&broken);
    let last_off = u64::from_le_bytes(broken[header_end + 24..header_end + 32].try_into().expect("8")) as usize;
    broken[last_off..last_off + 4].copy_from_slice(&i32::MAX.to_le_bytes());
    assert!(decode_deep_exr(&broken, &Limits::none()).is_err());
    assert!(decode(&broken).is_err());
    // Two chunks claiming the same block: duplicate offsets.
    let mut dup = bytes.clone();
    let header_end = header_end_of(&dup);
    let first: [u8; 8] = dup[header_end..header_end + 8].try_into().expect("8");
    dup[header_end + 8..header_end + 16].copy_from_slice(&first);
    assert!(matches!(decode_deep_exr(&dup, &Limits::none()), Err(CodecError::Malformed { .. })));
    // An offset pointing past the file.
    let mut far = bytes.clone();
    far[header_end..header_end + 8].copy_from_slice(&u64::MAX.to_le_bytes());
    assert!(matches!(decode_deep_exr(&far, &Limits::none()), Err(CodecError::Malformed { .. })));
}

/// #4958: a deep tile's count table may be stored at the full tile size, and that size comes
/// from the header alone: nothing ties it to the data window. One 1024×1024 tile over a 4×4
/// image, its table stored at that size, makes a compressed file of tens of KB whose table
/// inflates to 4 MiB, plus as much again for the copy that undoes the byte split. Under a
/// budget smaller than both the table must be refused before it is allocated; with room for
/// both it decodes. Tables stored at the clipped size, and the full-size edge tables of
/// ordinary tiles, still decode under a small budget.
#[test]
fn deep_tile_count_table_is_admitted_against_max_alloc() {
    let budget = |max_alloc: u64| Limits { max_alloc, ..Limits::none() };
    let tight = budget(1 << 20);
    let table: u64 = 1024 * 1024 * 4; // the full tile's count table, in bytes
    let scan = decode_deep_exr(&gen_deep(4, 4, &RGBAZ, 0, None, COUNTS_ONE, &sample_rgba), &Limits::none()).expect("scan lines");
    for compression in [0u8, 1, 2] {
        let big = gen_deep_tables(4, 4, &RGBAZ, compression, Some((1024, 1024)), COUNTS_ONE, &sample_rgba, true);
        assert_eq!(decode_deep_exr(&big, &Limits::none()).expect("full-size table, no limits"), scan, "compression {compression}");
        let got = decode_deep_exr(&big, &tight);
        assert!(
            matches!(got, Err(CodecError::LimitExceeded(_))),
            "compression {compression}: a 4 MiB count table under a 1 MiB budget gave {:?}",
            got.map(|d| d.total_samples())
        );
        let flat = craft_image_codecs::decode_with(&big, &craft_image_codecs::DecodeOptions { limits: tight, ..Default::default() });
        assert!(matches!(flat, Err(CodecError::LimitExceeded(_))), "compression {compression}: the flat decode gave {:?}", flat.map(|i| i.width()));
        // The same tile with its table stored at the clipped size fits the small budget.
        let clipped = gen_deep(4, 4, &RGBAZ, compression, Some((1024, 1024)), COUNTS_ONE, &sample_rgba);
        assert_eq!(decode_deep_exr(&clipped, &tight).expect("clipped table, 1 MiB budget"), scan, "compression {compression}");
        if compression != 0 {
            assert!(big.len() < 1 << 20, "compression {compression}: the file is {} bytes", big.len());
        }
        // The table and its scratch copy are admitted together: 8 MiB.
        assert_eq!(decode_deep_exr(&big, &budget(2 * table)).expect("room for the table and its copy"), scan, "compression {compression}");
        let got = decode_deep_exr(&big, &budget(2 * table - 1));
        assert!(
            matches!(got, Err(CodecError::LimitExceeded(_))),
            "compression {compression}: one byte short gave {:?}",
            got.map(|d| d.total_samples())
        );
    }
    // Ordinary tiles: full-size edge tables decode under the small budget, like clipped ones.
    let counts: Vec<u32> = (0..35).map(|p| (p * 3) % 4).collect();
    let sample = |p: usize, ci: usize| -> Vec<f32> { (0..counts[p]).map(|k| [0.5, 0.25, 0.125 * k as f32, 1.0, 10.0 - k as f32][ci]).collect() };
    for compression in [0u8, 1, 2] {
        let full = gen_deep_tables(7, 5, &RGBAZ, compression, Some((3, 2)), &counts, &sample, true);
        let clipped = gen_deep(7, 5, &RGBAZ, compression, Some((3, 2)), &counts, &sample);
        assert_eq!(
            decode_deep_exr(&full, &tight).expect("full-size edge tables"),
            decode_deep_exr(&clipped, &tight).expect("clipped edge tables"),
            "compression {compression}"
        );
    }
}

/// #4958, the edges of that admission. A tile of the largest size exr accepts (2^30 - 2 a side,
/// a 2^62 byte count table) decodes from a clipped table under every budget, including no limits,
/// where its full size passes the check and is only tried as the data arrives. Each tile is
/// admitted on its own, because its table is freed before the next chunk: three 16×16 tiles over
/// a 48×1 image are all edge tiles, so every full-size table reaches the check (an interior tile's
/// full size equals its clipped size, so it uses the clipped check). They need 2 KiB each for
/// the table and its copy, 6 KiB in all, and decode under a budget of 4096 bytes. Half and uint
/// channels decode after full-size tables like float ones.
#[test]
fn deep_tile_table_admission_edges() {
    let budget = |max_alloc: u64| Limits { max_alloc, ..Limits::none() };
    let scan = decode_deep_exr(&gen_deep(4, 4, &RGBAZ, 0, None, COUNTS_ONE, &sample_rgba), &Limits::none()).expect("scan lines");
    let side = (1 << 30) - 2;
    for compression in [0u8, 1, 2] {
        let bytes = gen_deep(4, 4, &RGBAZ, compression, Some((side, side)), COUNTS_ONE, &sample_rgba);
        for limits in [Limits::none(), Limits::default(), budget(1 << 20)] {
            assert_eq!(decode_deep_exr(&bytes, &limits).expect("the largest tile, clipped table"), scan, "compression {compression}, {limits:?}");
        }
    }
    let counts: Vec<u32> = (0..48).map(|p| (p % 3) as u32).collect();
    let half: [Chan; 5] = RGBAZ.map(|c| Chan { ty: 1, ..c });
    let [a, b, g, r, z] = RGBAZ;
    let with_id = [a, b, g, r, z, Chan { name: "id", ty: 0 }];
    for channels in [&RGBAZ[..], &half[..], &with_id[..]] {
        let sample = |p: usize, ci: usize| -> Vec<f32> { (0..counts[p]).map(|k| [0.5, 0.25, 0.125, 1.0, 2.0 + k as f32, 7.0][ci]).collect() };
        for compression in [0u8, 1, 2] {
            let full = gen_deep_tables(48, 1, channels, compression, Some((16, 16)), &counts, &sample, true);
            let clipped = gen_deep(48, 1, channels, compression, Some((16, 16)), &counts, &sample);
            let what = format!("{} channels, compression {compression}", channels.len());
            let want = decode_deep_exr(&clipped, &Limits::none()).expect("clipped tables");
            assert_eq!(decode_deep_exr(&full, &budget(4096)).expect("full-size tables, admitted one at a time"), want, "{what}");
        }
    }
}

/// A one-channel data-window check admits only four bytes per pixel, but a compressed count
/// table and its scratch copy need eight. Cover scanlines, interior tiles and the clipped fallback;
/// the existing multichannel cases have enough pixel allowance to hide this gap.
#[test]
fn clipped_count_tables_include_scratch_in_the_budget() {
    let channels = [Chan { name: "Y", ty: 2 }];
    let counts = vec![0u32; 64];
    let sample = |_: usize, _: usize| Vec::new();
    let budget = |max_alloc: u64| Limits { max_alloc, ..Limits::none() };
    for tile in [None, Some((64, 1)), Some((128, 2))] {
        for compression in [0u8, 1, 2] {
            let bytes = gen_deep(64, 1, &channels, compression, tile, &counts, &sample);
            let what = format!("tile {tile:?}, compression {compression}");
            let want = decode_deep_exr(&bytes, &Limits::none()).expect("unlimited decode");
            let required = if compression == 0 { 256 } else { 512 };
            assert_eq!(decode_deep_exr(&bytes, &budget(required)).expect("table and any scratch fit exactly"), want, "{what}");
            assert!(matches!(decode_deep_exr(&bytes, &budget(required - 1)), Err(CodecError::LimitExceeded(_))), "{what}");
            let flat =
                craft_image_codecs::decode_with(&bytes, &craft_image_codecs::DecodeOptions { limits: budget(required - 1), ..Default::default() });
            assert!(matches!(flat, Err(CodecError::LimitExceeded(_))), "{what}");
            if compression == 0 {
                assert_eq!(decode_deep_exr(&bytes, &budget(511)).expect("raw table needs no scratch"), want, "{what}");
            }
        }
    }
}

/// RLE and ZIPS store incompressible tables raw too. Their actual payload, not the header's
/// compression tag, decides whether a scratch buffer is needed.
#[test]
fn raw_count_tables_need_no_scratch_even_with_compression() {
    let channels = [Chan { name: "Y", ty: 2 }];
    let limits = Limits { max_alloc: 4, ..Limits::none() };
    for tile in [None, Some((1, 1)), Some((2, 2))] {
        for compression in [0u8, 1, 2] {
            let bytes = gen_deep(1, 1, &channels, compression, tile, &[0], &|_, _| Vec::new());
            let header_end = header_end_of(&bytes);
            let chunk = u64::from_le_bytes(bytes[header_end..header_end + 8].try_into().expect("chunk offset")) as usize;
            let table_size = chunk + if tile.is_some() { 16 } else { 4 };
            let packed = u64::from_le_bytes(bytes[table_size..table_size + 8].try_into().expect("table size"));
            assert_eq!(packed, 4, "the four-byte count table is stored raw");
            let want = decode_deep_exr(&bytes, &Limits::none()).expect("unlimited decode");
            assert_eq!(decode_deep_exr(&bytes, &limits).expect("raw table fits"), want, "tile {tile:?}, compression {compression}");
        }
    }
}

/// Walk the attributes like the format prescribes: the offset table starts after the
/// header's terminating zero byte.
fn header_end_of(bytes: &[u8]) -> usize {
    let mut p = 8usize; // magic + version
    loop {
        let Some(nul) = bytes[p..].iter().position(|&b| b == 0) else { panic!("unterminated attribute name") };
        if nul == 0 {
            return p + 1; // the empty name terminates the header
        }
        p += nul + 1;
        let Some(t) = bytes[p..].iter().position(|&b| b == 0) else { panic!("unterminated attribute type") };
        p += t + 1;
        let size = i32::from_le_bytes(bytes[p..p + 4].try_into().expect("size")) as usize;
        p += 4 + size;
    }
}

#[test]
fn inconsistent_count_table_errors() {
    // The declared unpacked size disagrees with the count table.
    let mut bytes = gen_deep(4, 4, &RGBAZ, 0, None, COUNTS_ONE, &sample_rgba);
    // Find the first chunk: the offset table follows the header (find the "version" attr end).
    let header_end = header_end_of(&bytes);
    let first_chunk = u64::from_le_bytes(bytes[header_end..header_end + 8].try_into().expect("8")) as usize;
    // Scanline chunk: y (4), three u64s (24); bump the unpacked size by one.
    let at = first_chunk + 4 + 16;
    let v = u64::from_le_bytes(bytes[at..at + 8].try_into().expect("8"));
    bytes[at..at + 8].copy_from_slice(&(v + 1).to_le_bytes());
    assert!(matches!(decode_deep_exr(&bytes, &Limits::none()), Err(CodecError::Malformed { .. })));
}

/// A two-part file made from the single-part deep file `deep` and a 1×1 flat scan-line part,
/// the deep part first when `deep_first`: each header gains `name` and `chunkCount`, the
/// version field the multipart bit, and each chunk its part number. The offset tables follow
/// the headers in header order, then the chunks. Returns the file, where the first table
/// starts, where the second table starts and where the last table ends.
fn two_part(deep: &[u8], deep_first: bool) -> (Vec<u8>, usize, usize, usize) {
    let end = header_end_of(deep);
    let first = u64::from_le_bytes(deep[end..end + 8].try_into().expect("8")) as usize;
    let offsets: Vec<usize> =
        (0..(first - end) / 8).map(|i| u64::from_le_bytes(deep[end + 8 * i..end + 8 * i + 8].try_into().expect("8")) as usize).collect();
    let deep_chunks: Vec<&[u8]> = offsets.iter().enumerate().map(|(i, &o)| &deep[o..offsets.get(i + 1).copied().unwrap_or(deep.len())]).collect();

    let mut deep_header = deep[8..end - 1].to_vec();
    attr(&mut deep_header, "name", "string", b"deep");
    attr(&mut deep_header, "chunkCount", "int", &(deep_chunks.len() as i32).to_le_bytes());
    let mut flat_header = Vec::new();
    let mut chlist = b"R\0".to_vec();
    chlist.extend_from_slice(&1i32.to_le_bytes()); // half
    chlist.extend_from_slice(&[0u8; 4]); // pLinear, reserved
    chlist.extend_from_slice(&1i32.to_le_bytes());
    chlist.extend_from_slice(&1i32.to_le_bytes());
    chlist.push(0);
    attr(&mut flat_header, "channels", "chlist", &chlist);
    attr(&mut flat_header, "compression", "compression", &[0]);
    let dw: Vec<u8> = [0i32; 4].iter().flat_map(|v| v.to_le_bytes()).collect();
    attr(&mut flat_header, "dataWindow", "box2i", &dw);
    attr(&mut flat_header, "displayWindow", "box2i", &dw);
    attr(&mut flat_header, "lineOrder", "lineOrder", &[0]);
    attr(&mut flat_header, "pixelAspectRatio", "float", &1f32.to_le_bytes());
    attr(&mut flat_header, "screenWindowCenter", "v2f", &[0f32.to_le_bytes(), 0f32.to_le_bytes()].concat());
    attr(&mut flat_header, "screenWindowWidth", "float", &1f32.to_le_bytes());
    attr(&mut flat_header, "name", "string", b"flat");
    attr(&mut flat_header, "type", "string", b"scanlineimage");
    attr(&mut flat_header, "chunkCount", "int", &1i32.to_le_bytes());
    // The flat part's one chunk: y, the data size, one half sample.
    let mut flat_chunk = 0i32.to_le_bytes().to_vec();
    flat_chunk.extend_from_slice(&2i32.to_le_bytes());
    flat_chunk.extend_from_slice(&f16::from_f32(0.5).to_le_bytes());
    let flat_chunks: Vec<&[u8]> = vec![&flat_chunk];

    let parts: [(&[u8], &[&[u8]]); 2] = if deep_first {
        [(&deep_header, &deep_chunks), (&flat_header, &flat_chunks)]
    } else {
        [(&flat_header, &flat_chunks), (&deep_header, &deep_chunks)]
    };
    let mut out = deep[..4].to_vec();
    out.extend_from_slice(&(2u32 | 0x800 | 0x1000).to_le_bytes());
    for (header, _) in &parts {
        out.extend_from_slice(header);
        out.push(0); // end of this header
    }
    out.push(0); // end of the headers
    let first_table = out.len();
    let second_table = first_table + parts[0].1.len() * 8;
    let tables_end = second_table + parts[1].1.len() * 8;
    let mut pos = tables_end;
    for (_, chunks) in &parts {
        for c in chunks.iter() {
            out.extend_from_slice(&(pos as u64).to_le_bytes());
            pos += 4 + c.len();
        }
    }
    for (part, (_, chunks)) in parts.iter().enumerate() {
        for c in chunks.iter() {
            out.extend_from_slice(&(part as i32).to_le_bytes());
            out.extend_from_slice(c);
        }
    }
    (out, first_table, second_table, tables_end)
}

/// The deep part of a two-part file decodes as the single-part file does, first or second.
/// Its chunks start after every part's offset table: a chunk offset into the other part's
/// table, or one byte before the end of the last table, is refused as table data, and a file
/// that ends inside the last table is refused even when the deep part's own table is whole.
#[test]
fn multipart_deep_chunks_start_after_every_offset_table() {
    let single = gen_deep(4, 4, &RGBAZ, 0, None, COUNTS_ONE, &sample_rgba);
    let want = decode_deep_exr(&single, &Limits::none()).expect("single part");
    for deep_first in [true, false] {
        let (bytes, ..) = two_part(&single, deep_first);
        let deep = decode_deep_exr(&bytes, &Limits::none()).expect("two parts");
        assert_eq!(deep.counts, want.counts);
        assert_eq!(deep.channel("Z").expect("Z").samples, want.channel("Z").expect("Z").samples);
    }

    let refused = |bytes: &[u8], what: &str| match decode_deep_exr(bytes, &Limits::none()) {
        Err(CodecError::Malformed { message, .. }) => assert!(message.contains(what), "{message}"),
        Err(e) => panic!("{e}, want an error about {what}"),
        Ok(_) => panic!("decoded, want an error about {what}"),
    };
    let (bytes, first_table, second_table, tables_end) = two_part(&single, true);
    for off in [second_table, tables_end - 1] {
        let mut into_table = bytes.clone();
        into_table[first_table..first_table + 8].copy_from_slice(&(off as u64).to_le_bytes());
        refused(&into_table, "outside the chunk area");
    }
    refused(&bytes[..tables_end - 4], "past the end of the file");
}

// ---------------------------------------------------------------------------------------------
// Real reference files: the deep test images of the OpenEXR repository at commit
// `69b2604fc76e370615438bdc8d2cd95b9349c12e`, read from `corpus/exr/` at the repo root with the
// cargo feature `corpus`. `REAL_FILE_SHA256` pins each file's SHA-256, and the test refuses a
// file that is missing, is not a regular file, is larger than `MAX_REAL_FILE_BYTES` or does not
// match its digest. The crate README says how to fill the folder. Their `.txt` sidecars upstream
// list the ID manifest of each file, an independent oracle for the u32 ID channels.

/// The pinned files as name and lowercase hex SHA-256, the same five names as `REAL_FILES`.
const REAL_FILE_SHA256: &[(&str, &str)] = &[
    ("11.deep.exr", "800ec94ffb2085da9a730dcda2f9611f5fb62f922a3f2aef77287e36106b9667"),
    ("42.deep.exr", "439907f3448480894264f0c8847f6c572f645441db34a5ad9f36f3f020d101b5"),
    ("64.deep.exr", "4d9bfa646323b80630933e114a3112095488e8c008cdf908e210979638e65fe4"),
    ("multivariate.deep.exr", "2335ed6052258fe693cb795f2bea57f22ba0564281691c82f2987d31d96e9e74"),
    ("objectid.deep.exr", "af8940912bfa856b2650d2005462fd348656385161ccf0745b91b143e934c5f6"),
];

/// The largest file read from `corpus/exr/`. The largest pinned file is 611,840 bytes.
#[cfg(feature = "corpus")]
const MAX_REAL_FILE_BYTES: u64 = 4 * 1024 * 1024;

/// The lowercase hex SHA-256 of `bytes`.
fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// Reads a regular file of at most `limit` bytes. A symbolic link, a folder or a larger file is an
/// error, and a file that grows during the read is cut off at `limit + 1` bytes and refused.
fn read_bounded(path: &std::path::Path, limit: u64) -> Result<Vec<u8>, String> {
    let meta = std::fs::symlink_metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if !meta.is_file() || meta.len() > limit {
        return Err(format!("{}: expected a regular file of at most {limit} bytes", path.display()));
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path).and_then(|f| f.take(limit + 1).read_to_end(&mut bytes)).map_err(|e| format!("{}: {e}", path.display()))?;
    if bytes.len() as u64 > limit {
        return Err(format!("{}: exceeds {limit} bytes", path.display()));
    }
    Ok(bytes)
}

/// Reads `dir/name` within `limit` bytes and checks its SHA-256 against `pinned`.
fn read_pinned(dir: &std::path::Path, name: &str, pinned: &str, limit: u64) -> Result<Vec<u8>, String> {
    let bytes = read_bounded(&dir.join(name), limit)?;
    let got = sha256_hex(&bytes);
    if got != pinned {
        return Err(format!("{name}: SHA-256 {got} does not match the pinned {pinned}"));
    }
    Ok(bytes)
}

/// One pinned file: name, total samples, most samples in one pixel, and per ID channel the
/// manifest IDs its sidecar lists.
#[cfg(feature = "corpus")]
type RealFile = (&'static str, u64, u64, &'static [(&'static str, &'static [u32])]);

#[cfg(feature = "corpus")]
const MULTIVARIATE_IDS: &[u32] = &[
    127035619, 147085119, 319332210, 409096490, 523461226, 678492822, 683859976, 1058109025, 1193275200, 1503246798, 1552124883, 1613501244,
    1672208113, 1687665471, 1741895317, 1841370478, 1863854195, 1865183722, 2056745377, 2202049807, 2303547305, 2369798380, 2433329986, 2439705590,
    2505186565, 2698909330, 2862333315, 2898564507, 2979371527, 3007860936, 3182339638, 3236172513, 3497437340, 3626859046, 3656820528, 3693645783,
    3739501011, 3936563811, 4048943790,
];

#[cfg(feature = "corpus")]
const REAL_FILES: &[RealFile] = &[
    (
        "11.deep.exr",
        6056,
        2,
        &[(
            "id",
            &[523461226, 683859976, 1552124883, 2202049807, 2303547305, 2433329986, 2698909330, 3182339638, 3236172513, 3626859046, 3739501011],
        )],
    ),
    (
        "42.deep.exr",
        67605,
        8,
        &[
            ("modelid", &[147148503, 1252544961, 1917926828, 3170049167, 3338960663, 3411631724]),
            ("materialid", &[489905694, 507819813, 2340321182, 3683581971, 3905155331, 3947898621, 4179825792]),
        ],
    ),
    // 64-bit IDs split over `.id0`/`.id1`: only the sample counts are checked.
    ("64.deep.exr", 64543, 6, &[]),
    ("multivariate.deep.exr", 64543, 6, &[("id", MULTIVARIATE_IDS)]),
    // The same samples as multivariate; its sidecar has no manifest.
    ("objectid.deep.exr", 64543, 6, &[]),
];

#[cfg(feature = "corpus")]
#[test]
fn feature_config_real_deep_files_decode() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/exr");
    let decoded: Vec<&str> = REAL_FILES.iter().map(|f| f.0).collect();
    let pinned: Vec<&str> = REAL_FILE_SHA256.iter().map(|p| p.0).collect();
    assert_eq!(decoded, pinned, "REAL_FILES and REAL_FILE_SHA256 must list the same files");
    for (&(name, total, max_per_pixel, ids), &(_, sha256)) in REAL_FILES.iter().zip(REAL_FILE_SHA256) {
        let bytes = read_pinned(&dir, name, sha256, MAX_REAL_FILE_BYTES)
            .unwrap_or_else(|e| panic!("{}: {e}; place the pinned OpenEXR deep test images in this folder (see the crate README)", dir.display()));
        let deep = decode_deep_exr(&bytes, &Limits::none()).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!((deep.width, deep.height), (256, 256), "{name}");
        assert_eq!(deep.total_samples(), total, "{name}");
        assert_eq!(deep.counts.len(), 256 * 256 + 1, "{name}");
        assert!(deep.counts.windows(2).all(|w| w[0] <= w[1]), "{name}: counts must be cumulative");
        assert_eq!(deep.counts.windows(2).map(|w| w[1] - w[0]).max(), Some(max_per_pixel), "{name}");
        for c in &deep.channels {
            assert_eq!(c.samples.len() as u64, total, "{name}: channel {}", c.name);
        }
        // Every decoded ID is one the sidecar lists, and every listed ID occurs. Samples hold
        // u32 IDs as f32 (exact only to 2^24), so compare at f32 precision.
        for &(channel, manifest) in ids {
            let c = deep.channel(channel).unwrap_or_else(|| panic!("{name}: no channel {channel}"));
            let want: std::collections::BTreeSet<u32> = manifest.iter().map(|&id| (id as f32).to_bits()).collect();
            let got: std::collections::BTreeSet<u32> = c.samples.iter().map(|v| v.to_bits()).collect();
            assert_eq!(got, want, "{name}: channel {channel}");
        }
        // The flat decode composites the same samples.
        let img = decode(&bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!((img.width(), img.height()), (deep.width, deep.height), "{name}");
        assert_eq!(img.to_f32_samples().expect("f32").len(), 256 * 256 * 4, "{name}");
        assert!(img.warnings.iter().any(|w| matches!(w, craft_image_codecs::DecodeWarning::DeepFlattened { .. })), "{name}");
    }
}

#[test]
fn real_file_pins_are_well_formed() {
    let mut names = std::collections::BTreeSet::new();
    for &(name, sha256) in REAL_FILE_SHA256 {
        assert!(name.ends_with(".deep.exr") && !name.contains(['/', '\\']), "{name}");
        assert!(names.insert(name), "{name} is pinned twice");
        assert_eq!(sha256.len(), 64, "{name}");
        assert!(sha256.bytes().all(|c| matches!(c, b'0'..=b'9' | b'a'..=b'f')), "{name}: {sha256}");
    }
    assert_eq!(names.len(), 5);
}

/// A folder under the system temp directory, removed when dropped.
struct ScratchDir(std::path::PathBuf);

impl ScratchDir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("craft-image-codecs-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        Self(dir)
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn pinned_read_refuses_a_changed_byte() {
    let dir = ScratchDir::new("deep-exr-pin");
    // SHA-256 of "abc" (FIPS 180-2, appendix B.1).
    let abc = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
    std::fs::write(dir.0.join("a.deep.exr"), b"abc").expect("write");
    assert_eq!(read_pinned(&dir.0, "a.deep.exr", abc, 16).as_deref(), Ok(&b"abc"[..]));

    std::fs::write(dir.0.join("a.deep.exr"), b"abd").expect("write");
    let err = read_pinned(&dir.0, "a.deep.exr", abc, 16).expect_err("a changed byte must be refused");
    assert!(err.starts_with("a.deep.exr: ") && err.contains(abc) && err.contains(&sha256_hex(b"abd")), "{err}");
}

#[test]
fn pinned_read_refuses_missing_oversize_and_folders() {
    let dir = ScratchDir::new("deep-exr-bound");
    let pin = sha256_hex(b"12345678");
    std::fs::write(dir.0.join("eight.deep.exr"), b"12345678").expect("write");
    assert!(read_pinned(&dir.0, "eight.deep.exr", &pin, 8).is_ok());
    let err = read_pinned(&dir.0, "eight.deep.exr", &pin, 7).expect_err("a file over the limit must be refused");
    assert!(err.contains("at most 7 bytes"), "{err}");

    let err = read_pinned(&dir.0, "missing.deep.exr", &pin, 8).expect_err("a missing file must be refused");
    assert!(err.contains("missing.deep.exr"), "{err}");

    std::fs::create_dir(dir.0.join("folder.deep.exr")).expect("mkdir");
    let err = read_pinned(&dir.0, "folder.deep.exr", &pin, 8).expect_err("a folder must be refused");
    assert!(err.contains("regular file"), "{err}");
}

/// `gen_deep` for a cropped render: the data window sits at `(dwx, dwy)` inside a larger
/// display window of `sw × sh` (the stage the renderer composed onto). Scanline chunk Y
/// coordinates are absolute display rows, so they carry the data-window offset.
#[allow(clippy::too_many_arguments)]
fn gen_deep_cropped(
    w: usize,
    h: usize,
    dwx: i32,
    dwy: i32,
    sw: usize,
    sh: usize,
    channels: &[Chan],
    counts: &[u32],
    sample: &dyn Fn(usize, usize) -> Vec<f32>,
) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&0x0131_2f76u32.to_le_bytes());
    out.extend_from_slice(&(2u32 | 0x800).to_le_bytes());

    let mut chlist = Vec::new();
    for c in channels {
        chlist.extend_from_slice(c.name.as_bytes());
        chlist.push(0);
        chlist.extend_from_slice(&c.ty.to_le_bytes());
        chlist.push(0);
        chlist.extend_from_slice(&[0u8; 3]);
        chlist.extend_from_slice(&1i32.to_le_bytes());
        chlist.extend_from_slice(&1i32.to_le_bytes());
    }
    chlist.push(0);
    attr(&mut out, "channels", "chlist", &chlist);
    attr(&mut out, "compression", "compression", &[0u8]);
    // box2i is inclusive: the windows end at position + size - 1.
    let dw: Vec<u8> = [dwx, dwy, dwx + w as i32 - 1, dwy + h as i32 - 1].iter().flat_map(|v| v.to_le_bytes()).collect();
    attr(&mut out, "dataWindow", "box2i", &dw);
    let disp: Vec<u8> = [0i32, 0, sw as i32 - 1, sh as i32 - 1].iter().flat_map(|v| v.to_le_bytes()).collect();
    attr(&mut out, "displayWindow", "box2i", &disp);
    attr(&mut out, "lineOrder", "lineOrder", &[0]);
    attr(&mut out, "pixelAspectRatio", "float", &1f32.to_le_bytes());
    attr(&mut out, "screenWindowCenter", "v2f", &[0f32.to_le_bytes(), 0f32.to_le_bytes()].concat());
    attr(&mut out, "screenWindowWidth", "float", &1f32.to_le_bytes());
    attr(&mut out, "type", "string", b"deepscanline");
    attr(&mut out, "version", "int", &1i32.to_le_bytes());
    out.push(0);

    // One scanline block per data row; the pixel indexing stays data-relative.
    let mut chunks: Vec<Vec<u8>> = Vec::new();
    for y in 0..h {
        let pixels: Vec<usize> = (y * w..y * w + w).collect();
        let mut c = Vec::new();
        c.extend_from_slice(&(dwy + y as i32).to_le_bytes());
        let mut table = Vec::with_capacity(pixels.len() * 4);
        let mut acc = 0u32;
        for &p in &pixels {
            acc += counts[p];
            table.extend_from_slice(&acc.to_le_bytes());
        }
        let mut data = Vec::new();
        for (ci, _) in channels.iter().enumerate() {
            for &p in &pixels {
                for v in sample(p, ci) {
                    data.extend_from_slice(&v.to_le_bytes());
                }
            }
        }
        c.extend_from_slice(&(table.len() as u64).to_le_bytes());
        c.extend_from_slice(&(data.len() as u64).to_le_bytes());
        c.extend_from_slice(&(data.len() as u64).to_le_bytes());
        c.extend_from_slice(&table);
        c.extend_from_slice(&data);
        chunks.push(c);
    }
    let mut pos = out.len() + chunks.len() * 8;
    let mut table = Vec::new();
    for c in &chunks {
        table.extend_from_slice(&(pos as u64).to_le_bytes());
        pos += c.len();
    }
    out.extend_from_slice(&table);
    for c in chunks {
        out.extend_from_slice(&c);
    }
    out
}

/// A cropped render opens on the full display-window stage: MY27-style renders carry a
/// `display_window` of the full frame (e.g. 4000×3000) while the deep data window is the
/// render pass's crop inside it. The image must open at stage size with the data at its
/// crop offset, transparent elsewhere — not cropped to the data window.
#[test]
fn cropped_deep_data_opens_on_the_display_stage() {
    let (w, h, dwx, dwy, sw, sh) = (24usize, 16usize, 8i32, 6i32, 48usize, 32usize);
    let counts = vec![1u32; w * h];
    let sample = |p: usize, ci: usize| -> Vec<f32> {
        // chlist order (RGBA_NO_Z): A, B, G, R.
        let x = p % w;
        let y = p / w;
        match ci {
            0 => vec![1.0],                 // A
            1 => vec![0.5],                 // B
            2 => vec![y as f32 / h as f32], // G
            _ => vec![x as f32 / w as f32], // R
        }
    };
    let bytes = gen_deep_cropped(w, h, dwx, dwy, sw, sh, &RGBA_NO_Z, &counts, &sample);
    let img = decode(&bytes).expect("cropped deep EXR decodes");
    assert_eq!((img.width(), img.height()), (sw as u32, sh as u32), "opens on the display window stage");
    assert!(img.warnings.iter().any(|w| matches!(w, craft_image_codecs::DecodeWarning::DisplayWindowStage { origin, .. } if *origin == (dwx, dwy))));

    // The data sits at its crop offset: the first data pixel's green channel carries its
    // data-row fraction, everything outside the data region stays transparent black.
    let s = img.to_f32_samples().expect("f32");
    let nc = s.len() / (sw * sh);
    let px = |x: usize, y: usize| -> Vec<f32> { s[y * sw * nc + x * nc..y * sw * nc + x * nc + nc].to_vec() };
    let first = px(dwx as usize, dwy as usize);
    assert!(
        (first[0] - 0.0).abs() < 1e-5 && (first[1] - 0.0).abs() < 1e-5 && (first[2] - 0.5).abs() < 1e-5 && (first[3] - 1.0).abs() < 1e-5,
        "first data pixel at the offset: {first:?}"
    );
    let last = px(dwx as usize + w - 1, dwy as usize + h - 1);
    assert!(
        (last[0] - (w - 1) as f32 / w as f32).abs() < 1e-5 && (last[1] - (h - 1) as f32 / h as f32).abs() < 1e-5,
        "last data pixel keeps its coordinates: {last:?}"
    );
    let outside = px(0, 0);
    assert_eq!(outside, vec![0.0, 0.0, 0.0, 0.0], "outside the data region: transparent");
}
