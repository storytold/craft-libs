//! Shape trees: autoshapes, pictures, groups, connectors, tables, charts, media, ink, 3-D models and kept objects.

use std::collections::{HashMap, HashSet};

use crate::style::Fill;
use crate::table::Table;
use crate::{Geom, PhType, Shape, ShapeId, ShapeKind, ShapeStyle, walk};
use craft_drawingml::Xfrm;

use craft_drawingml::write::{self as dml, emu, emu_pos};
use craft_drawingml::xmlx::{self, A, TagExt};
use craft_xml::{Local, Writer};

use super::{Exp, NS_MC, Out, Snk};

/// cNvPr ids for one part.
pub struct IdMap {
    pub map: HashMap<ShapeId, u32>,
    next: u32,
}

impl IdMap {
    pub fn build(shapes: &[Shape]) -> Self {
        let mut map = HashMap::new();
        let mut used = HashSet::new();
        let mut max = 1u32;
        walk(shapes, &mut |s, _| {
            if s.id.0 >= 2 && s.id.0 < i32::MAX as u32 {
                max = max.max(s.id.0);
            }
        });
        let mut next = max;
        walk(shapes, &mut |s, _| {
            let id = if s.id.0 >= 2 && s.id.0 < i32::MAX as u32 && !used.contains(&s.id.0) {
                s.id.0
            } else {
                next = next.saturating_add(1);
                next
            };
            used.insert(id);
            map.entry(s.id).or_insert(id);
        });
        IdMap { map, next }
    }
    pub fn get(&self, id: ShapeId) -> Option<u32> {
        self.map.get(&id).copied()
    }
    pub fn fresh(&mut self) -> u32 {
        self.next = self.next.saturating_add(1);
        self.next
    }
}

pub struct Ctx<'a> {
    pub ids: &'a mut IdMap,
    /// The shape id written for the shape currently being written (duplicate model ids get fresh ones).
    pub written: HashSet<u32>,
}

impl Ctx<'_> {
    fn id_for(&mut self, s: &Shape) -> u32 {
        let id = self.ids.get(s.id).unwrap_or(0);
        if id >= 2 && self.written.insert(id) {
            return id;
        }
        let f = self.ids.fresh();
        self.written.insert(f);
        f
    }
}

fn preset_name(name: &str) -> &str {
    if name == "textBox" || name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric()) { "rect" } else { name }
}

/// Non-adjust shape guides some presets carry in their `avLst` (ECMA-376 default values).
fn extra_guides(name: &str) -> &'static [(&'static str, i64)] {
    match name {
        "star5" | "pentagon" => &[("hf", 105_146), ("vf", 110_557)],
        "star7" | "heptagon" => &[("hf", 102_572), ("vf", 105_210)],
        "star6" => &[("hf", 115_470)],
        "star10" => &[("hf", 105_146)],
        "hexagon" => &[("vf", 115_470)],
        "decagon" => &[("vf", 105_146)],
        _ => &[],
    }
}

pub fn geometry(w: &mut Writer, g: &Geom) {
    match g {
        Geom::Preset { name, adj } => {
            let name = preset_name(name);
            w.tag("a:prstGeom").attr("prst", name).open();
            if adj.is_empty() {
                w.empty("a:avLst", &[]);
            } else {
                // PowerPoint wants the complete guide list once any is given.
                let defaults = craft_drawingml::preset::info(name).map(|i| i.defaults).unwrap_or(&[]);
                let mut vals: Vec<f64> = adj.iter().take(16).copied().collect();
                while vals.len() < defaults.len() {
                    vals.push(defaults.get(vals.len()).copied().unwrap_or(0.0));
                }
                w.open("a:avLst", &[]);
                // A lone guide is `adj`, except in these presets, which name it `adj1` (as PowerPoint
                // writes them); PowerPoint won't open a file that renames it.
                let adj1 = matches!(name, "bentConnector3" | "curvedConnector3" | "frame" | "mathPlus" | "mathMinus" | "mathMultiply");
                let single = !adj1 && if defaults.is_empty() { vals.len() == 1 } else { defaults.len() == 1 };
                for (i, v) in vals.iter().enumerate() {
                    let n = if single && i == 0 { "adj".to_string() } else { format!("adj{}", i + 1) };
                    let v = if v.is_finite() { v.round().clamp(-2.0e9, 2.0e9) as i64 } else { 0 };
                    w.tag("a:gd").attr("name", n).attr("fmla", format!("val {v}")).empty();
                }
                for (n, v) in extra_guides(name) {
                    w.tag("a:gd").attr("name", *n).attr("fmla", format!("val {v}")).empty();
                }
                w.close();
            }
            w.close();
        }
        Geom::Custom { paths } => {
            if paths.is_empty() {
                w.tag("a:prstGeom").attr("prst", "rect").open();
                w.empty("a:avLst", &[]);
                w.close();
            } else {
                craft_drawingml::custgeom::write_cust_geom(w, paths);
            }
        }
    }
}

fn cnvpr(w: &mut Writer, x: &mut Exp, o: &mut Out, id: u32, s: &Shape) {
    let a = A::new()
        .a("id", id)
        .a("name", if s.name.is_empty() { format!("Shape {id}") } else { s.name.clone() })
        .o("descr", (!s.descr.is_empty()).then_some(s.descr.as_str()))
        .t("hidden", s.hidden)
        .o("title", (!s.title.is_empty()).then_some(s.title.as_str()));
    if s.click.is_none() && s.hover.is_none() && !s.decorative {
        w.tag("p:cNvPr").list(a).empty();
        return;
    }
    w.tag("p:cNvPr").list(a).open();
    if let Some(h) = &s.click {
        dml::hyperlink(w, &mut Snk(x, o), "a:hlinkClick", h);
    }
    if let Some(h) = &s.hover {
        dml::hyperlink(w, &mut Snk(x, o), "a:hlinkHover", h);
    }
    if s.decorative {
        w.open("a:extLst", &[]);
        w.tag("a:ext").attr("uri", "{C183D7F6-B498-43B3-948B-1728B52AA6E4}").open();
        w.tag("adec:decorative").attr("xmlns:adec", "http://schemas.microsoft.com/office/drawing/2017/decorative").attr("val", "1").empty();
        w.close();
        w.close();
    }
    w.close();
}

