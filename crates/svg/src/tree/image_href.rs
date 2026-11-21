//! Bounded data-URL normalization over the shared checked XML document.
//! Only attribute values change; actual usvg callbacks decide image usage.
use super::Error;
#[cfg(test)]
use super::Limits;
use std::borrow::Cow;
use std::ops::Range;

#[cfg(test)]
const SVG_NS: &str = "http://www.w3.org/2000/svg";
const XLINK_NS: &str = "http://www.w3.org/1999/xlink";
const INVALID_IMAGE: &str = "data:image/png,%00";
const IGNORED_DATA: &str = "data:application/octet-stream,%00";

struct Patch {
    range: Range<usize>,
    replacement: Cow<'static, str>,
}

fn invalid(message: &str) -> Error {
    Error::Parse(message.into())
}

#[cfg(test)]
fn repair<'a>(text: &'a str, limits: &Limits) -> Result<Cow<'a, str>, Error> {
    normalize(&super::preflight(text, limits)?)
}

pub(super) fn normalize<'a>(checked: &super::Preflight<'a>) -> Result<Cow<'a, str>, Error> {
    let text = checked.xml.input_text();
    let limits = &checked.limits;
    let xml = &checked.xml;
    if !text.contains("href") {
        return Ok(Cow::Borrowed(text));
    }
    // Expanded values fit source + entity work. Twelve linear passes cover URL
    // scans/streaming decode, full attribute projection and worst-case XML escaping;
    // parameter compaction never allocates one record per original parameter.
    let mut work = text.len().saturating_add(limits.entity_bytes).saturating_mul(12);
    let max_patches = (limits.bytes / std::mem::size_of::<Patch>()).min(usize::try_from(limits.xml_nodes).unwrap_or(usize::MAX));
    let mut patches: Vec<Patch> = Vec::new();
    for node in xml.descendants().filter(|node| node.is_element()) {
        if !matches!(node.tag_name().name(), "image" | "feImage") {
            continue;
        }
        // Shared preflight inspects local image names in every namespace and
        // checks XLink before plain href. Normalize both representations, even
        // when one is inert or unselected. Only actual resolver callbacks count
        // omissions, so this does not turn hidden/unused resources into warnings.
        for href in node.attributes().filter(|a| a.name() == "href" && matches!(a.namespace(), None | Some(XLINK_NS))) {
            let Some(replacement) = data_url_replacement(href.value(), &mut work)? else { continue };
            let range = verified_value_range(text, node.range(), href.range(), href.name(), &mut work)?;
            if patches.len() == patches.capacity() {
                if patches.len() >= max_patches {
                    return Err(invalid("SVG data-URL repair metadata exceeds its limit"));
                }
                let capacity = patches.capacity().max(8).saturating_mul(2).min(max_patches);
                patches
                    .try_reserve_exact(capacity.saturating_sub(patches.len()))
                    .map_err(|_| invalid("could not allocate bounded SVG data-URL repairs"))?;
            }
            patches.push(Patch { range, replacement });
        }
    }
    if patches.is_empty() {
        return Ok(Cow::Borrowed(text));
    }
    patches.sort_unstable_by_key(|patch| (patch.range.start, patch.range.end));
    patches.dedup_by_key(|patch| (patch.range.start, patch.range.end));
    let mut end = 0;
    let mut length = text.len();
    for patch in &patches {
        let range = &patch.range;
        if range.start < end {
            return Err(invalid("SVG data-URL source ranges overlap"));
        }
        end = range.end;
        length = length
            .checked_sub(range.len())
            .and_then(|n| n.checked_add(patch.replacement.len()))
            .ok_or_else(|| invalid("SVG data-URL repair size overflows"))?;
    }
    if length > limits.bytes {
        return Err(invalid("repaired SVG exceeds the text byte limit"));
    }
    let mut result = String::new();
    result.try_reserve_exact(length).map_err(|_| invalid("could not allocate bounded repaired SVG text"))?;
    let mut cursor = 0;
    for patch in patches {
        result.push_str(text.get(cursor..patch.range.start).ok_or_else(|| invalid("invalid SVG data-URL source range"))?);
        result.push_str(&patch.replacement);
        cursor = patch.range.end;
    }
    result.push_str(text.get(cursor..).ok_or_else(|| invalid("invalid SVG data-URL source range"))?);
    Ok(Cow::Owned(result))
}

