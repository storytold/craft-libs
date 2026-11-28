//! Small text helpers of the reader: splitting, escapes, field commands, link targets.
//!
//! 본 제품은 한글과컴퓨터의 한글 문서 파일(.hwp) 공개 문서를 참고하여 개발하였습니다.

use crate::xml::Element;

/// Python's floor division for the few places that divide possibly negative numbers.
pub(crate) fn floor_div(a: i64, b: i64) -> i64 {
    if b == 0 { 0 } else { a.div_euclid(b) - if b < 0 && a.rem_euclid(b) != 0 { 1 } else { 0 } }
}

/// A Windows FILETIME (100 ns ticks since 1601-01-01 UTC) as ISO 8601 (`2025-01-02T03:04:05Z`);
/// `None` for 0 or a time before 1970 or after 9999.
pub(crate) fn iso_from_filetime(ft: u64) -> Option<String> {
    let secs = i64::try_from(ft / 10_000_000).ok()?.checked_sub(11_644_473_600)?;
    if secs <= 0 {
        return None;
    }
    let t = chrono::DateTime::from_timestamp(secs, 0)?;
    (t.format("%Y").to_string().len() == 4).then(|| t.format("%Y-%m-%dT%H:%M:%SZ").to_string())
}

/// `s` in pieces that are all spaces (U+0020) or have none.
pub(crate) fn split_spaces(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut prev: Option<bool> = None;
    for (i, c) in s.char_indices() {
        let space = c == ' ';
        if prev.is_some_and(|p| p != space) {
            out.push(s.get(start..i).unwrap_or(""));
            start = i;
        }
        prev = Some(space);
    }
    if start < s.len() {
        out.push(s.get(start..).unwrap_or(""));
    }
    out
}

pub(crate) fn no_spaces(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

/// Runs of white space -> one `with`.
pub(crate) fn squeeze_spaces(s: &str, with: &str) -> String {
    let mut out = String::new();
    let mut in_space = false;
    for c in s.chars() {
        if c.is_whitespace() {
            if !in_space {
                out.push_str(with);
            }
            in_space = true;
        } else {
            out.push(c);
            in_space = false;
        }
    }
    out
}

/// `s` split at each `sep` not behind a backslash (the escapes stay; see [`unescape_backslash`]).
pub(crate) fn split_unescaped(s: &str, sep: char) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut start, mut escaped) = (0, false);
    for (i, c) in s.char_indices() {
        if escaped {
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == sep {
            out.push(s.get(start..i).unwrap_or(""));
            start = i + c.len_utf8();
        }
    }
    out.push(s.get(start..).unwrap_or(""));
    out
}

/// A field command's text with its escapes undone: a backslash before a separator (`;`, `/`) or
/// another backslash. Any other backslash is the text's own, as in a Windows path written as it
/// is ("E:\Work\other.hwp" stays so; "E:\\Work\\other.hwp" becomes it).
pub(crate) fn unescape_backslash(s: &str) -> String {
    let mut out = String::new();
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        match (c, it.peek()) {
            ('\\', Some(&n)) if matches!(n, ';' | '/' | '\\') => {
                out.push(n);
                it.next();
            }
            _ => out.push(c),
        }
    }
    out
}

/// Unicode circled number for 1..50, else the number in parentheses.
pub(crate) fn circled(n: i64) -> String {
    let c = match n {
        1..=20 => char::from_u32(0x2460 + (n - 1) as u32),
        21..=35 => char::from_u32(0x3251 + (n - 21) as u32),
        36..=50 => char::from_u32(0x32B1 + (n - 36) as u32),
        _ => None,
    };
    c.map(String::from).unwrap_or_else(|| format!("({n})"))
}

/// Percent-encode as Python's `urllib.parse.quote(s, safe)`.
fn quote(s: &str, safe: &str) -> String {
    let mut o = String::new();
    for b in s.bytes() {
        let c = b as char;
        if c.is_ascii_alphanumeric() || "_.-~".contains(c) || safe.contains(c) {
            o.push(c);
        } else {
            o.push_str(&format!("%{b:02X}"));
        }
    }
    o
}

/// The guide text of an input field (Hancom: 누름틀), shown only while it is empty.
pub(crate) fn click_here_guide(el: &Element) -> String {
    if let Some(sp) = el.iter("hp:stringParam").find(|sp| sp.get("name") == Some("Direction")) {
        return sp.text.trim().to_string();
    }
    const KEY: &str = "Direction:wstring:";
    for sp in el.iter("hp:stringParam") {
        let s = sp.text.as_str();
        let mut from = 0;
        while let Some(i) = s.get(from..).and_then(|r| r.find(KEY)) {
            let ds = from + i + KEY.len();
            from = from + i + 1;
            let rest = s.get(ds..).unwrap_or("");
            let n = rest.bytes().take_while(u8::is_ascii_digit).count();
            if n > 0 && rest.as_bytes().get(n) == Some(&b':') {
                let count: usize = rest.get(..n).and_then(|d| d.parse().ok()).unwrap_or(0);
                let text = rest.get(n + 1..).unwrap_or("");
                return text.chars().take(count).collect::<String>().trim().to_string();
            }
        }
    }
    String::new()
}

fn safe_url(url: &str) -> bool {
    let l = url.to_ascii_lowercase();
    ["http:", "https:", "mailto:", "ftp:"].iter().any(|p| l.starts_with(p))
}

/// A link to another document (by extension).
fn doc_link(url: &str) -> bool {
    const EXTS: &[&str] = &["hwp", "hwpx", "hwt", "doc", "docx", "odt", "rtf", "pdf", "txt", "xls", "xlsx", "ppt", "pptx", "htm", "html"];
    let l = url.to_ascii_lowercase();
    l.rsplit_once('.').is_some_and(|(_, e)| EXTS.contains(&e))
}

/// Where a hyperlink field points: a web or mail address, or another document as a relative or
/// `file:` link. `None` for anything else (a script, a bookmark-only target).
pub(crate) fn hyperlink_url(el: &Element) -> Option<String> {
    for sp in el.iter("hp:stringParam") {
        if !matches!(sp.get("name"), Some("Command" | "Path")) {
            continue;
        }
        // up to the first ';' not escaped with a backslash
        let raw = &sp.text;
        let mut first = String::new();
        let mut prev_backslash = false;
        for ch in raw.chars() {
            if ch == ';' && !prev_backslash {
                break;
            }
            prev_backslash = ch == '\\';
            first.push(ch);
        }
        let url = unescape_backslash(&first).trim().to_string();
        if safe_url(&url) && !url.contains('"') {
            return Some(url);
        }
        if doc_link(&url) && !url.contains('"') {
            // A link to another document (".\other.hwp", "E:\Work\other.hwp").
            let path = url.replace('\\', "/");
            let b = path.as_bytes();
            if b.len() >= 3 && b.first().is_some_and(u8::is_ascii_alphabetic) && b.get(1..3) == Some(b":/") {
                return Some(format!("file:///{}", quote(&path, "/:")));
            }
            return Some(quote(path.strip_prefix("./").unwrap_or(&path), "/"));
        }
    }
    None
}

/// A binary item's file extension ("png" when it has none).
pub(crate) fn ext_of(name: &str) -> String {
    let base = name.rsplit('/').next().unwrap_or(name);
    match base.rsplit_once('.') {
        Some((stem, e)) if !stem.is_empty() && !e.is_empty() => e.to_string(),
        _ => "png".to_string(),
    }
}
