//! Incremental ASCII DXF file writer used by VectorCraft.
//!
//! This is the file skeleton around a pre-written `ENTITIES` section: group codes, handles, and
//! the HEADER / TABLES / BLOCKS / OBJECTS sections CAD apps expect. It is a different API from
//! [`crate::model::write`], which serializes a parsed [`crate::model`] drawing as tags.
//!
//! Moved from `vectorcraft-cad` (`crates/cad/src/writer.rs`). The bytes a [`Drawing`] writes must
//! stay the same: VectorCraft's export tests treat this as the format oracle.
//!
//! DXF R13 and later give every object a handle and subclass markers; R2000 and later add layouts
//! and plot styles.

use std::fmt::Write as _;

pub use crate::model::write::Format;

/// An object handle (written in hex).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Handle(pub u32);

/// The handles of the fixed objects every drawing has. Entities and table entries count up from
/// `FIRST_FREE`.
pub mod fixed {
    use super::Handle;
    pub const LAYER_TABLE: Handle = Handle(0x1);
    pub const LTYPE_TABLE: Handle = Handle(0x2);
    pub const APPID_TABLE: Handle = Handle(0x3);
    pub const DIMSTYLE_TABLE: Handle = Handle(0x4);
    pub const STYLE_TABLE: Handle = Handle(0x5);
    pub const UCS_TABLE: Handle = Handle(0x6);
    pub const VIEW_TABLE: Handle = Handle(0x7);
    pub const VPORT_TABLE: Handle = Handle(0x8);
    pub const BLOCK_RECORD_TABLE: Handle = Handle(0x9);
    pub const ROOT: Handle = Handle(0xA);
    pub const GROUPS: Handle = Handle(0xB);
    pub const LAYOUTS: Handle = Handle(0xC);
    pub const MODEL_RECORD: Handle = Handle(0xD);
    pub const PAPER_RECORD: Handle = Handle(0xE);
    pub const MODEL_BLOCK: Handle = Handle(0xF);
    pub const MODEL_END: Handle = Handle(0x10);
    pub const PAPER_BLOCK: Handle = Handle(0x11);
    pub const PAPER_END: Handle = Handle(0x12);
    pub const MODEL_LAYOUT: Handle = Handle(0x13);
    pub const PAPER_LAYOUT: Handle = Handle(0x14);
    pub const ACTIVE_VPORT: Handle = Handle(0x15);
    pub const ACAD_APPID: Handle = Handle(0x16);
    pub const STANDARD_DIMSTYLE: Handle = Handle(0x17);
    pub const IMAGE_DICT: Handle = Handle(0x18);
    pub const IMAGE_VARS: Handle = Handle(0x19);
    pub const PLOT_STYLES: Handle = Handle(0x1A);
    pub const PLOT_NORMAL: Handle = Handle(0x1B);
    pub const FIRST_FREE: u32 = 0x30;
}

/// A number as DXF writes it: up to 6 decimals, no trailing zeros, 0 for anything not finite.
pub fn num(v: f64) -> String {
    if !v.is_finite() {
        return "0".into();
    }
    let s = format!("{v:.6}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" || s.is_empty() { "0".into() } else { s.into() }
}

/// Text as a DXF string value: one line, `^` escaped, and before DXF 2007 (which reads UTF-8)
/// every character outside ASCII as `\U+XXXX`.
pub fn text(s: &str, utf8: bool) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\r' | '\n' | '\t' => out.push(' '),
            c if c.is_control() => {}
            '^' => out.push_str("^ "),
            c if !utf8 && !c.is_ascii() => match u16::try_from(u32::from(c)) {
                Ok(u) => _ = write!(out, "\\U+{u:04X}"),
                Err(_) => out.push('?'),
            },
            c => out.push(c),
        }
    }
    out
}

/// Group-code/value pairs in the order they are written.
pub struct Codes {
    pub v: Format,
    out: String,
}

