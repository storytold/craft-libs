//! Ordinals: [`I18n::tord`], and the English rule it falls back on.
//!
//! An ordinal's wording is an `@ordinal` row keyed by the four English forms, `one|two|few|other` (`Move to the {n}st place|Move
//! to the {n}nd place|Move to the {n}rd place|Move to the {n}th place`). A translation lists the forms of its locale's ordinal
//! categories in CLDR order (one for German, Polish, Arabic, Japanese, Spanish and Chinese; two for French), counted from the
//! CLDR data, or is a selector message ([`crate::select`]).

use icu_plurals::{PluralCategory, PluralOperands};

use crate::{Arg, I18n, Lang, fmt, ltr};

impl I18n {
    /// The ordinal wording of `n` in `lang`: the `@ordinal` row of the four English forms `english` (for `one`, `two`, `few` and
    /// `other`: 1st, 2nd, 3rd, 4th), or English's own form for `n` when the language has no row. `{n}` is filled as
    /// [`I18n::format`] writes numbers, as a left-to-right isolate ([`ltr`]) when the wording has a right-to-left letter.
    pub fn tord(&self, lang: Lang, n: u64, english: [&str; 4]) -> String {
        let key = english.join("|");
        let format = self.format();
        let shown = format.integer(n);
        let fallback = english.get(english_index(n)).or(english.last()).copied().unwrap_or("{n}");
        let filled = |text: &str| fmt(text, &[("n", &ltr(&shown))]);
        if let Some(pseudo) = lang.pseudo() {
            return pseudo.show(&filled(fallback));
        }
        let chain = self.locale(lang).map_or(&[][..], |l| l.chain.as_slice());
        let text = chain.iter().find_map(|&i| {
            let locale = self.locales.get(i)?;
            let parsed = locale.parsed();
            if let Some(message) = parsed.messages.get("@ordinal").and_then(|m| m.get(&key)) {
                return self.render_message(locale, message, &[("n", Arg::Count(n))], false);
            }
            let forms = parsed.rows.get("@ordinal")?.get(&key)?;
            let (rules, categories) = locale.ordinal.as_ref()?;
            let category = rules.category_for(PluralOperands::from(n));
            forms.split('|').nth(ordinal_index(categories, category)).map(&filled)
        });
        text.unwrap_or_else(|| filled(fallback))
    }
}

/// Which English ordinal form `n` takes: 0 `one` (1st, 21st, 101st), 1 `two` (2nd, 22nd), 2 `few` (3rd, 23rd), 3 `other` (4th,
/// 11th, 12th, 13th, 111th). English's own rule, so the fallback needs no data.
pub(crate) fn english_index(n: u64) -> usize {
    match (n % 10, n % 100) {
        (_, 11..=13) => 3,
        (1, _) => 0,
        (2, _) => 1,
        (3, _) => 2,
        _ => 3,
    }
}

/// The index of `category` among a locale's ordinal `categories`; a category whole numbers never reach takes the last.
pub(crate) fn ordinal_index(categories: &[PluralCategory], category: PluralCategory) -> usize {
    categories.iter().position(|c| *c == category).unwrap_or(categories.len().saturating_sub(1))
}
