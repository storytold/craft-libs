//! Nouns and their forms: what a language declares (`@grammar` rows) and what each noun is in it (`@noun` rows).
//!
//! - `@grammar ⇥ genders ⇥ masculine feminine neuter` and `@grammar ⇥ forms ⇥ accusative genitive instrumental` declare the
//!   genders and the named forms of a language. A language with no grammatical gender declares none.
//! - `@noun ⇥ page ⇥ feminine: strona; accusative=stronę` gives the translation of the English noun `page`, its gender and its
//!   named forms. A language without gender writes the translation alone (`@noun ⇥ page ⇥ ページ`). A form a noun does not list
//!   is its base form.

use crate::limits::{MAX_FORMS, MAX_NOUNS};

/// The genders and the named forms a language declares.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Grammar {
    /// The genders, in the order the row lists them.
    pub genders: Vec<String>,
    /// The named forms a noun may have besides its base form.
    pub forms: Vec<String>,
}

impl Grammar {
    /// Applies one `@grammar` row: `key` is `genders` or `forms`, `names` the names separated by white space. A later row of the
    /// same key replaces an earlier one, so an app's row wins over a shared one.
    pub fn set(&mut self, key: &str, names: &str) -> Result<(), String> {
        let list: Vec<String> = names.split_whitespace().map(str::to_owned).collect();
        if list.is_empty() {
            return Err(format!("@grammar {key}: no names"));
        }
        if list.len() > MAX_FORMS {
            return Err(format!("@grammar {key}: more than {MAX_FORMS} names"));
        }
        if let Some(bad) = list.iter().find(|n| !is_name(n)) {
            return Err(format!("@grammar {key}: {bad:?} is not a name (lowercase letters, digits and `-`)"));
        }
        if let Some(twice) = list.iter().enumerate().find_map(|(i, n)| list.get(..i).filter(|before| before.contains(n)).map(|_| n)) {
            return Err(format!("@grammar {key}: {twice:?} is listed twice"));
        }
        match key {
            "genders" => self.genders = list,
            "forms" => self.forms = list,
            other => return Err(format!("@grammar: unknown key {other:?} (genders or forms)")),
        }
        Ok(())
    }
}

/// A noun in one language: its base form, its gender if the language has them, and its other named forms.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Noun {
    /// The base form: what a placeholder shows, and what a form the noun does not list shows.
    pub text: String,
    /// The gender, one of the language's `@grammar genders`.
    pub gender: Option<String>,
    /// The named forms the noun lists, as (name, text).
    pub forms: Vec<(String, String)>,
}

impl Noun {
    /// Reads the translation column of a `@noun` row: `[gender:] base [; form=text]…`.
    pub fn parse(translation: &str) -> Result<Noun, String> {
        let mut segments = translation.split(';');
        let first = segments.next().unwrap_or("").trim();
        let (gender, text) = match first.split_once(':') {
            Some((gender, rest)) if is_name(gender.trim()) => (Some(gender.trim().to_owned()), rest.trim()),
            _ => (None, first),
        };
        if text.is_empty() {
            return Err("a noun needs its base form".into());
        }
        let mut forms: Vec<(String, String)> = Vec::new();
        for segment in segments {
            let Some((name, value)) = segment.split_once('=') else {
                return Err(format!("{:?} is not `form=text`", segment.trim()));
            };
            let (name, value) = (name.trim(), value.trim());
            if !is_name(name) || value.is_empty() {
                return Err(format!("{:?} is not `form=text`", segment.trim()));
            }
            if forms.iter().any(|(n, _)| n == name) {
                return Err(format!("form {name:?} is listed twice"));
            }
            if forms.len() >= MAX_FORMS {
                return Err(format!("more than {MAX_FORMS} forms"));
            }
            forms.push((name.to_owned(), value.to_owned()));
        }
        Ok(Noun { text: text.to_owned(), gender, forms })
    }

    /// The form called `name`: its listed text, else the base form.
    pub fn form(&self, name: &str) -> &str {
        self.forms.iter().find(|(n, _)| n == name).map_or(self.text.as_str(), |(_, text)| text.as_str())
    }

    /// Whether the gender and the forms are ones `grammar` declares.
    pub fn check(&self, grammar: &Grammar) -> Result<(), String> {
        if let Some(gender) = &self.gender
            && !grammar.genders.contains(gender)
        {
            return Err(format!("gender {gender:?} is not one of the language's @grammar genders"));
        }
        match self.forms.iter().find(|(name, _)| !grammar.forms.contains(name)) {
            Some((name, _)) => Err(format!("form {name:?} is not one of the language's @grammar forms")),
            None => Ok(()),
        }
    }
}

/// The nouns of one locale, by English noun, capped at [`MAX_NOUNS`].
pub(crate) type Nouns = std::collections::HashMap<String, Noun>;

/// Adds `noun` for `english`; `false` past the cap, when the noun is not kept. A later row of the same noun replaces the earlier.
pub(crate) fn insert(nouns: &mut Nouns, english: String, noun: Noun) -> bool {
    if nouns.len() >= MAX_NOUNS && !nouns.contains_key(&english) {
        return false;
    }
    nouns.insert(english, noun);
    true
}

/// A gender, form or `@grammar` name: lowercase ASCII letters, digits and `-`, starting with a letter, at most 32 bytes.
pub(crate) fn is_name(text: &str) -> bool {
    let mut bytes = text.bytes();
    bytes.next().is_some_and(|b| b.is_ascii_lowercase())
        && text.len() <= 32
        && bytes.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}
