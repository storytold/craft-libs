//! Relationships (`_rels/*.rels`) and content types (`[Content_Types].xml`).

use std::collections::HashMap;

use craft_xml::{El, Local, Options};

use crate::name::{dir_of, normalize, percent_decode, resolve};

/// Is relationship type `t` the type `ty`? `ty` is a full type URI or its last segment
/// (`image`). Only the last segment is compared when the URIs differ, so the transitional,
/// strict and vendor spellings of one type are the same type.
pub fn type_is(t: &str, ty: &str) -> bool {
    fn tail(s: &str) -> &str {
        s.rsplit('/').next().unwrap_or(s)
    }
    t == ty || tail(t) == tail(ty)
}

/// One relationship.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Rel {
    pub id: String,
    /// The full relationship type URI.
    pub ty: String,
    /// `Target` as the relationships part writes it (a relative reference or a URL).
    pub target_raw: String,
    /// The part name for an internal target, spelled as the package stores it when the package
    /// has the part; the URL for an external one.
    pub target: String,
    pub external: bool,
}

impl Rel {
    /// The last segment of the type (`slide`, `image`, `styles`…): the same for the
    /// transitional and strict namespaces.
    pub fn kind(&self) -> &str {
        self.ty.rsplit('/').next().unwrap_or(&self.ty)
    }
}

/// The relationships of one part, in document order.
#[derive(Clone, Debug, Default)]
pub struct Rels {
    pub list: Vec<Rel>,
    /// Each id's first relationship in `list`, so a lookup doesn't scan every relationship.
    ids: HashMap<String, usize>,
}

impl Rels {
    /// The relationships of `part` from the bytes of its relationships part. Internal targets
    /// resolve against `part`'s folder, with `%hh` escapes decoded. Unreadable XML gives what
    /// was read before the damage, or nothing.
    pub fn parse(bytes: &[u8], part: &str) -> Rels {
        Rels::parse_in(bytes, part, |_| None)
    }

    /// [`Rels::parse`] for a package: `stored` gives the stored spelling of a part name, or
    /// `None` when the package has no such part. A target is looked up as written, then with
    /// its `%hh` escapes decoded.
    pub fn parse_in(bytes: &[u8], part: &str, stored: impl Fn(&str) -> Option<String>) -> Rels {
        match Options::local().parse(bytes) {
            Ok(root) => Rels::from_el(&root, part, stored),
            Err(e) => {
                log::warn!("opc: ignoring unreadable relationships of {part}: {e}");
                Rels::default()
            }
        }
    }

    fn from_el(root: &El, part: &str, stored: impl Fn(&str) -> Option<String>) -> Rels {
        let dir = dir_of(part);
        let mut rels = Rels::default();
        for r in root.children(Local("Relationship")) {
            let (Some(id), Some(target)) = (r.attr("Id"), r.attr("Target")) else { continue };
            let external = r.attr("TargetMode").is_some_and(|m| m.eq_ignore_ascii_case("External"));
            let resolved = if external {
                target.to_string()
            } else {
                let path = resolve(dir, target);
                stored(&path).unwrap_or_else(|| {
                    let decoded = percent_decode(&path);
                    stored(&decoded).unwrap_or(decoded)
                })
            };
            rels.ids.entry(id.to_string()).or_insert(rels.list.len());
            rels.list.push(Rel {
                id: id.to_string(),
                ty: r.attr("Type").unwrap_or("").to_string(),
                target_raw: target.to_string(),
                target: resolved,
                external,
            });
        }
        rels
    }

    /// The relationship with this id: the first, when a damaged part repeats one.
    pub fn by_id(&self, id: &str) -> Option<&Rel> {
        self.ids.get(id).and_then(|&i| self.list.get(i))
    }
    /// The target of the relationship with this id.
    pub fn target(&self, id: &str) -> Option<&str> {
        self.by_id(id).map(|r| r.target.as_str())
    }
    /// The first relationship of type `ty` ([`type_is`]: a full URI or a last segment).
    pub fn first(&self, ty: &str) -> Option<&Rel> {
        self.list.iter().find(|r| type_is(&r.ty, ty))
    }
    /// Every relationship of type `ty`.
    pub fn all<'a>(&'a self, ty: &'a str) -> impl Iterator<Item = &'a Rel> + 'a {
        self.list.iter().filter(move |r| type_is(&r.ty, ty))
    }
}

/// Every internal part name the relationships part `name` (with bytes `bytes`) targets, as
/// written and with escapes decoded.
pub(crate) fn targets_of(name: &str, bytes: &[u8], mut each: impl FnMut(String)) {
    // `dir/_rels/file.rels` relates `dir/file`, so its targets resolve against `dir`.
    let lower = name.to_ascii_lowercase();
    let Some(at) = lower.rfind("_rels/").filter(|_| lower.ends_with(".rels")) else { return };
    let dir = name.get(..at).unwrap_or("").trim_end_matches('/');
    let Ok(root) = Options::local().parse(bytes) else { return };
    for r in root.children(Local("Relationship")) {
        let external = r.attr("TargetMode").is_some_and(|m| m.eq_ignore_ascii_case("External"));
        if let (Some(target), false) = (r.attr("Target"), external) {
            let path = resolve(dir, target);
            let decoded = percent_decode(&path);
            if decoded != path {
                each(decoded);
            }
            each(path);
        }
    }
}

/// `[Content_Types].xml`: Override by part name, then Default by extension (both
/// case-insensitive). The first entry for a name or an extension wins.
#[derive(Clone, Debug, Default)]
pub struct ContentTypes {
    overrides: HashMap<String, String>,
    defaults: HashMap<String, String>,
}

