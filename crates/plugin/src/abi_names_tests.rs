//! The shared ABI's names (`craft`, `craft_run`) and FigCraft's older ones (`figcraft`,
//! `figcraft_run`) run the same host functions.

use crate::{ENTRY, Host, IMPORT_MODULE, Limits, PluginError, check_module, run};
use crate::{LEGACY_ENTRY, LEGACY_IMPORT_MODULE};

#[derive(Default)]
struct Mock {
    calls: Vec<String>,
}

impl Host for Mock {
    fn call(&mut self, command: &str, _params: &str) -> Result<String, String> {
        self.calls.push(command.into());
        Ok("{}".into())
    }
    fn selection(&mut self) -> String {
        "[]".into()
    }
    fn notify(&mut self, _message: &str) {}
}

/// A module that calls `select.all` through `call_from` and writes `ok` through `output_from`,
/// exporting `entry`.
fn module(call_from: &str, output_from: &str, entry: &str) -> Vec<u8> {
    let wat = format!(
        r#"(module
          (import "{call_from}" "call" (func $call (param i32 i32 i32 i32) (result i32)))
          (import "{output_from}" "set_output" (func $set_output (param i32 i32)))
          (memory (export "memory") 1)
          (data (i32.const 0) "select.all") (data (i32.const 16) "ok")
          (func (export "{entry}") (param $n i32) (result i32)
            (drop (call $call (i32.const 0) (i32.const 10) (i32.const 0) (i32.const 0)))
            (call $set_output (i32.const 16) (i32.const 2))
            (i32.const 0)))"#
    );
    wat::parse_str(&wat).unwrap()
}

fn go(m: &[u8]) -> (Result<String, PluginError>, Vec<String>) {
    let mut host = Mock::default();
    let r = run(m, "{}", &Limits::default(), &mut host).map(|o| o.output);
    (r, host.calls)
}

#[test]
fn the_names_are_the_documented_ones() {
    assert_eq!((IMPORT_MODULE, ENTRY), ("craft", "craft_run"));
    assert_eq!((LEGACY_IMPORT_MODULE, LEGACY_ENTRY), ("figcraft", "figcraft_run"));
}

#[test]
fn craft_and_figcraft_modules_both_run() {
    for (imports, entry) in [("craft", "craft_run"), ("figcraft", "figcraft_run"), ("craft", "figcraft_run"), ("figcraft", "craft_run")] {
        let m = module(imports, imports, entry);
        check_module(&m, &Limits::default()).unwrap_or_else(|e| panic!("{imports}/{entry}: {e}"));
        let (out, calls) = go(&m);
        assert_eq!(out.as_deref(), Ok("ok"), "{imports}/{entry}");
        assert_eq!(calls, vec!["select.all".to_string()], "{imports}/{entry}");
    }
}

#[test]
fn one_module_may_mix_both_import_names() {
    let (out, calls) = go(&module("craft", "figcraft", "craft_run"));
    assert_eq!((out.as_deref(), calls.len()), (Ok("ok"), 1));
}

#[test]
fn craft_run_wins_when_both_entry_points_are_exported() {
    let wat = r#"(module
      (import "craft" "set_output" (func $set_output (param i32 i32)))
      (memory (export "memory") 1)
      (data (i32.const 0) "new") (data (i32.const 8) "old")
      (func (export "craft_run") (param i32) (result i32) (call $set_output (i32.const 0) (i32.const 3)) (i32.const 0))
      (func (export "figcraft_run") (param i32) (result i32) (call $set_output (i32.const 8) (i32.const 3)) (i32.const 0)))"#;
    assert_eq!(go(&wat::parse_str(wat).unwrap()).0.as_deref(), Ok("new"));
}

#[test]
fn other_import_modules_and_bad_entry_points_are_refused() {
    let invalid = |wat: &str| {
        let r = check_module(&wat::parse_str(wat).unwrap(), &Limits::default());
        assert!(matches!(r, Err(PluginError::InvalidModule(_))), "{wat}: {r:?}");
    };
    invalid(
        r#"(module (import "craftx" "call" (func (param i32 i32 i32 i32) (result i32))) (memory (export "memory") 1) (func (export "craft_run") (param i32) (result i32) (i32.const 0)))"#,
    );
    invalid(
        r#"(module (import "env" "log" (func (param i32 i32))) (memory (export "memory") 1) (func (export "craft_run") (param i32) (result i32) (i32.const 0)))"#,
    );
    // A `craft_run` of the wrong type isn't rescued by a correct `figcraft_run`.
    invalid(
        r#"(module (memory (export "memory") 1) (func (export "craft_run") (result i32) (i32.const 0)) (func (export "figcraft_run") (param i32) (result i32) (i32.const 0)))"#,
    );
    invalid(r#"(module (memory (export "memory") 1) (func (export "run") (param i32) (result i32) (i32.const 0)))"#);
}
