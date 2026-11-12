# Sony ARW6 (Compressed RAW 2)

The derivation record of this crate's ARW6 decoder, moved from LightCraft (`docs/arw6-compression.md`) with the code. It decodes the raw sensor data of Sony's *Compressed* and *Compressed (HQ)* ARW files (TIFF Compression 32766, SonyRawFileType 6, "ARW 6.0") written by the A7R VI (ILCE-7RM6), in pure Rust. The ILCE-7M5 (A7 V) writes the same codec; no sample of it was available, so it is unverified.

Decode timings: see the end of this document (measured in the performance pass).

## Format and implementation

The codec is Sony's use of SMPTE RDD 34 (LLVC, Low Latency Video Codec). `src/llvc.rs` is the codec core (bit reader, variable-length decoder, dequantiser, inverse 5/3 wavelet); `src/arw6.rs` is the Sony container, geometry and colour reconstruction; `src/arw6_curve.rs` is the companding curve; `arw.rs` dispatches Compression 32766 to it. A test-only encoder (`arw6_testenc.rs`) builds synthetic tiles for round-trip tests.

### Container

- Raw SubIFD: Compression 32766, 14 bits per sample, BlackLevel 512 x4 (`0x7310`), WB `0x7313`, crop `0x74c7/0x74c8` (full frame: origin 12,8, size 9984x6656; APS-C: origin 16,10, size 6528x4352), CFA RGGB. The Sony tag-based white level (15360) is not read; the DNG `WhiteLevel` is (16383).
- The single strip starts with a tile table: `u32` count, `u32` 0, then per tile a `u64` offset (relative to the strip) and `u32` x, y, width, height. Seen: full frame 4 tiles of 5008x3336 (HQ) or 5024 / 4992 wide (Compressed); APS-C (6592x4372) one tile. Bytes up to offset 512 follow the table (480 on APS-C, 408 on full frame; not interpreted, not needed).
- A tile is a sequence of 16-byte words. Word 0 is RDD 34 `Picture_info` (HS = tile width, VS = tile height / 2, BBD = 16, NC = 3, NW = 3, NP = `1000000000b`; other values are refused as unsupported). Word 1 holds five `u24` group byte-sums, word 2 is zero, words 3-7 describe the groups (a count byte, then `u24` stream sizes in words). Groups: g0 = LL3 (3 streams, one per component), g1 / g2 / g3 = level-3 / 2 / 1 bands (3 streams each), g4 = green residual (1 stream). Streams follow in that order from word 8.
- A stream is a header word (a `u16` count of index words, a `u24` count of data words, then 11 bytes that are not interpreted), an index table and data, each padded to a word. The index has one entry per transform unit (TU): `A(16) X B(16) X`, where A and B are the byte lengths of the TU's two halves and X are quantiser-index (QI) nibbles (one for g0 and g4, three for g1-g3: HL, LH, HH). Halves per tile = ceil((VS - 2 - s) / 8) + 1 with s the first-half shift (below), TUs = ceil(halves / 2): 105 and 137 for the sample sizes. Halves are numbered n = 2 * TU + h; each is byte-aligned and consecutive halves are contiguous.
- QI values seen, identical in Compressed and HQ files: level 1 component 0 (1,1,2), components 1-2 (2,2,3); level 2 (0,0,1); level 3 (0,0,0); LL3 0; residual 2. Compressed and HQ differ only in the coefficient data.

### Entropy coding and dequantisation

- Each line of a band is an RDD 34 section 6.3 coefficient-set stream: ceil(width / 4) sets of four coefficients driven by the DPT / ABS / SGN state machine, with zero runs and the "0^lim" end code. Values past the width are discarded. Lines of a half follow one another bit-contiguously; the state resets per line.
- Two deviations from the RDD 34 text were established by black-box comparison: in state DPTs = 0 with DPT = 0, a "0" code is a zero set that also enters DPTs = 1; and a run code of value V means V - 1 further zero sets before the post-run "0^n 1" depth code.
- Dequantisation is `recon(q) = sign(q) * (((2|q| + 1) << (QI - 1)) - (|q| & 1))`, `recon(0) = 0`, and QI = 0 leaves q unchanged. This replaces RDD 34's plain shift.
- LL3 lines are DPCM: a running sum along the line. The 2048 base (codes are unsigned 12-bit) applies to component 0 (the green mean) only; the chroma planes start at 0.
- Check: every half's lines end inside its last byte. The Phase 0 measurement found budget - 8 < consumed <= budget on every half of all 8 files; the decoder rejects a half that does not.

