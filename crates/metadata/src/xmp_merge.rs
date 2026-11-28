//! Merging a freshly written XMP packet into an existing one (a sidecar another application
//! wrote, e.g. with its develop settings and edit history), keeping everything the writer does not
//! own byte for byte.
//!
//! The existing packet is scanned once (namespace-resolved, with byte positions). Only top-level
//! properties of the `rdf:Description` elements directly under `rdf:RDF` are considered, in
//! attribute form (`xmp:Rating="3"`) and element form (`<dc:subject>…</dc:subject>`):
//! - properties the writer owns ([`MergeRules::owned`]) are removed, and written from the fresh
//!   packet when it has them (so a cleared field is cleared in the file too);
//! - properties owned only when stated ([`MergeRules::owned_if_present`]) are replaced when the
//!   fresh packet has them and kept otherwise;
//! - any other property of the fresh packet (e.g. `xmp:CreatorTool`) is not written: the file
//!   keeps describing itself as its creator left it;
//! - everything else — other namespaces (e.g. `crs:`), `xmpMM:History`, unknown properties,
//!   comments, the packet wrapper and padding — is copied unchanged.
//!
//! The written properties go into one new `rdf:Description` (declaring its own namespaces, so
//! the existing prefix bindings never matter) just before `</rdf:RDF>`. Names listed in
//! [`MergeRules::onto_first`] are the exception: they are written onto the first description,
//! because some readers treat that description as the whole packet and ignore a later one.
//! Attribute-form properties are written only through that list. A description left empty by the
//! removals is dropped, so repeated saves don't accumulate. The result is checked by parsing it
//! again: every written property must read back as in the fresh packet.

use std::collections::{BTreeSet, HashSet};

use quick_xml::events::Event;

use crate::xmp::{MAX_OPAQUE_NAME_BYTES, XmpError, XmpNamespace, canonical_name, check_scopes, parse_xmp_with, unescape_attr};

/// Which properties [`merge_xmp`] takes from the fresh packet. Names are canonical
/// `prefix:name` (prefixes as [`crate::parse_xmp`] reports them); `prefix:*` is a whole namespace.
#[derive(Clone, Copy, Debug, Default)]
pub struct MergeRules<'a> {
    /// Always replaced: removed from the existing packet, written from the fresh one if stated.
    pub owned: &'a [&'a str],
    /// Replaced only when the fresh packet states them; otherwise the existing value stays.
    pub owned_if_present: &'a [&'a str],
    /// Owned names written onto the first `rdf:Description` instead of a new one. Empty keeps
    /// every written property in that new description. Attribute-form properties are written
    /// only when listed here.
    pub onto_first: &'a [&'a str],
    /// Namespaces beyond the standard ones that the names above use, as for [`crate::parse_xmp_with`].
    pub namespaces: &'a [XmpNamespace<'a>],
}

fn listed(list: &[&str], name: &str) -> bool {
    list.iter().any(|p| match p.strip_suffix('*') {
        Some(ns) => name.starts_with(ns),
        None => *p == name,
    })
}

#[derive(Debug)]
struct Attr {
    raw: String,
    name: String,
    /// The value as written (still escaped).
    value: String,
}

#[derive(Debug)]
struct Prop {
    name: String,
    start: usize,
    end: usize,
}

#[derive(Debug, Default)]
struct Desc {
    tag_start: usize,
    tag_end: usize,
    /// The element name as written (`rdf:Description`, or another prefix for the RDF namespace).
    raw_name: String,
    empty: bool,
    attrs: Vec<Attr>,
    props: Vec<Prop>,
    /// The end of `</rdf:Description>` (`tag_end` when self-closing).
    end: usize,
}

#[derive(Debug, Default)]
struct Scan {
    descs: Vec<Desc>,
    /// Where the first `</rdf:RDF>` starts.
    rdf_close: Option<usize>,
    /// The span of a self-closing `<rdf:RDF/>` (used only when there is no `</rdf:RDF>`).
    rdf_empty: Option<(usize, usize)>,
}

enum Kind {
    Other,
    Rdf,
    Desc(usize),
    Prop { desc: usize, start: usize, name: String },
}

fn is_meta_attr(name: &str) -> bool {
    name.starts_with("xmlns")
        || matches!(name, "rdf:about" | "rdf:ID" | "rdf:nodeID" | "xml:lang" | "rdf:parseType" | "rdf:resource" | "rdf:datatype")
}

fn pos(r: &quick_xml::Reader<&[u8]>) -> usize {
    usize::try_from(r.buffer_position()).unwrap_or(usize::MAX)
}

