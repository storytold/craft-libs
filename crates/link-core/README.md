# craft-link-core (experimental)

Small extraction from a working local prototype connecting native VectorCraft and FigCraft rectangle properties. Apps retain their engines, native documents and standalone editing. The contract is provisional and seeks maintainer review before app adoption.

Default features provide UUID identities, serialized changes/manifests, capability checks and acyclic dependency traversal, with no storage, renderer, networking or app dependency. `native` adds SQLite WAL/FULL transactions, immutable SHA-256 snapshots using the existing `craft-atomic`, expected-revision checks, operation-idempotent receipts, durable event replay, history and rejected intents. Native storage is host-only; disable it for browser contracts.

```rust
use craft_link_core::{affected, validate_edge, Id};
let source = Id::from_u128(1);
let receiver = Id::from_u128(2);
validate_edge(&[], source, receiver)?;
assert!(affected(&[(source, receiver)], source).contains(&receiver));
# Ok::<(), craft_link_core::Error>(())
```

## Scope and guarantees

One SQLite commit publishes changed heads, immutable revision rows, history stacks, invalidation, an event and a receipt. A stale expected revision rejects the complete change set. Replaying an exact operation returns its original receipt; reusing its ID with different contents fails. Undo/redo make new monotonic revisions. Schema 0 initializes to 1 under the writer lock; newer database/manifest schemas fail closed. Native snapshots carry their originating app and media type, independently of object ownership.

Blob publication precedes metadata references. Crash leftovers can be unreferenced blobs; no garbage collector is included. Reads verify SHA-256 and bound individual blobs to 128 MiB. The store requires a private, trusted-user project directory: canonicalization and link checks do not defend against a malicious same-user process changing filesystem names between checks. Hardware/power-loss guarantees are limited by SQLite, craft-atomic and the underlying storage device.

These transactions **do not atomically edit two native applications**. Consumers project committed semantic state asynchronously, persist their applied baseline and replay events after reconnect. Unsupported editing stays a capability error; arbitrary native data stays opaque. Native application authentication, conditional writes and unsaved edit hooks belong to their adapters/engines.

Not included: an application, daemon, transport, command registry, renderer, media exchange ABI, native placed-file format, lifecycle discovery, per-app credential grants, retention or a complete semantic schema registry. `bindings` is opaque adapter-owned persistence. The native history API currently operates per canonical object, not a multi-object transaction undo command. This is not a universal lossless interchange format.

## Existing upstream seams

Related: [placed documents #59](https://github.com/storytold/craft-libs/issues/59), [project hand-off #414](https://github.com/storytold/craft-libs/issues/414), [command hooks #431](https://github.com/storytold/craft-libs/pull/431), [authenticated IPC #433](https://github.com/storytold/craft-libs/pull/433), [live media #230](https://github.com/storytold/craft-libs/issues/230). Those are independent owners/proposals. This crate provides durable revision/dependency metadata, leaving the transport and command feed replaceable. Naming/placement and whether native storage should be a separate crate remain maintainer decisions.

The prototype used both actual installed native app processes for saved name/opacity changes, reverse edits, reopen, conflicts and restart recovery. Those adapter tests belong in the consumer repository; they do not claim native app adoption of this extraction. The extraction's own tests check transactional behavior, revocation while waiting for a writer, concurrent initialization, snapshot limits, selective invalidation and an abort at the actual precommit seam (with an fsynced reached-marker).

```sh
cargo test -p craft-link-core --features native
cargo clippy -p craft-link-core --all-targets --features native -- -D warnings
cargo check -p craft-link-core --no-default-features --target wasm32-unknown-unknown
```

MIT OR Apache-2.0; workspace licenses apply. Native dependencies are optional and remain under their own licenses. This contribution contains source only, no binary artifacts or machine-local evidence.