fn nv_pr(w: &mut Writer, x: &mut Exp, o: &mut Out, s: &Shape) {
    if s.ph.is_none() && s.tags.is_empty() {
        w.empty("p:nvPr", &[]);
        return;
    }
    w.open("p:nvPr", &[]);
    if let Some(ph) = &s.ph {
        let ty = match ph.kind {
            PhType::Obj => None,
            k => Some(k.xml()),
        };
        let sz = ph.size.as_deref().filter(|z| matches!(*z, "full" | "half" | "quarter"));
        w.tag("p:ph")
            .attr_opt("type", ty)
            .attr_opt("orient", ph.vertical.then_some("vert"))
            .attr_opt("sz", sz)
            .attr_opt("idx", (ph.idx != 0).then_some(ph.idx))
            .attr_opt("hasCustomPrompt", (ph.has_custom_prompt).then_some(1))
            .empty();
    }
    super::cust_data(w, x, o, &s.tags);
    w.close();
}

/// Locking attributes. Lock writes the full set PowerPoint writes for the element's kind (and
/// reads back as locked): see `read::shapes::is_locked`.
/// `aspect`: also `noChangeAspect` (pictures).
fn locks(w: &mut Writer, tag: &str, s: &Shape, aspect: bool) {
    if !aspect && s.ph.is_none() && !s.locked {
        return;
    }
    let a = A::new().t("noChangeAspect", aspect).t("noGrp", s.ph.is_some() || s.locked);
    let a = match tag {
        "a:grpSpLocks" => a.t("noUngrp", s.locked).t("noRot", s.locked).t("noMove", s.locked).t("noResize", s.locked),
        _ => {
            let a = a
                .t("noRot", s.locked)
                .t("noMove", s.locked)
                .t("noResize", s.locked)
                .t("noEditPoints", s.locked)
                .t("noAdjustHandles", s.locked)
                .t("noChangeArrowheads", s.locked)
                .t("noChangeShapeType", s.locked);
            if tag == "a:picLocks" { a.t("noCrop", s.locked) } else { a }
        }
    };
    w.tag(tag).list(a).empty();
}

fn style(w: &mut Writer, st: &ShapeStyle) {
    style_tag(w, st, "p:style");
}

fn style_tag(w: &mut Writer, st: &ShapeStyle, tag: &str) {
    w.open(tag, &[]);
    for (tag, (idx, c)) in [("a:lnRef", &st.line_ref), ("a:fillRef", &st.fill_ref), ("a:effectRef", &st.effect_ref)] {
        w.tag(tag).attr("idx", idx).open();
        dml::color(w, c);
        w.close();
    }
    let idx = match st.font_ref.0.as_str() {
        "major" | "minor" | "none" => st.font_ref.0.as_str(),
        _ => "minor",
    };
    match &st.font_ref.1 {
        Some(c) => {
            w.tag("a:fontRef").attr("idx", idx).open();
            dml::color(w, c);
            w.close();
        }
        None => w.tag("a:fontRef").attr("idx", idx).empty(),
    }
    w.close();
}

/// `p:spPr` contents (also used for pictures and connectors).
fn sp_pr(w: &mut Writer, x: &mut Exp, o: &mut Out, s: &Shape, geom: bool) {
    sp_pr_tag(w, x, o, s, geom, "p:spPr");
}

fn sp_pr_tag(w: &mut Writer, x: &mut Exp, o: &mut Out, s: &Shape, geom: bool, tag: &str) {
    w.open(tag, &[]);
    if let Some(xf) = &s.xfrm {
        dml::xfrm(w, "a:xfrm", xf, None);
    }
    if geom && (s.xfrm.is_some() || s.ph.is_none() || !matches!(&s.geom, Geom::Preset { name, adj } if name == "rect" && adj.is_empty())) {
        geometry(w, &s.geom);
    }
    if let Some(f) = &s.fill {
        dml::fill(w, &mut Snk(x, o), f);
    }
    if let Some(l) = &s.line {
        dml::line(w, &mut Snk(x, o), "a:ln", l);
    }
    if let Some(fx) = &s.effects {
        dml::effects(w, fx);
        if let Some(raw) = &fx.raw3d {
            w.raw(raw);
        } else if let Some(xml) = fx.bevel.as_deref().and_then(dml::bevel_sp3d) {
            w.raw(&xml);
        }
    }
    w.close();
}

/// Write the children of a shape tree.
pub fn shapes(w: &mut Writer, x: &mut Exp, o: &mut Out, cx: &mut Ctx, list: &[Shape], depth: usize) {
    if depth > 32 {
        return;
    }
    for s in list {
        shape(w, x, o, cx, s, depth);
    }
}

