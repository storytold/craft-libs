# craft-plugin-abi

`craft-plugin-abi` holds the types and the rules for a WebAssembly plugin package in the Craft apps ("craft-abi 1"). It does not run code. It decides whether a package is allowed to load, before any guest code runs. The rules are in [ABI.md](ABI.md). The crate depends on `serde`, `serde_json` and `wasmparser`, on no other Craft crate and on no app, and it builds for `wasm32`.

## Boundaries

- Never crashes: no panics, no `unsafe`. A bad package returns a list of violations.
- The crate has no engine. An app (or a plugin host crate) runs the module after the gate accepts it.
- Permissions are denied unless the manifest asks for them and the user grants them.

## Modules

- `manifest`: the `craft.manifest` custom section. UTF-8 JSON, at most 256 KiB. The parser checks ids, contribution kinds, entry numbers and fields such as `budget`. A contribution kind this crate does not know stays in the manifest and is reported as partly supported. It never runs. The JSON Schema is `schemas/plugin-manifest.schema.json`, and `tests/schema.rs` checks that the Rust types and the schema agree.
- `gate`: `check_package`, the install-time policy check. It rejects a package that is over 32 MiB, has no valid manifest, has other than one exported memory, goes over the memory cap, has more than 4 tables or 10,000 functions, has a start function, imports anything but `craft_host.call`, exports with wrong signatures, or uses a feature outside the allow list. Legacy v1 modules (exports prefixed `ec_`, `pc_`, `vc_`) skip the export and import checks and keep the size and resource caps. `HostPolicy` sets what the host offers.
- `permissions`: permission names (`commands.run`, `doc.read`, `link.call` and others), the default-deny grant type and the closed list of link methods a plugin may forward.
- `ops`: host call numbers and status codes. The numbers are stable. New ops are additive.
- `descriptor`: the `craft_process` descriptor for image and audio bands.
- `format`: request and response shapes for `format` contributions (read and write a file type).
- `text`: checks and cleaners for text a plugin supplies and the host shows (control, bidi and zero-width characters).

## Use

```rust
use craft_plugin_abi::{HostPolicy, check_package};

let report = check_package(&wasm_bytes, &HostPolicy::native());
if !report.violations.is_empty() {
    // refuse the package and show the violations to the user
}
```

`PolicyReport` holds the parsed manifest, the legacy ABI if any, and every `Violation`. An empty violation list means the package passed.

## Tests

`tests/gate.rs` runs the gate on `.wat` fixtures, one per rejection rule plus accepted modules and legacy modules. `tests/schema.rs` checks the manifest types against the schema, with a table of documents that must fail.
