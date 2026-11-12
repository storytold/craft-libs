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
| `rw2` | Panasonic RW2, Leica RWL, older Panasonic / Leica RAW | Packed blocks (formats 2, 5, 7), formats 4 and 6, format 8 prefix-coded strips and tiles (a sample outside the bit depth makes the strip corrupt), 16-bit words; black per colour from `0x001c`–`0x001e` (plus the 15 counts formats 4 and 6 store above it, 15 when no black is recorded); white from the saturation plateau; sensor borders as the active area and the in-camera aspect crop; mapped-out defect pixels filled; as-shot white balance from `0x0024`–`0x0026`; the in-camera distortion correction (`0x0119`) as a `WarpRectilinear` opcode. Raw formats 1 and 3 and compression 34826 are refused (no samples of them) |
| `raf` | Fujifilm RAF | The RAF header, records and raw IFD; uncompressed Bayer and X-Trans (16-bit words, 12-bit LSB-first, 32-bit word packing); Fujifilm's lossless and lossy striped compression ([derivation](docs/raf-compression.md), [corpus](docs/raf-corpus.sha256)); black, as-shot white balance and, for bodies without colour matrices, the camera's neutral multipliers for Illuminant A and D65 (`ColorData::kelvin_presets`, a neutral pair); orientation, make and model from the embedded JPEG's Exif; the in-file distortion and vignetting corrections as opcodes; the embedded preview, also by range from the header alone (`bounded_preview`, `embedded_preview_from_range`). The pre-2010 FinePix layout without a raw IFD is refused |
| `orf` | Olympus / OM System ORF | Uncompressed: 16-bit words (12-bit values in the top bits shifted down), 12-bit packed in 32-bit words (XZ-2), 12-bit two-field (older compacts), 16-byte blocks of ten 12-bit samples (E-300, E-330, E-500, high-resolution modes); CFA from Exif `CFAPattern`; black, as-shot white balance (relative to `WB_GLevel`) and crop from the maker note's ImageProcessing, or from the main note on bodies without it; the CameraSettings preview. Compressed ORF is refused ([why](docs/orf-compression.md)); its preview still reads |
| `arw` | Sony ARW (and the SR2 / SRF containers) | Uncompressed 16-bit words and packed 12-bit, ARW2 (cRAW, compression 32767), lossless JPEG tiles of 2 × 2 cells (compression 7) and the linear YCbCr of downsized lossless files, ARW6 "Compressed RAW 2" (compression 32766, SMPTE RDD 34 LLVC; [derivation](docs/arw6-compression.md)); the encrypted SR2SubIFD (every key) for black, white balance, crop and the white-balance presets at their temperatures (`ColorData::kelvin_presets`); crop from Sony's crop tags, the DNG-style default crop or the recorded image size; the in-file distortion, shading and lateral CA tables decoded with their provenance (`RawImage::lens_metadata`), and the validated distortion as a `WarpRectilinear` opcode ([7RM4A and others](docs/sony-lens-corrections.md), [7RM6](docs/sony-ilce7rm6-lens-corrections.md)); the Dynamic Range Optimizer flag of the embedded preview (`embedded_preview_dynamic_range_optimized`) |
| `cr3` | Canon CR3 | CRX lossless Bayer and C-RAW (lossy) from the main raw track (`CMP1`), through craft-metadata's CR3 container parser; geometry from `CDI1`/`IAD1` and the maker note's `AspectInfo`; as-shot white balance, black and white from `ColorData` (static or the timed `CTMD` maker note), with the stated white extended to a saturation plateau above it (and BaselineExposure compensated); orientation, make and model from `CMT1`; the full-size JPEG track or the `PRVW` / `THMB` boxes as the embedded preview, and the preview colour space from the Canon maker note ([record](docs/cr3.md)). Unverified marker families are refused |
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
- Panasonic RW2 is LightCraft's reader. PhotoCraft's decodes raw formats 3, 4 and 5 and adds 15 to the
  recorded black for 3 and 4; LightCraft adds the same 15 for formats 4 and 6, measured on about 100 files,
  and uses 15 when the tags record no black (PhotoCraft assumed 0). Format 3 stays refused: no sample of it
  is known, and PhotoCraft's reading of it as format 4 is untested. PhotoCraft took white from the linearity
  limits (`0x000e`–`0x0010`); LightCraft measured those below the samples on extended-ISO shots and uses the
  saturation plateau. The ISO in the private IFD0 tags (`0x0017`, `0x0037`) is metadata, left to the
  metadata reader.
- Fujifilm RAF is LightCraft's reader. PhotoCraft's compressed-RAF decoder is a port of the same `rafc.rs`
  (its own copy and photocraft#9066): a diff shows message wording, its parallel helper, and two overflow guards
  in the adaptive Rice state (`k < 31`, a saturating sum). Neither guard can trigger here: residuals are bounded
  by the sample depth and the count is halved at 64, so the sum stays below 2^23. The Kelvin scale of bodies that
  record only an A / D65 pair is kept as that pair (`KelvinPresets::from_neutral_pair`); LightCraft placed five
  points along it with CIE temperature maths, which stays with the developer.
- Olympus ORF is LightCraft's reader with one thing from PhotoCraft's (uncompressed 16-bit only): the as-shot
  red and blue levels are divided by ImageProcessing `WB_GLevel` (`0x011f`) when it is recorded, not always by
  256. PhotoCraft's `ValidBits` (`0x0611`) and crop handling are covered by LightCraft's own (the main note's
  `0x102c`, and the top-bit test on the samples). Compressed ORF is not decoded: see `docs/orf-compression.md`.
- Sony ARW is LightCraft's reader. PhotoCraft's (uncompressed, ARW2, lossless JPEG; it refuses ARW6) adds
  nothing it lacks: its SR2SubIFD reader knows one measured key (`11 22 33 44`), LightCraft's generator reads every
  key. One difference was kept as LightCraft has it and needs the maintainer's eye: PhotoCraft frames the ILCE-7
  and ILCE-6000 at (12, 12, 6000, 4000) of the 6048 × 4024 data, measured on camera JPEGs and matching the file's
  SR2SubIFD area; LightCraft anchors the recorded image size at the left, (0, 0, 6000, 4000). Both windows hold
  picture (the data's last 26 columns are padding). Changing it would move every saved crop on those bodies in
  LightCraft.
- Exif-only inputs a decoder needs are read from the Exif IFD directly: the 7RM6 lens tables are chosen by
  `LensModel` and `FocalLength`.
- Canon CR3 is LightCraft's reader. PhotoCraft has no CR3 sensor decoder (it shows the JPEG track or `PRVW`, and
  reads `CMT1` orientation), which this crate also does. The CR3 container parser is craft-metadata's `cr3` module,
  the one CR3 parser in this repository.
- `sparse::header_plan` / `preview_plan` say which byte ranges of a file a header probe or preview read needs, so a
  caller on a slow drive can skip the sample data. `docs/coverage/` holds LightCraft's camera-coverage records
  (`coverage.py --check` regenerates `coverage.tsv` from the evidence and Adobe's list).
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
