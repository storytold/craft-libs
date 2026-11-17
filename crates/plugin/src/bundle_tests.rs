//! Plugin bundles: the binary form, its validation and directory versions.

use std::cmp::Ordering;

use crate::bundle::{Asset, Bundle, MAGIC, MAX_ASSETS, check_asset_name, compare_versions, newer, parse_version, sha256_hex};

fn wasm() -> Vec<u8> {
    wat::parse_str(r#"(module (memory (export "memory") 1) (func (export "figcraft_run") (param i32) (result i32) i32.const 0))"#).unwrap()
}

fn manifest(version: &str, api: u32) -> String {
    format!(r#"{{"id": "org.example.b", "name": "B", "version": "{version}", "author": "  Ada  ", "main": "b.wasm", "api": {api}}}"#)
}

fn bundle() -> Bundle {
    Bundle { manifest: manifest("1.2.0", 1), wasm: wasm(), assets: vec![Asset { name: "icon.png".into(), data: vec![1, 2, 3] }] }
}

#[test]
fn round_trips_and_checks() {
    let b = bundle();
    let bytes = b.encode();
    assert!(bytes.starts_with(MAGIC));
    assert_eq!(Bundle::decode(&bytes, bytes.len()).unwrap(), b);
    assert_eq!(sha256_hex(&bytes), sha256_hex(&b.encode()), "the same bundle hashes the same");
    assert_eq!(sha256_hex(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
    let m = b.check().unwrap();
    assert_eq!((m.id.as_str(), m.version.as_str(), m.author.as_str()), ("org.example.b", "1.2.0", "Ada"));
}

#[test]
fn decoding_refuses_damage() {
    let bytes = bundle().encode();
    assert!(Bundle::decode(&bytes, bytes.len() - 1).unwrap_err().contains("limit"));
    for n in 0..bytes.len() {
        assert!(Bundle::decode(&bytes[..n], usize::MAX).is_err(), "cut at {n}");
    }
    let mut extra = bytes.clone();
    extra.push(0);
    assert!(Bundle::decode(&extra, usize::MAX).unwrap_err().contains("after"));
    let mut magic = bytes.clone();
    magic[0] = b'X';
    assert!(Bundle::decode(&magic, usize::MAX).unwrap_err().contains("isn't"));
    // A length larger than what follows never allocates it.
    let mut huge = MAGIC.to_vec();
    huge.extend(u32::MAX.to_le_bytes());
    assert!(Bundle::decode(&huge, usize::MAX).unwrap_err().contains("cut short"));
    // Too many assets.
    let mut many = MAGIC.to_vec();
    for part in [&b"{}"[..], &b"\0asm"[..]] {
        many.extend((part.len() as u32).to_le_bytes());
        many.extend(part);
    }
    many.extend((MAX_ASSETS as u32 + 1).to_le_bytes());
    assert!(Bundle::decode(&many, usize::MAX).unwrap_err().contains("assets"));
    // Random damage never panics.
    let mut seed = 7u64;
    for _ in 0..2000 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let mut b = bytes.clone();
        let i = (seed >> 33) as usize % b.len();
        b[i] = (seed >> 13) as u8;
        if seed.is_multiple_of(3) {
            b.truncate(i);
        }
        if let Ok(x) = Bundle::decode(&b, usize::MAX) {
            let _ = x.check();
        }
    }
}

#[test]
fn checking_refuses_bad_plugins() {
    let with = |f: &dyn Fn(&mut Bundle)| {
        let mut b = bundle();
        f(&mut b);
        b.check().unwrap_err()
    };
    assert!(with(&|b| b.manifest = "{".into()).contains("manifest"));
    assert!(with(&|b| b.manifest = manifest("1.0.0", 99)).contains("API 99"));
    for v in ["", "1.0-beta", "v1", "1..2", "1.2.3.4.5", "99999999999999999999"] {
        assert!(with(&|b| b.manifest = manifest(v, 1)).contains("version"), "{v}");
    }
    assert!(with(&|b| b.wasm = b"MZ not wasm".to_vec()).contains("WebAssembly"));
    let wasi = wat::parse_str(r#"(module (import "wasi_snapshot_preview1" "fd_write" (func (param i32 i32 i32 i32) (result i32))) (memory (export "memory") 1) (func (export "figcraft_run") (param i32) (result i32) i32.const 0))"#).unwrap();
    assert!(with(&|b| b.wasm = wasi.clone()).contains("imports"));
    for name in ["../x", "a/b", "a\\b", ".hidden", "con", "NUL.txt", "com1.png", "x.", "", "é.png"] {
        assert!(check_asset_name(name).is_err(), "{name}");
        assert!(with(&|b| b.assets[0].name = name.into()).contains("asset"), "{name}");
    }
    assert!(check_asset_name("console.png").is_ok() && check_asset_name("LICENSE").is_ok() && check_asset_name("com10.txt").is_ok());
    assert!(with(&|b| b.assets.push(Asset { name: "ICON.png".into(), data: vec![] })).contains("two assets"));
}

#[test]
fn versions_compare_by_number() {
    assert_eq!(parse_version("1.10.0"), Some(vec![1, 10, 0]));
    assert_eq!(compare_versions("1.10.0", "1.9.9"), Some(Ordering::Greater));
    assert_eq!(compare_versions("1.2", "1.2.0"), Some(Ordering::Equal));
    assert_eq!(compare_versions("1.2", "beta"), None);
    assert!(newer("2.0.0", "1.9") && !newer("1.0.0", "1.0.0") && !newer("1.0.0", "") && !newer("x", "1.0"));
}
