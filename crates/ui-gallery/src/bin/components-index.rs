//! Writes `docs/components-index.md`: every registered component type and every craft-behavior module, with its
//! module and one line of purpose.
//!
//! `cargo run -p craft-ui-gallery --bin components-index` rewrites the file; with `--check` it exits with an error
//! when the file differs from what the registrations give. The `components_index` trial of craft-ui makes the same
//! comparison.

use craft_ui::testing::index::components_index;
use craft_ui::testing::library_registrations;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const OUTPUT: &str = "docs/components-index.md";

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn main() -> ExitCode {
    let root = workspace_root();
    let text = components_index(&root, &library_registrations());
    let target = root.join(OUTPUT);
    if std::env::args().any(|argument| argument == "--check") {
        let current = fs::read_to_string(&target).unwrap_or_default();
        if current == text {
            return ExitCode::SUCCESS;
        }
        let _ = writeln!(std::io::stderr(), "{OUTPUT} is stale; run the generator");
        return ExitCode::FAILURE;
    }
    match fs::write(&target, text) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let _ = writeln!(std::io::stderr(), "cannot write {OUTPUT}: {error}");
            ExitCode::FAILURE
        }
    }
}
