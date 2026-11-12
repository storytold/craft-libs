//! Making a server's text fit for an error message (requirement 3.5).
//!
//! An [`Excerpt`] is one line, without control or invisible characters, at most [`EXCERPT_MAX_CHARS`]
//! characters, and with every secret the [`Redactor`] knows replaced by `[redacted]`.
//!
//! How it works, and why:
//!
//! 1. The server's text is first *normalised*: every run of white space, control characters and line or
//!    paragraph separators becomes one space, and invisible formatting characters (zero-width spaces and
//!    joiners, bidirectional controls, the byte order mark, soft hyphens, tag characters) are removed. Secrets
//!    are normalised the same way. Matching happens on the normalised text, which is also what is shown, so
//!    the cleaning cannot rebuild a secret the matching missed (`open\nsesame` is found as `open sesame`).
//! 2. Matching ignores ASCII case and covers several written forms of each secret (as is, percent-encoded,
//!    form-encoded with `+` for spaces; the credentials add their base64 forms). Overlapping matches are
//!    merged. The search is linear in the text (Knuth–Morris–Pratt), so a hostile server cannot make it slow.
//! 3. Secrets are replaced before anything is cut: the scanned window reaches past its end by the length of
//!    the longest secret, so a secret straddling the cut is still found whole and never shows half of itself.
//!
//! Two strengths of entry: [`Redactor::add`] is for secrets proper (passwords, tokens). When one is shorter
//! than [`MIN_SECRET_LEN`] octets, replacing it would itself reveal it (every `a` gone tells that the password
//! is `a`), so all server text is then withheld. [`Redactor::add_personal`] is for personal data and
//! addresses (a user name, an account name, the session address); a form shorter than [`MIN_SECRET_LEN`] is
//! then simply not looked for, so that a server cannot blank every message by naming a user `x`.

use std::fmt;

use crate::credentials::Secret;
use crate::percent::encode_component;

/// The most characters an excerpt holds, not counting the final ellipsis.
pub const EXCERPT_MAX_CHARS: usize = 160;
/// Secrets shorter than this, in octets once normalised, cannot be replaced safely.
pub const MIN_SECRET_LEN: usize = 3;
/// How much normalised text is looked at, in octets; the rest is cut.
const SCAN_WINDOW: usize = 4 * 1024;
/// What replaces a secret.
const MARK: &str = "[redacted]";
/// What stands for a text that could not be shown safely.
const WITHHELD: &str = "[server text withheld]";
/// The number of characters of [`MARK`].
const MARK_CHARS: usize = 10;

/// A short, single-line, control-free piece of server text with secrets removed.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Excerpt(String);

impl Excerpt {
    /// The text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Excerpt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for Excerpt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.0, f)
    }
}

/// The secrets a client knows, and the means to keep them out of messages.
#[derive(Clone, Default)]
pub struct Redactor {
    needles: Vec<Secret>,
    withhold: bool,
}

impl fmt::Debug for Redactor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Redactor({} forms)", self.needles.len())
    }
}

/// Characters removed outright: they are invisible and could split a secret without showing.
fn is_invisible(c: char) -> bool {
    matches!(c,
        '\u{00AD}' | '\u{034F}' | '\u{061C}' | '\u{115F}' | '\u{1160}' | '\u{17B4}' | '\u{17B5}' | '\u{180B}'..='\u{180F}'
        | '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{206F}' | '\u{3164}' | '\u{FE00}'..='\u{FE0F}'
        | '\u{FEFF}' | '\u{FFA0}' | '\u{FFF0}'..='\u{FFFB}' | '\u{E0000}'..='\u{E0FFF}')
}

/// Characters that become a space: white space, controls (Unicode `Cc`), line and paragraph separators.
fn is_separator(c: char) -> bool {
    c.is_whitespace() || c.is_control() || matches!(c, '\u{2028}' | '\u{2029}')
}

/// Normalises `text` (see the module documentation), stopping once `limit` octets are produced. Returns the
/// text and whether anything visible was left out.
fn normalise(text: &str, limit: usize) -> (String, bool) {
    let mut out = String::new();
    let mut pending_space = false;
    for c in text.chars() {
        if is_invisible(c) {
            continue;
        }
        if is_separator(c) {
            pending_space = true;
            continue;
        }
        if out.len().saturating_add(c.len_utf8()).saturating_add(1) > limit {
            return (out, true);
        }
        if pending_space && !out.is_empty() {
            out.push(' ');
        }
        pending_space = false;
        out.push(c);
    }
    (out, false)
}

fn floor_boundary(text: &str, index: usize) -> usize {
    let mut index = index.min(text.len());
    while index > 0 && !text.is_char_boundary(index) {
        index = index.saturating_sub(1);
    }
    index
}

/// The Knuth–Morris–Pratt failure table: for each prefix of `needle`, the length of its longest proper
/// prefix that is also a suffix.
fn failure_table(needle: &[u8]) -> Vec<usize> {
    let mut table = vec![0usize; needle.len()];
    let mut k = 0usize;
    for (i, &b) in needle.iter().enumerate().skip(1) {
        while k > 0 && needle.get(k) != Some(&b) {
            k = table.get(k.saturating_sub(1)).copied().unwrap_or(0);
        }
        if needle.get(k) == Some(&b) {
            k = k.saturating_add(1);
        }
        if let Some(slot) = table.get_mut(i) {
            *slot = k;
        }
    }
    table
}