impl Codes {
    pub fn new(v: Format) -> Self {
        Self { v, out: String::new() }
    }

    pub fn into_string(self) -> String {
        self.out
    }

    pub fn append(&mut self, other: &str) {
        self.out.push_str(other);
    }

    /// A value written as it is (names and keywords the writer itself chose).
    pub fn raw(&mut self, code: u16, value: &str) {
        _ = writeln!(self.out, "{code:>3}\n{value}");
    }

    /// Text from the document, encoded for the version (see [`text`]).
    pub fn str(&mut self, code: u16, value: &str) {
        let t = text(value, self.v.utf8());
        self.raw(code, &t);
    }

    pub fn int(&mut self, code: u16, value: i64) {
        _ = writeln!(self.out, "{code:>3}\n{value}");
    }

    pub fn num(&mut self, code: u16, value: f64) {
        _ = writeln!(self.out, "{code:>3}\n{}", num(value));
    }

    pub fn handle(&mut self, code: u16, h: Handle) {
        _ = writeln!(self.out, "{code:>3}\n{:X}", h.0);
    }

    /// A 2D point at `code` (x), `code + 10` (y).
    pub fn xy(&mut self, code: u16, x: f64, y: f64) {
        self.num(code, x);
        self.num(code + 10, y);
    }

    /// A 3D point at `code`, `code + 10`, `code + 20` (z = 0).
    pub fn xyz(&mut self, code: u16, x: f64, y: f64) {
        self.xy(code, x, y);
        self.num(code + 20, 0.0);
    }

    /// The extrusion direction of planar entities: the z axis.
    pub fn normal(&mut self) {
        self.num(210, 0.0);
        self.num(220, 0.0);
        self.num(230, 1.0);
    }

    /// The start of an object: its type, and from R13 its handle, owner and first subclass.
    fn object(&mut self, kind: &str, h: Handle, owner: Handle, subclass: &str) {
        self.raw(0, kind);
        if self.v.handles() {
            self.handle(if kind == "DIMSTYLE" { 105 } else { 5 }, h);
            self.handle(330, owner);
            self.raw(100, subclass);
        }
    }

    /// A table entry: the shared record subclass, then the table's own (from R13).
    fn record(&mut self, kind: &str, h: Handle, table: Handle, subclass: &str) {
        self.object(kind, h, table, "AcDbSymbolTableRecord");
        if self.v.handles() {
            self.raw(100, subclass);
        }
    }

    fn section(&mut self, name: &str) {
        self.raw(0, "SECTION");
        self.raw(2, name);
    }

    fn end_section(&mut self) {
        self.raw(0, "ENDSEC");
    }

    fn table(&mut self, name: &str, h: Handle, count: usize) {
        self.raw(0, "TABLE");
        self.raw(2, name);
        if self.v.handles() {
            self.handle(5, h);
            self.raw(330, "0");
            self.raw(100, "AcDbSymbolTable");
        }
        self.int(70, count as i64);
        if name == "DIMSTYLE" && self.v.r2000() {
            self.raw(100, "AcDbDimStyleTable");
        }
    }

    fn end_table(&mut self) {
        self.raw(0, "ENDTAB");
    }
}

/// How an entity is drawn: its colour index, true colour, lineweight, linetype and transparency.
#[derive(Clone, Debug, Default)]
pub struct EntityStyle {
    pub aci: u8,
    /// `0xRRGGBB` (DXF 2004 and later).
    pub rgb: Option<u32>,
    /// Hundredths of a millimetre (DXF 2000 and later).
    pub lineweight: Option<i32>,
    pub linetype: Option<String>,
    /// Opacity 0–255 (DXF 2004 and later; opaque is left out).
    pub alpha: Option<u8>,
}