pub fn shape(w: &mut Writer, x: &mut Exp, o: &mut Out, cx: &mut Ctx, s: &Shape, depth: usize) {
    match &s.kind {
        ShapeKind::Shape => {
            if s.text.as_ref().is_some_and(dml::has_math) {
                // Equations: OMML for readers that know it, plain text otherwise.
                w.tag("mc:AlternateContent").attr("xmlns:mc", NS_MC).open();
                w.tag("mc:Choice").attr("xmlns:a14", craft_drawingml::ns::NS_A14).attr("Requires", "a14").open();
                let id = cx.id_for(s);
                sp(w, x, o, id, s, true);
                w.close();
                w.open("mc:Fallback", &[]);
                sp(w, x, o, id, s, false);
                w.close();
                w.close();
            } else {
                let id = cx.id_for(s);
                sp(w, x, o, id, s, false);
            }
        }
        ShapeKind::Picture { fill } => {
            let id = cx.id_for(s);
            w.open("p:pic", &[]);
            w.open("p:nvPicPr", &[]);
            cnvpr(w, x, o, id, s);
            w.open("p:cNvPicPr", &[]);
            locks(w, "a:picLocks", s, true);
            w.close();
            nv_pr(w, x, o, s);
            w.close();
            dml::blip_fill(w, &mut Snk(x, o), "p:blipFill", fill);
            let mut s2 = s.clone();
            if matches!(s2.fill, Some(Fill::Picture(_) | Fill::Background | Fill::Group)) {
                s2.fill = None;
            }
            sp_pr(w, x, o, &s2, true);
            if let Some(st) = &s.style {
                style(w, st);
            }
            w.close();
        }
        ShapeKind::Group { children, child } if s.diagram.is_some() && depth < 32 => {
            if let Some(d) = &s.diagram {
                diagram(w, x, o, cx, s, d, children, child);
            }
        }
        ShapeKind::Group { children, child } => {
            let id = cx.id_for(s);
            w.open("p:grpSp", &[]);
            w.open("p:nvGrpSpPr", &[]);
            cnvpr(w, x, o, id, s);
            w.open("p:cNvGrpSpPr", &[]);
            locks(w, "a:grpSpLocks", s, false);
            w.close();
            nv_pr(w, x, o, s);
            w.close();
            w.open("p:grpSpPr", &[]);
            let xf = s.xfrm.unwrap_or(*child);
            dml::xfrm(w, "a:xfrm", &xf, Some(child));
            if let Some(f) = &s.fill {
                dml::fill(w, &mut Snk(x, o), f);
            }
            if let Some(fx) = &s.effects {
                if fx.fill_overlay.is_some()
                    || fx.glow.is_some()
                    || fx.inner_shadow.is_some()
                    || fx.outer_shadow.is_some()
                    || fx.reflection.is_some()
                    || fx.soft_edge.is_some()
                {
                    dml::effects(w, fx);
                }
                if let Some(raw) =
                    fx.raw3d.as_deref().map(|s| s.find("<a:sp3d").map_or(s, |i| &s[..i])).and_then(|s| dml::valid_raw3d(s, &["scene3d"]))
                {
                    w.raw(raw);
                }
            }
            w.close();
            shapes(w, x, o, cx, children, depth + 1);
            w.close();
        }
        ShapeKind::Connector { start, end } => {
            let id = cx.id_for(s);
            w.open("p:cxnSp", &[]);
            w.open("p:nvCxnSpPr", &[]);
            cnvpr(w, x, o, id, s);
            let st = start.and_then(|(sid, site)| cx.ids.get(sid).map(|i| (i, site)));
            let en = end.and_then(|(sid, site)| cx.ids.get(sid).map(|i| (i, site)));
            if st.is_none() && en.is_none() && !s.locked {
                w.empty("p:cNvCxnSpPr", &[]);
            } else {
                w.open("p:cNvCxnSpPr", &[]);
                locks(w, "a:cxnSpLocks", s, false);
                if let Some((i, site)) = st {
                    w.tag("a:stCxn").attr("id", i).attr("idx", site).empty();
                }
                if let Some((i, site)) = en {
                    w.tag("a:endCxn").attr("id", i).attr("idx", site).empty();
                }
                w.close();
            }
            nv_pr(w, x, o, s);
            w.close();
            let mut s2 = s.clone();
            if matches!(s2.fill, Some(Fill::Background)) {
                s2.fill = None;
            }
            if s2.geom.preset_name() == Some("rect") {
                s2.geom = Geom::preset("line");
            }
            sp_pr(w, x, o, &s2, true);
            if let Some(st) = &s.style {
                style(w, st);
            }
            w.close();
        }
        ShapeKind::Table(t) => {
            let id = cx.id_for(s);
            frame_open(w, x, o, id, s);
            w.tag("a:graphicData").attr("uri", "http://schemas.openxmlformats.org/drawingml/2006/table").open();
            table(w, x, o, t);
            w.close();
            frame_close(w);
        }
        ShapeKind::Chart(c) => {
            let id = cx.id_for(s);
            // An unchanged Office 2016 chart goes back as itself, with a plain chart of the same
            // data for readers without `cx1`, as PowerPoint writes it.
            let ex = x.chartex_rel(o, c);
            if let Some(exid) = &ex {
                w.tag("mc:AlternateContent").attr("xmlns:mc", "http://schemas.openxmlformats.org/markup-compatibility/2006").open();
                w.tag("mc:Choice").attr("xmlns:cx1", "http://schemas.microsoft.com/office/drawing/2015/9/8/chartex").attr("Requires", "cx1").open();
                frame_open(w, x, o, id, s);
                w.tag("a:graphicData").attr("uri", craft_drawingml::chart::write::NS_CX).open();
                w.tag("cx:chart").attr("xmlns:cx", craft_drawingml::chart::write::NS_CX).attr("r:id", exid).empty();
                w.close();
                frame_close(w);
                w.close();
                w.open("mc:Fallback", &[]);
            }
            let mut plain = (**c).clone();
            plain.raw = plain.raw.filter(|r| !r.contains(craft_drawingml::chart::write::NS_CX));
            if ex.is_some() {
                // The kept workbook belongs to the `chartEx` part; the fallback has none.
                plain.kept = Default::default();
            }
            let rid = x.chart_rel(o, &plain);
            frame_open(w, x, o, id, s);
            w.tag("a:graphicData").attr("uri", "http://schemas.openxmlformats.org/drawingml/2006/chart").open();
            w.tag("c:chart").attr("xmlns:c", craft_drawingml::ns::NS_C).attr("r:id", rid).empty();
            w.close();
            frame_close(w);
            if ex.is_some() {
                w.close();
                w.close();
            }
        }
        ShapeKind::Media(m) => {
            let id = cx.id_for(s);
            media(w, x, o, id, s, m);
        }
        ShapeKind::Model3d { media, poster } => write_model(w, x, o, cx, s, *media, *poster),
        ShapeKind::Ink { strokes } => {
            // Ink becomes a group of freeform lines.
            let id = cx.id_for(s);
            let xf = s.xfrm.unwrap_or_default();
            w.open("p:grpSp", &[]);
            w.open("p:nvGrpSpPr", &[]);
            cnvpr(w, x, o, id, s);
            w.empty("p:cNvGrpSpPr", &[]);
            w.empty("p:nvPr", &[]);
            w.close();
            w.open("p:grpSpPr", &[]);
            dml::xfrm(w, "a:xfrm", &xf, Some(&xf));
            w.close();
            for (i, st) in strokes.iter().enumerate().take(10_000) {
                let Some(stroke) = ink_shape(st, i) else { continue };
                let sid = cx.ids.fresh();
                sp(w, x, o, sid, &stroke, false);
            }
            w.close();
        }
        ShapeKind::Opaque { xml, preview, label, parts } => opaque(w, x, o, cx, s, xml, *preview, label, parts),
    }
}

