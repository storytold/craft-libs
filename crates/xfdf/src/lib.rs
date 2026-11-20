//! Bounded XFDF field values and annotation/FDF grammar; PDF annotations and form application remain caller-owned.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

pub mod grammar;
pub use grammar::is_xfdf_comments;
const MAX_NODES: u32 = 2_000_000;
const MAX_DEPTH: usize = 64;
const MAX_XML_DEPTH: usize = 1024;
const MAX_XFDF_TEXT: usize = 64 << 20;
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("this file isn't XFDF, FDF, XML or text form data")]
    UnknownFormat,
    #[error("the file can't be read: {0}")]
    Malformed(String),
}
/// Read field names and values in file order. This does not apply values to a PDF.
/// UTF-8 replacement and leading BOM/space handling retain the source app policy.
pub fn read_fields(bytes: &[u8]) -> Result<Vec<(String, Vec<String>)>, Error> {
    let text = String::from_utf8_lossy(bytes);
    let text = text.trim_start_matches('\u{feff}').trim_start();
    check_form_xml_depth(text)?;
    let root = parse_form_xml(text).map_err(|_| Error::UnknownFormat)?;
    if root.local() != "xfdf" {
        return Err(Error::UnknownFormat);
    }
    let mut values = Vec::new();
    if let Some(fields) = root.child(craft_xml::Local("fields")) {
        walk_xfdf(fields, "", &mut values, 0, &mut 0)?;
    }
    Ok(values)
}
fn xml_nesting_depth(text: &str) -> usize {
    fn find(hay: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
        (from..hay.len()).find(|&p| hay[p..].starts_with(needle))
    }
    let b = text.as_bytes();
    let (mut i, mut depth, mut max) = (0usize, 0usize, 0usize);
    while i < b.len() {
        if b[i] != b'<' {
            i += 1;
        } else if b[i..].starts_with(b"<!--") {
            i = find(b, i + 4, b"-->").map_or(b.len(), |p| p + 3);
        } else if b[i..].starts_with(b"<![CDATA[") {
            i = find(b, i + 9, b"]]>").map_or(b.len(), |p| p + 3);
        } else if b[i..].starts_with(b"<?") {
            i = find(b, i + 2, b"?>").map_or(b.len(), |p| p + 2);
        } else if b[i..].starts_with(b"<!") {
            i = find(b, i + 2, b">").map_or(b.len(), |p| p + 1);
        } else if b[i..].starts_with(b"</") {
            depth = depth.saturating_sub(1);
            i = find(b, i + 2, b">").map_or(b.len(), |p| p + 1);
        } else {
            depth += 1;
            max = max.max(depth);
            // Skip the rest of the start tag, over any quoted attribute value, and remember its last
            // significant byte: a tag that ends in `/` closes itself, so it is one level deep while it
            // is read rather than a level left open.
            i += 1;
            let mut quote = 0u8;
            let mut last = 0u8;
            while i < b.len() {
                let c = b[i];
                if quote != 0 {
                    if c == quote {
                        quote = 0;
                    }
                } else if c == b'"' || c == b'\'' {
                    quote = c;
                } else if c == b'>' {
                    i += 1;
                    break;
                } else if !c.is_ascii_whitespace() {
                    last = c;
                }
                i += 1;
            }
            if last == b'/' {
                depth -= 1;
            }
        }
    }
    max
}

fn check_form_xml_depth(text: &str) -> Result<(), Error> {
    let depth = xml_nesting_depth(text);
    if depth > MAX_XML_DEPTH {
        return Err(Error::Malformed(format!("the file nests XML elements {depth} deep, past the {MAX_XML_DEPTH} the reader accepts")));
    }
    Ok(())
}

fn parse_form_xml(text: &str) -> Result<craft_xml::El, craft_xml::Error> {
    let mut opts = craft_xml::Options { strict: true, doctype: craft_xml::Doctype::Reject, ..Default::default() };
    opts.limits.max_nodes = MAX_NODES as usize;
    opts.limits.max_depth = MAX_XML_DEPTH.max(1024);
    opts.parse(text.as_bytes())
}

