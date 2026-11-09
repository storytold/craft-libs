# Local linesweeper 0.5.0 copy

This is the crates.io linesweeper 0.5.0 package (MIT OR Apache-2.0) with some files left out and the
changes listed under Local changes. `.cargo_vcs_info.json` names the commit it was packaged from,
`824b0791a750f7b27b8edcbb6c6244d0e629875e`. That commit is not in the GitHub repository the manifest
names, which is archived; its README says development moved off GitHub.

Every file here except this one, `SOURCE.json` and the files under Local changes is byte for byte the
file in the `.crate` archive. `SOURCE.json` records the hash of the `.crate` download
(`source_archive_sha256`) and, for every file kept, the hash in the package (`source_sha256`) and the
hash here (`sha256`). `upstream_repository` is null because no public repository this copy can name
holds `upstream_revision`. `local_patches` describes each local change.

## Local changes

| Change | Code comment label | Touches | Tests |
|---|---|---|---|
| `SegmentOrder::position` finds a segment by binary search over a key on each sweep-line entry | `Craft patch (sweep position)` | `src/sweep/sweep_line.rs` | `position_after_edits`, `position_after_repeated_insertion`, `position_after_insertion_next_to_the_extreme_keys`, `position_after_moving_a_contour_continuation`, `position_on_a_wide_sweep_line` |
| `ComparisonCache` skips the curve comparison for segments whose horizontal bounds are far apart | `Craft patch (far-apart comparison)` | `src/order.rs`, `src/curve/mod.rs` | `far_apart_order`, `far_apart_curves_compare_like_the_full_comparison`, `far_apart_steep_curves_never_reverse`, `far_apart_steep_curves_skip_the_full_comparison` |
| `Metrics::entries` holds two entries inline, as a `SmallVec` | `Craft patch (inline metrics)` | `src/curve/mod.rs` | `cargo check --example curve_metrics` |

- `docs/sweep.typ`: the intersection scan to the left compares each segment with `alpha^(i+1)`
  instead of `alpha^i`. With `alpha^i`, its first pass compared `alpha^i` with itself, and the lemma
  after it is stated for `alpha^(i+1)`.

| File | Change |
|---|---|
| `src/curve/line.rs` | `line_at_ys` divides each `y` offset by the line's height instead of multiplying it by the height's reciprocal. For a height below about 5.6e-309 the reciprocal is infinite, and `intersect_lines` returned NaN positions and failed its `z0 >= z1` debug assertion. The `subnormal_span` test covers two vertical lines 1e-310 tall. |
| `F64Perturbation::Ulp` on a subnormal value gives the value on the other side of zero when the step toward zero is larger than the value | `Craft patch (ulp across zero)` | `src/perturbation.rs` | `subnormal_ulp_step_crosses_zero` |

Every change here updates this table and `SOURCE.json` in the same commit. To take a new linesweeper
release, unpack the crates.io package, carry these changes over, regenerate `SOURCE.json`, and run the
tests below.
This is the crates.io linesweeper 0.5.0 package (MIT OR Apache-2.0) with some files left out and one
file changed. `.cargo_vcs_info.json` names the commit it was packaged from,
`824b0791a750f7b27b8edcbb6c6244d0e629875e`. That commit is not in the GitHub repository the manifest
names, which is archived; its README says development moved off GitHub.

Every file here except this one, `SOURCE.json` and `src/curve/mod.rs` is byte for byte the file in the
`.crate` archive. `SOURCE.json` records the hash of the `.crate` download (`source_archive_sha256`)
and, for every file kept, the hash in the package (`source_sha256`) and the hash here (`sha256`).
`upstream_repository` is null because no public repository this copy can name holds
`upstream_revision`. `local_patches` lists the local changes:

- `src/curve/mod.rs`: the `QuadraticApprox` doc says the quadratic gives `x` as a function of `y`, as
  `from_cubic`, `dmin` and `dmax` use it; the package's doc says `y` as a function of `x`. The test
  `curve::test::quad_approximation_gives_x_of_y` checks it on the curve `x = y^2`.
`824b0791a750f7b27b8edcbb6c6244d0e629875e`. That commit is not in the GitHub repository the manifest
names, which is archived; its README says development moved off GitHub.