fn model_rel(x: &mut Exp, o: &mut Out, media: crate::MediaId) -> Option<String> {
    if x.media(media).is_none_or(|m| m.data.is_empty()) {
        return None;
    }
    x.media_rel(o, media, super::RT_MODEL3D)
}

fn write_model_part(w: &mut Writer, x: &mut Exp, o: &mut Out, id: u32, s: &Shape, rid: &str) {
    w.tag("p:contentPart").attr("r:id", rid).open();
    w.open("p:nvContentPartPr", &[]);
    cnvpr(w, x, o, id, s);
    w.empty("p:cNvContentPartPr", &[]);
    w.empty("p:nvPr", &[]);
    w.close();
    if let Some(xf) = &s.xfrm {
        dml::xfrm(w, "p:xfrm", xf, None);
    }
    w.close();
}

fn write_model(w: &mut Writer, x: &mut Exp, o: &mut Out, cx: &mut Ctx, s: &Shape, media: crate::MediaId, poster: Option<crate::MediaId>) {
    let Some(rid) = model_rel(x, o, media) else {
        log::warn!("pptx: 3-D model '{}' has no embedded part", s.name);
        return;
    };
    let id = cx.id_for(s);
    let poster = poster.filter(|p| x.media(*p).is_some_and(|m| !m.data.is_empty()));
    let Some(poster) = poster else {
        write_model_part(w, x, o, id, s, &rid);
        return;
    };
    // Older readers draw the fallback picture; readers that know `am3d` use the model.
    w.tag("mc:AlternateContent").attr("xmlns:mc", NS_MC).open();
    w.tag("mc:Choice").attr("xmlns:am3d", "http://schemas.microsoft.com/office/drawing/2017/model3d").attr("Requires", "am3d").open();
    write_model_part(w, x, o, id, s, &rid);
    w.close();
    w.open("mc:Fallback", &[]);
    let pic = Shape {
        id: ShapeId(0),
        name: s.name.clone(),
        xfrm: s.xfrm,
        kind: ShapeKind::Picture { fill: crate::style::PictureFill { media: poster, ..Default::default() } },
        ..Default::default()
    };
    let pid = cx.ids.fresh();
    shape_with_id(w, x, o, cx, &pic, pid);
    w.close();
    w.close();
}