fn el_text(el: &craft_xml::El) -> String {
    el.children
        .iter()
        .filter_map(|n| match n {
            craft_xml::Node::Text(t) => Some(t.as_str()),
            _ => None,
        })
        .collect()
}

fn walk_xfdf(node: &craft_xml::El, prefix: &str, out: &mut Vec<(String, Vec<String>)>, depth: usize, bytes: &mut usize) -> Result<(), Error> {
    if depth > MAX_DEPTH {
        return Err(Error::Malformed(format!("XFDF fields nest more than {MAX_DEPTH} deep")));
    }
    for f in node.children(craft_xml::Local("field")) {
        let Some(name) = f.attr("name") else { continue };
        // Charge temporary parent names too, before allocation/recursion can amplify them.
        let full_len = if prefix.is_empty() { name.len() } else { prefix.len().saturating_add(1).saturating_add(name.len()) };
        charge_xfdf_text(bytes, full_len)?;
        let full = if prefix.is_empty() { name.to_string() } else { format!("{prefix}.{name}") };
        let mut vals = Vec::new();
        for value in f.children(craft_xml::Local("value")) {
            let len = value
                .children
                .iter()
                .filter_map(|n| match n {
                    craft_xml::Node::Text(t) => Some(t.len()),
                    _ => None,
                })
                .fold(0, usize::saturating_add);
            charge_xfdf_text(bytes, len)?;
            vals.push(el_text(value));
        }
        // A leaf with no `<value>` is an empty field (the export writes one that way), so it
        // clears the field like FDF's `/V ()`; a field with kids only names their parent. A
        // leaf holding only `<value-richtext>` is not read yet, so it leaves the field alone.
        let names_kids_or_rich = f.elements().any(|c| matches!(c.local(), "field" | "value-richtext"));
        if !vals.is_empty() || !names_kids_or_rich {
            charge_xfdf_text(bytes, full.len())?; // The emitted name is a second owned copy.
            out.push((full.clone(), vals));
        }
        walk_xfdf(f, &full, out, depth + 1, bytes)?;
    }
    Ok(())
}

fn charge_xfdf_text(bytes: &mut usize, amount: usize) -> Result<(), Error> {
    *bytes = bytes.saturating_add(amount);
    if *bytes > MAX_XFDF_TEXT {
        return Err(Error::Malformed(format!("the file expands to more than {} MiB of field names and values", MAX_XFDF_TEXT >> 20)));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn field_order_empty_rich_and_split_values() {
        let xml = b"<xfdf><fields><field name='parent'><field name='child'><value>a<!--x-->b</value><value>c</value></field></field><field name='empty'/><field name='rich'><value-richtext>skip</value-richtext></field><field name='parent.child'><value>last</value></field></fields></xfdf>";
        assert_eq!(
            read_fields(xml).unwrap(),
            vec![("parent.child".into(), vec!["ab".into(), "c".into()]), ("empty".into(), vec![]), ("parent.child".into(), vec!["last".into()])]
        );
    }
    #[test]
    fn bounds_and_strict_xml_hardening() {
        assert_eq!(read_fields(b"<other/>"), Err(Error::UnknownFormat));
        assert!(read_fields(b"<!DOCTYPE xfdf><xfdf/>").is_err());
        assert!(read_fields(b"<xfdf a='1' a='2'/>").is_err());
        let text = format!("<xfdf><fields>{}{}{}</fields></xfdf>", "<field name='n'>".repeat(65), "<value>x</value>", "</field>".repeat(65));
        assert!(read_fields(text.as_bytes()).is_err());
        let fields = parse_form_xml("<fields><field name='parent'><value-richtext/></field></fields>").unwrap();
        let mut out = Vec::new();
        assert!(walk_xfdf(&fields, "", &mut out, 0, &mut (MAX_XFDF_TEXT - 5)).is_err());
        assert!(out.is_empty());
        walk_xfdf(&fields, "", &mut out, 0, &mut (MAX_XFDF_TEXT - 6)).unwrap();
    }
}
