//! Reading one catalog. The rules are craft-i18n's `docs/catalog-format.md`; craft-i18n's CI
//! applies them strictly, and here a row that breaks one is skipped and reported, so an app built
//! against a newer catalog still runs, with that string in English.
//!
//! [`I18n`](crate::I18n) reads the embedded catalogs with these functions. An app's build script
//! or test that reads, merges or writes catalog rows can use them to read rows exactly as the
//! lookups do: [`parse`] for a catalog, [`parse_row`] for one line, and [`escape`] and [`unescape`]
//! for the columns. [`plural_forms`](crate::plural_forms) gives the `forms` argument for a locale.

use std::collections::HashSet;

use crate::message::Template;

/// A row that passed, with its columns unescaped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// The context column: empty, a context such as `menu`, or `@plural`, `@id`, `@msg` or
    /// `@msgplural`.
    pub context: String,
    /// The English source text; for `@plural` and `@msgplural`, its two forms joined by `|`.
    pub source: String,
    /// The translation; for `@plural` and `@msgplural`, its forms joined by `|`.
    pub translation: String,
}

/// The contexts with a meaning here. Any other context starting with `@` is skipped.
const RESERVED: &[&str] = &["@plural", "@id", "@msg", "@msgplural"];

/// The rows of a catalog whose locale has `forms` plural forms, and one problem (`line N: …`) per
/// row skipped. Of a row defined twice, the first is kept, as in craft-i18n's checker. A leading
/// byte order mark, `\r` line ends, empty lines and `#` comment lines are skipped.
pub fn parse(text: &str, forms: usize) -> (Vec<Entry>, Vec<String>) {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut rows = Vec::new();
    let mut problems = Vec::new();
    let mut seen = HashSet::new();
    for (i, line) in text.split('\n').enumerate() {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        match parse_row(line, forms) {
            Ok(row) if !seen.insert((row.context.clone(), row.source.clone())) => {
                problems.push(format!("line {}: defined twice (same context and source text); the first is kept", i + 1));
            }
            Ok(row) => rows.push(row),
            Err(e) => problems.push(format!("line {}: {e}", i + 1)),
        }
    }
    (rows, problems)
}

/// One row, `context⇥source⇥translation` without its line end, checked as [`parse`] checks it;
/// the error is the problem `parse` reports after `line N: `.
pub fn parse_row(text: &str, forms: usize) -> Result<Entry, String> {
    let columns: Vec<&str> = text.split('\t').collect();
    let [context, source, translation] = columns.as_slice() else {
        return Err(format!("{} columns; expected 3", columns.len()));
    };
    if text.chars().any(|c| c != '\t' && (c.is_control() || c == '\u{feff}')) {
        return Err("invisible character in the row".into());
    }
    let row = Entry { context: unescape(context)?, source: unescape(source)?, translation: unescape(translation)? };
    if row.source.is_empty() || row.translation.is_empty() {
        return Err("empty source text or translation".into());
    }
    match row.context.as_str() {
        "@id" => {}
        "@plural" => check_plural(&row.source, &row.translation, forms)?,
        "@msgplural" => {
            check_plural(&row.source, &row.translation, forms)?;
            row.source.split('|').try_for_each(check_matchable)?;
        }
        c if c.starts_with('@') && !RESERVED.contains(&c) => {
            return Err(format!("unknown reserved context {c:?}"));
        }
        _ => {
            if placeholders(&row.source) != placeholders(&row.translation) {
                return Err("placeholders differ from the source text".into());
            }
            if row.source.ends_with('…') != row.translation.ends_with('…') {
                return Err("trailing … differs from the source text".into());
            }
            if row.context == "@msg" {
                check_matchable(&row.source)?;
            }
        }
    }
    Ok(row)
}

/// A message is matched against its English text, so a template with no literal text or two
/// placeholders in a row (where one ends and the next begins is unknowable) could never be used.
fn check_matchable(english: &str) -> Result<(), String> {
    Template::parse(english, Vec::new(), false).map(|_| ()).ok_or_else(|| "unmatchable message template".to_string())
}

fn check_plural(source: &str, translation: &str, forms: usize) -> Result<(), String> {
    let english: Vec<&str> = source.split('|').collect();
    let [one, other] = english.as_slice() else {
        return Err("plural source text must be one|other".into());
    };
    if one.is_empty() || other.is_empty() || placeholders(one) != placeholders(other) {
        return Err("plural source forms must be nonempty with the same placeholders".into());
    }
    let translated: Vec<&str> = translation.split('|').collect();
    if translated.len() != forms || translated.iter().any(|f| f.is_empty()) {
        return Err(format!("expected {forms} nonempty plural forms"));
    }
    if translated.iter().any(|f| placeholders(f) != placeholders(other)) {
        return Err("plural form placeholders differ from the source text".into());
    }
    Ok(())
}

/// A column as a catalog writes it: `\` as `\\`, a line break as `\n` and a tab as `\t`. Other
/// characters are kept, so a column with another control character still gives a row that
/// [`parse_row`] rejects.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out
}

/// A column as a catalog writes it, read back: `\n`, `\t` and `\\` are the only escapes, and any
/// other backslash is an error.
pub fn unescape(text: &str) -> Result<String, String> {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('\\') => out.push('\\'),
            Some(other) => return Err(format!("unknown escape \\{other}")),
            None => return Err("trailing backslash".into()),
        }
    }
    Ok(out)
}

/// `{name}` placeholders, sorted: translations may reorder them.
fn placeholders(text: &str) -> Vec<&str> {
    let mut names = Vec::new();
    let mut rest = text;
    while let Some((_, after)) = rest.split_once('{') {
        let Some((name, after)) = after.split_once('}') else {
            break;
        };
        names.push(name);
        rest = after;
    }
    names.sort_unstable();
    names
}
