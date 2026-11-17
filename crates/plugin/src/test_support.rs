//! Bundles for the store, catalog and fetch tests.

use crate::bundle::{Asset, Bundle};

/// A module that does nothing, under the shared ABI's names.
pub fn wasm() -> Vec<u8> {
    wat::parse_str(r#"(module (memory (export "memory") 1) (func (export "craft_run") (param i32) (result i32) i32.const 0))"#).unwrap()
}

/// The encoded bundle of plug-in `id` at `version`, asking for `access`.
pub fn bundle_bytes(id: &str, version: &str, access: &str) -> Vec<u8> {
    let manifest = format!(
        r#"{{"id": "{id}", "name": "Test", "version": "{version}", "main": "t.wasm", "apps": {{"figcraft": {{"api": 6}}}}, "permissions": {{"access": "{access}"}}}}"#
    );
    Bundle { manifest, wasm: wasm(), assets: vec![Asset { name: "icon.png".into(), data: vec![1, 2, 3] }] }.encode()
}