impl Codes {
    /// The start of an entity of `kind` owned by `owner` (model space, or a polyline for its
    /// vertices) on `layer` with its style, ending with `subclass` (none when empty).
    pub fn entity(&mut self, kind: &str, h: Handle, owner: Handle, layer: &str, style: &EntityStyle, subclass: &str) {
        self.object(kind, h, owner, "AcDbEntity");
        self.str(8, layer);
        if let Some(lt) = &style.linetype {
            self.str(6, lt);
        }
        self.int(62, i64::from(style.aci));
        if self.v.r2000()
            && let Some(lw) = style.lineweight
        {
            self.int(370, i64::from(lw));
        }
        if self.v.true_color() {
            if let Some(rgb) = style.rgb {
                self.int(420, i64::from(rgb));
            }
            if let Some(a) = style.alpha {
                self.int(440, 0x0200_0000 | i64::from(a));
            }
        }
        if self.v.handles() && !subclass.is_empty() {
            self.raw(100, subclass);
        }
    }
}

/// A DXF layer.
#[derive(Clone, Debug)]
pub struct Layer {
    pub name: String,
    pub aci: u8,
    /// Switched off (a hidden layer).
    pub off: bool,
    pub locked: bool,
    /// Plotted (printable).
    pub plot: bool,
}

/// A dash pattern: dash, gap, dash… lengths in drawing units.
#[derive(Clone, Debug)]
pub struct LineType {
    pub name: String,
    pub pattern: Vec<f64>,
}

/// A placed image file and the image entities that show it.
#[derive(Clone, Debug)]
pub struct ImageDef {
    pub name: String,
    pub size: (u32, u32),
    pub handle: Handle,
    /// `(reactor, image entity)` per image entity.
    pub reactors: Vec<(Handle, Handle)>,
}

/// Everything around the entities, collected while they were written.
pub struct Drawing {
    pub version: Format,
    pub layers: Vec<Layer>,
    pub linetypes: Vec<LineType>,
    /// Text styles: (style name, font family).
    pub styles: Vec<(String, String)>,
    pub images: Vec<ImageDef>,
    /// The entities' group codes.
    pub entities: String,
    /// `[x0, y0, x1, y1]` of everything written (drawing units).
    pub extents: [f64; 4],
    /// The export region (drawing units).
    pub limits: [f64; 4],
    /// `$INSUNITS`.
    pub insunits: u8,
    pub metric: bool,
    /// The next free handle.
    pub next: u32,
}

/// The text style every drawing has.
pub const STANDARD_STYLE: &str = "Standard";

