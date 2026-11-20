//! Generated v1 plugin modules: the PhotoCraft (`pc_`) and VectorCraft (`vc_`) ABIs as WAT text, and
//! two small stand-in plugins, `invert` and `desaturate`. Used by the shim tests and the install tests.

#![allow(dead_code, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

/// The manifest of the stand-in PhotoCraft plugin.
pub const INVERT_MANIFEST: &str = r#"{"id":"test.photocraft.invert","name":"Invert","version":"1","kind":"filter"}"#;
/// The manifest of the stand-in VectorCraft plugin.
pub const DESATURATE_MANIFEST: &str = r#"{"id":"test.vectorcraft.desaturate","name":"Desaturate","version":"1","kind":"filter","params":{"amount":{"type":"number","min":0,"max":100,"default":100}}}"#;

/// WAT escape: every byte that is not a letter, digit or space becomes `\xx`.
pub fn wat_escape(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b == b' ' { char::from(b).to_string() } else { format!("\\{b:02x}") }).collect()
}

pub fn build(src: &str) -> Vec<u8> {
    wat::parse_str(src).unwrap_or_else(|e| panic!("bad test WAT: {e}\n{src}"))
}

/// A module implementing the PhotoCraft ABI whose `manifest`, `version`, `filter` body (params `$buf
/// $len $w $h $ch $fmt $pp $pl`, result i32) and extra items can be swapped.
pub fn pc_module(manifest: &str, version: i32, filter: &str, extra: &str) -> Vec<u8> {
    build(&format!(
        r#"(module
  {extra}
  (memory (export "memory") 1)
  (data (i32.const 16) "{data}")
  (func (export "pc_abi_version") (result i32) (i32.const {version}))
  (func (export "pc_manifest") (result i64)
    (i64.or (i64.shl (i64.const {len}) (i64.const 32)) (i64.const 16)))
  (func (export "pc_alloc") (param $n i32) (result i32) (local $old i32)
    (local.set $old (memory.grow (i32.shr_u (i32.add (local.get $n) (i32.const 65535)) (i32.const 16))))
    (if (result i32) (i32.eq (local.get $old) (i32.const -1))
      (then (i32.const 0)) (else (i32.mul (local.get $old) (i32.const 65536)))))
  (func (export "pc_filter") (param $buf i32) (param $len i32) (param $w i32) (param $h i32)
    (param $ch i32) (param $fmt i32) (param $pp i32) (param $pl i32) (result i32)
    {filter})
)"#,
        data = wat_escape(manifest),
        len = manifest.len()
    ))
}

/// The `pc_filter` body of the invert plugin: every colour sample `x` becomes `1 - x`. The last
/// channel is alpha when bit 8 of `format` is set, and alpha stays as it is.
pub const INVERT_FILTER: &str = r#"(local $i i32) (local $n i32) (local $c i32) (local $alpha i32) (local $at i32)
  (local.set $n (i32.mul (i32.mul (local.get $w) (local.get $h)) (local.get $ch)))
  (local.set $alpha (i32.and (i32.shr_u (local.get $fmt) (i32.const 8)) (i32.const 1)))
  (block $done (loop $l
    (br_if $done (i32.ge_u (local.get $i) (local.get $n)))
    (local.set $c (i32.rem_u (local.get $i) (local.get $ch)))
    (if (i32.eqz (i32.and (local.get $alpha) (i32.eq (local.get $c) (i32.sub (local.get $ch) (i32.const 1)))))
      (then
        (local.set $at (i32.add (local.get $buf) (i32.shl (local.get $i) (i32.const 2))))
        (f32.store (local.get $at) (f32.sub (f32.const 1) (f32.load (local.get $at))))))
    (local.set $i (i32.add (local.get $i) (i32.const 1)))
    (br $l)))
  (i32.const 0)"#;

/// The stand-in PhotoCraft plugin: an invert filter with no parameters.
pub fn invert_module() -> Vec<u8> {
    pc_module(INVERT_MANIFEST, 1, INVERT_FILTER, "")
}

/// A VectorCraft plugin with `manifest`, a `vc_run` body, extra module fields and `pages` of memory.
pub fn vc_wat(manifest: &str, run: &str, extra: &str, pages: u32) -> String {
    format!(
        r#"(module
  {extra}
  (memory (export "memory") {pages})
  (data (i32.const 16) "{data}")
  (func (export "vc_abi_version") (result i32) (i32.const 1))
  (func (export "vc_manifest") (result i64) (i64.or (i64.shl (i64.const {len}) (i64.const 32)) (i64.const 16)))
  (func (export "vc_alloc") (param $n i32) (result i32) (local $old i32)
    (local.set $old (memory.grow (i32.shr_u (i32.add (local.get $n) (i32.const 65535)) (i32.const 16))))
    (if (result i32) (i32.eq (local.get $old) (i32.const -1)) (then (i32.const 0)) (else (i32.mul (local.get $old) (i32.const 65536)))))
  (func (export "vc_run") (param $in i32) (param $il i32) (param $pp i32) (param $pl i32) (result i64)
    {run}))"#,
        data = wat_escape(manifest),
        len = manifest.len(),
    )
}

pub fn vc_module(manifest: &str, run: &str, extra: &str) -> Vec<u8> {
    build(&vc_wat(manifest, run, extra, 1))
}

/// The `vc_run` body that answers with its input.
pub const ECHO_INPUT: &str = "(i64.or (i64.shl (i64.extend_i32_u (local.get $il)) (i64.const 32)) (i64.extend_i32_u (local.get $in)))";

/// The `vc_run` body that answers with its params, so a test sees what the host resolved.
pub const ECHO_PARAMS: &str = "(i64.or (i64.shl (i64.extend_i32_u (local.get $pl)) (i64.const 32)) (i64.extend_i32_u (local.get $pp)))";

/// The stand-in VectorCraft plugin: a `filter` with one `amount` parameter that answers with the
/// params it received.
pub fn desaturate_module() -> Vec<u8> {
    vc_module(DESATURATE_MANIFEST, ECHO_PARAMS, "")
}
