//! {{name}}: a craft-abi 1 plugin. The manifest is craft.toml.
//!
//! `craft build` makes target/{{name}}.wasm. `cargo test` runs the tests below on your machine
//! with a fake host, no Craft app needed.

use craft_plugin_sdk::craft_plugin;

#[craft_plugin]
mod plugin {
    use craft_plugin_sdk::prelude::*;
{{handlers}}}
{{tests}}
