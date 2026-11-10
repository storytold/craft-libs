//! [`StackSummary`]: counts over a document's layers that say what a layer import can compute
//! (uniform fills, bitmap fills, masks by kind) and what needs more (procedurals, strokes).

use super::document::ProjectDocument;
use super::effect::ProjectEffect;
use super::layer::{LayerKind, ProjectLayer};
use super::source::ProjectSource;
use std::collections::BTreeMap;

/// What a layer's content (or a mask) is made of, for counting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ContentKind {
    /// Only fills whose sources are all uniform values.
    Uniform,
    /// Fills with at least one bitmap source and otherwise uniform values.
    Bitmap,
    /// At least one procedural (a graph package) fill or filter.
    Procedural,
    /// Brush strokes only (possibly with uniform fills).
    Strokes,
    /// Several of the above, or anchors/references/levels/selection.
    Mixed,
    /// No effects.
    Empty,
}

impl ContentKind {
    pub fn name(self) -> &'static str {
        match self {
            ContentKind::Uniform => "uniform",
            ContentKind::Bitmap => "bitmap",
            ContentKind::Procedural => "procedural",
            ContentKind::Strokes => "strokes",
            ContentKind::Mixed => "mixed",
            ContentKind::Empty => "empty",
        }
    }

    /// Classifies an effect stack.
    pub fn of(effects: &[ProjectEffect]) -> ContentKind {
        let mut kinds = Vec::new();
        collect(effects, &mut kinds);
        kinds.sort();
        kinds.dedup();
        match kinds.as_slice() {
            [] => ContentKind::Empty,
            [one] => *one,
            [ContentKind::Uniform, other] => *other,
            _ => ContentKind::Mixed,
        }
    }
}

fn collect(effects: &[ProjectEffect], out: &mut Vec<ContentKind>) {
    for e in effects {
        match e {
            ProjectEffect::Fill(f) => out.push(sources_kind(&f.sources)),
            ProjectEffect::Paint(p) if p.stroke_count() > 0 => out.push(ContentKind::Strokes),
            ProjectEffect::Paint(_) => {}
            ProjectEffect::Group(g) => collect(&g.effects, out),
            ProjectEffect::Filter(_) => out.push(ContentKind::Procedural),
            _ => out.push(ContentKind::Mixed),
        }
    }
}

fn sources_kind(sources: &[ProjectSource]) -> ContentKind {
    let has = |k: &str| sources.iter().any(|s| s.kind() == k);
    if has("procedural") {
        ContentKind::Procedural
    } else if has("reference") || has("other") {
        ContentKind::Mixed
    } else if has("bitmap") {
        ContentKind::Bitmap
    } else {
        ContentKind::Uniform
    }
}

/// Counts over every layer of a document (folders included, their children too).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StackSummary {
    pub layers: usize,
    pub folders: usize,
    pub fill_layers: usize,
    pub paint_layers: usize,
    pub other_layers: usize,
    pub hidden: usize,
    /// Layers (not folders) by content kind.
    pub content: BTreeMap<ContentKind, usize>,
    /// Layers and folders with a mask, by mask content kind.
    pub masks: BTreeMap<ContentKind, usize>,
}

impl StackSummary {
    pub fn of(doc: &ProjectDocument) -> StackSummary {
        let mut s = StackSummary::default();
        for set in &doc.texture_sets {
            for layer in set.all_layers() {
                s.add(layer);
            }
        }
        s
    }

    fn add(&mut self, layer: &ProjectLayer) {
        self.layers += 1;
        self.hidden += usize::from(!layer.visible);
        match layer.kind {
            LayerKind::Group(_) => self.folders += 1,
            LayerKind::Fill => self.fill_layers += 1,
            LayerKind::Paint => self.paint_layers += 1,
            LayerKind::Other(_) => self.other_layers += 1,
        }
        if !matches!(layer.kind, LayerKind::Group(_)) {
            *self.content.entry(ContentKind::of(&layer.content)).or_default() += 1;
        }
        if let Some(mask) = &layer.mask {
            *self.masks.entry(ContentKind::of(&mask.effects)).or_default() += 1;
        }
    }
}
