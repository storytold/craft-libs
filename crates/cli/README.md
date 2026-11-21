# craft-cli

`craft-cli` builds the `craft` program, the developer CLI for Craft plugins. It scaffolds a plugin, builds it for `wasm32-unknown-unknown`, runs the install gate on the result, shows what is inside a `.wasm`, runs one contribution in a plugin host without an app, and reloads a plugin when its file changes.

The crate is native only. It uses `notify` for file events and runs `cargo`, so it has no wasm32 build and no CI job for that target.

## Build

```sh
rustup target add wasm32-unknown-unknown
cargo build --release -p craft-cli      # writes target/release/craft
```

## Commands

- `craft new <name> [--kind command,hook,effect,panel] [--dir DIR] [--id ID] [--sdk-path PATH]` writes a plugin package: `craft.toml`, `src/lib.rs` with handlers and unit tests, `Cargo.toml`, `.cargo/config.toml` and a `Cargo.lock`. It refuses a name that exists. The package depends on `craft-plugin-sdk` by path. The default path is the `crates/plugin-sdk` of the craft-libs checkout this binary was built from. Fixed at build time, so a copied binary needs `--sdk-path`.
- `craft build [DIR] [--features F] [--out-name N]` runs `cargo build --release --target wasm32-unknown-unknown` in the package, runs the install gate on the module and prints the report. It writes `target/<name>.wasm` only when the gate accepts the module. A rejected build deletes an older `target/<name>.wasm`. Exit code 1 means rejected.
- `craft check <WASM>...` runs the install gate on each file and prints one line per file: `ACCEPTED`, or `REJECTED` and one line per violation. Exit code 1 if any file is rejected or unreadable.
- `craft inspect <WASM> [--json]` prints the manifest, each op with the permission it needs, the permissions with their prompt level, the memory limits, imports, exports, custom sections and the gate result.
- `craft run --headless <WASM> --invoke <ID> [JSON] [--grant PERMISSION] [--app NAME]` installs the plugin into a real `PluginManager` with a host that has no document and no app commands, and calls one contribution. A command goes through the command registry. A hook or panel gets the JSON as its request. Data plane contributions (effects) are refused with the reason. Log lines and call events go to stderr. The answer goes to stdout.
- `craft watch [DIR] [--install-into FOLDER]` builds, then rebuilds after each burst of changes to `src/`, `craft.toml` or `Cargo.toml`. It copies each accepted module into the folder under a hidden temporary name and renames it.
- `craft dev <FOLDER> [--invoke ID [JSON]] [--grant PERMISSION] [--poll-ms N]` loads every `.wasm` in the folder into the headless host and replaces a plugin in place when its file changes. It removes a plugin when its file goes away. A file that fails to load is reported, and the old version keeps running.
- `craft bindgen --app NAME [--from FILE|-] [--out FILE]` turns an app's command list (`<NAME>-cli commands --json`, or a saved file) into a Rust module of typed calls to `craft_plugin_sdk::cmd::execute`.

## Library

The `craft` binary is a thin wrapper over the library `craft_cli`: `new`, `build`, `gate`, `inspect`, `run`, `devfolder`, `watch`, `bindgen`, `cli` and `error`. `devfolder::DevFolder<T: DevTarget>` is the folder watcher. An app can run it over its own plugin manager by implementing `DevTarget`.

## Limits

- `run --headless` has no app. A plugin that calls `cmd.execute`, `doc.query` or `link.call` gets the answer of an app with no commands, no document and no running peers.
- Launching an app to host the plugin, reading a command list from a running app, and running the data plane from the command line are not built.

## Tests

- `tests/e2e.rs` runs the real binary and cargo. It scaffolds plugins, builds them for wasm32, checks and inspects them, runs them in the wasmi engine, and drives the edit, rebuild and reload loop. The first run compiles the SDK for wasm32, so it takes a while. It needs the wasm32 target and the `crates/plugin-sdk` crate next to `crates/cli`.
- `tests/devfolder.rs` tests the folder watcher against WAT plugins.
- `tests/bindgen.rs` tests the generator, including a command list captured from an app CLI (`tests/fixtures/filmcraft-commands.json`).
