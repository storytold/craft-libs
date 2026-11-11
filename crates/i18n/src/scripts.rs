//! The scripts a language is written in, its direction, and which regional forms its Han
//! characters take, for picking fonts and laying out the interface.

/// A writing system, by its ISO 15924 code.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum Script {
    #[allow(missing_docs)]
    Arab,
    #[allow(missing_docs)]
    Armn,
    #[allow(missing_docs)]
    Beng,
    #[allow(missing_docs)]
    Cyrl,
    #[allow(missing_docs)]
    Deva,
    #[allow(missing_docs)]
    Ethi,
    #[allow(missing_docs)]
    Geor,
    #[allow(missing_docs)]
    Grek,
    #[allow(missing_docs)]
    Gujr,
    #[allow(missing_docs)]
    Guru,
    #[allow(missing_docs)]
    Hang,
    #[allow(missing_docs)]
    Hani,
    #[allow(missing_docs)]
    Hebr,
    #[allow(missing_docs)]
    Hira,
    #[allow(missing_docs)]
    Kana,
    #[allow(missing_docs)]
    Khmr,
    #[allow(missing_docs)]
    Knda,
    #[allow(missing_docs)]
    Laoo,
    #[allow(missing_docs)]
    Latn,
    #[allow(missing_docs)]
    Mlym,
    #[allow(missing_docs)]
    Mong,
    #[allow(missing_docs)]
    Mymr,
    #[allow(missing_docs)]
    Orya,
    #[allow(missing_docs)]
    Sinh,
    #[allow(missing_docs)]
    Syrc,
    #[allow(missing_docs)]
    Taml,
    #[allow(missing_docs)]
    Telu,
    #[allow(missing_docs)]
    Thaa,
    #[allow(missing_docs)]
    Thai,
    #[allow(missing_docs)]
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
#[non_exhaustive]
pub enum Direction {
    /// Left to right.
    Ltr,
    /// Right to left.
    Rtl,
}

impl Direction {
    /// Whether the direction is right to left.
    pub const fn is_rtl(self) -> bool {
        matches!(self, Direction::Rtl)
    }
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
