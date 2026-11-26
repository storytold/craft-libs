//! The selector-message subset: its grammar, each refusal, and the rules a translation must meet (`check_message`).

use craft_i18n::noun::Grammar;
use craft_i18n::select::{
    Body, Category, Function, Key, Locale, LocaleRules, Message, Part, RowKind, Select, Type, check_message, source_placeholders,
};

/// The Arabic plural row of the design: six categories, with the words for 0, 1 and 2 saying the number.
const ARABIC: &str = "{n} shape selected|{n} shapes selected";
const ARABIC_MESSAGE: &str = ".input {$n :number} .match $n zero {{لا أشكال محددة}} one {{شكل واحد محدد}} two {{شكلان محددان}} few {{{$n} أشكال محددة}} many {{{$n} شكلًا محددًا}} * {{{$n} شكل محدد}}";

fn parse(text: &str) -> Message {
    Message::parse(text).unwrap_or_else(|e| panic!("{text}: {e}"))
}

fn refused(text: &str) -> String {
    match Message::parse(text) {
        Ok(m) => panic!("{text} was accepted: {m:?}"),
        Err(e) => e,
    }
}

fn grammar(genders: &str, forms: &str) -> Grammar {
    let mut grammar = Grammar::default();
    if !genders.is_empty() {
        grammar.set("genders", genders).unwrap();
    }
    if !forms.is_empty() {
        grammar.set("forms", forms).unwrap();
    }
    grammar
}

/// `message` as a translation in `code` with `grammar`: the rules' verdict.
fn check(kind: RowKind, source: &str, message: &str, code: &str, grammar: &Grammar) -> Result<Message, String> {
    let rules = LocaleRules::for_locale(code);
    check_message(kind, source, message, &Locale { rules: rules.as_ref(), grammar })
}

fn plural_ok(code: &str, message: &str) -> Result<Message, String> {
    check(RowKind::Plural, ARABIC, message, code, &Grammar::default())
}

// ---------------------------------------------------------------------------------------------------------------------
// The grammar

#[test]
fn every_construct_of_the_subset_parses() {
    // Declarations of each function and option, the three keys, and a local that derives a noun's form.
    let m = parse(
        ".input {$n :number select=ordinal} .input {$k :integer} .input {$item :noun} .local $d = {$item :noun form=genitive} \
         .match $n $item one feminine {{{$n} {$d}}} * * {{{$k}}}",
    );
    assert_eq!(m.declarations.len(), 4);
    let first = &m.declarations[0];
    assert_eq!((first.name.as_str(), first.function, first.select, first.from.as_deref()), ("n", Function::Number, Select::Ordinal, None));
    assert_eq!(m.declarations[1].function, Function::Integer);
    assert_eq!(m.declarations[2].function, Function::Noun);
    let local = &m.declarations[3];
    assert_eq!((local.name.as_str(), local.from.as_deref(), local.form.as_deref()), ("d", Some("item"), Some("genitive")));
    let Body::Match { selectors, variants } = &m.body else { panic!("a match") };
    assert_eq!(selectors, &["n", "item"]);
    assert_eq!(variants[0].keys, [Key::Name("one".into()), Key::Name("feminine".into())]);
    assert_eq!(variants[1].keys, [Key::Catchall, Key::Catchall]);
}

#[test]
fn a_pattern_has_text_variables_forms_and_four_escapes() {
    let m = parse(r".input {$item :noun} {{Usunąć \{{$item :noun form=accusative}\} \| \\ ?}}");
    let Body::Pattern(parts) = &m.body else { panic!("a bare pattern") };
    assert_eq!(
        parts,
        &[Part::Text("Usunąć {".into()), Part::Var { name: "item".into(), form: Some("accusative".into()) }, Part::Text("} | \\ ?".into()),]
    );
}

#[test]
fn white_space_between_tokens_is_free_and_a_pattern_keeps_its_own() {
    let m = parse("\n  .input {  $n   :number }\n.match   $n\n  one   {{ a  b }}\n  *\t{{c}}\n");
    let Body::Match { variants, .. } = &m.body else { panic!("a match") };
    assert_eq!(variants[0].pattern, [Part::Text(" a  b ".into())]);
    assert_eq!(variants[1].pattern, [Part::Text("c".into())]);
}

