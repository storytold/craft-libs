#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
//! Adobe Camera Raw settings: the `crs:` namespace (`http://ns.adobe.com/camera-raw-settings/1.0/`)
//! on the `rdf:Description`s of an XMP packet, as Camera Raw's Save Settings writes them to an
//! `.xmp` file and as Camera Raw, Lightroom and Bridge write them to a raw file's sidecar
//! (`IMG_0001.xmp` beside `IMG_0001.CR2`).
//!
//! - [`Settings::parse`] reads a file; [`Settings::from_packet`] reads a packet that is already
//!   parsed. Either gives the settings as written, in file order: a name and a [`Value`]
//!   (text, an `rdf:Seq` of items, or a structure this crate does not read).
//! - [`number`], [`points`] and [`Settings::crop`] read what the text means: Camera Raw's
//!   numbers (`+0.50`), tone curve points (`"32, 22"`) and the crop rectangle.
//! - [`Settings::to_xmp`] writes a settings file Camera Raw's Load Settings reads, and
//!   [`toolkit`] says which application wrote one.
//!
//! Which setting drives which control is the application's to decide. This crate reads the XMP
//! with `craft-xml`; a general XMP reader can replace that part without changing the model.

#[cfg(test)]
mod tests;

use craft_xml::{Canonical, El, Options};

/// The Camera Raw settings namespace.
pub const CRS_NS: &str = "http://ns.adobe.com/camera-raw-settings/1.0/";
/// The RDF namespace.
pub const RDF_NS: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";
/// The namespace of `x:xmpmeta`.
pub const META_NS: &str = "adobe:ns:meta/";
/// Largest settings file read; real ones are a few kilobytes.
pub const MAX_XMP_BYTES: usize = 4 * 1024 * 1024;
/// Deepest element nesting read.
pub const MAX_XML_DEPTH: usize = 512;

/// The prefixes [`packet`] gives the three namespaces, whatever the file calls them.
pub const CANONICAL: Canonical<'static> = Canonical::new(&[("crs", CRS_NS), ("rdf", RDF_NS), ("x", META_NS)]);

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
    /// An attribute's value, or the text of an element without child elements.
    Text(String),
    /// The items of an `rdf:Seq` that are all plain text: a tone curve's points, one `"x, y"`
    /// each.
    Seq(Vec<String>),
    /// Any other element with child elements (masks and local corrections, looks, lens
    /// profiles): the `crs:` element itself, its names with the prefixes of [`CANONICAL`],
    /// for the caller to read. [`Settings::to_xmp`] writes it back as it is.
    Structured(El),
}

/// One `crs:` setting.
#[derive(Clone, Debug, PartialEq)]
pub struct Setting {
    /// The name without its prefix: `Exposure2012`, `ToneCurvePV2012`.
    pub name: String,
    pub value: Value,
}

/// The `crs:` settings of a packet, in file order: for each `rdf:Description`, its attributes
/// and then its child elements. A name may occur more than once; a reader applying settings in
/// order lets the last win.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Settings {
    pub entries: Vec<Setting>,
}

/// Parses XMP text into its root element with the namespaces of [`CANONICAL`] as `crs:`, `rdf:`
/// and `x:`. A byte order mark and an `<?xpacket?>` wrapper are accepted. Refuses more than
/// [`MAX_XMP_BYTES`], nesting past [`MAX_XML_DEPTH`], a document type declaration and
/// malformed XML.
pub fn packet(bytes: &[u8]) -> Result<El, Error> {
    if bytes.len() > MAX_XMP_BYTES {
        return Err(Error::TooLarge);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| Error::NotText)?;
    let options = Options {
        doctype: craft_xml::Doctype::Reject,
        limits: craft_xml::Limits { max_input: MAX_XMP_BYTES, max_depth: MAX_XML_DEPTH, ..craft_xml::Limits::DEFAULT },
        ..Options::canonical(&CANONICAL).strict()
    };
    options.parse(text.trim_start_matches('\u{feff}').as_bytes()).map_err(|e| Error::Xml(e.to_string()))
}

/// The XMP toolkit a file names on `x:xmpmeta` (`x:xmptk`): the application that last wrote
/// it. `None` for a file that is not XMP or names none.
pub fn toolkit(bytes: &[u8]) -> Option<String> {
    let root = packet(bytes).ok()?;
    std::iter::once(&root).chain(root.descendants()).find(|e| e.name == "x:xmpmeta")?.attr("x:xmptk").map(str::to_string)
}

impl Settings {
    /// Reads a settings file or sidecar. An error when it is not XMP ([`packet`]) or holds no
    /// `crs:` setting.
    pub fn parse(bytes: &[u8]) -> Result<Settings, Error> {
        let settings = Settings::from_packet(&packet(bytes)?);
        if settings.entries.is_empty() { Err(Error::NoSettings) } else { Ok(settings) }
    }

