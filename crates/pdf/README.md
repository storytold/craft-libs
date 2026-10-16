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

## Reading and rendering

`open` accepts shared bytes without copying. `open_slice` reserves storage fallibly before
copying. Both preserve typed password errors. Their `_with_options` variants accept
`OpenOptions`: a caller-owned input limit and immutable per-document `syntax::DecodeLimits`.
The default input limit is 512 MiB; callers can preserve their own accepted-file policy.
Ordinary Flate/LZW/RunLength expansion defaults to 256 MiB per stream and can be lowered,
including during initial xref reads, object streams, repair and fallback. Known-size image
pixels and streaming page content have separate backend budgets. These controls do not bound
total parser memory or execution time.

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
Cancellation is polled between operators, so a decoder may finish before noticing it.

## Backend provenance and consumer pins

The shared reader vendors the current patched Hayro trees from PdfCraft revision
`7437bde378f43856eada40af25b983812e615e8a`. See
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
Run `bash scripts/test-pdf-backends.sh` from the repository root to test all four vendored
backends, the syntax doctest and the syntax-only configuration. The script uses temporary
source copies and the complete local patch set, preserving the vendored lockfiles.
Run `cargo test --manifest-path crates/pdf/read-only-contract/Cargo.toml --locked` for the
read-only consumer, then repeat with `--target wasm32-unknown-unknown` and the test runner.
These backend checks supplement the root workspace gates; excluded vendor crates are not
covered by `cargo test --workspace` alone.

## Writing

Plain `write` retains Krilla raster images without its optional text shaper. Applications
that supply shaped glyphs do not need that shaper. Adapters that call `draw_text`, such as
DeckCraft, also need `write-simple-text` when adopting this revision. `write-pdf` adds
embedded-PDF support independently of text shaping.

The writer pins Krilla 0.8.2, with a local unwind-cleanup patch documented in
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
