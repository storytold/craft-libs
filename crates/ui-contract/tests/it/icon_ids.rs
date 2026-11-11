//! An icon id missing from the catalog draws nothing and raises no error at build time, so every id written as
//! a literal in the workspace's sources must name a catalog icon. Ids built in match arms, tuple tables and struct
//! fields are not scanned.

use craft_behavior::IconId;
use craft_ui::primitives::has_icon;
use craft_ui::testing::trial;
use std::path::{Path, PathBuf};

/// The ways a source file writes an icon id as a string literal; a `const NAME_ICON: &str = ".."` is read by
/// `const_icon`.
const FORMS: [&str; 3] = ["IconId::new(\"", "icon(\"", "icon = \""];

/// The string a `const <NAME>ICON: <type> = "<id>"` declaration holds, and its byte offset in `line`.
fn const_icon(line: &str) -> Option<(usize, &str)> {
    let rest = line.trim_start().trim_start_matches("pub ").trim_start_matches("pub(crate) ").strip_prefix("const ")?;
    let (name, after) = rest.split_once(':')?;
    if !name.ends_with("ICON") || !name.chars().all(|c| c.is_ascii_uppercase() || c == '_') {
        return None;
    }
    let (_, value) = after.split_once("= \"")?;
    let at = line.len() - value.len();
    Some((at, &value[..value.find('"')?]))
}

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|name| name != "target") {
                sources(&path, out);
            }
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

#[trial]
fn every_icon_id_literal_in_the_workspace_names_a_catalog_icon() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    for krate in std::fs::read_dir(root.join("crates")).unwrap().flatten() {
        sources(&krate.path().join("src"), &mut files);
    }
    let mut seen = 0;
    let mut unknown = Vec::new();
    for file in files.iter() {
        let text = std::fs::read_to_string(file).unwrap();
        for (number, line) in text.lines().enumerate().filter(|(_, line)| !line.trim_start().starts_with("//")) {
            if let Some((at, id)) = const_icon(line) {
                seen += 1;
                if !has_icon(&IconId::from(std::borrow::Cow::Owned(id.to_owned()))) {
                    unknown.push(format!("{}:{}: {:?}", file.strip_prefix(&root).unwrap().display(), number + 1, &line[at..at + id.len()]));
                }
            }
            for form in FORMS {
                for (at, _) in line.match_indices(form) {
                    let before = line[..at].chars().next_back();
                    if form == "icon(\"" && before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                        continue;
                    }
                    let rest = &line[at + form.len()..];
                    let Some(end) = rest.find('"') else { continue };
                    seen += 1;
                    if !has_icon(&IconId::from(std::borrow::Cow::Owned(rest[..end].to_owned()))) {
                        unknown.push(format!("{}:{}: {:?}", file.strip_prefix(&root).unwrap().display(), number + 1, &rest[..end]));
                    }
                }
            }
        }
    }
    assert!(seen > 100, "the scan found {seen} icon ids: its patterns no longer match the sources");
    assert!(unknown.is_empty(), "icon ids that are not in the catalog:\n{}", unknown.join("\n"));
}
