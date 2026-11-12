# craft-heif

An optional HEIF / HEIC decoder for iPhone and Mac photos, shared by LightCraft and PhotoCraft. It was moved from
LightCraft's `lightcraft-heif`. [`heic-rs`](https://github.com/tbraun96/heic-rs)
(pure Rust, no `unsafe`, MIT OR Apache-2.0, written from the H.265 / HEIF specifications) reads the
container and reconstructs the HEVC pictures; this crate turns them into 16-bit RGB itself so the
result matches libheif to within one code value:

- the colour description is the item's `colr nclx` (or its first tile's), else the HEVC VUI
  (iPhones: full range), else the H.265 defaults; an unspecified matrix is BT.601. heic-rs alone
  assumes BT.709 limited range, about 8 code values off on every iPhone photo;
- chroma is upsampled by nearest neighbour, or centred bilinear when a `clap` starts on an odd
  pixel, as libheif does;
- `imir` axis 0 flips top and bottom, as libheif writes and reads it (heic-rs names it the other way);
- a grid's ICC profile may sit on its tiles.

Single pictures and grid-tiled photos, 4:2:0 and monochrome, 8- to 10-bit, alpha auxiliary images
(other auxiliary images — HDR gain maps, depth — are ignored), the container's rotation/mirror/crop,
ICC, EXIF and XMP, and the file's thumbnail item. 4:2:2 and 4:4:4 (Canon/Sony "HIF") are refused
with `Error::Unsupported`: heic-rs 0.1.1 loses sync on them. Read-only.

```rust
let info = craft_heif::probe(&bytes)?;           // size, depth, alpha: check limits first
let img = craft_heif::decode(&bytes, &craft_heif::Options::default())?;
// img.width, img.height, img.has_alpha, img.bit_depth, img.data (RGB/RGBA u16), img.colour, img.icc, img.exif, img.xmp
let thumb = craft_heif::decode_thumbnail(&bytes, 256, &Default::default())?; // Option<Decoded>
let (exif, xmp) = craft_heif::read_metadata(&bytes)?;  // no picture decoded: for cataloguing
```

Chroma is upsampled like libheif by default.
`craft_heif::Options { chroma: craft_heif::Chroma::Apple, ..Default::default() }` matches Apple's
decoder (and so Photoshop on macOS) instead; the two differ only next to sharp colour edges, where
Apple's blends the neighbouring column and, for 4:2:0, the neighbouring row.

Errors are `Error::Unsupported` (image sequences, overlays, 4:2:2/4:4:4, …), `Error::Limit` or
`Error::Malformed`. It never panics: every call into heic-rs runs under `catch_unwind`, and a panic
inside it (PhotoCraft's fuzzing found two in 0.1.1) becomes `Error::Malformed`.

Features: `decode` (default) is the decoder above. `metadata` adds `craft_heif::items::read`: the
first `Exif` item and XMP packet from the box structure alone (`iinf`, `iloc`, `idat`; at most 4096
boxes, 1024 items, 1024 extents per item, 32 MiB per item), without heic-rs. A build with
`default-features = false, features = ["metadata"]` has no heic-rs dependency, and its
`read_metadata` is that reader. With both features, a test checks that the two readers agree.

Feature `testdata` exposes `craft_heif::testdata`: lossless synthetic HEIF files (PCM-coded
32 × 32 HEVC pictures, grids, every property above) for the tests of the crates above.

## Why it is a separate, optional crate

- **A young decoder.** heic-rs is new and has a single maintainer. It is pinned exactly
  (`=0.1.4`); moving the pin is a reviewed change (re-run the crate tests and each app's HEIF tests,
  e.g. `cargo test -p lightcraft-codecs --features heif`). The other permissive candidates were
  rejected: heif-oxide's HEVC decoder (rust_h265) follows FFmpeg's (LGPL) code structure, and
  gamut-heic has no HEVC decoder of its own.
- **HEVC patents are a distributor's call.** Apps use this crate only behind a non-default
  `heif` feature, which their release packages enable. Without it, HEIC files are still
  recognised, and opening one gives a clear "HEIC/HEIF support isn't included in this build" error.

The crate depends on no other Craft crate.
