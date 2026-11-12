# craft-pcraft-format

The envelope of PhotoCraft's `.pcraft` bundle: the manifest around the document, tile
references, the entry paths, the limits for reading a manifest, and the runner for manifest
migrations. The document model stays in PhotoCraft and is a type parameter here. The crate
does not read ZIP archives, open files or decode tiles. No `unsafe`, no panics on malformed
input, and it builds for `wasm32-unknown-unknown`. The dependencies are `serde`, `serde_json`
and `thiserror`.

```toml
craft-pcraft-format = { git = "https://github.com/storytold/craft-libs", rev = "<commit>" }
```

## Bundle layout

A bundle is a ZIP archive or a directory with these entries:

| Entry | Constant or function | Contents |
|---|---|---|
| `manifest.json` | `MANIFEST` | the versioned manifest |
| `tiles/<blake3>.zst` | `tile_path(hash)` | a 256×256 tile, zstd, little-endian samples |
| `blobs/<blake3>.zst` | `blob_path(hash)` | binary data, zstd |
| `thumb.png` | `THUMB` | optional thumbnail |
| `composite/preview.png` | `COMPOSITE` | optional flattened preview |
| `composite/full.png` | `FULL_COMPOSITE` | optional full-canvas flattened image |
| `save.lock` | `SAVE_LOCK` | directory bundles only: held while a save runs (empty) |

`<blake3>` is the BLAKE3 hash of the uncompressed bytes as 64 lowercase hex digits.
`is_valid_hash` checks that form. `tile_path` and `blob_path` return `None` for any other
string, so a hash read from a file never becomes an entry name such as `tiles/../x.zst`.
`TILES_DIR` (`tiles`), `BLOBS_DIR` (`blobs`) and `COMPOSITE_DIR` (`composite`) name the
directories, and `OBJECT_SUFFIX` (`.zst`) ends every tile and blob entry. `EXTENSION` is
`pcraft` and `FORMAT_VERSION` is 1.

## Use

```rust
use craft_pcraft_format::{FORMAT_VERSION, Limits, Manifest, Step};

const STEPS: &[Step] = &[];

let manifest: Manifest<MyDocument> = craft_pcraft_format::read_manifest(&manifest_json, &Limits::default(), STEPS, FORMAT_VERSION)?;
```

- `read_manifest(json, &Limits, steps, target)` is the reader of a manifest. It runs
  `parse_manifest`, then `migrate`, which refuses a version newer than `target`, then reads the
  members, the document as `D`. `read_manifest_with` takes a function that reads the
  document from its `serde_json::Value` instead of `D`'s `Deserialize`.
- `Manifest<D>` has `format_version`, `generator`, `document: D`, `thumbnail` and `composite`,
  and serializes them in that order. Absent previews serialize as `null`. On read, missing
  previews are `None`, members the type does not name are skipped, and a `thumbnail` other
  than `thumb.png` or a `composite` other than `composite/preview.png` gives `Corrupt`.
  `Manifest` has no `Deserialize` and is `#[non_exhaustive]`: `Manifest::new(generator,
  document)` builds one at `FORMAT_VERSION` without previews.
- `Manifest::full_composite` optionally names `composite/full.png`. A missing or null member
  reads as `None`; any other path gives `Corrupt`. When absent, serialization omits this member,
  preserving the bytes of manifests without a full-canvas image. It does not change the format version.
- `TileRef` has `tx`, `ty` and `hash`, and is `#[non_exhaustive]`: `TileRef::new(tx, ty, hash)`
  builds one. `TileRef::path` gives its entry, or `None` when the hash is not valid.
- `check_manifest(json, &Limits)` checks the size, the nesting and the item count in one pass
  without recursion. PhotoCraft's own check of a manifest it saves tests the nesting and the
  item count only. `parse_manifest(json, &Limits)` runs that check and then parses into a
  `serde_json::Value` with serde_json's 128-level recursion limit turned off.
- `check_manifest_with_cancel(json, &Limits, &cancelled)` applies those same checks while
  polling a callback between bounded portions of JSON. A true result stops with `Error::Cancelled`.
- `migrate(&mut value, steps, target)` upgrades the JSON in place, where `steps[i]` upgrades
  version `i + 1` to `i + 2`, and returns the version the file had. `Step<E>` is
  `fn(&mut Value) -> Result<(), E>`; with `E: From<Error>`, the steps return the caller's own
  error type.
- `Error` is `Corrupt`, `Unsupported`, `TooNew`, `LimitExceeded`, `Json` or `Cancelled`. A manifest whose
  `format_version` is newer than `target` gives `TooNew`, and one with no step to the next
  version gives `Unsupported`. A missing or invalid `format_version` gives `Corrupt`, and any
  other missing or mistyped member gives `Json`. `TooNew` holds `found`, `supported` and the manifest's
  `generator` (`None` when it is missing or not a string), and is matched with `..`.

## Limits

`Limits` (fields `max_manifest_bytes`, `max_blob_bytes`, `max_total_bytes`, `max_depth`,
`max_items`) holds the limits PhotoCraft loads with:

- `max_manifest_bytes` (256 MiB): the size of `manifest.json`.
- `max_blob_bytes` (1 GiB): one decompressed blob.
- `max_total_bytes` (16 GiB): every decompressed tile and blob in one bundle.
- `max_depth` (367, `manifest_depth(MAX_GROUP_DEPTH)`): the nesting of JSON arrays and objects.
  PhotoCraft nests up to `MAX_GROUP_DEPTH` (100, a copy of PhotoCraft's own constant) layer
  groups, three levels each, and allows 64 more levels for the document and leaf layers. A
  value above `MAX_DEPTH` (400) is treated as 400, so parsing stays within the stack.
- `max_items` (`MAX_MANIFEST_ITEMS`, 4,194,304): arrays and objects opened, plus elements and
  members after a comma.

`check_manifest`, `parse_manifest` and `read_manifest` apply the manifest limits. The crate
does not read tiles or blobs, so it does not apply `max_blob_bytes` or `max_total_bytes`: the
caller that decompresses the entries enforces them. The struct is `#[non_exhaustive]`: start from the
defaults and set the fields you change.

```rust
let mut limits = craft_pcraft_format::Limits::default();
limits.max_manifest_bytes = 16 << 20;
```

## Testing

`cargo test -p craft-pcraft-format` runs:

- `tests/manifest.rs`: a manifest with and without previews serializes with
  `serde_json::to_vec_pretty` to JSON written by hand in the test, byte for byte, and reads
  back equal; missing previews and unknown members; entry names, hash checks, and no entry name
  for an invalid hash.
- `tests/read.rs`: `read_manifest` and `read_manifest_with` read the envelope and the document,
  refuse a newer version with the writer's `generator`, refuse a manifest past the limits, run
  the steps first, and report missing or mistyped members as a struct with a derived
  `Deserialize` reports them, in the same order; a missing or invalid `format_version` is
  `Corrupt`, and so is a preview that names another entry, such as `../x` or an absolute path.
- Doc tests: `Manifest` and `TileRef` are not built by struct literal, `Manifest` has no
  `Deserialize`, and `Error::TooNew` is matched with `..`.
- `tests/limits.rs`: the default limits, nesting at and past `max_depth`, items at and past
  `max_items`, brackets inside strings, the manifest size, JSON errors and the messages.
- `tests/migrate.rs`: steps in order, partial chains, the current version, newer, missing,
  zero and out-of-range versions, a missing step, and steps that return the caller's error type.

`.github/workflows/pcraft-format.yml` runs fmt, clippy and the tests on Linux and Windows, and
a `wasm32-unknown-unknown` check.
