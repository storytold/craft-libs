# craft-pdf

Shared PDF infrastructure with independent optional features. Applications own document
models, editing, layout, font policy, worker scheduling, output presets and printing.
No feature is enabled by default.

| Feature | Provides |
| --- | --- |
| `read` | Bounded opening and the patched Hayro syntax types |
| `interpret` | `read` plus an app-owned Device interpreter, fonts and CMaps |
| `raster` | `interpret` plus premultiplied RGBA8 rendering |
| `write` | Guarded Krilla authoring; no shared reader dependency |
| `write-pdf` | `write` plus Krilla's embedded-PDF support |
| `write-simple-text` | `write` plus Krilla's optional text shaper and `draw_text` |
| `filters` | The `/Filter` chain for editing and writing PDF files: decode and encode, with limits and accounting. No reader or writer dependency |
| `crypt` | The standard security handler: decrypt, encrypt and create encryption. No reader or writer dependency |
| `cos` | `crypt` and `filters` plus the object layer: open, edit and save PDF files (incremental and full saves). No reader or writer dependency |

## Reading and rendering

`open` accepts shared bytes without copying. `open_slice` reserves storage fallibly before
copying. Both preserve typed password errors. Their `_with_options` variants accept
`OpenOptions`: a caller-owned input limit and immutable per-document `syntax::DecodeLimits`.
The default input limit is 512 MiB; callers can preserve their own accepted-file policy.
Ordinary Flate/LZW/RunLength expansion defaults to 256 MiB per stream and can be lowered,
including during initial xref reads, object streams, repair and fallback. Known-size image
pixels and streaming page content have separate backend budgets. These controls do not bound
total parser memory or execution time.

`open_with_options_and_cancel` and `open_slice_with_options_and_cancel` additionally accept
`&syntax::Cancellation`. It owns a `Send + Sync + 'static` callback, which runs on the opening
thread. Capture shared state such as an `Arc<AtomicBool>` to request cancellation from a worker's
caller. Each open latches a true result and returns `CancellableOpenError::Cancelled`; other
failures wrap the existing `OpenError`. Existing opening APIs and their error types are unchanged.
The returned PDF does not retain the callback or poll it during later reads.

Opening checks cancellation during xref searches and recovery, byte and object scans, ordinary
stream decoding and predictors, and initial page-tree traversal. The borrowed-input variant also
checks while copying input. Cryptographic and image-codec calls run to completion between checkpoints.
These are opening checkpoints; interpretation and rasterization have their own cancellation policy.

For a bare Flate page, `Page::page_stream` materializes content under the ordinary limit,
while `Page::content_reader` decodes incrementally under the separate streaming budget.
Interpretation and rasterization use that incremental path. Lowering the ordinary limit
(even to zero) therefore does not limit streamed Flate page content or cause a truncation
warning by itself. Whole-stream fallback decoders still use the ordinary limit.

`interpret_page` guards an app-owned Device. `raster::render` and `render_into` accept explicit
background, scale, viewport, offset and optional solid-colour replacement. Output is
premultiplied RGBA8. Geometry is cropped, rotated and scaled for UserUnit by the backend.
Dimensions without an explicit viewport floor to at least one pixel per side; callers can
request rounded-up sizes for partial edge pixels. Output is limited to 8192 pixels per side
and 64,000,000 pixels total. Invalid geometry and buffer lengths fail before drawing.

`InterpreterSettings` preserves caller font resolution, annotation policy, OCG object
identities, cancellation, preview quality and warning callbacks. Preview caches remain
separate from final-quality image entries. Reading-colour presets and print bypass policy
belong to the app; the shared layer only maps the requested colours.

