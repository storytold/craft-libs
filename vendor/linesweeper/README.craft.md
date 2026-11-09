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
| `checkerboard(0)` and `slanted_checkerboard(0)` return empty collections; the inner count was `n - 1`, which overflowed | `Craft patch (empty checkerboard)` | `src/generators.rs`, `scripts/test-pdf-backends.sh` | `checkerboards_of_size_zero_and_one` |
| `another_monotonic_bezier` returns `arbitrary::Error::IncorrectFormat` when no piece of the generated cubic increases in y; it returned that cubic, which can be horizontal | `Craft patch (flat cubic)` | `src/arbitrary.rs` | `another_monotonic_bezier_increases_in_y_or_fails` |
| A ulp step past `f64::MAX` in the test-only `F64Perturbation::apply` gives infinity; `Ulp(2)` of `f64::MAX` gave NaN | `Craft patch (ulp overflow)` | `src/perturbation.rs` | `ulp_step_past_f64_max_is_infinite` |

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
| `Topology` allocates one output segment for each run of coincident horizontal pieces between two points instead of one per piece, and none for a run whose winding numbers cancel. n overlapping rectangles that share their top and bottom edges give 6n - 2 output segments instead of 2n² + 2n. The contours do not change. `has_normal_contours` and `is_disjoint_tags` read the winding numbers on either side of a run, not the ones between its pieces. | `Craft patch (coincident horizontals)` | `src/topology.rs` | `topology::tests::coincident_horizontals_get_one_output_segment_per_run` |

Every change here updates this table and `SOURCE.json` in the same commit.
| `src/sweep/sweep_line.rs`, `src/treevec.rs` | `intersection_scan_right` and `intersection_scan_left` step over each run of sweep-line segments that end at the current height with one pass of a `TreeVec` iterator, instead of one indexed tree lookup per segment. `TreeVec::range_rev_to` iterates the elements before an index from back to front for the left scan. The scans compare the same segments and push the same events, so the output does not change; a polygon with 40,000 segments ending and 40,000 starting at one height sweeps in about 60% of the time. The `scans_step_over_segments_that_end` test checks that a segment ending at the current height does not stop the scan, `runs_of_ending_segments_keep_the_output` checks the area of such a polygon, and the `treevec` test `range_rev_to` checks the iterator. |
| `intersect_cubics_with_precomputed_decomp` compares the precomputed approximations of two pieces with `ish_tolerance` before it cuts either cubic to their shared y-range, and when that gives one `Left` or `Right` interval it records that order without cutting them | `Craft patch (approximation first)` | `src/curve/mod.rs` | `curve::test::approximation_first_keeps_the_curve_order` |
| `intersect_cubics_rec` gives the precomputed approximations the end x values of the cut curves instead of those of the whole pieces, so an almost-horizontal piece compared over part of its height is not reported as close to a curve that is far from it | `Craft patch (cut end bounds)` | `src/curve/mod.rs` | `curve::test::almost_horizontal_piece_cut_to_the_overlap` |
| `CurveOrder` records `Metrics` entries only with the new `metrics` feature, which is off by default; without it `metrics.entries` stays empty. The `curve_metrics` example requires the feature | `Craft patch (metrics opt-in)` | `src/curve/mod.rs`, `Cargo.toml` | `curve::test::crossing_cubics_metrics_follow_the_feature`, `curve::test::crossing_cubics_order_does_not_depend_on_metrics`, `curve::test::crossing_cubics_metrics_record_the_recursion_depth`, `curve::test::curve_order_no_overlap` |

