//! The install-time policy gate from `ABI.md` section 2. It reads the package bytes with
//! `wasmparser` only. No guest code runs. Every violation is listed, not just the first.

use std::fmt;

use wasmparser::{CompositeInnerType, ExternalKind, FuncType, Parser, Payload, TypeRef, ValType, Validator, WasmFeatures};

use crate::manifest::{Feature, Manifest, ManifestError};
use crate::ops::Op;

pub const MAX_PACKAGE_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_FUNCTIONS: usize = 10_000;
pub const MAX_TABLES: usize = 4;
pub const MAX_TABLE_ELEMENTS: u64 = 100_000;
pub const MANIFEST_SECTION: &str = "craft.manifest";

/// What this host and engine can offer. Defaults match `ABI.md` section 2.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HostPolicy {
    /// Largest declared memory maximum accepted, in MiB.
    pub memory_cap_mb: u32,
    /// True when the engine reports `SIMD128`. Off until the engine passes its conformance tests.
    pub simd128: bool,
}

impl HostPolicy {
    pub const fn native() -> HostPolicy {
        HostPolicy { memory_cap_mb: 1024, simd128: false }
    }

    pub const fn web() -> HostPolicy {
        HostPolicy { memory_cap_mb: 256, simd128: false }
    }
}

impl Default for HostPolicy {
    fn default() -> Self {
        HostPolicy::native()
    }
}

/// A v1 plugin ABI that loads through a shim and skips the gate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LegacyAbi {
    Effectcraft,
    Photocraft,
    Vectorcraft,
}

impl LegacyAbi {
    pub fn export_prefix(self) -> &'static str {
        match self {
            LegacyAbi::Effectcraft => "ec_",
            LegacyAbi::Photocraft => "pc_",
            LegacyAbi::Vectorcraft => "vc_",
        }
    }

    const ALL: [LegacyAbi; 3] = [LegacyAbi::Effectcraft, LegacyAbi::Photocraft, LegacyAbi::Vectorcraft];
}

/// A proposal the gate refuses to let through.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ForbiddenFeature {
    RelaxedSimd,
    Threads,
    Memory64,
    Gc,
    Exceptions,
    TailCall,
    MultiMemory,
    /// SIMD that the manifest did not request or the host cannot run.
    Simd128,
}

/// One broken rule.
#[derive(Clone, Debug, PartialEq)]
pub enum Violation {
    PackageTooLarge {
        bytes: usize,
    },
    /// The bytes do not parse as a core module.
    Malformed(String),
    /// The module parses but fails validation for a reason the gate cannot name.
    InvalidModule(String),
    Feature(ForbiddenFeature),
    ManifestMissing,
    ManifestDuplicate,
    ManifestInvalid(ManifestError),
    UnknownOp(String),
    /// The manifest asks for `simd128` and this engine does not report it.
    Simd128Unsupported,
    MemoryCount {
        count: usize,
    },
    MemoryNotExported,
    MemoryMaxMissing,
    MemoryMaxTooLarge {
        max_bytes: u128,
        limit_bytes: u128,
    },
    TooManyTables {
        count: usize,
    },
    TableTooLarge {
        index: usize,
        elements: u64,
    },
    TooManyFunctions {
        count: usize,
    },
    StartFunction,
    ExtraImport {
        module: String,
        name: String,
    },
    WrongImportSignature {
        found: String,
    },
    MissingExport {
        name: &'static str,
    },
    WrongExportKind {
        name: &'static str,
    },
    WrongExportSignature {
        name: &'static str,
        found: String,
        expected: String,
    },
    /// Exports from more than one legacy family (`ec_`, `pc_`, `vc_`).
    MixedLegacyPrefixes,
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Violation::PackageTooLarge { bytes } => write!(f, "package is {bytes} bytes, cap is {MAX_PACKAGE_BYTES}"),
            Violation::Malformed(m) => write!(f, "malformed module: {m}"),
            Violation::InvalidModule(m) => write!(f, "invalid module: {m}"),
            Violation::Feature(x) => write!(f, "module uses a rejected feature: {x:?}"),
            Violation::ManifestMissing => write!(f, "custom section {MANIFEST_SECTION} is missing"),
            Violation::ManifestDuplicate => write!(f, "custom section {MANIFEST_SECTION} appears more than once"),
            Violation::ManifestInvalid(e) => write!(f, "{e}"),
            Violation::UnknownOp(n) => write!(f, "requires.ops names unknown op {n:?}"),
            Violation::Simd128Unsupported => f.write_str("manifest requires simd128 and this engine does not report it"),
            Violation::MemoryCount { count } => write!(f, "module has {count} memories, exactly 1 is required"),
            Violation::MemoryNotExported => f.write_str("the memory is not exported as \"memory\""),
            Violation::MemoryMaxMissing => f.write_str("memory has no declared maximum"),
            Violation::MemoryMaxTooLarge { max_bytes, limit_bytes } => {
                write!(f, "memory maximum {max_bytes} bytes exceeds the limit {limit_bytes} bytes")
            }
            Violation::TooManyTables { count } => write!(f, "module has {count} tables, at most {MAX_TABLES} allowed"),
            Violation::TableTooLarge { index, elements } => write!(f, "table {index} has {elements} elements, at most {MAX_TABLE_ELEMENTS} allowed"),
            Violation::TooManyFunctions { count } => write!(f, "module has {count} functions, at most {MAX_FUNCTIONS} allowed"),
            Violation::StartFunction => f.write_str("module has a start function"),
            Violation::ExtraImport { module, name } => write!(f, "import {module}.{name} is not allowed"),
            Violation::WrongImportSignature { found } => write!(f, "craft_host.call has signature {found}, expected (i32 i32 i32 i32 i32) -> i64"),
            Violation::MissingExport { name } => write!(f, "required export {name} is missing"),
            Violation::WrongExportKind { name } => write!(f, "export {name} has the wrong kind"),
            Violation::WrongExportSignature { name, found, expected } => write!(f, "export {name} has signature {found}, expected {expected}"),
            Violation::MixedLegacyPrefixes => f.write_str("exports mix ec_, pc_ and vc_ prefixes"),
        }
    }
}

