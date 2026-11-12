//! `multipart/related` (RFC 2387) and MHTML web archives (RFC 2557): which part is the root,
//! and which part a reference in it (`cid:…`, a location) names.

use std::collections::HashMap;

use crate::part::Part;

/// The part that the `start` parameter of `root`'s `Content-Type` names by its `Content-ID`
/// (RFC 2387 §3.2), looked for among all the leaves of `root`. `None` when there is no `start`
/// or no part has that id; then the root of a `multipart/related` is its first part, or
/// whichever the caller's format says (MHTML: the first HTML part).
pub fn start_part(root: &Part) -> Option<&Part> {
    let start = root.content_type.get("start")?.trim().trim_start_matches('<').trim_end_matches('>').trim();
    root.leaves().into_iter().find(|p| p.content_id() == Some(start))
}

/// The leaves of a message by `Content-ID` and by `Content-Location`, for the references of a
/// page to its pictures. The first part of each id and of each location wins. Locations are
/// compared without regard to ASCII letter case.
#[derive(Clone, Debug, Default)]
pub struct Resources<'a> {
    by_id: HashMap<&'a str, &'a Part>,
    by_location: HashMap<String, &'a Part>,
}

impl<'a> Resources<'a> {
    /// The index of every leaf of `root`.
    pub fn of(root: &'a Part) -> Self {
        let mut resources = Resources::default();
        for part in root.leaves() {
            if let Some(id) = part.content_id() {
                resources.by_id.entry(id).or_insert(part);
            }
            if let Some(location) = part.content_location() {
                resources.by_location.entry(location.to_ascii_lowercase()).or_insert(part);
            }
        }
        resources
    }

    /// The part with this `Content-ID` (without angle brackets and without `cid:`).
    pub fn by_content_id(&self, id: &str) -> Option<&'a Part> {
        self.by_id.get(id).copied()
    }

    /// The part with this `Content-Location`, as written.
    pub fn by_location(&self, location: &str) -> Option<&'a Part> {
        self.by_location.get(&location.to_ascii_lowercase()).copied()
    }

    /// The part a reference in a page names: `cid:id` by `Content-ID`; anything else by
    /// `Content-Location`, as written, or resolved against `base`, the page's own location
    /// ([`resolve_location`]; empty when it has none). Nothing is fetched.
    pub fn find(&self, reference: &str, base: &str) -> Option<&'a Part> {
        let reference = reference.trim();
        match reference.get(..4).filter(|s| s.eq_ignore_ascii_case("cid:")).and(reference.get(4..)) {
            Some(id) => self.by_content_id(id),
            None => self.by_location(reference).or_else(|| self.by_location(&resolve_location(base, reference))),
        }
    }
}

/// `reference` resolved against the page's location `base`, as RFC 2557 §5 asks (RFC 3986
/// §5.2.2): a reference with a scheme stays as it is, `//host/…` takes the base's scheme, `/…`
/// its scheme and host, an empty path its path and query, anything else is merged with its path
/// (§5.2.3); `.` and `..` segments go (§5.2.4) and fragments are dropped.
pub fn resolve_location(base: &str, reference: &str) -> String {
    let (b, r) = (UriParts::of(base), UriParts::of(reference));
    let t = if r.scheme.is_some() {
        UriParts { path: remove_dots(&r.path), ..r }
    } else if r.authority.is_some() {
        UriParts { scheme: b.scheme, path: remove_dots(&r.path), ..r }
    } else if r.path.is_empty() {
        UriParts { query: r.query.or(b.query), ..b }
    } else if r.path.starts_with('/') {
        UriParts { path: remove_dots(&r.path), query: r.query, ..b }
    } else {
        let merged = if b.authority.is_some() && b.path.is_empty() {
            format!("/{}", r.path)
        } else {
            format!("{}{}", b.path.rfind('/').and_then(|i| b.path.get(..=i)).unwrap_or(""), r.path)
        };
        UriParts { path: remove_dots(&merged), query: r.query, ..b }
    };
    t.to_string()
}

/// A URI's parts (RFC 3986 §3): `scheme:`, `//authority`, path and `?query`; the fragment left out.
struct UriParts<'a> {
    scheme: Option<&'a str>,
    authority: Option<&'a str>,
    path: String,
    query: Option<&'a str>,
}

impl<'a> UriParts<'a> {
    fn of(uri: &'a str) -> Self {
        let uri = uri.split('#').next().unwrap_or("");
        // A one-letter scheme is a Windows drive (`C:/…`), not a scheme.
        let scheme = uri.split_once(':').map(|(k, _)| k).filter(|k| {
            k.len() > 1 && k.starts_with(|c: char| c.is_ascii_alphabetic()) && k.chars().all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))
        });
        let rest = scheme.and_then(|k| uri.get(k.len().saturating_add(1)..)).unwrap_or(uri);
        let (authority, rest) = match rest.strip_prefix("//") {
            Some(r) => {
                let end = r.find(['/', '?']).unwrap_or(r.len());
                (r.get(..end), r.get(end..).unwrap_or(""))
            }
            None => (None, rest),
        };
        let (path, query) = match rest.split_once('?') {
            Some((p, q)) => (p, Some(q)),
            None => (rest, None),
        };
        UriParts { scheme, authority, path: path.to_string(), query }
    }
}

impl std::fmt::Display for UriParts<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(s) = self.scheme {
            write!(f, "{s}:")?;
        }
        if let Some(a) = self.authority {
            write!(f, "//{a}")?;
        }
        f.write_str(&self.path)?;
        if let Some(q) = self.query {
            write!(f, "?{q}")?;
        }
        Ok(())
    }
}

