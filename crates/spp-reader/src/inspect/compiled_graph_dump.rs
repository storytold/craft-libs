//! The compiled graphs of a project's embedded graph packages as text, for decoding work.
//! Per node: opcode, variant, flags, raw body words; every body word that points into the node and decodes
//! as an expression is listed with graph-input uids replaced by their identifiers.

use crate::decode::DecodeError;
use crate::decode::graph_package::compiled::{Assembly, AtomicNode, Expr, Kind, NodeRecord, Operands, atomic};
use crate::decode::graph_package::{Description, GraphPackage};
use std::fmt::Write;

/// Text for every graph in `package` whose label, url or member name contains `filter` (all when empty).
pub fn dump_package(package: &[u8], filter: &str, out: &mut String) -> Result<usize, DecodeError> {
    let archive = GraphPackage::read(package)?;
    let mut shown = 0;
    for g in archive.graphs() {
        let desc = Description::parse(g.xml)?;
        if !(filter.is_empty() || g.stem.contains(filter) || desc.pkg_url.contains(filter) || desc.label.contains(filter)) {
            continue;
        }
        let asm = Assembly::parse(g.compiled_graph)?;
        dump_graph(g.stem, &desc, &asm, out);
        shown += 1;
    }
    Ok(shown)
}

pub fn dump_graph(stem: &str, desc: &Description, asm: &Assembly, out: &mut String) {
    let name = |uid: u32| desc.input(uid).map(|i| i.identifier.clone()).or_else(|| desc.output(uid).map(|o| format!("out:{}", o.identifier)));
    let _ = writeln!(out, "== {stem} ({}, {}) version {} nodes {}", desc.label, desc.pkg_url, asm.version, asm.nodes.len());
    for i in &asm.inputs {
        let _ = writeln!(out, "  input {:#010x} type {} {}", i.uid, i.ty, name(i.uid).unwrap_or_default());
    }
    for o in &asm.outputs {
        let _ = writeln!(out, "  output {o:#010x} {}", name(*o).unwrap_or_default());
    }
    let s = asm.section();
    for n in &asm.nodes {
        let _ = writeln!(out, "#{} @{:x} op {:02x} var {:02x} flags {:04x} | {}", n.index, n.offset, n.opcode, n.variant, n.flags, words(n));
        for (i, &w) in n.body.iter().enumerate() {
            if !n.points_inside(w) || w <= n.word_offset(i) {
                continue;
            }
            let Ok(e) = Expr::parse(&s, w) else { continue };
            if e.end > n.end {
                continue;
            }
            let _ = writeln!(out, "    [{i}] -> @{w:x} expr {} instrs{}", e.instrs.len(), summary(&e, &name));
            if e.instrs.len() > 1 {
                for line in e.listing() {
                    let _ = writeln!(out, "        {}", resolve(&line, &e, &name));
                }
            }
        }
    }
}

fn words(n: &NodeRecord) -> String {
    n.body.iter().take(24).map(|w| format!("{w:08x}")).collect::<Vec<_>>().join(" ") + if n.body.len() > 24 { " …" } else { "" }
}

fn summary(e: &Expr, name: &dyn Fn(u32) -> Option<String>) -> String {
    if let Some(uid) = e.as_input() {
        return format!(": input {}", name(uid).unwrap_or_else(|| format!("{uid:#x}")));
    }
    if let Some(v) = e.as_constant() {
        return format!(": const {v:08x?}");
    }
    String::new()
}

fn resolve(line: &str, e: &Expr, name: &dyn Fn(u32) -> Option<String>) -> String {
    let mut line = line.to_string();
    for ins in &e.instrs {
        if let Operands::Input(uid) = ins.operands
            && let Some(n) = name(uid)
        {
            line = line.replace(&format!("{uid:#010x}"), &n);
        }
    }
    line
}

