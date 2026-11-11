//! Checks on SVG text made before any XML reader sees it.
//!
//! XML readers and SVG converters read elements by recursion and expand a document type's
//! entities with no total limit, so a short hostile file can overflow the stack (an abort, not
//! an error) or allocate far more than it holds. [`prescan`] measures how deep the elements nest
//! in one pass over the text; [`entities_expand_past`] bounds what the entity references expand
//! to. Both read the text as it is written and never build a tree.

use std::borrow::Cow;
use std::collections::HashMap;

/// The most text the references to entities may expand to in all (bytes), as a limit for
/// [`entities_expand_past`]: far more than the namespace entities drawing apps declare.
pub const MAX_ENTITY_EXPANSION: usize = 16 << 20;

/// How deep entities that refer to entities are followed (XML readers' own limits are lower).
const MAX_NESTING: u32 = 16;

/// The longest entity name a reference is looked for with ([`references`]).
const MAX_NAME: usize = 256;

/// The internal general entities declared in `svg`: name → every value declared for it. Text that
/// only looks like a declaration (in a comment, or in another entity's value) is kept too: the
/// largest value counts, so it can only make the estimate larger.
fn declared(svg: &str) -> HashMap<&str, Vec<&str>> {
    let mut map: HashMap<&str, Vec<&str>> = HashMap::new();
    for (at, _) in svg.match_indices("<!ENTITY") {
        let rest = svg.get(at + "<!ENTITY".len()..).unwrap_or("").trim_start();
        // A parameter entity (`%`) isn't referenced as `&name;`.
        if rest.starts_with('%') {
            continue;
        }
        let Some(end) = rest.find(char::is_whitespace) else { continue };
        let (name, rest) = rest.split_at(end);
        let rest = rest.trim_start();
        let Some(quote) = rest.chars().next().filter(|q| matches!(q, '"' | '\'')) else { continue };
        let Some(value) = rest.get(1..).and_then(|r| r.split_once(quote)).map(|(v, _)| v) else { continue };
        map.entry(name).or_default().push(value);
    }
    map
}

/// The names of the `&name;` references in `text`.
fn references(text: &str) -> impl Iterator<Item = &str> {
    text.match_indices('&').filter_map(|(at, _)| {
        let rest = text.get(at + 1..)?;
        // Look no further than the longest name: a file of `&`s without `;` would be read quadratically.
        let end = rest.bytes().take(MAX_NAME + 1).position(|b| b == b';')?;
        let name = rest.get(..end)?;
        (!name.is_empty() && name.len() <= MAX_NAME && !name.starts_with('#') && !name.contains(char::is_whitespace)).then_some(name)
    })
}

/// What entity `name` expands to (bytes), references in its value followed.
fn expanded<'a>(name: &'a str, entities: &HashMap<&'a str, Vec<&'a str>>, memo: &mut HashMap<&'a str, usize>, depth: u32) -> usize {
    if let Some(n) = memo.get(name) {
        return *n;
    }
    let Some(values) = entities.get(name) else { return 0 };
    if depth >= MAX_NESTING {
        return usize::MAX;
    }
    let mut most = 0usize;
    for value in values {
        let mut total = value.len();
        for inner in references(value) {
            total = total.saturating_add(expanded(inner, entities, memo, depth + 1));
        }
        most = most.max(total);
    }
    memo.insert(name, most);
    most
}

/// Whether the references to the entities `svg` declares expand to more than `most` bytes in
/// all. False for a file that declares none.
pub fn entities_expand_past(svg: &str, most: usize) -> bool {
    let entities = declared(svg);
    if entities.is_empty() {
        return false;
    }
    // A reference to a longer name isn't looked for: refuse such a declaration instead.
    if entities.keys().any(|name| name.len() > MAX_NAME) {
        return true;
    }
    let mut memo = HashMap::new();
    let mut total = 0usize;
    for name in references(svg) {
        total = total.saturating_add(expanded(name, &entities, &mut memo, 0));
        if total > most {
            return true;
        }
    }
    false
}

/// What one pass over the XML text of an SVG finds before any XML reader ([`prescan`]).
pub struct Prescan<'s> {
    /// Whether the elements nest deeper than the cap. The pass stops there.
    pub too_deep: bool,
    /// The text with each data URL in an element's attribute values left out: a `data:` value's
    /// quotes kept, a `url(…)`'s parentheses and quotes. Borrowed when there is none, and the
    /// whole text when `too_deep`. An embedded picture is a data URL and most of the text when
    /// there is one, and no tag, attribute name, selector or CSS declaration can be inside it:
    /// substring searches for markers go over this instead of the text.
    pub markers: Cow<'s, str>,
}