/// Calls `found` with the start of every occurrence of `needle` in `hay`, overlapping ones included.
fn for_each_match(hay: &[u8], needle: &[u8], mut found: impl FnMut(usize)) {
    let len = needle.len();
    if len == 0 || len > hay.len() {
        return;
    }
    let table = failure_table(needle);
    let mut k = 0usize;
    for (i, &b) in hay.iter().enumerate() {
        while k > 0 && needle.get(k) != Some(&b) {
            k = table.get(k.saturating_sub(1)).copied().unwrap_or(0);
        }
        if needle.get(k) == Some(&b) {
            k = k.saturating_add(1);
        }
        if k == len {
            found(i.saturating_add(1).saturating_sub(len));
            k = table.get(len.saturating_sub(1)).copied().unwrap_or(0);
        }
    }
}

impl Redactor {
    /// An empty redactor.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a secret proper (a password, a token) in each of its written forms.
    ///
    /// A form shorter than [`MIN_SECRET_LEN`] once normalised makes every excerpt withheld.
    pub fn add(&mut self, secret: &str) {
        self.add_forms(secret, true);
    }

    /// Adds personal data or an address (a user name, an account name, the session address) in each of its
    /// written forms. A form shorter than [`MIN_SECRET_LEN`] is not looked for.
    pub fn add_personal(&mut self, text: &str) {
        self.add_forms(text, false);
    }

    fn add_forms(&mut self, text: &str, strong: bool) {
        if text.is_empty() {
            return;
        }
        let encoded = Secret::new(encode_component(text));
        let form_encoded = Secret::new(encoded.expose().replace("%20", "+"));
        for form in [text, encoded.expose(), form_encoded.expose()] {
            self.push(form, strong);
        }
    }

    fn push(&mut self, form: &str, strong: bool) {
        let normalised = Secret::new(normalise(form, usize::MAX).0);
        let needle = Secret::new(normalised.expose().to_ascii_lowercase());
        if needle.expose().len() < MIN_SECRET_LEN {
            if strong {
                self.withhold = true;
            }
            return;
        }
        if !self.needles.iter().any(|known| known.expose() == needle.expose()) {
            self.needles.push(needle);
        }
    }

    /// How many written forms are looked for.
    pub fn len(&self) -> usize {
        self.needles.len()
    }

    /// Whether nothing is looked for.
    pub fn is_empty(&self) -> bool {
        self.needles.is_empty()
    }

    /// Whether `text` holds a known secret (or excerpts are withheld altogether).
    pub fn mentions_secret(&self, text: &str) -> bool {
        self.withhold || self.excerpt(text).as_str() != normalise(text, usize::MAX).0
    }

    /// Turns a server's text into an excerpt fit for an error message.
    pub fn excerpt(&self, text: &str) -> Excerpt {
        if self.withhold {
            return Excerpt(WITHHELD.to_owned());
        }
        let longest = self.needles.iter().map(|needle| needle.expose().len()).max().unwrap_or(0);
        let (normalised, cut_short) = normalise(text, SCAN_WINDOW.saturating_add(longest));
        let normalised = Secret::new(normalised);
        let shown = normalised.expose();
        let window_end = floor_boundary(shown, SCAN_WINDOW);
        let folded = Secret::new(shown.to_ascii_lowercase());
        // ends[i]: the furthest end of a secret starting at octet i.
        let mut ends = vec![0usize; window_end];
        for needle in &self.needles {
            let needle_len = needle.expose().len();
            for_each_match(folded.expose().as_bytes(), needle.expose().as_bytes(), |start| {
                if let Some(slot) = ends.get_mut(start) {
                    *slot = (*slot).max(start.saturating_add(needle_len));
                }
            });
        }
        let mut out = String::new();
        let mut count = 0usize;
        let mut covered_until = 0usize;
        let mut in_mark = false;
        let mut truncated = cut_short || window_end < shown.len();
        for (index, c) in shown.get(..window_end).unwrap_or("").char_indices() {
            for end in ends.iter().take(index.saturating_add(c.len_utf8())).skip(index) {
                covered_until = covered_until.max(*end);
            }
            if index < covered_until {
                if !in_mark {
                    if count.saturating_add(MARK_CHARS) > EXCERPT_MAX_CHARS {
                        truncated = true;
                        break;
                    }
                    out.push_str(MARK);
                    count = count.saturating_add(MARK_CHARS);
                    in_mark = true;
                }
                continue;
            }
            in_mark = false;
            if count >= EXCERPT_MAX_CHARS {
                truncated = true;
                break;
            }
            out.push(c);
            count = count.saturating_add(1);
        }
        if truncated {
            let kept = out.trim_end().len();
            out.truncate(kept);
            out.push('…');
        }
        Excerpt(out)
    }
}

#[cfg(test)]
mod tests;