Diagnostics report known image failures and incomplete page or nested painting content,
including omitted streams, failed decoding and exhausted content budgets. They are not a fidelity certificate: the
backend may approximate unsupported content, does not currently emit `UnsupportedFont`,
and cached objects need not repeat previous warnings. Structural xref repair and objects
omitted by a decode limit have no separate warning; a successful open does not certify
that all objects survived. Some tolerant whole-stream decoders can return an incomplete
prefix without a failure status. Use fresh caches for checks, while retaining these limitations.
Native unwinding panics become errors; discard partial devices, buffers, parsers and caches
after backend failure. WASM aborts and process-wide allocation failure cannot be caught.
During interpretation, cancellation is polled between operators, so a decoder may finish before noticing it.

## Backend provenance and consumer pins

The shared reader vendors the current patched Hayro trees from PdfCraft revision
`ec0a965074f376a5bf610435948e4e5184bf3299`. See
[`vendor/README.craft-reader.md`](../../vendor/README.craft-reader.md) for the transfer and
local patches. Lopdf, COS editing, inspection fallback and application caches remain app-owned.

The patched 0.7 reader also carries focused compatibility changes from Hayro 0.8:
whole-dictionary resource scopes, a 64-level direct-container limit, whole-run text
fill/stroke order and byte transfer-function sampling. It retains the existing Lab
Bradford and floating-color behavior, Type 3 handling, decoding limits and streaming
completion diagnostics. Iterative page-tree traversal preserves deep valid trees and
valid siblings around cycles, with a 100,000-entry work budget that fails closed rather
than exposing a partial page list. These changes do not imply full equivalence to the 0.8 backend;
the vendor notes record the boundaries. Existing consumers remain on their immutable
revision until they explicitly adopt a new one.

Consumers pin `craft-pdf` and their required root `[patch.crates-io]` entries to the same full
`https://github.com/storytold/craft-libs` revision. Cargo ignores dependency-local patches.
Reader consumers patch `hayro-syntax`, `hayro-interpret` and `hayro-jbig2`; raster consumers
also patch `hayro`. Writer consumers patch `krilla`. A plain `write` consumer does not need
reader patches. Keep unrelated craft-ui/craft-print versions unchanged when adopting PDF.
A pinned commit must be available remotely before a fresh public checkout can resolve it.

Synthetic native and WASM contracts cover opening, caller limits, alpha, geometry, settings,
preview/final caching, diagnostics, cancellation and native panic recovery. Vendored syntax
unit tests additionally cover decoder limits and incomplete streams. App-owned regression
suites retain higher-level fidelity, editing, text and worker contracts. Real-file corpora
stay outside this repository; dependency assets and licences are in the root attribution.

## Reader verification

The Shared PDF workflow runs native and WebAssembly facade contracts, an independent
`read-only-contract` consumer, and the transferred backend tests. The independent consumer
keeps interpreter-bearing dev-dependencies out of the shipping `read` feature graph.
Run `bash scripts/test-pdf-backends.sh` from the repository root to test the vendored
backends, the syntax doctest, syntax-only configuration and alternate `unsafe` codec implementation.
The script uses temporary
source copies and the complete local patch set, preserving the vendored lockfiles.
Run `cargo test --manifest-path crates/pdf/read-only-contract/Cargo.toml --locked` for the
read-only consumer, then repeat with `--target wasm32-unknown-unknown` and the test runner.
These backend checks supplement the root workspace gates; excluded vendor crates are not
covered by `cargo test --workspace` alone.

## Object layer

`cos` is an object layer for apps that edit PDF files (ISO 32000-2 §7). It parses the object
graph lazily and tolerantly, keeps edits in a copy-on-write overlay, and writes the file back
incrementally (the original bytes untouched) or as a full, garbage-collected rewrite. It uses
`crypt` and `filters`; the reader (`read`) and writer (`write`) are separate and do not use it.

```rust
use craft_pdf::cos::{Document, ObjRef, Object, SaveOptions, write_full, write_incremental};

let doc = Document::open(bytes)?;                              // Arc<Vec<u8>>
let doc = Document::open_with_password(bytes, Some("owner"))?; // encrypted files
let obj = doc.get(ObjRef::new(12, 0));                         // loaded on first use, cached
let data = stream.decoded()?;                                  // filters applied on demand
doc.set(r, Object::Dict(d));                                   // edits land in the overlay
let r = doc.add(obj);
let bytes = write_incremental(&doc, &SaveOptions::default())?; // append a revision
let bytes = write_full(&doc, &SaveOptions::default())?;        // rewrite what the trailer reaches
```