/// Do the elements of the XML text `svg` nest deeper than `most`? See [`prescan`].
pub fn deeper_than(svg: &str, most: usize) -> bool {
    prescan(svg, most).too_deep
}

/// The one pass, with elements allowed to nest `most` deep (the root alone is 1): tags are
/// counted as the text has them; markup in the values of a document type's entities counts ten
/// times (how deep entities may refer to one another), on top of the deepest element. Comments,
/// character data and processing instructions hold no elements, and a `>` or a tag inside a
/// quoted attribute value is not one.
pub fn prescan(svg: &str, most: usize) -> Prescan<'_> {
    let b = svg.as_bytes();
    let find =
        |from: usize, what: &[u8]| b.get(from..).and_then(|rest| rest.windows(what.len()).position(|w| w == what)).map(|at| from + at + what.len());
    // The `data:` attribute values seen: each from past its opening quote to its closing quote.
    let mut data_values: Vec<(usize, usize)> = vec![];
    let is = |at: usize, what: &[u8]| b.get(at..at + what.len()).is_some_and(|v| v.eq_ignore_ascii_case(what));
    // The end of the tag starting at `from` (past its `>`), and whether it closes itself. An
    // element's `data:` values are noted, and in its other values (a `style`, a `fill`) the data
    // URL of each `url(…)`, up to its closing quote or `)`: what follows it, `display:none` say,
    // stays. The document type's quoted values are entities, whose markup becomes elements, so
    // none of them is. Where a URL's end is unclear, less is left out, never more.
    // The first `what` or `quote` at or after `at`: a data URL's end, or its value's.
    let next = |at: usize, what: u8, quote: u8| b.get(at..).and_then(|rest| rest.iter().position(|c| *c == what || *c == quote)).map(|n| at + n);
    let mut tag_end = |from: usize, element: bool| {
        let mut quote = None;
        let mut i = from;
        while let Some(&c) = b.get(i) {
            match quote {
                None if c == b'"' || c == b'\'' => {
                    quote = Some(c);
                    // A `data:` value: one search for its closing quote, not a step a byte.
                    if element && is(i + 1, b"data:") {
                        let Some(end) = next(i + 1, c, c) else { return (b.len(), false) };
                        data_values.push((i + 1, end));
                        i = end;
                        continue;
                    }
                }
                None if c == b'>' => return (i + 1, b.get(i.wrapping_sub(1)) == Some(&b'/')),
                Some(q) if c == q => quote = None,
                Some(q) if element && is(i, b"url(") => {
                    let at = b.get(i + 4..).and_then(|rest| rest.iter().position(|c| !c.is_ascii_whitespace())).map_or(b.len(), |n| i + 4 + n);
                    let (at, close) = match b.get(at) {
                        Some(&inner @ (b'"' | b'\'')) if inner != q => (at + 1, inner),
                        _ => (at, b')'),
                    };
                    // Its data URL, up to the byte that closes it; a value that ends first keeps it.
                    if is(at, b"data:") {
                        let Some(end) = next(at, close, q) else { return (b.len(), false) };
                        if b.get(end) == Some(&close) {
                            data_values.push((at, end));
                        }
                        i = end;
                        continue;
                    }
                }
                _ => {}
            }
            i += 1;
        }
        (b.len(), false)
    };
    // (now, deepest) of the elements, and of the markup inside the document type.
    let (mut elements, mut entities) = ((0usize, 0usize), (0usize, 0usize));
    let mut doctype_end = 0;
    let mut i = 0;
    while let Some(at) = b.get(i..).and_then(|rest| rest.iter().position(|c| *c == b'<')).map(|at| i + at) {
        let rest = b.get(at..).unwrap_or_default();
        let depth = if at < doctype_end { &mut entities } else { &mut elements };
        i = if rest.starts_with(b"<!--") {
            find(at, b"-->").unwrap_or(b.len())
        } else if rest.starts_with(b"<![CDATA[") {
            find(at, b"]]>").unwrap_or(b.len())
        } else if rest.starts_with(b"<?") {
            find(at, b"?>").unwrap_or(b.len())
        } else if rest.starts_with(b"<!DOCTYPE") {
            // Up to the `>` that ends it: after its internal subset (`[` … `]`) when it has one.
            let plain = tag_end(at, false).0;
            let subset = b.get(at..plain).is_some_and(|head| head.contains(&b'['));
            doctype_end = if subset { find(at, b"]>").unwrap_or(b.len()) } else { plain };
            at + 2
        } else if rest.starts_with(b"<!") {
            // A declaration of the document type: the markup in its value is counted as it comes.
            at + 2
        } else if rest.starts_with(b"</") {
            depth.0 = depth.0.saturating_sub(1);
            tag_end(at, false).0
        } else if at < doctype_end {
            // In an entity's value, itself in quotes: up to the tag's `>`.
            let end = find(at, b">").unwrap_or(b.len());
            depth.1 = depth.1.max(depth.0 + 1);
            depth.0 += usize::from(b.get(end.wrapping_sub(2)) != Some(&b'/'));
            end
        } else {
            let (end, empty) = tag_end(at, true);
            depth.1 = depth.1.max(depth.0 + 1);
            depth.0 += usize::from(!empty);
            end
        };
        if elements.1.saturating_add(entities.1.saturating_mul(10)) > most {
            return Prescan { too_deep: true, markers: Cow::Borrowed(svg) };
        }
    }
    let markers = if data_values.is_empty() {
        Cow::Borrowed(svg)
    } else {
        let mut text = String::with_capacity(svg.len().saturating_sub(data_values.iter().map(|(start, end)| end.saturating_sub(*start)).sum()));
        let mut from = 0;
        for &(start, end) in &data_values {
            // Each end is at a quote or a `)`, so both are character boundaries.
            text.push_str(svg.get(from..start).unwrap_or_default());
            from = end;
        }
        text.push_str(svg.get(from..).unwrap_or_default());
        Cow::Owned(text)
    };
    Prescan { too_deep: false, markers }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEPTH: usize = 256;

    fn too_deep(svg: &str) -> bool {
        deeper_than(svg, DEPTH)
    }

    fn expansion_too_large(svg: &str) -> bool {
        entities_expand_past(svg, MAX_ENTITY_EXPANSION)
    }

    #[test]
    fn a_repeated_large_entity_is_too_large_and_a_namespace_entity_is_not() {
        let big = "x".repeat(1 << 20);
        let refs = "&a;".repeat(20);
        let bomb = format!("<!DOCTYPE svg [<!ENTITY a \"{big}\">]><svg xmlns=\"http://www.w3.org/2000/svg\" id=\"{refs}\"/>");
        assert!(expansion_too_large(&bomb));
        let nested = format!("<!DOCTYPE svg [<!ENTITY a \"{big}\"><!ENTITY b \"{}\">]><svg id=\"{}\"/>", "&a;".repeat(8), "&b;".repeat(8));
        assert!(expansion_too_large(&nested));
        let ns = "<!DOCTYPE svg [<!ENTITY ns_svg \"http://www.w3.org/2000/svg\">]><svg xmlns=\"&ns_svg;\"/>";
        assert!(!expansion_too_large(ns));
        assert!(!expansion_too_large("<svg a=\"&amp;&#10;\"/>"));
        // Many `&`s with no `;` after them are read in linear time.
        let ampersands = format!("<!DOCTYPE svg [<!ENTITY a \"x\">]><svg id=\"{}\"/>", "&".repeat(1 << 20));
        let start = std::time::Instant::now();
        assert!(!expansion_too_large(&ampersands));
        // A quadratic read would take hours; the bound is loose for a loaded machine.
        assert!(start.elapsed() < std::time::Duration::from_secs(120), "{:?}", start.elapsed());
    }

    #[test]
    fn declarations_in_comments_and_long_names_cannot_hide_a_large_entity() {
        let big = "x".repeat(1 << 20);
        let refs = "&a;".repeat(20);
        let commented = format!("<!DOCTYPE svg [<!-- <!ENTITY a \"x\"> --><!ENTITY a \"{big}\">]><svg id=\"{refs}\"/>");
        assert!(expansion_too_large(&commented));
        let in_value = format!("<!DOCTYPE svg [<!ENTITY c \"<!ENTITY a 'x'>\"><!ENTITY a \"{big}\">]><svg id=\"{refs}\"/>");
        assert!(expansion_too_large(&in_value));
        let long = "n".repeat(MAX_NAME + 1);
        let long_name = format!("<!DOCTYPE svg [<!ENTITY {long} \"x\">]><svg id=\"&{long};\"/>");
        assert!(expansion_too_large(&long_name));
    }

    fn svg(body: &str) -> String {
        format!(r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="10" height="10">{body}</svg>"#)
    }

    /// `n` groups in one another around a rectangle.
    fn groups(n: usize) -> String {
        format!("{}<rect width='1' height='1'/>{}", "<g>".repeat(n), "</g>".repeat(n))
    }

    #[test]
    fn elements_nest_up_to_the_cap() {
        // The root, the groups and the rectangle.
        let (at, past) = (svg(&groups(DEPTH - 2)), svg(&groups(DEPTH - 1)));
        assert!(!too_deep(&at));
        assert!(too_deep(&past));
        assert!(too_deep(&svg(&groups(200_000))), "any depth is measured");
        let ten = svg(&groups(8));
        assert!(!deeper_than(&ten, 10) && deeper_than(&ten, 9));
        assert!(!deeper_than("<svg/>", 1) && deeper_than("<svg/>", 0));
    }

    #[test]
    fn the_text_is_measured_as_xml_reads_it() {
        let many = r#"<g a='>' b="<g>"><rect/></g>"#.repeat(500);
        assert!(!too_deep(&svg(&many)), "side by side, with `>` and tags in attribute values");
        let hidden = format!("<!-- {0} --><![CDATA[{0}]]><?pi {0}?>", "<g>".repeat(500));
        assert!(!too_deep(&svg(&hidden)), "comments, character data and instructions hold no elements");
        assert!(too_deep(&format!("<svg>{}", "<g>".repeat(DEPTH))), "never closed");
        assert!(!too_deep(&format!("{}<svg/>", "</g>".repeat(2 * DEPTH))), "ends without beginnings");
        // Entities: namespaces as drawing apps declare them, and markup that would be read ten deep.
        let names = r#"<!DOCTYPE svg PUBLIC "-//W3C//DTD SVG 1.1//EN" "http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd" [ <!ENTITY ns_svg "http://www.w3.org/2000/svg"> ]>"#;
        assert!(!too_deep(&format!("{names}{}", svg(&groups(30)))));
        let tenth = DEPTH / 10 + 1;
        let bomb = format!("<!DOCTYPE svg [ <!ENTITY a \"{}&a;{}\"> ]>", "<g>".repeat(tenth), "</g>".repeat(tenth));
        assert!(too_deep(&format!("{bomb}<svg>&a;</svg>")));
        assert!(!too_deep(""));
    }

    /// Searches go over the markers, not the text: an embedded picture is left out of them, and
    /// everything a search may need to find stays, a short `style` value included.
    #[test]
    fn the_markers_leave_out_every_data_url_and_keep_the_rest() {
        let picture = format!("data:image/png;base64,{}", "<use>display<script>".repeat(1000));
        let text = svg(&format!(
            r#"<image href="{picture}"/><image xlink:href='{picture}'/><rect style="display:none" data-x='1' d="M0 0h1"/><a HREF="DATA:text/plain,<g>"><g/></a>"#
        ));
        let scan = prescan(&text, DEPTH);
        assert!(!scan.too_deep);
        let markers = scan.markers.as_ref();
        assert!(!markers.contains("<use") && !markers.contains("script") && !markers.contains("base64"), "{markers}");
        assert!(markers.contains(r#"href=""/>"#) && markers.contains("xlink:href=''/>") && markers.contains(r#"HREF="""#), "{markers}");
        assert!(markers.contains(r#"style="display:none" data-x='1' d="M0 0h1""#) && markers.contains("<g/></a>"), "{markers}");
        assert!(markers.len() < text.len() / 10);
        // Without one, the text itself is the markers, unchanged.
        let plain = svg("<rect width='1' height='1' style='fill:url(#p)'/>");
        assert!(matches!(prescan(&plain, DEPTH).markers, Cow::Borrowed(m) if m == plain));
        // A quote inside a comment or character data starts no value.
        let odd = svg(r#"<!-- "data:x --><![CDATA["data:y]]><rect/>"#);
        assert!(matches!(prescan(&odd, DEPTH).markers, Cow::Borrowed(_)));
        // Nor does one in the document type: an entity's value is markup the tree will hold.
        let entity = format!(r#"<!DOCTYPE svg [<!ENTITY e "data:<script href='a.js'/>">]>{}"#, svg("<g>&e;</g>"));
        assert!(matches!(prescan(&entity, DEPTH).markers, Cow::Borrowed(_)));
        // In a style or a paint, the data URL of a `url(…)` alone, up to its quote or `)`; what
        // follows it stays.
        let styled = svg(&format!(
            r#"<rect style="fill:url(data:image/png;base64,{0});display:none"/><rect fill='url( "{1}" )' stroke="URL(DATA:x)"/>"#,
            "script".repeat(500),
            picture
        ));
        let markers = prescan(&styled, DEPTH).markers.into_owned();
        assert!(!markers.contains("script") && !markers.contains("base64") && !markers.contains("DATA:"), "{markers}");
        assert!(markers.contains(r#"style="fill:url();display:none""#) && markers.contains(r#"fill='url( "" )' stroke="URL()""#), "{markers}");
        // A `url(…)` that does not hold a data URL, or never closes inside its value, is kept.
        let kept = svg(r#"<rect style="fill:url(#p);stroke:url( data:x" fill="url('data:y)"/><rect fill="url("data:z)"/>"#);
        assert!(matches!(prescan(&kept, DEPTH).markers, Cow::Borrowed(_)));
        // Past the cap, nothing more is read.
        assert!(prescan(&svg(&groups(DEPTH)), DEPTH).too_deep);
    }
}
