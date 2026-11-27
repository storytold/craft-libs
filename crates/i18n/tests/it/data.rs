//! The CLDR data blobs load for the ICU4X in use. A blob made for another ICU4X minor version fails to load, and a failed
//! load is silent by design (plurals show in English, dates in ISO form), so these tests ask for the real words.

use craft_i18n::{Digits, Format, I18n, Lang, Region};

fn catalogs() -> &'static craft_i18n::Embedded {
    &[
        ("de", "@plural\t{n} file|{n} files\t{n} Datei|{n} Dateien\n", ""),
        ("ar", "@plural\t{n} file|{n} files\tzero {n}|one {n}|two {n}|few {n}|many {n}|other {n}\n", ""),
    ]
}

// Number data: German takes its own plural forms and the decimal mark of its region.
#[test]
fn the_number_data_loads() {
    let (i18n, problems) = I18n::new(catalogs(), &["de", "ar"]);
    assert!(problems.iter().all(|p| !p.contains("no plural rules")), "{problems:?}");
    let de = i18n.from_code("de").unwrap();
    assert_eq!(i18n.trn_plain(de, 1, "{n} file", "{n} files"), "1 Datei");
    assert_eq!(i18n.trn_plain(de, 2, "{n} file", "{n} files"), "2 Dateien");
    assert_ne!(de, Lang::EN);
    let format = Format::new(&Region::from_setting("de-DE"), Digits::Locale);
    assert_eq!(format.number(1234.5, 1), "1.234,5");
}

// Arabic reaches all six plural categories, so its row needs six forms and all six are found.
#[test]
fn the_plural_data_has_every_category_for_arabic() {
    let (i18n, problems) = I18n::new(catalogs(), &["ar"]);
    assert!(problems.is_empty(), "{problems:?}");
    let ar = i18n.from_code("ar").unwrap();
    let shown: Vec<String> = [0, 1, 2, 3, 11, 100].iter().map(|&n| i18n.trn_plain(ar, n, "{n} file", "{n} files")).collect();
    assert_eq!(shown, ["zero 0", "one 1", "two 2", "few 3", "many 11", "other 100"]);
}

// CLDR 49.0.0 added plural rules for Tajik; the data of the release before it left `tg` without, which shows its plurals
// in English. A language the data has rules for takes them, with the form count of its categories.
#[test]
fn the_plural_data_is_cldr_49() {
    let embedded: &'static craft_i18n::Embedded = &[("tg", "@plural\t{n} file|{n} files\t{n} файл|{n} файлҳо\n", "")];
    let (i18n, problems) = I18n::new(embedded, &["tg"]);
    assert!(problems.is_empty(), "{problems:?}");
    let tg = i18n.from_code("tg").unwrap();
    assert!(i18n.check_all().is_empty(), "{:?}", i18n.check_all());
    assert_eq!(i18n.trn_plain(tg, 1, "{n} file", "{n} files"), "1 файл");
    assert_eq!(i18n.trn_plain(tg, 5, "{n} file", "{n} files"), "5 файлҳо");
}