/// The gate result. `violations` is empty exactly when the package may load.
#[derive(Clone, Debug, PartialEq)]
pub struct PolicyReport {
    pub violations: Vec<Violation>,
    /// Set when the exports carry a legacy prefix. The package then skips the export, manifest and
    /// import checks, but still meets the size, memory count, table and function caps.
    pub legacy: Option<LegacyAbi>,
    /// Set when the manifest parsed and validated.
    pub manifest: Option<Manifest>,
}

impl PolicyReport {
    pub fn is_accepted(&self) -> bool {
        self.violations.is_empty()
    }
}

/// Signature as text, for violation messages.
fn sig_text(params: &[ValType], results: &[ValType]) -> String {
    let list = |v: &[ValType]| v.iter().map(|t| t.to_string()).collect::<Vec<_>>().join(" ");
    format!("({}) -> ({})", list(params), list(results))
}

fn ft_text(f: &FuncType) -> String {
    sig_text(f.params(), f.results())
}

struct ExpectedExport {
    name: &'static str,
    params: &'static [ValType],
    results: &'static [ValType],
}

const I32: ValType = ValType::I32;
const I64: ValType = ValType::I64;

const CRAFT_ABI: ExpectedExport = ExpectedExport { name: "craft_abi", params: &[], results: &[I32] };
const CRAFT_ALLOC: ExpectedExport = ExpectedExport { name: "craft_alloc", params: &[I32, I32], results: &[I32] };
const CRAFT_FREE: ExpectedExport = ExpectedExport { name: "craft_free", params: &[I32, I32, I32], results: &[] };
const CRAFT_INVOKE: ExpectedExport = ExpectedExport { name: "craft_invoke", params: &[I32, I32, I32, I32, I32], results: &[I64] };
const CRAFT_PROCESS: ExpectedExport = ExpectedExport { name: "craft_process", params: &[I32, I32], results: &[I32] };
const CRAFT_STATE_SAVE: ExpectedExport = ExpectedExport { name: "craft_state_save", params: &[I32], results: &[I32] };
const CRAFT_STATE_LOAD: ExpectedExport = ExpectedExport { name: "craft_state_load", params: &[I32, I32, I32, I32], results: &[I32] };

