# Local linesweeper 0.5.0 copy

This is the crates.io linesweeper 0.5.0 package (MIT OR Apache-2.0) with some files left out and the
local patches listed below. `.cargo_vcs_info.json` names the commit it was packaged from,
`824b0791a750f7b27b8edcbb6c6244d0e629875e`. That commit is not in the GitHub repository the manifest
names, which is archived; its README says development moved off GitHub.

The patched files are `src/sweep/sweep_line.rs`, `src/topology.rs` and `docs/sweep.typ`.

Every file here except this one, `SOURCE.json` and the patched files below is byte for byte the file
in the `.crate` archive. `SOURCE.json` records the hash of the `.crate` download
(`source_archive_sha256`) and, for every file kept, the hash in the package (`source_sha256`) and the
hash here (`sha256`). `upstream_repository` is null because no public repository this copy can name
holds `upstream_revision`. `local_patches` describes each local patch.

## Local patches

| File | Change |
|---|---|
| `src/sweep/sweep_line.rs` | The doc comment of `SweepLine::line_segment` says it can return a segment that has exited, which `segment_range` leaves out. Code comments label the change `Craft patch (exited slots)`. |
| `src/curve/mod.rs` | `fit_quadratic` multiplies the heights by the power of two that brings the largest of them to between 2 and 4 when one of them has a magnitude above 2^500, does the same to the x coordinates when one of them has a magnitude above 2^500, and scales the quadratic's coefficients back at the end. `QuadraticApprox::from_cubic` multiplies by the quadratic's `y^2` coefficient before the second y coefficient when the product of two y coefficients is not finite. Before, a curve taller than about 1.3e154 gave a product of two heights that overflowed to infinity, and `inf * 0` gave a NaN coefficient, or an error bound that left out the curve's extremes. The curve comparison then reported a definite order that could be wrong, and `binary_op` returned a wrong result without an error. The quadratic sign checks used by curve comparisons now report the order as unknown (or close) over the whole interval when a coefficient is NaN or infinite, instead of a definite order. The `far_apart_with_huge_coordinates` test compares two curves with heights up to 1e155, 1e200 and 1e300, `tall_lines_at_any_height` compares two lines from 1 to 1e300 tall at three heights, `quadratic_approx_error_bounds_on_tall_curves` checks the coefficients and error bounds of an S-shaped curve up to 1e300 tall and 1e300 wide, `non_finite_quadratics_have_no_definite_order` checks the sign checks, and `fit_quadratic_with_huge_x` fits vertical curves at x = 1e307, -1e307 and 5e307 that are 1, 2^501 and 1e300 tall. |
| `src/curve/line.rs` | `intersect_lines` compares the x coordinates and the tolerance times 1/16 when one of them has a magnitude above 2^1019, works with half of each height for a line or height range more than `f64::MAX` tall, and divides by a line's height when the height's reciprocal is not finite. Before, two vertical lines at x = -1e308 and x = 1e308 came back `Right`, because the difference of their x values overflowed, and lines from y = -1e308 to 1e308 gave NaN positions. The `unchanging_with_huge_coordinates` test covers 1e300, 1e308 and `f64::MAX`, and `huge_coordinates_on_one_axis` covers lines near x = 1e308 that are 1e-307 tall, lines from y = -1e308 to 1e308 that stay apart or cross, and a line 1e-308 tall beside one of them, and `subnormal_height` covers two vertical lines 1e-310 tall. |
| `src/geom.rs` | For a straight segment more than `f64::MAX` long on an axis, `Segment::at_y` works with half of each height and takes a weighted sum of the x coordinates, and `Segment::to_kurbo_cubic` takes weighted sums of the end points on that axis. Before, both returned NaN or infinite coordinates for such a segment. The `line_wider_and_taller_than_f64_max` test covers a segment from (-1e308, -1e308) to (1e308, 1e308), `line_wider_than_f64_max_and_very_short` covers a segment from (-1e308, 0) to (1e308, 3 times the smallest positive `f64`), and `line_wider_than_f64_max_with_uneven_ends` covers a segment from (-2^1023, 0) to (`f64::MAX`, 1). |
| `src/lib.rs` | The `union_with_huge_coordinates` test checks the union of a circle and a rectangle at sizes 1e155, 1e200 and 1e300. |
| `src/curve/mod.rs` | When the chord that `fit_quadratic` takes for a very short curve gives a constant term that is not finite, `fit_quadratic` takes the line through the curve's start point with the chord's slope instead. Before, a curve near x = 1e305 with heights near 1e4 and less than about 3e-12 tall gave a product of an x coordinate and a height above `f64::MAX`, and the chord's constant term was NaN. The `short_curve_chord_with_huge_x` test fits vertical curves one ulp tall at y = 1e4 and x = 1e305, -1e305 and 5e307. Code comments label the change `Craft patch (huge chord)`. |
| `src/curve/line.rs` | `line_at_ys` multiplies the x coordinates by 2^600 before the weighted sum and the sum by 2^-600 after it when both x coordinates have a magnitude below 2^-500. Before, a vertical line at x = 5e-324 gave x = 0 halfway along its height. The `subnormal_x` test checks that line and a line from x = -1.5e-323 to 1.5e-323. Code comments label the change `Craft patch (subnormal x)`. |
| `docs/sweep.typ` | The intersection scan to the left compares each segment with `alpha^(i+1)` instead of `alpha^i`. With `alpha^i`, its first pass compared `alpha^i` with itself, and the lemma after it is stated for `alpha^(i+1)`. |
| `docs/sweep.typ` | Two typos are corrected. "condi0.2tions" reads "conditions", and `q(i+i)` reads `q(i+1)`, the first index for which `q` is false. |
| `docs/sweep.typ` | The lemma for the intersection scan to the left asks for the ordering invariant on `(alpha^1, ..., alpha^(i+1))` and the crossing invariant on `(alpha^1, ..., alpha^i)`, which is how the bidirectional lemma uses it. The argument for the order invariant cites the crossing invariant instead of a "third invariant"; the document defines two. |
| `src/sweep/sweep_line.rs`, `src/topology.rs` | Clippy 1.100.0 with `-D warnings` and every feature passes on the library, its unit tests and the `regression` test: the `slow-asserts` invariant check in `sweep_line.rs` passes `self.line` to `assert!` without a `&` (`useless_borrows_in_formatting`), and `Contours::grouped` iterates over the contours with `enumerate` instead of indexing them (`needless_range_loop`). The behavior does not change. Code comments label the change `Craft patch (clippy lints)`. |

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
tests with the `flatten` feature run against this copy. `pdf.yml`'s path filter lists this directory,
so a pull request that changes files here starts `pdf.yml`. No workflow runs clippy on this copy. Run
the tests and clippy from this directory:

```sh
cargo test --locked --lib --test regression
cargo test --locked --doc
cargo clippy --locked --all-features --lib --test regression -- -D warnings
cargo clippy --locked --all-features --lib --profile test -- -D warnings
```

`Cargo.toml` leaves out the `kompari`, `linesweeper_util` and `path-bool` dev-dependencies, which
`Cargo.toml.orig` gives as git or path dependencies. The `fonts` and `snapshots` tests, the
`winding-numbers` and `boolean_op` examples and the `linebender_logo` bench do not compile here.

Cargo writes to `target/` in this directory, which the root `.gitignore` excludes.
