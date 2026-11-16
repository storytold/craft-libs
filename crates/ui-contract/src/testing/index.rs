//! The components index: every registered component type, grouped by gallery level, with its module and the purpose its derive recorded
//! from the doc comment, and every craft-behavior module with the first sentence of its module docs.
//!
//! A page is not Rustdoc: an intra-doc link to a qualified Rust path keeps its label and loses its destination, which the
//! page would read as a file beside it (`super::CustomSettings`).
//!
//! The generator binary writes `docs/components-index.md` from [`components_index`] over
//! `craft_ui::testing::library_registrations()`; a trial of craft-ui compares the two.

use std::fs;
use std::path::{Path, PathBuf};

use crate::contract::{Level, Registration, catalog};

/// The first sentence of the first paragraph of `lines`, without its final full stop, as a table cell.
fn purpose(lines: &[&str]) -> String {
    let paragraph: Vec<&str> = lines.iter().map(|line| line.trim()).take_while(|line| !line.is_empty()).collect();
    let text = paragraph.join(" ");
    let sentence = text.split_once(". ").map_or(text.as_str(), |(first, _)| first);
    cell_text(sentence.trim_end_matches('.'))
}

/// `text` as one Markdown table cell: `|` escaped so it does not end the cell, and an intra-doc link to a qualified Rust
/// path kept as its label alone, which the page would otherwise read as a file beside it.
///
/// A destination's own parentheses are counted, so a URL keeps them and a method link ends with the call's `()`.
fn cell_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    // `copied` is where `out` has reached in `text`; `at` is where the next link starts being looked for.
    let (mut copied, mut at) = (0, 0);
    while let Some(bracket) = text[at..].find('[').map(|open| at + open) {
        let label_start = bracket + 1;
        let Some(label_end) = text[label_start..].find(']').map(|close| label_start + close) else { break };
        // Only `[label](destination)` is a link; a bracketed label on its own is the doc comment's own text.
        if !text[label_end + 1..].starts_with('(') {
            at = label_end + 1;
            continue;
        }
        // A destination may hold its own parentheses, so it ends where they balance.
        let Some(close) = closing_paren(text, label_end + 1) else { break };
        if is_rust_path(&text[label_end + 2..close]) {
            out.push_str(&text[copied..bracket]);
            out.push_str(&text[label_start..label_end]);
            copied = close + 1;
        }
        at = close + 1;
    }
    out.push_str(&text[copied..]);
    out.replace('|', "\\|")
}

/// The index of the `)` that closes the `(` at `open`, counting the parentheses inside it: a URL holds its own
/// (`…/Rust_(programming_language)`) and a method's destination ends with the call's `()`.
fn closing_paren(text: &str, open: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (at, character) in text[open..].char_indices() {
        match character {
            '(' => depth += 1,
            ')' if depth == 1 => return Some(open + at),
            ')' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    None
}

/// Whether `destination` is a qualified Rust path, as a doc comment's intra-doc link writes one: `super::CustomSettings`,
/// `ParamDialog::previews`, `Self::extent()`.
///
/// Only a qualified path is flattened. `Canvas` and `README` are written the same way, and an extensionless destination is
/// more likely a file beside the page, so one without `::` stays a link.
fn is_rust_path(destination: &str) -> bool {
    // A segment is a name, a type's name before its arguments, or a method's call.
    let is_segment = |segment: &str| {
        let name = segment.trim_end_matches("()").split('<').next().unwrap_or(segment);
        !name.is_empty() && name.chars().all(|character| character.is_ascii_alphanumeric() || character == '_')
    };
    let segments: Vec<&str> = destination.trim().split("::").collect();
    segments.len() > 1 && segments.iter().all(|segment| is_segment(segment))
}

fn table_row(cells: &[&str]) -> String {
    format!("| {} |\n", cells.join(" | "))
}

/// The `.rs` files directly in `dir`, sorted.
fn top_level_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "rs"))
        .collect();
    files.sort();
    files
}

/// One row of the registered-types tables.
struct Row {
    level: Level,
    name: String,
    shape: String,
    module: String,
    purpose: String,
}

/// The shape column's sort position: components by shape, then the pattern kinds.
fn shape_rank(shape: &str) -> usize {
    ["Leaf", "Body", "Items", "Pattern", "Tool", "Dialog"].iter().position(|known| *known == shape).unwrap_or(usize::MAX)
}

/// The components and patterns of `registered`, ordered by level (the gallery's order), shape, then name.
fn registered_rows(registered: &[&'static Registration]) -> Vec<Row> {
    let mut rows = Vec::new();
    for entry in catalog(registered).entries {
        let level = entry.level;
        if let Some(probe) = entry.registration.probe() {
            rows.push(Row {
                level,
                name: entry.name.clone(),
                shape: format!("{:?}", probe.shape),
                module: probe.module.to_owned(),
                purpose: cell_text(probe.purpose),
            });
        }
        if let Some(probe) = entry.registration.pattern_probe() {
            rows.push(Row {
                level,
                name: entry.name.clone(),
                shape: probe.kind.label().to_owned(),
                module: probe.module.to_owned(),
                purpose: cell_text(probe.purpose),
            });
        }
    }
    rows.sort_by_cached_key(|row| (row.level, shape_rank(&row.shape), row.name.to_lowercase(), row.module.clone()));
    rows
}

/// The types of `registered` that have no purpose: their types lack a doc comment.
pub fn types_without_purpose(registered: &[&'static Registration]) -> Vec<String> {
    registered_rows(registered).into_iter().filter(|row| row.purpose.is_empty()).map(|row| format!("{}::{}", row.module, row.name)).collect()
}

/// The text of `docs/components-index.md` for the workspace at `root` and the types of `registered`.
pub fn components_index(root: &Path, registered: &[&'static Registration]) -> String {
    let mut out = String::from("# Built components\n\n");
    out.push_str("Generated by `cargo run -p craft-ui-gallery --bin components-index`; edit the doc comments, not this file.\n\n");

    out.push_str("## Registered types\n\nGrouped by gallery level, then shape, then name.\n");
    let rows = registered_rows(registered);
    for level in Level::ALL {
        let in_level: Vec<&Row> = rows.iter().filter(|row| row.level == level).collect();
        if in_level.is_empty() {
            continue;
        }
        out.push_str(&format!("\n### {}\n\n| Type | Shape | Module | Purpose |\n|---|---|---|---|\n", level.title()));
        for row in in_level {
            out.push_str(&table_row(&[&format!("`{}`", row.name), &row.shape, &format!("`{}`", row.module), &row.purpose]));
        }
    }

    out.push_str("\n## Behaviours\n\n| Module | Purpose |\n|---|---|\n");
    for file in top_level_files(&root.join("crates/behavior/src")) {
        let stem = file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        if matches!(stem.as_str(), "lib" | "main") {
            continue;
        }
        let Ok(text) = fs::read_to_string(&file) else { continue };
        let docs: Vec<&str> = text.lines().take_while(|line| line.starts_with("//!")).map(|line| &line[3..]).collect();
        out.push_str(&table_row(&[&format!("`craft_behavior::{stem}`"), &purpose(&docs)]));
    }
    out
}
