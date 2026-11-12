//! Writing the model: a drawing as tags, for [`crate::write_ascii_as`] or [`crate::write_binary`].
//!
//! [`Drawing::tags`] writes the sections, tables and objects every reader expects around the
//! content it is given: the header, the classes, the nine symbol tables (with the linetypes
//! `ByBlock`, `ByLayer` and `Continuous`, layer `0`, style `Standard`, the `ACAD` application and
//! the model and paper space block records), the blocks, the entities, and the dictionaries,
//! plot style placeholder and layouts of R2000 and later. [`entity_tags`] writes one entity.
//!
//! What a version does not have is left out: handles and subclass markers before R13, layouts,
//! lineweights and plot styles before R2000, true colour and transparency before R2004. An
//! LWPOLYLINE is written as a POLYLINE before R14. The caller chooses entities the version has
//! (R12 has no ELLIPSE, SPLINE, MTEXT, HATCH or LEADER): they are written as given.
//!
//! The file skeleton is VectorCraft's (`vectorcraft-cad`, `writer.rs`); the groups of each
//! entity follow the DXF Reference, as CADCraft's `dxf_write` writes them.

use std::collections::{HashMap, HashSet};

use super::entity::{Attrib, Common, Dimension, Entity, EntityKind, Image, Insert, Leader, LwPolyline, Polyline, Spline, Text, Viewport};
use super::hatch::{Hatch, HatchEdge, HatchPath};
use super::mleader::{MLeader, MLeaderStyle};
use super::table::{Table, TableStyle};
use super::tables::{DimStyle, Layer, Linetype, TextStyle};
use super::text::{MText, encode_contents, encode_line};
use super::{Color, Lineweight, Point, Transparency, aci};
use crate::{Tag, Version, string_groups};

/// A DXF version to write, R12 to 2018.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum Format {
    /// AC1009: no handles, no subclass markers.
    R12,
    /// AC1012.
    R13,
    /// AC1014: the first with LWPOLYLINE.
    R14,
    /// AC1015: layouts, lineweights, plot styles.
    R2000,
    /// AC1018: true colour and transparency.
    R2004,
    /// AC1021: text is UTF-8.
    R2007,
    R2010,
    R2013,
    #[default]
    R2018,
}

impl Format {
    pub const ALL: [Format; 9] =
        [Format::R12, Format::R13, Format::R14, Format::R2000, Format::R2004, Format::R2007, Format::R2010, Format::R2013, Format::R2018];

    /// The `$ACADVER` code.
    pub fn acadver(self) -> &'static str {
        match self {
            Format::R12 => "AC1009",
            Format::R13 => "AC1012",
            Format::R14 => "AC1014",
            Format::R2000 => "AC1015",
            Format::R2004 => "AC1018",
            Format::R2007 => "AC1021",
            Format::R2010 => "AC1024",
            Format::R2013 => "AC1027",
            Format::R2018 => "AC1032",
        }
    }
    /// Objects have handles, owners and subclass markers.
    pub fn handles(self) -> bool {
        self >= Format::R13
    }
    pub fn lwpolyline(self) -> bool {
        self >= Format::R14
    }
    pub fn r2000(self) -> bool {
        self >= Format::R2000
    }
    pub fn true_color(self) -> bool {
        self >= Format::R2004
    }
    /// Text is UTF-8: pass this to [`crate::write_ascii_as`].
    pub fn utf8(self) -> bool {
        self >= Format::R2007
    }
}

impl From<Version> for Format {
    fn from(v: Version) -> Format {
        match v {
            Version::R2000 => Format::R2000,
            Version::R2004 => Format::R2004,
            Version::R2007 => Format::R2007,
            Version::R2010 => Format::R2010,
            Version::R2013 => Format::R2013,
            Version::R2018 => Format::R2018,
        }
    }
}

/// The handles of the objects every drawing has. Handles given with the content are kept when
/// they are [`fixed::FIRST_FREE`] or more and not taken; anything else gets a new one.
pub mod fixed {
    pub const LAYER_TABLE: u64 = 0x1;
    pub const LTYPE_TABLE: u64 = 0x2;
    pub const APPID_TABLE: u64 = 0x3;
    pub const DIMSTYLE_TABLE: u64 = 0x4;
    pub const STYLE_TABLE: u64 = 0x5;
    pub const UCS_TABLE: u64 = 0x6;
    pub const VIEW_TABLE: u64 = 0x7;
    pub const VPORT_TABLE: u64 = 0x8;
    pub const BLOCK_RECORD_TABLE: u64 = 0x9;
    pub const ROOT: u64 = 0xA;
    pub const GROUPS: u64 = 0xB;
    pub const LAYOUTS: u64 = 0xC;
    pub const MODEL_RECORD: u64 = 0xD;
    pub const PAPER_RECORD: u64 = 0xE;
    pub const MODEL_BLOCK: u64 = 0xF;
    pub const MODEL_END: u64 = 0x10;
    pub const PAPER_BLOCK: u64 = 0x11;
    pub const PAPER_END: u64 = 0x12;
    pub const MODEL_LAYOUT: u64 = 0x13;
    pub const PAPER_LAYOUT: u64 = 0x14;
    pub const ACTIVE_VPORT: u64 = 0x15;
    pub const ACAD_APPID: u64 = 0x16;
    pub const STANDARD_DIMSTYLE: u64 = 0x17;
    pub const PLOT_STYLES: u64 = 0x1A;
    pub const PLOT_NORMAL: u64 = 0x1B;
    pub const BYBLOCK_LTYPE: u64 = 0x1C;
    pub const BYLAYER_LTYPE: u64 = 0x1D;
    pub const CONTINUOUS_LTYPE: u64 = 0x1E;
    pub const LAYER_0: u64 = 0x1F;
    pub const STANDARD_STYLE: u64 = 0x20;
    pub const FIRST_FREE: u64 = 0x30;
}

/// Tags in the order they are written.
struct W {
    v: Format,
    t: Vec<Tag>,
    /// Upper-case block name → its BLOCK_RECORD handle (an ACAD_TABLE's group 343).
    blocks: HashMap<String, u64>,
}

impl W {
    fn s(&mut self, code: i32, v: impl Into<String>) {
        self.t.push(Tag::s(code, v));
    }
    fn f(&mut self, code: i32, v: f64) {
        self.t.push(Tag::f(code, v));
    }
    fn i(&mut self, code: i32, v: i64) {
        self.t.push(Tag::i(code, v));
    }
    fn h(&mut self, code: i32, h: u64) {
        self.s(code, format!("{h:X}"));
    }
    fn xy(&mut self, code: i32, p: [f64; 2]) {
        self.f(code, p[0]);
        self.f(code + 10, p[1]);
    }
    fn p(&mut self, code: i32, p: Point) {
        self.xy(code, [p[0], p[1]]);
        self.f(code + 20, p[2]);
    }
    /// A subclass marker (R13 and later).
    fn sub(&mut self, marker: &str) {
        if self.v.handles() {
            self.s(100, marker);
        }
    }
    /// The start of an object: its type, and from R13 its handle, owner and first subclass.
    fn object(&mut self, kind: &str, h: u64, owner: u64, subclass: &str) {
        self.s(0, kind);
        if self.v.handles() {
            self.h(if kind == "DIMSTYLE" { 105 } else { 5 }, h);
            self.h(330, owner);
            self.s(100, subclass);
        }
    }
    /// A table record: the shared record subclass, then the table's own.
    fn record(&mut self, kind: &str, h: u64, table: u64, subclass: &str) {
        self.object(kind, h, table, "AcDbSymbolTableRecord");
        self.sub(subclass);
    }
    fn table(&mut self, name: &str, h: u64, count: usize) {
        self.s(0, "TABLE");
        self.s(2, name);
        if self.v.handles() {
            self.h(5, h);
            self.s(330, "0");
            self.s(100, "AcDbSymbolTable");
        }
        self.i(70, i64::try_from(count).unwrap_or(i64::MAX));
        if name == "DIMSTYLE" && self.v.r2000() {
            self.s(100, "AcDbDimStyleTable");
        }
    }
    fn end_table(&mut self) {
        self.s(0, "ENDTAB");
    }
    fn section(&mut self, name: &str) {
        self.s(0, "SECTION");
        self.s(2, name);
    }
    fn end_section(&mut self) {
        self.s(0, "ENDSEC");
    }
    /// The thickness and extrusion of a planar entity, when they are not the defaults.
    fn planar(&mut self, c: &Common) {
        if c.thickness != 0.0 {
            self.f(39, c.thickness);
        }
        if c.extrusion != [0.0, 0.0, 1.0] {
            self.p(210, c.extrusion);
        }
    }

    /// The start of an entity: type, handle, owner and the properties every entity has.
    fn common(&mut self, kind: &str, c: &Common, h: u64, owner: u64) {
        self.object(kind, h, owner, "AcDbEntity");
        if c.paper_space {
            self.i(67, 1);
        }
        self.s(8, c.layer.as_str());
        if let Some(lt) = &c.linetype {
            self.s(6, lt.as_str());
        }
        match c.color {
            Color::ByLayer => {}
            Color::ByBlock => self.i(62, 0),
            Color::Index(i) => self.i(62, i64::from(i)),
            Color::Rgb(rgb) => {
                // The nearest index for readers (and versions) without true colour.
                self.i(62, i64::from(aci::nearest(rgb, 1..=255)));
                if self.v.true_color() {
                    self.i(420, i64::from(u32::from_be_bytes([0, rgb[0], rgb[1], rgb[2]])));
                }
            }
        }
        if c.ltscale != 1.0 {
            self.f(48, c.ltscale);
        }
        if !c.visible {
            self.i(60, 1);
        }
        if self.v.r2000() && c.lineweight != Lineweight::ByLayer {
            self.i(370, c.lineweight.to_dxf());
        }
        if self.v.true_color() && c.transparency != Transparency::ByLayer {
            self.i(440, c.transparency.to_dxf());
        }
        if let Some(ps) = c.plot_style.filter(|_| self.v.r2000()) {
            self.h(390, ps);
        }
    }

