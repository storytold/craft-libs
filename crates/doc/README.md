# craft-doc

Reads PhotoCraft `.pcraft`, VectorCraft `.vectorcraft` and EffectCraft `.ecproj` documents
without the apps: which app wrote a file, its format version, the previews and PDF it carries,
and its pages and layers. It takes bytes and returns plain data. It does not open files, render
or composite. No `unsafe`, no panics on malformed input, and it builds for
`wasm32-unknown-unknown`. The dependencies are `serde`, `serde_json`, `thiserror` and `flate2`
(its pure-Rust backend).

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
  recognized only when `head` holds the whole file.
- `info(bytes, &Limits) -> Result<Info, Error>`: the kind, the version the file declares, the
  writer it names, and `supported`, which is `false` when the version is newer than the crate
  reads.
- `renditions(bytes, &Limits) -> Result<Vec<Rendition>, Error>`: each `Rendition` has a `kind`
  (`Pdf`, `Thumbnail`, `Composite`), a `mime` and the `bytes`.
- `outline(bytes, &Limits) -> Result<Outline, Error>`: `canvas`, `pages` and `layers`.
- `Error` is `Unrecognized`, `Malformed`, `TooNew`, `Unsupported` or `LimitExceeded`. A file
  whose version is newer than the crate reads gives `TooNew` from `renditions` and `outline`.

## What each format gives

| | `.vectorcraft` | `.pcraft` | `.ecproj` |
|---|---|---|---|
| Container | JSON, plain or gzip; since version 4, large members after the JSON (`\n\0VCBLOBS\0`) | ZIP of STORE or DEFLATE entries | JSON |
| Version read | `version` up to 4 (`"format": "vectorcraft"` or `"drawcraft"`) | `format_version` 1 | `schema` up to 2 |
| Writer | `generator` | `generator` | `savedBy` |
| Renditions | `preview` (Thumbnail) and `pdf` (Pdf), when the file was saved with them | the `thumbnail` and `composite` the manifest names | none |
| Pages | artboards, in points | none | compositions by item id, in pixels, with frame rate and duration |
| Layers | top-level layers | the layer tree with groups | each composition's layers |

Layers are listed bottom first in every format. A layer's `kind` is the name the file uses:
`layer` in VectorCraft; `raster`, `group`, `adjustment`, `fill`, `text`, `shape` or `smart` in
PhotoCraft; the source type in EffectCraft (`Footage`, `Comp`, `Solid`, `Text` and others).
EffectCraft layers have no `opacity`, because opacity is an animated property there.

A `.pcraft` bundle saved as a folder, ZIP64 archives and encrypted entries are not read. The
`.ecprojx` XML copies are not read.

## Limits

`Limits` (fields `max_json`, `max_entry`, `max_total`, `max_depth`) bounds each call:

- `max_json` (256 MiB): the JSON read, which is the VectorCraft file's JSON, the PhotoCraft
  manifest or the EffectCraft project. The names and other strings returned are parts of it.
- `max_entry` (256 MiB): one rendition or one ZIP entry. The size a file states is checked before
  any of it is copied or inflated.
- `max_total` (1 GiB): every byte one call decompresses from gzip, reads out of ZIP entries or
  returns as a rendition, together.
- `max_depth` (400, also the largest value it takes, `MAX_DEPTH`): the nesting of JSON arrays and
  objects, checked before parsing. PhotoCraft nests up to 100 groups of layers, three levels each.

On a compressed `.vectorcraft` file, `info` and `outline` decompress only as far as the end of
the JSON. `renditions` checks the size of each member after the JSON against `max_entry`
before it decompresses any of that data, then decompresses only as far as the members end. The
struct is `#[non_exhaustive]`: start from the defaults and set the fields you
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
  compression methods and CRC mismatches.

`.github/workflows/doc.yml` runs fmt, clippy and the tests on Linux and Windows, and a
`wasm32-unknown-unknown` check.
