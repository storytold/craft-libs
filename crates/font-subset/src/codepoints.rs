//! The canonical codepoint string used in on-demand URLs.
//!
//! Rules (shared with the client through `tests/data/preview-vectors.json`):
//! drop C0 controls, DEL, C1 controls and surrogates; deduplicate and sort;
//! write lowercase hex without leading zeros; write a run of two or more
//! consecutive codepoints as `first-last`; join with commas.

use std::collections::BTreeSet;
use std::fmt::Write;

/// The most codepoints one on-demand request may carry.
pub const MAX_CODEPOINTS: usize = 256;

/// Why [`parse`] refused a codepoint list.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum ParseError {
    /// The list is not in canonical form.
    #[error("codepoint list is not in canonical form")]
    NotCanonical,
    /// The list has more than [`MAX_CODEPOINTS`] codepoints.
    #[error("more than {MAX_CODEPOINTS} codepoints")]
    TooMany,
}

/// Whether a codepoint may appear in a canonical list.
pub fn is_allowed(cp: u32) -> bool {
    !(cp < 0x20 || (0x7f..=0x9f).contains(&cp) || (0xd800..=0xdfff).contains(&cp) || cp > 0x10ffff)
}

/// The allowed codepoints of `text`, sorted and deduplicated.
pub fn of_text(text: &str) -> BTreeSet<u32> {
    text.chars().map(u32::from).filter(|&cp| is_allowed(cp)).collect()
}

/// Writes a sorted codepoint set in canonical form.
pub fn format(set: &BTreeSet<u32>) -> String {
    let mut out = String::new();
    let mut iter = set.iter().copied().peekable();
    while let Some(first) = iter.next() {
        let mut last = first;
        while iter.peek() == Some(&(last + 1)) {
            last += 1;
            iter.next();
        }
        if !out.is_empty() {
            out.push(',');
        }
        if last == first {
            let _ = write!(out, "{first:x}");
        } else {
            let _ = write!(out, "{first:x}-{last:x}");
        }
    }
    out
}

/// The canonical codepoint string of `text`.
pub fn canonical(text: &str) -> String {
    format(&of_text(text))
}

/// Parses a canonical codepoint string. Anything that would not round-trip
/// through [`format`](fn@format) unchanged is rejected, so one set has one spelling.
pub fn parse(s: &str) -> Result<Vec<u32>, ParseError> {
    if s.is_empty() {
        return Ok(Vec::new());
    }
    let mut ranges = Vec::new();
    let mut count = 0usize;
    for item in s.split(',') {
        let (first, last) = match item.split_once('-') {
            Some((a, b)) => (hex(a)?, hex(b)?),
            None => (hex(item)?, hex(item)?),
        };
        if last < first {
            return Err(ParseError::NotCanonical);
        }
        count += (last - first) as usize + 1;
        if count > MAX_CODEPOINTS {
            return Err(ParseError::TooMany);
        }
        ranges.push((first, last));
    }
    let set: BTreeSet<u32> = ranges.iter().flat_map(|&(a, b)| a..=b).collect();
    if set.iter().any(|&cp| !is_allowed(cp)) || format(&set) != s {
        return Err(ParseError::NotCanonical);
    }
    Ok(set.into_iter().collect())
}

fn hex(s: &str) -> Result<u32, ParseError> {
    let valid = !s.is_empty() && s.len() <= 6 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    if !valid {
        return Err(ParseError::NotCanonical);
    }
    u32::from_str_radix(s, 16).map_err(|_| ParseError::NotCanonical)
}

#[cfg(test)]
mod tests {
    use super::{ParseError, canonical, parse};

    #[test]
    fn parse_rejects_non_canonical_and_oversized_lists() {
        assert_eq!(parse("20,41-5a,e9"), Ok([0x20].into_iter().chain(0x41..=0x5a).chain([0xe9]).collect()));
        for bad in ["41,20", "41-42,43", "41,42", "0041", "4A", "41-41", "+41", "41,,42", "a", "d800", "110000"] {
            assert_eq!(parse(bad), Err(ParseError::NotCanonical), "{bad}");
        }
        assert_eq!(parse("100-200"), Err(ParseError::TooMany));
        assert_eq!(parse("100-1ff").map(|v| v.len()), Ok(256));
        assert_eq!(parse(&canonical("héllo")).map(|v| v.len()), Ok(4));
    }
}
