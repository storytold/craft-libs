//! The styles part (`xl/styles.xml`, ECMA-376 Part 1, 18.8) as the file has it: tables that
//! cells refer to by index. Attributes a file may leave out are `Option`s, because a
//! differential format (`<dxf>`) states only what it changes.

use craft_sheet::{BorderStyle, DEFAULT_PALETTE, HAlign, Pattern, Script, Underline, VAlign, builtin_format};
use craft_xml::El;

use crate::write::{flag, list, num, set};

/// A value's spelling, by table: the first entry for a value is the one written.
fn spelled<T: PartialEq + Copy>(table: &[(&'static str, T)], v: T) -> Option<&'static str> {
    table.iter().find(|(_, t)| *t == v).map(|(n, _)| *n)
}
fn spelling<T: Copy>(table: &[(&str, T)], s: &str) -> Option<T> {
    table.iter().find(|(n, _)| *n == s).map(|(_, t)| *t)
}

const UNDERLINES: &[(&str, Underline)] = &[
    ("single", Underline::Single),
    ("double", Underline::Double),
    ("singleAccounting", Underline::SingleAccounting),
    ("doubleAccounting", Underline::DoubleAccounting),
    ("none", Underline::None),
];
const SCRIPTS: &[(&str, Script)] = &[("superscript", Script::Superscript), ("subscript", Script::Subscript), ("baseline", Script::Baseline)];
const SCHEMES: &[(&str, FontScheme)] = &[("major", FontScheme::Major), ("minor", FontScheme::Minor), ("none", FontScheme::None)];
const PATTERNS: &[(&str, Pattern)] = &[
    ("solid", Pattern::Solid),
    ("mediumGray", Pattern::Gray50),
    ("darkGray", Pattern::Gray75),
    ("lightGray", Pattern::Gray25),
    ("gray125", Pattern::Gray125),
    ("gray0625", Pattern::Gray0625),
    ("darkHorizontal", Pattern::DarkHorizontal),
    ("darkVertical", Pattern::DarkVertical),
    ("darkDown", Pattern::DarkDown),
    ("darkUp", Pattern::DarkUp),
    ("darkGrid", Pattern::DarkGrid),
    ("darkTrellis", Pattern::DarkTrellis),
    ("lightHorizontal", Pattern::LightHorizontal),
    ("lightVertical", Pattern::LightVertical),
    ("lightDown", Pattern::LightDown),
    ("lightUp", Pattern::LightUp),
    ("lightGrid", Pattern::LightGrid),
    ("lightTrellis", Pattern::LightTrellis),
    ("none", Pattern::None),
];
const BORDER_STYLES: &[(&str, BorderStyle)] = &[
    ("thin", BorderStyle::Thin),
    ("medium", BorderStyle::Medium),
    ("thick", BorderStyle::Thick),
    ("dashed", BorderStyle::Dashed),
    ("dotted", BorderStyle::Dotted),
    ("double", BorderStyle::Double),
    ("hair", BorderStyle::Hair),
    ("mediumDashed", BorderStyle::MediumDashed),
    ("dashDot", BorderStyle::DashDot),
    ("mediumDashDot", BorderStyle::MediumDashDot),
    ("dashDotDot", BorderStyle::DashDotDot),
    ("mediumDashDotDot", BorderStyle::MediumDashDotDot),
    ("slantDashDot", BorderStyle::SlantDashDot),
    ("none", BorderStyle::None),
];
const HALIGNS: &[(&str, HAlign)] = &[
    ("left", HAlign::Left),
    ("center", HAlign::Center),
    ("right", HAlign::Right),
    ("fill", HAlign::Fill),
    ("justify", HAlign::Justify),
    ("centerContinuous", HAlign::CenterAcross),
    ("distributed", HAlign::Distributed),
    ("general", HAlign::General),
];
const VALIGNS: &[(&str, VAlign)] = &[
    ("top", VAlign::Top),
    ("center", VAlign::Center),
    ("justify", VAlign::Justify),
    ("distributed", VAlign::Distributed),
    ("bottom", VAlign::Bottom),
];

/// `<name/>` for true, `<name val="0"/>` for false.
fn bool_el(name: &str, on: bool) -> El {
    if on { El::new(name) } else { El::new(name).with_attr("val", "0") }
}

/// Characters of a font name kept (Excel allows 31; damaged files have far more).
const MAX_FONT_NAME_CHARS: usize = 255;

/// Where a colour comes from (`CT_Color`, 18.8.9).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ColorSource {
    /// `auto="1"`, or no attribute that names a colour.
    #[default]
    Auto,
    /// `rgb`: `0xAARRGGBB`. Six hex digits read as opaque.
    Rgb(u32),
    /// `theme`: an index into the theme's colour scheme.
    Theme(u32),
    /// `indexed`: an index into [`Styles::palette`].
    Indexed(u32),
}

/// A colour with its tint (`-1.0` darkest to `1.0` lightest, 0 for none).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Color {
    pub source: ColorSource,
    pub tint: f64,
}

impl Color {
    pub fn read(e: &El) -> Color {
        let source = if e.flag("auto", false) {
            ColorSource::Auto
        } else if let Some(rgb) = e.attr("rgb") {
            parse_argb(rgb).map_or(ColorSource::Auto, ColorSource::Rgb)
        } else if let Some(t) = e.attr_u32("theme") {
            ColorSource::Theme(t)
        } else if let Some(i) = e.attr_u32("indexed") {
            ColorSource::Indexed(i)
        } else {
            ColorSource::Auto
        };
        Color { source, tint: e.attr_f64("tint").map_or(0.0, |t| t.clamp(-1.0, 1.0)) }
    }

    /// `0xRRGGBB` for an RGB colour, or an indexed one in `palette`; `None` for automatic and
    /// theme colours. The tint is not applied.
    /// The colour as an element named `name` (`color`, `fgColor`, `tabColor`).
    pub fn to_el(&self, name: &str) -> El {
        let e = El::new(name);
        let mut e = match self.source {
            ColorSource::Auto => e.with_attr("auto", "1"),
            ColorSource::Rgb(v) => e.with_attr("rgb", format!("{v:08X}")),
            ColorSource::Theme(t) => e.with_attr("theme", t),
            ColorSource::Indexed(i) => e.with_attr("indexed", i),
        };
        set(&mut e, "tint", (self.tint != 0.0).then(|| num(self.tint)));
        e
    }

    pub fn rgb(&self, palette: &[u32]) -> Option<u32> {
        match self.source {
            ColorSource::Rgb(v) => Some(v & 0xFF_FFFF),
            ColorSource::Indexed(i) => palette.get(i as usize).copied(),
            ColorSource::Auto | ColorSource::Theme(_) => None,
        }
    }
}

fn parse_argb(s: &str) -> Option<u32> {
    let h = s.trim().trim_start_matches('#');
    let v = u32::from_str_radix(h, 16).ok().filter(|_| h.bytes().all(|b| b.is_ascii_hexdigit()))?;
    match h.len() {
        8 => Some(v),
        6 => Some(0xFF00_0000 | v),
        _ => None,
    }
}

/// `scheme` of a font: which theme font it follows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontScheme {
    None,
    Major,
    Minor,
}

