//! `craft_platform::temp`.
#![allow(clippy::unwrap_used, reason = "tests fail by panicking")]

use crate::temp::*;

// TempDir is mode 0700 on Unix, its files 0600, and the folder and its files are gone after drop
// unless kept.
#[test]
fn private_dir_is_owner_only_and_removed_on_drop() {
    let dir = TempDir::new("craft-temp-test").unwrap();
    let path = dir.path().to_path_buf();
    assert!(is_private_dir(&std::fs::symlink_metadata(&path).unwrap()));
    let (file_path, _file) = dir.create("a.txt").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o700);
        assert_eq!(std::fs::metadata(&file_path).unwrap().permissions().mode() & 0o777, 0o600);
    }
    assert!(dir.create("a.txt").is_err(), "an existing name is refused");
    assert!(dir.create("../b.txt").is_err(), "a name with a separator is refused");
    drop(dir);
    assert!(!path.exists());

    let kept = TempDir::new("craft-temp-test").unwrap().keep();
    assert!(kept.is_dir());
    std::fs::remove_dir_all(kept).unwrap();
}