- Reads every cross-reference form (tables, streams, hybrid `/XRefStm`) and object streams. A
  file whose cross-reference data is wrong or missing is rebuilt by scanning for object headers,
  and each repair is recorded (`repair_log`). Malformed input gives `CosError`, never a panic.
- Encryption on open and save through `crypt`: revisions 2 to 6, crypt filters, `/EFF` and
  `/EncryptMetadata`. `set_encryption` and `remove_encryption` change it.
- `write_incremental` appends the edited objects with a cross-reference section in the style of
  the file's last one, so earlier revisions and their signatures stay as they were; it falls back
  to `write_full` when an edit cannot be appended. `write_full` renumbers what the trailer
  reaches and, by default, packs objects into object streams. Both return the bytes.
- `Document` is cheap to clone: the bytes, index and caches are shared, so a clone is a
  snapshot for undo or a background job.
- Images: `ImageBudget::decode` gives an image's samples after its filters, CCITT fax and JBIG2
  included, all images of one operation spending from one allowance. DCT and JPX are not decoded
  here.
- Not here: streaming files larger than memory, linearized output.

The code comes from PdfCraft's `pdfcraft-cos` crate (revision
`1d2af9907ba772b368f6b54811359805fa5428e9`), unchanged apart from module paths and doc
comments. Its test helpers come along: `src/cos/testkit.rs` (synthetic files, from
`pdfcraft-testkit`) and `tests/cos/oracle.rs` (qpdf runs, from `pdfcraft-oracle`; a missing
`qpdf` skips those tests, or fails them when `CRAFT_PDF_ORACLES_REQUIRED` is set). Tests:
`cargo test -p craft-pdf --no-default-features --features cos`. Examples: `cos_info` and
`cos_protect` (`--features cos`).

## Stream filters

`filters` is the `/Filter` chain of a PDF stream (ISO 32000-2 §7.4) for code that edits or
writes PDF files. The reader decodes streams on its own and does not use it. The codecs are
[`craft-stream-codecs`](../stream-codecs/README.md); this module adds Flate (flate2 with its
zlib-rs backend), the chain and its accounting.

- `decode(chain, data, max_output)` runs the general-purpose filters first to last (Flate,
  LZW, ASCIIHex, ASCII85, RunLength, with the TIFF and PNG predictors). Image codecs, `Crypt`,
  unknown filters and a chain longer than `MAX_CHAIN` (8) give `Unsupported`, so the caller
  keeps such data encoded. `max_output` bounds every stage and gives `LimitExceeded`.
- `decode_tolerant` returns what was decoded before damage, with `partial = true`, and runs the
  rest of the chain on it.
- `decode_counted`, `decode_tolerant_counted` and `decode_tolerant_counted_each` also return the
  bytes every stage decoded, on success and on failure, for a caller charging a budget.
- `decode_within`, `decode_tolerant_within` and `decode_image_tolerant_within` spend from a
  caller-owned `work` allowance shared by a run of images, as `decode_ccitt_within` does; what
  `work` cannot pay for gives `Spent`. Flate pays `FLATE_READ_WORK` steps a byte it reads.
- `encode(filter, params, data)` writes Flate (zlib level 6), LZW, ASCIIHex, ASCII85 and
  RunLength, with a predictor first for Flate and LZW. `encode_g4`, `encode_jbig2`,
  `encode_jbig2_generic` and `encode_jbig2_shared` code 1-bit images.