/// A font (`<font>`, or a run's `<rPr>`, 18.8.22 and 18.4.7).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Font {
    pub name: Option<String>,
    /// Points.
    pub size: Option<f64>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub strike: Option<bool>,
    pub outline: Option<bool>,
    pub shadow: Option<bool>,
    pub condense: Option<bool>,
    pub extend: Option<bool>,
    pub underline: Option<Underline>,
    pub vert_align: Option<Script>,
    pub color: Option<Color>,
    pub family: Option<u32>,
    pub charset: Option<u32>,
    pub scheme: Option<FontScheme>,
}

impl Font {
    pub fn read(e: &El) -> Font {
        let flag = |n: &str| e.child(n).map(|c| c.flag("val", true));
        let val = |n: &str| e.child_attr(n, "val");
        Font {
            name: val("name").or_else(|| val("rFont")).map(|n| n.chars().take(MAX_FONT_NAME_CHARS).collect()),
            size: e.child("sz").and_then(|s| s.attr_f64("val")),
            bold: flag("b"),
            italic: flag("i"),
            strike: flag("strike"),
            outline: flag("outline"),
            shadow: flag("shadow"),
            condense: flag("condense"),
            extend: flag("extend"),
            underline: e.child("u").map(|u| spelling(UNDERLINES, u.attr("val").unwrap_or("single")).unwrap_or(Underline::Single)),
            vert_align: val("vertAlign").map(|v| spelling(SCRIPTS, v).unwrap_or(Script::Baseline)),
            color: e.child("color").map(Color::read),
            family: e.child("family").and_then(|c| c.attr_u32("val")),
            charset: e.child("charset").and_then(|c| c.attr_u32("val")),
            scheme: val("scheme").map(|s| spelling(SCHEMES, s).unwrap_or(FontScheme::None)),
        }
    }

