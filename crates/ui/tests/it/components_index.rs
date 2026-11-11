//! The components index (`docs/components-index.md`) is what the registrations give.

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
