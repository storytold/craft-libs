//! Policy gate tests. Every reject rule has a WAT fixture in `tests/fixtures/`. The manifest
//! section is appended by `package()` so one fixture can pair with different manifests.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use craft_plugin_abi::gate::{ForbiddenFeature as F, MAX_PACKAGE_BYTES};
use craft_plugin_abi::{HostPolicy, LegacyAbi, PolicyReport, Violation, check_package};
use serde_json::{Value, json};

fn fixture(name: &str) -> Vec<u8> {
    let path = format!("{}/tests/fixtures/{name}.wat", env!("CARGO_MANIFEST_DIR"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    wat::parse_str(&text).unwrap_or_else(|e| panic!("{path}: {e}"))
}

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

fn with_section(mut wasm: Vec<u8>, name: &str, data: &[u8]) -> Vec<u8> {
    let mut body = Vec::new();
    leb(name.len(), &mut body);
    body.extend_from_slice(name.as_bytes());
    body.extend_from_slice(data);
    wasm.push(0);
    leb(body.len(), &mut wasm);
    wasm.extend_from_slice(&body);
    wasm
}

fn manifest_json() -> Value {
    json!({
        "schema": 1, "id": "org.example.t", "name": "T", "version": "1.0.0",
        "abi": { "craft": 1 },
        "requires": { "ops": ["host.log"] },
        "contributions": [{ "kind": "command", "id": "c", "entry": 0 }]
    })
}

fn package(fixture_name: &str, manifest: &Value) -> Vec<u8> {
    with_section(fixture(fixture_name), "craft.manifest", &serde_json::to_vec(manifest).unwrap())
}

fn gate(bytes: &[u8]) -> PolicyReport {
    check_package(bytes, &HostPolicy::native())
}

fn assert_only(report: &PolicyReport, expected: &[Violation]) {
    assert_eq!(report.violations, expected, "full report: {report:#?}");
}

#[test]
fn accepts_the_good_module() {
    for name in ["accept", "accept_no_import"] {
        let report = gate(&package(name, &manifest_json()));
        assert_only(&report, &[]);
        assert!(report.is_accepted());
        assert_eq!(report.legacy, None);
        assert_eq!(report.manifest.as_ref().map(|m| m.id.as_str()), Some("org.example.t"), "{name}");
    }
}

#[test]
fn rejects_missing_memory_maximum() {
    assert_only(&gate(&package("reject_no_memory_max", &manifest_json())), &[Violation::MemoryMaxMissing]);
}

#[test]
fn rejects_memory_maximum_above_host_cap_and_above_manifest_limit() {
    // 20000 pages = 1,310,720,000 bytes.
    let over_native = gate(&package("reject_memory_too_large", &manifest_json()));
    assert_only(&over_native, &[Violation::MemoryMaxTooLarge { max_bytes: 20_000 * 65_536, limit_bytes: 1024 * 1024 * 1024 }]);

    // accept.wat declares max 16 pages = 1 MiB. The web cap and limits.memory_mb must still allow it,
    // and a lowered cap must reject it.
    let bytes = package("accept", &manifest_json());
    assert!(check_package(&bytes, &HostPolicy::web()).is_accepted());
    let tiny = HostPolicy { memory_cap_mb: 0, simd128: false };
    assert_only(&check_package(&bytes, &tiny), &[Violation::MemoryMaxTooLarge { max_bytes: 16 * 65_536, limit_bytes: 0 }]);

    let mut m = manifest_json();
    m["limits"] = json!({ "memory_mb": 1 });
    assert!(gate(&package("accept", &m)).is_accepted(), "exactly at limit passes");
    let mut m = manifest_json();
    m["limits"] = json!({ "memory_mb": 1 });
    let report = gate(&package("reject_memory_too_large", &m));
    assert_only(&report, &[Violation::MemoryMaxTooLarge { max_bytes: 20_000 * 65_536, limit_bytes: 1024 * 1024 }]);
}

#[test]
fn rejects_start_function() {
    assert_only(&gate(&package("reject_start_function", &manifest_json())), &[Violation::StartFunction]);
}

#[test]
fn rejects_extra_import() {
    assert_only(&gate(&package("reject_extra_import", &manifest_json())), &[Violation::ExtraImport { module: "env".into(), name: "abort".into() }]);
}

#[test]
fn rejects_wrong_import_signature() {
    assert_only(
        &gate(&package("reject_wrong_import_signature", &manifest_json())),
        &[Violation::WrongImportSignature { found: "(i32 i32 i32 i32) -> (i64)".into() }],
    );
}

#[test]
fn rejects_relaxed_simd_threads_memory64_tail_calls_exceptions() {
    for (name, feature) in [
        ("reject_relaxed_simd", F::RelaxedSimd),
        ("reject_threads", F::Threads),
        ("reject_memory64", F::Memory64),
        ("reject_tail_call", F::TailCall),
        ("reject_exceptions", F::Exceptions),
        ("reject_multi_memory", F::MultiMemory),
    ] {
        let report = gate(&package(name, &manifest_json()));
        assert!(report.violations.contains(&Violation::Feature(feature)), "{name}: {report:#?}");
        assert!(!report.is_accepted(), "{name}");
    }
}

#[test]
fn relaxed_simd_is_reported_as_relaxed_even_when_simd_is_requested_and_supported() {
    let mut m = manifest_json();
    m["requires"]["features"] = json!(["simd128"]);
    let host = HostPolicy { simd128: true, ..HostPolicy::native() };
    let report = check_package(&package("reject_relaxed_simd", &m), &host);
    assert_only(&report, &[Violation::Feature(F::RelaxedSimd)]);
}

#[test]
fn simd128_needs_both_the_manifest_request_and_the_engine() {
    let simd_unrequested = gate(&package("reject_simd_unrequested", &manifest_json()));
    assert_only(&simd_unrequested, &[Violation::Feature(F::Simd128)]);

    let mut m = manifest_json();
    m["requires"]["features"] = json!(["simd128"]);
    // Requested, engine lacks it: the request and the use are both violations.
    let report = gate(&package("reject_simd_unrequested", &m));
    assert_eq!(report.violations, vec![Violation::Simd128Unsupported, Violation::Feature(F::Simd128)]);
    // Requested and supported: accepted.
    let host = HostPolicy { simd128: true, ..HostPolicy::native() };
    assert_only(&check_package(&package("reject_simd_unrequested", &m), &host), &[]);
}

/// A packaged module with 1 imported function, 4 exported ones, and `extra` empty ones.
fn module_with_extra_functions(extra: usize) -> Vec<u8> {
    let body: String = (0..extra).map(|_| "(func)\n").collect();
    let wat = format!(
        r#"(module (import "craft_host" "call" (func (param i32 i32 i32 i32 i32) (result i64)))
        (memory (export "memory") 1 16)
        (func (export "craft_abi") (result i32) i32.const 1)
        (func (export "craft_alloc") (param i32 i32) (result i32) i32.const 0)
        (func (export "craft_free") (param i32 i32 i32))
        (func (export "craft_invoke") (param i32 i32 i32 i32 i32) (result i64) i64.const 0)
        {body})"#
    );
    with_section(wat::parse_str(&wat).unwrap(), "craft.manifest", &serde_json::to_vec(&manifest_json()).unwrap())
}

#[test]
fn rejects_more_than_10000_functions() {
    let report = gate(&module_with_extra_functions(10_000));
    assert_only(&report, &[Violation::TooManyFunctions { count: 10_005 }]);
}

#[test]
fn function_count_includes_imports_and_boundary_is_inclusive() {
    assert_only(&gate(&module_with_extra_functions(9_995)), &[]);
    assert_only(&gate(&module_with_extra_functions(9_996)), &[Violation::TooManyFunctions { count: 10_001 }]);
}

#[test]
fn rejects_missing_and_mistyped_exports() {
    assert_only(&gate(&package("reject_missing_export", &manifest_json())), &[Violation::MissingExport { name: "craft_free" }]);
    assert_only(&gate(&package("reject_memory_not_exported", &manifest_json())), &[Violation::MemoryNotExported]);

    // And a wrong signature on an optional-or-conditional export is still caught when present.
    assert_only(
        &gate(&package("reject_wrong_export_signature", &manifest_json())),
        &[Violation::WrongExportSignature { name: "craft_process", found: "(i32) -> (i32)".into(), expected: "(i32 i32) -> (i32)".into() }],
    );
}

#[test]
fn craft_invoke_is_required_only_for_control_plane_contributions() {
    // accept.wat has craft_invoke and no craft_process: a command-only manifest passes (see accepts_the_good_module).
    // An effect-only manifest needs craft_process but not craft_invoke.
    let mut m = manifest_json();
    m["contributions"] = json!([{ "kind": "effect", "id": "e", "entry": 0 }]);
    let report = gate(&package("accept_no_import", &m));
    assert_only(&report, &[]);
    let report = gate(&package("accept", &m));
    assert_only(&report, &[Violation::MissingExport { name: "craft_process" }]);
}

#[test]
fn rejects_oversized_tables_and_too_many_tables() {
    assert_only(&gate(&package("reject_table_too_large", &manifest_json())), &[Violation::TableTooLarge { index: 0, elements: 100_001 }]);
    assert_only(&gate(&package("reject_too_many_tables", &manifest_json())), &[Violation::TooManyTables { count: 5 }]);
}

#[test]
fn lists_every_violation_not_just_the_first() {
    // Start function + extra import + a bad manifest op, in one package.
    let wat = r#"(module
        (import "env" "abort" (func))
        (memory (export "memory") 1)
        (func $s)
        (start $s)
        (func (export "craft_abi") (result i32) i32.const 1)
        (func (export "craft_alloc") (param i32 i32) (result i32) i32.const 0))"#;
    let mut m = manifest_json();
    m["requires"]["ops"] = json!(["host.log", "frames.input_at"]);
    let report = gate(&with_section(wat::parse_str(wat).unwrap(), "craft.manifest", &serde_json::to_vec(&m).unwrap()));
    for expected in [
        Violation::UnknownOp("frames.input_at".into()),
        Violation::MemoryMaxMissing,
        Violation::StartFunction,
        Violation::ExtraImport { module: "env".into(), name: "abort".into() },
        Violation::MissingExport { name: "craft_free" },
        Violation::MissingExport { name: "craft_invoke" },
    ] {
        assert!(report.violations.contains(&expected), "missing {expected:?} in {report:#?}");
    }
    assert_eq!(report.violations.len(), 6, "{report:#?}");
}

