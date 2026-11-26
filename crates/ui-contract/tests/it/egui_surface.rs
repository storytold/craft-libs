//! Tests of the egui APIs, behaviours and fork patches the runtime relies on; run on every egui or fork bump.

use craft_ui::testing::trial;
use std::path::Path;

fn sources(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            sources(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// IN4: a swallowed press gets empty `press_hits` while `hits` still names the widget, so egui and the runtime
/// would disagree; nothing in the library may swallow presses.
#[trial]
fn no_library_code_swallows_presses() {
    let mut files = Vec::new();
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    for name in ["ui", "ui-contract", "ui-widgets", "ui-panels", "ui-canvas", "ui-patterns", "ui-shell"] {
        sources(&crates.join(name).join("src"), &mut files);
    }
    assert!(!files.is_empty(), "the library sources were found");
    let banned = ["swallow_presses_outside", "swallow_outside_press", "allow_presses_in"];
    for file in files {
        let text = std::fs::read_to_string(&file).unwrap_or_default();
        for word in banned {
            assert!(!text.contains(word), "{} names {word}", file.display());
        }
    }
}
