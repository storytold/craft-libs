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

/// XML text escaping (`&`, `<`, `>`).
pub(crate) fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
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
