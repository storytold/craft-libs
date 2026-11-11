//! The components index (`docs/components-index.md`) is what the registrations give, and the level tables name only
//! registered types.

use craft_ui::contract::stale_table_rows;
use craft_ui::testing::index::components_index;
use craft_ui::testing::library_registrations;
use craft_ui::testing::trial;
use std::path::Path;

#[trial]
fn the_committed_components_index_is_what_the_registrations_give() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let committed = std::fs::read_to_string(root.join("docs/components-index.md")).unwrap_or_default();
    assert!(
        committed == components_index(&root, &library_registrations()),
        "docs/components-index.md is stale; run `cargo run -p craft-ui-gallery --bin components-index`"
    );
}

/// Guards against a surface or story layout row whose type or story group is gone; the library's crates are linked
/// together only here.
#[trial]
fn the_level_tables_name_only_registered_types_and_story_groups() {
    let stale = stale_table_rows(&library_registrations());
    assert!(stale.is_empty(), "stale level table rows: {stale:#?}");
}
