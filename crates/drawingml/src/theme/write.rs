//! Theme parts.

use crate::color::{ColorScheme, Rgba, SchemeSlot};
use crate::style::{ColorRef, Effects, Fill};
use crate::theme::{FontSet, Theme, default_format};

use craft_xml::Writer;

use crate::ns::{NS_A, NS_R};
use crate::part::Sink;
use crate::write as dml;

fn font_set(w: &mut Writer, tag: &str, f: &FontSet) {
    w.open(tag, &[]);
    w.tag("a:latin").attr("typeface", if f.latin.is_empty() { "Arial" } else { &f.latin }).empty();
    w.tag("a:ea").attr("typeface", &f.ea).empty();
    w.tag("a:cs").attr("typeface", &f.cs).empty();
    for sf in &f.scripts {
        w.tag("a:font").attr("script", &sf.script).attr("typeface", &sf.typeface).empty();
    }
    w.close();
}

fn style_fill(f: &Fill) -> Fill {
    match f {
        Fill::Background | Fill::Group => Fill::solid(ColorRef::scheme(SchemeSlot::PhClr)),
        f => f.clone(),
    }
}

/// The scheme's extra named colours, explicit tint rows and `computed_tints`, as an `a:ext`
/// ([`super::EXT_EXTRA_COLORS`]) after the twelve slots: `<dc:extraColors computed="0"?>` holding
/// `<dc:tints slot="accent1"><dc:clr val="RRGGBB"/>…</dc:tints>` per slot with a list (an empty
/// list is an empty element) and `<dc:color name="Sand" val="RRGGBB">(<dc:tints>…</dc:tints>
/// when it has a list)</dc:color>` per extra colour. Nothing when there is nothing to keep.
fn extra_colors(w: &mut Writer, c: &ColorScheme) {
    if c.extra.is_empty() && c.tints.is_empty() && c.computed_tints {
        return;
    }
    w.open("a:extLst", &[]);
    w.tag("a:ext").attr("uri", super::EXT_EXTRA_COLORS).open();
    w.tag("dc:extraColors").attr("xmlns:dc", super::NS_EXTRA_COLORS).attr_opt("computed", (!c.computed_tints).then_some("0")).open();
    for (slot, tints) in &c.tints {
        w.tag("dc:tints").attr("slot", slot.xml_name()).open();
        rows(w, tints);
    }
    // What the reader takes back, no more.
    for e in c.extra.iter().take(crate::color::MAX_EXTRA_COLORS) {
        w.tag("dc:color").attr("name", &e.name).attr("val", e.color.hex()).open();
        if let Some(t) = &e.tints {
            w.open("dc:tints", &[]);
            rows(w, t);
        }
        w.close();
    }
    w.close();
    w.close();
    w.close();
}

/// The rows of an open `dc:tints`, cut to `MAX_TINT_ROWS` as the reader takes them; closes it.
fn rows(w: &mut Writer, tints: &[Rgba]) {
    for t in tints.iter().take(crate::color::MAX_TINT_ROWS) {
        w.tag("dc:clr").attr("val", t.hex()).empty();
    }
    w.close();
}

pub fn theme_xml(out: &mut dyn Sink, t: &Theme) -> Vec<u8> {
    let mut w = Writer::document();
    w.tag("a:theme").attr("xmlns:a", NS_A).attr("xmlns:r", NS_R).attr("name", if t.name.is_empty() { "Theme" } else { &t.name }).open();
    w.open("a:themeElements", &[]);
    w.tag("a:clrScheme").attr("name", if t.colors.name.is_empty() { &t.name } else { &t.colors.name }).open();
    for slot in SchemeSlot::THEME {
        let tag = format!("a:{}", slot.xml_name());
        w.open(&tag, &[]);
        w.tag("a:srgbClr").attr("val", t.colors.get(slot).hex()).empty();
        w.close();
    }
    extra_colors(&mut w, &t.colors);
    w.close();
    w.tag("a:fontScheme").attr("name", if t.fonts.name.is_empty() { &t.name } else { &t.fonts.name }).open();
    font_set(&mut w, "a:majorFont", &t.fonts.major);
    font_set(&mut w, "a:minorFont", &t.fonts.minor);
    w.close();
    let d = default_format();
    let f = &t.format;
    w.tag("a:fmtScheme").attr("name", if f.name.is_empty() { &t.name } else { &f.name }).open();
    let three = |v: &[Fill], d: &[Fill]| -> Vec<Fill> {
        let mut out: Vec<Fill> = v.iter().map(style_fill).collect();
        while out.len() < 3 {
            out.push(d.get(out.len()).cloned().unwrap_or(Fill::solid(ColorRef::scheme(SchemeSlot::PhClr))));
        }
        out
    };
    w.open("a:fillStyleLst", &[]);
    for fl in three(&f.fills, &d.fills) {
        dml::fill(&mut w, out, &fl);
    }
    w.close();
    w.open("a:lnStyleLst", &[]);
    let mut lines = f.lines.clone();
    while lines.len() < 3 {
        lines.push(d.lines.get(lines.len()).cloned().unwrap_or_default());
    }
    for l in &lines {
        let mut l = l.clone();
        if l.fill.is_none() {
            l.fill = Some(Fill::solid(ColorRef::scheme(SchemeSlot::PhClr)));
        }
        if let Some(fl) = &l.fill {
            l.fill = Some(style_fill(fl));
        }
        dml::line(&mut w, out, "a:ln", &l);
    }
    w.close();
    w.open("a:effectStyleLst", &[]);
    let mut fx = f.effects.clone();
    while fx.len() < 3 {
        fx.push(Effects::default());
    }
    for e in &fx {
        w.open("a:effectStyle", &[]);
        dml::effects(&mut w, e);
        if let Some(raw) = &e.raw3d {
            w.raw(raw);
        } else if let Some(xml) = e.bevel.as_deref().and_then(dml::bevel_sp3d) {
            w.raw(&xml);
        }
        w.close();
    }
    w.close();
    w.open("a:bgFillStyleLst", &[]);
    for fl in three(&f.bg_fills, &d.bg_fills) {
        dml::fill(&mut w, out, &fl);
    }
    w.close();
    w.close();
    w.close();
    match &t.raw_extra {
        Some(raw) if raw.contains("objectDefaults") => w.raw(raw),
        Some(raw) => {
            w.empty("a:objectDefaults", &[]);
            w.raw(raw);
        }
        None => {
            w.empty("a:objectDefaults", &[]);
            w.empty("a:extraClrSchemeLst", &[]);
        }
    }
    w.close();
    w.into_bytes()
}
