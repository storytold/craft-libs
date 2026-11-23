//! Tests for the i18n facade.

use super::*;
use crate::matcher::{Matcher, Rank};
use unicode_bidi::{BidiClass, bidi_class};

impl I18n {
    /// The embedded locales parsed so far, by code.
    fn parsed(&self) -> Vec<&str> {
        let mut codes: Vec<&str> = self.locales.iter().skip(2).filter(|l| l.parsed.get().is_some()).map(|l| l.code.as_str()).collect();
        codes.sort_unstable();
        codes
    }
}

static E: &Embedded = &[
    ("fr", "\tSave\tEnregistrer\n\tOpen\tOuvrir\nmenu\tOpen\tOuvrir le menu\n@id\tfile.save\tEnregistrer le fichier\n", "\tSave\tSauvegarder\n"),
    ("pt", "\tSave\tGuardar\n\tUndo\tDesfazer\n", ""),
    ("pt-br", "\tSave\tSalvar\n", ""),
    ("de", "\tBad\n@nope\tA\tB\n\tUndo\tRückgängig\n", ""),
];

/// `embedded`, offering every locale in it, with `en-US` numbers rather than the machine's.
fn all(embedded: &'static Embedded) -> (I18n, Vec<String>) {
    let codes: Vec<&str> = embedded.iter().map(|(c, _, _)| *c).collect();
    let (i, problems) = I18n::new(embedded, &codes);
    i.set_format(Format::from_tag("en-US"));
    (i, problems)
}

fn lang(i: &I18n, code: &str) -> Lang {
    i.from_code(code).unwrap()
}

#[test]
fn translated_labels_share_storage_across_clones_and_follow_their_own_overlay() {
    let word = Word::plain("Save");
    let words = Words::new(|_| Some("Enregistrer".to_owned()));
    let first = words.translated_label(word).unwrap();
    let cloned = words.clone().translated_label(word).unwrap();
    assert!(std::sync::Arc::ptr_eq(&first, &cloned));

    let overlay = words.overlay(&WordRows::new().row(word, "Sauvegarder"));
    assert_eq!(overlay.translated_label(word).as_deref(), Some("Sauvegarder"));
    assert_eq!(words.translated_label(word).as_deref(), Some("Enregistrer"));
}

static P: &Embedded = &[
    ("fr", "@plural\t{n} page|{n} pages\t{n} page|{n} pages\n", ""),
    ("ru", "@plural\t{n} file|{n} files\t{n} файл|{n} файла|{n} файлов\n", ""),
    ("ja", "@plural\t{n} file|{n} files\t{n} 個\n", ""),
    ("ar", "@plural\t{n} file|{n} files\t{n} ملفات|{n} ملف واحد|{n} ملفان|{n} ملفات|{n} ملفًا|{n} ملف\n", ""),
    ("fa", "@plural\t{n} page|{n} pages\t{n} صفحه|{n} صفحه\n", ""),
];

#[test]
fn fmt_fills_once() {
    assert_eq!(fmt("{a} to {b}", &[("a", "x"), ("b", "{a}")]), "x to {a}");
    assert_eq!(fmt("{z} {", &[]), "{z} {");
}

/// `text` in display order, as one line in a left-to-right paragraph, without bidi controls.
fn visual(text: &str) -> String {
    let info = unicode_bidi::BidiInfo::new(text, Some(unicode_bidi::Level::ltr()));
    let para = &info.paragraphs[0];
    let shown = info.reorder_line(para, para.range.clone());
    shown
        .chars()
        .filter(|c| bidi_class(*c) != BidiClass::BN && !"\u{202A}\u{202B}\u{202C}\u{202D}\u{202E}\u{2066}\u{2067}\u{2068}\u{2069}".contains(*c))
        .collect()
}