`824b0791a750f7b27b8edcbb6c6244d0e629875e`. That commit is not in the GitHub repository the manifest
names, which is archived; its README says development moved off GitHub.

`824b0791a750f7b27b8edcbb6c6244d0e629875e`. That commit is not in the GitHub repository the manifest
names, which is archived; its README says development moved off GitHub.

`824b0791a750f7b27b8edcbb6c6244d0e629875e`. That commit is not in the GitHub repository the manifest
names, which is archived; its README says development moved off GitHub.

`824b0791a750f7b27b8edcbb6c6244d0e629875e`. That commit is not in the GitHub repository the manifest
names, which is archived; its README says development moved off GitHub.

`824b0791a750f7b27b8edcbb6c6244d0e629875e`. That commit is not in the GitHub repository the manifest
names, which is archived; its README says development moved off GitHub.

`824b0791a750f7b27b8edcbb6c6244d0e629875e`. That commit is not in the GitHub repository the manifest
names, which is archived; its README says development moved off GitHub.

`824b0791a750f7b27b8edcbb6c6244d0e629875e`. That commit is not in the GitHub repository the manifest
names, which is archived; its README says development moved off GitHub.

`824b0791a750f7b27b8edcbb6c6244d0e629875e`. That commit is not in the GitHub repository the manifest
names, which is archived; its README says development moved off GitHub.

`824b0791a750f7b27b8edcbb6c6244d0e629875e`. That commit is not in the GitHub repository the manifest
names, which is archived; its README says development moved off GitHub.

`824b0791a750f7b27b8edcbb6c6244d0e629875e`. That commit is not in the GitHub repository the manifest
names, which is archived; its README says development moved off GitHub.

`824b0791a750f7b27b8edcbb6c6244d0e629875e`. That commit is not in the GitHub repository the manifest
names, which is archived; its README says development moved off GitHub.

`824b0791a750f7b27b8edcbb6c6244d0e629875e`. That commit is not in the GitHub repository the manifest
names, which is archived; its README says development moved off GitHub.

`824b0791a750f7b27b8edcbb6c6244d0e629875e`. That commit is not in the GitHub repository the manifest
names, which is archived; its README says development moved off GitHub.

`824b0791a750f7b27b8edcbb6c6244d0e629875e`. That commit is not in the GitHub repository the manifest
names, which is archived; its README says development moved off GitHub.

`824b0791a750f7b27b8edcbb6c6244d0e629875e`. That commit is not in the GitHub repository the manifest
names, which is archived; its README says development moved off GitHub.

Every file here except this one, `SOURCE.json` and the files under Local changes is byte for byte the
file in the `.crate` archive. `SOURCE.json` records the hash of the `.crate` download
(`source_archive_sha256`) and, for every file kept, the hash in the package (`source_sha256`) and the
hash here (`sha256`). `upstream_repository` is null because no public repository this copy can name
holds `upstream_revision`. `local_patches` describes each local change.

## Local changes

| Change | Code comment label | Touches | Tests |
|---|---|---|---|
| The SVG stroke width is the larger of the bounding box's width and height divided by 512 | `Craft patch (example stroke width)` | `examples/boolean_op.rs`, `examples/visualize_op.rs`, `examples/winding-numbers.rs` | `stroke_width_follows_the_larger_side` in `examples/visualize_op.rs` |

Every change here updates this table and `SOURCE.json` in the same commit.
Every file here except this one, `SOURCE.json` and `Cargo.toml` is byte for byte the file in the
`.crate` archive. `SOURCE.json` records the hash of the `.crate` download (`source_archive_sha256`)
and, for every file kept, the hash in the package (`source_sha256`) and the hash here (`sha256`).
`upstream_repository` is null because no public repository this copy can name holds
`upstream_revision`. `local_patches` describes each change.
| `ComparisonCache::compare_segments` panics with a message when either segment is horizontal, in every build profile, and its doc comment states it | `Craft patch (horizontal comparison)` | `src/order.rs` | `horizontal_and_cubic`, `horizontal_and_line` |
| `compute_positions` starts each output segment's `last_y` at the segment's start height instead of 0, so a segment that starts at `y < 0` is not skipped at its first queue entry and close segments there are put in order | `Craft patch (start height)` | `src/position.rs` | `position::tests::correct_positions_at_negative_y` |