### Frame geometry

- Each tile holds three W/2 x H/2 planes (W, H = tile size): component 0 = M, the green mean; components 1-2 = chroma c1, c2; plus a W/2 x H/2 residual plane from g4. A three-level 5/3 wavelet is inverted (vertical, then horizontal) with whole-sample symmetric extension at every edge and integer arithmetic. Tiles are independent.
- The TU grid and the lifting phases are anchored to the full sensor, not to the tile. Let s = (lines in g4's first half) - 2, read from the stream (residual lines are decoded while at least 8 bits of the half remain, at most 8): 0 on full frame, 3 on the APS-C crop (the crop's sensor-row offset 573 is 5 mod 8). Half n then covers plane rows 8n - 6 + s .. 8n + 1 + s, clipped to the frame. The lifting phases follow from s: full frame (0,1,1), APS-C (1,0,0). Each half holds, in clipped natural order, its rows of HL1, LH1, HH1, HL2, LH2, HH2, LL3, HL3, LH3, HH3 and, in g4, 8 residual rows; edge halves are shorter. With the right s every line of every file is used.

### Colour reconstruction

With plane row j, column x (x +- 1 clamped at the edges), in 12-bit code units:

- `G1[j][x] = M[j][x] - ((res[j-1][x] + res[j-1][x+1] + res[j][x] + res[j][x+1] + 4) >> 3)`, with `res[-1] := res[0]`.
- `G2[j][x] = ((G1[j][x-1] + G1[j][x] + G1[j+1][x-1] + G1[j+1][x]) >> 2) + res[j][x]` (unclipped G1; the last row uses G1[j] for G1[j+1]).
- `R = 2*c1 + ((clamp(G1, 0, 4095) + clamp(G2, 0, 4095)) >> 1)`, `B = 2*c2 + (same)` (a negative green, as in deep high-ISO shadows, counts as 0).
- Every code is clipped to 0..4095. Per 2x2 cell: R (0,0), G1 (0,1), G2 (1,0), B (1,1).

### Output range (companding curve)

Codes map through one monotone curve, the same for every file and both modes. It is not stored in the file (below); it is our own measurement. Output values are in 2 x 14-bit DN: black 2 x 512 = 1024, the 14-bit white 16383 maps to 32766.

- The curve is piecewise linear through knots (c_k, u_k), k = 0..306: u_k = 128 k (one knot per 64 DN of the 14-bit scale); c_k = u_k for k <= 11 (the identity through code 1408); c_k = 356.89 + 655.646 ln(k - 6) for k >= 12 (that is, c = -2824.32 + 655.646 ln(u - 768)). F(code) is the linear interpolation of u over c, rounded half up (floor(x + 0.5)). Values: F(1408) = 1408, F(1536) = 1542, F(2048) = 2457, F(3000) = 7979, F(3500) = 16227, F(3977) = 32767, F(4095) = 39077.
- `arw6_curve.rs` builds the 4096-entry table from the formula at first use; a unit test pins the whole table (its sum and position-weighted sum), so a platform whose `ln` rounds differently fails it.
- The decoder reports black 1024 and white = 2 x the file's DNG `WhiteLevel` (16383), i.e. 32766; a file without a usable white level gets 32766 too (2 x 16383), not a data-derived value. Codes above 3977 sit above white and clip as usual.

#### How it was measured

Each of the four scenes of the CC0 set (APS-C and full frame, landscape and portrait) has a lossless (LJ92) file and a Compressed and an HQ file, shot handheld one after another. Seven Compressed / HQ files share shutter, aperture and ISO with their lossless sibling. The eighth, APSC_land RAW-C (1/500 s against 1/1000 s), was left out: its brightness ratio to the lossless file is 1.37 rather than 2 and drifts with level, so the light changed between the shots.

- Data: the ARW6 12-bit codes from LightCraft's decoder with the curve replaced by the identity, and the lossless 14-bit DN from LightCraft's LJ92 path. Both modes share the sensor origin (their crop tags are equal); the lossless frame is the ARW6 frame padded with zeros at the right and bottom.
- Registration: the handheld shifts are large (median 3 to 102 green cells). Patches were registered locally on the green planes with a curve-independent rank transform, normalised cross-correlation and a neighbour-consistency filter.
- Two estimators per pair and CFA channel, each capture with its own gain and offset (the gains' geometric mean fixed to 1, the offsets' mean to 0):
  - quantile (CDF) matching over all overlapping pixels: for each code, the mean lossless value over the slice of the distribution the code occupies (precise, slightly smoothed by compression noise);
  - block pairs: smooth 8 x 8-cell blocks (16 x 16 sensor pixels) away from clipping, a model linear in F, fitted robustly (unbiased).