#[test]
fn a_value_with_bidi_controls_cannot_reorder_the_sentence() {
    let file_names = [
        "\u{202E}fdp.exe",
        "\u{202B}abc def",
        "\u{202D}abc def",
        "\u{2067}abc def",
        "\u{2068}abc\u{2069}\u{202E}def",
        "\u{202E}abc\u{2069}",
        "x\u{200F}",
        "x\u{061C}",
    ];
    for name in file_names {
        let text = fmt("Open {name} -> {name} now?", &[("name", name)]);
        let shown = visual(&text);
        assert!(shown.starts_with("Open ") && shown.contains(" -> ") && shown.ends_with(" now?"), "{name:?}: {shown:?}");
        let i = all(P).0;
        let wording = i.trn(Lang::EN, 2, "{n} page in {name}, done", "{n} pages in {name}, done");
        let shown = visual(&fmt(&wording, &[("name", name)]));
        assert!(shown.starts_with("2 pages in ") && shown.ends_with(", done"), "{name:?}: {shown:?}");
    }
    // An override the value opens is closed inside its isolate, and a stray PDF dropped.
    assert_eq!(fmt("Open {name}.", &[("name", "\u{202E}abc")]), "Open \u{2068}\u{202E}abc\u{202C}\u{2069}.");
    assert_eq!(fmt("Open {name}.", &[("name", "a\u{202C}b")]), "Open \u{2068}ab\u{2069}.");
    assert_eq!(fmt("Open {n}.", &[("n", &ltr("\u{202B}1"))]), "Open \u{2066}\u{202B}1\u{202C}\u{2069}.");
    assert_eq!(isolate("\u{2067}a\u{202E}b\u{2069}c"), "\u{2068}\u{2067}a\u{202E}b\u{202C}\u{2069}c\u{2069}");
    // A PDI closes the embeddings opened after its isolate, and one with no isolate open is
    // dropped, however many embeddings are open.
    assert_eq!(isolate("\u{2066}\u{202A}a\u{2069}b"), "\u{2068}\u{2066}\u{202A}a\u{202C}\u{2069}b\u{2069}");
    let embeddings = "\u{202A}".repeat(50_000);
    let wrapped = isolate(&format!("{embeddings}{}", "\u{2069}".repeat(50_000)));
    assert_eq!(wrapped, format!("\u{2068}{embeddings}{}\u{2069}", "\u{202C}".repeat(50_000)));
}

#[test]
fn script_subtag_never_picks_another_script() {
    let i = all(&[("sr", "", ""), ("zh-hant", "", ""), ("fr", "", ""), ("be", "", "")]).0;
    for (tag, want) in [
        ("sr-Latn-RS", None),
        ("sr-Cyrl-RS", Some("sr")),
        ("zh-Hant-TW", Some("zh-hant")),
        ("zh-Hans", None),
        ("fr-Latn-FR", Some("fr")),
        ("be_BY@latin", None),
        ("be_BY", Some("be")),
    ] {
        assert_eq!(i.match_tag(tag).map(|l| i.code(l)), want, "{tag}");
    }
}

#[test]
fn system_tags_match_languages_in_order() {
    let i = all(&[("fr", "", ""), ("pt-br", "", ""), ("zh-hant", "", ""), ("de", "", "")]).0;
    for (tag, want) in [
        ("fr_CA.UTF-8", Some("fr")),
        ("fr-FR", Some("fr")),
        ("pt-BR", Some("pt-br")),
        ("zh-Hant-MO", Some("zh-hant")),
        ("de_DE@euro", Some("de")),
        ("xx-YY", None),
    ] {
        assert_eq!(i.match_tag(tag).map(|l| i.code(l)), want, "{tag}");
    }
    assert!(i.match_tag("C") == Some(Lang::EN) && i.match_tag("POSIX") == Some(Lang::EN));
    assert_eq!(i.code(i.match_tags(["xx", "de", "fr"])), "de", "the first known tag wins");
    assert!(i.match_tags(["xx", "yy"]) == Lang::EN && i.match_tags([]) == Lang::EN, "English when none fits");
}

#[test]
fn lookup_falls_back_to_english_and_context_never_borrows_the_plain_row() {
    let (i, _) = all(E);
    let fr = lang(&i, "fr");
    assert_eq!(i.t(fr, "Save"), "Sauvegarder", "the app's row wins over the shared one");
    assert_eq!(i.t(fr, "Open"), "Ouvrir");
    assert_eq!(i.tc(fr, "menu", "Open"), "Ouvrir le menu");
    assert_eq!(i.tid(fr, "file.save", "Save file"), "Enregistrer le fichier");
    assert_eq!(i.t(fr, "Not translated"), "Not translated");
    assert_eq!(i.get(fr, "menu", "Save"), None, "a context row is not the plain row");
    assert_eq!(i.t(Lang::EN, "Save"), "Save");
}

#[test]
fn plurals_take_the_languages_categories() {
    let i = all(P).0;
    let ru = lang(&i, "ru");
    for (n, want) in [(1, "1 файл"), (2, "2 файла"), (5, "5 файлов"), (11, "11 файлов"), (21, "21 файл"), (22, "22 файла")]
    {
        assert_eq!(i.trn(ru, n, "{n} file", "{n} files"), want, "{n}");
    }
}

