//! Message templates: which row a filled-in message matches, and how its values are translated.

use crate::{I18n, Lang, MessageMatch};

/// A German catalog of `(context, source, translation)` rows, and German.
fn german(rows: &[(&str, &str, &str)]) -> (I18n, Lang) {
    let text = rows.iter().map(|(c, s, t)| format!("{c}\t{s}\t{t}")).collect::<Vec<_>>().join("\n");
    let (i18n, problems) = I18n::from_owned([("de".to_string(), String::new(), text)], &["de"]);
    assert!(problems.is_empty(), "{problems:?}");
    let lang = i18n.languages()[1];
    (i18n, lang)
}

#[test]
fn a_filled_message_matches_its_template_and_takes_its_plural_form() {
    let (i18n, de) = german(&[("@msg", "Open {name}", "Öffne {name}"), ("@msg", "Open a.eps", "Exakt")]);
    assert_eq!(i18n.message(de, "Open a.eps"), "Exakt");
    assert_eq!(i18n.message(de, "Open b.eps"), "Öffne b.eps");

    // The template with leading text wins over one that starts with a hole, whatever the file order.
    let (i18n, de) = german(&[("@msg", "{path}: {e}", "{e} ← {path}"), ("@msg", "Open {name}: {e}", "Öffne {name}: {e}")]);
    assert_eq!(i18n.message(de, "Open a: b"), "Öffne a: b");

    let (i18n, de) = german(&[("@msg", "a{x}", "kurz {x}"), ("@msg", "ab{x}", "lang {x}")]);
    assert_eq!(i18n.message(de, "abc"), "lang c");

    let (i18n, de) = german(&[("@msg", "{x} of {y}", "zwei {x} {y}"), ("@msg", "{x} of {y} files", "drei {x} {y}")]);
    assert_eq!(i18n.message(de, "1 of 2 files"), "drei 1 2");

    let (i18n, de) = german(&[("@msgplural", "{n} file|{n} files", "{n} Datei|{n} Dateien")]);
    assert_eq!(i18n.message(de, "1 file"), "1 Datei");
    assert_eq!(i18n.message(de, "3 files"), "3 Dateien");
    assert_eq!(i18n.message(de, "x files"), "x files", "{{n}} matches only a count");

    // Rows the matcher cannot use are reported and the message shows in English.
    let (i18n, de) = german(&[("@msg", "{a}{b}", "{a}{b}"), ("@msgplural", "{n} cat|{n} cats", "{n} Katze")]);
    let problems = i18n.check_all();
    assert!(problems.iter().any(|p| p.contains("unmatchable message template")), "{problems:?}");
    assert!(problems.iter().any(|p| p.contains("plural forms")), "{problems:?}");
    assert_eq!(i18n.message(de, "xy"), "xy");
    assert_eq!(i18n.message(de, "2 cats"), "2 cats");

    let (i18n, _) = german(&[("@msgplural", "{n} file|{n} files", "{n} Datei|{n} Dateien")]);
    assert_eq!(i18n.message(Lang::EN, "1 file"), "1 file");
    assert_eq!(i18n.message(Lang::PSEUDO, "1 file"), i18n.t(Lang::PSEUDO, "1 file"));
}

#[test]
fn placeholder_values_translate_to_a_bounded_depth_and_quoted_ones_stay() {
    let chain: Vec<(&str, String, String)> = (1..=6).map(|i| ("@msg", format!("w{i} {{x}}"), format!("V{i} {{x}}"))).collect();
    let rows: Vec<(&str, &str, &str)> = chain.iter().map(|(c, s, t)| (*c, s.as_str(), t.as_str())).collect();
    let (i18n, de) = german(&rows);
    assert_eq!(i18n.message(de, "w1 w2 w3 w4 w5 w6 end"), "V1 V2 V3 V4 V5 w6 end");

    let (i18n, de) = german(&[
        ("@msg", "Reply: {r}", "Antwort: {r}"),
        ("@msg", "Failed '{why}'", "Fehler '{why}'"),
        ("@msg", "disk full", "Datenträger voll"),
        ("@msg", "{{\"a\": 1}}", "{{\"a\": 1}} (de)"),
        ("@msg", "[{i}] {why}", "[{i}] Grund: {why}"),
    ]);
    assert_eq!(i18n.message(de, "Reply: disk full"), "Antwort: Datenträger voll");
    assert_eq!(i18n.message(de, "Failed 'disk full'"), "Fehler 'disk full'");
    assert_eq!(i18n.message(de, "Reply: {\"a\": 1}"), "Antwort: {\"a\": 1} (de)");
    assert_eq!(i18n.message(de, "Reply: {\"b\": 2}"), "Antwort: {\"b\": 2}");
    assert_eq!(i18n.message(de, "Reply: [0] bad"), "Antwort: [0] Grund: bad");
    assert_eq!(i18n.message(de, "Reply: [x] bad"), "Antwort: [x] bad");
    assert_eq!(i18n.message(de, "Reply: see {who}"), "Antwort: see {who}");

    let upper = |m: &MessageMatch<'_>, key: &str, value: &str| (m.english == "Reply: {r}" && key == "r").then(|| value.to_uppercase());
    assert_eq!(i18n.message_with(de, "Reply: x", &upper), "Antwort: X");
    let always = |_: &MessageMatch<'_>, _: &str, _: &str| Some("HOOK".to_string());
    assert_eq!(i18n.message_with(de, "Failed 'disk full'", &always), "Fehler 'disk full'");

    let long = format!("Reply: {}", "a".repeat(2001));
    assert_eq!(i18n.message(de, &long), long);

    assert_eq!(i18n.message_keeping(de, "Reply: ", "x", ""), "Antwort: x");
    assert_eq!(i18n.message_keeping(de, "Reply: \u{FFFC}", "x", ""), "Reply: \u{FFFC}x");
    let (i18n, de) = german(&[("@msg", "A \u{FFFC}", "B")]);
    assert_eq!(i18n.message_keeping(de, "A ", "kept", ""), "A kept");
}
