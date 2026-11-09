# craft-jpeg

A clean-room JPEG decoder for the Craft apps, written from ITU-T T.81 (ISO/IEC 10918-1). It
covers the processes that common Rust JPEG decoders skip: 12-bit samples and arithmetic coding.
It also decodes the common sequential 8-bit files a band of rows at a time ([`Bands`](#decoding-in-bands)),
for images too large to decode into one buffer.
No `unsafe`, no panics on malformed input, and it builds for `wasm32-unknown-unknown`. The only
dependency is `thiserror`.

```toml
craft-jpeg = { git = "https://github.com/storytold/craft-libs", rev = "<commit>" }
```

## What it decodes

| Frame | Process | Precision |
|---|---|---|
| SOF0, SOF1 | sequential DCT, Huffman coded | 8 or 12-bit |
| SOF9 | extended sequential DCT, arithmetic coded | 8 or 12-bit |
| SOF10 | progressive DCT, arithmetic coded | 8 or 12-bit |

Every other frame returns `Error::Unsupported`: progressive Huffman (SOF2), lossless (SOF3,
SOF11), hierarchical (SOF5 to SOF7, SOF13 to SOF15), other precisions, more than one frame, and
a height given only in a DNL marker. Images have 1, 3 or 4 components with any sampling factors
and restart intervals.

## Use

```rust
use craft_jpeg::{decode, Limits, Samples};

let img = decode(&bytes, &Limits::default(), adobe_transform)?;
match img.samples {
    Samples::U8(px) => { /* 8-bit frames */ }
    Samples::U16(px) => { /* 12-bit frames, scaled to 16 bits */ }
}
if img.damaged { /* warn: part of the image is missing */ }
```

- `decode(bytes, &Limits, adobe_transform: Option<u8>) -> Result<Decoded, Error>`.
  `adobe_transform` is the transform byte of the APP14 `Adobe` segment, when the app found one.
- `frame(bytes) -> Option<FrameKind>` reads the headers up to the first SOFn marker and returns
  that marker and the declared sample precision, without decoding.
- `Error` is `Malformed`, `Unsupported` or `LimitExceeded`, each with a message.

## Output

`Decoded` has `width`, `height`, `components`, `samples` and `damaged`.

- `samples` is interleaved by pixel, rows top to bottom: `Samples::U8` for 8-bit frames,
  `Samples::U16` for 12-bit frames, scaled to the full 16-bit range
  (`round(v × 65535 / 4095)`).
- `components` is 1 (gray), 3 (RGB) or 4 (CMYK).
  - 3 components are converted from YCbCr to RGB (JFIF, T.871), unless the Adobe transform is
    0 or the component IDs are `R`, `G`, `B`.
  - 4 components are ink amounts (0 = no ink). The stored values are taken to follow Adobe's
    inverted convention (maximum = no ink) and are inverted back. With Adobe transform 2 (YCCK),
    C, M and Y are converted from YCbCr, which yields ink amounts directly, and K is inverted.
- `damaged` is set when entropy-coded data was cut short or corrupt after at least one MCU had
  decoded, or its scans would decode more than 64 passes over the frame's blocks. The rest of
  that scan, and every scan past the budget, is left out; those blocks keep what earlier scans
  decoded. A stream with no decodable image data is an error. Arithmetic-coded data that stops
  early at a marker reads as the zero bytes encoders drop at the end of a scan (libjpeg-turbo
  treats it the same way), so for arithmetic scans only data that runs to the end of the file
  and a missing restart marker count as damage.

## Limits

`Limits` (fields `max_pixels`, `max_alloc`) is checked against the frame header before anything
is allocated: `width × height` against `max_pixels`, and the output buffer plus the decoder's
own coefficient and sample buffers, together, against `max_alloc`. The defaults are 2³⁰ pixels
and 8 GiB on 64-bit targets, 1 GiB on 32-bit ones (wasm32). The struct is `#[non_exhaustive]`:
start from the defaults and set the fields you change:

```rust
let mut limits = craft_jpeg::Limits::default();
limits.max_alloc = 512 << 20;
```

A file of under a hundred bytes can declare a frame that fills the default limits, and
decoding it still renders every sample. Callers that decode for a UI or for previews should
pass a `max_alloc` well below the default.

## Routing

An app that decodes baseline JPEG with another decoder keeps its own rule for which files come
here, built on `frame`. PhotoCraft sends arithmetic-coded files (SOF9 to SOF11) and SOF0/SOF1
files whose precision is not 8 bits:

```rust
fn use_craft_jpeg(bytes: &[u8]) -> bool {
    match craft_jpeg::frame(bytes) {
        Some(f) if (0xC9..=0xCB).contains(&f.marker) => true,
        Some(f) if matches!(f.marker, 0xC0 | 0xC1) => f.precision != 8,
        _ => false,
    }
}
```

SOF11 (lossless) falls in that range and returns `Unsupported`.

## Decoding in bands

`Bands` decodes a sequential Huffman-coded (SOF0, SOF1) 8-bit file whose first scan holds every
component, one line of MCUs at a time: a band is 8 rows times the largest vertical sampling
factor (8 rows for a single-component frame, whatever its sampling factors), and the last band
may be shorter. Its samples equal `decode`'s for the same rows (the same inverse transform, upsampling
and colour conversion); only a few MCU lines are held at once. Other files return
`Error::Unsupported`: decode them whole.

```rust
use craft_jpeg::{Bands, Limits};

let mut limits = Limits::default();
limits.max_pixels = 65_535 * 65_535; // the image is never held whole
let bands = Bands::new(&bytes, &limits, adobe_transform)?;
let mut reader = bands.reader();
while let Some(band) = reader.next_band()? {
    // band.y, band.rows, band.samples: interleaved like `Decoded::samples`
}
if reader.damaged() { /* warn: part of the image is missing */ }
```

- `limits.max_pixels` bounds the image; `limits.max_alloc` bounds one reader's buffers
  (`Bands::reader_bytes`) and the upsampling maps, not the image.
- **In parallel.** A band that starts a restart interval is a seek point once
  `Bands::find_seek_points` has found the restart marker before it (one pass over the scan
  data). The k-th marker in the data begins interval k, as a reader that reads in order pairs
  them, so missing, misnumbered or added markers in a damaged file change nothing but which
  bands are seek points. Readers on separate threads can `seek` to seek points and decode the
  bands up to the next one. Files with a restart interval at every line of MCUs, common in files
  Photoshop saved, decode on every core.
- **Damaged data** leaves the rest of its restart interval at zero coefficients (mid grey) and
  decoding resumes at the next restart marker; without restart intervals the rest of the scan
  stays at zero, as in `decode`. An interval, or the scan, that leaves more than its padding
  unread before the next marker counts as damaged too. `Reader::damaged` is set either way, and
  the samples don't depend on where reading started. Damage never makes a read fail, also in the
  first MCU (where `decode` reports that there is no image data). `Reader::damaged` does not
  tell that case from later damage, and neither does the size of the scan: data that fails at
  its first code can be any length.

## Boundaries

- In scope: decoding the frames above into samples.
- Out of scope: metadata. The decoder skips every APPn and COM segment. ICC profiles, EXIF,
  XMP, Photoshop resources, resolution and the APP14 `Adobe` segment are the app's to read.
  Color management and encoding also stay in the apps.

## Testing

`cargo test -p craft-jpeg` runs:

- unit tests in `src/tests.rs`: arithmetic-coded streams from a test-only encoder must decode to
  the coefficients that went in, and Huffman streams are compared against `zune-jpeg`.
- `tests/conformance.rs`, on libjpeg-turbo's test images in `tests/data` (provenance and how
  the derived files were made: `tests/data/SOURCE.md`):
  - `testimgari.jpg` (SOF9) decodes to exactly the samples of its jpegtran Huffman transcode.
  - jpegtran's arithmetic and progressive arithmetic transcodes of `testorig.jpg` (one with
    restarts) and `testorig12.jpg` decode to exactly the samples of their sources.
  - `whitetail-ari.jpg` (SOF9 from cjpeg, a white lower part that decodes from the zeros
    supplied at EOI) decodes undamaged and equal to its Huffman transcode.
  - `testorig.jpg` matches `zune-jpeg`, and `testorig12.jpg` matches libjpeg-turbo's 12-bit
    decode, within the float-against-integer IDCT difference (mean under 0.75, max 4).
