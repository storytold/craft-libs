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
    discover_with(root, files, &metadata_following_links);
}

/// The metadata of what a path points at, so a link to a folder or project counts as that folder or project.
fn metadata_following_links(path: &Path) -> std::io::Result<std::fs::Metadata> {
    std::fs::metadata(path)
}

/// `discover` with the metadata lookup passed in, so a test can make it fail without changing file permissions.
fn discover_with(root: &Path, files: &mut Vec<PathBuf>, metadata_of: &dyn Fn(&Path) -> std::io::Result<std::fs::Metadata>) {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let read = std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("cannot read the corpus folder {}: {e}", dir.display()));
        for entry in read {
            let path = entry.unwrap_or_else(|e| panic!("cannot read an entry of the corpus folder {}: {e}", dir.display())).path();
            let metadata = match metadata_of(&path) {
                Ok(metadata) => metadata,
                // a link whose target is gone is not a project
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => panic!("cannot read the metadata of {}: {e}", path.display()),
            };

            if metadata.is_dir() {
                stack.push(path);
            } else if metadata.is_file() && path.extension().is_some_and(|e| e.eq_ignore_ascii_case(PROJECT_EXT)) {
                files.push(path);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch folder that is removed when the test ends, also when it panics.
    struct ScratchFolder(PathBuf);

    impl ScratchFolder {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!("craft_spp_reader_it_{name}_{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(path.join("nested")).unwrap();
            Self(path)
        }
    }

    impl Drop for ScratchFolder {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn discover_finds_projects_in_nested_folders() {
        let folder = ScratchFolder::new("discover_nested");
        let top = folder.0.join(format!("top.{PROJECT_EXT}"));
        let nested = folder.0.join("nested").join(format!("deep.{PROJECT_EXT}"));
        std::fs::write(&top, b"").unwrap();
        std::fs::write(&nested, b"").unwrap();
        std::fs::write(folder.0.join("notes.md"), b"").unwrap();
        let mut files = Vec::new();

        discover(&folder.0, &mut files);

        let mut expected = vec![top, nested];
        files.sort();
        expected.sort();
        assert_eq!(files, expected);
    }

    #[test]
    #[should_panic(expected = "cannot read the metadata of")]
    fn discover_stops_when_an_entry_cannot_be_classified() {
        let folder = ScratchFolder::new("discover_metadata_error");
        std::fs::write(folder.0.join(format!("hidden.{PROJECT_EXT}")), b"").unwrap();
        let denied = |_: &Path| Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied));
        let mut files = Vec::new();

        discover_with(&folder.0, &mut files, &denied);
    }

    #[test]
    fn discover_skips_an_entry_that_vanished() {
        let folder = ScratchFolder::new("discover_vanished");
        std::fs::write(folder.0.join(format!("gone.{PROJECT_EXT}")), b"").unwrap();
        let vanished = |_: &Path| Err(std::io::Error::from(std::io::ErrorKind::NotFound));
        let mut files = Vec::new();

        discover_with(&folder.0, &mut files, &vanished);

        assert!(files.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn discover_follows_a_link_to_a_folder() {
        let folder = ScratchFolder::new("discover_link");
        let outside = ScratchFolder::new("discover_link_target");
        let project = outside.0.join(format!("linked.{PROJECT_EXT}"));
        std::fs::write(&project, b"").unwrap();
        std::os::unix::fs::symlink(&outside.0, folder.0.join("link")).unwrap();
        let mut files = Vec::new();

        discover(&folder.0, &mut files);

        assert_eq!(files, vec![folder.0.join("link").join(format!("linked.{PROJECT_EXT}"))]);
    }
}
