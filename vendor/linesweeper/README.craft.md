# Local linesweeper 0.5.0 copy

This is the crates.io linesweeper 0.5.0 package (MIT OR Apache-2.0) with some files left out and the
local patches listed below. `.cargo_vcs_info.json` names the commit it was packaged from,
`824b0791a750f7b27b8edcbb6c6244d0e629875e`. That commit is not in the GitHub repository the manifest
names, which is archived; its README says development moved off GitHub.

The patched files are `docs/sweep.typ`, `src/curve/line.rs`, `src/curve/mod.rs`, `src/order.rs`,
`src/sweep/sweep_line.rs` and `src/topology.rs`.

Every file here except this one, `SOURCE.json` and the patched files below is byte for byte the file
in the `.crate` archive. `SOURCE.json` records the hash of the `.crate` download
(`source_archive_sha256`) and, for every file kept, the hash in the package (`source_sha256`) and the
hash here (`sha256`). `upstream_repository` is null because no public repository this copy can name
holds `upstream_revision`. `local_patches` describes each local patch.

## Local patches

| File | Change |
|---|---|
| `src/sweep/sweep_line.rs` | The doc comment of `SweepLine::line_segment` says it can return a segment that has exited, which `segment_range` leaves out. Code comments label the change `Craft patch (exited slots)`. |
| `src/order.rs`, `src/sweep/sweep_line.rs` | Segments that start at one point all compare "ish" there, so each intersection scan and insertion search run at that point passes every other segment that starts there. Those comparisons are no longer stored: `ComparisonCache::with_order` uses a stored comparison if there is one and otherwise computes it the same way and drops it. `n` segments that start at one point and do not cross later now store `O(n)` comparisons instead of `O(n^2)` (comparisons made at a later crossing are stored as before); 1,000 triangles that share their apex peak at 8 MiB instead of 2,431 MiB and sweep in 0.56 s instead of 7.7 s. The comparisons are the same, so the output does not change. The `segments_starting_at_one_point_are_not_stored` and `curves_starting_at_one_point_are_not_stored` tests count the stored comparisons, `segments_starting_at_one_point_sweep_as_with_stored_comparisons` checks that every sweep line is the same as with all comparisons stored, and the `order` test `unstored_order_matches_stored_order` checks `with_order` against `compare_segments`. |
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
