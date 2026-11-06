//! The formatting region: how numbers, times and measurements are written, apart from the
//! interface language. An English interface in Poland still writes `2,54 cm` on A4.
//!
//! The territory lists come from Unicode CLDR's supplemental `measurementData` (Unicode License):
//! `paperSize` US-Letter and `measurementSystem` US. Decimal and grouping separators come from
//! the number locale's CLDR data ([`crate::Format`]); the region carries its own only where the
//! system sets them.

use std::sync::OnceLock;

/// Territories whose default paper is US Letter (CLDR `paperSize type="US-Letter"`).
const LETTER: &[&str] = &["BZ", "CA", "CL", "CO", "CR", "GT", "MX", "NI", "PA", "PH", "PR", "SV", "US", "VE"];

/// Territories that measure in inches (CLDR `measurementSystem type="US"`).
const INCHES: &[&str] = &["LR", "MM", "US"];

/// How lengths, distances and weights are measured.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Measurement {
    /// Centimetres, kilometres and kilograms (CLDR `metric` and `UK`).
    Metric,
    /// Inches, miles and pounds (CLDR `US`).
    Us,
}

/// The paper a new document starts on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Paper {
    A4,
    Letter,
}

/// A clock the user chose, where the system says so apart from the locale.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HourCycle {
    /// 1 to 12 with AM and PM.
    H12,
    /// 0 to 23.
    H23,
}

/// The conventions numbers, times and measurements follow. Read it with [`Region::system`] or,
/// for the user's saved choice, [`Region::from_setting`]; the fields are public so an app can
/// keep a separate choice of, say, measurement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Region {
    /// The locale numbers are written in, BCP 47 (`pl-PL`, `sr-Latn-RS`).
    pub numbers: String,
    /// The locale dates and times are written in, BCP 47.
    pub time: String,
    pub measurement: Measurement,
    pub paper: Paper,
    /// The decimal separator the system sets (Windows, macOS); `None` follows the locale.
    pub decimal: Option<char>,
    /// The grouping separator the system sets (Windows, macOS): `Some(None)` for none at all
    /// (`1234567`); `None` follows the locale.
    pub grouping: Option<Option<char>>,
    /// The clock the system sets (Windows, macOS); `None` follows the locale.
    pub hour_cycle: Option<HourCycle>,
}

impl Default for Region {
    /// `en-US`: Letter, inches.
    fn default() -> Self {
        Region::from_parts(&Tag::default())
    }
}

impl Region {
    /// The region a BCP 47 or POSIX locale tag names (`pl-PL`, `de_AT.UTF-8@euro`, `zh-Hans-CN`,
    /// `en-US-u-ms-metric`, Apple's `en_US@rg=plzzzz`). The Unicode extensions `ms` (measurement),
    /// `hc` (clock), `rg` (region override) and `nu` (digits; Apple's `@numbers=`) are read, and
    /// `nu` stays in the number and time locales (`ar-EG-u-nu-latn`). `C` and `POSIX` are the C
    /// locale: English, A4, metric, a `.` decimal and no grouping. An empty or unreadable tag is
    /// `en-US`; a bare `en` is the United States, any other bare language has no territory.
    pub fn from_tag(tag: &str) -> Region {
        match Tag::parse(tag) {
            Some(tag) => Region::from_parts(&tag),
            None if is_c_locale(tag) => {
                Region { measurement: Measurement::Metric, paper: Paper::A4, decimal: Some('.'), grouping: Some(None), ..Region::default() }
            }
            None => Region::default(),
        }
    }

    /// The region for an app's saved region setting: `""` or `auto` (any case) is
    /// [`Region::system`], anything else a locale tag for [`Region::from_tag`].
    pub fn from_setting(setting: &str) -> Region {
        let setting = setting.trim();
        if setting.is_empty() || setting.eq_ignore_ascii_case("auto") { Region::system() } else { Region::from_tag(setting) }
    }

