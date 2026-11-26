//! The pseudo locales: debug builds only, never offered, every lookup marked; `pseudo-long` also accents and lengthens.

use craft_i18n::{I18n, Lang};

use crate::common::i18n;

fn say(i18n: &I18n, lang: Lang, text: &str) -> String {
    i18n.t(lang, text).to_owned()
}

#[test]
fn pseudo_marks_each_line_and_pseudo_long_accents_and_lengthens_it() {
    let i18n = i18n(&[]);
    assert_eq!(say(&i18n, Lang::PSEUDO, "Page Setup"), "⟦Page Setup⟧");
    assert_eq!(say(&i18n, Lang::PSEUDO, "a\nb"), "⟦a⟧\n⟦b⟧");
    // 10 characters and 4 more: accented letters, a space, and the padding inside the marks.
    let long = say(&i18n, Lang::PSEUDO_LONG, "Page Setup");
    assert_eq!(long, "⟦Ṕáğé Šéţúṕ····⟧");
    assert_eq!(say(&i18n, Lang::PSEUDO_LONG, "a\nb"), "⟦á·⟧\n⟦ƀ·⟧");
}

// A placeholder keeps its name, so the filling that follows still finds it; digits and `…` stay.
#[test]
fn pseudo_long_leaves_placeholders_digits_and_the_ellipsis() {
    let i18n = i18n(&[]);
    let long = say(&i18n, Lang::PSEUDO_LONG, "Copy {n} of {name} 2…");
    assert!(long.contains("{n}") && long.contains("{name}") && long.contains('2') && long.contains("…"), "{long}");
    assert!(!long.contains("Copy"), "{long}");
    let filled = craft_i18n::fmt(&long, &[("n", "7"), ("name", "X")]);
    assert!(filled.contains('7') && filled.contains('X') && !filled.contains('{'), "{filled}");
}

#[test]
fn counts_and_ordinals_are_marked_too() {
    let i18n = i18n(&[]);
    assert_eq!(i18n.trn(Lang::PSEUDO, 2, "{n} file", "{n} files"), "⟦2 files⟧");
    let long = i18n.trn(Lang::PSEUDO_LONG, 1, "{n} file", "{n} files");
    assert!(long.starts_with('⟦') && long.ends_with('⟧') && !long.contains("file"), "{long}");
    let english = ["{n}st", "{n}nd", "{n}rd", "{n}th"];
    assert_eq!(i18n.tord(Lang::PSEUDO, 3, english), "⟦3rd⟧");
}

// Both are usable in debug builds only, and neither is ever listed or matched from a system language.
#[test]
fn the_pseudo_locales_are_never_offered() {
    let i18n = i18n(&[]);
    assert_eq!(i18n.languages(), [Lang::EN]);
    assert_eq!(i18n.from_code("pseudo").is_some(), cfg!(debug_assertions));
    assert_eq!(i18n.from_code("pseudo-long").is_some(), cfg!(debug_assertions));
    assert_eq!(i18n.code(Lang::PSEUDO_LONG), "pseudo-long");
    assert_eq!(i18n.match_tag("pseudo"), None);
    assert_eq!(i18n.match_tag("pseudo-long"), None);
    assert_eq!(i18n.looked_up().len(), 0, "nothing looked up yet in pseudo");
    let _ = i18n.t(Lang::PSEUDO_LONG, "Save");
    let _ = i18n.t(Lang::PSEUDO, "Open");
    assert_eq!(i18n.looked_up().into_iter().collect::<Vec<_>>(), ["Open", "Save"]);
}
