# craft-raster-codecs

Small raster image formats for the Craft apps, from PhotoCraft's `photocraft-codecs`: one module
per format, one implementation of each. Decoders take untrusted bytes and `Limits`, check the
declared size before allocating, and never panic. No `unsafe`; it builds for
`wasm32-unknown-unknown`. Dependencies: `thiserror`, and `rayon` on native targets (the DDS
encoder compresses rows of blocks in parallel).

```toml
craft-raster-codecs = { git = "https://github.com/storytold/craft-libs", rev = "<commit>" }
```

| Module | Format | Read | Write |
|---|---|---|---|
| `dds` | DirectDraw Surface: uncompressed masks, luminance, BC1/BC2/BC3 (DXT1/3/5), legacy and DX10 headers; top mip level | yes | uncompressed and BC1-BC3, optional mip chain |
| `dicom` | DICOM Part 10, uncompressed transfer syntaxes: MONOCHROME1/2, RGB, YBR_FULL, 8 and 16 bits | yes | no |
| `fits` | FITS primary HDU or first `IMAGE` extension, every `BITPIX`, `BSCALE`/`BZERO`, header cards as text | yes | gray or RGB; 8-bit, 16-bit, float |
| `iff` | IFF ILBM (1 to 8 planes with a palette, 24 planes, mask plane or transparent colour) and PBM, stored or ByteRun1; HAM and EHB are refused | yes | 24-plane ILBM with an optional mask plane |
| `pcx` | ZSoft PCX: RLE scanlines, 16 and 256-colour palettes, 8-bit three-plane RGB and four-plane RGBA; a `dimensions` probe | yes | no |
| `pnm` | Netpbm: PBM, PGM, PPM (plain and raw, 8 and 16 bits), PAM, PFM | yes | PGM, PPM, PAM, PFM |
| `xpm` | X PixMap: XPM3 and XPM2, X11 colour names | yes | no |

## The model

`Image` is a flat raster: width, height, a `ChannelLayout` (gray, RGB or CMYK, each with or
without straight alpha), a `SampleType` (`U8`, `U16`, `F32`) and interleaved, native-endian
samples, top row first. It also carries what these formats state beside the pixels: `dpi` (PCX),
`text` (FITS header cards) and `warnings` (a FITS cube opened as its first plane).

```rust
use craft_raster_codecs::{Limits, pnm};

let image = pnm::decode(b"P2\n2 1\n255\n0 255\n", &Limits::default())?;
assert_eq!(image.data(), &[0, 255]);
let bytes = pnm::encode(&image)?;
```

Each module has a `looks_like_*` (or `is_dicom`) sniffer for the format's signature. `dds` and
`fits` also have `has_additional_images`, which says without decoding whether a file holds
more than the one image `decode` returns (cube faces, array slices, further HDUs).

`xpm::x11_color_rgb` also looks up exact normalized X11 names as RGB channels in `0..=1`.
It accepts only 1–64 lowercase ASCII letters or digits, without normalization, XPM-only aliases,
CSS overrides or color-space conversion. XPM decoding keeps its existing name and alias policy.

Encoders take an image already in a layout and depth the format stores, and return
`CodecError::Encode` for any other. Choosing the nearest layout and converting to it stays in the
app. The same goes for colour management, orientation, format detection across a whole format
list, and mapping `Image` onto the app's own document.

## Limits

`Limits` bounds width, height, pixel count and the bytes of the decoded buffer. The default is
2^18 pixels a side, 2^30 pixels and 8 GiB (1 GiB on 32-bit targets). Compressed formats (IFF,
PCX) also refuse a file too small to hold what its header declares, before allocating. Text
formats (plain PNM, XPM) reserve from the bytes present, not from the header.

## Tests

`cargo test -p craft-raster-codecs`. `tests/roundtrip.rs` writes every layout and depth each
writer stores and reads it back exactly, then truncates, bit-flips and overwrites header bytes of
a valid file of every format: a reader may refuse, never panic. Two tests are ignored by default:
a DDS encode timing, and a PNM peak-memory check that reads the process's RSS.
