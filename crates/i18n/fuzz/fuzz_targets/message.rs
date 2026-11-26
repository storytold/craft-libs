#![no_main]

//! Any bytes as a selector message: the parser, the rules of a translation in four languages and three row kinds, and the
//! evaluator through a row. No panic, and the output within its cap.

use craft_i18n::limits::MAX_OUTPUT_CHARS;
use craft_i18n::noun::Grammar;
use craft_i18n::select::{Locale, LocaleRules, Message, RowKind, check_message};
use craft_i18n::{Arg, I18n};
use libfuzzer_sys::fuzz_target;

const CODES: [&str; 4] = ["de", "ar", "pl", "ja"];

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data).into_owned();
    let Ok(message) = Message::parse(&text) else { return };
    let _ = message.patterns();
    let mut grammar = Grammar::default();
    let _ = grammar.set("genders", "masculine feminine neuter");
    let _ = grammar.set("forms", "accusative genitive");
    for code in CODES {
        let rules = LocaleRules::for_locale(code);
        let locale = Locale { rules: rules.as_ref(), grammar: &grammar, rtl: code == "ar" };
        for (kind, source) in [
            (RowKind::Plain, "K {n :number} {item :noun} {k :integer}"),
            (RowKind::Plural, "{n} shape|{n} shapes"),
            (RowKind::Ordinal, "{n}st|{n}nd|{n}rd|{n}th"),
        ] {
            let _ = check_message(kind, source, &text, &locale);
        }
    }
    let escaped = text.replace('\\', "\\\\").replace('\t', "\\t").replace('\n', "\\n");
    let row = format!("@grammar\tgenders\tmasculine feminine neuter\n@noun\tpage\tfeminine: Seite\n\tK {{n :number}} {{item :noun}} {{k :integer}}\t{escaped}\n");
    let owned = CODES.iter().map(|code| ((*code).to_owned(), row.clone(), String::new()));
    let (i18n, _) = I18n::from_owned(owned, &CODES);
    let args = [("n", Arg::Count(1)), ("item", Arg::Noun("page")), ("k", Arg::number(2.0, 0))];
    for code in CODES {
        if let Some(lang) = i18n.from_code(code) {
            assert!(i18n.tm(lang, "", "K {n :number} {item :noun} {k :integer}", &args).chars().count() <= MAX_OUTPUT_CHARS);
        }
    }
});
