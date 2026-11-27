# Local kurbo 0.13.1 copy

This is the crates.io kurbo 0.13.1 package (MIT OR Apache-2.0) with no file changed.
`.cargo_vcs_info.json` names the commit it was packaged from,
`838b69ef35f3381cefc007d86aa8364f5a6dc673`, the `v0.13.1` tag of
[linebender/kurbo](https://github.com/linebender/kurbo/tree/838b69ef35f3381cefc007d86aa8364f5a6dc673/kurbo).

Every file here except this one and `SOURCE.json` is byte for byte the file in the `.crate` archive.
`SOURCE.json` records the hash of the `.crate` download (`source_archive_sha256`) and, for every file
kept, the hash in the package (`source_sha256`) and the hash here (`sha256`). `local_patches` is
empty, and no file of the package is left out.

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

`scripts/test-pdf-backends.sh` runs the package's unit and doc tests with every feature, and its
other backend tests build against this copy. The workspace's own tests also build against it through
the patch entry. No workflow runs clippy on this copy. Run the tests and the builds without `std` and
for Wasm from this directory:

```sh
cargo test --locked --all-features
cargo check --locked --no-default-features --features libm
cargo test --locked --all-features --target wasm32-unknown-unknown --no-run
```

The package's tests need the `std` feature. Clippy 1.100.0 with `-D warnings` stops at one lint in the
package as published, `needless_late_init` in `src/common.rs`; the copy keeps that code unchanged.

Cargo writes to `target/` in this directory, which the root `.gitignore` excludes.
