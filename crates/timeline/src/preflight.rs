//! Allocation-free lexical nesting preflight for timeline XML and JSON.
//!
//! Extracted from EffectCraft's interchange reader; original attribution is retained in Git history.
//! These functions preserve its lightweight byte-scanner behavior. They do not validate
//! syntax, expand entities, decode encodings, or replace a bounded format parser. In
//! particular XML self-closing elements do not increase the count, and declarations are
//! skipped lexically rather than parsed as DTDs. Apply document/encoding/entity policy
//! separately; callers choose the maximum acceptable nesting for their parser.

/// The deepest element nesting of an XML document (a quick scan that skips comments, CDATA,
/// processing instructions, declarations and quoted attribute values).
pub fn xml_depth(b: &[u8]) -> usize {
    let (mut i, mut depth, mut max) = (0usize, 0usize, 0usize);
    let skip_to =
        |from: usize, end: &[u8]| b.get(from..).and_then(|r| r.windows(end.len()).position(|w| w == end)).map_or(b.len(), |p| from + p + end.len());
    while let Some(&c) = b.get(i) {
        if c != b'<' {
            i += 1;
            continue;
        }
        let rest = b.get(i..).unwrap_or_default();
        if rest.starts_with(b"<!--") {
            i = skip_to(i + 4, b"-->");
        } else if rest.starts_with(b"<![CDATA[") {
            i = skip_to(i + 9, b"]]>");
        } else if rest.starts_with(b"<?") {
            i = skip_to(i + 2, b"?>");
        } else if rest.starts_with(b"<!") {
            i = skip_to(i + 2, b">");
        } else if rest.starts_with(b"</") {
            depth = depth.saturating_sub(1);
            i = skip_to(i + 2, b">");
        } else {
            // A start tag: find its end outside quotes; `/>` closes it at once.
            let mut j = i + 1;
            let mut quote = None;
            while let Some(&c) = b.get(j) {
                match quote {
                    Some(q) if c == q => quote = None,
                    Some(_) => {}
                    None if c == b'"' || c == b'\'' => quote = Some(c),
                    None if c == b'>' => break,
                    None => {}
                }
                j += 1;
            }
            if b.get(j.wrapping_sub(1)) != Some(&b'/') {
                depth += 1;
                max = max.max(depth);
            }
            i = j + 1;
        }
    }
    max
}

/// The deepest array / object nesting of a JSON document (strings skipped).
pub fn json_depth(b: &[u8]) -> usize {
    let (mut depth, mut max, mut in_str, mut esc) = (0usize, 0usize, false, false);
    for &c in b {
        if in_str {
            if esc {
                esc = false;
            } else if c == b'\\' {
                esc = true;
            } else if c == b'"' {
                in_str = false;
            }
            continue;
        }
        match c {
            b'"' => in_str = true,
            b'[' | b'{' => {
                depth += 1;
                max = max.max(depth);
            }
            b']' | b'}' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    max
}