    fn text(&mut self, t: &Text, c: &Common, attribute: bool) {
        self.sub("AcDbText");
        self.p(10, t.insert);
        self.f(40, t.height);
        self.s(1, encode_line(&t.value));
        self.f(50, t.rotation);
        self.f(41, t.width_factor);
        self.f(51, t.oblique);
        self.s(7, t.style.as_str());
        self.i(71, t.generation);
        self.i(72, t.halign);
        if let Some(a) = t.align {
            self.p(11, a);
        }
        self.planar(c);
        // An attribute's vertical alignment is group 74 of its own subclass.
        if !attribute {
            self.sub("AcDbText");
            self.i(73, t.valign);
        }
    }

    fn attrib(&mut self, kind: &str, a: &Attrib, c: &Common) {
        self.text(&a.text, c, true);
        self.sub(if kind == "ATTDEF" { "AcDbAttributeDefinition" } else { "AcDbAttribute" });
        if self.v.r2000() {
            self.i(280, 0);
        }
        if kind == "ATTDEF" {
            self.s(3, encode_line(&a.prompt));
        }
        self.s(2, a.tag.as_str());
        self.i(70, a.flags);
        self.i(74, a.text.valign);
        if self.v.r2000() {
            self.i(280, i64::from(a.lock_position));
        }
        // The embedded MTEXT of a multiline attribute (R2018; older versions keep the one line).
        if let Some(m) = a.multiline.as_ref().filter(|_| self.v >= Format::R2018) {
            self.s(101, "Embedded Object");
            self.mtext_groups(m);
        }
    }

    fn mtext_groups(&mut self, m: &MText) {
        self.p(10, m.insert);
        self.f(40, m.height);
        self.f(41, m.width);
        self.i(71, i64::from(m.attach));
        self.i(72, m.drawing_direction);
        let mut chunks = string_groups(&encode_contents(&m.contents));
        let last = chunks.pop().unwrap_or_default();
        for chunk in chunks {
            self.s(3, chunk);
        }
        self.s(1, last);
        self.s(7, m.style.as_str());
        match m.direction {
            Some(d) => self.p(11, d),
            None => self.f(50, m.rotation),
        }
        self.i(73, if m.line_spacing_exact { 2 } else { 1 });
        self.f(44, m.line_spacing);
        if let Some((flags, color, offset)) = m.background {
            self.i(90, flags);
            match color {
                Color::Rgb(rgb) => self.i(421, i64::from(u32::from_be_bytes([0, rgb[0], rgb[1], rgb[2]]))),
                Color::Index(i) => self.i(63, i64::from(i)),
                Color::ByBlock => self.i(63, 0),
                Color::ByLayer => self.i(63, 256),
            }
            self.f(45, offset);
        }
    }

    /// The column groups of an MTEXT: the legacy groups before R2018, an embedded object from
    /// R2018 on.
    fn mtext_columns(&mut self, m: &MText) {
        let Some(c) = &m.columns else { return };
        let manual = c.dynamic && !c.auto_height;
        let heights: Vec<f64> = if manual { c.heights.clone() } else { vec![c.height; usize::from(c.count.max(1))] };
        let count = if manual { heights.len().max(1) } else { usize::from(c.count.max(1)) };
        let n = i64::try_from(count).unwrap_or(1);
        let kind = if c.dynamic { 2 } else { 1 };
        if self.v < Format::R2018 {
            self.i(75, kind);
            self.i(76, n);
            self.i(78, i64::from(c.flow_reversed));
            self.i(79, i64::from(c.auto_height));
            self.f(48, c.width);
            self.f(49, c.gutter);
            // No count for an empty list: a lone 50 = 0 would read back as one height of 0.
            if !heights.is_empty() {
                self.i(50, i64::try_from(heights.len()).unwrap_or(0));
            }
            for h in heights {
                self.f(50, h);
            }
            return;
        }
        let height = if manual { heights.iter().copied().fold(0.0, f64::max) } else { c.height };
        let (sin, cos) = m.angle().sin_cos();
        #[expect(clippy::cast_precision_loss, reason = "a column count is at most 1,000")]
        let total = c.width * count as f64 + c.gutter * (count as f64 - 1.0);
        self.f(46, height);
        self.s(101, "Embedded Object");
        self.i(70, 1);
        self.p(10, [cos, sin, 0.0]);
        self.p(11, m.insert);
        self.f(40, c.width);
        self.f(41, height);
        self.f(42, total);
        self.f(43, height);
        self.i(71, kind);
        self.i(72, n);
        self.f(44, c.width);
        self.f(45, c.gutter);
        self.i(73, i64::from(c.auto_height));
        self.i(74, i64::from(c.flow_reversed));
        if manual {
            for h in heights {
                self.f(46, h);
            }
        }
    }

    fn lwpolyline(&mut self, p: &LwPolyline, c: &Common) {
        self.sub("AcDbPolyline");
        self.i(90, i64::try_from(p.vertices.len()).unwrap_or(0));
        self.i(70, i64::from(p.closed) | if p.plinegen { 128 } else { 0 });
        self.f(43, p.const_width);
        self.f(38, p.elevation);
        if c.thickness != 0.0 {
            self.f(39, c.thickness);
        }
        for v in &p.vertices {
            self.xy(10, [v.x, v.y]);
            if v.start_width != 0.0 || v.end_width != 0.0 {
                self.f(40, v.start_width);
                self.f(41, v.end_width);
            }
            if v.bulge != 0.0 {
                self.f(42, v.bulge);
            }
        }
        if c.extrusion != [0.0, 0.0, 1.0] {
            self.p(210, c.extrusion);
        }
    }

    fn spline(&mut self, s: &Spline) {
        let n = |v: usize| i64::try_from(v).unwrap_or(0);
        self.sub("AcDbSpline");
        self.i(70, s.flags);
        self.i(71, s.degree);
        self.i(72, n(s.knots.len()));
        self.i(73, n(s.control.len()));
        self.i(74, n(s.fit.len()));
        self.f(42, 1e-7);
        self.f(43, 1e-7);
        self.f(44, s.fit_tolerance);
        if let Some(t) = s.start_tangent {
            self.p(12, t);
        }
        if let Some(t) = s.end_tangent {
            self.p(13, t);
        }
        for k in &s.knots {
            self.f(40, *k);
        }
        for (i, p) in s.control.iter().enumerate() {
            self.p(10, *p);
            if let Some(w) = s.weights.get(i).filter(|_| s.weights.len() == s.control.len()) {
                self.f(41, *w);
            }
        }
        for p in &s.fit {
            self.p(11, *p);
        }
    }

    fn dimension(&mut self, d: &Dimension, c: &Common) {
        self.sub("AcDbDimension");
        if let Some(b) = &d.block {
            self.s(2, b.as_str());
        }
        self.p(10, d.defpt);
        self.p(11, d.text_mid);
        self.p(12, d.insertion);
        self.i(70, d.flags);
        self.s(1, encode_line(&d.text));
        if let Some(m) = d.measurement {
            self.f(42, m);
        }
        self.s(3, d.style.as_str());
        if d.text_rotation != 0.0 {
            self.f(53, d.text_rotation);
        }
        if d.horizontal != 0.0 {
            self.f(51, d.horizontal);
        }
        if c.extrusion != [0.0, 0.0, 1.0] {
            self.p(210, c.extrusion);
        }
        match d.kind() {
            0 | 1 => {
                self.sub("AcDbAlignedDimension");
                self.p(13, d.p13);
                self.p(14, d.p14);
                if d.kind() == 0 {
                    self.f(50, d.rotation);
                    if d.oblique != 0.0 {
                        self.f(52, d.oblique);
                    }
                    self.sub("AcDbRotatedDimension");
                }
            }
            2 => {
                self.sub("AcDb2LineAngularDimension");
                for (code, p) in [(13, d.p13), (14, d.p14), (15, d.p15), (16, d.p16)] {
                    self.p(code, p);
                }
            }
            3 | 4 => {
                self.sub(if d.kind() == 3 { "AcDbDiametricDimension" } else { "AcDbRadialDimension" });
                self.p(15, d.p15);
                self.f(40, d.leader_length);
            }
            5 => {
                self.sub("AcDb3PointAngularDimension");
                for (code, p) in [(13, d.p13), (14, d.p14), (15, d.p15)] {
                    self.p(code, p);
                }
            }
            _ => {
                self.sub("AcDbOrdinateDimension");
                self.p(13, d.p13);
                self.p(14, d.p14);
            }
        }
    }

    fn leader(&mut self, l: &Leader) {
        self.sub("AcDbLeader");
        self.s(3, l.style.as_str());
        self.i(71, i64::from(l.arrow));
        self.i(72, i64::from(l.spline));
        self.i(73, l.annotation_type);
        self.i(76, i64::try_from(l.vertices.len()).unwrap_or(0));
        for v in &l.vertices {
            self.p(10, *v);
        }
        if let Some(a) = l.annotation {
            self.h(340, a);
        }
    }

    fn hatch_path(&mut self, path: &HatchPath) {
        let n = |v: usize| i64::try_from(v).unwrap_or(0);
        match path {
            HatchPath::Polyline { closed, vertices } => {
                let bulges = vertices.iter().any(|v| v[2] != 0.0);
                self.i(72, i64::from(bulges));
                self.i(73, i64::from(*closed));
                self.i(93, n(vertices.len()));
                for v in vertices {
                    self.xy(10, [v[0], v[1]]);
                    if bulges {
                        self.f(42, v[2]);
                    }
                }
            }
            HatchPath::Edges(edges) => {
                self.i(93, n(edges.len()));
                for e in edges {
                    self.hatch_edge(e);
                }
            }
        }
    }

