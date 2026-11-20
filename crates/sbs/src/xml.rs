//! SBS XML attribute escaping, number parsing, and element traversal helpers.

use roxmltree::Node as XmlNode;

pub fn parse_floats(text: &str) -> Vec<f32> {
    text.split_whitespace().take(16).map(|t| t.parse::<f32>().ok().filter(|v| v.is_finite()).unwrap_or(0.0)).collect()
}

pub fn parse_ints(text: &str) -> Vec<i32> {
    let parse =
        |t: &str| t.parse::<i64>().map(|v| v.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32).or_else(|_| t.parse::<f64>().map(|v| v as i32));
    text.split_whitespace().take(16).map(|t| parse(t).unwrap_or(0)).collect()
}

pub fn format_float(v: f32) -> String {
    if v.is_finite() { format!("{v}") } else { "0".into() }
}

pub fn child_element<'a, 'input>(node: XmlNode<'a, 'input>, tag: &str) -> Option<XmlNode<'a, 'input>> {
    node.children().find(|c| c.is_element() && c.tag_name().name() == tag)
}

pub fn children_named<'a, 'input: 'a>(node: XmlNode<'a, 'input>, tag: &'a str) -> impl Iterator<Item = XmlNode<'a, 'input>> + 'a {
    node.children().filter(move |c| c.is_element() && c.tag_name().name() == tag)
}

pub fn child_value<'a>(node: XmlNode<'a, '_>, tag: &str) -> Option<&'a str> {
    child_element(node, tag).and_then(|c| c.attribute("v"))
}

pub fn xml_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    // SBS historically escapes apostrophes too, although attributes are double quoted.
    // Keep that byte-level policy while sharing XML validity/attribute escaping.
    for (i, part) in text.split('\'').enumerate() {
        if i != 0 {
            out.push_str("&apos;");
        }
        craft_xml::push_esc_attr(&mut out, part);
    }
    out
}

pub fn is_parameters_element(xml: &str) -> bool {
    validate_preserved_element(xml, "parameters", (16 << 20) - 1, 1_000_000).is_some()
}

pub fn validate_preserved_element<'a>(xml: &'a str, expected_tag: &str, max_bytes: usize, max_nodes: u32) -> Option<&'a str> {
    if xml.len() > max_bytes {
        return None;
    }
    let options = roxmltree::ParsingOptions { allow_dtd: false, nodes_limit: max_nodes, ..roxmltree::ParsingOptions::default() };
    let document = roxmltree::Document::parse_with_options(xml, options).ok()?;
    let root = document.root_element();
    if root.tag_name().name() != expected_tag {
        return None;
    }
    xml.get(root.range())
}

#[cfg(test)]
mod tests {
    use super::xml_escape;

    #[test]
    fn sbs_attribute_wire_spelling_is_preserved() {
        assert_eq!(xml_escape("&<>\"'\t\n\r"), "&amp;&lt;&gt;&quot;&apos;&#9;&#10;&#13;");
        assert_eq!(xml_escape("\0\u{1f}\u{fffe}\u{ffff}é😀"), "é😀");
        assert_eq!(xml_escape("''"), "&apos;&apos;");
        assert_eq!(xml_escape(""), "");
    }

    #[test]
    fn written_attribute_roundtrips_valid_unicode_and_whitespace() {
        let value = "élan 😀 ' \" & < >\t\n\r";
        let xml = format!("<value v=\"{}\"/>", xml_escape(value));
        let doc = roxmltree::Document::parse(&xml).expect("valid xml");
        assert_eq!(doc.root_element().attribute("v"), Some(value));
    }
}
