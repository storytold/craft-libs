# craft-plugin-sdk

`craft-plugin-sdk` is the guest side of "craft-abi 1" (see [`craft-plugin-abi`](../plugin-abi/ABI.md)). A plugin crate is a `cdylib` that depends on this crate and on nothing else. The crate builds for `wasm32-unknown-unknown`, where it is the plugin runtime, and for a host target, where plugin code runs in a plain `cargo test`.

## Modules

- `host`: `call_json`, the one function that talks to `craft_host.call`, and typed helpers: `log`, `info`, `cancelled`, `progress`, and the modules `cmd`, `state`, `doc`, `clock`, `random` and `link`. A reply that does not fit the buffer is retried once with the size the host asks for.
- `entry`: the request and response types a handler uses, and the adapters the macro wires (`command`, `hook`, `panel`, `format`, `image`).
- `rt`: the runtime behind the exports. The functions without a target gate are safe Rust over slices and run natively. The `wasm` module (wasm32 only) turns the raw pointers of the ABI into those slices and owns `alloc` and `free`.
- `backend`: the `craft_host.call` import on wasm32. On other targets a test installs a fake host. With no fake host a call returns `HostError::NoHost`. It never returns a canned answer.
- `testing` (native only): `with_host`, `reply` and `status`.
- `ui`: a builder for panel widget trees. `Widget::build` rejects duplicate ids and trees over 2,000 nodes.

The attribute `#[craft_plugin(manifest = "craft.toml")]` comes from [`craft-plugin-sdk-macros`](../plugin-sdk-macros/README.md) and is re-exported here.

## Unsafe code

The workspace sets `unsafe_code = "forbid"`. This crate sets `deny` in its own manifest, because guest glue needs `unsafe` in two places. Each place is a module with an explicit `#[allow(unsafe_code)]`:

- `backend::imp` on wasm32: the `craft_host.call` import and its one call.
- `rt::wasm` on wasm32: reading and writing linear memory by address, and `alloc` and `free` for the exports.

Plugin code written against the SDK needs no `unsafe`. Every `unsafe` block has a `SAFETY` comment. Do not add a third module.

## Tests

- `tests/host_calls.rs`: the typed helpers against a fake host that follows ABI.md section 4, including the `E_TOO_SMALL` retry.
- `tests/macro_plugin.rs`: a plugin built from `tests/fixtures/plugin.toml`, driven through the generated `craft_testing` module.
- `tests/format_plugin.rs`, `tests/rt.rs`, `tests/ui.rs`: the format adapter, the safe runtime cores and the panel builder.
- `tests/wasm_plugin.rs`: builds a plugin for wasm32 and runs it through the install gate. It skips with a message when the wasm32 target is not installed.