    fn hatch_edge(&mut self, e: &HatchEdge) {
        let n = |v: usize| i64::try_from(v).unwrap_or(0);
        match e {
            HatchEdge::Line { start, end } => {
                self.i(72, 1);
                self.xy(10, *start);
                self.xy(11, *end);
            }
            HatchEdge::Arc { center, radius, start_angle, end_angle, ccw } => {
                self.i(72, 2);
                self.xy(10, *center);
                self.f(40, *radius);
                self.f(50, *start_angle);
                self.f(51, *end_angle);
                self.i(73, i64::from(*ccw));
            }
            HatchEdge::Ellipse { center, major, ratio, start_angle, end_angle, ccw } => {
                self.i(72, 3);
                self.xy(10, *center);
                self.xy(11, *major);
                self.f(40, *ratio);
                self.f(50, *start_angle);
                self.f(51, *end_angle);
                self.i(73, i64::from(*ccw));
            }
            HatchEdge::Spline { degree, rational, periodic, knots, control, weights, fit, start_tangent, end_tangent } => {
                self.i(72, 4);
                self.i(94, *degree);
                self.i(73, i64::from(*rational));
                self.i(74, i64::from(*periodic));
                self.i(95, n(knots.len()));
                self.i(96, n(control.len()));
                for k in knots {
                    self.f(40, *k);
                }
                for (i, p) in control.iter().enumerate() {
                    self.xy(10, *p);
                    if let Some(w) = weights.get(i).filter(|_| weights.len() == control.len()) {
                        self.f(42, *w);
                    }
                }
                // Fit data came with R2010.
                if self.v >= Format::R2010 {
                    self.i(97, n(fit.len()));
                    for p in fit {
                        self.xy(11, *p);
                    }
                    if let Some(t) = start_tangent {
                        self.xy(12, *t);
                    }
                    if let Some(t) = end_tangent {
                        self.xy(13, *t);
                    }
                }
            }
        }
    }

    fn hatch(&mut self, h: &Hatch, c: &Common) {
        let n = |v: usize| i64::try_from(v).unwrap_or(0);
        self.sub("AcDbHatch");
        self.p(10, [0.0, 0.0, h.elevation]);
        self.p(210, c.extrusion);
        self.s(2, h.pattern.as_str());
        self.i(70, i64::from(h.solid));
        self.i(71, i64::from(h.associative));
        self.i(91, n(h.loops.len()));
        for l in &h.loops {
            self.i(92, l.flags);
            self.hatch_path(&l.path);
            self.i(97, n(l.sources.len()));
            for s in &l.sources {
                self.h(330, *s);
            }
        }
        self.i(75, h.style);
        self.i(76, h.pattern_type);
        if !h.solid {
            self.f(52, h.angle);
            self.f(41, h.scale);
            self.i(77, i64::from(h.double));
            self.i(78, n(h.pattern_lines.len()));
            for l in &h.pattern_lines {
                self.f(53, l.angle);
                self.f(43, l.base[0]);
                self.f(44, l.base[1]);
                self.f(45, l.offset[0]);
                self.f(46, l.offset[1]);
                self.i(79, n(l.dashes.len()));
                for d in &l.dashes {
                    self.f(49, *d);
                }
            }
        }
        self.i(98, n(h.seeds.len()));
        for s in &h.seeds {
            self.xy(10, *s);
        }
        if let Some(g) = h.gradient.as_ref().filter(|_| self.v.true_color()) {
            self.i(450, 1);
            self.i(451, 0);
            self.f(460, g.angle);
            self.f(461, g.shift);
            self.i(452, i64::from(g.single));
            self.f(462, g.tint);
            self.i(453, n(g.colors.len()));
            for (i, color) in g.colors.iter().enumerate() {
                #[expect(clippy::cast_precision_loss, reason = "a gradient has a handful of colours")]
                self.f(463, i as f64);
                match color {
                    Color::Rgb(rgb) => {
                        self.i(63, i64::from(aci::nearest(*rgb, 1..=255)));
                        self.i(421, i64::from(u32::from_be_bytes([0, rgb[0], rgb[1], rgb[2]])));
                    }
                    Color::Index(i) => self.i(63, i64::from(*i)),
                    Color::ByBlock => self.i(63, 0),
                    Color::ByLayer => self.i(63, 256),
                }
            }
            self.s(470, g.name.as_str());
        }
    }

    fn viewport(&mut self, v: &Viewport) {
        self.sub("AcDbViewport");
        self.p(10, v.center);
        self.f(40, v.width);
        self.f(41, v.height);
        self.i(68, i64::from(v.status & 131_072 == 0));
        self.i(69, v.id);
        self.xy(12, v.view_center);
        self.p(16, v.view_direction);
        self.p(17, v.view_target);
        self.f(45, v.view_height);
        self.f(51, v.twist);
        for l in &v.frozen_layers {
            self.h(331, *l);
        }
        self.i(90, v.status);
        if let Some(c) = v.clip {
            self.h(340, c);
        }
    }

    fn image(&mut self, kind: &str, i: &Image) {
        self.sub(if kind == "WIPEOUT" { "AcDbWipeout" } else { "AcDbRasterImage" });
        self.i(90, 0);
        self.p(10, i.insert);
        self.p(11, i.u);
        self.p(12, i.v);
        self.xy(13, i.size);
        if let Some(d) = i.image_def {
            self.h(340, d);
        }
        self.i(70, i.display);
        self.i(280, i64::from(i.clipping));
        self.i(281, i.brightness);
        self.i(282, i.contrast);
        self.i(283, i.fade);
        self.i(71, i.clip_type);
        self.i(91, i64::try_from(i.clip.len()).unwrap_or(0));
        for q in &i.clip {
            self.xy(14, *q);
        }
        if self.v >= Format::R2010 {
            self.i(290, i64::from(i.clip_inverted));
        }
    }

    /// A POLYLINE's own groups; its vertices and SEQEND follow in [`Ids::entity`].
    fn polyline_head(&mut self, p: &Polyline, c: &Common) {
        self.sub(polyline_subclass(p.flags).0);
        self.i(66, 1);
        self.p(10, [0.0, 0.0, p.elevation]);
        self.i(70, p.flags);
        if p.start_width != 0.0 || p.end_width != 0.0 {
            self.f(40, p.start_width);
            self.f(41, p.end_width);
        }
        if p.is_mesh() || p.is_polyface() {
            self.i(71, p.m);
            self.i(72, p.n);
        }
        if p.smooth != 0 {
            self.i(75, p.smooth);
        }
        self.planar(c);
    }

    fn insert_head(&mut self, i: &Insert, c: &Common) {
        self.sub("AcDbBlockReference");
        if !i.attribs.is_empty() {
            self.i(66, 1);
        }
        self.s(2, i.block.as_str());
        self.p(10, i.insert);
        self.f(41, i.scale[0]);
        self.f(42, i.scale[1]);
        self.f(43, i.scale[2]);
        self.f(50, i.rotation);
        if i.columns > 1 || i.rows > 1 {
            self.i(70, i.columns);
            self.i(71, i.rows);
            self.f(44, i.column_spacing);
            self.f(45, i.row_spacing);
        }
        if c.extrusion != [0.0, 0.0, 1.0] {
            self.p(210, c.extrusion);
        }
    }

    fn corners(&mut self, corners: &[Point; 4]) {
        for (code, p) in (10..).zip(corners) {
            self.p(code, *p);
        }
    }

    /// The groups of an entity after its common properties; `false` for a kind that has none
    /// here ([`EntityKind::Other`]).
    fn geometry(&mut self, kind: &EntityKind, c: &Common) -> bool {
        match kind {
            EntityKind::Line { start, end } => {
                self.sub("AcDbLine");
                self.p(10, *start);
                self.p(11, *end);
                self.planar(c);
            }
            EntityKind::Point { location, angle } => {
                self.sub("AcDbPoint");
                self.p(10, *location);
                self.planar(c);
                if *angle != 0.0 {
                    self.f(50, *angle);
                }
            }
            EntityKind::Circle { center, radius } => {
                self.sub("AcDbCircle");
                self.p(10, *center);
                self.f(40, *radius);
                self.planar(c);
            }
            EntityKind::Arc { center, radius, start_angle, end_angle } => {
                self.sub("AcDbCircle");
                self.p(10, *center);
                self.f(40, *radius);
                self.planar(c);
                self.sub("AcDbArc");
                self.f(50, *start_angle);
                self.f(51, *end_angle);
            }
            EntityKind::Ellipse { center, major, ratio, start, end } => {
                self.sub("AcDbEllipse");
                self.p(10, *center);
                self.p(11, *major);
                if c.extrusion != [0.0, 0.0, 1.0] {
                    self.p(210, c.extrusion);
                }
                self.f(40, *ratio);
                self.f(41, *start);
                self.f(42, *end);
            }
            EntityKind::LwPolyline(p) => self.lwpolyline(p, c),
            EntityKind::Polyline(p) => self.polyline_head(p, c),
            EntityKind::Spline(s) => self.spline(s),
            EntityKind::Helix(h) => {
                self.spline(&h.spline);
                self.sub("AcDbHelix");
                // The release numbers the format expects.
                self.i(90, 29);
                self.i(91, 63);
                self.p(10, h.axis_base);
                self.p(11, h.start);
                self.p(12, h.axis);
                self.f(40, h.radius);
                self.f(41, h.turns);
                self.f(42, h.turn_height);
                self.i(290, i64::from(h.ccw));
                self.i(280, h.constrain);
            }
            EntityKind::MLine(m) => {
                let n = |v: usize| i64::try_from(v).unwrap_or(0);
                self.sub("AcDbMline");
                self.s(2, m.style.as_str());
                self.h(340, m.style_handle.unwrap_or(0));
                self.f(40, m.scale);
                self.i(70, m.justification);
                self.i(71, m.flags);
                self.i(72, n(m.vertices.len()));
                self.i(73, m.elements);
                self.p(10, m.start);
                if c.extrusion != [0.0, 0.0, 1.0] {
                    self.p(210, c.extrusion);
                }
                for v in &m.vertices {
                    self.p(11, v.location);
                    self.p(12, v.direction);
                    self.p(13, v.miter);
                    for (k, params) in v.element_params.iter().enumerate() {
                        self.i(74, n(params.len()));
                        for x in params {
                            self.f(41, *x);
                        }
                        let fill = v.fill_params.get(k).map_or(&[][..], Vec::as_slice);
                        self.i(75, n(fill.len()));
                        for x in fill {
                            self.f(42, *x);
                        }
                    }
                }
            }
            EntityKind::Ray { base, direction } | EntityKind::XLine { base, direction } => {
                self.sub(if matches!(kind, EntityKind::Ray { .. }) { "AcDbRay" } else { "AcDbXline" });
                self.p(10, *base);
                self.p(11, *direction);
            }
            EntityKind::Text(t) => self.text(t, c, false),
            EntityKind::MText(m) => {
                self.sub("AcDbMText");
                self.mtext_groups(m);
                if c.extrusion != [0.0, 0.0, 1.0] {
                    self.p(210, c.extrusion);
                }
                self.mtext_columns(m);
            }
            EntityKind::AttDef(a) => self.attrib("ATTDEF", a, c),
            EntityKind::Insert(i) => self.insert_head(i, c),
            EntityKind::Dimension(d) => self.dimension(d, c),
            EntityKind::Leader(l) => self.leader(l),
            EntityKind::Solid { corners } | EntityKind::Trace { corners } => {
                self.sub("AcDbTrace");
                self.corners(corners);
                self.planar(c);
            }
            EntityKind::Face3d { corners, hidden_edges } => {
                self.sub("AcDbFace");
                self.corners(corners);
                if *hidden_edges != 0 {
                    self.i(70, *hidden_edges);
                }
            }
            EntityKind::Hatch(h) => self.hatch(h, c),
            EntityKind::Viewport(v) => self.viewport(v),
            EntityKind::Image(i) => self.image("IMAGE", i),
            EntityKind::Wipeout(i) => self.image("WIPEOUT", i),
            EntityKind::Tolerance { insert, direction, style, text } => {
                self.sub("AcDbFcf");
                self.s(3, style.as_str());
                self.p(10, *insert);
                self.s(1, text.as_str());
                if c.extrusion != [0.0, 0.0, 1.0] {
                    self.p(210, c.extrusion);
                }
                self.p(11, *direction);
            }
            EntityKind::MLeader(m) => self.mleader(m),
            EntityKind::Table(t) => self.acad_table(t, c),
            EntityKind::Other => return false,
        }
        true
    }
}

