//! The caps of `craft_i18n::limits`, each with a counted test: a row, a file, a locale or a message one past its cap is skipped
//! and reported, one at the cap is used. The timed twins are `#[ignore]`d: `cargo test -- --ignored`.

use craft_i18n::limits::*;
use craft_i18n::{Arg, I18n};

use crate::common::{build, i18n, lang};

/// A catalog of `n` plain rows `k0 → v0`, `k1 → v1`…
fn many_rows(n: usize) -> String {
    (0..n).map(|i| format!("\tk{i}\tv{i}\n")).collect()
}

fn problems_of(i18n: &I18n) -> Vec<String> {
    i18n.check_all()
}

#[test]
fn a_catalog_over_2_mib_is_not_used_and_one_at_the_cap_is() {
    let row = "\tk\tv\n";
    let padding = |bytes: usize| format!("#{}\n", "x".repeat(bytes - 2));
    let at_cap = format!("{row}{}", padding(MAX_CATALOG_BYTES - row.len()));
    assert_eq!(at_cap.len(), MAX_CATALOG_BYTES);
    let (i18n, _) = build(&[("de", &at_cap, "")]);
    assert!(problems_of(&i18n).is_empty(), "{:?}", problems_of(&i18n));
    assert_eq!(i18n.t(lang(&i18n, "de"), "k"), "v");
    let over = format!("{at_cap}#");
    let (i18n, _) = build(&[("de", &over, "")]);
    let problems = problems_of(&i18n);
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(problems[0].contains(&format!("larger than {MAX_CATALOG_BYTES} bytes")), "{problems:?}");
    assert_eq!(i18n.t(lang(&i18n, "de"), "k"), "k", "the file is not used");
    // A byte-order mark is not counted as the file's own bytes.
    let (i18n, _) = build(&[("de", &format!("\u{feff}{at_cap}"), "")]);
    assert!(problems_of(&i18n).is_empty());
}

#[test]
fn rows_past_20_000_are_skipped_and_said_once() {
    let text = many_rows(MAX_ROWS + 1);
    let (i18n, _) = build(&[("de", &text, "")]);
    let de = lang(&i18n, "de");
    assert_eq!(i18n.t(de, "k0"), "v0");
    assert_eq!(i18n.t(de, &format!("k{}", MAX_ROWS - 1)), format!("v{}", MAX_ROWS - 1), "the 20,000th row is used");
    assert_eq!(i18n.t(de, &format!("k{MAX_ROWS}")), format!("k{MAX_ROWS}"), "the 20,001st is not");
    let problems = problems_of(&i18n);
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(
        problems[0].contains("more than 20000 rows; the rest are skipped") && problems[0].contains(&format!("line {}:", MAX_ROWS + 1)),
        "{problems:?}"
    );
    // Comments and blank lines are not rows.
    let spaced = format!("{}\n\n# note\n{}", many_rows(MAX_ROWS - 1), "\tlast\tzuletzt\n");
    let (i18n, _) = build(&[("de", &spaced, "")]);
    assert!(problems_of(&i18n).is_empty(), "{:?}", problems_of(&i18n));
    assert_eq!(i18n.t(lang(&i18n, "de"), "last"), "zuletzt");
}

#[test]
fn a_row_over_16_kib_is_skipped() {
    let row = |bytes: usize| format!("\tbig\t{}\n\tsmall\tklein\n", "ä".repeat(bytes));
    // `\tbig\t` is 6 bytes; the rest is two-byte letters, and 16,384 is even.
    let at_cap = row((MAX_ROW_BYTES - 6) / 2);
    let (i18n, _) = build(&[("de", &at_cap, "")]);
    assert!(problems_of(&i18n).is_empty(), "{:?}", problems_of(&i18n));
    assert!(i18n.t(lang(&i18n, "de"), "big").starts_with('ä'));
    let over = row((MAX_ROW_BYTES - 6) / 2 + 1);
    let (i18n, _) = build(&[("de", &over, "")]);
    let problems = problems_of(&i18n);
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(problems[0].starts_with("de/common: line 1: row longer than 16384 bytes"), "{problems:?}");
    assert_eq!(i18n.t(lang(&i18n, "de"), "big"), "big");
    assert_eq!(i18n.t(lang(&i18n, "de"), "small"), "klein", "the next row is read");
}

