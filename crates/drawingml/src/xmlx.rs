//! What the readers and writers of the office XML formats need on top of `craft-xml`: the part
//! parser with their limits, markup-compatibility branches, fragments kept as text with their
//! namespace declarations, and relationship ids inside a fragment.
//!
//! Readers here match elements with [`craft_xml::Local`] and attributes with
//! [`El::attr_local`], so a document may use any prefixes. Integers are read with
//! [`craft_xml::value::to_i64_lossy`] and its siblings: producers write `12700.0` and values
//! far out of range, and neither should lose the element.

use std::cell::Cell;
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

/// Longest namespace name copied onto a kept fragment or a resolved `mc` branch. Real ones are
/// URIs of well under 100 bytes; a longer one is not copied, so one declaration cannot be
/// multiplied by every fragment that uses its prefix.
pub const MAX_NS_VALUE: usize = 2048;

/// Namespace declaration bytes one part may copy onto all its kept fragments together (the
/// budget a caller gives [`keep_with`]); past it a fragment keeps its markup but is not given
/// further declarations.
pub const MAX_COPIED_NS_BYTES: usize = 8 << 20;

/// Prefixes `el` and its descendants use in element and attribute names, those listed in
/// `mc:Ignorable` or `mc:MustUnderstand` (which must stay declared for the markup-compatibility
/// rules to hold), and those starting a QName-like attribute value.
fn used_prefixes(el: &El) -> HashSet<&str> {
    fn rec<'a>(e: &'a El, used: &mut HashSet<&'a str>, depth: usize) {
        if depth > MAX_DEPTH {
            return;
        }
        if let Some((p, _)) = e.name.split_once(':') {
            used.insert(p);
        }
        for (k, v) in &e.attrs {
            if let Some((p, local)) = k.split_once(':')
                && p != "xmlns"
                && p != "xml"
            {
                used.insert(p);
                if local == "Ignorable" || local == "MustUnderstand" {
                    used.extend(v.split_whitespace());
                }
            }
            // QName-valued attributes (`xsi:type="q:Item"`) need their prefix declared too.
            if k != "xmlns" && !k.starts_with("xmlns:") {
                for t in v.split_whitespace() {
                    if let Some((p, _)) = t.split_once(':')
                        && !p.is_empty()
                    {
                        used.insert(p);
                    }
                }
            }
        }
        for c in e.elements() {
            rec(c, used, depth + 1);
        }
    }
    let mut used = HashSet::new();
    rec(el, &mut used, 0);
    used
}

/// Serialize `el`, declaring every prefix it uses that the source root declared.
pub fn keep_xml(el: &El, ns: &[(String, String)]) -> String {
    keep_with(el, &[ns], None)
}

