//! The scripts a language is written in, its direction, and which regional forms its Han
//! characters take, for picking fonts and laying out the interface.

/// A writing system, by its ISO 15924 code.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum Script {
    Arab,
    Armn,
    Beng,
    Cyrl,
    Deva,
    Ethi,
    Geor,
    Grek,
    Gujr,
    Guru,
    Hang,
    Hani,
    Hebr,
    Hira,
    Kana,
    Khmr,
    Knda,
    Laoo,
    Latn,
    Mlym,
    Mong,
    Mymr,
    Orya,
    Sinh,
    Syrc,
    Taml,
    Telu,
    Thaa,
    Thai,
    Tibt,
}

impl Script {
    /// The ISO 15924 code, `Latn`.
    pub fn code(self) -> &'static str {
        match self {
            Script::Arab => "Arab",
            Script::Armn => "Armn",
            Script::Beng => "Beng",
            Script::Cyrl => "Cyrl",
            Script::Deva => "Deva",
            Script::Ethi => "Ethi",
            Script::Geor => "Geor",
            Script::Grek => "Grek",
            Script::Gujr => "Gujr",
            Script::Guru => "Guru",
            Script::Hang => "Hang",
            Script::Hani => "Hani",
            Script::Hebr => "Hebr",
            Script::Hira => "Hira",
            Script::Kana => "Kana",
            Script::Khmr => "Khmr",
            Script::Knda => "Knda",
            Script::Laoo => "Laoo",
            Script::Latn => "Latn",
            Script::Mlym => "Mlym",
            Script::Mong => "Mong",
            Script::Mymr => "Mymr",
            Script::Orya => "Orya",
            Script::Sinh => "Sinh",
            Script::Syrc => "Syrc",
            Script::Taml => "Taml",
            Script::Telu => "Telu",
            Script::Thaa => "Thaa",
            Script::Thai => "Thai",
            Script::Tibt => "Tibt",
        }
    }

    /// Whether text in this script runs right to left.
    pub fn rtl(self) -> bool {
        matches!(self, Script::Arab | Script::Hebr | Script::Syrc | Script::Thaa)
    }
}

/// Which way a language's text, and an interface in it, runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Direction {
    Ltr,
    Rtl,
}

/// The regional glyph forms Han characters take, as in the Noto CJK face names (SC, TC, HK, JP,
/// KR). The same code point is drawn differently in each, so a font stack picks its Han face by
/// this, not by script.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Han {
    /// Simplified Chinese, mainland China and Singapore (SC).
    Simplified,
    /// Traditional Chinese, Taiwan (TC).
    Traditional,
    /// Traditional Chinese, Hong Kong and Macau (HK).
    HongKong,
    /// Japanese kanji (JP).
    Japanese,
    /// Korean hanja (KR).
    Korean,
}

/// The scripts a language tag's interface text is written in (`ja` → Hani, Hira, Kana; `sr` →
/// Cyrl; `sr-latn` → Latn), the main one first. Any case and glibc form (`sr_RS.UTF-8@latin`). A
/// script subtag wins, then a region that implies a script (`pa-PK` → Arab), then the language's
/// usual script. Latin product names and units appear in every language, so Latn is listed only
/// where it is the language's own script. A language not in the table is taken to be written in
/// Latin.
// ponytail: the languages a Craft app ships or could plausibly ship; add a row with the locale.
pub fn scripts(tag: &str) -> &'static [Script] {
    use Script::*;
    let tag = Subtags::parse(tag);
    if let Some(s) = tag.script.as_deref().and_then(subtag_scripts) {
        return s;
    }
    if let Some(s) = tag.region.as_deref().and_then(|region| region_script(&tag.language, region)).and_then(subtag_scripts) {
        return s;
    }
    match tag.language.as_str() {
        "ja" => &[Hani, Hira, Kana],
        "ko" => &[Hang, Hani],
        "zh" | "yue" | "wuu" => &[Hani],
        "be" | "bg" | "ba" | "ce" | "cv" | "kk" | "ky" | "mk" | "mn" | "ru" | "sah" | "sr" | "tg" | "tt" | "uk" => &[Cyrl],
        "el" => &[Grek],
        "ar" | "ckb" | "fa" | "ks" | "ps" | "sd" | "ug" | "ur" => &[Arab],
        "he" | "yi" => &[Hebr],
        "syr" => &[Syrc],
        "dv" => &[Thaa],
        "ka" => &[Geor],
        "hy" => &[Armn],
        "hi" | "mr" | "ne" | "sa" | "kok" | "mai" | "bho" => &[Deva],
        "bn" | "as" | "mni" => &[Beng],
        "pa" => &[Guru],
        "gu" => &[Gujr],
        "or" => &[Orya],
        "ta" => &[Taml],
        "te" => &[Telu],
        "kn" => &[Knda],
        "ml" => &[Mlym],
        "si" => &[Sinh],
        "th" => &[Thai],
        "lo" => &[Laoo],
        "km" => &[Khmr],
        "my" => &[Mymr],
        "am" | "ti" => &[Ethi],
        "bo" | "dz" => &[Tibt],
        _ => &[Latn],
    }
}