    /// The font as `<name>`: `font` in the styles part, `rPr` in a rich-text run (`run`, which
    /// names the typeface `rFont`).
    pub fn to_el(&self, name: &str, run: bool) -> El {
        let mut e = El::new(name);
        let flags = [
            ("b", self.bold),
            ("i", self.italic),
            ("strike", self.strike),
            ("condense", self.condense),
            ("extend", self.extend),
            ("outline", self.outline),
            ("shadow", self.shadow),
        ];
        for (n, v) in flags {
            if let Some(v) = v {
                e.push(bool_el(n, v));
            }
        }
        let val = |n: &str, v: &str| El::new(n).with_attr("val", v);
        if let Some(u) = self.underline {
            e.push(if u == Underline::Single { El::new("u") } else { val("u", spelled(UNDERLINES, u).unwrap_or("none")) });
        }
        if let Some(v) = self.vert_align.and_then(|v| spelled(SCRIPTS, v)) {
            e.push(val("vertAlign", v));
        }
        if let Some(sz) = self.size {
            e.push(val("sz", &num(sz)));
        }
        if let Some(c) = &self.color {
            e.push(c.to_el("color"));
        }
        if let Some(n) = &self.name {
            e.push(val(if run { "rFont" } else { "name" }, n));
        }
        if let Some(f) = self.family {
            e.push(val("family", &f.to_string()));
        }
        if let Some(c) = self.charset {
            e.push(val("charset", &c.to_string()));
        }
        if let Some(s) = self.scheme.and_then(|s| spelled(SCHEMES, s)) {
            e.push(val("scheme", s));
        }
        e
    }
}

/// `<patternFill>` (18.8.32).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PatternFill {
    /// `None` when `patternType` is absent: in a cell format that is no fill, in a differential
    /// format it means solid.
    pub pattern: Option<Pattern>,
    pub fg: Option<Color>,
    pub bg: Option<Color>,
}

/// `<gradientFill>` (18.8.24).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GradientFill {
    /// `type="path"`; linear otherwise.
    pub path: bool,
    pub degree: f64,
    pub left: f64,
    pub right: f64,
    pub top: f64,
    pub bottom: f64,
    /// `(position, colour)` in file order.
    pub stops: Vec<(f64, Color)>,
}

/// `<fill>` (18.8.20).
#[derive(Clone, Debug, PartialEq)]
pub enum Fill {
    Pattern(PatternFill),
    Gradient(GradientFill),
}

impl Default for Fill {
    fn default() -> Self {
        Fill::Pattern(PatternFill::default())
    }
}

impl Fill {
    pub fn read(e: &El) -> Fill {
        if let Some(g) = e.child("gradientFill") {
            let f = |n: &str| g.attr_f64(n).unwrap_or(0.0);
            return Fill::Gradient(GradientFill {
                path: g.attr("type") == Some("path"),
                degree: f("degree"),
                left: f("left"),
                right: f("right"),
                top: f("top"),
                bottom: f("bottom"),
                stops: g
                    .children("stop")
                    .map(|s| (s.attr_f64("position").unwrap_or(0.0), s.child("color").map(Color::read).unwrap_or_default()))
                    .collect(),
            });
        }
        let Some(p) = e.child("patternFill") else { return Fill::default() };
        Fill::Pattern(PatternFill {
            pattern: p.attr("patternType").map(|t| spelling(PATTERNS, t).unwrap_or(Pattern::None)),
            fg: p.child("fgColor").map(Color::read),
            bg: p.child("bgColor").map(Color::read),
        })
    }

    /// `<fill>`.
    pub fn to_el(&self) -> El {
        let inner = match self {
            Fill::Pattern(p) => {
                let mut e = El::new("patternFill");
                set(&mut e, "patternType", p.pattern.and_then(|t| spelled(PATTERNS, t)).map(str::to_string));
                for (n, c) in [("fgColor", &p.fg), ("bgColor", &p.bg)] {
                    if let Some(c) = c {
                        e.push(c.to_el(n));
                    }
                }
                e
            }
            Fill::Gradient(g) => {
                let mut e = El::new("gradientFill");
                set(&mut e, "type", g.path.then(|| "path".to_string()));
                for (n, v) in [("degree", g.degree), ("left", g.left), ("right", g.right), ("top", g.top), ("bottom", g.bottom)] {
                    set(&mut e, n, (v != 0.0).then(|| num(v)));
                }
                for (pos, c) in &g.stops {
                    e.push(El::new("stop").with_attr("position", num(*pos)).with_child(c.to_el("color")));
                }
                e
            }
        };
        El::new("fill").with_child(inner)
    }
}