#[test]
fn manifest_problems_are_violations() {
    let wasm = fixture("accept");
    assert_only(&gate(&wasm), &[Violation::ManifestMissing]);

    let dup = with_section(package("accept", &manifest_json()), "craft.manifest", &serde_json::to_vec(&manifest_json()).unwrap());
    assert_only(&gate(&dup), &[Violation::ManifestDuplicate]);

    let mut reserved = manifest_json();
    reserved["id"] = json!("craft.sneaky");
    let report = gate(&package("accept", &reserved));
    assert!(matches!(report.violations.as_slice(), [Violation::ManifestInvalid(_)]), "{report:#?}");
    assert!(report.manifest.is_none());

    let mut abi2 = manifest_json();
    abi2["abi"]["craft"] = json!(2);
    assert!(matches!(gate(&package("accept", &abi2)).violations.as_slice(), [Violation::ManifestInvalid(_)]));

    let mut bad_perm = manifest_json();
    bad_perm["permissions"] = json!({ "required": ["root.everything"] });
    assert!(matches!(gate(&package("accept", &bad_perm)).violations.as_slice(), [Violation::ManifestInvalid(_)]));

    let garbage = with_section(fixture("accept"), "craft.manifest", b"not json");
    assert!(matches!(gate(&garbage).violations.as_slice(), [Violation::ManifestInvalid(_)]));
}

