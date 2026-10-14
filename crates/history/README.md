# Craft History

`craft-history` is an app-independent snapshot history engine. It retains alternate
branches, stable state/parent/branch identities, undo/redo cursors, direct navigation,
lazy archives, checkpoint validation, state limits, and managed-memory accounting.
It has no dependency on a Craft app, UI toolkit, document format, or filesystem.

Extracted from [PhotoCraft's history port](https://github.com/storytold/photocraft/pull/5404).
PhotoCraft is the first production adapter. Independent spreadsheet-like and
slide/text document models exercise the same engine in `tests/history.rs`;
other applications have not yet been migrated.

## Integration

Implement `Policy` using an app-owned marker type. Supply the document type,
target context (layers, cells, tracks, slides, or `()`), identity remapping,
selection metadata, and deduplicated payload accounting. The policy does not
serialize documents. `History<YourPolicy>` stores immutable `Arc` snapshots.

```rust
use craft_history::{History, Policy};
use std::{collections::HashSet, sync::Arc};

#[derive(Clone, Debug)]
struct TextApp;
impl Policy for TextApp {
    type Document = String;
    type Target = (usize, usize);
    type Id = ();
    fn has_selection(_: &String) -> bool { false }
    fn remap_id(_: &mut Arc<String>, _: ()) {}
    fn document_bytes(text: &String, _: &mut HashSet<usize>) -> usize { text.capacity() }
}
let mut history = History::<TextApp>::default();
history.record("Typing", Arc::new("Before".to_owned()), (0, 5));
let restored = history.try_undo(Arc::new("After".to_owned()))?;
# Ok::<(), String>(())
```

An `ArchivedDocument<YourPolicy>` provides fallible restoration and optional resident
backing for compact replay. The app chooses codecs, scratch directories, worker
scheduling, RAM spill policy, durable publication, file locks, and recovery cleanup.
The library does not promise crash durability itself.

`checkpoint`, `timeline`, and selection metadata queries never load archives.
Undo/redo load the destination before modifying navigation; a failed load leaves
the cursor and stacks intact. Direct jumps accept an already-loaded destination,
so the app can perform I/O off the UI thread without loading intervening states.
The app must revalidate its worker result against its document/history revision
before committing it. Allocation identity guards stale asynchronous spill results.

`max_states = 0` and `max_bytes = 0` mean unlimited. An app with unlimited history
must arrange backing storage and reject edits on storage failure rather than use
the destructive bounded-history eviction helpers. Byte accounting is an estimate
of app-managed payloads, not total process RAM; metadata is reported separately.

Checkpoint IDs permit missing parents after eviction, but reject duplicate IDs,
invalid forward/self parent references, invalid branches, exhausted identity ranges,
and excessive labels. Applications must impose serialized input-size/state-count
limits before decoding a checkpoint, and validate document/target semantics.

PhotoCraft keeps pixel serialization, exact raster move replay, its recovery format,
thumbnail production, selection restoration, commands, and egui history panel.
These are adapters, not dependencies of this crate.