/// Counts over many packages: graphs that parse, node opcodes (with the graphs using them) and how many
/// pointed-to expressions decode, with the first failure reasons.
pub fn stats<'a>(packages: impl Iterator<Item = (&'a str, &'a [u8])>) -> String {
    use std::collections::BTreeMap;
    let (mut graphs, mut failed) = (0usize, Vec::new());
    let mut ops: BTreeMap<u8, (usize, Vec<String>)> = BTreeMap::new();
    // per opcode: layouts that end where the record's data starts, that do not, that fail to decode
    let mut layouts: BTreeMap<u8, [usize; 3]> = BTreeMap::new();
    let mut layout_bad: BTreeMap<u8, String> = BTreeMap::new();
    let (mut exprs, mut expr_fail) = (0usize, BTreeMap::<String, usize>::new());
    for (name, data) in packages {
        let archive = match GraphPackage::read(data) {
            Ok(a) => a,
            Err(e) => {
                failed.push(format!("{name}: {e}"));
                continue;
            }
        };
        for g in archive.graphs() {
            let label = Description::parse(g.xml).map(|d| d.pkg_url).unwrap_or_default();
            let asm = match Assembly::parse(g.compiled_graph) {
                Ok(a) => a,
                Err(e) => {
                    failed.push(format!("{name} {}: {e}", g.stem));
                    continue;
                }
            };
            graphs += 1;
            let s = asm.section();
            for n in &asm.nodes {
                match atomic::decode(n, &s) {
                    Ok(a) if a.kind == Kind::Unknown => {}
                    Ok(a) => {
                        let ok = layout_fits(n, &a);
                        let e = layouts.entry(n.opcode).or_default();
                        if ok {
                            e[0] += 1;
                        } else {
                            e[1] += 1;
                        }
                        if !ok {
                            layout_bad.entry(n.opcode).or_insert_with(|| format!("{label} #{} flags {:04x} | {}", n.index, n.flags, words(n)));
                        }
                    }
                    Err(e) => {
                        layouts.entry(n.opcode).or_default()[2] += 1;
                        layout_bad.entry(n.opcode).or_insert_with(|| format!("{label} #{}: {e} | {}", n.index, words(n)));
                    }
                }
                let entry = ops.entry(n.opcode).or_default();
                entry.0 += 1;
                if entry.1.len() < 6 && !entry.1.contains(&label) {
                    entry.1.push(label.clone());
                }
                for (i, &w) in n.body.iter().enumerate() {
                    if !n.points_inside(w) || w <= n.word_offset(i) {
                        continue;
                    }
                    // only words that look like an expression header: a small count and a const/input/get token
                    let Ok(head) = s.u32(w) else { continue };
                    let (count, tok) = (head & 0xffff, (head >> 16) & 0x3f);
                    if count == 0 || count > 4096 || !matches!(tok, 0x00 | 0x01 | 0x02 | 0x04) {
                        continue;
                    }
                    match Expr::parse(&s, w) {
                        Ok(_) => exprs += 1,
                        Err(e) => {
                            let key = e.to_string();
                            let key = key.split(" at ").next().unwrap_or(&key).to_string();
                            *expr_fail.entry(key).or_default() += 1;
                        }
                    }
                }
            }
        }
    }
    let mut out = String::new();
    let _ = writeln!(out, "graphs {graphs}, failed {}", failed.len());
    for f in failed.iter().take(20) {
        let _ = writeln!(out, "  !! {f}");
    }
    let _ = writeln!(out, "expressions decoded {exprs}");
    for (k, v) in &expr_fail {
        let _ = writeln!(out, "  {v:6} {k}");
    }
    for (op, [ok, bad, err]) in &layouts {
        let _ = writeln!(out, "layout {op:02x}: fits {ok}, does not fit {bad}, fails {err}");
        if let Some(b) = layout_bad.get(op) {
            let _ = writeln!(out, "    e.g. {}", b.chars().take(220).collect::<String>());
        }
    }
    for (op, (count, users)) in &ops {
        let _ = writeln!(out, "op {op:02x} {count:7}  {}", users.join(" "));
    }
    out
}

/// Whether a decoded header ends where the record's own expression data starts: no body word before the
/// header end may point into the record past the header, and some data pointer (if any) points to the end.
fn layout_fits(n: &NodeRecord, a: &AtomicNode) -> bool {
    let header_words = ((a.header_end - n.offset - 4) / 4) as usize;
    if header_words > n.body.len() {
        return false;
    }
    let inside: Vec<u32> = n.body.iter().take(header_words).copied().filter(|&w| w > n.offset + 4 && w < n.end && w % 4 == 0).collect();
    match inside.iter().min() {
        Some(&first) => first == a.header_end,
        None => true,
    }
}

/// The decoded (typed) form of every graph in `package` matching `filter`: one line per node with its
/// operation, inputs and parameters; undecodable nodes are marked.
pub fn ir_package(package: &[u8], filter: &str, out: &mut String) -> Result<usize, DecodeError> {
    let archive = GraphPackage::read(package)?;
    let mut shown = 0;
    for g in archive.graphs() {
        let desc = Description::parse(g.xml)?;
        if !(filter.is_empty() || g.stem.contains(filter) || desc.pkg_url.contains(filter) || desc.label.contains(filter)) {
            continue;
        }
        let asm = Assembly::parse(g.compiled_graph)?;
        let name = |uid: u32| desc.input(uid).map(|i| i.identifier.clone()).unwrap_or_else(|| format!("{uid:#x}"));
        let _ = writeln!(out, "== {} ({}) nodes {}", desc.label, desc.pkg_url, asm.nodes.len());
        let (mut ok, mut unknown, mut failed) = (0, 0, 0);
        for (i, n) in asm.atomic_nodes().into_iter().enumerate() {
            match n {
                Ok(a) if a.kind == Kind::Unknown => {
                    unknown += 1;
                    let _ = writeln!(out, "#{i} ?? op {:02x}", asm.nodes.get(i).map_or(0, |r| r.opcode));
                }
                Ok(a) => {
                    ok += 1;
                    let params: Vec<String> = a
                        .params
                        .iter()
                        .filter(|p| p.value != atomic::Value::Default)
                        .map(|p| match &p.value {
                            atomic::Value::Const(_) => format!("{}={:?}", p.name, p.value.floats().unwrap_or_default()),
                            atomic::Value::Expr(e) => match e.as_input() {
                                Some(uid) => format!("{}=${}", p.name, name(uid)),
                                None => format!("{}=expr[{}]", p.name, e.instrs.len()),
                            },
                            atomic::Value::Image(k) => format!("{}=#{k}", p.name),
                            atomic::Value::Default => String::new(),
                        })
                        .collect();
                    let extra = match (&a.program, a.keys.len(), &a.function) {
                        (Some(p), _, _) => format!(" program[{} records]", p.count()),
                        (_, k, _) if k > 0 => format!(" keys{:?}", a.keys.iter().map(|k| (k.position, k.color[0])).collect::<Vec<_>>()),
                        (_, _, Some(f)) => format!(" function[{}]", f.instrs.len()),
                        _ => String::new(),
                    };
                    let _ = writeln!(
                        out,
                        "#{i} {:?}{} mode={:x?} in={:?} {}{}",
                        a.kind,
                        if a.color { " color" } else { "" },
                        a.mode,
                        a.inputs,
                        params.join(" "),
                        extra
                    );
                }
                Err(e) => {
                    failed += 1;
                    let _ = writeln!(out, "#{i} !! {e}");
                }
            }
        }
        let _ = writeln!(out, "-- decoded {ok}, unknown {unknown}, failed {failed}");
        shown += 1;
    }
    Ok(shown)
}
