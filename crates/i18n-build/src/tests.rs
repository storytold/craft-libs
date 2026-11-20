use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::{Catalogs, Env, Error, Locale, MAX_CATALOG_BYTES, merge};

/// A fresh scratch directory under the system temp dir.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("craft-i18n-build-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    /// Write `text` to `rel` under the scratch directory, creating its folders.
    fn file(&self, rel: &str, text: &str) -> PathBuf {
        let path = self.0.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, text).unwrap();
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Run {
    result: Result<Vec<Locale>, Error>,
    cargo: Vec<String>,
    table: String,
}

/// Run `catalogs` with `CRAFT_I18N_DIR` and `CRAFT_I18N_REQUIRED` as given, `OUT_DIR` and the
/// package directory in `scratch`.
fn run(catalogs: &Catalogs, scratch: &Scratch, dir: Option<&Path>, required: Option<&str>) -> Run {
    let out = scratch.0.join("out");
    std::fs::create_dir_all(&out).unwrap();
    let env = Env {
        dir: dir.map(OsString::from),
        required: required.map(OsString::from),
        out_dir: Some(out.clone().into()),
        manifest_dir: Some(scratch.0.clone().into()),
    };
    let mut cargo = Vec::new();
    let result = catalogs.run(&env, Path::new("craft_i18n.rs"), &mut cargo);
    let table = std::fs::read_to_string(out.join("craft_i18n.rs")).unwrap();
    Run { result, cargo, table }
}

fn codes(locales: &[Locale]) -> Vec<&str> {
    locales.iter().map(|l| l.code.as_str()).collect()
}

fn warnings(cargo: &[String]) -> Vec<&String> {
    cargo.iter().filter(|l| l.starts_with("cargo::warning=")).collect()
}

#[test]
fn without_a_checkout_the_table_is_empty_and_nothing_is_reported() {
    let scratch = Scratch::new("unset");
    let r = run(&Catalogs::new("demo"), &scratch, None, None);
    assert!(r.result.unwrap().is_empty());
    assert!(r.table.contains("CRAFT_I18N: &[(&str, &str, &str)] = &[\n];"), "{}", r.table);
    assert!(warnings(&r.cargo).is_empty(), "{:?}", r.cargo);
}

#[test]
fn required_without_a_checkout_is_an_error_with_an_empty_table() {
    let scratch = Scratch::new("required");
    scratch.file("i18n/fr/demo.tsv", "\tOpen\tOuvrir\n");
    let r = run(&Catalogs::new("demo").app_dir("i18n"), &scratch, None, Some("1"));
    assert!(matches!(r.result, Err(Error::NotSet)));
    assert!(!r.table.contains("include_str!"), "{}", r.table);
    // Only exactly `1` requires it.
    let r = run(&Catalogs::new("demo").app_dir("i18n"), &scratch, None, Some("yes"));
    assert_eq!(codes(&r.result.unwrap()), ["fr"]);
}

#[test]
fn a_region_s_parent_is_embedded_unoffered_and_locales_without_the_app_are_left_out() {
    let scratch = Scratch::new("parent");
    scratch.file("checkout/locales/pt/common.tsv", "\tClose\tFechar\n");
    scratch.file("checkout/locales/pt-br/demo.tsv", "\tOpen\tAbrir\n");
    scratch.file("checkout/locales/zh-hant/common.tsv", "\tClose\t關閉\n");
    scratch.file("checkout/locales/zh-hant-hk/demo.tsv", "\tOpen\t打開\n");
    // A script is not a region: sr-latn must not pull in Cyrillic sr.
    scratch.file("checkout/locales/sr/common.tsv", "\tClose\tЗатвори\n");
    scratch.file("checkout/locales/sr-latn/demo.tsv", "\tOpen\tOtvori\n");
    scratch.file("checkout/locales/de/common.tsv", "\tClose\tSchließen\n");
    scratch.file("checkout/locales/en/demo.tsv", "\tOpen\tOpen\n");
    scratch.file("checkout/locales/pseudo/demo.tsv", "\tOpen\tOpen\n");
    let r = run(&Catalogs::new("demo"), &scratch, Some(&scratch.0.join("checkout")), None);
    let locales = r.result.unwrap();
    assert_eq!(codes(&locales), ["pt", "pt-br", "sr-latn", "zh-hant", "zh-hant-hk"]);
    let offered: Vec<&str> = locales.iter().filter(|l| l.offered()).map(|l| l.code.as_str()).collect();
    assert_eq!(offered, ["pt-br", "sr-latn", "zh-hant-hk"]);
    assert!(r.table.contains("CRAFT_I18N_OFFERED: &[&str] = &[\"pt-br\", \"sr-latn\", \"zh-hant-hk\"];"), "{}", r.table);
}

