//! No family crate's `ui` depends on another family crate (Q50: craft-color is the base family).

use super::common;

use common::{Member, members};

/// Family crates from the spec's crate list (inventory/crates.md), with the ones that exist.
/// Kernel and non-UI data crates (craft-ui, craft-theme, craft-i18n, the layer-0 crates craft-host,
/// craft-platform, craft-keymap, craft-quit and craft-window, and the engines) are not listed: any family may use them.
const FAMILIES: &[&str] = &[
    "audio",
    "automation",
    "browse",
    "code",
    "color",
    "dialogs",
    "draw",
    "font-manager",
    "gpu",
    "imaging",
    "layers",
    "menu",
    "pages",
    "print",
    "review",
    "table",
    "text",
    "theme-store",
    "timeline",
    "update",
];

/// Family crates any family's `ui` may depend on.
const BASE: &[&str] = &["color"];

/// craft-menu composes the menu bar from craft-keymap and craft-quit; it is a shell over the families
/// and not a family itself.
const COMPOSITION: &[&str] = &["menu"];

/// Family crates `m`'s `ui` depends on: through its `ui` feature, and through dependencies that turn `ui` on.
fn ui_edges(m: &Member) -> Vec<String> {
    let mut edges: Vec<String> = m.features.get("ui").into_iter().flatten().filter_map(|e| common::crate_of(e)).map(str::to_owned).collect();
    for table in ["dependencies", "build-dependencies"] {
        let Some(deps) = m.manifest.get(table).and_then(|d| d.as_table()) else { continue };
        for (name, dep) in deps {
            let ui_on = dep.get("features").and_then(|f| f.as_array()).is_some_and(|f| f.iter().any(|f| f.as_str() == Some("ui")));
            let always = !dep.get("optional").and_then(|o| o.as_bool()).unwrap_or(false);
            if let (true, true, Some(dir)) = (ui_on, always, name.strip_prefix("craft-")) {
                edges.push(dir.to_owned());
            }
        }
    }
    edges
}

#[test]
fn family_ui_depends_on_no_other_family() {
    let mut offenders = Vec::new();
    for m in members().iter().filter(|m| FAMILIES.contains(&m.dir.as_str()) && !COMPOSITION.contains(&m.dir.as_str())) {
        for dep in ui_edges(m) {
            if dep != m.dir && FAMILIES.contains(&dep.as_str()) && !BASE.contains(&dep.as_str()) {
                offenders.push(format!("craft-{}'s ui depends on craft-{dep}", m.dir));
            }
        }
    }
    assert!(offenders.is_empty(), "family ui crates may depend only on craft-color's ui, not on each other:\n{}", offenders.join("\n"));
}
