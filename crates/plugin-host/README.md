# craft-plugin-host

`craft-plugin-host` runs craft-abi 1 plugins. It has two layers.

- The engine layer (`engine`): an `Engine` trait, a wasmi engine, an optional wasmtime engine and browser engines. An engine loads a package that `craft-plugin-abi` has accepted and runs its exports under limits.
- The host layer: the checks on each host call, the loaded plugin and its reload, the stores, and the contributions (commands, hooks, panels, effects and file formats) that an app receives.

The crate depends on `craft-plugin-abi` and `craft-command`, and on no app. `craft-ipc` is an optional dependency (feature `link`).

## Boundaries

- Never crashes: no panics, no `unsafe`. A bad guest returns a `Trap`.
- Permissions are denied unless the manifest asks for them and the user grants them.
- Guest code never runs before the install gate accepts the package (`load_package`).
- No `std::time::Instant` in the engine layer, because wasm32 has none. The host injects a `Clock`.
- An untrusted plugin does not load on an engine that cannot enforce a deadline (`require_deadline_enforcement`).

## Engine layer

All modules are under `engine`.

- `mod.rs`: `Engine`, `Module`, `Instance`, `Budget`, `Caps`, `Trap`, `Export`, `LoadError`, `Clock`, `HostDispatch`.
- `load`: `load_package` runs the deadline rule, then the plugin-abi gate, then `Engine::compile`.
- `wasmi`: the default engine, on every target. It runs the guest in fuel slices (`WasmiEngine::with_slice_fuel`). The deadline and the cancel flag are checked at each slice boundary, so a deadline overshoots by at most one slice. A `ResourceLimiter` caps memory, the initial memory included. The stack size comes from `CompileOptions::stack_bytes` and is fixed at compile.
- `hostcall`: the body of the `craft_host.call` import. wasmi, wasmtime and the browser engines share it.
- `wasmtime` (feature `wasmtime`, native only): fuel per call, an epoch deadline with one shared timer thread that parks when no call runs, a `ResourceLimiter`, `max_wasm_stack`, NaN canonicalization and a pooling allocator (`with_pool_instances`, default 64). The epoch callback reads the injected `Clock` and the cancel flag, so it follows the same rules as the wasmi slice boundary. Fuel units differ from wasmi's.
- `browser` and `browser::worker` (feature `browser`, wasm32 only): the page's own WebAssembly through js-sys. `BrowserEngine` runs on the calling thread (`SYNC_CALL | MEMORY_CAP`, no deadline, so untrusted plugins are refused). `BrowserWorkerEngine` runs a pure kernel in a Web Worker that the page kills on deadline or cancel (`DEADLINE_KILL | MEMORY_CAP`). `WebClock` is the `Clock` for wasmi on a page.
- `memory_patch`: lowers the declared memory maximum before compile, which is how the browser engines cap memory. It is pure code, so native tests cover it. A grow past the cap returns -1 in a browser, where wasmi traps `OutOfMemory`.
- `js_fault`: maps JS exceptions to traps by name and message.

## Host layer

The app implements `Services`. The trait has no default bodies on purpose, so every method is a decision the app makes.

- `services`: `Services` (commands, document query, log, notices, clock, entropy, `lend_session`, `begin_call` and `end_call`, `link_apps` and `link_call`). `lend_session(&mut dyn Any, run)` is how a host call reaches the app session: a command body or a `before` gate holds `&mut S`, and the host lends it around the guest call only. `end_call(Faulted)` tells the app to restore its undo checkpoint. The host cannot restore a document itself.
- `dispatch`: `HostDispatcher`, one per instance. Check order: the ops the manifest lists, the permission kind, the parse, the per-argument grant, then the call. A side effect op replays its answer on an identical retry after `E_TOO_SMALL`. An op that is listed but not served returns `E_UNSUPPORTED`.
- `invoke`: `Generation` (one loaded version) and `Router` (which generation takes new calls). `Generation::call` is the only code that moves bytes in and out of the guest. It pairs `begin_call` and `end_call`, tells the user, and counts faults.
- `budget`: `BudgetClass`, `BudgetConfig`, `Limits` and `FaultTracker`. Three faults in a row open a circuit until the plugin reloads.
- `store`: `PackageStore` and `StateStore` traits, `MemoryStore` and `DirStore` (native). The quota is enforced inside the store, in the same write as the data. `load_all_lenient` is what `restore` uses, so one bad entry does not stop the rest.
- `params`: checks `params` against the schema of a contribution.
- `process`: `Generation::process_image` is the one pixel entry. It tiles the image into bands, runs one fresh instance per band, keeps the inner rectangle, applies the output rules and runs bands in parallel on native targets. Nothing reaches the caller until every band succeeded.
- `missing`: `MissingPluginRef`, the record a document keeps for a contribution whose plugin is not installed.
- `version`: semver precedence for the update check.
- `lifecycle` (native only): `PluginManager` installs, grants, loads, reloads, disables and removes plugins. Read the module comment before you change the reload order. A package with the id of an installed plugin keeps the old grants only when it names the same publisher and has a higher version (`UpdateRisk`). The publisher name is not verified, so this stops accidents and lazy swaps, not an author who copies a name.
- `contrib::command` (native only): `command` and `menu` contributions as `DynCommand`. The closure goes through the `Router`, so a reload does not register again.
- `contrib::hook` (native only): `before`, `after` and `change` contributions through `craft_command::Hooks`. A `before` gate has a 2 ms deadline and fails open, except a gate that declares it guards safety. `after` and `change` deliver on their own threads.
- `contrib::surface` (native only): `SurfaceSink`, which the app implements to receive panels, effects, filters and formats. `PluginManager::attach_surfaces` wires it up.
- `contrib::format` (native only): `FormatBook`, a `SurfaceSink` that keeps `format` surfaces and runs them (`handles`, `extensions`, `read`, `write`). It checks `fs.picked` on every call and has a 30 s deadline.

