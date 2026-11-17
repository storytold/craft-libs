//! WebAssembly modules: validation, sections, imports, exports, functions with their signatures
//! and names, memories, tables, globals, custom sections (names, producers, target features,
//! source map URL) and a text (WAT) rendering. Built on the pure-Rust `wasmparser` and
//! `wasmprinter` crates; never executes anything.

use crate::FormatError;
use crate::bytes::Reader;
use serde::Serialize;
use std::collections::BTreeMap;
use wasmparser::{CompositeInnerType, Encoding, ExternalKind, FuncType, KnownCustom, Name, Parser, Payload, TypeRef, Validator, WasmFeatures};

/// Largest module we read (256 MiB).
pub const MAX_INPUT: usize = 256 << 20;
/// Most entries kept per list.
pub const MAX_ITEMS: usize = 100_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WasmSection {
    pub id: u8,
    pub name: String,
    pub offset: u64,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WasmImport {
    pub module: String,
    pub name: String,
    /// func, table, memory, global or tag.
    pub kind: &'static str,
    pub type_index: Option<u32>,
    /// Function signature or a short description of the imported item.
    pub signature: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WasmExport {
    pub name: String,
    pub kind: &'static str,
    pub index: u32,
    /// Name of the exported function from the name section, when known.
    pub target_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WasmFunction {
    /// Index in the function index space (imports first).
    pub index: u32,
    pub type_index: u32,
    pub signature: String,
    pub body_offset: u64,
    pub body_size: u64,
    pub locals: u64,
    pub name: Option<String>,
    pub exported_as: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WasmMemory {
    pub index: u32,
    pub initial: u64,
    pub maximum: Option<u64>,
    pub shared: bool,
    pub memory64: bool,
    pub imported: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WasmTable {
    pub index: u32,
    pub element_type: String,
    pub initial: u64,
    pub maximum: Option<u64>,
    pub imported: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WasmGlobal {
    pub index: u32,
    pub value_type: String,
    pub mutable: bool,
    pub imported: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WasmCustom {
    pub name: String,
    pub offset: u64,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProducerValue {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProducerField {
    pub field: String,
    pub values: Vec<ProducerValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WasmValidation {
    pub valid: bool,
    pub error: Option<String>,
    pub offset: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WasmReport {
    /// `module` or `component`.
    pub encoding: &'static str,
    pub version: u16,
    pub size: usize,
    pub validation: WasmValidation,
    /// Set when the structure could not be read to the end.
    pub parse_error: Option<String>,
    pub sections: Vec<WasmSection>,
    pub types: usize,
    pub imports: Vec<WasmImport>,
    pub imported_functions: u32,
    pub exports: Vec<WasmExport>,
    /// Functions defined in the module (imports are listed under `imports`).
    pub functions: Vec<WasmFunction>,
    pub memories: Vec<WasmMemory>,
    pub tables: Vec<WasmTable>,
    pub globals: Vec<WasmGlobal>,
    pub start_function: Option<u32>,
    pub data_segments: u32,
    pub element_segments: u32,
    pub data_count: Option<u32>,
    pub custom_sections: Vec<WasmCustom>,
    pub module_name: Option<String>,
    pub producers: Vec<ProducerField>,
    pub target_features: Vec<String>,
    pub source_mapping_url: Option<String>,
    pub external_debug_info: Option<String>,
    pub truncated: bool,
    pub warnings: Vec<String>,
}

/// True when the bytes start with the WebAssembly magic.
pub fn is_wasm(data: &[u8]) -> bool {
    data.starts_with(b"\0asm")
}

fn sig(f: &FuncType) -> String {
    let p: Vec<String> = f.params().iter().map(|v| v.to_string()).collect();
    let r: Vec<String> = f.results().iter().map(|v| v.to_string()).collect();
    format!("({}) -> ({})", p.join(", "), r.join(", "))
}

fn section_name(id: u8) -> &'static str {
    match id {
        0 => "custom",
        1 => "type",
        2 => "import",
        3 => "function",
        4 => "table",
        5 => "memory",
        6 => "global",
        7 => "export",
        8 => "start",
        9 => "element",
        10 => "code",
        11 => "data",
        12 => "datacount",
        13 => "tag",
        _ => "unknown",
    }
}

fn kind_name(k: ExternalKind) -> &'static str {
    match k {
        ExternalKind::Func | ExternalKind::FuncExact => "func",
        ExternalKind::Table => "table",
        ExternalKind::Memory => "memory",
        ExternalKind::Global => "global",
        ExternalKind::Tag => "tag",
    }
}

/// Validate with every feature enabled; the result never fails the caller.
pub fn validate(data: &[u8]) -> WasmValidation {
    match Validator::new_with_features(WasmFeatures::all()).validate_all(data) {
        Ok(_) => WasmValidation { valid: true, error: None, offset: None },
        Err(e) => WasmValidation { valid: false, error: Some(e.message().to_string()), offset: Some(e.offset()) },
    }
}

/// A length-prefixed UTF-8 string at the start of a custom section payload.
fn leading_string(data: &[u8]) -> Option<String> {
    let r = Reader::new(data);
    let (n, used) = r.uleb128(0)?;
    let s = r.slice(used, usize::try_from(n).ok()?)?;
    Some(String::from_utf8_lossy(s).into_owned())
}

fn target_features(data: &[u8]) -> Vec<String> {
    let r = Reader::new(data);
    let mut out = Vec::new();
    let Some((n, mut p)) = r.uleb128(0) else { return out };
    for _ in 0..n.min(1024) {
        let Some(prefix) = r.u8(p) else { break };
        let Some((len, used)) = r.uleb128(p + 1) else { break };
        let Some(s) = usize::try_from(len).ok().and_then(|l| r.slice(p + 1 + used, l)) else { break };
        out.push(format!("{}{}", char::from(prefix), String::from_utf8_lossy(s)));
        p = p + 1 + used + s.len();
    }
    out
}

/// Read a module. Fails only when the bytes are not WebAssembly at all or too large; structural
/// damage is reported in `parse_error` and `validation`.
pub fn inspect(data: &[u8]) -> Result<WasmReport, FormatError> {
    if data.len() > MAX_INPUT {
        return Err(FormatError::TooLarge(format!("{} bytes; the limit is {MAX_INPUT}", data.len())));
    }
    if !is_wasm(data) {
        return Err(FormatError::Malformed("no WebAssembly magic (\\0asm)".into()));
    }
    let mut rep = WasmReport {
        encoding: "module",
        version: 0,
        size: data.len(),
        validation: validate(data),
        parse_error: None,
        sections: Vec::new(),
        types: 0,
        imports: Vec::new(),
        imported_functions: 0,
        exports: Vec::new(),
        functions: Vec::new(),
        memories: Vec::new(),
        tables: Vec::new(),
        globals: Vec::new(),
        start_function: None,
        data_segments: 0,
        element_segments: 0,
        data_count: None,
        custom_sections: Vec::new(),
        module_name: None,
        producers: Vec::new(),
        target_features: Vec::new(),
        source_mapping_url: None,
        external_debug_info: None,
        truncated: false,
        warnings: Vec::new(),
    };
    let mut types: Vec<Option<FuncType>> = Vec::new();
    let mut func_types: Vec<u32> = Vec::new();
    let mut fn_names: BTreeMap<u32, String> = BTreeMap::new();
    let mut imported_tables = 0u32;
    let mut imported_memories = 0u32;
    let mut imported_globals = 0u32;
    let mut code_index = 0u32;
    let type_sig = |types: &[Option<FuncType>], t: u32| -> String {
        match types.get(t as usize) {
            Some(Some(f)) => sig(f),
            Some(None) => format!("type {t} (not a function type)"),
            None => format!("type {t} (missing)"),
        }
    };
    let cap = |len: usize, truncated: &mut bool| -> bool {
        if len >= MAX_ITEMS {
            *truncated = true;
            false
        } else {
            true
        }
    };
    let result: Result<(), wasmparser::BinaryReaderError> = (|| {
        for payload in Parser::new(0).parse_all(data) {
            let payload = payload?;
            if let Some((id, range)) = payload.as_section()
                && cap(rep.sections.len(), &mut rep.truncated)
            {
                let name = match &payload {
                    Payload::CustomSection(c) => format!("custom:{}", c.name()),
                    _ => section_name(id).to_string(),
                };
                rep.sections.push(WasmSection { id, name, offset: range.start, size: range.end.saturating_sub(range.start) });
            }
            match payload {
                Payload::Version { num, encoding, .. } => {
                    rep.version = num;
                    if encoding == Encoding::Component {
                        rep.encoding = "component";
                        rep.warnings.push("this is a component, not a core module; only validation and the top-level layout are reported".into());
                        break;
                    }
                }
                Payload::TypeSection(r) => {
                    for rg in r {
                        for st in rg?.into_types() {
                            if types.len() >= MAX_ITEMS {
                                rep.truncated = true;
                                break;
                            }
                            types.push(match st.composite_type.inner {
                                CompositeInnerType::Func(f) => Some(f),
                                _ => None,
                            });
                        }
                    }
                    rep.types = types.len();
                }
                Payload::ImportSection(r) => {
                    for imp in r.into_imports() {
                        let imp = imp?;
                        let (kind, type_index, signature) = match imp.ty {
                            TypeRef::Func(t) | TypeRef::FuncExact(t) => {
                                rep.imported_functions = rep.imported_functions.saturating_add(1);
                                ("func", Some(t), type_sig(&types, t))
                            }
                            TypeRef::Table(t) => {
                                if cap(rep.tables.len(), &mut rep.truncated) {
                                    rep.tables.push(WasmTable {
                                        index: imported_tables,
                                        element_type: format!("{:?}", t.element_type),
                                        initial: t.initial,
                                        maximum: t.maximum,
                                        imported: true,
                                    });
                                }
                                imported_tables = imported_tables.saturating_add(1);
                                ("table", None, format!("table {} min {}", format!("{:?}", t.element_type).to_lowercase(), t.initial))
                            }
                            TypeRef::Memory(m) => {
                                if cap(rep.memories.len(), &mut rep.truncated) {
                                    rep.memories.push(WasmMemory {
                                        index: imported_memories,
                                        initial: m.initial,
                                        maximum: m.maximum,
                                        shared: m.shared,
                                        memory64: m.memory64,
                                        imported: true,
                                    });
                                }
                                imported_memories = imported_memories.saturating_add(1);
                                ("memory", None, format!("memory min {} pages", m.initial))
                            }
                            TypeRef::Global(g) => {
                                if cap(rep.globals.len(), &mut rep.truncated) {
                                    rep.globals.push(WasmGlobal {
                                        index: imported_globals,
                                        value_type: g.content_type.to_string(),
                                        mutable: g.mutable,
                                        imported: true,
                                    });
                                }
                                imported_globals = imported_globals.saturating_add(1);
                                ("global", None, format!("{}{}", if g.mutable { "mut " } else { "" }, g.content_type))
                            }
                            TypeRef::Tag(t) => ("tag", Some(t.func_type_idx), type_sig(&types, t.func_type_idx)),
                        };
                        if cap(rep.imports.len(), &mut rep.truncated) {
                            rep.imports.push(WasmImport { module: imp.module.to_string(), name: imp.name.to_string(), kind, type_index, signature });
                        }
                    }
                }
                Payload::FunctionSection(r) => {
                    for t in r {
                        if func_types.len() >= MAX_ITEMS {
                            rep.truncated = true;
                            break;
                        }
                        func_types.push(t?);
                    }
                }
                Payload::TableSection(r) => {
                    for (i, t) in r.into_iter().enumerate() {
                        let t = t?;
                        if !cap(rep.tables.len(), &mut rep.truncated) {
                            break;
                        }
                        rep.tables.push(WasmTable {
                            index: imported_tables.saturating_add(u32::try_from(i).unwrap_or(u32::MAX)),
                            element_type: format!("{:?}", t.ty.element_type),
                            initial: t.ty.initial,
                            maximum: t.ty.maximum,
                            imported: false,
                        });
                    }
                }
                Payload::MemorySection(r) => {
                    for (i, m) in r.into_iter().enumerate() {
                        let m = m?;
                        if !cap(rep.memories.len(), &mut rep.truncated) {
                            break;
                        }
                        rep.memories.push(WasmMemory {
                            index: imported_memories.saturating_add(u32::try_from(i).unwrap_or(u32::MAX)),
                            initial: m.initial,
                            maximum: m.maximum,
                            shared: m.shared,
                            memory64: m.memory64,
                            imported: false,
                        });
                    }
                }
                Payload::GlobalSection(r) => {
                    for (i, g) in r.into_iter().enumerate() {
                        let g = g?;
                        if !cap(rep.globals.len(), &mut rep.truncated) {
                            break;
                        }
                        rep.globals.push(WasmGlobal {
                            index: imported_globals.saturating_add(u32::try_from(i).unwrap_or(u32::MAX)),
                            value_type: g.ty.content_type.to_string(),
                            mutable: g.ty.mutable,
                            imported: false,
                        });
                    }
                }
                Payload::ExportSection(r) => {
                    for e in r {
                        let e = e?;
                        if !cap(rep.exports.len(), &mut rep.truncated) {
                            break;
                        }
                        rep.exports.push(WasmExport { name: e.name.to_string(), kind: kind_name(e.kind), index: e.index, target_name: None });
                    }
                }
                Payload::StartSection { func, .. } => rep.start_function = Some(func),
                Payload::ElementSection(r) => rep.element_segments = r.count(),
                Payload::DataCountSection { count, .. } => rep.data_count = Some(count),
                Payload::DataSection(r) => rep.data_segments = r.count(),
                Payload::CodeSectionEntry(body) => {
                    let ti = func_types.get(code_index as usize).copied();
                    let mut locals = 0u64;
                    let lr = body.get_locals_reader()?;
                    for l in lr.into_iter().take(10_000) {
                        let (n, _) = l?;
                        locals = locals.saturating_add(u64::from(n));
                    }
                    let range = body.range();
                    if cap(rep.functions.len(), &mut rep.truncated) {
                        rep.functions.push(WasmFunction {
                            index: rep.imported_functions.saturating_add(code_index),
                            type_index: ti.unwrap_or(u32::MAX),
                            signature: ti.map(|t| type_sig(&types, t)).unwrap_or_else(|| "no matching function section entry".into()),
                            body_offset: range.start,
                            body_size: range.end.saturating_sub(range.start),
                            locals,
                            name: None,
                            exported_as: Vec::new(),
                        });
                    }
                    code_index = code_index.saturating_add(1);
                }
                Payload::CustomSection(c) => {
                    if cap(rep.custom_sections.len(), &mut rep.truncated) {
                        let range = c.range();
                        rep.custom_sections.push(WasmCustom { name: c.name().to_string(), offset: range.start, size: range.end.saturating_sub(range.start) });
                    }
                    match c.name() {
                        "sourceMappingURL" => rep.source_mapping_url = leading_string(c.data()),
                        "external_debug_info" => rep.external_debug_info = leading_string(c.data()),
                        "target_features" => rep.target_features = target_features(c.data()),
                        _ => {}
                    }
                    match c.as_known() {
                        KnownCustom::Name(names) => {
                            // A damaged name section is reported but does not stop the rest.
                            for sub in names.into_iter().take(64) {
                                let Ok(sub) = sub else {
                                    rep.warnings.push("the name section is damaged".into());
                                    break;
                                };
                                match sub {
                                    Name::Module { name, .. } => rep.module_name = Some(name.to_string()),
                                    Name::Function(map) => {
                                        for n in map.into_iter().take(MAX_ITEMS) {
                                            match n {
                                                Ok(n) => {
                                                    fn_names.insert(n.index, n.name.to_string());
                                                }
                                                Err(_) => {
                                                    rep.warnings.push("a function name entry is damaged".into());
                                                    break;
                                                }
                                            }
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                        KnownCustom::Producers(p) => {
                            for f in p.into_iter().take(64) {
                                let Ok(f) = f else {
                                    rep.warnings.push("the producers section is damaged".into());
                                    break;
                                };
                                let values = f
                                    .values
                                    .into_iter()
                                    .take(64)
                                    .filter_map(Result::ok)
                                    .map(|v| ProducerValue { name: v.name.to_string(), version: v.version.to_string() })
                                    .collect();
                                rep.producers.push(ProducerField { field: f.name.to_string(), values });
                            }
                        }
                        _ => {}
                    }
                }
                Payload::End(_) => {}
                _ => {}
            }
        }
        Ok(())
    })();
    if let Err(e) = result {
        rep.parse_error = Some(format!("{} (at byte {})", e.message(), e.offset()));
    }
    // Attach names and exports to functions.
    let mut exported: BTreeMap<u32, Vec<String>> = BTreeMap::new();
    for e in rep.exports.iter_mut() {
        if e.kind == "func" {
            e.target_name = fn_names.get(&e.index).cloned();
            exported.entry(e.index).or_default().push(e.name.clone());
        }
    }
    for f in rep.functions.iter_mut() {
        f.name = fn_names.get(&f.index).cloned();
        f.exported_as = exported.remove(&f.index).unwrap_or_default();
    }
    if func_types.len() != code_index as usize && rep.parse_error.is_none() {
        rep.warnings.push(format!("{} function declarations but {} bodies", func_types.len(), code_index));
    }
    Ok(rep)
}

struct CappedText {
    out: String,
    cap: usize,
    truncated: bool,
}

impl wasmprinter::Print for CappedText {
    fn write_str(&mut self, s: &str) -> std::io::Result<()> {
        if self.out.len().saturating_add(s.len()) > self.cap {
            let room = self.cap.saturating_sub(self.out.len());
            let mut end = room.min(s.len());
            while end > 0 && !s.is_char_boundary(end) {
                end -= 1;
            }
            self.out.push_str(s.get(..end).unwrap_or(""));
            self.truncated = true;
            return Err(std::io::Error::other("output cap reached"));
        }
        self.out.push_str(s);
        Ok(())
    }
}

/// The text format of a module, at most `cap` bytes: (text, truncated). Only call this on
/// modules that passed [`validate`]; damaged modules are not rendered.
pub fn to_wat(data: &[u8], cap: usize) -> Result<(String, bool), String> {
    let mut p = CappedText { out: String::new(), cap, truncated: false };
    let mut cfg = wasmprinter::Config::new();
    cfg.name_unnamed(false);
    match cfg.print(data, &mut p) {
        Ok(()) => Ok((p.out, false)),
        Err(_) if p.truncated => Ok((p.out, true)),
        Err(e) => Err(e.to_string()),
    }
}

/// What a JavaScript glue file says about the module.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GlueScan {
    /// String literals ending in `.wasm`.
    pub wasm_references: Vec<String>,
    /// Imports (`module.name`) whose module and field names both appear in the glue.
    pub matched_imports: Vec<String>,
    pub mentions_instantiate: bool,
}

/// Scan glue JavaScript for `.wasm` literals and the import names the module expects.
pub fn scan_glue(text: &str, imports: &[WasmImport]) -> GlueScan {
    let mut refs = Vec::new();
    let b = text.as_bytes();
    let mut i = 0usize;
    while let Some(p) = text.get(i..).and_then(|t| t.find(".wasm")) {
        let at = i + p;
        let end = at + 5;
        if let Some(&q) = b.get(end)
            && matches!(q, b'"' | b'\'' | b'`')
        {
            let start = b.get(..at).and_then(|h| h.iter().rposition(|&c| c == q)).map(|s| s + 1);
            if let Some(s) = start
                && at - s <= 256
                && let Some(lit) = text.get(s..end)
                && !lit.contains(['\n', ' '])
                && !refs.iter().any(|r: &String| r == lit)
                && refs.len() < 256
            {
                refs.push(lit.to_string());
            }
        }
        i = end;
    }
    let matched = imports
        .iter()
        .filter(|im| text.contains(im.module.as_str()) && text.contains(im.name.as_str()))
        .take(MAX_ITEMS)
        .map(|im| format!("{}.{}", im.module, im.name))
        .collect();
    GlueScan { wasm_references: refs, matched_imports: matched, mentions_instantiate: text.contains("WebAssembly.instantiate") }
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft_anything_testkit::formats as fx;

    #[test]
    fn reads_the_sample_module() {
        let m = fx::wasm_module();
        let r = inspect(&m).unwrap();
        assert!(r.validation.valid, "{:?}", r.validation);
        assert!(r.parse_error.is_none());
        assert_eq!(r.types, 3);
        assert_eq!(r.imports.len(), 1);
        assert_eq!(r.imports[0].signature, "(i32) -> ()");
        assert_eq!(r.imported_functions, 1);
        assert_eq!(r.functions.len(), 2);
        assert_eq!(r.functions[0].name.as_deref(), Some("add"));
        assert_eq!(r.functions[0].signature, "(i32, i32) -> (i32)");
        assert_eq!(r.functions[0].exported_as, vec!["add"]);
        assert_eq!(r.exports.len(), 3);
        assert_eq!(r.memories.len(), 1);
        assert_eq!(r.globals.len(), 1);
        assert!(r.globals[0].mutable);
        assert_eq!(r.data_segments, 1);
        assert_eq!(r.module_name.as_deref(), Some("demo"));
        assert_eq!(r.producers.len(), 2);
        assert_eq!(r.producers[1].values[0].name, "rustc");
        assert_eq!(r.source_mapping_url.as_deref(), Some("demo.wasm.map"));
        assert_eq!(r.target_features, vec!["+mutable-globals"]);
        let (wat, trunc) = to_wat(&m, 1 << 20).unwrap();
        assert!(!trunc);
        assert!(wat.contains("(module $demo"), "{wat}");
        assert!(wat.contains("i32.add"));
        let (short, trunc) = to_wat(&m, 40).unwrap();
        assert!(trunc && short.len() <= 40);
        let g = scan_glue(fx::WASM_GLUE, &r.imports);
        assert_eq!(g.wasm_references, vec!["./demo.wasm"]);
        assert_eq!(g.matched_imports, vec!["env.log"]);
        assert!(g.mentions_instantiate);
    }

    #[test]
    fn damaged_modules_are_reported_not_fatal() {
        let m = fx::wasm_module();
        let mut bad = m.clone();
        let len = bad.len();
        bad.truncate(len - 10);
        let r = inspect(&bad).unwrap();
        assert!(!r.validation.valid);
        assert!(r.parse_error.is_some());
        assert!(inspect(b"not wasm").is_err());
    }

    #[test]
    fn hostile_inputs_do_not_panic() {
        let m = fx::wasm_module();
        for x in craft_anything_testkit::mutations(&m) {
            if let Ok(r) = inspect(&x)
                && r.validation.valid
            {
                let _ = to_wat(&x, 4096);
            }
        }
        for n in 0..m.len() {
            let _ = inspect(&m[..n]);
        }
        for seed in 0..100 {
            let mut n = craft_anything_testkit::noise(seed, 256);
            n[..8].copy_from_slice(b"\0asm\x01\0\0\0");
            let _ = inspect(&n);
            let _ = scan_glue(&String::from_utf8_lossy(&n), &[]);
        }
    }
}