/// One edge of a border.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BorderEdge {
    pub style: BorderStyle,
    pub color: Option<Color>,
}

impl BorderEdge {
    fn read(e: &El) -> BorderEdge {
        let style = e.attr("style").and_then(|s| spelling(BORDER_STYLES, s)).unwrap_or_default();
        BorderEdge { style, color: e.child("color").map(Color::read) }
    }

    fn to_el(self, name: &str) -> El {
        let mut e = El::new(name);
        if self.style != BorderStyle::None {
            set(&mut e, "style", spelled(BORDER_STYLES, self.style).map(str::to_string));
        }
        if let Some(c) = self.color {
            e.push(c.to_el("color"));
        }
        e
    }
}

/// `<border>` (18.8.4). An edge is `None` when its element is absent. `start` and `end` (the
/// strict spellings) read as left and right.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Border {
    pub left: Option<BorderEdge>,
    pub right: Option<BorderEdge>,
    pub top: Option<BorderEdge>,
    pub bottom: Option<BorderEdge>,
    pub diagonal: Option<BorderEdge>,
    pub vertical: Option<BorderEdge>,
    pub horizontal: Option<BorderEdge>,
    pub diagonal_up: bool,
    pub diagonal_down: bool,
    pub outline: bool,
}

impl Border {
    pub fn read(e: &El) -> Border {
        let edge = |a: &str, b: &str| e.child(a).or_else(|| e.child(b)).map(BorderEdge::read);
        Border {
            left: edge("left", "start"),
            right: edge("right", "end"),
            top: edge("top", "top"),
            bottom: edge("bottom", "bottom"),
            diagonal: edge("diagonal", "diagonal"),
            vertical: edge("vertical", "vertical"),
            horizontal: edge("horizontal", "horizontal"),
            diagonal_up: e.flag("diagonalUp", false),
            diagonal_down: e.flag("diagonalDown", false),
            outline: e.flag("outline", true),
        }
    }

    /// `<border>`; the strict `start` and `end` are written as `left` and `right`.
    pub fn to_el(&self) -> El {
        let mut e = El::new("border");
        set(&mut e, "diagonalUp", flag(self.diagonal_up));
        set(&mut e, "diagonalDown", flag(self.diagonal_down));
        set(&mut e, "outline", (!self.outline).then(|| "0".to_string()));
        let edges = [
            ("left", self.left),
            ("right", self.right),
            ("top", self.top),
            ("bottom", self.bottom),
            ("diagonal", self.diagonal),
            ("vertical", self.vertical),
            ("horizontal", self.horizontal),
        ];
        for (n, edge) in edges {
            if let Some(edge) = edge {
                e.push(edge.to_el(n));
            }
        }
        e
    }
}

/// `<alignment>` (18.8.1).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Alignment {
    pub horizontal: Option<HAlign>,
    pub vertical: Option<VAlign>,
    /// 0 to 90 counter-clockwise, 91 to 180 clockwise (as 90 + degrees), 255 stacked.
    pub text_rotation: Option<u32>,
    pub wrap_text: bool,
    pub shrink_to_fit: bool,
    pub indent: Option<u32>,
    /// 0 context, 1 left to right, 2 right to left.
    pub reading_order: Option<u32>,
    pub justify_last_line: bool,
}

impl Alignment {
    pub fn read(a: &El) -> Alignment {
        Alignment {
            horizontal: a.attr("horizontal").map(|h| spelling(HALIGNS, h).unwrap_or(HAlign::General)),
            vertical: a.attr("vertical").map(|v| spelling(VALIGNS, v).unwrap_or(VAlign::Bottom)),
            text_rotation: a.attr_u32("textRotation"),
            wrap_text: a.flag("wrapText", false),
            shrink_to_fit: a.flag("shrinkToFit", false),
            indent: a.attr_u32("indent"),
            reading_order: a.attr_u32("readingOrder"),
            justify_last_line: a.flag("justifyLastLine", false),
        }
    }

