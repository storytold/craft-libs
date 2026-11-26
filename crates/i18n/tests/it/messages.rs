//! Typed messages (`I18n::tm`), selector messages in `@plural` rows, nouns and their forms: the examples of the design, in
//! German, Polish, Arabic, French and Japanese, from catalogs written here.

use craft_i18n::{Arg, I18n, Lang, strip_isolates};

use crate::common::{build, i18n, lang};

/// Catalog rows, each written with `\t` between its three columns.
fn rows(lines: &[&str]) -> String {
    let mut text = lines.join("\n");
    text.push('\n');
    text
}

const DELETE: &str = "Delete this {item :noun}?";

fn delete(i18n: &I18n, code: &str, noun: &str) -> String {
    strip_isolates(&i18n.tm(lang(i18n, code), "", DELETE, &[("item", Arg::Noun(noun))]))
}

fn german() -> String {
    rows(&[
        "@grammar\tgenders\tmasculine feminine neuter",
        "@noun\tpage\tfeminine: Seite",
        "@noun\tconnector\tmasculine: Verbinder",
        "@noun\tpicture\tneuter: Bild",
        &format!(
            "\t{DELETE}\t.input {{$item :noun}} .match $item feminine {{{{Diese {{$item}} löschen?}}}} masculine {{{{Diesen {{$item}} löschen?}}}} * {{{{Dieses {{$item}} löschen?}}}}"
        ),
    ])
}

fn polish() -> String {
    rows(&[
        "@grammar\tgenders\tmasculine feminine neuter",
        "@grammar\tforms\taccusative genitive instrumental locative",
        "@noun\tpage\tfeminine: strona; accusative=stronę",
        "@noun\tconnector\tmasculine: łącznik",
        "@noun\tpicture\tmasculine: obraz",
        &format!(
            "\t{DELETE}\t.input {{$item :noun}} .match $item feminine {{{{Usunąć tę {{$item :noun form=accusative}}?}}}} neuter {{{{Usunąć to {{$item :noun form=accusative}}?}}}} * {{{{Usunąć ten {{$item :noun form=accusative}}?}}}}"
        ),
    ])
}

fn arabic() -> String {
    rows(&[
        "@grammar\tgenders\tmasculine feminine",
        "@grammar\tforms\tdefinite",
        "@noun\tpage\tfeminine: صفحة; definite=الصفحة",
        "@noun\tconnector\tmasculine: موصل; definite=الموصل",
        "@noun\tpicture\tfeminine: صورة; definite=الصورة",
        &format!(
            "\t{DELETE}\t.input {{$item :noun}} .match $item feminine {{{{حذف هذه {{$item :noun form=definite}}؟}}}} * {{{{حذف هذا {{$item :noun form=definite}}؟}}}}"
        ),
    ])
}

fn japanese() -> String {
    rows(&["@noun\tpage\tページ", "@noun\tconnector\tコネクタ", "@noun\tpicture\t画像", &format!("\t{DELETE}\tこの{{item}}を削除しますか？")])
}

// A gender agreement: German chooses the demonstrative by the noun's gender.
#[test]
fn german_agrees_the_demonstrative_with_the_nouns_gender() {
    let de = german();
    let i18n = i18n(&[("de", &de, "")]);
    assert_eq!(delete(&i18n, "de", "page"), "Diese Seite löschen?");
    assert_eq!(delete(&i18n, "de", "connector"), "Diesen Verbinder löschen?");
    assert_eq!(delete(&i18n, "de", "picture"), "Dieses Bild löschen?");
}

// Polish chooses by gender and puts the noun in the accusative; a noun that does not list a form shows its base form.
#[test]
fn polish_uses_the_accusative_and_the_base_form_where_none_is_listed() {
    let pl = polish();
    let i18n = i18n(&[("pl", &pl, "")]);
    assert_eq!(delete(&i18n, "pl", "page"), "Usunąć tę stronę?");
    assert_eq!(delete(&i18n, "pl", "connector"), "Usunąć ten łącznik?");
    assert_eq!(delete(&i18n, "pl", "picture"), "Usunąć ten obraz?");
}

