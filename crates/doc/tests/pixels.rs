//! `pcraft_layer_rgba` on `.pcraft` bundles assembled here: manifests written by hand and tiles
//! compressed with `ruzstd`, each stored under the BLAKE3 hash of its samples.
#![cfg(feature = "pcraft-pixels")]

mod common;

use common::{store, zip};
use craft_doc::{Error, LayerPixels, Limits, Samples, pcraft_layer_rgba};

const TILE: usize = 256;

/// The hash a tile is stored under.
fn hash(tile: &[u8]) -> String {
    blake3::hash(tile).to_hex().to_string()
}

/// A valid hash that names no tile in the bundle.
fn absent(n: u32) -> String {
    format!("{n:064x}")
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

fn malformed(file: &[u8]) -> String {
    match pcraft_layer_rgba(file, &[0], &Limits::default()) {
        Err(Error::Malformed(m)) => m,
        other => panic!("not malformed: {other:?}"),
    }
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
fn sixteen_bit_tiles_without_alpha_are_opaque() {
    let pixel: Vec<u8> = [1000u16, 30000, 65000].iter().flat_map(|v| v.to_le_bytes()).collect();
    let default: Vec<u8> = [1u16, 2, 3].iter().flat_map(|v| v.to_le_bytes()).collect();
    let tile = solid(&pixel);
    let s = surface("Rgb", "U16", false, &default, &[(0, 0, &hash(&tile))]);
    let file = bundle(257, 2, "Rgb", &[layer(&raster(&s), None)], &[&tile]);
    let p = read(&file, &[0]);
    let Samples::U16(v) = &p.rgba else { panic!("not 16-bit: {:?}", p.rgba) };
    assert_eq!(v.len(), 257 * 2 * 4);
    assert_eq!(&v[..4], [1000, 30000, 65000, 65535]);
    assert_eq!(&v[256 * 4..257 * 4], [1, 2, 3, 65535]);
}

#[test]
fn thirty_two_bit_grayscale_tiles_repeat_the_gray_value() {
    let pixel: Vec<u8> = [0.25f32, 0.5].iter().flat_map(|v| v.to_le_bytes()).collect();
    let default: Vec<u8> = [0.0f32, 0.0].iter().flat_map(|v| v.to_le_bytes()).collect();
    let tile = solid(&pixel);
    let s = surface("Grayscale", "F32", true, &default, &[(0, 0, &hash(&tile))]);
    let file = bundle(1, 1, "Grayscale", &[layer(&raster(&s), None)], &[&tile]);
    assert_eq!(read(&file, &[0]).rgba, Samples::F32(vec![0.25, 0.25, 0.25, 0.5]));
}

#[test]
fn tiles_at_negative_coordinates_are_outside_the_canvas_and_not_read() {
    // The tiles at negative columns and rows are not in the bundle; only (0, 0) is.
    let tile = solid(&[9, 8, 7, 6]);
    let s = surface("Rgb", "U8", true, &[0; 4], &[(-1, -1, &absent(1)), (-1, 0, &absent(2)), (0, -3, &absent(3)), (0, 0, &hash(&tile))]);
    let file = bundle(4, 4, "Rgb", &[layer(&raster(&s), None)], &[&tile]);
    assert_eq!(read(&file, &[0]).rgba, Samples::U8([9, 8, 7, 6].repeat(16)));
    // A tile right of or below the canvas is not read either.
    let s = surface("Rgb", "U8", true, &[1, 2, 3, 4], &[(1, 0, &absent(1)), (0, 1, &absent(2))]);
    let file = bundle(4, 4, "Rgb", &[layer(&raster(&s), None)], &[]);
    assert_eq!(read(&file, &[0]).rgba, Samples::U8([1, 2, 3, 4].repeat(16)));
}

#[test]
fn tile_coordinates_past_the_grid_and_repeated_tiles_are_malformed() {
    let (a, b) = (absent(1), absent(2));
    for tiles in [&[(i32::MAX, 0, a.as_str())][..], &[(0, i32::MIN, &a)], &[(-1, 2, &a), (-1, 2, &b)]] {
        let s = surface("Rgb", "U8", true, &[0; 4], tiles);
        malformed(&bundle(4, 4, "Rgb", &[layer(&raster(&s), None)], &[]));
    }
}

#[test]
fn a_layer_mask_comes_back_as_its_own_plane() {
    let mut mask_tile = vec![255u8; TILE * TILE];
    mask_tile[1] = 0;
    let s = surface("Rgb", "U8", true, &[1, 2, 3, 4], &[]);
    let m = surface("Grayscale", "U8", false, &[255], &[(0, 0, &hash(&mask_tile))]);
    let file = bundle(3, 1, "Rgb", &[layer(&raster(&s), Some(&m))], &[&mask_tile]);
    let mask = read(&file, &[0]).mask.unwrap();
    assert_eq!(mask.alpha, Samples::U8(vec![255, 0, 255]));
    assert!(!mask.enabled);
    assert_eq!((mask.density, mask.feather), (0.5, 2.0));
    // A mask stored as RGB gives its first channel.
    let m = surface("Rgb", "U8", false, &[7, 8, 9], &[]);
    assert_eq!(read(&bundle(3, 1, "Rgb", &[layer(&raster(&s), Some(&m))], &[]), &[0]).mask.unwrap().alpha, Samples::U8(vec![7; 3]));
    // A mask stored as CMYK is unsupported.
    let m = surface("Cmyk", "U8", false, &[0; 4], &[]);
    let file = bundle(3, 1, "Rgb", &[layer(&raster(&s), Some(&m))], &[]);
    assert!(matches!(pcraft_layer_rgba(&file, &[0], &Limits::default()), Err(Error::Unsupported(m)) if m.contains("Cmyk")));
}

#[test]
fn bitmap_and_indexed_documents_are_read_from_their_expanded_layers() {
    let gray = surface("Grayscale", "U8", false, &[9], &[]);
    assert_eq!(read(&bundle(1, 1, "Bitmap", &[layer(&raster(&gray), None)], &[]), &[0]).rgba, Samples::U8(vec![9, 9, 9, 255]));
    let rgb = surface("Rgb", "U8", false, &[1, 2, 3], &[]);
    assert_eq!(read(&bundle(1, 1, "Indexed", &[layer(&raster(&rgb), None)], &[]), &[0]).rgba, Samples::U8(vec![1, 2, 3, 255]));
}

#[test]
fn an_empty_canvas_is_malformed() {
    let s = surface("Rgb", "U8", true, &[0; 4], &[]);
    for (w, h) in [(0, 4), (4, 0)] {
        assert!(malformed(&bundle(w, h, "Rgb", &[layer(&raster(&s), None)], &[])).contains("empty"));
    }
}

#[test]
fn a_tile_that_several_references_name_is_decoded_once() {
    // 64 references to one grayscale tile of 64 KiB: 4 MiB if each were decoded.
    let tile: Vec<u8> = (0..TILE * TILE).map(|i| i as u8).collect();
    let h = hash(&tile);
    let refs: Vec<(i32, i32, &str)> = (0..64).map(|i| (i, 0, h.as_str())).collect();
    let s = surface("Grayscale", "U8", false, &[0], &refs);
    let file = bundle(64 * TILE as u32, 1, "Grayscale", &[layer(&raster(&s), None)], &[&tile]);
    let mut limits = Limits::default();
    limits.max_total = 160 << 10;
    let p = pcraft_layer_rgba(&file, &[0], &limits).unwrap().unwrap();
    let px = u8s(&p.rgba);
    assert_eq!(px.len(), 64 * TILE * 4);
    assert!((0..64).all(|t| px[(t * TILE + 7) * 4..(t * TILE + 8) * 4] == [7, 7, 7, 255]));
}

#[test]
fn a_tile_referenced_at_two_pixel_sizes_is_malformed() {
    // The layer reads the tile as RGBA (256 KiB), the mask as gray (64 KiB).
    let tile = solid(&[1, 2, 3, 4]);
    let h = hash(&tile);
    let s = surface("Rgb", "U8", true, &[0; 4], &[(0, 0, &h)]);
    let m = surface("Grayscale", "U8", false, &[255], &[(0, 0, &h)]);
    assert!(malformed(&bundle(2, 2, "Rgb", &[layer(&raster(&s), Some(&m))], &[&tile])).contains("different sizes"));
}

#[test]
fn a_cmyk_document_is_unsupported() {
    let s = surface("Cmyk", "U8", true, &[0; 5], &[]);
    let file = bundle(4, 4, "Cmyk", &[layer(&raster(&s), None)], &[]);
    assert!(matches!(pcraft_layer_rgba(&file, &[0], &Limits::default()), Err(Error::Unsupported(m)) if m.contains("Cmyk")));
    // So is a Lab surface in an RGB document.
    let s = surface("Lab", "U8", true, &[0; 4], &[]);
    let file = bundle(4, 4, "Rgb", &[layer(&raster(&s), None)], &[]);
    assert!(matches!(pcraft_layer_rgba(&file, &[0], &Limits::default()), Err(Error::Unsupported(m)) if m.contains("Lab")));
}

#[test]
fn a_zstd_bomb_is_refused() {
    // 64 MiB of zeros compresses to a few kilobytes; the tile holds 256 KiB.
    let bomb = vec![0; 64 << 20];
    let s = surface("Rgb", "U8", true, &[0; 4], &[(0, 0, &hash(&bomb))]);
    let file = bundle(4, 4, "Rgb", &[layer(&raster(&s), None)], &[&bomb]);
    assert!(file.len() < 1 << 20);
    assert!(malformed(&file).contains("more than 262144 bytes"));
    // A tile that decodes short is malformed too.
    let short = vec![0; 100];
    let s = surface("Rgb", "U8", true, &[0; 4], &[(0, 0, &hash(&short))]);
    malformed(&bundle(4, 4, "Rgb", &[layer(&raster(&s), None)], &[&short]));
}

#[test]
fn decoded_tiles_and_the_canvas_count_against_the_limits() {
    // Four distinct tiles of 256 KiB each, 1 MiB together, over a 512 × 512 canvas of 1 MiB.
    let tiles: Vec<Vec<u8>> = (1..=4).map(|v| solid(&[v, 2, 3, 4])).collect();
    let h: Vec<String> = tiles.iter().map(|t| hash(t)).collect();
    let s = surface("Rgb", "U8", true, &[0; 4], &[(0, 0, &h[0]), (1, 0, &h[1]), (0, 1, &h[2]), (1, 1, &h[3])]);
    let refs: Vec<&[u8]> = tiles.iter().map(Vec::as_slice).collect();
    let file = bundle(512, 512, "Rgb", &[layer(&raster(&s), None)], &refs);
    assert!(pcraft_layer_rgba(&file, &[0], &Limits::default()).is_ok());
    // The canvas and the decoded tiles take 2 MiB, and the manifest and the frames read take more.
    let mut limits = Limits::default();
    limits.max_total = 2 << 20;
    assert!(matches!(pcraft_layer_rgba(&file, &[0], &limits), Err(Error::LimitExceeded(_))));
    let mut limits = Limits::default();
    limits.max_entry = (1 << 20) - 1;
    assert!(matches!(pcraft_layer_rgba(&file, &[0], &limits), Err(Error::LimitExceeded(m)) if m.contains("the layer's pixels")));
    // A canvas the size of PhotoCraft's largest is refused before anything is allocated for it.
    let file = bundle(300_000, 300_000, "Rgb", &[layer(&raster(&s), None)], &[]);
    assert!(matches!(pcraft_layer_rgba(&file, &[0], &Limits::default()), Err(Error::LimitExceeded(_))));
}

#[test]
fn layers_are_found_by_path_and_layers_without_pixels_give_none() {
    let s = |v: u8| surface("Rgb", "U8", true, &[v, v, v, 255], &[]);
    let text = format!(
        r#"{{"kind":"text","text":"Hi","font_family":"Sans","size_pt":12.0,"color":{{}},"transform":[1,0,0,1,0,0],"cache":{},"psd_raw":null}}"#,
        s(3)
    );
    let uncached = r#"{"kind":"shape","fill":null,"cache":null,"psd_raw":null}"#;
    let shape = format!(r#"{{"kind":"shape","fill":null,"cache":{},"psd_raw":null}}"#, s(4));
    let smart = format!(r#"{{"kind":"smart","source":{{}},"transform":[1,0,0,1,0,0],"smart_filters":[],"cache":{},"psd_raw":null}}"#, s(5));
    let group = format!(r#"{{"kind":"group","expanded":true,"children":[{},{}]}}"#, layer(&raster(&s(1)), None), layer(&text, None));
    let adjustment = r#"{"kind":"adjustment","adjustment":{}}"#;
    // A group with a mask gives no pixels and no mask.
    let masked_group = r#"{"kind":"group","expanded":true,"children":[]}"#;
    let layers = [
        layer(&raster(&s(0)), None),
        layer(&group, None),
        layer(uncached, None),
        layer(adjustment, None),
        layer(&shape, None),
        layer(&smart, None),
        layer(masked_group, Some(&surface("Grayscale", "U8", false, &[0], &[]))),
        // A video layer: a raster layer with its frames in `video`.
        layer(&raster(&s(6)), None)
            .replace(r#""fill_cache":null"#, &format!(r#""fill_cache":null,"video":{{"frames":[{}],"source":"Blank","fps":30.0}}"#, s(7))),
    ];
    let file = bundle(1, 1, "Rgb", &layers, &[]);
    assert_eq!(read(&file, &[0]).rgba, Samples::U8(vec![0, 0, 0, 255]));
    assert_eq!(read(&file, &[1, 0]).rgba, Samples::U8(vec![1, 1, 1, 255]));
    assert_eq!(read(&file, &[1, 1]).rgba, Samples::U8(vec![3, 3, 3, 255]));
    assert_eq!(read(&file, &[4]).rgba, Samples::U8(vec![4, 4, 4, 255]));
    assert_eq!(read(&file, &[5]).rgba, Samples::U8(vec![5, 5, 5, 255]));
    assert_eq!(read(&file, &[7]).rgba, Samples::U8(vec![6, 6, 6, 255]));
    for path in [&[][..], &[1], &[2], &[3], &[6], &[8], &[1, 2], &[0, 0]] {
        assert_eq!(pcraft_layer_rgba(&file, path, &Limits::default()), Ok(None), "{path:?}");
    }
}

#[test]
fn damaged_surfaces_are_malformed() {
    let tile = solid(&[5, 5, 5, 5]);
    let other = solid(&[6, 6, 6, 6]);
    let layers = |s: &str| [layer(&raster(s), None)];
    // A tile the bundle does not have.
    let s = surface("Rgb", "U8", true, &[0; 4], &[(0, 0, &hash(&tile))]);
    assert!(malformed(&bundle(2, 2, "Rgb", &layers(&s), &[])).contains("does not have"));
    // A tile whose samples are not the ones its hash names.
    assert!(malformed(&bundle_with(2, 2, "Rgb", &layers(&s), &[(hash(&tile), other.clone())])).contains("do not match their hash"));
    // A tile that is not zstd.
    let name = format!("tiles/{}.zst", hash(&tile));
    let manifest = format!(
        r#"{{"format_version":1,"generator":"photocraft-format 0.1.0","document":{{"size":{{"width":2,"height":2}},"mode":"Rgb","layers":[{}]}}}}"#,
        layer(&raster(&s), None)
    );
    assert!(malformed(&zip(&[store(&name, b"not zstd"), store("manifest.json", manifest.as_bytes())])).contains("zstd"));
    // A frame larger than any tile's frame is refused before it is read.
    let big = vec![1u8; 300 << 10];
    let file = zip(&[store(&name, &big), store("manifest.json", manifest.as_bytes())]);
    assert!(matches!(pcraft_layer_rgba(&file, &[0], &Limits::default()), Err(Error::LimitExceeded(_))));
    // A default pixel of the wrong size, or not hex.
    assert!(malformed(&bundle(2, 2, "Rgb", &layers(&surface("Rgb", "U8", true, &[0; 3], &[])), &[])).contains("6 hex digits, not 8"));
    assert!(malformed(&bundle(2, 2, "Rgb", &layers(&surface("Rgb", "U8", true, &[0; 4096], &[])), &[])).contains("8192 hex digits"));
    malformed(&bundle(2, 2, "Rgb", &layers(&surface("Rgb", "U8", true, &[0; 4], &[]).replace("00000000", "0000000g")), &[]));
    // A hash that is not 64 hex digits.
    let s = surface("Rgb", "U8", true, &[0; 4], &[(0, 0, "../manifest")]);
    assert!(malformed(&bundle(2, 2, "Rgb", &layers(&s), &[])).contains("invalid hash"));
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

#[test]
fn other_files_are_refused() {
    assert_eq!(pcraft_layer_rgba(b"%PDF-1.7", &[0], &Limits::default()), Err(Error::Unrecognized));
    let vc = common::vectorcraft_json(4, "");
    assert!(matches!(pcraft_layer_rgba(vc.as_bytes(), &[0], &Limits::default()), Err(Error::Unsupported(m)) if m.contains(".vectorcraft")));
}

#[test]
fn a_newer_bundle_is_too_new() {
    let manifest = r#"{"format_version":2,"generator":"photocraft-format 9.0.0","document":{"layers":"anything"}}"#;
    let file = zip(&[store("manifest.json", manifest.as_bytes())]);
    assert!(matches!(pcraft_layer_rgba(&file, &[0], &Limits::default()), Err(Error::TooNew { version: 2, .. })));
}