The code comes from PdfCraft's `pdfcraft-filters` crate (revision
`80e26e674d1aa33078e83c8139875047f3289eb6`): its `lib.rs` and `flate.rs` here, its codecs in
`craft-stream-codecs`. Tests: `cargo test -p craft-pdf --no-default-features --features filters`.
A fuzz target is in `crates/pdf/fuzz` (`cargo +nightly fuzz run filters`, from `crates/pdf`).

## Encryption

`crypt` is the PDF standard security handler (ISO 32000-2 §7.6) for code that edits or writes
PDF files. The reader decrypts on its own and does not use it. It covers revisions 2 to 6:
40-bit and 128-bit RC4, AES-128 crypt filters, and AES-256 (R5 is read, R6 is read and written).

- `SecurityHandler::open(dict, id0, password)` authenticates the owner password first, then the
  user password. Passwords are tried as PDFDocEncoding and as typed (R2–R4), or in their
  SASLprep form and as typed (R5/R6). `authenticate_owner` upgrades a user handler.
  `without_key` is for a file opened without a password whose only encrypted data is behind
  crypt filters authorised when an embedded file is opened; that data is left as stored.
- `decrypt_string`, `encrypt_string`, `decrypt_stream` and `encrypt_stream` take the object
  number and generation. `StreamKind` selects the crypt filter: `/StmF`, `/EFF` for embedded
  files, or a stream's own `/Crypt` filter name. `encrypts_metadata` says whether the caller
  passes `/Metadata` streams through at all. AES decryption is tolerant: a trailing partial
  block is ignored and bad padding is left in.
- `permissions()` gives the `/P` flags; for R5/R6 a permission counts only when `/Perms` grants
  it too. Owners have every permission.
- `create(&NewEncryption, id0)` builds the `/Encrypt` values for RC4-40, RC4-128, AES-128 or
  AES-256. The caller supplies 32 bytes of entropy (`seed`), so the module needs no OS access
  and builds for the web. `file_id` derives a file identifier from the same seed.
- `EncryptDict` holds the dictionary's values. The caller reads them from, and writes them to,
  its own object model; the module does not parse PDF syntax. Keys and prepared passwords are
  zeroized when dropped.

The code and its tests come from PdfCraft's `pdfcraft-crypt` crate (revision
`2b65ab000bfe0997dec5fbed2769d3da3799355b`), unchanged apart from the module paths. Tests:
`cargo test -p craft-pdf --no-default-features --features crypt`. `tests/crypt/qpdf_literals.rs`
checks key derivation against values another implementation wrote; only the numbers are stored.

## Writing

Plain `write` retains Krilla raster images without its optional text shaper. Applications
that supply shaped glyphs do not need that shaper. Adapters that call `draw_text`, such as
DeckCraft, also need `write-simple-text` when adopting this revision. `write-pdf` adds
embedded-PDF support independently of text shaping.

The writer pins Krilla 0.8.2, with local unwind-cleanup, print color and optional content patches documented in
[`vendor/krilla/README.craft.md`](../../vendor/krilla/README.craft.md). Successful
serialization uses the same backend version as before extraction. It provides:

- `write::configuration`: version and archival validation configuration with typed failures.
- `write::Metadata` and `write::Writer`: common metadata setup and consuming serialization;
  compression, colour profiles and other serialization settings pass through unchanged.
  `Writer::new_with_language` also accepts an optional document language for the catalog
  and XMP metadata; empty language is treated as absent, while callers retain
  selection and validation policy for nonempty tags.
- `write::font`: fallible font copying/preparation for subset embedding, with a 512 MiB limit
  per font, at most 256 variation coordinates, and rejection of non-finite coordinates.
- `write::guard`: a native unwind guard for the whole app export, including drawing and
  postprocessing. App errors pass through; serialization errors retain backend diagnostics.
  Caught panic messages retain up to 1024 Unicode scalar values; the installed panic hook
  remains responsible for logging source locations and full diagnostics.
- `write::uri_target`: the Krilla link target for a URI link, or `None` when `safe_url`
  refuses it. Apps build URI links with it instead of `backend::action::LinkAction`, so a
  link target from someone else's document becomes a PDF link only when it is an `http`/`https`
  URL with a host or a `mailto` address. A refused target writes no link annotation.
