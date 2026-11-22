//! Writes the node kind versions caches key on (`src/versions.rs`), from the source of what each node kind reaches:
//! see `build/source`. Nothing is written by hand, so changing a node can't be forgotten.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing, clippy::unimplemented, clippy::todo, clippy::unreachable)]

#[allow(dead_code)]
#[path = "build/digest.rs"]
mod digest;
#[allow(dead_code)]
#[path = "build/source/mod.rs"]
mod source;

use digest::Fingerprint;
use source::items::Dispatcher;
use source::modules::reachable;
use source::{SourceFile, SourceTree};
use std::path::{Path, PathBuf};

/// The functions that pick by node kind.
const DISPATCHERS: &[Dispatcher] = &[
    Dispatcher { file: "src/graph.rs", function: "compute" },
    Dispatcher { file: "src/graph.rs", function: "compute_generator" },
    Dispatcher { file: "src/graph.rs", function: "compute_host_node" },
    Dispatcher { file: "src/graph.rs", function: "compute_resident_image" },
    Dispatcher { file: "src/graph.rs", function: "compute_resident_grey" },
    Dispatcher { file: "src/nodes2.rs", function: "compute" },
    Dispatcher { file: "src/atomic.rs", function: "compute" },
    Dispatcher { file: "src/compiled_ops.rs", function: "compute" },
];

/// What every kind goes through: evaluating a graph, and reading and writing images.
const SHARED: &[&str] = &["eval_node", "evaluate_with_inputs", "parse", "encode", "decode"];

/// The code results come out of: what this file's module paths reach.
const START: &str = "src/graph.rs";

fn rust_files(dir: &Path, root: &Path, out: &mut Vec<SourceFile>) {
    let Ok(list) = std::fs::read_dir(dir) else { return };
    let mut paths: Vec<PathBuf> = list.flatten().map(|e| e.path()).collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            rust_files(&path, root, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            let Ok(text) = std::fs::read_to_string(&path) else { continue };
            let name = path.strip_prefix(root).unwrap_or(&path).display().to_string().replace('\\', "/");
            let included = included_files(&text, path.parent().unwrap_or(root));
            out.push(SourceFile { name, text, included });
        }
    }
}

/// The contents of the files `text` names in `include_bytes!` and the like, relative to its folder.
fn included_files(text: &str, folder: &Path) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    for marker in ["include_bytes!(\"", "include_str!(\"", "include!(\""] {
        for (at, _) in text.match_indices(marker) {
            let rest = text.get(at + marker.len()..).unwrap_or("");
            let name = rest.split('"').next().unwrap_or("").to_string();
            if let Ok(data) = std::fs::read(folder.join(&name)) {
                out.push((name, data));
            }
        }
    }
    out
}

/// What isn't source but changes results: the compiler, its settings, the dependencies.
fn toolchain(lock: &Path) -> String {
    let mut f = Fingerprint::of("toolchain");
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_string());
    let version = std::process::Command::new(rustc).arg("-vV").output().map(|o| o.stdout).unwrap_or_default();
    f.bytes(&version);
    for name in ["TARGET", "OPT_LEVEL", "PROFILE", "CARGO_CFG_TARGET_FEATURE", "CARGO_ENCODED_RUSTFLAGS"] {
        f.text(name);
        f.text(&std::env::var(name).unwrap_or_default());
    }
    f.bytes(&std::fs::read(lock).unwrap_or_default());
    f.finish().to_hex()
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=build");
    println!("cargo:rerun-if-changed=src");
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default());
    // the workspace's lock file, two folders up (`crates/material-graph`)
    let lock = manifest.join("../../Cargo.lock");
    println!("cargo:rerun-if-changed={}", lock.display());
    let mut files = Vec::new();
    rust_files(&manifest.join("src"), &manifest, &mut files);
    let engine: Vec<SourceFile> = reachable(&files, "src", &[START]).into_iter().cloned().collect();
    let tree = SourceTree::scan(&engine, DISPATCHERS);

    let mut out = String::new();
    out.push_str(&format!("pub const TOOLCHAIN: &str = {:?};\n", toolchain(&lock)));
    out.push_str(&format!("pub const ALL: &str = {:?};\n", tree.all("all kinds").to_hex()));
    out.push_str(&format!("pub const FILES: usize = {};\n", engine.len()));
    out.push_str("pub const KINDS: &[(&str, &str)] = &[\n");
    for kind in tree.kinds() {
        if let Some(version) = tree.kind_version(kind, SHARED) {
            out.push_str(&format!("    ({kind:?}, {:?}),\n", version.to_hex()));
        }
    }
    out.push_str("];\n");
    let dir = PathBuf::from(std::env::var("OUT_DIR").unwrap_or_default());
    if let Err(e) = std::fs::write(dir.join("kind_versions.rs"), out) {
        println!("cargo:warning=node kind versions not written: {e}");
    }
}
