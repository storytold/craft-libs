//! [`ModelText`]: a [`ProjectDocument`] as an indented tree for the `model` subcommand: texture sets, then layers
//! **top → bottom as the reference app's layer panel shows them** (the `[n]` is the stored position, 0 = bottom),
//! each with blending, mask and content summaries, then the [`ModelReport`] and a [`StackSummary`].

mod blend;
mod effects;

use blend::{BlendContext, blending_text};
use effects::{effect_lines, short_list};
use std::fmt::{self, Write};
use texturecraft_import_reader::model::{LayerKind, ModelReport, ProjectDocument, ProjectLayer, ProjectTextureSet, StackSummary};

pub struct ModelText<'a> {
    pub document: &'a ProjectDocument,
    pub report: &'a ModelReport,
}

impl fmt::Display for ModelText<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let doc = self.document;
        writeln!(f, "document format {} · {} texture sets", doc.format_version, doc.texture_sets.len())?;
        let mut out = String::new();
        for set in &doc.texture_sets {
            texture_set(&mut out, set);
        }
        f.write_str(&out)?;
        write!(f, "{}", ReportText(self.report))?;
        write!(f, "{}", SummaryText(&StackSummary::of(doc)))
    }
}

fn texture_set(out: &mut String, set: &ProjectTextureSet) {
    let size = set.resolution().map_or("?".into(), |[w, h]| format!("{w}×{h}"));
    let uid = set.material_uid.map_or("-".into(), |u| u.to_string());
    let _ = writeln!(out, "\ntexture set {:?} · material {uid} · {size}", set.name);
    let channels: Vec<String> = set.channels.iter().map(|c| c.channel.to_string()).collect();
    let _ = writeln!(out, "  channels: {}", channels.join(", "));
    let _ = writeln!(out, "  layers, top → bottom:");
    layers(out, &set.layers, 2);
}

fn layers(out: &mut String, list: &[ProjectLayer], indent: usize) {
    if list.is_empty() {
        let _ = writeln!(out, "{}(none)", "  ".repeat(indent));
    }
    for (i, layer) in list.iter().enumerate().rev() {
        self::layer(out, i, layer, indent);
    }
}

fn layer(out: &mut String, index: usize, layer: &ProjectLayer, indent: usize) {
    let pad = "  ".repeat(indent);
    let hidden = if layer.visible { "" } else { " (hidden)" };
    let _ = writeln!(out, "{pad}[{index}] {} {:?}{hidden}", layer.kind.name(), layer.name);
    let pad = "  ".repeat(indent + 2);
    if let Some(text) = blending_text(&layer.blending, BlendContext::Layer) {
        let _ = writeln!(out, "{pad}blend: {text}");
    }
    if let Some(g) = layer.geometry.as_ref().filter(|g| g.restricts()) {
        let default = if g.mesh_default { "on" } else { "off" };
        let _ = writeln!(out, "{pad}geometry mask ({:?}): meshes {default} except {}", g.kind, g.mesh_list.join(", "));
    }
    if let Some(mask) = &layer.mask {
        let state = if mask.enabled { "" } else { ", disabled" };
        let _ = writeln!(out, "{pad}mask ({}{state}):", mask.initial.name());
        effect_lines(out, &mask.effects, indent + 3, true);
    }
    if !layer.content.is_empty() {
        let _ = writeln!(out, "{pad}content:");
        effect_lines(out, &layer.content, indent + 3, false);
    }
    if let LayerKind::Group(children) = &layer.kind {
        layers(out, children, indent + 1);
    }
}

struct ReportText<'a>(&'a ModelReport);

impl fmt::Display for ReportText<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let r = self.0;
        writeln!(f, "\nmodel report: {} objects · recognised {} · raw {} · not mirrored {}", r.total(), r.recognised(), r.raw(), r.not_modelled())?;
        let fmt_list = |v: Vec<(&str, usize)>| {
            let items: Vec<String> = v.iter().map(|(c, n)| format!("{c} {n}")).collect();
            short_list(&items, 10)
        };
        if r.raw() > 0 {
            writeln!(f, "  raw classes: {}", fmt_list(r.raw_classes()))?;
        }
        if r.not_modelled() > 0 {
            writeln!(f, "  not mirrored: {}", fmt_list(r.not_modelled_classes()))?;
        }
        if r.unknown_tags + r.depth_limited + r.extra_stacks > 0 {
            writeln!(f, "  unknown tags {} · nesting cut {} · extra stacks {}", r.unknown_tags, r.depth_limited, r.extra_stacks)?;
        }
        Ok(())
    }
}

struct SummaryText<'a>(&'a StackSummary);

impl fmt::Display for SummaryText<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = self.0;
        let kinds = |m: &std::collections::BTreeMap<texturecraft_import_reader::model::ContentKind, usize>| {
            let v: Vec<String> = m.iter().map(|(k, n)| format!("{} {n}", k.name())).collect();
            v.join(", ")
        };
        writeln!(
            f,
            "layers: {} ({} folders, {} fill, {} paint, {} other; {} hidden)",
            s.layers, s.folders, s.fill_layers, s.paint_layers, s.other_layers, s.hidden
        )?;
        writeln!(f, "  content: {}", kinds(&s.content))?;
        writeln!(f, "  masks: {}", kinds(&s.masks))
    }
}