    /// The system's region, read once, then cached:
    ///
    /// - Windows: the regional format and its customised separators, measurement system, paper
    ///   and short time format (Settings › Time & language › Language & region).
    /// - macOS and iOS: the current locale (its identifier carries the Region setting), its
    ///   separators and metric setting, and its hour format.
    /// - Linux and the BSDs: `LC_ALL`, else each of `LC_NUMERIC`, `LC_TIME`, `LC_MEASUREMENT` and
    ///   `LC_PAPER` on its own, else `LANG`.
    /// - The web and Android: the preferred language, the only locale they expose.
    pub fn system() -> Region {
        static SYSTEM: OnceLock<Region> = OnceLock::new();
        SYSTEM.get_or_init(read_system).clone()
    }

    fn from_parts(tag: &Tag) -> Region {
        let territory = tag.territory.as_str();
        let measurement = tag.measurement.unwrap_or(if INCHES.contains(&territory) { Measurement::Us } else { Measurement::Metric });
        Region {
            numbers: tag.bcp47(),
            time: tag.bcp47(),
            measurement,
            paper: if LETTER.contains(&territory) { Paper::Letter } else { Paper::A4 },
            decimal: None,
            grouping: None,
            hour_cycle: tag.hour_cycle,
        }
    }
}

/// The parts of a locale tag the region uses.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Tag {
    /// Lower case (`pl`).
    language: String,
    /// Title case (`Latn`); empty when the tag has none.
    script: String,
    /// Upper case (`PL`, or a UN M.49 number like `419`); empty when the tag has none.
    territory: String,
    measurement: Option<Measurement>,
    hour_cycle: Option<HourCycle>,
    /// The `nu` keyword (`latn`, `arab`); empty when the tag has none.
    digits: String,
}

impl Default for Tag {
    fn default() -> Self {
        Tag { language: "en".into(), script: String::new(), territory: "US".into(), measurement: None, hour_cycle: None, digits: String::new() }
    }
}

impl Tag {
    fn parse(tag: &str) -> Option<Tag> {
        // Locale tags are short; anything long is junk and only its start is read.
        let tag: String = tag.chars().take(64).collect();
        let (tag, modifier) = tag.split_once('@').unwrap_or((&tag, ""));
        let tag = tag.split('.').next().unwrap_or_default().trim();
        if is_c_locale(tag) {
            return None;
        }
        let mut parts = tag.split(['-', '_']);
        let language = parts.next().unwrap_or_default();
        if !(2..=3).contains(&language.len()) || !language.chars().all(|c| c.is_ascii_alphabetic()) {
            return None;
        }
        let mut out = Tag {
            language: language.to_ascii_lowercase(),
            script: String::new(),
            territory: String::new(),
            measurement: None,
            hour_cycle: None,
            digits: String::new(),
        };
        let mut extension = None;
        for p in parts.by_ref() {
            // A singleton starts the extensions (`-u-…`, `-x-…`).
            if p.len() == 1 {
                extension = Some(p.to_ascii_lowercase());
                break;
            }
            if out.territory.is_empty() && out.script.is_empty() && p.len() == 4 && p.chars().all(|c| c.is_ascii_alphabetic()) {
                out.script = title_case(p);
            } else if out.territory.is_empty() && is_territory(p) {
                out.territory = p.to_ascii_uppercase();
            }
        }
        let mut keywords: Vec<(String, String)> = Vec::new();
        if extension.as_deref() == Some("u") {
            let mut key = String::new();
            for p in parts {
                if p.len() == 1 {
                    break;
                }
                if p.len() == 2 {
                    key = p.to_ascii_lowercase();
                } else if !key.is_empty() {
                    keywords.push((std::mem::take(&mut key), p.to_ascii_lowercase()));
                }
            }
        }
        // Apple's and ICU's `@rg=plzzzz;ms=metric` keywords, and glibc's `@latin` script modifier.
        for m in modifier.split(';') {
            match m.split_once('=') {
                Some((k, v)) => keywords.push((k.trim().to_ascii_lowercase(), v.trim().to_ascii_lowercase())),
                None if m.eq_ignore_ascii_case("latin") => out.script = "Latn".into(),
                None if m.eq_ignore_ascii_case("cyrillic") => out.script = "Cyrl".into(),
                None => {}
            }
        }
        for (key, value) in keywords {
            match (key.as_str(), value.as_str()) {
                ("ms", "metric" | "uksystem") => out.measurement = Some(Measurement::Metric),
                ("ms", "ussystem") => out.measurement = Some(Measurement::Us),
                ("hc", "h11" | "h12") => out.hour_cycle = Some(HourCycle::H12),
                ("hc", "h23" | "h24") => out.hour_cycle = Some(HourCycle::H23),
                ("nu" | "numbers", v) if (3..=8).contains(&v.len()) && v.chars().all(|c| c.is_ascii_alphanumeric()) => out.digits = v.into(),
                ("rg", v) if v.len() == 6 && v.ends_with("zzzz") && v.get(..2).is_some_and(is_territory) => {
                    out.territory = v.get(..2).unwrap_or_default().to_ascii_uppercase();
                }
                _ => {}
            }
        }
        if out.territory.is_empty() && out.language == "en" {
            out.territory = "US".into();
        }
        Some(out)
    }

