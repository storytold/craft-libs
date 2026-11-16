//! Office Math Markup (OMML, ECMA-376 Part 1 §22.1) read into and written from the equation tree
//! ([`crate::math`]).
//!
//! The reader takes an `m:oMath` element of a [`craft_xml`] tree read with the canonical
//! prefixes of [`NAMESPACES`] (`m:` for math, `w:` for WordprocessingML, `mc:` for markup
//! compatibility), whatever prefixes the file used; [`parse`] does that for a string. It keeps
//! the source XML in [`Math::omml`], and the writer writes that back unchanged while the
//! equation's colour is the same, so a saved file carries the equation exactly as Word wrote it.
//! The writer otherwise writes the tree: `m:oMath`, inside `m:oMathPara` for a display equation.

use craft_xml::{Canonical, El, Options};

use crate::math::{Math, MathJc};
use crate::rgb::Rgb;

mod read;
mod write;

pub use read::{lift_color, para_jc, read_math_pr, read_omath};
pub use write::{math_pr, resolve_numbers, write_equation};

/// The OMML namespace (transitional), and the strict one.
pub const MATH_NS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/math";
pub const MATH_NS_STRICT: &str = "http://purl.oclc.org/ooxml/officeDocument/math";
/// The WordprocessingML namespace OMML runs borrow (`w:rPr`, `w:t`, `w:sym`), transitional and
/// strict.
pub const W_NS: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
pub const W_NS_STRICT: &str = "http://purl.oclc.org/ooxml/wordprocessingml/main";
/// Markup compatibility (`mc:AlternateContent`).
pub const MC_NS: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";

/// The canonical prefixes the reader expects, for [`craft_xml::Canonical`].
pub const NAMESPACES: &[(&str, &str)] = &[("m", MATH_NS), ("m", MATH_NS_STRICT), ("w", W_NS), ("w", W_NS_STRICT), ("mc", MC_NS)];

/// Most elements [`parse`] reads from one string.
pub const MAX_ELEMENTS: usize = 250_000;

/// OMML text read into a tree with the canonical prefixes of [`NAMESPACES`]: the first
/// `m:oMathPara` or `m:oMath` element in it (the root or any descendant). `None` when there is
/// none, or the text is not XML. Text kept by the reader ([`Math::omml`]) declares no
/// namespaces; its prefixes are already the canonical ones and are read as written.
pub fn parse(xml: &str) -> Option<El> {
    let canonical = Canonical::new(NAMESPACES);
    let limits = craft_xml::Limits { max_nodes: MAX_ELEMENTS.saturating_mul(2), ..craft_xml::Limits::DEFAULT };
    let first = |root: El| -> Option<El> {
        if matches!(root.name.as_str(), "m:oMath" | "m:oMathPara") {
            return Some(root);
        }
        root.find("m:oMathPara").or_else(|| root.find("m:oMath")).cloned()
    };
    let read = Options::canonical(&canonical).with_limits(limits).parse(xml.as_bytes()).ok().and_then(first);
    read.or_else(|| parse_kept(xml).and_then(first))
}

/// Kept OMML (canonical prefixes, no declarations) read as written.
fn parse_kept(xml: &str) -> Option<El> {
    let limits = craft_xml::Limits { max_nodes: MAX_ELEMENTS.saturating_mul(2), ..craft_xml::Limits::DEFAULT };
    Options { ns_decls: false, ..Options::default() }.with_limits(limits).parse(xml.as_bytes()).ok()
}

/// Is `xml` (kept OMML) well-formed enough to write back as it is?
fn is_well_formed(xml: &str) -> bool {
    xml.starts_with("<m:oMath") && Options::default().strict().parse(xml.as_bytes()).is_ok()
}

/// An equation from OMML text ([`parse`]): an `m:oMathPara`'s first `m:oMath` with the
/// paragraph's justification, or an `m:oMath`. `None` when there is none.
pub fn read_str(xml: &str) -> Option<Math> {
    let e = parse(xml)?;
    if e.name == "m:oMathPara" {
        let jc = para_jc(&e);
        return e.find("m:oMath").map(|m| read_omath(m, jc));
    }
    Some(read_omath(&e, MathJc::default()))
}

/// An equation as OMML text ([`write_equation`] into a string, no XML declaration).
pub fn write_str(linear: &str, display: bool, math: &Math, color: Option<Rgb>) -> String {
    let mut w = W::default();
    write_equation(&mut w, linear, display, math, color);
    w.s
}