#[test]
fn the_app_s_rows_replace_the_checkout_s_with_the_same_context_and_source() {
    let ours = "# ours\n\tExport\tExportar\nmenu\tOpen\tAbrir…\n";
    let theirs = "\u{feff}\tExport\tExportar arquivo\r\n\tOpen\tAbrir\r\n\r\n# theirs\r\n";
    assert_eq!(merge(ours, theirs), "\tExport\tExportar\nmenu\tOpen\tAbrir…\n\tOpen\tAbrir\n");
    // A row craft-i18n would skip doesn't hide the checkout's.
    let ours = "\tOpen\t\n\tSave\n";
    let theirs = "\tOpen\tAbrir\n\tSave\tSalvar\n";
    assert_eq!(merge(ours, theirs), "\tOpen\t\n\tSave\n\tOpen\tAbrir\n\tSave\tSalvar\n");
}

#[test]
fn an_app_row_craft_i18n_rejects_keeps_the_checkout_s_row() {
    // Each row of `ours` breaks one of craft-i18n's parser rules for its context.
    let ours = concat!(
        "\tOpen…\tOuvrir\n",
        "\tSave {name}\tEnregistrer\n",
        "\tClose\tFer\\mer\n",
        "\tQuit\tQuit\u{7}ter\n",
        "@nope\tCut\tCouper\n",
        "@plural\t{n} file|{n} files\t{n} fichier|{n} fichiers|{n} fichiers\n",
        "@plural\tone|{n} pages\tune|{n} pages\n",
    );
    let theirs = concat!(
        "\tOpen…\tOuvrir…\n",
        "\tSave {name}\tEnregistrer {name}\n",
        "\tClose\tFermer\n",
        "\tQuit\tQuitter\n",
        "@nope\tCut\tCouper\n",
        "@plural\t{n} file|{n} files\t{n} fichier|{n} fichiers\n",
        "@plural\tone|{n} pages\t{n} page|{n} pages\n",
    );
    assert_eq!(merge(ours, theirs), format!("{ours}{theirs}"));
    // Rows craft-i18n accepts still hide the checkout's, `@plural` ones with the checkout's count.
    let ours = "\tOpen…\tOuvrir…\n@id\tfile.open\tOuvrir\n@msg\tSaved {path}\t{path} enregistré\n@plural\t{n} file|{n} files\t{n} f|{n} fs\n";
    let theirs =
        "\tOpen…\tOuvrir…\n@id\tfile.open\tOuvre\n@msg\tSaved {path}\tEnregistré {path}\n@plural\t{n} file|{n} files\t{n} fichier|{n} fichiers\n";
    assert_eq!(merge(ours, theirs), ours);
}

#[cfg(unix)]
#[test]
fn a_locale_entry_whose_type_cannot_be_read_is_an_error() {
    let scratch = Scratch::new("dangling");
    scratch.file("checkout/locales/fr/demo.tsv", "\tOpen\tOuvrir\n");
    std::os::unix::fs::symlink(scratch.0.join("nowhere"), scratch.0.join("checkout/locales/de")).unwrap();
    let r = run(&Catalogs::new("demo"), &scratch, Some(&scratch.0.join("checkout")), Some("1"));
    assert!(matches!(&r.result, Err(Error::Read { path, .. }) if path.ends_with("locales/de")), "{:?}", r.result);
}

#[test]
fn a_checkout_without_locales_says_how_to_rebuild_after_adding_it() {
    let scratch = Scratch::new("no-locales");
    std::fs::create_dir_all(scratch.0.join("checkout")).unwrap();
    let r = run(&Catalogs::new("demo"), &scratch, Some(&scratch.0.join("checkout")), None);
    let warning = warnings(&r.cargo).into_iter().next().unwrap();
    assert!(warning.contains("cargo clean -p"), "{warning}");
    assert!(!r.cargo.iter().any(|l| l.starts_with("cargo::rerun-if-changed=")), "{:?}", r.cargo);
}

