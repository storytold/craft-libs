//! Menu mnemonics (spec 9.7): the letter that opens a menu with Alt, or runs an entry of an open menu.
//!
//! A label's `&` marker, written in its words row (`&File`), names the letter. Without one the letter is the
//! label's first letter not yet used in the level, so a translated label needs no extra work. Letters are ASCII
//! letters and digits, the keys the keyboard reports as such, and upper case in the folder's language.

use craft_i18n::Folder;

/// The label without its `&` marker, and the letter the first marker names. `&&` is a literal ampersand, a
/// marker before something other than an ASCII letter or digit names no letter, and later markers are dropped.
pub fn marked(label: &str, folder: &Folder) -> (String, Option<char>) {
    let (text, mark) = marked_at(label, folder);
    (text, mark.map(|(letter, _)| letter))
}

/// [`marked`] with the character index of the marked letter in the returned text.
pub fn marked_at(label: &str, folder: &Folder) -> (String, Option<(char, usize)>) {
    let turkic = folder.turkic();
    let mut text = String::with_capacity(label.len());
    let mut mark = None;
    let mut at = 0;
    let mut chars = label.chars();
    while let Some(c) = chars.next() {
        if c != '&' {
            text.push(c);
            at += 1;
            continue;
        }
        let Some(next) = chars.next() else { break };
        if mark.is_none()
            && next != '&'
            && let Some(key) = letter(next, turkic)
        {
            mark = Some((key, at));
        }
        text.push(next);
        at += 1;
    }
    (text, mark)
}

/// The mnemonic of each label in one menu level: the letter, upper case, and its character index in the label
/// without its markers. A `&` marker wins; a label whose marked letter an earlier label holds, and a label without a
/// marker, takes the first letter or digit of its text not yet used, trying the first character of each word before
/// the others.
pub fn assign(labels: &[&str], folder: &Folder) -> Vec<Option<(char, usize)>> {
    let parsed: Vec<(String, Option<(char, usize)>)> = labels.iter().map(|label| marked_at(label, folder)).collect();
    let mut used: Vec<char> = Vec::new();
    let mut out: Vec<Option<(char, usize)>> = Vec::with_capacity(parsed.len());
    for (_, mark) in &parsed {
        let held = mark.filter(|(letter, _)| !used.contains(letter));
        used.extend(held.map(|(letter, _)| letter));
        out.push(held);
    }
    for ((text, _), slot) in parsed.iter().zip(&mut out) {
        if slot.is_none() {
            *slot = first_unused(text, &used, folder.turkic());
            used.extend(slot.map(|(letter, _)| letter));
        }
    }
    out
}

fn first_unused(text: &str, used: &[char], turkic: bool) -> Option<(char, usize)> {
    let mut previous_alnum = false;
    let mut starts = Vec::new();
    for (at, c) in text.chars().enumerate() {
        if c.is_alphanumeric() && !previous_alnum {
            starts.push((c, at));
        }
        previous_alnum = c.is_alphanumeric();
    }
    let all = text.chars().enumerate().map(|(at, c)| (c, at));
    starts.into_iter().chain(all).filter_map(|(c, at)| Some((letter(c, turkic)?, at))).find(|(c, _)| !used.contains(c))
}

/// The key that types `c`, as a mnemonic letter: its upper case, which must be an ASCII letter or digit. In Turkish
/// and Azerbaijani `I` is the capital of `ı`, so `ı` takes the I key, while `i` and `İ` have no key of their own.
fn letter(c: char, turkic: bool) -> Option<char> {
    match c {
        'ı' if turkic => Some('I'),
        'i' if turkic => None,
        c if c.is_ascii_alphanumeric() => Some(c.to_ascii_uppercase()),
        _ => None,
    }
}
