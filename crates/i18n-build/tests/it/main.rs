//! The table the consumer crate's build.rs generated from the fixtures, read by craft-i18n.

use craft_i18n::I18n;
use craft_i18n_build_consumer::{CRAFT_I18N, CRAFT_I18N_OFFERED};

fn i18n() -> I18n {
    let (i18n, problems) = I18n::new(CRAFT_I18N, CRAFT_I18N_OFFERED);
    assert!(problems.is_empty(), "{problems:?}");
    i18n
}

#[test]
fn locales_with_an_app_catalog_are_offered_and_their_region_parent_is_embedded() {
    let codes: Vec<&str> = CRAFT_I18N.iter().map(|(code, ..)| *code).collect();
    assert_eq!(codes, ["fr", "ja", "pt", "pt-br"]);
    assert_eq!(CRAFT_I18N_OFFERED, ["fr", "ja", "pt-br"]);
    let i18n = i18n();
    let offered: Vec<&str> = i18n.languages().into_iter().map(|l| i18n.code(l)).collect();
    assert!(!offered.contains(&"pt"), "{offered:?}");
}

#[test]
fn a_regional_locale_falls_back_to_its_parent_s_shared_rows() {
    let i18n = i18n();
    let Some(lang) = i18n.from_code("pt-br") else { panic!("pt-br is not offered") };
    assert_eq!(i18n.t(lang, "Close"), "Fechar");
    assert_eq!(i18n.t(lang, "Save"), "Salvar");
}

#[test]
fn the_app_s_own_rows_replace_the_checkout_s_and_the_rest_are_kept() {
    let i18n = i18n();
    let Some(lang) = i18n.from_code("pt-br") else { panic!("pt-br is not offered") };
    assert_eq!(i18n.t(lang, "Export"), "Exportar");
    assert_eq!(i18n.t(lang, "Open"), "Abrir");
    assert!(i18n.check_all().is_empty(), "{:?}", i18n.check_all());
}
