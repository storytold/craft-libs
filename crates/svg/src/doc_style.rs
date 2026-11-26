//! Compatibility SVG document/CSS interpretation helpers extracted from EffectCraft.
//!
//! Provides presentation-attribute + stylesheet + inline-`style` declaration collection
//! ([`declared`](crate::doc_style::declared)), property cascading ([`cascade`](crate::doc_style::cascade)), unit/percentage length resolution
//! ([`length`](crate::doc_style::length), [`attr_len`](crate::doc_style::attr_len)), paint reference parsing/resolution ([`PaintRef`](crate::doc_style::PaintRef), [`paint_ref`](crate::doc_style::paint_ref),
//! [`resolve_paint`](crate::doc_style::resolve_paint)), and `viewBox` / `preserveAspectRatio` transforms ([`viewbox`](crate::doc_style::viewbox),
//! [`viewbox_transform`](crate::doc_style::viewbox_transform)).

use std::collections::HashMap;

use kurbo::Affine;

use crate::{
    color::parse_rgba_compat,
    prefix_path::numbers_bounded,
    simple_css::{StyleSheet, declarations},
};

/// SVG `fill-rule` property.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FillRule {
    #[default]
    NonZero,
    EvenOdd,
}

/// SVG `stroke-linecap` property.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Cap {
    #[default]
    Butt,
    Round,
    Square,
}

/// SVG `stroke-linejoin` property.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Join {
    #[default]
    Miter,
    Round,
    Bevel,
}

/// Parsed SVG paint reference (`none`, `currentColor`, `url(#id) [fallback]`, or RGBA color).
#[derive(Clone, Debug, PartialEq)]
pub enum PaintRef {
    /// Solid straight RGBA color `[r, g, b, a]` in `0.0..=1.0`.
    Color([f64; 4]),
    /// Paint server reference `url(#id)` with an optional fallback RGBA color.
    Url(String, Option<[f64; 4]>),
    /// `currentColor`.
    Current,
}

/// Caller outcome when resolving a `url(#id)` paint reference.
#[derive(Clone, Debug, PartialEq)]
pub enum UrlPaint<S> {
    /// Resolved to a caller paint server (for example, a gradient or pattern).
    Server(S),
    /// Resolved to a solid RGBA color (for example, a single-stop gradient).
    Color([f64; 4]),
    /// No paint server found; fall back to the `url(#id) fallback` color if present.
    Fallback,
    /// Paint server found but empty or invalid; do not fall back.
    None,
}

/// Result of resolving a [`PaintRef`] without committing to a caller's paint-server type.
#[derive(Clone, Debug, PartialEq)]
pub enum ResolvedPaint<S> {
    /// Solid RGB color `[r, g, b]` with alpha `a`.
    Color([f64; 3], f64),
    /// A caller-resolved paint server (`url(#id)`).
    Server(S),
}

/// Inherited SVG style state.
#[derive(Clone, Debug, PartialEq)]
pub struct Style {
    pub fill: Option<PaintRef>,
    pub fill_opacity: f64,
    pub fill_rule: FillRule,
    pub stroke: Option<PaintRef>,
    pub stroke_opacity: f64,
    pub stroke_width: f64,
    pub cap: Cap,
    pub join: Join,
    pub miter: f64,
    pub dash: Option<Vec<f64>>,
    pub dash_offset: f64,
    pub color: [f64; 4],
    pub visible: bool,
    pub font_size: f64,
}

impl Default for Style {
    fn default() -> Self {
        Style {
            fill: Some(PaintRef::Color([0.0, 0.0, 0.0, 1.0])),
            fill_opacity: 1.0,
            fill_rule: FillRule::NonZero,
            stroke: None,
            stroke_opacity: 1.0,
            stroke_width: 1.0,
            cap: Cap::Butt,
            join: Join::Miter,
            miter: 4.0,
            dash: None,
            dash_offset: 0.0,
            color: [0.0, 0.0, 0.0, 1.0],
            visible: true,
            font_size: 16.0,
        }
    }
}

/// Presentation attributes recognised by [`declared`].
pub const STYLE_PROPS: &[&str] = &[
    "fill",
    "fill-opacity",
    "fill-rule",
    "stroke",
    "stroke-width",
    "stroke-opacity",
    "stroke-linecap",
    "stroke-linejoin",
    "stroke-miterlimit",
    "stroke-dasharray",
    "stroke-dashoffset",
    "opacity",
    "display",
    "visibility",
    "color",
    "font-size",
    "transform",
    "stop-color",
    "stop-opacity",
];

