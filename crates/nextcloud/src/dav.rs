//! WebDAV pieces: the PROPFIND request body, the multistatus parser and HTTP dates.

use percent_encoding::percent_decode_str;
use quick_xml::NsReader;
use quick_xml::events::Event;
use quick_xml::name::ResolveResult;

use crate::path::check_name;
use crate::{Entry, Error};

const DAV: &[u8] = b"DAV:";
const OC: &[u8] = b"http://owncloud.org/ns";
const NC: &[u8] = b"http://nextcloud.org/ns";
const MAX_DEPTH: usize = 32;
pub(crate) const MAX_ENTRIES: usize = 100_000;

pub(crate) const PROPFIND_BODY: &str = r#"<?xml version="1.0"?><d:propfind xmlns:d="DAV:" xmlns:oc="http://owncloud.org/ns" xmlns:nc="http://nextcloud.org/ns"><d:prop><d:resourcetype/><d:getcontentlength/><d:getetag/><d:getlastmodified/><d:getcontenttype/><oc:fileid/><oc:size/><nc:has-preview/></d:prop></d:propfind>"#;

fn bad(what: &str) -> Error {
    Error::Malformed(what.into())
}

#[derive(Clone, Copy)]
enum Field {
    Href,
    Status,
    Length,
    Etag,
    Modified,
    Type,
    FileId,
    OcSize,
    Preview,
}

/// What one `200` propstat (or all of them) said about a response.
#[derive(Default)]
struct Props {
    is_dir: bool,
    length: Option<u64>,
    oc_size: Option<u64>,
    etag: String,
    modified: Option<i64>,
    content_type: Option<String>,
    fileid: Option<u64>,
    preview: bool,
}

fn is_dav(ns: &ResolveResult<'_>) -> bool {
    matches!(ns, ResolveResult::Bound(n) if n.as_ref() == DAV)
}

fn field(ns: &ResolveResult<'_>, local: &[u8]) -> Option<Field> {
    let ResolveResult::Bound(ns) = ns else { return None };
    Some(match (ns.as_ref(), local) {
        (DAV, b"href") => Field::Href,
        (DAV, b"status") => Field::Status,
        (DAV, b"getcontentlength") => Field::Length,
        (DAV, b"getetag") => Field::Etag,
        (DAV, b"getlastmodified") => Field::Modified,
        (DAV, b"getcontenttype") => Field::Type,
        (OC, b"fileid") => Field::FileId,
        (OC, b"size") => Field::OcSize,
        (NC, b"has-preview") => Field::Preview,
        _ => return None,
    })
}

