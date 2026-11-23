# craft-doc

Reads PhotoCraft `.pcraft`, VectorCraft `.vectorcraft` and EffectCraft `.ecproj` documents
without the apps: which app wrote a file, its format version, the previews and PDF it carries,
and its pages and layers. It takes bytes and returns plain data. It does not open files, render
or composite. No `unsafe`, no panics on malformed input, and it builds for
`wasm32-unknown-unknown`. Each format's envelope is read through its crate in this repository:
[`craft-pcraft-format`](../pcraft-format/README.md) (the `Manifest` members and checks, and
`check_manifest`),
[`craft-vectorcraft-format`](../vectorcraft-format/README.md) (`Image`, `sniff`, the version
and the marker) and [`craft-ecproj-format`](../ecproj-format/README.md) (`Head`, the
schema check). The outlines are read with craft-doc's own read-only views of the documents. The
other dependencies are `serde`, `serde_json`, `thiserror` and `flate2` (its pure-Rust backend).

```toml
craft-doc = { git = "https://github.com/storytold/craft-libs", rev = "<commit>" }
```

## Use

```rust
use craft_doc::{Kind, Limits, RenditionKind};

let limits = Limits::default();
match craft_doc::sniff(&bytes) {
    Some(Kind::VectorCraft) => {
        let pdf = craft_doc::renditions(&bytes, &limits)?.into_iter().find(|r| r.kind == RenditionKind::Pdf);
    }
    Some(kind) => { /* ... */ }
    None => { /* not a Craft document */ }
}
let outline = craft_doc::outline(&bytes, &limits)?;
```

- `sniff(head) -> Option<Kind>` reads the first bytes. The first 64 KiB are enough for every file
  the apps write. A re-zipped `.pcraft` whose `manifest.json` entry starts after the head is
  recognized only when `head` holds the whole file. When `head` holds a whole ZIP archive, the
  reader checks its central-directory names when the archive opens. Otherwise it checks its local
  headers. A VectorCraft file is recognized by its top-level
  `format` member, as `craft_vectorcraft_format::sniff` recognizes it, so a file written as a
  JSON array, which `File::read` also reads, is not.
- `info(bytes, &Limits) -> Result<Info, Error>`: the kind, the version the file declares, the
  writer it names, and `supported`, which is `false` when the version is newer than the crate
  reads.
- `renditions(bytes, &Limits) -> Result<Vec<Rendition>, Error>`: each `Rendition` has a `kind`
  (`Pdf`, `Thumbnail`, `Composite`), a `mime` and the `bytes`. `is_full_composite` is true only
  for a PhotoCraft composite selected from an explicit `full_composite` declaration; it is
  false for the legacy preview and every other rendition. It records the selected source,
  without decoding pixels or checking canvas dimensions. Derived equality and debug output
  include this provenance.
- `rendition(bytes, RenditionKind, &Limits) -> Result<Option<Rendition>, Error>`: the requested
  kind only, or `None` when absent. Container metadata and JSON limits are still checked. Other
  rendition payloads are not decoded or copied: a damaged preview does not prevent reading the
  PDF, and a missing thumbnail ZIP entry does not prevent reading the composite. A compressed
  member's offset still counts against `max_total`, including bytes traversed to reach it.
  Normal gzip reads remain buffered. If read-ahead reaches a damaged appended member, the
  metadata read retries with exact compressed input and output through the JSON marker;
  already-counted prefix bytes are replayed without retaining or charging another copy.
  JSON and selected-payload limits still apply, and requesting the damaged member still fails.