Every change here updates this table and `SOURCE.json` in the same commit.
| `src/topology.rs` | `is_disjoint_tags` no longer calls `dbg!`, which printed two lines to stderr for every output segment on every call, in release builds too. It loops over `segment_indices()` instead of every winding entry, so it skips the segments that merging coincident segments deleted. The `disjoint_skips_merged_segments` test covers the second change. |
| `src/treevec.rs` | The iterators that `iter`, `iter_mut`, `range` and `range_mut` return on a tree with more than one level keep their stack of child iterators in a `Stack`, which holds 8 levels in place and puts any deeper level in a `Vec`. Before, the stack was a `Vec` and each of these calls allocated once. The sweep line calls `range_mut` on a `TreeVec<_, 128>`, which needs at least `2 * 64^9` elements to have more than 8 levels. The iterators visit the same elements in the same order. The `treevec` test `iterators_on_short_and_tall_trees` compares the four iterators with a `Vec` on a tree of at most 8 levels and on one with more. |
| `src/curve/mod.rs` | `intersect_cubics` compares a curve whose `p3.y` is below its `p0.y` as the same curve reversed, instead of panicking in `clamp`. `intersect_cubics_with_precomputed_decomp` returns close (`Ish`) over the common range when `sure_tolerance` is not positive, is larger than `ish_tolerance`, or `ish_tolerance` is not finite, instead of panicking, overflowing the stack or returning a wrong order; when the curves' vertical ranges do not overlap, it returns an empty order without a debug assertion failure, and `CurveOrder::with_y_slop`, `CurveOrder::order_after` and `CurveOrder::check_invariants` accept that empty order instead of panicking on an `unwrap`. The docs of these functions, `from_monotone_cubic`, `solve_t_for_y` and `y_subsegment` state their input rules. The `curves_decreasing_in_y`, `invalid_tolerances` and `disjoint_vertical_ranges` tests cover the changes. |
| `src/topology.rs` | `Topology::bounding_box` returns `Rect::ZERO` for a topology with no segments, as kurbo does for an empty path, instead of a rectangle from infinity to negative infinity. The `empty_bounding_box` test covers it. |
| `src/curve/mod.rs` | `solve_t_for_y` divides every height by 32 before it builds the cubic's coefficients when the largest height or the target has a magnitude above `f64::MAX / 32`. Before, a segment with heights above about `f64::MAX / 3` gave an infinite coefficient, and polycool's critical point search recursed until the process aborted with a stack overflow. `Topology::from_path` reached it on two overlapping rectangles near y = 1e308. The `solve_for_t_near_the_float_limit` test solves straight segments with heights from 5.5e307 up to `f64::MAX`. `intersect_cubics_rec` and `AlmostHorizontalDecomposition::from_monotone_cubic` take the midpoint of two heights with `f64::midpoint` instead of `(a + b) / 2.0`. Before, two heights above about 9e307 summed to infinity, the comparison of two overlapping segments had no entries, and a release build panicked on the empty list. The `compare_near_the_float_limit` test compares two lines that start at heights up to 1.7e308. |
| `src/lib.rs`, `src/order.rs`, `src/sweep/sweep_line.rs`, `src/topology.rs` | `ComparisonCache` counts its calls, so `Sweeper` counts its segment comparisons, and `Sweeper` also counts the segment positions it checks against horizontal segments and contour endpoints while finding a sweep line's changed intervals, and one unit for every 16 segments that linear searches of the sweep line step over. `Sweeper::set_work_limit` sets the most work it may do; past it, `Sweeper::next_line` returns `None`, also between the enter, exit and intersection events of one sweep line and between the comparisons and position checks that find its changed intervals, never returns a sweep line past the limit, and keeps returning `None` if the limit is raised; `Sweeper::work_limit_exceeded` returns `true`. `Topology::from_paths_with_work_limit` returns the new `Error::WorkLimitExceeded` when its sweep passes the limit. The sweep had no limit: the union of 400 identical circles made 1.36 billion comparisons in 19 sweep lines and took 66 to 83 s and 1.3 GB in a release build. `Topology::from_paths` and the other constructors set no limit. The `work_limit` and `work_limit_stops_within_a_sweep_line` tests sweep 20 stacked circles with a limit of 1,000 comparisons, and `work_limit_returns_no_line_past_the_limit` sweeps 3 stacked circles with every limit below the sweep's total. `work_limit_counts_the_horizontal_scans` sweeps 150 bars crossed by the top and bottom edges of 30 wide rectangles, where the sweep makes 1,462 comparisons but checks at least 18,000 positions, and stops it at half its total work. `work_limit_counts_linear_searches` sweeps 2,000 disjoint rectangles sharing their top and bottom heights, where each exit searches the whole sweep line, and stops it at half its total work. |
| `src/topology.rs` | `Topology::can_merge` merges two output segments only when the neighbor at the shared point is the adjacent piece of the same input segment. Before, after coincident segments were deleted, a piece of one input curve could be merged with a piece of another, and the output rebuilt a sub-curve from the wrong curve's parameters: two arches whose right legs agree to within about 1e-6 (area about 0.00096) gave a contour of area 900 at eps 1e-6, and a path followed by its own reverse panicked in `segs_to_path`. `segs_to_path` also starts at the first segment instead of panicking when every segment looks mergeable. The `merge_only_pieces_of_one_input_curve` and `path_followed_by_its_reverse` tests cover both inputs. |
| `src/topology.rs` | With `Correctness::GoodEnough`, `Topology::contours` builds paths only for the output segments on the boundary of the result instead of for every output segment, which it then did not read. The contours it returns are unchanged. The `contour_positions_cover_only_boundary_segments` and `contours_match_contours_from_all_positions` tests cover the change. |
| `Segments::add_bez_path` leaves the arena unchanged when a subpath is not closed, instead of keeping the segments it pushed before the open subpath, the last of them linked to an index past the end of the arena | `Craft patch (add_bez_path error)` | `src/segments.rs`, `src/lib.rs` | `tests::add_bez_path_error_leaves_segments_unchanged` |