- `safe_url` (crate root, any feature): the same rule, for apps that open links themselves,
  such as a slide show.

### Print color

`write::print::cmyk_blending(profile)` returns `Result<SerializeSettings, WriteError>`
with the settings for a plain CMYK print PDF: DeviceCMYK color and transparency blending in
CMYK (`GroupColorSpace::Cmyk`), with no output intent. `write::print::settings` adds the
document's CMYK profile as a `GTS_PDFX` output intent and `/Trapped /False`, for PDF/X. Both
fail with `WriteError::InvalidProfile` when the profile is not a readable CMYK ICC profile.
The profile is the document's working space. craft-pdf takes the profile bytes and does not
depend on craft-cms. Its CC0 `builtin::COATED_FOGRA39_ICC` is the default working space, described by
`PrintColor::fogra39`:

```rust
use craft_pdf::write::print::{self, PrintColor};

let settings = print::settings(PrintColor::fogra39(craft_cms::builtin::COATED_FOGRA39_ICC))?;
let writer = Writer::new(SerializeSettings { configuration, ..settings }, metadata);
```

For another profile, fill `PrintColor` with its condition identifier (`Custom` when it is
not registered) and `info`. The settings claim no PDF/X version and are not checked against
PDF/X; apps that claim conformance set `pdfx_version` (and `pdfx_conformance` for PDF/X-1a
and X-3) and keep their own checks. Writing these
settings needs the patched Krilla (`CRAFT_PRINT_COLOR`, see
[`vendor/krilla/README.craft.md`](../../vendor/krilla/README.craft.md)).

### Optional content

Drawing layers are written as optional content groups. `Document::add_optional_content_group(name,
visible)` returns an `OcgId`; `Surface::start_optional_content(id)` and
`Surface::end_optional_content()` mark the content between them, and nest independently of
tagged sections. Both return `KrillaError` on misuse instead of panicking, and a mark left open
when its stream finishes fails `Writer::finish`. The catalog lists every group in
`/OCProperties`, hidden ones in `/OFF`; `Document::set_optional_content_page_mode(true)` adds
`/PageMode /UseOC`. Optional content needs PDF 1.5 and fails PDF/A-1 validation (patch 12 in
[`vendor/krilla/README.craft.md`](../../vendor/krilla/README.craft.md), marker
`CRAFT_OPTIONAL_CONTENT`).

`Writer::document_mut` and `write::backend` deliberately expose Krilla's page, resource and
painting APIs. The apps keep their document walks, pagination, font selection/licensing,
outline fallbacks and warnings, colour policy, PDF/X postprocessing, tagging semantics,
links, export presets and native editing payloads. Shared writing does not introduce a
universal document model or claim additional format conformance. Dates and branding are
provided by callers; the shared writer does not read a clock.

`Writer::finish` consumes its document even on error. When `guard` returns an internal failure,
discard any writer or drawing state captured by the closure; construct it inside the closure
where possible. Failed page/surface cleanup poisons the backend document, so retaining it
after a narrow catch cannot silently serialize a missing page. This also rejects failure
during explicit surface finalization. Other captured app state must still be discarded.
Native unwind recovery cannot catch WASM aborts or process-wide out-of-memory
failures. Font preparation bounds do not impose a total memory, output-size or time budget on
PDF serialization. Individual unsupported fonts still follow each app's existing outline
fallback and warning behavior.

Run `cargo test -p craft-pdf --no-default-features --features write`, and repeat with
`--features write-pdf` and `--all-features`. The same writer contracts run with
`--target wasm32-unknown-unknown` and a matching `wasm-bindgen-test-runner`. They exercise
metadata, page sizes, versions, reproducibility, typed validation failures, malformed fonts,
application error preservation, embedded pages, and native-only panic recovery.


## Source pinning and local development

