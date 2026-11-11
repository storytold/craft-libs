//! Bidirectional isolates: wrapping, detecting and stripping the marks around inserted values.

use super::*;

pub(super) const LRI: char = '\u{2066}';
pub(super) const RLI: char = '\u{2067}';
pub(super) const FSI: char = '\u{2068}';
pub(super) const PDI: char = '\u{2069}';
pub(super) const LRE: char = '\u{202A}';
pub(super) const RLE: char = '\u{202B}';
pub(super) const PDF: char = '\u{202C}';
pub(super) const LRO: char = '\u{202D}';
pub(super) const RLO: char = '\u{202E}';
pub(super) const LRM: char = '\u{200E}';
pub(super) const RLM: char = '\u{200F}';
pub(super) const ALM: char = '\u{061C}';

/// `value` as a first-strong isolate (FSI…PDI): it takes the direction of its first strong
/// letter and can't reorder the text around it. What [`fmt`] does to each value in text with a
/// right-to-left letter.
pub fn isolate(value: &str) -> String {
    wrap(FSI, value)
}

/// `value` as a left-to-right isolate (LRI…PDI), for numbers, paths, file sizes, timecodes and
/// codes, which read left to right in every language. In an RTL first-strong isolate, the
/// segments of `/home/مستخدم/file.txt` would run right to left. [`fmt`] takes the isolate off
/// again in text with no right-to-left letter.
pub fn ltr(value: &str) -> String {
    wrap(LRI, value)
}

/// `text` without isolate marks (LRI, RLI, FSI, PDI): a [`fmt`] or [`I18n::trn`] result made fit
/// for a file name, a log or a comparison.
pub fn strip_isolates(text: &str) -> String {
    text.chars().filter(|c| !matches!(*c, LRI | RLI | FSI | PDI)).collect()
}

/// `value` between `initiator` and PDI, with every embedding, override and isolate it opens
/// closed inside: a stray PDF or PDI is dropped, and open ones are closed in order. Under the
/// bidi algorithm the outer PDI alone would close them, but a renderer that handles only
/// embeddings would let them run on into the rest of the text.
pub(super) fn wrap(initiator: char, value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 6);
    out.push(initiator);
    // One entry per open initiator: true for an isolate, false for an embedding or override.
    let mut open: Vec<bool> = Vec::new();
    // How many of `open` are isolates, so a PDI needn't search the stack for one.
    let mut isolates = 0usize;
    for c in value.chars() {
        match c {
            LRI | RLI | FSI => {
                open.push(true);
                isolates += 1;
            }
            LRE | RLE | LRO | RLO => open.push(false),
            PDF if open.last() == Some(&false) => {
                open.pop();
            }
            PDF => continue,
            PDI if isolates > 0 => {
                while open.pop() == Some(false) {
                    out.push(PDF);
                }
                isolates -= 1;
            }
            PDI => continue,
            _ => {}
        }
        out.push(c);
    }
    while let Some(isolate) = open.pop() {
        out.push(if isolate { PDI } else { PDF });
    }
    out.push(PDI);
    out
}

/// Whether `value`, without its own isolate, has a bidi control: an embedding, override,
/// isolate or mark that could reorder the text around it unless it is contained.
pub(super) fn has_bidi_control(value: &str) -> bool {
    !value.is_ascii() && unwrap_isolate(value).chars().any(|c| matches!(c, LRE | RLE | PDF | LRO | RLO | LRI | RLI | FSI | PDI | LRM | RLM | ALM))
}

/// Whether `text` has a strong right-to-left character (bidi class R or AL): the text that needs
/// isolates.
pub(super) fn has_rtl(text: &str) -> bool {
    !text.is_ascii() && text.chars().any(|c| matches!(bidi_class(c), BidiClass::R | BidiClass::AL))
}

/// `value` without its isolate, if it is exactly one.
pub(super) fn unwrap_isolate(value: &str) -> &str {
    if is_one_isolate(value) {
        // The initiator and PDI are three bytes each in UTF-8.
        value.get(3..value.len() - 3).unwrap_or(value)
    } else {
        value
    }
}

/// Whether `value` is exactly one isolate: an initiator, then the PDI that closes it last.
pub(super) fn is_one_isolate(value: &str) -> bool {
    let mut depth = 0_usize;
    let mut chars = value.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            LRI | RLI | FSI => depth += 1,
            PDI if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    return chars.peek().is_none();
                }
            }
            _ if depth == 0 => return false,
            _ => {}
        }
    }
    false
}
