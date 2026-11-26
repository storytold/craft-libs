#![no_main]

//! Any bytes as the system's list of languages, and as one tag: matching never panics.

use craft_i18n::I18n;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data).into_owned();
    let (i18n, _) = I18n::from_owned([("de".to_owned(), String::new(), String::new()), ("pt-br".to_owned(), String::new(), String::new())], &["de", "pt-br"]);
    let _ = i18n.match_tags(text.split([',', ':', ';', '\n']));
    let _ = i18n.match_tag(&text);
});