    fn bcp47(&self) -> String {
        let tag = [self.language.as_str(), &self.script, &self.territory].into_iter().filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-");
        if self.digits.is_empty() { tag } else { format!("{tag}-u-nu-{}", self.digits) }
    }
}

fn is_territory(p: &str) -> bool {
    (p.len() == 2 && p.chars().all(|c| c.is_ascii_alphabetic())) || (p.len() == 3 && p.chars().all(|c| c.is_ascii_digit()))
}

fn title_case(p: &str) -> String {
    let lower = p.to_ascii_lowercase();
    let mut chars = lower.chars();
    chars.next().map(|c| c.to_ascii_uppercase().to_string() + chars.as_str()).unwrap_or_default()
}

/// Whether a locale tag names the C locale (`C`, `C.UTF-8`, `POSIX`).
fn is_c_locale(tag: &str) -> bool {
    let tag = tag.split(['@', '.']).next().unwrap_or_default().trim();
    tag.eq_ignore_ascii_case("c") || tag.eq_ignore_ascii_case("posix")
}

/// The region from the POSIX locale variables `var` reads. An empty variable is unset; `C` in
/// `LC_ALL` or an `LC_*` variable is the C locale's conventions. `LANG` set to `C` or unset gives
/// way to the platform's preferred locale.
#[cfg(any(test, all(unix, not(any(target_vendor = "apple", target_os = "android")))))]
fn from_posix(var: impl Fn(&str) -> Option<String>, platform: impl FnOnce() -> Option<String>) -> Region {
    let var = |name: &str| var(name).filter(|v| !v.trim().is_empty());
    if let Some(all) = var("LC_ALL") {
        return Region::from_tag(&all);
    }
    let lang = var("LANG").filter(|v| !is_c_locale(v)).or_else(platform).unwrap_or_default();
    let tag = |name: &str| var(name).unwrap_or_else(|| lang.clone());
    let time = Region::from_tag(&tag("LC_TIME"));
    Region {
        time: time.time,
        hour_cycle: time.hour_cycle,
        measurement: Region::from_tag(&tag("LC_MEASUREMENT")).measurement,
        paper: Region::from_tag(&tag("LC_PAPER")).paper,
        ..Region::from_tag(&tag("LC_NUMERIC"))
    }
}

/// The region from the values under `HKEY_CURRENT_USER\Control Panel\International` that `value`
/// reads, where Windows keeps the regional format and the user's changes to it. `LocaleName` is
/// the format (`pl-PL`), else the platform's preferred locale.
#[cfg(any(test, windows))]
pub(crate) fn from_windows(value: impl Fn(&str) -> Option<String>, platform: impl FnOnce() -> Option<String>) -> Region {
    let mut region = Region::from_tag(&value("LocaleName").or_else(platform).unwrap_or_default());
    region.decimal = value("sDecimal").as_deref().and_then(single_char);
    region.grouping = value("sThousand").as_deref().and_then(grouping);
    match value("iMeasure").as_deref().map(str::trim) {
        Some("0") => region.measurement = Measurement::Metric,
        Some("1") => region.measurement = Measurement::Us,
        _ => {}
    }
    // DMPAPER_LETTER and DMPAPER_A4; other sizes keep the territory's.
    match value("iPaperSize").as_deref().map(str::trim) {
        Some("1") => region.paper = Paper::Letter,
        Some("9") => region.paper = Paper::A4,
        _ => {}
    }
    region.hour_cycle = value("sShortTime").as_deref().and_then(hour_cycle_of).or(region.hour_cycle);
    region
}