/// Facts gathered in one pass over the module sections.
#[derive(Default)]
struct Facts<'a> {
    types: Vec<Option<FuncType>>,
    /// Type index of every function in index space order (imports first).
    func_types: Vec<u32>,
    imports: Vec<(&'a str, &'a str, TypeRef)>,
    memories: Vec<wasmparser::MemoryType>,
    tables: Vec<wasmparser::TableType>,
    exports: Vec<(&'a str, ExternalKind, u32)>,
    has_start: bool,
    manifests: Vec<&'a [u8]>,
}

fn gather<'a>(bytes: &'a [u8]) -> Result<Facts<'a>, String> {
    let mut facts = Facts::default();
    for payload in Parser::new(0).parse_all(bytes) {
        match payload.map_err(|e| e.to_string())? {
            Payload::TypeSection(reader) => {
                for group in reader {
                    for ty in group.map_err(|e| e.to_string())?.into_types() {
                        facts.types.push(match ty.composite_type.inner {
                            CompositeInnerType::Func(f) => Some(f),
                            _ => None,
                        });
                    }
                }
            }
            Payload::ImportSection(reader) => {
                for import in reader.into_imports() {
                    let import = import.map_err(|e| e.to_string())?;
                    match import.ty {
                        TypeRef::Func(t) | TypeRef::FuncExact(t) => facts.func_types.push(t),
                        TypeRef::Memory(m) => facts.memories.push(m),
                        TypeRef::Table(t) => facts.tables.push(t),
                        _ => {}
                    }
                    facts.imports.push((import.module, import.name, import.ty));
                }
            }
            Payload::FunctionSection(reader) => {
                for t in reader {
                    facts.func_types.push(t.map_err(|e| e.to_string())?);
                }
            }
            Payload::TableSection(reader) => {
                for t in reader {
                    facts.tables.push(t.map_err(|e| e.to_string())?.ty);
                }
            }
            Payload::MemorySection(reader) => {
                for m in reader {
                    facts.memories.push(m.map_err(|e| e.to_string())?);
                }
            }
            Payload::ExportSection(reader) => {
                for e in reader {
                    let e = e.map_err(|e| e.to_string())?;
                    facts.exports.push((e.name, e.kind, e.index));
                }
            }
            Payload::StartSection { .. } => facts.has_start = true,
            Payload::CustomSection(reader) if reader.name() == MANIFEST_SECTION => facts.manifests.push(reader.data()),
            _ => {}
        }
    }
    Ok(facts)
}

fn legacy_of(facts: &Facts<'_>) -> Result<Option<LegacyAbi>, ()> {
    let mut found: Option<LegacyAbi> = None;
    for (name, _, _) in &facts.exports {
        if let Some(abi) = LegacyAbi::ALL.into_iter().find(|a| name.starts_with(a.export_prefix())) {
            match found {
                Some(prev) if prev != abi => return Err(()),
                _ => found = Some(abi),
            }
        }
    }
    Ok(found)
}

fn func_type<'a>(facts: &'a Facts<'_>, func_index: u32) -> Option<&'a FuncType> {
    let ty = *facts.func_types.get(func_index as usize)?;
    facts.types.get(ty as usize)?.as_ref()
}

fn check_export(facts: &Facts<'_>, want: &ExpectedExport, required: bool, out: &mut Vec<Violation>) {
    let Some((_, kind, index)) = facts.exports.iter().find(|(n, _, _)| *n == want.name) else {
        if required {
            out.push(Violation::MissingExport { name: want.name });
        }
        return;
    };
    if *kind != ExternalKind::Func {
        out.push(Violation::WrongExportKind { name: want.name });
        return;
    }
    let expected = sig_text(want.params, want.results);
    match func_type(facts, *index) {
        Some(f) if f.params() == want.params && f.results() == want.results => {}
        Some(f) => out.push(Violation::WrongExportSignature { name: want.name, found: ft_text(f), expected }),
        None => out.push(Violation::WrongExportSignature { name: want.name, found: "unresolvable".to_owned(), expected }),
    }
}