/// The direction of a language tag's text: right to left when its main script is.
pub fn direction(tag: &str) -> Direction {
    match scripts(tag).first() {
        Some(s) if s.rtl() => Direction::Rtl,
        _ => Direction::Ltr,
    }
}

/// The direction of a paragraph of text, from its first strong character (UAX #9 rules P2 and
/// P3): the contents of isolates (LRI, RLI or FSI up to the matching PDI) are skipped, and the
/// search stops at the first paragraph separator. `None` when the paragraph has no strong
/// character; P3 then reads it as left to right, and an app may use the interface direction.
pub fn base_direction(text: &str) -> Option<Direction> {
    match unicode_bidi::get_base_direction(text) {
        unicode_bidi::Direction::Ltr => Some(Direction::Ltr),
        unicode_bidi::Direction::Rtl => Some(Direction::Rtl),
        unicode_bidi::Direction::Mixed => None,
    }
}

/// The forms a language tag's Han characters take: `zh-CN`, `zh-SG`, `zh-Hans` and plain `zh` →
/// Simplified; `zh-TW`, `zh-Hant` and the other regions CLDR writes in Traditional characters
/// (`zh-US`, `zh-AU`) → Traditional; `zh-HK`, `zh-MO`, `zh-Hant-HK` and `yue` → HongKong; `ja` →
/// Japanese; `ko` → Korean. `None` for a language not written with Han, which leaves the choice
/// to the font stack.
pub fn han(tag: &str) -> Option<Han> {
    let tag = Subtags::parse(tag);
    let hong_kong = matches!(tag.region.as_deref(), Some("hk" | "mo"));
    // `Hani` names Han without saying which forms, so the region still decides.
    let script = tag.script.as_deref().filter(|s| *s != "hani");
    let script = script.or_else(|| region_script(&tag.language, tag.region.as_deref()?));
    match (tag.language.as_str(), script) {
        ("ja", _) => Some(Han::Japanese),
        ("ko", _) => Some(Han::Korean),
        ("zh" | "yue" | "wuu", Some("hans")) => Some(Han::Simplified),
        ("yue", Some("hant")) => Some(if tag.region.as_deref() == Some("tw") { Han::Traditional } else { Han::HongKong }),
        ("zh" | "wuu", Some("hant")) => Some(if hong_kong { Han::HongKong } else { Han::Traditional }),
        // Cantonese is written in Traditional characters with Hong Kong forms unless it says otherwise.
        ("yue", _) => Some(Han::HongKong),
        ("zh" | "wuu", _) => Some(Han::Simplified),
        _ => None,
    }
}

/// The script subtag of a language in a region where CLDR's likely subtags give it a script
/// other than its usual one: `pa-PK` → `arab` (`pa-Arab-PK`), `zh-TW` → `hant`.
// ponytail: mn-CN → Mong is left out until vertical Mongolian layout is supported.
pub(crate) fn region_script(language: &str, region: &str) -> Option<&'static str> {
    Some(match (language, region) {
        ("az", "iq" | "ir") | ("ha", "cm" | "sd") | ("kk", "af" | "cn" | "ir" | "mn") | ("ku", "iq" | "ir" | "lb") => "arab",
        ("ky", "cn") | ("ms", "cc") | ("pa", "pk") | ("tg", "pk") | ("uz", "af") => "arab",
        ("az", "ru") | ("ku", "am" | "az" | "ge" | "tm") | ("ug", "kz" | "mn") | ("uz", "cn") => "cyrl",
        ("ky", "tr") | ("sr", "me" | "ro" | "tr") => "latn",
        ("sd", "in") => "deva",
        ("zh", "au" | "bn" | "gb" | "gf" | "hk" | "id" | "mo" | "pa" | "pf" | "ph" | "sr" | "th" | "tw" | "us" | "vn") => "hant",
        ("yue", "cn") => "hans",
        _ => return None,
    })
}

