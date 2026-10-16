//! Translations for the Craft apps.
//!
//! An app embeds the [craft-i18n](https://github.com/benandez/craft-i18n) catalogs at build time
//! as an [`Embedded`] static, builds one [`I18n`] from it, and looks English source text up in a
//! [`Lang`]. A missing translation is the English itself; nothing here fails.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod catalog;

use std::collections::HashMap;

use intl_pluralrules::{PluralCategory, PluralRuleType, PluralRules};
use unic_langid::LanguageIdentifier;

/// The catalogs an app embeds, one entry per locale: (locale, shared catalog, the app's catalog).
/// A missing catalog is `""`.
pub type Embedded = [(&'static str, &'static str, &'static str)];

/// A language the interface can be shown in: English, the pseudo locale, or an embedded locale.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Lang(usize);

impl Lang {
    /// English: the source text itself.
    pub const EN: Lang = Lang(0);
    /// A test-only locale that marks every looked-up string.
    pub const PSEUDO: Lang = Lang(1);
}

/// One locale's translations: context → source text → translation, app rows over shared ones.
struct Locale {
    code: String,
    rows: HashMap<String, HashMap<String, String>>,
    /// Itself, then its base language if that is embedded too (`pt-br`, `pt`). Empty for English
    /// and the pseudo locale.
    chain: Vec<usize>,
    /// Plural rules and the categories `@plural` forms list, in order.
    #[expect(dead_code, reason = "read by plural lookups")]
    plural: Option<(PluralRules, Vec<PluralCategory>)>,
}

/// The embedded catalogs, ready for lookups. Build it once, at start-up.
pub struct I18n {
    locales: Vec<Locale>,
}

impl I18n {
    /// Read `embedded`. Rows that can't be used are skipped and come back as problems
    /// (`<locale>/<common|app>: line N: …`) for the app to log.
    pub fn new(embedded: &'static Embedded) -> (I18n, Vec<String>) {
        let mut problems = Vec::new();
        let mut locales = vec![bare("en"), bare("pseudo")];
        for (code, shared, app) in embedded {
            let code = code.to_ascii_lowercase();
            let plural = plural_rules(&code);
            if plural.is_none() {
                problems.push(format!("{code}: no plural rules; plurals show in English"));
            }
            let forms = plural.as_ref().map_or(1, |(_, c)| c.len());
            let mut rows: HashMap<String, HashMap<String, String>> = HashMap::new();
            for (name, text) in [("common", shared), ("app", app)] {
                let (entries, errors) = catalog::parse(text, forms);
                problems.extend(errors.into_iter().map(|e| format!("{code}/{name}: {e}")));
                for e in entries {
                    rows.entry(e.context).or_default().insert(e.source, e.translation);
                }
            }
            locales.push(Locale { code, rows, chain: Vec::new(), plural });
        }
        for i in 2..locales.len() {
            let mut chain = vec![i];
            let base = locales.get(i).and_then(|l| l.code.split('-').next()).unwrap_or_default();
            if let Some(b) = locales.iter().skip(2).position(|l| l.code == base)
                && b + 2 != i
            {
                chain.push(b + 2);
            }
            if let Some(l) = locales.get_mut(i) {
                l.chain = chain;
            }
        }
        (I18n { locales }, problems)
    }

    /// The language with this code (any case): `en`, `pseudo`, or an embedded locale.
    pub fn from_code(&self, code: &str) -> Option<Lang> {
        self.locales.iter().position(|l| l.code.eq_ignore_ascii_case(code)).map(Lang)
    }

    /// The language's code, lowercase BCP 47 (`pt-br`).
    pub fn code(&self, lang: Lang) -> &str {
        self.locale(lang).map_or("en", |l| l.code.as_str())
    }

    /// `source` in `lang`, or `source` itself.
    pub fn t<'a>(&'a self, lang: Lang, source: &'a str) -> &'a str {
        self.find(lang, "", source).unwrap_or(source)
    }

    /// `source` with a disambiguating `context` (`menu`, `@msg`); falls back to [`I18n::t`].
    pub fn tc<'a>(&'a self, lang: Lang, context: &str, source: &'a str) -> &'a str {
        self.find(lang, context, source).unwrap_or_else(|| self.t(lang, source))
    }

    /// A command's label by its `id` (`@id` rows); falls back to [`I18n::t`] of the English `label`.
    pub fn tid<'a>(&'a self, lang: Lang, id: &str, label: &'a str) -> &'a str {
        self.find(lang, "@id", id).unwrap_or_else(|| self.t(lang, label))
    }

    fn locale(&self, lang: Lang) -> Option<&Locale> {
        self.locales.get(lang.0)
    }

    /// The first translation of (`context`, `source`) along `lang`'s chain.
    fn find(&self, lang: Lang, context: &str, source: &str) -> Option<&str> {
        let chain = self.locale(lang).map(|l| l.chain.as_slice())?;
        chain.iter().find_map(|&i| self.locales.get(i)?.rows.get(context)?.get(source).map(String::as_str))
    }
}

/// A locale with no catalogs (English, the pseudo locale).
fn bare(code: &str) -> Locale {
    Locale { code: code.to_owned(), rows: HashMap::new(), chain: Vec::new(), plural: None }
}

/// `code`'s CLDR cardinal rules and the categories whole numbers 0..=1000 reach, in CLDR order
/// (zero, one, two, few, many, other): one `@plural` form each. Regional and scripted codes fall
/// back to the language's rules.
fn plural_rules(code: &str) -> Option<(PluralRules, Vec<PluralCategory>)> {
    let id: LanguageIdentifier = code.parse().ok()?;
    let language = LanguageIdentifier::from_parts(id.language, None, None, &[]);
    let rules = PluralRules::create(id, PluralRuleType::CARDINAL).or_else(|_| PluralRules::create(language, PluralRuleType::CARDINAL)).ok()?;
    let mut categories = Vec::new();
    for n in 0..=1000_u32 {
        let c = rules.select(n).ok()?;
        if !categories.contains(&c) {
            categories.push(c);
        }
    }
    categories.sort_by_key(rank);
    Some((rules, categories))
}

fn rank(c: &PluralCategory) -> u8 {
    match c {
        PluralCategory::ZERO => 0,
        PluralCategory::ONE => 1,
        PluralCategory::TWO => 2,
        PluralCategory::FEW => 3,
        PluralCategory::MANY => 4,
        PluralCategory::OTHER => 5,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static E: &Embedded = &[
        ("fr", "\tSave\tEnregistrer\n\tOpen\tOuvrir\nmenu\tOpen\tOuvrir le menu\n@id\tfile.save\tEnregistrer le fichier\n", "\tSave\tSauvegarder\n"),
        ("pt", "\tSave\tGuardar\n\tUndo\tDesfazer\n", ""),
        ("pt-br", "\tSave\tSalvar\n", ""),
        ("de", "\tBad\n@nope\tA\tB\n\tUndo\tRückgängig\n", ""),
    ];

    fn i18n() -> I18n {
        I18n::new(E).0
    }

    fn lang(i: &I18n, code: &str) -> Lang {
        i.from_code(code).unwrap()
    }

    #[test]
    fn app_row_overrides_shared() {
        let i = i18n();
        assert_eq!(i.t(lang(&i, "fr"), "Save"), "Sauvegarder");
    }

    #[test]
    fn shared_row_when_no_override() {
        let i = i18n();
        assert_eq!(i.t(lang(&i, "fr"), "Open"), "Ouvrir");
    }

    #[test]
    fn region_falls_back_to_base() {
        let i = i18n();
        assert_eq!(i.t(lang(&i, "pt-br"), "Undo"), "Desfazer");
        assert_eq!(i.t(lang(&i, "pt-br"), "Save"), "Salvar");
    }

    #[test]
    fn missing_is_english() {
        let i = i18n();
        assert_eq!(i.t(lang(&i, "fr"), "Close"), "Close");
        assert_eq!(i.t(Lang::EN, "Save"), "Save");
    }

    #[test]
    fn context_then_plain() {
        let i = i18n();
        assert_eq!(i.tc(lang(&i, "fr"), "menu", "Open"), "Ouvrir le menu");
        assert_eq!(i.tc(lang(&i, "fr"), "menu", "Save"), "Sauvegarder");
    }

    #[test]
    fn id_then_label() {
        let i = i18n();
        assert_eq!(i.tid(lang(&i, "fr"), "file.save", "Save"), "Enregistrer le fichier");
        assert_eq!(i.tid(lang(&i, "fr"), "file.nope", "Open"), "Ouvrir");
    }

    #[test]
    fn bad_rows_skipped_and_reported() {
        let (i, problems) = I18n::new(E);
        assert_eq!(problems.len(), 2, "{problems:?}");
        assert!(problems[0].starts_with("de/common: line 1:"), "{problems:?}");
        assert!(problems[1].starts_with("de/common: line 2:"), "{problems:?}");
        assert_eq!(i.t(lang(&i, "de"), "Undo"), "Rückgängig");
    }

    #[test]
    fn from_code_is_case_insensitive() {
        let i = i18n();
        assert_eq!(i.from_code("PT-BR"), i.from_code("pt-br"));
        assert_eq!(i.from_code("xx"), None);
        assert_eq!(i.from_code("en"), Some(Lang::EN));
        assert_eq!(i.code(Lang::EN), "en");
    }
}