/// The caps every package meets, legacy or not: tables, function count, and (legacy only here, the
/// craft-abi 1 path checks memory itself) the memory count.
fn check_resource_caps(facts: &Facts<'_>, legacy: bool, out: &mut Vec<Violation>) {
    if legacy && facts.memories.len() > 1 {
        out.push(Violation::MemoryCount { count: facts.memories.len() });
    }
    if facts.tables.len() > MAX_TABLES {
        out.push(Violation::TooManyTables { count: facts.tables.len() });
    }
    for (index, t) in facts.tables.iter().enumerate() {
        let elements = t.maximum.map_or(t.initial, |m| m.max(t.initial));
        if elements > MAX_TABLE_ELEMENTS {
            out.push(Violation::TableTooLarge { index, elements });
        }
    }
    if facts.func_types.len() > MAX_FUNCTIONS {
        out.push(Violation::TooManyFunctions { count: facts.func_types.len() });
    }
}

fn check_structure(facts: &Facts<'_>, manifest: Option<&Manifest>, host: &HostPolicy, out: &mut Vec<Violation>) {
    // Memory: exactly one, exported as `memory`, bounded maximum.
    if facts.memories.len() != 1 {
        out.push(Violation::MemoryCount { count: facts.memories.len() });
    }
    match facts.exports.iter().find(|(n, _, _)| *n == "memory") {
        Some((_, ExternalKind::Memory, _)) => {}
        _ => out.push(Violation::MemoryNotExported),
    }
    let mut limit_mb = u64::from(host.memory_cap_mb);
    if let Some(m) = manifest.and_then(|m| m.limits.as_ref()).and_then(|l| l.memory_mb) {
        limit_mb = limit_mb.min(u64::from(m));
    }
    let limit_bytes = u128::from(limit_mb) * 1024 * 1024;
    for mem in &facts.memories {
        match mem.maximum {
            None => out.push(Violation::MemoryMaxMissing),
            Some(max_pages) => {
                let page = 1u128 << mem.page_size_log2.unwrap_or(16);
                let max_bytes = u128::from(max_pages) * page;
                if max_bytes > limit_bytes {
                    out.push(Violation::MemoryMaxTooLarge { max_bytes, limit_bytes });
                }
            }
        }
    }

    check_resource_caps(facts, false, out);
    if facts.has_start {
        out.push(Violation::StartFunction);
    }

    // Imports: exactly craft_host.call (i32 x5) -> i64, or none.
    let want_import = FuncType::new([I32, I32, I32, I32, I32], [I64]);
    for (module, name, ty) in &facts.imports {
        if *module != "craft_host" || *name != "call" {
            out.push(Violation::ExtraImport { module: (*module).to_owned(), name: (*name).to_owned() });
            continue;
        }
        match ty {
            TypeRef::Func(t) | TypeRef::FuncExact(t) => match facts.types.get(*t as usize).and_then(Option::as_ref) {
                Some(f) if *f == want_import => {}
                Some(f) => out.push(Violation::WrongImportSignature { found: ft_text(f) }),
                None => out.push(Violation::WrongImportSignature { found: "unresolvable".to_owned() }),
            },
            _ => out.push(Violation::ExtraImport { module: (*module).to_owned(), name: (*name).to_owned() }),
        }
    }

    // Exports: section 3.
    let (control, data) = manifest.map_or((false, false), |m| {
        (m.contributions.iter().any(|c| c.kind.is_control_plane()), m.contributions.iter().any(|c| c.kind.is_data_plane()))
    });
    for want in [&CRAFT_ABI, &CRAFT_ALLOC, &CRAFT_FREE] {
        check_export(facts, want, true, out);
    }
    check_export(facts, &CRAFT_INVOKE, control, out);
    check_export(facts, &CRAFT_PROCESS, data, out);
    check_export(facts, &CRAFT_STATE_SAVE, false, out);
    check_export(facts, &CRAFT_STATE_LOAD, false, out);
}

fn check_manifest(facts: &Facts<'_>, host: &HostPolicy, out: &mut Vec<Violation>) -> Option<Manifest> {
    let data = match facts.manifests.as_slice() {
        [] => {
            out.push(Violation::ManifestMissing);
            return None;
        }
        [one] => *one,
        [first, ..] => {
            out.push(Violation::ManifestDuplicate);
            first
        }
    };
    let manifest = match Manifest::parse(data) {
        Ok(m) => m,
        Err(e) => {
            out.push(Violation::ManifestInvalid(e));
            return None;
        }
    };
    if let Some(req) = &manifest.requires {
        for name in &req.ops {
            if Op::from_name(name).is_none() {
                out.push(Violation::UnknownOp(name.clone()));
            }
        }
        if req.features.contains(&Feature::Simd128) && !host.simd128 {
            out.push(Violation::Simd128Unsupported);
        }
    }
    Some(manifest)
}