/// `path` without its `.` and `..` segments (RFC 3986 §5.2.4); `..` stops at the root.
fn remove_dots(path: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    let mut segments = path.split('/').peekable();
    while let Some(seg) = segments.next() {
        match seg {
            "." => {}
            ".." => {
                if out.len() > 1 || out.first().is_some_and(|f| !f.is_empty()) {
                    out.pop();
                }
            }
            s => out.push(s),
        }
        if segments.peek().is_none() && matches!(seg, "." | "..") {
            out.push("");
        }
    }
    out.join("/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Limits, parse};

    const PNG_1X1: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";

    fn page_and_logo(page: &str) -> crate::Message {
        let m = format!(
            "MIME-Version: 1.0\r\nContent-Type: multipart/related; boundary=b\r\n\r\n\
             --b\r\nContent-Location: {page}\r\nContent-Type: text/html\r\n\r\n<p>x</p>\r\n\
             --b\r\nContent-Location: https://example.test/images/logo.png\r\nContent-Transfer-Encoding: base64\r\nContent-Type: image/png\r\n\r\n{PNG_1X1}\r\n--b--\r\n"
        );
        parse(m.as_bytes(), &Limits::default())
    }

    #[test]
    fn references_relative_to_the_page_resolve_as_uris() {
        // The page in `reports/`, its picture in `images/` (RFC 2557 §5).
        let m = page_and_logo("https://example.test/reports/page.html");
        let resources = Resources::of(&m.root);
        let base = "https://example.test/reports/page.html";
        for src in [
            "../images/logo.png",
            "/images/logo.png",
            "./../images/./logo.png",
            "//example.test/images/logo.png",
            "https://example.test/images/logo.png",
            " HTTPS://EXAMPLE.TEST/images/LOGO.png ",
            // Nothing past the root: `..` there stays at the root.
            "../../../images/logo.png",
        ] {
            let part = resources.find(src, base);
            assert_eq!(part.map(|p| p.media_type()), Some("image/png"), "{src}");
            assert!(part.is_some_and(|p| p.data().starts_with(b"\x89PNG")));
        }
        assert!(resources.find("images/logo.png", base).is_none());
        assert!(resources.find("cid:logo", base).is_none());
        assert!(resources.by_location("https://example.test/reports/page.html").is_some());
    }

    #[test]
    fn locations_resolve_as_rfc_3986_says() {
        assert_eq!(resolve_location("https://h/a/b/c.html", "../d/./e.png#f"), "https://h/a/d/e.png");
        assert_eq!(resolve_location("file:///C:/x/page.htm", "page_files/dot.png"), "file:///C:/x/page_files/dot.png");
        assert_eq!(resolve_location("", "dot.png"), "dot.png");
        // A host with no path, and a base whose query holds slashes (§5.2.3: the path merges,
        // not the query; an authority with an empty path gets its `/`).
        assert_eq!(resolve_location("https://example.test", "logo.png"), "https://example.test/logo.png");
        assert_eq!(resolve_location("https://example.test/page.html?redirect=/other/", "logo.png"), "https://example.test/logo.png");
        assert_eq!(resolve_location("https://h/a/p.html?q", "?r"), "https://h/a/p.html?r");
        assert_eq!(resolve_location("https://h/a/p.html?q", "x.png?v=1"), "https://h/a/x.png?v=1");
        assert_eq!(resolve_location("https://h/a/p.html", "mailto:x@y"), "mailto:x@y");
        assert_eq!(resolve_location("https://h/a/p.html", ".."), "https://h/");
        assert_eq!(resolve_location("https://h/a/p.html", ""), "https://h/a/p.html");
    }

    #[test]
    fn the_start_part_is_the_root_and_cid_names_pictures() {
        let m = format!(
            "MIME-Version: 1.0\nContent-Type: multipart/related; type=\"text/html\"; start=\"<page>\"; boundary=b\n\n\
             --b\nContent-Type: text/html\n\n<p>Not this one</p>\n\
             --b\nContent-ID: <page>\nContent-Type: text/html\n\n<p>This one</p><img src=\"cid:dot\">\n\
             --b\nContent-ID: <dot>\nContent-Transfer-Encoding: base64\nContent-Type: image/png\n\n{PNG_1X1}\n\
             --b\nContent-ID: <dot>\nContent-Type: text/plain\n\nA second part of the same id.\n--b--\n"
        );
        let m = parse(m.as_bytes(), &Limits::default());
        assert_eq!(start_part(&m.root).map(Part::text).as_deref(), Some("<p>This one</p><img src=\"cid:dot\">"));
        let resources = Resources::of(&m.root);
        assert_eq!(resources.find("CID:dot", "").map(|p| p.media_type()), Some("image/png"));
        assert_eq!(resources.by_content_id("page").map(|p| p.media_type()), Some("text/html"));
        assert!(resources.by_content_id("<dot>").is_none() && resources.find("cid:", "").is_none());
        // No `start`, or one that names nothing.
        assert!(start_part(&page_and_logo("https://example.test/p.html").root).is_none());
        let m = parse(b"Content-Type: multipart/related; start=\"<gone>\"; boundary=b\n\n--b\nContent-ID: <here>\n\nx\n--b--\n", &Limits::default());
        assert!(start_part(&m.root).is_none());
    }

    proptest::proptest! {
        #[test]
        fn any_reference_resolves(base in "[a-c:/?#.]{0,24}", reference in "[a-c:/?#.]{0,24}") {
            let resolved = resolve_location(&base, &reference);
            proptest::prop_assert!(resolved.len() <= base.len() + reference.len() + 1);
        }

        #[test]
        fn any_text_resolves(base in proptest::prelude::any::<String>(), reference in proptest::prelude::any::<String>()) {
            let _ = resolve_location(&base, &reference);
            let _ = Resources::default().find(&reference, &base);
        }
    }
}
