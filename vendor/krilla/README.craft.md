# Local Krilla 0.8.2 patches

This is the crates.io Krilla 0.8.2 package (MIT OR Apache-2.0), upstream commit
`3ffdf0588cf98050aad6edba51ca70162e1fb5b5`. `SOURCE.json` records each retained
upstream file hash and the patched hash. Registry bookkeeping files,
`Cargo.lock` and the redundant original manifest are omitted. Upstream MIT/Apache licence texts are retained under `../../licenses/pdf-writer`;
the registry package itself omits those texts. Its examples and four CC0 ICC
profiles are retained without changes.

The patch changes two destructors: `Surface::drop` and `Page::drop` return early
when `std::thread::panicking()` is true. Drawing can panic with an unfinished
transform, clip, isolated group or tagged section. Finalizing that state during
unwind can panic a second time and abort before an outer catch can recover.
Discarding the unfinished state permits native unwind recovery. Cleanup marks the
serialization context poisoned until it completes successfully, preserving any prior
poison. Document::finish returns KrillaError::Poisoned after failed cleanup, including
explicit surface finalization, even if a caller retains the writer after a narrow
catch. A caller must still discard other captured app state after a panic. Normal
finalization, including checks for unbalanced state, remains unchanged.

Public marker constants, `CRAFT_UNWIND_SAFE_DROP` and `CRAFT_UNWIND_POISONING`, let craft-pdf reject an accidentally
unpatched dependency at compile time. Consumers must copy the Krilla root patch
from the craft-pdf README because Cargo ignores dependency workspaces' patches.

The shared writer's native subprocess regression covers the previously aborting
cleanup and a successful fresh export afterward. Native and WASM export tests
cover the unchanged successful path. WASM aborts, aborting panic configurations
and process-wide allocation failures remain outside unwind recovery.

## COLR backport

`src/text/glyph/colr.rs` is replaced by its upstream version from Krilla commit
[`acd6891f`](https://github.com/LaurenzV/krilla/commit/acd6891f7435d3272274989faa0cb12b79203258),
which applies the only three upstream changes to that file since 0.8.2, unchanged:
[#436](https://github.com/LaurenzV/krilla/pull/436) (`d29cec50`) looks colours up in palette 0
through its colour record index and rejects out-of-range palette indices, which 0.8.2 indexed
into the whole record array, ignoring the palette offset and panicking past its end;
[#437](https://github.com/LaurenzV/krilla/pull/437) (`dc97b994`) rejects colour glyphs that
no glyph or clip box bounds and fills unclipped paints within the glyph's bounds, where
0.8.2 panicked on a fill without a clip; [#438](https://github.com/LaurenzV/krilla/pull/438)
(`acd6891f`) clamps paint alpha to 0–1, which 0.8.2 rejected. A rejected colour glyph falls
back to its outline. `crates/pdf/tests/colr.rs` covers each change with a font built in the
test. Drop this backport when a Krilla release includes these commits.
