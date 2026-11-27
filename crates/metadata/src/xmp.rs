//! XMP packets (ISO 16684-1 / Adobe XMP Specification Part 1–2): read and write.
//!
//! Reading builds a small namespace-resolved tree with `quick-xml`, then collects every property of every
//! `rdf:Description` (attribute form, element form, `rdf:Seq`/`rdf:Bag`/`rdf:Alt` arrays, `rdf:parseType="Resource"`
//! and nested-description structs, flattened as `parent/field`). Prefixes are canonicalised by namespace URI, so
//! a packet that binds `http://purl.org/dc/elements/1.1/` to `foo:` still yields `dc:title`.
//!
//! Writing emits the interchange subset in standard namespaces (dc, xmp, photoshop, exif, exifEX, tiff, lr) and
//! any simple properties of one further namespace the caller names ([`XmpWrite`]).

use crate::geom::{Point, Rect};
use crate::{DateTime, Flash, Gps, Metadata, Orientation, Region, RegionKind, parse_number};
use quick_xml::escape::{escape, partial_escape};
use quick_xml::events::Event;
use std::collections::BTreeMap;

/// A namespace beyond the standard ones, such as a program's own: properties in it are reported
/// under `prefix`, whatever prefix the packet binds the URI to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct XmpNamespace<'a> {
    pub prefix: &'a str,
    pub uri: &'a str,
}

/// What [`write_xmp_with`] writes beside the metadata.
#[derive(Clone, Copy, Debug)]
pub struct XmpWrite<'a> {
    /// The `x:xmptk` attribute: the name of the writing program.
    pub toolkit: &'a str,
    /// The namespace of `properties`. Without one, `properties` are not written.
    pub namespace: Option<XmpNamespace<'a>>,
    /// Simple properties in `namespace`, as `(local name, text)`. A name that is not ASCII letters,
    /// digits and `_` is left out.
    pub properties: &'a [(&'a str, &'a str)],
}

impl Default for XmpWrite<'_> {
    fn default() -> Self {
        Self { toolkit: "craft-metadata", namespace: None, properties: &[] }
    }
}

const NAMESPACES: &[(&str, &str)] = &[
    ("x", "adobe:ns:meta/"),
    ("rdf", "http://www.w3.org/1999/02/22-rdf-syntax-ns#"),
    ("dc", "http://purl.org/dc/elements/1.1/"),
    ("xmp", "http://ns.adobe.com/xap/1.0/"),
    ("xmpRights", "http://ns.adobe.com/xap/1.0/rights/"),
    ("photoshop", "http://ns.adobe.com/photoshop/1.0/"),
    ("exif", "http://ns.adobe.com/exif/1.0/"),
    ("exifEX", "http://cipa.jp/exif/1.0/"),
    ("tiff", "http://ns.adobe.com/tiff/1.0/"),
    ("aux", "http://ns.adobe.com/exif/1.0/aux/"),
    ("lr", "http://ns.adobe.com/lightroom/1.0/"),
    ("Iptc4xmpCore", "http://iptc.org/std/Iptc4xmpCore/1.0/xmlns/"),
    ("Iptc4xmpExt", "http://iptc.org/std/Iptc4xmpExt/2008-02-29/"),
    ("xmpNote", "http://ns.adobe.com/xmp/note/"),
    // Read-only: develop settings written by other raw developers.
    ("crs", CRS_NS),
    // Read-only: face/pet/focus regions written by Lightroom, digiKam, Picasa and others (MWG Region
    // Guidelines v2.0). Canonicalising these three lets a packet use any prefix for them.
    ("mwg-rs", "http://www.metadataworkinggroup.com/schemas/regions/"),
    ("stArea", "http://ns.adobe.com/xmp/sType/Area#"),
    ("stDim", "http://ns.adobe.com/xap/1.0/sType/Dimensions#"),
];

/// The camera-raw-settings namespace URI (prefix `crs`), read for interchange only.
pub const CRS_NS: &str = "http://ns.adobe.com/camera-raw-settings/1.0/";

/// XMP read failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum XmpError {
    Xml(String),
    /// Nesting / size limits exceeded.
    TooComplex,
}

impl std::fmt::Display for XmpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            XmpError::Xml(e) => write!(f, "XMP is not well-formed XML: {e}"),
            XmpError::TooComplex => write!(f, "XMP packet too deeply nested or too large"),
        }
    }
}

impl std::error::Error for XmpError {}

/// Result of [`parse_xmp`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct XmpData {
    pub metadata: Metadata,
    /// Every property found, keyed `prefix:name` (struct fields as `prefix:name/prefix:field`); arrays keep
    /// item order, language alternatives put `x-default` first. Foreign namespaces that reuse a recognized
    /// prefix use `Q{hex-encoded namespace URI}local` instead, so they cannot impersonate known properties.
    pub properties: BTreeMap<String, Vec<String>>,
    /// True when flattened properties were omitted after reaching the cumulative key-size limit.
    pub properties_truncated: bool,
    /// Every top-level property with its full structure (arrays of structs such as local
    /// corrections, which [`XmpData::properties`] flattens).
    pub values: BTreeMap<String, XmpValue>,
    /// The `x:xmptk` attribute of `x:xmpmeta` (older packets: `x:xaptk` of `x:xapmeta`), the name
    /// of the program that wrote the packet. [`write_xmp_with`] writes [`XmpWrite::toolkit`] there.
    pub toolkit: Option<String>,
    /// True when the parsed tree, before flattening or merging repeated properties, names an Exif
    /// GPS property anywhere or holds a place property (see `LOCATION_PROPERTIES`) with non-blank
    /// text anywhere, including inside array items that [`XmpData::properties`] flattens to their
    /// text and repeated structs that [`XmpData::values`] keeps only once.
    pub names_location: bool,
}

/// Properties that name a place; a blank one is not a location.
const LOCATION_PROPERTIES: [&str; 7] = [
    "Iptc4xmpCore:Location",
    "Iptc4xmpCore:CountryCode",
    "photoshop:City",
    "photoshop:State",
    "photoshop:Country",
    "Iptc4xmpExt:LocationShown",
    "Iptc4xmpExt:LocationCreated",
];

impl XmpData {
    /// Whether the XMP inventory names GPS or a non-empty place field. LocationShown and
    /// LocationCreated are IPTC Extension structures; their flattened array entries can be empty
    /// strings even when their struct children contain a city or coordinates, so inspect their
    /// retained structured value and flattened child fields as well. Callers that must prove
    /// absence must check [`Self::properties_truncated`] first.
    pub fn has_location_metadata(&self) -> bool {
        const IPTC_LOCATION_STRUCTS: [&str; 2] = ["Iptc4xmpExt:LocationShown", "Iptc4xmpExt:LocationCreated"];
        // Every flattened child of one LocationShown/LocationCreated root shares its answer.
        let mut checked_roots: BTreeMap<&str, bool> = BTreeMap::new();
        if self.names_location {
            return true;
        }

        for (key, values) in &self.properties {
            let parts = key.split('/');
            if parts.clone().any(is_gps_name) {
                // Even an isolated/blank GPS property is not proof that GPS was removed.
                return true;
            }

            let Some(location_part) = parts.clone().find(|part| LOCATION_PROPERTIES.contains(part)) else {
                continue;
            };

            if values.iter().any(|value| !value.trim().is_empty()) {
                return true;
            }
            if IPTC_LOCATION_STRUCTS.contains(&location_part) {
                // The scope is the path up to the location field itself, so text elsewhere in an
                // enclosing struct doesn't count as a place.
                let scope_len =
                    key.split('/').take_while(|part| *part != location_part).map(|part| part.len() + 1).sum::<usize>() + location_part.len();
                let scope = key.get(..scope_len).unwrap_or(key);
                let scope_has_text = *checked_roots.entry(scope).or_insert_with(|| self.location_scope_has_text(scope));
                if scope_has_text {
                    return true;
                }
            }
        }

        false
    }