// Arabic chooses the demonstrative by gender and uses the noun's definite form; the right-to-left sentence isolates the noun.
#[test]
fn arabic_uses_the_definite_form_and_isolates_the_noun() {
    let ar = arabic();
    let i18n = i18n(&[("ar", &ar, "")]);
    assert_eq!(delete(&i18n, "ar", "page"), "حذف هذه الصفحة؟");
    assert_eq!(delete(&i18n, "ar", "connector"), "حذف هذا الموصل؟");
    let raw = i18n.tm(lang(&i18n, "ar"), "", DELETE, &[("item", Arg::Noun("picture"))]);
    assert!(raw.contains('\u{2068}') && raw.contains('\u{2069}'), "the noun is a first-strong isolate: {raw:?}");
}

// Japanese has no gender: a plain row takes the noun's translation.
#[test]
fn japanese_has_a_plain_row_and_no_selector() {
    let ja = japanese();
    let i18n = i18n(&[("ja", &ja, "")]);
    assert_eq!(delete(&i18n, "ja", "page"), "このページを削除しますか？");
    assert_eq!(delete(&i18n, "ja", "picture"), "この画像を削除しますか？");
}

// A message the language cannot show shows in English, whole: a noun with no row, or a value of the wrong type.
#[test]
fn what_the_language_cannot_show_shows_in_english() {
    let de = german();
    let i18n = i18n(&[("de", &de, "")]);
    assert_eq!(delete(&i18n, "de", "layer"), "Delete this layer?");
    let de = lang(&i18n, "de");
    assert_eq!(i18n.tm(de, "", DELETE, &[("item", Arg::Text("thing"))]), "Delete this thing?");
    assert_eq!(i18n.tm(de, "", DELETE, &[]), "Delete this {item :noun}?", "no value: the placeholder stays as written");
    assert_eq!(i18n.tm(Lang::EN, "", DELETE, &[("item", Arg::Noun("page"))]), "Delete this page?");
}

// The `@plural` row of Arabic is a selector message: zero, one and two say the number in words, and the others carry it.
#[test]
fn arabic_counts_choose_among_six_forms() {
    let ar = rows(&[
        "@plural\t{n} shape selected|{n} shapes selected\t.input {$n :number} .match $n zero {{لا أشكال محددة}} one {{شكل واحد محدد}} two {{شكلان محددان}} few {{{$n} أشكال محددة}} many {{{$n} شكلًا محددًا}} * {{{$n} شكل محدد}}",
    ]);
    let i18n = i18n(&[("ar", &ar, "")]);
    let ar = lang(&i18n, "ar");
    let say = |n: u64| strip_isolates(&i18n.trn(ar, n, "{n} shape selected", "{n} shapes selected"));
    assert_eq!(say(0), "لا أشكال محددة");
    assert_eq!(say(1), "شكل واحد محدد");
    assert_eq!(say(2), "شكلان محددان");
    assert_eq!(say(3), "3 أشكال محددة");
    assert_eq!(say(10), "10 أشكال محددة");
    assert_eq!(say(11), "11 شكلًا محددًا");
    assert_eq!(say(100), "100 شكل محدد");
    assert_eq!(strip_isolates(&i18n.trn_decimal(ar, 1.5, 1, "{n} shape selected", "{n} shapes selected")), "1.5 شكل محدد");
    // For a document or a file name the number is 0-9 with no isolate marks.
    assert_eq!(i18n.trn_plain(ar, 1234, "{n} shape selected", "{n} shapes selected"), "1234 شكلًا محددًا");
}

// Polish: one, few and many for whole numbers, and `other` for fractions (the catch-all).
#[test]
fn polish_counts_choose_one_few_many_and_the_catch_all_for_fractions() {
    let pl = rows(&[
        "@plural\t{n} shape selected|{n} shapes selected\t.input {$n :number} .match $n one {{Zaznaczono {$n} kształt}} few {{Zaznaczono {$n} kształty}} many {{Zaznaczono {$n} kształtów}} * {{Zaznaczono {$n} kształtu}}",
    ]);
    let i18n = i18n(&[("pl", &pl, "")]);
    let pl = lang(&i18n, "pl");
    let say = |n: u64| i18n.trn(pl, n, "{n} shape selected", "{n} shapes selected");
    assert_eq!(say(1), "Zaznaczono 1 kształt");
    for n in [2, 3, 4, 22, 23, 24, 102] {
        assert_eq!(say(n), format!("Zaznaczono {n} kształty"));
    }
    for n in [0, 5, 12, 14, 21, 25, 100] {
        assert_eq!(say(n), format!("Zaznaczono {n} kształtów"));
    }
    assert_eq!(i18n.trn_decimal(pl, 1.5, 1, "{n} shape selected", "{n} shapes selected"), "Zaznaczono 1.5 kształtu");
}

