# craft-testkit

Test support shared by the Craft apps and the craft-libs crates. It is unpublished and is used as a
dev-dependency:

```toml
[dev-dependencies]
craft-testkit = { workspace = true }
```

The crate uses only `std`. The `deadline` module is not built for `wasm32`.

## deadline

`with_deadline(limit, label, f)` runs `f` on its own thread and returns its value. When `f` does not
finish within the limit, the test fails with a message that names `label`, the limit and the variable
that sets it. A panic in `f` is raised again in the test with its original payload. The limit counts
from the call, including the time spent starting the thread. When `f` returns or panics after the
limit has passed, the test fails with the label and the limit, and with the panic's message if it has
one.

`wait_until(limit, what, cond)` calls `cond` on the test's thread until it returns `true`. It calls
`cond` at once, then after pauses that start at 1 ms and double up to 50 ms, and does not call it
again once the limit has passed. When the limit passes
first, or `cond` returns `true` only after the limit has passed, the test fails with a message that
names `what`.

```rust
use std::time::Duration;
use craft_testkit::deadline::{DeadlineLimit, wait_until, with_deadline};

// The default of 30 s; CRAFT_TEST_TIMEOUT_SECS replaces it.
let pixels = with_deadline(Duration::from_secs(30), "decode seed 7", move || decode(&bytes));

// The app's own variable replaces the default instead.
let limit = DeadlineLimit::new(Duration::from_secs(30)).with_variable("FILMCRAFT_HOSTILE_TIMEOUT_SECS");
with_deadline(limit, "hostile input 12", move || parse(&input));

wait_until(Duration::from_secs(10), "the export to finish", || job.is_done());
```

### The limit variable

A `Duration` reads `CRAFT_TEST_TIMEOUT_SECS`. `DeadlineLimit::with_variable` names another variable.
The variable is read on every call; neither function sets or caches it.

| Value | Result |
|---|---|
| Not set | The default limit applies. |
| Decimal digits above zero, such as `30` or `007` | That many seconds replace the default. |
| Empty, signed (`+5`, `-5`), fractional (`1.5`), with spaces or units (`5s`), zero, larger than a `u64`, or not Unicode | The test fails with the variable's name and value, before `f` runs or `cond` is called. |

A zero default, a variable name that is empty or holds `=` or a NUL character, and a limit that ends past
the latest time the platform's clock can represent also fail the test before any work starts.

`craft-media-testkit`'s `with_deadline` and FilmCraft's `filmcraft-testkit` use the default when the
variable holds something other than a number. FilmCraft also uses the default for zero, and
`craft-media-testkit` accepts zero. Both fail the test here.

### Limits

- A body that runs past its limit fails the test, but its thread is left running until the test
  process exits. A body that never returns can keep the test binary alive after its tests finish.
- `wait_until` checks its limit only between calls of `cond`. A `cond` that blocks is not interrupted,
  so it must return promptly.
- Neither function returns elapsed time. Tests of this crate use channel handshakes, and the only time
  bounds in them are the limits under test.

## Origin

| Code | Repository | Path | SHA |
|---|---|---|---|
| `with_deadline`, its panic and timeout tests (`a_slow_body_passes`, `a_panic_in_the_body_is_re_raised`, `a_hang_fails_and_names_the_input`) | storytold/filmcraft | `crates/testkit/src/deadline.rs` | `cff7e144` |
| `with_deadline` under `CRAFT_TEST_TIMEOUT_SECS`, tests `with_deadline_completes` and `with_deadline_times_out` | storytold/craft-libs | `crates/media-testkit/src/lib.rs` | `75a654f4` |
| `within`, a deadline that returns the body's value | storytold/lightcraft | `crates/engine/src/demo/tests.rs` | `487833ad` |
| `within`, a deadline that returns the body's value | storytold/gridcraft | `crates/math/src/lib.rs` | `c2aba7a3` |
| `wait_until` | storytold/photocraft | `crates/ui-egui/src/canvas/paint_thread_tests.rs` | `0d3b1e85` |
| `wait_until` | storytold/lightcraft | `crates/engine/src/availability.rs` | `7ebbb17a` |
| `wait_until` | storytold/effectcraft | `crates/engine/src/footage/footage_reload.rs` | `53576d22` |
| `wait_until` | storytold/filmcraft | `crates/ui-egui/tests/it/export_ui.rs` | `cff7e144` |

## Migration

| Consumer | Code it replaces | State |
|---|---|---|
| FilmCraft | `filmcraft-testkit`'s `with_deadline` (`FILMCRAFT_HOSTILE_TIMEOUT_SECS`) and the export UI tests' `wait_until` | Pending |
| LightCraft | `within` in the demo tests and `wait_until` in the availability tests | Pending |
| GridCraft | `within` in the tests of its `crates/math` | Pending |
| PhotoCraft | `wait_until` in the paint thread tests | Pending |
| EffectCraft | `wait_until` in the footage reload tests | Pending |
| `craft-media-testkit` | `with_deadline` (`CRAFT_TEST_TIMEOUT_SECS`) | Pending |

An adoption keeps the variable name the app uses today.
