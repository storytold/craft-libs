# Local kurbo 0.13.1 copy

This is the crates.io kurbo 0.13.1 package (MIT OR Apache-2.0) with the local patches listed below.
`.cargo_vcs_info.json` names the commit it was packaged from,
`838b69ef35f3381cefc007d86aa8364f5a6dc673`, the `v0.13.1` tag of
[linebender/kurbo](https://github.com/linebender/kurbo/tree/838b69ef35f3381cefc007d86aa8364f5a6dc673/kurbo).

The patched file is `src/stroke.rs`.

Every file here except this one, `SOURCE.json` and the patched file is byte for byte the file in the
`.crate` archive. `SOURCE.json` records the hash of the `.crate` download (`source_archive_sha256`)
and, for every file kept, the hash in the package (`source_sha256`) and the hash here (`sha256`).
`local_patches` describes each local change. No file of the package is left out.

## Local changes

| File | Change |
|---|---|
| `src/stroke.rs` | `StrokeCtx::do_linear` skips a cusp, or the end point, whose tangent from the point before it is exactly zero, instead of passing the tangent to `do_join` and `do_line`, which divide by its length. After rounding, a collinear cubic that runs back and forth can put two cusps at the same point, or a cusp on the end point, and the whole outline then held NaN coordinates. The `linear_cubic_with_coincident_cusps` test strokes one cubic of each kind with every join, two widths and two tolerances. Code comments label the change `Craft patch (zero tangent)`. |
| `src/stroke.rs` | The stroker skips a quadratic or cubic whose start or end tangent is zero, as it skips a zero-length line. `PathSeg::tangents` falls back to the chord when the control arms are shorter than 1e-6, and a curve that ends where it starts has a zero chord, so such a closed loop gave NaN coordinates in the outline. A quadratic is stroked as the cubic it raises to, whose control arms are 2/3 as long, so the quadratic's own tangents are passed on to `do_cubic` and `do_linear`. The `tiny_closed_loops` test strokes loops with arms of 1e-7, 1.25e-6 and 1e-5, on their own and after a line. Code comments label the change `Craft patch (zero tangent)`. |

Every change here updates this table and `SOURCE.json` in the same commit.

## Using it

The directory is excluded from the workspace. With the root manifest's `[patch.crates-io]` entry,
every crate here that depends on kurbo 0.13 builds against this copy. Cargo ignores a dependency's
patches, so a consuming workspace adds its own entry:

```toml
[patch.crates-io]
kurbo = { git = "https://github.com/storytold/craft-libs", rev = "<rev>" }
```

The entry applies to kurbo 0.13 only. A dependency on another kurbo version, such as 0.11, still
resolves to crates.io.

## Tests

`scripts/test-pdf-backends.sh` runs the package's unit and doc tests with every feature, and builds
its `hayro` and `hayro-interpret` tests against this copy. The workspace's own tests also build
against it through the patch entry. No workflow runs clippy on this copy. Run the tests and the
builds without `std` and for Wasm from this directory:

```sh
cargo test --locked --all-features
cargo check --locked --no-default-features --features libm
cargo test --locked --all-features --target wasm32-unknown-unknown --no-run
```

The package's tests need the `std` feature. Clippy 1.100.0 with `-D warnings` reports two lints in
the package as published, `needless_late_init` in `src/common.rs` and `useless_borrows_in_formatting`
in a test in `src/cubicbez.rs`; the copy keeps that code unchanged.

Cargo writes to `target/` in this directory, which the root `.gitignore` excludes.
