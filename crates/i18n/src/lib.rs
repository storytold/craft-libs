//! Translations for the Craft apps.
//!
//! An app embeds the [craft-i18n](https://github.com/benandez/craft-i18n) catalogs at build time
//! as an [`Embedded`] static, builds one [`I18n`] from it, and looks English source text up in a
//! [`Lang`]. A missing translation is the English itself; nothing here fails.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod catalog;
mod scripts;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Mutex, OnceLock, PoisonError};

use intl_pluralrules::{PluralCategory, PluralRuleType, PluralRules};
use unic_langid::LanguageIdentifier;

pub use scripts::{Direction, Han, Script, base_direction, direction, han, scripts};

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
    /// Itself, then its parent without the region if that is embedded too (`pt-br`, `pt`;
    /// `zh-hant-hk`, `zh-hant`). Empty for English and the pseudo locale.
    chain: Vec<usize>,
    /// The app offers it: in [`I18n::languages`], found by code and system tag. An embedded locale
    /// the app doesn't offer still serves as a fallback.
    offered: bool,
    /// Plural rules and the categories `@plural` forms list, in order.
    plural: Option<(PluralRules, Vec<PluralCategory>)>,
}

/// The embedded catalogs, ready for lookups. Build it once, at start-up.
pub struct I18n {
    locales: Vec<Locale>,
    /// Marked strings for the pseudo locale by source text, leaked so lookups can hand out `&str`.
    /// Debug builds only; the keys are what [`I18n::looked_up`] reports.
    // ponytail: grows by one leak per distinct source text, fine for a test run; an arena if the
    // pseudo locale ever ships.
    pseudo: Mutex<BTreeMap<String, &'static str>>,
    /// [`I18n::system_lang`], read once.
    system: OnceLock<Lang>,
}

impl I18n {
    /// Read `embedded`, offering the locales in `offered` (any case; the app's own language list),
    /// so an app never offers a language its interface isn't ready for (right to left, say). Rows
    /// and locales that can't be used are skipped and come back as problems
    /// (`<locale>/<common|app>: line N: …`) for the app to log.
    pub fn new(embedded: &'static Embedded, offered: &[&str]) -> (I18n, Vec<String>) {
        let mut problems = Vec::new();
        let mut locales = vec![bare("en"), bare("pseudo")];
        for (code, shared, app) in embedded {
            let code = code.to_ascii_lowercase();
            if locales.iter().any(|l| l.code == code) {
                problems.push(format!("{code}: reserved or embedded twice; skipped"));
                continue;
            }
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
            let offered = offered.iter().any(|o| o.eq_ignore_ascii_case(&code));
            locales.push(Locale { code, rows, chain: Vec::new(), plural, offered });
        }
        for i in 2..locales.len() {
            let mut chain = vec![i];
            // Only a region falls back (`pt-br` → `pt`, `zh-hant-hk` → `zh-hant`); a script
            // doesn't, since `sr-latn` → `sr` would show Cyrillic.
            let code = locales.get(i).map_or("", |l| l.code.as_str());
            let base = match code.rsplit_once('-') {
                Some((parent, region)) if is_region(region) => parent,
                _ => "",
            };
            if let Some(b) = locales.iter().skip(2).position(|l| l.code == base)
                && b + 2 != i
            {
                chain.push(b + 2);
            }
            if let Some(l) = locales.get_mut(i) {
                l.chain = chain;
            }
        }
        (I18n { locales, pseudo: Mutex::default(), system: OnceLock::new() }, problems)
    }

    /// The language with this code (any case): `en`, an offered locale, or in debug builds
    /// `pseudo`.
    pub fn from_code(&self, code: &str) -> Option<Lang> {
        let i = self.locales.iter().position(|l| l.code.eq_ignore_ascii_case(code))?;
        let usable = match Lang(i) {
            Lang::EN => true,
            Lang::PSEUDO => cfg!(debug_assertions),
            _ => self.locales.get(i).is_some_and(|l| l.offered),
        };
        usable.then_some(Lang(i))
    }