- Fitting c = C + A ln(u - B) through the knots gives A 655.88, B 766.9, C -2826.5 on the blocks and A 653.47, B 770.1 on the quantile curves. Holding B at 768 changes the curve by at most 0.08 %; the formula above (A 655.646, C -2824.32) uses it.

Uncertainty: 95 % intervals of a cluster bootstrap over the 7 captures (40 replicates), as a share of F - 1024 and in codes, and the largest deviation of any subset or method fit (Compressed only, HQ only, greens, red + blue, APS-C, full frame, quantile method):

| Codes | Bootstrap 95 % | In codes | Largest subset / method deviation |
|---|---|---|---|
| 1408-1536 | -0.42..+0.35 % | -1.6..+2.0 | 0.75 % |
| 1536-2048 | -0.65..+0.58 % | -3.0..+2.8 | 0.80 % |
| 2048-2560 | -0.55..+0.57 % | -3.1..+3.3 | 0.78 % |
| 2560-3072 | -0.59..+0.50 % | -3.1..+3.7 | 0.64 % |
| 3072-3584 | -0.76..+0.54 % | -3.5..+4.9 | 0.77 % (quantile) |
| 3584-3840 | -0.84..+0.57 % | -3.7..+5.5 | 0.90 % (quantile) |
| 3840-3977 | -0.88..+0.59 % | -3.8..+5.8 | 0.97 % (quantile) |
| 3977-4095 | -0.91..+0.62 % | -4.0..+5.9 | 1.03 %, extrapolated |

The lossless files clip at the 14-bit white, so codes above about 3977 rest on the formula alone. The camera's exact integer table can't be recovered: integer knot codes, the encoder's rounding and a few codes around the knee are below the measurement's resolution.

#### Evidence for the structure

- Knots every 128 u: the measured slope dF/dc is flat within segments and jumps at codes 1531, 1632, 1719, 1796, 1865, 1928, 1984, 2037, 2085…, where u = 1536, 1664, 1792…. On codes 1380-2200 a spacing of 128 leaves 0.021 RMS relative slope error; the next best, 126, 64 and a smooth log, leave 0.037, 0.041 and 0.068. Above code 2200 the segments are too short to resolve, but there the piecewise-linear and smooth forms differ by less than 1.
- Log spacing: the measured knot codes follow c = C + A ln(u - B) with chi-squared per degree of freedom 0.90 (bootstrap 95 %: A 654.3-658.2, B 762.8-771.1).
- The knee: the log branch at u = 1408 gives code 1413 (95 % 1410-1415), so the identity runs through knot 11 and the log knots start at k = 12 (segment slope 1.035 between them). Starting them at k = 11 fits as well but would need a slope below 1 right after the identity; a smooth log leaving the identity near code 1340 fits the blocks clearly worse (cost 25.4 against 22.9).
- One curve for everything: Compressed and HQ, all four CFA channels, APS-C and full frame agree within 0.8 %, and the ratio is flat with frame radius to +-0.5 % (no gain map).

#### Scale

The x2 unit is measured, not assumed:

- Below the knee F(c) = c: the measured slope is 1.000 +- 0.01 for codes 1150-1408, with offsets within +-2.
- The overall level is fixed by the captures' average exposure to +-0.4 % (per-capture gains 0.99-1.02). Independently, flat clipped areas of the Compressed files sit at code 3977 +- 1 in all four scenes, and the formula, not fitted to that, puts the 14-bit white (32766) at code 3977.0 (bootstrap 95 % 3973-3983).
- HQ files carry data above the 14-bit clip: their clipped highlights sit at codes 3996-4095 depending on the scene (about 16.8k-19.5k DN), so the curve keeps its values above 32766. The reported white level is unaffected.

#### Not stored in the file

Every IFD of all 12 files was dumped and compared across modes:

- Raw SubIFD: `0x7010` (SonyToneCurve) and `0x7011` are zero. The monotone arrays `0x7032`, `0x7035` and `0x7037` are lens corrections, the same in every mode. The only mode-dependent tag is `0x7000` SonyRawFileType (4 against 6).
- Maker note (`0x9050`, `0x94xx`, deciphered): the Compressed and HQ bytes differ from the lossless ones only in 1-2 bytes (the quality setting), and hold no curve-shaped monotone run.
- SR2SubIFD (encrypted, 42478 bytes): every file uses the same keystream, so XOR-ing two files compares their plaintexts. The lossless and HQ files of a scene differ in only 34 bytes, so there is no ARW6-only region. A curve stored identically in every mode can't be ruled out, but none is known there. `0x7250` likewise has no mode-specific region.
- The bytes after the tile table (480 on APS-C, 408 on full frame; the last 120 high-entropy) are identical in the Compressed and HQ files of a scene, even at different exposure, and differ between scenes: not a curve.

#### Verification against the oracle

After the derivation was frozen, the derived table was compared with the black-box oracle's output curve (below), as a check only, with no tuning. They are identical through code 1422 (both are the identity through 1408), then within -1.0 % to +0.25 % of (value - black); the largest difference is 75 (0.2 %) at code 4093. Ours minus the oracle's: -5..+1 for codes 1408-1536, -13..-2 for 1536-2048, -33..-12 for 2048-3072, -34..+40 for 3072-3977, +1..+75 for 3977-4095.

## Verification

Method: LibRaw master (commit `d1f0dd95`, 2026-10-08, run as `unprocessed_raw`, 16-bit output before any processing) was used only as a black-box oracle: it was built and run, and its source was never read. A Python reference model of the format (the Phase 0 reference model, local, not in the repo) was written from RDD 34 and from the oracle's behaviour on real and synthetic input, then reproduced the oracle exactly on every tile of the 8 Compressed / HQ files below. A test-only encoder, validated byte for byte against real tiles, probed the oracle with synthetic coefficients (dequantiser, phases, edge mapping). The Rust decoder reproduced the Phase 0 reference sums on all 8 files. Its output then went through a table read off the oracle's output; that table was strictly increasing, so these exact matches prove the 12-bit codes before the curve identical to the oracle's. The table has since been replaced by our own measured curve (*Output range*), so the output values now differ from the oracle's by the amounts in *Verification against the oracle*.

Geometry was checked against each file's embedded full-resolution JPEG at the DefaultCropOrigin: our output aligns at offset 0 on full frame and APS-C.

### LibRaw's APS-C bug

LibRaw assumes the full-frame grid (s = 0) for every tile. On the APS-C crop that drops the first three plane rows, leaves the last three unfed, and shifts its whole output up by 6 sensor rows; its first rows and last rows are also wrong. Our decoder differs from LibRaw there on purpose. To compare against LibRaw on APS-C files, our row y+6 equals LibRaw's row y for 12 <= y < h - 24 (LibRaw's rows 0-11 and the last 18 rows of the shifted overlap differ); on full frame compare directly: the 12-bit codes match over the whole frame.

### Corpus

