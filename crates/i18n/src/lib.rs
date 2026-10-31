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

mod catalog;
#[cfg(feature = "collate")]
mod collate;
mod fold;
mod format;
mod number;
mod region;
mod scripts;
mod size;

/// The CLDR number and plural data for every CLDR locale, from `data/generate.sh`.
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
}

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, Mutex, OnceLock, PoisonError, RwLock};

use icu_locale_core::Locale as IcuLocale;
use icu_plurals::{PluralCategory, PluralOperands, PluralRules};
use unicode_bidi::{BidiClass, bidi_class};

#[cfg(feature = "collate")]
pub use collate::Collator;
pub use fold::{Folded, Folder};
pub use format::{Digits, Format};
pub use number::{NumberError, parse_number};
pub use region::{HourCycle, Measurement, Paper, Region};
pub use scripts::{Direction, Han, Script, base_direction, direction, han, scripts};
pub use size::{FileSize, SizeUnit};

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
    shared: &'static str,
    app: &'static str,
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
            for (name, text) in [("common", self.shared), ("app", self.app)] {
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
                [self.app, self.shared].into_iter().find_map(|text| {
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
}

impl I18n {
    /// Index `embedded` without parsing it, offering the locales in `offered` (any case; the app's
    /// own language list), so an app never offers a language its interface isn't ready for (right
    /// to left, say). Locales that can't be used are skipped and come back as problems for the app
    /// to log. A locale's bad rows are logged (`log::warn!`) when it is first looked up in, and
    /// [`I18n::check_all`] lists them all.
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
        (I18n { locales, pseudo: Mutex::default(), system: OnceLock::new(), format: RwLock::default() }, problems)
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

const LRI: char = '\u{2066}';
const RLI: char = '\u{2067}';
const FSI: char = '\u{2068}';
const PDI: char = '\u{2069}';
const LRE: char = '\u{202A}';
const RLE: char = '\u{202B}';
const PDF: char = '\u{202C}';
const LRO: char = '\u{202D}';
const RLO: char = '\u{202E}';
const LRM: char = '\u{200E}';
const RLM: char = '\u{200F}';
const ALM: char = '\u{061C}';

/// `value` as a first-strong isolate (FSI…PDI): it takes the direction of its first strong
/// letter and can't reorder the text around it. What [`fmt`] does to each value in text with a
/// right-to-left letter.
pub fn isolate(value: &str) -> String {
    wrap(FSI, value)
}

/// `value` as a left-to-right isolate (LRI…PDI), for numbers, paths, file sizes, timecodes and
/// codes, which read left to right in every language. In an RTL first-strong isolate, the
/// segments of `/home/مستخدم/file.txt` would run right to left. [`fmt`] takes the isolate off
/// again in text with no right-to-left letter.
pub fn ltr(value: &str) -> String {
    wrap(LRI, value)
}

/// `text` without isolate marks (LRI, RLI, FSI, PDI): a [`fmt`] or [`I18n::trn`] result made fit
/// for a file name, a log or a comparison.
pub fn strip_isolates(text: &str) -> String {
    text.chars().filter(|c| !matches!(*c, LRI | RLI | FSI | PDI)).collect()
}

/// `value` between `initiator` and PDI, with every embedding, override and isolate it opens
/// closed inside: a stray PDF or PDI is dropped, and open ones are closed in order. Under the
/// bidi algorithm the outer PDI alone would close them, but a renderer that handles only
/// embeddings would let them run on into the rest of the text.
fn wrap(initiator: char, value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 6);
    out.push(initiator);
    // One entry per open initiator: true for an isolate, false for an embedding or override.
    let mut open: Vec<bool> = Vec::new();
    for c in value.chars() {
        match c {
            LRI | RLI | FSI => open.push(true),
            LRE | RLE | LRO | RLO => open.push(false),
            PDF if open.last() == Some(&false) => {
                open.pop();
            }
            PDF => continue,
            PDI if open.contains(&true) => {
                while open.pop() == Some(false) {
                    out.push(PDF);
                }
            }
            PDI => continue,
            _ => {}
        }
        out.push(c);
    }
    while let Some(isolate) = open.pop() {
        out.push(if isolate { PDI } else { PDF });
    }
    out.push(PDI);
    out
}

/// Whether `value`, without its own isolate, has a bidi control: an embedding, override,
/// isolate or mark that could reorder the text around it unless it is contained.
fn has_bidi_control(value: &str) -> bool {
    !value.is_ascii() && unwrap_isolate(value).chars().any(|c| matches!(c, LRE | RLE | PDF | LRO | RLO | LRI | RLI | FSI | PDI | LRM | RLM | ALM))
}

/// Whether `text` has a strong right-to-left character (bidi class R or AL): the text that needs
/// isolates.
fn has_rtl(text: &str) -> bool {
    !text.is_ascii() && text.chars().any(|c| matches!(bidi_class(c), BidiClass::R | BidiClass::AL))
}

/// `value` without its isolate, if it is exactly one.
fn unwrap_isolate(value: &str) -> &str {
    if is_one_isolate(value) {
        // The initiator and PDI are three bytes each in UTF-8.
        value.get(3..value.len() - 3).unwrap_or(value)
    } else {
        value
    }
}

/// Whether `value` is exactly one isolate: an initiator, then the PDI that closes it last.
fn is_one_isolate(value: &str) -> bool {
    let mut depth = 0_usize;
    let mut chars = value.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            LRI | RLI | FSI => depth += 1,
            PDI if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    return chars.peek().is_none();
                }
            }
            _ if depth == 0 => return false,
            _ => {}
        }
    }
    false
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
    Locale {
        code: code.to_owned(),
        shared: "",
        app: "",
        forms: 1,
        parsed: OnceLock::new(),
        name: OnceLock::new(),
        chain: Vec::new(),
        plural: None,
        offered: false,
    }
}