/// Fall back to the original, checked DOM when a repair cannot be projected.
/// Only MIME headers already safe for usvg's parser can take this route. Count
/// malformed declarations conservatively because usvg may drop them before
/// invoking a callback.
pub(super) fn unrepaired_image_omissions(checked: &super::Preflight<'_>) -> Option<usize> {
    let mut work = checked.xml.input_text().len().saturating_add(checked.limits.entity_bytes).saturating_mul(6);
    let mut omitted = 0usize;
    for node in checked.xml.descendants().filter(|node| node.is_element() && matches!(node.tag_name().name(), "image" | "feImage")) {
        for href in node.attributes().filter(|a| a.name() == "href" && matches!(a.namespace(), None | Some(XLINK_NS))) {
            let value = href.value();
            work = work.checked_sub(value.len())?;
            if !mime_header_admitted(value) {
                return None;
            }
        }
        if let Some(value) = node.attribute((XLINK_NS, "href")).or_else(|| node.attribute("href"))
            && malformed_data_url_replacement(value, &mut work).ok()? == Some(INVALID_IMAGE)
        {
            omitted = omitted.saturating_add(1);
        }
    }
    Some(omitted)
}

// The pinned MIME parser searches all earlier parameters for each new name.
// Cap even quoted semicolons before invoking it: source-byte work is already
// charged, so at most this constant times the bounded XML work reaches MIME
// parsing. Bodies can remain large without acquiring MIME parameter records.
const MAX_MIME_SEPARATORS: usize = 64;

fn mime_header_admitted(value: &str) -> bool {
    let trimmed = value.trim_start_matches(|ch| ch <= ' ');
    let mut bytes = trimmed.bytes();
    for expected in b"data:" {
        let Some(actual) = bytes.by_ref().find(|&byte| !matches!(byte, b'\t' | b'\n' | b'\r')) else { return true };
        if !actual.eq_ignore_ascii_case(expected) {
            return true; // Not a data URL: preserve external/fragment handling.
        }
    }
    let mut separators = 0;
    for byte in bytes {
        if matches!(byte, b',' | b'#') {
            break;
        }
        if byte == b';' {
            separators += 1;
            if separators > MAX_MIME_SEPARATORS {
                return false;
            }
        }
    }
    true
}

