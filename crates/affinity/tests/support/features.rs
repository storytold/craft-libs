//! Structural regression signatures, measured from the pinned files through our own reader.
//! These freeze feature retention; they are not independent visual or compatibility oracles.
//! The model CRC covers every decoded field via its Debug representation (including paths,
//! paint, masks, image pixels and text runs). A deliberate reader/Debug change needs review;
//! input authenticity is independently checked with SHA256 by the corpus gate.
use craft_affinity::model::{Document, Kind, Node};
use craft_affinity::paint::Paint;
use std::collections::{BTreeMap, BTreeSet};

pub fn signature(doc: &Document) -> String {
    fn visit(nodes: &[Node], counts: &mut BTreeMap<&'static str, usize>, fonts: &mut BTreeSet<String>, boards: &mut Vec<String>, depth: usize) {
        for n in nodes {
            *counts.entry("nodes").or_default() += 1;
            counts.entry("depth").and_modify(|v| *v = (*v).max(depth)).or_insert(depth);
            let key = match &n.kind {
                Kind::Layer => "layers",
                Kind::Group => "groups",
                Kind::Artboard { rect, .. } => {
                    boards.push(format!("{}:{:.4},{:.4},{:.4},{:.4}", n.name, rect.x0, rect.y0, rect.x1, rect.y1));
                    "artboards"
                }
                Kind::Shape { path, fills, strokes, .. } => {
                    *counts.entry("subpaths").or_default() += path.subpaths.len();
                    *counts.entry("segments").or_default() += path.subpaths.iter().map(|p| p.segments.len()).sum::<usize>();
                    *counts.entry("fills").or_default() += fills.len();
                    *counts.entry("strokes").or_default() += strokes.len();
                    *counts.entry("gradients").or_default() += fills.iter().filter(|p| matches!(p, Paint::Gradient(_))).count();
                    "shapes"
                }
                Kind::Text(t) => {
                    *counts.entry("text_runs").or_default() += t.runs.len();
                    *counts.entry("text_chars").or_default() += t.runs.iter().map(|r| r.text.chars().count()).sum::<usize>();
                    for r in &t.runs {
                        fonts.insert(format!("{}:{}:{}:{}", r.family, r.postscript, r.weight, r.italic));
                    }
                    "texts"
                }
                Kind::Table(t) => {
                    *counts.entry("table_cells").or_default() += t.cells.len();
                    "tables"
                }
                Kind::Image(_) => "images",
                Kind::MasterInstance { .. } => "master_instances",
                Kind::Unsupported => "unsupported",
            };
            *counts.entry(key).or_default() += 1;
            for (key, yes) in
                [("hidden", !n.visible), ("vector_masks", n.mask.is_some()), ("pixel_masks", n.pixel_mask.is_some()), ("opacity", n.opacity < 1.0)]
            {
                if yes {
                    *counts.entry(key).or_default() += 1;
                }
            }
            visit(&n.children, counts, fonts, boards, depth + 1);
        }
    }
    let (mut counts, mut fonts, mut boards) = (BTreeMap::new(), BTreeSet::new(), Vec::new());
    for spread in &doc.spreads {
        visit(&spread.nodes, &mut counts, &mut fonts, &mut boards, 1);
    }
    let model_crc32 = crc32fast::hash(format!("{:?}", doc.spreads).as_bytes());
    format!(
        "dpi={};spreads={};model_crc32={model_crc32:08x};counts={counts:?};boards={boards:?};fonts={fonts:?};warnings={:?}",
        doc.dpi,
        doc.spreads.len(),
        doc.warnings
    )
}