#[test]
fn exact_numbers_are_keys_beside_categories() {
    let m = parse(".input {$n :number} .match $n 0 {{none}} 1 {{one thing}} one {{{$n}}} * {{{$n}}}");
    let Body::Match { variants, .. } = &m.body else { panic!("a match") };
    assert_eq!(variants[0].keys, [Key::Number(0)]);
    assert_eq!(variants[1].keys, [Key::Number(1)]);
}

#[test]
fn only_a_message_that_begins_with_a_declaration_or_match_is_a_selector_message() {
    for text in [".input {$n :number} {{x}}", "  .match $n * {{x}}", ".local $a = {$b :noun form=x} {{y}}"] {
        assert!(Message::is_complex(text), "{text}");
    }
    for text in ["{n} files|{n} files", "{{x}}", "a .input", ""] {
        assert!(!Message::is_complex(text), "{text}");
    }
}

#[test]
fn each_refusal_says_what_it_refuses() {
    for (text, words) in [
        (".input {$n :number} {{{#b}x{/b}}}", "markup is not supported"),
        (".input {$n :number} {{{#b/}}}", "markup is not supported"),
        (".input {$n :string} {{x}}", "unknown function :string"),
        (".input {$n :date} {{x}}", "unknown function :date"),
        (".input {$n :number} {{{$n :integer}}}", "takes only `:noun form=<name>`"),
        (".input {$n :number} {{{|x|}}}", "a placeholder in a pattern is a variable"),
        (".input {$n :number} {{{:number}}}", "a placeholder in a pattern is a variable"),
        (".input {$n :number minimumFractionDigits=2} {{x}}", "unknown option minimumFractionDigits"),
        (".input {$n :number select=exact} {{x}}", "select=exact is not supported"),
        (".input {$n :number form=genitive} {{x}}", "form applies to a noun"),
        (".input {$n :noun select=ordinal} {{x}}", "select applies to a number"),
        (".input {$n} {{x}}", "a declaration applies a function"),
        (".input {$a :number} .input {$b :number} .input {$c :number} .input {$d :number} .match $a $b $c $d * * * * {{x}}", "more than 3 selectors"),
        (".input {$n :number} .match $n one {{x}}", "catch-all"),
        (".input {$n :number} .match $n {{x}}", "a variant has 0 keys; the `.match` has 1 selectors"),
        (".input {$n :number} .match $n one * {{x}}", "a variant has a key for each selector"),
        (".input {$n :number} .match $n", "`.match` needs variants"),
        (".input {$n :number} .match * {{x}}", "`.match` needs a selector variable"),
        (".input {$n :number} .match $n $n * * {{x}}", "listed twice"),
        (".input {$n :number} {{x}} trailing", "unexpected text after the message"),
        (".input {$n :number} {{x", "not closed"),
        (".input {$n :number} {{a } b}}", "a `}` in a pattern is written"),
        (r".input {$n :number} {{\n}}", "an escape is"),
        (".input {$n :number} x", "expected a quoted pattern"),
        (".match $n 99999999999999999999 {{x}} * {{x}}", "a whole number key too large"),
    ] {
        let error = Message::parse(text).err().unwrap_or_else(|| panic!("{text} was accepted"));
        assert!(error.contains(words), "{text}: {error}");
    }
}

#[test]
fn a_65th_variant_a_9th_declaration_and_a_65th_part_are_refused_and_the_cap_itself_is_not() {
    let declarations = |n: usize| (0..n).map(|i| format!(".input {{$v{i} :number}} ")).collect::<String>();
    let variants = |n: usize| (0..n - 1).map(|i| format!("{i} {{{{x}}}} ")).collect::<String>() + "* {{x}}";
    let parts = |n: usize| (0..n).map(|i| if i % 2 == 0 { "-" } else { "{$n}" }).collect::<String>();
    assert!(Message::parse(&format!(".input {{$n :number}} .match $n {}", variants(64))).is_ok());
    assert!(refused(&format!(".input {{$n :number}} .match $n {}", variants(65))).contains("more than 64 variants"));
    assert!(Message::parse(&format!("{}{{{{x}}}}", declarations(8))).is_ok());
    assert!(refused(&format!("{}{{{{x}}}}", declarations(9))).contains("more than 8 declarations"));
    // 64 parts: 32 texts and 32 variables, alternating.
    assert!(Message::parse(&format!(".input {{$n :number}} {{{{{}}}}}", parts(64))).is_ok());
    assert!(refused(&format!(".input {{$n :number}} {{{{{}}}}}", parts(65))).contains("more than 64 parts"));
}

