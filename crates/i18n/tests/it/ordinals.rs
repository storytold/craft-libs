//! Ordinals (`@ordinal` rows, `I18n::tord`): CLDR's ordinal categories per language, and the English fallback.

use craft_i18n::I18n;

use crate::common::{build, i18n, lang};

/// The English source of the row: the forms for `one`, `two`, `few` and `other`.
const MOVE: [&str; 4] = ["Move to the {n}st place", "Move to the {n}nd place", "Move to the {n}rd place", "Move to the {n}th place"];
const KEY: &str = "Move to the {n}st place|Move to the {n}nd place|Move to the {n}rd place|Move to the {n}th place";

fn row(forms: &str) -> String {
    format!("@ordinal\t{KEY}\t{forms}\n")
}

fn say(i18n: &I18n, code: &str, n: u64) -> String {
    i18n.tord(lang(i18n, code), n, MOVE)
}

// English takes its four ordinal forms from the number: 1st, 2nd, 3rd, 4th, but 11th, 12th and 13th, and 21st, 101st.
#[test]
fn english_ordinals_follow_the_last_digits() {
    let i18n = i18n(&[]);
    let shown: Vec<String> =
        [1, 2, 3, 4, 11, 12, 13, 21, 22, 23, 101, 111, 112, 1000].iter().map(|&n| i18n.tord(craft_i18n::Lang::EN, n, MOVE)).collect();
    let expected = [
        "Move to the 1st place",
        "Move to the 2nd place",
        "Move to the 3rd place",
        "Move to the 4th place",
        "Move to the 11th place",
        "Move to the 12th place",
        "Move to the 13th place",
        "Move to the 21st place",
        "Move to the 22nd place",
        "Move to the 23rd place",
        "Move to the 101st place",
        "Move to the 111th place",
        "Move to the 112th place",
        "Move to the 1,000th place",
    ];
    assert_eq!(shown, expected);
}

// French has two ordinal categories, one (1re) and other (2e, 3e); German, Polish, Arabic and Japanese have one (other).
#[test]
fn a_language_lists_its_own_ordinal_categories() {
    let fr = row("Déplacer à la {n}re place|Déplacer à la {n}e place");
    let de = row("An die {n}. Stelle verschieben");
    let pl = row("Przenieś na {n}. miejsce");
    let ar = row("انقل إلى الموضع {n}");
    let ja = row("{n} 番目に移動");
    let i18n = i18n(&[("fr", &fr, ""), ("de", &de, ""), ("pl", &pl, ""), ("ar", &ar, ""), ("ja", &ja, "")]);
    assert_eq!(say(&i18n, "fr", 1), "Déplacer à la 1re place");
    assert_eq!(say(&i18n, "fr", 2), "Déplacer à la 2e place");
    assert_eq!(say(&i18n, "fr", 21), "Déplacer à la 21e place");
    assert_eq!(say(&i18n, "de", 3), "An die 3. Stelle verschieben");
    assert_eq!(say(&i18n, "de", 21), "An die 21. Stelle verschieben");
    assert_eq!(say(&i18n, "pl", 5), "Przenieś na 5. miejsce");
    assert_eq!(say(&i18n, "ja", 1), "1 番目に移動");
    // Arabic digits follow the region (the test's is en-US), isolated left to right inside the right-to-left sentence.
    assert!(say(&i18n, "ar", 7).contains('7'));
}

// A row with the wrong number of forms for the locale is skipped and reported, and the English shows: French needs two
// ordinal forms, and the four of English are the cardinal count of no language here.
#[test]
fn the_form_count_is_the_locales_ordinal_categories() {
    let one_form = row("Déplacer à la {n}e place");
    let (i18n, _) = build(&[("fr", &one_form, "")]);
    assert_eq!(i18n.check_all().len(), 1, "{:?}", i18n.check_all());
    assert!(i18n.check_all()[0].contains("2 nonempty ordinal forms"), "{:?}", i18n.check_all());
    assert_eq!(say(&i18n, "fr", 3), "Move to the 3rd place", "English shows for a skipped row");
    let two_forms_for_german = row("An die {n}. Stelle|An die {n}.");
    let (i18n, _) = build(&[("de", &two_forms_for_german, "")]);
    assert!(i18n.check_all()[0].contains("1 nonempty ordinal form"), "{:?}", i18n.check_all());
}

// The source lists the four English forms: another count is no ordinal row.
#[test]
fn an_ordinal_source_has_four_english_forms() {
    let bad = "@ordinal\tMove to the {n}st place|Move to the {n}th place\tAn die {n}. Stelle\n";
    let (i18n, _) = build(&[("de", bad, "")]);
    assert!(i18n.check_all()[0].contains("ordinal source text must be one|two|few|other"), "{:?}", i18n.check_all());
}

// A language the data has no rules for, and the pseudo locale, show the English forms.
#[test]
fn an_unknown_language_shows_english() {
    let (i18n, problems) = build(&[("xx", &row("X {n}"), "")]);
    assert!(problems.iter().any(|p| p.contains("no plural rules")), "{problems:?}");
    assert_eq!(say(&i18n, "xx", 2), "Move to the 2nd place");
}
