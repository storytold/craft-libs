/// Length → points.
pub(super) fn length(v: &str) -> Option<f32> {
    let v = v.trim().to_ascii_lowercase();
    let num = |s: &str| s.trim().parse::<f32>().ok().filter(|x| x.is_finite());
    if let Some(n) = v.strip_suffix("pt") {
        num(n)
    } else if let Some(n) = v.strip_suffix("in") {
        num(n).map(|x| x * 72.0)
    } else if let Some(n) = v.strip_suffix("cm") {
        num(n).map(|x| x * 72.0 / 2.54)
    } else if let Some(n) = v.strip_suffix("mm") {
        num(n).map(|x| x * 72.0 / 25.4)
    } else if let Some(n) = v.strip_suffix("pc") {
        num(n).map(|x| x * 12.0)
    } else if let Some(n) = v.strip_suffix("px") {
        num(n).map(|x| x * 0.75)
    } else {
        num(&v)
    }
}

pub(super) type Attrs = Vec<(String, String)>;

pub(super) fn get<'a>(a: &'a Attrs, k: &str) -> Option<&'a str> {
    a.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str())
}

pub(super) fn local(q: &str) -> &str {
    q.rsplit(':').next().unwrap_or(q)
}

pub(super) enum Ev {
    Start(String, Attrs),
    End(String),
    Text(String),
}

/// Stream XML events (empty elements become start + end). Malformed XML ends the stream.
pub(super) fn events(xml: &[u8], mut f: impl FnMut(Ev)) {
    events_at(xml, |ev, _| f(ev));
}

/// [`events`], each with the byte range of `xml` its markup takes (an empty element's start
/// and end both take the whole element).
pub(super) fn events_at(xml: &[u8], mut f: impl FnMut(Ev, (usize, usize))) {
    // Names and attributes as written; no limit but the input's own size (each part is at most a
    // package entry), and at most 128 attributes kept per element.
    let limits = craft_xml::Limits {
        max_input: usize::MAX,
        max_depth: usize::MAX,
        max_nodes: usize::MAX,
        max_elements: usize::MAX,
        max_attrs: usize::MAX,
        max_attrs_total: usize::MAX,
    };
    let opts = craft_xml::Options::default().with_limits(limits);
    let mut events = opts.events(xml);
    // Malformed XML ends the stream.
    while let Ok(Some(ev)) = events.next() {
        let span = events.span();
        let at = (span.start, span.end);
        match ev {
            craft_xml::Event::Start { name, mut attrs } => {
                attrs.truncate(128);
                f(Ev::Start(name, attrs), at)
            }
            craft_xml::Event::End { name } => f(Ev::End(name), at),
            craft_xml::Event::Text(t) | craft_xml::Event::CData(t) => f(Ev::Text(t), at),
            _ => {}
        }
    }
}

/// Rewrite only the first interpreted draw:z-index value in each start tag.
/// Event spans exclude comments/CDATA and retain the reader's first-128-attribute
/// policy. All bytes outside changed values (including unknown markup) survive.
pub(super) fn remap_orders(xml: &[u8], map: impl Fn(u32) -> u32) -> Result<std::borrow::Cow<'_, [u8]>, String> {
    // Callers only remove one unused upper-order slot: unchanged MAX means identity.
    if map(u32::MAX) == u32::MAX {
        return Ok(std::borrow::Cow::Borrowed(xml));
    }
    let mut out = Vec::new();
    let mut copied = 0;
    let mut error = None;
    events_at(xml, |ev, (start, end)| {
        if error.is_some() {
            return;
        }
        let Ev::Start(_, attrs) = ev else { return };
        let Some(raw) = get(&attrs, "draw:z-index").and_then(|s| s.trim().parse::<u32>().ok()) else { return };
        let mapped = map(raw);
        if mapped == raw {
            return;
        }
        let Some(tag) = xml.get(start..end) else { return };
        let Some(range) = order_value(tag) else {
            error = Some("ODT: cannot locate retained stacking attribute".to_string());
            return;
        };
        let from = start + range.start;
        let to = start + range.end;
        // Maps never increase a value; decimal replacement cannot exceed its source bytes.
        if out.is_empty() && out.try_reserve_exact(xml.len()).is_err() {
            error = Some("ODT: cannot allocate normalized stacking markup".to_string());
            return;
        }
        out.extend_from_slice(&xml[copied..from]);
        out.extend_from_slice(mapped.to_string().as_bytes());
        copied = to;
    });
    if let Some(error) = error {
        return Err(error);
    }
    if copied == 0 {
        return Ok(std::borrow::Cow::Borrowed(xml));
    }
    out.extend_from_slice(&xml[copied..]);
    Ok(std::borrow::Cow::Owned(out))
}

