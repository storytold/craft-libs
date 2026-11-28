//! Portable XFDF annotation and FDF grammar (no PDF COS document or annotation models).

use std::fmt::Write as _;

use craft_xml::esc_attr as esc;

use crate::{check_form_xml_depth, el_text, parse_form_xml, xml_nesting_depth};

/// XFDF's own namespace (ISO 19444-1).
pub const XFDF_NS: &str = "http://ns.adobe.com/xfdf/";

/// Rich text deeper than this is written as text rather than as markup, so the XFDF it goes into
/// stays well inside the nesting the reader accepts (`MAX_XML_DEPTH`).
const MAX_RICH_DEPTH: usize = 512;

/// PDF annotation `/Subtype` names paired with their XFDF element names.
pub const SUBTYPES: [(&str, &str); 17] = [
    ("Text", "text"),
    ("FreeText", "freetext"),
    ("Line", "line"),
    ("Square", "square"),
    ("Circle", "circle"),
    ("Polygon", "polygon"),
    ("PolyLine", "polyline"),
    ("Highlight", "highlight"),
    ("Underline", "underline"),
    ("Squiggly", "squiggly"),
    ("StrikeOut", "strikeout"),
    ("Stamp", "stamp"),
    ("Caret", "caret"),
    ("Ink", "ink"),
    ("FileAttachment", "fileattachment"),
    ("Sound", "sound"),
    ("Redact", "redact"),
];

/// PDF annotation `/F` bitflags paired with their XFDF `flags` attribute tokens.
pub const FLAGS: [(i64, &str); 9] = [
    (1, "invisible"),
    (2, "hidden"),
    (4, "print"),
    (8, "nozoom"),
    (16, "norotate"),
    (32, "noview"),
    (64, "readonly"),
    (128, "locked"),
    (256, "togglenoview"),
];

/// PDF annotation border style (`/BS /S`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BorderStyle {
    Solid,
    Dash,
    Bevelled,
    Inset,
    Underline,
}

impl BorderStyle {
    /// Return the PDF `/BS /S` name for this border style.
    pub fn as_pdf_name(self) -> &'static str {
        match self {
            Self::Solid => "S",
            Self::Dash => "D",
            Self::Bevelled => "B",
            Self::Inset => "I",
            Self::Underline => "U",
        }
    }

    fn as_xfdf_attr(self) -> Option<&'static str> {
        match self {
            Self::Dash => Some("dash"),
            Self::Bevelled => Some("bevelled"),
            Self::Inset => Some("inset"),
            Self::Underline => Some("underline"),
            Self::Solid => None,
        }
    }
}

/// PDF Caret annotation `/Sy` symbol.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaretSymbol {
    Paragraph,
    None,
}

impl CaretSymbol {
    /// Return the PDF `/Sy` name for this caret symbol.
    pub fn as_pdf_name(self) -> &'static str {
        match self {
            Self::Paragraph => "P",
            Self::None => "None",
        }
    }
}

/// Portable XFDF 3.0 annotation formatting attributes.
#[derive(Default, Debug, PartialEq)]
pub struct AnnotFormat {
    pub width: Option<f64>,
    pub border_style: Option<BorderStyle>,
    pub dashes: Option<Vec<f64>>,
    pub cloudy_intensity: Option<Option<f64>>,
    pub rotation: Option<i64>,
    pub justification: Option<i64>,
    pub fringe: Option<[f64; 4]>,
    pub leader_length: Option<f64>,
    pub leader_extend: Option<f64>,
    pub leader_offset: Option<f64>,
    pub caption: Option<bool>,
    pub caption_style: Option<String>,
    pub caption_offset: Option<[f64; 2]>,
    pub symbol: Option<CaretSymbol>,
    pub overlay_text: Option<String>,
    pub overlay_text_repeat: Option<bool>,
}