impl W {
    fn opt_p(&mut self, code: i32, p: Option<Point>) {
        if let Some(p) = p {
            self.p(code, p);
        }
    }
    fn opt_f(&mut self, code: i32, v: Option<f64>) {
        if let Some(v) = v {
            self.f(code, v);
        }
    }
    fn opt_i(&mut self, code: i32, v: Option<i64>) {
        if let Some(v) = v {
            self.i(code, v);
        }
    }
    fn opt_h(&mut self, code: i32, v: Option<u64>) {
        if let Some(v) = v {
            self.h(code, v);
        }
    }

    /// A MULTILEADER's groups after its common properties, in the order and with the fixed
    /// values CADCraft's writer uses (`dxf_mleader.rs` `entity_tags`): the context block with its
    /// roots and lines, then the entity's own overrides. A value the model does not have is not
    /// written.
    fn mleader(&mut self, m: &MLeader) {
        // ByBlock and "no colour" as 32-bit colour values (0xC1000000, 0xC8000000).
        const BYBLOCK: i64 = -1_056_964_608;
        const NO_COLOR: i64 = -939_524_096;
        let n = |v: usize| i64::try_from(v).unwrap_or(i64::MAX);
        self.sub("AcDbMLeader");
        self.i(270, 2);
        self.s(300, "CONTEXT_DATA{");
        self.f(40, 1.0);
        self.opt_p(10, m.base);
        self.opt_f(41, m.text_height);
        self.opt_f(140, m.context_arrow_size);
        self.opt_f(145, m.landing_gap);
        self.opt_i(174, m.context_left_attachment);
        self.opt_i(175, m.context_right_attachment);
        // The horizontal alignment (176) is the text alignment (171) less one.
        self.opt_i(176, m.text_alignment.map(|a| a - 1));
        self.i(177, 1);
        self.opt_i(290, m.has_text.map(i64::from));
        if let Some(t) = &m.text {
            self.s(304, encode_contents(t));
            self.p(11, [0.0, 0.0, 1.0]);
        }
        self.opt_h(340, m.context_text_style);
        self.opt_p(12, m.text_location);
        self.opt_p(13, m.text_direction);
        self.opt_f(42, m.text_rotation);
        self.opt_f(43, m.text_width);
        if m.text.is_some() {
            self.f(44, 0.0);
        }
        self.opt_f(45, m.line_spacing);
        self.opt_i(170, m.line_spacing_style);
        if m.text.is_some() {
            self.i(90, BYBLOCK);
        }
        self.opt_i(171, m.text_alignment);
        if m.text.is_some() {
            self.i(172, 1);
            self.i(91, NO_COLOR);
            self.f(141, 1.5);
            for code in [92, 291, 292, 173, 293] {
                self.i(code, 0);
            }
            self.f(142, 0.0);
            self.f(143, 0.0);
            self.i(294, 0);
            self.i(295, 0);
        }
        self.opt_i(296, m.has_block.map(i64::from));
        self.opt_h(341, m.context_block);
        if m.context_block.is_some() {
            self.p(14, [0.0, 0.0, 1.0]);
        }
        self.opt_p(15, m.block_location);
        self.opt_p(16, m.context_block_scale);
        self.opt_f(46, m.context_block_rotation);
        if let Some(at) = m.block_location {
            self.i(93, BYBLOCK);
            // Block to world, row by row. shortcut: takes the block's base point as its origin,
            // as CADCraft's blocks are; a caller with another base adjusts the location.
            let [sx, sy, sz] = m.context_block_scale.unwrap_or([1.0; 3]);
            let (sin, cos) = m.context_block_rotation.unwrap_or(0.0).sin_cos();
            for v in [sx * cos, -sy * sin, 0.0, at[0], sx * sin, sy * cos, 0.0, at[1], 0.0, 0.0, sz, at[2], 0.0, 0.0, 0.0, 1.0] {
                self.f(47, v);
            }
        }
        self.p(110, [0.0, 0.0, m.base.map_or(0.0, |b| b[2])]);
        self.p(111, [1.0, 0.0, 0.0]);
        self.p(112, [0.0, 1.0, 0.0]);
        self.i(297, 0);
        for r in &m.roots {
            self.s(302, "LEADER{");
            self.i(290, 1);
            self.i(291, 1);
            self.opt_p(10, r.landing);
            self.opt_p(11, r.direction);
            self.i(90, 0);
            self.opt_f(40, r.dogleg);
            for (k, l) in r.lines.iter().enumerate() {
                self.s(304, "LEADER_LINE{");
                for v in l {
                    self.p(10, *v);
                }
                self.i(91, n(k));
                self.s(305, "}");
            }
            self.i(271, 0);
            self.s(303, "}");
        }
        self.i(272, 9);
        self.i(273, 9);
        self.s(301, "}");
        self.opt_h(340, m.style);
        self.i(90, 0x1 | 0x10 | 0x40 | 0x80 | 0x200 | 0x400 | 0x800 | 0x1_0000 | 0x8_0000);
        self.opt_i(170, m.leader_type);
        self.i(91, BYBLOCK);
        self.i(171, -2);
        self.i(290, 1);
        self.opt_i(291, m.dogleg_on.map(i64::from));
        self.opt_f(41, m.dogleg);
        self.opt_f(42, m.arrow_size);
        self.opt_i(172, m.content_type);
        self.opt_h(343, m.text_style);
        self.opt_i(173, m.left_attachment);
        self.opt_i(95, m.right_attachment);
        self.i(174, 1);
        self.i(175, m.text_alignment.map_or(0, |a| a - 1));
        self.i(92, BYBLOCK);
        self.i(292, 0);
        self.opt_h(344, m.block);
        if m.block.is_some() {
            self.i(93, BYBLOCK);
        }
        self.opt_p(10, m.block_scale);
        self.opt_f(43, m.block_rotation);
        if m.block.is_some() {
            self.i(176, 1);
        }
        for (k, (h, text)) in m.attribs.iter().enumerate() {
            self.h(330, *h);
            self.i(177, n(k));
            self.f(44, 0.0);
            self.s(302, text.as_str());
        }
        self.i(293, 0);
        self.i(294, 0);
        self.i(178, 0);
        self.i(179, 1);
        self.f(45, 1.0);
        self.i(271, 0);
        self.i(272, 9);
        self.i(273, 9);
    }

    /// An ACAD_TABLE's groups after its common properties, as CADCraft's writer orders them
    /// (`dxf_write/entity.rs`). Group 343 is the record of the table's block when the drawing
    /// being written has a block of that name.
    fn acad_table(&mut self, t: &Table, c: &Common) {
        let n = |v: usize| i64::try_from(v).unwrap_or(i64::MAX);
        self.sub("AcDbBlockReference");
        if let Some(b) = &t.block {
            self.s(2, b.as_str());
        }
        self.p(10, t.insert);
        if c.extrusion != [0.0, 0.0, 1.0] {
            self.p(210, c.extrusion);
        }
        self.sub("AcDbTable");
        self.i(280, 0);
        self.opt_h(342, t.style);
        let record = t.block.as_ref().and_then(|b| self.blocks.get(&b.to_ascii_uppercase()).copied());
        self.opt_h(343, record);
        self.p(11, t.direction);
        self.i(90, 0);
        self.i(91, n(t.rows));
        self.i(92, n(t.columns));
        for code in [93, 94, 95, 96] {
            self.i(code, 0);
        }
        for h in &t.row_heights {
            self.f(141, *h);
        }
        for w in &t.column_widths {
            self.f(142, *w);
        }
        for cell in &t.cells {
            self.i(171, cell.cell_type);
            self.i(172, 0);
            self.i(173, i64::from(cell.merged));
            self.i(174, 0);
            // A cell without a span of its own is a single cell.
            let (rs, cs) = cell.span.unwrap_or((1, 1));
            self.i(175, i64::from(cs));
            self.i(176, i64::from(rs));
            self.i(91, 0);
            self.i(178, 0);
            self.f(145, 0.0);
            self.opt_f(140, cell.text_height);
            self.opt_i(170, cell.alignment);
            if let Some(st) = &cell.text_style {
                self.s(7, st.as_str());
            }
            self.opt_h(340, cell.block);
            // Long text: code-2 chunks, the last part in code 1.
            let mut chunks = string_groups(&cell.text);
            let last = chunks.pop().unwrap_or_default();
            for ch in chunks {
                self.s(2, ch);
            }
            self.s(1, last);
        }
    }

