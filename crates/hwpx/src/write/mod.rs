//! Writing HWPX: a new package from a document ([`new`]).
//!
//! 본 제품은 한글과컴퓨터의 한글 문서 파일(.hwp) 공개 문서를 참고하여 개발하였습니다.

pub(crate) mod back;
pub(crate) mod marks;
pub(crate) mod new;
pub(crate) mod restyle;

/// What a save did and left out. Only what was left out is told: each line is logged with
/// `log::warn!` when the save ends (see [`SaveReport::log`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SaveReport {
    pub modified: usize,
    pub inserted: usize,
    pub deleted: usize,
    pub unchanged: usize,
    /// Paragraphs given another paragraph shape (alignment, indents, spacing).
    pub restyled: usize,
    /// Tables written again or added.
    pub tables: usize,
    /// Pictures added, replaced, resized or removed.
    pub pictures: usize,
    /// Edits not written, with why ("section 1, paragraph 12: text and objects in one run").
    pub skipped: Vec<String>,
}

impl SaveReport {
    /// Log what was left out, one warning each.
    pub(crate) fn log(&self) {
        for s in &self.skipped {
            log::warn!("HWPX save: {s}");
        }
    }
}

/// `xml` with each prefix of `prefixes` that it uses (`<pfx:`) and its root element doesn't
/// bind given its namespace there. Written parts copy a source part's root as it was; the
/// elements made for them use the usual prefixes, which a source written with others (or with a
/// default namespace) doesn't declare.
pub(crate) fn bind_prefixes(xml: String, prefixes: &[(&str, &str)]) -> String {
    let mut at = 0;
    // the root start tag: past the declaration, instructions and comments
    while let Some(i) = xml.get(at..).and_then(|r| r.find('<')) {
        let rest = xml.get(at + i + 1..).unwrap_or("");
        if rest.starts_with('?') || rest.starts_with('!') {
            at += i + 1;
            continue;
        }
        at += i;
        break;
    }
    let Some(end) = xml.get(at..).and_then(|r| r.find('>')).map(|e| at + e) else { return xml };
    let root = xml.get(at..end).unwrap_or("");
    let name_end = root.find(|c: char| c.is_whitespace() || c == '/').unwrap_or(root.len());
    let mut add = String::new();
    for (pfx, uri) in prefixes {
        if xml.contains(&format!("<{pfx}:")) && !root.contains(&format!("xmlns:{pfx}=")) {
            add.push_str(&format!(" xmlns:{pfx}=\"{uri}\""));
        }
    }
    if add.is_empty() {
        return xml;
    }
    let mut out = xml;
    out.insert_str(at + name_end, &add);
    out
}

/// XML text escaping (`&`, `<`, `>`); characters XML 1.0 can't hold are left out.
pub(crate) fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            // characters XML 1.0 has no place for (U+0000, other C0 controls, U+FFFE, U+FFFF):
            // written, they leave the part unreadable
            '\t' | '\n' | '\r' => o.push(c),
            c if c < ' ' || c == '\u{FFFE}' || c == '\u{FFFF}' => {}
            _ => o.push(c),
        }
    }
    o
}

/// A quoted attribute value: double quotes unless the value has some.
pub(crate) fn attr(s: &str) -> String {
    let mut v = esc(s).replace('\n', "&#10;").replace('\r', "&#13;").replace('\t', "&#9;");
    if v.contains('"') {
        if v.contains('\'') {
            v = v.replace('"', "&quot;");
            format!("\"{v}\"")
        } else {
            format!("'{v}'")
        }
    } else {
        format!("\"{v}\"")
    }
}

/// The caller's font for Hangul text whose own font has none ([`crate::WriteOptions::hangul_font`]).
pub(crate) type HangulFont = Option<fn(&str) -> Option<String>>;

#[cfg(test)]
pub(crate) mod tests;
#[cfg(test)]
mod tests_back;