    /// `<alignment>`.
    pub fn to_el(&self) -> El {
        let mut e = El::new("alignment");
        set(&mut e, "horizontal", self.horizontal.and_then(|h| spelled(HALIGNS, h)).map(str::to_string));
        set(&mut e, "vertical", self.vertical.and_then(|v| spelled(VALIGNS, v)).map(str::to_string));
        set(&mut e, "textRotation", self.text_rotation.map(|v| v.to_string()));
        set(&mut e, "wrapText", flag(self.wrap_text));
        set(&mut e, "indent", self.indent.map(|v| v.to_string()));
        set(&mut e, "justifyLastLine", flag(self.justify_last_line));
        set(&mut e, "shrinkToFit", flag(self.shrink_to_fit));
        set(&mut e, "readingOrder", self.reading_order.map(|v| v.to_string()));
        e
    }
}

/// `<protection>` (18.8.33).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Protection {
    pub locked: Option<bool>,
    pub hidden: Option<bool>,
}

impl Protection {
    fn read(p: &El) -> Protection {
        Protection { locked: p.attr_bool("locked"), hidden: p.attr_bool("hidden") }
    }

    fn to_el(self) -> El {
        let mut e = El::new("protection");
        set(&mut e, "locked", self.locked.map(|v| u8::from(v).to_string()));
        set(&mut e, "hidden", self.hidden.map(|v| u8::from(v).to_string()));
        e
    }
}

/// A cell format or a cell style format (`<xf>`, 18.8.45): indexes into the tables of
/// [`Styles`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Xf {
    pub num_fmt_id: u32,
    pub font_id: u32,
    pub fill_id: u32,
    pub border_id: u32,
    /// The cell style format this cell format is based on (`cellXfs` only).
    pub xf_id: Option<u32>,
    pub alignment: Option<Alignment>,
    pub protection: Option<Protection>,
    pub quote_prefix: bool,
    /// The element as written, for its `apply*` flags and extensions.
    pub el: El,
}

impl Xf {
    fn read(xf: &El) -> Xf {
        let id = |n: &str| xf.attr_u32(n).unwrap_or(0);
        Xf {
            num_fmt_id: id("numFmtId"),
            font_id: id("fontId"),
            fill_id: id("fillId"),
            border_id: id("borderId"),
            xf_id: xf.attr_u32("xfId"),
            alignment: xf.child("alignment").map(Alignment::read),
            protection: xf.child("protection").map(Protection::read),
            quote_prefix: xf.flag("quotePrefix", false),
            el: xf.clone(),
        }
    }

    /// `<xf>`: the typed fields, then the rest of [`Xf::el`] (its `apply*` flags and
    /// extensions) as written.
    pub fn to_el(&self) -> El {
        const TYPED: &[&str] = &["numFmtId", "fontId", "fillId", "borderId", "xfId", "quotePrefix"];
        let mut e = El::new("xf")
            .with_attr("numFmtId", self.num_fmt_id)
            .with_attr("fontId", self.font_id)
            .with_attr("fillId", self.fill_id)
            .with_attr("borderId", self.border_id);
        set(&mut e, "xfId", self.xf_id.map(|v| v.to_string()));
        set(&mut e, "quotePrefix", flag(self.quote_prefix));
        e.attrs.extend(self.el.attrs.iter().filter(|(k, _)| !TYPED.contains(&k.as_str())).cloned());
        if let Some(a) = &self.alignment {
            e.push(a.to_el());
        }
        if let Some(p) = self.protection {
            e.push(p.to_el());
        }
        e.children.extend(self.el.elements().filter(|c| c.name != "alignment" && c.name != "protection").cloned().map(craft_xml::Node::El));
        e
    }
}

/// A named cell style (`<cellStyle>`, 18.8.7).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CellStyle {
    pub name: String,
    /// Index into [`Styles::cell_style_xfs`].
    pub xf_id: u32,
    pub builtin_id: Option<u32>,
    /// `iLevel`: the outline level of a `RowLevel_n` or `ColLevel_n` style.
    pub level: Option<u32>,
    pub hidden: bool,
    pub custom_builtin: bool,
}

/// A number format as a differential format states it: an id, a code, or both.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NumFmt {
    pub id: Option<u32>,
    pub code: Option<String>,
}

/// A differential format (`<dxf>`, 18.8.14), used by conditional formats and table styles.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Dxf {
    pub font: Option<Font>,
    pub num_fmt: Option<NumFmt>,
    pub fill: Option<Fill>,
    pub border: Option<Border>,
    pub alignment: Option<Alignment>,
    pub protection: Option<Protection>,
}

