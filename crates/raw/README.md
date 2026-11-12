# craft-raw

Camera raw files, the file-format layer: bytes in, the sensor data out as a neutral `RawImage` with
what the file says about rendering it, and back for DNG. No `unsafe`, no panics on malformed input,
no I/O (`&[u8]` in), and it builds for `wasm32-unknown-unknown`.

```toml
craft-raw = { git = "https://github.com/storytold/craft-libs", rev = "<commit>" }
```

```rust
match craft_raw::probe(&bytes) {
    Some(format) if format.is_supported() => {
        let raw = craft_raw::decode(&bytes)?;      // or decode_with(&bytes, &limits)
        // raw.data: the samples as stored; raw.cfa, raw.black, raw.white, raw.active_area, raw.crop,
        // raw.orientation, raw.color (matrices, as-shot white), raw.opcodes, raw.camera
    }
    Some(_) => { /* a raw this build does not decode: show craft_raw::embedded_preview(&bytes) */ }
    None => { /* not a raw file */ }
}
```

It is the first of the three crates planned in craft-libs#587 (`craft-raw`, `craft-develop-model`,
`craft-develop`) and part of the move of all format code into craft-libs (#778). The base is
LightCraft's `lightcraft-raw`; what PhotoCraft's `photocraft-raw` and FilmCraft's `filmcraft-dng`
did that it did not was ported, each with a test (see [Ported behaviour](#ported-behaviour)).

## Where the crate stops

The format layer ends where arithmetic on the picture begins.

| In this crate | In the develop pipeline (not here) |
|---|---|
| Recognising containers (`probe`), walking TIFF/IFD structure on `craft-tiff` | Black subtraction, white scaling, cropping to the active area |
| Decompressing the samples: unpacking, lossless JPEG, lossy JPEG, Deflate, JPEG XL | Demosaicing, binned previews, hot-pixel and highlight handling |
| A DNG `LinearizationTable` (a storage code, applied so the samples are sensor values) | Raising samples stuck below the stated white level (`lift_clipped_samples`) |
| Black and white levels, CFA layout, active area, default crop, orientation, pixel aspect | The DNG colour model's arithmetic (illuminant interpolation, camera to working space) |
| Colour tags as data: matrices, calibration illuminants, as-shot neutral / white | White balance, Kelvin scales, spectral or fitted camera colour |
| Opcode lists parsed and written (`opcodes`), profile tables and gain table maps as data | Applying opcodes (gain maps, warps, bad pixels), profile look tables, tone curves |
| Embedded previews and semantic masks, as stored | Rendering, resampling, lens corrections, tone, colour, masks, GPU kernels |
| Writing DNG from a `RawImage` (uncompressed, lossless JPEG, Deflate) | Lossy DNG and rendered DNG export (they start from a developed picture and call this writer) |

`RawImage` carries no Exif/XMP/IPTC model. `Camera` holds the make, the model and DNG's
`UniqueCameraModel` (decoders and colour lookups need them) and, for a container without an Exif
block (CRW), the capture time. A metadata reader reads the rest from the same bytes (#783).

## Formats

One module and one Cargo feature per maker or container family. All are on by default.

| Feature | Format | Status |
|---|---|---|
| `dng` | Adobe DNG | Uncompressed at 1 to 16 bits (packed MSB-first, or 8/16-bit in file byte order), lossless JPEG (all seven predictors, 1 to 4 components, point transform, restart intervals), lossy JPEG (compression 34892, 8-bit baseline tiles or strips), Deflate with predictors 1, 2 and the floating-point predictors (16, 24 and 32-bit floats), strips and tiles, planar or interleaved, CFA of any period up to 16 × 16 (Bayer, X-Trans) and LinearRaw with 1, 3 or 4 samples, DNG 1.7 row and column interleave, reduced-resolution LinearRaw copies (`decode_reduced`), semantic masks, CinemaDNG frame tags. Writing: uncompressed, lossless JPEG tiles, Deflate tiles (integer, half and single floats) |
| `jxl` | JPEG XL DNG (DNG 1.7, compression 52546) | Lossless and lossy tiles through `jxl-oxide`; implies `dng` |
| `crw` | Canon CRW (CIFF) | Decoded by [`craft-crw`](../crw/README.md), the one CRW decoder, and returned through this crate's types |
| `cr2` | Canon CR2 | The lossless-JPEG frame of IFD 3 de-sliced (`0xc640`); the layout from `CR2CFAPattern` (`0xc5e0`); image area from `SensorInfo`; black measured on the masked columns; white from the saturation plateau; as-shot white balance from `ColorData` or `WBInfo`; the white-balance presets at their temperatures (`ColorData::kelvin_presets`). sRAW / mRAW (subsampled YCbCr, 4:2:2 and 4:2:0) come out as linear white-balanced RGB with three samples per pixel, the chroma gains measured against the file's own 16-bit RGB image in IFD 2 |
| `nef` | Nikon NEF / NRW | Uncompressed (16-bit words, 12-bit packed, Coolpix word packing), the packed layouts labelled compression 34713, Nikon's Huffman coding (lossless and lossy types, 12 and 14 bits, "lossy after split") with the `0x0096` linearisation table, small raw (YCbCr 4:2:2) as linear white-balanced RGB; black from `0x003d` or the NRW block, as-shot white balance from `0x000c`, `CropArea` or the measured image area of bodies that record none, masked trailing columns left out of the active area, the D1X pixel aspect, and the in-file distortion correction as a `WarpRectilinear` opcode ([derivation](docs/nikon-lens-corrections.md)). High Efficiency (HE / HE★) is refused by name; the previews still read |
| `mrw` | Minolta / Konica Minolta MRW | The `\0MRM` block container (`PRD`, `WBG`, `TTW`), 12-bit packed and 16-bit word mosaics, as-shot white balance from `WBG`, white from the saturation plateau |
| `erf` | Epson ERF (R-D1, R-D1s, R-D1x) | 12-bit samples in 16-byte blocks, black levels from the maker note, the measured picture origin. No white balance (the file states none) |
| `mef` | Mamiya MEF (ZD) | Uncompressed 12-bit packed; default crop from the RGB preview's measured window. No white balance (the file states none) |
| `kodak` | Kodak DCS520C TIFF | Two-component lossless JPEG CFA with the `GrayResponseCurve`, white balance from the `KodakIFD` |
| `pef` | Pentax PEF | Uncompressed (packed 12-bit or 16-bit words, also under the PackBits tag some bodies write) and Pentax's Huffman coding (compression 65535, the table from the maker note); image area from the maker note's crop tags, else the dark borders trimmed from the samples; black from `BlackPoint` or the masked columns; as-shot white balance from the maker note; the layout from the Exif `CFAPattern` |
| `srw` | Samsung SRW | The uncompressed variants: 16-bit words (compression 32769) and 12 bits packed (32770 with exactly the packed size; the bit order is decided from the samples); levels and white balance from the keyed maker-note vectors; framing from `SensorAreas`. The compressed codings (32770 with fewer bytes, 32772, 32773) are reported as unsupported |
| `hasselblad` | Hasselblad 3FR and FFF, and other CFA TIFFs in the DNG layout without a DNG version tag (`RawFormat::CfaTiff`) | Uncompressed and Hasselblad's lossless JPEG (predictor selection value 8); RGGB from the sensor origin when the file states no `CFAPattern`; an FFF's colour matrix from the 3FR header it embeds; the 8-bit RGB preview handed on as a baseline TIFF. Implies `dng` |

Not decoded: GoPro VC-5 DNG (GPR), CFA with more than three colours, non-rectangular CFA layouts.
`probe` names every container it can tell apart whatever the features (CR3, NEF, NRW, ARW,
RAF, ORF, RW2, X3F, other TIFF-based raws);
their decoders are still in the apps and move here one maker family at a time.
`RawFormat::is_supported` says what this build decodes.

## Limits

`Limits` (`#[non_exhaustive]`; start from `Limits::default()`) caps what a file may make the
decoder allocate. Each has a test.

| Limit | Default | Checked |
|---|---|---|
| `max_samples` | 2³⁰ (the crate's own ceiling, `MAX_SAMPLES`) | against `width × height × samples per pixel` before any allocation, in the header probe too |
| `max_lossy_tile_samples` | 2²⁶ | against a lossy tile's own JPEG header before it is decoded |

Independent of the limits, sizes are checked against the bytes present: an image must have a chunk
for every tile or strip, its chunks may not be much larger than the image, and no coding may claim
more than 2048 samples per stored byte, so a small file cannot ask for a large buffer. CRW keeps
`craft-crw`'s own cap of 2²³ samples (or the caller's, when lower). Semantic masks are capped at 16
masks of 2²⁵ samples; opcode lists at 4096 opcodes.

## Ported behaviour

Where the three readers disagreed on the same file, the reading that the specification or two of
the three supported was kept:

| From | What | Before (LightCraft's reader) |
|---|---|---|
| PhotoCraft | `DefaultCropSize` without `DefaultCropOrigin` crops from the active area's corner: each tag has its own default (DNG 1.7.1) | both tags were required; a size alone was ignored |
| PhotoCraft | A default crop that is empty, starts outside the area or is not a pixel count is the whole active area | an empty rectangle stayed in `crop` |
| PhotoCraft | A non-finite `WhiteLevel` (a DOUBLE past `f32::MAX`) is ignored | the white level became infinite |
| PhotoCraft | Without `WhiteLevel`, a linearized image's white is the table's largest output (DNG chapter 5 linearizes before it rescales) | the stored depth's maximum, 255 for an 8-bit file whose table expands to 16 bits |
| PhotoCraft | `AsShotWhiteXY` must lie inside (0, 1) | any positive pair |
| PhotoCraft, FilmCraft | Lossy JPEG DNG is 8 bits per sample; another depth is `Unsupported` | 8-bit codes were read as the declared depth |
| PhotoCraft, FilmCraft | A lossy tile's JPEG must cover the part of its tile inside the image | a smaller JPEG left the rest of the tile black |
| FilmCraft | CinemaDNG `FrameRate`, `TimeCodes` and `ReelName` (`RawImage::cinema`), read and written | not read |
| craft-crw | A big-endian (`MM`) CIFF header is a CRW | `probe` knew only `II` |
| PhotoCraft | Hasselblad: four `BlackLevel` values without `BlackLevelRepeatDim` are one per site of the 2 × 2 CFA (files that are not DNGs only) | the first value was used for every site |

Left as LightCraft has it, with the reason:

- An `ActiveArea` that reaches outside the image is ignored (the whole image is the area).
  PhotoCraft clips it to the image instead. The specification requires the area to be inside the
  image and says nothing about a file that breaks that; no real file settles it.
- An edge tile's lossless or lossy JPEG smaller than the nominal tile is accepted when it covers
  the rows inside the image. PhotoCraft and FilmCraft refuse any size mismatch; LightCraft's
  tolerance is for writers that encode edge tiles at their visible size.
- A Hasselblad-layout file without `AsShotNeutral` is not claimed (`OtherTiff`, preview only):
  nothing in it says how to read its colour. PhotoCraft decodes any Hasselblad CFA IFD and measures
  a missing white level from the samples, which is develop-side work. PhotoCraft refuses the
  lossless-JPEG files; LightCraft's selection-value-8 decoder reads them. Both measured the same
  CFA phase (red at the sensor origin) on different bodies (H4D-40, X2D II 100C).
- Epson ERF and Mamiya MEF record no as-shot white balance in any tag found. LightCraft's decoder
  estimated one from the embedded preview's neutral pixels; that is an estimate from picture
  content, so it stays with the developer and `wb_multipliers` is `None` here. The inputs it needs
  (samples, crop, CFA, levels, preview) are all in `RawImage` and `embedded_preview`.
- Pentax PEF and Samsung SRW are LightCraft's readers unchanged. PhotoCraft has no reader for either: it sends
  them down its generic TIFF/EP path, which reads an uncompressed CFA IFD and refuses Pentax's Huffman coding
  and Samsung's private compressions, so there was nothing of PhotoCraft's to fold in.
- Canon CR2 is LightCraft's reader. Two things PhotoCraft's reader does differently were not taken over:
  - Colour filter phase. PhotoCraft measures the row phase from the green diagonal of the samples, with a table of
    five model IDs for flat frames, because it found no tag for it. LightCraft reads `CR2CFAPattern` (`0xc5e0`)
    from the raw IFD, which states the layout per file, and falls back to RGGB without it; both end at RGGB when
    nothing else is known. The tag is kept: it needs no samples, so `probe_info` and `decode` agree.
  - sRAW / mRAW. PhotoCraft multiplies the stored RGB by a gain set from `ColorData` (word `0x80`, version 15 only,
    observed on one body) to get camera RGB and then applies the as-shot white balance. LightCraft leaves the
    camera's white balance in (`wb_multipliers` is `[1, 1, 1]`) and measures the chroma scale per file against the
    IFD 2 image, on 31 files of 16 models including the 2008 to 2010 bodies whose chroma scale differs. That reading
    is kept; a developer that wants camera RGB for version 15 can divide by the same word.
- Nikon NEF / NRW is LightCraft's reader with two things from PhotoCraft's:
  - The maker note's compression mode (`0x0093`, or the `u16` at byte 10 of `0x0051`) is read first. 6 and 10
    send the strip to the packed reader even when a `0x0096` table is present, and allow 14-bit rows without
    padding; 13 and 14 are High Efficiency. LightCraft chose the packed reader only when there was no table and
    recognised High Efficiency by the JPEG XS markers; both rules remain for files without the mode.
  - A 12-bit Nikon D4 file takes the measured 14-bit saturation level shifted like its samples (3880).
    LightCraft measured it from the data.
  LightCraft's built-in white-balance preset table for bodies that record none is not file data and stays with
  the developer. `CropArea`, the D90 / D5100 / D800E areas and the 14-bit-unit black level are the same in both.
- Lossy JPEG is accepted for CFA data (one grey component) as in PhotoCraft; FilmCraft refuses it
  because DNG 1.4 allows the coding for LinearRaw only.

## Sources (clean room)

Written only from public specifications and from observation of files:

- TIFF 6.0, TIFF/EP (ISO 12234-2), TIFF Technical Note 1 (SubIFDs).
- Adobe DNG Specification 1.4 to 1.7.1; Adobe's TIFF technical notes for Deflate and the
  floating-point predictor; the CinemaDNG Image Data Format Specification 1.0 and SMPTE ST 12-1
  (time code packing).
- ITU-T T.81 (ISO/IEC 10918-1) Annex H: lossless JPEG.
- ExifTool's tag-name documentation (prose), for maker-note and private tag numbers.
- Canon CRW: see `crates/crw/clean-room/`.

No code from dcraw, LibRaw, rawspeed, rawloader/rawler, dnglab, darktable, RawTherapee, ExifTool's
Perl or Adobe's SDKs was read or used, and no camera colour tables or profiles are shipped. The
code was moved from the Craft apps (see `NOTICE`, *Source provenance*); nothing was added from
another project to fill a gap. The derivation records of the maker formats that are not here yet
(LightCraft's `docs/arw6-compression.md`, `docs/raf-compression.md`, `docs/orf-compression.md`,
`docs/cr3.md` and their logs) move with those formats.

## Dependencies

`craft-tiff` (IFD parsing and writing), `craft-crw` (feature `crw`), `miniz_oxide` (Deflate),
`zune-jpeg` (the 8-bit baseline JPEG of lossy DNG tiles: `craft-jpeg` covers the JPEG processes
baseline decoders lack and sends baseline frames to one), `jxl-oxide` (feature `jxl`), `rayon`,
`serde`, `thiserror`, `chrono` (feature `crw`: the plausibility of a capture time).

## Tests

No sample files are committed. Every storage variant is written by the crate's own DNG writer or by
hand with `craft-tiff`'s `TiffWriter` and the lossless-JPEG encoder (`ljpeg::encode`), and must
decode bit-exactly; lossy tiles are made with `jpeg-encoder`, JPEG XL tiles with `zune-jpegxl`
(tests only). Hostile input: every truncation of each sample, byte mutations across the
header, 400 random mutations per property, random TIFF-like, lossless-JPEG, opcode-list and CIFF
bytes, and a test per limit.