#[test]
fn a_message_is_at_most_4_kib() {
    let pattern = |bytes: usize| format!(".input {{$n :number}} {{{{{}}}}}", "x".repeat(bytes));
    let fixed = pattern(0).len();
    assert!(Message::parse(&pattern(4096 - fixed)).is_ok());
    assert!(refused(&pattern(4097 - fixed)).contains("at most 4096 bytes"));
}

#[test]
fn a_hostile_message_is_refused_without_a_panic() {
    for text in [
        "{{".repeat(5000),
        ".match ".repeat(500),
        ".input ".repeat(500),
        "\u{0}.input {$n :number} {{x}}".to_owned(),
        ".input {$n :number} {{\u{202e}}}".to_owned(),
        format!(".input {{$n :number}} .match $n {}* {{{{x}}}}", "one ".repeat(2000)),
        ".input {$\u{1F600} :number} {{x}}".to_owned(),
        ".local".to_owned(),
        ".input {".to_owned(),
        ".match $".to_owned(),
    ] {
        // A refusal or an acceptance, never a panic or a hang.
        let _ = Message::parse(&text);
    }
}

#[test]
fn source_placeholders_are_typed_by_their_annotation() {
    let found = source_placeholders("Delete {n :number} of {item :noun} on {when :date} for {who} {k :integer} {who}").unwrap();
    assert_eq!(
        found,
        [
            ("n".to_owned(), Type::Number),
            ("item".to_owned(), Type::Noun),
            ("when".to_owned(), Type::Date),
            ("who".to_owned(), Type::Text),
            ("k".to_owned(), Type::Integer),
        ]
    );
    assert!(source_placeholders("{x :colour}").unwrap_err().contains("unknown placeholder type"));
    assert!(source_placeholders("{x :number :noun}").is_err());
    assert!(source_placeholders("{x :number} {x :noun}").unwrap_err().contains("typed two ways"));
}

// ---------------------------------------------------------------------------------------------------------------------
// What CLDR says, from the runtime's data

#[test]
fn the_rules_of_a_locale_come_from_the_data() {
    let rules = |code: &str| LocaleRules::for_locale(code).unwrap_or_else(|| panic!("{code}"));
    let (en, pl, ar, ja, fr, de) = (rules("en"), rules("pl"), rules("ar"), rules("ja"), rules("fr"), rules("de"));
    assert_eq!(en.cardinal, [Category::One, Category::Other]);
    assert_eq!(en.ordinal, [Category::One, Category::Two, Category::Few, Category::Other]);
    assert_eq!(en.cardinal_single, [Category::One], "English one holds 1 only");
    assert_eq!(pl.cardinal, [Category::One, Category::Few, Category::Many], "Polish other is reached only by fractions");
    assert_eq!(pl.cardinal_single, [Category::One]);
    assert_eq!(ar.cardinal, Category::ALL);
    assert_eq!(ar.cardinal_single, [Category::Zero, Category::One, Category::Two]);
    assert_eq!(ja.cardinal, [Category::Other]);
    assert_eq!(fr.cardinal, [Category::One, Category::Other], "French many is reached by millions only");
    assert_eq!(fr.ordinal, [Category::One, Category::Other]);
    assert_eq!(fr.ordinal_single, [Category::One]);
    for rules in [&de, &pl, &ar, &ja] {
        assert_eq!(rules.ordinal, [Category::Other], "German, Polish, Arabic and Japanese have one ordinal category");
    }
    assert!(LocaleRules::for_locale("xx").is_none());
    // The whole numbers of each category, from the same data.
    assert_eq!(en.whole_numbers(Select::Plural, Category::One), [1]);
    assert_eq!(de.whole_numbers(Select::Plural, Category::One), [1]);
    assert_eq!(ar.whole_numbers(Select::Plural, Category::Zero), [0]);
    assert_eq!(ar.whole_numbers(Select::Plural, Category::Two), [2]);
    assert_eq!(pl.whole_numbers(Select::Plural, Category::One), [1]);
    assert_eq!(&pl.whole_numbers(Select::Plural, Category::Few)[..7], [2, 3, 4, 22, 23, 24, 32]);
    assert!(pl.whole_numbers(Select::Plural, Category::Other).is_empty(), "Polish other is reached only by fractions");
    assert_eq!(fr.whole_numbers(Select::Ordinal, Category::One), [1]);
    assert_eq!(en.whole_numbers(Select::Ordinal, Category::Two)[..3], [2, 22, 32]);
    assert_eq!(ja.whole_numbers(Select::Plural, Category::Other).len(), 1001);
    let total: usize = Category::ALL.iter().map(|c| ar.whole_numbers(Select::Plural, *c).len()).sum();
    assert_eq!(total, 1001, "every whole number from 0 to 1,000 is in one category");
    assert_eq!(Category::from_key("few"), Some(Category::Few));
    assert_eq!(Category::from_key("many").map(Category::key), Some("many"));
    assert_eq!(Category::from_key("1"), None);
}