/// The scripts a script subtag stands for, including the CLDR unions (`jpan`, `kore`). `None` for
/// one not in [`Script`], so the language's own scripts apply.
fn subtag_scripts(subtag: &str) -> Option<&'static [Script]> {
    use Script::*;
    Some(match subtag {
        "hans" | "hant" | "hani" => &[Hani],
        "jpan" => &[Hani, Hira, Kana],
        "kore" => &[Hang, Hani],
        "arab" => &[Arab],
        "armn" => &[Armn],
        "beng" => &[Beng],
        "cyrl" => &[Cyrl],
        "deva" => &[Deva],
        "ethi" => &[Ethi],
        "geor" => &[Geor],
        "grek" => &[Grek],
        "gujr" => &[Gujr],
        "guru" => &[Guru],
        "hang" => &[Hang],
        "hebr" => &[Hebr],
        "khmr" => &[Khmr],
        "knda" => &[Knda],
        "laoo" => &[Laoo],
        "latn" => &[Latn],
        "mlym" => &[Mlym],
        "mong" => &[Mong],
        "mymr" => &[Mymr],
        "orya" => &[Orya],
        "sinh" => &[Sinh],
        "syrc" => &[Syrc],
        "taml" => &[Taml],
        "telu" => &[Telu],
        "thaa" => &[Thaa],
        "thai" => &[Thai],
        "tibt" => &[Tibt],
        _ => return None,
    })
}

/// A language tag's language, script and region subtags, lowercase.
struct Subtags {
    language: String,
    script: Option<String>,
    region: Option<String>,
}

impl Subtags {
    /// `zh-Hant-HK`, `zh_TW.UTF-8` or `sr_RS@latin` (glibc's script modifier).
    fn parse(tag: &str) -> Subtags {
        let tag = tag.trim().to_ascii_lowercase();
        let base = tag.split(['.', '@']).next().unwrap_or("").replace('_', "-");
        let mut parts = base.split('-').filter(|p| !p.is_empty());
        let language = parts.next().unwrap_or("").to_owned();
        let mut script = match tag.split_once('@').map(|(_, m)| m) {
            Some("latin") => Some("latn".to_owned()),
            Some("cyrillic") => Some("cyrl".to_owned()),
            _ => None,
        };
        let mut region = None;
        for part in parts {
            if part.len() == 4 && part.bytes().all(|b| b.is_ascii_alphabetic()) {
                script.get_or_insert_with(|| part.to_owned());
            } else if super::is_region(part) {
                region = Some(part.to_owned());
                break;
            }
        }
        Subtags { language, script, region }
    }
}

#[cfg(test)]
mod tests {
    use super::Script::*;
    use super::*;

    #[test]
    fn base_direction_of_text() {
        assert_eq!(base_direction("Hello"), Some(Direction::Ltr));
        assert_eq!(base_direction("123 שלום"), Some(Direction::Rtl));
        assert_eq!(base_direction("(مرحبا) hi"), Some(Direction::Rtl));
        // The isolate's Hebrew is skipped; the Latin after it decides.
        assert_eq!(base_direction("\u{2067}שלום\u{2069} hello"), Some(Direction::Ltr));
        assert_eq!(base_direction("\u{2068}abc\u{2069} שלום"), Some(Direction::Rtl));
        // Only the first paragraph counts.
        assert_eq!(base_direction("123\nשלום"), None);
        assert_eq!(base_direction("1920 × 1080"), None);
        assert_eq!(base_direction(""), None);
    }

    /// Every locale the craft-i18n catalogs or an app ship, as the apps spell them.
    #[test]
    fn shipped_locales() {
        for (tag, want, han_forms) in [
            ("en", &[Latn][..], None),
            ("ar", &[Arab], None),
            ("bg", &[Cyrl], None),
            ("cs", &[Latn], None),
            ("da", &[Latn], None),
            ("de", &[Latn], None),
            ("el", &[Grek], None),
            ("es", &[Latn], None),
            ("es-419", &[Latn], None),
            ("et", &[Latn], None),
            ("fi", &[Latn], None),
            ("fr", &[Latn], None),
            ("hr", &[Latn], None),
            ("hu", &[Latn], None),
            ("id", &[Latn], None),
            ("it", &[Latn], None),
            ("ja", &[Hani, Hira, Kana], Some(Han::Japanese)),
            ("ka", &[Geor], None),
            ("kk", &[Cyrl], None),
            ("ko", &[Hang, Hani], Some(Han::Korean)),
            ("lt", &[Latn], None),
            ("nl", &[Latn], None),
            ("pl", &[Latn], None),
            ("pt", &[Latn], None),
            ("pt-br", &[Latn], None),
            ("pt_BR", &[Latn], None),
            ("ru", &[Cyrl], None),
            ("sl", &[Latn], None),
            ("sr", &[Cyrl], None),
            ("sr-latn", &[Latn], None),
            ("sv", &[Latn], None),
            ("te", &[Telu], None),
            ("tr", &[Latn], None),
            ("uk", &[Cyrl], None),
            ("vi", &[Latn], None),
            ("zh", &[Hani], Some(Han::Simplified)),
            ("zh-cn", &[Hani], Some(Han::Simplified)),
            ("zh-CN", &[Hani], Some(Han::Simplified)),
            ("zh-hans", &[Hani], Some(Han::Simplified)),
            ("zh-Hans", &[Hani], Some(Han::Simplified)),
            ("zh-hant", &[Hani], Some(Han::Traditional)),
        ] {
            assert_eq!(scripts(tag), want, "{tag}");
            assert_eq!(han(tag), han_forms, "{tag}");
            let rtl = tag == "ar";
            assert_eq!(direction(tag), if rtl { Direction::Rtl } else { Direction::Ltr }, "{tag}");
        }
    }