#[test]
fn unknown_requires_op_is_rejected_and_known_ops_pass() {
    let mut m = manifest_json();
    m["requires"]["ops"] = json!(["host.log", "cmd.execute", "random.bytes"]);
    assert_only(&gate(&package("accept", &m)), &[]);
    m["requires"]["ops"] = json!(["host.log", "teleport"]);
    assert_only(&gate(&package("accept", &m)), &[Violation::UnknownOp("teleport".into())]);
}

#[test]
fn unknown_kinds_need_no_entry_export_but_known_kinds_still_do() {
    // Only an unknown kind: nothing to run, so craft_invoke and craft_process are not required.
    let mut m = manifest_json();
    m["contributions"] = json!([{ "kind": "teleport", "id": "t", "entry": 0 }]);
    let report = gate(&package("accept_minimal", &m));
    assert_only(&report, &[]);
    assert_eq!(report.manifest.as_ref().unwrap().unsupported_contributions().count(), 1);
    // A known command beside it brings the craft_invoke rule back.
    m["contributions"] = json!([{ "kind": "teleport", "id": "t", "entry": 0 }, { "kind": "command", "id": "c", "entry": 1 }]);
    let report = gate(&package("accept_minimal", &m));
    assert!(matches!(report.violations.as_slice(), [Violation::MissingExport { name: "craft_invoke" }]), "{report:#?}");
    // A known effect beside it brings the craft_process rule back.
    m["contributions"] = json!([{ "kind": "teleport", "id": "t", "entry": 0 }, { "kind": "effect", "id": "e", "entry": 1 }]);
    let report = gate(&package("accept_minimal", &m));
    assert!(matches!(report.violations.as_slice(), [Violation::MissingExport { name: "craft_process" }]), "{report:#?}");
}

