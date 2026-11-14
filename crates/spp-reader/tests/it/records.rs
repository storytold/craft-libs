//! Runs the record decoders over real projects named by `TC_IMPORT_CORPUS` (ignored by default; skipped without it).
//! No bytes from those files are kept in the repository.

use crate::common;

use std::path::Path;
use texturecraft_import_reader::decode::image::ImageHeader;
use texturecraft_import_reader::decode::resource_toc::ResourceToc;
use texturecraft_import_reader::decode::settings::ProjectSettings;
use texturecraft_import_reader::decode::texture_cache::CacheHeader;
use texturecraft_import_reader::project::{ProjectFile, ResourceKind, paths};

/// Enough for every header seen so far (they are under 500 bytes).
const PREFIX: usize = 4096;

fn check_project(path: &Path) -> Vec<String> {
    let mut failures = Vec::new();
    let mut fail = |what: &str, e: &dyn std::fmt::Display| failures.push(format!("{}: {what}: {e}", path.display()));
    let project = match ProjectFile::open(path) {
        Ok(p) => p,
        Err(e) => {
            fail("open", &e);
            return failures;
        }
    };
    for (dataset, parse) in [(paths::PROJECT_SETTINGS, 0), (paths::RESOURCE_TOC, 1)] {
        if project.layout().dataset(dataset).is_none() {
            continue;
        }
        let data = project.read(dataset).unwrap();
        let result = if parse == 0 { ProjectSettings::parse(&data).map(drop) } else { ResourceToc::parse(&data).map(drop) };
        if let Err(e) = result {
            fail(dataset, &e);
        }
    }
    for r in project.resources() {
        let prefix = project.read_prefix(&r.path, PREFIX).unwrap();
        if r.file_name.contains('/') || ResourceKind::classify(&r.file_name, &prefix) != ResourceKind::Image {
            continue;
        }
        if let Err(e) = ImageHeader::parse(&prefix) {
            fail(&r.path, &e);
        }
    }
    for entry in project.cache_entries() {
        let prefix = project.read_prefix(&entry.path, PREFIX).unwrap();
        if let Err(e) = CacheHeader::parse(&prefix) {
            fail(&entry.path, &e);
        }
    }
    failures
}

#[test]
#[ignore = "needs TC_IMPORT_CORPUS"]
fn decodes_records_of_every_corpus_project() {
    let Some(files) = common::files() else {
        eprintln!("{} not set; skipping", common::CORPUS_ENV);
        return;
    };
    let failures: Vec<String> = files.iter().flat_map(|f| check_project(f)).collect();
    assert!(failures.is_empty(), "{} failures:\n{}", failures.len(), failures.join("\n"));
    eprintln!("checked {} projects", files.len());
}
