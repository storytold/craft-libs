#![cfg(not(target_arch = "wasm32"))]

use craft_testkit::{SourceLimits, rust_sources_with_limits};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

/// A scratch folder that this test made and removes on drop, also when the test panics.
struct ScratchDir(PathBuf);

impl ScratchDir {
    fn new(tag: &str) -> Self {
        let base = std::env::temp_dir();
        for n in 0..1000u32 {
            let path = base.join(format!("craft_testkit_limits_{tag}_{}_{n}", std::process::id()));
            match std::fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
                Err(e) => panic!("create scratch folder {}: {e}", path.display()),
            }
        }
        panic!("no free scratch folder name for {tag}");
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn write(&self, relative: &str, content: &str) {
        let path = self.0.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create parent folder");
        }
        std::fs::write(path, content).expect("write file");
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn three_files(tag: &str) -> ScratchDir {
    let dir = ScratchDir::new(tag);
    dir.write("src/a.rs", "0123456789");
    dir.write("src/b.rs", "0123456789");
    dir.write("src/c.rs", "0123456789");
    dir
}

#[test]
fn defaults_are_documented_values() {
    let limits = SourceLimits::default();
    assert_eq!(limits.max_file_bytes, 8 << 20);
    assert_eq!(limits.max_total_bytes, 256 << 20);
    assert_eq!(limits.max_files, 50_000);
}

#[test]
fn files_within_the_limits_are_read() {
    let dir = three_files("within");
    let limits = SourceLimits { max_file_bytes: 10, max_total_bytes: 30, max_files: 3 };
    let sources = rust_sources_with_limits(dir.path(), limits).expect("within limits");
    assert_eq!(sources.len(), 3);
    assert!(sources.iter().all(|(_, content)| content == "0123456789"));
}

#[test]
fn a_file_over_the_size_limit_is_an_error() {
    let dir = three_files("file");
    let limits = SourceLimits { max_file_bytes: 9, ..SourceLimits::default() };
    let err = rust_sources_with_limits(dir.path(), limits).expect_err("file over the limit");
    assert_eq!(err.kind(), ErrorKind::FileTooLarge);
}

#[test]
fn files_over_the_total_limit_are_an_error() {
    let dir = three_files("total");
    let limits = SourceLimits { max_total_bytes: 29, ..SourceLimits::default() };
    let err = rust_sources_with_limits(dir.path(), limits).expect_err("total over the limit");
    assert_eq!(err.kind(), ErrorKind::QuotaExceeded);
}

#[test]
fn more_files_than_the_count_limit_is_an_error() {
    let dir = three_files("count");
    let limits = SourceLimits { max_files: 2, ..SourceLimits::default() };
    let err = rust_sources_with_limits(dir.path(), limits).expect_err("too many files");
    assert_eq!(err.kind(), ErrorKind::QuotaExceeded);
}

#[test]
fn files_the_filter_skips_do_not_count_against_the_limits() {
    let dir = three_files("skipped");
    dir.write("src/tests.rs", &"x".repeat(100));
    dir.write("tests/big.rs", &"x".repeat(100));
    let limits = SourceLimits { max_file_bytes: 10, max_total_bytes: 30, max_files: 3 };
    assert_eq!(rust_sources_with_limits(dir.path(), limits).expect("skipped files are not read").len(), 3);
}