// ---------------------------------------------------------------------------------------------------------------------
// The rules of a translation

#[test]
fn arabic_with_its_six_categories_passes_and_its_words_for_one_may_leave_out_the_number() {
    plural_ok("ar", ARABIC_MESSAGE).unwrap();
}

#[test]
fn polish_without_many_is_an_error_and_not_a_silent_wrong_form() {
    let with_many = ".input {$n :number} .match $n one {{{$n} kształt}} few {{{$n} kształty}} many {{{$n} kształtów}} * {{{$n} kształtu}}";
    plural_ok("pl", with_many).unwrap();
    let without = ".input {$n :number} .match $n one {{{$n} kształt}} few {{{$n} kształty}} * {{{$n} kształtów}}";
    let error = plural_ok("pl", without).unwrap_err();
    assert!(error.contains("no variant for `many`"), "{error}");
}

#[test]
fn a_key_must_be_a_category_the_language_reaches() {
    let error = plural_ok("de", ".input {$n :number} .match $n one {{{$n}}} many {{{$n}}} * {{{$n}}}").unwrap_err();
    assert!(error.contains("key \"many\" is not a plural category the language reaches"), "{error}");
    let error = plural_ok("ja", ".input {$n :number} .match $n one {{{$n}}} * {{{$n}}}").unwrap_err();
    assert!(error.contains("\"one\""), "{error}");
}

// `other` is never a key: `*` is the `other` category.
#[test]
fn other_is_never_written_as_a_key() {
    let error = plural_ok("en", ".input {$n :number} .match $n one {{One}} other {{{$n}}} * {{{$n}}}").unwrap_err();
    assert!(error.contains("`other` is never written as a key"), "{error}");
}

// A category needs a variant of its own, or else an exact variant for each of its whole numbers: German and English `one` is only 1.
#[test]
fn exact_variants_for_every_number_of_a_category_stand_for_it() {
    for code in ["de", "en"] {
        plural_ok(code, ".input {$n :number} .match $n 1 {{Eine Datei}} * {{{$n} Dateien}}").unwrap();
    }
    // Polish `few` has 3 numbers below 5 and more above: some of them is not all of them.
    let polish = ".input {$n :number} .match $n one {{{$n} a}} 2 {{{$n} b}} 3 {{{$n} c}} 4 {{{$n} d}} many {{{$n} e}} * {{{$n} f}}";
    let error = plural_ok("pl", polish).unwrap_err();
    assert!(error.contains("no variant for `few`") && error.contains("one for each of its whole numbers"), "{error}");
    // Arabic `zero`, `one` and `two` each hold one number, `few` and `many` many.
    let arabic = ".input {$n :number} .match $n 0 {{a}} 1 {{b}} 2 {{c}} few {{{$n} d}} many {{{$n} e}} * {{{$n} f}}";
    plural_ok("ar", arabic).unwrap();
}

