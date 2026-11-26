//! Reading one catalog. The rules are craft-i18n's `docs/catalog-format.md`; craft-i18n's CI
//! applies them strictly, and here a row that breaks one is skipped and reported, so an app built
//! against a newer catalog still runs, with that string in English.
//!
//! Beyond the classic rows (`@id`, `@plural`, `@msg`, `@msgplural`, and plain rows) a catalog may hold `@ordinal` rows (the forms
//! of an ordinal, `one|two|few|other` in English), `@grammar` rows (a language's genders and named forms), `@noun` rows, and
//! translations that are selector messages ([`crate::select`]). Every file is read under the caps of [`crate::limits`].

use std::collections::HashSet;

use crate::limits::{MAX_CATALOG_BYTES, MAX_ROW_BYTES, MAX_ROWS};
use crate::message::Template;
use crate::noun::{Grammar, Noun};
use crate::select::{Locale as RuleLocale, Message, RowKind, check_message};

/// A row that passed, with its columns unescaped.
pub(crate) struct Entry {
    pub context: String,
    pub source: String,
    pub translation: String,
}

/// The contexts with a meaning here. Any other context starting with `@` is skipped.
const RESERVED: &[&str] = &["@plural", "@ordinal", "@id", "@msg", "@msgplural", "@grammar", "@noun"];

/// What a locale's rows are checked against: its form counts, plural rules, declared grammar and direction.
pub(crate) struct Shape<'a> {
    /// Cardinal forms a `@plural` row lists.
    pub cardinal: usize,
    /// Ordinal forms an `@ordinal` row lists.
    pub ordinal: usize,
    pub locale: RuleLocale<'a>,
}

/// The rows of one catalog that passed.
#[derive(Default)]
pub(crate) struct Catalog {
    /// Plain translations, including `@grammar` rows (read by [`grammar`]) and excluding the rows below.
    pub entries: Vec<Entry>,
    /// Rows whose translation is a selector message, parsed.
    pub messages: Vec<(Entry, Message)>,
    /// `@noun` rows: the English noun and its words.
    pub nouns: Vec<(String, Noun)>,
    /// One problem (`line N: …`) per row skipped, and for what a cap cut off.
    pub problems: Vec<String>,
}

/// The `@grammar` rows of the catalogs of one locale, in order (a later one replaces an earlier of the same key); a row that
/// fails is left out here and reported by [`parse`].
pub(crate) fn grammar<'t>(texts: impl IntoIterator<Item = &'t str>) -> Grammar {
    let mut grammar = Grammar::default();
    for text in texts {
        if text.len() > MAX_CATALOG_BYTES {
            continue;
        }
        for line in text.split('\n').filter(|l| l.starts_with("@grammar\t")) {
            let columns: Vec<&str> = line.strip_suffix('\r').unwrap_or(line).split('\t').collect();
            if let [_, key, names] = columns.as_slice() {
                // A bad row is reported by `parse`; here it only fails to declare.
                let _ = grammar.set(key, names);
            }
        }
    }
    grammar
}

/// The rows of a catalog read for a locale of `shape`, and one problem (`line N: …`) per row skipped. Of a row defined twice, the
/// first is kept, as in craft-i18n's checker.
pub(crate) fn parse(text: &str, shape: &Shape<'_>) -> Catalog {
    let mut catalog = Catalog::default();
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    if text.len() > MAX_CATALOG_BYTES {
        catalog.problems.push(format!("larger than {MAX_CATALOG_BYTES} bytes; the catalog is not used"));
        return catalog;
    }
    let mut seen = HashSet::new();
    let mut rows = 0_usize;
    for (i, line) in text.split('\n').enumerate() {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        rows += 1;
        if rows > MAX_ROWS {
            catalog.problems.push(format!("line {}: more than {MAX_ROWS} rows; the rest are skipped", i + 1));
            break;
        }
        if line.len() > MAX_ROW_BYTES {
            catalog.problems.push(format!("line {}: row longer than {MAX_ROW_BYTES} bytes", i + 1));
            continue;
        }
        match parse_row(line, shape) {
            Ok(_) if !seen.insert(key_of(line)) => {
                catalog.problems.push(format!("line {}: defined twice (same context and source text); the first is kept", i + 1));
            }
            Ok(Parsed::Entry(entry)) => catalog.entries.push(entry),
            Ok(Parsed::Message(entry, message)) => catalog.messages.push((entry, message)),
            Ok(Parsed::Noun(english, noun)) => catalog.nouns.push((english, noun)),
            Err(e) => catalog.problems.push(format!("line {}: {e}", i + 1)),
        }
    }
    catalog
}

/// The first two columns of a row as written, which make its identity.
fn key_of(line: &str) -> (&str, &str) {
    let mut columns = line.splitn(3, '\t');
    (columns.next().unwrap_or(""), columns.next().unwrap_or(""))
}

/// What a row is once read.
pub(crate) enum Parsed {
    Entry(Entry),
    Message(Entry, Message),
    Noun(String, Noun),
}