- `src/bands/tests.rs`: `Bands` against `decode`, sample for sample, over gray, YCbCr, CMYK and
  YCCK streams from `jpeg-encoder` (4:4:4, 4:2:2, 4:2:0, 4:4:0; restart intervals of 1 and 7
  MCUs and of one and two MCU lines) and over interleaved layouts from a test-only encoder
  (factors of 3 and 4 across and down, four components), read in order and by seeking; against
  `zune-jpeg`; on damaged data (with and without restarts, in the first interval, and bytes
  added before a restart marker), limits, and the files it refuses.
- `tests/conformance.rs` also decodes the Huffman-coded 8-bit test images in bands.
- `tests/mutation.rs`: a seeded mutation run (truncations, bit flips, byte and frame-header
  overwrites, deleted and repeated spans) over the test images and the fuzz seeds. Decoding must
  not panic and must stay within small `Limits`. The band decoder gets the same mutations, and
  wherever seek points survive one, reading from them must give the samples reading in order
  does.

`.github/workflows/jpeg.yml` runs fmt, clippy, the tests and a `wasm32-unknown-unknown` check on
Linux.

### Fuzzing

`fuzz/` is a cargo-fuzz crate outside the workspace, with two targets: `decode`, and `bands`,
which also checks that reading from the seek points gives the samples reading in order does.
The seeds in `fuzz/corpus/decode` are the test images plus small SOF9 (with restarts), 12-bit
SOF9, SOF10 and 12-bit SOF1 streams; `bands` starts from them too. With nightly and
`cargo install cargo-fuzz`:

```sh
cd crates/jpeg/fuzz
cargo +nightly fuzz run decode
cargo +nightly fuzz run bands corpus/bands corpus/decode
```

Each target adds the inputs it finds to the first corpus directory it reads: `decode` to
`fuzz/corpus/decode`, `bands` to `fuzz/corpus/bands`. Commit only seeds that cover something
new.

## Credits

The decoder was written by Michael Higgins (Higgs32584) for PhotoCraft.
