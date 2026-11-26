#![no_main]

//! Any bytes as a catalog, shared and as the app's, in four languages: parsing under the caps, the rules, and every lookup
//! with values of every type. No panic, and a typed message never longer than its cap.

use craft_i18n::limits::MAX_OUTPUT_CHARS;
use craft_i18n::{Arg, I18n};
use libfuzzer_sys::fuzz_target;

const CODES: [&str; 4] = ["de", "ar", "pl", "ja"];

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data).into_owned();
    let owned = CODES.iter().enumerate().map(|(i, code)| {
        ((*code).to_owned(), if i % 2 == 0 { text.clone() } else { String::new() }, if i % 2 == 1 { text.clone() } else { String::new() })
    });
    let (i18n, _) = I18n::from_owned(owned, &CODES);
    let _ = i18n.check_all();
    let long = "ا".repeat(2000);
    let args = [("item", Arg::Noun("page")), ("n", Arg::Count(3)), ("k", Arg::number(2.5, usize::MAX)), ("name", Arg::Name(&long)), ("why", Arg::Text("x\u{202e}y"))];
    for code in CODES {
        let Some(lang) = i18n.from_code(code) else { continue };
        let _ = (i18n.t(lang, "Open"), i18n.tc(lang, "menu", "Open"), i18n.tid(lang, "file.save", "Save"), i18n.message(lang, "Could not open a: b"));
        for n in [0, 1, 2, 3, 5, 11, 100, u64::MAX] {
            let _ = i18n.trn(lang, n, "{n} shape", "{n} shapes");
            let _ = i18n.tord(lang, n, ["{n}st", "{n}nd", "{n}rd", "{n}th"]);
        }
        let _ = i18n.trn_decimal(lang, 1.5, 2, "{n} shape", "{n} shapes");
        for source in ["Delete this {item :noun}?", "Copy {n :number} of {name}", "K {n :number} {item :noun} {k :integer}"] {
            for context in ["", "@msg", "menu"] {
                assert!(i18n.tm(lang, context, source, &args).chars().count() <= MAX_OUTPUT_CHARS);
            }
        }
    }
});