    /// A MLEADERSTYLE object, with the fixed values CADCraft's writer uses (`dxf_write/objects.rs`
    /// `mleader_style_obj`).
    fn mleader_style(&mut self, s: &MLeaderStyle, h: u64, dict: u64) {
        const BYBLOCK: i64 = -1_056_964_608;
        self.owned_object("MLEADERSTYLE", h, dict, "AcDbMLeaderStyle");
        self.i(179, 2);
        self.i(170, s.content_type);
        self.i(171, 1);
        self.i(172, 0);
        self.i(90, 2);
        self.f(40, 0.0);
        self.f(41, 0.0);
        self.i(173, s.leader_type);
        self.i(91, BYBLOCK);
        self.h(340, fixed::BYBLOCK_LTYPE);
        self.i(92, -2);
        self.i(290, 1);
        self.f(42, s.landing_gap);
        self.i(291, 1);
        self.f(43, s.dogleg);
        self.s(3, s.description.as_str());
        self.f(44, s.arrow_size);
        self.s(300, "");
        self.opt_h(342, s.text_style);
        self.i(174, 1);
        self.i(178, 1);
        self.i(175, 1);
        self.i(176, 0);
        self.i(93, BYBLOCK);
        self.f(45, s.text_height);
        self.i(292, 0);
        self.i(297, 0);
        self.f(46, 4.0);
        self.opt_h(343, s.block);
        self.i(94, BYBLOCK);
        for code in [47, 49, 140] {
            self.f(code, 1.0);
        }
        self.i(293, 1);
        self.f(141, 0.0);
        self.i(294, 1);
        self.i(177, 0);
        self.f(142, 1.0);
        self.i(295, 0);
        self.i(296, i64::from(s.annotative));
        self.f(143, 0.125);
        self.i(271, 0);
        self.i(272, 9);
        self.i(273, 9);
    }

    /// A TABLESTYLE object, as CADCraft's writer orders it (`dxf_write/objects.rs`
    /// `table_style_obj`, `dxf_table.rs` `cell_type_tags`).
    fn table_style(&mut self, s: &TableStyle, h: u64, dict: u64) {
        let color = |c: Color| match c {
            Color::ByLayer => 256,
            Color::ByBlock => 0,
            Color::Index(i) => i64::from(i),
            Color::Rgb(rgb) => i64::from(aci::nearest(rgb, 1..=255)),
        };
        self.owned_object("TABLESTYLE", h, dict, "AcDbTableStyle");
        self.s(3, s.description.as_str());
        self.i(70, s.flow);
        self.i(71, 0);
        self.opt_f(40, s.margin_h);
        self.opt_f(41, s.margin_v);
        self.i(280, i64::from(s.title_suppressed));
        self.i(281, i64::from(s.header_suppressed));
        for ct in &s.cell_types {
            self.s(7, ct.text_style.as_str());
            self.opt_f(140, ct.text_height);
            self.opt_i(170, ct.alignment);
            self.opt_i(62, ct.text_color.map(color));
            match ct.fill {
                Some(f) => {
                    self.i(63, color(f));
                    self.i(283, 1);
                }
                None => self.i(283, 0),
            }
            for (k, b) in (0..).zip(&ct.borders) {
                self.opt_i(274 + k, b.lineweight.map(Lineweight::to_dxf));
            }
            for (k, b) in (0..).zip(&ct.borders) {
                self.opt_i(284 + k, b.visible.map(i64::from));
            }
            for (k, b) in (0..).zip(&ct.borders) {
                self.opt_i(64 + k, b.color.map(color));
            }
        }
    }

    /// An object owned by a dictionary, with that dictionary as its reactor too.
    fn owned_object(&mut self, kind: &str, h: u64, dict: u64, subclass: &str) {
        self.s(0, kind);
        self.h(5, h);
        self.s(102, "{ACAD_REACTORS");
        self.h(330, dict);
        self.s(102, "}");
        self.h(330, dict);
        self.s(100, subclass);
    }
}

/// The subclass markers of a POLYLINE with these flags, and of its vertices.
fn polyline_subclass(flags: i64) -> (&'static str, &'static str) {
    if flags & 64 != 0 {
        ("AcDbPolyFaceMesh", "AcDbPolyFaceMeshVertex")
    } else if flags & 16 != 0 {
        ("AcDbPolygonMesh", "AcDbPolygonMeshVertex")
    } else if flags & 8 != 0 {
        ("AcDb3dPolyline", "AcDb3dPolylineVertex")
    } else {
        ("AcDb2dPolyline", "AcDb2dVertex")
    }
}

/// The record type of an entity kind; `None` for one the writer leaves out.
fn kind_name(kind: &EntityKind, format: Format) -> Option<&'static str> {
    Some(match kind {
        EntityKind::Line { .. } => "LINE",
        EntityKind::Point { .. } => "POINT",
        EntityKind::Circle { .. } => "CIRCLE",
        EntityKind::Arc { .. } => "ARC",
        EntityKind::Ellipse { .. } => "ELLIPSE",
        EntityKind::LwPolyline(_) => "LWPOLYLINE",
        EntityKind::Polyline(_) => "POLYLINE",
        EntityKind::Spline(_) => "SPLINE",
        EntityKind::Helix(_) => "HELIX",
        EntityKind::MLine(_) => "MLINE",
        EntityKind::Ray { .. } => "RAY",
        EntityKind::XLine { .. } => "XLINE",
        EntityKind::Text(_) => "TEXT",
        EntityKind::MText(_) => "MTEXT",
        EntityKind::AttDef(_) => "ATTDEF",
        EntityKind::Insert(_) => "INSERT",
        EntityKind::Dimension(_) => "DIMENSION",
        EntityKind::Leader(_) => "LEADER",
        EntityKind::Solid { .. } => "SOLID",
        EntityKind::Trace { .. } => "TRACE",
        EntityKind::Face3d { .. } => "3DFACE",
        EntityKind::Hatch(_) => "HATCH",
        EntityKind::Viewport(_) => "VIEWPORT",
        EntityKind::Image(_) => "IMAGE",
        EntityKind::Wipeout(_) => "WIPEOUT",
        EntityKind::Tolerance { .. } => "TOLERANCE",
        // MULTILEADER came with the 2007 format, ACAD_TABLE with 2004.
        EntityKind::MLeader(_) if format >= Format::R2007 => "MULTILEADER",
        EntityKind::Table(_) if format >= Format::R2004 => "ACAD_TABLE",
        EntityKind::MLeader(_) | EntityKind::Table(_) | EntityKind::Other => return None,
    })
}

/// An LWPOLYLINE as the POLYLINE versions before R14 have.
fn heavy(p: &LwPolyline) -> Polyline {
    use super::entity::Vertex;
    let vertices = p
        .vertices
        .iter()
        .map(|v| Vertex {
            location: [v.x, v.y, p.elevation],
            bulge: v.bulge,
            start_width: v.start_width,
            end_width: v.end_width,
            flags: 0,
            face: [0; 4],
        })
        .collect();
    Polyline {
        flags: i64::from(p.closed) | if p.plinegen { 128 } else { 0 },
        elevation: p.elevation,
        start_width: p.const_width,
        end_width: p.const_width,
        m: 0,
        n: 0,
        smooth: 0,
        vertices,
    }
}

/// Handles: those given are kept when free, the rest are numbered after them.
struct Ids {
    used: HashSet<u64>,
    /// The handle tried next for content without one.
    next: u64,
    /// The least handle that content may keep.
    floor: u64,
}

impl Ids {
    fn new(given: impl Iterator<Item = u64>) -> Ids {
        let mut next = fixed::FIRST_FREE;
        for h in given {
            next = next.max(h.saturating_add(1));
        }
        Ids { used: (0..fixed::FIRST_FREE).collect(), next, floor: fixed::FIRST_FREE }
    }
    fn take(&mut self, given: Option<u64>) -> u64 {
        if let Some(h) = given.filter(|h| *h >= self.floor && self.used.insert(*h)) {
            return h;
        }
        while !self.used.insert(self.next) {
            self.next = self.next.saturating_add(1);
        }
        self.next
    }

    /// One entity with what follows it (vertices, attributes, SEQEND); `false` when its kind
    /// can't be written.
    fn entity(&mut self, w: &mut W, e: &Entity, owner: u64) -> bool {
        let converted;
        let kind = match &e.kind {
            EntityKind::LwPolyline(p) if !w.v.lwpolyline() => {
                converted = EntityKind::Polyline(heavy(p));
                &converted
            }
            k => k,
        };
        let Some(name) = kind_name(kind, w.v) else { return false };
        let h = self.take(e.common.handle);
        w.common(name, &e.common, h, owner);
        w.geometry(kind, &e.common);
        // The followers take the entity's layer; each has a handle and is owned by the entity.
        let follower = Common { handle: None, ..e.common.clone() };
        match kind {
            EntityKind::Polyline(p) => {
                let (_, vertex) = polyline_subclass(p.flags);
                for v in &p.vertices {
                    let vh = self.take(None);
                    w.common("VERTEX", &follower, vh, h);
                    w.sub(if v.is_face() { "AcDbFaceRecord" } else { "AcDbVertex" });
                    if !v.is_face() {
                        w.sub(vertex);
                    }
                    w.p(10, v.location);
                    if v.start_width != 0.0 || v.end_width != 0.0 {
                        w.f(40, v.start_width);
                        w.f(41, v.end_width);
                    }
                    if v.bulge != 0.0 {
                        w.f(42, v.bulge);
                    }
                    w.i(70, v.flags);
                    for (code, ix) in (71..).zip(v.face).filter(|(_, ix)| *ix != 0) {
                        w.i(code, ix);
                    }
                }
                let sh = self.take(None);
                w.common("SEQEND", &follower, sh, h);
            }
            EntityKind::Insert(i) if !i.attribs.is_empty() => {
                for (c, a) in &i.attribs {
                    let ah = self.take(c.handle);
                    w.common("ATTRIB", c, ah, h);
                    w.attrib("ATTRIB", a, c);
                }
                let sh = self.take(None);
                w.common("SEQEND", &follower, sh, h);
            }
            _ => {}
        }
        true
    }
}

