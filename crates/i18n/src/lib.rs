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

    /// A count's wording in `lang`, `{n}` filled in. `one` and `other` are the English forms;
    /// they are also the key of the `@plural` row, and the fallback.
    pub fn trn(&self, lang: Lang, n: u64, one: &str, other: &str) -> String {
        let key = format!("{one}|{other}");
        let chain = self.locale(lang).map_or(&[][..], |l| l.chain.as_slice());
        let text = chain
            .iter()
            .find_map(|&i| {
                let locale = self.locales.get(i)?;
                let forms = locale.rows.get("@plural")?.get(&key)?;
                let (rules, categories) = locale.plural.as_ref()?;
                let category = rules.select(n).ok()?;
                // A category whole numbers up to 1000 never reach (French `many`) takes the last form.
                let index = categories.iter().position(|c| *c == category).unwrap_or(categories.len().saturating_sub(1));
                forms.split('|').nth(index)
            })
            .unwrap_or(if n == 1 { one } else { other });
        fmt(text, &[("n", &n.to_string())])
    }

    /// The languages to offer: English first, then the embedded locales by code. Never the
    /// pseudo locale.
    pub fn languages(&self) -> Vec<Lang> {
        let mut embedded: Vec<usize> = (2..self.locales.len()).collect();
        embedded.sort_by_key(|&i| self.locales.get(i).map(|l| l.code.as_str()));
        std::iter::once(Lang::EN).chain(embedded.into_iter().map(Lang)).collect()
    }

    /// The language's name in itself, for a language picker: its shared catalog's
    /// `@id language.name` row, or the code when there is none.
    pub fn name(&self, lang: Lang) -> &str {
        if lang == Lang::EN {
            return "English";
        }
        self.locale(lang).and_then(|l| l.rows.get("@id")?.get("language.name")).map_or_else(|| self.code(lang), String::as_str)
    }

    /// Whether the language is written right to left. Putting text into display order is the
    /// app's job.
    pub fn rtl(&self, lang: Lang) -> bool {
        let code = self.code(lang);
        let language = code.split('-').next().unwrap_or(code);
        matches!(language, "ar" | "fa" | "he" | "ur")
    }

    /// The language for a system locale tag (`fr_CA.UTF-8`, `zh-Hant-MO`); `C` and `POSIX` are
    /// English. `None` when no embedded language fits.
    pub fn match_tag(&self, tag: &str) -> Option<Lang> {
        let candidates = candidates(tag);
        if matches!(candidates.first().map(String::as_str), Some("c" | "posix")) {
            return Some(Lang::EN);
        }
        candidates.iter().filter(|c| *c != "pseudo").find_map(|c| self.from_code(c))
    }

    /// The first of `tags` (most preferred first) that [`I18n::match_tag`] knows; English if none.
    pub fn match_tags<'t>(&self, tags: impl IntoIterator<Item = &'t str>) -> Lang {
        tags.into_iter().find_map(|t| self.match_tag(t)).unwrap_or(Lang::EN)
    }

    /// The language the system asks for (`LANG` and friends, the macOS and Windows preferred
    /// languages, the browser's on the web); English if none fits.
    pub fn system_lang(&self) -> Lang {
        let tags: Vec<String> = sys_locale::get_locales().collect();
        self.match_tags(tags.iter().map(String::as_str))
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

/// Fill `{name}` placeholders from `args` in one pass. Unknown placeholders stay as written, and
/// inserted values are never read as templates, so a file name containing `{n}` stays intact.
pub fn fmt(template: &str, args: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some((before, after_open)) = rest.split_once('{') {
        out.push_str(before);
        let Some((name, after_close)) = after_open.split_once('}') else {
            out.push('{');
            rest = after_open;
            break;
        };
        match args.iter().find(|(k, _)| *k == name) {
            Some((_, value)) => out.push_str(value),
            None => {
                out.push('{');
                out.push_str(name);
                out.push('}');
            }
        }
        rest = after_close;
    }
    out.push_str(rest);
    out
}

/// Codes to try for a locale tag, most specific first: `zh_TW.UTF-8` → `zh-tw`, `zh-hant`, `zh`.
/// Chinese without a script picks one by region; Portuguese regions without a catalog use `pt-br`.
fn candidates(tag: &str) -> Vec<String> {
    let base = tag.trim().split(['.', '@']).next().unwrap_or("").replace('_', "-").to_ascii_lowercase();
    let parts: Vec<&str> = base.split('-').filter(|p| !p.is_empty()).collect();
    let Some(&primary) = parts.first() else {
        return Vec::new();
    };
    let mut out: Vec<String> = (1..=parts.len()).rev().map(|n| parts.get(..n).unwrap_or_default().join("-")).collect();
    if primary == "zh" && !parts.iter().any(|p| matches!(*p, "hans" | "hant")) {
        let script = if parts.iter().any(|p| matches!(*p, "tw" | "hk" | "mo")) { "zh-hant" } else { "zh-hans" };
        out.insert(out.len().saturating_sub(1), script.to_owned());
    }
    if primary == "pt" && !out.iter().any(|c| c == "pt-br") {
        out.insert(out.len().saturating_sub(1), "pt-br".to_owned());
    }
    out
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

    static P: &Embedded = &[
        ("fr", "@plural\t{n} page|{n} pages\t{n} page|{n} pages\n", ""),
        ("ru", "@plural\t{n} file|{n} files\t{n} файл|{n} файла|{n} файлов\n", ""),
        ("ja", "@plural\t{n} file|{n} files\t{n} 個\n", ""),
    ];

    #[test]
    fn fmt_fills_once() {
        assert_eq!(fmt("{a} to {b}", &[("a", "x"), ("b", "{a}")]), "x to {a}");
        assert_eq!(fmt("{z} {", &[]), "{z} {");
    }

    #[test]
    fn one_other() {
        let (i, problems) = I18n::new(P);
        assert!(problems.is_empty(), "{problems:?}");
        let fr = lang(&i, "fr");
        assert_eq!(i.trn(fr, 0, "{n} page", "{n} pages"), "0 page");
        assert_eq!(i.trn(fr, 2, "{n} page", "{n} pages"), "2 pages");
    }

    #[test]
    fn three_forms_ru() {
        let i = I18n::new(P).0;
        let ru = lang(&i, "ru");
        for (n, want) in [(1, "1 файл"), (3, "3 файла"), (5, "5 файлов"), (11, "11 файлов"), (21, "21 файл")] {
            assert_eq!(i.trn(ru, n, "{n} file", "{n} files"), want);
        }
    }

    #[test]
    fn one_form_ja() {
        let i = I18n::new(P).0;
        assert_eq!(i.trn(lang(&i, "ja"), 7, "{n} file", "{n} files"), "7 個");
    }

    #[test]
    fn english_fallback() {
        let i = I18n::new(P).0;
        assert_eq!(i.trn(Lang::EN, 1, "{n} file", "{n} files"), "1 file");
        assert_eq!(i.trn(lang(&i, "fr"), 2, "{n} x", "{n} xs"), "2 xs");
    }

    #[test]
    fn beyond_sweep() {
        let i = I18n::new(P).0;
        assert_eq!(i.trn(lang(&i, "fr"), 1_000_000, "{n} page", "{n} pages"), "1000000 pages");
    }

    static L: &Embedded =
        &[("fr", "@id\tlanguage.name\tFrançais\n", ""), ("pt-br", "", ""), ("zh-hans", "", ""), ("zh-hant", "", ""), ("ar", "", "")];

    #[test]
    fn languages_order() {
        let i = I18n::new(L).0;
        let codes: Vec<&str> = i.languages().into_iter().map(|l| i.code(l)).collect();
        assert_eq!(codes, ["en", "ar", "fr", "pt-br", "zh-hans", "zh-hant"]);
    }

    #[test]
    fn names() {
        let i = I18n::new(L).0;
        assert_eq!(i.name(Lang::EN), "English");
        assert_eq!(i.name(lang(&i, "fr")), "Français");
        assert_eq!(i.name(lang(&i, "ar")), "ar");
    }

    #[test]
    fn rtl() {
        let i = I18n::new(L).0;
        assert!(i.rtl(lang(&i, "ar")));
        assert!(!i.rtl(lang(&i, "fr")));
        assert!(!i.rtl(Lang::EN));
    }

    #[test]
    fn tags() {
        let i = I18n::new(L).0;
        for (tag, want) in [
            ("fr_CA.UTF-8", Some("fr")),
            ("pt-PT", Some("pt-br")),
            ("pt_BR", Some("pt-br")),
            ("zh_TW.UTF-8", Some("zh-hant")),
            ("zh-HK", Some("zh-hant")),
            ("zh", Some("zh-hans")),
            ("zh-Hant-MO", Some("zh-hant")),
            ("zh_CN", Some("zh-hans")),
            ("C", Some("en")),
            ("POSIX", Some("en")),
            ("en_US", Some("en")),
            ("ja_JP", None),
            ("", None),
            ("@@", None),
        ] {
            assert_eq!(i.match_tag(tag).map(|l| i.code(l)), want, "{tag}");
        }
    }

    #[test]
    fn match_tags_takes_first_match() {
        let i = I18n::new(L).0;
        assert_eq!(i.match_tags(["ja_JP", "fr_FR"]), lang(&i, "fr"));
        assert_eq!(i.match_tags(["ja"]), Lang::EN);
        assert_eq!(i.match_tags([]), Lang::EN);
    }
}
