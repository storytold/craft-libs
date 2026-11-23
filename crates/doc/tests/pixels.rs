//! `pcraft_layer_rgba` on `.pcraft` bundles assembled here: manifests written by hand and tiles
//! compressed with `ruzstd`, each stored under the BLAKE3 hash of its samples.
#![cfg(feature = "pcraft-pixels")]

use super::common;

use common::{store, zip};
use craft_doc::{LayerPixels, Limits, Samples, pcraft_layer_rgba};

const TILE: usize = 256;

/// The hash a tile is stored under.
fn hash(tile: &[u8]) -> String {
    blake3::hash(tile).to_hex().to_string()
}

fn zstd(bytes: &[u8]) -> Vec<u8> {
    ruzstd::encoding::compress_to_vec(bytes, ruzstd::encoding::CompressionLevel::Fastest)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// A surface's JSON: `mode`, `sample` and `alpha`, the default pixel's bytes and `(tx, ty, hash)`
/// tiles.
fn surface(mode: &str, sample: &str, alpha: bool, default: &[u8], tiles: &[(i32, i32, &str)]) -> String {
    let tiles: Vec<String> = tiles.iter().map(|(tx, ty, h)| format!(r#"{{"tx":{tx},"ty":{ty},"hash":"{h}"}}"#)).collect();
    format!(r#"{{"format":{{"mode":"{mode}","sample":"{sample}","alpha":{alpha}}},"default":"{}","tiles":[{}]}}"#, hex(default), tiles.join(","))
}

/// A layer's JSON with `content` and an optional mask surface.
fn layer(content: &str, mask: Option<&str>) -> String {
    let mask = match mask {
        Some(s) => format!(r#"{{"surface":{s},"enabled":false,"linked":true,"density":0.5,"feather":2.0}}"#),
        None => "null".into(),
    };
    format!(
        r#"{{"id":1,"name":"L","visible":true,"locks":{{}},"blend":"Normal","opacity":1.0,"fill_opacity":1.0,"clipped":false,"mask":{mask},
            "effects":{{"enabled":true,"items":[],"psd_raw":null}},"label":"None","content":{content},"psd_blocks":[],"psd_id":null,"fill_cache":null}}"#
    )
}

fn raster(surface: &str) -> String {
    format!(r#"{{"kind":"raster","surface":{surface}}}"#)
}

/// A bundle of a `width` × `height` canvas in `mode` with `layers`, and `(entry hash, uncompressed
/// tile)` entries.
fn bundle_with(width: u32, height: u32, mode: &str, layers: &[String], tiles: &[(String, Vec<u8>)]) -> Vec<u8> {
    let manifest = format!(
        r#"{{"format_version":1,"generator":"photocraft-format 0.1.0",
            "document":{{"id":1,"name":"Untitled","size":{{"width":{width},"height":{height}}},"resolution_dpi":72.0,"mode":"{mode}","depth":"U8",
              "icc_profile":null,"layers":[{}],"channels":[],"guides":{{}},"selection":null,"metadata":{{}},"global_light":{{}}}},
            "thumbnail":null,"composite":null}}"#,
        layers.join(",")
    );
    let names: Vec<String> = tiles.iter().map(|(h, _)| format!("tiles/{h}.zst")).collect();
    let frames: Vec<Vec<u8>> = tiles.iter().map(|(_, t)| zstd(t)).collect();
    let mut entries: Vec<common::Entry> = names.iter().zip(&frames).map(|(n, f)| store(n, f)).collect();
    entries.push(store("manifest.json", manifest.as_bytes()));
    zip(&entries)
}

/// [`bundle_with`], each tile stored under its own hash.
fn bundle(width: u32, height: u32, mode: &str, layers: &[String], tiles: &[&[u8]]) -> Vec<u8> {
    let tiles: Vec<(String, Vec<u8>)> = tiles.iter().map(|t| (hash(t), t.to_vec())).collect();
    bundle_with(width, height, mode, layers, &tiles)
}

/// A tile whose every pixel is `pixel`'s bytes.
fn solid(pixel: &[u8]) -> Vec<u8> {
    pixel.repeat(TILE * TILE)
}

fn read(bytes: &[u8], path: &[usize]) -> LayerPixels {
    pcraft_layer_rgba(bytes, path, &Limits::default()).unwrap().unwrap()
}

fn u8s(s: &Samples) -> &[u8] {
    match s {
        Samples::U8(v) => v,
        _ => panic!("not 8-bit: {s:?}"),
    }
}

fn rgba_at(p: &LayerPixels, x: usize, y: usize) -> &[u8] {
    let i = (y * p.width as usize + x) * 4;
    &u8s(&p.rgba)[i..i + 4]
}

#[test]
fn eight_bit_tiles_land_on_the_canvas_and_missing_tiles_hold_the_default_pixel() {
    // Tile (0, 0) is a gradient, tile (1, 1) is solid; (1, 0) and (0, 1) are missing.
    let mut gradient = vec![];
    for y in 0..TILE {
        for x in 0..TILE {
            gradient.extend([x as u8, y as u8, 7, 255]);
        }
    }
    let fill = solid(&[200, 100, 50, 128]);
    let s = surface("Rgb", "U8", true, &[10, 20, 30, 40], &[(0, 0, &hash(&gradient)), (1, 1, &hash(&fill))]);
    let file = bundle(300, 260, "Rgb", &[layer(&raster(&s), None)], &[&gradient, &fill]);
    let p = read(&file, &[0]);
    assert_eq!((p.width, p.height, p.rgba.len()), (300, 260, 300 * 260 * 4));
    assert_eq!(rgba_at(&p, 0, 0), [0, 0, 7, 255]);
    assert_eq!(rgba_at(&p, 255, 3), [255, 3, 7, 255]);
    assert_eq!(rgba_at(&p, 256, 0), [10, 20, 30, 40]);
    assert_eq!(rgba_at(&p, 0, 256), [10, 20, 30, 40]);
    assert_eq!(rgba_at(&p, 299, 259), [200, 100, 50, 128]);
    assert_eq!(rgba_at(&p, 256, 256), [200, 100, 50, 128]);
    assert_eq!(p.mask, None);
}

#[test]
fn every_truncation_and_flipped_byte_is_an_error_not_a_panic() {
    let tile = solid(&[1, 2, 3, 4]);
    let mask_tile = vec![9u8; TILE * TILE];
    let s = surface("Rgb", "U8", true, &[0; 4], &[(0, 0, &hash(&tile))]);
    let m = surface("Grayscale", "U8", false, &[255], &[(0, 0, &hash(&mask_tile))]);
    let file = bundle(3, 2, "Rgb", &[layer(&raster(&s), Some(&m))], &[&tile, &mask_tile]);
    assert!(read(&file, &[0]).mask.is_some());
    for n in 0..file.len() {
        let _ = pcraft_layer_rgba(&file[..n], &[0], &Limits::default());
        let mut flipped = file.clone();
        flipped[n] ^= 0xff;
        let _ = pcraft_layer_rgba(&flipped, &[0], &Limits::default());
    }
}
