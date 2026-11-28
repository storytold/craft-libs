//! Reading catalog rows from outside the crate, as an app's build script or test does.

use craft_i18n::catalog::{Entry, escape, parse, parse_row, unescape};
use craft_i18n::plural_forms;

fn entry(context: &str, source: &str, translation: &str) -> Entry {
    Entry { context: context.into(), source: source.into(), translation: translation.into() }
}

#[test]
fn parse_reads_rows_as_the_lookups_do() {
    let text =
        "\u{feff}# comment\r\n\r\n\tOpen…\tÖffnen…\r\nmenu\tLine\\nbreak\tZeilen\\numbruch\n@plural\t{n} file|{n} files\t{n} Datei|{n} Dateien\n";
    let (rows, problems) = parse(text, plural_forms("de"));
    assert_eq!(problems, Vec::<String>::new());
    assert_eq!(
        rows,
        [
            entry("", "Open…", "Öffnen…"),
            entry("menu", "Line\nbreak", "Zeilen\numbruch"),
            entry("@plural", "{n} file|{n} files", "{n} Datei|{n} Dateien"),
        ]
    );
}

#[test]
fn parse_reports_bad_and_repeated_rows_by_line() {
    let text = "\tOpen\tÖffnen\n\tSave\n\tOpen\tAufmachen\n\tBad\\q\tSchlecht\n";
    let (rows, problems) = parse(text, 2);
    assert_eq!(rows, [entry("", "Open", "Öffnen")]);
    assert_eq!(
        problems,
        ["line 2: 2 columns; expected 3", "line 3: defined twice (same context and source text); the first is kept", "line 4: unknown escape \\q"]
    );
}

#[test]
fn parse_row_checks_one_row() {
    assert_eq!(parse_row("@id\tlanguage.name\tDeutsch", 2), Ok(entry("@id", "language.name", "Deutsch")));
    assert_eq!(parse_row("\t{n} left\t{m} übrig", 2), Err("placeholders differ from the source text".into()));
    assert_eq!(parse_row("@plural\t{n} file|{n} files\t{n} файл|{n} файла", plural_forms("ru")), Err("expected 3 nonempty plural forms".into()));
    assert_eq!(
        parse_row("@msgplural\t{n} file|{n} files\t{n} Datei|{n} Dateien", plural_forms("de")),
        Ok(entry("@msgplural", "{n} file|{n} files", "{n} Datei|{n} Dateien"))
    );
    assert_eq!(parse_row("@msgplural\t{n}|{n}\t{n}|{n}", plural_forms("de")), Err("unmatchable message template".into()));
}

#[test]
fn escape_round_trips_through_unescape_and_parse_row() {
    for text in ["plain", "a\\b", "two\nlines", "tab\there", "\\n is not a line break", "trailing \\"] {
        let escaped = escape(text);
        assert!(!escaped.contains(['\n', '\t']), "{escaped:?}");
        assert_eq!(unescape(&escaped).as_deref(), Ok(text));
        let row = parse_row(&format!("\t{escaped}\t{escaped}"), 2).map(|e| e.translation);
        assert_eq!(row.as_deref(), Ok(text));
    }
    assert_eq!(escape("a\\b\nc\td"), "a\\\\b\\nc\\td");
    assert_eq!(unescape("trailing \\"), Err("trailing backslash".into()));
}

#[test]
fn plural_forms_follow_the_cldr_rules() {
    assert_eq!(plural_forms("de"), 2);
    assert_eq!(plural_forms("pt-BR"), 2);
    assert_eq!(plural_forms("ru"), 3);
    assert_eq!(plural_forms("ar"), 6);
    assert_eq!(plural_forms("ja"), 1);
    assert_eq!(plural_forms("tlh"), 1);
}