/// `code`'s CLDR cardinal rules and the categories whole numbers 0..=1000 reach, in CLDR order
/// (zero, one, two, few, many, other): one `@plural` form each. The rules are looked up by
/// language alone: CLDR's parent of `sr-Latn` and `zh-Hant` is the root, whose only category is
/// `other`. CLDR's `pt` is Brazilian (0 is singular), so Portuguese other than `pt-br` takes
/// `pt-PT`'s. `None` for a language CLDR has no plural rules for.
fn plural_rules(code: &str) -> Option<(PluralRules, Vec<PluralCategory>)> {
    let id = IcuLocale::try_from_str(code).ok()?.id;
    let language = id.language.as_str();
    if !data::PLURAL_LANGUAGES.contains(&language) {
        return None;
    }
    let brazil = id.region.is_some_and(|r| r.as_str() == "BR");
    let locale = IcuLocale::try_from_str(if language == "pt" && !brazil { "pt-PT" } else { language }).ok()?;
    let rules = PluralRules::try_new_cardinal_with_buffer_provider(data::provider()?, (&locale).into()).ok()?;
    let mut categories = Vec::new();
    for n in 0..=1000_u32 {
        let c = rules.category_for(n);
        if !categories.contains(&c) {
            categories.push(c);
        }
    }
    categories.sort_by_key(rank);
    Some((rules, categories))
}

