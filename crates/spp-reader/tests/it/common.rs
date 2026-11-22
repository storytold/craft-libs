//! Test helper: the projects named by `TC_IMPORT_CORPUS` (folders searched for project files, `.txt` lists of paths,
//! or single files, separated like `PATH`). Tests over real projects are `#[ignore]` and skip when it is not set;
//! nothing read from those files is ever kept in the repository.

use craft_spp_reader::format_names::PROJECT_EXT;
use std::path::{Path, PathBuf};

/// The environment variable naming the projects.
#[allow(dead_code)]
pub const CORPUS_ENV: &str = "TC_IMPORT_CORPUS";

/// The project files `TC_IMPORT_CORPUS` names, sorted and without duplicates; `None` when it is not set.
#[allow(dead_code)]
pub fn files() -> Option<Vec<PathBuf>> {
    let value = std::env::var_os(CORPUS_ENV)?;
    let mut files = Vec::new();
    for entry in std::env::split_paths(&value) {
        if entry.is_dir() {
            discover(&entry, &mut files);
        } else if entry.extension().is_some_and(|e| e.eq_ignore_ascii_case("txt")) {
            match std::fs::read_to_string(&entry) {
                Ok(list) => files.extend(list.lines().map(str::trim).filter(|l| !l.is_empty()).map(PathBuf::from)),
                Err(_) => files.push(entry),
            }
        } else {
            files.push(entry);
        }
    }
    files.sort();
    files.dedup();
    Some(files)
}

/// A folder or entry that cannot be read stops the test: an incomplete corpus must not pass as a clean one.
fn discover(root: &Path, files: &mut Vec<PathBuf>) {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let read = std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("cannot read the corpus folder {}: {e}", dir.display()));
        for entry in read {
            let path = entry.unwrap_or_else(|e| panic!("cannot read an entry of the corpus folder {}: {e}", dir.display())).path();
            if path.is_dir() {
                stack.push(path);
            } else if path.is_file() && path.extension().is_some_and(|e| e.eq_ignore_ascii_case(PROJECT_EXT)) {
                files.push(path);
            }
        }
    }
}