/// A long parameter list must not reach the quadratic MIME parser, but its
/// essence, final base64 marker and body still determine the original image.
/// Compact only that header, then use the same parser/streaming decoder. Escape
/// the decoded attribute value once when projecting it back into XML.
fn data_url_replacement(value: &str, work: &mut usize) -> Result<Option<Cow<'static, str>>, Error> {
    if mime_header_admitted(value) {
        return malformed_data_url_replacement(value, work).map(|replacement| replacement.map(Cow::Borrowed));
    }
    *work = work.checked_sub(value.len()).ok_or_else(|| invalid("SVG data-URL validation work exceeds its limit"))?;
    let normalized = value.trim_matches(|ch| ch <= ' ');
    let delimiter = normalized.find([',', '#']);
    let Some(comma) = delimiter.filter(|&at| normalized.as_bytes().get(at) == Some(&b',')) else {
        return missing_comma_replacement(value, work).map(|replacement| Some(Cow::Borrowed(replacement)));
    };
    let header = normalized.get(..comma).ok_or_else(|| invalid("invalid SVG data-URL header framing"))?;
    let essence = header.split_once(';').map_or(header, |(essence, _)| essence);
    // Retaining the final parameter lets the pinned parser decide whether its
    // suffix is a base64 marker, including its case/ASCII-whitespace rules.
    let suffix = header.rfind(';').and_then(|at| header.get(at..)).unwrap_or("");
    let body = normalized.get(comma..).ok_or_else(|| invalid("invalid SVG data-URL body framing"))?;
    let length = essence
        .len()
        .checked_add(suffix.len())
        .and_then(|n| n.checked_add(body.len()))
        .ok_or_else(|| invalid("SVG data-URL repair size overflows"))?;
    let mut compact = String::new();
    compact.try_reserve_exact(length).map_err(|_| invalid("could not allocate bounded SVG data-URL header"))?;
    compact.push_str(essence);
    compact.push_str(suffix);
    compact.push_str(body);
    if let Some(replacement) = malformed_data_url_replacement(&compact, work)? {
        return Ok(Some(Cow::Borrowed(replacement)));
    }
    let escaped_len = compact
        .chars()
        .try_fold(0usize, |n, ch| {
            n.checked_add(match ch {
                '&' | '\t' | '\n' | '\r' => 5,
                '\'' | '"' => 6,
                '<' | '>' => 4,
                _ => ch.len_utf8(),
            })
        })
        .ok_or_else(|| invalid("SVG data-URL repair size overflows"))?;
    *work = work.checked_sub(escaped_len).ok_or_else(|| invalid("SVG data-URL validation work exceeds its limit"))?;
    let mut escaped = String::new();
    escaped.try_reserve_exact(escaped_len).map_err(|_| invalid("could not allocate bounded SVG data-URL repair"))?;
    for ch in compact.chars() {
        match ch {
            '&' => escaped.push_str("&amp;"),
            '\'' => escaped.push_str("&apos;"),
            '"' => escaped.push_str("&quot;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '\t' => escaped.push_str("&#x9;"),
            '\n' => escaped.push_str("&#xA;"),
            '\r' => escaped.push_str("&#xD;"),
            _ => escaped.push(ch),
        }
    }
    Ok(Some(Cow::Owned(escaped)))
}

fn malformed_data_url_replacement(value: &str, work: &mut usize) -> Result<Option<&'static str>, Error> {
    malformed_data_url_replacement_with(value, work, data_url::DataUrl::process)
}

fn malformed_data_url_replacement_with<'a>(
    value: &'a str,
    work: &mut usize,
    parse: impl FnOnce(&'a str) -> Result<data_url::DataUrl<'a>, data_url::DataUrlError>,
) -> Result<Option<&'static str>, Error> {
    *work = work.checked_sub(value.len()).ok_or_else(|| invalid("SVG data-URL validation work exceeds its limit"))?;
    if !mime_header_admitted(value) {
        // Strip parameters before recovering MIME disposition; no original
        // parameter list reaches either our parser or shared whole-DOM preflight.
        return missing_comma_replacement(value, work).map(Some);
    }
    let url = match parse(value) {
        Ok(url) => url,
        Err(data_url::DataUrlError::NoComma) => return missing_comma_replacement(value, work).map(Some),
        Err(data_url::DataUrlError::NotADataUrl) => return Ok(None),
    };
    let mut exhausted = false;
    let decoded = url.decode(|bytes| {
        if let Some(left) = work.checked_sub(bytes.len()) {
            *work = left;
            Ok(())
        } else {
            exhausted = true;
            Err(())
        }
    });
    if exhausted {
        return Err(invalid("SVG data-URL validation work exceeds its limit"));
    }
    Ok(decoded.is_err().then(|| replacement_for_mime(url.mime_type())))
}

/// Preflight must see a valid URL, while the resolver must keep its original
/// disposition. Non-image MIME types are ignored even when their body is bad.
fn replacement_for_mime(mime: &data_url::mime::Mime) -> &'static str {
    if mime.type_ == "image" || mime.matches("text", "plain") { INVALID_IMAGE } else { IGNORED_DATA }
}

