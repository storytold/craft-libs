//! Theme parts (`a:theme`).

use crate::color::{ColorScheme, Rgba, SchemeSlot};
use crate::style::{ColorBase, Effects};
use crate::theme::{FontScheme, FontSet, FormatScheme, ScriptFont, Theme, default_format};

use craft_xml::{El, Local};

use crate::part::Source;
use crate::read as dml;

fn font_set(e: Option<&El>) -> FontSet {
    let tf = |n: &str| e.and_then(|e| e.child(Local(n))).and_then(|c| c.attr_local("typeface")).unwrap_or("").to_string();
    let scripts = e
        .map(|e| {
            e.children(Local("font"))
                .filter_map(|f| {
                    Some(ScriptFont { script: f.attr_local("script")?.to_string(), typeface: f.attr_local("typeface").unwrap_or("").to_string() })
                })
                .take(256)
                .collect()
        })
        .unwrap_or_default();
    FontSet { latin: tf("latin"), ea: tf("ea"), cs: tf("cs"), scripts }
}

pub fn read_theme(src: &mut dyn Source, root: &El) -> Theme {
    let mut t = Theme { name: root.attr_local("name").unwrap_or("Theme").to_string(), ..Default::default() };
    let Some(te) = root.child(Local("themeElements")) else { return t };
    if let Some(cs) = te.child(Local("clrScheme")) {
        let mut scheme = ColorScheme::new(cs.attr_local("name").unwrap_or(""), t.colors.colors);
        for slot in SchemeSlot::THEME {
            if let Some(c) = cs.child(Local(slot.xml_name())).and_then(dml::color) {
                let rgb = match c.base {
                    ColorBase::Rgb { rgb } => rgb,
                    ColorBase::System { last, .. } => last,
                    ColorBase::Preset { name } => crate::color::preset(&name).unwrap_or(Rgba::BLACK),
                    ColorBase::Scheme { .. } => continue,
                };
                scheme.set(slot, rgb);
            }
        }
        extra_colors(cs, &mut scheme);
        t.colors = scheme;
    }
    if let Some(fs) = te.child(Local("fontScheme")) {
        t.fonts = FontScheme {
            name: fs.attr_local("name").unwrap_or("").to_string(),
            major: font_set(fs.child(Local("majorFont"))),
            minor: font_set(fs.child(Local("minorFont"))),
        };
    }
    if let Some(fm) = te.child(Local("fmtScheme")) {
        let dflt = default_format();
        let mut f = FormatScheme { name: fm.attr_local("name").unwrap_or("").to_string(), ..Default::default() };
        if let Some(l) = fm.child(Local("fillStyleLst")) {
            f.fills = l.elements().take(16).filter_map(|e| dml::fill_el(src, e)).collect();
        }
        if let Some(l) = fm.child(Local("lnStyleLst")) {
            f.lines = l.children(Local("ln")).take(16).map(|e| dml::line(src, e)).collect();
        }
        if let Some(l) = fm.child(Local("effectStyleLst")) {
            f.effects = l
                .children(Local("effectStyle"))
                .take(16)
                .map(|e| {
                    let mut fx = e.child(Local("effectLst")).map(dml::effects).unwrap_or_default();
                    if let Some(b) = e.path(&[Local("sp3d"), Local("bevelT")]) {
                        fx.bevel = Some(b.attr_local("prst").unwrap_or("circle").to_string());
                    }
                    let raw: String = e.elements().filter(|c| c.is(Local("scene3d")) || c.is(Local("sp3d"))).map(|c| src.keep(c)).collect();
                    if !raw.is_empty() {
                        fx.raw3d = Some(raw);
                    }
                    fx
                })
                .collect();
        }
        if let Some(l) = fm.child(Local("bgFillStyleLst")) {
            f.bg_fills = l.elements().take(16).filter_map(|e| dml::fill_el(src, e)).collect();
        }
        // The style matrix needs three entries of each kind.
        pad(&mut f.fills, &dflt.fills);
        pad(&mut f.lines, &dflt.lines);
        pad(&mut f.bg_fills, &dflt.bg_fills);
        while f.effects.len() < 3 {
            f.effects.push(Effects::default());
        }
        t.format = f;
    }
    let extra: String = root
        .elements()
        .filter(|e| e.is(Local("objectDefaults")) || e.is(Local("extraClrSchemeLst")))
        .filter(|e| !crate::xmlx::has_rel_refs(e))
        .map(|e| src.keep(e))
        .collect();
    if !extra.is_empty() {
        t.raw_extra = Some(extra);
    }
    t
}

/// The extra named colours and tint rows (see `write::extra_colors`); a scheme saved by another
/// application has none.
fn extra_colors(cs: &El, scheme: &mut ColorScheme) {
    let Some(x) = cs
        .child(Local("extLst"))
        .and_then(|l| l.children(Local("ext")).find(|e| e.attr_local("uri") == Some(super::EXT_EXTRA_COLORS)))
        .and_then(|e| e.child(Local("extraColors")))
    else {
        return;
    };
    let clrs = |e: &El| -> Vec<Rgba> {
        e.children(Local("clr")).filter_map(|c| c.attr_local("val").and_then(Rgba::from_hex)).take(crate::color::MAX_TINT_ROWS).collect()
    };
    scheme.computed_tints = x.attr_local("computed") != Some("0");
    for t in x.children(Local("tints")).take(SchemeSlot::THEME.len()) {
        if let Some(slot) = t.attr_local("slot").and_then(SchemeSlot::from_xml)
            && SchemeSlot::THEME.contains(&slot)
        {
            scheme.tints.insert(slot, clrs(t));
        }
    }
    for c in x.children(Local("color")).take(crate::color::MAX_EXTRA_COLORS) {
        let (Some(name), Some(color)) = (c.attr_local("name"), c.attr_local("val").and_then(Rgba::from_hex)) else { continue };
        if name.trim().is_empty() {
            continue;
        }
        scheme.extra.push(crate::color::ExtraColor { name: name.to_string(), color, tints: c.child(Local("tints")).map(clrs) });
    }
}

fn pad<T: Clone>(v: &mut Vec<T>, d: &[T]) {
    while v.len() < 3 {
        match d.get(v.len()) {
            Some(x) => v.push(x.clone()),
            None => break,
        }
    }
}