Every change here updates this table and `SOURCE.json` in the same commit.
| `src/topology.rs` | `contours_for_positions` finds the nearest boundary segment west of each new contour through `Topology::west_boundary`, which remembers the answer for every segment it passes, so each link of a scan-west chain is followed once per call. A segment's boundary test runs the first time the walk asks for it, and its result is kept. Before, every contour walked its whole chain again, and `n` bars of one set west of `m` squares of the other with the test "first set and not second" called `inside` about `4nm` times: 1,024,240,000 calls for `n = m = 16,000`. The contours do not change. The `contour_starts_follow_each_west_chain_once` test counts the calls to `inside` for 200 bars and 200 squares, and `contour_starts_find_the_boundary_past_non_boundary_segments` checks the parents of squares whose chains pass non-boundary segments. `contours_of_one_polygon_test_the_boundary_only_where_needed` counts the calls to `inside` for one polygon with 1,000 edges. |
| `src/curve/mod.rs` | `solve_t_for_y` asks the root finder for `t` to the precision of f64 (`x_error` 0) instead of to within 1e-12. The x error of a `t` that is off by 1e-12 grows with the coordinates: on a curve 1e9 tall it reached 166 times `64 * f64::EPSILON * 1e9`, and pieces that `y_subsegment` cut from curves near 3e7 tall moved by tens of tolerances. Comparing two near-parallel curves of that size recursed down to y intervals shorter than the tolerance: 950,130 entries and 9 s for a pair 3e7 tall, minutes for a pair 1e8 tall. The `solve_x_for_y_on_tall_curve` test checks x on a curve 1e9 tall against a bisection, and the `near_parallel_tall_curves` test compares two curves 2e7 tall, 1.2 tolerances apart, and allows at most 64 entries. |
| `src/curve/mod.rs` | `CurveOrder::next_touch_after(y)` walks the order once and compares each touch with `y`. Before, when `y` was inside a touch followed by a Left interval and another touch, it called itself with the end of the Left interval, which is where the next touch starts, so it skipped that touch too and returned `None`. Two curves that start at one point and touch again later gave no later touch at the start height. The `next_touch_after_a_touch_that_contains_y` and `next_touch_after_on_two_curves_that_start_together` tests cover a built order and two curves from `intersect_cubics`. |
| `ComparisonCache` holds the comparisons of one `Segments` arena; given another arena, a clone included, it drops them and compares again | `Craft patch (comparison cache arena)` | `src/order.rs`, `src/segments.rs` | `other_arena_is_compared_again`, `diverged_clone_is_compared_again`, `appending_keeps_cached_comparisons` |