/// A SmartArt graphic: a graphic frame naming its data, layout, quick style and colour parts,
/// with the group's shapes as the cached drawing PowerPoint shows until it lays the diagram out.
#[allow(clippy::too_many_arguments)]
fn diagram(w: &mut Writer, x: &mut Exp, o: &mut Out, cx: &mut Ctx, s: &Shape, d: &crate::Diagram, children: &[Shape], child: &Xfrm) {
    use craft_drawingml::diagram::parts as dgm;
    let id = cx.id_for(s);
    let n = x.next_diagram();
    let frame = s.xfrm.unwrap_or(*child);
    let name = |kind: &str| format!("ppt/diagrams/{kind}{n}.xml");
    // The drawing: the group's shapes in the frame's own coordinates.
    let dname = name("drawing");
    let mut dout = Out::new(&dname);
    let mut dw = Writer::document();
    dw.tag("dsp:drawing")
        .attr("xmlns:dgm", dgm::NS_DGM)
        .attr("xmlns:dsp", dgm::NS_DSP)
        .attr("xmlns:a", super::NS_A)
        .attr("xmlns:r", super::NS_R)
        .open();
    dw.open("dsp:spTree", &[]);
    dw.open("dsp:nvGrpSpPr", &[]);
    dw.tag("dsp:cNvPr").attr("id", 0).attr("name", "").empty();
    dw.empty("dsp:cNvGrpSpPr", &[]);
    dw.close();
    dw.empty("dsp:grpSpPr", &[]);
    let (sx, sy) = (frame.w / child.w.max(1e-6), frame.h / child.h.max(1e-6));
    let mut k = 0;
    let mut flat = vec![];
    walk(children, &mut |c, _| match &c.kind {
        ShapeKind::Shape => flat.push(c.clone()),
        ShapeKind::Connector { .. } => {
            let mut sc = c.clone();
            if sc.geom.preset_name() == Some("rect") {
                sc.geom = Geom::preset("line");
            }
            flat.push(sc);
        }
        ShapeKind::Picture { fill } => {
            let mut sc = c.clone();
            sc.kind = ShapeKind::Shape;
            sc.fill = Some(crate::style::Fill::Picture(fill.clone()));
            flat.push(sc);
        }
        _ => {}
    });
    for mut c in flat.into_iter().take(10_000) {
        if let Some(xf) = &mut c.xfrm {
            *xf = Xfrm { x: (xf.x - child.x) * sx, y: (xf.y - child.y) * sy, w: xf.w * sx, h: xf.h * sy, ..*xf };
        }
        k += 1;
        dw.tag("dsp:sp").attr("modelId", format!("{{{:08X}-5D1A-4C0A-9E5B-{k:012X}}}", s.id.0)).open();
        dw.open("dsp:nvSpPr", &[]);
        dw.tag("dsp:cNvPr").attr("id", 0).attr("name", "").empty();
        dw.empty("dsp:cNvSpPr", &[]);
        dw.close();
        sp_pr_tag(&mut dw, x, &mut dout, &c, true, "dsp:spPr");
        if let Some(st) = &c.style {
            style_tag(&mut dw, st, "dsp:style");
        }
        if let Some(tb) = &c.text {
            dml::text_body(&mut dw, &mut Snk(x, &mut dout), "dsp:txBody", tb, false);
        }
        dw.close();
    }
    dw.close();
    dw.close();
    x.pkg.xml_part(&dname, dgm::CT_DRAWING, dw.into_bytes());
    x.pkg.rels(&dname, &dout.rels);
    let rel = |o: &mut Out, kind: &str, part: &str| o.rels.add_new(kind, &craft_opc::relative(&o.name, part), false);
    let drawing_rel = rel(o, dgm::RT_DRAWING, &dname);
    let data = name("data");
    // Node pictures: on the data model's presentation points, through the data part's rels.
    let mut mout = Out::new(&data);
    let pics: Vec<(usize, String)> =
        d.nodes.iter().enumerate().filter_map(|(k, n)| n.picture.and_then(|m| x.media_rel(&mut mout, m, "image")).map(|r| (k, r))).collect();
    x.pkg.xml_part(&data, dgm::CT_DATA, dgm::data_xml(d, s.id.0, &drawing_rel, &pics));
    x.pkg.rels(&data, &mout.rels);
    let dm = rel(o, "diagramData", &data);
    let layout = name("layout");
    let lo_xml = d.layout_xml.clone().map(String::into_bytes).unwrap_or_else(|| dgm::layout_xml(&d.kind));
    x.pkg.xml_part(&layout, dgm::CT_LAYOUT, lo_xml);
    let lo = rel(o, "diagramLayout", &layout);
    let style = name("quickStyle");
    x.pkg.xml_part(&style, dgm::CT_STYLE, d.style_xml.clone().map(String::into_bytes).unwrap_or_else(|| dgm::style_xml(&d.style)));
    let qs = rel(o, "diagramQuickStyle", &style);
    let colors = name("colors");
    x.pkg.xml_part(&colors, dgm::CT_COLORS, d.colors_xml.clone().map(String::into_bytes).unwrap_or_else(|| dgm::colors_xml(&d.colors)));
    let cs = rel(o, "diagramColors", &colors);
    let mut fs = s.clone();
    fs.xfrm = Some(frame);
    frame_open(w, x, o, id, &fs);
    w.tag("a:graphicData").attr("uri", "http://schemas.openxmlformats.org/drawingml/2006/diagram").open();
    w.tag("dgm:relIds").attr("xmlns:dgm", dgm::NS_DGM).attr("r:dm", dm).attr("r:lo", lo).attr("r:qs", qs).attr("r:cs", cs).empty();
    w.close();
    frame_close(w);
}

fn ink_shape(st: &crate::InkStroke, i: usize) -> Option<Shape> {
    let pts: Vec<(f64, f64)> = st.points.iter().filter(|p| p.0.is_finite() && p.1.is_finite()).map(|p| (p.0, p.1)).collect();
    let (x0, y0) = pts.iter().fold((f64::MAX, f64::MAX), |a, p| (a.0.min(p.0), a.1.min(p.1)));
    let (x1, y1) = pts.iter().fold((f64::MIN, f64::MIN), |a, p| (a.0.max(p.0), a.1.max(p.1)));
    if pts.len() < 2 {
        return None;
    }
    let (w, h) = ((x1 - x0).max(1.0), (y1 - y0).max(1.0));
    // PPTX keeps one line width. A pressured stroke exports at its mean sample width;
    // a legacy flat `0.5` stroke stays at the pen width.
    let legacy = crate::ink_pressure_is_legacy(&st.points);
    let width = if legacy {
        st.width
    } else {
        let n = st.points.len().max(1) as f64;
        st.points.iter().map(|p| crate::ink_sample_width(st.width, p.2, false)).sum::<f64>() / n
    };
    let mut d = String::new();
    for (k, (px, py)) in pts.iter().enumerate() {
        d.push_str(if k == 0 { "M " } else { "L " });
        d.push_str(&format!("{:.2} {:.2} ", px - x0, py - y0));
    }
    let mut c = crate::style::ColorRef::rgb(st.color.with_alpha(255));
    if st.highlighter {
        c = c.with(craft_drawingml::color::ColorTransform::Alpha(50_000));
    }
    Some(Shape {
        id: ShapeId(0),
        name: format!("Ink {}", i + 1),
        xfrm: Some(Xfrm::new(x0, y0, w, h)),
        geom: Geom::Custom { paths: vec![crate::CustomPath { w, h, d, fill: craft_drawingml::preset::FillMode::None, stroke: true }] },
        fill: Some(Fill::None),
        line: Some(crate::Line {
            cap: Some(crate::style::LineCap::Round),
            join: Some(crate::style::LineJoin::Round),
            ..crate::Line::solid(c, width.max(0.25))
        }),
        ..Default::default()
    })
}