Every change here updates this table and `SOURCE.json` in the same commit.
| `Segment::lower` and `Segment::upper` bound a horizontal line by both of its endpoints | `Craft patch (horizontal line bounds)` | `src/geom.rs` | `horizontal_line_bounds_cover_both_endpoints` |
| The `visualize_op` example draws its two input paths with `fill-rule="evenodd"`, the fill rule its operation uses | `Craft patch (example fill rule)` | `examples/visualize_op.rs` | `inputs_are_drawn_with_the_operation_fill_rule`, `svg_fill_rule_names` |
| The `Ish` entry that `CurveOrder::push` inserts between opposite `Left` and `Right` intervals ends where the earlier interval ends, so it has zero height | `Craft patch (zero-height Ish)` | `src/curve/mod.rs` | `opposite_orders_get_a_zero_height_ish` |
| `float_in_range` interpolates as `start * (1 - t) + end * t`, so a range such as `-1e308..1e308` gives a value in the range instead of NaN | `Craft patch (wide float range)` | `src/arbitrary.rs` | `float_in_range_handles_a_range_wider_than_f64_max` |
| `OutputEvent`'s `Ord` breaks ties between events with the same horizontal range by segment and then by the other fields, so two events compare equal only when they are equal; the sweep line still sorts its events by horizontal range alone | `Craft patch (output event order)` | `src/sweep/output_event.rs`, `src/sweep/sweep_line.rs` | `events_on_different_segments_with_the_same_range_are_not_equal`, `events_differing_only_in_flags_are_not_equal` |
| The `paperjs` test's image comparison counts a difference in alpha | `Craft patch (image distance alpha)` | `tests/paperjs.rs` | `pixmap_distance_counts_coverage` |

Every change here updates this table and `SOURCE.json` in the same commit. To take a new linesweeper
release, unpack the crates.io package, carry these changes over, regenerate `SOURCE.json`, and run the
tests below.
| The floating-point ratio lemma assumes `fl(y_1 - y_0)` is finite and `y_0 < y_1`, and its proof bounds `fl(y_1 - y_0)` instead of `fl(y + y_0)`; the x-coordinate accuracy lemma takes `M` as a floating-point number and shows both assumptions hold | `Craft patch (ratio lemma)` | `docs/sweep.typ` | None (documentation only) |
| `Segments::add_points` skips a point equal to the one before it, as the closed-polyline path does, so it adds no zero-length segment | `Craft patch (repeated points)` | `src/segments.rs` | `segments::tests::add_points_skips_repeated_points` |

Every change here updates this table and `SOURCE.json` in the same commit.
Every file here except this one, `SOURCE.json` and `examples/curve_metrics.rs` is byte for byte the
file in the `.crate` archive. `SOURCE.json` records the hash of the `.crate` download
(`source_archive_sha256`) and, for every file kept, the hash in the package (`source_sha256`) and the
hash here (`sha256`). `upstream_repository` is null because no public repository this copy can name
holds `upstream_revision`. `local_patches` describes each change:

- `examples/curve_metrics.rs`: the SVG's `viewBox` starts at x = -1.5 instead of 0, so the left half
  of each curve, which spans x = -1 to 1, is drawn. A test checks that every curve fits in the
  `viewBox`.
| When `b * b` or `a * c` would underflow or overflow, `quadratic_signs` multiplies the coefficients by one power of two before it computes the discriminant, so a cubic with very small or very large y coordinates is split where its y direction turns | `Craft patch (quadratic scale)` | `src/curve/mod.rs` | `curve::test::quadratic_signs_at_extreme_scales` |

Every change here updates this table and `SOURCE.json` in the same commit.
local patches listed below. `.cargo_vcs_info.json` names the commit it was packaged from,
`824b0791a750f7b27b8edcbb6c6244d0e629875e`. That commit is not in the GitHub repository the manifest
names, which is archived; its README says development moved off GitHub.

local patches listed below. `.cargo_vcs_info.json` names the commit it was packaged from,
`824b0791a750f7b27b8edcbb6c6244d0e629875e`. That commit is not in the GitHub repository the manifest
names, which is archived; its README says development moved off GitHub.