/// The tags of one entity (with its vertices or attributes and their SEQEND) for `format`, owned
/// by `owner`: for a caller that writes its own file around them. Its handle is the entity's, or
/// one from `next_handle`; followers get handles from `next_handle` too, which is moved past
/// those used. Empty for [`EntityKind::Other`], and a MULTILEADER before 2007 or an ACAD_TABLE before 2004.
pub fn entity_tags(e: &Entity, format: Format, owner: u64, next_handle: &mut u64) -> Vec<Tag> {
    let mut w = W { v: format, t: Vec::new(), blocks: HashMap::new() };
    let mut ids = Ids { used: HashSet::new(), next: *next_handle, floor: 0 };
    let written = ids.entity(&mut w, e, owner);
    *next_handle = ids.next.saturating_add(1);
    if written { w.t } else { Vec::new() }
}

/// A block definition to write.
#[derive(Clone, Debug, Default)]
pub struct BlockDef {
    pub name: String,
    pub base: Point,
    /// Group 4.
    pub description: String,
    pub entities: Vec<Entity>,
}

/// A drawing to write: the content, around which [`Drawing::tags`] writes the file.
#[derive(Clone, Debug)]
pub struct Drawing<'a> {
    pub format: Format,
    /// `$DWGCODEPAGE` (`ANSI_1252` when empty). Before R2007, write the tags with
    /// [`crate::write_ascii_in`] in the same code page.
    pub code_page: String,
    /// `$INSUNITS` (R2000 and later).
    pub insunits: i64,
    /// `$MEASUREMENT`: metric (R2000 and later).
    pub metric: bool,
    /// `[x0, y0, x1, y1]` of the content: `$EXTMIN`/`$EXTMAX`, and the view the file opens at.
    pub extents: [f64; 4],
    /// `$LIMMIN`/`$LIMMAX`.
    pub limits: [f64; 4],
    /// More header variables: the name (with its `$`) and its groups, written after the ones
    /// above. One named like a variable this writer writes replaces it.
    pub header: Vec<(String, Vec<Tag>)>,
    /// Layers other than `0`. A layer's linetype is written as named.
    pub layers: Vec<Layer>,
    /// Linetypes other than `ByBlock`, `ByLayer` and `Continuous`.
    pub linetypes: Vec<Linetype>,
    /// Text styles other than `Standard`.
    pub styles: Vec<TextStyle>,
    /// Dimension styles other than `Standard`.
    pub dim_styles: Vec<DimStyle<'a>>,
    pub blocks: Vec<BlockDef>,
    /// The ENTITIES section: model space, and the paper space entities ([`Common::paper_space`])
    /// of the one layout. An entity's owner is set by the writer.
    pub entities: Vec<Entity>,
    /// Records written after the entities as they are (types this model does not have). Their
    /// handles must not collide with the content's: the writer does not read them.
    pub raw_entities: Vec<Tag>,
    /// CLASS records and OBJECTS records written as they are, and the entries that name those
    /// objects in the root dictionary (R13 and later).
    pub raw_classes: Vec<Tag>,
    pub raw_objects: Vec<Tag>,
    pub root_entries: Vec<(String, u64)>,
    /// Multileader styles by name, in the `ACAD_MLEADERSTYLE` dictionary (2007 and later). A
    /// MULTILEADER names its style by handle: give the style a handle and the entity the same.
    pub mleader_styles: Vec<(String, MLeaderStyle)>,
    /// Table styles by name, in the `ACAD_TABLESTYLE` dictionary (2004 and later).
    pub table_styles: Vec<(String, TableStyle)>,
}

impl Default for Drawing<'_> {
    fn default() -> Self {
        Drawing {
            format: Format::default(),
            code_page: String::new(),
            insunits: 0,
            metric: true,
            extents: [0.0, 0.0, 1.0, 1.0],
            limits: [0.0, 0.0, 420.0, 297.0],
            header: Vec::new(),
            layers: Vec::new(),
            linetypes: Vec::new(),
            styles: Vec::new(),
            dim_styles: Vec::new(),
            blocks: Vec::new(),
            entities: Vec::new(),
            raw_entities: Vec::new(),
            raw_classes: Vec::new(),
            raw_objects: Vec::new(),
            root_entries: Vec::new(),
            mleader_styles: Vec::new(),
            table_styles: Vec::new(),
        }
    }
}

const MODEL: &str = "*Model_Space";
const PAPER: &str = "*Paper_Space";

