# craft-image-codecs

Codecs for flat raster image formats, written in pure Rust. This crate defines its own `Image`
type and shares its baseline and extended JPEG decoders through `craft-jpeg`, its optional HEIF
decoder through `craft-heif`, and its optional AVIF decoder through `craft-video`'s AV1 decoder.
It builds for `wasm32-unknown-unknown` and does no filesystem I/O: decoders take `&[u8]` and
encoders return `Vec<u8>`.

It follows the "Avoiding GIMP's hole" rules in `docs/architecture.md` §1.1:

* **Symmetric.** Every format we write, we can also read. The only exceptions are listed in
  `ASYMMETRIC_EXCEPTIONS`, each with a reason. The test suite checks this.
* **Depth-preserving.** An image can hold U8, U16, F16 or F32 samples, and each format keeps
  what it can hold.
* **Metadata-preserving.** ICC, EXIF, XMP, DPI and text are kept wherever the format supports
  them.
* **Honest.** `caps(format)` says what each format holds, and
  `fidelity_warnings(&image, format)` lists what an export will lose before you write it. Both
  functions use the same encode plan as `encode`, so a warning appears exactly when the file
  changes. On the way in, `Image::warnings` lists what the decoded image doesn't show: frames or
  pages left out, or data that ended early.

```rust
use craft_image_codecs::*;

let img = decode(&bytes)?;                               // detect + decode (default Limits)
for w in fidelity_warnings(&img, Format::Jpeg) {         // e.g. "16-bit will be reduced to 8-bit"
    println!("{w}");
}
let out = encode(&img, Format::Tiff, &EncodeOptions::default())?;
```

```toml
craft-image-codecs = { git = "https://github.com/storytold/craft-libs", rev = "<commit>" }
```

## Capability matrix (default build)

