//! Android binary XML (AXML): the compiled form of `AndroidManifest.xml` and `res/*.xml`.
//! Chunks, the string pool (UTF-8 and UTF-16), the resource-id map, namespaces, elements and
//! typed attribute values. Also the string pool and typed-value formatting that resource tables
//! share. Manifest inventory and policy are left to callers.

use crate::Error;
use crate::bytes::Reader;
use serde::Serialize;

pub const RES_STRING_POOL: u16 = 0x0001;
pub const RES_XML: u16 = 0x0003;
const XML_START_NS: u16 = 0x0100;
const XML_END_NS: u16 = 0x0101;
const XML_START_ELEMENT: u16 = 0x0102;
const XML_END_ELEMENT: u16 = 0x0103;
const XML_CDATA: u16 = 0x0104;
const XML_RESOURCE_MAP: u16 = 0x0180;

/// Most strings decoded from one pool.
const MAX_STRINGS: usize = 1_000_000;
/// Longest string kept (characters).
const MAX_STRING: usize = 64 * 1024;
/// Default budget for copied resource and XML values, and the base string-pool byte cap.
pub(crate) const MAX_RETAINED_VALUE_BYTES: usize = 64 << 20;
/// Base budget for decoded string bytes; larger chunks allow at most three times their size.
const BASE_POOL_BYTES: usize = MAX_RETAINED_VALUE_BYTES;
/// Most extra UTF-8 length candidates examined across one string pool.
const MAX_RECOVERY_PROBES: usize = 65_536;
/// Most XML nodes decoded.
const MAX_NODES: usize = 200_000;
/// Deepest element nesting tracked.
const MAX_DEPTH: usize = 512;

pub const ANDROID_NS: &str = "http://schemas.android.com/apk/res/android";

/// A chunk header: type, header size, total size, at `off`.
pub(crate) fn chunk(r: &Reader<'_>, off: usize) -> Option<(u16, usize, usize)> {
    let t = r.u16_le(off)?;
    let h = usize::from(r.u16_le(off.checked_add(2)?)?);
    let s = r.u32_le(off.checked_add(4)?)? as usize;
    if h < 8 || s < h || off.checked_add(s)? > r.len() {
        return None;
    }
    Some((t, h, s))
}

/// Reads the bounded Android chunk header at the given byte offset.
pub fn chunk_header(data: &[u8], off: usize) -> Option<(u16, usize, usize)> {
    chunk(&Reader::new(data), off)
}

/// A decoded string pool.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StringPool {
    pub strings: Vec<String>,
    pub utf8: bool,
    /// Strings the pool declared (may exceed `strings.len()` when capped).
    pub declared: usize,
    /// Entries left empty because the UTF-8 recovery probe budget ran out.
    pub recovery_omitted: usize,
}

impl StringPool {
    pub fn get(&self, i: u32) -> Option<&str> {
        if i == u32::MAX {
            return None;
        }
        self.strings.get(i as usize).map(String::as_str)
    }

    /// Parse the pool whose chunk starts at `off`, retaining a prefix within the string and byte budgets.
    pub fn parse(data: &[u8], off: usize) -> Result<StringPool, Error> {
        let mut recovery_left = MAX_RECOVERY_PROBES;
        Self::parse_with_recovery_budget(data, off, &mut recovery_left)
    }

    fn parse_with_recovery_budget(data: &[u8], off: usize, recovery_left: &mut usize) -> Result<StringPool, Error> {
        let r = Reader::new(data);
        let (t, hdr, size) = chunk(&r, off).ok_or_else(|| Error::Truncated("string pool header".into()))?;
        if t != RES_STRING_POOL {
            return Err(Error::Malformed(format!("expected a string pool, found chunk type {t:#x}")));
        }
        if hdr < 28 {
            return Err(Error::Malformed("string pool header is smaller than 28 bytes".into()));
        }
        let r = Reader::new(r.slice(off, size).ok_or_else(|| Error::Truncated("string pool".into()))?);
        let count = r.u32_le(8).ok_or_else(|| Error::Truncated("string pool".into()))? as usize;
        let styles = r.u32_le(12).ok_or_else(|| Error::Truncated("string pool".into()))? as usize;
        let flags = r.u32_le(16).ok_or_else(|| Error::Truncated("string pool".into()))?;
        let strings_start = r.u32_le(20).ok_or_else(|| Error::Truncated("string pool".into()))? as usize;
        let styles_start = r.u32_le(24).ok_or_else(|| Error::Truncated("string pool".into()))? as usize;
        let utf8 = flags & (1 << 8) != 0;
        let offsets_end = count
            .checked_add(styles)
            .and_then(|n| n.checked_mul(4))
            .and_then(|n| hdr.checked_add(n))
            .ok_or_else(|| Error::Malformed("string pool offset tables are too large".into()))?;
        if offsets_end > size || strings_start > size || (count != 0 && strings_start < offsets_end) {
            return Err(Error::Malformed("string pool offset tables or string data lie outside their area".into()));
        }
        if (styles != 0 && styles_start == 0)
            || (styles_start != 0 && (styles_start < offsets_end || styles_start < strings_start || styles_start > size))
        {
            return Err(Error::Malformed("string pool style data lie outside their area".into()));
        }
        let end = if styles_start == 0 { size } else { styles_start };
        let string_bytes = r.slice(strings_start, end - strings_start).ok_or_else(|| Error::Truncated("string data".into()))?;
        let strings_reader = Reader::new(string_bytes);
        let mut strings = Vec::with_capacity(count.min(MAX_STRINGS).min(65_536));
        let mut bytes_left = BASE_POOL_BYTES.max(size.saturating_mul(3));
        let mut recovery_omitted = 0;
        for i in 0..count.min(MAX_STRINGS) {
            let at = r.u32_le(hdr + i * 4).ok_or_else(|| Error::Truncated("string offsets".into()))? as usize;
            let s = if at >= string_bytes.len() {
                None
            } else if utf8 {
                utf8_string(&strings_reader, at, string_bytes.len(), recovery_left, &mut recovery_omitted)
            } else {
                utf16_string(&strings_reader, at, string_bytes.len())
            };
            let s = s.unwrap_or_default();
            if s.len() > bytes_left {
                break;
            }
            bytes_left -= s.len();
            strings.push(s);
        }
        Ok(StringPool { strings, utf8, declared: count, recovery_omitted })
    }
}

