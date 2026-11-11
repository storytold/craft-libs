//! Translations for the Craft apps.
//!
//! An app embeds the [craft-i18n](https://github.com/benandez/craft-i18n) catalogs at build time
//! as an [`Embedded`] static, builds one [`I18n`] from it, and looks English source text up in a
//! [`Lang`]. A missing translation is the English itself; nothing here fails.
//!
//! Counts ([`I18n::trn`]), file sizes and measures write their numbers as the formatting
//! [`Region`] does ([`I18n::format`]): grouped (`1.234`, `12,34,567`) and in the region's digits
//! (`١٬٢٣٤`). Text that goes into documents or file names should use [`I18n::trn_plain`], which
//! writes `0`–`9` without grouping.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod age;
mod catalog;
#[cfg(feature = "collate")]
mod collate;
mod date;
mod fold;
mod format;
pub mod matcher;
mod number;
mod region;
mod scripts;
mod size;
mod words;

mod bidi;
mod negotiation;

use bidi::*;
use negotiation::*;

pub use bidi::{isolate, ltr, strip_isolates};

/// The CLDR number, plural and date data for every CLDR locale, from `data/generate.sh`.
mod data {
    use std::sync::OnceLock;

    use icu_locale_fallback::LocaleFallbacker;
    use icu_provider_adapters::fallback::LocaleFallbackProvider;
    use icu_provider_blob::BlobDataProvider;

    include!("../data/locales.rs");

    /// The number and plural data, read once; `None` if the blob doesn't load.
    pub(crate) fn provider() -> Option<&'static LocaleFallbackProvider<BlobDataProvider>> {
        static PROVIDER: OnceLock<Option<LocaleFallbackProvider<BlobDataProvider>>> = OnceLock::new();
        PROVIDER
            .get_or_init(|| {
                let blob = BlobDataProvider::try_new_from_static_blob(include_bytes!("../data/numbers.postcard")).ok()?;
                Some(LocaleFallbackProvider::new(blob, LocaleFallbacker::new().static_to_owned()))
            })
            .as_ref()
    }

    /// The date, time, name and week data (with the digits they use), read once; `None` if the
    /// blob doesn't load.
    pub(crate) fn dates_provider() -> Option<&'static LocaleFallbackProvider<BlobDataProvider>> {
        static PROVIDER: OnceLock<Option<LocaleFallbackProvider<BlobDataProvider>>> = OnceLock::new();
        PROVIDER
            .get_or_init(|| {
                let blob = BlobDataProvider::try_new_from_static_blob(include_bytes!("../data/dates.postcard")).ok()?;
                Some(LocaleFallbackProvider::new(blob, LocaleFallbacker::new().static_to_owned()))
            })
            .as_ref()
    }
}

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, Mutex, OnceLock, PoisonError, RwLock};

use icu_locale_core::Locale as IcuLocale;
use icu_plurals::{PluralCategory, PluralOperands, PluralRules};
use unicode_bidi::{BidiClass, bidi_class};

pub use age::{AGE_SHOWN_AS_DATE, AgeStyle, relative_age};
#[cfg(feature = "collate")]
pub use collate::Collator;
pub use date::{
    CalendarDate, CalendarKind, DateFormat, DateLength, DateStyle, DateTime, LocalDate, NameLength, TimePrecision, TimeZone, Weekday, utc_stamp,
};
pub use fold::{Folded, Folder};
pub use format::{Digits, Format};
pub use number::{NumberError, parse_number};
pub use region::{HourCycle, Measurement, Paper, Region};
pub use scripts::{Direction, Han, Script, base_direction, direction, han, scripts};
pub use size::{FileSize, SizeUnit};
pub use words::{Count, Counted, Word, WordRows, WordSet, WordTemplate, Words};

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
    /// The shared and the app's catalog text, parsed on first use.
    shared: Cow<'static, str>,
    app: Cow<'static, str>,
    /// Plural forms `@plural` rows must list, for [`catalog::parse`].
    forms: usize,
    parsed: OnceLock<Parsed>,
    /// [`I18n::name`], read from the text on first use.
    name: OnceLock<Option<String>>,
    /// Itself, then its parent without the region if that is embedded too (`pt-br`, `pt`;
    /// `zh-hant-hk`, `zh-hant`). Empty for English and the pseudo locale.
    chain: Vec<usize>,
    /// The app offers it: in [`I18n::languages`], found by code and system tag. An embedded locale
    /// the app doesn't offer still serves as a fallback.
    offered: bool,
    /// Plural rules and the categories `@plural` forms list, in order.
    plural: Option<(PluralRules, Vec<PluralCategory>)>,
}

