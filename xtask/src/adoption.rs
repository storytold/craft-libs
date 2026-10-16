//! Consumer dependency adoption reporting: tracks which applications across the
//! Craft suite have adopted shared libraries and which revision they are pinned to.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::Command;

use serde::{Deserialize, Serialize};

pub const ALL_APPS: &[(&str, &str, &str)] = &[
    ("photocraft", "photocraft-ui-egui", "Raster Image Editing"),
    ("vectorcraft", "vectorcraft-ui-egui", "Vector Illustration & Geometry"),
    ("designcraft", "designcraft-ui-egui", "Page Layout & Typography"),
    ("pdfcraft", "pdfcraft-ui-egui", "PDF Document Inspection"),
    ("lightcraft", "lightcraft-ui-egui", "Non-Destructive RAW Grading"),
    ("filmcraft", "filmcraft-ui-egui", "Nonlinear Video Editing"),
    ("effectcraft", "effectcraft-ui-egui", "Motion Graphics & Compositing"),
    ("cadcraft", "cadcraft-ui-egui", "Precision 2D/3D CAD Drafting"),
    ("soundcraft", "soundcraft-ui-egui", "Digital Audio Workstation & DSP"),
    ("wordcraft", "wordcraft", "Structured Document Authoring"),
    ("gridcraft", "gridcraft-ui-egui", "Calculation Engine & Spreadsheets"),
    ("deckcraft", "deckcraft-ui-egui", "Slide Presentations & Vector Shapes"),
    ("figcraft", "figcraft-ui", "Collaborative Interface Design"),
    ("artcraft", "artcraft", "ArtCraft Suite"),
    ("craft-launcher", "craft-launcher", "Craft Launcher"),
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsumerAdoption {
    pub name: String,
    pub target_crate: String,
    pub target_product: String,
    pub exists: bool,
    pub pinned_revision: Option<String>,
    pub pinned_version: Option<String>,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdoptionReport {
    pub version: String,
    pub target_revision: String,
    pub timestamp: String,
    pub consumers: BTreeMap<String, ConsumerAdoption>,
}

pub fn extract_craft_ui_pin(lockfile_content: &str) -> Option<(String, String)> {
    let mut in_craft_ui = false;
    let mut version = None;
    let mut source = None;

    for line in lockfile_content.lines() {
        let trimmed = line.trim();
        if trimmed == "[[package]]" {
            if let (true, Some(v), Some(s)) = (in_craft_ui, version.take(), source.take()) {
                return Some((v, s));
            }
            in_craft_ui = false;
        } else if trimmed == "name = \"craft-ui\"" {
            in_craft_ui = true;
        } else if in_craft_ui && trimmed.starts_with("version = ") {
            version = trimmed.strip_prefix("version = ").map(|s| s.trim_matches('"').to_string());
        } else if in_craft_ui && trimmed.starts_with("source = ") {
            source = trimmed.strip_prefix("source = ").map(|s| s.trim_matches('"').to_string());
        }
    }

    if let (true, Some(v), Some(s)) = (in_craft_ui, version, source) {
        return Some((v, s));
    }

    None
}

pub fn parse_rev_from_source(source: &str) -> Option<String> {
    // Cargo.lock records exact resolved Git commits after '#':
    // git+https://github.com/storytold/craft-libs?rev=...#<resolved-commit-sha>
    if let Some(pos) = source.rfind('#') {
        let rev = &source[pos + 1..];
        let trimmed = rev.split(['&', '"']).next().unwrap_or(rev);
        if trimmed.len() >= 7 {
            return Some(trimmed.to_string());
        }
    }
    if let Some(pos) = source.find("rev=") {
        let rest = &source[pos + 4..];
        let rev = rest.split(['#', '&', '"']).next().unwrap_or(rest);
        return Some(rev.to_string());
    }
    None
}

pub fn run_adoption(root_dir: &Path, consumers_dir: &Path, target_rev: &str, json_output: bool) -> Result<(), String> {
    // Resolve target revision SHA
    let target_sha = {
        let out = Command::new("git")
            .current_dir(root_dir)
            .args(["rev-parse", target_rev])
            .output()
            .map_err(|e| format!("git rev-parse {target_rev}: {e}"))?;
        if !out.status.success() {
            return Err(format!("cannot resolve revision '{target_rev}' in craft-libs"));
        }
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };

    let mut report_consumers = BTreeMap::new();

    for &(name, crate_name, product) in ALL_APPS {
        let app_dir = consumers_dir.join(name);
        if !app_dir.exists() {
            report_consumers.insert(
                name.to_string(),
                ConsumerAdoption {
                    name: name.to_string(),
                    target_crate: crate_name.to_string(),
                    target_product: product.to_string(),
                    exists: false,
                    pinned_revision: None,
                    pinned_version: None,
                    status: "NOT_CHECKED_OUT".into(),
                },
            );
            continue;
        }

        let lock_path = app_dir.join("Cargo.lock");
        if !lock_path.exists() {
            report_consumers.insert(
                name.to_string(),
                ConsumerAdoption {
                    name: name.to_string(),
                    target_crate: crate_name.to_string(),
                    target_product: product.to_string(),
                    exists: true,
                    pinned_revision: None,
                    pinned_version: None,
                    status: "NO_LOCKFILE".into(),
                },
            );
            continue;
        }

        let lock_content = fs::read_to_string(&lock_path).unwrap_or_default();
        if let Some((ver, src)) = extract_craft_ui_pin(&lock_content) {
            let rev = parse_rev_from_source(&src).unwrap_or_else(|| src.clone());
            let is_up_to_date = rev == target_sha || (target_sha.starts_with(&rev) && rev.len() >= 7);
            let status = if is_up_to_date { "UP_TO_DATE" } else { "UPDATE_AVAILABLE" };

            report_consumers.insert(
                name.to_string(),
                ConsumerAdoption {
                    name: name.to_string(),
                    target_crate: crate_name.to_string(),
                    target_product: product.to_string(),
                    exists: true,
                    pinned_revision: Some(rev),
                    pinned_version: Some(ver),
                    status: status.into(),
                },
            );
        } else {
            report_consumers.insert(
                name.to_string(),
                ConsumerAdoption {
                    name: name.to_string(),
                    target_crate: crate_name.to_string(),
                    target_product: product.to_string(),
                    exists: true,
                    pinned_revision: None,
                    pinned_version: None,
                    status: "NOT_ADOPTED".into(),
                },
            );
        }
    }

    let report = AdoptionReport {
        version: "1.0".into(),
        target_revision: target_sha.clone(),
        timestamp: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs().to_string()).unwrap_or_default(),
        consumers: report_consumers,
    };

    let report_json = serde_json::to_string_pretty(&report).map_err(|e| format!("serialize: {e}"))?;

    if json_output {
        println!("{report_json}");
    } else {
        println!("==========================================================================================");
        println!("Craft Suite Dependency Adoption Report");
        println!("Target shared library: craft-libs ({})", &target_sha[0..8.min(target_sha.len())]);
        println!("==========================================================================================");
        println!("{:<13} {:<34} {:<18} {:<18}", "Consumer", "Domain Role", "Pinned Revision", "Status");
        println!("{:-<13} {:-<34} {:-<18} {:-<18}", "", "", "", "");

        for c in report.consumers.values() {
            let rev_display = c.pinned_revision.as_ref().map(|r| if r.len() >= 8 { &r[0..8] } else { r.as_str() }).unwrap_or("—");
            println!("{:<13} {:<34} {:<18} {:<18}", c.name, c.target_product, rev_display, c.status);
        }
        println!();
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_craft_ui_pin() {
        let lock = r#"
[[package]]
name = "craft-ui"
version = "0.1.0"
source = "git+https://github.com/storytold/craft-libs?rev=5c874d51eeb3de70e735c8ce87a59a7dedd47675#5c874d51eeb3de70e735c8ce87a59a7dedd47675"
dependencies = [
 "egui",
]
"#;
        let res = extract_craft_ui_pin(lock);
        assert!(res.is_some());
        let (ver, src) = res.unwrap();
        assert_eq!(ver, "0.1.0");
        assert!(src.contains("5c874d51"));
        assert_eq!(parse_rev_from_source(&src), Some("5c874d51eeb3de70e735c8ce87a59a7dedd47675".into()));
    }

    #[test]
    fn test_not_adopted_when_missing() {
        let lock = r#"
[[package]]
name = "serde"
version = "1.0.0"
"#;
        assert!(extract_craft_ui_pin(lock).is_none());
    }
}
