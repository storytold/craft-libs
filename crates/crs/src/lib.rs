#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
//! Adobe Camera Raw settings: the `crs:` namespace (`http://ns.adobe.com/camera-raw-settings/1.0/`)
//! on the `rdf:Description`s of an XMP packet, as Camera Raw's Save Settings writes them to an
//! `.xmp` file and as Camera Raw, Lightroom and Bridge write them to a raw file's sidecar
//! (`IMG_0001.xmp` beside `IMG_0001.CR2`).
//!
//! - [`Settings::parse`] reads a file or sidecar; [`Settings::from_xmp`] takes a packet that is
//!   already parsed with `craft_metadata::parse_xmp`. Either gives the settings by name: a
//!   [`Value`] (text, the plain items of an `rdf:Seq`, or a structure such as local corrections).
//! - [`number`], [`points`] and [`Settings::crop`] read what the text means: Camera Raw's
//!   numbers (`+0.50`), tone curve points (`"32, 22"`) and the crop rectangle.
//! - [`Settings::to_xmp`] writes a settings file Camera Raw's Load Settings reads, and
//!   [`toolkit`] says which application wrote one.
//!
//! Which setting drives which control is the application's to decide. The packet is read by
//! `craft-metadata`'s XMP reader, so there is one XMP reader for every crate.

#[cfg(test)]
mod tests;

use std::collections::BTreeMap;

use craft_metadata::{XmpData, XmpValue};

/// The Camera Raw settings namespace.
pub use craft_metadata::CRS_NS;
/// The RDF namespace.
pub const RDF_NS: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";
/// The namespace of `x:xmpmeta`.
pub const META_NS: &str = "adobe:ns:meta/";
/// Largest settings file read; real ones are a few kilobytes.
pub const MAX_XMP_BYTES: usize = 4 * 1024 * 1024;
/// Deepest structure [`Settings::to_xmp`] writes; deeper levels are left out. The reader stops
/// well before this.
pub const MAX_WRITE_DEPTH: usize = 64;

/// Why a settings file was not read.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("the settings file is too large")]
    TooLarge,
    #[error("not a Camera Raw settings file (not UTF-8 text)")]
    NotText,
    #[error("not a Camera Raw settings file ({0})")]
    Xml(String),
    #[error("no Camera Raw settings in the file")]
    NoSettings,
}

/// What a setting holds.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// An attribute's value or a plain element's text, trimmed.
    Text(String),
    /// The items of an `rdf:Seq` (or `rdf:Bag`, `rdf:Alt`) that are all plain text: a tone
    /// curve's points, one `"x, y"` each.
    Seq(Vec<String>),
    /// Anything else: a structure or an array of structures (masks and local corrections,
    /// looks, lens profiles), with its fields named `crs:Field`, for the caller to read.
    /// [`Settings::to_xmp`] writes it back.
    Structured(XmpValue),
}

impl Value {
    fn from_xmp(v: &XmpValue) -> Value {
        match v {
            XmpValue::Text(t) => Value::Text(t.clone()),
            XmpValue::Array(items) if items.iter().all(|i| matches!(i, XmpValue::Text(_))) => {
                Value::Seq(items.iter().filter_map(|i| i.text().map(str::to_string)).collect())
            }
            other => Value::Structured(other.clone()),
        }
    }
}

/// The `crs:` settings of a packet, by name without the prefix (`Exposure2012`). Where a name
/// occurs more than once (in several `rdf:Description`s), the last one is kept.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Settings {
    pub entries: BTreeMap<String, Value>,
}

/// The XMP toolkit a file names on `x:xmpmeta` (`x:xmptk`): the application that last wrote
/// it. `None` for a file that is not XMP or names none. Only the first start tag named
/// `x:xmpmeta` (by namespace) is looked at: a bounded scan, not a parse of the packet.
pub fn toolkit(bytes: &[u8]) -> Option<String> {
    if bytes.len() > MAX_XMP_BYTES {
        return None;
    }
    let canonical = craft_xml::Canonical::new(&[("x", META_NS)]);
    let el = craft_xml::Options::canonical(&canonical).first_start(bytes, "x:xmpmeta")?;
    el.attr("x:xmptk").map(str::to_string)
}

impl Settings {
    /// Reads a settings file or sidecar. An error when it is not XMP or holds no `crs:`
    /// setting.
    pub fn parse(bytes: &[u8]) -> Result<Settings, Error> {
        if bytes.len() > MAX_XMP_BYTES {
            return Err(Error::TooLarge);
        }
        let text = std::str::from_utf8(bytes).map_err(|_| Error::NotText)?;
        let xmp = craft_metadata::parse_xmp(text).map_err(|e| Error::Xml(e.to_string()))?;
        let settings = Settings::from_xmp(&xmp);
        if settings.entries.is_empty() { Err(Error::NoSettings) } else { Ok(settings) }
    }