| File | Source path (RawDB, ILCE-7RM6) | Size (bytes) | SHA-256 | Fetched by |
|---|---|---|---|---|
| `arw6-sony-ilce7rm6-apsc-land-hq.arw` | `arw6/ILCE-7RM6_APSC_land_RAW-C-HD.ARW` | 29 720 576 | `b60a7e752d2a8102ccbe6eebcfdbf533c6b0f2545739090873e9e8eb32cc0637` | `cargo xtask corpus --download` |
| `arw6-sony-ilce7rm6-apsc-land-c.arw` | `arw6/ILCE-7RM6_APSC_land_RAW-C.ARW` | 30 900 224 | `1c803304e63ec508b2b5f9d59d4a7e30ed279315b35fa4c5d75f959bed62198b` | `curl -L` |
| `arw6-sony-ilce7rm6-apsc-port-hq.arw` | `arw6/ILCE-7RM6_APSC_port_RAW-C-HD.ARW` | 28 758 016 | `0b6d782571e86868670f613de9fa1f1364623b24fbfd21b704b2c516d7a1c1cb` | `curl -L` |
| `arw6-sony-ilce7rm6-apsc-port-c.arw` | `arw6/ILCE-7RM6_APSC_port_RAW-C.ARW` | 29 044 736 | `c41674a485ffcf6d873e20e545ae62936192d94e95be3d53548e93bc543ecfd4` | `curl -L` |
| `arw6-sony-ilce7rm6-ff-land-hq.arw` | `arw6/ILCE-7RM6_FF_land_RAW-C-HD.ARW` | 63 451 136 | `559fa26501d7813e56afecde2aea3d9965a99228162b194752d986193abb172c` | `curl -L` |
| `arw6-sony-ilce7rm6-ff-land-c.arw` | `arw6/ILCE-7RM6_FF_land_RAW-C.ARW` | 63 844 352 | `d900d6cbe838be864269ad54de4a33fcac7eb3a3d4f91009739f770b8f85a23b` | `cargo xtask corpus --download` |
| `arw6-sony-ilce7rm6-ff-port-hq.arw` | `arw6/ILCE-7RM6_FF_port_RAW-C-HD.ARW` | 62 640 128 | `81c709633cb8f6340e612c3f53bb6ec6b0cccfe5b713c67efb1e977ece9e266d` | `curl -L` |
| `arw6-sony-ilce7rm6-ff-port-c.arw` | `arw6/ILCE-7RM6_FF_port_RAW-C.ARW` | 63 168 512 | `f4448485629c3f82ff1955fd046404350e9a1f09b346038574b89c074d7c50fd` | `curl -L` |
| `arw-sony-ilce7rm6-apsc-land-lossless.arw` | `raw_modes/ILCE-7RM6_APSC_land_RAW.ARW` | 44 167 168 | `7aaf0b3941509d7364a6f03844783155ebc912d246d1c456e5cdb4693ec09fbc` | `curl -L` |
| `arw-sony-ilce7rm6-apsc-port-lossless.arw` | `raw_modes/ILCE-7RM6_APSC_port_RAW.ARW` | 43 692 032 | `0f011933ee96525a39710daa50b59d39702b4ec5934fa0206742e2128c55f366` | `curl -L` |
| `arw-sony-ilce7rm6-ff-land-lossless.arw` | `raw_modes/ILCE-7RM6_FF_land_RAW.ARW` | 100 851 712 | `5cb3c870bc961cf6b079d56c860d0cdf11acd1afd04e5fb8dbe041bcfd2d6201` | `curl -L` |
| `arw-sony-ilce7rm6-ff-port-lossless.arw` | `raw_modes/ILCE-7RM6_FF_port_RAW.ARW` | 100 884 480 | `b1a4026aa6adbaa27973b8b3426d8b4aa0c1cae1845cf5102359327d3b069691` | `curl -L` |

These 12 files are CC0 1.0. RawDB's record of the set, <https://rawdb.dnglab.org/api/sets/Sony/ILCE-7RM6> (the data behind <https://rawdb.dnglab.org/sets/Sony/ILCE-7RM6>, accessed 2026-10-10), says `"license":"CC0 1.0"` for the set (uploaded by abbradar, 2026-08-29) and again for each of the 12 files, next to the SHA-256 values in the table.

They live in the gitignored corpus. `cargo xtask corpus --download` fetches two of them, one per geometry and compression mode (full frame Compressed, APS-C HQ), and checks each against its SHA-256, pinned in `RAW_SAMPLES` (`xtask/src/main.rs`). Fetch the other ten by hand: the server answers 303 to a signed object URL, so `curl -L -o corpus/raw/<file> https://rawdb.dnglab.org/api/download/Sony/ILCE-7RM6/<source path>`, then compare with the SHA-256 above. `cargo test -p lightcraft-raw corpus_sony -- --nocapture` checks the decoder's sensor sum and position-weighted sum of the two pinned files and skips absent files. The sums are regression values from LightCraft's decoder with the derived curve; the 12-bit codes behind them were verified against the oracle in Phase 0.

Nine further full-frame HQ photos from the maintainer's A7R VI were compared with the oracle locally; they are private and never committed or published. On all 9 (ISO 100-12800, incl. two near-black night shots) the 12-bit codes matched the oracle over the full frame (0 mismatching pixels of 66.8 MP each).

Unconditional synthetic tests (encode -> decode round trips for both geometries, the RDD 34 worked example, the dequantiser, hostile and truncated input) run without the corpus.

## Coverage