pub(crate) fn parse_row(text: &str, shape: &Shape<'_>) -> Result<Parsed, String> {
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
    let complex = Message::is_complex(&row.translation);
    let kind = match row.context.as_str() {
        "@plural" => RowKind::Plural,
        "@ordinal" => RowKind::Ordinal,
        _ => RowKind::Plain,
    };
    match row.context.as_str() {
        "@id" | "@msgplural" if complex => return Err("a selector message is not supported in this kind of row".into()),
        "@id" => {}
        "@plural" => {
            check_plural_source(&row.source)?;
            if complex {
                return selector(row, kind, shape);
            }
            check_forms(&row.source, &row.translation, shape.cardinal, "plural")?;
        }
        "@ordinal" => {
            check_ordinal_source(&row.source)?;
            if complex {
                return selector(row, kind, shape);
            }
            check_forms(&row.source, &row.translation, shape.ordinal, "ordinal")?;
        }
        "@msgplural" => {
            check_plural(&row.source, &row.translation, shape.cardinal)?;
            row.source.split('|').try_for_each(check_matchable)?;
        }
        "@grammar" => {
            Grammar::default().set(&row.source, &row.translation)?;
        }
        "@noun" => {
            if row.source.contains(['{', '}']) {
                return Err("a noun is a plain word: no placeholders".into());
            }
            let noun = Noun::parse(&row.translation)?;
            noun.check(shape.locale.grammar)?;
            return Ok(Parsed::Noun(row.source, noun));
        }
        c if c.starts_with('@') && !RESERVED.contains(&c) => {
            return Err(format!("unknown reserved context {c:?}"));
        }
        _ => {
            if complex {
                if row.context == "@msg" {
                    check_matchable(&row.source)?;
                }
                return selector(row, kind, shape);
            }
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
    Ok(Parsed::Entry(row))
}

/// A row whose translation is a selector message: it must meet the rules of [`check_message`].
fn selector(row: Entry, kind: RowKind, shape: &Shape<'_>) -> Result<Parsed, String> {
    let message = check_message(kind, &row.source, &row.translation, &shape.locale)?;
    Ok(Parsed::Message(row, message))
}

/// A message is matched against its English text, so a template with no literal text or two
/// placeholders in a row (where one ends and the next begins is unknowable) could never be used.
fn check_matchable(english: &str) -> Result<(), String> {
    Template::parse(english, Vec::new(), false).map(|_| ()).ok_or_else(|| "unmatchable message template".to_string())
}

/// The source of a `@plural` row: `one|other`, nonempty, with the same placeholders.
fn check_plural_source(source: &str) -> Result<(), String> {
    let english: Vec<&str> = source.split('|').collect();
    let [one, other] = english.as_slice() else {
        return Err("plural source text must be one|other".into());
    };
    if one.is_empty() || other.is_empty() || placeholders(one) != placeholders(other) {
        return Err("plural source forms must be nonempty with the same placeholders".into());
    }
    Ok(())
}

/// The source of an `@ordinal` row: `one|two|few|other`, nonempty, with the same placeholders.
fn check_ordinal_source(source: &str) -> Result<(), String> {
    let english: Vec<&str> = source.split('|').collect();
    let [one, two, few, other] = english.as_slice() else {
        return Err("ordinal source text must be one|two|few|other".into());
    };
    if [one, two, few, other].iter().any(|f| f.is_empty()) || [one, two, few].iter().any(|f| placeholders(f) != placeholders(other)) {
        return Err("ordinal source forms must be nonempty with the same placeholders".into());
    }
    Ok(())
}

/// The forms of a `@plural` or `@ordinal` translation: `forms` of them, nonempty, each with the source's placeholders.
fn check_forms(source: &str, translation: &str, forms: usize, what: &str) -> Result<(), String> {
    let other = source.rsplit('|').next().unwrap_or(source);
    let translated: Vec<&str> = translation.split('|').collect();
    if translated.len() != forms || translated.iter().any(|f| f.is_empty()) {
        return Err(format!("expected {forms} nonempty {what} forms"));
    }
    if translated.iter().any(|f| placeholders(f) != placeholders(other)) {
        return Err(format!("{what} form placeholders differ from the source text"));
    }
    Ok(())
}

fn check_plural(source: &str, translation: &str, forms: usize) -> Result<(), String> {
    check_plural_source(source)?;
    check_forms(source, translation, forms, "plural")
}

fn unescape(text: &str) -> Result<String, String> {
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

/// `{name}` placeholders, sorted: translations may reorder them. A typed placeholder (`{n :number}`) is its name.
fn placeholders(text: &str) -> Vec<&str> {
    let mut names = Vec::new();
    let mut rest = text;
    while let Some((_, after)) = rest.split_once('{') {
        let Some((name, after)) = after.split_once('}') else {
            break;
        };
        names.push(name.split_whitespace().next().unwrap_or(name));
        rest = after;
    }
    names.sort_unstable();
    names
}

/// The translation of an `@id` row written as `line` (`None` for any other line, or a row that has an invisible character, an
/// unknown escape or an empty column), for reading a language's name without parsing its catalog.
pub(crate) fn id_translation(line: &str) -> Option<String> {
    let [context, source, translation] = line.split('\t').collect::<Vec<_>>()[..] else { return None };
    if context != "@id" || line.chars().any(|c| c != '\t' && (c.is_control() || c == '\u{feff}')) {
        return None;
    }
    let (source, translation) = (unescape(source).ok()?, unescape(translation).ok()?);
    (!source.is_empty() && !translation.is_empty()).then_some(translation)
}
