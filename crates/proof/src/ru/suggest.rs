//! Russian spelling suggestions, best first: the shared ranking ([`crate::affix_suggest`]) with
//! Russian's own letter groups and confusions, since Russian Hunspell dictionaries carry no `MAP`
//! or `REP` tables (`е`/`ё` first, then unstressed vowels, voiced and voiceless pairs, `жы` →
//! `жи`, `тся`/`ться`, dropped or doubled consonants; up to three letters changed), and its rule
//! for a missing space (`незнаю` → `не знаю`, `вобщем` → `в общем`).

use crate::affix::AffixDict;
use crate::affix_suggest::{COST_OTHER_MAP, COST_SPLIT, COST_SPLIT_LONG, Split, Tuning};

/// Letters confused with each other: a slip inside a group is cheaper than another typo.
const MAP: &[&str] = &["её", "оа", "еия", "зс", "дт", "бп", "вф", "гк", "жш", "ьъ", "еэ", "ыи"];
/// Common confusions: a written sequence and what was meant.
const REP: &[(&str, &str)] = &[
    ("жы", "жи"),
    ("шы", "ши"),
    ("чя", "ча"),
    ("щя", "ща"),
    ("чю", "чу"),
    ("щю", "щу"),
    ("тся", "ться"),
    ("ться", "тся"),
    ("сч", "щ"),
    ("щ", "сч"),
    ("нн", "н"),
    ("н", "нн"),
    ("сс", "с"),
    ("с", "сс"),
    ("лл", "л"),
    ("л", "лл"),
    ("тт", "т"),
    ("т", "тт"),
    ("мм", "м"),
    ("м", "мм"),
    ("рр", "р"),
    ("р", "рр"),
    ("ье", "ъе"),
    ("ъе", "ье"),
    ("раст", "рост"),
    ("рост", "раст"),
    ("лож", "лаг"),
    ("лаг", "лож"),
    // Unpronounced consonants: `здраствуй`, `чуствовать`, `солнце`, `лесница`, `праздник`.
    ("аст", "авст"),
    ("уст", "увст"),
    ("лнц", "лц"),
    ("лц", "лнц"),
    ("сн", "стн"),
    ("стн", "сн"),
    ("зн", "здн"),
    ("здн", "зн"),
    ("ц", "тс"),
    ("тс", "ц"),
];
/// Short words often run into the next one (`не знаю`, `в общем`, `на верху`, `потому что`).
const LEADING_WORDS: &[&str] = &[
    "не",
    "ни",
    "в",
    "во",
    "на",
    "с",
    "со",
    "к",
    "ко",
    "о",
    "об",
    "обо",
    "от",
    "ото",
    "до",
    "из",
    "изо",
    "за",
    "по",
    "под",
    "над",
    "при",
    "про",
    "без",
    "для",
    "у",
    "а",
    "и",
    "но",
    "да",
    "же",
    "как",
    "так",
    "то",
    "что",
    "уже",
    "еще",
    "ещё",
    "все",
    "всё",
    "потому",
    "чтобы",
    "это",
];
/// Prepositions among [`LEADING_WORDS`].
const PREPOSITIONS: &[&str] = &[
    "в", "во", "на", "с", "со", "к", "ко", "о", "об", "обо", "от", "ото", "до", "из", "изо", "за", "по", "под", "над", "при", "про", "без", "для",
    "у",
];
/// Short words often run into the one before (`мог бы`, `он же`, `так ли`).
const TRAILING_WORDS: &[&str] = &["ли", "же", "бы", "ж", "б", "что"];
/// The Russian alphabet, when the dictionary has no `TRY` line.
const ALPHABET: &str = "оеаинтсрвлкмдпуяыьгзбчйхжшюцщэфъё";

/// `е` written for `ё`, or the other way round, is barely a slip.
const COST_YO: u32 = 3;
/// A one- or two-letter preposition run into the next word (`вдоме` → `в доме`).
const COST_SPLIT_PREPOSITION: u32 = 17;

/// What Russian adds to its dictionary's tables.
const TUNING: Tuning =
    Tuning { split: Split::Custom(split), alphabet: ALPHABET, map: MAP, rep: REP, rep_cost: 9, max_map_changes: 3, map_cost, ..Tuning::DEFAULT };

/// The cost of changing `from` to `to` within a letter group: `е`/`ё` cheapest.
fn map_cost(from: char, to: char) -> u32 {
    if matches!((from, to), ('е', 'ё') | ('ё', 'е')) { COST_YO } else { COST_OTHER_MAP }
}

/// A missing space: cheap after a short leading word or before a short trailing one, dearer after
/// a short preposition or between two longer words, never leaving a stray short half.
fn split(a: &str, b: &str) -> Option<u32> {
    let long = |p: &str| p.chars().count() >= 3;
    // A preposition takes a noun, not a verb (`с генерировать` isn't a fix for
    // `сгенерировать`); after one or two letters a split is likelier wrong than a typo.
    let preposition = PREPOSITIONS.contains(&a);
    if preposition && ["ть", "ться", "ти", "чь"].iter().any(|e| b.ends_with(e)) {
        return None;
    }
    if preposition && a.chars().count() <= 2 && LEADING_WORDS.contains(&a) && long(b) {
        Some(COST_SPLIT_PREPOSITION)
    } else if (LEADING_WORDS.contains(&a) && long(b)) || (long(a) && TRAILING_WORDS.contains(&b)) {
        Some(COST_SPLIT)
    } else if long(a) && long(b) {
        Some(COST_SPLIT_LONG)
    } else {
        None
    }
}

/// Spelling suggestions for a Russian word, best first, at most `max`, in the word's letter case.
pub fn suggest(word: &str, max: usize) -> Vec<String> {
    suggest_in(&super::dictionary(), word, max)
}

/// [`suggest`] against a given dictionary.
pub fn suggest_in(d: &AffixDict, word: &str, max: usize) -> Vec<String> {
    crate::affix_suggest::suggest_in(d, word, max, &TUNING)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Russian's own costs: `е`/`ё` is the cheapest slip, so a `ё` and one more group slip
    /// (made-up `нёснаю`) beat a missing space; a REP confusion beats a `ё` and a group slip;
    /// a preposition is never split off a verb, and after a one-letter one a typo is likelier.
    #[test]
    fn russian_costs_rank_its_slips() {
        let dict = |words: &[&str]| AffixDict::from_hunspell("", &format!("{}\n{}\n", words.len(), words.join("\n")), &[]).unwrap();
        assert_eq!(suggest_in(&dict(&["не", "знаю", "нёснаю"]), "незнаю", 1), ["нёснаю"]);
        assert_eq!(suggest_in(&dict(&["еча", "ёчи"]), "ечя", 1), ["еча"]);
        assert!(!suggest_in(&dict(&["с", "генерировать"]), "сгенерировать", 8).contains(&"с генерировать".into()));
        let s = suggest_in(&dict(&["в", "доме"]), "вдоме", 4);
        assert_eq!(s.first().map(String::as_str), Some("доме"), "{s:?}");
        assert!(s.contains(&"в доме".into()), "{s:?}");
    }
}