impl Drawing {
    /// The whole file.
    pub fn write(self) -> String {
        let v = self.version;
        let mut c = Codes::new(v);
        // Table entries get the handles after the entities'.
        let mut next = self.next;
        let mut take = |n: usize| {
            let start = next;
            next = next.saturating_add(u32::try_from(n).unwrap_or(u32::MAX));
            start
        };
        let ltype_base = take(3 + self.linetypes.len());
        let layer_base = take(1 + self.layers.len());
        let style_base = take(1 + self.styles.len());
        self.header(&mut c, next);
        if v.handles() {
            self.classes(&mut c);
        }
        c.section("TABLES");
        self.vport(&mut c);
        self.ltypes(&mut c, ltype_base);
        self.layer_table(&mut c, layer_base);
        self.style_table(&mut c, style_base);
        if v.handles() {
            for (name, h) in [("VIEW", fixed::VIEW_TABLE), ("UCS", fixed::UCS_TABLE)] {
                c.table(name, h, 0);
                c.end_table();
            }
            c.table("APPID", fixed::APPID_TABLE, 1);
            c.record("APPID", fixed::ACAD_APPID, fixed::APPID_TABLE, "AcDbRegAppTableRecord");
            c.raw(2, "ACAD");
            c.int(70, 0);
            c.end_table();
            c.table("DIMSTYLE", fixed::DIMSTYLE_TABLE, 1);
            c.record("DIMSTYLE", fixed::STANDARD_DIMSTYLE, fixed::DIMSTYLE_TABLE, "AcDbDimStyleTableRecord");
            c.raw(2, "Standard");
            c.int(70, 0);
            c.end_table();
            c.table("BLOCK_RECORD", fixed::BLOCK_RECORD_TABLE, 2);
            for (name, h, layout) in
                [("*Model_Space", fixed::MODEL_RECORD, fixed::MODEL_LAYOUT), ("*Paper_Space", fixed::PAPER_RECORD, fixed::PAPER_LAYOUT)]
            {
                c.record("BLOCK_RECORD", h, fixed::BLOCK_RECORD_TABLE, "AcDbBlockTableRecord");
                c.raw(2, name);
                if v.r2000() {
                    c.handle(340, layout);
                    c.int(70, 0);
                    c.int(280, 1);
                    c.int(281, 0);
                }
            }
            c.end_table();
        }
        c.end_section();
        if v.handles() {
            c.section("BLOCKS");
            for (name, record, begin, end) in [
                ("*Model_Space", fixed::MODEL_RECORD, fixed::MODEL_BLOCK, fixed::MODEL_END),
                ("*Paper_Space", fixed::PAPER_RECORD, fixed::PAPER_BLOCK, fixed::PAPER_END),
            ] {
                c.object("BLOCK", begin, record, "AcDbEntity");
                c.raw(8, "0");
                c.raw(100, "AcDbBlockBegin");
                c.raw(2, name);
                c.int(70, 0);
                c.xyz(10, 0.0, 0.0);
                c.raw(3, name);
                c.raw(1, "");
                c.object("ENDBLK", end, record, "AcDbEntity");
                c.raw(8, "0");
                c.raw(100, "AcDbBlockEnd");
            }
            c.end_section();
        }
        c.section("ENTITIES");
        c.append(&self.entities);
        c.end_section();
        if v.handles() {
            self.objects(&mut c);
        }
        c.raw(0, "EOF");
        c.into_string()
    }

    fn header(&self, c: &mut Codes, handseed: u32) {
        let v = self.version;
        let var = |c: &mut Codes, name: &str| c.raw(9, name);
        c.section("HEADER");
        var(c, "$ACADVER");
        c.raw(1, v.acadver());
        var(c, "$DWGCODEPAGE");
        c.raw(3, "ANSI_1252");
        var(c, "$INSBASE");
        c.xyz(10, 0.0, 0.0);
        let [x0, y0, x1, y1] = self.extents;
        var(c, "$EXTMIN");
        c.xyz(10, x0, y0);
        var(c, "$EXTMAX");
        c.xyz(10, x1, y1);
        let [lx0, ly0, lx1, ly1] = self.limits;
        var(c, "$LIMMIN");
        c.xy(10, lx0, ly0);
        var(c, "$LIMMAX");
        c.xy(10, lx1, ly1);
        if v.r2000() {
            var(c, "$LWDISPLAY");
            c.int(290, 1);
            var(c, "$INSUNITS");
            c.int(70, i64::from(self.insunits));
            var(c, "$MEASUREMENT");
            c.int(70, i64::from(self.metric));
        }
        if v.handles() {
            var(c, "$HANDSEED");
            c.handle(5, Handle(handseed));
        }
        c.end_section();
    }

    fn classes(&self, c: &mut Codes) {
        let mut classes: Vec<(&str, &str, &str, i64, bool)> = vec![];
        if self.version.r2000() {
            classes.extend([
                ("ACDBDICTIONARYWDFLT", "AcDbDictionaryWithDefault", "ObjectDBX Classes", 0, false),
                ("ACDBPLACEHOLDER", "AcDbPlaceHolder", "ObjectDBX Classes", 0, false),
                ("LAYOUT", "AcDbLayout", "ObjectDBX Classes", 0, false),
            ]);
        }
        if !self.images.is_empty() {
            classes.extend([
                ("RASTERVARIABLES", "AcDbRasterVariables", "ISM", 0, false),
                ("IMAGE", "AcDbRasterImage", "ISM", 2175, true),
                ("IMAGEDEF", "AcDbRasterImageDef", "ISM", 0, false),
                ("IMAGEDEF_REACTOR", "AcDbRasterImageDefReactor", "ISM", 1, false),
            ]);
        }
        c.section("CLASSES");
        for (name, cpp, app, flags, entity) in classes {
            c.raw(0, "CLASS");
            c.raw(1, name);
            c.raw(2, cpp);
            c.raw(3, app);
            c.int(90, flags);
            if self.version.true_color() {
                c.int(91, 0);
            }
            c.int(280, 0);
            c.int(281, i64::from(entity));
        }
        c.end_section();
    }

