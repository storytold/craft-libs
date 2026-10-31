# craft-crw

A clean-room Canon CRW (CIFF) raw decoder for the Craft apps: the raw format of Canon's bodies
before the CR2 era (EOS D30, D60, 10D, 300D / Digital Rebel; PowerShot G1 and G6). No `unsafe`,
no panics on malformed input, and it builds for `wasm32-unknown-unknown`. The only dependency is
`thiserror`.

```toml
craft-crw = { git = "https://github.com/storytold/craft-libs", rev = "<commit>" }
```

The container (a CIFF heap file) follows Canon's published CIFF specification and ExifTool's prose
tag documentation. The compression was never documented by Canon: it was **derived** from CC0 and
maintainer-owned sample files, frozen, and verified sample-exact against a reference decoder's
command-line output, treated as a black box. No raw-decoder source was consulted, at any point.
The whole record is in [`clean-room/`](clean-room/README.md); the frozen format is
[`clean-room/02-crw-format.md`](clean-room/02-crw-format.md).

## What it decodes

| Body | Selector(s) seen | Verified | Notes |
|---|---|---|---|
| EOS D30 | 1 | exact (1 CC0 file) | no white-balance record: `wb_multipliers` is `None` |
| EOS D60 | 0, 1, 2 | exact (1 CC0 + 1,561 maintainer files) | sensor clips near 4000 at low ISO (`white`) |
| EOS 10D | 0, 2 | exact (3 CC0 files) | |
| EOS 300D / Digital Rebel | 0, 2 | exact (3 CC0 files) | portrait files carry the rotation (`exif_orientation` 8) |
| PowerShot G1 | 1 | exact (1 CC0 file) | 10-bit (no low-bits plane), `Cfa::Gbrg`, no white balance |
| PowerShot G6 | 0 | exact (1 CC0 file) | no white balance |
| PowerShot G5 | 1 | **refused** | its entropy stream diverges from the model from sample 2056 on (`Error::Unsupported`); apps show the embedded JPEG |
| PowerShot G2, G3, Pro1, Pro70, S30–S70 | | unverified | no CC0 samples exist; they decode through the same model if their records match, with no guarantee |

"Exact" means every sensor sample equals the reference decode (`clean-room/03-verification.md`).
Big-endian (`MM`) files are handled from the specification and tested synthetically; none has
been seen in the wild. CHDK sensor dumps named `.CRW` are a different format and are not CIFF files
(`probe` says no).

## Use

```rust
use craft_crw::{decode, header, probe, Error, Limits};

if craft_crw::probe(&bytes) {
    let img = decode(&bytes, &Limits::default())?;
    let h = &img.header;
    // h.width × h.height samples, h.bits deep, Bayer phase h.cfa at sensor (0, 0);
    // the picture is h.active_area; img.black (per CFA site) and img.white are sensor levels;
    // h.wb_multipliers is the as-shot white balance when the body records one.
}
```

- `probe(bytes) -> bool`: the CIFF signature (`II`/`MM`, `HEAPCCDR` at offset 6).
- `header(bytes, &Limits) -> Result<Header, Error>`: geometry, bit depth, CFA phase, selector,
  make and model, capture time, orientation and white balance, without decoding the samples
  (fast: it walks the heap and reads a few records).
- `decode(bytes, &Limits) -> Result<Image, Error>`: the header plus the samples and the measured
  black and white levels.

### Output

`Image { header, samples, black, white }`:

- `samples`: `width × height` `u16`s, row-major, the whole sensor including the masked borders,
  `bits` (10 or 12) deep, black not subtracted. Rows are in sensor order; apply
  `header.exif_orientation` (1, 3, 6 or 8) when displaying.