/// The region from Apple's current locale: its identifier (`en_PL`, `en_GB@rg=uszzzz`), its
/// metric setting and separators, and the hour format its `j` skeleton gives (`HH`, `h a`).
#[cfg(any(test, target_vendor = "apple"))]
fn from_apple(identifier: &str, metric: bool, decimal: &str, grouping: &str, hours: Option<&str>) -> Region {
    let mut region = Region::from_tag(identifier);
    region.measurement = if metric { Measurement::Metric } else { Measurement::Us };
    region.decimal = single_char(decimal);
    region.grouping = self::grouping(grouping);
    region.hour_cycle = hours.and_then(hour_cycle_of).or(region.hour_cycle);
    region
}

/// A grouping separator the system sets: none at all when empty (Windows' `sThousand` the user
/// cleared), else one character.
#[cfg(any(test, windows, target_vendor = "apple"))]
fn grouping(s: &str) -> Option<Option<char>> {
    if s.is_empty() { Some(None) } else { single_char(s).map(Some) }
}

/// A separator that is one character; Windows allows up to three, which the region can't carry.
#[cfg(any(test, windows, target_vendor = "apple"))]
fn single_char(s: &str) -> Option<char> {
    let mut chars = s.chars();
    let c = chars.next()?;
    chars.next().is_none().then_some(c)
}

/// The clock a time pattern (`HH:mm`, `h:mm tt`, `h 'Uhr'`) uses; quoted literals are skipped.
#[cfg(any(test, windows, target_vendor = "apple"))]
fn hour_cycle_of(pattern: &str) -> Option<HourCycle> {
    let mut quoted = false;
    for c in pattern.chars() {
        match c {
            '\'' => quoted = !quoted,
            'H' | 'k' if !quoted => return Some(HourCycle::H23),
            'h' | 'K' if !quoted => return Some(HourCycle::H12),
            _ => {}
        }
    }
    None
}

#[cfg(windows)]
fn read_system() -> Region {
    let key = windows_registry::CURRENT_USER.open(r"Control Panel\International").ok();
    from_windows(|name| key.as_ref()?.get_string(name).ok(), sys_locale::get_locale)
}

#[cfg(target_vendor = "apple")]
fn read_system() -> Region {
    use objc2_foundation::{NSDateFormatter, NSLocale, NSString};
    let locale = NSLocale::currentLocale();
    let hours = NSDateFormatter::dateFormatFromTemplate_options_locale(&NSString::from_str("j"), 0, Some(&locale)).map(|f| f.to_string());
    from_apple(
        &locale.localeIdentifier().to_string(),
        locale.usesMetricSystem(),
        &locale.decimalSeparator().to_string(),
        &locale.groupingSeparator().to_string(),
        hours.as_deref(),
    )
}

#[cfg(all(unix, not(any(target_vendor = "apple", target_os = "android"))))]
fn read_system() -> Region {
    from_posix(|name| std::env::var(name).ok(), sys_locale::get_locale)
}

#[cfg(not(any(windows, unix)))]
fn read_system() -> Region {
    Region::from_tag(&sys_locale::get_locale().unwrap_or_default())
}