    /// The `crs:` settings of a parsed packet whose names carry the prefixes of [`CANONICAL`]
    /// (as [`packet`] returns it). `root` may be `x:xmpmeta`, `rdf:RDF` or anything around them.
    /// Settings are read from every `rdf:Description` that is not inside a `crs:` element: one
    /// inside belongs to that setting's structure.
    pub fn from_packet(root: &El) -> Settings {
        let mut entries = Vec::new();
        // Iterative, so a hand-built tree of any depth is walked without recursion.
        let mut descriptions = Vec::new();
        let mut open = vec![root];
        while let Some(el) = open.pop() {
            if el.name == "rdf:Description" {
                descriptions.push(el);
            }
            open.extend(el.elements().filter(|c| !c.name.starts_with("crs:")).collect::<Vec<_>>().into_iter().rev());
        }
        for desc in descriptions {
            for (name, value) in &desc.attrs {
                if let Some(name) = name.strip_prefix("crs:") {
                    entries.push(Setting { name: name.to_string(), value: Value::Text(value.clone()) });
                }
            }
            for child in desc.elements() {
                let Some(name) = child.name.strip_prefix("crs:") else { continue };
                let plain_seq = child.child("rdf:Seq").filter(|seq| {
                    child.elements().count() == 1
                        && seq.elements().all(|li| li.name == "rdf:li" && li.attrs.is_empty() && li.elements().next().is_none())
                });
                let value = match plain_seq {
                    Some(seq) => Value::Seq(seq.elements().map(|li| li.text().into_owned()).collect()),
                    None if child.elements().next().is_some() => Value::Structured(child.clone()),
                    None => Value::Text(child.text().into_owned()),
                };
                entries.push(Setting { name: name.to_string(), value });
            }
        }
        Settings { entries }
    }

    /// The last value of `name`.
    pub fn get(&self, name: &str) -> Option<&Value> {
        self.entries.iter().rev().find(|s| s.name == name).map(|s| &s.value)
    }

    /// The last text value of `name`.
    pub fn text(&self, name: &str) -> Option<&str> {
        match self.get(name)? {
            Value::Text(t) => Some(t),
            _ => None,
        }
    }

    /// Adds a text setting (written as an attribute).
    pub fn push(&mut self, name: &str, value: impl Into<String>) {
        self.entries.push(Setting { name: name.to_string(), value: Value::Text(value.into()) });
    }

    /// Adds a sequence setting (written as an element holding an `rdf:Seq`).
    pub fn push_seq(&mut self, name: &str, items: Vec<String>) {
        self.entries.push(Setting { name: name.to_string(), value: Value::Seq(items) });
    }

    /// Adds the keys of a crop: `HasCrop`, the four edges to six decimals, and a zero angle.
    pub fn push_crop(&mut self, crop: Crop) {
        self.push("HasCrop", "True");
        for (key, v) in CROP_KEYS.iter().zip([crop.top, crop.left, crop.bottom, crop.right]) {
            self.push(key, format!("{v:.6}"));
        }
        self.push("CropAngle", "0");
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
    /// that wrote it: text settings as attributes of one `rdf:Description`, sequences as its
    /// child elements, each in the order given, then structures as they are. Names that are
    /// not XML names are left out, as is a structure whose element is not the `crs:` element
    /// of that name.
    pub fn to_xmp(&self, toolkit: &str) -> String {
        let named = |s: &&Setting| craft_xml::is_name(&s.name) && !s.name.contains(':');
        let mut out = format!(
            "<x:xmpmeta xmlns:x=\"{META_NS}\" x:xmptk=\"{}\">\n <rdf:RDF xmlns:rdf=\"{RDF_NS}\">\n  <rdf:Description rdf:about=\"\"\n    xmlns:crs=\"{CRS_NS}\"",
            craft_xml::esc_attr(toolkit)
        );
        for s in self.entries.iter().filter(named) {
            if let Value::Text(text) = &s.value {
                out.push_str(&format!("\n   crs:{}=\"{}\"", s.name, craft_xml::esc_attr(text)));
            }
        }
        out.push_str(">\n");
        for s in self.entries.iter().filter(named) {
            if let Value::Seq(items) = &s.value {
                out.push_str(&format!("   <crs:{}>\n    <rdf:Seq>\n", s.name));
                for item in items {
                    out.push_str(&format!("     <rdf:li>{}</rdf:li>\n", craft_xml::esc(item)));
                }
                out.push_str(&format!("    </rdf:Seq>\n   </crs:{}>\n", s.name));
            }
        }
        for s in self.entries.iter().filter(named) {
            if let Value::Structured(el) = &s.value
                && el.name.strip_prefix("crs:") == Some(s.name.as_str())
            {
                out.push_str("   ");
                el.write_xml(&mut out);
                out.push('\n');
            }
        }
        out.push_str("  </rdf:Description>\n </rdf:RDF>\n</x:xmpmeta>\n");
        out
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