local patches listed below. `.cargo_vcs_info.json` names the commit it was packaged from,
`824b0791a750f7b27b8edcbb6c6244d0e629875e`. That commit is not in the GitHub repository the manifest
names, which is archived; its README says development moved off GitHub.

local patches listed below. `.cargo_vcs_info.json` names the commit it was packaged from,
`824b0791a750f7b27b8edcbb6c6244d0e629875e`. That commit is not in the GitHub repository the manifest
names, which is archived; its README says development moved off GitHub.

local patches listed below. `.cargo_vcs_info.json` names the commit it was packaged from,
`824b0791a750f7b27b8edcbb6c6244d0e629875e`. That commit is not in the GitHub repository the manifest
names, which is archived; its README says development moved off GitHub.

local patches listed below. `.cargo_vcs_info.json` names the commit it was packaged from,
`824b0791a750f7b27b8edcbb6c6244d0e629875e`. That commit is not in the GitHub repository the manifest
names, which is archived; its README says development moved off GitHub.

local patches listed below. `.cargo_vcs_info.json` names the commit it was packaged from,
`824b0791a750f7b27b8edcbb6c6244d0e629875e`. That commit is not in the GitHub repository the manifest
names, which is archived; its README says development moved off GitHub.

local patches listed below. `.cargo_vcs_info.json` names the commit it was packaged from,
`824b0791a750f7b27b8edcbb6c6244d0e629875e`. That commit is not in the GitHub repository the manifest
names, which is archived; its README says development moved off GitHub.

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
| `src/sweep/sweep_line.rs` | `choose_cross_height` takes the midpoint of a crossing's height range with `f64::midpoint` instead of `(y0 + y1) / 2.0`. For heights above about 9e307 the sum overflowed, so a range from 1e308 to 1.5e308 gave an infinite crossing height instead of 1.25e308. The `cross_height_near_f64_max` test covers that range. |
| `TreeVec::range` and `range_mut` panic with a message when a bound is past `usize::MAX` or the start is after the end, instead of overflowing | `Craft patch (range bounds)` | `src/treevec.rs` | `treevec::tests::range_reversed`, `range_mut_reversed`, `range_start_past_max`, `range_end_past_max` |

Every change here updates this table and `SOURCE.json` in the same commit.
Every file here except this one, `SOURCE.json` and `docs/sweep.typ` is byte for byte the file in the
`.crate` archive. `SOURCE.json` records the hash of the `.crate` download (`source_archive_sha256`)
and, for every file kept, the hash in the package (`source_sha256`) and the hash here (`sha256`).
`upstream_repository` is null because no public repository this copy can name holds
`upstream_revision`. `local_patches` lists each change:

- `docs/sweep.typ`: the crossing-height lemma (`lem-intersection-height-error`) and its proof divide
  the starting gap `beta(y_0) - alpha(y_0)` by the sum of both gaps. With the ending gap
  `alpha(y_1) - beta(y_1)` as the numerator, gaps of 1 and 3 gave `t = 0.75`, while the segments
  cross at `t = 0.25`. The proof's inverse-slope difference is `d/(y_1 - y_0)`; the package has
  `d/(y_1 - y_1)`, whose denominator is zero. The lemma's bound and the later bound on the crossing
  height use the absolute values of `1/m_alpha` and `1/m_beta`, because the slopes can be negative.
