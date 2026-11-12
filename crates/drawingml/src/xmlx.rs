//! What the readers and writers of the office XML formats need on top of `craft-xml`: the part
//! parser with their limits, markup-compatibility branches, fragments kept as text with their
//! namespace declarations, and relationship ids inside a fragment.
//!
//! Readers here match elements with [`craft_xml::Local`] and attributes with
//! [`El::attr_local`], so a document may use any prefixes. Integers are read with
//! [`craft_xml::value::to_i64_lossy`] and its siblings: producers write `12700.0` and values
//! far out of range, and neither should lose the element.

use std::collections::{HashMap, HashSet};

use craft_xml::{El, Limits, Local, Node, Options, Tag};

/// Deepest element nesting any walk in this crate follows; also the parser's limit.
pub const MAX_DEPTH: usize = 256;
/// Most nodes in one part.
pub const MAX_NODES: usize = 4_000_000;

/// Parses a part: names as written, namespace declarations kept, [`MAX_DEPTH`] and
/// [`MAX_NODES`].
pub fn parse(bytes: &[u8]) -> Result<El, craft_xml::Error> {
    Options { limits: Limits { max_depth: MAX_DEPTH, max_nodes: MAX_NODES, ..Limits::DEFAULT }, ..Options::default() }.parse(bytes)
}

/// The namespace declarations on `root` as (`xmlns:p`, URI) pairs.
pub fn ns_decls(root: &El) -> Vec<(String, String)> {
    root.attrs.iter().filter(|(k, _)| k.starts_with("xmlns")).cloned().collect()
}

/// An attribute list built before its tag is started.
#[derive(Default)]
pub struct A {
    v: Vec<(&'static str, String)>,
}

impl A {
    pub fn new() -> Self {
        A { v: vec![] }
    }
    #[must_use]
    pub fn a(mut self, k: &'static str, v: impl ToString) -> Self {
        self.v.push((k, v.to_string()));
        self
    }
    #[must_use]
    pub fn o<T: ToString>(mut self, k: &'static str, v: Option<T>) -> Self {
        if let Some(v) = v {
            self.v.push((k, v.to_string()));
        }
        self
    }
    /// A boolean written as `1` or `0` when there is one.
    #[must_use]
    pub fn b(self, k: &'static str, v: Option<bool>) -> Self {
        self.o(k, v.map(u8::from))
    }
    /// A boolean written only when true.
    #[must_use]
    pub fn t(self, k: &'static str, v: bool) -> Self {
        self.o(k, v.then_some(1))
    }
}

/// [`A`] on a [`Tag`].
pub trait TagExt {
    #[must_use]
    fn list(self, a: A) -> Self;
}

impl TagExt for Tag<'_> {
    fn list(self, a: A) -> Self {
        a.v.iter().fold(self, |t, (k, v)| t.attr(k, v))
    }
}

/// Prefixes declared below the root but not on it, skipping any bound to different namespaces.
pub fn inner_decls(root: &El, ns: &[(String, String)]) -> Vec<(String, String)> {
    fn rec(e: &El, out: &mut Vec<(String, String)>, out_map: &mut HashMap<String, String>, clash: &mut HashSet<String>, depth: usize) {
        if depth > MAX_DEPTH {
            return;
        }
        for (k, v) in e.attrs.iter().filter(|(k, _)| k.starts_with("xmlns:")) {
            if let Some(existing) = out_map.get(k) {
                if existing != v {
                    clash.insert(k.clone());
                }
            } else if out.len() < 1000 {
                out_map.insert(k.clone(), v.clone());
                out.push((k.clone(), v.clone()));
            }
        }
        for c in e.elements() {
            rec(c, out, out_map, clash, depth + 1);
        }
    }
    let mut out = vec![];
    let mut out_map = HashMap::new();
    let mut clash = HashSet::new();
    let root_ns: HashSet<&str> = ns.iter().map(|(r, _)| r.as_str()).collect();
    for c in root.elements() {
        rec(c, &mut out, &mut out_map, &mut clash, 1);
    }
    out.retain(|(k, _)| !clash.contains(k) && !root_ns.contains(k.as_str()));
    out
}

