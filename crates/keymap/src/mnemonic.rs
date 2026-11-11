//! Menu mnemonics (spec 9.7): the letter that opens a menu with Alt, or runs an entry of an open menu.
//!
//! A label's `&` marker, written in its words row (`&File`), names the letter. Without one the letter is the
//! label's first letter not yet used in the level, so a translated label needs no extra work. Letters are ASCII
//! letters and digits, the keys the keyboard reports as such, and upper case.

/// The label without its `&` marker, and the letter the first marker names. `&&` is a literal ampersand, a
/// marker before something other than an ASCII letter or digit names no letter, and later markers are dropped.
pub fn marked(label: &str) -> (String, Option<char>) {
    let (text, mark) = marked_at(label);
    (text, mark.map(|(letter, _)| letter))
}

/// [`marked`] with the character index of the marked letter in the returned text.
pub fn marked_at(label: &str) -> (String, Option<(char, usize)>) {
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
        if mark.is_none() && next != '&' && next.is_ascii_alphanumeric() {
            mark = Some((next.to_ascii_uppercase(), at));
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
pub fn assign(labels: &[&str]) -> Vec<Option<(char, usize)>> {
    let parsed: Vec<(String, Option<(char, usize)>)> = labels.iter().map(|label| marked_at(label)).collect();
    let mut used: Vec<char> = Vec::new();
    let mut out: Vec<Option<(char, usize)>> = Vec::with_capacity(parsed.len());
    for (_, mark) in &parsed {
        let held = mark.filter(|(letter, _)| !used.contains(letter));
        used.extend(held.map(|(letter, _)| letter));
        out.push(held);
    }
    for ((text, _), slot) in parsed.iter().zip(&mut out) {
        if slot.is_none() {
            *slot = first_unused(text, &used);
            used.extend(slot.map(|(letter, _)| letter));
        }
    }
    out
}

fn first_unused(text: &str, used: &[char]) -> Option<(char, usize)> {
    let mut previous_alnum = false;
    let mut starts = Vec::new();
    for (at, c) in text.chars().enumerate() {
        if c.is_alphanumeric() && !previous_alnum {
            starts.push((c, at));
        }
        previous_alnum = c.is_alphanumeric();
    }
    let all = text.chars().enumerate().map(|(at, c)| (c, at));
    starts
        .into_iter()
        .chain(all)
        .filter(|(c, _)| c.is_ascii_alphanumeric())
        .map(|(c, at)| (c.to_ascii_uppercase(), at))
        .find(|(c, _)| !used.contains(c))
}
