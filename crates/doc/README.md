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
  recognized only when `head` holds the whole file. When the ZIP archive opens, the names in its
  central directory decide. Otherwise detection falls back to the local headers, including for
  a complete damaged archive; reading that archive can then return `Malformed`.
  A VectorCraft file is recognized by its top-level
  `format` member, as `craft_vectorcraft_format::sniff` recognizes it, so a file written as a
  JSON array, which `File::read` also reads, is not.
- `info(bytes, &Limits) -> Result<Info, Error>`: the kind, the version the file declares, the
  writer it names, and `supported`, which is `false` when the version is newer than the crate
  reads.
- `renditions(bytes, &Limits) -> Result<Vec<Rendition>, Error>`: each `Rendition` has a `kind`
  (`Pdf`, `Thumbnail`, `Composite`), a `mime` and the `bytes`.
- `outline(bytes, &Limits) -> Result<Outline, Error>`: `canvas`, `pages` and `layers`.
- `Error` is `Unrecognized`, `Malformed`, `TooNew`, `Unsupported` or `LimitExceeded`. A file
  whose version is newer than the crate reads gives `TooNew` from `renditions` and `outline`. A
  ZIP archive whose `manifest.json` has no `format_version` gives `Unrecognized`.

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
- `max_total` (1 GiB): the JSON and marker consumed from gzip, the data after the marker consumed
  to reach the requested members, bytes read out of ZIP entries, and returned renditions,
  together. A rendition decompressed from gzip is counted once, and a second rendition of the
  same bytes is counted as another copy. Initial gzip reads use chunks of up to 64 KiB, so
  bytes past the marker can be decompressed before they are consumed; unused lookahead bytes
  do not count against this limit.
- `max_depth` (400, also the largest value it takes, `MAX_DEPTH`): the nesting of JSON arrays and
  objects, checked before parsing. PhotoCraft nests up to 100 groups of layers, three levels each.
  `renditions` and `outline` refuse a PhotoCraft manifest nested deeper than 367 levels
  (`craft_pcraft_format::manifest_depth(MAX_GROUP_DEPTH)`), as PhotoCraft does.

A PhotoCraft manifest with more than 4,194,304 JSON items
(`craft_pcraft_format::MAX_MANIFEST_ITEMS`) is refused with `Error::LimitExceeded`, as PhotoCraft
reads it.

On a compressed `.vectorcraft` file, the initial read finds the end of the JSON in chunks of up
to 64 KiB, so it can also decompress data past the marker. `info` and `outline` do not consume
that data. Before reading further, `renditions` checks each member against `max_entry` and the
last member's end plus the cost of duplicate copies against the remaining `max_total` budget.
It then consumes data through the last requested member, keeping only the members' bytes.
`Limits` is `#[non_exhaustive]`: start from the defaults and set the fields you change.

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