impl Drawing<'_> {
    /// The whole file as tags, ending with `0 EOF`, and how many entities were left out
    /// ([`EntityKind::Other`], and a MULTILEADER before 2007 or an ACAD_TABLE before 2004, which it has no groups for: pass
    /// those in `raw_entities`).
    pub fn tags(&self) -> (Vec<Tag>, usize) {
        let v = self.format;
        let mut w = W { v, t: Vec::new(), blocks: HashMap::new() };
        let entities = self.entities.iter().chain(self.blocks.iter().flat_map(|b| &b.entities));
        let attribs = entities.clone().filter_map(|e| match &e.kind {
            EntityKind::Insert(i) => Some(i.attribs.iter().filter_map(|(c, _)| c.handle)),
            _ => None,
        });
        let tables = self.layers.iter().map(|l| l.handle).chain(self.linetypes.iter().map(|l| l.handle)).chain(self.styles.iter().map(|s| s.handle));
        let given = entities
            .filter_map(|e| e.common.handle)
            .chain(attribs.flatten())
            .chain(tables.flatten())
            .chain(self.dim_styles.iter().filter_map(|d| d.handle))
            .chain(self.mleader_styles.iter().filter_map(|(_, s)| s.handle))
            .chain(self.table_styles.iter().filter_map(|(_, s)| s.handle));
        let mut ids = Ids::new(given.chain(self.root_entries.iter().map(|(_, h)| *h)));
        // Table records first, so entities numbered after them never take a handle a record asks for.
        let layer_h: Vec<u64> = self.layers.iter().map(|l| ids.take(l.handle)).collect();
        let ltype_h: Vec<u64> = self.linetypes.iter().map(|l| ids.take(l.handle)).collect();
        let style_h: Vec<u64> = self.styles.iter().map(|s| ids.take(s.handle)).collect();
        let dim_h: Vec<u64> = self.dim_styles.iter().map(|d| ids.take(d.handle)).collect();
        let block_h: Vec<[u64; 3]> = self.blocks.iter().map(|_| [ids.take(None), ids.take(None), ids.take(None)]).collect();
        // The style dictionaries and their styles, in the versions that have them.
        let styles = |on: bool, handles: Vec<Option<u64>>, ids: &mut Ids| {
            (on && !handles.is_empty()).then(|| (ids.take(None), handles.into_iter().map(|h| ids.take(h)).collect::<Vec<u64>>()))
        };
        let mleader_h = styles(v >= Format::R2007, self.mleader_styles.iter().map(|(_, s)| s.handle).collect(), &mut ids);
        let table_h = styles(v >= Format::R2004, self.table_styles.iter().map(|(_, s)| s.handle).collect(), &mut ids);

        // The content first: the header's handle seed comes after every handle.
        let names = self.blocks.iter().zip(&block_h).map(|(b, h)| (b.name.to_ascii_uppercase(), h[0]));
        let mut body = W { v, t: Vec::new(), blocks: names.collect() };
        let mut skipped = 0usize;
        if v.handles() {
            body.section("BLOCKS");
            for (name, record, begin, end) in [
                (MODEL, fixed::MODEL_RECORD, fixed::MODEL_BLOCK, fixed::MODEL_END),
                (PAPER, fixed::PAPER_RECORD, fixed::PAPER_BLOCK, fixed::PAPER_END),
            ] {
                block_begin(&mut body, name, [0.0; 3], "", begin, record);
                block_end(&mut body, end, record);
            }
        } else if !self.blocks.is_empty() {
            body.section("BLOCKS");
        }
        for (b, [record, begin, end]) in self.blocks.iter().zip(&block_h) {
            block_begin(&mut body, &b.name, b.base, &b.description, *begin, *record);
            for e in &b.entities {
                skipped += usize::from(!ids.entity(&mut body, e, *record));
            }
            block_end(&mut body, *end, *record);
        }
        if v.handles() || !self.blocks.is_empty() {
            body.end_section();
        }
        body.section("ENTITIES");
        for e in &self.entities {
            let owner = if e.common.paper_space { fixed::PAPER_RECORD } else { fixed::MODEL_RECORD };
            skipped += usize::from(!ids.entity(&mut body, e, owner));
        }
        body.t.extend(self.raw_entities.iter().cloned());
        body.end_section();
        if v.handles() {
            self.objects(&mut body, mleader_h.as_ref(), table_h.as_ref());
        }

        self.header(&mut w, ids.take(None));
        if v.handles() {
            let all = || self.entities.iter().chain(self.blocks.iter().flat_map(|b| &b.entities));
            let mut classes = Vec::new();
            if table_h.is_some() {
                classes.push(("TABLESTYLE", "AcDbTableStyle", 4095, false));
            }
            if mleader_h.is_some() {
                classes.push(("MLEADERSTYLE", "AcDbMLeaderStyle", 4095, false));
            }
            if v >= Format::R2004 && all().any(|e| matches!(e.kind, EntityKind::Table(_))) {
                classes.push(("ACAD_TABLE", "AcDbTable", 1025, true));
            }
            if v >= Format::R2007 && all().any(|e| matches!(e.kind, EntityKind::MLeader(_))) {
                classes.push(("MULTILEADER", "AcDbMLeader", 1025, true));
            }
            self.classes(&mut w, &classes);
        }
        w.section("TABLES");
        self.vport(&mut w);
        self.ltypes(&mut w, &ltype_h);
        self.layer_table(&mut w, &layer_h);
        self.style_table(&mut w, &style_h);
        if v.handles() {
            for (name, h) in [("VIEW", fixed::VIEW_TABLE), ("UCS", fixed::UCS_TABLE)] {
                w.table(name, h, 0);
                w.end_table();
            }
            w.table("APPID", fixed::APPID_TABLE, 1);
            w.record("APPID", fixed::ACAD_APPID, fixed::APPID_TABLE, "AcDbRegAppTableRecord");
            w.s(2, "ACAD");
            w.i(70, 0);
            w.end_table();
            self.dim_style_table(&mut w, &dim_h);
            w.table("BLOCK_RECORD", fixed::BLOCK_RECORD_TABLE, 2 + self.blocks.len());
            let user = self.blocks.iter().zip(&block_h).map(|(b, h)| (b.name.as_str(), h[0], None));
            for (name, h, layout) in
                [(MODEL, fixed::MODEL_RECORD, Some(fixed::MODEL_LAYOUT)), (PAPER, fixed::PAPER_RECORD, Some(fixed::PAPER_LAYOUT))]
                    .into_iter()
                    .chain(user)
            {
                w.record("BLOCK_RECORD", h, fixed::BLOCK_RECORD_TABLE, "AcDbBlockTableRecord");
                w.s(2, name);
                if v.r2000() {
                    w.h(340, layout.unwrap_or(0));
                    w.i(70, 0);
                    w.i(280, 1);
                    w.i(281, 0);
                }
            }
            w.end_table();
        }
        w.end_section();
        w.t.append(&mut body.t);
        w.s(0, "EOF");
        (w.t, skipped)
    }

    fn header(&self, w: &mut W, handseed: u64) {
        let v = self.format;
        let [x0, y0, x1, y1] = self.extents;
        let [lx0, ly0, lx1, ly1] = self.limits;
        let code_page = if self.code_page.is_empty() { "ANSI_1252" } else { self.code_page.as_str() };
        let mut vars: Vec<(&str, Vec<Tag>)> = vec![
            ("$ACADVER", vec![Tag::s(1, v.acadver())]),
            ("$DWGCODEPAGE", vec![Tag::s(3, code_page)]),
            ("$INSBASE", vec![Tag::f(10, 0.0), Tag::f(20, 0.0), Tag::f(30, 0.0)]),
            ("$EXTMIN", vec![Tag::f(10, x0), Tag::f(20, y0), Tag::f(30, 0.0)]),
            ("$EXTMAX", vec![Tag::f(10, x1), Tag::f(20, y1), Tag::f(30, 0.0)]),
            ("$LIMMIN", vec![Tag::f(10, lx0), Tag::f(20, ly0)]),
            ("$LIMMAX", vec![Tag::f(10, lx1), Tag::f(20, ly1)]),
        ];
        if v.r2000() {
            vars.push(("$INSUNITS", vec![Tag::i(70, self.insunits)]));
            vars.push(("$MEASUREMENT", vec![Tag::i(70, i64::from(self.metric))]));
        }
        w.section("HEADER");
        let given = |name: &str| self.header.iter().any(|(n, _)| n.eq_ignore_ascii_case(name));
        // The version and the handle seed are the writer's to say.
        for (name, tags) in vars.into_iter().filter(|(n, _)| *n == "$ACADVER" || !given(n)) {
            w.s(9, name);
            w.t.extend(tags);
        }
        for (name, tags) in self.header.iter().filter(|(n, _)| !n.eq_ignore_ascii_case("$ACADVER") && !n.eq_ignore_ascii_case("$HANDSEED")) {
            w.s(9, name.as_str());
            w.t.extend(tags.iter().cloned());
        }
        if v.handles() {
            w.s(9, "$HANDSEED");
            w.h(5, handseed);
        }
        w.end_section();
    }

    /// The CLASSES section: the classes every R2000 file has, then `extra` (name, C++ class,
    /// proxy flags, is an entity), then the caller's.
    fn classes(&self, w: &mut W, extra: &[(&str, &str, i64, bool)]) {
        w.section("CLASSES");
        if self.format.r2000() {
            for (name, cpp) in
                [("ACDBDICTIONARYWDFLT", "AcDbDictionaryWithDefault"), ("ACDBPLACEHOLDER", "AcDbPlaceHolder"), ("LAYOUT", "AcDbLayout")]
            {
                w.s(0, "CLASS");
                w.s(1, name);
                w.s(2, cpp);
                w.s(3, "ObjectDBX Classes");
                w.i(90, 0);
                if self.format.true_color() {
                    w.i(91, 0);
                }
                w.i(280, 0);
                w.i(281, 0);
            }
        }
        for (name, cpp, flags, entity) in extra {
            w.s(0, "CLASS");
            w.s(1, *name);
            w.s(2, *cpp);
            w.s(3, "ObjectDBX Classes");
            w.i(90, *flags);
            if self.format.true_color() {
                w.i(91, 0);
            }
            w.i(280, 0);
            w.i(281, i64::from(*entity));
        }
        w.t.extend(self.raw_classes.iter().cloned());
        w.end_section();
    }

    /// The active viewport, zoomed to the drawing's extents.
    fn vport(&self, w: &mut W) {
        let [x0, y0, x1, y1] = self.extents;
        w.table("VPORT", fixed::VPORT_TABLE, 1);
        w.record("VPORT", fixed::ACTIVE_VPORT, fixed::VPORT_TABLE, "AcDbViewportTableRecord");
        w.s(2, if self.format.handles() { "*Active" } else { "*ACTIVE" });
        w.i(70, 0);
        w.xy(10, [0.0, 0.0]);
        w.xy(11, [1.0, 1.0]);
        w.xy(12, [(x0 + x1) / 2.0, (y0 + y1) / 2.0]);
        w.xy(13, [0.0, 0.0]);
        w.xy(14, [1.0, 1.0]);
        w.xy(15, [10.0, 10.0]);
        w.p(16, [0.0, 0.0, 1.0]);
        w.p(17, [0.0; 3]);
        let (width, height) = ((x1 - x0).abs(), (y1 - y0).abs());
        // The view's height, with a margin; its width follows from the aspect ratio.
        w.f(40, (height.max(width / 1.5) * 1.1).max(1.0));
        w.f(41, 1.5);
        w.f(42, 50.0);
        for code in [43, 44, 50, 51] {
            w.f(code, 0.0);
        }
        for (code, value) in [(71, 0), (72, 1000), (73, 1), (74, 3), (75, 0), (76, 0), (77, 0), (78, 0)] {
            w.i(code, value);
        }
        if self.format.r2000() {
            w.i(281, 0);
            w.i(65, 0);
            w.f(146, 0.0);
        }
        w.end_table();
    }

    fn ltypes(&self, w: &mut W, handles: &[u64]) {
        let n = |v: usize| i64::try_from(v).unwrap_or(0);
        w.table("LTYPE", fixed::LTYPE_TABLE, 3 + self.linetypes.len());
        for (name, desc, h) in
            [("ByBlock", "", fixed::BYBLOCK_LTYPE), ("ByLayer", "", fixed::BYLAYER_LTYPE), ("Continuous", "Solid line", fixed::CONTINUOUS_LTYPE)]
        {
            w.record("LTYPE", h, fixed::LTYPE_TABLE, "AcDbLinetypeTableRecord");
            w.s(2, name);
            w.i(70, 0);
            w.s(3, desc);
            w.i(72, 65);
            w.i(73, 0);
            w.f(40, 0.0);
        }
        for (l, h) in self.linetypes.iter().zip(handles) {
            w.record("LTYPE", *h, fixed::LTYPE_TABLE, "AcDbLinetypeTableRecord");
            w.s(2, l.name.as_str());
            w.i(70, 0);
            w.s(3, l.description.as_str());
            w.i(72, 65);
            w.i(73, n(l.pattern.len()));
            w.f(40, l.pattern.iter().map(|e| e.length.abs()).sum());
            for e in &l.pattern {
                w.f(49, e.length);
                if !w.v.handles() {
                    continue;
                }
                let complex = e.text.is_some() || e.shape.is_some();
                w.i(
                    74,
                    i64::from(e.absolute && complex)
                        | if e.text.is_some() {
                            2
                        } else if e.shape.is_some() {
                            4
                        } else {
                            0
                        },
                );
                if complex {
                    w.i(75, i64::from(e.shape.unwrap_or(0)));
                    w.h(340, e.style.unwrap_or(0));
                    w.f(46, e.scale);
                    w.f(50, e.rotation);
                    w.f(44, e.offset[0]);
                    w.f(45, e.offset[1]);
                    if let Some(t) = &e.text {
                        w.s(9, t.as_str());
                    }
                }
            }
        }
        w.end_table();
    }

    fn layer_table(&self, w: &mut W, handles: &[u64]) {
        let zero = Layer {
            handle: None,
            name: "0".into(),
            color: Color::Index(7),
            on: true,
            frozen: false,
            vp_freeze_new: false,
            locked: false,
            plot: true,
            linetype: "Continuous".into(),
            lineweight: Lineweight::Default,
            plot_style: None,
            transparency: Transparency::ByLayer,
            description: None,
        };
        w.table("LAYER", fixed::LAYER_TABLE, 1 + self.layers.len());
        for (l, h) in std::iter::once((&zero, fixed::LAYER_0)).chain(self.layers.iter().zip(handles.iter().copied())) {
            w.record("LAYER", h, fixed::LAYER_TABLE, "AcDbLayerTableRecord");
            w.s(2, l.name.as_str());
            w.i(70, i64::from(l.frozen) | if l.vp_freeze_new { 2 } else { 0 } | if l.locked { 4 } else { 0 });
            let (index, rgb) = match l.color {
                Color::Index(i) => (i.max(1), None),
                Color::Rgb(rgb) => (aci::nearest(rgb, 1..=255), Some(rgb)),
                Color::ByLayer | Color::ByBlock => (7, None),
            };
            // A negative colour switches the layer off.
            w.i(62, if l.on { i64::from(index) } else { -i64::from(index) });
            if let Some(rgb) = rgb.filter(|_| w.v.true_color()) {
                w.i(420, i64::from(u32::from_be_bytes([0, rgb[0], rgb[1], rgb[2]])));
            }
            w.s(6, l.linetype.as_str());
            if w.v.r2000() {
                w.i(290, i64::from(l.plot));
                w.i(370, l.lineweight.to_dxf());
                w.h(390, l.plot_style.unwrap_or(fixed::PLOT_NORMAL));
                if let Some(d) = &l.description {
                    w.s(1001, "AcAecLayerStandard");
                    w.s(1000, "");
                    w.s(1000, d.as_str());
                }
                if l.transparency != Transparency::ByLayer && w.v.true_color() {
                    w.s(1001, "AcCmTransparency");
                    w.i(1071, l.transparency.to_dxf());
                }
            }
        }
        w.end_table();
    }

    fn style_table(&self, w: &mut W, handles: &[u64]) {
        let standard = TextStyle {
            handle: None,
            name: "Standard".into(),
            font_file: "txt".into(),
            big_font: String::new(),
            typeface: None,
            height: 0.0,
            width_factor: 1.0,
            oblique: 0.0,
            shape_file: false,
            vertical: false,
            backwards: false,
            upside_down: false,
            annotative: false,
        };
        w.table("STYLE", fixed::STYLE_TABLE, 1 + self.styles.len());
        for (s, h) in std::iter::once((&standard, fixed::STANDARD_STYLE)).chain(self.styles.iter().zip(handles.iter().copied())) {
            w.record("STYLE", h, fixed::STYLE_TABLE, "AcDbTextStyleTableRecord");
            w.s(2, s.name.as_str());
            w.i(70, i64::from(s.shape_file) | if s.vertical { 4 } else { 0 });
            w.f(40, s.height);
            w.f(41, s.width_factor);
            w.f(50, s.oblique);
            w.i(71, if s.backwards { 2 } else { 0 } | if s.upside_down { 4 } else { 0 });
            w.f(42, 2.5);
            w.s(3, s.font_file.as_str());
            w.s(4, s.big_font.as_str());
            // From 2000 the typeface too, which is how CAD apps find a TrueType font.
            if let Some(face) = s.typeface.as_ref().filter(|_| w.v.r2000()) {
                w.s(1001, "ACAD");
                w.s(1000, face.as_str());
                w.i(1071, 0);
            }
        }
        w.end_table();
    }

    fn dim_style_table(&self, w: &mut W, handles: &[u64]) {
        w.table("DIMSTYLE", fixed::DIMSTYLE_TABLE, 1 + self.dim_styles.len());
        w.record("DIMSTYLE", fixed::STANDARD_DIMSTYLE, fixed::DIMSTYLE_TABLE, "AcDbDimStyleTableRecord");
        w.s(2, "Standard");
        w.i(70, 0);
        for (d, h) in self.dim_styles.iter().zip(handles) {
            w.record("DIMSTYLE", *h, fixed::DIMSTYLE_TABLE, "AcDbDimStyleTableRecord");
            w.s(2, d.name.as_str());
            // The flags come first: a later 70 is DIMTFILLCLR.
            w.i(70, d.flags);
            w.t.extend(d.vars.iter().map(|(_, t)| (*t).clone()));
        }
        w.end_table();
    }

    fn objects(&self, w: &mut W, mleader_h: Option<&(u64, Vec<u64>)>, table_h: Option<&(u64, Vec<u64>)>) {
        let v = self.format;
        w.section("OBJECTS");
        let dict = |w: &mut W, h: u64, owner: u64, entries: &[(&str, u64)]| {
            w.object("DICTIONARY", h, owner, "AcDbDictionary");
            w.i(281, 1);
            for (name, e) in entries {
                w.s(3, *name);
                w.h(350, *e);
            }
        };
        let mut root = vec![("ACAD_GROUP", fixed::GROUPS)];
        if v.r2000() {
            root.extend([("ACAD_LAYOUT", fixed::LAYOUTS), ("ACAD_PLOTSTYLENAME", fixed::PLOT_STYLES)]);
        }
        if let Some((d, _)) = mleader_h {
            root.push(("ACAD_MLEADERSTYLE", *d));
        }
        if let Some((d, _)) = table_h {
            root.push(("ACAD_TABLESTYLE", *d));
        }
        root.extend(self.root_entries.iter().map(|(n, h)| (n.as_str(), *h)));
        dict(w, fixed::ROOT, 0, &root);
        dict(w, fixed::GROUPS, fixed::ROOT, &[]);
        if v.r2000() {
            dict(w, fixed::LAYOUTS, fixed::ROOT, &[("Layout1", fixed::PAPER_LAYOUT), ("Model", fixed::MODEL_LAYOUT)]);
            w.object("ACDBDICTIONARYWDFLT", fixed::PLOT_STYLES, fixed::ROOT, "AcDbDictionary");
            w.i(281, 1);
            w.s(3, "Normal");
            w.h(350, fixed::PLOT_NORMAL);
            w.s(100, "AcDbDictionaryWithDefault");
            w.h(340, fixed::PLOT_NORMAL);
            // A placeholder has no data and no subclass marker of its own.
            w.s(0, "ACDBPLACEHOLDER");
            w.h(5, fixed::PLOT_NORMAL);
            w.h(330, fixed::PLOT_STYLES);
            self.layout(w, fixed::MODEL_LAYOUT, "Model", 0, fixed::MODEL_RECORD);
            self.layout(w, fixed::PAPER_LAYOUT, "Layout1", 1, fixed::PAPER_RECORD);
        }
        if let Some((d, handles)) = mleader_h {
            let entries: Vec<(&str, u64)> = self.mleader_styles.iter().map(|(n, _)| n.as_str()).zip(handles.iter().copied()).collect();
            dict(w, *d, fixed::ROOT, &entries);
            for ((_, s), h) in self.mleader_styles.iter().zip(handles) {
                w.mleader_style(s, *h, *d);
            }
        }
        if let Some((d, handles)) = table_h {
            let entries: Vec<(&str, u64)> = self.table_styles.iter().map(|(n, _)| n.as_str()).zip(handles.iter().copied()).collect();
            dict(w, *d, fixed::ROOT, &entries);
            for ((_, s), h) in self.table_styles.iter().zip(handles) {
                w.table_style(s, *h, *d);
            }
        }
        w.t.extend(self.raw_objects.iter().cloned());
        w.end_section();
    }

    /// A layout (R2000 and later): its plot settings, then the layout itself.
    fn layout(&self, w: &mut W, h: u64, name: &str, tab: i64, record: u64) {
        let [x0, y0, x1, y1] = self.extents;
        let [lx0, ly0, lx1, ly1] = self.limits;
        w.object("LAYOUT", h, fixed::LAYOUTS, "AcDbPlotSettings");
        w.s(1, "");
        w.s(2, "none_device");
        w.s(4, "");
        w.s(6, "");
        for code in 40..=49 {
            w.f(code, 0.0);
        }
        w.f(140, 0.0);
        w.f(141, 0.0);
        w.f(142, 1.0);
        w.f(143, 1.0);
        // Plot flags; 1024 marks the model space layout.
        w.i(70, if record == fixed::MODEL_RECORD { 688 | 1024 } else { 688 });
        w.i(72, 1);
        w.i(73, 0);
        w.i(74, 5);
        w.s(7, "");
        w.i(75, 16);
        if self.format.true_color() {
            // Shade plot mode (as displayed), resolution level (normal) and custom DPI.
            w.i(76, 0);
            w.i(77, 2);
            w.i(78, 300);
        }
        w.f(147, 1.0);
        // The paper image origin is 148 (x) and 149 (y), not a point at 148 and 158.
        w.f(148, 0.0);
        w.f(149, 0.0);
        w.s(100, "AcDbLayout");
        w.s(1, name);
        w.i(70, 1);
        w.i(71, tab);
        w.xy(10, [lx0, ly0]);
        w.xy(11, [lx1, ly1]);
        w.p(12, [0.0; 3]);
        w.p(14, [x0, y0, 0.0]);
        w.p(15, [x1, y1, 0.0]);
        w.f(146, 0.0);
        w.p(13, [0.0; 3]);
        w.p(16, [1.0, 0.0, 0.0]);
        w.p(17, [0.0, 1.0, 0.0]);
        w.i(76, 1);
        w.h(330, record);
    }
}

