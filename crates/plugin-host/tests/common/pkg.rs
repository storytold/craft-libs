//! A package is a core module plus a `craft.manifest` custom section. Shared by the native tests
//! and by `tests/web.rs`, which includes this file with `#[path]`.

#![allow(dead_code, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::Value;

fn leb(mut n: usize, out: &mut Vec<u8>) {
    loop {
        let b = (n & 0x7F) as u8;
        n >>= 7;
        if n == 0 {
            out.push(b);
            return;
        }
        out.push(b | 0x80);
    }
}

pub fn with_section(mut wasm: Vec<u8>, name: &str, data: &[u8]) -> Vec<u8> {
    let mut body = Vec::new();
    leb(name.len(), &mut body);
    body.extend_from_slice(name.as_bytes());
    body.extend_from_slice(data);
    wasm.push(0);
    leb(body.len(), &mut wasm);
    wasm.extend_from_slice(&body);
    wasm
}

/// A package from WAT text and a manifest value.
pub fn package_from(wat_text: &str, manifest: &Value) -> Vec<u8> {
    let wasm = wat::parse_str(wat_text).unwrap_or_else(|e| panic!("wat: {e}"));
    with_section(wasm, "craft.manifest", &serde_json::to_vec(manifest).unwrap())
}