/// Serialize `el`, declaring every prefix it uses that the source root declared.
pub fn keep_xml(el: &El, ns: &[(String, String)]) -> String {
    let mut used = HashSet::new();
    fn rec(e: &El, used: &mut HashSet<String>, depth: usize) {
        if depth > MAX_DEPTH {
            return;
        }
        if let Some((p, _)) = e.name.split_once(':') {
            used.insert(p.to_string());
        }
        for (k, _) in &e.attrs {
            if let Some((p, _)) = k.split_once(':')
                && p != "xmlns"
                && p != "xml"
            {
                used.insert(p.to_string());
            }
        }
        for c in e.elements() {
            rec(c, used, depth + 1);
        }
    }
    rec(el, &mut used, 0);
    let mut el = el.clone();
    for (k, v) in ns {
        let p = k.strip_prefix("xmlns:").unwrap_or("");
        if !p.is_empty() && used.contains(p) && !el.attrs.iter().any(|(a, _)| a == k) {
            el.attrs.push((k.clone(), v.clone()));
        }
    }
    el.to_xml()
}

/// Is this attribute a relationship id (`r:id`, `r:embed`, `r:dm`…)?
pub fn is_rel_attr(k: &str, v: &str) -> bool {
    !k.starts_with("xmlns")
        && k.contains(':')
        && matches!(k.rsplit(':').next(), Some("id" | "embed" | "link" | "dm" | "lo" | "qs" | "cs" | "pict" | "href"))
        && !v.is_empty()
}

/// Every relationship id `el` and its descendants use, sorted and without repeats.
pub fn rel_refs(el: &El) -> Vec<String> {
    fn rec(e: &El, out: &mut Vec<String>, depth: usize) {
        if depth > MAX_DEPTH {
            return;
        }
        out.extend(e.attrs.iter().filter(|(k, v)| is_rel_attr(k, v)).map(|(_, v)| v.clone()));
        for c in e.elements() {
            rec(c, out, depth + 1);
        }
    }
    let mut out = vec![];
    rec(el, &mut out, 0);
    out.sort();
    out.dedup();
    out
}

/// Does `el` or anything under it use a relationship id? Such a fragment cannot be kept as
/// text: the id means nothing in the part it is written back to.
pub fn has_rel_refs(el: &El) -> bool {
    !rel_refs(el).is_empty()
}

/// Namespace prefixes whose `mc:Choice` content the readers understand.
pub const SUPPORTED_MC: [&str; 10] = ["p14", "p15", "p159", "a14", "a16", "p1510", "a15", "c14", "p188", "cx1"];

/// An Office 2016 chart sits in a `cx1` choice; it is taken only when the choice holds a
/// `chartex` graphic (the fallback is a picture or "this chart isn't available" text).
fn chartex_readable(ch: &El) -> bool {
    ch.find_all(Local("graphicData")).any(|g| g.attr_local("uri").is_some_and(|u| u.ends_with("/chartex")))
}

/// Replaces every `mc:AlternateContent` by the children of the branch the readers handle best:
/// the first `mc:Choice` whose `Requires` prefixes are all in [`SUPPORTED_MC`], else
/// `mc:Fallback`. Ink content parts take the fallback picture.
pub fn resolve_mc(el: &mut El) {
    resolve_mc_keeping(el, &|_| false);
}

/// [`resolve_mc`], leaving in place any `mc:AlternateContent` for which `keep` is true.
pub fn resolve_mc_keeping(el: &mut El, keep: &dyn Fn(&El) -> bool) {
    rec_mc(el, 0, keep);
}