Every change here updates this table and `SOURCE.json` in the same commit.
| `src/curve/mod.rs` | `fit_quadratic` multiplies the heights by the power of two that brings the largest of them to between 2 and 4 when one of them has a magnitude above 2^500, and scales the quadratic's coefficients back at the end. `QuadraticApprox::from_cubic` multiplies by the quadratic's `y^2` coefficient before the second y coefficient when the product of two y coefficients is not finite. Before, a curve taller than about 1.3e154 gave a product of two heights that overflowed to infinity, and `inf * 0` gave a NaN coefficient, or an error bound that left out the curve's extremes. The curve comparison then reported a definite order that could be wrong, and `binary_op` returned a wrong result without an error. The quadratic sign checks used by curve comparisons now report the order as unknown (or close) over the whole interval when a coefficient is NaN or infinite, instead of a definite order. The `far_apart_with_huge_coordinates` test compares two curves with heights up to 1e155, 1e200 and 1e300, `tall_lines_at_any_height` compares two lines from 1 to 1e300 tall at three heights, `quadratic_approx_error_bounds_on_tall_curves` checks the coefficients and error bounds of an S-shaped curve up to 1e300 tall and 1e300 wide, and `non_finite_quadratics_have_no_definite_order` checks the sign checks. |
| `src/curve/line.rs` | `intersect_lines` compares the x coordinates and the tolerance times 1/16 when one of them has a magnitude above 2^1019, and works with half of each height for a line or height range more than `f64::MAX` tall. Before, two vertical lines at x = -1e308 and x = 1e308 came back `Right`, because the difference of their x values overflowed, and lines from y = -1e308 to 1e308 gave NaN positions. The `unchanging_with_huge_coordinates` test covers 1e300, 1e308 and `f64::MAX`, and `huge_coordinates_on_one_axis` covers lines near x = 1e308 that are 1e-307 tall, lines from y = -1e308 to 1e308 that stay apart or cross, and a line 1e-308 tall beside one of them. |
| `src/geom.rs` | For a straight segment more than `f64::MAX` long on an axis, `Segment::at_y` works with half of each coordinate on that axis and `Segment::to_kurbo_cubic` takes weighted sums of the end points on that axis. Before, both returned NaN or infinite coordinates for such a segment. The `line_wider_and_taller_than_f64_max` test covers a segment from (-1e308, -1e308) to (1e308, 1e308), and `line_wider_than_f64_max_and_very_short` covers a segment from (-1e308, 0) to (1e308, 3 times the smallest positive `f64`). |
| `src/lib.rs` | The `union_with_huge_coordinates` test checks the union of a circle and a rectangle at sizes 1e155, 1e200 and 1e300. |
| `src/lib.rs` | `binary_op_with_eps` returns the new `Error::InvalidEps` when `eps` is NaN, infinite, zero or negative, instead of returning a wrong region (NaN or negative `eps`) or hanging (`-inf`). The `binary_op_with_eps_rejects_invalid_eps`, `topology_raises_unusable_eps` and `binary_op_with_eps_raises_eps_below_coordinate_resolution` tests cover the changes in this file, `src/segments.rs`, `src/sweep/sweep_line.rs` and `src/topology.rs`. |
| `src/segments.rs`, `src/sweep/sweep_line.rs`, `src/topology.rs` | The `Topology` constructors and `Sweeper::new_with_correctness` replace an `eps` that is NaN, infinite, not positive, or below `64 * f64::EPSILON` times the largest coordinate magnitude (and at least 64 times the spacing of subnormal floats) with that value, the scale `binary_op` uses. Before, an infinite `eps` made a lens of two arcs vanish, and an `eps` below one ulp of the coordinates (0 included, or 1e-10 at 1.5e8) overflowed the stack and aborted the process. |
| `src/curve/mod.rs` | `intersect_cubics` and `intersect_cubics_with_precomputed_decomp` raise a positive `ish_tolerance` below `4 * f64::EPSILON` times the two curves' largest x coordinate magnitude (two to four ulps of it, and at least four times the spacing of subnormal floats) to that value, and a positive `sure_tolerance` below half of it to half of it. Below that, rounding made the comparison report the wrong order. `intersect_cubics_rec` returns close (`Ish`) for an interval whose midpoint rounds to one of its ends instead of splitting it again, which repeated the same call until the stack overflowed. The `ish_tolerance_below_float_spacing` test covers both. `raised_tolerance_keeps_far_curves_apart`, `raised_tolerance_ignores_large_y` and `raised_tolerance_keeps_subnormal_curves_apart` check that lines 1.0 apart at x = 1e15, lines 1.0 apart at y = 2e15, and lines at x = 1e-310 and 2e-310 stay ordered. |
| `src/position.rs` | `approximate` stops halving an interval whose midpoint rounds to one of its ends. Before, it could return the interval's start, and the loop in `compute_positions_correct` then made no progress. The `approx_stops_at_one_float` test covers it. |
| `src/curve/mod.rs` | `intersect_cubics_rec` calls the new `order_from_control_points` after cutting both cubics to their shared y-range. It bounds the x gap between the two pieces by the smallest and largest control point x gap, widened by the largest control point y gap times a bound on `\|dx/dy\|` from the control polygon (`max_slope`). When that range stays more than `ish_tolerance` on one side of zero, the whole range gets `Left` or `Right` and the comparison does not recurse. Before, two nearly parallel cubics split their shared y-range into about `(height / ish_tolerance)^(1/3)` pieces even when they stayed more than `ish_tolerance` apart: a tall S-shaped cubic and a copy moved 10 * `ish_tolerance` sideways took 4,096 recursion leaves at height 1e6 and 16,384 at height 1e8; now each takes one. The `near_parallel_cubics_are_ordered_without_recursing` test counts the entries for that pair, `control_point_order_is_sound` checks every order the control points give against sampled gaps, and `control_point_order_keeps_band_contours` checks that the contours of a tall wavy band and a moved copy do not change. |
| `src/order.rs`, `src/sweep/sweep_line.rs`, `src/position.rs` | `ComparisonCache` stores one `CurveOrder` per pair of segments instead of one for each direction, and flips it when the pair is looked up in the other order. `ComparisonCache::evict_ended` drops the pairs in which a segment ends above a given height, once the cache has doubled since its last scan. `Sweeper` calls it on both of its caches each time the sweep line moves down, and `compute_positions` calls it each time it takes the next height from its queue. Both caches used to keep every pair the sweep compared until the sweep ended, so a sweep of a 2,001-point star polygon held 4,001,998 cached orders (1,463 MiB). Code comments label the changes `Craft patch (comparison cache size)`. The `one_order_per_pair` and `reversed_lookup_flips_the_stored_order` tests check the stored and flipped orders, `ended_pairs_leave_the_comparison_cache` checks that a sweep of a tall band keeps fewer than a tenth of the pairs, and `evicting_comparisons_keeps_the_sweep_lines` and `evicting_comparisons_keeps_the_correct_positions` check that the sweep lines, output events and positions do not change. |
| `src/position.rs` | `PositionedOutputSeg::from_cubic` and `from_quad`, and the copied pieces of the strict layout (`next_subsegment`), cut the input curve at the points nearest the output piece's end points, found by Newton steps from the parameters `solve_t_for_y` gives (restarted from the nearest point of the whole curve at a point where a step cannot leave, such as a repeated handle), and move the piece onto its end points: each end moves sideways with its handle (a quadratic's control point by the mean of its ends' moves), and the heights are mapped linearly, so the piece stays monotonic in `y`. `from_cubic` and `from_quad` used to move the inner control points sideways by their height's share of the gap at each end, and `next_subsegment` moved only the end points. Where a curve is almost flat at an end, a tiny error in that end's height is a large gap: the union of `M0,0 C0,1000 300,1000 1000,1000 L1000,2000 L-1000,2000 Z` with the rectangle `M998.37,500 L1500,500 L1500,1500 L998.37,1500 Z` at eps 1e-6 gave an outline 1.05 from both inputs and 815 too small in area, and with the rectangle at x = 990.37 the strict layout gave an outline 9.9e-4 from both inputs. Code comments label the changes `Craft patch (flat-end shear)`. `repeated_end_handle_stays_near_the_end` places cubics with a repeated handle at the flat end between end points moved along it, the `flat_end_shift_stays_near_the_end` test places steep-to-flat and flat-to-steep cubics and quadratics between end points moved along their flat ends, `flat_end_pieces_stay_monotonic` places a quadratic and a cubic between end points off the curve and checks that their heights do not decrease or pass the end point, `flat_end_union_stays_on_the_inputs` checks the union of that shape with a copy moved by (-3.26e-6, -4.66e-8), and `flat_end_crossing_stays_on_the_inputs` checks the unions with the two rectangles in both layouts. |
| `src/sweep/sweep_line.rs` | `Sweeper::scan_for_removal` starts its intersection scans from the nearest segment left of the removed slot that does not end at the current `y`. Segments that end at `y` stay in the sweep line until `advance`, and a scan from one of them returns at once, so the live segments on both sides of the gap were never compared and their crossing was missed. The `removal_scan_skips_segments_that_end` test sweeps two integer polygons whose crossing at (0.75, 1.5) used to be missed; their Intersection came out with area 1.5 instead of 1.125. Both removal paths call this function: a segment's exit and the removals in `handle_intersection`. During the exits at one height, `EndedRun` keeps the walk to that segment, so many segments ending at one height do not each repeat it; `removal_scans_share_the_run_of_ended_segments` counts the positions visited. |
| `binary_op` returns the new `Error::FeatureBelowTolerance { eps }` when its automatic `eps` is above `MIN_AUTO_EPS` (1e-6) and an input subpath at least `4 * MIN_AUTO_EPS` wide and high is narrower than `FEATURE_EPS_RATIO * eps` (4 * eps) in x or y, measured on the subpath's exact bounding box. Before, one far point raised `eps` for every segment and `binary_op` returned `Ok` without the small subpaths: with a triangle at 3e13 (eps 0.43) a 0.3 x 10 sliver was lost, and at 1e15 (eps 14.2) most of a 10 x 10 square was. The docs of `binary_op` and `binary_op_with_eps` state that `eps` is one absolute tolerance and what the check does not cover | `Craft patch (far-point eps)` | `src/lib.rs` | `tests::far_point_eps` |

