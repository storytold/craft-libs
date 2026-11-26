//! Bidirectional isolates: wrapping, detecting and stripping the marks around inserted values.

use unicode_bidi::{BidiClass, bidi_class};

use crate::scripts::Direction;

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
/// letter and can't reorder the text around it. What [`crate::fmt`] does to each value in text with a
/// right-to-left letter.
pub fn isolate(value: &str) -> String {
    wrap(FSI, value)
}

/// `value` as a left-to-right isolate (LRI…PDI), for numbers, paths, file sizes, timecodes and
/// codes, which read left to right in every language. In an RTL first-strong isolate, the
/// segments of `/home/مستخدم/file.txt` would run right to left. [`crate::fmt`] takes the isolate off
/// again in text with no right-to-left letter.
pub fn ltr(value: &str) -> String {
    wrap(LRI, value)
}

/// A file name for display in an interface of `direction`: unchanged for left-to-right, an
/// isolate with a left-to-right mark before a short extension for right-to-left.
pub fn file_name(name: &str, direction: Direction) -> String {
    if direction.is_rtl() { isolate(&mark_extension(name)) } else { name.to_string() }
}

/// A path for display in an interface of `direction`: unchanged for left-to-right, a left-to-right
/// isolate with marks after every separator and before a short extension for right-to-left.
pub fn path(value: &str, direction: Direction) -> String {
    if !direction.is_rtl() {
        return value.to_string();
    }
    let marked = mark_extension(value);
    let mut out = String::with_capacity(marked.len() + 8);
    for c in marked.chars() {
        out.push(c);
        // Keeps two right-to-left folder names from joining into one run that swaps them.
        if matches!(c, '/' | '\\') {
            out.push(LRM);
        }
    }
    ltr(&out)
}

/// `name` with a left-to-right mark before its extension (the text after the last `.`, not at the start, 1 to 8 ASCII
/// letters or digits), which a right-to-left line would otherwise draw as `docx.2` for `2.docx`.
fn mark_extension(name: &str) -> String {
    if let Some(dot) = name.rfind('.')
        && dot > 0
        && name.get(dot + 1..).is_some_and(|ext| !ext.is_empty() && ext.len() <= 8 && ext.bytes().all(|b| b.is_ascii_alphanumeric()))
        && let (Some(stem), Some(ext)) = (name.get(..dot), name.get(dot..))
    {
        return format!("{stem}{LRM}{ext}");
    }
    name.to_string()
}

/// `text` without isolate marks (LRI, RLI, FSI, PDI): a [`crate::fmt`] or [`crate::I18n::trn`] result made fit
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
            PDI if isolates > 0 => {
                while open.pop() == Some(false) {
                    out.push(PDF);
                }
                isolates -= 1;
            }
            PDF | PDI => continue,
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

/// The initiators `text` leaves open, outermost first: true for an isolate, false for an embedding or override. A PDF closes only an
/// embedding at the top, a PDI closes the innermost isolate and what is open inside it, and a stray one closes nothing.
fn open_initiators(text: &str) -> Vec<bool> {
    let mut open: Vec<bool> = Vec::new();
    let mut isolates = 0usize;
    for c in text.chars() {
        step(&mut open, &mut isolates, c);
    }
    open
}

/// One character of the stack logic of [`wrap`], [`open_initiators`] and [`cap_balanced`].
fn step(open: &mut Vec<bool>, isolates: &mut usize, c: char) {
    match c {
        LRI | RLI | FSI => {
            open.push(true);
            *isolates += 1;
        }
        LRE | RLE | LRO | RLO => open.push(false),
        PDF if open.last() == Some(&false) => {
            open.pop();
        }
        PDI if *isolates > 0 => {
            while open.pop() == Some(false) {}
            *isolates -= 1;
        }
        _ => {}
    }
}

/// `text` closed, and cut to `max` characters: whatever it opens (an embedding, an override, an isolate) is closed before it
/// ends, so what follows it in a window is not reordered by it. When it is longer than `max`, it ends with `…` and the closers
/// of what is open at the cut, all within `max` characters; a cut never leaves an isolate open, whatever it falls inside.
pub(super) fn cap_balanced(text: &str, max: usize) -> String {
    let total = text.chars().count();
    // The most characters kept: the prefix, then `…` unless all of it is kept, then one closer for each initiator open there.
    let fits = |kept: usize, depth: usize| kept + usize::from(kept < total) + depth <= max;
    if !fits(0, 0) {
        // Not even `…` fits.
        return String::new();
    }
    let mut open: Vec<bool> = Vec::new();
    let mut isolates = 0usize;
    let mut keep = 0usize;
    for (i, c) in text.chars().enumerate() {
        step(&mut open, &mut isolates, c);
        if fits(i + 1, open.len()) {
            keep = i + 1;
        }
    }
    let kept: String = text.chars().take(keep).collect();
    let mut out = String::with_capacity(kept.len() + 4);
    out.push_str(&kept);
    if keep < total {
        out.push('…');
    }
    // What is open at the cut, innermost closed first.
    let mut open = open_initiators(&kept);
    while let Some(isolate) = open.pop() {
        out.push(if isolate { PDI } else { PDF });
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn open(text: &str) -> usize {
        open_initiators(text).len()
    }

    // Uncut, a message that opens nothing is the message; one that opens something is closed, innermost first.
    #[test]
    fn a_message_is_closed_at_its_end() {
        assert_eq!(cap_balanced("plain text", 100), "plain text");
        assert_eq!(cap_balanced("a\u{202B}b\u{2067}c", 100), "a\u{202B}b\u{2067}c\u{2069}\u{202C}");
        assert_eq!(cap_balanced("\u{2068}x\u{2069}", 100), "\u{2068}x\u{2069}");
        // A stray closer closes nothing and is left as it is.
        assert_eq!(cap_balanced("x\u{2069}\u{202C}y", 100), "x\u{2069}\u{202C}y");
    }

    // A cut ends with `…`, then the closers of what the kept text left open, in a total of at most `max`.
    #[test]
    fn a_cut_closes_what_it_leaves_open_within_the_cap() {
        let text = format!("\u{2068}{}\u{2069}", "ا".repeat(50));
        for max in 3..60 {
            let shown = cap_balanced(&text, max);
            assert!(shown.chars().count() <= max, "{max}: {}", shown.chars().count());
            assert_eq!(open(&shown), 0, "{max}");
        }
        let shown = cap_balanced(&text, 10);
        assert_eq!(shown, format!("\u{2068}{}…\u{2069}", "ا".repeat(7)));
        // Nested initiators are closed in order: the embedding first, then its isolate.
        let nested = format!("\u{2068}\u{202B}{}", "x".repeat(20));
        assert_eq!(cap_balanced(&nested, 8), "\u{2068}\u{202B}xxx…\u{202C}\u{2069}");
    }

    // Closers that do not fit leave a shorter prefix, down to `…` alone; a cap too small for even that keeps nothing.
    #[test]
    fn deep_nesting_shortens_the_prefix_and_not_the_closers() {
        let deep = "\u{202B}".repeat(50);
        for max in 0..120 {
            let shown = cap_balanced(&deep, max);
            assert!(shown.chars().count() <= max, "{max}");
            assert_eq!(open(&shown), 0, "{max}");
        }
        assert_eq!(cap_balanced(&deep, 0), "");
        assert_eq!(cap_balanced(&deep, 1), "…");
        assert_eq!(cap_balanced("abc", 0), "");
    }
}