Consumers pin both the facade and patched writer to the same full commit:

```toml
[workspace.dependencies]
craft-pdf = { git = "https://github.com/storytold/craft-libs", rev = "<full-commit-id>", version = "=0.1.0", default-features = false, features = ["write"] }

[patch.crates-io]
krilla = { git = "https://github.com/storytold/craft-libs", rev = "<full-commit-id>", version = "=0.8.2" }
```

Choose `write-pdf` only when embedding PDF pages. Cargo ignores dependency workspaces'
root patches; copy the Krilla patch into the consuming workspace. A compile-time marker
rejects unpatched Krilla. Check the normal dependency graph separately from the test graph
and ensure there is one Krilla source, including uses through `krilla-svg`. This writer
migration does not change an application's reader backend or its other craft-libs pins.
The selected revision must be remotely available before fresh consumers can resolve it.
Local path replacements are development-only and should not be committed to an app.

Run `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings` and
`cargo test --workspace`, then test `craft-pdf` with `--features write` and
`--features write-pdf` (the default feature set intentionally enables no PDF operations).
Run the same writer contracts on `wasm32-unknown-unknown` with a matching
`wasm-bindgen-test-runner`. Native recovery tests are subprocess or unwind checks;
WASM tests exercise successful paths and typed input errors, not recovery from aborts.

## Unicode and font contracts

The writer embeds the font and glyph run supplied by the application. Applications
must select a font that covers the requested characters, shape text and resolve
bidirectional layout before drawing. Passing correct Unicode only in document
metadata does not preserve page text. A selectable-text export must retain the
actual glyph outlines and a matching ToUnicode map, and applications must report
unsupported font preparation rather than silently dropping characters. Existing
app-specific outline fallback remains an explicit policy choice.

`tests/unicode_writing.rs` adds opt-in, externally supplied font fixtures. It exports,
reopens and interprets real page text without putting the source text in metadata,
checks nonzero glyph IDs, embedded fonts and visible non-space outlines, and asserts
exact extraction for Latin accents, Greek, Cyrillic, Vietnamese, Arabic and Thai.
A variable-font test checks default/empty coordinates and distinct light/bold
embedded outlines while preserving extraction. These are embedding/mapping
contracts; they do not qualify Arabic joining, RTL caret behavior, shaping quality,
or lossless text editing in an application's separate COS editor.

No PDF test fonts are stored in this repository. Obtain the licensed fixtures listed in
[`tests/fonts.json`](tests/fonts.json), verify their SHA-256 hashes, place them in
one directory using the listed names, then run:

```sh
CRAFT_PDF_TEST_FONT_DIR=/absolute/path/to/licensed-fonts \
RUSTFLAGS="--cfg craft_pdf_unicode_fixtures" \
cargo test -p craft-pdf --all-features --test unicode_writing
```

Repeat with `--target wasm32-unknown-unknown` and the matching test runner for
runtime WASM coverage. Ordinary builds and tests do not require these fonts or the
fixture environment variable. With write enabled, the complete test code is
compiled even without fixtures; its two runtime tests are explicitly ignored with a
provisioning reason. Running ignored tests without fixtures fails with setup instructions.
The Shared PDF CI workflow fetches and hash-verifies these fonts outside the checkout
and enables the contracts on native and WASM. Locally, run
`python3 scripts/fetch-pdf-test-fonts.py /absolute/path/to/licensed-fonts` to provision
them. `tests/upstream_reading.rs` uses the same Inter fixture to verify TrueType
WinAnsi fallback with missing or partial ToUnicode maps, explicit Encoding and
Differences precedence, and a ToUnicode override. The fallback changes extraction
only: existing nonsymbolic glyph selection can still choose StandardEncoding glyphs
or `.notdef`. Tests pin that distinction and the upstream WinAnsi unused-code bullet
mapping; fallback text is not proof of the original author's text. Without fixtures those reader
contracts are also compiled and explicitly ignored; provisioned CI executes them.