fn format_num(v: f64) -> String {
    let s = format!("{v:.4}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.into() }
}

fn parse_csv(list: &str) -> Vec<f64> {
    let v: Vec<f64> = list.split([',', ';', ' ']).filter(|t| !t.is_empty()).filter_map(|t| t.trim().parse().ok()).collect();
    if v.iter().all(|x| x.is_finite()) { v } else { Vec::new() }
}

fn hex_color(c: &[f64]) -> Option<String> {
    let rgb = match c.len() {
        1 => [c[0]; 3],
        3 => [c[0], c[1], c[2]],
        4 => [(1.0 - c[0]) * (1.0 - c[3]), (1.0 - c[1]) * (1.0 - c[3]), (1.0 - c[2]) * (1.0 - c[3])],
        _ => return None,
    };
    Some(format!(
        "#{:02X}{:02X}{:02X}",
        (rgb[0].clamp(0.0, 1.0) * 255.0).round() as u8,
        (rgb[1].clamp(0.0, 1.0) * 255.0).round() as u8,
        (rgb[2].clamp(0.0, 1.0) * 255.0).round() as u8
    ))
}

/// Format XFDF 3.0 annotation formatting attributes into `attrs`.
pub fn write_format_attrs(fmt: &AnnotFormat, is_free_text: bool, attrs: &mut String) {
    let list = |v: &[f64]| v.iter().map(|x| format_num(*x)).collect::<Vec<_>>().join(",");
    if let Some(w) = fmt.width.filter(|w| w.is_finite()) {
        let _ = write!(attrs, " width=\"{}\"", format_num(w));
    }
    if let Some(intensity) = fmt.cloudy_intensity {
        attrs.push_str(" style=\"cloudy\"");
        if let Some(i) = intensity.filter(|i| i.is_finite()) {
            let _ = write!(attrs, " intensity=\"{}\"", format_num(i));
        }
    } else if let Some(s) = fmt.border_style.and_then(BorderStyle::as_xfdf_attr) {
        let _ = write!(attrs, " style=\"{s}\"");
    }
    if let Some(dashes) = &fmt.dashes
        && !dashes.is_empty()
        && dashes.iter().all(|v| v.is_finite())
    {
        let _ = write!(attrs, " dashes=\"{}\"", list(dashes));
    }
    if let Some(r) = fmt.rotation {
        let _ = write!(attrs, " rotation=\"{r}\"");
    }
    if let Some(q) = fmt.justification {
        match (is_free_text, q) {
            (true, 0) => attrs.push_str(" justification=\"left\""),
            (true, 1) => attrs.push_str(" justification=\"centered\""),
            (true, 2) => attrs.push_str(" justification=\"right\""),
            (true, _) => {}
            (false, q) => {
                let _ = write!(attrs, " justification=\"{q}\"");
            }
        }
    }
    if let Some(rd) = fmt.fringe
        && rd.iter().all(|v| v.is_finite())
    {
        let _ = write!(attrs, " fringe=\"{}\"", list(&rd));
    }
    if let Some(ll) = fmt.leader_length.filter(|v| v.is_finite()) {
        let _ = write!(attrs, " leaderLength=\"{}\"", format_num(ll));
    }
    if let Some(lle) = fmt.leader_extend.filter(|v| v.is_finite()) {
        let _ = write!(attrs, " leaderExtend=\"{}\"", format_num(lle));
    }
    if let Some(llo) = fmt.leader_offset.filter(|v| v.is_finite()) {
        let _ = write!(attrs, " leader-offset=\"{}\"", format_num(llo));
    }
    if let Some(cap) = fmt.caption {
        let _ = write!(attrs, " caption=\"{}\"", if cap { "yes" } else { "no" });
    }
    if let Some(cp) = &fmt.caption_style {
        let _ = write!(attrs, " caption-style=\"{}\"", esc(cp));
    }
    if let Some([h, v]) = fmt.caption_offset
        && h.is_finite()
        && v.is_finite()
    {
        let _ = write!(attrs, " caption-offset-h=\"{}\" caption-offset-v=\"{}\"", format_num(h), format_num(v));
    }
    match fmt.symbol {
        Some(CaretSymbol::Paragraph) => attrs.push_str(" symbol=\"paragraph\""),
        Some(CaretSymbol::None) => attrs.push_str(" symbol=\"none\""),
        None => {}
    }
    if let Some(t) = &fmt.overlay_text {
        let _ = write!(attrs, " overlay-text=\"{}\"", esc(t));
    }
    if let Some(repeat) = fmt.overlay_text_repeat {
        let _ = write!(attrs, " overlay-text-repeat=\"{repeat}\"");
    }
}

/// Parse XFDF 3.0 annotation formatting attributes from an XML element.
pub fn parse_format_attrs(node: &craft_xml::El) -> AnnotFormat {
    let num = |a: &str| node.attr(a).and_then(|v| v.trim().parse::<f64>().ok()).filter(|v| v.is_finite());
    let style = node.attr("style").map(str::trim);
    let dashes = node.attr("dashes").map(parse_csv).filter(|v| !v.is_empty() && v.iter().all(|x| *x >= 0.0) && v.iter().any(|x| *x > 0.0));
    let border_style = match style {
        Some("dash") => Some(BorderStyle::Dash),
        Some("bevelled" | "beveled") => Some(BorderStyle::Bevelled),
        Some("inset") => Some(BorderStyle::Inset),
        Some("underline") => Some(BorderStyle::Underline),
        Some("solid") => Some(BorderStyle::Solid),
        _ => None,
    };
    let cloudy_intensity = (style == Some("cloudy")).then(|| Some(num("intensity").unwrap_or(1.0).clamp(0.0, 2.0)));
    let (h, v) = (num("caption-offset-h"), num("caption-offset-v"));
    AnnotFormat {
        width: num("width"),
        border_style,
        dashes,
        cloudy_intensity,
        rotation: node.attr("rotation").and_then(|r| r.trim().parse::<i32>().ok()).map(i64::from),
        justification: match node.attr("justification").map(str::trim) {
            Some("left" | "0") => Some(0),
            Some("centered" | "center" | "1") => Some(1),
            Some("right" | "2") => Some(2),
            _ => None,
        },
        fringe: node.attr("fringe").map(parse_csv).and_then(|r| r.try_into().ok()),
        leader_length: num("leaderLength"),
        leader_extend: num("leaderExtend").filter(|v| *v >= 0.0),
        leader_offset: num("leader-offset").filter(|v| *v >= 0.0),
        caption: match node.attr("caption").map(str::trim) {
            Some("yes" | "true") => Some(true),
            Some("no" | "false") => Some(false),
            _ => None,
        },
        caption_style: node.attr("caption-style").map(str::trim).filter(|c| matches!(*c, "Inline" | "Top")).map(str::to_owned),
        caption_offset: (h.is_some() || v.is_some()).then(|| [h.unwrap_or(0.0), v.unwrap_or(0.0)]),
        symbol: match node.attr("symbol").map(str::trim) {
            Some("paragraph" | "P") => Some(CaretSymbol::Paragraph),
            Some("none" | "None") => Some(CaretSymbol::None),
            _ => None,
        },
        overlay_text: node.attr("overlay-text").map(str::to_owned),
        overlay_text_repeat: match node.attr("overlay-text-repeat").map(str::trim) {
            Some("true" | "yes") => Some(true),
            Some("false" | "no") => Some(false),
            _ => None,
        },
    }
}

/// Format PDF `/F` annotation bitflags as a comma-separated XFDF `flags` string.
pub fn format_flags(flags: i64) -> String {
    FLAGS.iter().filter(|(b, _)| flags & b != 0).map(|(_, s)| *s).collect::<Vec<_>>().join(",")
}

/// Parse an XFDF `flags` attribute into PDF `/F` annotation bitflags.
pub fn parse_flags(attr: Option<&str>) -> i64 {
    attr.unwrap_or("print")
        .split(',')
        .filter_map(|f| FLAGS.iter().find(|(_, s)| s.eq_ignore_ascii_case(f.trim())).map(|x| x.0))
        .fold(0, |bits, flag| bits | flag)
}

/// Pairs `x,y;x,y;…` (XFDF vertices and ink gestures).
pub fn pairs(text: &str) -> Vec<[f64; 2]> {
    text.split(';').filter_map(|p| if let [x, y] = parse_csv(p).as_slice() { Some([*x, *y]) } else { None }).collect()
}

/// Format coordinate pairs as `x,y;x,y;…`.
pub fn format_pairs(pts: &[[f64; 2]]) -> String {
    pts.iter().map(|p| format!("{},{}", format_num(p[0]), format_num(p[1]))).collect::<Vec<_>>().join(";")
}

/// Rich text (`/RC`) as `<contents-richtext>` holds it: its root element alone (no XML
/// declaration), when the text is well-formed XML; `None` otherwise.
pub fn rich_text_element(rc: &str) -> Option<String> {
    let t = rc.trim_start_matches('\u{feff}');
    if xml_nesting_depth(t) > MAX_RICH_DEPTH {
        return None;
    }
    let parsed = parse_form_xml(t).ok()?;
    Some(parsed.to_xml())
}

/// Recursively extract plain text from a `<contents-richtext>` element, a `\r`-separated line
/// per paragraph or `<br>` (what `/Contents` gets when `<contents-richtext>` comes without
/// `<contents>`). Unlike [`direct_text`], this walks descendant elements.
pub fn rich_plain(node: &craft_xml::El) -> String {
    fn walk(node: &craft_xml::El, out: &mut String) {
        for n in &node.children {
            match n {
                craft_xml::Node::El(el) => {
                    if el.local() == "br" || (matches!(el.local(), "p" | "div" | "li") && !out.is_empty() && !out.ends_with('\r')) {
                        out.push('\r');
                    }
                    walk(el, out);
                }
                craft_xml::Node::Text(t) if !(t.trim().is_empty() && t.contains('\n')) => {
                    out.push_str(&t.replace("\r\n", "\r").replace('\n', "\r"));
                }
                _ => {}
            }
        }
    }
    let mut out = String::new();
    walk(node, &mut out);
    out.trim_end_matches('\r').to_string()
}

/// The one element `markup` (cut out of `rich`, a `<contents-richtext>`) as an XML
/// document of its own: the namespaces it inherits from the XFDF around it are declared on it.
pub fn standalone(el: &craft_xml::El, ns: &[(String, String)]) -> String {
    let mut clone = (*el).clone();
    let markup = clone.to_xml();
    if let Some((_, uri)) = ns.iter().rfind(|(p, _)| p.is_empty())
        && !uri.is_empty()
        && uri != XFDF_NS
        && !clone.has_attr("xmlns")
    {
        clone.set_attr("xmlns", uri);
    }
    for (prefix, uri) in ns {
        if prefix.is_empty() || prefix == "xml" {
            continue;
        }
        let attr_name = format!("xmlns:{prefix}");
        if markup.contains(&format!("{prefix}:")) && !clone.has_attr(&attr_name) {
            clone.set_attr(&attr_name, uri);
        }
    }
    clone.to_xml()
}

/// Whether `text` is XFDF with comments in it (what a paste of copied comments carries).
pub fn is_xfdf_comments(text: &str) -> bool {
    let head: String = text.chars().take(400).collect();
    head.contains("<xfdf") && text.contains("<annots")
}

/// Every comment element of an XFDF text, in order (none when it is no XFDF).
pub fn with_comment_nodes(text: &str, mut f: impl FnMut(&craft_xml::El)) {
    if check_form_xml_depth(text).is_err() {
        return;
    }
    let Ok(root) = parse_form_xml(text) else { return };
    if root.local() != "xfdf" {
        return;
    }
    let Some(annots) = root.child(craft_xml::Local("annots")) else { return };
    for node in annots.elements() {
        if SUBTYPES.iter().any(|(_, t)| *t == node.local().to_ascii_lowercase()) {
            f(node);
        }
    }
}

/// Whether an annotation dictionary key is skipped when exporting into FDF (ISO 32000-2 §12.7.8.3.4).
pub fn fdf_annot_skip_key(key: &[u8], is_stamp: bool) -> bool {
    matches!(key, b"P" | b"Popup" | b"IRT" | b"Parent" | b"FS" | b"Sound") || (key == b"AP" && !is_stamp)
}

/// Format a colour attribute (` name="#RRGGBB"`) when `color` is 1, 3, or 4 components.
pub fn format_color_attr(name: &str, color: &[f64], out: &mut String) {
    if let Some(c) = hex_color(color) {
        let _ = write!(out, " {name}=\"{c}\"");
    }
}

/// Concatenate only the direct text children of `node`, without recursing into child elements
/// (unlike [`rich_plain`]).
pub fn direct_text(node: &craft_xml::El) -> String {
    el_text(node)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_attrs_round_trip() {
        let fmt = AnnotFormat {
            width: Some(2.5),
            border_style: Some(BorderStyle::Dash),
            dashes: Some(vec![3.0, 1.5]),
            rotation: Some(90),
            justification: Some(1),
            caption: Some(true),
            symbol: Some(CaretSymbol::Paragraph),
            overlay_text: Some("REDACT".into()),
            overlay_text_repeat: Some(true),
            ..AnnotFormat::default()
        };
        let mut attrs = String::new();
        write_format_attrs(&fmt, true, &mut attrs);
        let xml = format!("<freetext{attrs}/>");
        let parsed_xml = parse_form_xml(&xml);
        assert!(parsed_xml.is_ok(), "failed to parse generated attributes: {xml}");
        if let Ok(el) = parsed_xml {
            let parsed = parse_format_attrs(&el);
            assert_eq!(parsed.width, Some(2.5));
            assert_eq!(parsed.border_style, Some(BorderStyle::Dash));
            assert_eq!(parsed.dashes.as_deref(), Some(&[3.0, 1.5][..]));
            assert_eq!(parsed.rotation, Some(90));
            assert_eq!(parsed.justification, Some(1));
            assert_eq!(parsed.caption, Some(true));
            assert_eq!(parsed.symbol, Some(CaretSymbol::Paragraph));
            assert_eq!(parsed.overlay_text.as_deref(), Some("REDACT"));
            assert_eq!(parsed.overlay_text_repeat, Some(true));
        }
    }

    #[test]
    fn flags_and_pairs_and_fdf_skip_keys() {
        assert_eq!(format_flags(parse_flags(Some("print,locked"))), "print,locked");
        assert_eq!(pairs("1,2;3.5,4"), vec![[1.0, 2.0], [3.5, 4.0]]);
        assert_eq!(format_pairs(&[[1.0, 2.0], [3.5, 4.0]]), "1,2;3.5,4");
        assert!(fdf_annot_skip_key(b"AP", false));
        assert!(!fdf_annot_skip_key(b"AP", true));
        assert!(fdf_annot_skip_key(b"Popup", true));
        assert!(is_xfdf_comments("<xfdf><annots><text/></annots></xfdf>"));
    }
}