impl Dxf {
    fn read(x: &El) -> Dxf {
        Dxf {
            font: x.child("font").map(Font::read),
            num_fmt: x.child("numFmt").map(|n| NumFmt { id: n.attr_u32("numFmtId"), code: n.attr("formatCode").map(str::to_string) }),
            fill: x.child("fill").map(Fill::read),
            border: x.child("border").map(Border::read),
            alignment: x.child("alignment").map(Alignment::read),
            protection: x.child("protection").map(Protection::read),
        }
    }

    /// `<dxf>`.
    pub fn to_el(&self) -> El {
        let mut e = El::new("dxf");
        if let Some(f) = &self.font {
            e.push(f.to_el("font", false));
        }
        if let Some(n) = &self.num_fmt {
            let mut f = El::new("numFmt");
            set(&mut f, "numFmtId", n.id.map(|v| v.to_string()));
            set(&mut f, "formatCode", n.code.clone());
            e.push(f);
        }
        if let Some(f) = &self.fill {
            e.push(f.to_el());
        }
        if let Some(a) = &self.alignment {
            e.push(a.to_el());
        }
        if let Some(p) = self.protection {
            e.push(p.to_el());
        }
        if let Some(b) = &self.border {
            e.push(b.to_el());
        }
        e
    }
}

/// The styles part.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Styles {
    /// Custom number formats `(numFmtId, formatCode)` in file order.
    pub num_fmts: Vec<(u32, String)>,
    pub fonts: Vec<Font>,
    pub fills: Vec<Fill>,
    pub borders: Vec<Border>,
    pub cell_style_xfs: Vec<Xf>,
    /// What a cell's `s` attribute indexes.
    pub cell_xfs: Vec<Xf>,
    pub cell_styles: Vec<CellStyle>,
    pub dxfs: Vec<Dxf>,
    /// `colors/indexedColors` as `0xRRGGBB`, empty when the file keeps the default palette.
    pub indexed_colors: Vec<u32>,
    /// The whole part, for what is not lifted out (table styles, recent colours, extensions).
    pub root: El,
}

impl Styles {
    pub fn read(root: El) -> Styles {
        fn list<'r>(root: &'r El, name: &str, item: &'static str) -> Vec<&'r El> {
            root.child(name).map(|l| l.children(item).collect()).unwrap_or_default()
        }
        let mut s = Styles {
            num_fmts: list(&root, "numFmts", "numFmt")
                .into_iter()
                .filter_map(|f| Some((f.attr_u32("numFmtId")?, f.attr("formatCode")?.to_string())))
                .collect(),
            fonts: list(&root, "fonts", "font").into_iter().map(Font::read).collect(),
            fills: list(&root, "fills", "fill").into_iter().map(Fill::read).collect(),
            borders: list(&root, "borders", "border").into_iter().map(Border::read).collect(),
            cell_style_xfs: list(&root, "cellStyleXfs", "xf").into_iter().map(Xf::read).collect(),
            cell_xfs: list(&root, "cellXfs", "xf").into_iter().map(Xf::read).collect(),
            cell_styles: list(&root, "cellStyles", "cellStyle")
                .into_iter()
                .map(|c| CellStyle {
                    name: c.attr("name").unwrap_or_default().to_string(),
                    xf_id: c.attr_u32("xfId").unwrap_or(0),
                    builtin_id: c.attr_u32("builtinId"),
                    level: c.attr_u32("iLevel"),
                    hidden: c.flag("hidden", false),
                    custom_builtin: c.flag("customBuiltin", false),
                })
                .collect(),
            dxfs: list(&root, "dxfs", "dxf").into_iter().map(Dxf::read).collect(),
            indexed_colors: root
                .path(&["colors", "indexedColors"])
                .map(|ix| ix.children("rgbColor").map(|c| Color::read(c).rgb(&DEFAULT_PALETTE).unwrap_or(0)).collect())
                .unwrap_or_default(),
            root: El::default(),
        };
        s.root = root;
        s
    }

    /// The indexed colour table: the file's `indexedColors`, or [`DEFAULT_PALETTE`].
    pub fn palette(&self) -> &[u32] {
        if self.indexed_colors.is_empty() { &DEFAULT_PALETTE } else { &self.indexed_colors }
    }