#[test]
fn a_selector_message_over_4_kib_is_skipped() {
    let message = |bytes: usize| format!(".input {{$n :number}} {{{{{}{{$n}}}}}}", "x".repeat(bytes));
    let source = "\tOpen {n :number}\t";
    let fixed = message(0).len();
    let (i18n, _) = build(&[("de", &format!("{source}{}\n", message(MAX_MESSAGE_BYTES - fixed)), "")]);
    assert!(problems_of(&i18n).is_empty(), "{:?}", problems_of(&i18n));
    let (i18n, _) = build(&[("de", &format!("{source}{}\n", message(MAX_MESSAGE_BYTES - fixed + 1)), "")]);
    let problems = problems_of(&i18n);
    assert!(problems.len() == 1 && problems[0].contains("at most 4096 bytes"), "{problems:?}");
}

#[test]
fn nouns_past_2_000_are_skipped_and_a_noun_with_17_forms_is_refused() {
    let grammar = "@grammar\tforms\tf1 f2 f3 f4 f5 f6 f7 f8 f9 f10 f11 f12 f13 f14 f15 f16\n";
    let nouns = |n: usize| (0..n).map(|i| format!("@noun\tn{i}\tword{i}\n")).collect::<String>();
    let (i18n, _) = build(&[("de", &format!("{grammar}{}", nouns(MAX_NOUNS)), "")]);
    assert!(problems_of(&i18n).is_empty(), "{:?}", problems_of(&i18n));
    let (i18n, _) = build(&[("de", &format!("{grammar}{}", nouns(MAX_NOUNS + 1)), "")]);
    let problems = problems_of(&i18n);
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(problems[0].contains("more than 2000 nouns; the rest are skipped"), "{problems:?}");
    // 16 forms are the cap.
    let forms = |n: usize| (1..=n).map(|i| format!("f{i}=w{i}")).collect::<Vec<_>>().join("; ");
    let (i18n, _) = build(&[("de", &format!("{grammar}@noun\tpage\tSeite; {}\n", forms(16)), "")]);
    assert!(problems_of(&i18n).is_empty(), "{:?}", problems_of(&i18n));
    let seventeen = grammar.replace("f16", "f16 f17");
    let (i18n, _) = build(&[("de", &format!("{seventeen}@noun\tpage\tSeite; {}\n", forms(17)), "")]);
    let problems = problems_of(&i18n);
    assert_eq!(problems.len(), 2, "{problems:?}");
    assert!(problems.iter().any(|p| p.contains("more than 16 names")) && problems.iter().any(|p| p.contains("more than 16 forms")), "{problems:?}");
}

#[test]
fn locales_past_64_are_ignored() {
    let codes: Vec<String> = ('a'..='c').flat_map(|a| ('a'..='z').map(move |b| format!("{a}{b}"))).take(MAX_LOCALES + 1).collect();
    let catalogs: Vec<(&str, &str, &str)> = codes.iter().map(|c| (c.as_str(), "\tk\tv\n", "")).collect();
    let (i18n, problems) = build(&catalogs);
    assert_eq!(problems.iter().filter(|p| p.contains("more than 64 locales; the rest are ignored")).count(), 1, "{problems:?}");
    assert!(i18n.from_code(&codes[MAX_LOCALES - 1]).is_some(), "the 64th is embedded");
    assert!(i18n.from_code(&codes[MAX_LOCALES]).is_none(), "the 65th is not");
}

#[test]
fn a_tag_over_35_bytes_and_languages_past_16_are_ignored() {
    let i18n = i18n(&[("de", "", ""), ("fr", "", "")]);
    let tag = |bytes: usize| format!("de-x-{}", "a".repeat(bytes - 5));
    assert!(i18n.match_tag(&tag(MAX_TAG_BYTES)).is_some(), "35 bytes is a tag");
    assert!(i18n.match_tag(&tag(MAX_TAG_BYTES + 1)).is_none(), "36 bytes is not");
    let mut preferred = vec!["xx"; MAX_PREFERRED - 1];
    preferred.push("fr");
    assert_eq!(i18n.match_tags(preferred.iter().copied()), lang(&i18n, "fr"), "the 16th is read");
    preferred.insert(0, "xx");
    assert_eq!(i18n.match_tags(preferred.iter().copied()), craft_i18n::Lang::EN, "the 17th is not");
}

