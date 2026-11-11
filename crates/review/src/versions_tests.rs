//! Version history rows.

use crate::versions::*;
use craft_ui::testing::trial;

fn version(id: u64, created: i64, autosave: bool) -> Version {
    Version {
        id: VersionId(id),
        title: if autosave { String::new() } else { format!("v{id}") },
        description: String::new(),
        created,
        author: "Ana".into(),
        autosave,
    }
}

#[trial]
fn autosave_runs_group_between_named_versions() {
    let versions = [
        version(1, 10, false),
        version(2, 20, true),
        version(3, 30, true),
        version(4, 40, false),
        version(5, 50, true),
        version(6, 60, true),
        version(7, 70, true),
    ];
    assert_eq!(
        history_rows(&versions),
        vec![HistoryRow::Autosaves(vec![6, 5, 4]), HistoryRow::Version(3), HistoryRow::Autosaves(vec![2, 1]), HistoryRow::Version(0),]
    );
    let lone = [version(1, 10, false), version(2, 20, true), version(3, 30, false)];
    assert_eq!(history_rows(&lone), vec![HistoryRow::Version(2), HistoryRow::Version(1), HistoryRow::Version(0)]);
}