    /// The active viewport, zoomed to the drawing's extents.
    fn vport(&self, c: &mut Codes) {
        let [x0, y0, x1, y1] = self.extents;
        c.table("VPORT", fixed::VPORT_TABLE, 1);
        c.record("VPORT", fixed::ACTIVE_VPORT, fixed::VPORT_TABLE, "AcDbViewportTableRecord");
        c.raw(2, if self.version.handles() { "*Active" } else { "*ACTIVE" });
        c.int(70, 0);
        c.xy(10, 0.0, 0.0);
        c.xy(11, 1.0, 1.0);
        c.xy(12, (x0 + x1) / 2.0, (y0 + y1) / 2.0);
        c.xy(13, 0.0, 0.0);
        c.xy(14, 1.0, 1.0);
        c.xy(15, 10.0, 10.0);
        // The view direction looks down the z axis: (0, 0, 1), each code once.
        c.xy(16, 0.0, 0.0);
        c.num(36, 1.0);
        c.xyz(17, 0.0, 0.0);
        let (w, h) = ((x1 - x0).abs(), (y1 - y0).abs());
        // The view's height, with a margin; its width follows from the aspect ratio.
        c.num(40, (h.max(w / 1.5) * 1.1).max(1.0));
        c.num(41, 1.5);
        c.num(42, 50.0);
        c.num(43, 0.0);
        c.num(44, 0.0);
        c.num(50, 0.0);
        c.num(51, 0.0);
        for (code, value) in [(71, 0), (72, 1000), (73, 1), (74, 3), (75, 0), (76, 0), (77, 0), (78, 0)] {
            c.int(code, value);
        }
        if self.version.r2000() {
            c.int(281, 0);
            c.int(65, 0);
            c.num(146, 0.0);
        }
        c.end_table();
    }

    fn ltypes(&self, c: &mut Codes, base: u32) {
        c.table("LTYPE", fixed::LTYPE_TABLE, 3 + self.linetypes.len());
        let solid = [("ByBlock", ""), ("ByLayer", ""), ("Continuous", "Solid line")];
        let rows = solid.iter().map(|(n, d)| (*n, *d, &[][..])).chain(self.linetypes.iter().map(|l| (l.name.as_str(), "", l.pattern.as_slice())));
        for (i, (name, desc, pattern)) in rows.enumerate() {
            c.record("LTYPE", Handle(base.saturating_add(i as u32)), fixed::LTYPE_TABLE, "AcDbLinetypeTableRecord");
            c.str(2, name);
            c.int(70, 0);
            c.str(3, if pattern.is_empty() { desc } else { "Dashed" });
            c.int(72, 65);
            c.int(73, pattern.len() as i64);
            c.num(40, pattern.iter().map(|v| v.abs()).sum());
            for v in pattern {
                c.num(49, *v);
                if self.version.handles() {
                    c.int(74, 0);
                }
            }
        }
        c.end_table();
    }

