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
| `src/curve/mod.rs` | `fit_quadratic` multiplies the heights by the power of two that brings the largest of them to between 2 and 4 when one of them has a magnitude above 2^500, and scales the quadratic's coefficients back at the end. `QuadraticApprox::from_cubic` multiplies by the quadratic's `y^2` coefficient before the second y coefficient when the product of two y coefficients is not finite. Before, a curve taller than about 1.3e154 gave a product of two heights that overflowed to infinity, and `inf * 0` gave a NaN coefficient, or an error bound that left out the curve's extremes. The curve comparison then reported a definite order that could be wrong, and `binary_op` returned a wrong result without an error. The quadratic sign checks used by curve comparisons now report the order as unknown (or close) over the whole interval when a coefficient is NaN or infinite, instead of a definite order. The `far_apart_with_huge_coordinates` test compares two curves with heights up to 1e155, 1e200 and 1e300, `tall_lines_at_any_height` compares two lines from 1 to 1e300 tall at three heights, `quadratic_approx_error_bounds_on_tall_curves` checks the coefficients and error bounds of an S-shaped curve up to 1e300 tall and 1e300 wide, and `non_finite_quadratics_have_no_definite_order` checks the sign checks. |
| `src/curve/line.rs` | `intersect_lines` compares the x coordinates and the tolerance times 1/16 when one of them has a magnitude above 2^1019, and works with half of each height for a line or height range more than `f64::MAX` tall. Before, two vertical lines at x = -1e308 and x = 1e308 came back `Right`, because the difference of their x values overflowed, and lines from y = -1e308 to 1e308 gave NaN positions. The `unchanging_with_huge_coordinates` test covers 1e300, 1e308 and `f64::MAX`, and `huge_coordinates_on_one_axis` covers lines near x = 1e308 that are 1e-307 tall, lines from y = -1e308 to 1e308 that stay apart or cross, and a line 1e-308 tall beside one of them. |
| `src/geom.rs` | For a straight segment more than `f64::MAX` long on an axis, `Segment::at_y` works with half of each coordinate on that axis and `Segment::to_kurbo_cubic` takes weighted sums of the end points on that axis. Before, both returned NaN or infinite coordinates for such a segment. The `line_wider_and_taller_than_f64_max` test covers a segment from (-1e308, -1e308) to (1e308, 1e308), and `line_wider_than_f64_max_and_very_short` covers a segment from (-1e308, 0) to (1e308, 3 times the smallest positive `f64`). |
| `src/lib.rs` | The `union_with_huge_coordinates` test checks the union of a circle and a rectangle at sizes 1e155, 1e200 and 1e300. |

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
