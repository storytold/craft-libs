//! Charts in graphic frames: the chart part, read within the presentation's budgets, and the parts
//! it points at (workbook, styles, colours) kept to be written back.

use craft_drawingml::chart::chartex::read_chartex_xml;
use craft_drawingml::chart::read::{MAX_CHART_DECODE_BYTES, point_count, read_chart_xml};
use craft_drawingml::chart::write::sidecar_part;

pub(crate) use craft_drawingml::chart::read::MAX_PRESENTATION_CHART_POINTS;

use crate::chart::Chart;

/// The chart a graphic frame points at. A chart that would cross the presentation's point or
/// decode limits is not modelled: it is kept as its original part with the relationships it
/// reaches (`verbatim`), so a save writes it back whole instead of dropping it and its workbook.
pub(crate) fn frame_chart(imp: &mut super::Imp, part: &super::Part, rid: &str, uri: &str) -> Option<Chart> {
    let rel = part.rels.by_id(rid)?;
    let chartex = uri.ends_with("/chartex");
    let bytes = imp.pkg.xml_bytes(&rel.target)?;
    if imp.chart_decode_bytes.saturating_add(bytes.len()) > MAX_CHART_DECODE_BYTES {
        log::warn!("pptx: chart {} is past the presentation's chart decode limit and is not kept", rel.target);
        return None;
    }
    imp.chart_decode_bytes += bytes.len();
    let parsed = if imp.chart_points < imp.chart_points_cap {
        let c = if chartex { read_chartex_xml(bytes) } else { read_chart_xml(bytes) }?;
        let n = point_count(&c);
        if imp.chart_points.saturating_add(n) <= imp.chart_points_cap {
            imp.chart_points = imp.chart_points.saturating_add(n);
            let mut c = c;
            c.kept = capture_kept(imp, &rel.target);
            return Some(c);
        }
        Some(c)
    } else {
        None
    };
    // Past the limit: keep the part itself (charged to the sidecar budget) and show an empty chart.
    if chartex || imp.kept_sidecar_bytes.saturating_add(bytes.len()) > MAX_KEPT_TOTAL_BYTES {
        log::warn!("pptx: chart {} is past the presentation's chart limits and is not kept", rel.target);
        return None;
    }
    imp.kept_sidecar_bytes += bytes.len();
    log::warn!("pptx: chart {} is past the presentation's chart limits; shown empty and written back unchanged", rel.target);
    let mut c = parsed.unwrap_or_default();
    c.series.clear();
    c.categories.clear();
    c.category_levels.clear();
    c.raw = Some(String::from_utf8_lossy(bytes).into_owned());
    c.verbatim = true;
    c.kept = capture_kept(imp, &rel.target);
    Some(c)
}

const MAX_KEPT_PARTS: usize = 256;
const MAX_KEPT_DEPTH: usize = 8;
pub const MAX_KEPT_TOTAL_BYTES: usize = 32 * 1024 * 1024;

/// Relationships and parts a chart points at, so a later save can write them back.
pub(crate) fn capture_kept(imp: &mut super::Imp, chart_part: &str) -> crate::chart::ChartKept {
    use crate::chart::{ChartKept, ChartRel};

    let key = craft_opc::normalize(chart_part);
    if let Some(cached) = imp.kept_charts.get(&key) {
        // Every frame that shows this chart gets its own copy, so each copy is charged like the first.
        let bytes: usize = cached.parts.iter().map(part_bytes).sum::<usize>() + cached.rels.iter().map(rel_bytes).sum::<usize>();
        if imp.kept_sidecar_bytes.saturating_add(bytes) > MAX_KEPT_TOTAL_BYTES {
            log::warn!("pptx: chart sidecar retained byte budget exceeded for {chart_part}");
            return ChartKept::default();
        }
        imp.kept_sidecar_bytes += bytes;
        return cached.clone();
    }

    let charged_before = imp.kept_sidecar_bytes;
    if imp.rejected_kept.get(&key) == Some(&charged_before) {
        return ChartKept::default();
    }
    let mut kept = ChartKept::default();
    // The chart document itself is rewritten; don't store a second copy of it.
    let mut seen = std::collections::HashMap::from([(key.clone(), true)]);
    for r in imp.pkg.rels(chart_part).list {
        if !safe_rel(&r.id, &r.ty, &r.target_raw) {
            log::warn!("pptx: skipping chart relationship {}", r.id);
            continue;
        }
        if r.external {
            kept.rels.push(ChartRel { id: r.id, rel_type: r.ty, target: r.target_raw, part: String::new(), external: true });
            continue;
        }
        if copy_part(imp.pkg, &r.target, &mut kept.parts, &mut seen, &mut imp.kept_sidecar_bytes, 0) {
            kept.rels.push(ChartRel { id: r.id, rel_type: r.ty, target: r.target_raw, part: r.target, external: false });
        }
    }
    // The first capture's relationship strings and the parts' names and content types are retained
    // too (the part bytes were charged as copied).
    let rel_total: usize =
        kept.rels.iter().map(rel_bytes).sum::<usize>() + kept.parts.iter().map(|p| p.name.len() + p.content_type.len()).sum::<usize>();
    if imp.kept_sidecar_bytes.saturating_add(rel_total) > MAX_KEPT_TOTAL_BYTES {
        log::warn!("pptx: chart sidecar retained byte budget exceeded for {chart_part}");
        // The dropped parts must not stay charged against later charts.
        imp.kept_sidecar_bytes = charged_before;
        imp.rejected_kept.insert(key, charged_before);
        return ChartKept::default();
    }
    imp.kept_sidecar_bytes += rel_total;
    imp.kept_charts.insert(key, kept.clone());
    kept
}

