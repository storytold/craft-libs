# Local linesweeper 0.5.0 copy

This is the crates.io linesweeper 0.5.0 package (MIT OR Apache-2.0) with some files left out and the
local patch listed below. `.cargo_vcs_info.json` names the commit it was packaged from,
`824b0791a750f7b27b8edcbb6c6244d0e629875e`. That commit is not in the GitHub repository the manifest
names, which is archived; its README says development moved off GitHub.

Every file here except this one, `SOURCE.json` and the patched file below is byte for byte the file
in the `.crate` archive. `SOURCE.json` records the hash of the `.crate` download
(`source_archive_sha256`) and, for every file kept, the hash in the package (`source_sha256`) and the
hash here (`sha256`). `upstream_repository` is null because no public repository this copy can name
holds `upstream_revision`. `local_patches` describes the local patch.

## Local patches

| File | Change |
|---|---|
| `src/sweep/sweep_line.rs` | The doc comment of `SweepLine::line_segment` says it can return a segment that has exited, which `segment_range` leaves out. Code comments label the change `Craft patch (exited slots)`. |
| `src/sweep/sweep_line.rs` | `Sweeper::next_line` bounds the loop over the intersection events at one height. Where many curves lie within the tolerance of one another and cross, the order the sweep gives them is not transitive and swapping a pair makes events for its neighbours at that height that swap them back, for ever (a stroke of 80 segments that goes back and forth over one line, with round joins and points moved by a hash of their index, was still at one height after 2 million events, and after a hundred seconds). After 4,096 events at one height the pairs they are of are kept, and the sweep panics with `linesweeper (craft patch): the intersection events at one height did not end` once the events since are more than 8 for each pair kept; 21 inputs of the craft-pathops tests (stacks of copies, pencils of lines, fans, windows of strokes, meshes, stars) handled each pair 1.5 times at the most. A caller that wraps the sweep in `catch_unwind` (craft-pathops does) gets an error and not a hang. Code comments label the change `Craft patch (event loop bound)`. A `linesweeper` taken from the registry has no such bound. |

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
