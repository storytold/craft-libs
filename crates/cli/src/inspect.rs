//! `craft inspect`: what is in a `.wasm` plugin, and does the install gate accept it.
//!
//! The gate is `craft_plugin_abi::check_package`, the code the apps run at install. Nothing here
//! executes guest code.

use std::fmt::Write as _;
use std::path::Path;

use craft_plugin_abi::manifest::Manifest;
use craft_plugin_abi::permissions::PromptLevel;
use craft_plugin_abi::{Op, PolicyReport};
use serde_json::{Value, json};
use wasmparser::{Parser, Payload, TypeRef};

use crate::error::{Result, io_at};
use crate::gate;

/// Facts read straight from the module, for a developer who wants to know why the gate said no.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ModuleFacts {
    /// `(minimum pages, maximum pages)` of each memory.
    pub memories: Vec<(u64, Option<u64>)>,
    pub imports: Vec<String>,
    pub exports: Vec<String>,
    /// `(name, bytes)` of each custom section.
    pub custom_sections: Vec<(String, usize)>,
}

pub struct Inspection {
    pub path: String,
    pub size: usize,
    pub report: PolicyReport,
    pub facts: ModuleFacts,
}

impl Inspection {
    pub fn accepted(&self) -> bool {
        self.report.is_accepted()
    }
}

fn read_facts(bytes: &[u8]) -> ModuleFacts {
    let mut facts = ModuleFacts::default();
    for payload in Parser::new(0).parse_all(bytes) {
        // A malformed tail stops the walk. The gate report carries the reason.
        let Ok(payload) = payload else { break };
        match payload {
            Payload::ImportSection(reader) => {
                for import in reader.into_imports().flatten() {
                    if let TypeRef::Memory(m) = import.ty {
                        facts.memories.push((m.initial, m.maximum));
                    }
                    facts.imports.push(format!("{}.{}", import.module, import.name));
                }
            }
            Payload::MemorySection(reader) => {
                for m in reader.into_iter().flatten() {
                    facts.memories.push((m.initial, m.maximum));
                }
            }
            Payload::ExportSection(reader) => facts.exports.extend(reader.into_iter().flatten().map(|e| e.name.to_owned())),
            Payload::CustomSection(reader) => facts.custom_sections.push((reader.name().to_owned(), reader.data().len())),
            _ => {}
        }
    }
    facts
}

pub fn inspect_bytes(path: &str, bytes: &[u8]) -> Inspection {
    Inspection { path: path.to_owned(), size: bytes.len(), report: gate::check(bytes), facts: read_facts(bytes) }
}

pub fn inspect_file(path: &Path) -> Result<Inspection> {
    let bytes = std::fs::read(path).map_err(|e| io_at(path, e))?;
    Ok(inspect_bytes(&path.display().to_string(), &bytes))
}

fn level_name(level: PromptLevel) -> &'static str {
    match level {
        PromptLevel::Low => "low, shown at install",
        PromptLevel::Normal => "normal prompt",
        PromptLevel::High => "high prompt",
        PromptLevel::OnFirstUse => "asked on first use",
    }
}

fn pages_to_mib(pages: u64) -> f64 {
    pages as f64 * 65_536.0 / (1024.0 * 1024.0)
}

fn write_manifest(out: &mut String, m: &Manifest) {
    let _ = writeln!(out, "plugin:       {} {:?} version {}", m.id, m.name, m.version);
    if let Some(d) = &m.description {
        let _ = writeln!(out, "description:  {d}");
    }
    let _ = writeln!(out, "abi:          craft {}", m.abi.craft);
    let ops = m.requires.as_ref().map(|r| r.ops.as_slice()).unwrap_or_default();
    let _ = writeln!(out, "ops:          {} declared", ops.len());
    for name in ops {
        match Op::from_name(name) {
            Some(op) => {
                let needs = op.permission().map_or_else(|| "no permission".to_owned(), |k| format!("permission kind {k:?}"));
                let _ = writeln!(out, "  {name:<16} 0x{:04X}  {needs}", op.number());
            }
            None => {
                let _ = writeln!(out, "  {name:<16} unknown to this build");
            }
        }
    }
    match m.parsed_permissions() {
        Ok((required, optional)) => {
            let _ = writeln!(out, "permissions:  {} required, {} optional", required.len(), optional.len());
            for (group, list) in [("required", &required), ("optional", &optional)] {
                for p in list {
                    let _ = writeln!(out, "  {group:<9} {p}  [{}]", level_name(p.prompt_level()));
                }
            }
        }
        Err(e) => {
            let _ = writeln!(out, "permissions:  {e}");
        }
    }
    if let Some(l) = &m.limits {
        let show = |v: Option<u32>| v.map_or_else(|| "unset".to_owned(), |n| n.to_string());
        let _ = writeln!(out, "limits:       memory_mb {}, stack_kb {}", show(l.memory_mb), show(l.stack_kb));
    }
    let _ = writeln!(out, "contributions ({}):", m.contributions.len());
    for c in &m.contributions {
        let plane = if c.kind.is_data_plane() {
            "craft_process"
        } else if c.kind.is_control_plane() {
            "craft_invoke"
        } else {
            "not run by this build"
        };
        let _ = writeln!(
            out,
            "  [{}] {:<12} {:<20} via {plane}{}",
            c.entry,
            c.kind.name(),
            c.id,
            c.label.as_deref().map(|l| format!("  {l:?}")).unwrap_or_default()
        );
    }
}

/// The text `craft inspect` prints.
pub fn render_text(i: &Inspection) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "package:      {} ({} bytes)", i.path, i.size);
    let _ = writeln!(out, "gate:         {}", gate::summary(&i.report));
    match &i.report.manifest {
        Some(m) => write_manifest(&mut out, m),
        None => {
            let _ = writeln!(out, "manifest:     none usable (see the gate result)");
        }
    }
    for (n, (min, max)) in i.facts.memories.iter().enumerate() {
        let max_text = max.map_or_else(|| "none (the gate rejects this)".to_owned(), |m| format!("{m} pages = {:.0} MiB", pages_to_mib(m)));
        let _ = writeln!(out, "memory {n}:     initial {min} pages = {:.1} MiB, maximum {max_text}", pages_to_mib(*min));
    }
    let _ = writeln!(out, "imports:      {}", if i.facts.imports.is_empty() { "none".to_owned() } else { i.facts.imports.join(", ") });
    let _ = writeln!(out, "exports:      {}", i.facts.exports.join(", "));
    let sections: Vec<String> = i.facts.custom_sections.iter().map(|(n, len)| format!("{n} ({len} bytes)")).collect();
    let _ = writeln!(out, "custom:       {}", sections.join(", "));
    out
}

/// The JSON `craft inspect --json` prints.
pub fn to_json(i: &Inspection) -> Value {
    json!({
        "path": i.path,
        "size": i.size,
        "accepted": i.accepted(),
        "legacy": i.report.legacy.map(|l| l.export_prefix()),
        "violations": i.report.violations.iter().map(ToString::to_string).collect::<Vec<_>>(),
        "manifest": i.report.manifest.as_ref().and_then(|m| serde_json::to_value(m).ok()),
        "memories": i.facts.memories.iter().map(|(min, max)| json!({ "initial_pages": min, "maximum_pages": max })).collect::<Vec<_>>(),
        "imports": i.facts.imports,
        "exports": i.facts.exports,
        "custom_sections": i.facts.custom_sections.iter().map(|(n, len)| json!({ "name": n, "bytes": len })).collect::<Vec<_>>(),
    })
}
