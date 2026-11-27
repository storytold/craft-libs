//! Helpers the part writers share: numbers as SpreadsheetML writes them, rebuilding a kept part
//! around regenerated children, and writing a tree with one child streamed.

use std::fmt::Write as _;

use craft_xml::{El, Node, Writer, is_name};

/// A number for an attribute or a `<v>`: integers without a fraction, others in the shortest
/// form that reads back to the same value, `E` for the exponent. Not finite is `0`.
pub fn num(n: f64) -> String {
    if !n.is_finite() {
        return "0".into();
    }
    if n == n.trunc() && n.abs() < 1e15 {
        return format!("{}", n as i64);
    }
    let mut s = String::new();
    let _ = write!(s, "{n:?}");
    s.replace('e', "E")
}

/// `root` with the children named in `owned` regenerated. Each `(name, new)` takes the place
/// of the first child of that name, and later children of that name are dropped; when `root`
/// has none, `new` goes where `order` (the schema's sequence) puts it. No element (`None`)
/// removes the children; several (repeated blocks) go in a row. Every other child stays where it is, so elements the schema does not list
/// (`mc:AlternateContent`) keep their place.
pub(crate) fn assemble<I: IntoIterator<Item = El>>(root: &El, order: &[&str], owned: Vec<(&str, I)>) -> El {
    let mut owned: Vec<(&str, Vec<El>)> = owned.into_iter().map(|(n, e)| (n, e.into_iter().collect())).collect();
    let pos = |n: &str| order.iter().position(|o| *o == n).unwrap_or(order.len());
    owned.sort_by_key(|(n, _)| pos(n));
    // The children of `root` each owned name replaces, so a regenerated one is not also written
    // where `order` would put it.
    let present: Vec<&str> = owned.iter().map(|(n, _)| *n).filter(|n| root.child(*n).is_some()).collect();
    let mut out = El { name: root.name.clone(), attrs: root.attrs.clone(), children: Vec::new() };
    let mut pending: Vec<(&str, Vec<El>)> = Vec::new();
    let mut later: Vec<(&str, Vec<El>)> = Vec::new();
    for (n, e) in owned {
        if present.contains(&n) { later.push((n, e)) } else { pending.push((n, e)) }
    }
    let mut done: Vec<&str> = Vec::new();
    for node in &root.children {
        let Node::El(c) = node else {
            out.children.push(node.clone());
            continue;
        };
        let p = pos(&c.name);
        if p < order.len() {
            // Missing children that come before this one.
            while pending.first().is_some_and(|(n, _)| pos(n) < p) {
                let (_, e) = pending.remove(0);
                out.children.extend(e.into_iter().map(Node::El));
            }
        }
        if let Some(i) = later.iter().position(|(n, _)| *n == c.name) {
            let (n, e) = later.remove(i);
            out.children.extend(e.into_iter().map(Node::El));
            done.push(n);
            continue;
        }
        if done.iter().any(|n| *n == c.name) {
            continue;
        }
        out.children.push(node.clone());
    }
    out.children.extend(pending.into_iter().flat_map(|(_, e)| e).map(Node::El));
    out
}

/// Writes `root` as a document, with the child named `streamed` written by `each` instead of
/// from the tree (a worksheet's `sheetData`).
pub(crate) fn write_streamed(root: &El, streamed: &str, mut each: impl FnMut(&mut Writer)) -> Vec<u8> {
    let mut w = Writer::document();
    let mut tag = w.tag(&root.name);
    for (k, v) in root.attrs.iter().filter(|(k, _)| is_name(k)) {
        tag = tag.attr(k, v);
    }
    tag.open();
    for node in &root.children {
        match node {
            Node::El(c) if c.name == streamed => {
                w.tag(streamed).open();
                each(&mut w);
                w.close();
            }
            Node::El(c) => w.tree(c),
            Node::Text(t) => w.text(t),
            _ => {}
        }
    }
    w.into_bytes()
}

/// A `count` attribute and the given children, on a copy of `root`'s child `name` (keeping its
/// other attributes, `x14ac:knownFonts` and the like); `None` when there are no items.
pub(crate) fn list(root: &El, name: &str, items: Vec<El>) -> Option<El> {
    if items.is_empty() {
        return None;
    }
    let mut e = El::new(name);
    if let Some(old) = root.child(name) {
        e.attrs = old.attrs.iter().filter(|(k, _)| k != "count").cloned().collect();
    }
    e.attrs.insert(0, ("count".into(), items.len().to_string()));
    e.children = items.into_iter().map(Node::El).collect();
    Some(e)
}

/// Sets attribute `name` to `value`, or removes it for `None`.
pub(crate) fn set(e: &mut El, name: &str, value: Option<String>) {
    match value {
        Some(v) => e.set_attr(name, v),
        None => {
            e.remove_attr(name);
        }
    }
}

/// `Some("1")` for true: a flag written only when it is set.
pub(crate) fn flag(on: bool) -> Option<String> {
    on.then(|| "1".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers() {
        assert_eq!(num(3.0), "3");
        assert_eq!(num(-0.0), "0");
        assert_eq!(num(0.1), "0.1");
        assert_eq!(num(1e20), "1E20");
        assert_eq!(num(1.5e-7), "1.5E-7");
        assert_eq!(num(f64::NAN), "0");
        for n in [0.1 + 0.2, 1.0 / 3.0, 123456789.123, -2.5e-300] {
            assert_eq!(num(n).parse::<f64>().ok(), Some(n));
        }
    }

    #[test]
    fn assembling_keeps_unlisted_children_in_place() {
        let root = El::new("w")
            .with_child(El::new("a"))
            .with_child(El::new("mc:AlternateContent"))
            .with_child(El::new("c").with_attr("old", 1))
            .with_child(El::new("c"))
            .with_child(El::new("ext"));
        let order = ["a", "b", "c", "d", "ext"];
        let out = assemble(
            &root,
            &order,
            vec![("c", vec![El::new("c").with_attr("new", 1), El::new("c")]), ("b", vec![El::new("b")]), ("d", vec![El::new("d")]), ("a", vec![])],
        );
        assert_eq!(out.to_xml(), r#"<w><mc:AlternateContent/><b/><c new="1"/><c/><d/><ext/></w>"#);
        let empty = assemble(&El::new("w"), &order, vec![("d", Some(El::new("d"))), ("a", Some(El::new("a")))]);
        assert_eq!(empty.to_xml(), "<w><a/><d/></w>");
    }
}