fn missing_comma_replacement(value: &str, work: &mut usize) -> Result<&'static str, Error> {
    // NoComma does not expose its MIME type. Ask the same parser about a header
    // with an empty body rather than guessing MIME defaults, case or parameters.
    // Match its URL C0 trimming; fragments and MIME parameters cannot affect
    // the image/text-plain essence. Do not allocate parameter bookkeeping for a
    // hostile missing-comma header just to recover that disposition.
    let normalized = value.trim_matches(|ch| ch <= ' ');
    let header = normalized.split_once('#').map_or(normalized, |(header, _)| header);
    let essence = header.split_once(';').map_or(header, |(essence, _)| essence);
    *work = work.checked_sub(essence.len()).ok_or_else(|| invalid("SVG data-URL validation work exceeds its limit"))?;
    let length = essence.len().checked_add(1).ok_or_else(|| invalid("SVG data-URL repair size overflows"))?;
    let mut candidate = String::new();
    candidate.try_reserve_exact(length).map_err(|_| invalid("could not allocate bounded SVG data-URL header"))?;
    candidate.push_str(essence);
    candidate.push(',');
    // This candidate has no MIME parameters, including for oversized original
    // lists. Its byte scan/allocation is charged above before the linear parse.
    if !mime_header_admitted(&candidate) {
        return Err(invalid("SVG data-URL MIME header exceeds its limit"));
    }
    let url = data_url::DataUrl::process(&candidate).map_err(|_| invalid("invalid SVG data-URL header framing"))?;
    Ok(replacement_for_mime(url.mime_type()))
}