/// Parses a 207 multistatus into `(decoded href path, entry)` for the entry at `base_path` (decoded, no trailing
/// `/` needed) and its direct children. Children whose name is not a valid path segment are dropped. Only `200`
/// propstats count. `Entry::name` is the last path segment.
pub(crate) fn parse_multistatus(xml: &[u8], base_path: &str) -> Result<Vec<(String, Entry)>, Error> {
    let base = base_path.strip_suffix('/').unwrap_or(base_path);
    let mut reader = NsReader::from_reader(xml);
    let (mut depth, mut seen, mut root_ok) = (0usize, 0usize, false);
    let mut current: Option<Field> = None;
    let (mut text, mut href, mut status) = (String::new(), String::new(), String::new());
    let (mut props, mut propstat, mut in_collection) = (Props::default(), Props::default(), false);
    let mut out = Vec::new();
    loop {
        let (ns, event) = reader.read_resolved_event().map_err(|e| Error::Malformed(format!("bad XML: {e}")))?;
        match event {
            Event::Empty(e) => {
                if is_dav(&ns) && e.local_name().as_ref() == b"collection" {
                    in_collection = true;
                }
            }
            Event::Start(e) => {
                let local = e.local_name();
                let local = local.as_ref();
                if depth == 0 && !(is_dav(&ns) && local == b"multistatus") {
                    return Err(bad("not a multistatus"));
                }
                root_ok = true;
                match local {
                    b"collection" if is_dav(&ns) => in_collection = true,
                    b"response" if is_dav(&ns) => {
                        props = Props::default();
                        href.clear();
                        seen += 1;
                        if seen > MAX_ENTRIES {
                            return Err(Error::TooLarge);
                        }
                    }
                    b"propstat" if is_dav(&ns) => {
                        propstat = Props::default();
                        status.clear();
                    }
                    _ => {}
                }
                current = field(&ns, local);
                text.clear();
                depth += 1;
                if depth > MAX_DEPTH {
                    return Err(bad("XML nested too deeply"));
                }
            }
            Event::Text(t) => {
                if current.is_some() {
                    text.push_str(&t.xml_content(quick_xml::XmlVersion::Explicit1_0).map_err(|e| Error::Malformed(e.to_string()))?);
                }
            }
            Event::GeneralRef(r) => {
                if current.is_some() {
                    match r.resolve_char_ref() {
                        Ok(Some(c)) => text.push(c),
                        _ => match r.as_ref() {
                            b"amp" => text.push('&'),
                            b"lt" => text.push('<'),
                            b"gt" => text.push('>'),
                            b"quot" => text.push('"'),
                            b"apos" => text.push('\''),
                            _ => return Err(bad("unknown entity")),
                        },
                    }
                }
            }
            Event::End(e) => {
                depth = depth.saturating_sub(1);
                let local = e.local_name();
                let local = local.as_ref();
                let is_dav = is_dav(&ns);
                if let Some(f) = current.take() {
                    let t = text.trim();
                    match f {
                        Field::Href => href = t.to_string(),
                        Field::Status => status = t.to_string(),
                        Field::Length => propstat.length = t.parse().ok(),
                        Field::Etag => propstat.etag = t.strip_prefix("W/").unwrap_or(t).trim_matches('"').to_string(),
                        Field::Modified => propstat.modified = http_date(t),
                        Field::Type => propstat.content_type = Some(t.to_string()).filter(|t| !t.is_empty()),
                        Field::FileId => propstat.fileid = t.parse().ok(),
                        Field::OcSize => propstat.oc_size = t.parse().ok(),
                        Field::Preview => propstat.preview = t == "true",
                    }
                }
                if is_dav && local == b"resourcetype" {
                    propstat.is_dir = in_collection;
                    in_collection = false;
                }
                if is_dav && local == b"propstat" && status.split_whitespace().nth(1) == Some("200") {
                    props = std::mem::take(&mut propstat);
                }
                if is_dav
                    && local == b"response"
                    && let Some(item) = finish(&href, &props, base)
                {
                    out.push(item);
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if !root_ok {
        return Err(bad("not a multistatus"));
    }
    Ok(out)
}

/// Builds the entry if the href is `base` itself or a direct child of it with a valid name.
fn finish(href: &str, props: &Props, base: &str) -> Option<(String, Entry)> {
    // Absolute URLs are reduced to their path.
    let raw = match href.split_once("://") {
        Some((_, rest)) => format!("/{}", rest.split_once('/')?.1),
        None => href.to_string(),
    };
    let decoded = percent_decode_str(&raw).decode_utf8().ok()?;
    let path = decoded.strip_suffix('/').unwrap_or(&decoded);
    if path != base {
        check_name(path.strip_prefix(base)?.strip_prefix('/')?).ok()?;
    }
    let entry = Entry {
        name: path.rsplit('/').next().unwrap_or_default().to_string(),
        is_dir: props.is_dir,
        size: if props.is_dir { props.oc_size } else { props.length }.unwrap_or(0),
        etag: props.etag.clone(),
        fileid: props.fileid,
        mtime: props.modified,
        content_type: props.content_type.clone(),
        has_preview: props.preview,
    };
    Some((path.to_string(), entry))
}

/// RFC 1123 (`Sun, 06 Nov 1994 08:49:37 GMT`) to Unix seconds; anything else is `None`.
pub(crate) fn http_date(s: &str) -> Option<i64> {
    let mut parts = s.split_whitespace();
    let (day, d, mon, y, time, zone) = (parts.next()?, parts.next()?, parts.next()?, parts.next()?, parts.next()?, parts.next()?);
    if !day.ends_with(',') || zone != "GMT" || parts.next().is_some() {
        return None;
    }
    let month = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"].iter().position(|m| *m == mon)? as i64 + 1;
    let (d, y) = (d.parse::<i64>().ok().filter(|d| (1..=31).contains(d))?, y.parse::<i64>().ok().filter(|y| (1..=9999).contains(y))?);
    let mut hms = time.split(':').map(|p| p.parse::<i64>().ok().filter(|v| (0..=60).contains(v)));
    let (h, m, sec) = (hms.next()??, hms.next()??, hms.next()??);
    if hms.next().is_some() || h > 23 {
        return None;
    }
    // Days from civil (Howard Hinnant).
    let y = if month <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + h * 3600 + m * 60 + sec)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn depth_cap() {
        let open: String = (0..33).map(|_| "<d:a>").collect();
        let xml = format!(r#"<d:multistatus xmlns:d="DAV:">{open}"#);
        assert!(matches!(parse_multistatus(xml.as_bytes(), "/"), Err(Error::Malformed(_))));
    }

    #[test]
    fn http_date_parses() {
        assert_eq!(http_date("Sun, 06 Nov 1994 08:49:37 GMT"), Some(784_111_777));
        assert_eq!(http_date("Sunday, 06-Nov-94 08:49:37 GMT"), None);
        assert_eq!(http_date("garbage"), None);
    }

    #[test]
    fn etag_quotes_stripped() {
        let xml = r#"<D:multistatus xmlns:D="DAV:"><D:response><D:href>/dav/a%20b</D:href><D:propstat><D:prop>
            <D:getetag>&quot;abc&quot;</D:getetag><D:getcontentlength>5</D:getcontentlength></D:prop>
            <D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response></D:multistatus>"#;
        let entries = parse_multistatus(xml.as_bytes(), "/dav").unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0, "/dav/a b");
        assert_eq!(entries[0].1.etag, "abc");
        assert_eq!(entries[0].1.size, 5);
    }
}