#[test]
fn a_merged_catalog_is_written_under_out_dir_and_embedded_from_there() {
    let scratch = Scratch::new("merged");
    scratch.file("checkout/locales/fr/demo.tsv", "\tOpen\tOuvrir\n\tSave\tEnregistrer\n");
    scratch.file("i18n/fr/demo.tsv", "\tSave\tSauvegarder\n");
    scratch.file("i18n/ja/demo.tsv", "\tOpen\t開く\n");
    let r = run(&Catalogs::new("demo").app_dir("i18n"), &scratch, Some(&scratch.0.join("checkout")), None);
    let locales = r.result.unwrap();
    assert_eq!(codes(&locales), ["fr", "ja"]);
    let merged = locales[0].app.clone().unwrap();
    assert!(merged.starts_with(scratch.0.join("out")), "{}", merged.display());
    assert_eq!(std::fs::read_to_string(merged).unwrap(), "\tSave\tSauvegarder\n\tOpen\tOuvrir\n");
    assert_eq!(locales[1].app, Some(scratch.0.join("i18n/ja/demo.tsv")));
}

#[test]
fn a_relative_or_missing_checkout_warns_and_is_an_error_when_required() {
    let scratch = Scratch::new("relative");
    scratch.file("i18n/fr/demo.tsv", "\tOpen\tOuvrir\n");
    let catalogs = Catalogs::new("demo").app_dir("i18n");
    for dir in [PathBuf::from("craft-i18n"), scratch.0.join("missing")] {
        let r = run(&catalogs, &scratch, Some(&dir), None);
        assert_eq!(codes(&r.result.unwrap()), ["fr"], "the app's own catalogs stay");
        assert_eq!(warnings(&r.cargo).len(), 1, "{:?}", r.cargo);
        let r = run(&catalogs, &scratch, Some(&dir), Some("1"));
        assert!(matches!(r.result, Err(Error::NotAbsolute { .. } | Error::NoLocales { .. })), "{:?}", r.result);
    }
}

#[test]
fn a_checkout_with_a_bad_catalog_or_a_locale_twice_is_not_used() {
    let scratch = Scratch::new("bad");
    let big = scratch.file("big/locales/fr/demo.tsv", "");
    std::fs::File::options().write(true).open(&big).unwrap().set_len(MAX_CATALOG_BYTES + 1).unwrap();
    scratch.file("latin1/locales/fr/demo.tsv", "");
    std::fs::write(scratch.0.join("latin1/locales/fr/demo.tsv"), b"\tOpen\tOuvr\xe9\n").unwrap();
    scratch.file("twice/locales/fr/demo.tsv", "\tOpen\tOuvrir\n");
    scratch.file("twice/locales/FR/common.tsv", "\tSave\tEnregistrer\n");
    // A folder where a catalog belongs.
    std::fs::create_dir_all(scratch.0.join("folder/locales/fr/demo.tsv")).unwrap();
    scratch.file("folder/locales/fr/common.tsv", "\tSave\tEnregistrer\n");
    for checkout in ["big", "latin1", "twice", "folder"] {
        let dir = scratch.0.join(checkout);
        // A case-insensitive file system has one `fr` folder.
        if checkout == "twice" && std::fs::read_dir(dir.join("locales")).unwrap().count() == 1 {
            continue;
        }
        let r = run(&Catalogs::new("demo"), &scratch, Some(&dir), None);
        assert!(r.result.unwrap().is_empty(), "{checkout}");
        assert_eq!(warnings(&r.cargo).len(), 1, "{checkout}: {:?}", r.cargo);
        // Repairing the checkout reruns the build.
        let watch = format!("cargo::rerun-if-changed={}", dir.join("locales").display());
        assert!(r.cargo.contains(&watch), "{checkout}: {:?}", r.cargo);
        let r = run(&Catalogs::new("demo"), &scratch, Some(&dir), Some("1"));
        assert!(matches!(r.result, Err(Error::Read { .. } | Error::LocaleTwice { .. })), "{checkout}: {:?}", r.result);
    }
}

#[test]
fn an_app_name_that_is_not_a_file_name_is_an_error() {
    let scratch = Scratch::new("name");
    for app in ["", "Demo", "../demo", "de mo"] {
        let r = run(&Catalogs::new(app), &scratch, None, None);
        assert!(matches!(r.result, Err(Error::AppName { .. })), "{app:?}");
    }
}

#[test]
fn an_unreadable_app_dir_is_an_error_even_when_not_required() {
    let scratch = Scratch::new("app-dir");
    let r = run(&Catalogs::new("demo").app_dir("missing"), &scratch, None, None);
    assert!(matches!(r.result, Err(Error::Read { .. })), "{:?}", r.result);
    std::fs::create_dir_all(scratch.0.join("i18n/fr/demo.tsv")).unwrap();
    let r = run(&Catalogs::new("demo").app_dir("i18n"), &scratch, None, None);
    assert!(matches!(r.result, Err(Error::Read { .. })), "{:?}", r.result);
}