/// [`keep_xml`] over several declaration lists (the root's, then an enclosing wrapper's), taking
/// the bytes of the declarations it copies from `budget` when there is one (see
/// [`MAX_COPIED_NS_BYTES`]).
pub fn keep_with(el: &El, decls: &[&[(String, String)]], budget: Option<&Cell<usize>>) -> String {
    let used = used_prefixes(el);
    let mut have: HashSet<&str> = el.attrs.iter().map(|(a, _)| a.as_str()).collect();
    let mut extra = Vec::new();
    for (k, v) in decls.iter().flat_map(|d| d.iter()) {
        let p = k.strip_prefix("xmlns:").unwrap_or("");
        if !p.is_empty() && used.contains(p) && v.len() <= MAX_NS_VALUE && !have.contains(k.as_str()) {
            let cost = k.len() + v.len();
            if let Some(b) = budget {
                if b.get() < cost {
                    continue;
                }
                b.set(b.get() - cost);
            }
            have.insert(k);
            extra.push((k.clone(), v.clone()));
        }
    }
    let mut el = el.clone();
    el.attrs.extend(extra);
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
                        // The branch's namespace declarations (the first usable one of each name,
                        // in order), indexed by prefix once: each child then looks up the prefixes
                        // it uses instead of scanning every declaration.
                        let mut decls: Vec<(&String, &String)> = Vec::new();
                        // An oversized declaration still shadows an outer one of its name: its
                        // content is left unbound rather than bound to the outer namespace.
                        let mut by_prefix: HashMap<&str, Option<usize>> = HashMap::new();
                        for (k, v) in b.attrs.iter().chain(ac_el.attrs.iter()) {
                            let prefix = if k == "xmlns" { Some("") } else { k.strip_prefix("xmlns:") };
                            if let Some(p) = prefix
                                && !by_prefix.contains_key(p)
                            {
                                by_prefix.insert(p, (v.len() <= MAX_NS_VALUE).then_some(decls.len()));
                                if v.len() <= MAX_NS_VALUE {
                                    decls.push((k, v));
                                }
                            }
                        }
                        for n in &b.children {
                            if let Node::El(e) = n {
                                // Carry the declarations from the branch that its content uses
                                // (all of them per child made children x declarations copies).
                                let used = used_prefixes(e);
                                let have: HashSet<&str> = e.attrs.iter().map(|(a, _)| a.as_str()).collect();
                                let mut wanted: Vec<usize> = std::iter::once("")
                                    .chain(used.iter().copied())
                                    .filter_map(|p| by_prefix.get(p).copied().flatten())
                                    .filter(|&i| decls.get(i).is_some_and(|(k, _)| !have.contains(k.as_str())))
                                    .collect();
                                wanted.sort_unstable();
                                wanted.dedup();
                                let mut e = e.clone();
                                e.attrs.extend(wanted.into_iter().filter_map(|i| decls.get(i)).map(|(k, v)| ((*k).clone(), (*v).clone())));
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

    #[test]
    fn mc_branches_carry_only_the_declarations_their_content_uses() {
        // 1,000 declarations on the branch (the parser takes at most 1,024 attributes on a tag)
        // and 2,000 children: each child gets the one it uses (and the default), not all 1,000.
        let decls: String = (0..1000).map(|i| format!(r#" xmlns:n{i}="urn:n{i}""#)).collect();
        let kids: String = (0..2000).map(|i| format!("<n{}:x/>", i % 1000)).collect();
        let x = format!(r#"<r xmlns:mc="M"><mc:AlternateContent><mc:Fallback xmlns="urn:d"{decls}>{kids}</mc:Fallback></mc:AlternateContent></r>"#);
        let mut d = parse(x.as_bytes()).unwrap();
        resolve_mc(&mut d);
        let kids: Vec<&El> = d.elements().collect();
        assert_eq!(kids.len(), 2000);
        assert_eq!(kids[7].attrs, vec![("xmlns".to_string(), "urn:d".to_string()), ("xmlns:n7".to_string(), "urn:n7".to_string())]);
    }

    /// The branch's own declaration of a prefix wins over the `AlternateContent`'s; an oversized
    /// one still shadows it (its content stays unbound); a child's own declaration is left alone.
    #[test]
    fn mc_branch_declarations_keep_their_precedence() {
        let long = "u".repeat(MAX_NS_VALUE + 1);
        let x = format!(
            r#"<r xmlns:mc="M"><mc:AlternateContent xmlns:p="urn:ac" xmlns:q="urn:q-short"><mc:Fallback xmlns:p="urn:b" xmlns:q="{long}"><p:x/><q:y/><p:z xmlns:p="urn:own"/></mc:Fallback></mc:AlternateContent></r>"#
        );
        let mut d = parse(x.as_bytes()).unwrap();
        resolve_mc(&mut d);
        let kids: Vec<&El> = d.elements().collect();
        let attrs = |i: usize| kids[i].attrs.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>();
        assert_eq!(attrs(0), ["xmlns:p=urn:b"]);
        assert!(attrs(1).is_empty(), "{:?}", attrs(1));
        assert_eq!(attrs(2), ["xmlns:p=urn:own"]);
    }

    #[test]
    fn mc_branches_keep_prefixes_named_by_must_understand_and_in_qname_values() {
        let x = br#"<r xmlns:mc="M"><mc:AlternateContent><mc:Fallback xmlns:q="urn:q"><a mc:MustUnderstand="q"/></mc:Fallback></mc:AlternateContent></r>"#;
        let mut d = parse(x).unwrap();
        resolve_mc(&mut d);
        let kid = d.elements().next().unwrap();
        assert!(kid.attrs.iter().any(|(k, v)| k == "xmlns:q" && v == "urn:q"), "{:?}", kid.attrs);
        let x = br#"<r xmlns:mc="M"><mc:AlternateContent><mc:Fallback xmlns:q="urn:q" xmlns:z="urn:z"><n xmlns:xsi="urn:xsi" xsi:type="q:Item"/></mc:Fallback></mc:AlternateContent></r>"#;
        let mut d = parse(x).unwrap();
        resolve_mc(&mut d);
        let kid = d.elements().next().unwrap();
        assert!(kid.attrs.iter().any(|(k, v)| k == "xmlns:q" && v == "urn:q"), "{:?}", kid.attrs);
        assert!(!kid.attrs.iter().any(|(k, _)| k == "xmlns:z"), "unused declarations stay out: {:?}", kid.attrs);
    }

    #[test]
    fn kept_fragments_skip_oversized_namespace_names_and_spend_a_budget() {
        let big = "u".repeat(MAX_NS_VALUE + 1);
        let ns = vec![("xmlns:a".to_string(), "urn:a".to_string()), ("xmlns:b".to_string(), big)];
        let mut el = parse(br#"<a:x xmlns:a="urn:a" xmlns:b="urn:b"><b:y/></a:x>"#).unwrap();
        el.attrs.clear();
        let out = keep_xml(&el, &ns);
        assert!(out.contains(r#"xmlns:a="urn:a""#), "{out}");
        assert!(!out.contains("uuuu"), "the oversized declaration is not copied");
        let budget = Cell::new(15);
        assert!(keep_with(&el, &[&ns], Some(&budget)).contains(r#"xmlns:a="urn:a""#), "within budget");
        assert!(budget.get() < 15);
        let again = keep_with(&el, &[&ns], Some(&budget));
        assert!(!again.contains("xmlns:a"), "budget spent, no further declarations: {again}");
    }
}