    /// Whether a LocationShown/LocationCreated field (a `/`-separated path from a top-level
    /// property) holds text in its retained structured value or any flattened child. `values`
    /// keeps the first text-bearing occurrence across RDF descriptions, while flattened child
    /// properties retain every occurrence.
    fn location_scope_has_text(&self, scope: &str) -> bool {
        let mut parts = scope.split('/');
        let structured = parts.next().and_then(|root| self.values.get(root)).and_then(|value| parts.try_fold(value, |value, part| value.field(part)));
        if structured.is_some_and(value_has_text) {
            return true;
        }
        let child_prefix = format!("{scope}/");
        self.properties
            .range::<str, _>((std::ops::Bound::Included(child_prefix.as_str()), std::ops::Bound::Unbounded))
            .take_while(|(candidate, _)| candidate.starts_with(&child_prefix))
            .any(|(_, nested_values)| nested_values.iter().any(|value| !value.trim().is_empty()))
    }

    /// The GUID of the JPEG Extended XMP continuation the packet announces
    /// (`xmpNote:HasExtendedXMP`), matched by local name so an undeclared or foreign prefix can't
    /// hide the announcement. `Some("")` when the announcement is blank.
    pub fn extended_xmp_guid(&self) -> Option<&str> {
        self.properties
            .iter()
            .find(|(key, _)| !key.contains('/') && key.rsplit([':', '}']).next() == Some("HasExtendedXMP"))
            .map(|(_, values)| values.first().map_or("", |value| value.trim()))
    }
}

impl XmpData {
    /// The pixel size the packet's regions were drawn on (`mwg-rs:AppliedToDimensions`), unless one of them says how
    /// the photo was turned (`mwg-rs:Rotation`: Lightroom's boxes, which the reader already turns upright). It tells
    /// boxes drawn on the photo as stored from boxes drawn on it upright: see [`crate::upright_regions`].
    pub fn regions_drawn_on(&self) -> Option<(f64, f64)> {
        let regions = self.values.get("mwg-rs:Regions")?;
        let turned = regions.field("mwg-rs:RegionList").is_some_and(|l| l.items().iter().any(|r| r.field("mwg-rs:Rotation").is_some()));
        if turned { None } else { applied_to(regions) }
    }
}

/// A structured XMP value.
#[derive(Clone, Debug, PartialEq)]
pub enum XmpValue {
    Text(String),
    Array(Vec<XmpValue>),
    Struct(BTreeMap<String, XmpValue>),
}

impl XmpValue {
    /// A struct field (`prefix:name`).
    pub fn field(&self, k: &str) -> Option<&XmpValue> {
        match self {
            XmpValue::Struct(m) => m.get(k),
            _ => None,
        }
    }
    pub fn text(&self) -> Option<&str> {
        match self {
            XmpValue::Text(s) => Some(s.trim()),
            XmpValue::Array(a) => a.first().and_then(XmpValue::text),
            XmpValue::Struct(_) => None,
        }
    }
    /// Array items (a lone value is a one-item array).
    pub fn items(&self) -> &[XmpValue] {
        match self {
            XmpValue::Array(a) => a,
            other => std::slice::from_ref(other),
        }
    }
}

/// Whether a structured value holds any non-blank text. A tree larger than the parser's node
/// budget counts as text-bearing (the conservative answer for a privacy check): parsed trees are
/// capped at MAX_NODES, so only a caller-built `XmpData` can reach it, and the walk stays bounded.
fn value_has_text(value: &XmpValue) -> bool {
    let mut pending = Vec::new();
    if pending.try_reserve(1).is_err() {
        return true;
    }
    pending.push(value);
    let mut visited = 0usize;
    while let Some(value) = pending.pop() {
        visited = visited.saturating_add(1);
        if visited > MAX_NODES {
            return true;
        }
        let children = match value {
            XmpValue::Text(text) if !text.trim().is_empty() => return true,
            XmpValue::Text(_) => continue,
            XmpValue::Array(items) => items.len(),
            XmpValue::Struct(fields) => fields.len(),
        };
        if pending.len().saturating_add(children) > MAX_NODES || pending.try_reserve(children).is_err() {
            return true;
        }
        match value {
            XmpValue::Array(items) => pending.extend(items),
            XmpValue::Struct(fields) => pending.extend(fields.values()),
            XmpValue::Text(_) => {}
        }
    }
    false
}

/// Insert a property into a struct or description map, keeping an earlier text-bearing value when
/// the same name repeats with a blank one: an IPTC location struct repeated with an empty
/// `rdf:li` must not hide the place an export's Remove Location has to refuse. `has_text`
/// remembers each key's answer so every value is walked at most once.
fn insert_keeping_text(map: &mut BTreeMap<String, XmpValue>, has_text: &mut BTreeMap<String, bool>, key: String, value: XmpValue) {
    match map.entry(key) {
        std::collections::btree_map::Entry::Vacant(slot) => {
            slot.insert(value);
        }
        std::collections::btree_map::Entry::Occupied(mut slot) => {
            let existing_has_text = match has_text.get(slot.key()) {
                Some(known) => *known,
                None => value_has_text(slot.get()),
            };
            if existing_has_text {
                has_text.insert(slot.key().clone(), true);
                return;
            }
            let new_has_text = value_has_text(&value);
            has_text.insert(slot.key().clone(), new_has_text);
            slot.insert(value);
        }
    }
}

/// A description or struct node as a map of its fields (attributes and child elements).
fn node_struct(n: &Node) -> XmpValue {
    let mut m = BTreeMap::new();
    let mut has_text = BTreeMap::new();
    for (k, v) in &n.attrs {
        if !is_rdf_meta_attr(k) {
            insert_keeping_text(&mut m, &mut has_text, k.clone(), XmpValue::Text(v.clone()));
        }
    }
    for c in &n.children {
        insert_keeping_text(&mut m, &mut has_text, c.name.clone(), node_value(c));
    }
    XmpValue::Struct(m)
}

fn node_value(n: &Node) -> XmpValue {
    if let Some(arr) = n.children.iter().find(|c| matches!(c.name.as_str(), "rdf:Seq" | "rdf:Bag" | "rdf:Alt")) {
        return XmpValue::Array(arr.children.iter().filter(|li| li.name == "rdf:li").map(node_value).collect());
    }
    if let Some(d) = n.children.iter().find(|c| c.name == "rdf:Description") {
        return node_struct(d);
    }
    if !n.children.is_empty() || n.attrs.iter().any(|(k, _)| !is_rdf_meta_attr(k)) {
        return node_struct(n);
    }
    XmpValue::Text(n.attr("rdf:resource").map(str::to_string).unwrap_or_else(|| n.text.trim().to_string()))
}

#[derive(Debug, Default)]
struct Node {
    name: String,
    attrs: Vec<(String, String)>,
    children: Vec<Node>,
    text: String,
}

impl Node {
    fn attr(&self, n: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == n).map(|(_, v)| v.as_str())
    }
}

const MAX_DEPTH: usize = 64;
const MAX_NODES: usize = 200_000;
// Expanding a long namespace URI on many short names must not amplify a small packet without bound.
pub(crate) const MAX_OPAQUE_NAME_BYTES: usize = 8 * 1024 * 1024;

/// Most text a converter's flattened property keys may add up to. Keys repeat their whole ancestor
/// path (`parent/field`), so a document with a long nested name and a wide child fan-out otherwise
/// retains its prefix once per property: node limits alone cannot bound that duplication.
const MAX_KEY_BYTES: usize = 1 << 20;

/// Most namespace declarations in scope at once. Prefix lookup scans them, so declarations times the names of a
/// start tag is the work; real packets declare a few dozen.
const MAX_NS_DECLS: usize = 512;