fn sp(w: &mut Writer, x: &mut Exp, o: &mut Out, id: u32, s: &Shape, math: bool) {
    w.tag("p:sp").attr_opt("useBgFill", (matches!(s.fill, Some(Fill::Background))).then_some(1)).open();
    w.open("p:nvSpPr", &[]);
    cnvpr(w, x, o, id, s);
    let mut cnv = A::new();
    if s.text_box {
        cnv = cnv.a("txBox", "1");
    }
    let lock = s.ph.is_some() || s.locked;
    if lock {
        w.tag("p:cNvSpPr").list(cnv).open();
        locks(w, "a:spLocks", s, false);
        w.close();
    } else {
        w.tag("p:cNvSpPr").list(cnv).empty();
    }
    nv_pr(w, x, o, s);
    w.close();
    let tmp;
    let sref = if matches!(s.fill, Some(Fill::Background)) {
        let mut c = s.clone();
        c.fill = None;
        tmp = c;
        &tmp
    } else {
        s
    };
    sp_pr(w, x, o, sref, true);
    if let Some(st) = &s.style {
        style(w, st);
    }
    if let Some(tb) = &s.text {
        // A shape's text without its own anchor sits in the middle in the model; a file without
        // one would put it at the top, so say so.
        if s.ph.is_none() && !s.text_box && tb.body.anchor.is_none() {
            let mut tb = tb.clone();
            tb.body.anchor = Some(crate::text::Anchor::Middle);
            dml::text_body(w, &mut Snk(x, o), "p:txBody", &tb, math);
        } else {
            dml::text_body(w, &mut Snk(x, o), "p:txBody", tb, math);
        }
    }
    w.close();
}

fn frame_open(w: &mut Writer, x: &mut Exp, o: &mut Out, id: u32, s: &Shape) {
    w.open("p:graphicFrame", &[]);
    w.open("p:nvGraphicFramePr", &[]);
    cnvpr(w, x, o, id, s);
    w.open("p:cNvGraphicFramePr", &[]);
    w.tag("a:graphicFrameLocks")
        .attr("noGrp", "1")
        .attr_opt("noDrilldown", (s.locked).then_some(1))
        .attr_opt("noMove", (s.locked).then_some(1))
        .attr_opt("noResize", (s.locked).then_some(1))
        .empty();
    w.close();
    nv_pr(w, x, o, s);
    w.close();
    let xf = s.xfrm.unwrap_or_default();
    let mut xf0 = xf;
    xf0.rot = 0.0;
    xf0.flip_h = false;
    xf0.flip_v = false;
    dml::xfrm(w, "p:xfrm", &xf0, None);
    w.open("a:graphic", &[]);
}

fn frame_close(w: &mut Writer) {
    w.close();
    w.close();
}

fn table(w: &mut Writer, x: &mut Exp, o: &mut Out, t: &Table) {
    let mut t = t.clone();
    t.normalize();
    w.open("a:tbl", &[]);
    let (guid, _) = craft_drawingml::table::styles::guid_for_style(&t.style);
    if x.table_style_set.insert(t.style.clone()) {
        x.table_styles.push(t.style.clone());
    }
    w.tag("a:tblPr")
        .attr_opt("firstRow", (t.first_row).then_some(1))
        .attr_opt("firstCol", (t.first_col).then_some(1))
        .attr_opt("lastRow", (t.last_row).then_some(1))
        .attr_opt("lastCol", (t.last_col).then_some(1))
        .attr_opt("bandRow", (t.band_row).then_some(1))
        .attr_opt("bandCol", (t.band_col).then_some(1))
        .attr_opt("rtl", (t.rtl).map(u8::from))
        .open();
    if let Some(f) = &t.background {
        dml::fill(w, &mut Snk(x, o), f);
    }
    w.leaf("a:tableStyleId", &[], &guid);
    w.close();
    w.open("a:tblGrid", &[]);
    for c in &t.cols {
        w.tag("a:gridCol").attr("w", emu_pos(*c)).empty();
    }
    w.close();
    for row in &t.rows {
        w.tag("a:tr").attr("h", emu_pos(row.height)).open();
        for c in &row.cells {
            w.tag("a:tc")
                .attr_opt("gridSpan", (c.grid_span > 1).then_some(c.grid_span))
                .attr_opt("rowSpan", (c.row_span > 1).then_some(c.row_span))
                .attr_opt("hMerge", (c.h_merge).then_some(1))
                .attr_opt("vMerge", (c.v_merge).then_some(1))
                .open();
            dml::text_body(w, &mut Snk(x, o), "a:txBody", &c.text, false);
            let mut a = A::new();
            if let Some(m) = c.margins {
                a = a.a("marL", emu(m[0])).a("marR", emu(m[1])).a("marT", emu(m[2])).a("marB", emu(m[3]));
            }
            a = a.o("vert", c.vertical.map(|v| v.xml())).o(
                "anchor",
                c.anchor.map(|a| match a {
                    crate::text::Anchor::Justified | crate::text::Anchor::Distributed => "t",
                    other => other.xml(),
                }),
            );
            a = a.b("anchorCtr", c.anchor_ctr);
            a = a.o("horzOverflow", crate::text::TextOverflow::horz_xml(c.horz_overflow));
            w.tag("a:tcPr").list(a).open();
            for (tag, l) in [
                ("a:lnL", &c.borders[0]),
                ("a:lnR", &c.borders[1]),
                ("a:lnT", &c.borders[2]),
                ("a:lnB", &c.borders[3]),
                ("a:lnTlToBr", &c.diag_down),
                ("a:lnBlToTr", &c.diag_up),
            ] {
                if let Some(l) = l {
                    dml::line(w, &mut Snk(x, o), tag, l);
                }
            }
            if let Some(f) = &c.fill {
                match f {
                    Fill::Background => {}
                    f => dml::fill(w, &mut Snk(x, o), f),
                }
            }
            w.close();
            w.close();
        }
        w.close();
    }
    w.close();
}