    #[test]
    fn regional_han() {
        for (tag, want) in [
            ("zh-TW", Han::Traditional),
            ("zh_TW.UTF-8", Han::Traditional),
            ("zh-Hant-TW", Han::Traditional),
            ("zh-HK", Han::HongKong),
            ("zh-MO", Han::HongKong),
            ("zh-Hant-HK", Han::HongKong),
            ("zh-hant-hk", Han::HongKong),
            ("zh-Hans-HK", Han::Simplified),
            ("zh-SG", Han::Simplified),
            ("zh-US", Han::Traditional),
            ("zh_AU.UTF-8", Han::Traditional),
            ("zh-GB", Han::Traditional),
            ("zh-Hans-US", Han::Simplified),
            ("zh-MY", Han::Simplified),
            ("yue-CN", Han::Simplified),
            ("yue-HK", Han::HongKong),
            ("zh-Hans-SG", Han::Simplified),
            ("yue", Han::HongKong),
            ("yue-Hant", Han::HongKong),
            ("yue-Hant-TW", Han::Traditional),
            ("zh-Hani-HK", Han::HongKong),
            ("zh-Hani", Han::Simplified),
            ("yue-Hans", Han::Simplified),
            ("ja-JP", Han::Japanese),
            ("ko_KR.UTF-8", Han::Korean),
        ] {
            assert_eq!(han(tag), Some(want), "{tag}");
        }
        assert_eq!(han("en"), None);
        assert_eq!(han(""), None);
    }

    #[test]
    fn script_subtag_wins() {
        for (tag, want) in [
            ("sr-Latn-RS", &[Latn][..]),
            ("sr_RS@latin", &[Latn]),
            ("sr-Cyrl", &[Cyrl]),
            ("sr_RS.UTF-8", &[Cyrl]),
            ("az-Cyrl", &[Cyrl]),
            ("pa-Arab", &[Arab]),
            ("pa", &[Guru]),
            ("mn-Mong", &[Mong]),
            ("sd-Deva", &[Deva]),
            ("fr-Zyyy", &[Latn]),
        ] {
            assert_eq!(scripts(tag), want, "{tag}");
        }
        assert_eq!(direction("pa-Arab"), Direction::Rtl);
        assert_eq!(direction("sd-Deva"), Direction::Ltr);
        assert_eq!(direction("he_IL.UTF-8"), Direction::Rtl);
    }

    /// CLDR likely subtags: a region can switch a language to another script.
    #[test]
    fn region_implies_script() {
        for (tag, want) in [
            ("pa-PK", &[Arab][..]),
            ("pa_PK.UTF-8", &[Arab]),
            ("pa-IN", &[Guru]),
            ("uz-AF", &[Arab]),
            ("uz-UZ", &[Latn]),
            ("uz-CN", &[Cyrl]),
            ("az-IR", &[Arab]),
            ("az-AZ", &[Latn]),
            ("sd-IN", &[Deva]),
            ("sd-PK", &[Arab]),
            ("ks-IN", &[Arab]),
            ("ms-CC", &[Arab]),
            ("ms-BN", &[Latn]),
            ("sr-ME", &[Latn]),
            ("kk-CN", &[Arab]),
            ("ug-KZ", &[Cyrl]),
            ("pa-Guru-PK", &[Guru]),
            ("sd-Arab-IN", &[Arab]),
        ] {
            assert_eq!(scripts(tag), want, "{tag}");
        }
        assert_eq!(direction("pa-PK"), Direction::Rtl);
        assert_eq!(direction("uz-AF"), Direction::Rtl);
        assert_eq!(direction("az-IR"), Direction::Rtl);
        assert_eq!(direction("sd-IN"), Direction::Ltr);
        assert_eq!(direction("ug-KZ"), Direction::Ltr);
    }

    /// The usual scripts tag matching relies on agree with the table.
    #[test]
    fn agrees_with_usual_script() {
        for language in ["be", "kk", "ky", "mn", "sr", "tg", "zh", "az", "bs", "ha", "ku", "ms", "uz", "ks", "sd", "ug", "pa"] {
            let usual = crate::usual_script(language).and_then(subtag_scripts);
            assert_eq!(usual, Some(scripts(language)), "{language}");
        }
    }
}
