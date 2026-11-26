//! Hostile catalogs and messages: damaged copies of real ones, and every truncation, through parsing, the rules, and every
//! lookup. Each case is a result, never a panic, a hang (the hammer's limit) or an output past its cap.

use craft_i18n::limits::MAX_OUTPUT_CHARS;
use craft_i18n::noun::Grammar;
use craft_i18n::select::{Locale, LocaleRules, Message, RowKind, check_message};
use craft_i18n::{Arg, I18n, Lang};
use craft_testkit::hostile::{HammerConfig, hammer};

use crate::common::open_controls;

const CODES: [&str; 4] = ["de", "ar", "pl", "ja"];

/// Rows of every kind this crate reads, each in a form that passes.
fn seed_catalog() -> String {
    let key = "Move to the {n}st place|Move to the {n}nd place|Move to the {n}rd place|Move to the {n}th place";
    format!(
        "@grammar\tgenders\tmasculine feminine neuter\n@grammar\tforms\taccusative genitive\n\
         @noun\tpage\tfeminine: Seite; accusative=Seite\n@noun\tconnector\tmasculine: Verbinder\n\
         \tDelete this {{item :noun}}?\t.input {{$item :noun}} .match $item feminine {{{{Diese {{$item}} löschen?}}}} * {{{{Dieses {{$item :noun form=accusative}} löschen?}}}}\n\
         @plural\t{{n}} shape|{{n}} shapes\t.input {{$n :number}} .match $n zero {{{{kein}}}} one {{{{eins}}}} two {{{{zwei}}}} few {{{{{{$n}} f}}}} many {{{{{{$n}} m}}}} * {{{{{{$n}} x}}}}\n\
         @plural\t{{n}} file|{{n}} files\t{{n}} Datei|{{n}} Dateien\n\
         @ordinal\t{key}\t.input {{$n :number select=ordinal}} .match $n one {{{{{{$n}}re}}}} * {{{{{{$n}}e}}}}\n\
         @ordinal\t{key}\tAn die {{n}}. Stelle\n\
         @msg\tCould not open {{name}}: {{why}}\tKonnte {{name}} nicht öffnen: {{why}}\n\
         @msgplural\t{{n}} page|{{n}} pages\t{{n}} Seite|{{n}} Seiten\n\
         menu\tOpen\tÖffnen\n@id\tfile.save\tSpeichern\n\tCopy {{n :number}} of {{name}}\t{{n}} Kopien von {{name}}\n"
    )
}

const MESSAGES: [&str; 3] = [
    ".input {$n :number} .match $n zero {{لا أشكال}} one {{شكل}} two {{شكلان}} few {{{$n} أشكال}} many {{{$n} شكلًا}} * {{{$n} شكل}}",
    ".input {$item :noun} .local $d = {$item :noun form=accusative} .match $item feminine {{Diese {$d} \\{x\\}}} * {{Dieses {$d}}}",
    ".input {$n :number select=ordinal} .input {$k :integer} .match $n $k one 1 {{{$n} {$k}}} * * {{{$k}…}}",
];

/// Every kind of lookup, with values of every type, on each language of `i18n`.
fn lookups(i18n: &I18n) {
    for code in CODES {
        let Some(lang) = i18n.from_code(code) else { continue };
        let _ = (i18n.t(lang, "Open"), i18n.tc(lang, "menu", "Open"), i18n.tid(lang, "file.save", "Save"), i18n.message(lang, "Could not open a: b"));
        for n in [0, 1, 2, 3, 5, 11, 100, u64::MAX] {
            let _ = i18n.trn(lang, n, "{n} shape", "{n} shapes");
            let _ = i18n.trn_plain(lang, n, "{n} file", "{n} files");
            let _ = i18n.tord(lang, n, ["{n}st", "{n}nd", "{n}rd", "{n}th"]);
        }
        for value in [0.0, 1.5, -2.0, f64::NAN, f64::INFINITY, 1e300] {
            let _ = i18n.trn_decimal(lang, value, 2, "{n} shape", "{n} shapes");
        }
        let long = "ا".repeat(5000);
        let args = [
            ("item", Arg::Noun("page")),
            ("n", Arg::Count(3)),
            ("k", Arg::number(2.5, usize::MAX)),
            ("name", Arg::Name(&long)),
            ("why", Arg::Text("x\u{202e}y")),
        ];
        for source in
            ["Delete this {item :noun}?", "Copy {n :number} of {name}", "K {n :number} {item :noun} {k :integer}", "Could not open {name}: {why}"]
        {
            for context in ["", "@msg", "menu"] {
                let shown = i18n.tm(lang, context, source, &args);
                assert!(shown.chars().count() <= MAX_OUTPUT_CHARS, "{} characters", shown.chars().count());
                assert_eq!(open_controls(&shown), Vec::<char>::new(), "a typed message leaves a bidi control open");
            }
        }
    }
    let _ = (i18n.t(Lang::PSEUDO_LONG, "x"), i18n.tm(Lang::PSEUDO, "", "{a}", &[("a", Arg::Count(1))]));
}

