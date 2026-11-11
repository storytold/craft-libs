//! The facade's `[features]` table is written from the member crates' manifests.
//! `UPDATE_FACADE=1 cargo test -p craft -- integration::features` rewrites the checked-in table.

use super::common;

use common::{Member, members};
use std::collections::BTreeMap;

/// Crate features forwarded by their own rule, not as `<crate>-<feature>`.
const SHARED: &[&str] = &["ui", "screen-testing"];

/// Facade features that do not build for `wasm32-unknown-unknown`, noted on their line.
const NATIVE_ONLY: &[(&str, &str)] = &[
    ("dialogs", "native only: rfd's folder picker and file paths"),
    ("font-manager", "native only: needs craft-font-store's store and network client"),
    ("update", "native only: downloads and installs through getrandom without wasm support"),
];

fn native_only(name: &str) -> Option<&'static str> {
    NATIVE_ONLY.iter().find(|(n, _)| *n == name).map(|(_, note)| *note)
}

fn quote(items: &[String]) -> String {
    items.iter().map(|i| format!("\"{i}\"")).collect::<Vec<_>>().join(", ")
}

fn line(name: &str, items: &[String], note: Option<&str>) -> String {
    let flat = format!("{name} = [{}]", quote(items));
    let body = if flat.len() <= 100 {
        flat
    } else {
        let mut s = format!("{name} = [\n");
        for item in items {
            s.push_str(&format!("    \"{item}\",\n"));
        }
        s.push(']');
        s
    };
    match note {
        Some(note) => format!("{body} # {note}"),
        None => body,
    }
}

/// `craft-x/f` for an always-on crate, `craft-x?/f` for an optional one.
fn forward(m: &Member, feature: &str) -> String {
    let weak = if m.always_on() { "" } else { "?" };
    format!("craft-{}{weak}/{feature}", m.dir)
}

fn forwarded_to_all(members: &[Member], feature: &str) -> Vec<String> {
    members.iter().filter(|m| m.features.contains_key(feature)).map(|m| forward(m, feature)).collect()
}

fn expected(members: &[Member]) -> String {
    let optional = |dir: &str| members.iter().any(|m| m.dir == dir && !m.always_on());
    let mut out = String::from("[features]\n");

    let mut ui = vec!["dep:craft-ui".to_owned(), "dep:egui".to_owned()];
    ui.extend(forwarded_to_all(members, "ui"));
    out += &line("ui", &ui, None);
    out += "\n";
    let mut testing = vec!["ui".to_owned(), "craft-ui/testing".to_owned()];
    testing.extend(forwarded_to_all(members, "screen-testing"));
    out += &line("screen-testing", &testing, None);
    out += "\n";

    let mut seen: BTreeMap<String, String> = BTreeMap::new();
    for m in members {
        let mut group = String::new();
        let mut add = |name: String, text: String| {
            if let Some(other) = seen.insert(name.clone(), m.dir.clone()) {
                panic!("facade feature `{name}` is produced by both craft-{other} and craft-{}", m.dir);
            }
            group += &text;
            group += "\n";
        };
        let is_ui = m.dir == "ui";
        let first = if m.always_on() {
            None
        } else if is_ui {
            Some("ui".to_owned())
        } else {
            Some(m.dir.clone())
        };
        if !m.always_on() && !is_ui {
            let mut items = vec![format!("dep:craft-{}", m.dir)];
            for entry in m.features.get("ui").into_iter().flatten() {
                match common::crate_of(entry) {
                    Some(dep) if dep != m.dir && optional(dep) && dep != "ui" && !items.contains(&dep.to_owned()) => items.push(dep.to_owned()),
                    _ => {}
                }
            }
            add(m.dir.clone(), line(&m.dir, &items, native_only(&m.dir)));
        }
        for feature in m.features.keys().filter(|f| *f != "default" && !SHARED.contains(&f.as_str())) {
            let mut items: Vec<String> = first.iter().cloned().collect();
            items.push(format!("craft-{}/{feature}", m.dir));
            let name = format!("{}-{feature}", m.dir);
            add(name.clone(), line(&name, &items, native_only(&name)));
        }
        if !group.is_empty() {
            out += &group;
            out += "\n";
        }
    }
    out.trim_end().to_owned() + "\n"
}

/// The `[features]` section of the facade's manifest, as text.
fn checked_in(manifest: &str) -> (usize, usize) {
    let start = manifest.find("\n[features]").map_or(0, |i| i + 1);
    let end = manifest[start + 1..].find("\n[").map_or(manifest.len(), |i| start + 1 + i + 1);
    (start, end)
}

#[test]
fn features_table_matches_member_manifests() {
    let members = members();
    let want = expected(&members);
    let path = common::workspace_root().join("crates/craft/Cargo.toml");
    let manifest = std::fs::read_to_string(&path).unwrap();
    let (start, end) = checked_in(&manifest);
    if std::env::var_os("UPDATE_FACADE").is_some() {
        let mut text = manifest[..start].to_owned();
        text += &want;
        text += &manifest[end..];
        std::fs::write(&path, text).unwrap();
        return;
    }
    assert!(
        manifest[start..end].trim_end() == want.trim_end(),
        "crates/craft/Cargo.toml [features] differs from the member crates' manifests; \
         run `UPDATE_FACADE=1 cargo test -p craft -- integration::features` and commit the result"
    );
}