    fn layer_table(&self, c: &mut Codes, base: u32) {
        let zero = Layer { name: "0".into(), aci: 7, off: false, locked: false, plot: true };
        c.table("LAYER", fixed::LAYER_TABLE, 1 + self.layers.len());
        for (i, l) in std::iter::once(&zero).chain(&self.layers).enumerate() {
            c.record("LAYER", Handle(base.saturating_add(i as u32)), fixed::LAYER_TABLE, "AcDbLayerTableRecord");
            c.str(2, &l.name);
            c.int(70, if l.locked { 4 } else { 0 });
            // A negative colour switches the layer off.
            let aci = i64::from(l.aci.max(1));
            c.int(62, if l.off { -aci } else { aci });
            c.raw(6, "Continuous");
            if self.version.r2000() {
                c.int(290, i64::from(l.plot));
                c.int(370, -3);
                c.handle(390, fixed::PLOT_NORMAL);
            }
        }
        c.end_table();
    }

    fn style_table(&self, c: &mut Codes, base: u32) {
        c.table("STYLE", fixed::STYLE_TABLE, 1 + self.styles.len());
        let standard = (STANDARD_STYLE.to_string(), String::new());
        for (i, (name, family)) in std::iter::once(&standard).chain(&self.styles).enumerate() {
            c.record("STYLE", Handle(base.saturating_add(i as u32)), fixed::STYLE_TABLE, "AcDbTextStyleTableRecord");
            c.str(2, name);
            c.int(70, 0);
            c.num(40, 0.0);
            c.num(41, 1.0);
            c.num(50, 0.0);
            c.int(71, 0);
            c.num(42, 2.5);
            // The font file; from 2000 the family name too, which is how CAD apps find a TrueType font.
            if family.is_empty() {
                c.raw(3, "txt");
            } else {
                c.str(3, &format!("{family}.ttf"));
            }
            c.raw(4, "");
            if self.version.r2000() && !family.is_empty() {
                c.raw(1001, "ACAD");
                c.str(1000, family);
                c.int(1071, 0);
            }
        }
        c.end_table();
    }

    fn objects(&self, c: &mut Codes) {
        let v = self.version;
        let images = !self.images.is_empty();
        c.section("OBJECTS");
        let dict = |c: &mut Codes, h: Handle, owner: Handle, entries: &[(&str, Handle)]| {
            c.object("DICTIONARY", h, owner, "AcDbDictionary");
            c.int(281, 1);
            for (name, e) in entries {
                c.str(3, name);
                c.handle(350, *e);
            }
        };
        let mut root = vec![("ACAD_GROUP", fixed::GROUPS)];
        if images {
            root.extend([("ACAD_IMAGE_DICT", fixed::IMAGE_DICT), ("ACAD_IMAGE_VARS", fixed::IMAGE_VARS)]);
        }
        if v.r2000() {
            root.extend([("ACAD_LAYOUT", fixed::LAYOUTS), ("ACAD_PLOTSTYLENAME", fixed::PLOT_STYLES)]);
        }
        dict(c, fixed::ROOT, Handle(0), &root);
        dict(c, fixed::GROUPS, fixed::ROOT, &[]);
        if v.r2000() {
            dict(c, fixed::LAYOUTS, fixed::ROOT, &[("Layout1", fixed::PAPER_LAYOUT), ("Model", fixed::MODEL_LAYOUT)]);
            c.object("ACDBDICTIONARYWDFLT", fixed::PLOT_STYLES, fixed::ROOT, "AcDbDictionary");
            c.int(281, 1);
            c.raw(3, "Normal");
            c.handle(350, fixed::PLOT_NORMAL);
            c.raw(100, "AcDbDictionaryWithDefault");
            c.handle(340, fixed::PLOT_NORMAL);
            // A placeholder has no data and no subclass marker of its own.
            c.raw(0, "ACDBPLACEHOLDER");
            c.handle(5, fixed::PLOT_NORMAL);
            c.handle(330, fixed::PLOT_STYLES);
            self.layout(c, fixed::MODEL_LAYOUT, "Model", 0, fixed::MODEL_RECORD);
            self.layout(c, fixed::PAPER_LAYOUT, "Layout1", 1, fixed::PAPER_RECORD);
        }
        if images {
            c.object("RASTERVARIABLES", fixed::IMAGE_VARS, fixed::ROOT, "AcDbRasterVariables");
            c.int(90, 0);
            c.int(70, 0);
            c.int(71, 1);
            c.int(72, 0);
            let keys: Vec<String> = self.images.iter().map(|i| format!("IMAGE_{:X}", i.handle.0)).collect();
            let entries: Vec<(&str, Handle)> = keys.iter().zip(&self.images).map(|(key, image)| (key.as_str(), image.handle)).collect();
            dict(c, fixed::IMAGE_DICT, fixed::ROOT, &entries);
            for img in &self.images {
                c.raw(0, "IMAGEDEF");
                c.handle(5, img.handle);
                c.raw(102, "{ACAD_REACTORS");
                c.handle(330, fixed::IMAGE_DICT);
                for (reactor, _) in &img.reactors {
                    c.handle(330, *reactor);
                }
                c.raw(102, "}");
                c.handle(330, fixed::IMAGE_DICT);
                c.raw(100, "AcDbRasterImageDef");
                c.int(90, 0);
                c.str(1, &img.name);
                c.xy(10, f64::from(img.size.0), f64::from(img.size.1));
                c.xy(11, 1.0, 1.0);
                c.int(280, 1);
                c.int(281, 0);
                for (reactor, image) in &img.reactors {
                    c.object("IMAGEDEF_REACTOR", *reactor, *image, "AcDbRasterImageDefReactor");
                    c.int(90, 2);
                    c.handle(330, *image);
                }
            }
        }
        c.end_section();
    }