    /// The language's code, lowercase BCP 47 (`pt-br`).
    pub fn code(&self, lang: Lang) -> &str {
        self.locale(lang).map_or("en", |l| l.code.as_str())
    }

    /// `source` in `lang`, or `source` itself.
    pub fn t<'a>(&'a self, lang: Lang, source: &'a str) -> &'a str {
        if lang == Lang::PSEUDO {
            return self.mark(source);
        }
        self.find(lang, &[("", source)]).unwrap_or(source)
    }

    /// `source` with a disambiguating `context` (`menu`, `@msg`); in each locale of the chain the
    /// context row, then the plain one, so `pt-br`'s plain row beats `pt`'s context row.
    pub fn tc<'a>(&'a self, lang: Lang, context: &str, source: &'a str) -> &'a str {
        if lang == Lang::PSEUDO {
            return self.mark(source);
        }
        self.find(lang, &[(context, source), ("", source)]).unwrap_or(source)
    }

    /// The translation of (`context`, `source`) along `lang`'s chain, with no fallback to the plain
    /// row or the English: for strings that must not borrow another meaning's wording (a key cap's
    /// name under context `key`), or to ask whether a translation exists.
    pub fn get<'a>(&'a self, lang: Lang, context: &str, source: &'a str) -> Option<&'a str> {
        if lang == Lang::PSEUDO {
            return Some(self.mark(source));
        }
        self.find(lang, &[(context, source)])
    }

    /// A command's label by its `id` (`@id` rows); in each locale of the chain the `@id` row, then
    /// the plain row of the English `label`.
    pub fn tid<'a>(&'a self, lang: Lang, id: &str, label: &'a str) -> &'a str {
        if lang == Lang::PSEUDO {
            return self.mark(label);
        }
        self.find(lang, &[("@id", id), ("", label)]).unwrap_or(label)
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
        let text = fmt(text, &[("n", &n.to_string())]);
        if lang == Lang::PSEUDO {
            return marked(&text);
        }
        text
    }