fn media(w: &mut Writer, x: &mut Exp, o: &mut Out, id: u32, s: &Shape, m: &crate::MediaClip) {
    let kind = if m.video { "video" } else { "audio" };
    let link = x.media_rel(o, m.media, kind);
    let embed = x.media_rel_new(o, m.media, super::RT_MEDIA);
    w.open("p:pic", &[]);
    w.open("p:nvPicPr", &[]);
    let mut s2 = s.clone();
    if s2.click.is_none() {
        s2.click = Some(crate::text::Hyperlink {
            action: crate::text::Action::PlayMedia,
            tooltip: String::new(),
            highlight_click: false,
            sound: None,
            stop_sound: false,
            zoom_return: None,
        });
    }
    cnvpr(w, x, o, id, &s2);
    w.open("p:cNvPicPr", &[]);
    w.tag("a:picLocks").attr("noChangeAspect", "1").empty();
    w.close();
    w.open("p:nvPr", &[]);
    if let Some(ph) = &s.ph {
        let _ = ph;
    }
    let tag = if m.video { "a:videoFile" } else { "a:audioFile" };
    if let Some(l) = &link {
        w.tag(tag).attr("r:link", l).empty();
    }
    if let Some(e) = &embed {
        w.open("p:extLst", &[]);
        w.tag("p:ext").attr("uri", "{DAA4B4D4-6D71-4841-9C94-3DE7FCFB9230}").open();
        w.tag("p14:media").attr("xmlns:p14", super::NS_P14).attr("r:embed", e).open();
        if m.trim_start_ms > 0 || m.trim_end_ms > 0 {
            w.tag("p14:trim")
                .attr_opt("st", (m.trim_start_ms > 0).then_some(m.trim_start_ms))
                .attr_opt("end", (m.trim_end_ms > 0).then_some(m.trim_end_ms))
                .empty();
        }
        if m.fade_in_ms > 0 || m.fade_out_ms > 0 {
            w.tag("p14:fade")
                .attr_opt("in", (m.fade_in_ms > 0).then_some(m.fade_in_ms))
                .attr_opt("out", (m.fade_out_ms > 0).then_some(m.fade_out_ms))
                .empty();
        }
        if !m.bookmarks.is_empty() {
            w.open("p14:bmkLst", &[]);
            for (n, t) in m.bookmarks.iter().take(1000) {
                w.tag("p14:bmk").attr("name", n).attr("time", t).empty();
            }
            w.close();
        }
        w.close();
        w.close();
        w.close();
    }
    w.close();
    w.close();
    let poster = crate::style::PictureFill { media: m.poster.unwrap_or_default(), crop: m.crop, ..Default::default() };
    dml::blip_fill(w, &mut Snk(x, o), "p:blipFill", &poster);
    let mut s3 = s.clone();
    s3.fill = None;
    sp_pr(w, x, o, &s3, true);
    w.close();
}

#[allow(clippy::too_many_arguments)]
fn opaque(
    w: &mut Writer,
    x: &mut Exp,
    o: &mut Out,
    cx: &mut Ctx,
    s: &Shape,
    xml: &str,
    preview: Option<crate::MediaId>,
    label: &str,
    parts: &[(String, String, crate::MediaId)],
) {
    // Kept XML is written back as it was when every part it references was kept too, with those
    // parts re-added under this slide's relationship ids.
    if let Ok(doc) = crate::package::parse(xml.as_bytes())
        && matches!(doc.root.local(), "graphicFrame" | "sp" | "grpSp" | "cxnSp")
    {
        let mut part_map: std::collections::HashMap<&str, (&str, crate::MediaId)> = std::collections::HashMap::new();
        for (id, kind, m) in parts {
            part_map.entry(id.as_str()).or_insert((kind.as_str(), *m));
        }
        if let Some(ids) = xmlx::rel_refs(&doc.root)
            .iter()
            .map(|r| {
                let (kind, m) = part_map.get(r.as_str())?;
                Some((r.clone(), x.media_rel(o, *m, kind)?))
            })
            .collect::<Option<std::collections::HashMap<_, _>>>()
        {
            let mut root = doc.root;
            set_rel_ids(&mut root, &ids, 0);
            ole_fallback(&mut root, x, o, preview);
            let id = cx.id_for(s);
            set_cnvpr_id(&mut root, id);
            w.raw(&root.to_xml());
            return;
        }
    }
    log::warn!("pptx: object '{label}' written as its preview picture");
    let id = cx.id_for(s);
    let mut p = s.clone();
    if p.name.is_empty() {
        p.name = label.to_string();
    }
    match preview {
        Some(m) if x.media(m).is_some() => {
            p.kind = ShapeKind::Picture { fill: crate::style::PictureFill { media: m, ..Default::default() } };
            p.ph = None;
            shape_with_id(w, x, o, cx, &p, id);
        }
        _ => {
            p.kind = ShapeKind::Shape;
            p.ph = None;
            p.geom = Geom::preset("rect");
            p.fill = Some(Fill::None);
            p.text = None;
            sp(w, x, o, id, &p, false);
        }
    }
}

