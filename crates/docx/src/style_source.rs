//! Keep unchanged themed style and number-label formatting instead of flattening it to RGB.
//!
//! The editable model resolves colours, but that conversion loses theme tint
//! precision. Source fragments are hints, bounded and invalidated by edits.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;

use craft_wordmodel::Document;
use craft_wordmodel::props::CharProps;
use craft_wordmodel::styles::Style;

use crate::xml::{self, El};

const PREFIX: &str = "wordcraft:style-format:";
const MAX_STYLE: usize = 64 * 1024;
const MAX_TOTAL: usize = 4 * 1024 * 1024;

pub(crate) enum Fragments {
    Complete(String),
    Run(String),
}

fn signature(doc: &Document, style: &Style) -> String {
    let mut hash = DefaultHasher::new();
    // A version-dependent fingerprint is intentional: an implementation change
    // may invalidate a hint, but must never cause stale formatting to be used.
    format!("{style:?}").hash(&mut hash);
    doc.settings.theme_colors.hash(&mut hash);
    doc.settings.major_font.hash(&mut hash);
    doc.settings.minor_font.hash(&mut hash);
    doc.passthrough.get(crate::package::THEME_FONTS).hash(&mut hash);
    format!("{:016x}", hash.finish())
}

fn usable(e: &El) -> bool {
    !matches!(e.name.as_str(), "w:numPr" | "w:pPrChange" | "w:rPrChange" | "w:tblPrChange" | "w:tcPrChange")
        && !e.attrs.iter().any(|(k, _)| k.starts_with("r:"))
        && e.els().all(usable)
}

fn has_theme(e: &El) -> bool {
    e.attrs.iter().any(|(k, _)| matches!(k.as_str(), "w:themeColor" | "w:themeFill")) || e.els().any(has_theme)
}

fn property(e: &El) -> bool {
    matches!(e.name.as_str(), "w:pPr" | "w:rPr" | "w:tblPr" | "w:trPr" | "w:tcPr" | "w:tblStylePr")
}

fn canonicalize(e: &mut El, aliases: &HashMap<String, String>, mapping: Option<[usize; 12]>) {
    let logical = [
        "text1",
        "background1",
        "text2",
        "background2",
        "accent1",
        "accent2",
        "accent3",
        "accent4",
        "accent5",
        "accent6",
        "hyperlink",
        "followedHyperlink",
    ];
    let physical =
        ["dark1", "light1", "dark2", "light2", "accent1", "accent2", "accent3", "accent4", "accent5", "accent6", "hyperlink", "followedHyperlink"];
    for (key, value) in &mut e.attrs {
        if matches!(key.as_str(), "w:themeColor" | "w:themeFill") {
            if let Some(slot) = logical.iter().position(|n| *n == value.as_str()) {
                let mapped = mapping.map_or(slot, |m| m[slot]);
                if let Some(name) = physical.get(mapped) {
                    *value = (*name).into();
                }
            }
        } else if key == "w:val"
            && matches!(e.name.as_str(), "w:pStyle" | "w:rStyle")
            && let Some(id) = aliases.get(value)
        {
            *value = id.clone();
        }
    }
    for child in &mut e.kids {
        if let xml::Node::El(child) = child {
            canonicalize(child, aliases, mapping);
        }
    }
}

/// Does nothing unless the document has a theme (`themed`): without one there is no transform to keep.
pub(crate) fn capture(doc: &mut Document, root: &El, aliases: &HashMap<String, String>, mapping: Option<[usize; 12]>, themed: bool) {
    if !themed {
        return;
    }
    let styles: HashMap<_, _> = doc.styles.styles.iter().map(|s| (s.id.as_str(), s)).collect();
    let mut total = 0usize;
    for source in root.children("w:style").take(10_000) {
        let Some(id) = source.attr("w:styleId") else { continue };
        let id = aliases.get(id).map_or(id, String::as_str);
        let key = format!("{PREFIX}{id}");
        // A duplicate style uses the last definition, including its provenance.
        doc.passthrough.remove(&key);
        let Some(style) = styles.get(id) else { continue };
        let mut fragments: Vec<_> = source.els().filter(|e| property(e)).cloned().collect();
        let run_only = !fragments.iter().all(usable);
        // A list reference in pPr must pass through the normal writer. It must
        // not prevent an independent, unchanged rPr theme tint from surviving.
        if run_only {
            fragments.retain(|e| e.name == "w:rPr" && usable(e));
        }
        if !fragments.iter().any(has_theme) {
            continue;
        }
        let mut w = xml::W::new();
        let mut ns = xml::body_ns();
        if run_only {
            ns.push(("mode".into(), "run".into()));
        }
        let refs: Vec<_> = ns.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        w.open("w:style", &refs);
        for e in &mut fragments {
            canonicalize(e, aliases, mapping);
            w.raw(&e.to_xml());
        }
        w.close("w:style");
        let xml = w.into_bytes();
        if xml.len() > MAX_STYLE || total.saturating_add(xml.len()).saturating_add(17) > MAX_TOTAL {
            continue;
        }
        let mut bytes = signature(doc, style).into_bytes();
        bytes.push(b'\n');
        bytes.extend(xml);
        total = total.saturating_add(bytes.len());
        doc.passthrough.insert(key, Arc::new(bytes));
    }
}