fn safe_rel(id: &str, ty: &str, target: &str) -> bool {
    [id, ty, target].iter().all(|s| !s.is_empty() && s.len() <= 4096 && !s.chars().any(|c| c.is_control() || matches!(c, '<' | '>' | '"' | '\'')))
}

/// `true` when `part` is stored (now or from an earlier relationship).
fn copy_part(
    pkg: &crate::package::Package,
    part: &str,
    out: &mut Vec<crate::chart::ChartPart>,
    seen: &mut std::collections::HashMap<String, bool>,
    retained_bytes: &mut usize,
    depth: usize,
) -> bool {
    use crate::chart::ChartPart;

    if depth > MAX_KEPT_DEPTH || out.len() >= MAX_KEPT_PARTS {
        log::warn!("pptx: chart part closure stopped at {part}");
        return false;
    }
    let key = craft_opc::normalize(part);
    if let Some(ok) = seen.get(&key) {
        return *ok;
    }
    if !sidecar_part(part) {
        log::warn!("pptx: chart relationship target {part} is not kept");
        seen.insert(key, false);
        return false;
    }
    let Some(bytes) = pkg.get(part) else {
        log::warn!("pptx: chart relationship target {part} is missing");
        seen.insert(key, false);
        return false;
    };
    let name = pkg.original_name(part).unwrap_or(part).trim_start_matches('/').to_string();
    if !sidecar_part(&name) {
        log::warn!("pptx: chart part name {name} is not kept");
        seen.insert(key, false);
        return false;
    }
    let content_type = pkg.content_type(part).unwrap_or_default();
    let stored_bytes = if *retained_bytes + bytes.len() <= MAX_KEPT_TOTAL_BYTES {
        *retained_bytes += bytes.len();
        bytes.to_vec()
    } else {
        log::warn!("pptx: chart sidecar retained byte budget exceeded for {part}");
        Vec::new()
    };
    out.push(ChartPart { name, content_type, bytes: stored_bytes, native_entry: None });
    seen.insert(key, true);
    let rels_name = craft_opc::rels_path(part);
    if sidecar_part(&rels_name)
        && seen.get(&craft_opc::normalize(&rels_name)).is_none()
        && let Some(rels_bytes) = pkg.get(&rels_name)
    {
        seen.insert(craft_opc::normalize(&rels_name), true);
        let name = pkg.original_name(&rels_name).unwrap_or(&rels_name).trim_start_matches('/').to_string();
        let stored_rels = if *retained_bytes + rels_bytes.len() <= MAX_KEPT_TOTAL_BYTES {
            *retained_bytes += rels_bytes.len();
            rels_bytes.to_vec()
        } else {
            Vec::new()
        };
        out.push(ChartPart {
            name,
            content_type: "application/vnd.openxmlformats-package.relationships+xml".into(),
            bytes: stored_rels,
            native_entry: None,
        });
        let targets: Vec<String> = pkg.rels(part).list.into_iter().filter(|r| !r.external).map(|r| r.target).collect();
        for target in targets {
            copy_part(pkg, &target, out, seen, retained_bytes, depth + 1);
        }
    }
    true
}

/// Bytes of the strings a kept relationship owns.
fn part_bytes(p: &crate::chart::ChartPart) -> usize {
    p.bytes.len() + p.name.len() + p.content_type.len()
}

fn rel_bytes(r: &crate::chart::ChartRel) -> usize {
    r.id.len() + r.rel_type.len() + r.target.len() + r.part.len()
}