- `outline(bytes, &Limits) -> Result<Outline, Error>`: `canvas`, `pages` and `layers`.
- `pcraft_document::<D>(bytes, &Limits) -> Result<D, Error>`: a PhotoCraft manifest's `document`
  deserialized directly into the caller's metadata type. It reads no previews, tiles or blobs
  and does not check that referenced payload entries exist. Envelope, version, JSON byte and
  nesting checks still apply. Before the caller's deserializer runs, `max_items` bounds the
  whole manifest's structural JSON items, including skipped fields, as the shared format's
  `check_manifest` counts them (opening array/object brackets and commas outside strings),
  capped at that format's own ceiling. The caller owns its type's
  allocations and behavior. No pixel feature is required. A cancellation variant can follow
  after the cooperative document-read APIs in [#571](https://github.com/storytold/craft-libs/pull/571)
  merge.
- `Error` is `Unrecognized`, `Malformed`, `TooNew`, `Unsupported`, `LimitExceeded` or `Cancelled`. A file
  whose version is newer than the crate reads gives `TooNew` from `renditions` and `outline`. A
  ZIP archive whose `manifest.json` has no `format_version` gives `Unrecognized`.

### Cooperative cancellation

`info_with_cancel`, `renditions_with_cancel` and `outline_with_cancel` take the same bytes and
limits plus `&dyn Fn() -> bool`. The callback runs on the calling thread. Once it returns true,
that call stops with `Error::Cancelled`, even if a later callback result would be false.

```rust
use std::sync::atomic::{AtomicBool, Ordering};

let stopped = AtomicBool::new(false);
let renditions = craft_doc::renditions_with_cancel(&bytes, &limits, &|| stopped.load(Ordering::Relaxed))?;
```

Checks occur during format detection, JSON scans and parsing, gzip input and output, rendition
copies, ZIP directory and entry reads, CRC traversal, and outline conversion, including shared
header, manifest and base64 helpers.
The existing entry points keep their behavior and limits. PDF parsing/rendering and optional
PhotoCraft layer pixel decoding are separate operations.

## What each format gives

| | `.vectorcraft` | `.pcraft` | `.ecproj` |
|---|---|---|---|
| Container | JSON, plain or gzip; since version 4, large members after the JSON (`\n\0VCBLOBS\0`) | ZIP of STORE or DEFLATE entries | JSON |
| Version read | `version` up to 4 (`"format": "vectorcraft"` or `"drawcraft"`) | `format_version` 1 | `schema` up to 2 |
| Writer | `generator` | `generator` | `savedBy` |
| Renditions | `preview` (Thumbnail) and `pdf` (Pdf), when the file was saved with them | the `thumbnail` and flattened composite the manifest names | none |
| Pages | artboards, in points | none | compositions by item id, in pixels, with frame rate and duration |
| Layers | top-level layers | the layer tree with groups | each composition's layers |

Layers are listed bottom first in every format. A layer's `kind` is the name the file uses:
`layer` in VectorCraft; `raster`, `group`, `adjustment`, `fill`, `text`, `shape` or `smart` in
PhotoCraft; the source type in EffectCraft (`Footage`, `Comp`, `Solid`, `Text` and others).
EffectCraft layers have no `opacity`, because opacity is an animated property there.

A `.pcraft` bundle saved as a folder, ZIP64 archives and encrypted entries are not read. The
`.ecprojx` XML copies are not read.

For PhotoCraft, `Composite` prefers `full_composite` (`composite/full.png`) when declared,
otherwise `composite` (`composite/preview.png`). The unused preview payload is not read. A
declared full-canvas image that is missing or exceeds the limits is an error. The reader does
not silently substitute the smaller preview. Returned PNG bytes retain any embedded color profile;
the reader does not decode pixels or convert colors.

## PhotoCraft layer pixels

With the feature `pcraft-pixels`, which adds `ruzstd` and `blake3` (both pure Rust),
`pcraft_layer_rgba(bytes, layer, &Limits)` decodes one layer of a `.pcraft` bundle over the
canvas:

```toml
craft-doc = { git = "https://github.com/storytold/craft-libs", rev = "<commit>", features = ["pcraft-pixels"] }
```

- `layer` is a path of indexes through `Outline::layers` and each group's `children`, bottom
  first: `&[1, 0]` is the bottom layer inside the second layer from the bottom.
- A raster layer gives its surface; a text, shape or smart object layer gives the rendered copy
  PhotoCraft saves with it. A video layer is a raster layer whose frames are in its `video`
  member; it gives its surface, which holds the frame at the playhead, and the other frames are
  not read. A group, an adjustment or fill layer, a layer saved
  without a rendered copy, or a path that names no layer gives `Ok(None)`, and the mask of such a
  layer is not returned.
- `LayerPixels::rgba` holds `width` × `height` RGBA pixels, row by row, in the layer's sample
  type: `Samples::U8`, `U16` or `F32`. Grayscale repeats the gray value in red, green and blue,
  and a surface without alpha is opaque. Pixels without a tile hold the surface's default pixel.
  Tiles outside the canvas, at negative columns and rows among them, are not read.
- `LayerPixels::mask` holds the layer mask, one sample per pixel, with its `enabled`, `density`
  and `feather` as saved. Density and feather are not applied. A mask stored as RGB gives its
  first channel.
- Surfaces stored as RGB or grayscale are read at 8, 16 and 32 bits; PhotoCraft stores the layers
  of bitmap, duotone, indexed and multichannel documents that way too. A layer or mask stored in
  another color mode, such as CMYK or Lab, gives `Error::Unsupported`. A canvas of zero width or
  height gives `Error::Malformed`.
- The decoded layer and the decoded mask each count as one entry against `max_entry`, and with
  each distinct tile read and decompressed against `max_total`, checked before they are
  allocated. A tile that several references name is read and decoded once per call. A tile is
  decoded only up to its size, 256 × 256 pixels, and its samples must match the BLAKE3 hash that
  names them. A tile's zstd frame larger than the tile plus an eighth and 4 KiB, or one that
  declares a window larger than 8 MiB, or has bytes after its frame, is refused. Tile references
  in parsed surfaces, rendered caches and masks throughout the layer tree count against
  `max_items`, including other layers. Video frames and other unrecognized members are skipped.

`tests/pixels.rs` builds bundles with tiles compressed by `ruzstd` and runs with
`cargo test -p craft-doc --all-features`: 8, 16 and 32-bit tiles, the default pixel, negative
tile coordinates, a mask, layers in groups and layers without pixels, text, shape and smart
object copies, bitmap and indexed documents, a CMYK document, a zstd bomb, a tile shared by many
references, and the limits.

## PhotoCraft ICC payload

With `pcraft-pixels`, `pcraft_icc_profile(bytes, &limits)` returns the raw blob named by
`document.icc_profile`. Missing or null gives `Ok(None)`; a declared missing or damaged blob
is an error. The payload's BLAKE3 hash is checked, but its ICC semantics are not validated.
An empty verified payload is returned as `Some`, and no pixels or colors are converted.
Previews, tiles and unrelated blobs are not read.

The manifest, the selected ZIP entry holding one zstd frame, and the decoded profile count
against `max_total`. Each is limited by `max_entry`, and the manifest also by `max_json` and
the normal PhotoCraft envelope/depth checks. Decoding admits only the remaining output budget,
at most an 8 MiB window, and no bytes after the frame. Structural manifest items (objects and
arrays opened, plus commas outside strings) count against `max_items` before metadata is read,
including skipped fields, with PhotoCraft's structural ceiling of 4,194,304. This is different
from the layer-pixel reader's counting of parsed layer and tile-reference lists. An absent
profile does not bypass manifest limits. Limits apply per call; a consumer reading
several layers and a profile must also bound its combined work and retained memory.

This call takes no cancellation callback. It returns payload bytes rather than a parsed or
validated color profile and does not promise that a renderer honors those bytes.

## Limits

`Limits` (fields `max_json`, `max_entry`, `max_total`, `max_depth`) bounds each call:

- `max_json` (256 MiB): the JSON read, which is the VectorCraft file's JSON, the PhotoCraft
  manifest or the EffectCraft project. The names and other strings returned are parts of it.
- `max_entry` (256 MiB): one rendition or one ZIP entry. The size a file states is checked before
  any of it is copied or inflated.
- `max_total` (1 GiB): the limit counts decompressed gzip bytes a call uses, bytes it reads out of
  ZIP entries, and bytes it returns as renditions. A rendition decompressed from gzip is counted once, and a
  second rendition of the same bytes is counted as another copy. The gzip reader may decode a
  64 KiB chunk past the JSON or final rendition. Unused bytes in that chunk do not count.
- `max_depth` (400, also the largest value it takes, `MAX_DEPTH`): the nesting of JSON arrays and
  objects, checked before parsing. PhotoCraft nests up to 100 groups of layers, three levels each.
  `renditions` and `outline` refuse a PhotoCraft manifest nested deeper than 367 levels
  (`craft_pcraft_format::manifest_depth(MAX_GROUP_DEPTH)`), as PhotoCraft does.

A PhotoCraft manifest with more than 4,194,304 JSON items
(`craft_pcraft_format::MAX_MANIFEST_ITEMS`) is refused with `Error::LimitExceeded`, as PhotoCraft
reads it.

On a compressed `.vectorcraft` file, `info` and `outline` read the JSON from the decompressed
stream. The reader decodes chunks of up to 64 KiB, so the chunk holding the end of the JSON may also hold
data after its marker. `renditions` checks each member's size against `max_entry` and the last
member's ending offset plus duplicate-copy sizes against the remaining `max_total` budget before
reading the tail. Beyond the buffered chunk, it decompresses only as far as the members end and
keeps only their bytes. The struct is `#[non_exhaustive]`: start from the defaults and set the fields you
change.

```rust
let mut limits = craft_doc::Limits::default();
limits.max_entry = 32 << 20;
```

## Testing

`cargo test -p craft-doc` runs:

- `tests/formats.rs`: documents of each format written by the tests (JSON by hand, PNG and ZIP
  bytes assembled in the test, gzip through `flate2`), read back through each call: plain and
  gzip VectorCraft files, members inline as base64 and after the JSON, `.pcraft` bundles with
  STORE and DEFLATE entries and the manifest last, `.ecproj` compositions, newer versions, and
  files `sniff` must not claim (PDF, PNG, other ZIP and JSON files).
- `tests/hostile.rs`: every truncation of each sample, seeded mutations, proptest on arbitrary
  bytes, and inputs that pass each limit: a gzip bomb, a DEFLATE entry larger than its stated
  size, members longer than `max_entry`, JSON nested past `max_depth`, ZIP64, encryption, other
  compression methods and CRC mismatches. An array of a million elements in place of a
  `generator` or a preview is skipped.
- `tests/envelopes.rs`: files whose envelope is written with the format crates' types read
  through each call, and envelopes those crates refuse (a manifest without `generator`, a
  manifest that is a JSON array, a preview that names another entry, a `format` member inside
  another member) are refused. A preview or PDF member without `mime` gives no rendition, and a
  VectorCraft `generator`, `preview` or `pdf` reads as `File::read` reads it: as `None` when it
  has the wrong shape, and as its last value when it is repeated. A value of the wrong shape is
  skipped as it is parsed, and none of it is kept. A repeated `version`, and in `renditions` a
  missing or repeated `document`, are refused, as `File::read` refuses them.

`.github/workflows/doc.yml` runs fmt, clippy and the tests on Linux and Windows, and a
`wasm32-unknown-unknown` check.