/// A locale's rows, app over shared, and what was wrong with the catalogs it came from.
struct Parsed {
    rows: HashMap<String, HashMap<String, String>>,
    problems: Vec<String>,
}

impl Locale {
    /// The rows; the catalogs are parsed on the first call and any problems logged then.
    fn parsed(&self) -> &Parsed {
        self.parsed.get_or_init(|| {
            let mut problems = Vec::new();
            let mut rows: HashMap<String, HashMap<String, String>> = HashMap::new();
            for (name, text) in [("common", &self.shared), ("app", &self.app)] {
                let (entries, errors) = catalog::parse(text, self.forms);
                problems.extend(errors.into_iter().map(|e| format!("{}/{name}: {e}", self.code)));
                for e in entries {
                    rows.entry(e.context).or_default().insert(e.source, e.translation);
                }
            }
            problems.iter().for_each(|p| log::warn!("translation catalog: {p}; row skipped"));
            Parsed { rows, problems }
        })
    }

    /// The `@id language.name` row of the app's catalog, else the shared one, found by scanning the
    /// text so the locale needn't be parsed.
    fn language_name(&self) -> Option<&str> {
        self.name
            .get_or_init(|| {
                [&*self.app, &*self.shared].into_iter().find_map(|text| {
                    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
                    text.split('\n')
                        .filter(|line| line.starts_with("@id\tlanguage.name\t"))
                        .find_map(|line| catalog::parse_row(line.strip_suffix('\r').unwrap_or(line), self.forms).ok())
                        .map(|row| row.translation)
                })
            })
            .as_deref()
    }
}

/// The embedded catalogs, ready for lookups. Build it once, at start-up. Each locale is parsed on
/// its first lookup, which is also when its bad rows are logged.
pub struct I18n {
    locales: Vec<Locale>,
    /// Marked strings for the pseudo locale by source text, leaked so lookups can hand out `&str`.
    /// Debug builds only; the keys are what [`I18n::looked_up`] reports.
    // ponytail: grows by one leak per distinct source text, fine for a test run; an arena if the
    // pseudo locale ever ships.
    pseudo: Mutex<BTreeMap<String, &'static str>>,
    /// [`I18n::system_lang`], read once.
    system: OnceLock<Lang>,
    /// [`I18n::format`]; the system region's until [`I18n::set_format`].
    format: RwLock<Option<Arc<Format>>>,
    /// [`I18n::dates`]; the system region's until [`I18n::set_dates`].
    dates: RwLock<Option<Arc<DateFormat>>>,
}

impl I18n {
    /// Index `embedded` without parsing it, offering the locales in `offered` (any case; the app's
    /// own language list), so an app never offers a language its interface isn't ready for (right
    /// to left, say). Locales that can't be used are skipped and come back as problems for the app
    /// to log. A locale's bad rows are logged (`log::warn!`) when it is first looked up in, and
    /// [`I18n::check_all`] lists them all.
    pub fn new(embedded: &'static Embedded, offered: &[&str]) -> (I18n, Vec<String>) {
        Self::build(embedded.iter().map(|&(code, shared, app)| (code.into(), shared.into(), app.into())), offered)
    }

    /// [`I18n::new`] for catalogs the app holds as owned text (unpacked from a compressed bundle,
    /// read from disk): (locale, shared catalog, the app's catalog), `""` for a missing one.
    pub fn from_owned(catalogs: impl IntoIterator<Item = (String, String, String)>, offered: &[&str]) -> (I18n, Vec<String>) {
        Self::build(catalogs.into_iter().map(|(code, shared, app)| (code.into(), shared.into(), app.into())), offered)
    }