/// Use the complete source range rather than roxmltree's compact QName/equals
/// offsets (u16/u8). Scan only this validated attribute, never a DTD declaration
/// or another node. Charge the extra source scan before examining it.
fn verified_value_range(text: &str, owner: Range<usize>, attribute: Range<usize>, local_name: &str, work: &mut usize) -> Result<Range<usize>, Error> {
    let error = || invalid("SVG data-URL attribute source positions cannot be represented safely");
    if attribute.start < owner.start || attribute.end > owner.end {
        return Err(error());
    }
    *work = work.checked_sub(attribute.len()).ok_or_else(|| invalid("SVG data-URL validation work exceeds its limit"))?;
    let source = text.get(attribute.clone()).ok_or_else(error)?;
    let name_end = source.find([' ', '\t', '\r', '\n', '=']).ok_or_else(error)?;
    let name = source.get(..name_end).ok_or_else(error)?;
    if name.rsplit(':').next() != Some(local_name) {
        return Err(error());
    }
    let after_name = source.get(name_end..).ok_or_else(error)?.trim_start_matches([' ', '\t', '\r', '\n']);
    let after_equals = after_name.strip_prefix('=').ok_or_else(error)?.trim_start_matches([' ', '\t', '\r', '\n']);
    let quote = after_equals.chars().next().ok_or_else(error)?;
    if !matches!(quote, '\'' | '"') || !after_equals.ends_with(quote) || after_equals.len() < 2 {
        return Err(error());
    }
    let start = attribute.end.checked_sub(after_equals.len()).and_then(|n| n.checked_add(1)).ok_or_else(error)?;
    let end = attribute.end.checked_sub(1).ok_or_else(error)?;
    if text.get(start..end).is_none_or(|value| value.contains(quote)) {
        return Err(error());
    }
    Ok(start..end)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits() -> Limits {
        Limits { bytes: 1 << 20, depth: 32, tree_depth: 32, entity_bytes: 65_536, ..Default::default() }
    }

    #[test]
    fn original_dom_fallback_does_not_bypass_mime_admission() {
        let ordinary = "<svg><image href='data:;base64'/></svg>";
        assert_eq!(unrepaired_image_omissions(&super::super::preflight(ordinary, &limits()).unwrap()), Some(1));

        let header = "a=b;".repeat(MAX_MIME_SEPARATORS + 1);
        let excessive = format!("<svg><image href='data:image/png;{header}base64,A'/></svg>");
        assert!(unrepaired_image_omissions(&super::super::preflight(&excessive, &limits()).unwrap()).is_none());
    }

    #[test]
    fn malformed_image_urls_are_repaired_in_both_svg_namespaces_and_filter_resources() {
        for tag in ["image", "feImage"] {
            for text in [
                format!("<svg xmlns='{SVG_NS}'><{tag} href='data:image/png;base64,A'/></svg>"),
                format!("<svg><{tag} href='data:image/png;base64,A'/></svg>"),
                format!("<s:svg xmlns:s='{SVG_NS}' xmlns:link='{XLINK_NS}'><s:{tag} link:href='data:image/png;base64,A'/></s:svg>"),
            ] {
                let repaired = repair(&text, &limits()).unwrap();
                assert!(matches!(repaired, Cow::Owned(_)));
                assert_eq!(repaired.matches(INVALID_IMAGE).count(), 1);
            }
        }
    }

    #[test]
    fn unselected_href_and_foreign_local_image_names_are_normalized_for_shared_preflight() {
        for attrs in ["href='data:image/png,%00' x:href='data:image/png;base64,A'", "x:href='data:image/png;base64,A' href='data:image/png,%00'"] {
            let text = format!("<svg xmlns='{SVG_NS}' xmlns:x='{XLINK_NS}'><image {attrs}/></svg>");
            let repaired = repair(&text, &limits()).unwrap();
            assert!(matches!(repaired, Cow::Owned(_)));
            assert_eq!(repaired.matches(INVALID_IMAGE).count(), 2);
            assert!(!repaired.contains("base64,A"));
        }
        let text = format!(
            "<svg xmlns='{SVG_NS}' xmlns:x='{XLINK_NS}' xmlns:f='urn:foreign'><image x:href='data:image/png,%00' href='data:image/png;base64,A'/><f:image href='data:image/png;base64,A'/><image f:href='data:image/png;base64,A'/></svg>"
        );
        let repaired = repair(&text, &limits()).unwrap();
        assert_eq!(repaired.matches(INVALID_IMAGE).count(), 3);
        assert!(repaired.contains("f:href='data:image/png;base64,A'"), "foreign href attributes remain inert and unchanged");
    }

    #[test]
    fn both_href_forms_on_hidden_unused_and_foreign_resources_do_not_preflight_as_omissions() {
        for tag in ["image", "feImage"] {
            let text = format!(
                "<svg xmlns='{SVG_NS}' xmlns:x='{XLINK_NS}' xmlns:f='urn:foreign'><defs><{tag} id='unused' href='data:image/png;base64,A' x:href='data:image/png;base64,A'/></defs><{tag} display='none' x:href='data:image/png;base64,A' href='data:image/png;base64,A'/><f:{tag} href='data:image/png;base64,A' x:href='data:image/png;base64,A'/></svg>"
            );
            let original = text.clone();
            let repaired = repair(&text, &limits()).unwrap();
            assert_eq!(text, original, "repair never mutates the original source");
            assert_eq!(repaired.matches(INVALID_IMAGE).count(), 6);
            let resolver = crate::tree::ImageResolver::new(limits());
            resolver.check_document(&repaired).unwrap();
            let usage = resolver.usage();
            assert_eq!(
                (usage.invalid_raster_images, usage.invalid_svg_images),
                (0, 0),
                "shared whole-document scanning cannot invent omissions before resolution"
            );
            assert!(matches!(repair(&repaired, &limits()).unwrap(), Cow::Borrowed(_)), "normalization is idempotent");
        }
    }

    #[test]
    fn entity_expanded_urls_patch_the_owning_attributes_without_rewriting_the_doctype() {
        let text = format!(
            "<!DOCTYPE svg [<!ENTITY bad 'data:image/png;base64,A'>]><svg xmlns='{SVG_NS}'><image href='&bad;'/><image href='data:image/png;base64,&#65;'/></svg>"
        );
        let repaired = repair(&text, &limits()).unwrap();
        assert!(repaired.contains("<!ENTITY bad 'data:image/png;base64,A'>"));
        assert_eq!(repaired.matches(INVALID_IMAGE).count(), 2);
        assert!(!repaired.contains("href='&bad;'"));
        assert!(matches!(repair(&repaired, &limits()).unwrap(), Cow::Borrowed(_)));
    }

    #[test]
    fn valid_data_and_external_urls_are_not_rewritten() {
        for value in ["data:image/png;base64,AA==", "data:image/svg+xml,%3Csvg%2F%3E", "file.png", "#local"] {
            let text = format!("<svg><image href='{value}'/></svg>");
            assert!(matches!(repair(&text, &limits()).unwrap(), Cow::Borrowed(_)));
        }
        let text = "<svg><image href='data:image/png;base64'/></svg>";
        assert!(repair(text, &limits()).unwrap().contains(INVALID_IMAGE));
    }

    #[test]
    fn malformed_non_image_urls_keep_ignored_replacements_even_without_a_comma() {
        for value in [
            "data:application/octet-stream;base64,A",
            "data:application/octet-stream;base64",
            "data:APPLICATION/OCTET-STREAM;BASE64,A",
            "data:text/css;base64,A",
            "data:audio/ogg;base64,A",
            "data:application/octet-stream;base64#fragment,not-the-delimiter",
        ] {
            let text = format!("<svg><image href='{value}'/></svg>");
            let repaired = repair(&text, &limits()).unwrap();
            assert!(repaired.contains(IGNORED_DATA), "{value}: {repaired}");
            assert!(!repaired.contains(INVALID_IMAGE));
            assert!(matches!(repair(&repaired, &limits()).unwrap(), Cow::Borrowed(_)));
            let resolver = crate::tree::ImageResolver::new(limits());
            resolver.check_document(&repaired).unwrap();
            assert_eq!(resolver.usage().invalid_raster_images, 0);
        }
    }

    #[test]
    fn image_and_default_plain_mime_failures_remain_disclosed() {
        for value in [
            "data:IMAGE/PNG;base64,A",
            "data:image/png;base64",
            "data:text/plain;base64,A",
            "data:;base64,A",
            "data:;base64",
            "data:not-a-mime;base64",
        ] {
            let text = format!("<svg><image href='{value}'/></svg>");
            assert!(repair(&text, &limits()).unwrap().contains(INVALID_IMAGE), "{value}");
        }
        let value = "data:application/octet-stream;base64#fragment";
        let mut insufficient = value.len();
        assert!(malformed_data_url_replacement(value, &mut insufficient).is_err(), "the extra MIME-header parse has its own work charge");
        let mut sufficient = value.len() * 2;
        assert_eq!(malformed_data_url_replacement(value, &mut sufficient).unwrap(), Some(IGNORED_DATA));
    }

    #[test]
    fn long_equals_spacing_is_repaired_and_output_growth_stays_bounded() {
        let text = format!("<svg><image href{}='data:image/png;base64,A'/></svg>", " ".repeat(300));
        let repaired = repair(&text, &limits()).unwrap();
        assert!(repaired.contains(INVALID_IMAGE));
        assert!(repaired.contains(&format!("href{}=", " ".repeat(300))));
        let text = "<svg><image href='data:;base64,A'/></svg>";
        assert!(matches!(repair(text, &Limits { bytes: text.len(), ..limits() }), Err(Error::Parse(message)) if message.contains("byte limit")));
    }
}

#[cfg(test)]
#[path = "mime_header_tests.rs"]
mod mime_header_tests;
