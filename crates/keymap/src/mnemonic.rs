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
    let own = vec![None; labels.len()];
    assign_with(labels, &own, folder).into_iter().map(|mark| mark.and_then(|(letter, at)| Some((letter, at?)))).collect()
}

/// A mnemonic letter and its character index in the label's text, none when the text does not show it.
type Placed = (char, Option<usize>);

/// [`assign`] where a label may come with a letter of its own: a label without a `&` marker holds its `own` letter as a
/// marker would, at the first character of its text that types it, or with no index when the text has none (a
/// translated label keeps its English label's key). An own letter that is not a typable key is ignored.
pub fn assign_with(labels: &[&str], own: &[Option<char>], folder: &Folder) -> Vec<Option<(char, Option<usize>)>> {
    let turkic = folder.turkic();
    let parsed: Vec<(String, Option<Placed>)> = labels
        .iter()
        .enumerate()
        .map(|(i, label)| {
            let (text, mark) = marked_at(label, folder);
            let own = own
                .get(i)
                .copied()
                .flatten()
                .and_then(|c| letter(c, turkic))
                .map(|key| (key, text.chars().position(|c| letter(c, turkic) == Some(key))));
            (text, mark.map(|(letter, at)| (letter, Some(at))).or(own))
        })
        .collect();
    let mut used: Vec<char> = Vec::new();
    let mut out: Vec<Option<Placed>> = Vec::with_capacity(parsed.len());
    for (_, mark) in &parsed {
        let held = mark.filter(|(letter, _)| !used.contains(letter));
        used.extend(held.map(|(letter, _)| letter));
        out.push(held);
    }
    for ((text, _), slot) in parsed.iter().zip(&mut out) {
        if slot.is_none() {
            *slot = first_unused(text, &used, turkic).map(|(letter, at)| (letter, Some(at)));
            used.extend(slot.map(|(letter, _)| letter));
        }
    }
    out
}

/// The keytip badge of each `(label, requested badge)` in a bar, in three passes; `reserved` letters are never
/// assigned and no badge is a prefix of another.
///
/// 1. A requested badge is kept, in order, when it is typable (ASCII letters and digits, upper-cased in the folder's
///    language) and neither starts nor is the start of a badge already kept. It may use a reserved letter: the app
///    gives a control the letter of its own Alt shortcut.
/// 2. Every other label takes one letter: its `&` marked letter, else the first free letter of its text, word starts
///    first, else the first free letter of the alphabet. A letter is free when no badge uses it, it is not reserved,
///    and no kept badge of two or more letters starts with it.
/// 3. The labels left over get two letters. Their first letter is one no one-letter badge keeps (so typing it waits for
///    the second): the letters of the last one-letter badges of pass 2 are given up for this until the alphabet's
///    unreserved second letters cover the overflow, and those labels take two letters too, in label order.
///
/// A label gets an empty badge, which shows nothing, only when the alphabet cannot hold every label.
pub fn assign_keytips(labels: &[(&str, Option<&str>)], reserved: &[char], folder: &Folder) -> Vec<String> {
    let turkic = folder.turkic();
    let clashes = |a: &str, b: &str| a.starts_with(b) || b.starts_with(a);
    let mut out: Vec<Option<String>> = vec![None; labels.len()];

    // Pass 1: the badges the app asks for; one that clashes with an earlier kept badge is left to pass 2.
    let mut kept: Vec<String> = Vec::new();
    for ((_, requested), slot) in labels.iter().zip(&mut out) {
        let Some(requested) = requested else { continue };
        let typed: Option<String> = requested.chars().map(|c| letter(c, turkic)).collect();
        let Some(typed) = typed.filter(|typed| !typed.is_empty()) else { continue };
        if kept.iter().any(|earlier| clashes(earlier, &typed)) {
            continue;
        }
        kept.push(typed.clone());
        *slot = Some(typed);
    }

    // Pass 2: one letter from the label.
    let mut blocked: Vec<char> = reserved.iter().filter_map(|&c| letter(c, turkic)).collect();
    blocked.extend(out.iter().flatten().filter_map(|badge| badge.chars().next()));
    let mut named: Vec<usize> = Vec::new();
    let mut overflow: Vec<usize> = Vec::new();
    for (at, (label, _)) in labels.iter().enumerate() {
        if out[at].is_some() {
            continue;
        }
        let (text, mark) = marked(label, folder);
        let chosen = mark
            .filter(|c| !blocked.contains(c))
            .or_else(|| first_unused(&text, &blocked, turkic).map(|(c, _)| c))
            .or_else(|| ('A'..='Z').find(|c| !blocked.contains(c)));
        match chosen {
            Some(c) => {
                blocked.push(c);
                out[at] = Some(c.to_string());
                named.push(at);
            }
            None => overflow.push(at),
        }
    }

    // Pass 3: two letters, whose first letter no one-letter badge keeps.
    let reserved_keys: Vec<char> = reserved.iter().filter_map(|&c| letter(c, turkic)).collect();
    let seconds: Vec<char> = ('A'..='Z').filter(|c| !reserved_keys.contains(c)).collect();
    let mut firsts: Vec<char> = Vec::new();
    let mut two = overflow;
    while !two.is_empty() && firsts.len() * seconds.len() < two.len() {
        let Some(at) = named.pop() else { break };
        firsts.extend(out[at].take().and_then(|badge| badge.chars().next()));
        two.push(at);
    }
    two.sort_unstable();
    let mut pairs = firsts.iter().flat_map(|a| seconds.iter().map(move |b| format!("{a}{b}")));
    for at in two {
        out[at] = pairs.next();
    }
    out.into_iter().map(Option::unwrap_or_default).collect()
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