// An exact number is the best match for that number, then its category, then the catch-all.
#[test]
fn an_exact_number_wins_over_its_category() {
    let de = rows(&["@plural\t{n} file|{n} files\t.input {$n :number} .match $n 0 {{Keine Datei}} one {{{$n} Datei}} * {{{$n} Dateien}}"]);
    let i18n = i18n(&[("de", &de, "")]);
    let de = lang(&i18n, "de");
    assert_eq!(i18n.trn(de, 0, "{n} file", "{n} files"), "Keine Datei");
    assert_eq!(i18n.trn(de, 1, "{n} file", "{n} files"), "1 Datei");
    assert_eq!(i18n.trn(de, 2, "{n} file", "{n} files"), "2 Dateien");
    assert_eq!(i18n.trn(de, 1000, "{n} file", "{n} files"), "1,000 Dateien", "the count is written for the region");
}

// A plural form is chosen from the digits shown: French 1,5 is singular.
#[test]
fn a_fraction_takes_the_category_of_the_digits_shown() {
    let fr = rows(&["@plural\t{n} page|{n} pages\t.input {$n :number} .match $n one {{{$n} page}} * {{{$n} pages}}"]);
    let i18n = i18n(&[("fr", &fr, "")]);
    let fr = lang(&i18n, "fr");
    assert_eq!(i18n.trn_decimal(fr, 1.5, 1, "{n} page", "{n} pages"), "1.5 page");
    assert_eq!(i18n.trn_decimal(fr, 2.5, 1, "{n} page", "{n} pages"), "2.5 pages");
    assert_eq!(i18n.trn(fr, 0, "{n} page", "{n} pages"), "0 page", "French counts zero as one");
}

// French ordinals: the selector message chooses `1re` against `2e`.
#[test]
fn a_french_ordinal_chooses_its_form_with_a_selector() {
    let key = "Move to the {n}st place|Move to the {n}nd place|Move to the {n}rd place|Move to the {n}th place";
    let fr = rows(&[&format!(
        "@ordinal\t{key}\t.input {{$n :number select=ordinal}} .match $n one {{{{Déplacer à la {{$n}}re place}}}} * {{{{Déplacer à la {{$n}}e place}}}}"
    )]);
    let i18n = i18n(&[("fr", &fr, "")]);
    let fr = lang(&i18n, "fr");
    let english = ["Move to the {n}st place", "Move to the {n}nd place", "Move to the {n}rd place", "Move to the {n}th place"];
    assert_eq!(i18n.tord(fr, 1, english), "Déplacer à la 1re place");
    assert_eq!(i18n.tord(fr, 2, english), "Déplacer à la 2e place");
    assert_eq!(i18n.tord(fr, 11, english), "Déplacer à la 11e place");
    assert_eq!(i18n.tord(fr, 21, english), "Déplacer à la 21e place");
}

// A plain row with typed placeholders takes its values, written for the region.
#[test]
fn typed_values_fill_a_plain_row() {
    let de = rows(&["\tCopy {n :number} of {name} ({share :number})\t{n} Kopien von {name} ({share})"]);
    let i18n = i18n(&[("de", &de, "")]);
    let de = lang(&i18n, "de");
    let source = "Copy {n :number} of {name} ({share :number})";
    let args = [("n", Arg::Count(1234)), ("name", Arg::Name("Seite 1")), ("share", Arg::number(0.5, 2))];
    assert_eq!(i18n.tm(de, "", source, &args), "1,234 Kopien von Seite 1 (0.50)");
    assert_eq!(i18n.tm(Lang::EN, "", source, &args), "Copy 1,234 of Seite 1 (0.50)");
}

// A context row beats the plain row of the same text; without one the plain row serves.
#[test]
fn a_context_row_beats_the_plain_row() {
    let de = rows(&["\tOpen {name}\t{name} öffnen", "menu\tOpen {name}\t{name} im Menü öffnen"]);
    let i18n = i18n(&[("de", &de, "")]);
    let de = lang(&i18n, "de");
    let args = [("name", Arg::Text("A"))];
    assert_eq!(i18n.tm(de, "menu", "Open {name}", &args), "A im Menü öffnen");
    assert_eq!(i18n.tm(de, "", "Open {name}", &args), "A öffnen");
    assert_eq!(i18n.tm(de, "toolbar", "Open {name}", &args), "A öffnen");
}