impl ContentTypes {
    /// Unreadable XML gives no content types.
    pub fn parse(bytes: &[u8]) -> ContentTypes {
        let mut ct = ContentTypes::default();
        let root = match Options::local().parse(bytes) {
            Ok(root) => root,
            Err(e) => {
                log::warn!("opc: ignoring unreadable [Content_Types].xml: {e}");
                return ct;
            }
        };
        for e in root.elements() {
            let Some(t) = e.attr("ContentType") else { continue };
            let (map, key) = match (e.name.as_str(), e.attr("PartName"), e.attr("Extension")) {
                ("Override", Some(part), _) => (&mut ct.overrides, normalize(part)),
                ("Default", _, Some(ext)) => (&mut ct.defaults, ext.to_ascii_lowercase()),
                _ => continue,
            };
            map.entry(key).or_insert_with(|| t.to_string());
        }
        ct
    }

    /// The content type of `part`.
    pub fn of(&self, part: &str) -> Option<&str> {
        let part = normalize(part);
        if let Some(t) = self.overrides.get(&part) {
            return Some(t);
        }
        let ext = part.rsplit_once('.').map(|(_, e)| e)?;
        self.defaults.get(ext).map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NS_RELATIONSHIPS, RT_BASE};

    #[test]
    fn types_compare_by_last_segment() {
        let strict = "http://purl.oclc.org/ooxml/officeDocument/relationships/image";
        assert!(type_is(strict, &format!("{RT_BASE}image")));
        assert!(type_is(strict, "image"));
        assert!(type_is("image", "image"));
        assert!(!type_is(strict, "images"));
        assert!(!type_is(&format!("{RT_BASE}notimage"), "image"));
        assert!(!type_is("", "image"));
    }

    #[test]
    fn many_relationships_are_found_by_index_and_first_id_wins() {
        let n = 50_000;
        let mut x = format!(r#"<Relationships xmlns="{NS_RELATIONSHIPS}">"#);
        for i in 0..n {
            x.push_str(&format!(r#"<Relationship Id="rId{i}" Type="{RT_BASE}image" Target="../media/image{i}.png"/>"#));
        }
        x.push_str(&format!(r#"<Relationship Id="rId0" Type="{RT_BASE}image" Target="../media/dup.png"/></Relationships>"#));
        let rels = Rels::parse(x.as_bytes(), "ppt/slides/slide1.xml");
        assert_eq!(rels.target("rId0"), Some("ppt/media/image0.png"), "the first of a repeated id wins");
        assert!((0..n).all(|i| rels.target(&format!("rId{i}")) == Some(&*format!("ppt/media/image{i}.png"))));
        assert!(rels.by_id(&format!("rId{n}")).is_none());
        assert_eq!(rels.all("image").count(), n + 1);
    }

    #[test]
    fn targets_keep_what_was_written_and_resolve() {
        let x = format!(
            r#"<r:Relationships xmlns:r="{NS_RELATIONSHIPS}"><r:Relationship Id="a" Type="{RT_BASE}image" Target="../media/a%20b.png#x"/><r:Relationship Id="b" Type="{RT_BASE}hyperlink" Target="https://example.com/a%20b" TargetMode="external"/><r:Relationship Type="t" Target="no-id"/></r:Relationships>"#
        );
        let rels = Rels::parse(x.as_bytes(), "/ppt/slides/slide1.xml");
        assert_eq!(rels.list.len(), 2, "a relationship without an id is left out");
        let a = rels.by_id("a").unwrap();
        assert_eq!((a.target.as_str(), a.target_raw.as_str(), a.kind(), a.external), ("ppt/media/a b.png", "../media/a%20b.png#x", "image", false));
        let b = rels.first("hyperlink").unwrap();
        assert_eq!((b.target.as_str(), b.external), ("https://example.com/a%20b", true), "an external target is not resolved");
        assert!(Rels::parse(b"<Relationships><Relationship", "a.xml").list.is_empty());
    }

    #[test]
    fn a_target_is_looked_up_as_written_before_it_is_decoded() {
        let x = r#"<Relationships><Relationship Id="a" Type="image" Target="a%20b.png"/><Relationship Id="b" Type="image" Target="c%20d.png"/></Relationships>"#;
        let stored = |n: &str| ["word/a%20b.png", "word/C d.png"].iter().find(|s| s.eq_ignore_ascii_case(n)).map(|s| s.to_string());
        let rels = Rels::parse_in(x.as_bytes(), "word/document.xml", stored);
        assert_eq!(rels.target("a"), Some("word/a%20b.png"));
        assert_eq!(rels.target("b"), Some("word/C d.png"), "decoded, in the stored spelling");
    }

    #[test]
    fn content_types_by_override_then_extension() {
        let ct = ContentTypes::parse(
            br#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="PNG" ContentType="image/png"/><Default Extension="png" ContentType="image/other"/><Override PartName="/Word/Document.xml" ContentType="main"/><Override PartName="/word/document.xml" ContentType="second"/><Default ContentType="none"/></Types>"#,
        );
        assert_eq!(ct.of("word/document.xml"), Some("main"), "the first override wins, in any case");
        assert_eq!(ct.of("/word/media/A.PNG"), Some("image/png"));
        assert_eq!(ct.of("word/styles.xml"), None);
        assert_eq!(ct.of("noextension"), None);
        assert_eq!(ContentTypes::parse(b"<Types><Default").of("a.png"), None);
    }
}