fn utf8_len(r: &Reader<'_>, at: usize) -> Option<(usize, usize)> {
    let b0 = usize::from(r.u8(at)?);
    if b0 & 0x80 != 0 {
        let b1 = usize::from(r.u8(at + 1)?);
        Some((((b0 & 0x7f) << 8) | b1, 2))
    } else {
        Some((b0, 1))
    }
}

fn utf8_string(r: &Reader<'_>, at: usize, end: usize, recovery_left: &mut usize, recovery_omitted: &mut usize) -> Option<String> {
    let (_chars, n1) = utf8_len(r, at)?;
    let (mut bytes, n2) = utf8_len(r, at.checked_add(n1)?)?;
    let start = at.checked_add(n1 + n2)?;
    // Older AAPT keeps only the low 15 length bits; probe matching lengths inside this pool.
    loop {
        let terminator = start.checked_add(bytes)?;
        if terminator >= end {
            return None;
        }
        // The declared candidate costs one read per entry, already bounded by MAX_STRINGS.
        if bytes >= 0x8000 {
            if *recovery_left == 0 {
                *recovery_omitted += 1;
                return None;
            }
            *recovery_left -= 1;
        }
        if r.u8(terminator)? == 0 {
            break;
        }
        bytes = bytes.checked_add(0x8000)?;
    }
    let b = r.slice(start, bytes.min(MAX_STRING * 4))?;
    Some(String::from_utf8_lossy(b).into_owned())
}

fn utf16_string(r: &Reader<'_>, at: usize, end: usize) -> Option<String> {
    let u0 = usize::from(r.u16_le(at)?);
    let (len, hdr) = if u0 & 0x8000 != 0 { (((u0 & 0x7fff) << 16) | usize::from(r.u16_le(at + 2)?), 4) } else { (u0, 2) };
    let start = at.checked_add(hdr)?;
    let terminator = start.checked_add(len.checked_mul(2)?)?;
    if terminator.checked_add(2)? > end || r.u16_le(terminator)? != 0 {
        return None;
    }
    let n = len.min(MAX_STRING);
    let mut units = Vec::with_capacity(n);
    for i in 0..n {
        units.push(r.u16_le(start + i * 2)?);
    }
    Some(String::from_utf16_lossy(&units))
}

/// Record that a string pool was capped or omitted recovery entries.
pub(crate) fn report_partial_string_pool(pool: &StringPool, prefix: &str, truncated: &mut bool, warnings: &mut Vec<String>) {
    if pool.strings.len() < pool.declared {
        *truncated = true;
        warnings.push(format!("{prefix} was capped at {} of {} strings", pool.strings.len(), pool.declared));
    }
    if pool.recovery_omitted > 0 {
        *truncated = true;
        warnings.push(format!("{prefix} omitted {} strings after the UTF-8 recovery budget was exhausted", pool.recovery_omitted));
    }
}

/// Charge `value` against a retained-byte budget. When it does not fit, report truncation once
/// and return `None`, so a later shorter value can still be kept.
pub(crate) fn retain_value(
    left: &mut usize,
    value: String,
    truncated: &mut bool,
    warned: &mut bool,
    warnings: &mut Vec<String>,
    warning: &str,
) -> Option<String> {
    if value.len() > *left {
        if !*warned {
            warnings.push(warning.to_string());
            *warned = true;
        }
        *truncated = true;
        None
    } else {
        *left -= value.len();
        Some(value)
    }
}

pub(crate) struct ValueBudget {
    left: usize,
    warned: bool,
}

impl ValueBudget {
    pub(crate) fn new(max_bytes: usize) -> Self {
        Self { left: max_bytes, warned: false }
    }

    pub(crate) fn take(&mut self, value: String, truncated: &mut bool, warnings: &mut Vec<String>, warning: &str) -> Option<String> {
        retain_value(&mut self.left, value, truncated, &mut self.warned, warnings, warning)
    }
}