#[test]
fn every_placeholder_of_the_source_is_in_every_variant_but_a_single_valued_ones_number() {
    // English `one` holds 1 only, so it may say "one shape"; `other` may not leave out the number.
    let english = |m: &str| plural_ok("en", m);
    english(".input {$n :number} .match $n one {{One shape}} * {{{$n} shapes}}").unwrap();
    let error = english(".input {$n :number} .match $n one {{{$n} shape}} * {{Shapes}}").unwrap_err();
    assert!(error.contains("leaves out {$n}"), "{error}");
    // An exact number may leave it out too.
    english(".input {$n :number} .match $n 0 {{No shapes}} one {{{$n} shape}} * {{{$n} shapes}}").unwrap();
    // French `one` holds 0 and 1: it must keep the number.
    let error = plural_ok("fr", ".input {$n :number} .match $n one {{Une forme}} * {{{$n} formes}}").unwrap_err();
    assert!(error.contains("leaves out {$n}"), "{error}");
}

#[test]
fn a_catch_all_alone_is_enough_where_a_language_has_one_category() {
    plural_ok("ja", ".input {$n :number} .match $n * {{{$n} 個}}").unwrap();
    plural_ok("ja", ".input {$n :number} {{{$n} 個}}").unwrap();
}

#[test]
fn a_selector_must_be_declared_with_a_function_that_fits_the_source() {
    let g = Grammar::default();
    let source = "Delete {item :noun} {name}";
    let kind = RowKind::Plain;
    let error = check(kind, source, ".input {$name :number} .match $name * {{{$name} {$item}}}", "en", &g).unwrap_err();
    assert!(error.contains("$name is text in the source text; the function does not fit it"), "{error}");
    let error = check(kind, source, ".input {$item :number} .match $item * {{{$item} {$name}}}", "en", &g).unwrap_err();
    assert!(error.contains("$item is a noun in the source text"), "{error}");
    let error = check(kind, source, ".match $item * {{{$item} {$name}}}", "en", &g).unwrap_err();
    assert!(error.contains("selector $item is not declared"), "{error}");
    let error = check(kind, source, ".input {$other :number} {{{$item} {$name}}}", "en", &g).unwrap_err();
    assert!(error.contains("$other is not a placeholder of the source text"), "{error}");
    let error = check(kind, source, ".input {$item :noun} {{{$item} {$name} {$nope}}}", "en", &g).unwrap_err();
    assert!(error.contains("$nope is neither a placeholder of the source text nor declared"), "{error}");
}

#[test]
fn gender_keys_are_the_languages_declared_genders() {
    let german = grammar("masculine feminine neuter", "");
    let source = "Delete this {item :noun}?";
    let message =
        ".input {$item :noun} .match $item feminine {{Diese {$item} löschen?}} masculine {{Diesen {$item} löschen?}} * {{Dieses {$item} löschen?}}";
    check(RowKind::Plain, source, message, "de", &german).unwrap();
    // A language that declares none cannot select by gender.
    let error = check(RowKind::Plain, source, message, "ja", &Grammar::default()).unwrap_err();
    assert!(error.contains("key \"feminine\" is not a gender the language declares"), "{error}");
    let error = check(RowKind::Plain, source, ".input {$item :noun} .match $item 1 {{x {$item}}} * {{y {$item}}}", "de", &german).unwrap_err();
    assert!(error.contains("a noun selects on genders, not numbers"), "{error}");
}

#[test]
fn a_form_must_be_declared_and_belongs_to_a_noun() {
    let polish = grammar("masculine feminine neuter", "accusative genitive");
    let source = "Delete this {item :noun} {n :number}";
    let ok = ".input {$item :noun} {{Usunąć {$item :noun form=accusative} {$n}}}";
    check(RowKind::Plain, source, ok, "pl", &polish).unwrap();
    let error = check(RowKind::Plain, source, &ok.replace("accusative", "locative"), "pl", &polish).unwrap_err();
    assert!(error.contains("form=locative is not declared"), "{error}");
    check(
        RowKind::Plain,
        source,
        ".input {$n :number} .input {$item :noun} .local $d = {$item :noun form=genitive} {{{$d} {$n} {$item}}}",
        "pl",
        &polish,
    )
    .unwrap();
    let error =
        check(RowKind::Plain, source, ".input {$n :number} .input {$item :noun} {{{$n :noun form=accusative} {$item}}}", "pl", &polish).unwrap_err();
    assert!(error.contains("not a noun"), "{error}");
}

