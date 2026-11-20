# craft-plugin-host

`craft-plugin-host` runs craft-abi 1 plugins. This release has the engine layer: an `Engine` trait, a wasmi engine, an optional wasmtime engine and browser engines. An engine loads a package that `craft-plugin-abi` has accepted and runs its exports under limits. The crate depends on `craft-plugin-abi` and on no app.

## Boundaries

- Never crashes: no panics, no `unsafe`. A bad guest returns a `Trap`.
- Guest code never runs before the install gate accepts the package (`load_package`).
- No `std::time::Instant` in the engine layer, because wasm32 has none. The host injects a `Clock`.
- An untrusted plugin does not load on an engine that cannot enforce a deadline (`require_deadline_enforcement`).

## Modules

All modules are under `engine`.

- `mod.rs`: `Engine`, `Module`, `Instance`, `Budget`, `Caps`, `Trap`, `Export`, `LoadError`, `Clock`, `HostDispatch`.
- `load`: `load_package` runs the deadline rule, then the plugin-abi gate, then `Engine::compile`.
- `wasmi`: the default engine, on every target. It runs the guest in fuel slices (`WasmiEngine::with_slice_fuel`). The deadline and the cancel flag are checked at each slice boundary, so a deadline overshoots by at most one slice. A `ResourceLimiter` caps memory, the initial memory included. The stack size comes from `CompileOptions::stack_bytes` and is fixed at compile.
- `hostcall`: the body of the `craft_host.call` import. wasmi, wasmtime and the browser engines share it.
- `wasmtime` (feature `wasmtime`, native only): fuel per call, an epoch deadline with one shared timer thread that parks when no call runs, a `ResourceLimiter`, `max_wasm_stack`, NaN canonicalization and a pooling allocator (`with_pool_instances`, default 64). The epoch callback reads the injected `Clock` and the cancel flag, so it follows the same rules as the wasmi slice boundary. Fuel units differ from wasmi's.
- `browser` and `browser::worker` (feature `browser`, wasm32 only): the page's own WebAssembly through js-sys. `BrowserEngine` runs on the calling thread (`SYNC_CALL | MEMORY_CAP`, no deadline, so untrusted plugins are refused). `BrowserWorkerEngine` runs a pure kernel in a Web Worker that the page kills on deadline or cancel (`DEADLINE_KILL | MEMORY_CAP`). `WebClock` is the `Clock` for wasmi on a page.
- `memory_patch`: lowers the declared memory maximum before compile, which is how the browser engines cap memory. It is pure code, so native tests cover it. A grow past the cap returns -1 in a browser, where wasmi traps `OutOfMemory`.
- `js_fault`: maps JS exceptions to traps by name and message.

## Features

| Feature | Default | Meaning |
|---|---|---|
| `wasmi` | yes | the wasmi engine. Builds for wasm32. |
| `deterministic` | yes | wasm 3.0 deterministic profile (canonical NaN). |
| `indirect-dispatch` | no | wasmi indirect dispatch next to portable dispatch. |
| `wasmtime` | no | the wasmtime engine. Native only. |
| `browser` | no | the browser engines. wasm32 only. |

## Tests

- `tests/wasmi_engine.rs`: deadline, fuel, cancel, memory, stack, host call and gate behavior on wasmi.
- `tests/conformance.rs` (needs `wasmtime`): the plugins in `tests/fixtures/conformance/` run on wasmi and wasmtime. Each scenario asserts that both outcomes are equal and equal to the ABI value, with a control that shows the budget is the cause. Kernel output is compared to the plain Rust versions in `reference.rs`.
- `tests/wasmtime_engine.rs` (needs `wasmtime`): timer thread, clock and pool.
- `tests/browser_support.rs`: `memory_patch` and `js_fault` on native.
- `examples/engines_bench.rs` (needs `wasmi` and `wasmtime`): one 1080p kernel call on both engines and native Rust.

Not covered: a headless browser run of the browser engines, NaN payloads, simd128 kernels.
