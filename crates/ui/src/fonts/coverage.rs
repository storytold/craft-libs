//! The coverage check each app runs over its shipped translations.

use std::fmt;

use egui::{FontFamily, FontId};

use super::Stack;

/// A string the UI shows: one translation, or a sample of content text.
#[derive(Clone, Copy, Debug)]
pub struct Sample<'a> {
    /// The language tag the string belongs to.
    pub language: &'a str,
    /// The message key, or a name for a content sample.
    pub key: &'a str,
    pub text: &'a str,
}

/// A character the stack can't draw.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Missing {
    pub language: String,
    pub key: String,
    pub text: String,
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
pub fn missing_glyphs<'a>(stack: &Stack, family: &FontFamily, samples: impl IntoIterator<Item = Sample<'a>>, with_system: bool) -> Vec<Missing> {
    let mut fonts = stack.fonts(with_system);
    let font_id = FontId::new(14.0, family.clone());
    let mut missing = Vec::new();
    for sample in samples {
        let mut seen = Vec::new();
        for c in sample.text.chars().filter(|c| draws(*c)) {
            if !seen.contains(&c) && !fonts.has_glyph(&font_id, c) {
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
    fail_on(&missing_glyphs(stack, &FontFamily::Proportional, samples, false));
}

/// [`assert_covers`] with the stack's system font provider asked too. The result depends on the
/// machine's fonts; use it for languages the app leaves to them.
pub fn assert_covers_with_system<'a>(stack: &Stack, samples: impl IntoIterator<Item = Sample<'a>>) {
    fail_on(&missing_glyphs(stack, &FontFamily::Proportional, samples, true));
}

#[allow(clippy::panic, reason = "a test assertion")]
fn fail_on(missing: &[Missing]) {
    if !missing.is_empty() {
        let lines: Vec<String> = missing.iter().take(50).map(|m| format!("  {m}")).collect();
        let more = if missing.len() > lines.len() { format!("\n  … and {} more", missing.len() - lines.len()) } else { String::new() };
        panic!("the UI fonts can't draw {} characters:\n{}{more}", missing.len(), lines.join("\n"));
    }
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

/// Whether `c` shows a glyph: not whitespace, a control, a format character or another invisible
/// mark.
pub(super) fn draws(c: char) -> bool {
    !(c.is_whitespace()
        || c.is_control()
        || matches!(
            c,
            '\u{00AD}' | '\u{034F}' | '\u{061C}' | '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{206F}' | '\u{FEFF}'
        )
        || matches!(c, '\u{FE00}'..='\u{FE0F}' | '\u{E0100}'..='\u{E01EF}'))
}
