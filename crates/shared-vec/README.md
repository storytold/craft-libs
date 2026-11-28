# Craft Shared Vec

`craft-shared-vec` provides `SharedVec<T>`, an ordered and indexed list whose clones share their
storage. It is meant for document trees that keep earlier versions in an undo history, where each
container holds its children in a list.

A tree that keeps its children as `Arc<Vec<Arc<Node>>>` and edits them with `Arc::make_mut` copies
the whole child list on every edit while an earlier version still holds it. On a layer of 100,000
objects, one edit copies 100,000 handles (800 KB on a 64-bit build), and an undo history that
charges each copy reaches a 512 MiB limit after about 670 edits. `SharedVec` copies only the blocks on
the path to the edited element.

## Structure

- Elements sit in leaves of up to 64; branches hold up to 64 children and the length of each.
  Every node except the root holds at least 32, and every leaf is at the same depth.
- Every block sits behind an `Arc`. `clone` copies one handle. An edit copies each block on its path
  that another list still holds.
- `get`, `set`, `get_mut`, `insert`, `remove`, `push` and `pop` take O(log n) and copy one leaf and
  one branch per level. A removal that leaves a node too small also copies the neighbor it merges
  with or takes elements from.
- `append`, `split_off`, `truncate`, `splice` and `retain` make their changes as single edits when
  they remove or insert only a few elements. When they change more, they rebuild the branches from
  the leaves: they keep every leaf they do not cut or change, and copy O(n / 64) handles. A rebuild
  is chosen only when the single edits could copy as many blocks, so a loop that removes one element
  at a time through `splice` or `retain` copies about one path per removal, as `remove` does.
- An empty list does not allocate.
- If `T::clone` panics during an edit, the list stays memory-safe, but which elements it holds
  afterwards is unspecified. A panic in the iterator passed to `splice`, or in the closure passed to
  `retain`, leaves the list as it was.
- `iter` is double-ended and exact-size; `fold` and `rfold` run a leaf slice at a time. `chunks`
  returns the leaves as slices.

## Counting retained bytes

`list.bytes_not_in(&other)` returns the bytes of the blocks that `list` holds and `other` does not:
what keeping both versions costs over keeping `other` alone. A block's bytes are the sizes it asks
the allocator for (the `Arc` counts, its fields and each buffer's capacity); the allocator's own
rounding and bookkeeping are not included. It walks the two trees level by level
and leaves out every block that both hold, so after one edit it visits about one block per level.
`bytes_not_in_with(&other, key, payload)` adds the heap bytes of the elements that are new: an
element is counted when `other` holds no element with the same `key` (for an `Arc`, its address).
The unchanged elements of a copied leaf, and elements that an edit moved to another leaf, are not
counted.

`blocks()` lists every block with its id, kind, slot count and bytes, so a test can check which
blocks an edit copied.

## Serde

With the `serde` feature, a `SharedVec<T>` serializes and deserializes as a plain sequence, the same
as a `Vec<T>`. A field changed from `Vec<T>` to `SharedVec<T>` reads the same files and writes the
same bytes. The reader does not use the length an input announces for an allocation.

## Usage

```rust
use craft_shared_vec::SharedVec;

let before: SharedVec<u32> = (0..100_000).collect();
let mut after = before.clone();
after.set(50_000, 7).ok();
assert_eq!(before.get(50_000), Some(&50_000));
// One leaf and two branches are new.
assert!(after.bytes_not_in(&before) < 4_000);
```

## Benchmarks

`cargo run --release -p craft-shared-vec --example bench` times 2,000 edits, each beside a kept
snapshot, and iteration. On both sides the timer covers the edit alone; the snapshot is taken
before it starts and dropped after it stops. The bytes per edit are computed rather than measured:
the new vector's capacity times the element size for `Vec`, and `bytes_not_in` for `SharedVec`.
Results on a 128-core x86-64 Linux host (Rust 1.100.0), with `Arc<u64>` elements for the edits:

| Elements | `Arc<Vec<_>>` copied on write | `SharedVec` |
|---|---|---|
| 1,000 | 3.0 µs and 8,000 B per edit | 0.24 µs and 1,000 B per edit |
| 10,000 | 30 µs and 80,000 B per edit | 0.38 µs and 2,025 B per edit |
| 100,000 | 296 µs and 800,000 B per edit | 0.55 µs and 2,796 B per edit |

Removing one element of the 100,000 with `splice` or `retain` counts 2,808 B.

Summing 100,000 `u64` values takes 0.08 ns per element with `Vec::iter().fold`, 0.18 ns with
`SharedVec::iter().fold`, and 0.84 ns with a `for` loop over a `SharedVec` (0.39 ns over a `Vec`).

## License

MIT OR Apache-2.0, like the rest of the workspace.