/// Locate a quoted value in an already parsed start tag, not in arbitrary XML.
/// Skip entire quoted values so attribute-looking text inside them is untouched.
fn order_value(tag: &[u8]) -> Option<std::ops::Range<usize>> {
    let mut i = 1;
    while i < tag.len() && !tag[i].is_ascii_whitespace() && !matches!(tag[i], b'/' | b'>') {
        i += 1;
    }
    while i < tag.len() {
        while i < tag.len() && tag[i].is_ascii_whitespace() {
            i += 1;
        }
        let begin = i;
        while i < tag.len() && !tag[i].is_ascii_whitespace() && !matches!(tag[i], b'=' | b'/' | b'>') {
            i += 1;
        }
        let name = tag.get(begin..i)?;
        if name.is_empty() {
            // The lenient parser can recover after a stray delimiter before an attribute.
            i += 1;
            continue;
        }
        while i < tag.len() && tag[i].is_ascii_whitespace() {
            i += 1;
        }
        if tag.get(i) != Some(&b'=') {
            continue;
        }
        i += 1;
        while i < tag.len() && tag[i].is_ascii_whitespace() {
            i += 1;
        }
        let quote = *tag.get(i)?;
        if !matches!(quote, b'\'' | b'"') {
            while i < tag.len() && !tag[i].is_ascii_whitespace() {
                i += 1;
            }
            continue;
        }
        i += 1;
        let from = i;
        while i < tag.len() && tag[i] != quote {
            i += 1;
        }
        if i == tag.len() {
            return None;
        }
        if name == b"draw:z-index" {
            return Some(from..i);
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod order_tests {
    use super::*;

    #[test]
    fn order_remap_preserves_every_other_byte() {
        let xml = br#"<p a="draw:z-index='99'"><!-- draw:z-index="99" --><![CDATA[draw:z-index="99"]]><x q='&amp;' draw:z-index = ' 9&#57; ' weird="stay" draw:z-index="88"/><y draw:z-index="bad"/><z draw:z-index="0"/></p>"#;
        let expected = br#"<p a="draw:z-index='99'"><!-- draw:z-index="99" --><![CDATA[draw:z-index="99"]]><x q='&amp;' draw:z-index = '98' weird="stay" draw:z-index="88"/><y draw:z-index="bad"/><z draw:z-index="0"/></p>"#;
        assert_eq!(remap_orders(xml, |n| n.saturating_sub(1)).unwrap().as_ref(), expected);
        assert!(matches!(remap_orders(xml, |n| n).unwrap(), std::borrow::Cow::Borrowed(_)));
        let late = format!("<x {} draw:z-index='99'/>", (0..128).map(|i| format!("a{i}='x'")).collect::<Vec<_>>().join(" "));
        assert_eq!(remap_orders(late.as_bytes(), |n| n - 1).unwrap().as_ref(), late.as_bytes());
        for broken in ["=bad", "x=bad", "x==bad", "x", "x='a' x", "x=\"bad\"", "'odd'", "/odd", "x='a>b'"] {
            let input = format!("<x {broken} draw:z-index='99'/>");
            let mut seen = false;
            events(input.as_bytes(), |e| {
                if let Ev::Start(_, a) = e {
                    seen |= get(&a, "draw:z-index") == Some("99");
                }
            });
            if seen {
                let changed = remap_orders(input.as_bytes(), |n| n - 1).unwrap_or_else(|e| panic!("{input}: {e}"));
                let mut found = false;
                events(&changed, |e| {
                    if let Ev::Start(_, a) = e {
                        found |= get(&a, "draw:z-index") == Some("98");
                    }
                });
                assert!(found, "{input}");
            }
        }
        // Lenient malformed attributes are skipped just as in the reader.
        let malformed = b"<x broken draw:z-index='99'/>";
        assert_eq!(remap_orders(malformed, |n| n - 1).unwrap().as_ref(), b"<x broken draw:z-index='98'/>");
    }
}
