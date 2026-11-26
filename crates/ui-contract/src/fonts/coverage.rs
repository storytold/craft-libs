//! The coverage check each app runs over its shipped translations.

use std::fmt;

use super::FontRef;
use super::faces::draws;
use super::stack::Stack;

/// How many uncovered characters a failure names.
const MAX_LISTED: usize = 50;

/// A string the UI shows: one translation, or a sample of content text.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct Sample<'a> {
    /// The language tag the string belongs to.
    pub language: &'a str,
    /// The message key, or a name for a content sample.
    pub key: &'a str,
    /// The text to check.
    pub text: &'a str,
}

impl<'a> Sample<'a> {
    /// The string `text` of `language`, called `key`.
    pub fn new(language: &'a str, key: &'a str, text: &'a str) -> Self {
        Self { language, key, text }
    }
}

/// A character the stack can't draw.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Missing {
    /// Language tag of the string.
    pub language: String,
    /// Message key, or the name of a content sample.
    pub key: String,
    /// The string that holds the uncovered character.
    pub text: String,
    /// A character of `text` that no face of the family has.
    pub character: char,
}

impl fmt::Display for Missing {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}: U+{:04X} {:?} in {:?}", self.language, self.key, u32::from(self.character), self.character, self.text)
    }
}

/// Every character in `samples` that no face of `family` in `stack` has. `with_system` also asks
/// the stack's system font provider (the `system_fonts` feature, or [`super::UiFonts::system_provider`]),
/// for an app that relies on installed fonts for some languages.
///
/// Whitespace, control characters and invisible format characters (joiners, direction marks,
/// variation selectors) are skipped: they draw nothing.
pub fn missing_glyphs<'a>(stack: &Stack, family: &FontRef, samples: impl IntoIterator<Item = Sample<'a>>, with_system: bool) -> Vec<Missing> {
    let mut probe = stack.probe(with_system);
    let mut missing = Vec::new();
    for sample in samples {
        let mut seen = Vec::new();
        for c in sample.text.chars().filter(|c| draws(*c)) {
            if !seen.contains(&c) && !probe.has(family, c) {
                seen.push(c);
                missing.push(Missing {
                    language: sample.language.to_owned(),
                    key: sample.key.to_owned(),
                    text: sample.text.to_owned(),
                    character: c,
                });
            }
        }
    }
    missing
}

/// Fail, naming each language, string and character, if the stack can't draw all of `samples`
/// in the proportional family without system fonts. For an app's coverage test.
pub fn assert_covers<'a>(stack: &Stack, samples: impl IntoIterator<Item = Sample<'a>>) {
    fail_on(&missing_glyphs(stack, &FontRef::proportional(), samples, false));
}

/// [`assert_covers`] with the stack's system font provider asked too. The result depends on the
/// machine's fonts; use it for languages the app leaves to them.
pub fn assert_covers_with_system<'a>(stack: &Stack, samples: impl IntoIterator<Item = Sample<'a>>) {
    fail_on(&missing_glyphs(stack, &FontRef::proportional(), samples, true));
}

fn fail_on(missing: &[Missing]) {
    let lines = || {
        let shown: Vec<String> = missing.iter().take(MAX_LISTED).map(|m| format!("  {m}")).collect();
        let more = if missing.len() > shown.len() { format!("\n  … and {} more", missing.len() - shown.len()) } else { String::new() };
        format!("{}{more}", shown.join("\n"))
    };
    assert!(missing.is_empty(), "the UI fonts can't draw {} characters:\n{}", missing.len(), lines());
}

/// A few common letters of an ISO 15924 script, for [`Stack::covers_language`]. `None` for a
/// script without a sample, which counts as covered.
pub(super) fn script_sample(script: &str) -> Option<&'static str> {
    Some(match script {
        "Arab" => "عربي",
        "Armn" => "Հայերեն",
        "Beng" => "বাংলা",
        "Bopo" => "ㄅㄆㄇ",
        "Cyrl" => "Кирилица",
        "Deva" => "हिन्दी",
        "Ethi" => "አማርኛ",
        "Geor" => "ქართული",
        "Grek" => "Ελληνικά",
        "Gujr" => "ગુજરાતી",
        "Guru" => "ਪੰਜਾਬੀ",
        "Hang" => "한국어",
        "Hani" => "日本中文",
        "Hebr" => "עברית",
        "Hira" => "の",
        "Kana" => "ファイル",
        "Khmr" => "ខ្មែរ",
        "Knda" => "ಕನ್ನಡ",
        "Laoo" => "ລາວ",
        "Latn" => "Latin",
        "Mlym" => "മലയാളം",
        "Mong" => "ᠮᠣᠩᠭᠣᠯ",
        "Mymr" => "မြန်မာ",
        "Orya" => "ଓଡ଼ିଆ",
        "Sinh" => "සිංහල",
        "Syrc" => "ܣܘܪܝܝܐ",
        "Taml" => "தமிழ்",
        "Telu" => "తెలుగు",
        "Thaa" => "ދިވެހި",
        "Thai" => "ไทย",
        "Tibt" => "བོད་ཡིག",
        _ => return None,
    })
}