fn allowed_features(manifest: Option<&Manifest>, host: &HostPolicy) -> WasmFeatures {
    let mut f = WasmFeatures::MUTABLE_GLOBAL
        | WasmFeatures::SATURATING_FLOAT_TO_INT
        | WasmFeatures::SIGN_EXTENSION
        | WasmFeatures::REFERENCE_TYPES
        | WasmFeatures::MULTI_VALUE
        | WasmFeatures::BULK_MEMORY
        | WasmFeatures::FLOATS;
    let wants_simd = manifest.and_then(|m| m.requires.as_ref()).is_some_and(|r| r.features.contains(&Feature::Simd128));
    if wants_simd && host.simd128 {
        f |= WasmFeatures::SIMD;
    }
    f
}

/// Validate with the allowed features. On failure, name the rejected feature by re-validating with
/// each one switched on: a module that then passes uses that feature.
fn check_features(bytes: &[u8], allowed: WasmFeatures, out: &mut Vec<Violation>) {
    let first = match Validator::new_with_features(allowed).validate_all(bytes) {
        Ok(_) => return,
        Err(e) => e,
    };
    let probes = [
        // Plain SIMD first. Relaxed SIMD implies SIMD, so its probe would also pass for plain SIMD.
        (ForbiddenFeature::Simd128, WasmFeatures::SIMD),
        (ForbiddenFeature::RelaxedSimd, WasmFeatures::SIMD | WasmFeatures::RELAXED_SIMD),
        (ForbiddenFeature::Threads, WasmFeatures::THREADS),
        (ForbiddenFeature::Memory64, WasmFeatures::MEMORY64),
        (ForbiddenFeature::Gc, WasmFeatures::GC | WasmFeatures::FUNCTION_REFERENCES | WasmFeatures::GC_TYPES),
        (ForbiddenFeature::Exceptions, WasmFeatures::EXCEPTIONS),
        (ForbiddenFeature::TailCall, WasmFeatures::TAIL_CALL),
        (ForbiddenFeature::MultiMemory, WasmFeatures::MULTI_MEMORY),
    ];
    let before = out.len();
    for (feature, flags) in probes {
        if feature == ForbiddenFeature::RelaxedSimd && out.contains(&Violation::Feature(ForbiddenFeature::Simd128)) {
            continue;
        }
        if Validator::new_with_features(allowed | flags).validate_all(bytes).is_ok() {
            out.push(Violation::Feature(feature));
        }
    }
    if out.len() == before {
        out.push(Violation::InvalidModule(first.to_string()));
    }
}

/// Run the gate on a package. Always returns a report. Never panics on hostile input.
pub fn check_package(bytes: &[u8], host: &HostPolicy) -> PolicyReport {
    let mut report = PolicyReport { violations: Vec::new(), legacy: None, manifest: None };
    let facts = match gather(bytes) {
        Ok(f) => f,
        Err(m) => {
            report.violations.push(Violation::Malformed(m));
            return report;
        }
    };
    match legacy_of(&facts) {
        Ok(Some(abi)) => {
            // A legacy prefix changes the export names the shim wants, not the resource caps.
            // The size, memory count, table and function caps still apply. Imports are refused by the
            // shim itself. A start function runs under the shim's fuel and wall clock.
            report.legacy = Some(abi);
            if bytes.len() > MAX_PACKAGE_BYTES {
                report.violations.push(Violation::PackageTooLarge { bytes: bytes.len() });
            }
            check_resource_caps(&facts, true, &mut report.violations);
            return report;
        }
        Ok(None) => {}
        Err(()) => {
            report.violations.push(Violation::MixedLegacyPrefixes);
            return report;
        }
    }
    if bytes.len() > MAX_PACKAGE_BYTES {
        report.violations.push(Violation::PackageTooLarge { bytes: bytes.len() });
    }
    report.manifest = check_manifest(&facts, host, &mut report.violations);
    check_structure(&facts, report.manifest.as_ref(), host, &mut report.violations);
    check_features(bytes, allowed_features(report.manifest.as_ref(), host), &mut report.violations);
    report
}