    /// The part, with every table regenerated from the fields above and the rest of
    /// [`Styles::root`] (table styles, recent colours, extensions) kept in place.
    pub fn to_xml(&self) -> Vec<u8> {
        const ORDER: &[&str] =
            &["numFmts", "fonts", "fills", "borders", "cellStyleXfs", "cellXfs", "cellStyles", "dxfs", "tableStyles", "colors", "extLst"];
        let root = if self.root.name.is_empty() { El::new("styleSheet") } else { self.root.clone() };
        let num_fmts = self.num_fmts.iter().map(|(id, code)| El::new("numFmt").with_attr("numFmtId", id).with_attr("formatCode", code)).collect();
        let cell_styles = self
            .cell_styles
            .iter()
            .map(|c| {
                let mut e = El::new("cellStyle").with_attr("name", &c.name).with_attr("xfId", c.xf_id);
                set(&mut e, "builtinId", c.builtin_id.map(|v| v.to_string()));
                set(&mut e, "iLevel", c.level.map(|v| v.to_string()));
                set(&mut e, "hidden", flag(c.hidden));
                set(&mut e, "customBuiltin", flag(c.custom_builtin));
                e
            })
            .collect();
        let mut colors = root.child("colors").cloned().unwrap_or_else(|| El::new("colors"));
        colors.children.retain(|n| !matches!(n, craft_xml::Node::El(c) if c.name == "indexedColors"));
        if !self.indexed_colors.is_empty() {
            let mut ix = El::new("indexedColors");
            for c in &self.indexed_colors {
                ix.push(El::new("rgbColor").with_attr("rgb", format!("FF{:06X}", c & 0xFF_FFFF)));
            }
            colors.children.insert(0, craft_xml::Node::El(ix));
        }
        let owned = vec![
            ("numFmts", list(&root, "numFmts", num_fmts)),
            ("fonts", list(&root, "fonts", self.fonts.iter().map(|f| f.to_el("font", false)).collect())),
            ("fills", list(&root, "fills", self.fills.iter().map(Fill::to_el).collect())),
            ("borders", list(&root, "borders", self.borders.iter().map(Border::to_el).collect())),
            ("cellStyleXfs", list(&root, "cellStyleXfs", self.cell_style_xfs.iter().map(Xf::to_el).collect())),
            ("cellXfs", list(&root, "cellXfs", self.cell_xfs.iter().map(Xf::to_el).collect())),
            ("cellStyles", list(&root, "cellStyles", cell_styles)),
            ("dxfs", list(&root, "dxfs", self.dxfs.iter().map(Dxf::to_el).collect())),
            ("colors", if colors.elements().next().is_some() { Some(colors) } else { None }),
        ];
        let mut out = crate::write::assemble(&root, ORDER, owned);
        crate::ns::declare(&mut out);
        let mut w = craft_xml::Writer::document();
        w.tree(&out);
        w.into_bytes()
    }

    /// The format code of number format `id`: the file's custom format, else the built-in one.
    /// `None` for an id that is neither (Excel shows General).
    pub fn number_format(&self, id: u32) -> Option<&str> {
        self.num_fmts.iter().find(|(i, _)| *i == id).map(|(_, c)| c.as_str()).or_else(|| builtin_format(id))
    }