#[test]
fn a_value_and_a_message_are_cut_to_their_caps() {
    let i18n = i18n(&[]);
    let name = "x".repeat(MAX_ARG_CHARS + 1);
    let shown = i18n.tm(craft_i18n::Lang::EN, "", "{a}", &[("a", Arg::Name(&name))]);
    assert_eq!(shown.chars().count(), MAX_ARG_CHARS);
    let sixty_four_names = "{a}".repeat(5);
    let shown = i18n.tm(craft_i18n::Lang::EN, "", &sixty_four_names, &[("a", Arg::Name(&"y".repeat(MAX_ARG_CHARS)))]);
    assert_eq!(shown.chars().count(), MAX_OUTPUT_CHARS);
    assert!(shown.ends_with('…'));
}

#[test]
fn decimals_of_a_hostile_value_are_bounded() {
    let i18n = i18n(&[]);
    let shown = i18n.tm(craft_i18n::Lang::EN, "", "{n :number}", &[("n", Arg::number(1.0 / 3.0, usize::MAX))]);
    assert!(shown.chars().count() < 40, "{shown}");
    let counted = i18n.trn_decimal(craft_i18n::Lang::EN, 1.0 / 3.0, usize::MAX, "{n} file", "{n} files");
    assert!(counted.chars().count() < 40, "{counted}");
    let shown = i18n.tm(craft_i18n::Lang::EN, "", "{n :number}", &[("n", Arg::number(f64::NAN, 2))]);
    assert!(!shown.is_empty());
    let shown = i18n.tm(craft_i18n::Lang::EN, "", "{n :number}", &[("n", Arg::number(f64::INFINITY, 2))]);
    assert!(!shown.is_empty());
}

// ---------------------------------------------------------------------------------------------------------------------
// The timed twins: a hostile input in a bounded time. Run with `cargo test -p craft-i18n -- --ignored`.

fn within(seconds: u64, run: impl FnOnce()) {
    let start = std::time::Instant::now();
    run();
    assert!(start.elapsed().as_secs() < seconds, "took {:?}", start.elapsed());
}

#[test]
#[ignore = "timed"]
fn timed_a_million_short_rows_stop_at_the_cap() {
    // A 2 MiB file of 1,000,000 short rows: 20,000 are read and the rest skipped, in linear time.
    let text: String = (0..1_000_000).map(|i| format!("\t{i}\ta\n")).collect();
    assert!(text.len() > MAX_CATALOG_BYTES, "{}", text.len());
    within(5, || {
        let (i18n, _) = build(&[("de", &text[..MAX_CATALOG_BYTES], "")]);
        let _ = i18n.t(lang(&i18n, "de"), "0");
        assert_eq!(i18n.check_all().len(), 1);
    });
}

#[test]
#[ignore = "timed"]
fn timed_one_two_mib_row_and_a_flood_of_unknown_contexts() {
    let one_row = format!("\tbig\t{}\n", "x".repeat(MAX_CATALOG_BYTES - 8));
    let flood: String = (0..MAX_ROWS).map(|i| format!("@x{i}\ta\tb\n")).collect();
    within(5, || {
        for text in [&one_row, &flood] {
            let (i18n, _) = build(&[("de", text, "")]);
            let _ = i18n.t(lang(&i18n, "de"), "big");
            assert!(!i18n.check_all().is_empty());
        }
    });
}

#[test]
#[ignore = "timed"]
fn timed_64_variants_over_three_selectors_and_ten_thousand_braces() {
    let variants: String = (0..63).map(|i| format!("{} {} * {{{{x{{$a}}{{$b}}{{$c}}}}}} ", i % 2, i)).collect();
    let message =
        format!(".input {{$a :number}} .input {{$b :number}} .input {{$c :number}} .match $a $b $c {variants}* * * {{{{y{{$a}}{{$b}}{{$c}}}}}}");
    let braces = "{".repeat(10_000);
    within(5, || {
        let _ = craft_i18n::select::Message::parse(&message);
        let _ = craft_i18n::select::Message::parse(&braces);
        let _ = craft_i18n::select::Message::parse(&format!(".input {{$n :number}} {{{{{braces}}}}}"));
    });
}

#[test]
#[ignore = "timed"]
fn timed_a_noun_with_ten_thousand_forms() {
    let forms = (0..10_000).map(|i| format!("f{i}=w")).collect::<Vec<_>>().join("; ");
    within(5, || {
        let (i18n, _) = build(&[("de", &format!("@noun\tpage\tSeite; {forms}\n"), "")]);
        assert_eq!(i18n.check_all().len(), 1);
    });
}