#[test]
fn package_size_cap_is_32_mib() {
    let mut big = package("accept", &manifest_json());
    let pad = vec![0u8; MAX_PACKAGE_BYTES];
    big = with_section(big, "padding", &pad);
    assert_only(&gate(&big), &[Violation::PackageTooLarge { bytes: big.len() }]);
}

#[test]
fn malformed_bytes_never_panic_and_are_reported() {
    for bytes in [&b""[..], b"\0asm", b"\0asm\x01\0\0\0\xFF", b"not wasm at all", &[0u8; 64]] {
        let report = gate(bytes);
        assert!(matches!(report.violations.as_slice(), [Violation::Malformed(_)]), "{bytes:?}: {report:#?}");
    }
    // Every truncation of a good package is either malformed or a rejected report, never accepted.
    let good = package("accept", &manifest_json());
    for n in 0..good.len() {
        assert!(!gate(&good[..n]).is_accepted(), "truncation at {n} was accepted");
    }
}

#[test]
fn legacy_modules_are_detected_by_export_prefix_and_bypass_the_gate() {
    for (name, abi) in [("legacy_ec", LegacyAbi::Effectcraft), ("legacy_pc", LegacyAbi::Photocraft), ("legacy_vc", LegacyAbi::Vectorcraft)] {
        // No manifest, no max on memory, no craft exports: all would be violations on the native path.
        let report = gate(&fixture(name));
        assert_eq!(report.legacy, Some(abi), "{name}");
        assert!(report.is_accepted(), "{name}: {report:#?}");
        assert_eq!(report.manifest, None);
    }
    let mixed = gate(&fixture("legacy_mixed"));
    assert_only(&mixed, &[Violation::MixedLegacyPrefixes]);
    assert_eq!(mixed.legacy, None);
}

/// A legacy prefix renames the exports the shim wants. It does not lift the resource caps.
#[test]
fn legacy_modules_still_meet_the_size_table_and_memory_caps() {
    for prefix in ["ec_", "pc_", "vc_"] {
        let legacy =
            |extra: &str| gate(&wat::parse_str(format!(r#"(module (memory (export "memory") 1) {extra} (func (export "{prefix}run")))"#)).unwrap());
        // Control: a small module within the caps loads.
        let ok = legacy("(table 10 funcref)");
        assert!(ok.legacy.is_some() && ok.is_accepted(), "{prefix}: {ok:#?}");
        let big_table = legacy("(table 200000000 funcref)");
        assert!(big_table.violations.contains(&Violation::TableTooLarge { index: 0, elements: 200_000_000 }), "{prefix}: {big_table:#?}");
        let many = legacy("(table 1 funcref) (table 1 funcref) (table 1 funcref) (table 1 funcref) (table 1 funcref)");
        assert!(many.violations.contains(&Violation::TooManyTables { count: 5 }), "{prefix}: {many:#?}");
        let two_mem = legacy("(memory 1)");
        assert!(two_mem.violations.contains(&Violation::MemoryCount { count: 2 }), "{prefix}: {two_mem:#?}");
        // The size cap runs before the legacy return.
        let mut huge = wat::parse_str(format!(r#"(module (memory (export "memory") 1) (func (export "{prefix}run")))"#)).unwrap();
        huge = with_section(huge, "padding", &vec![0u8; MAX_PACKAGE_BYTES]);
        let report = gate(&huge);
        assert!(report.violations.contains(&Violation::PackageTooLarge { bytes: huge.len() }), "{prefix}");
    }
}

#[test]
fn native_module_is_not_mistaken_for_legacy() {
    let report = gate(&package("accept", &manifest_json()));
    assert_eq!(report.legacy, None);
    // A prefix match must be on the export name start, not a substring.
    let wat = r#"(module (memory (export "memory") 1) (func (export "spec_run")) (func (export "myec_x")))"#;
    let report = gate(&wat::parse_str(wat).unwrap());
    assert_eq!(report.legacy, None);
    assert!(!report.is_accepted());
}
