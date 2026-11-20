# craft-plugin-sdk-macros

`craft-plugin-sdk-macros` holds `#[craft_plugin(manifest = "craft.toml")]`. [`craft-plugin-sdk`](../plugin-sdk/README.md) re-exports it, so a plugin crate does not depend on this crate directly.

```rust
#[craft_plugin(manifest = "craft.toml")]
mod plugin {
    use craft_plugin_sdk::prelude::*;

    fn say(call: CommandCall) -> Result<Value, String> {
        Ok(json!({ "said": call.params }))
    }
}
```

## What the macro does

- Reads `craft.toml` at compile time (relative to `CARGO_MANIFEST_DIR`), converts it to JSON, sets the `schema` and `abi` defaults, numbers each contribution and removes its `handler` key.
- Checks the result with `craft_plugin_abi::manifest::Manifest::parse`, the parser the install gate uses. A manifest the gate would reject is a compile error that points at the module. A missing handler, a duplicate id, a hand-written `entry` and an unsupported kind are compile errors too.
- Emits adapters and the two dispatch tables inside the module, a `craft_testing` module for native tests, and, on wasm32 only, the `craft.manifest` custom section and the exports `craft_abi`, `craft_alloc`, `craft_free`, plus `craft_invoke` and `craft_process` when a contribution needs them.
- Re-reads the manifest when it changes (an `include_bytes!` on the file).

## Unsafe code

`#[unsafe(no_mangle)]` and `#[unsafe(link_section)]` need `unsafe`. The generated code carries `#[allow(unsafe_code)]` on those items, and they exist on wasm32 only. A crate that sets `forbid(unsafe_code)` cannot use the macro (E0453). Plugin crates set `deny` or nothing.

## Tests

The unit tests in `src/lib.rs` call `expand` on token streams with the manifests in `tests/fixtures/`. One test reads the SDK fixture `../plugin-sdk/tests/fixtures/plugin.toml`.