/// Normalize only numeric CSS length-unit suffixes in the properties this cascade resolves.
/// Raw SVG attributes and opaque CSS values keep their existing spelling and compatibility.
fn css_length_units(property: &str, value: String) -> String {
    if !matches!(property, "font-size" | "stroke-width" | "stroke-dasharray" | "stroke-dashoffset") {
        return value;
    }
    let separator = |c: char| c == ',' || c.is_whitespace();
    let finite = |number: &str| number.parse::<f64>().ok().is_some_and(f64::is_finite);
    let supported = |unit: &str| matches!(unit.to_ascii_lowercase().as_str(), "px" | "pt" | "pc" | "mm" | "cm" | "in" | "em" | "ex");
    let numeric_length = |token: &str| {
        finite(token.strip_suffix('%').unwrap_or(token))
            || token.len().checked_sub(2).is_some_and(|split| token.get(..split).is_some_and(finite) && token.get(split..).is_some_and(supported))
    };
    let eligible = if property == "stroke-dasharray" {
        value.split(separator).filter(|token| !token.is_empty()).all(numeric_length)
    } else {
        numeric_length(value.trim())
    };
    if !eligible {
        return value;
    }
    let mut out = String::with_capacity(value.len());
    for part in value.split_inclusive(separator) {
        let token = part.trim_end_matches(separator);
        if let Some(split) = token.len().checked_sub(2)
            && let (Some(number), Some(unit)) = (token.get(..split), token.get(split..))
            && number.parse::<f64>().ok().is_some_and(f64::is_finite)
        {
            let unit = unit.to_ascii_lowercase();
            if matches!(unit.as_str(), "px" | "pt" | "pc" | "mm" | "cm" | "in" | "em" | "ex") {
                out.push_str(number);
                out.push_str(&unit);
                out.push_str(&part[token.len()..]);
                continue;
            }
        }
        out.push_str(part);
    }
    out
}

/// Normalize supported CSS value identifiers without changing opaque payloads or raw attributes.
fn css_value(property: &str, value: String) -> String {
    let mut value = css_length_units(property, value);
    let token = value.trim();
    let keywords: &[&str] = match property {
        "display" => &["none"],
        "visibility" => &["visible", "hidden", "collapse"],
        "fill-rule" => &["nonzero", "evenodd"],
        "stroke-linecap" => &["butt", "round", "square"],
        "stroke-linejoin" => &["miter", "round", "bevel"],
        "fill" | "stroke" => &["none", "currentColor"],
        _ => &[],
    };
    let start = value.len() - value.trim_start().len();
    if let Some(keyword) = keywords.iter().find(|keyword| token.eq_ignore_ascii_case(keyword)) {
        let end = start + token.len();
        value.replace_range(start..end, keyword);
    } else if matches!(property, "fill" | "stroke") && token.get(..4).is_some_and(|prefix| prefix.eq_ignore_ascii_case("url(")) {
        value.replace_range(start..start + 3, "url");
    }
    value
}

/// Collect the declared properties of an element: presentation attributes, then CSS rules
/// (by specificity and source order, bounded by `charge_css`), then the inline `style` attribute.
/// Supported numeric length-unit suffixes and value identifiers are normalized only for
/// CSS declarations; raw presentation attributes retain their existing spelling and compatibility.
pub fn declared(n: roxmltree::Node, css: &StyleSheet, charge_css: impl FnMut(usize) -> bool) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for a in n.attributes() {
        if STYLE_PROPS.contains(&a.name()) && a.name() != "transform" {
            out.insert(a.name().to_string(), a.value().to_string());
        }
    }
    let mut css_values = HashMap::new();
    css.apply(n.tag_name().name(), n.attribute("id"), n.attribute("class").unwrap_or_default(), &mut css_values, charge_css);
    out.extend(css_values.into_iter().map(|(k, v)| {
        let v = css_value(&k, v);
        (k, v)
    }));
    if let Some(st) = n.attribute("style") {
        for (k, v) in declarations(st) {
            let v = css_value(&k, v);
            out.insert(k, v);
        }
    }
    out
}