#[test]
fn a_file_name_or_path_keeps_its_extension_and_separators_left_to_right_in_a_right_to_left_text() {
    let rtl = Direction::Rtl;
    let lrm = '\u{200E}';
    // What the isolates and marks wrap is the input, unchanged.
    let bare = |shown: &str| strip_isolates(shown).replace(lrm, "");

    let named = file_name("تقرير 2.docx", rtl);
    assert!(named.contains(&format!("{lrm}.docx")), "a mark directly before the extension: {named:?}");
    assert_eq!(bare(&named), "تقرير 2.docx");
    for text in ["ملاحظات", "v1.2 تقرير"] {
        let shown = file_name(text, rtl);
        assert!(!shown.contains(lrm), "no extension, no mark: {shown:?}");
        assert_eq!(bare(&shown), text);
    }

    for sep in ['/', '\\'] {
        let input = format!("C:{sep}ملفات{sep}تقرير.docx");
        let shown = path(&input, rtl);
        assert_eq!(shown.matches(&format!("{sep}{lrm}")).count(), 2, "a mark after every {sep:?}: {shown:?}");
        assert!(shown.contains(&format!("{lrm}.docx")), "a mark before the extension: {shown:?}");
        assert_eq!(bare(&shown), input);
    }

    for text in ["تقرير 2.docx", "v1.2 تقرير", "C:/ملفات/تقرير.docx"] {
        assert_eq!(file_name(text, Direction::Ltr), text);
        assert_eq!(path(text, Direction::Ltr), text);
    }
}

#[test]
fn check_all_reports_every_bad_row() {
    let (i, problems) = all(E);
    assert!(i.parsed().is_empty(), "new parses nothing");
    assert!(problems.iter().all(|p| !p.contains("line")), "{problems:?}");
    let found = i.check_all();
    assert_eq!(found.len(), 2, "{found:?}");
    assert!(found[0].starts_with("de/common: line 1:"), "{found:?}");
    assert!(found[1].starts_with("de/common: line 2:"), "{found:?}");
    assert_eq!(i.check_all(), found);
    assert_eq!(i.parsed(), ["de", "fr", "pt", "pt-br"]);
}

#[test]
fn lookup_parses_only_what_it_reaches() {
    static LAZY: &Embedded =
        &[("pt", "\tUndo\tDesfazer\n", ""), ("pt-br", "\tSave\tSalvar\n", ""), ("de", "\tUndo\tRückgängig\n", ""), ("fr", "\tUndo\tAnnuler\n", "")];
    let i = all(LAZY).0;
    assert!(i.parsed().is_empty());
    assert_eq!(i.t(lang(&i, "pt-br"), "Save"), "Salvar");
    assert_eq!(i.parsed(), ["pt-br"]);
    assert_eq!(i.t(lang(&i, "pt-br"), "Undo"), "Desfazer");
    assert_eq!(i.parsed(), ["pt", "pt-br"]);
    assert_eq!(i.t(lang(&i, "pt-br"), "Undo"), "Desfazer");
    assert_eq!(i.parsed(), ["pt", "pt-br"]);
    assert_eq!(i.t(Lang::EN, "Undo"), "Undo");
    assert_eq!(i.t(Lang::PSEUDO, "Undo"), "⟦Undo⟧");
    assert_eq!(i.parsed(), ["pt", "pt-br"]);
}

#[test]
fn words_in_any_order_match_after_a_whole_run() {
    // "Delete page" in Arabic and Hebrew, queried with its two words the other way round.
    for (locale, text, reversed, missing) in [("ar", "حذف صفحة", "صفحة حذف", "صفحة نسخ"), ("he", "מחק עמוד", "עמוד מחק", "עמוד העתק")]
    {
        let folder = Folder::new(locale);
        let matcher = Matcher::new(&folder, reversed);
        let found = matcher.score(text).map(|found| (found.rank, found.ranges));
        assert_eq!(found, Some((Rank::AllWords, vec![0..6, 7..15])), "{locale}: each word's first occurrence, in text order");
        let whole = matcher.score(reversed).map(|found| found.rank);
        assert_eq!(whole, Some(Rank::Whole), "{locale}");
        assert!(
            Matcher::new(&folder, text).score(&format!("x {text}")).is_some_and(|run| run.rank < Rank::AllWords),
            "{locale}: a whole run ranks first"
        );
        assert_eq!(Matcher::new(&folder, missing).score(text), None, "{locale}: every word must occur");
        assert_eq!(Matcher::new(&Folder::new("en"), reversed).score(text), None, "{locale}: a left-to-right folder keeps the whole run");
    }
}