Every change here updates this table and `SOURCE.json` in the same commit.
| `src/order.rs`, `src/sweep/sweep_line.rs` | Segments that start at one point all compare "ish" there, so each intersection scan and insertion search run at that point passes every other segment that starts there. Those comparisons are no longer stored: `ComparisonCache::with_order` uses a stored comparison if there is one and otherwise computes it the same way and drops it. `n` segments that start at one point and do not cross later now store `O(n)` comparisons instead of `O(n^2)` (comparisons made at a later crossing are stored as before); 1,000 triangles that share their apex peak at 8 MiB instead of 2,431 MiB and sweep in 0.56 s instead of 7.7 s. The comparisons are the same, so the output does not change. The `segments_starting_at_one_point_are_not_stored` and `curves_starting_at_one_point_are_not_stored` tests count the stored comparisons, `segments_starting_at_one_point_sweep_as_with_stored_comparisons` checks that every sweep line is the same as with all comparisons stored, and the `order` test `unstored_order_matches_stored_order` checks `with_order` against `compare_segments`. |
| `src/topology.rs` | `Topology` stores each side of an output segment's winding numbers behind an `Arc`, and the sweep gives a segment's clockwise side and the previous segment's counter-clockwise side the same `Arc`. Before, every segment held its own copy of both sides, so a winding number type that allocates (one entry per covering input) stored each value twice. For 500 squares that all cross each other, with a winding number type like `craft-prepress`'s, building the `Topology` peaked at 2,769.4 MB and now peaks at 2,102.5 MB, and `craft_prepress::flatten::geom::regions` peaked at 3,535.2 MB and now peaks at 2,842.1 MB, with the same regions. `winding_clockwise`, `winding_counter_clockwise` and `HalfSegmentWindingNumbers` do not change. A `Topology<W>` is now `Send` and `Sync` only when `W` is both `Send` and `Sync`. Code comments label the changes `Craft patch (winding storage)`. The `neighboring_segments_share_winding_numbers` test checks the sharing, and `staggered_squares_winding_numbers_count_the_covering_squares` checks the areas covered by each number of squares. |

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
cargo test --locked --lib --test regression
cargo test --locked --lib --features generators generators::
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