    /// The `crs:` settings of a packet `craft_metadata::parse_xmp` read.
    pub fn from_xmp(xmp: &XmpData) -> Settings {
        let entries = xmp.values.iter().filter_map(|(k, v)| Some((k.strip_prefix("crs:")?.to_string(), Value::from_xmp(v)))).collect();
        Settings { entries }
    }

    /// The value of `name`.
    pub fn get(&self, name: &str) -> Option<&Value> {
        self.entries.get(name)
    }

    /// The text value of `name`.
    pub fn text(&self, name: &str) -> Option<&str> {
        match self.get(name)? {
            Value::Text(t) => Some(t),
            _ => None,
        }
    }

    /// Sets a text setting (written as an attribute).
    pub fn set(&mut self, name: &str, value: impl Into<String>) {
        self.entries.insert(name.to_string(), Value::Text(value.into()));
    }

    /// Sets a sequence setting (written as an element holding an `rdf:Seq`).
    pub fn set_seq(&mut self, name: &str, items: Vec<String>) {
        self.entries.insert(name.to_string(), Value::Seq(items));
    }

    /// Sets the keys of a crop: `HasCrop`, the four edges to six decimals, and a zero angle.
    pub fn set_crop(&mut self, crop: Crop) {
        self.set("HasCrop", "True");
        for (key, v) in CROP_KEYS.iter().zip([crop.top, crop.left, crop.bottom, crop.right]) {
            self.set(key, format!("{v:.6}"));
        }
        self.set("CropAngle", "0");
    }

    /// The file's crop from its keys. `HasCrop` must be `True`: without it, or with `False`,
    /// the other keys are what Camera Raw left from an earlier or reset crop and mean nothing.
    pub fn crop(&self) -> CropState {
        match self.text("HasCrop").map(str::trim) {
            None => return CropState::None,
            Some(v) if v.eq_ignore_ascii_case("false") => return CropState::None,
            Some(v) if v.eq_ignore_ascii_case("true") => {}
            Some(_) => return CropState::Unusable(vec!["HasCrop"]),
        }
        let texts = CROP_KEYS.map(|k| self.text(k));
        let mut v = NO_CROP;
        let mut usable = true;
        for (slot, text) in v.iter_mut().zip(texts) {
            if let Some(text) = text {
                match number(text) {
                    Some(n) => *slot = n,
                    None => usable = false,
                }
            }
        }
        let [top, left, bottom, right, angle] = v;
        let [top, left, bottom, right] = [top, left, bottom, right].map(|f| f.clamp(0.0, 1.0));
        // A straightened crop's rectangle is measured on the rotated frame: not usable as is.
        if !usable || angle != 0.0 || top >= bottom || left >= right {
            return CropState::Unusable(CROP_KEYS.into_iter().zip(texts).filter(|(_, t)| t.is_some()).map(|(k, _)| k).collect());
        }
        if [top, left, bottom, right] == [0.0, 0.0, 1.0, 1.0] { CropState::None } else { CropState::Crop(Crop { top, left, bottom, right }) }
    }

    /// Writes the settings as a Camera Raw settings file, naming `toolkit` as the application
    /// that wrote it: text settings as attributes of one `rdf:Description`, then sequences and
    /// structures as its child elements, each group by name. Left out: names that are not XML
    /// names, structure fields outside the `crs:` namespace, and levels past
    /// [`MAX_WRITE_DEPTH`].
    pub fn to_xmp(&self, toolkit: &str) -> String {
        let named = |(name, _): &(&String, &Value)| plain_name(name);
        let mut out = format!(
            "<x:xmpmeta xmlns:x=\"{META_NS}\" x:xmptk=\"{}\">\n <rdf:RDF xmlns:rdf=\"{RDF_NS}\">\n  <rdf:Description rdf:about=\"\"\n    xmlns:crs=\"{CRS_NS}\"",
            craft_xml::esc_attr(toolkit)
        );
        for (name, value) in self.entries.iter().filter(named) {
            if let Value::Text(text) = value {
                out.push_str(&format!("\n   crs:{name}=\"{}\"", craft_xml::esc_attr(text)));
            }
        }
        out.push_str(">\n");
        for (name, value) in self.entries.iter().filter(named) {
            match value {
                Value::Text(_) => {}
                Value::Seq(items) => {
                    out.push_str(&format!("   <crs:{name}>\n    <rdf:Seq>\n"));
                    for item in items {
                        out.push_str(&format!("     <rdf:li>{}</rdf:li>\n", craft_xml::esc(item)));
                    }
                    out.push_str(&format!("    </rdf:Seq>\n   </crs:{name}>\n"));
                }
                Value::Structured(v) => {
                    out.push_str("   ");
                    write_property(&mut out, &format!("crs:{name}"), v, 0);
                    out.push('\n');
                }
            }
        }
        out.push_str("  </rdf:Description>\n </rdf:RDF>\n</x:xmpmeta>\n");
        out
    }
}

/// A local name this writer can put after `crs:`.
fn plain_name(name: &str) -> bool {
    craft_xml::is_name(name) && !name.contains(':')
}