fn catalog_case(bytes: &[u8]) {
    let text = String::from_utf8_lossy(bytes).into_owned();
    let owned = CODES.iter().enumerate().map(|(i, code)| {
        ((*code).to_owned(), if i % 2 == 0 { text.clone() } else { String::new() }, if i % 2 == 1 { text.clone() } else { String::new() })
    });
    let (i18n, _) = I18n::from_owned(owned, &CODES);
    let _ = i18n.check_all();
    lookups(&i18n);
}

fn message_case(bytes: &[u8]) {
    let text = String::from_utf8_lossy(bytes).into_owned();
    let Ok(message) = Message::parse(&text) else { return };
    let _ = (message.patterns().len(), format!("{message:?}"));
    for code in CODES {
        let rules = LocaleRules::for_locale(code);
        let mut grammar = Grammar::default();
        let _ = grammar.set("genders", "masculine feminine neuter");
        let _ = grammar.set("forms", "accusative genitive");
        let locale = Locale { rules: rules.as_ref(), grammar: &grammar };
        for (kind, source) in [
            (RowKind::Plain, "K {n :number} {item :noun} {k :integer}"),
            (RowKind::Plural, "{n} shape|{n} shapes"),
            (RowKind::Ordinal, "{n}st|{n}nd|{n}rd|{n}th"),
        ] {
            let _ = check_message(kind, source, &text, &locale);
        }
    }
    // Shown through a row, whether or not the rules take it: a row that fails them is skipped, one that passes is evaluated.
    let escaped = text.replace('\\', "\\\\").replace('\t', "\\t").replace('\n', "\\n");
    let row = format!(
        "@grammar\tgenders\tmasculine feminine neuter\n@noun\tpage\tfeminine: Seite\n\tK {{n :number}} {{item :noun}} {{k :integer}}\t{escaped}\n"
    );
    let owned = CODES.iter().map(|code| ((*code).to_owned(), row.clone(), String::new()));
    let (i18n, _) = I18n::from_owned(owned, &CODES);
    lookups(&i18n);
    for code in CODES {
        if let Some(lang) = i18n.from_code(code) {
            let args = [("n", Arg::Count(1)), ("item", Arg::Noun("page")), ("k", Arg::number(2.0, 0))];
            let shown = i18n.tm(lang, "", "K {n :number} {item :noun} {k :integer}", &args);
            assert!(shown.chars().count() <= MAX_OUTPUT_CHARS);
            assert_eq!(open_controls(&shown), Vec::<char>::new(), "a typed message leaves a bidi control open");
        }
    }
}

#[test]
fn damaged_catalogs_never_panic_and_keep_their_caps() {
    hammer(&HammerConfig::new("catalog", 1_000), seed_catalog().as_bytes(), catalog_case);
}

#[test]
fn damaged_selector_messages_never_panic_and_keep_their_caps() {
    for (i, message) in MESSAGES.iter().enumerate() {
        hammer(&HammerConfig::new(&format!("message {i}"), 1_000), message.as_bytes(), message_case);
    }
}

#[test]
fn damaged_language_lists_never_panic() {
    let i18n = I18n::from_owned([("de".to_owned(), String::new(), String::new())], &["de"]).0;
    hammer(&HammerConfig::new("tags", 3_000), b"de-DE,fr;q=0.8,zh_Hant_TW.UTF-8@cyrillic,sr_RS@latin", move |bytes| {
        let text = String::from_utf8_lossy(bytes).into_owned();
        let _ = i18n.match_tags(text.split([',', ':', ';']));
        let _ = i18n.match_tag(&text);
    });
}