/// OMML on/off property of WordprocessingML (`w:b`, `w:i`): absent `w:val` = on.
fn on_off(e: &El) -> bool {
    match e.attr_or_plain("w:val") {
        None => true,
        Some(v) => !matches!(v.trim(), "0" | "false" | "off" | "False" | "FALSE"),
    }
}

/// Largest absolute length accepted, points (200 inches).
const MAX_LEN_PT: f32 = 14_400.0;

/// A twips value (`m:wrapIndent`), with or without an ISO 29500 universal measure (`1in`,
/// `2.5cm`, `12pt`), in points.
fn measure_twips(s: Option<&str>) -> Option<f32> {
    let t = s?.trim();
    if t.is_empty() || t.len() > 40 {
        return None;
    }
    let bytes = t.as_bytes();
    let mut split = t.len();
    let mut in_exp = false;
    for (i, &b) in bytes.iter().enumerate() {
        if b.is_ascii_digit() || b == b'.' {
            in_exp = false;
            continue;
        }
        if (b == b'-' || b == b'+') && (i == 0 || in_exp) {
            in_exp = false;
            continue;
        }
        if b == b'e' || b == b'E' {
            let rest = bytes.get(i + 1..).unwrap_or_default();
            let after_sign = match rest.first() {
                Some(&s) if s == b'+' || s == b'-' => rest.get(1..).unwrap_or_default(),
                _ => rest,
            };
            if after_sign.first().is_some_and(u8::is_ascii_digit) {
                in_exp = true;
                continue;
            }
        }
        split = i;
        break;
    }
    let (num, unit) = t.split_at(split);
    let v: f64 = num.parse().ok()?;
    if !v.is_finite() {
        return None;
    }
    let pt = match unit.trim().to_ascii_lowercase().as_str() {
        "" => v / 20.0,
        "pt" => v,
        "in" => v * 72.0,
        "cm" => v * 72.0 / 2.54,
        "mm" => v * 72.0 / 25.4,
        "pc" | "pi" => v * 12.0,
        "px" => v * 0.75,
        "emu" => v / 12_700.0,
        _ => return None,
    };
    let pt = pt as f32;
    pt.is_finite().then_some(pt.clamp(-MAX_LEN_PT, MAX_LEN_PT))
}

/// Points as a twips attribute value.
fn twips(pt: f32) -> String {
    let pt = pt.clamp(-MAX_LEN_PT, MAX_LEN_PT);
    let pt = if pt.is_finite() { pt } else { 0.0 };
    ((pt * 20.0).round() as i64).to_string()
}

fn xml_char_ok(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\r' | '\u{20}'..='\u{D7FF}' | '\u{E000}'..='\u{FFFD}' | '\u{10000}'..='\u{10FFFF}')
}

/// Escape text for element content or attribute values (drops characters XML can't carry).
fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&apos;"),
            '\t' => o.push_str("&#9;"),
            '\n' => o.push_str("&#10;"),
            '\r' => o.push_str("&#13;"),
            c if xml_char_ok(c) => o.push(c),
            _ => {}
        }
    }
    o
}

/// A string-building XML writer, as WordprocessingML writers write: names as given, values
/// escaped.
#[derive(Default)]
pub struct W {
    pub s: String,
}

impl W {
    fn attrs(&mut self, attrs: &[(&str, &str)]) {
        for (k, v) in attrs {
            self.s.push(' ');
            self.s.push_str(k);
            self.s.push_str("=\"");
            self.s.push_str(&esc(v));
            self.s.push('"');
        }
    }
    pub fn open(&mut self, name: &str, attrs: &[(&str, &str)]) {
        self.s.push('<');
        self.s.push_str(name);
        self.attrs(attrs);
        self.s.push('>');
    }
    pub fn empty(&mut self, name: &str, attrs: &[(&str, &str)]) {
        self.s.push('<');
        self.s.push_str(name);
        self.attrs(attrs);
        self.s.push_str("/>");
    }
    pub fn close(&mut self, name: &str) {
        self.s.push_str("</");
        self.s.push_str(name);
        self.s.push('>');
    }
    pub fn text(&mut self, t: &str) {
        self.s.push_str(&esc(t));
    }
    /// `<name>text</name>`.
    pub fn leaf(&mut self, name: &str, attrs: &[(&str, &str)], t: &str) {
        self.open(name, attrs);
        self.text(t);
        self.close(name);
    }
    /// `<name w:val="v"/>`.
    pub fn val(&mut self, name: &str, v: &str) {
        self.empty(name, &[("w:val", v)]);
    }
    pub fn raw(&mut self, s: &str) {
        self.s.push_str(s);
    }
}

#[cfg(test)]
mod tests;