// In an `@msg` row the doubled braces are single ones, as in a message template.
#[test]
fn msg_rows_read_doubled_braces_as_single_ones() {
    let de = rows(&["@msg\tCould not parse {{{what}}}\tKonnte {{{what}}} nicht lesen"]);
    let i18n = i18n(&[("de", &de, "")]);
    let de = lang(&i18n, "de");
    assert_eq!(i18n.tm(de, "@msg", "Could not parse {{{what}}}", &[("what", Arg::Text("x"))]), "Konnte {x} nicht lesen");
}

// A person's text is cut at 1,024 characters and the whole message at 4,096, each ending with `…`.
#[test]
fn a_persons_text_and_a_message_are_cut() {
    let i18n = i18n(&[]);
    let long = "ä".repeat(5000);
    let shown = i18n.tm(Lang::EN, "", "Open {name}", &[("name", Arg::Name(&long))]);
    let name = shown.strip_prefix("Open ").unwrap();
    assert_eq!(name.chars().count(), 1024);
    assert!(name.ends_with('…'));
    let exactly = "ä".repeat(1024);
    let shown = i18n.tm(Lang::EN, "", "Open {name}", &[("name", Arg::Name(&exactly))]);
    assert_eq!(shown.chars().count(), "Open ".chars().count() + 1024, "a name at the cap is not cut");
    // A message of many names is cut as a whole.
    let template = "{a}{b}{c}{d}{e}{f}";
    let names: Vec<(&str, Arg)> = ["a", "b", "c", "d", "e", "f"].into_iter().map(|k| (k, Arg::Name(&long))).collect();
    let shown = i18n.tm(Lang::EN, "", template, &names);
    assert_eq!(shown.chars().count(), 4096);
    assert!(shown.ends_with('…'));
}

// A name with a right-to-left letter in an English sentence is isolated; one with an override cannot reorder what follows.
#[test]
fn a_persons_text_is_isolated_when_it_could_reorder_the_sentence() {
    let i18n = i18n(&[]);
    let shown = i18n.tm(Lang::EN, "", "Open {name} now", &[("name", Arg::Name("مستند"))]);
    assert_eq!(shown, "Open \u{2068}مستند\u{2069} now");
    let shown = i18n.tm(Lang::EN, "", "Open {name} now", &[("name", Arg::Name("a\u{202e}b"))]);
    assert!(shown.contains('\u{2068}') && shown.contains('\u{2069}'), "{shown:?}");
    assert_eq!(i18n.tm(Lang::EN, "", "Open {name} now", &[("name", Arg::Name("plain"))]), "Open plain now");
}

// A selector message is never shown as its source: a plain lookup of the same text shows the English.
#[test]
fn a_selector_row_is_never_returned_by_the_plain_lookups() {
    let de = german();
    let i18n = i18n(&[("de", &de, "")]);
    let de = lang(&i18n, "de");
    assert_eq!(i18n.t(de, DELETE), DELETE);
    assert_eq!(i18n.tc(de, "", DELETE), DELETE);
    assert_eq!(i18n.get(de, "", DELETE), None);
}

// A row of the app replaces the shared row of the same key, whichever kind each is.
#[test]
fn an_app_row_replaces_the_shared_row_of_either_kind() {
    let shared_plain = rows(&["\tOpen {name :number}\t{name} geöffnet"]);
    let app_message = rows(&["\tOpen {name :number}\t.input {$name :number} .match $name one {{{$name} Datei}} * {{{$name} Dateien}}"]);
    let i18n = i18n(&[("de", &shared_plain, &app_message)]);
    let de = lang(&i18n, "de");
    assert_eq!(i18n.tm(de, "", "Open {name :number}", &[("name", Arg::Count(1))]), "1 Datei");
    let shared_message = app_message;
    let app_plain = shared_plain;
    let i18n = self::i18n(&[("de", &shared_message, &app_plain)]);
    assert_eq!(i18n.tm(lang(&i18n, "de"), "", "Open {name :number}", &[("name", Arg::Count(1))]), "1 geöffnet");
    assert_eq!(i18n.t(lang(&i18n, "de"), "Open {name :number}"), "{name} geöffnet");
}

