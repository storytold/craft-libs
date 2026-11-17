// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Bounded certificate display-name sanitization extracted from MailCraft.

/// Whether `c` is one of Unicode's format characters (general category Cf, Unicode 17.0): the
/// soft hyphen, the Arabic number signs and end-of-ayah marks, the Syriac abbreviation mark, the
/// Arabic pound and piastre marks above, the Mongolian vowel separator, zero-width marks and
/// joiners, bidirectional marks, embeddings, overrides and isolates, the word joiner and
/// invisible operators, the deprecated shaping controls, the byte-order mark, interlinear
/// annotations, the Kaithi number signs, the Egyptian hieroglyph, shorthand and musical format
/// controls, and tags.
pub fn is_format(c: char) -> bool {
    matches!(
        c,
        '\u{00ad}'
            | '\u{0600}'..='\u{0605}'
            | '\u{061c}'
            | '\u{06dd}'
            | '\u{070f}'
            | '\u{0890}'..='\u{0891}'
            | '\u{08e2}'
            | '\u{180e}'
            | '\u{200b}'..='\u{200f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{206f}'
            | '\u{feff}'
            | '\u{fff9}'..='\u{fffb}'
            | '\u{110bd}'
            | '\u{110cd}'
            | '\u{13430}'..='\u{1343f}'
            | '\u{1bca0}'..='\u{1bca3}'
            | '\u{1d173}'..='\u{1d17a}'
            | '\u{e0001}'
            | '\u{e0020}'..='\u{e007f}'
    )
}

/// Whether `c` is the zero-width non-joiner or joiner (U+200C, U+200D): format characters that
/// are part of a word where they stand inside one.
pub fn is_joiner(c: char) -> bool {
    matches!(c, '\u{200c}' | '\u{200d}')
}

/// A name as it is kept: one line (a line break, a tab or any other white space parts words, and
/// words are parted by one space), without control characters or the characters Unicode marks as
/// showing nothing (a joiner inside a word and one variation selector after a character stay),
/// trimmed, at most `max` characters. `None` when nothing is left.
pub fn incoming(text: &str, max: usize) -> Option<String> {
    let (name, _) = clean(text, max);
    (!name.is_empty()).then_some(name)
}

/// Whether `c` is one of the rest of Unicode's default-ignorable code points
/// (`Default_Ignorable_Code_Point`, Unicode 17.0, less the format characters and the variation
/// selectors): the combining grapheme joiner, the Hangul fillers, the Khmer inherent vowels, and
/// the code points Unicode keeps for more characters that show nothing.
fn is_ignorable(c: char) -> bool {
    matches!(
        c,
        '\u{034f}'
            | '\u{115f}'..='\u{1160}'
            | '\u{17b4}'..='\u{17b5}'
            | '\u{2065}'
            | '\u{3164}'
            | '\u{ffa0}'
            | '\u{fff0}'..='\u{fff8}'
            | '\u{e0000}'
            | '\u{e0002}'..='\u{e001f}'
            | '\u{e0080}'..='\u{e00ff}'
            | '\u{e01f0}'..='\u{e0fff}'
    )
}

/// Whether `c` is a variation selector (`Variation_Selector`, Unicode 17.0): it picks how the
/// character before it is drawn.
fn is_selector(c: char) -> bool {
    matches!(c, '\u{180b}'..='\u{180d}' | '\u{180f}' | '\u{fe00}'..='\u{fe0f}' | '\u{e0100}'..='\u{e01ef}')
}

/// The braille pattern blank: a symbol to Unicode, and in braille the space between words.
const BRAILLE_BLANK: char = '\u{2800}';

/// `text` cleaned and cut to at most `max` characters, and whether something was left out for
/// the length. Nothing past it is read.
fn clean(text: &str, max: usize) -> (String, bool) {
    let mut out = String::new();
    let mut count = 0usize;
    // A space between words, and a joiner inside one, come only with the character after them.
    let mut space = false;
    let mut joiner: Option<char> = None;
    // Whether the last character written shows, so that a variation selector may follow it.
    let mut selectable = false;
    for c in text.chars() {
        if is_joiner(c) {
            if count > 0 && !space {
                joiner = Some(c);
            }
        } else if c.is_whitespace() || c == BRAILLE_BLANK {
            space = count > 0;
            joiner = None;
        } else if is_selector(c) {
            if selectable && !space && joiner.is_none() {
                if count >= max {
                    return (out, true);
                }
                out.push(c);
                count = count.saturating_add(1);
                selectable = false;
            }
        } else if !c.is_control() && !is_format(c) && !is_ignorable(c) {
            let needed = 1 + usize::from(space) + usize::from(joiner.is_some());
            if count.saturating_add(needed) > max {
                return (out, true);
            }
            if std::mem::take(&mut space) {
                out.push(' ');
            }
            if let Some(j) = joiner.take() {
                out.push(j);
            }
            out.push(c);
            count = count.saturating_add(needed);
            selectable = true;
        }
    }
    (out, false)
}
