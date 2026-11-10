//! One text line per effect (fills with their values, paint with stroke counts, filters by package
//! name, …); effect groups are expanded below their line. Effects are listed top → bottom.

use super::blend::{BlendContext, blending_text};
use std::fmt::Write;
use texturecraft_import_reader::model::{FillEffect, PaintEffect, ProjectEffect, ProjectSource};

/// Writes `effects` top → bottom at `indent`; `in_mask` selects which blending entries matter.
/// Consecutive identical lines (typical for brush work) are written once with a count.
pub(super) fn effect_lines(out: &mut String, effects: &[ProjectEffect], indent: usize, in_mask: bool) {
    let pad = "  ".repeat(indent);
    if effects.is_empty() {
        let _ = writeln!(out, "{pad}(empty)");
    }
    let mut pending: Option<(String, usize)> = None;
    for effect in effects.iter().rev() {
        let text = effect_text(effect, in_mask);
        let expandable = matches!(effect, ProjectEffect::Group(g) if !g.effects.is_empty());
        match &mut pending {
            Some((last, n)) if *last == text && !expandable => *n += 1,
            _ => {
                flush(out, &pad, pending.take());
                pending = Some((text, 1));
            }
        }
        if let (true, ProjectEffect::Group(g)) = (expandable, effect) {
            flush(out, &pad, pending.take());
            effect_lines(out, &g.effects, indent + 1, in_mask);
        }
    }
    flush(out, &pad, pending);
}

fn flush(out: &mut String, pad: &str, line: Option<(String, usize)>) {
    match line {
        Some((text, 1)) => {
            let _ = writeln!(out, "{pad}{text}");
        }
        Some((text, n)) => {
            let _ = writeln!(out, "{pad}{text}  ×{n}");
        }
        None => {}
    }
}

/// The effect's line: what it is, whether it is off, and its non-default blending.
fn effect_text(effect: &ProjectEffect, in_mask: bool) -> String {
    let off = if effect.is_enabled() || matches!(effect, ProjectEffect::Other(_)) { "" } else { " (off)" };
    let context = match (in_mask, effect) {
        (true, _) => BlendContext::Mask,
        (false, ProjectEffect::Group(_)) => BlendContext::Group,
        (false, _) => BlendContext::Layer,
    };
    let blend = effect.header().and_then(|h| blending_text(&h.blending, context)).map_or(String::new(), |b| format!(" [blend {b}]"));
    format!("{}{off}{blend}", line(effect))
}

fn line(effect: &ProjectEffect) -> String {
    match effect {
        ProjectEffect::Fill(f) => format!("fill {}", fill_text(f)),
        ProjectEffect::Paint(p) => paint_text(p),
        ProjectEffect::Group(g) => format!("group, {} effects", g.effects.len()),
        ProjectEffect::Filter(f) => format!("filter {}", f.procedural.as_ref().map_or("(no procedural)", |p| p.name())),
        ProjectEffect::Levels(_) => "levels".into(),
        ProjectEffect::ColorSelection(c) => format!("color selection, {} colours", c.colors.len()),
        ProjectEffect::Anchor(h) => format!("anchor {:?}", h.label),
        ProjectEffect::PolygonFill(p) => format!("polygon fill, {} picks", p.hits.len() + p.unread),
        ProjectEffect::Other(raw) => format!("raw {}", raw.class),
    }
}

fn fill_text(f: &FillEffect) -> String {
    if f.sources.is_empty() {
        return "(no sources)".into();
    }
    short_list(&sorted_texts(&f.sources), 8)
}

fn paint_text(p: &PaintEffect) -> String {
    let colors = sorted_texts(&p.colors);
    let opacity = if p.opacity == 1.0 { String::new() } else { format!(" {:.0}%", p.opacity * 100.0) };
    format!("{}{opacity}, strokes {}, paints {}", p.tool.name(), p.stroke_count(), short_list(&colors, 3))
}

/// Source texts ordered by channel bit (base colour first), whatever order the file stores them in.
fn sorted_texts(sources: &[ProjectSource]) -> Vec<String> {
    let mut sorted: Vec<&ProjectSource> = sources.iter().collect();
    sorted.sort_by_key(|s| s.channels().0.trailing_zeros());
    sorted.into_iter().map(source_text).collect()
}

/// "base colour (0.5, 0.2, 0.1)", "height 0.24", "concrete_cast → base colour+height", "bitmap <name>".
pub(super) fn source_text(s: &ProjectSource) -> String {
    match s {
        ProjectSource::Uniform(u) => {
            let [r, g, b] = u.color;
            let value = if r == g && g == b { format!("{r:.3}") } else { format!("({r:.3}, {g:.3}, {b:.3})") };
            format!("{} {value}", u.channels)
        }
        ProjectSource::Bitmap(b) => {
            let name = b.url.split('?').next().unwrap_or("").trim_start_matches('/');
            format!("{} bitmap {name:?}", b.channels)
        }
        ProjectSource::Procedural(p) => {
            let name = p.procedural.as_ref().map_or("?", |p| p.name());
            let tweaks = p.procedural.as_ref().map_or(0, |p| p.tweaks.len());
            format!("{} ← {name} ({tweaks} tweaks)", p.channels)
        }
        ProjectSource::Reference(r) => format!("{} ← anchor {:?}", r.channels, r.anchor_label),
        ProjectSource::Other(raw) => format!("raw {}", raw.class),
    }
}

/// Joins at most `max` items and says how many more there are.
pub(super) fn short_list(items: &[String], max: usize) -> String {
    let mut text = items.iter().take(max).cloned().collect::<Vec<_>>().join(", ");
    if items.len() > max {
        let _ = write!(text, ", … {} more", items.len() - max);
    }
    text
}
