//! Tests for the i18n facade.

use super::*;
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
