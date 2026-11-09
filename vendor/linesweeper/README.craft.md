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
| `src/topology.rs`, `src/snapshots/linesweeper__topology__tests__*.snap` | Building a `Topology` and walking its contours no longer make small allocations per segment. The open output segments of each input segment are kept in an inline queue of three (`OpenSegs`) instead of a `VecDeque`, and that state now lives only for the sweep instead of in the `Topology`, so the nine topology snapshots no longer list `open_segs`. The segment buffers are kept for the whole sweep instead of being allocated for each sweep-line range, the winding number is cloned for horizontals only when a horizontal starts at the point, `can_merge` returns early at a point that is not simple before solving for `t`, and `segs_to_path` reverses first halves into one reused buffer instead of calling `reverse_subpaths` for each one. On a stroked serpentine of 501,993 segments, building allocates 4,324,964 times instead of 6,044,487 and holds 731.2 MB instead of 764.0 MB, and `contours` allocates 632,030 times instead of 834,041; the contours are the same. The `building_a_topology_does_not_allocate_per_segment` and `contours_do_not_allocate_per_reversed_segment` tests count the allocations per extra square or wave, `reverse_path_into_matches_reverse_subpaths` checks the reversal against `reverse_subpaths`, `open_segs_pop_like_a_deque` checks the queue against a `VecDeque`, and `contours_keep_their_elements_and_direction` compares two contours with the elements the unpatched code writes. |

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
