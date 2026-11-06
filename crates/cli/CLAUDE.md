# craft-cli

Library `craft_cli` plus the thin `craft` binary. Native only. Read `README.md` for the commands.

- `new.rs`: renders `templates/` (`common/` plus `kinds/<kind>.{toml,rs,test.rs}`). A `{{marker}}` with no value is an error. Each kind lists the ops and permissions its handlers use, and a unit test checks that list against the handler text. The default SDK path is fixed at build time (`CARGO_MANIFEST_DIR/../plugin-sdk`).
- `build.rs`: runs cargo with the package as the working directory, because cargo reads `.cargo/config.toml` from the working directory. The linker flags that declare the memory maximum live there. It finds the artifact in `--message-format=json` output, runs the gate, and writes `target/<name>.wasm` only if the gate accepts.
- `gate.rs`, `inspect.rs`: `craft_plugin_abi::check_package` plus a `wasmparser` pass for memories, imports, exports and custom sections. `build`, `check` and `inspect` share `gate::summary`, so they print the same verdict.
- `run.rs`: `Headless` is a real `PluginManager` with `HeadlessServices` (no commands, no document, no linked apps). Commands go through the registry. Hooks and panels go to `Generation::call`. Data plane kinds are refused.
- `devfolder.rs`: `DevFolder<T: DevTarget>`. A scan compares modified time and length. `notify` events only trigger a scan. `publish_atomic` copies under a hidden name, then renames.
- `watch.rs`: never watches `target/`.
- `bindgen.rs`: uses `craft_command::schema::from_doc`.

Gotchas:

- The gate rejects a wasm memory or table with no declared maximum. The scaffold sets the memory maximum with linker flags. A test removes that file and expects the rejection.
- `tests/e2e.rs` shares one `CARGO_TARGET_DIR` for every plugin it builds, so the SDK compiles once.
- A manifest `format` contribution needs `fs.picked`. No template scaffolds one.