    /// A layout (DXF 2000 and later): its plot settings, then the layout itself.
    fn layout(&self, c: &mut Codes, h: Handle, name: &str, tab: i64, record: Handle) {
        let [x0, y0, x1, y1] = self.extents;
        c.object("LAYOUT", h, fixed::LAYOUTS, "AcDbPlotSettings");
        c.raw(1, "");
        c.raw(2, "none_device");
        c.raw(4, "");
        c.raw(6, "");
        for code in 40..=49 {
            c.num(code, 0.0);
        }
        c.num(140, 0.0);
        c.num(141, 0.0);
        c.num(142, 1.0);
        c.num(143, 1.0);
        // Plot flags; 1024 marks the model space layout.
        c.int(70, if record == fixed::MODEL_RECORD { 688 | 1024 } else { 688 });
        c.int(72, 1);
        c.int(73, 0);
        c.int(74, 5);
        c.raw(7, "");
        c.int(75, 16);
        if self.version.true_color() {
            // Shade plot mode (as displayed), resolution level (normal) and custom DPI.
            c.int(76, 0);
            c.int(77, 2);
            c.int(78, 300);
        }
        c.num(147, 1.0);
        // The paper image origin is 148 (x) and 149 (y), not a point at 148 and 158.
        c.num(148, 0.0);
        c.num(149, 0.0);
        c.raw(100, "AcDbLayout");
        c.raw(1, name);
        c.int(70, 1);
        c.int(71, tab);
        let [lx0, ly0, lx1, ly1] = self.limits;
        c.xy(10, lx0, ly0);
        c.xy(11, lx1, ly1);
        c.xyz(12, 0.0, 0.0);
        c.xyz(14, x0, y0);
        c.xyz(15, x1, y1);
        c.num(146, 0.0);
        c.xyz(13, 0.0, 0.0);
        c.xyz(16, 1.0, 0.0);
        c.xyz(17, 0.0, 1.0);
        c.int(76, 1);
        c.handle(330, record);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_and_text_are_written_plainly() {
        assert_eq!(num(1.5), "1.5");
        assert_eq!(num(-0.0000001), "0");
        assert_eq!(num(2.0), "2");
        assert_eq!(num(f64::NAN), "0");
        assert_eq!(text("a^b\nc", true), "a^ b c");
        assert_eq!(text("Größe 😀", false), "Gr\\U+00F6\\U+00DFe ?");
        assert_eq!(text("Größe", true), "Größe");
    }

    /// A line from (0, 0) to (10, 20) on layer `Layer1`, in the style scene.rs writes entities.
    fn line_entities(v: Format) -> String {
        let mut e = Codes::new(v);
        let style = EntityStyle { aci: 1, rgb: Some(0xFF0000), lineweight: Some(25), linetype: None, alpha: None };
        e.entity("LINE", Handle(fixed::FIRST_FREE), fixed::MODEL_RECORD, "Layer1", &style, "AcDbLine");
        e.xyz(10, 0.0, 0.0);
        e.xyz(11, 10.0, 20.0);
        e.into_string()
    }

    fn sample(v: Format, with_line: bool) -> Drawing {
        Drawing {
            version: v,
            layers: vec![Layer { name: "Layer1".into(), aci: 1, off: false, locked: false, plot: true }],
            linetypes: vec![],
            styles: vec![],
            images: vec![],
            entities: if with_line { line_entities(v) } else { String::new() },
            extents: [0.0, 0.0, 200.0, 100.0],
            limits: [0.0, 0.0, 200.0, 100.0],
            insunits: 4,
            metric: true,
            next: fixed::FIRST_FREE + if with_line { 1 } else { 0 },
        }
    }

    fn assert_golden(name: &str, got: &str) {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data").join(name);
        if std::env::var_os("WRITE_DXF_GOLDENS").is_some() {
            std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
            std::fs::write(&path, got).expect("write golden");
            return;
        }
        let expected = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        assert_eq!(got, expected, "bytes changed versus {}", path.display());
    }

    use std::path::PathBuf;

    #[test]
    fn empty_and_line_drawings_match_goldens() {
        for (name, v, line) in [
            ("drawing_empty_r12.dxf", Format::R12, false),
            ("drawing_empty_r2018.dxf", Format::R2018, false),
            ("drawing_line_r12.dxf", Format::R12, true),
            ("drawing_line_r2000.dxf", Format::R2000, true),
            ("drawing_line_r2018.dxf", Format::R2018, true),
        ] {
            assert_golden(name, &sample(v, line).write());
        }
    }

    #[test]
    fn entity_preserves_byblock_colour() {
        for v in Format::ALL {
            let mut c = Codes::new(v);
            c.entity("LINE", Handle(fixed::FIRST_FREE), fixed::MODEL_RECORD, "0", &EntityStyle::default(), "AcDbLine");
            assert!(c.into_string().contains(" 62\n0\n"), "{v:?}");
        }
    }

    #[test]
    fn image_dictionary_keys_are_independent_of_filenames() {
        let mut drawing = sample(Format::R2018, false);
        for (index, name) in [r"C:\images\scan,1.png", "scan,1.png"].into_iter().enumerate() {
            drawing.images.push(ImageDef { name: name.into(), size: (10, 20), handle: Handle(fixed::FIRST_FREE + index as u32), reactors: vec![] });
        }
        drawing.next = fixed::FIRST_FREE + 2;
        let images = drawing.images.clone();
        let output = drawing.write();
        for image in &images {
            assert!(output.contains(&format!("  3\nIMAGE_{:X}\n350\n{:X}\n", image.handle.0, image.handle.0)));
            assert!(output.contains(&format!("  1\n{}\n", image.name)));
            assert!(!output.contains(&format!("  3\n{}\n", image.name)));
        }
    }

    #[test]
    fn every_format_writes_its_acadver_and_ends_at_eof() {
        for v in Format::ALL {
            let s = sample(v, true).write();
            assert!(s.contains(&format!("  1\n{}\n", v.acadver())), "{v:?}");
            assert!(s.ends_with("  0\nEOF\n"), "{v:?}");
            if v.handles() {
                assert!(s.contains("  0\nSECTION\n  2\nCLASSES\n"), "{v:?}");
            } else {
                assert!(!s.contains("\nCLASSES\n"), "{v:?}");
            }
        }
    }
}