- `header.active_area`: the picture (SensorInfo's inclusive crop); the rest is masked. The default
  crop of these bodies is the active area itself.
- `header.cfa`: `Cfa::Rggb` or `Cfa::Gbrg`, the phase at sensor (0, 0); `pattern()` gives the
  2×2 cell as 0 = red, 1 = green, 2 = blue.
- `black`: `BlackLevel { values: [f32; 4] }`, one level per CFA site of the 2×2 cell counted
  from the active area's origin (`at(active, x, y)`), measured on the darkest strips of the masked
  borders. These bodies sit near 128 (the Digital Rebel sample near 261, the G1 near 29).
- `white`: the saturation plateau when the image has one, else the sensor's clip level when at
  least 64 samples reach within 3 % of the full range (the D60, 10D and 300D clip near 4000 at
  ISO 100–400), else the full range (4095 or 1023).
- `header.wb_multipliers`: `Some([r, 1.0, b])` from the ColorBalance record on the D60, 10D and
  300D, the slot chosen by the ShotInfo white-balance setting; `None` on the D30 and PowerShots,
  whose records the derivation did not decipher. Apps fall back to their own estimate.
- `header.capture_unix`: the TimeStamp record, Unix seconds on the camera's local clock, no UTC
  offset. Convert with chrono (`DateTime::from_timestamp`), never by hand.
- `header.make` / `header.model`: the MakeModel record, e.g. `Canon` / `Canon EOS D60`. CRW
  files carry no Exif block, so this is where the camera name comes from.

### Errors and limits

`Error` is `NotCrw` (not a CIFF raw), `Corrupt(String)` (damaged structure or an undecodable
code), `Unsupported(String)` (an unknown decoder table, or the PowerShot G5) or
`LimitExceeded(&str)`. Every offset read from the file is bounds-checked, the heap walk is bounded
by depth (16) and by a per-file record budget (4,096), and `width × height` is checked against
`Limits::max_samples` (default 2²³, about 8.4 million, on every target; real frames hold at most
7.4 million) before anything is allocated. The decoder holds about three sample-sized `u16`
buffers at its peak. `Limits` is `#[non_exhaustive]`: start from `Limits::default()` and set the
fields you change.

A stream that ends early is not an error: the remaining samples keep their predictor values (the
frozen stream-end rule; real files end 1–3 samples short).

### Performance

About 60 ms for a 6.5 MP EOS file and 70 ms for the 7.4 MP G6 in release mode on a 2024 laptop
(`cargo test -p craft-crw --release --test samples -- --nocapture` prints the times).

## Tests

- `cargo test -p craft-crw`: synthetic CIFF files round-tripped through a test-only encoder (the
  model's inverse), the frozen tables' structure (prefix-free, all 162 symbols), hostile inputs
  (every truncation and byte mutation of a valid file, self-referencing heaps), and a proptest
  mutation sweep (`tests/robust.rs`). None needs sample files.
- `tests/samples.rs` pins the eleven CC0 sample files of [`tests/samples.json`](tests/samples.json)
  (raw.pixls.us): each decodes to exactly the samples the derivation verified against the
  reference, recorded as the SHA-256 of the sample buffer, with its levels, white balance and
  metadata. The files (5–11 MB each) are not in the repository:

  ```sh
  python scripts/fetch-crw-samples.py ~/crw-samples        # downloads and verifies by SHA-256
  CRAFT_CRW_SAMPLES=~/crw-samples cargo test -p craft-crw --release --test samples -- --nocapture
  ```

  Unset, the test prints that it skipped; CI sets `CRAFT_CRW_REQUIRE_SAMPLES=1` and runs it on
  Linux (`.github/workflows/crw.yml`; lint and the synthetic tests run on Linux, Windows and
  macOS, plus a wasm32 check).
- `CRAFT_CRW_ORACLE=<dir>` additionally compares against `<name>.crw.tiff` references next to the
  files (the reference decoder's `unprocessed_raw -T` output), the check the derivation used.
  Those references are not distributable and stay outside the repository.

## Origin

The decoder was written in LightCraft as `crates/raw/src/vendor/crw.rs` and `crw_tables.rs`
(LightCraft PR #3021, revision `6ca439b5155820d2befeacd81ec0c6a80b2bd3bc`,
[storytold/lightcraft](https://github.com/storytold/lightcraft); the decoder itself was ported at
`70c755e4`, and the branch's later review commits changed only a date conversion that is not part
of this crate) and moved here with its tests and
its derivation record. The move replaced LightCraft's types with this crate's own (`Error`,
`Limits`, `Cfa`, `Rect`, `BlackLevel`, `Header`, `Image`), exposed the capture time as Unix
seconds instead of converting it in the decoder, and copied LightCraft's shared plateau estimate
(`white_from_data`) as `saturation_plateau`. The algorithm, tables and thresholds are unchanged,
and the sample-file test proves the decode is byte-identical to the verified LightCraft decode.
Colour (camera matrices, profiles, the JPEG-fitted look) stayed in LightCraft.

## Frozen tables

`src/tables.rs` contains frozen code tables from the clean-room derivation record retained in
[LightCraft](https://github.com/storytold/lightcraft). Do not edit them by hand. A table change is
a change to the frozen format and must follow the reopening rule of `clean-room/02-crw-format.md`.

## Boundaries

- In scope: the CIFF container records a raw developer needs, the entropy coding, sensor levels,
  as-shot white balance, orientation and the records a catalog needs (make, model, time).
- Out of scope: colour (no CRW body carries matrices; apps own their profiles), demosaicing, the
  embedded JPEG preview (apps find it with their JPEG scanner; the record is `JpgFromRaw`, 0x2007),
  and the full CIFF metadata set (ExifTool's prose documents far more records than the decoder
  reads).