#[cfg(target_os = "android")]
fn read_system() -> Region {
    Region::from_tag(&sys_locale::get_locale().unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conventions(r: &Region) -> (Paper, Measurement) {
        (r.paper, r.measurement)
    }

    #[test]
    fn reads_locale_tags() {
        assert_eq!(Region::from_tag("pl-PL").numbers, "pl-PL");
        assert_eq!(Region::from_tag("de_AT.UTF-8@euro").numbers, "de-AT");
        assert_eq!(Region::from_tag("zh-hans-cn").numbers, "zh-Hans-CN");
        assert_eq!(Region::from_tag("sr_RS@latin").numbers, "sr-Latn-RS");
        assert_eq!(Region::from_tag("es-419").numbers, "es-419");
        assert_eq!(Region::from_tag("fr").numbers, "fr");
        assert_eq!(Region::from_tag("en").numbers, "en-US", "bare English is the US");
        for junk in ["", "1234", "x", "--", "ünïcode-ÄÄ", &"a".repeat(10_000)] {
            assert_eq!(Region::from_tag(junk), Region::default(), "{junk:?}");
        }
    }

    /// glibc's C locale: A4 and metric (`C-paper.c`, `C-measurement.c`), `.` and no grouping.
    #[test]
    fn the_c_locale_is_a4_metric_and_ungrouped() {
        for c in ["C", "POSIX", "C.UTF-8", "c.utf8", " C "] {
            let r = Region::from_tag(c);
            assert_eq!(
                (r.numbers.as_str(), r.paper, r.measurement, r.decimal, r.grouping),
                ("en-US", Paper::A4, Measurement::Metric, Some('.'), Some(None)),
                "{c:?}"
            );
        }
    }

    #[test]
    fn a_chosen_digit_system_stays_in_the_locale() {
        assert_eq!(Region::from_tag("ar-EG-u-nu-latn").numbers, "ar-EG-u-nu-latn");
        let r = Region::from_tag("ar_EG@numbers=latn");
        assert_eq!((r.numbers.as_str(), r.time.as_str()), ("ar-EG-u-nu-latn", "ar-EG-u-nu-latn"));
        assert_eq!(Region::from_tag("en-US-u-nu-arab-ms-metric").numbers, "en-US-u-nu-arab");
        assert_eq!(Region::from_tag("ar-EG-u-nu-x").numbers, "ar-EG", "not a numbering system");
    }

    #[test]
    fn unicode_extensions_and_apple_keywords() {
        let r = Region::from_tag("en-US-u-ms-metric-hc-h23");
        assert_eq!((r.numbers.as_str(), r.measurement, r.hour_cycle), ("en-US", Measurement::Metric, Some(HourCycle::H23)));
        assert_eq!(Region::from_tag("en-GB-u-hc-h12").hour_cycle, Some(HourCycle::H12));
        assert_eq!(Region::from_tag("de-DE-x-ms-ussystem").measurement, Measurement::Metric, "private use isn't read");
        let r = Region::from_tag("en_GB@rg=uszzzz");
        assert_eq!((r.numbers.as_str(), r.paper, r.measurement), ("en-US", Paper::Letter, Measurement::Us));
        assert_eq!(Region::from_tag("en_US@rg=plzzzz;ms=metric").numbers, "en-PL");
    }

    /// Paper and measurement follow the territory; separators are left to the number locale.
    #[test]
    fn conventions_follow_the_territory() {
        assert_eq!(conventions(&Region::from_tag("pl-PL")), (Paper::A4, Measurement::Metric));
        assert_eq!(conventions(&Region::from_tag("en-US")), (Paper::Letter, Measurement::Us));
        assert_eq!(conventions(&Region::from_tag("fr-CA")), (Paper::Letter, Measurement::Metric));
        assert_eq!(conventions(&Region::from_tag("en-GB")), (Paper::A4, Measurement::Metric));
        let r = Region::from_tag("de-CH");
        assert_eq!((r.decimal, r.grouping), (None, None));
        assert_eq!(Region::from_tag("es-MX").paper, Paper::Letter);
        assert_eq!(Region::from_tag("ja-JP").paper, Paper::A4);
    }

    #[test]
    fn a_saved_setting_overrides_the_system() {
        assert_eq!(Region::from_setting(" pl-PL "), Region::from_tag("pl-PL"));
        assert_eq!(Region::from_setting("AUTO"), Region::system());
        assert_eq!(Region::from_setting(""), Region::system());
    }

    #[test]
    fn posix_variables_set_each_convention() {
        let env = |pairs: &'static [(&'static str, &'static str)]| move |n: &str| pairs.iter().find(|(k, _)| *k == n).map(|(_, v)| v.to_string());
        let none = || None;
        let r = from_posix(env(&[("LANG", "en_US.UTF-8"), ("LC_NUMERIC", "pl_PL.UTF-8"), ("LC_TIME", "en_GB.UTF-8"), ("LC_PAPER", "de_DE")]), none);
        assert_eq!((r.numbers.as_str(), r.time.as_str(), r.decimal), ("pl-PL", "en-GB", None));
        assert_eq!((r.paper, r.measurement), (Paper::A4, Measurement::Us), "paper from LC_PAPER, measurement from LANG");
        assert_eq!(from_posix(env(&[("LANG", "it_IT.UTF-8"), ("LC_ALL", "en_US"), ("LC_PAPER", "de_DE")]), none).paper, Paper::Letter);
        assert_eq!(from_posix(env(&[("LANG", "en_US"), ("LC_MEASUREMENT", "it_IT")]), none).measurement, Measurement::Metric);
        assert_eq!(from_posix(env(&[("LANG", "C.UTF-8"), ("LC_ALL", "")]), || Some("it-IT".into())).numbers, "it-IT");
        assert_eq!(from_posix(env(&[]), none), Region::default());
        // `C` in an LC_* variable is the C locale's conventions, not an unset variable.
        let r = from_posix(env(&[("LANG", "de_DE.UTF-8"), ("LC_NUMERIC", "C"), ("LC_TIME", "C.UTF-8")]), none);
        assert_eq!((r.numbers.as_str(), r.time.as_str(), r.paper), ("en-US", "en-US", Paper::A4));
        assert_eq!(from_posix(env(&[("LANG", "de_DE.UTF-8"), ("LC_ALL", "C")]), none), Region::from_tag("C"));
        let r = from_posix(env(&[("LANG", "en_US.UTF-8"), ("LC_PAPER", "C"), ("LC_MEASUREMENT", "C.UTF-8")]), none);
        assert_eq!((r.paper, r.measurement, r.decimal), (Paper::A4, Measurement::Metric, None));
        assert_eq!(from_posix(env(&[("LANG", "C"), ("LC_NUMERIC", "")]), || Some("it-IT".into())).numbers, "it-IT");
    }

    #[test]
    fn windows_regional_format_and_its_changes() {
        let reg = |pairs: &'static [(&'static str, &'static str)]| move |n: &str| pairs.iter().find(|(k, _)| *k == n).map(|(_, v)| v.to_string());
        let r = from_windows(
            reg(&[("LocaleName", "pl-PL"), ("sDecimal", ","), ("sThousand", "\u{a0}"), ("iMeasure", "0"), ("sShortTime", "HH:mm")]),
            || None,
        );
        assert_eq!((r.numbers.as_str(), r.decimal, r.grouping, r.hour_cycle), ("pl-PL", Some(','), Some(Some('\u{a0}')), Some(HourCycle::H23)));
        // An English format in the US with the user's metric and A4 choices and a decimal comma.
        let r = from_windows(
            reg(&[("LocaleName", "en-US"), ("sDecimal", ","), ("iMeasure", "0"), ("iPaperSize", "9"), ("sShortTime", "h:mm tt")]),
            || None,
        );
        assert_eq!(
            (r.decimal, r.grouping, r.measurement, r.paper, r.hour_cycle),
            (Some(','), None, Measurement::Metric, Paper::A4, Some(HourCycle::H12))
        );
        let r = from_windows(reg(&[("sDecimal", "dec"), ("sThousand", "")]), || Some("de-DE".into()));
        assert_eq!((r.numbers.as_str(), r.decimal), ("de-DE", None), "a separator longer than one character keeps the locale's");
        assert_eq!(r.grouping, Some(None), "an empty grouping separator is no grouping");
        assert_eq!(from_windows(reg(&[("sThousand", "abc")]), || None).grouping, None);
    }

    #[test]
    fn apple_locale_values() {
        let r = from_apple("en_PL", true, ",", "\u{a0}", Some("HH"));
        assert_eq!(
            (r.numbers.as_str(), r.paper, r.decimal, r.grouping, r.hour_cycle),
            ("en-PL", Paper::A4, Some(','), Some(Some('\u{a0}')), Some(HourCycle::H23))
        );
        let r = from_apple("en_US", false, ".", ",", Some("h a"));
        assert_eq!((r.measurement, r.hour_cycle), (Measurement::Us, Some(HourCycle::H12)));
        assert_eq!(from_apple("de_DE", true, ",", ".", Some("H 'Uhr'")).hour_cycle, Some(HourCycle::H23));
        assert_eq!(hour_cycle_of("'h' mm"), None, "quoted letters are text");
    }
}
