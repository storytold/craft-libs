//! Integration tests of craft-settings: migrate.

use super::common;

use common::{Theme, open_in, read_config, write_config};

#[test]
fn older_schema_is_migrated_step_by_step_and_saved() {
    let tmp = tempfile::tempdir().unwrap();
    write_config(tmp.path(), "schema_version = 1\n\n# keep me\n[appearance]\ndark = true\nui_scale = 1.25\n");
    let (mut store, report) = open_in(tmp.path());
    assert_eq!(report.migrated_from, Some(1), "{report:?}");
    assert!(report.section_errors.is_empty(), "{report:?}");
    assert_eq!(store.get().appearance.theme, Theme::Dark);
    assert_eq!(store.get().appearance.ui_scale, 1.25);
    assert!(store.is_dirty(), "a migrated file is written back");

    store.flush().unwrap();
    let text = read_config(tmp.path());
    assert!(text.starts_with("schema_version = 2\n"), "{text}");
    assert!(text.contains("# keep me"), "{text}");
    assert!(text.contains("theme = \"dark\""), "{text}");
    assert!(!text.contains("dark = true"), "{text}");
}