/// Refuses a scope stack holding more than [`MAX_NS_DECLS`] declarations (one count per start tag over a stack at most [`MAX_DEPTH`] deep).
pub(crate) fn check_scopes(scopes: &[Vec<(String, String)>]) -> Result<(), XmpError> {
    if scopes.iter().map(Vec::len).sum::<usize>() > MAX_NS_DECLS {
        return Err(XmpError::TooComplex);
    }
    Ok(())
}

/// Names canonicalised by namespace URI (see [`NAMESPACES`]); conflicting foreign prefixes are opaque.
pub(crate) fn canonical_name(
    qname: &str,
    scopes: &[Vec<(String, String)>],
    extra: &[XmpNamespace],
    opaque_budget: &mut usize,
) -> Result<String, XmpError> {
    let known = || NAMESPACES.iter().copied().chain(extra.iter().map(|n| (n.prefix, n.uri)));
    let (prefix, local) = qname.split_once(':').unwrap_or(("", qname));
    if prefix == "xml" || prefix == "xmlns" {
        return Ok(qname.to_string());
    }
    let uri = scopes.iter().rev().flat_map(|s| s.iter().rev()).find(|(p, _)| p == prefix).map(|(_, u)| u.as_str());
    Ok(match uri {
        Some(u) => match known().find(|(_, nu)| *nu == u) {
            Some((p, _)) => format!("{p}:{local}"),
            None if known().any(|(p, _)| p == prefix) => {
                // A foreign `app:settings` must never match the caller's `app:*` ownership rule (nor may a
                // foreign `xmp:Rating` become a rating). Braces cannot occur in an XML QName;
                // hex keeps URI slashes out of the flattened property's `/` path separators.
                let size =
                    u.len().checked_mul(2).and_then(|n| n.checked_add(3)).and_then(|n| n.checked_add(local.len())).ok_or(XmpError::TooComplex)?;
                *opaque_budget = opaque_budget.checked_sub(size).ok_or(XmpError::TooComplex)?;
                let mut key = String::with_capacity(size);
                key.push_str("Q{");
                for byte in u.bytes() {
                    for nibble in [byte >> 4, byte & 15] {
                        key.push(char::from(if nibble < 10 { b'0' + nibble } else { b'a' + (nibble - 10) }));
                    }
                }
                key.push('}');
                key.push_str(local);
                key
            }
            None if prefix.is_empty() => local.to_string(),
            None => qname.to_string(),
        },
        None => qname.to_string(),
    })
}

fn entity(name: &str) -> Option<char> {
    Some(match name {
        "lt" => '<',
        "gt" => '>',
        "amp" => '&',
        "quot" => '"',
        "apos" => '\'',
        _ => {
            let n = name.strip_prefix('#')?;
            let v = match n.strip_prefix(['x', 'X']) {
                Some(h) if h.bytes().all(|b| b.is_ascii_hexdigit()) => u32::from_str_radix(h, 16).ok()?,
                Some(_) => return None,
                None if n.bytes().all(|b| b.is_ascii_digit()) => n.parse().ok()?,
                None => return None,
            };
            char::from_u32(v).filter(|&c| is_xml_char(c))?
        }
    })
}

/// The XML 1.0 `Char` production (the surrogate range can't occur in a `char`).
fn is_xml_char(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\r') || (c >= '\u{20}' && c != '\u{FFFE}' && c != '\u{FFFF}')
}

/// Drop the characters XML 1.0 forbids, which strict parsers (Adobe XMP Toolkit, expat, libxml2) reject the
/// whole document for.
fn xml_safe(s: &str) -> String {
    s.chars().filter(|&c| is_xml_char(c)).collect()
}

/// Escaped text with each carriage return as `&#13;`: XML end-of-line handling turns a literal one
/// into a line feed on read.
fn keep_cr(escaped: std::borrow::Cow<'_, str>) -> String {
    escaped.replace('\r', "&#13;")
}

/// Index of the `;` that ends a reference starting at `rest` (its `&`). The scan stops at the first
/// byte that cannot appear in a name or a numeric reference, so zero-padded references such as
/// `&#000000097;` are found and each run of name bytes is read once, not once per `&`.
fn entity_end(rest: &str) -> Option<usize> {
    let n = rest.bytes().skip(1).take_while(|b| b.is_ascii_alphanumeric() || *b == b'#').count();
    (rest.as_bytes().get(n + 1) == Some(&b';')).then_some(n + 1)
}

