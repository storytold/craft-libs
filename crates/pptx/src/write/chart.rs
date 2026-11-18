//! Charts: the parts an unchanged chart carried (workbook, styles, colours), written back under
//! fresh names.

use craft_drawingml::chart::write::sidecar_part;
pub(crate) use craft_drawingml::chart::write::{chart_xml, chartex_part};

use crate::chart::Chart;

/// A fresh name for a kept part: `ppt/<folder>/keptN.<ext>`. The writer generates no `kept…`
/// names and `n` is unique within the package, so a kept part never lands on (or duplicates) a
/// part the writer makes, whatever the source file called it.
fn kept_name(original: &str, n: u32) -> String {
    let norm = craft_opc::normalize(original);
    let area = norm.split('/').nth(1).unwrap_or("embeddings");
    let area = if matches!(area, "embeddings" | "charts" | "drawings" | "media") { area } else { "embeddings" };
    let file = craft_opc::file_of(&norm);
    let ext = file
        .rsplit_once('.')
        .map(|(_, e)| e)
        .filter(|e| !e.is_empty() && e.len() <= 8 && e.chars().all(|c| c.is_ascii_alphanumeric()))
        .unwrap_or("bin");
    format!("ppt/{area}/kept{n}.{ext}")
}

/// Write an unchanged chart's own relationships and the parts they reach. Each kept part is
/// written under a fresh name ([`kept_name`]) and the relationships between kept parts are
/// rewritten to those names; a relationship to anything that was not kept is dropped.
/// `n_kept` numbers kept parts across the package.
pub fn write_kept(pkg: &mut craft_opc::PackageOut, chart_name: &str, c: &Chart, n_kept: &mut u32) {
    use std::collections::HashMap;

    use craft_opc::{RelOut, RelsOut, normalize as norm, relative, rels_path};

    if c.kept.rels.is_empty() {
        return;
    }
    // Original part name (normalized) → the name written.
    let mut names: HashMap<String, String> = HashMap::new();
    for p in c.kept.parts.iter().filter(|p| kept_payload(&p.name)) {
        let key = norm(&p.name);
        if names.contains_key(&key) {
            continue;
        }
        *n_kept += 1;
        let new = kept_name(&p.name, *n_kept);
        let ct = if !p.content_type.is_empty() { p.content_type.clone() } else { crate::mime_for(&p.name).to_string() };
        // An override per part: a kept part never sets the package-wide default for its extension.
        pkg.xml_part(&new, &ct, p.bytes.clone());
        names.insert(key, new);
    }
    let mut rels = RelsOut::default();
    for r in &c.kept.rels {
        if r.external {
            rels.items.push(RelOut { id: r.id.clone(), ty: r.rel_type.clone(), target: r.target.clone(), external: true });
        } else if let Some(new) = names.get(&norm(&r.part)) {
            rels.items.push(RelOut { id: r.id.clone(), ty: r.rel_type.clone(), target: relative(chart_name, new), external: false });
        } else {
            log::warn!("pptx: chart relationship {} has no kept part; dropped", r.id);
        }
    }
    pkg.rels(chart_name, &rels);
    let mut parts_by_norm = std::collections::HashMap::new();
    for p in &c.kept.parts {
        parts_by_norm.entry(norm(&p.name)).or_insert(p);
    }
    let mut seen_payloads = std::collections::HashSet::new();
    // The kept parts' own relationships, pointed at the new names.
    for p in c.kept.parts.iter().filter(|p| kept_payload(&p.name)) {
        let norm_name = norm(&p.name);
        if !seen_payloads.insert(norm_name.clone()) {
            continue;
        }
        let Some(new_source) = names.get(&norm_name) else { continue };
        let rp = norm(&rels_path(&p.name));
        let Some(rels_part) = parts_by_norm.get(&rp) else { continue };
        let mut out = RelsOut::default();
        for r in craft_opc::Rels::parse(&rels_part.bytes, &p.name).list {
            if r.external {
                out.items.push(RelOut { id: r.id, ty: r.ty, target: r.target_raw, external: true });
            } else if let Some(new) = names.get(&norm(&r.target)) {
                out.items.push(RelOut { id: r.id, ty: r.ty, target: relative(new_source, new), external: false });
            } else {
                log::warn!("pptx: relationship {} of kept chart part {} has no kept target; dropped", r.id, p.name);
            }
        }
        pkg.rels(new_source, &out);
    }
}

/// A kept part to write itself (relationships parts are rewritten from the relationships).
fn kept_payload(name: &str) -> bool {
    !name.to_ascii_lowercase().ends_with(".rels") && sidecar_part(name)
}
