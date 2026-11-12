# Fujifilm compressed RAF

The derivation record of `src/rafc.rs`, moved from LightCraft (`docs/raf-compression.md`) with the decoder. "Supplied" files were supplied to LightCraft and are not public; the CC0 ones are pinned in [raf-corpus.sha256](raf-corpus.sha256) (paths as in LightCraft's corpus).

## Format and implementation

`src/raf.rs` reads the RAF header, raw TIFF IFD, crop, CFA, black level and white balance. The big-endian compression field at byte 108 distinguishes uncompressed (0), lossless compressed (2) and lossy compressed (3); testing only the strip's first two bytes would misclassify an uncompressed 16-bit sample of `0x5349`.

`src/rafc.rs` handles the compression stream:

- A 16-byte `IS` header carries compression mode, sensor type (Bayer or X-Trans), bit depth, dimensions, padded width, 768-pixel stripe width, stripe count and six-row group count. It must agree with the TIFF dimensions and CFA.
- A big-endian size table, padded to 16 bytes, locates independent entropy streams. Lossy files add a table of one quantizer byte per six-row group for each stripe, each table padded to 16 bytes.
- Each six-row group is collated into three red, six green and three blue vectors. Bayer vectors have 384 samples; X-Trans vectors have 512 slots, including interpolation slots that consume no bits. The last stripe's padding is decoded for predictor state but excluded from the output.
- Colour-vector pairs alternate R/G and G/B. Even columns lead odd columns by seven positions. Two preceding lines of each colour and the preceding line's edge values provide prediction context.
- Signed prediction errors use adaptive Rice coding: a unary prefix followed by a context-dependent number of bits, with a bounded escape for large errors. Three pair phases have separate even/odd statistics. The 81 signed gradient contexts fold into 41 states.
- Lossy quantizers change the error range and reconstruction step. Low-gradient sites retain finer precision (0, 1 or 2), with five folded contexts per precision and statistics that survive main-quantizer changes. The main context statistics reset when its quantizer changes. Their initial sum is `max(2, (range + 32) / 64)`; using 16 as the minimum breaks more heavily quantized files.

Stripes decode in parallel using the existing Rayon dependency. The entropy reader keeps up to 64 bits buffered and counts a unary prefix's zeros in one step instead of bit by bit; a property test checks it against reading bit by bit, errors included. Offsets, sizes, dimensions and entropy reads are checked; truncated data, impossible codes and inconsistent headers return errors. Unknown container compression modes, disagreements between container and stream modes, and compressed X-Trans layouts other than the supported CFA return errors. Header-only probing validates the tables and stripe ranges without decompressing the image.

## Verification

Complete sensor arrays were compared sample by sample with an external decoder used only as a black-box oracle (rawpy 0.27.1). No decoder source, Adobe data, camera matrices or profiles were used. This tool is not a product or test dependency.

| Coverage | Files |
|---|---|
| Supplied X-H2S | DSCF4192 uncompressed; DSCF4193 lossless; DSCF4194 lossy |
| Supplied X-T4 | _DSF2355 / _DSF2356 uncompressed; _DSF2357 lossless; _DSF2358 lossy |
| CC0 X-Trans | X-T2, X-T20, X-T4, X-H2, X-T5, X-M5, X-T50, X-E5 |
| CC0 Bayer | GFX 50S (14-bit), GFX 100 (14/16-bit), GFX100S (16-bit lossless/lossy), GFX100RF (16-bit lossy) |

All 24 files matched the reference sensor arrays exactly. This covers 13 camera bodies and both older and current compression variants. Synthetic tests also cover 12-bit streams; a real compressed 12-bit camera file has not been verified. Support is selected from the file's layout, without a camera-name allowlist. Other bodies using this layout should decode, but this is not a claim that every Fujifilm model has been tested. Older FinePix layouts without the supported raw TIFF IFD remain unsupported.

The 17 CC0 files live in the gitignored corpus. Their published SHA-256 identities are pinned in [raf-corpus.sha256](raf-corpus.sha256); their source licence records were checked as CC0 1.0. `cargo xtask corpus --download` fetches them. `cargo test -p lightcraft-raw corpus_fujifilm_compressed_samples -- --nocapture` verifies each available file's checksum before checking its reference sensor sum and position-weighted sum. The test skips absent corpus files; the full-array comparisons were performed locally. No supplied media or large binary fixtures are committed.

Unconditional procedural tests cover Bayer/X-Trans, 12/14/16 bits, partial final stripes, quantizer changes, residual signs and escapes, malformed headers and truncated/random entropy streams. Regressions also check overflowing uncompressed row strides, unsupported modes and CFA layouts, and container/stream mode mismatches.

Local release decode measurements: about 0.16–0.24 seconds for 24–40 MP X-Trans files, 0.29 seconds for 50 MP Bayer, and 0.5–0.6 seconds for 100 MP Bayer. These measure sensor decoding, not the full develop/export pipeline. The buffered prefix count took 13–17 % off the decode time on an Apple M2 (8 cores, minimum of 5 runs, old and new build alternated): X-T2 178 → 152 ms, X-S10 lossless 265 → 226 ms, X-E5 442 → 385 ms, X-H2 448 → 378 ms, GFX 50S 522 → 435 ms, GFX 100 762 → 644 ms, with sensor arrays unchanged bit for bit.

## Sources

- [Fabian's original prose description of lossless RAF compression](https://capnfabs.net/posts/fuji-raf-compression-algorithm/): independent stripes, colour vectors, iteration order, predictors' neighbourhoods, Rice coding and gradient adaptation. Its linked implementations were not consulted.
- [ExifTool Fujifilm tag-name documentation](https://exiftool.sourceforge.net/TagNames/FujiFilm.html): RAF header and raw-IFD metadata fields.
- [Fujifilm's compression-mode explanation](https://digitalcamera-support-en.fujifilm.com/digitalcameraengpcdetail?aid=000008311&wd=compressed+raw).
- [raw.pixls.us](https://raw.pixls.us/): CC0 camera samples, published checksums and licence metadata.
- [rawpy's documented `raw_image` API](https://letmaik.github.io/rawpy/api/rawpy.RawPy.html): the complete sensor array includes margins, allowing comparisons before demosaic or colour processing.

Exact byte layouts, colour-vector masks, prediction formulas, adaptation parameters and lossy precision selection were established through sample analysis and black-box comparisons.
