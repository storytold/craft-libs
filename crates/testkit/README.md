# craft-testkit

Shared test support for the Craft apps. The crate is not published, and apps and craft-libs crates
take it under `[dev-dependencies]`.

## serial

`serial` is native only.

- `TestLock::new()` is `const`, so a lock can be a `static`:
  `static L: TestLock = TestLock::new(); let _g = L.hold();`.
- The thread that holds a lock can call `hold()` again. The lock is released when the last of that
  thread's guards is dropped, in any order.
- A test that panics while it holds a lock does not block the next one, and a poisoned internal
  mutex is recovered.
- `gpu_lock()` is the one lock of the process for tests that build a GPU harness or device.
  `hold_gpu_lock()` takes it and keeps it until the calling thread ends; a second call on the same
  thread does nothing more.
- A guard is not `Send`. The lock covers one process: under nextest each test runs in its own
  process.
- The module reads no environment variables and prints nothing.

The tests use channel handshakes and contain no sleeps or time limits.

## Origin

| Module | Source repository | Path | SHA |
|---|---|---|---|
| `serial` | storytold/photocraft | `crates/testkit/src/gpu.rs` | `0d3b1e8523` |
| `serial` | storytold/deckcraft | `crates/ui-egui/src/gpu_lock.rs` | `b0c77ebd6a` |
| `serial` | storytold/effectcraft | `crates/ui-egui/tests/it/gpu_test.rs`, `crates/gpu/src/tests.rs` | `f029bbadb2` |
| `serial` | storytold/pdfcraft | `apps/pdfcraft/src/tests.rs` | `346d803080` |

`a_panicked_holder_does_not_block_the_next_test` and `holders_take_turns` keep their names from
PhotoCraft and DeckCraft. `holders_take_turns` waits on channels where the copies slept 2 ms.

## Migration

| Consumer | Module | Switched |
|---|---|---|
| PhotoCraft | `serial` | No |
| DeckCraft | `serial` | No |
| EffectCraft | `serial` | No |
| PdfCraft | `serial` | No |

## Maintainer

To be agreed in the review of the crate's first pull request.