| Coverage | Status |
|---|---|
| ILCE-7RM6 Compressed and HQ, full frame, landscape and portrait | 4 files, 12-bit codes exact against the oracle; output through our measured curve |
| ILCE-7RM6 Compressed and HQ, APS-C crop, landscape and portrait | 4 files, 12-bit codes exact against the Phase 0 model (interior-exact against LibRaw after the 6-row shift; LibRaw's own edge rows differ) |
| ILCE-7RM6 lossless (existing LJ92 path) | 4 files decode; not in the corpus test |
| Private A7R VI HQ photos | 9 files, 12-bit codes exact over the full frame against the oracle (local) |

## Not covered

- The 12-bit Compressed mode (electronic shutter with continuous drive): no sample exists, so it returns `Unsupported` unless its headers prove identical.
- The ILCE-7M5: same codec, no sample; unverified.
- Tiles whose plane width is not a multiple of 8.
- First-half shifts other than 0 and 3 (other crops / bodies): the geometry is generic and round-trip tested, but no real sample exists.
- Memory: one FF decode peaks at about 896 MB RSS (about 6.7 x the 134 MB output; the lossless path peaks at 249 MB). Follow-ups: dequantise straight into bands, place tiles as they arrive.
- Colour: the ILCE-7RM6 bundled colour profile and Sony's embedded lens corrections are separate work.

## Sources

- SMPTE RDD 34:2015 "LLVC (Low Latency Video Codec) for Network Transfer", <https://pub.smpte.org/latest/rdd34/rdd34-2015.pdf> (accessed 2026-10-09): picture / TU structure, VLD, inverse quantisation, 5/3 lifting with symmetric extension.
- LibRaw master `d1f0dd95`, run only as a black-box oracle (not a product, test or build dependency) for the 12-bit codes; the companding curve was measured without it and compared with it only afterwards.
- The embedded JPEG of each file, for the geometry check.
- Not read, by licence: LibRaw, rawspeed, dnglab, RawTherapee and other ARW6 decoder sources. Exact byte layouts, deviations and geometry were established through sample analysis and black-box comparison; the curve was measured from the CC0 lossless / ARW6 pairs.

## Timings

Raw decode only, 4 CPUs, release build, best of 3 wall-clock (2026-10-10). Method: a small throwaway Rust program linking `lightcraft-raw` reads the file into memory, then times `lightcraft_raw::decode` alone with `std::time::Instant` (no render, no JPEG encode), three decodes in one process; the best is reported. Files were already in the page cache.

| File | Size | Sensor | Decode |
|---|---|---|---|
| ILCE-7RM6 FF, Compressed (HQ) | 63.5 MB | 10016x6672 | 0.45 s |
| ILCE-7RM6 FF, Compressed | 63.8 MB | 10016x6672 | 0.51 s |
| ILCE-7RM6 APS-C, Compressed (HQ) | 29.7 MB | 6592x4372 | 0.35 s |
| ILCE-7RM6 APS-C, Compressed | 30.9 MB | 6592x4372 | 0.34 s |

The 1.0 s budget for the full-frame file is met. Parallelised: rayon over the tiles, over the 13 streams of each tile and over its 3 components, and the colour reconstruction over rows.

2026-10-12 performance pass (output unchanged, bit for bit): the vertical wavelet pass lifts whole rows instead of gathering each column, `0^n 1` codes are counted with one `leading_zeros`, and the colour reconstruction reads rows as slices, in parallel. Same method, best of 5, the previous and the new decoder run alternately on the same machine (4 CPUs, which was busier than on 2026-10-10), best of two rounds:

| File | Before | After |
|---|---|---|
| ILCE-7RM6 FF, Compressed (HQ) | 0.56 s | 0.46 s |
| ILCE-7RM6 FF, Compressed | 0.54 s | 0.38 s |
| ILCE-7RM6 APS-C, Compressed (HQ) | 0.34 s | 0.23 s |
| ILCE-7RM6 APS-C, Compressed | 0.35 s | 0.24 s |

2026-10-13 (output unchanged, bit for bit): the two halves of each stream, which are independent bit streams, decode in parallel; the vertical wavelet pass runs its rows in parallel; the horizontal pass lifts each row in place (scratch space reused per thread), rows in parallel, instead of allocating two vectors per row. Same method, best of 3, the previous and the new decoder run alternately, on another machine (Ryzen 7 9800X3D, a 4-CPU cgroup quota shared with other builds); CPU is the process CPU time of the decode (all threads):

| File | Before | After | CPU before | CPU after |
|---|---|---|---|---|
| ILCE-7RM6 FF, Compressed (HQ) | 0.60 s | 0.48 s | 1.14 s | 0.87 s |
| ILCE-7RM6 FF, Compressed | 0.62 s | 0.48 s | 1.14 s | 0.89 s |
| ILCE-7RM6 APS-C, Compressed (HQ) | 0.34 s | 0.23 s | 0.50 s | 0.40 s |
| ILCE-7RM6 APS-C, Compressed | 0.31 s | 0.24 s | 0.51 s | 0.42 s |