| `tests/paperjs.rs` | A test file in `boolean-test/data/` that cannot be loaded becomes a failing `<operation>/load` trial. The package does not include `boolean-test/data/`, and the packaged file skipped files it could not load, so the `paperjs` target passed with zero trials. The `load_errors_are_reported` trial covers a missing file; run it with `cargo test --locked --test paperjs load_errors_are_reported`. The full `paperjs` target fails here because the data files are not included. |
| `src/sweep/sweep_line.rs` | `Sweeper::next_line` handles a pair of segments at most once per sweep line for each pair of positions the two segments hold. A pair queued again at the same `y` and positions is queued at the pair's next order change instead, when that comes before both segments end. Nearly coincident segments whose comparisons are not transitive used to queue the same pairs at one `y` forever while memory grew. The `repeated_intersections_at_one_y` test sweeps five circles 7e-7 apart; test builds panic after `n^4` intersection events on one sweep line. |
| `src/topology.rs` | `contours_for_positions` returns `Error::InconsistentWindingNumbers` when the contour walk finds no boundary segment leaving a point, or re-enters a segment it already walked. Its `debug_assert!` checks of the windings along the walk and its panic for a western boundary segment with no contour return the same error. The new `Topology::try_contours` returns that error, and `Topology::contours` returns no contours in that case. Before, a path whose winding numbers came out inconsistent around a point made `contours` loop forever in a build without debug assertions, and in one case allocate a new contour on every lap. The `contours_finish_*` tests cover both loops. |
| `src/lib.rs` | `Error` has the `InconsistentWindingNumbers` variant, and `binary_op_with_eps` returns it from `Topology::try_contours`. The `inconsistent_winding_numbers_are_an_error` test covers it. |
| `src/sweep/sweep_line.rs` | `Sweeper::handle_intersection` queues a delayed intersection only when its `y` is before the ends of both segments, as the intersection scans do. A delayed intersection at a segment's end used to panic in `CurveOrder::entry_at`. The `delayed_intersection_at_a_segment_end` test sweeps three wide, flat contours whose sweep delayed an intersection to a segment's end. |
| `src/lib.rs`, `src/topology.rs` | `Topology::from_paths`, `from_paths_with_correctness`, `from_path` and `from_paths_binary` return `Error` instead of `NonClosedPath`, and return `Error::NaN`, `Error::Infinity` or the new `Error::TooLarge` when a path has a coordinate that is not finite or has a magnitude above `MAX_COORDINATE` (`f64::MAX / 16`). `binary_op` returns `Error::TooLarge` through `from_paths_binary`. Before, both panicked on a cubic with a coordinate above about `f64::MAX / 3`. A release build panicked in `f64::clamp` with "min > max, or either was NaN", and a debug build failed the `y.is_finite()` assertion in `Point::new`. The `coordinates_above_max_coordinate` test covers 6e307, 7e307 and 1.7e308. |
| `src/topology.rs` | `Contours::grouped` walks each containment tree with an explicit stack instead of one recursive call per nesting level, and returns the contours in the same order. Before, contours nested about 10,000 deep overflowed a 1 MiB stack, which aborts the process. The `grouped_*` tests cover the order and a 30,000-deep nesting on a 256 KiB stack. |
| `AlmostHorizontalDecomposition::from_monotone_cubic` keeps each piece's handles on the right side of its end points instead of clamping them into the piece's y-range, so the pieces the sweep compares follow the input curve | `Craft patch (monotone handles)` | `src/curve/mod.rs` | `curve::test::decomposition_keeps_handles_outside_the_y_range` |
| `Topology::from_path`, the other `Topology` constructors that take Bézier paths, `Segments::add_bez_path` and `binary_op` skip a `ClosePath` that comes before the first point of a path instead of panicking with "Can't start a segment on a ClosePath" | `Craft patch (leading ClosePath)` | `src/segments.rs`, `src/lib.rs` | `tests::leading_close_path` |

Every change here updates this table and `SOURCE.json` in the same commit.
| `src/topology.rs` | `Topology::merge_coincident` compares a segment with its nearest west neighbor that an earlier merge did not delete. Before, it used the west neighbor recorded when the segment was created, so a run of three or more coincident segments merged only in pairs: four copies of one square, two in each direction, left 4 segments and 2 zero-area contours instead of none. The `merge_coincident_runs_longer_than_two` test covers that input and three copies, which leave the square once. |
| `src/topology.rs` | `contours_for_positions` sorts the pieces it splits off a contour walk at a repeated point by the way they wind. A piece that winds like the walked contour becomes its sibling, and a piece that winds the other way becomes a child of the piece that contains it. Before, every piece split off became a child of the walk with the opposite `outer` flag, so of two holes touching at a point, the second came out with `outer = true` and the first hole as its parent, although it winds counter-clockwise. The `holes_touching_at_a_point_are_siblings` test covers it. |
| `src/sweep/sweep_line.rs` | The touch branch of `Sweeper::handle_intersection` runs its scan right from a segment, or left from a segment, at most once per sweep line until the sweep-line order changes. Exact copies of a shape queue a touch event for every pair of coincident segments, and each event used to scan the whole bundle in both directions, so sweeping `n` copies took time growing faster than `n^3`. The skipped scans would only push events already queued, so the output does not change. The `coincident_copies_scan_once_per_order` test counts the scans for 60 copies of two shapes, and `coincident_copies_keep_the_fill` compares the output fill with the input's nonzero fill for 200 inputs with exact, reversed and split copies. |
| `AlmostHorizontalDecomposition` stores up to two pieces inline in a `SmallVec` instead of six in an `ArrayVec`, which takes it from 824 to 288 bytes; `Segments` keeps one for every segment | `Craft patch (inline pieces)` | `src/curve/mod.rs` | `curve::test::horizontal_decomposition_stores_two_pieces_inline`, `curve::test::horizontal_decomposition_keeps_every_piece` |