`lifecycle` and `contrib` are native only because `DynRun` must be `Send + Sync` and browser engine instances are not `Send`. A web path needs a proxy engine that is.

Known limits: the control plane needs a synchronous engine (`poll_ready` fails with `EngineNotSync`). The control plane is `craft_invoke` only, and pixels go through `process_image`. Registered commands send `"origin": "unknown"`.

## Features

| Feature | Default | Meaning |
|---|---|---|
| `wasmi` | yes | the wasmi engine. Builds for wasm32. |
| `deterministic` | yes | wasm 3.0 deterministic profile (canonical NaN). |
| `indirect-dispatch` | no | wasmi indirect dispatch next to portable dispatch. |
| `wasmtime` | no | the wasmtime engine. Native only. |
| `browser` | no | the browser engines. wasm32 only. |
| `link` | no | `src/link.rs` and the `craft-ipc` dependency: `LinkClient`, a ready `Services::link_apps` and `link_call`. Native only. `link` on wasm32 is a `compile_error!`. |
| `legacy-v1` | no | `src/shim`: adapters for the v1 plugin ABIs (`ec_` EffectCraft, `pc_` PhotoCraft, `vc_` VectorCraft). Builds for wasm32. Without it, a v1 package is refused at install. |

`Services::link_apps` and `link_call` exist with or without `link`, with no default body. An app without Craft Link returns `Ok(Vec::new())` and `LinkFailure::NotRunning(..)`. An app that uses `LinkClient` turns on `link`.

`cargo test --workspace` covers one feature set, because Cargo unifies features across members. `check-features.sh` runs clippy with `-D warnings` and the tests for no features, `link`, `legacy-v1`, `link,legacy-v1` and `wasmtime`. CI runs it.

## Tests

- `tests/wasmi_engine.rs`: deadline, fuel, cancel, memory, stack, host call and gate behavior on wasmi.
- `tests/conformance.rs` (needs `wasmtime`): the plugins in `tests/fixtures/conformance/` run on wasmi and wasmtime. Each scenario asserts that both outcomes are equal and equal to the ABI value, with a control that shows the budget is the cause. It also covers permissions through `HostDispatcher`, and kernel output against the plain Rust versions in `reference.rs`.
- `tests/wasmtime_engine.rs` (needs `wasmtime`): timer thread, clock and pool.
- `tests/browser_support.rs`: `memory_patch` and `js_fault` on native.
- `tests/dispatch.rs`, `budget.rs`, `store.rs`, `lifecycle.rs`, `missing.rs`, `command_contrib.rs`, `hooks.rs`, `dataplane.rs`, `format.rs`, `text_safety.rs`: the host layer against the WAT plugins in `tests/fixtures/`. `echo.wat` is a real plugin with a bump allocator. Tests build variants with `common::swap`, which fails when the marker is absent.
- `tests/grants_and_namespace.rs` and `tests/batch_and_legacy_gate.rs`: rules that must not regress (`link.call` stays inside its lists, a same-id update from another publisher keeps no grants, `batch` stays finite, a v1 prefix does not skip the resource caps).
- `tests/link.rs` (feature `link`): real `craft_ipc` peers over local sockets.
- `tests/legacy_*.rs` (feature `legacy-v1`): the v1 shims against stand-in plugins written as WAT in `tests/common/v1.rs` and `tests/common/gain.rs`. The crate ships no binary plugins.
- `tests/web.rs` (feature `browser`, wasm32): wasmi on a page, the browser engine and the Worker engine in headless Chromium, run by `run-web-tests.sh`. CI does not run it.
- `examples/engines_bench.rs` (needs `wasmi` and `wasmtime`): one 1080p kernel call on both engines and native Rust.

Not covered: Firefox, NaN payloads, simd128 kernels.
