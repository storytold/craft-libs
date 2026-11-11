//! Replace with text: parsing into pieces and expansion for one match.

use super::pattern::numeric_code;
use super::{FindOptions, PARAGRAPH_MARK, SECTION_MARK};

/// A piece of Replace with text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReplacePiece {
    /// The text to put in, as written.
    Literal(String),
    /// `^&`: the text found.
    Found,
    /// `\n` with wildcards, or `$n` / `${n}` / `\n` with an expression: group `n` (0 is the
    /// match).
    Group(usize),
    /// `${name}` / `$name` with an expression; resolve it with [`resolve_group_names`] and
    /// [`FindPattern::group_index`]; unresolved it expands to nothing.
    Named(String),
    /// `^c`: the clipboard.
    Clipboard,
}

/// Read Replace with text: `^` codes (unless `literal`), `\1`–`\9` with wildcards, and with an
/// expression also `$n`, `${n}`, `${name}`, `$name`, `$$`, `\n` (paragraph mark), `\t` and `\\`.
// wordcraft: crates/engine/src/cmd/find_pattern.rs:384 `parse_replace`.
pub fn parse_replacement(with: &str, options: &FindOptions) -> Vec<ReplacePiece> {
    // With an expression, `literal` turns off only the `^` codes; group references still expand.
    if options.literal && !options.regex {
        return vec![ReplacePiece::Literal(with.to_string())];
    }
    let chars: Vec<char> = with.chars().collect();
    let mut out = Vec::new();
    let mut current = String::new();
    let mut i = 0;
    while let Some(&c) = chars.get(i) {
        i += 1;
        if options.regex && (c == '$' || c == '\\') {
            match expression_reference(&chars, i, c) {
                Some((Reference::Char(ch), used)) => {
                    current.push(ch);
                    i += used;
                }
                Some((Reference::Group(piece), used)) => {
                    flush_literal(&mut current, &mut out);
                    out.extend(piece);
                    i += used;
                }
                None => current.push(c),
            }
            continue;
        }
        if c == '\\' && options.wildcards {
            match chars.get(i) {
                Some(d) if d.is_ascii_digit() && *d != '0' => {
                    flush_literal(&mut current, &mut out);
                    out.push(ReplacePiece::Group(d.to_digit(10).unwrap_or(0) as usize));
                    i += 1;
                    continue;
                }
                Some(&n) => {
                    current.push(n);
                    i += 1;
                    continue;
                }
                None => {}
            }
        }
        if c == '^'
            && !options.literal
            && let Some(used) = caret_code(&chars, i, &mut current, &mut out)
        {
            i += used;
            continue;
        }
        current.push(c);
    }
    flush_literal(&mut current, &mut out);
    out
}

/// [`ReplacePiece::Named`] turned into [`ReplacePiece::Group`] where `index` knows the name.
// wordcraft: crates/engine/src/cmd/find_pattern.rs:491 `resolve_names`.
pub fn resolve_group_names(pieces: Vec<ReplacePiece>, index: &dyn Fn(&str) -> Option<usize>) -> Vec<ReplacePiece> {
    pieces
        .into_iter()
        .filter_map(|piece| match piece {
            ReplacePiece::Named(name) => index(&name).map(ReplacePiece::Group),
            piece => Some(piece),
        })
        .collect()
}

/// The replacement text for one match: `found` is the matched text, `group(n)` its group `n`
/// (`None` expands to nothing), `clipboard` the clipboard's text.
// wordcraft: crates/engine/src/cmd/find_pattern.rs:507 `expand`.
pub fn expand_replacement(pieces: &[ReplacePiece], found: &str, group: &dyn Fn(usize) -> Option<String>, clipboard: &str) -> String {
    // A section mark is inserted again as a plain paragraph mark.
    let as_inserted = |s: &str| s.replace(SECTION_MARK, "\r");
    let mut out = String::new();
    for piece in pieces {
        match piece {
            ReplacePiece::Literal(text) => out.push_str(text),
            ReplacePiece::Found => out.push_str(&as_inserted(found)),
            ReplacePiece::Group(n) => out.push_str(&as_inserted(&group(*n).unwrap_or_default())),
            ReplacePiece::Clipboard => out.push_str(&clipboard.replace("\r\n", "\r").replace('\n', "\r")),
            ReplacePiece::Named(_) => {}
        }
    }
    out
}