fn shape_with_id(w: &mut Writer, x: &mut Exp, o: &mut Out, cx: &mut Ctx, s: &Shape, id: u32) {
    // Pictures only: reuse the given id.
    if let ShapeKind::Picture { fill } = &s.kind {
        w.open("p:pic", &[]);
        w.open("p:nvPicPr", &[]);
        cnvpr(w, x, o, id, s);
        w.open("p:cNvPicPr", &[]);
        w.tag("a:picLocks").attr("noChangeAspect", "1").empty();
        w.close();
        w.empty("p:nvPr", &[]);
        w.close();
        dml::blip_fill(w, &mut Snk(x, o), "p:blipFill", fill);
        // As for any picture, a colour or pattern fill shows behind the image (an object's preview
        // keeps its fallback picture's fill).
        let mut s2 = s.clone();
        if matches!(s2.fill, Some(Fill::Picture(_) | Fill::Background | Fill::Group)) {
            s2.fill = None;
        }
        sp_pr(w, x, o, &s2, true);
        w.close();
    } else {
        shape(w, x, o, cx, s, 0);
    }
}

/// A legacy OLE object finds its preview in the slide's VML drawing through `spid`, and PowerPoint
/// won't open the slide without it. We don't write that drawing, so write PowerPoint's own
/// fallback form instead: no `spid`, and the preview as a `p:pic`.
fn ole_fallback(root: &mut craft_xml::El, x: &mut Exp, o: &mut Out, preview: Option<crate::MediaId>) {
    let xfrm = root.child(Local("xfrm")).map(|t| {
        let at = |e: &str, a: &str| t.child(Local(e)).and_then(|c| c.attr_local(a)).unwrap_or("0").to_string();
        (at("off", "x"), at("off", "y"), at("ext", "cx"), at("ext", "cy"))
    });
    let Some(ole) = root
        .elements_mut()
        .find(|e| e.is(Local("graphic")))
        .and_then(|g| g.elements_mut().find(|e| e.is(Local("graphicData"))))
        .and_then(|d| d.elements_mut().find(|e| e.is(Local("oleObj"))))
    else {
        return;
    };
    if ole.child(Local("pic")).is_some() || ole.attr_local("spid").is_none() {
        return;
    }
    ole.attrs.retain(|(k, _)| k != "spid");
    let rel = preview.and_then(|m| x.media_rel(o, m, "image")).unwrap_or_else(|| x.placeholder_rel(o));
    let (ox, oy, cx, cy) = xfrm.unwrap_or_default();
    let pic = format!(
        r#"<p:pic><p:nvPicPr><p:cNvPr id="0" name=""/><p:cNvPicPr/><p:nvPr/></p:nvPicPr><p:blipFill><a:blip r:embed="{rel}"/><a:stretch><a:fillRect/></a:stretch></p:blipFill><p:spPr><a:xfrm><a:off x="{ox}" y="{oy}"/><a:ext cx="{cx}" cy="{cy}"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></p:spPr></p:pic>"#
    );
    if let Ok(d) = crate::package::parse(pic.as_bytes()) {
        ole.children.push(craft_xml::Node::El(d.root));
    }
}

fn set_rel_ids(e: &mut craft_xml::El, ids: &std::collections::HashMap<String, String>, depth: usize) {
    if depth > xmlx::MAX_DEPTH {
        return;
    }
    for (k, v) in &mut e.attrs {
        if xmlx::is_rel_attr(k, v)
            && let Some(new) = ids.get(v.as_str())
        {
            v.clone_from(new);
        }
    }
    for c in e.elements_mut() {
        set_rel_ids(c, ids, depth + 1);
    }
}

fn set_cnvpr_id(el: &mut craft_xml::El, id: u32) {
    fn rec(e: &mut craft_xml::El, id: u32, depth: usize) -> bool {
        if depth > 8 {
            return false;
        }
        if e.is(Local("cNvPr")) {
            e.set_attr("id", id);
            return true;
        }
        e.elements_mut().any(|c| rec(c, id, depth + 1))
    }
    rec(el, id, 0);
}

/// The fixed group header every shape tree starts with.
pub fn tree_header(w: &mut Writer) {
    w.open("p:nvGrpSpPr", &[]);
    w.tag("p:cNvPr").attr("id", 1).attr("name", "").empty();
    w.empty("p:cNvGrpSpPr", &[]);
    w.empty("p:nvPr", &[]);
    w.close();
    w.open("p:grpSpPr", &[]);
    w.open("a:xfrm", &[]);
    w.tag("a:off").attr("x", 0).attr("y", 0).empty();
    w.tag("a:ext").attr("cx", 0).attr("cy", 0).empty();
    w.tag("a:chOff").attr("x", 0).attr("y", 0).empty();
    w.tag("a:chExt").attr("cx", 0).attr("cy", 0).empty();
    w.close();
    w.close();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// PowerPoint asks to repair a `star5` whose guide list lacks `hf`/`vf` (found black-box).
    #[test]
    fn preset_guides_are_complete() {
        let mut w = Writer::new();
        geometry(&mut w, &Geom::Preset { name: "star5".into(), adj: vec![20000.0] });
        let s = w.finish();
        assert_eq!(
            s,
            r#"<a:prstGeom prst="star5"><a:avLst><a:gd name="adj" fmla="val 20000"/><a:gd name="hf" fmla="val 105146"/><a:gd name="vf" fmla="val 110557"/></a:avLst></a:prstGeom>"#
        );
        let mut w = Writer::new();
        geometry(&mut w, &Geom::Preset { name: "rightArrow".into(), adj: vec![10000.0] });
        let s = w.finish();
        assert!(s.contains(r#"name="adj1" fmla="val 10000""#) && s.contains(r#"name="adj2" fmla="val 50000""#), "{}", s);
        let mut w = Writer::new();
        geometry(&mut w, &Geom::Preset { name: "textBox".into(), adj: vec![] });
        let s = w.finish();
        assert_eq!(s, r#"<a:prstGeom prst="rect"><a:avLst/></a:prstGeom>"#);
    }
}