// The genders and forms come from both catalogs of the locale, whichever holds the `@grammar` rows and whichever comes first.
#[test]
fn the_grammar_of_either_catalog_serves_the_rows_of_the_other() {
    let shared = rows(&["@grammar\tgenders\tmasculine feminine neuter"]);
    let app = german().replace("@grammar\tgenders\tmasculine feminine neuter\n", "");
    let (i18n, problems) = build(&[("de", &shared, &app)]);
    assert!(problems.is_empty(), "{problems:?}");
    assert!(i18n.check_all().is_empty(), "{:?}", i18n.check_all());
    assert_eq!(delete(&i18n, "de", "page"), "Diese Seite löschen?");
    // The same with the rows of the first catalog placed after the nouns of the second.
    let (i18n, _) = build(&[("de", &app, &shared)]);
    assert!(i18n.check_all().is_empty(), "{:?}", i18n.check_all());
}

// A row that breaks a rule is skipped and reported, and the English shows.
#[test]
fn a_row_that_breaks_a_rule_is_skipped_reported_and_shown_in_english() {
    let without_many = rows(&[
        "@plural\t{n} shape selected|{n} shapes selected\t.input {$n :number} .match $n one {{{$n} kształt}} few {{{$n} kształty}} * {{{$n} kształtów}}",
        "\tOpen {n :number}\t.input {$n :number} .match $n one {{{$n} a}} {{",
        "@noun\tpage\tfeminine: strona",
        "@grammar\tcolours\tred green",
    ]);
    let (i18n, _) = build(&[("pl", &without_many, "")]);
    let problems = i18n.check_all();
    assert_eq!(problems.len(), 4, "{problems:?}");
    assert!(problems.iter().any(|p| p.starts_with("pl/common: line 1:") && p.contains("no variant for `many`")), "{problems:?}");
    assert!(problems.iter().any(|p| p.starts_with("pl/common: line 2:")), "{problems:?}");
    assert!(
        problems
            .iter()
            .any(|p| p.starts_with("pl/common: line 3:") && p.contains("gender \"feminine\" is not one of the language's @grammar genders")),
        "{problems:?}"
    );
    assert!(problems.iter().any(|p| p.starts_with("pl/common: line 4:") && p.contains("unknown key \"colours\"")), "{problems:?}");
    let pl = lang(&i18n, "pl");
    assert_eq!(i18n.trn(pl, 5, "{n} shape selected", "{n} shapes selected"), "5 shapes selected");
}

// A message of the pseudo locales is the English filled in, inside the marks.
#[test]
fn the_pseudo_locales_mark_a_typed_message() {
    let de = german();
    let i18n = i18n(&[("de", &de, "")]);
    let shown = i18n.tm(Lang::PSEUDO, "", DELETE, &[("item", Arg::Noun("page"))]);
    assert_eq!(shown, "⟦Delete this page?⟧");
    let long = i18n.tm(Lang::PSEUDO_LONG, "", DELETE, &[("item", Arg::Noun("page"))]);
    assert!(long.starts_with('⟦') && long.ends_with('⟧'), "{long}");
    assert!(long.chars().count() >= "Delete this page?".chars().count() * 14 / 10 + 2, "{long}");
    assert!(!long.contains("Delete"), "the letters are accented: {long}");
}

#[cfg(feature = "dates")]
mod dates {
    use craft_i18n::{CalendarDate, DateStyle, DateTime, Digits, Region};

    use super::*;

    // A date is written for the region of the process, in the style given.
    #[test]
    fn a_date_value_is_written_for_the_region() {
        let de = rows(&["\tChanged {when :date}\tGeändert {when}"]);
        let i18n = i18n(&[("de", &de, "")]);
        i18n.set_region(&Region::from_tag("de-DE"), Digits::Latin);
        let at = DateTime { date: CalendarDate::new(2026, 10, 31).unwrap(), hour: 14, minute: 5, second: 11 };
        let shown = i18n.tm(lang(&i18n, "de"), "", "Changed {when :date}", &[("when", Arg::Date(at, DateStyle::DATE))]);
        assert!(shown.starts_with("Geändert 31.") && shown.contains("2026"), "{shown}");
        i18n.set_region(&Region::from_tag("en-US"), Digits::Latin);
        let shown = i18n.tm(Lang::EN, "", "Changed {when :date}", &[("when", Arg::Date(at, DateStyle::DATE))]);
        assert!(shown.starts_with("Changed Oct 31, 2026"), "{shown}");
    }
}