/// What a `^` code stands for in Replace with.
fn replace_code(c: char) -> Option<char> {
    Some(match c {
        'p' => PARAGRAPH_MARK,
        't' => '\t',
        '^' => '^',
        'l' => '\n',
        'm' => '\u{0C}',
        'n' => '\u{0E}',
        '+' => '\u{2014}',
        '=' => '\u{2013}',
        's' => '\u{A0}',
        '~' => '\u{2011}',
        '-' => '\u{AD}',
        '%' => '§',
        'v' => '¶',
        _ => return None,
    })
}

/// Pushes the pending literal text, if any.
fn flush_literal(current: &mut String, out: &mut Vec<ReplacePiece>) {
    if !current.is_empty() {
        out.push(ReplacePiece::Literal(std::mem::take(current)));
    }
}

/// The `^` code whose letter is `chars[i]` (`^&`, `^c`, a replace code or a numeric one): pushes
/// its piece and returns the characters it used, or `None` when it isn't a code.
fn caret_code(chars: &[char], i: usize, current: &mut String, out: &mut Vec<ReplacePiece>) -> Option<usize> {
    let &n = chars.get(i)?;
    if n == '&' || n == 'c' {
        flush_literal(current, out);
        out.push(if n == '&' { ReplacePiece::Found } else { ReplacePiece::Clipboard });
        return Some(1);
    }
    if let Some(ch) = replace_code(n) {
        current.push(ch);
        return Some(1);
    }
    let (ch, used) = numeric_code(chars, i)?;
    current.push(ch);
    Some(used)
}

/// What a `$` or `\` starts in Replace with text for an expression.
enum Reference {
    /// An escape: `$$`, `\n`, `\t`, `\\`.
    Char(char),
    /// A group reference; `None` for a number too large to be a group (expands to nothing).
    Group(Option<ReplacePiece>),
}

/// A group named by its number or name, as the `regex` crate reads it.
fn group_ref(name: &str) -> Option<ReplacePiece> {
    if name.bytes().all(|b| b.is_ascii_digit()) { name.parse().ok().map(ReplacePiece::Group) } else { Some(ReplacePiece::Named(name.to_string())) }
}

/// Read a reference or escape after `c` (`$` or `\`) at `chars[i..]`: what it is and how many
/// characters it uses after `c`, or `None` when `c` is a plain character.
fn expression_reference(chars: &[char], i: usize, c: char) -> Option<(Reference, usize)> {
    let next = *chars.get(i)?;
    if c == '\\' {
        return match next {
            'n' => Some((Reference::Char(PARAGRAPH_MARK), 1)),
            't' => Some((Reference::Char('\t'), 1)),
            // `\$` and `\^` keep a `$` or `^` as typed (no group, no `^` code).
            '\\' | '$' | '^' => Some((Reference::Char(next), 1)),
            '1'..='9' => Some((Reference::Group(next.to_digit(10).map(|d| ReplacePiece::Group(d as usize))), 1)),
            _ => None,
        };
    }
    if next == '$' {
        return Some((Reference::Char('$'), 1));
    }
    let rest = chars.get(i..).unwrap_or_default();
    if next == '{' {
        // `${…}`: everything up to the closing brace; unclosed or empty, the `$` is literal.
        let close = rest.iter().position(|&ch| ch == '}')?;
        let name: String = rest.get(1..close)?.iter().collect();
        if name.is_empty() {
            return None;
        }
        return Some((Reference::Group(group_ref(&name)), close + 1));
    }
    // `$name`: the longest run of letters, digits and `_`.
    let len = rest.iter().take_while(|ch| ch.is_ascii_alphanumeric() || **ch == '_').count();
    if len == 0 {
        return None;
    }
    let name: String = rest.get(..len)?.iter().collect();
    Some((Reference::Group(group_ref(&name)), len))
}