#[test]
fn a_local_counts_as_a_use_of_the_noun_it_derives_from() {
    let polish = grammar("masculine feminine neuter", "accusative");
    let message = ".input {$item :noun} .local $a = {$item :noun form=accusative} {{Usunąć {$a}}}";
    check(RowKind::Plain, "Delete {item :noun}", message, "pl", &polish).unwrap();
}

#[test]
fn counts_select_cardinal_in_plural_rows_and_ordinal_in_ordinal_rows() {
    let source = "Move to the {n}st place|Move to the {n}nd place|Move to the {n}rd place|Move to the {n}th place";
    let french = ".input {$n :number select=ordinal} .match $n one {{Déplacer à la {$n}re place}} * {{Déplacer à la {$n}e place}}";
    check(RowKind::Ordinal, source, french, "fr", &Grammar::default()).unwrap();
    let error = check(RowKind::Ordinal, source, &french.replace(" select=ordinal", ""), "fr", &Grammar::default()).unwrap_err();
    assert!(error.contains("select=ordinal"), "{error}");
    let error = plural_ok("fr", ".input {$n :number select=ordinal} .match $n one {{{$n}}} * {{{$n}}}").unwrap_err();
    assert!(error.contains("cardinal"), "{error}");
}

#[test]
fn a_variant_agrees_with_the_source_on_a_trailing_ellipsis() {
    let g = Grammar::default();
    check(RowKind::Plain, "Save {n :number} as…", ".input {$n :number} {{Speichern {$n} unter…}}", "de", &g).unwrap();
    let error = check(RowKind::Plain, "Save {n :number} as…", ".input {$n :number} {{Speichern {$n} unter}}", "de", &g).unwrap_err();
    assert!(error.contains("trailing `…`"), "{error}");
    let error = check(RowKind::Plain, "Save {n :number} as", ".input {$n :number} {{Speichern {$n} unter…}}", "de", &g).unwrap_err();
    assert!(error.contains("trailing `…`"), "{error}");
}

#[test]
fn a_translation_holds_no_bidi_embedding_override_or_isolate_and_the_marks_are_free() {
    let g = Grammar::default();
    let message = |c: char| format!(".input {{$n :number}} {{{{{c}{{$n}}}}}}");
    for c in ['\u{202A}', '\u{202B}', '\u{202C}', '\u{202D}', '\u{202E}', '\u{2066}', '\u{2067}', '\u{2068}', '\u{2069}'] {
        for code in ["ar", "de"] {
            let error = check(RowKind::Plain, "{n :number}", &message(c), code, &g).unwrap_err();
            assert!(error.contains("bidi embedding, override or isolate"), "U+{:04X}: {error}", c as u32);
        }
    }
    // The marks LRM and RLM, in any language: the format's rule names only embeddings, overrides and isolates.
    for c in ['\u{200E}', '\u{200F}'] {
        for code in ["ar", "de"] {
            check(RowKind::Plain, "{n :number}", &message(c), code, &g).unwrap();
        }
    }
}

#[test]
fn a_language_without_plural_rules_selects_only_by_exact_numbers_and_the_catch_all() {
    let g = Grammar::default();
    let source = "{n} files|{n} files";
    check(RowKind::Plural, source, ".input {$n :number} .match $n 0 {{none}} * {{{$n}}}", "xx", &g).unwrap();
    let error = check(RowKind::Plural, source, ".input {$n :number} .match $n one {{{$n}}} * {{{$n}}}", "xx", &g).unwrap_err();
    assert!(error.contains("\"one\""), "{error}");
}

#[test]
fn rules_are_checked_for_a_message_with_two_selectors() {
    let german = grammar("masculine feminine neuter", "");
    let source = "{n :number} {item :noun} deleted";
    let message = ".input {$n :number} .input {$item :noun} .match $n $item one feminine {{Eine {$item} gelöscht ({$n})}} \
                   one * {{Ein {$item} gelöscht ({$n})}} * * {{{$n} {$item} gelöscht}}";
    check(RowKind::Plain, source, message, "de", &german).unwrap();
}
