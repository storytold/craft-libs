//! Helpers the test modules share: an `I18n` built from catalogs written in the test, with `en-US` numbers rather than the
//! machine's, so no test reads the machine's region or language.

use craft_i18n::{Format, I18n, Lang};

/// One locale: its code, shared catalog and the app's catalog.
pub type Catalog<'a> = (&'a str, &'a str, &'a str);

/// An `I18n` offering every locale of `catalogs`, and the problems found building it.
pub fn build(catalogs: &[Catalog<'_>]) -> (I18n, Vec<String>) {
    let owned = catalogs.iter().map(|(code, shared, app)| ((*code).to_owned(), (*shared).to_owned(), (*app).to_owned()));
    let codes: Vec<&str> = catalogs.iter().map(|(code, _, _)| *code).collect();
    let (i18n, problems) = I18n::from_owned(owned, &codes);
    i18n.set_format(Format::from_tag("en-US"));
    (i18n, problems)
}

/// An `I18n` of `catalogs`, which must build without a problem.
pub fn i18n(catalogs: &[Catalog<'_>]) -> I18n {
    let (i18n, problems) = build(catalogs);
    assert!(problems.is_empty(), "{problems:?}");
    i18n
}

/// The language with `code`, which `i18n` must offer.
pub fn lang(i18n: &I18n, code: &str) -> Lang {
    i18n.from_code(code).unwrap_or_else(|| panic!("{code} is not offered"))
}

/// The bidi controls still open at the end of `text`, innermost last: an embedding or override that no PDF closed and an isolate
/// that no PDI closed (a PDI also closes what is open inside its isolate; a stray PDF or PDI closes nothing).
pub fn open_controls(text: &str) -> Vec<char> {
    let mut open: Vec<char> = Vec::new();
    for c in text.chars() {
        match c {
            '\u{2066}' | '\u{2067}' | '\u{2068}' | '\u{202A}' | '\u{202B}' | '\u{202D}' | '\u{202E}' => open.push(c),
            '\u{202C}' if matches!(open.last(), Some('\u{202A}' | '\u{202B}' | '\u{202D}' | '\u{202E}')) => {
                open.pop();
            }
            '\u{2069}' if open.iter().any(|o| matches!(o, '\u{2066}' | '\u{2067}' | '\u{2068}')) => {
                while let Some(top) = open.pop() {
                    if matches!(top, '\u{2066}' | '\u{2067}' | '\u{2068}') {
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    open
}