/// Writes `v` as the element `tag`: text as content, an array as an `rdf:Seq`, a structure as
/// `rdf:parseType="Resource"` with its `crs:` fields.
fn write_property(out: &mut String, tag: &str, v: &XmpValue, depth: usize) {
    match v {
        XmpValue::Text(t) => out.push_str(&format!("<{tag}>{}</{tag}>", craft_xml::esc(t))),
        XmpValue::Array(items) => {
            out.push_str(&format!("<{tag}><rdf:Seq>"));
            if depth < MAX_WRITE_DEPTH {
                for item in items {
                    write_property(out, "rdf:li", item, depth + 1);
                }
            }
            out.push_str(&format!("</rdf:Seq></{tag}>"));
        }
        XmpValue::Struct(fields) => {
            out.push_str(&format!("<{tag} rdf:parseType=\"Resource\">"));
            if depth < MAX_WRITE_DEPTH {
                for (key, field) in fields {
                    if let Some(local) = key.strip_prefix("crs:").filter(|l| plain_name(l)) {
                        write_property(out, &format!("crs:{local}"), field, depth + 1);
                    }
                }
            }
            out.push_str(&format!("</{tag}>"));
        }
    }
}

/// A Camera Raw number: optional leading `+`, surrounding whitespace, finite.
pub fn number(s: &str) -> Option<f32> {
    let s = s.trim();
    let v: f32 = s.strip_prefix('+').unwrap_or(s).parse().ok()?;
    v.is_finite().then_some(v)
}

/// A number as Camera Raw writes a signed setting: `+0.50`, `-12`, `0`.
pub fn signed(v: f32, decimals: usize) -> String {
    let s = format!("{v:.decimals$}");
    if v > 0.0 && !s.starts_with('+') { format!("+{s}") } else { s }
}

/// The points of a tone curve from its `rdf:Seq` items (`"x, y"`, 0 to 255 each). `None` when
/// an item is not two numbers. Whether the points make a usable curve is the caller's to check.
pub fn points(items: &[String]) -> Option<Vec<[f32; 2]>> {
    items
        .iter()
        .map(|item| {
            let (x, y) = item.split_once(',')?;
            Some([number(x)?, number(y)?])
        })
        .collect()
}

/// The crop's keys besides `HasCrop`.
pub const CROP_KEYS: [&str; 5] = ["CropTop", "CropLeft", "CropBottom", "CropRight", "CropAngle"];
/// Their values without a crop: the whole frame, level.
const NO_CROP: [f32; 5] = [0.0, 0.0, 1.0, 1.0, 0.0];

/// What [`Settings::crop`] found.
#[derive(Clone, Debug, PartialEq)]
pub enum CropState {
    /// No crop, or one that is the whole frame.
    None,
    /// A level crop smaller than the frame.
    Crop(Crop),
    /// A crop that cannot be used as a rectangle (straightened, unreadable or empty): the keys
    /// present, for reporting.
    Unusable(Vec<&'static str>),
}

/// A Camera Raw crop: fractions of the developed frame in the raw's stored orientation, before
/// its Orientation tag turns it upright, with `0 <= left < right <= 1` and
/// `0 <= top < bottom <= 1`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Crop {
    pub top: f32,
    pub left: f32,
    pub bottom: f32,
    pub right: f32,
}

impl Crop {
    /// The crop of the frame turned upright by TIFF orientation `o` (1–8; anything else is 1),
    /// as fractions of the upright frame: the corners map as the pixels do.
    pub fn oriented(self, o: u16) -> Crop {
        let map = |u: f32, v: f32| match o {
            2 => (1.0 - u, v),
            3 => (1.0 - u, 1.0 - v),
            4 => (u, 1.0 - v),
            5 => (v, u),
            6 => (1.0 - v, u),
            7 => (1.0 - v, 1.0 - u),
            8 => (v, 1.0 - u),
            _ => (u, v),
        };
        let (ax, ay) = map(self.left, self.top);
        let (bx, by) = map(self.right, self.bottom);
        Crop { top: ay.min(by), left: ax.min(bx), bottom: ay.max(by), right: ax.max(bx) }
    }

    /// The crop on a `w × h` frame in pixels, `(x0, y0, x1, y1)` with `x1` and `y1` exclusive:
    /// at least one pixel, inside the frame. `None` for an empty frame.
    pub fn pixels(self, w: usize, h: usize) -> Option<(usize, usize, usize, usize)> {
        let edge = |f: f32, n: usize| ((f64::from(f).clamp(0.0, 1.0) * n as f64).round() as usize).min(n);
        let span = |a: f32, b: f32, n: usize| {
            let lo = edge(a, n).min(n.checked_sub(1)?);
            Some((lo, edge(b, n).max(lo + 1)))
        };
        let (x0, x1) = span(self.left, self.right, w)?;
        let (y0, y1) = span(self.top, self.bottom, h)?;
        Some((x0, y0, x1, y1))
    }
}
