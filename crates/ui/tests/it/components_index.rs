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

/// A scratch workspace contains only this trial's behaviour docs, never a user's settings or files.
struct Scratch(std::path::PathBuf);

impl Scratch {
    fn new() -> Self {
        let base = Path::new(env!("CARGO_TARGET_TMPDIR"));
        std::fs::create_dir_all(base).expect("the test target's temporary directory exists");
        for attempt in 0..32 {
            let path = base.join(format!("components-index-links-{}-{attempt}", std::process::id()));
            match std::fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("create private scratch directory {path:?}: {error}"),
            }
        }
        panic!("no unused components-index scratch directory");
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Generated Markdown keeps valid reader-facing links, but a qualified Rustdoc target is rendered as its label.
#[trial]
fn purpose_links_keep_web_files_and_anchors_but_not_rustdoc_paths() {
    let scratch = Scratch::new();
    let modules = scratch.0.join("crates/behavior/src");
    std::fs::create_dir_all(&modules).expect("scratch behaviour modules");
    let cases = [
        ("the view of the [`Canvas`](super::Canvas)", "the view of the `Canvas`"),
        ("when it [`previews`](ParamDialog::previews)", "when it `previews`"),
        ("the [`extent`](Self::extent()) it has", "the `extent` it has"),
        ("the [`Rust`](https://example.com/wiki/Rust_(language)) book", "the [`Rust`](https://example.com/wiki/Rust_(language)) book"),
        ("the [guide](README)", "the [guide](README)"),
        ("the [licence](LICENSE)", "the [licence](LICENSE)"),
        ("the [guide](docs/guide.md)", "the [guide](docs/guide.md)"),
        ("the [note](#anchor)", "the [note](#anchor)"),
        ("a | b", r"a \| b"),
        ("a [`x`](super::X", "a [`x`](super::X"),
        ("a [`Param`]", "a [`Param`]"),
        ("the [bare](Canvas)", "the [bare](Canvas)"),
    ];
    for (index, (source, _)) in cases.iter().enumerate() {
        std::fs::write(modules.join(format!("case_{index}.rs")), format!("//! {source}\n")).expect("write one owned scratch module");
    }
    let index = components_index(&scratch.0, &[]);
    for (number, (_, expected)) in cases.iter().enumerate() {
        assert!(
            index.contains(&format!("| `craft_behavior::case_{number}` | {expected} |")),
            "case {number} did not preserve its reader-facing text: {index}"
        );
    }
}

/// A Rustdoc target is not a file beside this page. Exercise the actual library registrations as well as the fixture.
#[trial]
fn purpose_links_in_the_real_index_have_no_rustdoc_destinations() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = components_index(&root, &library_registrations());
    let targets: Vec<_> = index.split("](").skip(1).filter_map(|rest| rest.split(')').next()).filter(|target| target.contains("::")).collect();
    assert!(targets.is_empty(), "the rendered index still links qualified Rust paths: {targets:?}");
}
