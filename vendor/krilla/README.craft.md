# Local Krilla 0.8.2 patch

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

A PNG whose header implies more than 1 GiB of decoded output is refused by
`Image::from_png` and by the deferred decoder, and the decoded buffer is reserved
fallibly, so a small file (or a bitmap glyph in a font) cannot request an allocation that
ends the process. The colour and alpha buffers that PNG, GIF and WebP decoding split an
image into are reserved fallibly too, and a failure there is an image error.

A glyph counts as an SVG glyph only when the font's SVG table has a document for it.
Upstream's check asked only whether the font has the table, so every glyph of such a font
went into a Type 3 font instead of the embedded font.
