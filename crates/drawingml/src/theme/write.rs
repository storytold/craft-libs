//! Theme parts.

use crate::color::SchemeSlot;
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
