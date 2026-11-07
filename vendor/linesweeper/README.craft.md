# Local linesweeper 0.5.0 copy

This is the crates.io linesweeper 0.5.0 package (MIT OR Apache-2.0) with some files left out and the
local patches listed below. `.cargo_vcs_info.json` names the commit it was packaged from,
`824b0791a750f7b27b8edcbb6c6244d0e629875e`. That commit is not in the GitHub repository the manifest
names, which is archived; its README says development moved off GitHub.

Every file here except this one, `SOURCE.json` and the patched files below is byte for byte the file
in the `.crate` archive. `SOURCE.json` records the hash of the `.crate` download
(`source_archive_sha256`) and, for every file kept, the hash in the package (`source_sha256`) and the
hash here (`sha256`). `upstream_repository` is null because no public repository this copy can name
holds `upstream_revision`. `local_patches` describes each local patch.

## Local patches

| File | Change |
|---|---|
| `src/lib.rs`, `src/topology.rs` | `Topology::from_paths`, `from_paths_with_correctness`, `from_path` and `from_paths_binary` return `Error` instead of `NonClosedPath`, and return `Error::NaN`, `Error::Infinity` or the new `Error::TooLarge` when a path has a coordinate that is not finite or has a magnitude above `MAX_COORDINATE` (`f64::MAX / 16`). `binary_op` returns `Error::TooLarge` through `from_paths_binary`. Before, both panicked on a cubic with a coordinate above about `f64::MAX / 3`. A release build panicked in `f64::clamp` with "min > max, or either was NaN", and a debug build failed the `y.is_finite()` assertion in `Point::new`. The `coordinates_above_max_coordinate` test covers 6e307, 7e307 and 1.7e308. |

`omitted_files` lists the 43 files of the package left out here, with their hashes:

- `tests/fonts/Inconsolata-Regular.ttf`: font files live in craft-fonts.
- `docs/assets/logo.svg` and `examples/linebender.svg`: the package gives no license for this artwork
  apart from its `license` field.
- `tests/snapshots/`: 12 SVG and 20 PNG files.
- `.config/`, `.helix/`, `.typos.toml`, `.gitmodules`, `flake.nix`, `flake.lock` and `nix/`: the
  package's repository tooling.

Only the targets listed under Tests read the font, the logos and the snapshot files, and those targets
do not compile from the published package.

## Using it

The directory is excluded from the workspace. With the root manifest's `[patch.crates-io]` entry,
`craft-prepress` builds its `flatten` feature against this copy. Cargo ignores a dependency's patches,
so a consuming workspace adds its own entry:

```toml
[patch.crates-io]
linesweeper = { git = "https://github.com/storytold/craft-libs", rev = "<rev>" }
```

## Tests

`scripts/test-pdf-backends.sh` runs the package's unit, regression and doc tests. The `craft-prepress`
tests with the `flatten` feature run against this copy. `pdf.yml`'s path filter does not list this
directory, so a pull request that changes only files here does not start `pdf.yml`. Run the tests from
this directory:

```sh
cargo test --locked --lib --test regression
cargo test --locked --doc
```

`Cargo.toml` leaves out the `kompari`, `linesweeper_util` and `path-bool` dev-dependencies, which
`Cargo.toml.orig` gives as git or path dependencies. The `fonts` and `snapshots` tests, the
`winding-numbers` and `boolean_op` examples and the `linebender_logo` bench do not compile here.

Cargo writes to `target/` in this directory, which the root `.gitignore` excludes.
