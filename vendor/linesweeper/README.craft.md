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
| `src/position.rs` | `PositionedOutputSeg::from_cubic` and `from_quad`, and the copied pieces of the strict layout (`next_subsegment`), cut the input curve at the points nearest the output piece's end points, found by Newton steps from the parameters `solve_t_for_y` gives (restarted from the nearest point of the whole curve at a point where a step cannot leave, such as a repeated handle), and move the piece onto its end points: each end moves sideways with its handle (a quadratic's control point by the mean of its ends' moves), and the heights are mapped linearly, so the piece stays monotonic in `y`. `from_cubic` and `from_quad` used to move the inner control points sideways by their height's share of the gap at each end, and `next_subsegment` moved only the end points. Where a curve is almost flat at an end, a tiny error in that end's height is a large gap: the union of `M0,0 C0,1000 300,1000 1000,1000 L1000,2000 L-1000,2000 Z` with the rectangle `M998.37,500 L1500,500 L1500,1500 L998.37,1500 Z` at eps 1e-6 gave an outline 1.05 from both inputs and 815 too small in area, and with the rectangle at x = 990.37 the strict layout gave an outline 9.9e-4 from both inputs. Code comments label the changes `Craft patch (flat-end shear)`. `repeated_end_handle_stays_near_the_end` places cubics with a repeated handle at the flat end between end points moved along it, the `flat_end_shift_stays_near_the_end` test places steep-to-flat and flat-to-steep cubics and quadratics between end points moved along their flat ends, `flat_end_pieces_stay_monotonic` places a quadratic and a cubic between end points off the curve and checks that their heights do not decrease or pass the end point, `flat_end_union_stays_on_the_inputs` checks the union of that shape with a copy moved by (-3.26e-6, -4.66e-8), and `flat_end_crossing_stays_on_the_inputs` checks the unions with the two rectangles in both layouts. |

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
