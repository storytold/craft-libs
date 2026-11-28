//! The gallery registry must follow its manifest's suite dependencies.

use craft_ui::testing::trial;
use std::collections::BTreeSet;

fn suite_dependencies(table: &toml::Table, found: &mut BTreeSet<String>) {
    for (name, value) in table {
        let Some(dependency) = value.as_table() else { continue };
        if name != "craft-ui"
            && dependency
                .get("features")
                .and_then(toml::Value::as_array)
                .is_some_and(|features| features.iter().any(|feature| feature.as_str() == Some("suite")))
        {
            found.insert(name.clone());
        }
    }
}

#[trial]
fn gallery_registers_each_suite_dependency() {
    let manifest: toml::Value = toml::from_str(include_str!("../../Cargo.toml")).expect("gallery manifest parses");
    let mut dependencies = BTreeSet::new();
    if let Some(table) = manifest.get("dependencies").and_then(toml::Value::as_table) {
        suite_dependencies(table, &mut dependencies);
    }
    if let Some(targets) = manifest.get("target").and_then(toml::Value::as_table) {
        for target in targets.values() {
            if let Some(table) = target.get("dependencies").and_then(toml::Value::as_table) {
                suite_dependencies(table, &mut dependencies);
            }
        }
    }
    if let Some(features) = manifest.get("features").and_then(toml::Value::as_table) {
        for value in features.values().filter_map(toml::Value::as_array).flatten().filter_map(toml::Value::as_str) {
            if let Some(name) = value.strip_suffix("/suite")
                && name.starts_with("craft-")
                && name != "craft-ui"
            {
                dependencies.insert(name.to_owned());
            }
        }
    }
    let listed: BTreeSet<_> = include_str!("../../src/lib.rs")
        .lines()
        .filter_map(|line| line.trim().strip_prefix("crates.push((\""))
        .filter_map(|line| line.split_once('"').map(|(name, _)| name.to_owned()))
        .collect();
    assert_eq!(listed, dependencies, "gallery registry and suite dependencies differ");
}
