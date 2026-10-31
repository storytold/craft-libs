# craft-vectorcraft-format

The envelope of VectorCraft's `.vectorcraft` file, for VectorCraft and for the apps that read its
files. The document is a type parameter: VectorCraft supplies its own document type, and a reader
that needs only the envelope uses `serde::de::IgnoredAny`. The crate takes and returns bytes; it
does not open files. No `unsafe`, no panics on malformed input, and it builds for
`wasm32-unknown-unknown`. The dependencies are `serde`, `serde_json`, `thiserror` and `flate2`
(its pure-Rust backend).

```toml
craft-vectorcraft-format = { git = "https://github.com/storytold/craft-libs", rev = "<commit>" }
```

## The file

A `.vectorcraft` file is UTF-8 JSON, gzip-compressed when saved with Use Compression. Since
version 4, entries of `INLINE_MAX` (8 KiB) or more follow the JSON after `BLOB_MAGIC`
(`\n\0VCBLOBS\0`) and give `"blob": [offset, length]` instead of base64 `data`.

| Member | Type | Written |
|---|---|---|
| `format` | `String` | `"vectorcraft"`, or `"drawcraft"` in versions 1 and 2 |
| `version` | `u32` | 1 to `VERSION` (4) |
| `generator` | `Option<String>` | `"VectorCraft <app version>"` |
| `preview` | `Option<Image>` | a PNG of the first artboard, at most `PREVIEW_MAX` (256) pixels on its longer side |
| `profiles` | `BTreeMap<String, Image>` | ICC files by profile name, when there are any |
| `pdf` | `Option<Image>` | a PDF of every artboard |
| `document` | `D` | the document |
| `images` | `BTreeMap<String, Image>` | the images the document uses, always written |

`File<D>` holds these members in this order. `serde_json::to_vec` and `to_vec_pretty` of a
`File<D>` give the bytes VectorCraft writes before the data after the JSON. `Head` reads only
`preview` and `pdf`. `Image` has `mime`, `data` (base64), `blob` and `proxy` (a linked image's
low-resolution preview).

## Use

```rust
use craft_vectorcraft_format::{File, Tail, VERSION, split_tail};

// Writing
let mut tail = Tail::new(VERSION);
let mut file = File::new(document);
file.generator = Some(format!("VectorCraft {version}"));
file.pdf = Some(tail.image("application/pdf", &pdf, false));
let mut out = serde_json::to_vec(&file)?;
tail.write_to(&mut out)?;

// Reading (decompress first when `is_compressed(&bytes)`)
let (json, after) = split_tail(&bytes);
let file: File<serde::de::IgnoredAny> = serde_json::from_slice(json)?;
file.check()?;
let pdf = file.pdf.and_then(|p| p.bytes(after));
```

- `sniff(head)` recognizes a file, plain or gzip, from its first 256 bytes after decompression:
  they start with `{` and the first `"format"` member is `"vectorcraft"` or `"drawcraft"`.
- `is_compressed(bytes)` checks for the gzip magic bytes.
- `split_tail(bytes)` and `json_part(bytes)` find the JSON and the data after it in an
  uncompressed or decompressed file.
- `File::check` and `check` return `Error::Format` for another format name and `Error::TooNew`
  for a version newer than `VERSION`.
- `Image::len` gives the size an entry states without decoding it; `Image::bytes(after)` decodes
  `data` or copies the `blob` range, and `entry(after, [offset, length])` returns the range.
- `Tail::new(version)` places each entry with `Tail::image`: after the JSON for version 4 and
  entries of at least `INLINE_MAX` bytes, inline otherwise. `Tail::write_to` writes the marker and
  the entries.
- `base64::encode`, `decode` and `decoded_len` handle the `data` members.

Size limits on the JSON and on decompression stay with each reader.

## Testing

`cargo test -p craft-vectorcraft-format` runs `tests/envelope.rs`, which compares files written
through `File` and `Tail` with JSON written out by hand in VectorCraft's layout (compact and
pretty, version 4 with data after the JSON, version 2 under the former name), reads them back,
and checks `split_tail`, `sniff` on plain, gzip and truncated gzip input, and the base64 vectors of
RFC 4648.

`.github/workflows/vectorcraft-format.yml` runs fmt, clippy and the tests on Linux and Windows,
and a `wasm32-unknown-unknown` check.