pub(crate) fn fragments(doc: &Document, style: &Style) -> Option<Fragments> {
    let bytes = doc.passthrough.get(&format!("{PREFIX}{}", style.id))?;
    if bytes.len() > MAX_STYLE + 17 {
        return None;
    }
    let newline = bytes.iter().position(|b| *b == b'\n')?;
    if bytes.get(..newline)? != signature(doc, style).as_bytes() {
        return None;
    }
    // Native JSON may contain damaged hints; parse rather than injecting bytes.
    let root = xml::parse(bytes.get(newline + 1..)?).ok()?;
    if root.name != "w:style" || !root.els().all(|e| property(e) && usable(e)) {
        return None;
    }
    if root.attr("mode") == Some("run") {
        let rpr = root.child("w:rPr")?;
        (root.els().count() == 1).then(|| Fragments::Run(rpr.to_xml()))
    } else if root.attr("mode").is_none() {
        Some(Fragments::Complete(root.els().map(El::to_xml).collect()))
    } else {
        None
    }
}

fn run_signature(doc: &Document, chr: &CharProps) -> String {
    let mut hash = DefaultHasher::new();
    format!("{chr:?}").hash(&mut hash);
    doc.settings.theme_colors.hash(&mut hash);
    doc.settings.major_font.hash(&mut hash);
    doc.settings.minor_font.hash(&mut hash);
    doc.passthrough.get(crate::package::THEME_FONTS).hash(&mut hash);
    format!("{:016x}", hash.finish())
}

pub(crate) fn numbering_key(kind: &str, id: u32, level: usize) -> String {
    format!("wordcraft:numbering-format:{kind}:{id}:{level}")
}

/// Number labels have their own character properties, independent of the
/// paragraph style. Keep their original theme transforms under the same bounds
/// and edit guards as style formatting, including per-list level overrides.
/// Does nothing unless the document has a theme (`themed`), as [`capture`].
pub(crate) fn capture_numbering(doc: &mut Document, root: &El, aliases: &HashMap<String, String>, mapping: Option<[usize; 12]>, themed: bool) {
    if !themed {
        return;
    }
    let mut total = 0usize;
    let mut abstracts = HashMap::new();
    let mut nums = HashMap::new();
    for a in &doc.numbering.abstracts {
        abstracts.entry(a.id).or_insert(a);
    }
    for n in &doc.numbering.nums {
        nums.entry(n.id).or_insert(n);
    }
    for kind in ["abstractNum", "num"] {
        let element = format!("w:{kind}");
        let attr = format!("w:{kind}Id");
        for source in root.children(&element).take(10_000) {
            let Some(id) = source.attr(&attr).and_then(|s| s.parse::<u32>().ok()) else { continue };
            // A list that only links to a numbering style takes that style's levels (see
            // `read_numbering`), so its own XML is not where their formatting came from.
            if kind == "abstractNum" && source.child("w:numStyleLink").is_some() {
                continue;
            }
            let index = |e: &El| e.attr("w:ilvl").unwrap_or("0").trim().parse::<usize>().ok().filter(|i| *i < 9);
            let levels: Vec<_> = if kind == "abstractNum" {
                source.children("w:lvl").take(10_000).filter_map(|e| index(e).map(|i| (i, e))).collect()
            } else {
                // The first override of a level is the one read and written; later repeats are ignored.
                let mut seen = [false; 9];
                source
                    .children("w:lvlOverride")
                    .take(9)
                    .filter_map(|e| Some((index(e)?, e)))
                    .filter(|(i, _)| !std::mem::replace(&mut seen[*i], true))
                    .filter_map(|(i, e)| Some((i, e.child("w:lvl")?)))
                    .collect()
            };
            for (index, source_level) in levels {
                let key = numbering_key(kind, id, index);
                doc.passthrough.remove(&key);
                let chr = if kind == "abstractNum" {
                    abstracts.get(&id).and_then(|a| a.levels.get(index)).map(|l| &l.chr)
                } else {
                    nums.get(&id).and_then(|n| n.level_overrides.iter().find(|(i, _)| usize::from(*i) == index)).map(|(_, l)| &l.chr)
                };
                let (Some(chr), Some(rpr)) = (chr, source_level.child("w:rPr")) else { continue };
                if !has_theme(rpr) || !usable(rpr) {
                    continue;
                }
                let mut rpr = rpr.clone();
                canonicalize(&mut rpr, aliases, mapping);
                let mut w = xml::W::new();
                let ns = xml::body_ns();
                let refs: Vec<_> = ns.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
                w.open("w:source", &refs);
                w.raw(&rpr.to_xml());
                w.close("w:source");
                let xml = w.into_bytes();
                if xml.len() > MAX_STYLE || total.saturating_add(xml.len()).saturating_add(17) > MAX_TOTAL {
                    continue;
                }
                let mut bytes = run_signature(doc, chr).into_bytes();
                bytes.push(b'\n');
                bytes.extend(xml);
                total = total.saturating_add(bytes.len());
                doc.passthrough.insert(key, Arc::new(bytes));
            }
        }
    }
}

pub(crate) fn numbering_rpr(doc: &Document, key: &str, chr: &CharProps) -> Option<String> {
    let bytes = doc.passthrough.get(key)?;
    if bytes.len() > MAX_STYLE + 17 {
        return None;
    }
    let newline = bytes.iter().position(|b| *b == b'\n')?;
    if bytes.get(..newline)? != run_signature(doc, chr).as_bytes() {
        return None;
    }
    let root = xml::parse(bytes.get(newline + 1..)?).ok()?;
    let rpr = root.child("w:rPr")?;
    if root.name != "w:source" || root.els().count() != 1 || !usable(rpr) {
        return None;
    }
    Some(rpr.to_xml())
}
