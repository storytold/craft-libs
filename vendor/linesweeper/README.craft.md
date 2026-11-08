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
| `src/lib.rs` | `binary_op_with_eps` returns the new `Error::InvalidEps` when `eps` is NaN, infinite, zero or negative, instead of returning a wrong region (NaN or negative `eps`) or hanging (`-inf`). The `binary_op_with_eps_rejects_invalid_eps`, `topology_raises_unusable_eps` and `binary_op_with_eps_raises_eps_below_coordinate_resolution` tests cover the changes in this file, `src/segments.rs`, `src/sweep/sweep_line.rs` and `src/topology.rs`. |
| `src/segments.rs`, `src/sweep/sweep_line.rs`, `src/topology.rs` | The `Topology` constructors and `Sweeper::new_with_correctness` raise an `eps` that is NaN, not positive, or below `64 * f64::EPSILON` times the largest coordinate magnitude (and at least 64 times the spacing of subnormal floats) to that value, the scale `binary_op` uses. Before, an `eps` below one ulp of the coordinates (0 included, or 1e-10 at 1.5e8) overflowed the stack and aborted the process. |
| `src/curve/mod.rs` | `intersect_cubics` and `intersect_cubics_with_precomputed_decomp` raise a positive `ish_tolerance` below `4 * f64::EPSILON` times the two curves' largest x coordinate magnitude (two to four ulps of it, and at least four times the spacing of subnormal floats) to that value, and a positive `sure_tolerance` below half of it to half of it. Below that, rounding made the comparison report the wrong order. `intersect_cubics_rec` returns close (`Ish`) for an interval whose midpoint rounds to one of its ends instead of splitting it again, which repeated the same call until the stack overflowed. The `ish_tolerance_below_float_spacing` test covers both. `raised_tolerance_keeps_far_curves_apart`, `raised_tolerance_ignores_large_y` and `raised_tolerance_keeps_subnormal_curves_apart` check that lines 1.0 apart at x = 1e15, lines 1.0 apart at y = 2e15, and lines at x = 1e-310 and 2e-310 stay ordered. |
| `src/position.rs` | `approximate` stops halving an interval whose midpoint rounds to one of its ends. Before, it could return the interval's start, and the loop in `compute_positions_correct` then made no progress. The `approx_stops_at_one_float` test covers it. |

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
