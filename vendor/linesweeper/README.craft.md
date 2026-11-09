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

Every change here updates this table and `SOURCE.json` in the same commit. To take a new linesweeper
release, unpack the crates.io package, carry these changes over, regenerate `SOURCE.json`, and run the
tests below.

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

```sh
cargo test --locked --lib --test regression
cargo test --locked --doc
cargo test --locked --example visualize_op
cargo check --locked --all-targets --all-features
```

`Cargo.toml` leaves out the `kompari`, `linesweeper_util` and `path-bool` dev-dependencies, which
`Cargo.toml.orig` gives as git or path dependencies. The `fonts` and `snapshots` tests, the
`winding-numbers` and `boolean_op` examples and the `linebender_logo` bench need them, or the
omitted `examples/linebender.svg`, and do not compile here. The package's `Cargo.toml` registers
these five targets; the copy's `Cargo.toml` does not, and their source files are kept unchanged.

Cargo writes to `target/` in this directory, which the root `.gitignore` excludes.