pub(crate) fn unescape_attr(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        match entity_end(rest).and_then(|j| entity(&rest[1..j]).map(|c| (c, j))) {
            Some((c, j)) => {
                out.push(c);
                rest = &rest[j + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn parse_tree(s: &str, extra: &[XmpNamespace]) -> Result<Node, XmpError> {
    let mut reader = quick_xml::Reader::from_str(s);
    reader.config_mut().check_end_names = false;
    let mut stack: Vec<Node> = vec![Node { name: "#root".into(), ..Default::default() }];
    let mut scopes: Vec<Vec<(String, String)>> = vec![vec![("xml".into(), "http://www.w3.org/XML/1998/namespace".into())]];
    let mut count = 0usize;
    let mut opaque_budget = MAX_OPAQUE_NAME_BYTES;
    loop {
        let ev = reader.read_event().map_err(|e| XmpError::Xml(e.to_string()))?;
        match ev {
            Event::Start(ref e) | Event::Empty(ref e) => {
                count += 1;
                if count > MAX_NODES || stack.len() > MAX_DEPTH {
                    return Err(XmpError::TooComplex);
                }
                let mut scope = Vec::new();
                let mut raw_attrs = Vec::new();
                for a in e.attributes().with_checks(false).flatten() {
                    let k = String::from_utf8_lossy(a.key.as_ref()).into_owned();
                    let v = unescape_attr(&String::from_utf8_lossy(&a.value));
                    if k == "xmlns" {
                        scope.push((String::new(), v));
                    } else if let Some(p) = k.strip_prefix("xmlns:") {
                        scope.push((p.to_string(), v));
                    } else {
                        raw_attrs.push((k, v));
                    }
                }
                scopes.push(scope);
                check_scopes(&scopes)?;
                let name = canonical_name(&String::from_utf8_lossy(e.name().as_ref()), &scopes, extra, &mut opaque_budget)?;
                let attrs = raw_attrs.into_iter().map(|(k, v)| Ok((canonical_name(&k, &scopes, extra, &mut opaque_budget)?, v))).collect::<Result<
                    _,
                    XmpError,
                >>(
                )?;
                let node = Node { name, attrs, ..Default::default() };
                if matches!(ev, Event::Empty(_)) {
                    scopes.pop();
                    if let Some(parent) = stack.last_mut() {
                        parent.children.push(node);
                    }
                } else {
                    stack.push(node);
                }
            }
            Event::End(_) if stack.len() > 1 => {
                let Some(n) = stack.pop() else { return Err(XmpError::Xml("end tag without an open element".into())) };
                scopes.pop();
                if let Some(parent) = stack.last_mut() {
                    parent.children.push(n);
                }
            }
            Event::Text(t) => {
                if let Some(top) = stack.last_mut() {
                    top.text.push_str(&t.decode().map_err(|e| XmpError::Xml(e.to_string()))?);
                }
            }
            Event::CData(t) => {
                if let Some(top) = stack.last_mut() {
                    top.text.push_str(&String::from_utf8_lossy(&t));
                }
            }
            Event::GeneralRef(r) => {
                let name = String::from_utf8_lossy(&r).into_owned();
                if let Some(top) = stack.last_mut() {
                    match entity(&name) {
                        Some(c) => top.text.push(c),
                        None => {
                            top.text.push('&');
                            top.text.push_str(&name);
                            top.text.push(';');
                        }
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    // close unterminated elements leniently
    while stack.len() > 1 {
        let Some(n) = stack.pop() else { return Err(XmpError::Xml("unterminated element without a node".into())) };
        let Some(parent) = stack.last_mut() else { return Err(XmpError::Xml("unterminated element without a parent".into())) };
        parent.children.push(n);
    }
    stack.pop().ok_or_else(|| XmpError::Xml("xmp packet without a root element".into()))
}

fn is_rdf_meta_attr(k: &str) -> bool {
    matches!(k, "rdf:about" | "rdf:ID" | "rdf:nodeID" | "xml:lang" | "rdf:parseType" | "rdf:resource" | "rdf:datatype") || k.starts_with("xmlns")
}

fn collect_description(d: &Node, prefix: &str, props: &mut BTreeMap<String, Vec<String>>, budget: &mut usize, truncated: &mut bool) {
    for (k, v) in &d.attrs {
        if is_rdf_meta_attr(k) {
            continue;
        }
        // the budget is checked before the path is built, not after
        if *budget < prefix.len().saturating_add(k.len()) {
            *truncated = true;
            continue;
        }
        let key = format!("{prefix}{k}");
        if take_key_bytes(&key, budget, truncated) {
            props.entry(key).or_default().push(v.clone());
        }
    }
    for c in &d.children {
        collect_property(c, prefix, props, budget, truncated);
    }
}

/// Charge one flattened key's length to the key budget; `false` when the budget is spent (the
/// property is then left out, like other elements a limit has cut) and the map is marked partial.
fn take_key_bytes(key: &str, budget: &mut usize, dropped: &mut bool) -> bool {
    let n = key.len();
    if *budget < n {
        *dropped = true;
        return false;
    }
    *budget -= n;
    true
}

fn collect_property(p: &Node, prefix: &str, props: &mut BTreeMap<String, Vec<String>>, budget: &mut usize, truncated: &mut bool) {
    // a path that can't fit the budget isn't built (a long ancestor path shared by many children
    // would otherwise be copied for each of them after the budget is spent)
    if *budget < prefix.len().saturating_add(p.name.len()) {
        *truncated = true;
        return;
    }
    let key = format!("{prefix}{}", p.name);
    if let Some(arr) = p.children.iter().find(|c| matches!(c.name.as_str(), "rdf:Seq" | "rdf:Bag" | "rdf:Alt")) {
        let mut items: Vec<(bool, String)> = arr
            .children
            .iter()
            .filter(|li| li.name == "rdf:li")
            .map(|li| {
                (li.attr("xml:lang") == Some("x-default"), li.attr("rdf:resource").map(str::to_string).unwrap_or_else(|| li.text.trim().to_string()))
            })
            .collect();
        if arr.name == "rdf:Alt" {
            items.sort_by_key(|(default, _)| !*default);
        }
        if take_key_bytes(&key, budget, truncated) {
            props.entry(key).or_default().extend(items.into_iter().map(|(_, v)| v));
        }
        return;
    }
    let struct_prefix = format!("{key}/");
    // a struct's path is charged like a key before its fields copy it: that work too is bounded
    // by the budget, kept fields or not
    let is_struct = p.attr("rdf:parseType") == Some("Resource") || p.children.iter().any(|c| c.name == "rdf:Description");
    if is_struct && !take_key_bytes(&key, budget, truncated) {
        return;
    }
    if p.attr("rdf:parseType") == Some("Resource") {
        // a Resource struct carries fields as attributes as well as child elements, like a nested Description
        collect_description(p, &struct_prefix, props, budget, truncated);
        return;
    }
    if let Some(d) = p.children.iter().find(|c| c.name == "rdf:Description") {
        collect_description(d, &struct_prefix, props, budget, truncated);
        return;
    }
    let field_attrs: Vec<&(String, String)> = p.attrs.iter().filter(|(k, _)| !is_rdf_meta_attr(k)).collect();
    if !field_attrs.is_empty() && p.children.is_empty() && p.text.trim().is_empty() {
        for (k, v) in field_attrs {
            let full = format!("{struct_prefix}{k}");
            if take_key_bytes(&full, budget, truncated) {
                props.entry(full).or_default().push(v.clone());
            }
        }
        return;
    }
    let v = p.attr("rdf:resource").map(str::to_string).unwrap_or_else(|| p.text.trim().to_string());
    if take_key_bytes(&key, budget, truncated) {
        props.entry(key).or_default().push(v);
    }
}

fn find_descriptions<'a>(n: &'a Node, out: &mut Vec<&'a Node>) {
    if n.name == "rdf:Description" {
        out.push(n);
        return;
    }
    for c in &n.children {
        find_descriptions(c, out);
    }
}

/// Whether a canonical property name is an Exif GPS property.
fn is_gps_name(name: &str) -> bool {
    name.starts_with("exif:GPS") || name.starts_with("exifEX:GPS")
}

/// Whether the tree names an Exif GPS property anywhere or holds a place property with non-blank
/// text anywhere. Each node is visited at most twice (a place's subtree is scanned once and not
/// walked again), and the parser caps the tree at `MAX_NODES`.
fn tree_names_location(root: &Node) -> bool {
    let mut pending = vec![root];
    while let Some(node) = pending.pop() {
        if is_gps_name(&node.name) {
            return true;
        }
        for (name, value) in &node.attrs {
            if is_gps_name(name) || (LOCATION_PROPERTIES.contains(&name.as_str()) && !value.trim().is_empty()) {
                return true;
            }
        }
        if LOCATION_PROPERTIES.contains(&node.name.as_str()) {
            if subtree_has_text(node) {
                return true;
            }
            // A blank place may still enclose a GPS property.
            if subtree_names_gps(node) {
                return true;
            }
            continue;
        }
        pending.extend(&node.children);
    }
    false
}

/// Whether a node or any descendant holds non-blank text: element text, an `rdf:resource`, or a
/// field attribute (RDF syntax attributes such as `rdf:parseType` and `xml:lang` don't count).
fn subtree_has_text(root: &Node) -> bool {
    let mut pending = vec![root];
    while let Some(node) = pending.pop() {
        if !node.text.trim().is_empty() {
            return true;
        }
        if node.attrs.iter().any(|(name, value)| (name == "rdf:resource" || !is_rdf_meta_attr(name)) && !value.trim().is_empty()) {
            return true;
        }
        pending.extend(&node.children);
    }
    false
}

/// Whether a node or any descendant is an Exif GPS property, as an element or an attribute.
fn subtree_names_gps(root: &Node) -> bool {
    let mut pending = vec![root];
    while let Some(node) = pending.pop() {
        if is_gps_name(&node.name) || node.attrs.iter().any(|(name, _)| is_gps_name(name)) {
            return true;
        }
        pending.extend(&node.children);
    }
    false
}

/// An `exif:GPSLatitude` / `exif:GPSLongitude` pair → (latitude, longitude) in signed degrees; `None`
/// unless both parse and lie on the globe (|latitude| ≤ 90, |longitude| ≤ 180, as the Exif reader checks).
pub fn parse_gps_position(lat: &str, lon: &str) -> Option<(f64, f64)> {
    let (lat, lon) = (parse_gps_coord(lat, ['N', 'S'])?, parse_gps_coord(lon, ['E', 'W'])?);
    ((-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon)).then_some((lat, lon))
}

/// XMP GPS coordinate `DDD,MM,SSk` or `DDD,MM.mmk` → signed degrees, k being `[positive, negative]` of
/// its axis (N/S or E/W, either case). The sign comes only from the letter: a negative part
/// (`-10,30N`) is malformed.
fn parse_gps_coord(s: &str, [pos, neg]: [char; 2]) -> Option<f64> {
    let s = s.trim();
    let dir = s.chars().last()?;
    let body = &s[..s.len() - dir.len_utf8()];
    let parts: Vec<f64> = body.split(',').map(|p| p.trim().parse::<f64>().ok()).collect::<Option<_>>()?;
    if parts.iter().any(|p| p.is_sign_negative()) {
        return None;
    }
    let v = match parts.as_slice() {
        [d] => *d,
        [d, m] if (0.0..60.0).contains(m) => d + m / 60.0,
        [d, m, sec] if (0.0..60.0).contains(m) && (0.0..60.0).contains(sec) => d + m / 60.0 + sec / 3600.0,
        _ => return None,
    };
    let v = match dir.to_ascii_uppercase() {
        c if c == pos => v,
        c if c == neg => -v,
        _ => return None,
    };
    v.is_finite().then_some(v)
}

/// `mwg-rs:Regions` → every region in its `RegionList`, converted to the full-image normalized frame.
/// Malformed or out-of-range entries are skipped rather than rejecting the whole list: one bad region
/// (from Lightroom, digiKam, Picasa, or a hand-edited file) shouldn't hide the rest.
fn parse_regions(regions: &XmpValue) -> Vec<Region> {
    // `AppliedToDimensions` documents the pixel size regions were authored against. Here it's only consulted
    // for the rare region whose own Area is in pixel units instead of MWG's normalized default below.
    let px_dims = applied_to(regions);
    let Some(list) = regions.field("mwg-rs:RegionList") else { return Vec::new() };
    list.items().iter().filter_map(|item| parse_one_region(item, px_dims)).collect()
}

/// `mwg-rs:AppliedToDimensions` of a `mwg-rs:Regions` struct: (width, height) in pixels, both positive.
fn applied_to(regions: &XmpValue) -> Option<(f64, f64)> {
    regions.field("mwg-rs:AppliedToDimensions").and_then(|d| {
        let w = d.field("stDim:w").and_then(XmpValue::text).and_then(parse_number)?;
        let h = d.field("stDim:h").and_then(XmpValue::text).and_then(parse_number)?;
        (w > 0.0 && h > 0.0).then_some((w, h))
    })
}

fn parse_one_region(item: &XmpValue, px_dims: Option<(f64, f64)>) -> Option<Region> {
    let area = item.field("mwg-rs:Area")?;
    let num_attr = |k: &str| area.field(k).and_then(XmpValue::text).and_then(parse_number);
    // stArea:x/y are the region's CENTER, not its top-left corner (MWG Region Guidelines §Area).
    let (cx, cy, w, h) = (num_attr("stArea:x")?, num_attr("stArea:y")?, num_attr("stArea:w")?, num_attr("stArea:h")?);
    let finite_and_sane = [cx, cy, w, h].iter().all(|v| v.is_finite() && v.abs() <= 1.0e6);
    if !finite_and_sane || w <= 0.0 || h <= 0.0 {
        return None;
    }
    // `stArea:unit` is "normalized" (a fraction of AppliedToDimensions) in every writer we know of, and
    // that needs no cross-referencing at all. Only a region that explicitly says "pixel" gets divided
    // down, and only if AppliedToDimensions is there to divide by; otherwise it's dropped rather than
    // stored as a nonsense fraction.
    let is_pixels = area.field("stArea:unit").and_then(XmpValue::text).is_some_and(|u| u.eq_ignore_ascii_case("pixel"));
    let (cx, cy, w, h) = match (is_pixels, px_dims) {
        (false, _) => (cx, cy, w, h),
        (true, Some((dw, dh))) => (cx / dw, cy / dh, w / dw, h / dh),
        (true, None) => return None,
    };
    // Lightroom Classic stores the box of a photo with Exif orientation 3 / 6 / 8 in the sensor's own
    // frame and writes the orientation as the region's `mwg-rs:Rotation` (π for 3, −π/2 for 6, +π/2 for
    // 8; 0 for 1) while `AppliedToDimensions` names the upright size. Turn it into the upright frame.
    // MWG has no way to say "mirrored", so orientations 2/4/5/7 can't be told apart from their unmirrored
    // twins here; boxes on mirrored photos are taken as written .
    let rotation = item.field("mwg-rs:Rotation").and_then(XmpValue::text).and_then(parse_number).filter(|r| r.is_finite());
    let (quarter, half) = (std::f64::consts::FRAC_PI_2, std::f64::consts::PI);
    let (cx, cy, w, h) = match rotation {
        Some(r) if (r + quarter).abs() < 0.05 => (1.0 - cy, cx, h, w),
        Some(r) if (r - quarter).abs() < 0.05 => (cy, 1.0 - cx, h, w),
        Some(r) if (r.abs() - half).abs() < 0.05 => (1.0 - cx, 1.0 - cy, w, h),
        _ => (cx, cy, w, h),
    };
    // Keep only the part of the box inside the photo: a region hanging off an edge (common for faces
    // cut by the frame) is clipped, and one entirely outside it (hostile or stale) is dropped, so every
    // stored rect is within 0..1 and non-empty.
    let rect = Rect::from_center(Point::new(cx, cy), w, h).intersect(&Rect::UNIT);
    if rect.is_empty() {
        return None;
    }
    let name = item.field("mwg-rs:Name").and_then(XmpValue::text).map(str::to_string).filter(|s| !s.is_empty());
    let description = item.field("mwg-rs:Description").and_then(XmpValue::text).map(str::to_string).filter(|s| !s.is_empty());
    let kind = match item.field("mwg-rs:Type").and_then(XmpValue::text).unwrap_or("") {
        t if t.eq_ignore_ascii_case("face") => RegionKind::Face,
        t if t.eq_ignore_ascii_case("pet") => RegionKind::Pet,
        t if t.eq_ignore_ascii_case("focus") => RegionKind::Focus,
        t if t.eq_ignore_ascii_case("barcode") => RegionKind::BarCode,
        t => RegionKind::Other(t.to_string()),
    };
    Some(Region { rect, kind, name, description })
}

fn fmt_gps_coord(v: f64, pos: char, neg: char) -> String {
    let a = v.abs();
    let mut d = a.trunc();
    let mut m = ((a - d) * 60.0 * 1e6).round() / 1e6;
    if m >= 60.0 {
        d += 1.0;
        m = 0.0;
    }
    format!("{},{:.6}{}", d as i64, m, if v < 0.0 { neg } else { pos })
}

// Pixel dimensions are integers. Keep decimal coefficients as digits so wide rational components
// remain exact and fractions that `f64` rounds to an integer can still be rejected.
struct ExactDecimal {
    negative: bool,
    coefficient: String,
    exponent: i64,
}

struct ExactRatio {
    negative: bool,
    numerator: String,
    denominator: String,
    exponent: i64,
}

fn exact_decimal(value: &str) -> Option<ExactDecimal> {
    let value = value.trim();
    let (negative, value) = if let Some(value) = value.strip_prefix('-') {
        (true, value)
    } else if let Some(value) = value.strip_prefix('+') {
        (false, value)
    } else {
        (false, value)
    };
    let (mantissa, exponent) = if let Some((mantissa, exponent)) = value.split_once('e').or_else(|| value.split_once('E')) {
        if exponent.contains('e') || exponent.contains('E') {
            return None;
        }
        (mantissa, exponent.parse::<i64>().ok()?)
    } else {
        (value, 0)
    };
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    if fraction.contains('.') {
        return None;
    }
    let digits = whole.bytes().chain(fraction.bytes());
    if !digits.clone().all(|digit| digit.is_ascii_digit()) {
        return None;
    }
    let count = whole.len().checked_add(fraction.len())?;
    if count == 0 {
        return None;
    }
    let leading = digits.clone().take_while(|digit| *digit == b'0').count();
    if leading == count {
        return Some(ExactDecimal { negative, coefficient: "0".to_owned(), exponent: 0 });
    }
    let trailing = digits.clone().rev().take_while(|digit| *digit == b'0').count();
    let significant = count.checked_sub(leading)?.checked_sub(trailing)?;
    let mut coefficient = String::with_capacity(significant);
    for digit in digits.skip(leading).take(significant) {
        coefficient.push(char::from(digit));
    }
    let exponent = exponent.checked_sub(i64::try_from(fraction.len()).ok()?)?.checked_add(i64::try_from(trailing).ok()?)?;
    Some(ExactDecimal { negative, coefficient, exponent })
}

fn exact_ratio(numerator: ExactDecimal, denominator: ExactDecimal) -> Option<ExactRatio> {
    if denominator.coefficient == "0" {
        return None;
    }
    Some(ExactRatio {
        negative: numerator.negative ^ denominator.negative,
        numerator: numerator.coefficient,
        denominator: denominator.coefficient,
        exponent: numerator.exponent.checked_sub(denominator.exponent)?,
    })
}

fn multiply_decimal_by_u32(value: &str, multiplier: u32) -> String {
    if multiplier == 0 {
        return "0".to_owned();
    }
    let mut digits = Vec::with_capacity(value.len().saturating_add(10));
    let mut carry = 0u64;
    for digit in value.bytes().rev() {
        let product = u64::from(digit - b'0') * u64::from(multiplier) + carry;
        digits.push(b'0' + (product % 10) as u8);
        carry = product / 10;
    }
    while carry != 0 {
        digits.push(b'0' + (carry % 10) as u8);
        carry /= 10;
    }
    digits.reverse();
    digits.into_iter().map(char::from).collect()
}

fn decimal_significand(value: &str) -> Option<f64> {
    let mut prefix = 0u64;
    let mut digits = 0u32;
    for digit in value.bytes().take(17) {
        prefix = prefix.checked_mul(10)?.checked_add(u64::from(digit.checked_sub(b'0')?))?;
        digits = digits.checked_add(1)?;
    }
    let exponent = i32::try_from(digits.checked_sub(1)?).ok()?;
    Some(prefix as f64 / 10.0_f64.powi(exponent))
}

fn compare_scaled_decimal(left: &str, left_exponent: i64, right: &str, right_exponent: i64) -> Option<std::cmp::Ordering> {
    if left == "0" {
        return Some(if right == "0" { std::cmp::Ordering::Equal } else { std::cmp::Ordering::Less });
    }
    if right == "0" {
        return Some(std::cmp::Ordering::Greater);
    }
    let left_magnitude = i128::try_from(left.len()).ok()?.checked_add(i128::from(left_exponent))?;
    let right_magnitude = i128::try_from(right.len()).ok()?.checked_add(i128::from(right_exponent))?;
    match left_magnitude.cmp(&right_magnitude) {
        std::cmp::Ordering::Equal => {}
        ordering => return Some(ordering),
    }
    let length = left.len().max(right.len());
    for (left_digit, right_digit) in left.bytes().chain(std::iter::repeat(b'0')).zip(right.bytes().chain(std::iter::repeat(b'0'))).take(length) {
        match left_digit.cmp(&right_digit) {
            std::cmp::Ordering::Equal => {}
            ordering => return Some(ordering),
        }
    }
    Some(std::cmp::Ordering::Equal)
}

fn exact_ratio_to_u32(value: ExactRatio) -> Option<u32> {
    if value.negative || value.denominator == "0" {
        return None;
    }
    if value.numerator == "0" {
        return Some(0);
    }
    if value.exponent < -128 || value.exponent > 128 {
        return None;
    }
    let numerator_length = i128::try_from(value.numerator.len()).ok()?;
    let denominator_length = i128::try_from(value.denominator.len()).ok()?;
    let decimal_scale = numerator_length.checked_add(i128::from(value.exponent))?.checked_sub(denominator_length)?;
    if !(0..=10).contains(&decimal_scale) {
        return None;
    }
    // The prefix only estimates a candidate. Acceptance compares the full exact coefficients below.
    let approximate =
        decimal_significand(&value.numerator)? / decimal_significand(&value.denominator)? * 10.0_f64.powi(i32::try_from(decimal_scale).ok()?);
    if !approximate.is_finite() {
        return None;
    }
    let integer = approximate.round().clamp(0.0, f64::from(u32::MAX)) as u32;
    let product = multiply_decimal_by_u32(&value.denominator, integer);
    let ordering = if product == "0" {
        compare_scaled_decimal(&value.numerator, value.exponent, "0", 0)?
    } else {
        let trailing_zeroes = product.bytes().rev().take_while(|digit| *digit == b'0').count();
        let product_exponent = i64::try_from(trailing_zeroes).ok()?;
        compare_scaled_decimal(&value.numerator, value.exponent, product.trim_end_matches('0'), product_exponent)?
    };
    (ordering == std::cmp::Ordering::Equal).then_some(integer)
}

fn exact_u32_from_text(value: &str) -> Option<u32> {
    let value = value.trim();
    let ratio = if let Some((numerator, denominator)) = value.split_once('/') {
        exact_ratio(exact_decimal(numerator)?, exact_decimal(denominator)?)?
    } else {
        let value = exact_decimal(value)?;
        ExactRatio { negative: value.negative, numerator: value.coefficient, denominator: "1".to_owned(), exponent: value.exponent }
    };
    exact_ratio_to_u32(ratio)
}

/// A positive whole-number ISO, read exactly (a fraction or an out-of-range value is not one).
fn iso_from_text(value: &str) -> Option<u32> {
    exact_u32_from_text(value).filter(|&iso| iso > 0)
}

fn dimension_candidate(first: impl Fn(&str) -> Option<String>, key: &str) -> Option<u32> {
    first(key).and_then(|value| exact_u32_from_text(&value))
}

/// Parse an XMP packet (with or without the `<?xpacket?>` wrapper / `x:xmpmeta` element).
pub fn parse_xmp(s: &str) -> Result<XmpData, XmpError> {
    parse_xmp_with(s, &[])
}

/// [`parse_xmp`] that also knows the namespaces in `extra` by their URI.
pub fn parse_xmp_with(s: &str, extra: &[XmpNamespace]) -> Result<XmpData, XmpError> {
    let s = s.trim_start_matches('\u{feff}');
    let tree = parse_tree(s, extra)?;
    let names_location = tree_names_location(&tree);
    let mut descs = Vec::new();
    find_descriptions(&tree, &mut descs);
    let mut props = BTreeMap::new();
    let mut values = BTreeMap::new();
    let mut properties_truncated = false;
    let mut value_has_text_by_key = BTreeMap::new();
    // One budget for the whole document: several descriptions cannot each spend a fresh one.
    let mut key_budget = MAX_KEY_BYTES;
    for d in descs {
        collect_description(d, "", &mut props, &mut key_budget, &mut properties_truncated);
        if let XmpValue::Struct(m) = node_struct(d) {
            // Later descriptions can repeat a property too; the same rule keeps an earlier place.
            for (key, value) in m {
                insert_keeping_text(&mut values, &mut value_has_text_by_key, key, value);
            }
        }
    }
    let m = project(&props, &values);
    let toolkit = tree
        .children
        .iter()
        .find(|n| n.name == "x:xmpmeta" || n.name == "x:xapmeta")
        .and_then(|n| n.attr("x:xmptk").or_else(|| n.attr("x:xaptk")))
        .map(str::to_string);
    Ok(XmpData { metadata: m, properties: props, properties_truncated, values, toolkit, names_location })
}

/// The typed [`Metadata`] the flattened `props` and structured `values` of a packet state.
fn project(props: &BTreeMap<String, Vec<String>>, values: &BTreeMap<String, XmpValue>) -> Metadata {
    let first = |k: &str| props.get(k).and_then(|v| v.first()).map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let num = |k: &str| first(k).and_then(|s| parse_number(&s));
    let mut m = Metadata {
        make: first("tiff:Make"),
        model: first("tiff:Model"),
        serial_number: first("exifEX:BodySerialNumber").or_else(|| first("aux:SerialNumber")),
        software: first("xmp:CreatorTool"),
        lens_make: first("exifEX:LensMake"),
        lens_model: first("exifEX:LensModel").or_else(|| first("aux:Lens")),
        lens_serial_number: first("exifEX:LensSerialNumber").or_else(|| first("aux:LensSerialNumber")),
        exposure_time: num("exif:ExposureTime").filter(|v| *v > 0.0),
        f_number: num("exif:FNumber").filter(|v| *v > 0.0),
        focal_length: num("exif:FocalLength").filter(|v| *v > 0.0),
        focal_length_35mm: num("exif:FocalLengthIn35mmFilm").filter(|v| *v > 0.0),
        exposure_bias: num("exif:ExposureBiasValue"),
        exposure_program: num("exif:ExposureProgram").map(|v| v as u16),
        metering_mode: num("exif:MeteringMode").map(|v| v as u16),
        white_balance: num("exif:WhiteBalance").map(|v| v as u16),
        orientation: num("tiff:Orientation").filter(|v| (1.0..=8.0).contains(v) && v.fract() == 0.0).map(|v| Orientation::from_exif(v as u16)),
        width: dimension_candidate(first, "tiff:ImageWidth").or_else(|| dimension_candidate(first, "exif:PixelXDimension")),
        height: dimension_candidate(first, "tiff:ImageLength").or_else(|| dimension_candidate(first, "exif:PixelYDimension")),
        title: first("dc:title"),
        caption: first("dc:description"),
        alt_text: first("Iptc4xmpCore:AltTextAccessibility"),
        extended_description: first("Iptc4xmpCore:ExtDescrAccessibility"),
        sublocation: first("Iptc4xmpCore:Location"),
        city: first("photoshop:City"),
        state: first("photoshop:State"),
        country: first("photoshop:Country"),
        copyright: first("dc:rights"),
        copyright_marked: first("xmpRights:Marked").and_then(|s| match s.to_ascii_lowercase().as_str() {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        }),
        usage_terms: first("xmpRights:UsageTerms"),
        copyright_url: first("xmpRights:WebStatement"),
        rating: num("xmp:Rating").map(|v| v.round().clamp(-1.0, 5.0) as i8),
        label: first("xmp:Label"),
        ..Default::default()
    };
    let iso = |key: &str| first(key).and_then(|value| iso_from_text(&value));
    m.iso = [iso("exif:ISOSpeedRatings"), iso("exifEX:PhotographicSensitivity")].into_iter().flatten().next();
    if let Some(spec) = props.get("exifEX:LensSpecification").filter(|v| v.len() == 4) {
        let v: Vec<f64> = spec.iter().filter_map(|s| parse_number(s)).collect();
        if v.len() == 4 {
            m.lens_spec = Some([v[0], v[1], v[2], v[3]]);
        }
    }
    if let Some(creators) = props.get("dc:creator").filter(|c| !c.is_empty()) {
        m.artist = Some(creators.join("; "));
    }
    m.keywords = props.get("dc:subject").cloned().unwrap_or_default().into_iter().filter(|k| !k.is_empty()).collect();
    m.hierarchical_keywords = props.get("lr:hierarchicalSubject").cloned().unwrap_or_default().into_iter().filter(|k| !k.is_empty()).collect();
    m.capture_time =
        ["exif:DateTimeOriginal", "photoshop:DateCreated", "xmp:CreateDate"].iter().find_map(|k| first(k).and_then(|s| DateTime::parse_iso(&s)));
    if let Some(fired) = first("exif:Flash/exif:Fired") {
        let b = |k: &str| first(k).is_some_and(|s| s.eq_ignore_ascii_case("true"));
        let n = |k: &str| num(k).unwrap_or(0.0) as u16;
        let fired = fired.eq_ignore_ascii_case("true");
        let raw = fired as u16
            | (n("exif:Flash/exif:Return") & 3) << 1
            | (n("exif:Flash/exif:Mode") & 3) << 3
            | (b("exif:Flash/exif:Function") as u16) << 5
            | (b("exif:Flash/exif:RedEyeMode") as u16) << 6;
        m.flash = Some(Flash { fired, raw });
    }
    if let Some((lat, lon)) = first("exif:GPSLatitude").zip(first("exif:GPSLongitude")).and_then(|(la, lo)| parse_gps_position(&la, &lo)) {
        let altitude = num("exif:GPSAltitude").map(|a| if first("exif:GPSAltitudeRef").as_deref() == Some("1") { -a } else { a });
        m.gps = Some(Gps { latitude: lat, longitude: lon, altitude });
    }
    m.regions = values.get("mwg-rs:Regions").map(parse_regions).unwrap_or_default();
    m
}

/// A standard JPEG XMP packet and its Extended XMP continuation, read as one (Adobe XMP
/// Specification Part 3, 1.1.3.1): properties of the standard packet win, arrays and structs
/// stated in both are combined, and the `xmpNote:HasExtendedXMP` link itself is left out.
pub fn parse_xmp_pair(standard: &str, extended: &str, extra: &[XmpNamespace]) -> Result<XmpData, XmpError> {
    let ext = parse_xmp_with(extended, extra)?;
    let std = parse_xmp_with(standard, extra)?;
    let mut properties = ext.properties;
    properties.extend(std.properties);
    let mut values = ext.values;
    for (key, preferred) in std.values {
        match values.get_mut(&key) {
            Some(fallback) => *fallback = merge_xmp_value(&preferred, fallback),
            None => {
                values.insert(key, preferred);
            }
        }
    }
    properties.remove("xmpNote:HasExtendedXMP");
    values.remove("xmpNote:HasExtendedXMP");
    let metadata = project(&properties, &values);
    Ok(XmpData {
        metadata,
        properties,
        properties_truncated: std.properties_truncated || ext.properties_truncated,
        values,
        toolkit: std.toolkit,
        names_location: std.names_location || ext.names_location,
    })
}

/// `preferred` with what `fallback` adds: array items appended, struct fields merged.
fn merge_xmp_value(preferred: &XmpValue, fallback: &XmpValue) -> XmpValue {
    match (preferred, fallback) {
        (XmpValue::Array(preferred), XmpValue::Array(fallback)) => XmpValue::Array(preferred.iter().chain(fallback).cloned().collect()),
        (XmpValue::Struct(preferred), XmpValue::Struct(fallback)) => {
            let mut merged = fallback.clone();
            for (key, value) in preferred {
                let combined = merged.get(key).map_or_else(|| value.clone(), |fallback| merge_xmp_value(value, fallback));
                merged.insert(key.clone(), combined);
            }
            XmpValue::Struct(merged)
        }
        _ => preferred.clone(),
    }
}

/// The GUID a standard packet's `xmpNote:HasExtendedXMP` names, upper-cased; `Ok(None)` without
/// one. Only the XMP Note namespace counts (a foreign `HasExtendedXMP` is not a link), and the
/// property must occur once, as 32 hexadecimal digits.
pub fn extended_xmp_reference(data: &XmpData) -> Result<Option<String>, String> {
    let Some(values) = data.properties.get("xmpNote:HasExtendedXMP") else { return Ok(None) };
    let [value] = values.as_slice() else {
        return Err("XMP HasExtendedXMP reference must occur exactly once".into());
    };
    if value.len() != 32 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("XMP HasExtendedXMP reference must be 32 hexadecimal characters".into());
    }
    Ok(Some(value.to_ascii_uppercase()))
}

fn frac(v: f64) -> String {
    if v > 0.0 && v < 1.0 {
        let inv = 1.0 / v;
        if (inv - inv.round()).abs() < 1e-6 {
            return format!("1/{}", inv.round() as u64);
        }
    }
    let (n, d) = craft_tiff::writer::rational(v);
    format!("{n}/{d}")
}

fn sfrac(v: f64) -> String {
    let (n, d) = craft_tiff::writer::srational(v);
    format!("{n}/{d}")
}

/// Serialise the interchange subset of `meta` as a complete XMP packet.
pub fn write_xmp(meta: &Metadata) -> String {
    write_xmp_with(meta, &XmpWrite::default())
}

/// Like [`write_xmp`], with the writing program's name and its own simple properties.
pub fn write_xmp_with(meta: &Metadata, with: &XmpWrite) -> String {
    let mut simple: Vec<(&str, String)> = Vec::new();
    let mut push = |k: &'static str, v: Option<String>| {
        if let Some(v) = v {
            simple.push((k, v));
        }
    };
    push("tiff:Make", meta.make.clone());
    push("tiff:Model", meta.model.clone());
    push("tiff:Orientation", meta.orientation.map(|o| o.to_exif().to_string()));
    push("tiff:ImageWidth", meta.width.map(|v| v.to_string()));
    push("tiff:ImageLength", meta.height.map(|v| v.to_string()));
    push("xmp:CreatorTool", meta.software.clone());
    push("xmp:Rating", meta.rating.map(|v| v.to_string()));
    push("xmp:Label", meta.label.clone());
    // a year or a month alone goes in `photoshop:DateCreated` only, as Lightroom writes it: Exif's
    // date can't say it
    push("exif:DateTimeOriginal", meta.capture_time.filter(DateTime::is_full).map(|d| d.to_iso()));
    push("photoshop:DateCreated", meta.capture_time.map(|d| d.to_iso()));
    push("exif:ExposureTime", meta.exposure_time.map(frac));
    push("exif:FNumber", meta.f_number.map(frac));
    push("exif:FocalLength", meta.focal_length.map(frac));
    push("exif:FocalLengthIn35mmFilm", meta.focal_length_35mm.map(|v| format!("{}", v.round() as i64)));
    push("exif:ExposureBiasValue", meta.exposure_bias.map(sfrac));
    push("exif:ExposureProgram", meta.exposure_program.map(|v| v.to_string()));
    push("exif:MeteringMode", meta.metering_mode.map(|v| v.to_string()));
    push("exif:WhiteBalance", meta.white_balance.map(|v| v.to_string()));
    push("exifEX:BodySerialNumber", meta.serial_number.clone());
    push("exifEX:LensMake", meta.lens_make.clone());
    push("exifEX:LensModel", meta.lens_model.clone());
    push("exifEX:LensSerialNumber", meta.lens_serial_number.clone());
    push("Iptc4xmpCore:Location", meta.sublocation.clone());
    push("photoshop:City", meta.city.clone());
    push("photoshop:State", meta.state.clone());
    push("photoshop:Country", meta.country.clone());
    push("xmpRights:Marked", meta.copyright_marked.map(|m| if m { "True" } else { "False" }.to_string()));
    push("xmpRights:WebStatement", meta.copyright_url.clone());
    if let Some(g) = meta.gps {
        push("exif:GPSLatitude", Some(fmt_gps_coord(g.latitude, 'N', 'S')));
        push("exif:GPSLongitude", Some(fmt_gps_coord(g.longitude, 'E', 'W')));
        if let Some(a) = g.altitude {
            push("exif:GPSAltitude", Some(frac(a.abs())));
            push("exif:GPSAltitudeRef", Some(if a < 0.0 { "1" } else { "0" }.into()));
        }
    }

    let mut x = String::new();
    x.push_str("<?xpacket begin=\"\u{feff}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n");
    x.push_str(&format!("<x:xmpmeta xmlns:x=\"adobe:ns:meta/\" x:xmptk=\"{}\">\n", escape(xml_safe(with.toolkit))));
    x.push_str(" <rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\n");
    x.push_str("  <rdf:Description rdf:about=\"\"");
    // the caller's namespace takes the place of `crs`, which is read only, as is `xmpNote`
    let own = with.namespace.map(|n| (n.prefix, n.uri));
    for (p, u) in NAMESPACES.iter().skip(2).filter_map(|&ns| match ns.0 {
        "crs" => own,
        "xmpNote" => None,
        _ => Some(ns),
    }) {
        x.push_str(&format!("\n    xmlns:{p}=\"{}\"", escape(xml_safe(u))));
    }
    x.push_str(">\n");
    for (k, v) in &simple {
        x.push_str(&format!("   <{k}>{}</{k}>\n", keep_cr(escape(xml_safe(v)))));
    }
    for (k, v) in with.properties.iter().filter(|_| own.is_some()) {
        let p = own.map_or("", |n| n.0);
        if !k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            // element content: quotes needn't be escaped, which keeps JSON readable
            x.push_str(&format!("   <{p}:{k}>{}</{p}:{k}>\n", keep_cr(partial_escape(xml_safe(v)))));
        }
    }
    let array = |x: &mut String, k: &str, kind: &str, items: &[String], lang: bool| {
        if items.is_empty() {
            return;
        }
        x.push_str(&format!("   <{k}>\n    <rdf:{kind}>\n"));
        for it in items {
            if lang {
                x.push_str(&format!("     <rdf:li xml:lang=\"x-default\">{}</rdf:li>\n", keep_cr(escape(xml_safe(it)))));
            } else {
                x.push_str(&format!("     <rdf:li>{}</rdf:li>\n", keep_cr(escape(xml_safe(it)))));
            }
        }
        x.push_str(&format!("    </rdf:{kind}>\n   </{k}>\n"));
    };
    let one = |v: &Option<String>| v.iter().cloned().collect::<Vec<_>>();
    array(&mut x, "dc:title", "Alt", &one(&meta.title), true);
    array(&mut x, "dc:description", "Alt", &one(&meta.caption), true);
    array(&mut x, "Iptc4xmpCore:AltTextAccessibility", "Alt", &one(&meta.alt_text), true);
    array(&mut x, "Iptc4xmpCore:ExtDescrAccessibility", "Alt", &one(&meta.extended_description), true);
    array(&mut x, "dc:rights", "Alt", &one(&meta.copyright), true);
    array(&mut x, "xmpRights:UsageTerms", "Alt", &one(&meta.usage_terms), true);
    array(&mut x, "dc:creator", "Seq", &one(&meta.artist), false);
    array(&mut x, "dc:subject", "Bag", &meta.keywords, false);
    array(&mut x, "lr:hierarchicalSubject", "Bag", &meta.hierarchical_keywords, false);
    array(&mut x, "exif:ISOSpeedRatings", "Seq", &meta.iso.filter(|value| *value > 0).map(|v| vec![v.to_string()]).unwrap_or_default(), false);
    if let Some(s) = meta.lens_spec {
        array(&mut x, "exifEX:LensSpecification", "Seq", &s.iter().map(|&v| frac(v)).collect::<Vec<_>>(), false);
    }
    if let Some(f) = meta.flash {
        let r = f.raw;
        let tf = |b: bool| if b { "True" } else { "False" };
        x.push_str(&format!(
            "   <exif:Flash rdf:parseType=\"Resource\">\n    <exif:Fired>{}</exif:Fired>\n    <exif:Return>{}</exif:Return>\n    <exif:Mode>{}</exif:Mode>\n    <exif:Function>{}</exif:Function>\n    <exif:RedEyeMode>{}</exif:RedEyeMode>\n   </exif:Flash>\n",
            tf(f.fired),
            (r >> 1) & 3,
            (r >> 3) & 3,
            tf(r & 0x20 != 0),
            tf(r & 0x40 != 0)
        ));
    }
    x.push_str("  </rdf:Description>\n </rdf:RDF>\n</x:xmpmeta>\n<?xpacket end=\"w\"?>");
    x
}

#[cfg(test)]
mod tests;