/// Locate the descriptions, their top-level properties and `</rdf:RDF>` in a packet.
fn scan(s: &str, extra: &[XmpNamespace]) -> Result<Scan, XmpError> {
    let mut reader = quick_xml::Reader::from_reader(s.as_bytes());
    reader.config_mut().check_end_names = true;
    let mut out = Scan::default();
    let mut stack: Vec<Kind> = Vec::new();
    let mut scopes: Vec<Vec<(String, String)>> = vec![vec![("xml".into(), "http://www.w3.org/XML/1998/namespace".into())]];
    let mut count = 0usize;
    let mut opaque_budget = MAX_OPAQUE_NAME_BYTES;
    loop {
        let before = pos(&reader);
        let ev = reader.read_event().map_err(|e| XmpError::Xml(e.to_string()))?;
        let after = pos(&reader);
        match ev {
            Event::Start(ref e) | Event::Empty(ref e) => {
                count += 1;
                if count > 200_000 || stack.len() > 64 {
                    return Err(XmpError::TooComplex);
                }
                let empty = matches!(ev, Event::Empty(_));
                let mut scope = Vec::new();
                let mut raw_attrs = Vec::new();
                // repeated names are refused with a set, in linear time (quick-xml's own check is quadratic)
                let mut seen = HashSet::new();
                for a in e.attributes().with_checks(false) {
                    let a = a.map_err(|e| XmpError::Xml(e.to_string()))?;
                    let k = String::from_utf8_lossy(a.key.as_ref()).into_owned();
                    if !seen.insert(k.clone()) {
                        return Err(XmpError::Xml(format!("repeated attribute {k}")));
                    }
                    let v = String::from_utf8_lossy(&a.value).into_owned();
                    if k == "xmlns" {
                        scope.push((String::new(), unescape_attr(&v)));
                    } else if let Some(p) = k.strip_prefix("xmlns:") {
                        scope.push((p.to_string(), unescape_attr(&v)));
                    }
                    raw_attrs.push((k, v));
                }
                scopes.push(scope);
                check_scopes(&scopes)?;
                let raw_name = String::from_utf8_lossy(e.name().as_ref()).into_owned();
                let name = canonical_name(&raw_name, &scopes, extra, &mut opaque_budget)?;
                let kind = match stack.last() {
                    Some(Kind::Rdf) if name == "rdf:Description" => {
                        let attrs = raw_attrs
                            .into_iter()
                            .map(|(raw, value)| Ok(Attr { name: canonical_name(&raw, &scopes, extra, &mut opaque_budget)?, raw, value }))
                            .collect::<Result<_, XmpError>>()?;
                        out.descs.push(Desc { tag_start: before, tag_end: after, raw_name, empty, attrs, props: Vec::new(), end: after });
                        Kind::Desc(out.descs.len() - 1)
                    }
                    Some(Kind::Desc(i)) => Kind::Prop { desc: *i, start: before, name },
                    _ if name == "rdf:RDF" && out.rdf_close.is_none() => Kind::Rdf,
                    _ => Kind::Other,
                };
                if empty {
                    if matches!(kind, Kind::Rdf) {
                        out.rdf_empty = out.rdf_empty.or(Some((before, after)));
                    }
                    scopes.pop();
                    if let Kind::Prop { desc, start, name } = kind
                        && let Some(d) = out.descs.get_mut(desc)
                    {
                        d.props.push(Prop { name, start, end: after });
                    }
                } else {
                    stack.push(kind);
                }
            }
            Event::End(_) => {
                scopes.pop();
                match stack.pop() {
                    Some(Kind::Prop { desc, start, name }) => {
                        if let Some(d) = out.descs.get_mut(desc) {
                            d.props.push(Prop { name, start, end: after });
                        }
                    }
                    Some(Kind::Desc(i)) => {
                        if let Some(d) = out.descs.get_mut(i) {
                            d.end = after;
                        }
                    }
                    Some(Kind::Rdf) => out.rdf_close = out.rdf_close.or(Some(before)),
                    Some(Kind::Other) => {}
                    None => return Err(XmpError::Xml("unbalanced end tag".into())),
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if !stack.is_empty() {
        return Err(XmpError::Xml("unclosed element".into()));
    }
    Ok(out)
}

/// `start` moved back over the indentation and line break before it (so a removed property
/// takes its line with it).
fn line_start(s: &str, start: usize) -> usize {
    let b = s.as_bytes();
    let mut i = start;
    while i > 0 && matches!(b.get(i - 1), Some(b' ' | b'\t')) {
        i -= 1;
    }
    if i > 0 && b.get(i - 1) == Some(&b'\n') {
        i -= 1;
        if i > 0 && b.get(i - 1) == Some(&b'\r') {
            i -= 1;
        }
    }
    i
}

fn push_attr(tag: &mut String, raw: &str, value: &str) {
    tag.push_str("\n    ");
    tag.push_str(raw);
    tag.push_str("=\"");
    tag.push_str(&value.replace('"', "&quot;"));
    tag.push('"');
}

/// Where the first description's end tag starts. A self-closing description has no end tag.
fn close_at(desc: &Desc) -> usize {
    if desc.empty {
        return desc.tag_end;
    }
    let close_len = desc.raw_name.len().saturating_add(3);
    desc.end.saturating_sub(close_len)
}

/// Edits that put the chosen fresh properties onto `first` (attributes on its start tag,
/// elements just before its end tag).
fn place_on_first(
    fresh: &str,
    first: &Desc,
    fresh_desc: &Desc,
    remove: impl Fn(&str) -> bool,
    on_first: impl Fn(&str) -> bool,
) -> Result<Vec<(usize, usize, String)>, XmpError> {
    let mut edits = Vec::new();
    let mut tag = format!("<{}", first.raw_name);
    let mut seen_raw = HashSet::new();
    for attr in first.attrs.iter().filter(|attr| is_meta_attr(&attr.name) || !remove(&attr.name)) {
        push_attr(&mut tag, &attr.raw, &attr.value);
        seen_raw.insert(attr.raw.clone());
    }
    let mut added_xmlns = false;
    for attr in fresh_desc.attrs.iter().filter(|attr| attr.raw.starts_with("xmlns:") && attr.raw != "xmlns:rdf") {
        if seen_raw.contains(&attr.raw) {
            continue;
        }
        push_attr(&mut tag, &attr.raw, &attr.value);
        seen_raw.insert(attr.raw.clone());
        added_xmlns = true;
    }
    let mut added_value = false;
    for attr in fresh_desc.attrs.iter().filter(|attr| on_first(&attr.name)) {
        if seen_raw.contains(&attr.raw) {
            continue;
        }
        push_attr(&mut tag, &attr.raw, &attr.value);
        seen_raw.insert(attr.raw.clone());
        added_value = true;
    }
    let mut children = String::new();
    for prop in fresh_desc.props.iter().filter(|prop| on_first(&prop.name)) {
        let text = fresh.get(prop.start..prop.end).ok_or_else(|| XmpError::Xml("bad property span".into()))?;
        children.push_str("\n   ");
        children.push_str(text);
    }
    let removed_attr = first.attrs.iter().any(|attr| !is_meta_attr(&attr.name) && remove(&attr.name));
    let expand = first.empty && !children.is_empty();
    if removed_attr || added_xmlns || added_value || expand {
        if expand {
            tag.push('>');
            tag.push_str(&children);
            tag.push_str("\n  </");
            tag.push_str(&first.raw_name);
            tag.push('>');
        } else if first.empty {
            tag.push_str("/>");
        } else {
            tag.push('>');
        }
        edits.push((first.tag_start, first.tag_end, tag));
    }
    if !first.empty && !children.is_empty() {
        edits.push((close_at(first), close_at(first), children));
    }
    Ok(edits)
}

/// Merge `fresh` (a packet as [`crate::write_xmp_with`] writes it) into `existing` following
/// `rules` (see the module docs). Fails — and the caller must not overwrite `existing` blindly —
/// when `existing` isn't a well-formed XMP packet.
pub fn merge_xmp(existing: &str, fresh: &str, rules: MergeRules) -> Result<String, XmpError> {
    let (bom, old) = match existing.strip_prefix('\u{feff}') {
        Some(rest) => ("\u{feff}", rest),
        None => ("", existing),
    };
    parse_xmp_with(old, rules.namespaces)?;
    let ex = scan(old, rules.namespaces)?;
    if ex.rdf_close.is_none() && ex.rdf_empty.is_none() {
        return Err(XmpError::Xml("no rdf:RDF element".into()));
    }
    let new = scan(fresh, rules.namespaces)?;
    let fresh_desc = new.descs.first().ok_or_else(|| XmpError::Xml("the new packet has no rdf:Description".into()))?;
    let write = |name: &str| listed(rules.owned, name) || listed(rules.owned_if_present, name);
    let on_first = |name: &str| write(name) && listed(rules.onto_first, name);
    let mut fresh_names: BTreeSet<&str> = fresh_desc.props.iter().map(|prop| prop.name.as_str()).collect();
    for attr in &fresh_desc.attrs {
        if is_meta_attr(&attr.name) {
            continue;
        }
        if on_first(&attr.name) {
            fresh_names.insert(attr.name.as_str());
        }
    }
    let remove = |name: &str| listed(rules.owned, name) || (listed(rules.owned_if_present, name) && fresh_names.contains(name));
    let writing_first = !ex.descs.is_empty()
        && (fresh_desc.attrs.iter().any(|attr| on_first(&attr.name)) || fresh_desc.props.iter().any(|prop| on_first(&prop.name)));

    // edits on `old`: (start, end, replacement), applied back to front
    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    for (index, desc) in ex.descs.iter().enumerate() {
        let gone_attrs: Vec<&Attr> = desc.attrs.iter().filter(|attr| !is_meta_attr(&attr.name) && remove(&attr.name)).collect();
        let gone_props: Vec<&Prop> = desc.props.iter().filter(|prop| remove(&prop.name)).collect();
        if gone_attrs.is_empty() && gone_props.is_empty() {
            continue;
        }
        let attrs_left = desc.attrs.iter().any(|attr| !is_meta_attr(&attr.name) && !remove(&attr.name));
        let keep_for_stamp = writing_first && index == 0;
        if !attrs_left && gone_props.len() == desc.props.len() && !keep_for_stamp {
            // nothing of its own left: drop the whole description
            edits.push((line_start(old, desc.tag_start), desc.end, String::new()));
            continue;
        }
        if !gone_attrs.is_empty() && !keep_for_stamp {
            let mut tag = format!("<{}", desc.raw_name);
            for attr in desc.attrs.iter().filter(|attr| is_meta_attr(&attr.name) || !remove(&attr.name)) {
                push_attr(&mut tag, &attr.raw, &attr.value);
            }
            tag.push_str(if desc.empty { "/>" } else { ">" });
            edits.push((desc.tag_start, desc.tag_end, tag));
        }
        for prop in gone_props {
            edits.push((line_start(old, prop.start), prop.end, String::new()));
        }
    }
    if writing_first {
        let first = ex.descs.first().ok_or_else(|| XmpError::Xml("no description".into()))?;
        edits.extend(place_on_first(fresh, first, fresh_desc, remove, on_first)?);
    }
    // the new description: its own namespace declarations, the properties that were not placed above
    let mut block = String::from("\n  <rdf:Description rdf:about=\"\"\n    xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\"");
    for attr in fresh_desc.attrs.iter().filter(|attr| attr.raw.starts_with("xmlns:") && attr.raw != "xmlns:rdf") {
        block.push_str(&format!("\n    {}=\"{}\"", attr.raw, attr.value));
    }
    let nowhere_else = ex.descs.is_empty();
    let onto_first_attr = |name: &str| write(name) && listed(rules.onto_first, name);
    if nowhere_else {
        for attr in fresh_desc.attrs.iter().filter(|attr| onto_first_attr(&attr.name)) {
            push_attr(&mut block, &attr.raw, &attr.value);
        }
    }
    block.push('>');
    let mut written = 0;
    if nowhere_else {
        written += fresh_desc.attrs.iter().filter(|attr| onto_first_attr(&attr.name)).count();
    }
    let put_here = |name: &str| {
        if !write(name) {
            return false;
        }
        if nowhere_else {
            return true;
        }
        !listed(rules.onto_first, name)
    };
    for prop in fresh_desc.props.iter().filter(|prop| put_here(&prop.name)) {
        let text = fresh.get(prop.start..prop.end).ok_or_else(|| XmpError::Xml("bad property span".into()))?;
        block.push_str("\n   ");
        block.push_str(text);
        written += 1;
    }
    block.push_str("\n  </rdf:Description>");
    if written > 0 {
        if let Some(rdf_close) = ex.rdf_close {
            let at = old.get(..rdf_close).map_or(rdf_close, |h| h.trim_end().len());
            edits.push((at, at, block));
        } else if let Some((s, e)) = ex.rdf_empty {
            // a self-closing `<rdf:RDF .../>` is opened up around the new description
            let tag = old.get(s..e).ok_or_else(|| XmpError::Xml("bad span".into()))?;
            let open = tag.trim_end().strip_suffix("/>").ok_or_else(|| XmpError::Xml("bad rdf:RDF tag".into()))?;
            let name = tag.get(1..).and_then(|t| t.split(|c: char| c.is_whitespace() || c == '/' || c == '>').next()).unwrap_or("rdf:RDF");
            edits.push((s, e, format!("{open}>{block}\n</{name}>")));
        }
    }
    edits.sort_by_key(|e| (e.0, e.1));
    // overlapping edits can't happen (properties sit inside their description, which is either
    // dropped whole or edited piecewise), but never splice blindly
    let mut out = String::with_capacity(old.len() + 4096);
    out.push_str(bom);
    let mut at = 0;
    for (s, e, r) in &edits {
        if *s < at {
            return Err(XmpError::Xml("overlapping edits".into()));
        }
        out.push_str(old.get(at..*s).ok_or_else(|| XmpError::Xml("bad span".into()))?);
        out.push_str(r);
        at = *e;
    }
    out.push_str(old.get(at..).ok_or_else(|| XmpError::Xml("bad span".into()))?);

    // read back: everything written must come out as written
    let merged = parse_xmp_with(&out, rules.namespaces)?;
    let want = parse_xmp_with(fresh, rules.namespaces)?;
    for (k, v) in &want.properties {
        let top = k.split('/').next().unwrap_or(k);
        if write(top) && merged.properties.get(k) != Some(v) {
            return Err(XmpError::Xml(format!("{k} did not survive the merge")));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xmp::parse_xmp_with;
    use crate::{Metadata, XmpData, XmpWrite, write_xmp_with};

    /// A program's own namespace, as an application would pass it.
    const LC: XmpNamespace = XmpNamespace { prefix: "lc", uri: "http://ns.example.com/lc/1.0/" };

    fn write_lc(meta: &Metadata, properties: &[(&str, &str)]) -> String {
        write_xmp_with(meta, &XmpWrite { toolkit: "Editor", namespace: Some(LC), properties })
    }

    fn parse_xmp(s: &str) -> Result<XmpData, XmpError> {
        parse_xmp_with(s, &[LC])
    }

    const RULES: MergeRules = MergeRules {
        owned: &["xmp:Rating", "xmp:Label", "dc:subject", "dc:title", "lc:*"],
        owned_if_present: &["exif:DateTimeOriginal"],
        onto_first: &[],
        namespaces: &[LC],
    };

    /// A sidecar as another raw developer writes it: attribute-form simple properties, develop
    /// settings, a structured history, an unknown namespace.
    const FOREIGN: &str = "<?xpacket begin=\"\u{feff}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>
<x:xmpmeta xmlns:x=\"adobe:ns:meta/\" x:xmptk=\"Some Toolkit 1.0\">
 <rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">
  <rdf:Description rdf:about=\"\"
    xmlns:xmp=\"http://ns.adobe.com/xap/1.0/\"
    xmlns:xmpMM=\"http://ns.adobe.com/xap/1.0/mm/\"
    xmlns:stEvt=\"http://ns.adobe.com/xap/1.0/sType/ResourceEvent#\"
    xmlns:crs=\"http://ns.adobe.com/camera-raw-settings/1.0/\"
    xmlns:dc=\"http://purl.org/dc/elements/1.1/\"
    xmlns:foo=\"http://example.com/foo/\"
    xmp:Rating=\"2\"
    xmp:CreatorTool=\"Other App\"
    crs:Exposure2012=\"+0.50\"
    crs:Contrast2012=\"-12\"
    foo:Thing=\"a &amp; b\">
   <xmpMM:History>
    <rdf:Seq>
     <rdf:li stEvt:action=\"saved\" stEvt:when=\"2026-01-02T03:04:05\"/>
    </rdf:Seq>
   </xmpMM:History>
   <crs:ToneCurvePV2012>
    <rdf:Seq>
     <rdf:li>0, 0</rdf:li>
     <rdf:li>255, 255</rdf:li>
    </rdf:Seq>
   </crs:ToneCurvePV2012>
   <dc:subject>
    <rdf:Bag>
     <rdf:li>old</rdf:li>
    </rdf:Bag>
   </dc:subject>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>
";

    /// With the packet's whitespace padding and trailer.
    fn foreign() -> String {
        format!("{FOREIGN}{}\n<?xpacket end=\"w\"?>", " ".repeat(100))
    }

    fn fresh(rating: i8, keywords: &[&str]) -> String {
        let m = crate::Metadata {
            software: Some("Editor".into()),
            rating: Some(rating),
            keywords: keywords.iter().map(|k| k.to_string()).collect(),
            ..Default::default()
        };
        write_lc(&m, &[("settings", "{\"a\":1}"), ("flag", "pick")])
    }

    #[test]
    fn keeps_foreign_properties_and_replaces_owned_ones() {
        let out = merge_xmp(&foreign(), &fresh(4, &["new", "two"]), RULES).unwrap();
        let d = parse_xmp(&out).unwrap();
        let p = |k: &str| d.properties.get(k).cloned().unwrap_or_default();
        assert_eq!(p("xmp:Rating"), vec!["4"]);
        assert_eq!(p("dc:subject"), vec!["new", "two"]);
        assert_eq!(p("lc:settings"), vec!["{\"a\":1}"]);
        assert_eq!(p("lc:flag"), vec!["pick"]);
        assert_eq!(p("xmp:CreatorTool"), vec!["Other App"], "not ours: kept");
        assert_eq!(p("crs:Exposure2012"), vec!["+0.50"]);
        assert_eq!(p("foo:Thing").len(), 1);
        // untouched parts are byte for byte the same
        for part in [
            "   <xmpMM:History>\n    <rdf:Seq>\n     <rdf:li stEvt:action=\"saved\" stEvt:when=\"2026-01-02T03:04:05\"/>\n    </rdf:Seq>\n   </xmpMM:History>",
            "   <crs:ToneCurvePV2012>\n    <rdf:Seq>\n     <rdf:li>0, 0</rdf:li>",
            "crs:Contrast2012=\"-12\"",
            "foo:Thing=\"a &amp; b\"",
            "<?xpacket begin=\"\u{feff}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n<x:xmpmeta xmlns:x=\"adobe:ns:meta/\" x:xmptk=\"Some Toolkit 1.0\">",
            &format!("</x:xmpmeta>\n{}\n<?xpacket end=\"w\"?>", " ".repeat(100)),
        ] {
            assert!(out.contains(part), "lost {part:?} in\n{out}");
        }
        assert!(!out.contains("xmp:Rating=\"2\""));
        assert!(!out.contains("<rdf:li>old</rdf:li>"));
        // saving again changes only our part, and doesn't pile up descriptions
        let again = merge_xmp(&out, &fresh(1, &[]), RULES).unwrap();
        let d = parse_xmp(&again).unwrap();
        assert_eq!(d.properties.get("xmp:Rating"), Some(&vec!["1".to_string()]));
        assert!(!d.properties.contains_key("dc:subject"), "a cleared field is cleared");
        assert_eq!(again.matches("<rdf:Description").count(), 2, "{again}");
        assert_eq!(merge_xmp(&again, &fresh(1, &[]), RULES).unwrap(), again, "idempotent");
    }

    #[test]
    fn a_self_closing_rdf_element_is_opened_not_replaced() {
        let ex = "<!-- keep me --><x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><r:RDF xmlns:r=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\"/></x:xmpmeta>";
        let out = merge_xmp(ex, &fresh(3, &[]), RULES).unwrap();
        assert!(out.starts_with("<!-- keep me --><x:xmpmeta"), "{out}");
        assert!(out.contains("</r:RDF></x:xmpmeta>"), "{out}");
        assert_eq!(parse_xmp(&out).unwrap().properties.get("xmp:Rating"), Some(&vec!["3".to_string()]));
        assert!(merge_xmp("<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"/>", &fresh(3, &[]), RULES).is_err(), "no RDF element at all");
    }

    #[test]
    fn owned_if_present_and_prefix_independence() {
        // the existing packet binds the namespaces to other prefixes, and has a capture time
        let ex = "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><r:RDF xmlns:r=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\
            <r:Description r:about=\"\" xmlns:a=\"http://ns.adobe.com/xap/1.0/\" xmlns:e=\"http://ns.adobe.com/exif/1.0/\" \
            a:Rating=\"5\" e:DateTimeOriginal=\"2020-01-01T00:00:00\"/></r:RDF></x:xmpmeta>";
        let out = merge_xmp(ex, &fresh(3, &[]), RULES).unwrap();
        let d = parse_xmp(&out).unwrap();
        assert_eq!(d.properties.get("xmp:Rating"), Some(&vec!["3".to_string()]));
        assert_eq!(d.properties.get("exif:DateTimeOriginal"), Some(&vec!["2020-01-01T00:00:00".to_string()]), "fresh has none: kept");
        let mut m = crate::Metadata { rating: Some(3), ..Default::default() };
        m.capture_time = crate::DateTime::parse_iso("2024-05-06T07:08:09");
        let out = merge_xmp(ex, &write_lc(&m, &[]), RULES).unwrap();
        let d = parse_xmp(&out).unwrap();
        assert_eq!(d.properties.get("exif:DateTimeOriginal"), Some(&vec!["2024-05-06T07:08:09".to_string()]));
    }

    #[test]
    fn preserves_foreign_properties_using_known_prefixes() {
        for property in [r#"lc:Secret="keep-me"/>"#, r#"><lc:Secret>keep-me</lc:Secret></rdf:Description>"#] {
            let desc = format!(r#"<rdf:Description rdf:about="" xmlns:lc="https://example.org/vendor/" {property}"#);
            let old = format!(r#"<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">{desc}</rdf:RDF>"#);
            let out = merge_xmp(&old, &fresh(4, &[]), RULES).unwrap();
            assert!(out.contains(&desc), "lost foreign description: {out}");
            assert_eq!(parse_xmp(&out).unwrap().metadata.rating, Some(4));
            assert_eq!(merge_xmp(&out, &fresh(4, &[]), RULES).unwrap(), out);
        }
    }

    #[test]
    fn namespace_aliases_remain_distinct_during_merge() {
        // The alias URI includes a character reference: scanning and parsing must resolve it identically.
        let foreign = r#"<rdf:Description xmlns:xmp="https://example.org/vendor/" xmp:Rating="2"/>"#;
        let old = format!(
            r#"<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">{foreign}
            <rdf:Description xmlns:a="http://ns.adobe.com/xap/1.0&#47;" a:Rating="3"/>
            </rdf:RDF>"#
        );
        let out = merge_xmp(&old, &fresh(5, &[]), RULES).unwrap();
        assert!(out.contains(foreign), "lost foreign rating: {out}");
        assert!(!out.contains("a:Rating="), "genuine alias must be replaced: {out}");
        assert_eq!(parse_xmp(&out).unwrap().metadata.rating, Some(5));
        assert_eq!(merge_xmp(&out, &fresh(5, &[]), RULES).unwrap(), out);
    }

    #[test]
    fn opaque_namespace_expansion_is_bounded() {
        // A roughly 40 KiB packet must not expand the same long URI into every property name indefinitely.
        let packet = format!(
            r#"<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description
                xmlns:lc="https://example.org/{}">{}</rdf:Description></rdf:RDF>"#,
            "a".repeat(1024),
            "<lc:p/>".repeat(5000)
        );
        assert_eq!(parse_xmp(&packet).unwrap_err(), XmpError::TooComplex);
        assert!(matches!(scan(&packet, &[LC]), Err(XmpError::TooComplex)));
    }

    #[test]
    fn many_attributes_scan_in_linear_time() {
        // quick-xml's duplicate-name check compares each attribute with all earlier ones: 200 000 attributes on
        // one description took over 20 s to scan, about 30 ms with the set. Four times the attributes must take
        // about four times as long, not sixteen. A ratio of the fastest runs holds on a loaded machine, where
        // a fixed bound does not.
        let packet = |n: usize| {
            let mut s = String::from(
                "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\
                 <rdf:Description rdf:about=\"\" xmlns:foo=\"http://example.com/foo/\"",
            );
            for i in 0..n {
                s.push_str(&format!(" foo:a{i}=\"{i}\""));
            }
            s.push_str("/></rdf:RDF></x:xmpmeta>");
            s
        };
        let run = |s: &str, n: usize| {
            let t = std::time::Instant::now();
            let found = scan(s, &[]).unwrap();
            let took = t.elapsed();
            assert_eq!(found.descs[0].attrs.len(), n + 2, "every attribute is read");
            took
        };
        let (n, small_packet, large_packet) = (20_000, packet(20_000), packet(80_000));
        // interleaved, so a burst of load slows both sizes rather than one; the fastest run of each counts
        let (mut small, mut large) = (std::time::Duration::MAX, std::time::Duration::MAX);
        for _ in 0..3 {
            small = small.min(run(&small_packet, n));
            large = large.min(run(&large_packet, 4 * n));
        }
        let ratio = large.as_secs_f64() / small.as_secs_f64().max(1e-6);
        assert!(ratio < 10.0, "4x the attributes took {ratio:.1}x as long ({small:?}, then {large:?})");
    }

    #[test]
    fn chosen_properties_land_on_the_first_description() {
        let camera = r#"<?xpacket begin="&#xfeff;" id="W5M0MpCehiHzreSzNTczkc9d"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/" x:xmptk="Camera">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:xmp="http://ns.adobe.com/xap/1.0/"
    xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/"
    xmp:CreatorTool="Camera"
    crs:Contrast2012="-12">
   <xmp:Rating>0</xmp:Rating>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>
<?xpacket end="w"?>"#;
        let fresh = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:xmp="http://ns.adobe.com/xap/1.0/"
    xmlns:xmpDM="http://ns.adobe.com/xmp/1.0/DynamicMedia/"
    xmlns:photoshop="http://ns.adobe.com/photoshop/1.0/"
    xmlns:dc="http://purl.org/dc/elements/1.1/"
    xmlns:Iptc4xmpCore="http://iptc.org/std/Iptc4xmpCore/1.0/xmlns/"
    xmlns:lc="http://ns.example.com/lc/1.0/"
    xmpDM:pick="1"
    xmp:Rating="4"
    photoshop:City="Maia &amp; Co"
    Iptc4xmpCore:Location="Hotel">
   <dc:title><rdf:Alt><rdf:li xml:lang="x-default">Rita</rdf:li></rdf:Alt></dc:title>
   <dc:creator><rdf:Seq><rdf:li>Ada</rdf:li></rdf:Seq></dc:creator>
   <lc:settings>{"crop":1}</lc:settings>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;
        let rules = MergeRules {
            owned: &["xmp:Rating", "xmpDM:pick", "photoshop:City", "Iptc4xmpCore:Location", "dc:title", "dc:creator", "lc:*"],
            owned_if_present: &[],
            onto_first: &["xmp:Rating", "xmpDM:pick", "photoshop:City", "Iptc4xmpCore:Location", "dc:title", "dc:creator"],
            namespaces: &[LC],
        };
        let out = merge_xmp(camera, fresh, rules).unwrap();
        let parts: Vec<&str> = out.split("<rdf:Description").collect();
        assert!(parts.len() >= 3, "the settings stay on their own description:\n{out}");
        let first = parts[1];
        let second = parts[2];
        assert!(first.contains("xmpDM:pick=\"1\""), "{first}");
        assert!(first.contains("xmp:Rating=\"4\""), "{first}");
        assert!(first.contains("photoshop:City=\"Maia &amp; Co\""), "{first}");
        assert!(first.contains("Iptc4xmpCore:Location=\"Hotel\""), "{first}");
        assert!(first.contains("<dc:title>"), "{first}");
        assert!(first.contains("<dc:creator>"), "{first}");
        assert!(!first.contains("<xmp:Rating>0</xmp:Rating>"), "{first}");
        assert!(!first.contains("lc:settings"), "{first}");
        assert!(second.contains("lc:settings"), "{second}");
        assert!(!second.contains("dc:title"), "{second}");
        assert!(out.contains("xmp:CreatorTool=\"Camera\""), "{out}");
        assert!(out.contains("crs:Contrast2012=\"-12\""), "{out}");
        let read = parse_xmp(&out).unwrap();
        assert_eq!(read.properties.get("xmp:Rating"), Some(&vec!["4".to_string()]));
        assert_eq!(read.properties.get("photoshop:City"), Some(&vec!["Maia & Co".to_string()]));
        assert_eq!(read.properties.get("dc:title"), Some(&vec!["Rita".to_string()]));
        assert_eq!(read.properties.get("dc:creator"), Some(&vec!["Ada".to_string()]));
        assert_eq!(read.properties.get("lc:settings"), Some(&vec!["{\"crop\":1}".to_string()]));
        let again = merge_xmp(&out, fresh, rules).unwrap();
        assert_eq!(merge_xmp(&again, fresh, rules).unwrap(), again, "idempotent");

        let without_pick = fresh.replace("\n    xmpDM:pick=\"1\"", "");
        let keep_pick = MergeRules {
            owned: &["xmp:Rating", "photoshop:City", "Iptc4xmpCore:Location", "dc:title", "dc:creator", "lc:*"],
            owned_if_present: &["xmpDM:pick"],
            onto_first: rules.onto_first,
            namespaces: rules.namespaces,
        };
        let kept = merge_xmp(&out, &without_pick, keep_pick).unwrap();
        let kept_first = kept.split("<rdf:Description").nth(1).unwrap_or("");
        assert!(kept_first.contains("xmpDM:pick=\"1\""), "absent from the fresh packet: kept\n{kept_first}");
    }

    #[test]
    fn a_self_closing_description_opens_to_hold_the_chosen_properties() {
        let camera = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
            <rdf:Description rdf:about="" xmlns:xmp="http://ns.adobe.com/xap/1.0/" xmp:Rating="0"/>
            </rdf:RDF></x:xmpmeta>"#;
        let fresh = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
            <rdf:Description rdf:about="" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:xmp="http://ns.adobe.com/xap/1.0/" xmp:Rating="5">
            <dc:title><rdf:Alt><rdf:li xml:lang="x-default">Title</rdf:li></rdf:Alt></dc:title>
            </rdf:Description></rdf:RDF></x:xmpmeta>"#;
        let rules =
            MergeRules { owned: &["xmp:Rating", "dc:title"], owned_if_present: &[], onto_first: &["xmp:Rating", "dc:title"], namespaces: &[LC] };
        let out = merge_xmp(camera, fresh, rules).unwrap();
        assert_eq!(out.matches("<rdf:Description").count(), 1, "{out}");
        assert!(out.contains("xmp:Rating=\"5\""), "{out}");
        assert!(out.contains("<dc:title>"), "{out}");
        assert!(!out.contains("xmp:Rating=\"0\""), "{out}");
        assert_eq!(parse_xmp(&out).unwrap().properties.get("dc:title"), Some(&vec!["Title".to_string()]));
    }

    #[test]
    fn refuses_what_it_cannot_read() {
        for bad in ["", "not xml at all", "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF", "<a><b></a>", "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"/>"] {
            assert!(merge_xmp(bad, &fresh(1, &[]), RULES).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn refuses_a_repeated_attribute() {
        // not well-formed XML (strict readers such as expat refuse it): no merge, so the caller keeps a backup.
        // An owned name, a foreign one and one on a nested element (a merge would keep both copies of the last two).
        for (once, twice) in [
            ("xmp:Rating=\"2\"", "xmp:Rating=\"2\" xmp:Rating=\"3\""),
            ("foo:Thing=\"a &amp; b\"", "foo:Thing=\"a &amp; b\" foo:Thing=\"c\""),
            ("stEvt:action=\"saved\"", "stEvt:action=\"saved\" stEvt:action=\"saved\""),
        ] {
            let ex = foreign().replace(once, twice);
            assert!(merge_xmp(&ex, &fresh(1, &[]), RULES).is_err(), "{twice}");
        }
    }
}