| Format | Read | Write | Depths (native) | Layouts (native) | Alpha | ICC | EXIF | XMP | DPI | Text | Lossy write | Backend |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| PNG | yes | yes | U8, U16 | Gray, GrayA, RGB, RGBA | yes | yes (iCCP) | yes (eXIf) | yes (iTXt `XML:com.adobe.xmp`) | yes (pHYs) | yes (tEXt/zTXt/iTXt) | no | `png` |
| JPEG | yes | yes | U8 | Gray, RGB, CMYK | no | yes (multi-segment APP2) | yes (APP1) | yes (APP1) | yes (JFIF) | no | yes | `zune-jpeg` / `jpeg-encoder` |
| TIFF | yes | yes | U8, U16, F32 | all six (CMYK, CMYK+A included) | yes | yes (tag 34675) | no | yes (tag 700) | yes | yes (Description, Make, Model, Software, DateTime, Artist, Copyright) | no | `tiff` |
| WebP | yes | yes | U8 | RGB, RGBA | yes | yes | yes | yes | no | no | yes (VP8, when `webp_lossless` is off) | `image-webp` / built-in VP8 |
| GIF | yes | yes | U8 | RGBA | 1-bit | no | no | no | no | no | yes (256-colour palette) | `image` |
| BMP | yes | yes | U8 | RGB, RGBA | yes | no | no | no | yes (header pixels per metre) | no | no | `image` |
| TGA | yes | yes | U8 | Gray, GrayA, RGB, RGBA | yes | no | no | no | no | no | no | `image` (read) / built in (RLE write, one scanline per packet) |
| ICO | yes | yes | U8 | RGBA (at most 256×256) | yes | no | no | no | no | no | no | `image` |
| DDS | yes | yes | U8, U16 (U16 read only) | Gray, GrayA, RGB, RGBA | yes | no | no | no | no | no | yes (BC1–BC5, when `EncodeOptions::dds` asks for one) | [`craft-dds`](../dds/README.md) |
| Netpbm (PBM/PGM/PPM/PAM/PFM) | yes | yes | U8, U16, F32 (float: PFM gray/RGB only) | all six (PAM `CMYK`/`CMYK_ALPHA`) | yes (PAM) | no | no | no | no | no | no | built in |
| QOI | yes | yes | U8 | RGB, RGBA | yes | no | no | no | no | no | no | `image` |
| OpenEXR | yes | yes | F16, F32 | Y, YA, RGB, RGBA | yes | no | no | no | no | no | no | `exr` |
| Radiance HDR | yes | yes | F32 | RGB | no | no | no | no | no | no | yes (RGBE) | `image` |
| FITS | yes | yes | U8, U16, F32 | Gray, RGB | no | no | no | no | no | yes (header cards) | no | built in |
| JPEG XL (.jxl) | **no** (see #2868) | yes | U8, U16 | Gray, RGB, RGBA (GrayA expands to RGBA) | yes | no (bare codestream) | no | no | no | no | no | `zune-jpegxl` |
| DPX | yes | yes | U8, U16 (10/12-bit code values read and written exactly through U16) | Gray, RGB, RGBA | yes | no | no | no | no | DPX header fields only (creator, project, copyright, creation time, transfer, colorimetric, frame position, frame rate, time code) | no | built in |
| Cineon | yes | yes | U16 (10-bit code values read and written exactly through U16) | Gray, RGB | no | no | no | no | no | no | no | built in |
| DICOM | yes | **no** | U8, U16 | Gray, RGB | no | no | no | no | no | no | n/a | built in |
| Amiga IFF (ILBM/PBM) | yes | yes (24-bit ILBM) | U8 | RGB, RGBA | yes (1-bit mask plane on write) | no | no | no | no | no | no | built in |
| Sun Raster (`.ras`) | with feature `sunraster` (incl. RLE and colour maps) | with feature `sunraster` (uncompressed) | U8 | Gray, RGB, RGBA | yes | no | no | no | no | no | no | `sunraster` (off by default) |
| SGI (`.sgi`, `.rgb`, `.bw`) | with feature `sgi` | with feature `sgi` (RLE) | U8, U16 | Gray, Gray+A, RGB, RGBA | yes | no | no | no | no | no | no | `sgi` (off by default) |
| JPEG-LS (`.jls`) | yes (any NEAR; one scan per component) | yes (lossless) | U8, U16 | Gray, RGB | no | no | no | no | no | no | no | built in |
| AVIF | with feature `avif-decode` (native only) | only with feature `avif` | U8, U16 (10/12-bit decodes to U16; PQ/HLG to linear F32) | RGB, RGBA | yes (auxiliary alpha) | read (`colr` prof/rICC) | read | read | no | no | yes | `craft-video` AV1 decoder; `image` + `ravif` to write |
| HEIF/HEIC | with feature `heif` | **no** | U8, U16 (10/12-bit decodes to U16) | RGB, RGBA | yes (auxiliary alpha) | yes (`colr` prof) | yes | yes | no | no | n/a | `heic-rs` |

"Native" means the data is stored and read back without conversion. Anything else is converted by
the encode plan, and `fidelity_warnings` reports the conversion when it loses information:

* A float image written to an integer format goes to U16 where the format supports it. Values
  outside 0..1 raise `RangeClipped`.
* Integer data written to a float-only format (EXR, HDR) becomes F32 without loss.
* CMYK is converted to RGB with a naive formula that is not colour-managed. `CmykConverted` is
  raised and the ICC profile is dropped, because a CMYK profile does not describe RGB data.
* Gray written to an RGB-only format is expanded to RGB without loss, so no warning is raised.

`Image::convert` applies the same ICC rule: it keeps the profile only when the colour model stays
the same.

Palette BMP and PNG pixels decode as RGB(A) in `Image`; `bmp_palette` and `png_palette` expose the
embedded table (and a PNG's transparent entry) so `photocraft-io` can open them in Indexed Color mode.
`bmp_palette` returns the colours the header declares, not the decoder's 256-entry padding.
`encode_bmp_indexed` writes the inverse: indices plus a table as an 8-bit palette BMP;
`encode_bmp_indexed_with_dpi` also writes a resolution in the header.

## Documented asymmetries and limitations

* **JPEG XL export.** Writes lossless 8/16-bit bare codestreams with `zune-jpegxl`.
  Native layouts are Gray, RGB and RGBA. GrayA is expanded to RGBA at the
  original depth. For 16-bit RGBA (including expanded GrayA), we correct
  zune-jpegxl 0.5.2's extra-channel header, which otherwise describes the
  16-bit alpha samples as 8-bit. This preserves alpha precision; the independent
  jxl-oxide decoder test checks the actual gray and alpha samples. The expansion
  is not a native GrayA layout. No ICC, EXIF, XMP or DPI container boxes are written; the
  document pipeline converts tagged images to sRGB and reports metadata loss.
  Reading remains in the separate #2868 PR. The `ASYMMETRIC_EXCEPTIONS`
  entry stays until import is merged.

* **DDS** (through [`craft-dds`](../dds/README.md), whose `DdsCompression` and `DdsOptions` are
  re-exported). The top mip level of a 2D texture: uncompressed pixels described by channel masks
  (8, 16, 24 or 32 bits, RGB or luminance, with or without alpha), 16 bits per channel
  (`R16_UNORM`, `R16G16_UNORM`, `R16G16B16A16_UNORM` and their typeless forms, legacy FourCC 36),
  BC1/BC2/BC3 (DXT1/DXT3/DXT5) or unsigned BC4/BC5 (ATI1/ATI2, BC4U/BC5U, DXGI 79, 80, 82 and 83),
  with the legacy header or the DX10 one. BC4 opens as Gray, and BC5 and `R16G16_UNORM` as RGB
  with blue 0 (they store only red and green; a normal map's Z is not reconstructed). ATI2 is read
  as Microsoft reads it, red in the first block. Uncompressed row padding follows the header's
  `DDSD_PITCH` stride; without that flag, rows are tightly packed. The 16-bit formats and masks
  with a channel wider than 8 bits open as U16, everything else as U8; the other mip levels are
  ignored. Cubemaps, texture arrays, volume textures, premultiplied-alpha textures (DXT2/DXT4, or DX10 alpha mode 2), signed (SNORM) BC4/BC5 and other formats
  (BC6H, BC7, float) are refused with `CodecError::Unsupported`. Writing (`EncodeOptions::dds`)
  makes a 2D texture: uncompressed by default (BGRA, BGRX, or luminance with or without alpha, so
  it reads back exactly), BC1/BC2/BC3 (from RGBA) with the legacy header, or BC4 (from gray, for
  roughness, height and other single-channel maps) and BC5 (from RGB, blue dropped, for normal
  maps) with the DX10 header (`BC4_UNORM`, `BC5_UNORM`). The plan converts other layouts to what
  the compression stores, so `fidelity_warnings` reports colour to gray for BC4 and dropped alpha
  for both; BC5's dropped blue comes under `LossyCompression`. The block encoder is our own, from
  Microsoft's public block-compression documentation: RGB565 endpoints fitted along each block's
  principal axis and refined by least squares, indices picked against the palette the reader
  decodes; BC1 turns texels under half opacity into its transparent black, and BC3, BC4 and BC5
  pick whichever mode of their value blocks fits. `mipmaps` adds the full chain down to 1×1, each
  level a box filter of the one above (the last column and row of an odd-sized level average
  three pixels, so none is dropped). Cubemaps, arrays, BC6H, BC7 and sRGB (DX10-only) formats
  aren't written.

* **FITS.** The image in the primary HDU, or the first `IMAGE` extension, is read; `NAXIS3 = 3`
  opens as RGB and any other plane count opens the first plane with `DecodeWarning::MorePages`.
  Rows follow `ROWORDER` (bottom-up by default, as Siril and DS9 do). Physical values that fit
  stay exact as U8/U16 (a camera's `BITPIX = 16`, `BZERO = 32768`); the rest become F32, taken as
  normalised when at most 2 and scaled by 65535 (or the maximum) otherwise. Header cards are
  kept as text (`HIERARCH` for long names). Tile-compressed (fpack) files are refused with
  `CodecError::Unsupported`; Bayer mosaics open undebayered.

* **DPX.** SMPTE 268M frames in either byte order (`SDPX`, `XPDS`): the first image element,
  descriptor 6 (luma), 50 (RGB) or 51 (RGBA), unsigned and unencoded, at 8, 10, 12 or 16 bits.
  10-bit data may be packed (0) or filled three to a word (1, method A; 2, method B), 12-bit data
  packed (0) or one to a 16-bit word (1, method A). 10- and 12-bit code values open as U16
  `round(c × 65535 / (2^b − 1))`, so a frame written back at its depth keeps them exactly. No
  colour transform is applied: `DecodeWarning::DpxEncoding` names the transfer characteristic and
  colorimetric specification, so a log frame is reported as log. More elements than the first
  give `DecodeWarning::MoreElements`; the image orientation is applied on import. Writing is
  big-endian, one unencoded element, at `EncodeOptions::dpx_bits` (8/10/12/16; by default 8 for
  U8 and 16 otherwise), packing 1 for 10 and 12 bits; gray with alpha is written as luma.
  `FidelityWarning::BitsReduced` reports a 10- or 12-bit save (and every Cineon save) that drops
  precision: always for float samples, for U16 ones unless they are already code values at that
  depth.
  Other descriptors, signed or run-length-encoded data and other bit depths return
  `CodecError::Unsupported` naming the field.

* **Cineon.** Kodak's 10-bit logarithmic film-scan format (the one DPX grew from),
  read from the same Kodak header family: the generic 1024-byte header, big-endian, one (Gray)
  or three (RGB) channels, three 10-bit samples packed into each 32-bit word, lines on a word
  boundary. The 10-bit code values open as U16 `round(c × 65535 / 1023)`; the file's black/white
  reference points are a log-conversion choice and are not applied. A depth other than 10 bits or
  a channel count other than 1/3 returns `CodecError::Unsupported`; truncation errors rather than
  panicking. Writing follows the Cineon 4.5 layout: a 1024-byte generic header (version `V4.5`,
  one or three channels designated as universal-metric gray or R, G, B, 10 bits, data 0–1023 over
  densities 0–2.048) and a 1024-byte film header left undefined, then the samples three to a
  big-endian 32-bit word, left justified (packing 5), lines on a word boundary. Samples are written
  as `round(v × 1023 / 65535)`, so a frame opened from Cineon keeps its code values; any other
  image takes the encode plan (alpha dropped, CMYK and Lab to RGB) and is quantized to 10 bits.

* **DICOM (read-only).** Single-frame medical images over the uncompressed transfer syntaxes
  (implicit VR little-endian, explicit VR little/big-endian): MONOCHROME1/2 gray (with the
  1 = white inversion) and RGB/YBR_FULL, 8- and 16-bit allocated samples, interleaved or planar,
  scaled from `BitsStored`/`HighBit` and mapped up when `PixelRepresentation` is signed.
  Undefined-length sequences outside the pixel data are skipped, as clinical files contain them.
  Compressed transfer syntaxes, sequences over the pixel data and other photometric
  interpretations return `Unsupported`/`Malformed`. There is no DICOM writer
  (`ASYMMETRIC_EXCEPTIONS`).

* **Amiga IFF.** `FORM ... ILBM` stores interleaved 1–8 bitplanes;
  `FORM ... PBM ` stores 8-bit chunky palette indices with even-padded rows. Both support
  stored and ByteRun1 compression, a `CMAP` palette (grey ramp when absent, black for missing
  entries), and transparent-color masking. ILBM also supports a separate mask plane and
  24-bitplane RGB ("deep" ILBM: 8 planes each of red, green and blue, least significant bit
  first, no palette). HAM/EHB viewport modes, PBM mask planes, a transparent color in a
  24-bitplane file, other bit depths and compressions return `CodecError::Unsupported`.
  Writing produces a 24-bitplane ILBM, ByteRun1 compressed one plane row at a time, at most
  65535 × 65535 pixels. An image with alpha gets a mask plane (masking 1): alpha of 128 or more
  is opaque, and `AlphaBinarized` is raised when any pixel is not fully opaque. Gray is
  expanded to RGB, and 16-bit and float images are reduced to 8 bits.

* **Camera raw files** (DNG, CR2, NEF, ARW… which are TIFF-structured) are recognised and refused
  with `CodecError::Unsupported`: they are sensor data, not flat images. `photocraft-raw` decodes
  and develops them, and `photocraft-io` routes them there.

* **HEIF/HEIC (read-only, feature `heif`).** Decoding lives in the optional `craft-heif` crate
  behind the non-default `heif` feature (official PhotoCraft builds enable it; distributors choose).
  Without it, HEIF is still detected and decoding returns `CodecError::Unsupported` ("HEIC/HEIF
  support isn't included in this build"). iPhone and Mac photos decode with `heic-rs` (pure Rust, no `unsafe`):
  single pictures and grid-tiled photos, 8 to 12 bits, auxiliary alpha, ICC, EXIF and XMP. The
  container's `irot`/`imir`/`clap` are applied and EXIF Orientation is rewritten to 1 (HEIF's EXIF
  tag only repeats the container's, and applying both would turn the photo twice). Grayscale
  decodes to RGB. Image sequences, overlays and identity derivations, multilayer HEVC and some
  4:2:2/4:4:4 streams return `CodecError::Unsupported` or `Malformed`, never wrong pixels. Writing
  needs an HEVC encoder and every mature one is C, so HEIF is listed in `ASYMMETRIC_EXCEPTIONS`.
* **AVIF.** Encoding uses `ravif`, which is pure Rust, behind the non-default `avif` feature.
  Decoding (`codecs::avif_decode`, behind the non-default `avif-decode` feature) reads the HEIF
  boxes itself and decodes the AV1 pictures with `craft-video`'s clean-room pure-Rust AV1 decoder:
  one `av01` image or a `grid`, 8/10/12-bit, 4:2:0/4:2:2/4:4:4/monochrome, the `nclx` colour (or
  the sequence header's) and an ICC profile, an `auxl` alpha plane (`prem` too), the Exif and XMP
  items, and `clap`/`irot`/`imir`, which are applied unless `keep_orientation` is set, as for HEIF.
  PQ and HLG open as linear light in F32, as HEIF does. Image sequences (animated AVIF), depth and
  gain-map auxiliary images and layered progressive images aren't read. It is bounded like the
  other decoders: the `Limits` are checked against the declared size before any picture decodes,
  against every picture and every output buffer, `max_alloc` caps the AV1 decoder's picture store,
  box, item and extent counts are capped, item data can't exceed the file, and a decoder panic is
  `CodecError::Malformed`. wasm32 builds leave AVIF unreadable, since a panic there aborts. In a
  default build AVIF is neither readable nor writable, so the symmetric guarantee holds; a build
  with only one of the two features is listed in `ASYMMETRIC_EXCEPTIONS`.
* **Lossy WebP.** Lossless (VP8L, via `image-webp`) is the default. `webp_lossless: false`
  writes a lossy file with our own clean-room VP8 key-frame encoder (`codecs::vp8`, from
  RFC 6386; `webp_quality` 0–100 on libwebp's scale sets the quantizer). Alpha travels losslessly
  in an `ALPH` chunk and ICC/EXIF/XMP in their own chunks of the extended (`VP8X`) container. The
  encoder makes no rate-distortion decisions yet, and a side longer than 16383 px (the VP8 frame
  header's limit) is refused with `CodecError::Encode`. We read both lossy and lossless files.
* **Animation and multi-page files** (APNG, animated GIF/WebP, multi-page TIFF): only the first
  frame or page is decoded, and a single frame is written. `FormatCaps::animation` marks
  containers that can hold more frames. The decoded image then carries a
  `DecodeWarning::MoreFrames` / `MorePages` in `Image::warnings`, with the total when the file
  states it (frames and TIFF directories are counted without decoding them; reduced-resolution
  TIFF directories and transparency masks are not pages). Any TIFF page can still be opened
  with `decode_tiff_page` (and `photocraft_io::import_tiff_page`). Every frame of a GIF comes
  from `decode_gif_frames`: each frame composited onto the logical screen with the disposal
  methods applied, as RGBA8 with its delay in milliseconds, within `Limits` and
  `MAX_GIF_FRAMES`. Every frame of an animated WebP comes from `decode_webp_frames` the same
  way, composited onto its canvas, with its duration in milliseconds and the file's ICC profile.
  The video-layer commands use both (`photocraft_io::import_gif_frames`,
  `photocraft_io::import_webp_frames`).
* **EXIF in TIFF** is stored as a sub-IFD rather than a blob, so it is not preserved yet.
  `caps.exif = false` for TIFF, and a warning is raised.
* **Orientation** (EXIF tag 274 in JPEG, PNG `eXIf` and WebP; the decoded page's tag in TIFF and
  BigTIFF) is applied on
  decode, like Photoshop: the pixels come back upright and the EXIF/XMP orientation is rewritten
  to 1 (`DecodeOptions::keep_orientation` opts out). Encoders always write Orientation = 1
  (`upright_exif`, `upright_xmp`, which change only the tag's value, kept in its own type), so upright pixels
  are never rotated twice. Parsing is strict: an IFD0 inside the 8-byte header or cut off before
  its next-IFD pointer, an Orientation that is not exactly one SHORT or LONG, or an out-of-range value
  reads as 1; with duplicate entries the first wins. `Limits` are checked again on the upright
  size (5–8 swap width and height), and the rotated buffer is allocated fallibly.
  `Image::oriented` turns any layout and depth in parallel bands (about 10 ms for 24 MP RGB8 in
  release).
* **Resolution** (#1691, module `resolution`): a file without a resolution of its own format
  (no PNG `pHYs`, a WebP, HEIF, …) takes it from its XMP `tiff:XResolution` / `YResolution` /
  `ResolutionUnit`, else from EXIF IFD0 (RATIONAL X/YResolution, unit 2 inch — the default when
  absent — or 3 cm; unit 1 or anything malformed counts as no resolution). A JPEG uses, like
  Photoshop, the first usable of: APP13 Photoshop ResolutionInfo (0x03ED), XMP, EXIF, and only
  then the JFIF density (cameras write EXIF only, many tools write a default JFIF 72). On export
  the EXIF and XMP resolution are rewritten to the image's (`export_exif`, `export_xmp`): the
  RATIONAL values in place, ResolutionUnit set to 2, an entry that can't be rewritten removed
  from IFD0 without moving any other data. IFD1 (the thumbnail's) is left as it is.
* **Resolution without decoding** (module `resolution_probe`): `file_resolution(bytes)` reads
  the pixels per inch a JPEG, PNG, WebP, BMP or classic TIFF records from its headers alone, for
  placing a picture at its physical size before (or without) decoding it. JPEG, PNG, WebP and
  TIFF answer as `decode` would; BMP reads the info header's pixels per metre. Values outside
  1–100 000 ppi count as none.
* **JPEG**
  * 8-bit only. Neither decoder backend supports 12-bit.
  * A file cut off inside its image data decodes leniently (a baseline JPEG's missing rows come
    out grey) but never silently: the image carries `DecodeWarning::Truncated`. A file that ends
    before any scan data is an error.
  * CMYK is always written 4:4:4 (subsampled CMYK is not portable) as Adobe-inverted CMYK with an
    APP14 marker.
  * Text is not written because there is no COM-segment support.
  * EXIF and XMP must each fit in one APP1 segment (65 533 bytes with its header). Metadata that
    doesn't fit is dropped and the export still succeeds, with a
    `MetadataTooLarge` warning ("EXIF (… bytes) is too large for the format; it will be dropped").
    Before dropping XMP, the encoder first removes the layered-document properties
    (`DocumentAncestors`, `TextLayers`) that mean nothing in a flat JPEG. Extended XMP is not
    written.
* **TIFF decode**
  * Classic TIFF and BigTIFF (version 43, 8-byte offsets), both byte orders, strips or tiles,
    planar configuration 1 or 2.
  * Supports 1, 2 and 4-bit gray, 8/16/32/64-bit integers (32 and 64-bit are reduced to U16),
    F16, F32 and F64 (converted to F32), WhiteIsZero (#691: the tones are no longer inverted),
    palette images (opened as RGB) and associated alpha (made straight).
  * Extra samples: the first alpha sample (ExtraSamples 1 or 2, wherever it sits) is the image's
    alpha. Every other extra sample, such as a spot colour or a further alpha channel
    (ExtraSamples 0), comes back in `Metadata::extra_samples`: one plane per sample in the
    image's sample type, with its ExtraSamples value as `kind` (a reserved value above 2 reads as
    0). At most `MAX_EXTRA_SAMPLES` (16) are kept, and they count against `Limits::max_alloc` with
    the image: when the image fits but its extra samples don't, it opens without them and carries
    `DecodeWarning::ExtraSamplesDropped`.
  * Every directory is reachable: `tiff_info` lists the main IFD chain and its SubIFDs (tag 330)
    with each page's size, kind (page, reduced-resolution copy, mask), storage, InkSet and
    ExtraSamples, walked with a seen-set, a depth limit and a 10 000-directory cap;
    `decode_tiff_page` opens any of them.
    `decode` opens the first full-resolution page, like Photoshop (thumbnails are skipped).
  * Banded decoding: the image is allocated once at its final size and each strip or row of
    tiles is decoded straight into it, in parallel on native targets, with at most one
    strip/tile-sized scratch buffer per worker (30 MP RGB8: Deflate 345 → 32 ms, LZW 428 → 43
    ms in release). Image data cut off by the end of the file decodes with
    `DecodeWarning::Truncated`; a file far too small for its declared size (more than any
    compression could expand) is refused before allocating.
  * YCbCr with three 8- or 16-bit samples decodes to RGB. YCbCr with other sample counts or
    bit depths returns `Unsupported`. JPEG-compressed YCbCr uses the full-range JFIF conversion.
  * 8/16-bit CIELab and ICCLab decode to the Lab channel layout.
  * Signed integer samples and unsupported compression methods return `Unsupported`.
* **TIFF encode** writes classic TIFF, or BigTIFF when `EncodeOptions::tiff_bigtiff` is set or
  the file could pass 4 GiB. `Metadata::extra_samples` (up to 16) are written after the colour
  and alpha samples, with their `kind` in the ExtraSamples tag (a reserved `kind` above 2 is
  written as 0). Each plane must be exactly
  `width × height × sample.bytes()` bytes; a plane of any other length, and more than 16 planes,
  are encode errors rather than silently truncated or reinterpreted.
* **EXR**
  * Multi-part files (a Maya/Arnold render writes one part per AOV) open the part that
    looks most like a colour image — RGBA beats RGB beats YA beats a single channel, so a
    depth-only part no longer fails the file — with `DecodeWarning::MoreParts` naming how
    many parts exist. `exr_info` lists every part (name, view, size, channels, deep/tiled)
    without decoding pixels, and `decode_exr_part` decodes a chosen one. Deep parts stay
    with the deep decoder: a file that has any is refused here with a pointer to it.
  * Reads at full resolution, from the data window.
  * Channel names are matched by suffix, so `layer.R` counts as `R`.
  * Subsampled channels are unsupported.
  * Cryptomatte layers (specification 1.2, as written by Arnold, V-Ray, Redshift, Mantra/Karma
    and Cycles) are listed and decoded structurally: `cryptomatte_layers` reads the header
    attributes, channels and embedded manifest without pixels, `decode_cryptomatte` returns the
    per-pixel (ID, coverage) pairs, and `cryptomatte_id`/`cryptomatte_key`/`cryptomatte_preview_color`
    hash names to IDs the way the reference implementation does (official test vectors in the
    test suite).
  * Deep data (scanlines or single-level tiles, compressed NONE, RLE or ZIPS) opens as a flat
    image: each pixel's samples are composited into one, with
    `DecodeWarning::DeepFlattened` because per-pixel depth is not kept. Deep ZIP (16-line
    blocks) and deep mip-map or rip-map tiles return `Unsupported`.
  * Only lossless compressions are offered for writing: None, RLE, ZIP1, ZIP16 and PIZ.
* **Netpbm writing** picks the subtype from the image: float gives PFM, gray gives P5, RGB gives
  P6, and anything with alpha or CMYK gives P7 (PAM). Writing a `.pbm` file therefore produces
  whichever of these fits the data.
* **Colour conversions** (CMYK↔RGB, RGB→gray) are naive. Colour-managed conversion belongs in
  `photocraft-color`.

## Limits (decompression bombs)

`DecodeOptions { limits: Limits { max_width, max_height, max_pixels, max_alloc }, .. }` guards
decoding. Header dimensions are checked before the pixel buffer is allocated, and the budget is
also passed to the underlying decoders. A violation returns `CodecError::LimitExceeded`. The
defaults are 262144 px per side, 2^30 pixels and 8 GiB (1 GiB on 32-bit targets such as wasm).
PNG also bounds the aggregate decoded text and XMP (UTF-8 keywords and values) by `max_alloc`,
including chunks after IDAT. This budget is separate from pixels, not a total process-memory cap.
`Image::to_rgba_f32(&limits)` checks its output buffer against the same `Limits` and returns
`CodecError::LimitExceeded` before allocating when the RGBA f32 copy would exceed them.
PNG `cICP` and `cLLI` values are retained as image color metadata. `encode_png_pq` writes linear-light
Rec. 2020 RGB as 16-bit BT.2100 PQ, with measured content-light levels and the required chunks.

## Tests

`cargo test -p craft-image-codecs` covers:

* the full round-trip matrix (every writable format × 4 sample types × 6 layouts), exact for
  lossless formats and above a PSNR threshold for lossy ones;
* metadata preservation per format, checked against `caps`;
* magic-number detection, including negatives;
* fidelity warnings, and a cross-check that the warnings match the actual round-trip result;
* the symmetric-capability guarantee;
* malformed input: truncation, bit flips and proptest random bytes;
* Adam7 PNG against progressive PNG and against the `image` crate as an oracle;
* limit enforcement.

With the `corpus` feature, the PngSuite tests read `corpus/pngsuite/` at the repo root. They
take the PNGs that `tests/it/common/pngsuite.tsv` lists (one `sha256<TAB>name` row per file),
check each file's SHA-256 before decoding it, and compare it against the `image` crate's
decoder. A listed PNG that is missing or changed fails the tests; a file the list does not name is
not read. To fill the folder, download
[PngSuite-2017jul19.tgz](http://www.schaik.com/pngsuite/PngSuite-2017jul19.tgz) from the
[PngSuite page](http://www.schaik.com/pngsuite/), check that its SHA-256 is
`0294b244c95a8342c01b00010cf34abdcabc7c6a34fd0fe1bd963917537bfdc8`, and extract it into
`corpus/pngsuite/`. No PngSuite file is committed here; the list links the suite's
[terms](http://www.schaik.com/pngsuite/PngSuite.LICENSE).

With the `corpus` feature, the deep EXR test reads `11.deep.exr`, `42.deep.exr`, `64.deep.exr`,
`multivariate.deep.exr` and `objectid.deep.exr` from `corpus/exr/` at the repo root. These are the
deep test images in `src/test/bin/test_images` of the
[OpenEXR repository at commit `69b2604fc76e370615438bdc8d2cd95b9349c12e`](https://github.com/AcademySoftwareFoundation/openexr/tree/69b2604fc76e370615438bdc8d2cd95b9349c12e/src/test/bin/test_images).
The test checks each file against the SHA-256 pinned in `REAL_FILE_SHA256` in
`tests/it/deep_exr.rs` before decoding it, and fails when a file is missing, is not a regular
file, is larger than 4 MiB or does not match. It reads no other file in the folder. To fill the
folder, copy the five files from that commit into `corpus/exr/`, or run
`cargo xtask corpus --exr` in a checkout of [PhotoCraft](https://github.com/storytold/PhotoCraft)
at commit `74330157cb601573ae4523a95bb9132822fc3918` and copy its `corpus/exr/` here, as CI does.
The images are not committed here; OpenEXR distributes them under its
[license](https://github.com/AcademySoftwareFoundation/openexr/blob/69b2604fc76e370615438bdc8d2cd95b9349c12e/LICENSE.md).

## Redistribution notices

JPEG encoding uses `jpeg-encoder` 0.6.1, including work from the Independent JPEG Group.
Retain this crate's [NOTICE](NOTICE) with binary distributions using that encoder. It includes
its MIT license, the IJG acknowledgement and original IJG/SIMD notices. Both upstream license
alternatives and source checksums are retained in
[`licenses/jpeg-encoder-0.6.1`](https://github.com/storytold/craft-libs/tree/main/licenses/jpeg-encoder-0.6.1)
at the repository root.