    /// The format code of cell format `xf` (a cell's `s`).
    pub fn cell_number_format(&self, xf: u32) -> Option<&str> {
        self.number_format(self.cell_xfs.get(xf as usize)?.num_fmt_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn styles(xml: &str) -> Styles {
        Styles::read(craft_xml::Options::local().parse(xml.as_bytes()).unwrap_or_default())
    }

    #[test]
    fn tables_by_index() {
        let s = styles(
            r##"<x:styleSheet xmlns:x="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
            <x:numFmts count="1"><x:numFmt numFmtId="164" formatCode="yyyy-mm-dd"/></x:numFmts>
            <x:fonts><x:font><x:b/><x:i val="0"/><x:sz val="11"/><x:color theme="1" tint="-0.25"/><x:name val="Calibri"/><x:scheme val="minor"/></x:font></x:fonts>
            <x:fills><x:fill><x:patternFill patternType="none"/></x:fill><x:fill><x:patternFill patternType="solid"><x:fgColor rgb="FFFF0000"/><x:bgColor indexed="64"/></x:patternFill></x:fill>
              <x:fill><x:gradientFill degree="90"><x:stop position="0"><x:color rgb="00FF00"/></x:stop><x:stop position="1"><x:color theme="4"/></x:stop></x:gradientFill></x:fill></x:fills>
            <x:borders><x:border diagonalUp="1"><x:start style="thin"><x:color indexed="10"/></x:start><x:bottom style="double"/><x:diagonal style="hair"/></x:border></x:borders>
            <x:cellStyleXfs><x:xf numFmtId="0" fontId="0"/></x:cellStyleXfs>
            <x:cellXfs><x:xf/><x:xf numFmtId="164" fontId="0" fillId="1" borderId="0" xfId="0" applyAlignment="1" quotePrefix="1"><x:alignment horizontal="centerContinuous" textRotation="255" wrapText="1"/><x:protection locked="0"/></x:xf><x:xf numFmtId="14"/><x:xf numFmtId="99"/></x:cellXfs>
            <x:cellStyles><x:cellStyle name="Normal" xfId="0" builtinId="0"/></x:cellStyles>
            <x:dxfs><x:dxf><x:font><x:b/></x:font><x:numFmt numFmtId="200" formatCode="0.0"/><x:fill><x:patternFill><x:bgColor rgb="FFFFFF00"/></x:patternFill></x:fill></x:dxf></x:dxfs>
            <x:colors><x:indexedColors><x:rgbColor rgb="FF010203"/><x:rgbColor rgb="bad"/></x:indexedColors></x:colors>
            </x:styleSheet>"##,
        );
        let f = &s.fonts[0];
        assert_eq!((f.bold, f.italic, f.strike, f.size, f.name.as_deref()), (Some(true), Some(false), None, Some(11.0), Some("Calibri")));
        assert_eq!(f.color, Some(Color { source: ColorSource::Theme(1), tint: -0.25 }));
        assert_eq!(f.scheme, Some(FontScheme::Minor));
        assert_eq!(s.fills[0], Fill::Pattern(PatternFill { pattern: Some(Pattern::None), fg: None, bg: None }));
        let Fill::Pattern(solid) = &s.fills[1] else { panic!("{:?}", s.fills[1]) };
        assert_eq!((solid.pattern, solid.fg.map(|c| c.source)), (Some(Pattern::Solid), Some(ColorSource::Rgb(0xFFFF0000))));
        let Fill::Gradient(g) = &s.fills[2] else { panic!("{:?}", s.fills[2]) };
        assert_eq!((g.degree, g.stops.len(), g.stops[0].1.rgb(&[])), (90.0, 2, Some(0x00FF00)));
        let b = &s.borders[0];
        assert_eq!(b.left.map(|e| (e.style, e.color.and_then(|c| c.rgb(s.palette())))), Some((BorderStyle::Thin, None)));
        assert_eq!((b.bottom.map(|e| e.style), b.right, b.diagonal_up, b.diagonal_down), (Some(BorderStyle::Double), None, true, false));
        let xf = &s.cell_xfs[1];
        assert_eq!((xf.num_fmt_id, xf.fill_id, xf.xf_id, xf.quote_prefix), (164, 1, Some(0), true));
        let a = xf.alignment.unwrap_or_default();
        assert_eq!((a.horizontal, a.text_rotation, a.wrap_text, a.vertical), (Some(HAlign::CenterAcross), Some(255), true, None));
        assert_eq!(xf.protection, Some(Protection { locked: Some(false), hidden: None }));
        assert_eq!(xf.el.attr("applyAlignment"), Some("1"));
        assert_eq!(
            (s.cell_number_format(1), s.cell_number_format(2), s.cell_number_format(3), s.cell_number_format(9)),
            (Some("yyyy-mm-dd"), Some("m/d/yyyy"), None, None)
        );
        assert_eq!(s.cell_styles[0].builtin_id, Some(0));
        let d = &s.dxfs[0];
        assert_eq!(d.num_fmt, Some(NumFmt { id: Some(200), code: Some("0.0".into()) }));
        assert_eq!(
            d.fill,
            Some(Fill::Pattern(PatternFill { pattern: None, fg: None, bg: Some(Color { source: ColorSource::Rgb(0xFFFFFF00), tint: 0.0 }) }))
        );
        assert_eq!(s.indexed_colors, [0x010203, 0]);
        assert_eq!(s.palette().len(), 2);
        assert_eq!(styles("<styleSheet/>").palette(), DEFAULT_PALETTE);
    }

    #[test]
    fn long_font_names_are_cut() {
        let s = styles(&format!(r#"<styleSheet><fonts><font><name val="{}"/></font></fonts></styleSheet>"#, "é".repeat(300)));
        assert_eq!(s.fonts[0].name.as_ref().map(|n| n.chars().count()), Some(MAX_FONT_NAME_CHARS));
    }
}