/// Parse an SVG length or percentage into user-space pixels.
///
/// `axis` selects the percentage reference base: `'x'` uses `vw`, `'y'` uses `vh`, and any
/// other value uses the normalized diagonal `((vw^2 + vh^2) / 2).sqrt()`.
pub fn length(s: &str, axis: char, font: f64, vw: f64, vh: f64) -> Option<f64> {
    let s = s.trim();
    let num_end = s.find(|c: char| !(c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E'))).unwrap_or(s.len());
    // "e" also starts "em"/"ex": back off if the number does not parse.
    let (mut num, mut unit) = (&s[..num_end], &s[num_end..]);
    if num.parse::<f64>().is_err() && (num.ends_with('e') || num.ends_with('E')) {
        let back = num_end.saturating_sub(1);
        num = &s[..back];
        unit = &s[back..];
    }
    let v: f64 = num.parse().ok()?;
    let value = match unit.trim() {
        "" | "px" => v,
        "pt" => v * 4.0 / 3.0,
        "pc" => v * 16.0,
        "mm" => v * 96.0 / 25.4,
        "cm" => v * 96.0 / 2.54,
        "in" => v * 96.0,
        "em" => v * font,
        "ex" => v * font / 2.0,
        "%" => {
            let base = match axis {
                'x' => vw,
                'y' => vh,
                _ => ((vw * vw + vh * vh) / 2.0).sqrt(),
            };
            v / 100.0 * base
        }
        _ => v,
    };
    value.is_finite().then_some(value)
}

/// Parse a strict SVG length in user units, rejecting unknown unit suffixes.
///
/// `%` is relative to `reference`; `em`/`ex` assume a 16 px font.
pub fn length_strict(s: &str, reference: f64) -> Option<f64> {
    let v = s.trim();
    let units: [(&str, f64); 9] = [
        ("%", reference / 100.0),
        ("px", 1.0),
        ("pt", 96.0 / 72.0),
        ("pc", 16.0),
        ("mm", 96.0 / 25.4),
        ("cm", 96.0 / 2.54),
        ("in", 96.0),
        ("em", 16.0),
        ("ex", 8.0),
    ];
    let (num, k) = units.iter().find_map(|(u, k)| v.strip_suffix(u).map(|n| (n, *k))).unwrap_or((v, 1.0));
    let n: f64 = num.trim().parse().ok().filter(|n: &f64| n.is_finite())?;
    Some(n * k).filter(|x| x.is_finite())
}

/// Parse an SVG opacity or offset value (number or percentage), clamped to `[0.0, 1.0]`.
pub fn parse_opacity(s: &str) -> Option<f64> {
    let v = s.trim();
    let x = match v.strip_suffix('%') {
        Some(p) => p.parse::<f64>().ok()? / 100.0,
        None => v.parse::<f64>().ok()?,
    };
    x.is_finite().then(|| x.clamp(0.0, 1.0))
}

/// Parse an SVG `font-size` (keyword, relative `em`/`ex`/`%` to `parent`, or strict length).
pub fn parse_font_size(s: &str, parent: f64, reference: f64) -> Option<f64> {
    let v = s.trim();
    let keyword = match v {
        "xx-small" => Some(9.0),
        "x-small" => Some(10.0),
        "small" => Some(13.0),
        "medium" => Some(16.0),
        "large" => Some(18.0),
        "x-large" => Some(24.0),
        "xx-large" => Some(32.0),
        "larger" => Some(parent * 1.2),
        "smaller" => Some(parent / 1.2),
        _ => None,
    };
    let rel = |suffix: &str, k: f64| v.strip_suffix(suffix).and_then(|n| n.trim().parse::<f64>().ok()).map(|n| n * k);
    let size = keyword
        .or_else(|| rel("em", parent))
        .or_else(|| rel("ex", parent * 0.5))
        .or_else(|| rel("%", parent / 100.0))
        .or_else(|| length_strict(v, reference))?;
    size.is_finite().then(|| size.clamp(0.0, 2000.0))
}

/// Parse an SVG `stroke-dasharray` list (`none` or empty/negative/invalid returns `None`).
pub fn parse_dash_array(s: &str, reference: f64) -> Option<Vec<f64>> {
    if s.trim() == "none" {
        return None;
    }
    let mut out = Vec::new();
    for part in s.split(|c: char| c == ',' || c.is_whitespace()).filter(|p| !p.trim().is_empty()).take(1024) {
        let x = length_strict(part, reference)?;
        if x < 0.0 {
            return None;
        }
        out.push(x);
    }
    (!out.is_empty()).then_some(out)
}

/// What a percentage of geometry attribute `attr_name` is relative to in a `width` x `height` viewport.
pub fn symbol_percent_basis(attr_name: &str, width: f64, height: f64) -> Option<f64> {
    match attr_name {
        "x" | "x1" | "x2" | "cx" | "width" | "rx" => Some(width),
        "y" | "y1" | "y2" | "cy" | "height" | "ry" => Some(height),
        "r" => Some(((width * width + height * height) / 2.0).sqrt()),
        _ => None,
    }
}

/// Parse a length attribute on `n`, returning `0.0` if missing or invalid.
pub fn attr_len(n: roxmltree::Node, name: &str, axis: char, st: &Style, vw: f64, vh: f64) -> f64 {
    n.attribute(name).and_then(|v| length(v, axis, st.font_size, vw, vh)).unwrap_or(0.0)
}

/// Parse an SVG paint value (`none`, `currentColor`, `url(#id) [fallback]`, `inherit`, or color).
///
/// Returns `None` for `"inherit"` or unrecognized values, `Some(None)` for `"none"`, and
/// `Some(Some(PaintRef))` for concrete paints or `url(...)` / `currentColor` references.
pub fn paint_ref(v: &str) -> Option<Option<PaintRef>> {
    let v = v.trim();
    if v == "none" {
        return Some(None);
    }
    if v == "currentColor" || v == "currentcolor" {
        return Some(Some(PaintRef::Current));
    }
    if let Some(rest) = v.strip_prefix("url(") {
        let (url, fallback) = rest.split_once(')').unwrap_or((rest, ""));
        let url = url.trim().trim_matches(['"', '\'']);
        let id = url.strip_prefix('#').filter(|id| !id.is_empty()).unwrap_or_default().to_string();
        let fb = fallback.trim();
        let fb = if fb.is_empty() || fb == "none" { None } else { parse_rgba_compat(fb) };
        return Some(Some(PaintRef::Url(id, fb)));
    }
    if v == "inherit" {
        return None;
    }
    parse_rgba_compat(v).map(|c| Some(PaintRef::Color(c)))
}

/// Apply an element's declared inherited properties to `st`.
///
/// Returns `(opacity, display)` where `display` is `false` when `display: none` is set.
pub fn cascade(n: roxmltree::Node, css: &StyleSheet, st: &mut Style, vw: f64, vh: f64, charge_css: impl FnMut(usize) -> bool) -> (f64, bool) {
    let d = declared(n, css, charge_css);
    let num = |k: &str| {
        d.get(k)
            .and_then(|v| v.trim().trim_end_matches('%').parse::<f64>().ok().map(|x| if v.trim().ends_with('%') { x / 100.0 } else { x }))
            .filter(|x| x.is_finite())
    };
    if let Some(c) = d.get("color").and_then(|v| parse_rgba_compat(v)) {
        st.color = c;
    }
    let font_size = d.get("font-size").and_then(|v| match v.trim().strip_suffix('%') {
        Some(p) => p.trim().parse::<f64>().ok().map(|p| p / 100.0 * st.font_size),
        None => length(v, 'o', st.font_size, vw, vh),
    });
    if let Some(v) = font_size.filter(|v| v.is_finite()) {
        st.font_size = v;
    }
    if let Some(p) = d.get("fill").and_then(|v| paint_ref(v)) {
        st.fill = p;
    }
    if let Some(p) = d.get("stroke").and_then(|v| paint_ref(v)) {
        st.stroke = p;
    }
    if let Some(v) = num("fill-opacity") {
        st.fill_opacity = v.clamp(0.0, 1.0);
    }
    if let Some(v) = num("stroke-opacity") {
        st.stroke_opacity = v.clamp(0.0, 1.0);
    }
    if let Some(v) = d.get("stroke-width").and_then(|v| length(v, 'o', st.font_size, vw, vh)) {
        st.stroke_width = v.max(0.0);
    }
    if let Some(v) = num("stroke-miterlimit") {
        st.miter = v.max(1.0);
    }
    if let Some(v) = d.get("fill-rule") {
        st.fill_rule = if v.trim() == "evenodd" { FillRule::EvenOdd } else { FillRule::NonZero };
    }
    if let Some(v) = d.get("stroke-linecap") {
        st.cap = match v.trim() {
            "round" => Cap::Round,
            "square" => Cap::Square,
            _ => Cap::Butt,
        };
    }
    if let Some(v) = d.get("stroke-linejoin") {
        st.join = match v.trim() {
            "round" => Join::Round,
            "bevel" => Join::Bevel,
            _ => Join::Miter,
        };
    }
    if let Some(v) = d.get("stroke-dasharray") {
        st.dash = if v.trim() == "none" {
            None
        } else {
            let a: Vec<f64> = v
                .split(|c: char| c == ',' || c.is_whitespace())
                .filter(|t| !t.is_empty())
                .filter_map(|t| length(t, 'o', st.font_size, vw, vh))
                .collect();
            if a.iter().all(|x| *x >= 0.0) && a.iter().any(|x| *x > 0.0) { Some(a) } else { None }
        };
    }
    if let Some(v) = d.get("stroke-dashoffset").and_then(|v| length(v, 'o', st.font_size, vw, vh)) {
        st.dash_offset = v;
    }
    if let Some(v) = d.get("visibility") {
        st.visible = !matches!(v.trim(), "hidden" | "collapse");
    }
    let display = d.get("display").is_none_or(|v| v.trim() != "none");
    (num("opacity").unwrap_or(1.0).clamp(0.0, 1.0), display)
}

/// Resolve a [`PaintRef`] against `st` and a caller-supplied `url(#id)` resolver.
pub fn resolve_paint<S>(paint: &PaintRef, st: &Style, resolve_url: impl FnOnce(&str, Option<[f64; 4]>) -> UrlPaint<S>) -> Option<ResolvedPaint<S>> {
    match paint {
        PaintRef::Color(c) => Some(ResolvedPaint::Color([c[0], c[1], c[2]], c[3])),
        PaintRef::Current => Some(ResolvedPaint::Color([st.color[0], st.color[1], st.color[2]], st.color[3])),
        PaintRef::Url(id, fallback) => match resolve_url(id, *fallback) {
            UrlPaint::Server(s) => Some(ResolvedPaint::Server(s)),
            UrlPaint::Color(c) => Some(ResolvedPaint::Color([c[0], c[1], c[2]], c[3])),
            UrlPaint::Fallback => fallback.map(|c| ResolvedPaint::Color([c[0], c[1], c[2]], c[3])),
            UrlPaint::None => None,
        },
    }
}

/// Parse a `viewBox` attribute string into `([min_x, min_y], width, height)`.
pub fn parse_viewbox(s: &str) -> Option<([f64; 2], f64, f64)> {
    let (v, exceeded) = numbers_bounded(s, 5);
    if exceeded || !v.iter().all(|x| x.is_finite()) {
        return None;
    }
    let [x, y, width, height] = v.as_slice() else { return None };
    (*width > 0.0 && *height > 0.0).then_some(([*x, *y], *width, *height))
}

/// Compute the `viewBox` -> viewport transform for `preserveAspectRatio` (default `xMidYMid meet`).
pub fn viewbox_transform_with(vb: ([f64; 2], f64, f64), w: f64, h: f64, preserve_aspect_ratio: &str) -> Affine {
    let ([x, y], vw, vh) = vb;
    let par = if preserve_aspect_ratio.trim().is_empty() { "xMidYMid meet" } else { preserve_aspect_ratio };
    let mut parts = par.split_whitespace();
    let mut align = parts.next().unwrap_or("xMidYMid");
    if align == "defer" {
        align = parts.next().unwrap_or("xMidYMid");
    }
    let slice = parts.next() == Some("slice");
    let (sx, sy) = (w / vw, h / vh);
    if align == "none" {
        return Affine::scale_non_uniform(sx, sy) * Affine::translate((-x, -y));
    }
    let s = if slice { sx.max(sy) } else { sx.min(sy) };
    let fx = if align.contains("xMid") {
        0.5
    } else if align.contains("xMax") {
        1.0
    } else {
        0.0
    };
    let fy = if align.contains("YMid") {
        0.5
    } else if align.contains("YMax") {
        1.0
    } else {
        0.0
    };
    let tx = (w - vw * s) * fx;
    let ty = (h - vh * s) * fy;
    Affine::translate((tx, ty)) * Affine::scale(s) * Affine::translate((-x, -y))
}

/// Parse an element's `viewBox` attribute into `([min_x, min_y], width, height)`.
pub fn viewbox(n: roxmltree::Node) -> Option<([f64; 2], f64, f64)> {
    parse_viewbox(n.attribute("viewBox")?)
}

/// The `viewBox` -> viewport transform (`preserveAspectRatio`, default `xMidYMid meet`).
pub fn viewbox_transform(n: roxmltree::Node, w: f64, h: f64) -> Affine {
    let Some(vb) = viewbox(n) else { return Affine::IDENTITY };
    viewbox_transform_with(vb, w, h, n.attribute("preserveAspectRatio").unwrap_or("xMidYMid meet"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simple_css::parse as parse_css;

    #[test]
    fn declared_precedence_and_transform_exclusion() {
        let doc = roxmltree::Document::parse(
            r#"<svg xmlns="http://www.w3.org/2000/svg"><rect id="r" class="a" fill="black" transform="translate(5)" style="fill:blue"/></svg>"#,
        )
        .unwrap();
        let rect = doc.descendants().find(|n| n.tag_name().name() == "rect").unwrap();
        let mut css = StyleSheet::default();
        parse_css(".a { fill: green; stroke: red; }", &mut css).unwrap();
        css.index();
        let d = declared(rect, &css, |_| true);
        assert_eq!(d.get("fill").map(String::as_str), Some("blue"));
        assert_eq!(d.get("stroke").map(String::as_str), Some("red"));
        assert!(!d.contains_key("transform"));

        // SVG1.1 length syntax distinguishes CSS unit case from raw attributes:
        // https://www.w3.org/TR/SVG11/types.html#DataTypeLength
        let primary = roxmltree::Document::parse(
            r#"<svg xmlns="http://www.w3.org/2000/svg"><g id="lower" style="stroke-width:12pt"/><g id="upper" style="stroke-width:12PT"/></svg>"#,
        )
        .unwrap();
        for id in ["lower", "upper"] {
            let node = primary.descendants().find(|n| n.attribute("id") == Some(id)).unwrap();
            let mut st = Style::default();
            cascade(node, &StyleSheet::default(), &mut st, 200.0, 100.0, |_| true);
            assert_eq!(st.stroke_width, 16.0, "{id}: CSS unit case must not change stroke width");
        }

        let close = |actual: f64, expected: f64| {
            assert!((actual - expected).abs() <= 1e-12 * expected.abs().max(1.0), "{actual} != {expected}");
        };
        // Every supported suffix, through both CSS origins and every length property.
        // Absolute factors are CSS px/unit; relative factors use the inherited font.
        let units = [
            ("px", "pX", 1.0, 0.0),
            ("pt", "Pt", 4.0 / 3.0, 0.0),
            ("pc", "pC", 16.0, 0.0),
            ("mm", "Mm", 96.0 / 25.4, 0.0),
            ("cm", "cM", 96.0 / 2.54, 0.0),
            ("in", "In", 96.0, 0.0),
            ("em", "eM", 0.0, 1.0),
            ("ex", "Ex", 0.0, 0.5),
        ];
        for (unit, mixed, absolute, relative) in units {
            for inline in [false, true] {
                let separator = if inline { " " } else { "\t" };
                let values = format!(
                    "font-size:2{mixed};stroke-width:3{mixed};stroke-dasharray:1E1{mixed},{separator}+2.5{mixed};stroke-dashoffset:-5e-1{mixed}"
                );
                let inline_style = if inline { format!(" style=\"{values}\"") } else { String::new() };
                let xml = format!(
                    "<svg xmlns=\"http://www.w3.org/2000/svg\"><rect class=\"a\" font-size=\"7\" stroke-width=\"99\" stroke-dasharray=\"3 4\" stroke-dashoffset=\"9\"{inline_style}/></svg>"
                );
                let doc = roxmltree::Document::parse(&xml).unwrap();
                let rect = doc.descendants().find(|n| n.tag_name().name() == "rect").unwrap();
                let mut css = StyleSheet::default();
                let rule = if inline { ".a { stroke-width:1px; font-size:1px; }".to_owned() } else { format!(".a {{ {values} }}") };
                parse_css(&rule, &mut css).unwrap();
                css.index();
                let d = declared(rect, &css, |_| true);
                assert_eq!(d.get("font-size"), Some(&format!("2{unit}")));
                assert_eq!(d.get("stroke-width"), Some(&format!("3{unit}")));
                assert_eq!(d.get("stroke-dasharray"), Some(&format!("1E1{unit},{separator}+2.5{unit}")));
                assert_eq!(d.get("stroke-dashoffset"), Some(&format!("-5e-1{unit}")));
                let factor = |font| if relative == 0.0 { absolute } else { font * relative };
                let font = 2.0 * factor(20.0);
                let mut st = Style { font_size: 20.0, ..Style::default() };
                cascade(rect, &css, &mut st, 200.0, 100.0, |_| true);
                close(st.font_size, font);
                close(st.stroke_width, 3.0 * factor(font));
                let dash = st.dash.as_ref().unwrap();
                assert_eq!(dash.len(), 2);
                close(dash[0], 10.0 * factor(font));
                close(dash[1], 2.5 * factor(font));
                close(st.dash_offset, -0.5 * factor(font));
            }
        }

        let raw = roxmltree::Document::parse(
            r#"<svg xmlns="http://www.w3.org/2000/svg"><rect width="12PT" font-size="2EM" stroke-width="12PT" stroke-dasharray="1PT, 2PT" stroke-dashoffset="3PT"/></svg>"#,
        )
        .unwrap();
        let rect = raw.descendants().find(|n| n.tag_name().name() == "rect").unwrap();
        let d = declared(rect, &StyleSheet::default(), |_| true);
        assert_eq!(d.get("stroke-width").map(String::as_str), Some("12PT"));
        assert_eq!(d.get("font-size").map(String::as_str), Some("2EM"));
        let mut st = Style { font_size: 20.0, ..Style::default() };
        cascade(rect, &StyleSheet::default(), &mut st, 200.0, 100.0, |_| true);
        assert_eq!((st.font_size, st.stroke_width, st.dash_offset), (2.0, 12.0, 3.0));
        assert_eq!(st.dash, Some(vec![1.0, 2.0]));
        assert_eq!(attr_len(rect, "width", 'x', &st, 200.0, 100.0), 12.0);
        assert_eq!(length("12FURLONG", 'x', 20.0, 200.0, 100.0), Some(12.0));
        assert_eq!(length_strict("12PT", 100.0), None);
        assert_eq!(parse_font_size("2EM", 20.0, 100.0), None);
        assert_eq!(parse_dash_array("1PT 2PT", 100.0), None);

        let opaque = roxmltree::Document::parse(
            r##"<svg xmlns="http://www.w3.org/2000/svg"><rect style="stroke-width:var(--Case);font-size:1e999PT;stroke-dasharray:NaNPT, 2qU;stroke-dashoffset:12FURLONG;fill:url(#PaintID);font-family:MiXeD"/><g id="opaque-dash" style="stroke-dasharray:var(--Case, 12PT )"/></svg>"##,
        )
        .unwrap();
        let rect = opaque.descendants().find(|n| n.tag_name().name() == "rect").unwrap();
        let d = declared(rect, &StyleSheet::default(), |_| true);
        for (key, value) in [
            ("stroke-width", "var(--Case)"),
            ("font-size", "1e999PT"),
            ("stroke-dasharray", "NaNPT, 2qU"),
            ("stroke-dashoffset", "12FURLONG"),
            ("fill", "url(#PaintID)"),
            ("font-family", "MiXeD"),
        ] {
            assert_eq!(d.get(key).map(String::as_str), Some(value));
        }
        let opaque_dash = opaque.descendants().find(|n| n.attribute("id") == Some("opaque-dash")).unwrap();
        let d = declared(opaque_dash, &StyleSheet::default(), |_| true);
        assert_eq!(d.get("stroke-dasharray").map(String::as_str), Some("var(--Case, 12PT )"));

        // A normal matching rule is charged once, before its values are merged.
        let mut css = StyleSheet::default();
        parse_css(".a { stroke-width:12PT; }", &mut css).unwrap();
        css.index();
        let doc = roxmltree::Document::parse(
            r#"<svg xmlns="http://www.w3.org/2000/svg"><rect class="a" stroke-width="99" style="stroke-width:5pX"/></svg>"#,
        )
        .unwrap();
        let rect = doc.descendants().find(|n| n.tag_name().name() == "rect").unwrap();
        let mut charges = Vec::new();
        let d = declared(rect, &css, |cost| {
            charges.push(cost);
            true
        });
        assert_eq!(charges, [1, 1, 1, 1]);
        assert_eq!(d.get("stroke-width").map(String::as_str), Some("5px"));
    }

    #[test]
    fn length_units_and_exponents() {
        assert_eq!(length("10", 'x', 16.0, 200.0, 100.0), Some(10.0));
        assert_eq!(length("12pt", 'x', 16.0, 200.0, 100.0), Some(16.0));
        assert_eq!(length("1in", 'x', 16.0, 200.0, 100.0), Some(96.0));
        assert_eq!(length("2em", 'x', 16.0, 200.0, 100.0), Some(32.0));
        assert_eq!(length("2ex", 'x', 16.0, 200.0, 100.0), Some(16.0));
        assert_eq!(length("1e2em", 'x', 10.0, 200.0, 100.0), Some(1000.0));
        assert_eq!(length("50%", 'x', 16.0, 200.0, 100.0), Some(100.0));
        assert_eq!(length("50%", 'y', 16.0, 200.0, 100.0), Some(50.0));
        assert_eq!(length("nan", 'x', 16.0, 200.0, 100.0), None);
    }

    #[test]
    fn attr_len_reads_or_defaults_to_zero() {
        let doc = roxmltree::Document::parse(r#"<svg xmlns="http://www.w3.org/2000/svg"><rect x="50%" y="bad"/></svg>"#).unwrap();
        let rect = doc.descendants().find(|n| n.tag_name().name() == "rect").unwrap();
        let st = Style::default();
        assert_eq!(attr_len(rect, "x", 'x', &st, 200.0, 100.0), 100.0);
        assert_eq!(attr_len(rect, "y", 'y', &st, 200.0, 100.0), 0.0);
        assert_eq!(attr_len(rect, "width", 'x', &st, 200.0, 100.0), 0.0);
    }

    #[test]
    fn css_value_identifier_case() {
        // CSS Values 3 section 3.1: keywords are ASCII case-insensitive. CSS Syntax 3
        // section 4.3.4 matches the url function identifier without folding its payload.
        let seed = Style {
            fill: Some(PaintRef::Color([0.0, 1.0, 0.0, 1.0])),
            stroke: Some(PaintRef::Color([0.0, 0.0, 1.0, 1.0])),
            color: [1.0, 0.0, 0.0, 1.0],
            ..Style::default()
        };
        let evaluate = |property: &str, value: &str, inline: bool| {
            let declaration = format!("{property}:{value}");
            let inline_style =
                if inline { format!(" style=\"{}\"", declaration.replace('&', "&amp;").replace('"', "&quot;")) } else { String::new() };
            let xml = format!("<svg xmlns=\"http://www.w3.org/2000/svg\"><rect class=\"a\"{inline_style}/></svg>");
            let doc = roxmltree::Document::parse(&xml).unwrap();
            let rect = doc.descendants().find(|n| n.tag_name().name() == "rect").unwrap();
            let mut css = StyleSheet::default();
            if !inline {
                parse_css(&format!(".a {{ {declaration} }}"), &mut css).unwrap();
                css.index();
            }
            let d = declared(rect, &css, |_| true);
            let mut st = seed.clone();
            let (opacity, display) = cascade(rect, &css, &mut st, 200.0, 100.0, |_| true);
            assert_eq!(opacity, 1.0);
            (d, st, display)
        };
        let cases = [
            ("display", "none", "NONE", seed.clone(), false),
            ("visibility", "hidden", "HIDDEN", Style { visible: false, ..seed.clone() }, true),
            ("visibility", "collapse", "CoLlApSe", Style { visible: false, ..seed.clone() }, true),
            ("visibility", "visible", "VISIBLE", seed.clone(), true),
            ("fill-rule", "evenodd", "EVENODD", Style { fill_rule: FillRule::EvenOdd, ..seed.clone() }, true),
            ("fill-rule", "nonzero", "NONZERO", seed.clone(), true),
            ("stroke-linecap", "round", "ROUND", Style { cap: Cap::Round, ..seed.clone() }, true),
            ("stroke-linecap", "square", "SqUaRe", Style { cap: Cap::Square, ..seed.clone() }, true),
            ("stroke-linecap", "butt", "BUTT", seed.clone(), true),
            ("stroke-linejoin", "round", "ROUND", Style { join: Join::Round, ..seed.clone() }, true),
            ("stroke-linejoin", "bevel", "BeVeL", Style { join: Join::Bevel, ..seed.clone() }, true),
            ("stroke-linejoin", "miter", "MITER", seed.clone(), true),
            ("fill", "none", "NONE", Style { fill: None, ..seed.clone() }, true),
            ("stroke", "none", "NoNe", Style { stroke: None, ..seed.clone() }, true),
            ("fill", "currentColor", "CURRENTCOLOR", Style { fill: Some(PaintRef::Current), ..seed.clone() }, true),
            ("stroke", "currentColor", "CuRrEnTcOlOr", Style { stroke: Some(PaintRef::Current), ..seed.clone() }, true),
            ("fill", "url('#PaintID')", "URL('#PaintID')", Style { fill: Some(PaintRef::Url("PaintID".into(), None)), ..seed.clone() }, true),
            (
                "stroke",
                "url(\"#StrokeID\") red",
                "uRl(\"#StrokeID\") red",
                Style { stroke: Some(PaintRef::Url("StrokeID".into(), Some([1.0, 0.0, 0.0, 1.0]))), ..seed.clone() },
                true,
            ),
        ];
        // Every lower-case functional control runs before collecting any failing counterpart.
        for (property, lower, _, expected, display) in &cases {
            for inline in [false, true] {
                let (_, actual, actual_display) = evaluate(property, lower, inline);
                assert_eq!(&actual, expected, "lower control {property}:{lower}, inline={inline}");
                assert_eq!(&actual_display, display);
            }
        }
        let mut mismatches = Vec::new();
        for (property, _, mixed, expected, display) in &cases {
            for inline in [false, true] {
                let (_, actual, actual_display) = evaluate(property, mixed, inline);
                if &actual != expected || &actual_display != display {
                    let origin = if inline { "inline" } else { "stylesheet" };
                    let sector = if mixed.get(..4).is_some_and(|prefix| prefix.eq_ignore_ascii_case("url(")) { "url" } else { "keyword" };
                    mismatches.push(format!("{property} {sector} {origin}: {mixed}, style={actual:?}, display={actual_display}"));
                }
            }
        }
        // Preserve raw XML/direct-helper compatibility, including the case-sensitive ID.
        let raw = roxmltree::Document::parse(
            r##"<svg xmlns="http://www.w3.org/2000/svg"><rect display="NONE" visibility="HIDDEN" fill-rule="EVENODD" stroke-linecap="ROUND" stroke-linejoin="BEVEL" fill="URL('#PaintID')" stroke="CURRENTCOLOR"/></svg>"##,
        )
        .unwrap();
        let rect = raw.descendants().find(|n| n.tag_name().name() == "rect").unwrap();
        let d = declared(rect, &StyleSheet::default(), |_| true);
        for attribute in rect.attributes() {
            assert_eq!(d.get(attribute.name()).map(String::as_str), Some(attribute.value()));
        }
        let mut raw_style = seed.clone();
        assert_eq!(cascade(rect, &StyleSheet::default(), &mut raw_style, 200.0, 100.0, |_| true), (1.0, true));
        assert_eq!(raw_style, seed);
        assert_eq!(paint_ref("url('#PaintID')"), Some(Some(PaintRef::Url("PaintID".into(), None))));
        assert_eq!(paint_ref("URL('#PaintID')"), None);
        assert_eq!(paint_ref("NONE"), None);
        assert_eq!(paint_ref("CURRENTCOLOR"), None);
        // Unsupported expressions and unrelated values must retain their exact spelling.
        for (property, value) in [
            ("fill", "var(--Case, URL('#PaintID'))"),
            ("stroke", "PaintFunction('#PaintID')"),
            ("fill", "\"URL(#PaintID)\""),
            ("display", "var(--Case, NONE)"),
            ("visibility", "inherit"),
            ("font-size", "LARGE"),
            ("color", "ReD"),
        ] {
            for inline in [false, true] {
                let (d, _, _) = evaluate(property, value, inline);
                assert_eq!(d.get(property).map(String::as_str), Some(value));
            }
        }
        // The suffix includes quotes, URI/fragment case and the complete fallback spelling.
        for property in ["fill", "stroke"] {
            for inline in [false, true] {
                let value = "URL('Case/Art.svg#PaintID') ReD";
                let (d, _, _) = evaluate(property, value, inline);
                if d.get(property).map(String::as_str) != Some("url('Case/Art.svg#PaintID') ReD") {
                    let origin = if inline { "inline" } else { "stylesheet" };
                    mismatches.push(format!("{property} url {origin}: identifier/payload spelling"));
                }
            }
        }
        assert!(mismatches.is_empty(), "CSS identifier case mismatches: {mismatches:?}");
    }

    #[test]
    fn cascade_applies_inheritance_and_display() {
        let doc = roxmltree::Document::parse(
            r##"<svg xmlns="http://www.w3.org/2000/svg"><g color="red" font-size="20" fill="currentColor" stroke="#00ff00" stroke-width="2em" stroke-dasharray="4 2" fill-rule="evenodd" stroke-linecap="round" stroke-linejoin="bevel" opacity="50%" display="none" visibility="hidden"/></svg>"##,
        )
        .unwrap();
        let g = doc.descendants().find(|n| n.tag_name().name() == "g").unwrap();
        let css = StyleSheet::default();
        let mut st = Style::default();
        let (opacity, display) = cascade(g, &css, &mut st, 100.0, 100.0, |_| true);
        assert_eq!(opacity, 0.5);
        assert!(!display);
        assert!(!st.visible);
        assert_eq!(st.font_size, 20.0);
        assert_eq!(st.stroke_width, 40.0);
        assert_eq!(st.fill, Some(PaintRef::Current));
        assert_eq!(st.fill_rule, FillRule::EvenOdd);
        assert_eq!(st.cap, Cap::Round);
        assert_eq!(st.join, Join::Bevel);
        assert_eq!(st.dash, Some(vec![4.0, 2.0]));
    }

    #[test]
    fn paint_ref_and_resolve_paint_handle_fallbacks() {
        assert_eq!(paint_ref("none"), Some(None));
        assert_eq!(paint_ref("inherit"), None);
        assert_eq!(paint_ref("currentColor"), Some(Some(PaintRef::Current)));
        assert_eq!(paint_ref("url(#grad) red"), Some(Some(PaintRef::Url("grad".into(), Some([1.0, 0.0, 0.0, 1.0])))));

        let st = Style { color: [0.0, 1.0, 0.0, 0.5], ..Style::default() };
        assert_eq!(resolve_paint(&PaintRef::Current, &st, |_, _| UrlPaint::<()>::None), Some(ResolvedPaint::Color([0.0, 1.0, 0.0], 0.5)));

        let url = PaintRef::Url("g".into(), Some([1.0, 0.0, 0.0, 1.0]));
        assert_eq!(resolve_paint(&url, &st, |_, _| UrlPaint::<&str>::Server("ok")), Some(ResolvedPaint::Server("ok")));
        assert_eq!(resolve_paint(&url, &st, |_, _| UrlPaint::<&str>::Fallback), Some(ResolvedPaint::Color([1.0, 0.0, 0.0], 1.0)));
        assert_eq!(resolve_paint(&url, &st, |_, _| UrlPaint::<&str>::None), None);
    }

    #[test]
    fn viewbox_and_transform_preserve_aspect_ratio() {
        let doc = roxmltree::Document::parse(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="10 20 100 50" preserveAspectRatio="defer xMaxYMid meet"/>"#,
        )
        .unwrap();
        let root = doc.root_element();
        assert_eq!(viewbox(root), Some(([10.0, 20.0], 100.0, 50.0)));
        let m = viewbox_transform(root, 200.0, 200.0);
        // Scale is min(2, 4) = 2; tx = (200 - 200) * 1.0 = 0; ty = (200 - 100) * 0.5 = 50.
        let p = m * kurbo::Point::new(10.0, 20.0);
        assert_eq!(p, kurbo::Point::new(0.0, 50.0));
    }
}