    /// The languages to offer: English first, then the offered locales by code. Never the
    /// pseudo locale.
    pub fn languages(&self) -> Vec<Lang> {
        let mut embedded: Vec<usize> = (2..self.locales.len()).filter(|&i| self.locales.get(i).is_some_and(|l| l.offered)).collect();
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

    /// Whether the language is written right to left ([`direction`]). Putting text into display
    /// order is the app's job.
    pub fn rtl(&self, lang: Lang) -> bool {
        direction(self.code(lang)) == Direction::Rtl
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
    /// languages, the browser's on the web); English if none fits. Read once, then cached.
    pub fn system_lang(&self) -> Lang {
        *self.system.get_or_init(|| {
            // sys-locale drops glibc's script modifier (`sr_RS@latin` → `sr-RS`), so read its variables raw.
            #[cfg(all(unix, not(any(target_vendor = "apple", target_os = "android"))))]
            let tags = posix_env_tags(|k| std::env::var(k).ok());
            #[cfg(not(all(unix, not(any(target_vendor = "apple", target_os = "android")))))]
            let tags: Vec<String> = sys_locale::get_locales().collect();
            self.match_tags(tags.iter().map(String::as_str))
        })
    }

    /// Every source text looked up in the pseudo locale so far, for an audit to check against a
    /// complete catalog. Plurals aren't recorded.
    pub fn looked_up(&self) -> BTreeSet<String> {
        self.pseudo.lock().unwrap_or_else(PoisonError::into_inner).keys().cloned().collect()
    }

    /// `source` marked for the pseudo locale, made once per string.
    fn mark(&self, source: &str) -> &'static str {
        let mut cache = self.pseudo.lock().unwrap_or_else(PoisonError::into_inner);
        cache.entry(source.to_owned()).or_insert_with(|| Box::leak(marked(source).into_boxed_str()))
    }

    fn locale(&self, lang: Lang) -> Option<&Locale> {
        self.locales.get(lang.0)
    }

    /// Along `lang`'s chain, locale by locale, the first of `keys` (context, source text) found.
    fn find(&self, lang: Lang, keys: &[(&str, &str)]) -> Option<&str> {
        let chain = self.locale(lang).map(|l| l.chain.as_slice())?;
        chain.iter().find_map(|&i| {
            let rows = &self.locales.get(i)?.rows;
            keys.iter().find_map(|(context, source)| rows.get(*context)?.get(*source).map(String::as_str))
        })
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
/// Chinese without a script picks one by region; Portuguese falls back to `pt-br` last, when
/// there is no `pt`. A glibc script modifier (`sr_RS@latin`) becomes the script subtag. With a
/// script (`sr-Latn-RS`), a code without one is tried only when that is its language's usual
/// script, so Latin Serbian never picks Cyrillic `sr`, while `sr-Cyrl-RS` does. A region that
/// implies a script (CLDR likely subtags) counts as that script after the tag as written:
/// `pa-PK` → `pa-pk`, `pa-arab-pk`, `pa-arab`.
fn candidates(tag: &str) -> Vec<String> {
    let tag = tag.trim().to_ascii_lowercase();
    let base = tag.split(['.', '@']).next().unwrap_or("").replace('_', "-");
    let mut parts: Vec<&str> = base.split('-').filter(|p| !p.is_empty()).collect();
    let Some(&primary) = parts.first() else {
        return Vec::new();
    };
    let modifier = match tag.split_once('@').map(|(_, m)| m) {
        Some("latin") => Some("latn"),
        Some("cyrillic") => Some("cyrl"),
        _ => None,
    };
    if let Some(s) = modifier {
        parts.insert(1, s);
    }
    let script = parts.get(1).copied().filter(|s| s.len() == 4 && s.bytes().all(|b| b.is_ascii_alphabetic()));
    // Chinese picks its script by region below, keeping plain `zh` as the last resort.
    if script.is_none()
        && primary != "zh"
        && let Some(implied) = parts.get(1).filter(|r| is_region(r)).and_then(|r| scripts::region_script(primary, r))
    {
        let mut out = vec![parts.join("-")];
        parts.insert(1, implied);
        out.extend(candidates(&parts.join("-")));
        return out;
    }
    let mut out: Vec<String> = (1..=parts.len()).rev().map(|n| parts.get(..n).unwrap_or_default().join("-")).collect();
    if let Some(s) = script
        && usual_script(primary).is_some_and(|usual| usual != s)
    {
        out.retain(|c| c.split('-').any(|p| p == s));
    }
    if primary == "zh" && !parts.iter().any(|p| matches!(*p, "hans" | "hant")) {
        let hant = parts.iter().skip(1).any(|p| scripts::region_script("zh", p) == Some("hant"));
        let script = if hant { "zh-hant" } else { "zh-hans" };
        out.insert(out.len().saturating_sub(1), script.to_owned());
    }
    if primary == "pt" && !out.iter().any(|c| c == "pt-br") {
        out.push("pt-br".to_owned());
    }
    out
}

/// The locale variables sys-locale reads on Unix, in its order (`LANGUAGE`'s `:` list, then
/// `LC_ALL`, `LC_MESSAGES`, `LANG`), but unconverted, so `@latin` survives.
#[cfg(all(unix, not(any(target_vendor = "apple", target_os = "android"))))]
fn posix_env_tags(var: impl Fn(&str) -> Option<String>) -> Vec<String> {
    let mut tags: Vec<String> = var("LANGUAGE").map(|v| v.split(':').map(str::to_owned).collect()).unwrap_or_default();
    tags.extend(["LC_ALL", "LC_MESSAGES", "LANG"].into_iter().filter_map(&var));
    tags.retain(|t| !t.is_empty());
    tags
}

/// `⟦text⟧`, each line marked on its own, since multi-line labels are often drawn a line at a time.
fn marked(text: &str) -> String {
    format!("⟦{}⟧", text.replace('\n', "⟧\n⟦"))
}

/// The script a code without one means, for languages written in more than one (CLDR likely
/// subtags). Any other language is taken to have one script.
// ponytail: the multi-script languages a Craft app could plausibly ship; add a row with the locale.
fn usual_script(language: &str) -> Option<&'static str> {
    Some(match language {
        "be" | "kk" | "ky" | "mn" | "sr" | "tg" => "cyrl",
        "zh" => "hans",
        "az" | "bs" | "ha" | "ku" | "ms" | "uz" => "latn",
        "ks" | "sd" | "ug" => "arab",
        "pa" => "guru",
        _ => return None,
    })
}

/// A BCP 47 region subtag: two letters or three digits.
fn is_region(subtag: &str) -> bool {
    (subtag.len() == 2 && subtag.bytes().all(|b| b.is_ascii_alphabetic())) || (subtag.len() == 3 && subtag.bytes().all(|b| b.is_ascii_digit()))
}

/// A locale with no catalogs (English, the pseudo locale).
fn bare(code: &str) -> Locale {
    Locale { code: code.to_owned(), rows: HashMap::new(), chain: Vec::new(), plural: None, offered: false }
}

/// `code`'s CLDR cardinal rules and the categories whole numbers 0..=1000 reach, in CLDR order
/// (zero, one, two, few, many, other): one `@plural` form each. Regional and scripted codes fall
/// back to the language's rules. CLDR's `pt` is Brazilian (0 is singular), so Portuguese other
/// than `pt-br` takes `pt-PT`'s.
fn plural_rules(code: &str) -> Option<(PluralRules, Vec<PluralCategory>)> {
    let code = if code == "pt" || (code.starts_with("pt-") && code != "pt-br") { "pt-pt" } else { code };
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

    /// `embedded`, offering every locale in it.
    fn all(embedded: &'static Embedded) -> (I18n, Vec<String>) {
        let codes: Vec<&str> = embedded.iter().map(|(c, _, _)| *c).collect();
        I18n::new(embedded, &codes)
    }

    fn i18n() -> I18n {
        all(E).0
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
        let (i, problems) = all(E);
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
        let (i, problems) = all(P);
        assert!(problems.is_empty(), "{problems:?}");
        let fr = lang(&i, "fr");
        assert_eq!(i.trn(fr, 0, "{n} page", "{n} pages"), "0 page");
        assert_eq!(i.trn(fr, 2, "{n} page", "{n} pages"), "2 pages");
    }

    #[test]
    fn three_forms_ru() {
        let i = all(P).0;
        let ru = lang(&i, "ru");
        for (n, want) in [(1, "1 файл"), (3, "3 файла"), (5, "5 файлов"), (11, "11 файлов"), (21, "21 файл")] {
            assert_eq!(i.trn(ru, n, "{n} file", "{n} files"), want);
        }
    }

    #[test]
    fn one_form_ja() {
        let i = all(P).0;
        assert_eq!(i.trn(lang(&i, "ja"), 7, "{n} file", "{n} files"), "7 個");
    }

    #[test]
    fn english_fallback() {
        let i = all(P).0;
        assert_eq!(i.trn(Lang::EN, 1, "{n} file", "{n} files"), "1 file");
        assert_eq!(i.trn(lang(&i, "fr"), 2, "{n} x", "{n} xs"), "2 xs");
    }

    #[test]
    fn beyond_sweep() {
        let i = all(P).0;
        assert_eq!(i.trn(lang(&i, "fr"), 1_000_000, "{n} page", "{n} pages"), "1000000 pages");
    }

    static L: &Embedded =
        &[("fr", "@id\tlanguage.name\tFrançais\n", ""), ("pt-br", "", ""), ("zh-hans", "", ""), ("zh-hant", "", ""), ("ar", "", "")];

    #[test]
    fn languages_order() {
        let i = all(L).0;
        let codes: Vec<&str> = i.languages().into_iter().map(|l| i.code(l)).collect();
        assert_eq!(codes, ["en", "ar", "fr", "pt-br", "zh-hans", "zh-hant"]);
    }

    #[test]
    fn names() {
        let i = all(L).0;
        assert_eq!(i.name(Lang::EN), "English");
        assert_eq!(i.name(lang(&i, "fr")), "Français");
        assert_eq!(i.name(lang(&i, "ar")), "ar");
    }

    #[test]
    fn rtl() {
        let i = all(L).0;
        assert!(i.rtl(lang(&i, "ar")));
        assert!(!i.rtl(lang(&i, "fr")));
        assert!(!i.rtl(Lang::EN));
    }

    #[test]
    fn tags() {
        let i = all(L).0;
        for (tag, want) in [
            ("fr_CA.UTF-8", Some("fr")),
            ("pt-PT", Some("pt-br")),
            ("pt_BR", Some("pt-br")),
            ("zh_TW.UTF-8", Some("zh-hant")),
            ("zh-HK", Some("zh-hant")),
            ("zh", Some("zh-hans")),
            ("zh-Hant-MO", Some("zh-hant")),
            ("zh_CN", Some("zh-hans")),
            ("zh-US", Some("zh-hant")),
            ("zh_SG", Some("zh-hans")),
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
    fn region_implies_script() {
        assert_eq!(candidates("pa-PK"), ["pa-pk", "pa-arab-pk", "pa-arab"]);
        assert_eq!(candidates("pa_PK.UTF-8"), ["pa-pk", "pa-arab-pk", "pa-arab"]);
        assert_eq!(candidates("pa-IN"), ["pa-in", "pa"]);
        assert_eq!(candidates("sd-IN"), ["sd-in", "sd-deva-in", "sd-deva"]);
        assert_eq!(candidates("pa-Guru-PK"), ["pa-guru-pk", "pa-guru", "pa"]);
        assert_eq!(candidates("zh-US"), ["zh-us", "zh-hant", "zh"]);
        assert_eq!(candidates("zh-SG"), ["zh-sg", "zh-hans", "zh"]);
        assert_eq!(candidates("kk-CN"), ["kk-cn", "kk-arab-cn", "kk-arab"]);
        assert_eq!(candidates("kk-KZ"), ["kk-kz", "kk"]);
        assert_eq!(candidates("ug-KZ"), ["ug-kz", "ug-cyrl-kz", "ug-cyrl"]);
    }

    #[test]
    fn match_tags_takes_first_match() {
        let i = all(L).0;
        assert_eq!(i.match_tags(["ja_JP", "fr_FR"]), lang(&i, "fr"));
        assert_eq!(i.match_tags(["ja"]), Lang::EN);
        assert_eq!(i.match_tags([]), Lang::EN);
    }

    #[test]
    fn pseudo_marks_every_lookup() {
        let i = i18n();
        assert_eq!(i.t(Lang::PSEUDO, "Save"), "⟦Save⟧");
        assert_eq!(i.t(Lang::PSEUDO, "A\nB"), "⟦A⟧\n⟦B⟧");
        assert_eq!(i.tc(Lang::PSEUDO, "menu", "Open"), "⟦Open⟧");
        assert_eq!(i.tid(Lang::PSEUDO, "file.save", "Save"), "⟦Save⟧");
        assert_eq!(i.trn(Lang::PSEUDO, 2, "{n} x", "{n} xs"), "⟦2 xs⟧");
        assert!(!i.languages().contains(&Lang::PSEUDO));
        assert_eq!(i.from_code("pseudo"), cfg!(debug_assertions).then_some(Lang::PSEUDO));
        assert_eq!(i.get(Lang::PSEUDO, "key", "Esc"), Some("⟦Esc⟧"));
        assert_eq!(i.looked_up().into_iter().collect::<Vec<_>>(), ["A\nB", "Esc", "Open", "Save"]);
    }

    static F: &Embedded = &[
        ("pt", "", ""),
        ("pt-br", "", ""),
        ("sr", "\tSave\tСачувај\n", ""),
        ("sr-latn", "", ""),
        ("fr", "\tDelete\tSupprimer\nkey\tEsc\tÉchap\n", ""),
    ];

    #[test]
    fn european_portuguese_prefers_pt() {
        let i = all(F).0;
        for tag in ["pt", "pt-PT", "pt_PT.UTF-8"] {
            assert_eq!(i.match_tag(tag).map(|l| i.code(l)), Some("pt"), "{tag}");
        }
        assert_eq!(i.match_tag("pt_BR").map(|l| i.code(l)), Some("pt-br"));
    }

    #[test]
    fn script_locale_does_not_fall_back_across_scripts() {
        let i = all(F).0;
        assert_eq!(i.t(lang(&i, "sr-latn"), "Save"), "Save");
    }

    #[test]
    fn glibc_script_modifier_picks_the_script() {
        let i = all(F).0;
        for (tag, want) in [("sr_RS.UTF-8@latin", Some("sr-latn")), ("sr_RS@latin", Some("sr-latn")), ("sr_RS.UTF-8", Some("sr"))] {
            assert_eq!(i.match_tag(tag).map(|l| i.code(l)), want, "{tag}");
        }
        let cyrillic_only = all(&[("sr", "", "")]).0;
        assert_eq!(cyrillic_only.match_tag("sr_RS@latin"), None);
    }

    #[cfg(all(unix, not(any(target_vendor = "apple", target_os = "android"))))]
    #[test]
    fn posix_env_keeps_the_modifier() {
        let env = |vars: &'static [(&'static str, &'static str)]| move |k: &str| vars.iter().find(|(n, _)| *n == k).map(|(_, v)| (*v).to_owned());
        assert_eq!(posix_env_tags(env(&[("LANG", "sr_RS.UTF-8@latin")])), ["sr_RS.UTF-8@latin"]);
        assert_eq!(posix_env_tags(env(&[("LANGUAGE", "fr:de"), ("LC_ALL", ""), ("LANG", "sr_RS@latin")])), ["fr", "de", "sr_RS@latin"]);
        let i = all(F).0;
        assert_eq!(i.code(i.match_tags(posix_env_tags(env(&[("LANG", "sr_RS.UTF-8@latin")])).iter().map(String::as_str))), "sr-latn");
    }

    #[test]
    fn get_has_no_plain_fallback() {
        let i = all(F).0;
        let fr = lang(&i, "fr");
        assert_eq!(i.get(fr, "key", "Esc"), Some("Échap"));
        assert_eq!(i.get(fr, "key", "Delete"), None);
        assert_eq!(i.get(fr, "", "Delete"), Some("Supprimer"));
        assert_eq!(i.get(Lang::EN, "", "Delete"), None);
    }

    static C: &Embedded = &[("pt", "menu\tOpen\tAbrir menu\n@id\tfile.open\tAbrir ficheiro\n", ""), ("pt-br", "\tOpen\tAbrir\n", "")];

    #[test]
    fn locale_before_context() {
        let i = all(C).0;
        let br = lang(&i, "pt-br");
        assert_eq!(i.tc(br, "menu", "Open"), "Abrir");
        assert_eq!(i.tid(br, "file.open", "Open"), "Abrir");
        assert_eq!(i.tc(lang(&i, "pt"), "menu", "Open"), "Abrir menu");
    }

    static PT: &Embedded = &[
        ("pt", "@plural\t{n} file|{n} files\t{n} ficheiro|{n} ficheiros\n", ""),
        ("pt-br", "@plural\t{n} file|{n} files\t{n} arquivo|{n} arquivos\n", ""),
    ];

    #[test]
    fn european_portuguese_zero_is_plural() {
        let i = all(PT).0;
        assert_eq!(i.trn(lang(&i, "pt"), 0, "{n} file", "{n} files"), "0 ficheiros");
        assert_eq!(i.trn(lang(&i, "pt"), 1, "{n} file", "{n} files"), "1 ficheiro");
        assert_eq!(i.trn(lang(&i, "pt-br"), 0, "{n} file", "{n} files"), "0 arquivo");
    }

    /// The same counts as craft-i18n's checker (`plural_form_counts`), which rejects a catalog
    /// whose `@plural` rows list any other.
    #[test]
    fn plural_form_counts() {
        for (locale, n) in
            [("en", 2), ("fr", 2), ("ja", 1), ("vi", 1), ("ru", 3), ("pl", 3), ("cs", 3), ("sl", 4), ("ar", 6), ("pt", 2), ("pt-br", 2), ("tr", 2)]
        {
            assert_eq!(plural_rules(locale).map(|(_, c)| c.len()), Some(n), "{locale}");
        }
    }

    #[test]
    fn script_subtag_never_picks_another_script() {
        let i = all(&[("sr", "", ""), ("zh-hant", "", ""), ("fr", "", ""), ("be", "", "")]).0;
        for (tag, want) in [
            ("sr-Latn-RS", None),
            ("sr-Cyrl-RS", Some("sr")),
            ("zh-Hant-TW", Some("zh-hant")),
            ("zh-Hans", None),
            ("fr-Latn-FR", Some("fr")),
            ("be_BY@latin", None),
            ("be_BY", Some("be")),
        ] {
            assert_eq!(i.match_tag(tag).map(|l| i.code(l)), want, "{tag}");
        }
    }

    #[test]
    fn duplicate_row_keeps_the_first() {
        let (i, problems) = all(&[("fr", "\tSave\tEnregistrer\n\tSave\tSauver\n", "")]);
        assert_eq!(problems, ["fr/common: line 2: defined twice (same context and source text); the first is kept"]);
        assert_eq!(i.t(lang(&i, "fr"), "Save"), "Enregistrer");
    }

    #[test]
    fn reserved_and_repeated_codes_are_skipped() {
        let (i, problems) = all(&[("en", "\tSave\tX\n", ""), ("pseudo", "", ""), ("fr", "\tSave\tEnregistrer\n", ""), ("FR", "\tSave\tY\n", "")]);
        assert_eq!(problems.len(), 3, "{problems:?}");
        assert_eq!(i.t(Lang::EN, "Save"), "Save");
        assert_eq!(i.t(lang(&i, "fr"), "Save"), "Enregistrer");
        assert_eq!(i.languages().len(), 2);
    }

    #[test]
    fn only_offered_locales_are_offered() {
        let (i, _) = I18n::new(&[("ar", "\tSave\tحفظ\n", ""), ("pt", "\tUndo\tDesfazer\n", ""), ("pt-br", "", "")], &["PT-BR"]);
        let codes: Vec<&str> = i.languages().into_iter().map(|l| i.code(l)).collect();
        assert_eq!(codes, ["en", "pt-br"]);
        assert_eq!(i.from_code("ar"), None);
        assert_eq!(i.match_tag("ar_EG"), None);
        assert_eq!(i.match_tag("pt_PT"), i.from_code("pt-br"));
        assert_eq!(i.t(lang(&i, "pt-br"), "Undo"), "Desfazer");
    }

    #[test]
    fn region_after_script_falls_back() {
        let i = all(&[("zh-hant", "\tSave\t儲存\n", ""), ("zh-hant-hk", "", "")]).0;
        assert_eq!(i.t(lang(&i, "zh-hant-hk"), "Save"), "儲存");
    }

    #[test]
    fn more_right_to_left() {
        let i = all(&[("ps", "", ""), ("ckb", "", ""), ("yi", "", ""), ("ug", "", "")]).0;
        assert!(i.languages().into_iter().skip(1).all(|l| i.rtl(l)));
    }

    /// Prints what this machine reports, so CI logs show `system_lang` on each platform.
    #[test]
    fn system_lang_reads_the_system() {
        let i = all(&[("fr", "", ""), ("de", "", ""), ("pt-br", "", ""), ("sr-latn", "", "")]).0;
        let first = i.system_lang();
        println!("sys-locale: {:?}; system_lang: {}", sys_locale::get_locales().collect::<Vec<_>>(), i.code(first));
        assert_eq!(i.system_lang(), first);
    }
}
