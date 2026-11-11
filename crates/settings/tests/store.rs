//! Integration tests of craft-settings: store.

use super::common;

use common::{Theme, config_path, open_in, read_config, write_config};

#[test]
fn comments_unknown_keys_and_order_survive_a_save() {
    let tmp = tempfile::tempdir().unwrap();
    write_config(
        tmp.path(),
        "schema_version = 2\n\n# my notes\n[appearance]\nui_scale = 1.5 # big\nfuture_key = \"kept\"\n\n[general]\nlanguage = \"de\"\n\n[plugins]\nenabled = true\n",
    );
    let (mut store, report) = open_in(tmp.path());
    assert!(report.section_errors.is_empty(), "{report:?}");
    assert_eq!(store.get().general.language, "de");
    store.update(|p| p.appearance.theme = Theme::Dark);
    store.flush().unwrap();

    let text = read_config(tmp.path());
    assert!(text.contains("# my notes"), "{text}");
    assert!(text.contains("ui_scale = 1.5 # big"), "{text}");
    assert!(text.contains("future_key = \"kept\""), "{text}");
    assert!(text.contains("[plugins]\nenabled = true"), "{text}");
    assert!(text.contains("theme = \"dark\""), "{text}");
    assert!(text.find("[appearance]").unwrap() < text.find("[general]").unwrap(), "section order changed:\n{text}");
}

#[test]
fn outside_edit_with_the_same_mtime_is_still_seen() {
    // Coarse timestamps (FAT, HFS+, network shares) can leave mtime unchanged after an edit.
    let tmp = tempfile::tempdir().unwrap();
    let (mut store, _) = open_in(tmp.path());
    store.flush().unwrap();
    let mtime = std::fs::metadata(config_path(tmp.path())).unwrap().modified().unwrap();

    let edited = read_config(tmp.path()).replace("[general]", "# same second\n[general]");
    write_config(tmp.path(), &edited);
    std::fs::File::options().write(true).open(config_path(tmp.path())).unwrap().set_modified(mtime).unwrap();

    store.update(|p| p.general.language = "fr".into());
    store.flush().unwrap();
    let text = read_config(tmp.path());
    assert!(text.contains("# same second"), "{text}");
    assert!(text.contains("language = \"fr\""), "{text}");
}