/// A typed value (`Res_value`): data type and 32-bit data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ResValue {
    pub data_type: u8,
    pub data: u32,
}

impl ResValue {
    pub fn type_name(&self) -> &'static str {
        match self.data_type {
            0x00 => "null",
            0x01 => "reference",
            0x02 => "attribute",
            0x03 => "string",
            0x04 => "float",
            0x05 => "dimension",
            0x06 => "fraction",
            0x07 => "dynamic-reference",
            0x08 => "dynamic-attribute",
            0x10 => "int-dec",
            0x11 => "int-hex",
            0x12 => "boolean",
            0x1c => "color-argb8",
            0x1d => "color-rgb8",
            0x1e => "color-argb4",
            0x1f => "color-rgb4",
            _ => "unknown",
        }
    }

    /// Human form; strings come from `pool`.
    pub fn format(&self, pool: &StringPool) -> String {
        let d = self.data;
        match self.data_type {
            0x00 => {
                if d == 1 {
                    "@empty".into()
                } else {
                    "@null".into()
                }
            }
            0x01 | 0x07 => {
                if d == 0 {
                    "@null".into()
                } else {
                    format!("@{d:#010x}")
                }
            }
            0x02 | 0x08 => format!("?{d:#010x}"),
            0x03 => pool.get(d).unwrap_or("").to_string(),
            0x04 => format!("{}", f32::from_bits(d)),
            0x05 => complex(d, &["px", "dp", "sp", "pt", "in", "mm"]),
            0x06 => complex(d, &["%", "%p"]),
            0x10 => (d as i32).to_string(),
            0x11 => format!("{d:#x}"),
            0x12 => (if d != 0 { "true" } else { "false" }).into(),
            0x1c..=0x1f => format!("#{d:08x}"),
            _ => format!("{d:#x} (type {:#x})", self.data_type),
        }
    }
}

fn complex(d: u32, units: &[&str]) -> String {
    const RADIX: [f64; 4] = [1.0, 1.0 / 128.0, 1.0 / 32768.0, 1.0 / 8_388_608.0];
    let mantissa = f64::from((d & 0xffff_ff00) as i32 >> 8);
    let radix = RADIX.get(((d >> 4) & 3) as usize).copied().unwrap_or(1.0);
    let mut v = mantissa * radix;
    let unit = units.get((d & 0xf) as usize).copied().unwrap_or("?");
    if units.len() == 2 {
        v *= 100.0;
    }
    let v = (v * 10_000.0).round() / 10_000.0;
    format!("{v}{unit}")
}

/// One attribute of an element.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Attribute {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    pub name: String,
    /// The framework/app resource id of the attribute name, when the resource map gives one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_id: Option<String>,
    /// Formatted value.
    pub value: String,
    pub value_type: &'static str,
    #[serde(skip)]
    pub raw: ResValue,
}

/// One XML node event.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Node {
    Start { depth: usize, name: String, namespace: Option<String>, line: u32, attributes: Vec<Attribute> },
    End { depth: usize, name: String },
    Text { depth: usize, text: String },
    Namespace { prefix: String, uri: String },
}

/// A decoded binary XML document.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Document {
    pub nodes: Vec<Node>,
    #[serde(skip)]
    pub pool: StringPool,
    pub resource_ids: usize,
    pub namespaces: Vec<(String, String)>,
    pub truncated: bool,
    pub warnings: Vec<String>,
}

/// Names of common framework attributes by resource id, used when a name string was stripped.
pub fn framework_attr(id: u32) -> Option<&'static str> {
    Some(match id {
        0x0101_0000 => "theme",
        0x0101_0001 => "label",
        0x0101_0002 => "icon",
        0x0101_0003 => "name",
        0x0101_0006 => "permission",
        0x0101_0007 => "readPermission",
        0x0101_0008 => "writePermission",
        0x0101_0009 => "protectionLevel",
        0x0101_000e => "enabled",
        0x0101_000f => "debuggable",
        0x0101_0010 => "exported",
        0x0101_0018 => "authorities",
        0x0101_020c => "minSdkVersion",
        0x0101_021b => "versionCode",
        0x0101_021c => "versionName",
        0x0101_0270 => "targetSdkVersion",
        0x0101_0271 => "maxSdkVersion",
        0x0101_0280 => "allowBackup",
        _ => return None,
    })
}

/// Decode a binary XML file.
pub fn parse(data: &[u8]) -> Result<Document, Error> {
    parse_with_limits(data, MAX_NODES, MAX_DEPTH, MAX_RETAINED_VALUE_BYTES)
}

