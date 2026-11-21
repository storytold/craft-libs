# Local linesweeper 0.5.0 copy

This is the crates.io linesweeper 0.5.0 package (MIT OR Apache-2.0) with some files left out and the
local patches listed below, to `src/lib.rs`, `src/order.rs`, `src/sweep/range.rs`,
`src/sweep/sweep_line.rs`, `src/topology.rs`, `src/curve/mod.rs`, `src/curve/line.rs` and
`docs/sweep.typ`. `.cargo_vcs_info.json` names the commit it was packaged from,
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
| `src/sweep/sweep_line.rs` | The doc comment of `SweepLine::line_segment` says it can return a segment that has exited, which `segment_range` leaves out. Code comments label the change `Craft patch (exited slots)`. |
| `src/lib.rs`, `src/order.rs`, `src/sweep/range.rs`, `src/sweep/sweep_line.rs`, `src/topology.rs` | `ComparisonCache` counts its calls, so `Sweeper` counts its segment comparisons, and `Sweeper` also counts the segment positions it checks against horizontal segments and contour endpoints while finding a sweep line's changed intervals, n log2 n units for sorting a sweep line's n horizontal segments, one unit for every 16 segments that linear searches of the sweep line step over, and the active horizontal segments that each step of a `SweepLineRange` visits. `SweepLineRange::work_limit_exceeded` reports the limit to the code that processes a range, and the topology stops there; once a range step passes the limit, the sweep stays stopped if the limit is raised. `Sweeper::set_work_limit` sets the most work it may do; past it, `Sweeper::next_line` returns `None`, also between the enter, exit and intersection events of one sweep line and between the comparisons and position checks that find its changed intervals, never returns a sweep line past the limit, and keeps returning `None` if the limit is raised; `Sweeper::work_limit_exceeded` returns `true`. `Topology::from_paths_with_work_limit` returns the new `Error::WorkLimitExceeded` when its sweep passes the limit. `Topology::from_paths_with_limits` also returns it before the topology's output passes its `output_limit`, counting one unit per output segment plus the new `WindingNumber::storage_units` of its two winding numbers (1 by default), which bounds the memory the topology uses; with an output limit, the topology also preallocates its output vectors for no more than that many segments. The sweep had no limit: the union of 400 identical circles made 1.36 billion comparisons in 19 sweep lines and took 66 to 83 s and 1.3 GB in a release build. `Topology::from_paths` and the other constructors set no limit. The `work_limit` and `work_limit_stops_within_a_sweep_line` tests sweep 20 stacked circles with a limit of 1,000 comparisons, and `work_limit_returns_no_line_past_the_limit` sweeps 3 stacked circles with every limit below the sweep's total. `work_limit_counts_the_horizontal_scans` sweeps 150 bars crossed by the top and bottom edges of 30 wide rectangles, where the sweep makes 1,462 comparisons but checks at least 18,000 positions, and stops it at half its total work. `work_limit_counts_linear_searches` sweeps 2,000 disjoint rectangles sharing their top and bottom heights, where each exit searches the whole sweep line, and stops it at half its total work. `work_limit_counts_sorting_the_horizontals` sweeps 4,096 horizontal segments at one height with an empty sweep line, and stops it before the sort at half its total work. `work_limit_stays_exceeded_after_a_range_passes_it` steps a range of 300 overlapping horizontal segments past the limit, raises the limit, and checks that `Sweeper::next_line` still returns `None`. `work_limit_counts_the_horizontal_splits` builds the topology of 300 closed paths made only of overlapping horizontal segments at one height, which take little sweep work but make more than 100,000 output segments, and returns `Error::WorkLimitExceeded` at a limit of 100,000. `output_limit` builds the same paths with an output limit of 10,000 units, of one unit less than three per output segment, and of exactly three per output segment. `unlimited_constructors_ignore_storage_units` builds a square with a winding number whose `storage_units` is `usize::MAX` through `Topology::from_paths` and `Topology::from_paths_with_work_limit`, which count no output units. |
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