fn block_begin(w: &mut W, name: &str, base: Point, description: &str, h: u64, record: u64) {
    w.object("BLOCK", h, record, "AcDbEntity");
    w.s(8, "0");
    w.sub("AcDbBlockBegin");
    w.s(2, name);
    w.i(70, 0);
    w.p(10, base);
    w.s(3, name);
    w.s(1, "");
    if !description.is_empty() && w.v.handles() {
        w.s(4, description);
    }
}

fn block_end(w: &mut W, h: u64, record: u64) {
    w.object("ENDBLK", h, record, "AcDbEntity");
    w.s(8, "0");
    w.sub("AcDbBlockEnd");
}

/// Group codes whose values are handles of other objects (DXF Reference, group code ranges):
/// soft and hard pointers and owners (320–369), more of them (390–399), 480 and 481, and the
/// extended data handle (1005).
pub fn is_pointer_code(code: i32) -> bool {
    matches!(code, 320..=369 | 390..=399 | 480..=481 | 1005)
}

/// The tags of records kept from another file, for `raw_entities` or `raw_objects`, with every
/// handle renumbered by `map`: each record's own handle (group 5, or 105 of a DIMSTYLE) and
/// every pointer ([`is_pointer_code`]). A handle `map` gives nothing for is written as the null
/// handle `0`, so a kept record never points at an object the new file gives another meaning;
/// a record's own handle that `map` drops keeps its group with `0`. Groups inside an
/// application's own `102 {` ... `102 }` group are pointers too (reactors, extension
/// dictionaries) and are renumbered the same way. The null handle stays null, and text that is
/// no handle is left as it is.
///
/// From CADCraft's `cadcraft-io` `dxf_raw.rs` (5a6aa9b6), which renumbers what it keeps this way.
pub fn remap_handles(tags: &[Tag], map: impl Fn(u64) -> Option<u64>) -> Vec<Tag> {
    let mut kind = "";
    tags.iter()
        .map(|t| {
            if t.code == 0 {
                kind = t.str_ref().map_or("", str::trim);
                return t.clone();
            }
            let own = t.code == 5 || (t.code == 105 && kind == "DIMSTYLE");
            if !own && !is_pointer_code(t.code) {
                return t.clone();
            }
            match super::parse_handle(&t.str()) {
                Some(0) | None => t.clone(),
                Some(h) => Tag::s(t.code, format!("{:X}", map(h).unwrap_or(0))),
            }
        })
        .collect()
}