    fn build(catalogs: impl Iterator<Item = (Cow<'static, str>, Cow<'static, str>, Cow<'static, str>)>, offered: &[&str]) -> (I18n, Vec<String>) {
        let mut problems = Vec::new();
        let mut locales = vec![bare("en"), bare("pseudo")];
        for (code, shared, app) in catalogs {
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
            let offered = offered.iter().any(|o| o.eq_ignore_ascii_case(&code));
            locales.push(Locale { code, shared, app, forms, parsed: OnceLock::new(), name: OnceLock::new(), chain: Vec::new(), plural, offered });
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
        (I18n { locales, pseudo: Mutex::default(), system: OnceLock::new(), format: RwLock::default(), dates: RwLock::default() }, problems)
    }

    /// Parse every locale and list every problem, `<locale>/<common|app>: line N: …`, for a test or
    /// an audit; the same ones [`I18n::new`] leaves to lookups to log.
    pub fn check_all(&self) -> Vec<String> {
        self.locales.iter().flat_map(|l| l.parsed().problems.iter().cloned()).collect()
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

    /// A count's wording in `lang`, `{n}` filled in as [`I18n::format`] writes numbers: grouped and
    /// in the region's digits, as a left-to-right isolate ([`ltr`]) when the wording has a
    /// right-to-left letter. `one` and `other` are the English forms; they are also the key of the
    /// `@plural` row, and the fallback.
    pub fn trn(&self, lang: Lang, n: u64, one: &str, other: &str) -> String {
        let format = self.format();
        self.plural(lang, Some(PluralOperands::from(n)), n == 1, &format.integer(n), one, other)
    }

    /// [`I18n::trn`] with `{n}` in `0`–`9` without grouping (`Copy 1234`) and no isolate marks
    /// ([`strip_isolates`]), for text that goes into documents, file names or anything read back
    /// later.
    pub fn trn_plain(&self, lang: Lang, n: u64, one: &str, other: &str) -> String {
        strip_isolates(&self.plural(lang, Some(PluralOperands::from(n)), n == 1, &n.to_string(), one, other))
    }

    /// [`I18n::trn`] for a count with `decimals` digits after the decimal separator (`1.5 files`).
    /// The plural form follows the digits shown, so `1.0` takes English `other`; infinities and NaN
    /// (`∞`, `NaN`) take `other` too.
    pub fn trn_decimal(&self, lang: Lang, n: f64, decimals: usize, one: &str, other: &str) -> String {
        let format = self.format();
        let Some(d) = format::to_decimal(n, decimals) else {
            return self.plural(lang, None, false, &format.number(n, decimals), one, other);
        };
        let english_one = d.to_string() == "1";
        self.plural(lang, Some(PluralOperands::from(&d)), english_one, &format.write(&d), one, other)
    }

    /// `operands` `None` is the `other` category.
    fn plural(&self, lang: Lang, operands: Option<PluralOperands>, english_one: bool, n: &str, one: &str, other: &str) -> String {
        let key = format!("{one}|{other}");
        let chain = self.locale(lang).map_or(&[][..], |l| l.chain.as_slice());
        let text = chain
            .iter()
            .find_map(|&i| {
                let locale = self.locales.get(i)?;
                let forms = locale.parsed().rows.get("@plural")?.get(&key)?;
                let (rules, categories) = locale.plural.as_ref()?;
                let category = operands.map_or(PluralCategory::Other, |o| rules.category_for(o));
                // A category whole numbers up to 1000 never reach has no form of its own. Russian
                // and Ukrainian decimals (`other`) take the `few` form (`1,5 файла`); any other
                // (French `many` for a million) takes the last.
                let index = categories
                    .iter()
                    .position(|c| *c == category)
                    .or_else(|| categories.iter().position(|c| *c == PluralCategory::Few))
                    .unwrap_or(categories.len().saturating_sub(1));
                forms.split('|').nth(index)
            })
            .unwrap_or(if english_one { one } else { other });
        let text = fmt(text, &[("n", &ltr(n))]);
        if lang == Lang::PSEUDO {
            return marked(&text);
        }
        text
    }

    /// How counts, file sizes and measures write numbers: [`Region::system`]'s format with the
    /// locale's digits until [`I18n::set_format`], and again after [`I18n::reset_format`].
    pub fn format(&self) -> Arc<Format> {
        if let Some(f) = self.format.read().unwrap_or_else(PoisonError::into_inner).as_ref() {
            return Arc::clone(f);
        }
        let mut slot = self.format.write().unwrap_or_else(PoisonError::into_inner);
        Arc::clone(slot.get_or_insert_with(|| Arc::new(Format::new(&Region::system(), Digits::Locale))))
    }

    /// Write numbers and dates in `region` with `digits`: the user's region setting, in one call
    /// so the two cannot differ.
    pub fn set_region(&self, region: &Region, digits: Digits) {
        self.set_format(Format::new(region, digits));
        self.set_dates(DateFormat::new(region).with_digits(digits));
    }

    /// Write numbers as `format` does from now on: the user's region setting, or Latin digits.
    pub fn set_format(&self, format: Format) {
        *self.format.write().unwrap_or_else(PoisonError::into_inner) = Some(Arc::new(format));
    }

    /// Write numbers as the system region does again, undoing [`I18n::set_format`].
    pub fn reset_format(&self) {
        *self.format.write().unwrap_or_else(PoisonError::into_inner) = None;
    }

    /// The languages to offer: English first, then the offered locales by code. Never the
    /// pseudo locale.
    pub fn languages(&self) -> Vec<Lang> {
        let mut embedded: Vec<usize> = (2..self.locales.len()).filter(|&i| self.locales.get(i).is_some_and(|l| l.offered)).collect();
        embedded.sort_by_key(|&i| self.locales.get(i).map(|l| l.code.as_str()));
        std::iter::once(Lang::EN).chain(embedded.into_iter().map(Lang)).collect()
    }

    /// The language's name in itself, for a language picker: its `@id language.name` row (the
    /// app's catalog over the shared one), or the code when there is none. Doesn't parse the
    /// locale.
    pub fn name(&self, lang: Lang) -> &str {
        if lang == Lang::EN {
            return "English";
        }
        self.locale(lang).and_then(Locale::language_name).unwrap_or_else(|| self.code(lang))
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
            let rows = &self.locales.get(i)?.parsed().rows;
            keys.iter().find_map(|(context, source)| rows.get(*context)?.get(*source).map(String::as_str))
        })
    }
}

/// Fill `{name}` placeholders from `args` in one pass. When the template or an inserted value
/// has a right-to-left letter (Hebrew, Arabic and the like), or a value has a bidi control
/// (an embedding, override, isolate or mark), each value goes in as a first-strong isolate
/// (FSI…PDI), so its direction can't reorder the sentence around it: a Latin file name in an
/// Arabic message stays where the translator put it, and an override in a file name ends with
/// the name. Embeddings, overrides and isolates a value opens are closed inside its isolate. A
/// value that already is one isolate keeps it; pass numbers, paths and codes through [`ltr`].
/// Other text gets no marks, and a value's own isolate is taken off. Unknown placeholders stay as written,
/// and inserted values are never read as templates, so a file name containing `{n}` stays intact.
pub fn fmt(template: &str, args: &[(&str, &str)]) -> String {
    fill(template, args, true)
}

/// [`fmt`] without isolates, for text that goes where the marks would get in the way: file
/// names, logs, terminals, text compared or parsed later. Every isolate mark is taken off, in
/// the template and in the values.
pub fn fmt_plain(template: &str, args: &[(&str, &str)]) -> String {
    strip_isolates(&fill(template, args, false))
}

fn fill(template: &str, args: &[(&str, &str)], isolated: bool) -> String {
    let mut pieces: Vec<(&str, bool)> = Vec::new();
    let mut rest = template;
    while let Some((before, after_open)) = rest.split_once('{') {
        pieces.push((before, false));
        let Some((name, after_close)) = after_open.split_once('}') else {
            pieces.push(("{", false));
            rest = after_open;
            break;
        };
        match args.iter().find(|(k, _)| *k == name) {
            Some((_, value)) => pieces.push((value, true)),
            None => pieces.extend([("{", false), (name, false), ("}", false)]),
        }
        rest = after_close;
    }
    pieces.push((rest, false));
    let isolated = isolated && pieces.iter().any(|(text, value)| has_rtl(text) || (*value && has_bidi_control(text)));
    let mut out = String::with_capacity(template.len());
    for (text, value) in pieces {
        match (value, isolated) {
            (false, _) => out.push_str(text),
            (true, false) => out.push_str(unwrap_isolate(text)),
            (true, true) if text.is_empty() => {}
            // Rebuilt rather than copied, so what it opens inside is closed too.
            (true, true) if is_one_isolate(text) => {
                let initiator = text.chars().next().unwrap_or(FSI);
                out.push_str(&wrap(initiator, unwrap_isolate(text)));
            }
            (true, true) => out.push_str(&isolate(text)),
        }
    }
    out
}

#[cfg(test)]
mod tests;