fn rec_mc(el: &mut El, depth: usize, keep: &dyn Fn(&El) -> bool) {
    if depth > MAX_DEPTH {
        return;
    }
    let ac = Local("AlternateContent");
    if el.children.iter().any(|c| matches!(c, Node::El(e) if e.is(ac))) {
        let mut out = Vec::with_capacity(el.children.len());
        for c in std::mem::take(&mut el.children) {
            match c {
                Node::El(ac_el) if ac_el.is(ac) => {
                    if keep(&ac_el) {
                        out.push(Node::El(ac_el));
                        continue;
                    }
                    let choice = ac_el.children(Local("Choice")).find(|ch| {
                        let requires = ch.attr_local("Requires").unwrap_or("");
                        let req_ok = requires.split_whitespace().all(|r| SUPPORTED_MC.contains(&r));
                        let modelled = !ch.elements().any(|e| e.is(Local("contentPart"))) && (!requires.contains("cx1") || chartex_readable(ch));
                        req_ok && modelled
                    });
                    if let Some(b) = choice.or_else(|| ac_el.child(Local("Fallback"))) {
                        for n in &b.children {
                            if let Node::El(e) = n {
                                let mut e = e.clone();
                                // Carry namespace declarations from the branch onto its content.
                                for (k, v) in b.attrs.iter().chain(ac_el.attrs.iter()) {
                                    if k.starts_with("xmlns") && !e.attrs.iter().any(|(a, _)| a == k) {
                                        e.attrs.push((k.clone(), v.clone()));
                                    }
                                }
                                out.push(Node::El(e));
                            }
                        }
                    }
                }
                other => out.push(other),
            }
        }
        el.children = out;
    }
    for c in el.elements_mut() {
        rec_mc(c, depth + 1, keep);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_kept_fragment_declares_the_prefixes_it_uses_and_no_others() {
        let root = parse(br#"<p:sld xmlns:p="P" xmlns:a="A" xmlns:r="R"><p:x><a:y r:id="rId1"/></p:x><p:z/></p:sld>"#).unwrap();
        let ns = ns_decls(&root);
        let x = root.child(Local("x")).unwrap();
        assert_eq!(keep_xml(x, &ns), r#"<p:x xmlns:p="P" xmlns:a="A" xmlns:r="R"><a:y r:id="rId1"/></p:x>"#);
        assert_eq!(keep_xml(root.child(Local("z")).unwrap(), &ns), r#"<p:z xmlns:p="P"/>"#);
        assert_eq!(rel_refs(x), ["rId1"]);
        assert!(!has_rel_refs(root.child(Local("z")).unwrap()));
    }

    #[test]
    fn inner_declarations_are_found_and_clashing_ones_dropped() {
        let root = parse(br#"<r xmlns:a="A"><x xmlns:b="B"><y xmlns:c="C1"/></x><z xmlns:c="C2" xmlns:a="A"/></r>"#).unwrap();
        assert_eq!(inner_decls(&root, &ns_decls(&root)), [("xmlns:b".to_string(), "B".to_string())]);
    }

    #[test]
    fn alternate_content_takes_a_supported_choice_else_the_fallback() {
        let mut root = parse(
            br#"<r xmlns:mc="M"><mc:AlternateContent><mc:Choice Requires="p14"><a/></mc:Choice><mc:Fallback><b/></mc:Fallback></mc:AlternateContent><mc:AlternateContent><mc:Choice Requires="zz9"><c/></mc:Choice><mc:Fallback><d/></mc:Fallback></mc:AlternateContent></r>"#,
        )
        .unwrap();
        let mut kept = root.clone();
        resolve_mc(&mut root);
        assert_eq!(root.elements().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["a", "d"]);
        resolve_mc_keeping(&mut kept, &|ac| ac.find(Local("c")).is_some());
        assert_eq!(kept.elements().map(|e| e.local()).collect::<Vec<_>>(), ["a", "AlternateContent"]);
    }

    #[test]
    fn the_parser_refuses_nesting_past_the_depth_limit() {
        let deep = "<a>".repeat(MAX_DEPTH + 2);
        assert!(parse(deep.as_bytes()).is_err());
        assert!(parse(b"<a><b>12</b></a>").is_ok());
    }

    #[test]
    fn an_attribute_list_writes_booleans_as_digits_and_skips_absent_values() {
        let mut w = craft_xml::Writer::new();
        w.tag("x").list(A::new().a("k", 1).o("n", None::<i32>).b("b", Some(false)).t("t", true).t("f", false)).empty();
        assert_eq!(w.finish(), r#"<x k="1" b="0" t="1"/>"#);
    }
}