Every change here updates this table and `SOURCE.json` in the same commit.

`omitted_files` lists the 43 files of the package left out here, with their hashes:

- `tests/fonts/Inconsolata-Regular.ttf`: font files live in craft-fonts.
- `docs/assets/logo.svg` and `examples/linebender.svg`: the package gives no license for this artwork
  apart from its `license` field.
- `tests/snapshots/`: 12 SVG and 20 PNG files.
- `.config/`, `.helix/`, `.typos.toml`, `.gitmodules`, `flake.nix`, `flake.lock` and `nix/`: the
  package's repository tooling.

Only the targets listed under Tests read the font, the logos and the snapshot files. Those targets do
not compile from the published package, and `Cargo.toml` here does not register them.

## Using it

The directory is excluded from the workspace. With the root manifest's `[patch.crates-io]` entry,
`craft-prepress` builds its `flatten` feature against this copy. Cargo ignores a dependency's patches,
so a consuming workspace adds its own entry:

```toml
[patch.crates-io]
linesweeper = { git = "https://github.com/storytold/craft-libs", rev = "<rev>" }
```

## Tests

`scripts/test-pdf-backends.sh` runs the package's unit, regression and doc tests and the
`visualize_op` example's test. The `craft-prepress` tests with the `flatten` feature run against this
copy. `pdf.yml`'s path filter does not list this directory, so a pull request that changes only files
here does not start `pdf.yml`. Run the tests from this directory:
`scripts/test-pdf-backends.sh` runs the package's unit, regression and doc tests, and checks that
every registered target compiles with all features. The `craft-prepress` tests with the `flatten`
feature run against this copy. `pdf.yml`'s path filter does not list this directory, so a pull request
that changes only files here does not start `pdf.yml`. Run the tests from this directory:
`scripts/test-pdf-backends.sh` runs the package's unit, regression and doc tests and the tests in the
`visualize_op` example. The `craft-prepress` tests with the `flatten` feature run against this copy.
`pdf.yml`'s path filter does not list this directory, so a pull request that changes only files here
does not start `pdf.yml`. Run the tests from this directory:
`curve_metrics` example's test. The `craft-prepress`
`scripts/test-pdf-backends.sh` runs the package's unit, regression, `paperjs` and doc tests. The `craft-prepress`
tests with the `flatten` feature run against this copy. `pdf.yml`'s path filter does not list this
directory, so a pull request that changes only files here does not start `pdf.yml`. Run the tests from
this directory:

```sh
cargo test --locked --lib --test regression --test paperjs
cargo test --locked --doc
cargo test --locked --example visualize_op
cargo check --locked --all-targets --all-features
cargo test --locked --example curve_metrics
```

`Cargo.toml` leaves out the `kompari`, `linesweeper_util` and `path-bool` dev-dependencies, which
`Cargo.toml.orig` gives as git or path dependencies. The `fonts` and `snapshots` tests, the
`winding-numbers` and `boolean_op` examples and the `linebender_logo` bench need them, or the
omitted `examples/linebender.svg`, and do not compile here. The package's `Cargo.toml` registers
these five targets; the copy's `Cargo.toml` does not, and their source files are kept unchanged.
`winding-numbers` and `boolean_op` examples and the `linebender_logo` bench do not compile here. The
package does not include the `boolean-test/data/` files that the `paperjs` test reads, and the test
skips a file it cannot read, so here it runs only `pixmap_distance_counts_coverage`.

Cargo writes to `target/` in this directory, which the root `.gitignore` excludes.