#[test]
fn lookups_fall_back_to_the_english_row_then_the_source_by_context_and_id() {
    let i = all(E).0;
    let fr = lang(&i, "fr");
    // The app's row comes before the shared one.
    assert_eq!(i.t(fr, "Save"), "Sauvegarder");
    assert_eq!(i.tc(fr, "menu", "Open"), "Ouvrir le menu");
    assert_eq!(i.tc(fr, "no such context", "Open"), "Ouvrir", "a context with no row takes the plain row");
    assert_eq!(i.get(fr, "no such context", "Open"), None, "get takes no other row");
    assert_eq!(i.tid(fr, "file.save", "Save"), "Enregistrer le fichier");
    assert_eq!(i.tid(fr, "no.such.id", "Open"), "Ouvrir", "an id with no row takes the row of its English label");
    assert_eq!(i.tid(fr, "no.such.id", "No such label"), "No such label");
    assert_eq!(i.t(fr, "no such label"), "no such label");
    // A regional locale falls back to its language, and only then to the source.
    let pt_br = lang(&i, "pt-br");
    assert_eq!(i.t(pt_br, "Save"), "Salvar");
    assert_eq!(i.t(pt_br, "Undo"), "Desfazer");
    assert_eq!(i.t(pt_br, "Open"), "Open");
    assert_eq!(i.t(Lang::EN, "Save"), "Save");
}

#[test]
fn catalog_problems_are_reported_and_escapes_kept() {
    static C: &Embedded = &[(
        "ja",
        "# c\r\n\tLine\\nTab\\tPath\\\\\tOther\\nTab\\tPath\\\\\r\nweight\tLight\tThin\r\ntheme\tLight\tPale\r\nno tabs here\n\tonly\n\ta\tb\tc\textra\n\t\tempty source\n\tOpen…\tOpen\n\tok\tはい\n\tok\tまた\n\tBad\\q\tx\n",
        "",
    )];
    let (i, problems) = all(C);
    assert!(problems.is_empty(), "rows are checked when the locale is first read: {problems:?}");
    let ja = lang(&i, "ja");
    assert_eq!(i.t(ja, "Line\nTab\tPath\\"), "Other\nTab\tPath\\");
    assert_eq!(i.tc(ja, "weight", "Light"), "Thin");
    assert_eq!(i.tc(ja, "theme", "Light"), "Pale");
    assert_eq!(i.t(ja, "ok"), "はい", "of a row defined twice, the first is kept");
    assert_eq!(i.t(ja, "Open…"), "Open…", "a row that drops the ellipsis is skipped");
    let found = i.check_all();
    let lines: Vec<&str> = found.iter().filter_map(|p| p.strip_prefix("ja/common: line ")).filter_map(|p| p.split(':').next()).collect();
    assert_eq!(lines, ["5", "6", "7", "8", "9", "11", "12"], "{found:?}");
}

#[test]
fn plural_forms_follow_cldr_for_every_offered_language() {
    static C: &Embedded = &[
        ("fr", "@plural\t{n} file|{n} files\tun {n}|autre {n}\n", ""),
        ("ru", "@plural\t{n} file|{n} files\tone {n}|few {n}|many {n}\n", ""),
        ("ja", "@plural\t{n} file|{n} files\tother {n}\n", ""),
        ("ar", "@plural\t{n} file|{n} files\tzero {n}|one {n}|two {n}|few {n}|many {n}|other {n}\n", ""),
        ("pl", "@plural\t{n} file|{n} files\tone {n}|few {n}|many {n}\n", ""),
        ("pt", "@plural\t{n} file|{n} files\tum {n}|outro {n}\n", ""),
        ("pt-br", "@plural\t{n} file|{n} files\tum {n}|outro {n}\n", ""),
        ("de", "@plural\t{n} file|{n} files\teins {n}|zwei {n}|drei {n}\n", ""),
    ];
    let i = all(C).0;
    let counted = |code: &str, n: u64| i.trn_plain(lang(&i, code), n, "{n} file", "{n} files");
    for (code, expected) in [
        ("fr", &[(0, "un 0"), (1, "un 1"), (2, "autre 2"), (1_000_000, "autre 1000000")][..]),
        ("ru", &[(1, "one 1"), (2, "few 2"), (5, "many 5"), (11, "many 11"), (21, "one 21"), (22, "few 22"), (112, "many 112")]),
        ("ja", &[(1, "other 1"), (2, "other 2")]),
        ("ar", &[(0, "zero 0"), (1, "one 1"), (2, "two 2"), (3, "few 3"), (11, "many 11"), (100, "other 100"), (103, "few 103")]),
        ("pl", &[(1, "one 1"), (2, "few 2"), (5, "many 5"), (12, "many 12"), (22, "few 22")]),
        // CLDR's `pt` is Brazilian, where 0 is singular; Portugal's is not.
        ("pt", &[(0, "outro 0"), (1, "um 1"), (2, "outro 2")]),
        ("pt-br", &[(0, "um 0"), (1, "um 1"), (2, "outro 2")]),
        // German has two forms: the three-form row is skipped and the English shows.
        ("de", &[(1, "1 file"), (2, "2 files")]),
    ] {
        for &(n, want) in expected {
            assert_eq!(counted(code, n), want, "{code}: {n}");
        }
    }
    let found = i.check_all();
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].starts_with("de/common: line 1: expected 2"), "{found:?}");
}
