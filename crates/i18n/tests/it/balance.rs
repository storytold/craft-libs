//! A message is always balanced: whatever opens a bidi isolate, embedding or override inside it is closed before it ends, also
//! when the message is cut at its cap. An open isolate at the end would reorder the text that follows it in the window.

use craft_i18n::limits::{MAX_ARG_CHARS, MAX_OUTPUT_CHARS};
use craft_i18n::{Arg, Lang};

use crate::common::{i18n, lang, open_controls};

const ARABIC: &str = "ا";

fn names(letter: &str, count: usize) -> Vec<(&'static str, String)> {
    ["a", "b", "c", "d", "e", "f", "g", "h"].into_iter().take(count).map(|k| (k, letter.repeat(MAX_ARG_CHARS))).collect()
}

fn template(count: usize) -> String {
    ["{a}", "{b}", "{c}", "{d}", "{e}", "{f}", "{g}", "{h}"].into_iter().take(count).collect()
}

fn show(i18n: &craft_i18n::I18n, lang: Lang, count: usize, letter: &str) -> String {
    let values = names(letter, count);
    let args: Vec<(&str, Arg)> = values.iter().map(|(k, v)| (*k, Arg::Name(v))).collect();
    i18n.tm(lang, "", &template(count), &args)
}

// A cut that falls inside an isolated right-to-left value leaves no isolate open, and the whole stays within its cap.
#[test]
fn a_right_to_left_value_cut_by_the_output_cap_leaves_no_isolate_open() {
    let i18n = i18n(&[]);
    for count in 1..=8 {
        let shown = show(&i18n, Lang::EN, count, ARABIC);
        assert!(shown.chars().count() <= MAX_OUTPUT_CHARS, "{count}: {}", shown.chars().count());
        assert_eq!(open_controls(&shown), Vec::<char>::new(), "{count} values: {} characters", shown.chars().count());
        if count * MAX_ARG_CHARS > MAX_OUTPUT_CHARS {
            assert!(shown.contains('…'), "{count}: a cut message ends with the ellipsis inside its last isolate");
        }
    }
}

// The same for values that open embeddings and overrides: closed inside their isolate, and the isolate closed at the cut, within
// the cap even when the closers are many.
#[test]
fn values_that_open_embeddings_are_closed_at_the_cut_within_the_cap() {
    let i18n = i18n(&[]);
    for opener in ["\u{202B}", "\u{202E}", "\u{2067}", "\u{2066}\u{202D}"] {
        for count in [1, 4, 5, 8] {
            let shown = show(&i18n, Lang::EN, count, opener);
            assert!(shown.chars().count() <= MAX_OUTPUT_CHARS, "{opener:?} x {count}: {}", shown.chars().count());
            assert_eq!(open_controls(&shown), Vec::<char>::new(), "{opener:?} x {count}");
        }
    }
}

// Every length of one value around the cap: the cut falls at each place in the message, inside the isolate or beside it.
#[test]
fn a_cut_at_every_place_in_a_message_is_balanced() {
    let i18n = i18n(&[]);
    for length in (MAX_OUTPUT_CHARS - 12)..(MAX_OUTPUT_CHARS + 12) {
        let filler = "x".repeat(length - 1000);
        let value = ARABIC.repeat(900);
        let shown = i18n.tm(Lang::EN, "", "{pad}{v}{v}", &[("pad", Arg::Name(&filler[..filler.len().min(MAX_ARG_CHARS)])), ("v", Arg::Name(&value))]);
        let shown_long = i18n.tm(Lang::EN, "", &format!("{}{{v}}{{v}}", "y".repeat(length - 1900)), &[("v", Arg::Name(&value))]);
        for text in [shown, shown_long] {
            assert!(text.chars().count() <= MAX_OUTPUT_CHARS, "{length}: {}", text.chars().count());
            assert_eq!(open_controls(&text), Vec::<char>::new(), "{length}");
        }
    }
}

// A template with an embedding of its own that nothing closes is closed at the end of the message, so what follows the message in
// the window is not reordered either.
#[test]
fn a_stray_embedding_in_a_translation_is_closed_at_the_end() {
    let de = "\tOpen {name}\t\u{202E}{name} öffnen\n";
    let i18n = i18n(&[("de", de, "")]);
    let shown = i18n.tm(lang(&i18n, "de"), "", "Open {name}", &[("name", Arg::Text("x"))]);
    assert!(shown.starts_with('\u{202E}'), "{shown:?}");
    assert_eq!(open_controls(&shown), Vec::<char>::new(), "{shown:?}");
    // The English fallback, and the pseudo locales that wrap it.
    for lang in [Lang::EN, Lang::PSEUDO, Lang::PSEUDO_LONG] {
        let shown = i18n.tm(lang, "", "\u{202B}Open {name}", &[("name", Arg::Text("x"))]);
        assert_eq!(open_controls(&shown), Vec::<char>::new(), "{shown:?}");
    }
}

// A selector message's output and a count's are balanced and capped too.
#[test]
fn selector_messages_and_counts_are_balanced() {
    let ar = "@plural\t{n} files|{n} files\t.input {$n :number} .match $n one {{\u{202A}ملف}} * {{{$n} ملفات\u{202B}}}\n";
    let i18n = i18n(&[("ar", ar, "")]);
    for n in [0, 1, 2, 11, 100] {
        let shown = i18n.trn(lang(&i18n, "ar"), n, "{n} files", "{n} files");
        assert_eq!(open_controls(&shown), Vec::<char>::new(), "{n}: {shown:?}");
    }
}
