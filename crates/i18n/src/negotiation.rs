//! Locale candidate negotiation from tags and the environment, and plural rules.

use super::*;

/// Codes to try for a locale tag, most specific first: `zh_TW.UTF-8` → `zh-tw`, `zh-hant`, `zh`.
/// Chinese without a script picks one by region; Portuguese falls back to `pt-br` last, when
/// there is no `pt`. A glibc script modifier (`sr_RS@latin`) becomes the script subtag. With a
/// script (`sr-Latn-RS`), a code without one is tried only when that is its language's usual
/// script, so Latin Serbian never picks Cyrillic `sr`, while `sr-Cyrl-RS` does. A region that
/// implies a script (CLDR likely subtags) counts as that script after the tag as written:
/// `pa-PK` → `pa-pk`, `pa-arab-pk`, `pa-arab`.
pub(super) fn candidates(tag: &str) -> Vec<String> {
    let tag = tag.trim();
    if tag.len() > MAX_TAG_LEN {
        return Vec::new();
    }
    candidates_unchecked(tag)
}

/// A locale tag is no longer than this: a language, script, region and a few variants or a
/// glibc encoding and modifier fit with room to spare. A longer one is no locale, and isn't
/// split into a fallback for every subtag.
const MAX_TAG_LEN: usize = 128;

/// `candidates` without the length check: the implied-script recursion may push a tag just past the limit.
fn candidates_unchecked(tag: &str) -> Vec<String> {
    let tag = tag.to_ascii_lowercase();
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
        out.extend(candidates_unchecked(&parts.join("-")));
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
pub(super) fn posix_env_tags(var: impl Fn(&str) -> Option<String>) -> Vec<String> {
    let mut tags: Vec<String> = var("LANGUAGE").map(|v| v.split(':').map(str::to_owned).collect()).unwrap_or_default();
    tags.extend(["LC_ALL", "LC_MESSAGES", "LANG"].into_iter().filter_map(&var));
    tags.retain(|t| !t.is_empty());
    tags
}

/// `⟦text⟧`, each line marked on its own, since multi-line labels are often drawn a line at a time.
pub(super) fn marked(text: &str) -> String {
    format!("⟦{}⟧", text.replace('\n', "⟧\n⟦"))
}

/// The script a code without one means, for languages written in more than one (CLDR likely
/// subtags). Any other language is taken to have one script.
// ponytail: the multi-script languages a Craft app could plausibly ship; add a row with the locale.
pub(super) fn usual_script(language: &str) -> Option<&'static str> {
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
pub(super) fn is_region(subtag: &str) -> bool {
    (subtag.len() == 2 && subtag.bytes().all(|b| b.is_ascii_alphabetic())) || (subtag.len() == 3 && subtag.bytes().all(|b| b.is_ascii_digit()))
}

/// A locale with no catalogs (English, the pseudo locale).
pub(super) fn bare(code: &str) -> Locale {
    Locale {
        code: code.to_owned(),
        shared: Cow::Borrowed(""),
        app: Cow::Borrowed(""),
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
pub(super) fn plural_rules(code: &str) -> Option<(PluralRules, Vec<PluralCategory>)> {
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

pub(super) fn rank(c: &PluralCategory) -> u8 {
    match c {
        PluralCategory::Zero => 0,
        PluralCategory::One => 1,
        PluralCategory::Two => 2,
        PluralCategory::Few => 3,
        PluralCategory::Many => 4,
        PluralCategory::Other => 5,
    }
}