fn rank(c: &PluralCategory) -> u8 {
    match c {
        PluralCategory::Zero => 0,
        PluralCategory::One => 1,
        PluralCategory::Two => 2,
        PluralCategory::Few => 3,
        PluralCategory::Many => 4,
        PluralCategory::Other => 5,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    impl I18n {
        /// The embedded locales parsed so far, by code.
        fn parsed(&self) -> Vec<&str> {
            let mut codes: Vec<&str> = self.locales.iter().skip(2).filter(|l| l.parsed.get().is_some()).map(|l| l.code.as_str()).collect();
            codes.sort_unstable();
            codes
        }
    }

    static E: &Embedded = &[
        ("fr", "\tSave\tEnregistrer\n\tOpen\tOuvrir\nmenu\tOpen\tOuvrir le menu\n@id\tfile.save\tEnregistrer le fichier\n", "\tSave\tSauvegarder\n"),
        ("pt", "\tSave\tGuardar\n\tUndo\tDesfazer\n", ""),
        ("pt-br", "\tSave\tSalvar\n", ""),
        ("de", "\tBad\n@nope\tA\tB\n\tUndo\tRückgängig\n", ""),
    ];

    /// `embedded`, offering every locale in it, with `en-US` numbers rather than the machine's.
    fn all(embedded: &'static Embedded) -> (I18n, Vec<String>) {
        let codes: Vec<&str> = embedded.iter().map(|(c, _, _)| *c).collect();
        let (i, problems) = I18n::new(embedded, &codes);
        i.set_format(Format::from_tag("en-US"));
        (i, problems)
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
        let i = i18n();
        assert_eq!(i.t(lang(&i, "de"), "Undo"), "Rückgängig");
        assert_eq!(i.check_all().len(), 2);
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
        ("ar", "@plural\t{n} file|{n} files\t{n} ملفات|{n} ملف واحد|{n} ملفان|{n} ملفات|{n} ملفًا|{n} ملف\n", ""),
        ("fa", "@plural\t{n} page|{n} pages\t{n} صفحه|{n} صفحه\n", ""),
    ];

    #[test]
    fn fmt_fills_once() {
        assert_eq!(fmt("{a} to {b}", &[("a", "x"), ("b", "{a}")]), "x to {a}");
        assert_eq!(fmt("{z} {", &[]), "{z} {");
    }

    #[test]
    fn left_to_right_text_gets_no_marks() {
        assert_eq!(fmt("Saved {name} in {dir}.", &[("name", "Report 2.pdf"), ("dir", &ltr("/home/sara"))]), "Saved Report 2.pdf in /home/sara.");
        assert_eq!(fmt("{a}", &[("a", "x"), ("b", "שלום")]), "x", "an unused value doesn't count");
        assert_eq!(fmt_plain("{a}", &[("a", &isolate("x"))]), "x");
        assert_eq!(fmt_plain("\u{2066}{a} {b}", &[("a", "x\u{2069}"), ("b", &isolate(&ltr("y")))]), "x y");
        let i = all(P).0;
        assert_eq!(i.trn(Lang::EN, 2, "{n} page", "{n} pages"), "2 pages");
    }

    #[test]
    fn right_to_left_text_isolates_values() {
        let saved = fmt("تم حفظ {name} في {dir}.", &[("name", "Report 2.pdf"), ("dir", &ltr("/home/سارة/docs"))]);
        assert_eq!(saved, "تم حفظ \u{2068}Report 2.pdf\u{2069} في \u{2066}/home/سارة/docs\u{2069}.");
        assert_eq!(strip_isolates(&saved), "تم حفظ Report 2.pdf في /home/سارة/docs.");
        assert_eq!(fmt("Opened {name}.", &[("name", "שלום.txt")]), "Opened \u{2068}שלום.txt\u{2069}.");
        assert_eq!(fmt("[{a}] {b}", &[("a", ""), ("b", "ש")]), "[] \u{2068}ש\u{2069}");
        let i = all(P).0;
        assert_eq!(i.trn(lang(&i, "fa"), 3, "{n} page", "{n} pages"), "\u{2066}3\u{2069} صفحه");
    }

    #[test]
    fn a_value_cannot_escape_its_isolate() {
        assert_eq!(isolate("a\u{2069}b"), "\u{2068}ab\u{2069}");
        assert_eq!(isolate("a\u{2067}b"), "\u{2068}a\u{2067}b\u{2069}\u{2069}");
        let one = ltr("12");
        assert_eq!(fmt("ש {n}", &[("n", &one)]), format!("ש {one}"));
        // Two isolates side by side are not one, so they get wrapped.
        assert_eq!(fmt("ש{v}", &[("v", "\u{2066}a\u{2069}\u{2066}b\u{2069}")]), "ש\u{2068}\u{2066}a\u{2069}\u{2066}b\u{2069}\u{2069}");
    }

    /// `text` in display order, as one line in a left-to-right paragraph, without bidi controls.
    fn visual(text: &str) -> String {
        let info = unicode_bidi::BidiInfo::new(text, Some(unicode_bidi::Level::ltr()));
        let para = &info.paragraphs[0];
        let shown = info.reorder_line(para, para.range.clone());
        shown
            .chars()
            .filter(|c| bidi_class(*c) != BidiClass::BN && !"\u{202A}\u{202B}\u{202C}\u{202D}\u{202E}\u{2066}\u{2067}\u{2068}\u{2069}".contains(*c))
            .collect()
    }

    #[test]
    fn a_value_with_bidi_controls_cannot_reorder_the_sentence() {
        let file_names = [
            "\u{202E}fdp.exe",
            "\u{202B}abc def",
            "\u{202D}abc def",
            "\u{2067}abc def",
            "\u{2068}abc\u{2069}\u{202E}def",
            "\u{202E}abc\u{2069}",
            "x\u{200F}",
            "x\u{061C}",
        ];
        for name in file_names {
            let text = fmt("Open {name} -> {name} now?", &[("name", name)]);
            let shown = visual(&text);
            assert!(shown.starts_with("Open ") && shown.contains(" -> ") && shown.ends_with(" now?"), "{name:?}: {shown:?}");
            let i = all(P).0;
            let wording = i.trn(Lang::EN, 2, "{n} page in {name}, done", "{n} pages in {name}, done");
            let shown = visual(&fmt(&wording, &[("name", name)]));
            assert!(shown.starts_with("2 pages in ") && shown.ends_with(", done"), "{name:?}: {shown:?}");
        }
        // An override the value opens is closed inside its isolate, and a stray PDF dropped.
        assert_eq!(fmt("Open {name}.", &[("name", "\u{202E}abc")]), "Open \u{2068}\u{202E}abc\u{202C}\u{2069}.");
        assert_eq!(fmt("Open {name}.", &[("name", "a\u{202C}b")]), "Open \u{2068}ab\u{2069}.");
        assert_eq!(fmt("Open {n}.", &[("n", &ltr("\u{202B}1"))]), "Open \u{2066}\u{202B}1\u{202C}\u{2069}.");
        assert_eq!(isolate("\u{2067}a\u{202E}b\u{2069}c"), "\u{2068}\u{2067}a\u{202E}b\u{202C}\u{2069}c\u{2069}");
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
        i.set_format(Format::from_tag("fr"));
        assert_eq!(i.trn(lang(&i, "fr"), 1_000_000, "{n} page", "{n} pages"), "1\u{202f}000\u{202f}000 pages");
    }

    #[test]
    fn counts_follow_the_region() {
        let i = all(P).0;
        i.set_format(Format::from_tag("ar-EG"));
        assert_eq!(i.trn(lang(&i, "ar"), 1234, "{n} file", "{n} files"), "\u{2066}١٬٢٣٤\u{2069} ملفًا");
        i.set_format(Format::new(&Region::from_tag("ar-EG"), Digits::Latin));
        assert_eq!(i.trn(lang(&i, "ar"), 1234, "{n} file", "{n} files"), "\u{2066}1,234\u{2069} ملفًا");
        // An English interface with a German region.
        i.set_format(Format::from_tag("de-DE"));
        assert_eq!(i.trn(Lang::EN, 12_345, "{n} file", "{n} files"), "12.345 files");
    }

    #[test]
    fn plain_counts_and_the_system_format() {
        let i = all(P).0;
        i.set_format(Format::from_tag("ar-EG"));
        assert_eq!(i.trn_plain(lang(&i, "ar"), 1234, "{n} file", "{n} files"), "1234 ملفًا");
        i.set_format(Format::from_tag("de-DE"));
        assert_eq!(i.trn_plain(Lang::EN, 12_345, "copy {n}", "copy {n}"), "copy 12345");
        i.reset_format();
        assert_eq!(
            i.trn(Lang::EN, 5, "{n} file", "{n} files"),
            fmt("{n} files", &[("n", &Format::new(&Region::system(), Digits::Locale).integer(5))])
        );
    }

    #[test]
    fn decimal_counts() {
        let i = all(P).0;
        i.set_format(Format::from_tag("en"));
        assert_eq!(i.trn_decimal(Lang::EN, 1.5, 1, "{n} file", "{n} files"), "1.5 files");
        assert_eq!(i.trn_decimal(Lang::EN, 1.0, 1, "{n} file", "{n} files"), "1.0 files", "visible decimals are plural");
        assert_eq!(i.trn_decimal(Lang::EN, 1.0, 0, "{n} file", "{n} files"), "1 file");
        // Russian decimals are `other`, which integers never reach: the `few` form.
        i.set_format(Format::from_tag("ru"));
        let ru = lang(&i, "ru");
        assert_eq!(i.trn_decimal(ru, 1.5, 1, "{n} file", "{n} files"), "1,5 файла");
        assert_eq!(i.trn_decimal(ru, 21.0, 0, "{n} file", "{n} files"), "21 файл");
        // Arabic decimals are `other`.
        i.set_format(Format::from_tag("ar-EG"));
        let ar = lang(&i, "ar");
        assert_eq!(i.trn_decimal(ar, 2.5, 1, "{n} file", "{n} files"), "\u{2066}٢٫٥\u{2069} ملف");
        assert_eq!(i.trn_decimal(ar, 3.0, 0, "{n} file", "{n} files"), "\u{2066}٣\u{2069} ملفات");
        // Not a number: the translated `other` form.
        assert_eq!(i.trn_decimal(ru, f64::NAN, 1, "{n} file", "{n} files"), "NaN файла");
        assert_eq!(i.trn_decimal(Lang::EN, f64::INFINITY, 1, "{n} file", "{n} files"), "∞ files");
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

    /// The counts craft-i18n's checker expects, which rejects a catalog whose `@plural` rows list
    /// any other: every directory in craft-i18n's `locales/`, then languages it may add.
    #[test]
    fn plural_form_counts() {
        #[rustfmt::skip]
        let catalogs = [
            ("ar", 6), ("bg", 2), ("cs", 3), ("da", 2), ("de", 2), ("el", 2), ("es", 2), ("et", 2),
            ("fi", 2), ("fr", 2), ("hr", 3), ("hu", 2), ("id", 1), ("it", 2), ("ja", 1), ("ka", 2),
            ("kk", 2), ("ko", 1), ("lt", 3), ("nl", 2), ("pl", 3), ("pt-br", 2), ("ru", 3), ("sl", 4),
            ("sr", 3), ("sr-latn", 3), ("sv", 2), ("te", 2), ("tr", 2), ("uk", 3), ("vi", 1),
            ("zh-hans", 1), ("zh-hant", 1),
        ];
        let more = [("en", 2), ("pt", 2), ("he", 3), ("ro", 3), ("nb", 2), ("ca", 2), ("sk", 3), ("th", 1), ("ms", 1), ("fa", 2), ("hi", 2)];
        for (locale, n) in catalogs.into_iter().chain(more) {
            assert_eq!(plural_rules(locale).map(|(_, c)| c.len()), Some(n), "{locale}");
        }
        assert!(plural_rules("tlh").is_none(), "no CLDR rules");
    }

    #[test]
    fn serbian_latin_plurals() {
        let i = all(&[("sr-latn", "@plural\t{n} file|{n} files\t{n} fajl|{n} fajla|{n} fajlova\n", "")]).0;
        let sr = lang(&i, "sr-latn");
        let files = |n| i.trn(sr, n, "{n} file", "{n} files");
        assert_eq!((files(1), files(3), files(5)), ("1 fajl".into(), "3 fajla".into(), "5 fajlova".into()));
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
        let i = all(&[("fr", "\tSave\tEnregistrer\n\tSave\tSauver\n", "")]).0;
        assert_eq!(i.check_all(), ["fr/common: line 2: defined twice (same context and source text); the first is kept"]);
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

    #[test]
    fn check_all_reports_every_bad_row() {
        let (i, problems) = all(E);
        assert!(i.parsed().is_empty(), "new parses nothing");
        assert!(problems.iter().all(|p| !p.contains("line")), "{problems:?}");
        let found = i.check_all();
        assert_eq!(found.len(), 2, "{found:?}");
        assert!(found[0].starts_with("de/common: line 1:"), "{found:?}");
        assert!(found[1].starts_with("de/common: line 2:"), "{found:?}");
        assert_eq!(i.check_all(), found);
        assert_eq!(i.parsed(), ["de", "fr", "pt", "pt-br"]);
    }

    #[test]
    fn lookup_parses_only_what_it_reaches() {
        static LAZY: &Embedded = &[
            ("pt", "\tUndo\tDesfazer\n", ""),
            ("pt-br", "\tSave\tSalvar\n", ""),
            ("de", "\tUndo\tRückgängig\n", ""),
            ("fr", "\tUndo\tAnnuler\n", ""),
        ];
        let i = all(LAZY).0;
        assert!(i.parsed().is_empty());
        assert_eq!(i.t(lang(&i, "pt-br"), "Save"), "Salvar");
        assert_eq!(i.parsed(), ["pt-br"]);
        assert_eq!(i.t(lang(&i, "pt-br"), "Undo"), "Desfazer");
        assert_eq!(i.parsed(), ["pt", "pt-br"]);
        assert_eq!(i.t(lang(&i, "pt-br"), "Undo"), "Desfazer");
        assert_eq!(i.parsed(), ["pt", "pt-br"]);
        assert_eq!(i.t(Lang::EN, "Undo"), "Undo");
        assert_eq!(i.t(Lang::PSEUDO, "Undo"), "⟦Undo⟧");
        assert_eq!(i.parsed(), ["pt", "pt-br"]);
    }

    #[test]
    fn name_reads_escapes_without_parsing() {
        let i = all(&[("fr", "@id\tlanguage.name\tFran\\\\ais\\tx\n", ""), ("de", "\tA\tB\n@id\tlanguage.name\tDeutsch\n", "")]).0;
        assert_eq!(i.name(lang(&i, "fr")), "Fran\\ais\tx");
        assert_eq!(i.name(lang(&i, "de")), "Deutsch");
        assert!(i.parsed().is_empty());
    }

    #[test]
    fn app_name_overrides_shared() {
        let i = all(&[
            ("fr", "@id\tlanguage.name\tFrançais\n", "@id\tlanguage.name\tFrançais (app)\n"),
            ("de", "@id\tlanguage.name\tDeutsch\n", "\tA\tB\n"),
            ("es", "", ""),
        ])
        .0;
        assert_eq!(i.name(lang(&i, "fr")), "Français (app)");
        assert_eq!(i.name(lang(&i, "de")), "Deutsch");
        assert_eq!(i.name(lang(&i, "es")), "es");
    }

    #[test]
    fn i18n_is_send_and_sync() {
        fn check<T: Send + Sync>() {}
        check::<I18n>();
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