fn parse_with_limits(data: &[u8], max_nodes: usize, max_depth: usize, max_value_bytes: usize) -> Result<Document, Error> {
    let r = Reader::new(data);
    let (t, h, size) = chunk(&r, 0).ok_or_else(|| Error::Truncated("binary XML header".into()))?;
    if t != RES_XML {
        return Err(Error::Malformed(format!("not binary XML (chunk type {t:#x})")));
    }
    let data = r.slice(0, size).ok_or_else(|| Error::Truncated("binary XML document".into()))?;
    let r = Reader::new(data);
    let mut doc = Document::default();
    let mut res_map: Vec<u32> = Vec::new();
    let mut pos = h;
    let mut depth = 0usize;
    let mut ns_stack: Vec<(String, String)> = Vec::new();
    let mut value_budget = ValueBudget::new(max_value_bytes);
    while pos < size {
        let Some((ct, ch, cs)) = chunk(&r, pos) else {
            doc.truncated = true;
            doc.warnings.push(format!("unreadable chunk at offset {pos}"));
            break;
        };
        let extension = match ct {
            XML_START_NS | XML_END_NS | XML_END_ELEMENT => 8,
            XML_START_ELEMENT => 20,
            XML_CDATA => 12,
            _ => 0,
        };
        if extension != 0 && (ch < 16 || cs - ch < extension) {
            doc.truncated = true;
            doc.warnings.push(format!("incomplete XML node at offset {pos}"));
            break;
        }
        if matches!(ct, XML_START_NS | XML_START_ELEMENT | XML_END_ELEMENT | XML_CDATA) && doc.nodes.len() >= max_nodes {
            doc.truncated = true;
            doc.warnings.push("XML node limit reached; the remaining events were skipped".into());
            break;
        }
        match ct {
            RES_STRING_POOL => match StringPool::parse(data, pos) {
                Ok(p) => {
                    report_partial_string_pool(&p, "string pool", &mut doc.truncated, &mut doc.warnings);
                    doc.pool = p;
                }
                Err(e) => doc.warnings.push(format!("string pool: {e}")),
            },
            XML_RESOURCE_MAP => {
                let n = (cs - ch) / 4;
                res_map = (0..n.min(MAX_STRINGS)).filter_map(|i| r.u32_le(pos + ch + i * 4)).collect();
                doc.resource_ids = res_map.len();
            }
            XML_START_NS | XML_END_NS => {
                let prefix = r.u32_le(pos + ch).and_then(|i| doc.pool.get(i)).unwrap_or("").to_string();
                let uri = r.u32_le(pos + ch + 4).and_then(|i| doc.pool.get(i)).unwrap_or("").to_string();
                if ct == XML_START_NS {
                    if doc.namespaces.len() < 256 {
                        doc.namespaces.push((prefix.clone(), uri.clone()));
                    }
                    if ns_stack.len() < MAX_DEPTH {
                        ns_stack.push((prefix.clone(), uri.clone()));
                    }
                    doc.nodes.push(Node::Namespace { prefix, uri });
                } else {
                    ns_stack.pop();
                }
            }
            XML_START_ELEMENT => {
                if depth >= max_depth {
                    doc.truncated = true;
                    doc.warnings.push("XML nesting limit reached; the remaining events were skipped".into());
                    break;
                }
                let line = r.u32_le(pos + 8).unwrap_or(0);
                let ext = pos + ch;
                let ns = r.u32_le(ext).and_then(|i| doc.pool.get(i)).map(str::to_string);
                let name = r.u32_le(ext + 4).and_then(|i| doc.pool.get(i)).unwrap_or("?").to_string();
                let a_start = usize::from(r.u16_le(ext + 8).unwrap_or(20));
                let a_size = usize::from(r.u16_le(ext + 10).unwrap_or(20));
                let a_count = usize::from(r.u16_le(ext + 12).unwrap_or(0));
                if a_count != 0 && (a_start < 20 || a_size < 20) {
                    doc.truncated = true;
                    doc.warnings.push(format!("element {name}: invalid attribute table layout"));
                    break;
                }
                let mut attributes = Vec::new();
                for i in 0..a_count {
                    let relative = a_start + i * a_size;
                    if relative.checked_add(a_size).is_none_or(|end| end > cs - ch) {
                        doc.truncated = true;
                        doc.warnings.push(format!("element {name}: attribute {i} runs past its chunk"));
                        break;
                    }
                    let a = ext + relative;
                    let value_size = usize::from(r.u16_le(a + 12).unwrap_or(0));
                    if value_size < 8 || value_size > a_size - 12 {
                        doc.truncated = true;
                        doc.warnings.push(format!("element {name}: invalid typed value size in attribute {i}"));
                        break;
                    }
                    let ans = r.u32_le(a).and_then(|i| doc.pool.get(i)).map(str::to_string);
                    let name_idx = r.u32_le(a + 4).unwrap_or(u32::MAX);
                    let raw_str = r.u32_le(a + 8).unwrap_or(u32::MAX);
                    let vt = r.u8(a + 15).unwrap_or(0);
                    let vd = r.u32_le(a + 16).unwrap_or(0);
                    let rid = res_map.get(name_idx as usize).copied();
                    let mut aname = doc.pool.get(name_idx).unwrap_or("").to_string();
                    if aname.is_empty() {
                        aname = rid.and_then(framework_attr).map(str::to_string).unwrap_or_else(|| format!("attr_{name_idx}"));
                    }
                    let raw = ResValue { data_type: vt, data: vd };
                    let value = if vt == 0x03 || (raw_str != u32::MAX && vt == 0) {
                        doc.pool.get(if vt == 0x03 { vd } else { raw_str }).unwrap_or("").to_string()
                    } else {
                        raw.format(&doc.pool)
                    };
                    let value = value_budget
                        .take(value, &mut doc.truncated, &mut doc.warnings, "XML value budget reached; remaining values were omitted")
                        .unwrap_or_default();
                    attributes.push(Attribute {
                        namespace: ans,
                        name: aname,
                        resource_id: rid.map(|x| format!("{x:#010x}")),
                        value,
                        value_type: raw.type_name(),
                        raw,
                    });
                }
                doc.nodes.push(Node::Start { depth, name, namespace: ns, line, attributes });
                depth += 1;
            }
            XML_END_ELEMENT => {
                depth = depth.saturating_sub(1);
                let name = r.u32_le(pos + ch + 4).and_then(|i| doc.pool.get(i)).unwrap_or("?").to_string();
                doc.nodes.push(Node::End { depth, name });
            }
            XML_CDATA => {
                let text = r.u32_le(pos + ch).and_then(|i| doc.pool.get(i)).unwrap_or("").to_string();
                let text = value_budget
                    .take(text, &mut doc.truncated, &mut doc.warnings, "XML value budget reached; remaining values were omitted")
                    .unwrap_or_default();
                doc.nodes.push(Node::Text { depth, text });
            }
            _ => {}
        }
        pos = match pos.checked_add(cs) {
            Some(p) => p,
            None => break,
        };
    }
    Ok(doc)
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

impl Document {
    fn prefix_for(&self, uri: &str) -> Option<&str> {
        self.namespaces.iter().find(|(_, u)| u == uri).map(|(p, _)| p.as_str())
    }

    /// An XML text rendition (for reading, not byte-exact source).
    pub fn to_xml(&self, max_bytes: usize) -> String {
        let mut out = String::from("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n");
        let mut first = true;
        for n in &self.nodes {
            if out.len() > max_bytes {
                out.push_str("<!-- truncated -->\n");
                break;
            }
            match n {
                Node::Start { depth, name, attributes, .. } => {
                    out.push_str(&"  ".repeat((*depth).min(64)));
                    out.push('<');
                    out.push_str(name);
                    if first {
                        for (p, u) in &self.namespaces {
                            out.push_str(&format!(" xmlns:{p}=\"{}\"", esc(u)));
                        }
                        first = false;
                    }
                    for a in attributes {
                        let p = a.namespace.as_deref().and_then(|u| self.prefix_for(u)).map(|p| format!("{p}:")).unwrap_or_default();
                        out.push_str(&format!(" {p}{}=\"{}\"", a.name, esc(&a.value)));
                    }
                    out.push_str(">\n");
                }
                Node::End { depth, name } => {
                    out.push_str(&"  ".repeat((*depth).min(64)));
                    out.push_str(&format!("</{name}>\n"));
                }
                Node::Text { depth, text } => {
                    out.push_str(&"  ".repeat((*depth).min(64)));
                    out.push_str(&esc(text));
                    out.push('\n');
                }
                Node::Namespace { .. } => {}
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures as fx;

    fn pool_with_offsets(offsets: &[u32], string_data: &[u8]) -> Vec<u8> {
        let start = 28 + offsets.len() * 4;
        let mut data = vec![0; start];
        data[..2].copy_from_slice(&RES_STRING_POOL.to_le_bytes());
        data[2..4].copy_from_slice(&28u16.to_le_bytes());
        data[8..12].copy_from_slice(&(offsets.len() as u32).to_le_bytes());
        data[16..20].copy_from_slice(&0x100u32.to_le_bytes());
        data[20..24].copy_from_slice(&(start as u32).to_le_bytes());
        for (i, offset) in offsets.iter().enumerate() {
            let at = 28 + i * 4;
            data[at..at + 4].copy_from_slice(&offset.to_le_bytes());
        }
        data.extend_from_slice(string_data);
        data.resize(data.len().next_multiple_of(4), b'x');
        let size = data.len() as u32;
        data[4..8].copy_from_slice(&size.to_le_bytes());
        data
    }

    fn append_legacy_ascii(data: &mut Vec<u8>, len: usize) -> u32 {
        let offset = data.len() as u32;
        let encoded = [0x80 | ((len >> 8) & 0x7f) as u8, (len & 0xff) as u8];
        data.extend_from_slice(&encoded);
        data.extend_from_slice(&encoded);
        data.resize(data.len() + len, b'x');
        data.push(0);
        offset
    }

    fn shared_ascii_pool(utf8: bool, count: usize) -> Vec<u8> {
        let len = 16_384usize;
        let mut strings = Vec::new();
        if utf8 {
            append_legacy_ascii(&mut strings, len);
        } else {
            strings.extend_from_slice(&(len as u16).to_le_bytes());
            for _ in 0..len {
                strings.extend_from_slice(b"x\0");
            }
            strings.extend_from_slice(&[0, 0]);
        }
        let mut data = pool_with_offsets(&vec![0; count], &strings);
        if !utf8 {
            data[16..20].fill(0);
        }
        data
    }

    #[test]
    fn shared_offsets_spend_a_pool_byte_budget() {
        // Each parse keeps 64 MiB of strings: the gate runs the UTF-8 pool with the appended bytes
        // (the budget and the appended bytes together), nightly all four.
        let full = craft_testkit::hostile::divisor() == 1;
        let (encodings, appends): (&[bool], &[bool]) = if full { (&[true, false], &[false, true]) } else { (&[true], &[true]) };
        for &utf8 in encodings {
            let mut data = shared_ascii_pool(utf8, 8192);
            for &appended in appends {
                if appended {
                    // Unrelated bytes after the chunk cannot buy more retained copies.
                    data.resize(24 << 20, 0);
                }
                let pool = StringPool::parse(&data, 0).unwrap();
                assert_eq!(pool.declared, 8192);
                assert_eq!(pool.strings.len(), 4096);
                assert_eq!(pool.strings.iter().map(String::len).sum::<usize>(), 64 << 20);
                assert_eq!(pool.get(4095).map(str::len), Some(16_384));
                assert_eq!(pool.get(4096), None);
            }
            let data = shared_ascii_pool(utf8, 2);
            let pool = StringPool::parse(&data, 0).unwrap();
            assert_eq!(pool.strings.len(), 2);
            assert_eq!(pool.get(0), pool.get(1));
        }
    }

    fn pool_container(kind: u16, header: usize, children: &[u8]) -> Vec<u8> {
        let mut data = vec![0; header];
        data[0..2].copy_from_slice(&kind.to_le_bytes());
        data[2..4].copy_from_slice(&(header as u16).to_le_bytes());
        data.extend_from_slice(children);
        let len = data.len() as u32;
        data[4..8].copy_from_slice(&len.to_le_bytes());
        data
    }

    #[test]
    fn xml_and_resource_tables_disclose_partial_string_pools() {
        let pool = shared_ascii_pool(true, 8192);
        {
            let doc = parse(&pool_container(RES_XML, 8, &pool)).unwrap();
            assert!(doc.truncated);
            assert!(doc.warnings.iter().any(|w| w.contains("string pool") && w.contains("4096 of 8192")));
            assert_eq!(doc.pool.strings.len(), 4096);
        }
        let table = crate::arsc::parse(&pool_container(2, 12, &pool), crate::arsc::Limits::default()).unwrap();
        assert!(table.truncated);
        assert_eq!(table.global_strings, 8192);
        assert!(table.warnings.iter().any(|w| w.contains("global string pool") && w.contains("4096 of 8192")));
        for (field, label) in [(268, "type"), (276, "key")] {
            let mut package = pool_container(0x0200, 288, &pool);
            package[8..12].copy_from_slice(&0x7fu32.to_le_bytes());
            package[field..field + 4].copy_from_slice(&288u32.to_le_bytes());
            let table = crate::arsc::parse(&pool_container(2, 12, &package), crate::arsc::Limits::default()).unwrap();
            assert!(table.truncated, "{label}");
            assert!(table.warnings.iter().any(|w| w.contains(&format!("{label} string pool")) && w.contains("4096 of 8192")));
            assert_eq!(table.packages.len(), 1);
        }
    }

    #[test]
    fn repeated_offsets_share_the_utf8_recovery_budget() {
        let mut strings = Vec::new();
        let long = append_legacy_ascii(&mut strings, 0x8000);
        let short = strings.len() as u32;
        strings.extend_from_slice(b"\x02\x02ok\0");
        let data = pool_with_offsets(&[long, long, long, short], &strings);
        let mut left = 2;
        let parsed = StringPool::parse_with_recovery_budget(&data, 0, &mut left).unwrap();
        assert_eq!(left, 0);
        assert_eq!(parsed.declared, 4);
        assert_eq!(parsed.strings.len(), 4);
        assert_eq!(parsed.get(0).map(str::len), Some(0x8000));
        assert_eq!(parsed.get(1), parsed.get(0));
        assert_eq!(parsed.get(2), Some(""));
        assert_eq!(parsed.get(3), Some("ok"));
        assert_eq!(parsed.recovery_omitted, 1);
    }

    #[test]
    fn distinct_offsets_share_the_utf8_recovery_budget() {
        let mut strings = Vec::new();
        let mut offsets: Vec<u32> = (0..3).map(|_| append_legacy_ascii(&mut strings, 0x8000)).collect();
        offsets.push(strings.len() as u32);
        strings.extend_from_slice(b"\x02\x02ok\0");
        let data = pool_with_offsets(&offsets, &strings);
        let mut left = 2;
        let parsed = StringPool::parse_with_recovery_budget(&data, 0, &mut left).unwrap();
        assert_eq!(left, 0);
        assert_eq!(parsed.get(0).map(str::len), Some(0x8000));
        assert_eq!(parsed.get(1).map(str::len), Some(0x8000));
        assert_eq!(parsed.get(2), Some(""));
        assert_eq!(parsed.get(3), Some("ok"));
        assert_eq!(parsed.recovery_omitted, 1);
    }

    #[test]
    fn utf8_recovery_stops_before_a_probe_without_credit() {
        let mut strings = Vec::new();
        let long = append_legacy_ascii(&mut strings, 0x10000);
        let short = strings.len() as u32;
        strings.extend_from_slice(b"\x02\x02ok\0");
        let data = pool_with_offsets(&[long, short], &strings);
        for (allowance, length, remaining) in [(0, 0, 0), (1, 0, 0), (2, 0x10000, 0), (3, 0x10000, 1)] {
            let mut left = allowance;
            let parsed = StringPool::parse_with_recovery_budget(&data, 0, &mut left).unwrap();
            assert_eq!(left, remaining);
            assert_eq!(parsed.get(0).map(str::len), Some(length));
            assert_eq!(parsed.get(1), Some("ok"));
            assert_eq!(parsed.recovery_omitted, usize::from(length == 0));
        }
    }

    #[test]
    fn short_strings_and_out_of_bounds_candidates_spend_no_recovery_probes() {
        let data = pool_with_offsets(&[0, 5], b"\x80\0\x80\0x\x02\x02ok\0");
        let mut left = 7;
        let parsed = StringPool::parse_with_recovery_budget(&data, 0, &mut left).unwrap();
        assert_eq!(left, 7);
        assert_eq!(parsed.get(0), Some(""));
        assert_eq!(parsed.get(1), Some("ok"));
        assert_eq!(parsed.recovery_omitted, 0);
    }

    fn utf8_recovery_exhausted_pool() -> (Vec<u8>, usize) {
        let mut strings = vec![b'x'; 2 * 0x8000 + 16];
        strings[..4].copy_from_slice(&[0x80, 0, 0x80, 0]);
        let long = append_legacy_ascii(&mut strings, 0x8000);
        let short = strings.len() as u32;
        strings.extend_from_slice(b"\x02\x02ok\0");
        // Offset zero has three in-bounds extra candidates, all nonzero.
        let repeated = MAX_RECOVERY_PROBES / 3 + 1;
        let mut offsets = vec![0; repeated];
        offsets.extend([long, short]);
        (pool_with_offsets(&offsets, &strings), repeated)
    }

    #[test]
    fn default_pool_budget_stops_repeated_recovery_before_later_long_strings() {
        let (data, repeated) = utf8_recovery_exhausted_pool();
        let parsed = StringPool::parse(&data, 0).unwrap();
        assert_eq!(parsed.declared, repeated + 2);
        assert_eq!(parsed.strings.len(), repeated + 2);
        assert!(parsed.strings[..repeated].iter().all(String::is_empty));
        assert_eq!(parsed.get(repeated as u32), Some(""));
        assert_eq!(parsed.get((repeated + 1) as u32), Some("ok"));
        assert!(parsed.recovery_omitted > 0);
    }

    #[test]
    fn xml_and_resource_tables_disclose_recovery_omissions() {
        let (pool, repeated) = utf8_recovery_exhausted_pool();
        let short_index = (repeated + 1) as u32;
        {
            let doc = parse(&pool_container(RES_XML, 8, &pool)).unwrap();
            assert!(doc.truncated);
            assert!(doc.warnings.iter().any(|w| w.contains("string pool") && w.contains("UTF-8 recovery budget")));
            assert_eq!(doc.pool.get(short_index), Some("ok"));
            assert!(doc.pool.recovery_omitted > 0);
        }
        let table = crate::arsc::parse(&pool_container(2, 12, &pool), crate::arsc::Limits::default()).unwrap();
        assert!(table.truncated);
        assert!(table.warnings.iter().any(|w| w.contains("global string pool") && w.contains("UTF-8 recovery budget")));
        for (field, label) in [(268, "type"), (276, "key")] {
            let mut package = pool_container(0x0200, 288, &pool);
            package[8..12].copy_from_slice(&0x7fu32.to_le_bytes());
            package[field..field + 4].copy_from_slice(&288u32.to_le_bytes());
            let table = crate::arsc::parse(&pool_container(2, 12, &package), crate::arsc::Limits::default()).unwrap();
            assert!(table.truncated, "{label}");
            assert!(
                table.warnings.iter().any(|w| w.contains(&format!("{label} string pool")) && w.contains("UTF-8 recovery budget")),
                "{label}: {:?}",
                table.warnings
            );
        }
    }

    #[test]
    fn xml_value_budget_omits_long_copies_and_keeps_a_later_short_value() {
        let long = "x".repeat(16);
        let attrs = vec![
            fx::AxAttr { name: "a", res_id: None, val: fx::AxVal::Str(&long) },
            fx::AxAttr { name: "b", res_id: None, val: fx::AxVal::Str(&long) },
            fx::AxAttr { name: "c", res_id: None, val: fx::AxVal::Str("ok") },
        ];
        let bytes = fx::axml(&[fx::AxNode::Start("root", attrs), fx::AxNode::End("root")]);
        let complete = parse(&bytes).unwrap();
        assert!(!complete.truncated);
        let Node::Start { attributes, .. } = complete.nodes.iter().find(|n| matches!(n, Node::Start { name, .. } if name == "root")).unwrap() else {
            panic!("expected the root start");
        };
        assert_eq!(
            attributes.iter().map(|a| (a.name.as_str(), a.value.as_str())).collect::<Vec<_>>(),
            [("a", long.as_str()), ("b", long.as_str()), ("c", "ok")]
        );

        let partial = parse_with_limits(&bytes, MAX_NODES, MAX_DEPTH, 20).unwrap();
        assert!(partial.truncated);
        assert!(partial.warnings.iter().any(|w| w.contains("value budget")));
        let Node::Start { attributes, .. } = partial.nodes.iter().find(|n| matches!(n, Node::Start { name, .. } if name == "root")).unwrap() else {
            panic!("expected the root start");
        };
        assert_eq!(
            attributes.iter().map(|a| (a.name.as_str(), a.value.as_str())).collect::<Vec<_>>(),
            [("a", long.as_str()), ("b", ""), ("c", "ok")]
        );
    }

    fn chunk_at(data: &[u8], kind: u16) -> usize {
        let r = Reader::new(data);
        let mut pos = 8;
        loop {
            let (t, _, n) = chunk(&r, pos).unwrap();
            if t == kind {
                return pos;
            }
            pos += n;
        }
    }

    #[test]
    fn namespace_extensions_follow_the_declared_header() {
        let mut data = fx::manifest_axml();
        let pos = chunk_at(&data, XML_START_NS);
        data.splice(pos + 16..pos + 16, [0xff; 4]);
        data[pos + 2..pos + 4].copy_from_slice(&20u16.to_le_bytes());
        data[pos + 4..pos + 8].copy_from_slice(&28u32.to_le_bytes());
        let len = data.len() as u32;
        data[4..8].copy_from_slice(&len.to_le_bytes());
        let doc = parse(&data).unwrap();
        assert!(doc.warnings.is_empty());
        assert_eq!(doc.namespaces[0], ("android".into(), ANDROID_NS.into()));
    }

    #[test]
    fn extended_attribute_strides_keep_each_manifest_value() {
        let mut data = fx::manifest_axml();
        let pos = chunk_at(&data, XML_START_ELEMENT);
        let ext = pos + 16;
        // The fixture writes three 20-byte attributes with 8-byte typed values.
        for index in (1..=3).rev() {
            let end = ext + 20 + index * 20;
            data.splice(end..end, [0xff; 4]);
        }
        data[ext + 10..ext + 12].copy_from_slice(&24u16.to_le_bytes());
        data[pos + 4..pos + 8].copy_from_slice(&(36 + 3 * 24u32).to_le_bytes());
        let len = data.len() as u32;
        data[4..8].copy_from_slice(&len.to_le_bytes());

        let doc = parse(&data).unwrap();
        assert!(!doc.truncated);
        assert!(doc.warnings.is_empty(), "{:?}", doc.warnings);
        let Node::Start { attributes, .. } = &doc.nodes[1] else {
            panic!("expected the manifest start after the namespace");
        };
        let pairs: Vec<_> = attributes.iter().map(|a| (a.name.as_str(), a.value.as_str())).collect();
        assert_eq!(pairs, [("package", fx::PACKAGE), ("versionCode", "42"), ("versionName", "1.2.3")]);
    }

    #[test]
    fn dropping_a_close_event_marks_the_document_partial() {
        let bytes = fx::axml(&[fx::AxNode::Start("root", vec![]), fx::AxNode::End("root")]);
        let complete = parse_with_limits(&bytes, 3, 1, MAX_RETAINED_VALUE_BYTES).unwrap();
        assert!(!complete.truncated);
        assert!(matches!(complete.nodes.last(), Some(Node::End { depth: 0, .. })));

        let partial = parse_with_limits(&bytes, 2, 1, MAX_RETAINED_VALUE_BYTES).unwrap();
        assert!(partial.truncated);
        assert_eq!(partial.nodes, complete.nodes[..2]);
    }

    #[test]
    fn nesting_limit_keeps_an_exact_prefix_instead_of_saturating_depth() {
        let bytes =
            fx::axml(&[fx::AxNode::Start("root", vec![]), fx::AxNode::Start("child", vec![]), fx::AxNode::End("child"), fx::AxNode::End("root")]);
        let complete = parse_with_limits(&bytes, 5, 2, MAX_RETAINED_VALUE_BYTES).unwrap();
        assert!(!complete.truncated);
        assert!(matches!(complete.nodes[3], Node::End { depth: 1, .. }));
        assert!(matches!(complete.nodes[4], Node::End { depth: 0, .. }));

        let partial = parse_with_limits(&bytes, 5, 1, MAX_RETAINED_VALUE_BYTES).unwrap();
        assert!(partial.truncated);
        assert_eq!(partial.nodes, complete.nodes[..2]);
    }

    #[test]
    fn values_format() {
        let p = StringPool { strings: vec!["x".into()], utf8: false, declared: 1, recovery_omitted: 0 };
        assert_eq!(ResValue { data_type: 0x05, data: (16 << 8) | 1 }.format(&p), "16dp");
        assert_eq!(ResValue { data_type: 0x12, data: 0xffff_ffff }.format(&p), "true");
        assert_eq!(ResValue { data_type: 0x1c, data: 0xff00_ff00 }.format(&p), "#ff00ff00");
        assert_eq!(ResValue { data_type: 0x10, data: (-5i32) as u32 }.format(&p), "-5");
        assert_eq!(ResValue { data_type: 0x03, data: 0 }.format(&p), "x");
        assert_eq!(ResValue { data_type: 0x04, data: 1.5f32.to_bits() }.format(&p), "1.5");
    }
}
